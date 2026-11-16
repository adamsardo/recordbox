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
    if let Some(n) = track.codec_params.n_frames {
        out.reserve((n as usize).min(60 * 60 * 48000) * 2);
    }
    let mut sbuf: Option<SampleBuffer<f32>> = None;
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
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
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
        } else {
            for f in s.chunks_exact(ch) {
                out.push(f[0]);
                out.push(f[1]);
            }
        }
    }
    let mut raw_frames = out.len() / 2;
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
    Ok(Decoded { sr, samples: out, raw_frames })
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
    _stream: Option<cpal::Stream>,
    pub device_rate: u32,
    pub error: Option<String>,
}

impl Player {
    pub fn new() -> Player {
        let state = Arc::new(Mutex::new(PlayState::default()));
        match Self::open_stream(state.clone()) {
            Ok((s, rate)) => Player { state, _stream: Some(s), device_rate: rate, error: None },
            Err(e) => Player { state, _stream: None, device_rate: 44100, error: Some(e.to_string()) },
        }
    }

    fn open_stream(state: Arc<Mutex<PlayState>>) -> Result<(cpal::Stream, u32)> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let dev = host.default_output_device().ok_or_else(|| anyhow!("no audio output device"))?;
        let cfg = dev.default_output_config()?;
        let rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let config: cpal::StreamConfig = cfg.clone().into();
        let st = state.clone();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => dev.build_output_stream(
                &config,
                move |data: &mut [f32], _| render(&st, data, channels, rate),
                |e| eprintln!("audio error: {e}"),
                None,
            )?,
            _ => {
                let st2 = st.clone();
                dev.build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        let mut tmp = vec![0f32; data.len()];
                        render(&st2, &mut tmp, channels, rate);
                        for (d, s) in data.iter_mut().zip(tmp) {
                            *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                    },
                    |e| eprintln!("audio error: {e}"),
                    None,
                )?
            }
        };
        stream.play()?;
        Ok((stream, rate))
    }
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
                for (src, on) in stem_on.iter().enumerate() {
                    if *on {
                        let lb = src * 2 * st.len;
                        let rb = lb + st.len;
                        l += st.data[lb + si] * (1.0 - sf) + st.data[lb + si + 1] * sf;
                        r += st.data[rb + si] * (1.0 - sf) + st.data[rb + si + 1] * sf;
                    }
                }
                (l, r)
            }
            _ => (audio.samples[i * 2] * (1.0 - fr) + audio.samples[i * 2 + 2] * fr, audio.samples[i * 2 + 1] * (1.0 - fr) + audio.samples[i * 2 + 3] * fr),
        };
        let mut click = 0.0f32;
        if let Some(beats) = s.metronome.clone() {
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
