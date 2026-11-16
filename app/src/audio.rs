//! Audio decoding (symphonia) and the preview player (cpal).

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::sync::Arc;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decoded audio, interleaved stereo f32.
pub struct Decoded {
    pub sr: u32,
    pub samples: Vec<f32>,
    /// Length in sample frames of the file's full (untrimmed) decode.
    pub raw_frames: usize,
    /// Packets that could not be decoded and were skipped (damaged file).
    pub damaged: usize,
    /// The file ends more than a second before the length its header declares (cut short).
    pub truncated: bool,
}

impl Decoded {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sr.max(1) as f64
    }
    pub fn mono(&self) -> Vec<f32> {
        self.samples.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect()
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Mp3Timing {
    NotMp3,
    Lame,
    Ffmpeg,
    Untagged,
}

/// Samples dropped from the start of MP3s without an encoder tag (one MPEG-1 Layer III frame).
pub const MP3_UNTAGGED_SKIP: usize = 1152;

/// Look at the first MP3 frame for a LAME-style info tag and its encoder name.
pub fn mp3_timing_class(path: &str) -> Mp3Timing {
    use std::io::Read;
    if !path.to_ascii_lowercase().ends_with(".mp3") {
        return Mp3Timing::NotMp3;
    }
    let Ok(mut f) = std::fs::File::open(path) else { return Mp3Timing::NotMp3 };
    let mut head = vec![0u8; 10];
    if f.read_exact(&mut head).is_err() {
        return Mp3Timing::Untagged;
    }
    let mut skip = 0u64;
    if &head[..3] == b"ID3" {
        skip = 10 + (((head[6] as u64 & 0x7f) << 21) | ((head[7] as u64 & 0x7f) << 14) | ((head[8] as u64 & 0x7f) << 7) | (head[9] as u64 & 0x7f));
        if head[5] & 0x10 != 0 {
            skip += 10;
        }
    }
    use std::io::Seek;
    if f.seek(std::io::SeekFrom::Start(skip)).is_err() {
        return Mp3Timing::Untagged;
    }
    let mut buf = vec![0u8; 8192];
    let n = f.read(&mut buf).unwrap_or(0);
    let buf = &buf[..n];
    // first frame sync
    let Some(sync) = buf.windows(2).position(|w| w[0] == 0xff && (w[1] & 0xe0) == 0xe0) else { return Mp3Timing::Untagged };
    let frame = &buf[sync..(sync + 2000).min(buf.len())];
    let has = |pat: &[u8]| frame.windows(pat.len()).any(|w| w == pat);
    if !(has(b"Xing") || has(b"Info")) {
        return Mp3Timing::Untagged;
    }
    if has(b"LAME") || has(b"GOGO") {
        Mp3Timing::Lame
    } else if has(b"Lavc") || has(b"Lavf") {
        Mp3Timing::Ffmpeg
    } else {
        Mp3Timing::Untagged
    }
}

/// Decode a whole file. The source file is only read.
pub fn decode_file(path: &str) -> Result<Decoded> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    // MP3 start alignment follows the original's timeline (measured against its own grids,
    // see MECHANICS.md): LAME-tagged files keep the encoder lead-in, ffmpeg-tagged files have it
    // removed, untagged files lose one frame.
    let class = mp3_timing_class(path);
    let gapless = class == Mp3Timing::Ffmpeg;
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions { enable_gapless: gapless, ..Default::default() }, &MetadataOptions::default())?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no audio track"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let track_id = track.id;
    let mut sr = track.codec_params.sample_rate.unwrap_or(44100);
    let mut out: Vec<f32> = Vec::new();
    let declared_frames = track.codec_params.n_frames;
    let time_base = track.codec_params.time_base;
    if let Some(n) = track.codec_params.n_frames {
        out.reserve((n as usize).min(60 * 60 * 48000) * 2);
    }
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut damaged = 0usize;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                // Silence for the packet's length, so everything after stays where it belongs.
                damaged += 1;
                let n = (packet.dur as usize).min(1 << 20);
                out.extend(std::iter::repeat(0.0f32).take(n * 2));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        sr = spec.rate;
        let ch = spec.channels.count();
        if sbuf.as_ref().map(|b| b.capacity() < decoded.capacity() * ch).unwrap_or(true) {
            sbuf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let b = sbuf.as_mut().unwrap();
        b.copy_interleaved_ref(decoded);
        let s = b.samples();
        if ch == 1 {
            for v in s {
                out.push(*v);
                out.push(*v);
            }
        } else if ch == 2 {
            for f in s.chunks_exact(2) {
                out.push(f[0]);
                out.push(f[1]);
            }
        } else {
            // More than two channels: a standard stereo downmix by channel position (centre and
            // surrounds at -3 dB, LFE left out) instead of keeping only the first two.
            use symphonia::core::audio::Channels;
            let g = std::f32::consts::FRAC_1_SQRT_2;
            let w: Vec<(f32, f32)> = spec
                .channels
                .iter()
                .map(|c| {
                    if c == Channels::FRONT_LEFT {
                        (1.0, 0.0)
                    } else if c == Channels::FRONT_RIGHT {
                        (0.0, 1.0)
                    } else if c == Channels::FRONT_CENTRE {
                        (g, g)
                    } else if c == Channels::LFE1 || c == Channels::LFE2 {
                        (0.0, 0.0)
                    } else if c == Channels::REAR_LEFT || c == Channels::SIDE_LEFT || c == Channels::FRONT_LEFT_CENTRE || c == Channels::REAR_LEFT_CENTRE {
                        (g, 0.0)
                    } else if c == Channels::REAR_RIGHT || c == Channels::SIDE_RIGHT || c == Channels::FRONT_RIGHT_CENTRE || c == Channels::REAR_RIGHT_CENTRE {
                        (0.0, g)
                    } else {
                        (0.5 * g, 0.5 * g)
                    }
                })
                .collect();
            for f in s.chunks_exact(ch) {
                let (mut l, mut r) = (0f32, 0f32);
                for (v, (wl, wr)) in f.iter().zip(&w) {
                    l += v * wl;
                    r += v * wr;
                }
                out.push(l);
                out.push(r);
            }
        }
    }
    let decoded_frames = out.len() / 2;
    // Compared in seconds: some containers (MP4) count the declared length in their own time
    // units rather than sample frames.
    let declared_secs = declared_frames.map(|n| match time_base {
        Some(tb) => {
            let t = tb.calc_time(n);
            t.seconds as f64 + t.frac
        }
        None => n as f64 / sr.max(1) as f64,
    });
    let truncated = declared_secs.map(|d| decoded_frames as f64 / sr.max(1) as f64 + 1.0 < d).unwrap_or(false);
    let mut raw_frames = decoded_frames;
    if class == Mp3Timing::Ffmpeg {
        raw_frames += track.codec_params.delay.unwrap_or(0) as usize + track.codec_params.padding.unwrap_or(0) as usize;
    }
    if class == Mp3Timing::Untagged {
        let drop = (MP3_UNTAGGED_SKIP * 2).min(out.len());
        out.drain(..drop);
    }
    if out.is_empty() {
        return Err(anyhow!("no audio decoded"));
    }
    Ok(Decoded { sr, samples: out, raw_frames, damaged, truncated })
}

