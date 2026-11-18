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
    /// STEM ISO gain of each source (same order): 0 kills it, 1 leaves it, up to +6 dB
    /// (`stem_iso_gain` turns a knob position into this).
    pub stem_iso: [f32; 4],
    /// Search (scan): 0 = off; otherwise the speed and direction (e.g. 8.0 forward, -8.0
    /// backward). Short grains are played at normal pitch and the position skips between
    /// them, as a CDJ's SEARCH buttons do; it sounds whether or not the deck is playing.
    pub search: f64,
    /// Master mute (the level is kept).
    pub muted: bool,
    /// Playing speed: 1.0 = the track's own tempo (TEMPO − / +, typed BPM, BEAT SYNC).
    pub tempo: f64,
    /// Pitch bend: added to `tempo` while a bend button is held.
    pub bend: f64,
    /// Master Tempo (key lock): speed changes leave the pitch alone.
    pub master_tempo: bool,
    /// Key shift in semitones (KEY SYNC, key shift ‹ ›), independent of the tempo.
    pub key_shift: f32,
    /// Crossfader gain of this deck (1.0 outside the two-deck layouts).
    pub fader: f32,
    /// The 3-band isolator is in the signal path (Dual Player), with these band gains
    /// (low, mid, high; 0.0 = killed).
    pub eq_on: bool,
    pub eq_gain: [f32; 3],
    /// Beat times of the loaded track (seconds), for BEAT SYNC.
    pub grid: Option<Arc<Vec<f64>>>,
    /// BEAT SYNC: this deck follows the other deck's beat phase while both play.
    pub sync_follow: bool,
    /// Speed correction the audio callback applies to hold the beat phase (BEAT SYNC).
    pub sync_bend: f64,
    click_phase: usize,
    next_click: usize,
    search_count: usize,
    dsp: Box<DeckDsp>,
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
            stem_iso: [1.0; 4],
            search: 0.0,
            muted: false,
            tempo: 1.0,
            bend: 0.0,
            master_tempo: false,
            key_shift: 0.0,
            fader: 1.0,
            eq_on: false,
            eq_gain: [1.0; 3],
            grid: None,
            sync_follow: false,
            sync_bend: 0.0,
            click_phase: usize::MAX,
            next_click: 0,
            search_count: 0,
            dsp: Box::new(DeckDsp::new()),
        }
    }
}

pub struct Player {
    /// The active deck's playback state.
    pub state: Arc<Mutex<PlayState>>,
    /// The other deck's (2 Players layout); both are mixed to the output. The two are swapped
    /// when the active deck changes, so `state` always belongs to the deck being operated.
    pub other: Arc<Mutex<PlayState>>,
    /// Set by the stream when its device stops working (unplugged, switched off).
    pub lost: Arc<std::sync::atomic::AtomicBool>,
    _stream: Option<cpal::Stream>,
    pub device_rate: u32,
    pub error: Option<String>,
}

impl Player {
    #[allow(dead_code)]
    pub fn new() -> Player {
        Self::with_device(None)
    }

