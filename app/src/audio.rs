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

/// Decode a whole file. The source file is only read.
pub fn decode_file(path: &str) -> Result<Decoded> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?;
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
    if out.is_empty() {
        return Err(anyhow!("no audio decoded"));
    }
    Ok(Decoded { sr, samples: out })
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
        let l = audio.samples[i * 2] * (1.0 - fr) + audio.samples[i * 2 + 2] * fr;
        let r = audio.samples[i * 2 + 1] * (1.0 - fr) + audio.samples[i * 2 + 3] * fr;
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