/// Shared playback state read by the audio callback.
pub struct PlayState {
    pub audio: Option<Arc<Decoded>>,
    pub pos: f64,
    pub playing: bool,
    pub volume: f32,
    pub gain: f32,
    pub loop_range: Option<(f64, f64)>,
    pub metronome: Option<Arc<Vec<f64>>>,
    pub metronome_vol: f32,
    /// Separated stems of the loaded track (44.1 kHz) and which are audible, in source order
    /// drums, bass, other, vocals. With all on (or no stems) the original mix plays.
    pub stems: Option<Arc<crate::stems::Stems>>,
    pub stem_on: [bool; 4],
    click_phase: usize,
    next_click: usize,
}

impl Default for PlayState {
    fn default() -> Self {
        PlayState {
            audio: None,
            pos: 0.0,
            playing: false,
            volume: 0.9,
            gain: 1.0,
            loop_range: None,
            metronome: None,
            metronome_vol: 0.5,
            stems: None,
            stem_on: [true; 4],
            click_phase: usize::MAX,
            next_click: 0,
        }
    }
}

pub struct Player {
    pub state: Arc<Mutex<PlayState>>,
    /// Set by the stream when its device stops working (unplugged, switched off).
    pub lost: Arc<std::sync::atomic::AtomicBool>,
    _stream: Option<cpal::Stream>,
    pub device_rate: u32,
    pub error: Option<String>,
}