    /// Open the named output device (None: the system's default output).
    pub fn with_device(device: Option<&str>) -> Player {
        let state = Arc::new(Mutex::new(PlayState::default()));
        let other = Arc::new(Mutex::new(PlayState::default()));
        let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match Self::open_stream([state.clone(), other.clone()], device, lost.clone()) {
            Ok((s, rate)) => Player { state, other, lost, _stream: Some(s), device_rate: rate, error: None },
            Err(e) => Player { state, other, lost, _stream: None, device_rate: 44100, error: Some(e.to_string()) },
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
        let (s, rate) = Self::open_stream([self.state.clone(), self.other.clone()], device, self.lost.clone())?;
        self.lost.store(false, std::sync::atomic::Ordering::Relaxed);
        self._stream = Some(s);
        self.device_rate = rate;
        self.error = None;
        Ok(())
    }

    fn open_stream(state: [Arc<Mutex<PlayState>>; 2], device: Option<&str>, lost: Arc<std::sync::atomic::AtomicBool>) -> Result<(cpal::Stream, u32)> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let named = device.and_then(|want| Self::find_device(want, std::time::Duration::from_secs(3)));
        // A device that is no longer connected falls back to the default output.
        let dev = match named {
            Some(d) => d,
            None => host.default_output_device().ok_or_else(|| anyhow!("no audio output device"))?,
        };
        // Sample rate, buffer size and output channel pair from Preferences > Audio.
        let (cfg, config, first_ch) = crate::prefs_ext::stream_config(&dev)?;
        let rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let st = state.clone();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => {
                let lost = lost.clone();
                let mut pair: Vec<f32> = Vec::with_capacity(8192);
                dev.build_output_stream(
                &config,
                move |data: &mut [f32], _| if first_ch == 0 { render(&st, data, channels, rate) } else { crate::prefs_ext::render_pair(data, channels, first_ch, &mut pair, |b| render(&st, b, 2, rate)) },
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

// ---------------------------------------------------------------------- deck DSP

/// Crossfader law for a position `x` from 0.0 (deck A's end) to 1.0 (deck B's end): both decks
/// at full level in the middle, the far deck falling smoothly (a quarter sine) to silence over
/// its half of the travel. Returns (gain of A, gain of B).
pub fn crossfader_gains(x: f32) -> (f32, f32) {
    let x = x.clamp(0.0, 1.0);
    let g = |d: f32| (std::f32::consts::FRAC_PI_2 * (2.0 * d).clamp(0.0, 1.0)).sin();
    (g(1.0 - x), g(x))
}

/// How far through its beat a deck is at `t` seconds: 0.0 on a beat, 0.5 halfway to the
/// next. Before the first and after the last beat the nearest interval is continued. None
/// without at least two beats.
pub fn beat_phase(beats: &[f64], t: f64) -> Option<f64> {
    if beats.len() < 2 {
        return None;
    }
    let n = beats.len();
    let i = beats.partition_point(|b| *b <= t);
    let (a, len) = if i == 0 {
        (beats[0], beats[1] - beats[0])
    } else if i >= n {
        (beats[n - 1], beats[n - 1] - beats[n - 2])
    } else {
        (beats[i - 1], beats[i] - beats[i - 1])
    };
    if len <= 0.0 {
        return None;
    }
    Some(((t - a) / len).rem_euclid(1.0))
}

/// Beat phase of the follower minus the master's, wrapped to -0.5..0.5 beats (positive: the
/// follower is ahead).
pub fn phase_error(follower: &[f64], t_f: f64, master: &[f64], t_m: f64) -> Option<f64> {
    let e = beat_phase(follower, t_f)? - beat_phase(master, t_m)?;
    Some((e + 0.5).rem_euclid(1.0) - 0.5)
}

/// Speed correction (fraction of normal speed) that pulls a phase error of `e` beats back to
/// zero: proportional, at most 8 %, nothing inside a 0.3 % dead zone.
pub fn sync_correction(e: f64) -> f64 {
    if e.abs() < 0.003 {
        0.0
    } else {
        (-e).clamp(-0.08, 0.08)
    }
}

/// The speed at which a track of `own_bpm` plays at `master_bpm`. When the tempos are nearer
/// an octave apart the half or double speed is matched instead (a 70 BPM track against 140).
pub fn sync_rate(master_bpm: f64, own_bpm: f64) -> f64 {
    if master_bpm <= 0.0 || own_bpm <= 0.0 {
        return 1.0;
    }
    let r = master_bpm / own_bpm;
    [r, r * 2.0, r / 2.0].into_iter().min_by(|a, b| a.ln().abs().total_cmp(&b.ln().abs())).unwrap_or(1.0)
}

/// One second-order filter section for both channels (transposed direct form II).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    /// kind: 0 low-pass, 1 high-pass, 2 all-pass; Butterworth Q.
    fn set(&mut self, kind: u8, f: f32, rate: f32) {
        let w = 2.0 * std::f32::consts::PI * (f / rate).min(0.45);
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let b = match kind {
            0 => [(1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0],
            1 => [(1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0],
            _ => [1.0 - alpha, -2.0 * cs, 1.0 + alpha],
        };
        self.b = [b[0] / a0, b[1] / a0, b[2] / a0];
        self.a = [-2.0 * cs / a0, (1.0 - alpha) / a0];
    }
    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.b[0] * x + self.z1[ch];
        self.z1[ch] = self.b[1] * x - self.a[0] * y + self.z2[ch];
        self.z2[ch] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Crossover frequencies of the isolator (Hz): LOW below the first, HIGH above the second.
/// rekordbox's isolator crossovers (`Isolator::applyFilter`, 0x102ab9828 in 7.2.19).
pub const ISO_LOW_HZ: f32 = 284.0;
pub const ISO_HIGH_HZ: f32 = 4649.0;

/// 3-band isolator: fourth-order Linkwitz-Riley crossovers, whose bands add back to the
/// input's level at every frequency (the low band passes an all-pass matching the upper
/// crossover's phase), so with all three bands on it is inaudible and each band can be
/// switched off completely.
#[derive(Clone, Copy, Default)]
pub struct Iso3 {
    rate: u32,
    lp1: [Biquad; 2],
    hp1: [Biquad; 2],
    lp2: [Biquad; 2],
    hp2: [Biquad; 2],
    ap: Biquad,
    /// Band gains in use (moving toward the deck's `eq_gain`).
    g: [f32; 3],
}

impl Iso3 {
    pub fn new() -> Iso3 {
        Iso3 { g: [1.0; 3], ..Default::default() }
    }
    fn prepare(&mut self, rate: u32) {
        if self.rate == rate {
            return;
        }
        *self = Iso3 { rate, g: self.g, ..Default::default() };
        let r = rate as f32;
        for k in 0..2 {
            self.lp1[k].set(0, ISO_LOW_HZ, r);
            self.hp1[k].set(1, ISO_LOW_HZ, r);
            self.lp2[k].set(0, ISO_HIGH_HZ, r);
            self.hp2[k].set(1, ISO_HIGH_HZ, r);
        }
        self.ap.set(2, ISO_HIGH_HZ, r);
    }
    /// One sample of channel `ch` with the band gains `g`.
    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let low = self.lp1[0].run(ch, x);
        let low = self.lp1[1].run(ch, low);
        let rest = self.hp1[0].run(ch, x);
        let rest = self.hp1[1].run(ch, rest);
        let mid = self.lp2[0].run(ch, rest);
        let mid = self.lp2[1].run(ch, mid);
        let high = self.hp2[0].run(ch, rest);
        let high = self.hp2[1].run(ch, high);
        self.g[0] * self.ap.run(ch, low) + self.g[1] * mid + self.g[2] * high
    }
    /// A mono signal through the isolator at fixed band gains (tests and offline use).
    #[allow(dead_code)]
    pub fn process(rate: u32, gains: [f32; 3], x: &[f32]) -> Vec<f32> {
        let mut iso = Iso3::new();
        iso.prepare(rate);
        iso.g = gains;
        x.iter().map(|v| iso.run(0, *v)).collect()
    }
}

/// The loaded audio as the callback reads it: the original mix and, once the track is
/// separated, its parts, each with the gain in force for the frame being rendered.
///
/// As rekordbox 7.2.19 mixes them (`StretchableAudioSource::read`, 0x10287fa18): while no
/// part is muted or moved off unity, only the original file plays (bit for bit); otherwise
/// the original fades out and the audible parts fade in. The instruments part is what is
/// left of the mix after the other parts, so all parts together are always the mix.
struct Src<'a> {
    audio: &'a Decoded,
    frames: usize,
    stems: Option<&'a crate::stems::Stems>,
    /// Gains for this frame: the original, then drums, bass, other, vocals.
    g: std::cell::Cell<[f32; 5]>,
    stem_ratio: f64,
}

impl Src<'_> {
    /// Left and right at source frame `p` (fractional).
    #[inline]
    fn at(&self, p: f64) -> (f32, f32) {
        let p = p.max(0.0);
        // 4-point cubic (Catmull-Rom) between frames: exact at whole positions, far less
        // high-frequency loss than linear interpolation when the file's rate differs from
        // the device's (e.g. 44.1 kHz files on a 48 kHz output).
        let i = p as usize;
        let fr = (p - i as f64) as f32;
        let frames = self.frames as isize;
        let at = |k: isize, c: usize| self.audio.samples[(k.clamp(0, frames - 1) as usize) * 2 + c];
        let ii = i as isize;
        let (ol, or) = (cubic(at(ii - 1, 0), at(ii, 0), at(ii + 1, 0), at(ii + 2, 0), fr), cubic(at(ii - 1, 1), at(ii, 1), at(ii + 1, 1), at(ii + 2, 1), fr));
        let g = self.g.get();
        let st = match self.stems {
            Some(st) if st.len > 1 && (g[1] != 0.0 || g[2] != 0.0 || g[3] != 0.0 || g[4] != 0.0) => st,
            _ => return (ol * g[0], or * g[0]),
        };
        let sp = p * self.stem_ratio;
        let si = (sp as usize).min(st.len - 2);
        let sf = (sp - si as f64) as f32;
        let n = st.len as isize;
        let k = si as isize;
        let part = |src: usize| -> (f32, f32) {
            let lb = src * 2 * st.len;
            let rb = lb + st.len;
            let at = |base: usize, k: isize| st.data[base + k.clamp(0, n - 1) as usize];
            (cubic(at(lb, k - 1), at(lb, k), at(lb, k + 1), at(lb, k + 2), sf), cubic(at(rb, k - 1), at(rb, k), at(rb, k + 1), at(rb, k + 2), sf))
        };
        let (d, b, v) = (part(0), part(1), part(3));
        // Instruments: the mix less drums, bass and vocals.
        let o = (ol - d.0 - b.0 - v.0, or - d.1 - b.1 - v.1);
        (g[0] * ol + g[1] * d.0 + g[2] * b.0 + g[3] * o.0 + g[4] * v.0, g[0] * or + g[1] * d.1 + g[2] * b.1 + g[3] * o.1 + g[4] * v.1)
    }
    /// Left + right of whole source frame `i` (for aligning grains).
    #[inline]
    fn mono(&self, i: isize) -> f32 {
        let k = i.clamp(0, self.frames as isize - 1) as usize * 2;
        self.audio.samples[k] + self.audio.samples[k + 1]
    }
}

const ST_WIN: usize = 16384;
const ST_TMPL: usize = 1024;

/// Time stretcher (WSOLA: waveform-similarity overlap-add). The output is made of 40 ms
/// grains of the source, read at the wanted pitch and cross-faded every 20 ms; each new grain
/// starts where the play position says, moved by up to ±12 ms to the place where it best
/// continues the grain before it, so periodic material stays in phase across the joins. The
/// play position itself advances at exactly the wanted speed. It reads the source directly
/// (nothing is queued, no latency) and its buffers are allocated once, off the audio thread.
struct Stretch {
    active: bool,
    /// Source frame where the grain fading in began, and the one fading out.
    cur: f64,
    prev: f64,
    /// The two grains are the same stretch of audio (nothing to cross-fade).
    same: bool,
    /// Output frames into the current hop, and the hop length.
    n: usize,
    hop: usize,
    /// Source frames per output frame the grains are read at.
    ps: f64,
    /// The play position after the last frame, to notice seeks and loop jumps.
    expect: f64,
    win: Vec<f32>,
    tmpl: Vec<f32>,
    off: Vec<u32>,
}

impl Stretch {
    fn new() -> Stretch {
        Stretch { active: false, cur: 0.0, prev: 0.0, same: true, n: 0, hop: 882, ps: 1.0, expect: 0.0, win: vec![0.0; ST_WIN], tmpl: vec![0.0; ST_TMPL], off: vec![0; ST_TMPL] }
    }

    /// Where the next grain starts: near `nominal`, at the offset (within ±12 ms) whose audio
    /// best matches what follows the current grain (`natural`).
    fn pick(&mut self, src: &Src, nominal: f64, natural: f64) -> f64 {
        let sr = src.audio.sr as f64;
        let mut dec = 4usize;
        while self.hop / dec > ST_TMPL {
            dec *= 2;
        }
        let d = self.ps * dec as f64;
        let mut m = (self.hop / dec).max(1);
        let delta = ((0.012 * sr) as usize).clamp(8, ST_WIN / 4);
        while m > 8 && 2 * delta + (m as f64 * d) as usize + 4 > ST_WIN {
            m /= 2;
        }
        let span = 2 * delta + ((m - 1) as f64 * d).round() as usize + 2;
        if span > ST_WIN {
            return nominal;
        }
        let base = nominal.floor() as isize - delta as isize;
        for (i, w) in self.win[..span].iter_mut().enumerate() {
            *w = src.mono(base + i as isize);
        }
        let mut te = 0.0f32;
        for j in 0..m {
            let o = j as f64 * d;
            self.off[j] = o.round() as u32;
            let v = src.mono((natural + o).round() as isize);
            self.tmpl[j] = v;
            te += v * v;
        }
        if te < 1e-9 {
            return nominal;
        }
        let (tmpl, off, win) = (&self.tmpl[..m], &self.off[..m], &self.win[..span]);
        let score = |k: usize| -> f32 {
            let (mut c, mut e) = (0.0f32, 0.0f32);
            for j in 0..m {
                let v = win[k + off[j] as usize];
                c += tmpl[j] * v;
                e += v * v;
            }
            c / (e + 1e-9).sqrt()
        };
        let (mut best, mut best_k) = (f32::MIN, delta);
        let mut k = 0;
        while k <= 2 * delta {
            let sc = score(k);
            if sc > best {
                best = sc;
                best_k = k;
            }
            k += 2;
        }
        for k in [best_k.saturating_sub(1), (best_k + 1).min(2 * delta)] {
            let sc = score(k);
            if sc > best {
                best = sc;
                best_k = k;
            }
        }
        (base + best_k as isize) as f64 + (nominal - nominal.floor())
    }

    /// One output frame with the play position at source frame `pos`, read at `ps` source
    /// frames per output frame (the pitch); `hop` output frames per grain step.
    #[inline]
    fn frame(&mut self, src: &Src, pos: f64, ps: f64, hop: usize) -> (f32, f32) {
        let h = hop as f64;
        if !self.active || self.hop != hop {
            // Start: both grains are the audio from here on.
            self.active = true;
            self.hop = hop;
            self.ps = ps;
            self.cur = pos;
            self.prev = pos - h * ps;
            self.same = true;
            self.n = 0;
        } else if (pos - self.expect).abs() > 0.5 {
            // The position was moved (seek, loop, hot cue): fade from what was sounding into
            // the new place.
            self.prev = self.cur + (self.n as f64 - h) * self.ps;
            self.ps = ps;
            self.cur = pos;
            self.same = false;
            self.n = 0;
        } else if self.n >= hop {
            let natural = self.cur + h * self.ps;
            self.prev = self.cur;
            self.cur = if (ps - self.ps).abs() < 1e-12 { self.pick(src, pos, natural) } else { pos };
            // The grain fading out keeps the pitch it was started with.
            self.prev += h * (self.ps - ps);
            self.ps = ps;
            self.same = (self.cur - natural).abs() < 1e-6;
            self.n = 0;
        }
        let n = self.n as f64;
        self.n += 1;
        let i = src.at(self.cur + n * self.ps);
        if self.same {
            return i;
        }
        let o = src.at(self.prev + (h + n) * self.ps);
        let w = 0.5 - 0.5 * (std::f32::consts::PI * ((n as f32 + 0.5) / h as f32)).cos();
        (o.0 + (i.0 - o.0) * w, o.1 + (i.1 - o.1) * w)
    }
}

/// Per-deck signal processing state owned by the audio callback.
struct DeckDsp {
    st: Stretch,
    eq: Iso3,
    /// Crossfader gain in use (moving toward the deck's `fader`).
    fader: f32,
    /// Gains in use for the original and the four parts (moving toward their targets).
    stem_g: [f32; 5],
}

impl DeckDsp {
    fn new() -> DeckDsp {
        DeckDsp { st: Stretch::new(), eq: Iso3::new(), fader: 1.0, stem_g: [1.0, 0.0, 0.0, 0.0, 0.0] }
    }
}

/// BEAT SYNC, on the audio thread where both decks' positions are read at the same instant:
/// the speed correction that holds `follower` on the other deck's beat phase.
fn update_sync(follower: &mut PlayState, master: &PlayState) {
    follower.sync_bend = 0.0;
    if !follower.sync_follow || !follower.playing || !master.playing || follower.search != 0.0 || master.search != 0.0 {
        return;
    }
    let (Some(gf), Some(gm), Some(af), Some(am)) = (&follower.grid, &master.grid, &follower.audio, &master.audio) else { return };
    if let Some(e) = phase_error(gf, follower.pos / af.sr as f64, gm, master.pos / am.sr as f64) {
        follower.sync_bend = sync_correction(e);
    }
}

/// How often the audio callback found a deck's state held by the interface, and how often it
/// had to give the buffer up (silence) because the state stayed held.
pub static AUDIO_CONTENDED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static AUDIO_DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A deck's state for the audio callback. The interface holds it for well under a microsecond
/// at a time (to read the position or set a control), but it does so many times per frame, so
/// a single `try_lock` loses now and then, and losing means a whole buffer of silence: an
/// audible click. Waiting out such a hold costs far less than a buffer, so the callback spins
/// for it, and gives up only after a quarter of a millisecond (something is then really wrong).
fn lock_for_audio(m: &Mutex<PlayState>) -> Option<parking_lot::MutexGuard<'_, PlayState>> {
    use std::sync::atomic::Ordering::Relaxed;
    if let Some(g) = m.try_lock() {
        return Some(g);
    }
    AUDIO_CONTENDED.fetch_add(1, Relaxed);
    let t = std::time::Instant::now();
    loop {
        for _ in 0..64 {
            std::hint::spin_loop();
        }
        if let Some(g) = m.try_lock() {
            return Some(g);
        }
        if t.elapsed() > std::time::Duration::from_micros(250) {
            AUDIO_DROPPED.fetch_add(1, Relaxed);
            return None;
        }
    }
}

/// The audio callback: both decks mixed into `out`.
fn render(states: &[Arc<Mutex<PlayState>>; 2], out: &mut [f32], channels: usize, rate: u32) {
    for v in out.iter_mut() {
        *v = 0.0;
    }
    let mut a = lock_for_audio(&states[0]);
    let mut b = lock_for_audio(&states[1]);
    if let (Some(a), Some(b)) = (a.as_deref_mut(), b.as_deref_mut()) {
        update_sync(a, b);
        update_sync(b, a);
    }
    for s in [a.as_deref_mut(), b.as_deref_mut()].into_iter().flatten() {
        render_deck(s, out, channels, rate);
    }
}

/// Time a part takes to fade fully in or out when it is muted or unmuted.
pub const STEM_RAMP_SECS: f32 = 0.0033;

/// STEM ISO: the gain for a knob position (0 = fully left, 0.5 = centre, 1 = fully right), as
/// rekordbox 7.2.19's 256-step curve (`partIsoCurve`, read at `round(value * 255)`): kill at
/// the left end with a soft toe, a straight line up to unity, a flat zone around the centre
/// (steps 124 to 131, so a centred knob leaves the original playing), then a straight line
/// up to +6 dB.
pub fn stem_iso_gain(value: f32) -> f32 {
    let i = (value.clamp(0.0, 1.0) * 255.0).round() as i32;
    match i {
        0..=9 => 0.0006 * (i * i) as f32,
        10 => 0.0591,
        11 => 0.0691,
        12 => 0.0791,
        13 => 0.0895,
        14..=123 => 1.0 - (124 - i) as f32 / 122.0,
        124..=131 => 1.0,
        132..=252 => 1.0 + (i - 131) as f32 * (0.9952623 / 122.0),
        _ => 1.9952623,
    }
}

/// Frames per search grain at the device rate (60 ms).
fn search_grain(rate: u32) -> usize {
    (rate as usize * 60 / 1000).max(1)
}

/// Add one deck's audio to `out` (which the caller has cleared) and advance its position.
pub fn render_deck(s: &mut PlayState, out: &mut [f32], channels: usize, rate: u32) {
    let searching = s.search != 0.0;
    if !s.playing && !searching {
        s.search_count = 0;
        s.dsp.st.active = false;
        return;
    }
    let Some(audio) = s.audio.clone() else { return };
    let step = audio.sr as f64 / rate as f64;
    let frames = audio.frames();
    let vol = if s.muted { 0.0 } else { s.volume * s.gain * if searching { 0.7 } else { 1.0 } };
    let click_len = (rate as f64 * 0.03) as usize;
    let stems = s.stems.clone();
    // Where the gains of the original and the parts are heading. The parts are used only
    // while one of them is muted or off unity; until then the original plays untouched.
    let use_parts = stems.is_some() && (0..4).any(|i| !s.stem_on[i] || s.stem_iso[i] != 1.0);
    let mut stem_target = [if use_parts { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0, 0.0];
    if use_parts {
        for i in 0..4 {
            stem_target[1 + i] = if s.stem_on[i] { s.stem_iso[i].clamp(0.0, 10.0) } else { 0.0 };
        }
    }
    if stems.is_none() {
        s.dsp.stem_g = [1.0, 0.0, 0.0, 0.0, 0.0];
    }
    // A full on/off change takes 3.3 ms (rekordbox's `udsp::GainController`: 145.53 frames
    // at 44.1 kHz), at a constant slope, so smaller moves are proportionally quicker.
    let stem_k = 1.0 / (STEM_RAMP_SECS * rate as f32);
    let src = Src { audio: &audio, frames, stems: stems.as_deref(), g: std::cell::Cell::new(s.dsp.stem_g), stem_ratio: crate::stems::htdemucs::SAMPLE_RATE as f64 / audio.sr.max(1) as f64 };
    let metro = if searching { None } else { s.metronome.clone() };
    let grain = search_grain(rate);
    // Speed and pitch for this buffer. The pitch follows the speed unless Master Tempo holds
    // it; the key shift is on top of either.
    let speed = if searching { 1.0 } else { (s.tempo + s.bend + s.sync_bend).clamp(0.05, 4.0) };
    let pitch = if searching { 1.0 } else { (if s.master_tempo { 1.0 } else { speed }) * 2f64.powf(s.key_shift as f64 / 12.0) };
    let stretch = (pitch - speed).abs() > 1e-9;
    let hop = ((rate as usize) / 50).clamp(64, 4096);
    let fader_k = (1.0 / (0.005 * rate as f32)).min(1.0);
    let eq_on = s.eq_on;
    if eq_on {
        s.dsp.eq.prepare(rate);
    }
    if !stretch {
        s.dsp.st.active = false;
    }
    for f in out.chunks_mut(channels) {
        if searching {
            // After each grain the position skips so that, overall, it moves at `search`
            // times normal speed (backward for negative values).
            s.search_count += 1;
            if s.search_count >= grain {
                s.search_count = 0;
                s.pos = (s.pos + (s.search - 1.0) * grain as f64 * step).clamp(0.0, frames.saturating_sub(2) as f64);
            }
        } else if let Some((a, b)) = s.loop_range {
            let bf = b * audio.sr as f64;
            if s.pos >= bf {
                s.pos = a * audio.sr as f64 + (s.pos - bf);
            }
        }
        let p = s.pos;
        if p as usize + 1 >= frames {
            s.playing = false;
            s.pos = frames.saturating_sub(1) as f64;
            break;
        }
        if s.dsp.stem_g != stem_target {
            for (g, t) in s.dsp.stem_g.iter_mut().zip(stem_target) {
                *g += (t - *g).clamp(-stem_k, stem_k);
            }
            src.g.set(s.dsp.stem_g);
        }
        let (mut l, mut r) = if stretch { s.dsp.st.frame(&src, p, pitch * step, hop) } else { src.at(p) };
        if eq_on {
            let d = &mut s.dsp;
            for k in 0..3 {
                d.eq.g[k] += (s.eq_gain[k] - d.eq.g[k]) * fader_k;
            }
            l = d.eq.run(0, l);
            r = d.eq.run(1, r);
        }
        s.dsp.fader += (s.fader - s.dsp.fader) * fader_k;
        let vol = vol * s.dsp.fader;
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
                click = (s.click_phase as f32 * 2.0 * std::f32::consts::PI * 1500.0 / rate as f32).sin() * env * s.metronome_vol * if s.muted { 0.0 } else { 1.0 };
                s.click_phase += 1;
            }
        }
        if channels >= 2 {
            f[0] += l * vol + click;
            f[1] += r * vol + click;
        } else {
            f[0] += 0.5 * (l + r) * vol + click;
        }
        s.pos += step * speed;
        s.dsp.st.expect = s.pos;
    }
}

/// Metronome click level for the three volume steps of the GRID panel (1 = quiet … 3 = loud).
pub fn metronome_level(step: u8) -> f32 {
    match step {
        0 | 1 => 0.2,
        2 => 0.45,
        _ => 0.9,
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

#[cfg(test)]
mod deck_mix_tests {
    use super::*;

    fn dc(level: f32, frames: usize) -> Arc<Decoded> {
        Arc::new(Decoded { sr: 44100, samples: vec![level; frames * 2], raw_frames: frames, damaged: 0, truncated: false })
    }
    fn playing(level: f32) -> PlayState {
        PlayState { audio: Some(dc(level, 44100)), playing: true, volume: 1.0, gain: 1.0, ..Default::default() }
    }

    #[test]
    fn two_decks_are_summed_and_a_paused_deck_is_silent() {
        let (mut a, mut b) = (playing(0.25), playing(0.5));
        let mut out = vec![0.0f32; 512];
        render_deck(&mut a, &mut out, 2, 44100);
        render_deck(&mut b, &mut out, 2, 44100);
        assert!(out.iter().all(|v| (*v - 0.75).abs() < 1e-5), "{:?}", &out[..4]);
        assert_eq!(a.pos, 256.0);
        assert_eq!(b.pos, 256.0);
        // Deck B paused: only deck A sounds, and B's position stays.
        b.playing = false;
        let mut out = vec![0.0f32; 512];
        render_deck(&mut a, &mut out, 2, 44100);
        render_deck(&mut b, &mut out, 2, 44100);
        assert!(out.iter().all(|v| (*v - 0.25).abs() < 1e-5));
        assert_eq!(b.pos, 256.0);
        // Mute silences a deck but it keeps moving.
        a.muted = true;
        let mut out = vec![0.0f32; 512];
        render_deck(&mut a, &mut out, 2, 44100);
        assert!(out.iter().all(|v| *v == 0.0));
        assert_eq!(a.pos, 768.0);
    }

    #[test]
    fn search_moves_at_its_speed_in_both_directions_even_when_paused() {
        let mut s = PlayState { audio: Some(dc(0.5, 44100 * 60)), volume: 1.0, ..Default::default() };
        s.pos = 44100.0 * 30.0;
        s.search = 8.0;
        let mut out = vec![0.0f32; 44100 * 2];
        render_deck(&mut s, &mut out, 2, 44100);
        // One second of output: about eight seconds of track, and it is audible.
        let moved = (s.pos - 44100.0 * 30.0) / 44100.0;
        assert!((moved - 8.0).abs() < 0.5, "{moved}");
        assert!(out.iter().any(|v| *v > 0.1));
        s.search = -8.0;
        let from = s.pos;
        let mut out = vec![0.0f32; 44100 * 2];
        render_deck(&mut s, &mut out, 2, 44100);
        let moved = (s.pos - from) / 44100.0;
        assert!((moved + 8.0).abs() < 0.5, "{moved}");
        // Search off and not playing: silent, position kept.
        s.search = 0.0;
        let from = s.pos;
        let mut out = vec![0.0f32; 512];
        render_deck(&mut s, &mut out, 2, 44100);
        assert!(out.iter().all(|v| *v == 0.0));
        assert_eq!(s.pos, from);
    }

    #[test]
    fn metronome_has_three_rising_levels() {
        assert!(metronome_level(1) < metronome_level(2) && metronome_level(2) < metronome_level(3));
        let mut s = playing(0.0);
        s.metronome = Some(Arc::new(vec![0.0]));
        let peak = |s: &mut PlayState, step: u8| {
            s.pos = 0.0;
            s.next_click = 0;
            s.click_phase = usize::MAX;
            s.metronome_vol = metronome_level(step);
            let mut out = vec![0.0f32; 2048];
            render_deck(s, &mut out, 2, 44100);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };
        let (p1, p2, p3) = (peak(&mut s, 1), peak(&mut s, 2), peak(&mut s, 3));
        assert!(p1 > 0.05 && p2 > p1 * 1.5 && p3 > p2 * 1.5, "{p1} {p2} {p3}");
    }
}

#[cfg(test)]
mod dsp_tests {
    use super::*;

    fn tone(freq: f64, secs: f64, amp: impl Fn(f64) -> f32) -> Arc<Decoded> {
        let sr = 44100u32;
        let n = (secs * sr as f64) as usize;
        let mut samples = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f64 / sr as f64;
            let v = (2.0 * std::f64::consts::PI * freq * t).sin() as f32 * amp(t);
            samples.push(v);
            samples.push(v);
        }
        Arc::new(Decoded { sr, samples, raw_frames: n, damaged: 0, truncated: false })
    }

    fn deck(a: Arc<Decoded>) -> PlayState {
        PlayState { audio: Some(a), playing: true, volume: 1.0, gain: 1.0, ..Default::default() }
    }

    /// Render `secs` of output in device-sized buffers; left channel.
    fn run(s: &mut PlayState, secs: f64) -> Vec<f32> {
        let mut all = vec![];
        let mut buf = vec![0.0f32; 1024];
        let total = (secs * 44100.0) as usize;
        while all.len() < total {
            let n = (total - all.len()).min(512);
            buf[..n * 2].iter_mut().for_each(|v| *v = 0.0);
            render_deck(s, &mut buf[..n * 2], 2, 44100);
            all.extend(buf[..n * 2].chunks(2).map(|f| f[0]));
        }
        all
    }

    /// Frequency of a steady tone from its upward zero crossings (first to last).
    fn freq_of(x: &[f32]) -> f64 {
        let mut first = None;
        let mut last = 0.0f64;
        let mut count = 0usize;
        for i in 1..x.len() {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                let t = (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64;
                if first.is_none() {
                    first = Some(t);
                } else {
                    count += 1;
                }
                last = t;
            }
        }
        count as f64 / ((last - first.unwrap_or(0.0)) / 44100.0)
    }

    fn cents(f: f64, reference: f64) -> f64 {
        1200.0 * (f / reference).log2()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn tempo_without_master_tempo_changes_speed_and_pitch_together() {
        let mut s = deck(tone(440.0, 12.0, |_| 0.5));
        s.tempo = 1.1;
        let out = run(&mut s, 4.0);
        assert!((s.pos - 4.0 * 44100.0 * 1.1).abs() < 1.0, "{}", s.pos);
        assert!(cents(freq_of(&out[4410..]), 484.0).abs() < 2.0, "{}", freq_of(&out[4410..]));
    }

    #[test]
    fn master_tempo_keeps_the_pitch_within_a_few_cents_and_the_tempo_exact() {
        for (tempo, f) in [(1.16, 440.0), (0.84, 440.0), (1.06, 110.0), (0.92, 2000.0), (1.03, 55.0)] {
            let mut s = deck(tone(f, 14.0, |_| 0.5));
            s.tempo = tempo;
            s.master_tempo = true;
            let out = run(&mut s, 5.0);
            // The position moved at exactly the tempo…
            assert!((s.pos - 5.0 * 44100.0 * tempo).abs() < 1.0, "tempo {tempo}: {}", s.pos);
            // …the pitch stayed…
            let c = cents(freq_of(&out[8820..]), f);
            assert!(c.abs() < 3.0, "tempo {tempo}, {f} Hz: {c} cents");
            // …and the level did too (no holes or doubling at the joins).
            let lv = rms(&out[8820..]) / (0.5 * std::f32::consts::FRAC_1_SQRT_2);
            assert!((lv - 1.0).abs() < 0.03, "tempo {tempo}, {f} Hz: level {lv}");
        }
    }

    #[test]
    fn master_tempo_output_follows_the_source_position() {
        // The tone gets louder at 4.0 s of the track; at 125 % that is heard after 3.2 s.
        let mut s = deck(tone(330.0, 12.0, |t| if t < 4.0 { 0.2 } else { 0.8 }));
        s.tempo = 1.25;
        s.master_tempo = true;
        let out = run(&mut s, 5.0);
        let at = |t: f64| rms(&out[(t * 44100.0) as usize..((t + 0.02) * 44100.0) as usize]);
        assert!(at(3.15) < 0.2, "{}", at(3.15));
        assert!(at(3.25) > 0.5, "{}", at(3.25));
    }

    #[test]
    fn key_shift_moves_the_pitch_by_semitones_at_the_same_speed() {
        for semis in [3.0f32, -2.0, 12.0, -5.0] {
            let mut s = deck(tone(440.0, 10.0, |_| 0.5));
            s.key_shift = semis;
            let out = run(&mut s, 4.0);
            assert!((s.pos - 4.0 * 44100.0).abs() < 1.0);
            let want = 440.0 * 2f64.powf(semis as f64 / 12.0);
            let c = cents(freq_of(&out[8820..]), want);
            assert!(c.abs() < 3.0, "{semis} semitones: {c} cents");
        }
        // With a tempo change and Master Tempo off the shift is on top of the tempo's pitch.
        let mut s = deck(tone(440.0, 10.0, |_| 0.5));
        s.key_shift = 2.0;
        s.tempo = 1.05;
        let out = run(&mut s, 4.0);
        let want = 440.0 * 1.05 * 2f64.powf(2.0 / 12.0);
        assert!(cents(freq_of(&out[8820..]), want).abs() < 3.0);
        assert!((s.pos - 4.0 * 44100.0 * 1.05).abs() < 1.0);
    }

    #[test]
    fn a_seek_while_stretching_lands_on_the_new_position() {
        let mut s = deck(tone(440.0, 20.0, |t| if t < 10.0 { 0.2 } else { 0.8 }));
        s.tempo = 0.9;
        s.master_tempo = true;
        run(&mut s, 1.0);
        s.pos = 12.0 * 44100.0;
        let out = run(&mut s, 0.5);
        assert!(rms(&out[4410..]) > 0.5);
        assert!((s.pos - (12.0 + 0.45) * 44100.0).abs() < 2.0);
    }

    #[test]
    fn pitch_bend_adds_to_the_tempo_while_it_is_held() {
        let mut s = deck(tone(440.0, 10.0, |_| 0.5));
        s.tempo = 1.02;
        s.bend = 0.04;
        run(&mut s, 1.0);
        assert!((s.pos - 44100.0 * 1.06).abs() < 1.0);
        s.bend = 0.0;
        let from = s.pos;
        run(&mut s, 1.0);
        assert!((s.pos - from - 44100.0 * 1.02).abs() < 1.0);
    }

    #[test]
    fn crossfader_law() {
        // Middle: both decks at full level. Ends: one deck only.
        assert_eq!(crossfader_gains(0.5), (1.0, 1.0));
        let (a, b) = crossfader_gains(0.0);
        assert!(a == 1.0 && b == 0.0);
        let (a, b) = crossfader_gains(1.0);
        assert!(a.abs() < 1e-6 && b == 1.0);
        // Deck B fades in over the first half and deck A out over the second, without steps.
        let mut prev = crossfader_gains(0.0);
        for i in 1..=100 {
            let g = crossfader_gains(i as f32 / 100.0);
            assert!(g.0 <= prev.0 + 1e-6 && g.1 >= prev.1 - 1e-6);
            assert!((g.0 - prev.0).abs() < 0.04 && (g.1 - prev.1).abs() < 0.04);
            prev = g;
        }
        assert!((crossfader_gains(0.75).0 - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        // Symmetric.
        assert!((crossfader_gains(0.3).1 - crossfader_gains(0.7).0).abs() < 1e-6);
    }

    #[test]
    fn crossfader_gain_reaches_the_output_smoothly() {
        let dc = Arc::new(Decoded { sr: 44100, samples: vec![0.5; 44100 * 2], raw_frames: 44100, damaged: 0, truncated: false });
        let mut s = deck(dc);
        s.fader = 0.0;
        let out = run(&mut s, 0.2);
        // No step at the start, silent once the fade is through.
        assert!(out.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01));
        assert!(out[4410..].iter().all(|v| v.abs() < 1e-4));
    }

    #[test]
    fn isolator_band_gains() {
        let sine = |f: f64| -> Vec<f32> { (0..44100).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / 44100.0).sin() as f32).collect() };
        let level = |g: [f32; 3], f: f64| -> f32 { rms(&Iso3::process(44100, g, &sine(f))[22050..]) / std::f32::consts::FRAC_1_SQRT_2 };
        let db = |x: f32| 20.0 * x.max(1e-9).log10();
        // All bands on: the level is unchanged everywhere, the crossovers included.
        for f in [30.0, 60.0, 284.0, 700.0, 1000.0, 4649.0, 8000.0, 15000.0] {
            assert!(db(level([1.0; 3], f)).abs() < 0.1, "{f} Hz: {} dB", db(level([1.0; 3], f)));
        }
        // LOW off: bass gone, mids and highs untouched; -6 dB at the crossover.
        assert!(db(level([0.0, 1.0, 1.0], 60.0)) < -45.0);
        assert!(db(level([0.0, 1.0, 1.0], 1000.0)).abs() < 0.2);
        assert!(db(level([0.0, 1.0, 1.0], 8000.0)).abs() < 0.1);
        assert!((db(level([0.0, 1.0, 1.0], ISO_LOW_HZ as f64)) + 6.0).abs() < 0.3);
        // MID off: only the band between the crossovers goes.
        assert!(db(level([1.0, 0.0, 1.0], 800.0)) < -18.0);
        assert!(db(level([1.0, 0.0, 1.0], 40.0)).abs() < 0.2);
        assert!(db(level([1.0, 0.0, 1.0], 12000.0)).abs() < 0.2);
        // HIGH off.
        assert!(db(level([1.0, 1.0, 0.0], 18000.0)) < -40.0);
        assert!(db(level([1.0, 1.0, 0.0], 100.0)).abs() < 0.1);
        // Everything off is silence.
        assert!(db(level([0.0; 3], 1000.0)) < -100.0);
    }

    #[test]
    fn isolator_in_the_deck_follows_the_kill_switches() {
        let mut s = deck(tone(60.0, 10.0, |_| 0.5));
        s.eq_on = true;
        let on = rms(&run(&mut s, 1.0)[22050..]);
        s.eq_gain = [0.0, 1.0, 1.0];
        let off = rms(&run(&mut s, 1.0)[22050..]);
        assert!((on - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01, "{on}");
        assert!(off < 0.002, "{off}");
    }

    #[test]
    fn sync_phase_maths() {
        let grid = |first: f64, bpm: f64, n: usize| -> Vec<f64> { (0..n).map(|i| first + i as f64 * 60.0 / bpm).collect() };
        let a = grid(0.5, 120.0, 400);
        assert!((beat_phase(&a, 0.5).unwrap()).abs() < 1e-9);
        assert!((beat_phase(&a, 0.75).unwrap() - 0.5).abs() < 1e-9);
        // Before the first and after the last beat the grid is continued.
        assert!((beat_phase(&a, 0.125).unwrap() - 0.25).abs() < 1e-9);
        assert!((beat_phase(&a, 0.5 + 400.0 * 0.5 + 0.1).unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(beat_phase(&[1.0], 1.0), None);
        // A 128 BPM track against it: on its beat while the master is a quarter beat in.
        let b = grid(0.2, 128.0, 400);
        let e = phase_error(&b, b[10], &a, a[7] + 0.125).unwrap();
        assert!((e + 0.25).abs() < 1e-9, "{e}");
        // Errors wrap to the nearest beat.
        let e = phase_error(&b, b[10] + 0.9 * 60.0 / 128.0, &a, a[7]).unwrap();
        assert!((e + 0.1).abs() < 1e-9, "{e}");
        // Ahead: slow down; behind: speed up; tiny errors are left alone; bounded.
        assert!(sync_correction(0.1) < 0.0 && sync_correction(-0.1) > 0.0);
        assert_eq!(sync_correction(0.001), 0.0);
        assert_eq!(sync_correction(0.5), -0.08);
        // Tempo match, with the octave rule.
        assert!((sync_rate(128.0, 120.0) - 128.0 / 120.0).abs() < 1e-12);
        assert!((sync_rate(140.0, 70.0) - 1.0).abs() < 1e-12);
        assert!((sync_rate(87.0, 174.0) - 1.0).abs() < 1e-12);
        assert_eq!(sync_rate(0.0, 120.0), 1.0);
    }

    #[test]
    fn beat_sync_pulls_the_follower_onto_the_masters_beats() {
        // Master at 120 BPM, follower a 126 BPM track played at 120/126 and started 80 ms late.
        let grid = |first: f64, bpm: f64| -> Arc<Vec<f64>> { Arc::new((0..600).map(|i| first + i as f64 * 60.0 / bpm).collect()) };
        let silence = || Arc::new(Decoded { sr: 44100, samples: vec![0.0; 44100 * 2 * 60], raw_frames: 44100 * 60, damaged: 0, truncated: false });
        let m = Arc::new(Mutex::new(PlayState { grid: Some(grid(0.0, 120.0)), ..deck(silence()) }));
        let f = Arc::new(Mutex::new(PlayState { grid: Some(grid(0.3, 126.0)), tempo: 120.0 / 126.0, sync_follow: true, master_tempo: true, ..deck(silence()) }));
        f.lock().pos = (0.3 + 0.08) * 44100.0;
        let states = [m.clone(), f.clone()];
        let mut buf = vec![0.0f32; 1024];
        for _ in 0..(10 * 44100 / 512) {
            render(&states, &mut buf, 2, 44100);
        }
        let (pm, pf) = (m.lock().pos / 44100.0, f.lock().pos / 44100.0);
        let e = phase_error(f.lock().grid.as_ref().unwrap(), pf, m.lock().grid.as_ref().unwrap(), pm).unwrap();
        assert!(e.abs() < 0.01, "phase error after 10 s: {e} beats");
        // Without BEAT SYNC the offset stays.
        f.lock().sync_follow = false;
        f.lock().pos += 0.1 * 44100.0;
        for _ in 0..(2 * 44100 / 512) {
            render(&states, &mut buf, 2, 44100);
        }
        let (pm, pf) = (m.lock().pos / 44100.0, f.lock().pos / 44100.0);
        let e = phase_error(f.lock().grid.as_ref().unwrap(), pf, m.lock().grid.as_ref().unwrap(), pm).unwrap();
        assert!(e.abs() > 0.15, "{e}");
    }
}

#[cfg(test)]
mod stem_mix_tests {
    use super::*;

    /// A track whose parts are known: original = drums + bass + other + vocals, where the
    /// stored "other" is deliberately wrong (the instruments are taken as the residual).
    fn deck() -> (PlayState, Vec<f32>, [Vec<f32>; 4]) {
        let n = 44100;
        let tone = |f: f32, a: f32| -> Vec<f32> { (0..n).map(|i| (i as f32 * f * std::f32::consts::TAU / 44100.0).sin() * a).collect() };
        let parts = [tone(60.0, 0.2), tone(110.0, 0.15), tone(440.0, 0.1), tone(880.0, 0.12)];
        let mix: Vec<f32> = (0..n).map(|i| parts.iter().map(|p| p[i]).sum()).collect();
        let mut data = Vec::new();
        for (k, p) in parts.iter().enumerate() {
            let stored: Vec<f32> = if k == 2 { p.iter().map(|x| x * 0.5).collect() } else { p.clone() };
            data.extend_from_slice(&stored);
            data.extend_from_slice(&stored);
        }
        let audio = Decoded { sr: 44100, samples: mix.iter().flat_map(|x| [*x, *x]).collect(), raw_frames: n, damaged: 0, truncated: false };
        let s = PlayState { audio: Some(Arc::new(audio)), stems: Some(Arc::new(crate::stems::Stems { len: n, data })), playing: true, volume: 1.0, gain: 1.0, ..Default::default() };
        (s, mix, parts)
    }

    fn run(s: &mut PlayState, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        render_deck(s, &mut out, 2, 44100);
        out.chunks(2).map(|c| c[0]).collect()
    }

    #[test]
    fn all_parts_on_is_the_original_bit_for_bit() {
        let (mut s, mix, _) = deck();
        let out = run(&mut s, 4000);
        assert!(out.iter().zip(&mix).all(|(a, b)| a == b));
    }

    #[test]
    fn a_muted_part_fades_out_over_3_3_ms_and_the_rest_is_the_mix_without_it() {
        let (mut s, mix, parts) = deck();
        run(&mut s, 1000);
        s.stem_on[3] = false; // vocals
        let out = run(&mut s, 1000);
        // No jump at the switch: the first frame is still (almost) the whole mix.
        assert!((out[0] - mix[1000]).abs() < 0.01, "{} vs {}", out[0], mix[1000]);
        // Half way through the fade about half of the vocal is left.
        let half = 1000 + 73;
        let left = (out[73] - (mix[half] - parts[3][half])) / parts[3][half];
        assert!((left - 0.5).abs() < 0.08, "{left}");
        // After 146 frames (3.3 ms) it is the mix less the vocals, exactly as rekordbox sums it.
        for i in 150..1000 {
            let want = mix[1000 + i] - parts[3][1000 + i];
            assert!((out[i] - want).abs() < 1e-4, "frame {i}: {} vs {want}", out[i]);
        }
        // Back on: the original returns, bit for bit once the fade is over.
        s.stem_on[3] = true;
        let out = run(&mut s, 1000);
        assert!(out[200..].iter().zip(&mix[2200..]).all(|(a, b)| a == b));
    }

    #[test]
    fn instruments_are_what_is_left_of_the_mix() {
        let (mut s, mix, parts) = deck();
        s.stem_on = [false, false, true, false];
        let out = run(&mut s, 2000);
        // The stored "other" is half level; the residual is the true instruments part.
        for i in 300..2000 {
            assert!((out[i] - parts[2][i]).abs() < 1e-4, "frame {i}");
        }
        let _ = mix;
    }

    #[test]
    fn stem_iso_curve_and_gain() {
        assert_eq!(stem_iso_gain(0.0), 0.0);
        assert_eq!(stem_iso_gain(0.5), 1.0);
        assert!((stem_iso_gain(1.0) - 1.9952623).abs() < 1e-6);
        assert!((stem_iso_gain(64.0 / 255.0) - 0.5082).abs() < 1e-4);
        assert!((stem_iso_gain(16.0 / 255.0) - 0.1148).abs() < 1e-4);
        assert!((stem_iso_gain(132.0 / 255.0) - 1.0081578).abs() < 1e-5);
        assert!((stem_iso_gain(9.0 / 255.0) - 0.0486).abs() < 1e-6);
        // Drums at +6 dB: the mix plus one more helping of drums.
        let (mut s, mix, parts) = deck();
        s.stem_iso[0] = stem_iso_gain(1.0);
        let out = run(&mut s, 2000);
        for i in 300..2000 {
            assert!((out[i] - (mix[i] + 0.9952623 * parts[0][i])).abs() < 1e-4);
        }
    }
}