impl Player {
    pub fn new() -> Player {
        Self::with_device(None)
    }

    /// Open the named output device (None: the system's default output).
    pub fn with_device(device: Option<&str>) -> Player {
        let state = Arc::new(Mutex::new(PlayState::default()));
        let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match Self::open_stream(state.clone(), device, lost.clone()) {
            Ok((s, rate)) => Player { state, lost, _stream: Some(s), device_rate: rate, error: None },
            Err(e) => Player { state, lost, _stream: None, device_rate: 44100, error: Some(e.to_string()) },
        }
    }

    /// Names of the output devices this Mac offers. Listing devices can block indefinitely on
    /// some systems (seen with a CoreAudio device on the development Mac), so it runs on its
    /// own thread and gives up after `timeout`: None then.
    pub fn output_devices_within(timeout: std::time::Duration) -> Option<Vec<String>> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait};
            let names: Vec<String> = cpal::default_host().output_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default();
            let _ = tx.send(names);
        });
        rx.recv_timeout(timeout).ok()
    }

    /// The named output device, searched on its own thread (see `output_devices_within`).
    fn find_device(want: &str, timeout: std::time::Duration) -> Option<cpal::Device> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let want = want.to_string();
        std::thread::spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait};
            let d = cpal::default_host().output_devices().ok().and_then(|mut it| it.find(|d| d.name().map(|n| n == want).unwrap_or(false)));
            let _ = tx.send(d);
        });
        rx.recv_timeout(timeout).ok().flatten()
    }

    /// Switch to another output device, keeping what is loaded and playing.
    pub fn switch_device(&mut self, device: Option<&str>) -> Result<()> {
        let (s, rate) = Self::open_stream(self.state.clone(), device, self.lost.clone())?;
        self.lost.store(false, std::sync::atomic::Ordering::Relaxed);
        self._stream = Some(s);
        self.device_rate = rate;
        self.error = None;
        Ok(())
    }

    fn open_stream(state: Arc<Mutex<PlayState>>, device: Option<&str>, lost: Arc<std::sync::atomic::AtomicBool>) -> Result<(cpal::Stream, u32)> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let named = device.and_then(|want| Self::find_device(want, std::time::Duration::from_secs(3)));
        // A device that is no longer connected falls back to the default output.
        let dev = match named {
            Some(d) => d,
            None => host.default_output_device().ok_or_else(|| anyhow!("no audio output device"))?,
        };
        let cfg = dev.default_output_config()?;
        let rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let config: cpal::StreamConfig = cfg.clone().into();
        let st = state.clone();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => {
                let lost = lost.clone();
                dev.build_output_stream(
                &config,
                move |data: &mut [f32], _| render(&st, data, channels, rate),
                move |e| {
                    eprintln!("audio error: {e}");
                    if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                        lost.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                },
                None,
            )?
            }
            _ => {
                let st2 = st.clone();
                // Scratch buffer reused across callbacks (no allocation on the audio thread once
                // it has reached the device's buffer size).
                let mut tmp: Vec<f32> = Vec::with_capacity(8192);
                dev.build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        tmp.resize(data.len(), 0.0);
                        render(&st2, &mut tmp, channels, rate);
                        for (d, s) in data.iter_mut().zip(tmp.iter().copied()) {
                            *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                    },
                    move |e| {
                        eprintln!("audio error: {e}");
                        if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                            lost.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    },
                    None,
                )?
            }
        };
        stream.play()?;
        Ok((stream, rate))
    }
}

/// Catmull-Rom interpolation between y1 (t = 0) and y2 (t = 1).
#[inline]
fn cubic(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
    let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c = -0.5 * y0 + 0.5 * y2;
    ((a * t + b) * t + c) * t + y1
}

fn render(state: &Arc<Mutex<PlayState>>, out: &mut [f32], channels: usize, rate: u32) {
    for v in out.iter_mut() {
        *v = 0.0;
    }
    let Some(mut s) = state.try_lock() else { return };
    if !s.playing {
        return;
    }
    let Some(audio) = s.audio.clone() else { return };
    let step = audio.sr as f64 / rate as f64;
    let frames = audio.frames();
    let vol = s.volume * s.gain;
    let click_len = (rate as f64 * 0.03) as usize;
    let stems = if s.stem_on.iter().all(|x| *x) { None } else { s.stems.clone() };
    let stem_on = s.stem_on;
    let stem_ratio = crate::stems::htdemucs::SAMPLE_RATE as f64 / audio.sr.max(1) as f64;
    let metro = s.metronome.clone();
    for f in out.chunks_mut(channels) {
        if let Some((a, b)) = s.loop_range {
            let bf = b * audio.sr as f64;
            if s.pos >= bf {
                s.pos = a * audio.sr as f64 + (s.pos - bf);
            }
        }
        let p = s.pos;
        let i = p as usize;
        if i + 1 >= frames {
            s.playing = false;
            s.pos = frames.saturating_sub(1) as f64;
            break;
        }
        let fr = (p - i as f64) as f32;
        let (l, r) = match &stems {
            // Some stems muted: sum the audible ones at the same position.
            Some(st) if st.len > 1 => {
                let sp = p * stem_ratio;
                let si = (sp as usize).min(st.len - 2);
                let sf = (sp - si as f64) as f32;
                let (mut l, mut r) = (0.0f32, 0.0f32);
                let n = st.len as isize;
                for (src, on) in stem_on.iter().enumerate() {
                    if *on {
                        let lb = src * 2 * st.len;
                        let rb = lb + st.len;
                        let at = |base: usize, k: isize| st.data[base + k.clamp(0, n - 1) as usize];
                        let k = si as isize;
                        l += cubic(at(lb, k - 1), at(lb, k), at(lb, k + 1), at(lb, k + 2), sf);
                        r += cubic(at(rb, k - 1), at(rb, k), at(rb, k + 1), at(rb, k + 2), sf);
                    }
                }
                (l, r)
            }
            // 4-point cubic (Catmull-Rom) between frames: exact at whole positions, far less
            // high-frequency loss than linear interpolation when the file's rate differs from
            // the device's (e.g. 44.1 kHz files on a 48 kHz output).
            _ => {
                let at = |k: isize, c: usize| audio.samples[(k.clamp(0, frames as isize - 1) as usize) * 2 + c];
                let ii = i as isize;
                (cubic(at(ii - 1, 0), at(ii, 0), at(ii + 1, 0), at(ii + 2, 0), fr), cubic(at(ii - 1, 1), at(ii, 1), at(ii + 1, 1), at(ii + 2, 1), fr))
            }
        };
        let mut click = 0.0f32;
        if let Some(beats) = metro.as_ref() {
            // The beat list can be replaced by a shorter one (another track, a grid edit).
            if s.next_click > beats.len() {
                s.next_click = 0;
            }
            let t = p / audio.sr as f64;
            while s.next_click < beats.len() && beats[s.next_click] < t - 0.05 {
                s.next_click += 1;
            }
            if s.next_click > 0 && beats[s.next_click - 1] > t {
                s.next_click = 0;
            }
            if s.next_click < beats.len() && t >= beats[s.next_click] {
                s.click_phase = 0;
                s.next_click += 1;
            }
            if s.click_phase < click_len {
                let env = 1.0 - s.click_phase as f32 / click_len as f32;
                click = (s.click_phase as f32 * 2.0 * std::f32::consts::PI * 1500.0 / rate as f32).sin() * env * s.metronome_vol;
                s.click_phase += 1;
            }
        }
        if channels >= 2 {
            f[0] = l * vol + click;
            f[1] = r * vol + click;
        } else {
            f[0] = 0.5 * (l + r) * vol + click;
        }
        s.pos += step;
    }
}

pub fn has_audio_extension(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("mp3" | "m4a" | "aac" | "mp4" | "wav" | "aif" | "aiff" | "flac" | "alac" | "ogg")
    )
}

#[cfg(test)]
mod gapless_probe {
    #[test]
    #[ignore]
    fn trim_amounts() {
        use symphonia::core::formats::FormatOptions;
        for p in std::env::var("PREPDECK_AUDIO").unwrap_or_default().split('|').filter(|s| !s.is_empty()) {
            let mut n = [0usize; 2];
            for (k, g) in [false, true].iter().enumerate() {
                let file = std::fs::File::open(p).unwrap();
                let mss = symphonia::core::io::MediaSourceStream::new(Box::new(file), Default::default());
                let mut hint = symphonia::core::probe::Hint::new();
                hint.with_extension("mp3");
                let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions { enable_gapless: *g, ..Default::default() }, &Default::default()).unwrap();
                let mut f = probed.format;
                let tr = f.default_track().unwrap().clone();
                if k == 1 { eprintln!("delay {:?} padding {:?}", tr.codec_params.delay, tr.codec_params.padding); }
                let mut dec = symphonia::default::get_codecs().make(&tr.codec_params, &Default::default()).unwrap();
                while let Ok(pk) = f.next_packet() {
                    if let Ok(b) = dec.decode(&pk) { n[k] += b.frames(); }
                }
            }
            eprintln!("raw {} gapless {} diff {} :: {}", n[0], n[1], n[0] as i64 - n[1] as i64, p.rsplit('/').next().unwrap());
        }
    }
}

#[cfg(test)]
mod device_probe {
    #[test]
    #[ignore]
    fn lists_output_devices() {
        let t = std::time::Instant::now();
        let d = super::Player::output_devices_within(std::time::Duration::from_secs(5));
        eprintln!("devices after {:?}: {d:?}", t.elapsed());
    }
}

#[cfg(test)]
mod damaged_probe {
    /// Decode a file given in PREPDECK_DAMAGED and report skipped packets. --ignored.
    #[test]
    #[ignore]
    fn reports_damaged_packets() {
        let Ok(p) = std::env::var("PREPDECK_DAMAGED") else { return };
        let d = super::decode_file(&p).unwrap();
        println!("damaged packets: {}, truncated {}, duration {:.1}s", d.damaged, d.truncated, d.duration());
        let (l, r) = d.samples.chunks_exact(2).fold((0f32, 0f32), |a, f| (a.0.max(f[0].abs()), a.1.max(f[1].abs())));
        println!("peak left {l:.3}, right {r:.3}");
    }
}

#[cfg(test)]
mod interp_tests {
    #[test]
    fn cubic_is_exact_at_frames_and_better_than_linear() {
        assert_eq!(super::cubic(0.1, 0.2, 0.7, 0.4, 0.0), 0.2);
        assert!((super::cubic(0.1, 0.2, 0.7, 0.4, 1.0) - 0.7).abs() < 1e-6);
        // A 10 kHz sine sampled at 44.1 kHz, read halfway between samples.
        let f = |n: f64| (2.0 * std::f64::consts::PI * 10_000.0 * n / 44_100.0).sin() as f32;
        let (mut e_lin, mut e_cub) = (0f32, 0f32);
        for k in 1..400 {
            let n = k as f64;
            let truth = f(n + 0.5);
            e_lin += (0.5 * (f(n) + f(n + 1.0)) - truth).abs();
            e_cub += (super::cubic(f(n - 1.0), f(n), f(n + 1.0), f(n + 2.0), 0.5) - truth).abs();
        }
        assert!(e_cub < e_lin * 0.6, "cubic {e_cub} vs linear {e_lin}");
    }
}
