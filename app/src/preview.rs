//! Track-list preview playback ("Waveform Preview"): auditions a track from the browser
//! without loading it to the player. Own decode thread and own output stream; nothing is
//! shared with the player's audio engine.

use crate::audio::Decoded;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Default)]
pub struct PvState {
    /// Track being previewed (set as soon as it is asked for, before its audio is decoded).
    pub track: Option<i64>,
    pub audio: Option<Arc<Decoded>>,
    /// Play position in source frames.
    pub pos: f64,
    pub playing: bool,
    pub volume: f32,
    pub gain: f32,
    /// Where to start once the decode finishes (fraction of the track).
    pub pending: Option<f64>,
    pub error: Option<String>,
    /// Bumped on every request, so a decode that finishes late is dropped.
    pub generation: u64,
}

pub struct Preview {
    pub state: Arc<Mutex<PvState>>,
    stream: Option<cpal::Stream>,
    device: Option<String>,
    pub error: Option<String>,
}

impl Default for Preview {
    fn default() -> Self {
        Preview { state: Arc::new(Mutex::new(PvState { volume: 1.0, gain: 1.0, ..Default::default() })), stream: None, device: None, error: None }
    }
}

/// Advance the preview by one output buffer: linear resampling from the file's rate to the
/// device's; stops at the end of the track. Pure, so it can be tested without a device.
pub fn render(st: &mut PvState, out: &mut [f32], channels: usize, rate: u32) {
    for s in out.iter_mut() {
        *s = 0.0;
    }
    if !st.playing || channels == 0 {
        return;
    }
    let Some(a) = st.audio.clone() else { return };
    let frames = a.frames();
    let step = a.sr as f64 / rate.max(1) as f64;
    let g = st.gain * st.volume;
    for fr in out.chunks_mut(channels) {
        let i = st.pos.floor() as usize;
        if i + 1 >= frames {
            st.playing = false;
            break;
        }
        let t = (st.pos - i as f64) as f32;
        let l = a.samples[2 * i] * (1.0 - t) + a.samples[2 * i + 2] * t;
        let r = a.samples[2 * i + 1] * (1.0 - t) + a.samples[2 * i + 3] * t;
        match fr.len() {
            1 => fr[0] = 0.5 * (l + r) * g,
            _ => {
                fr[0] = l * g;
                fr[1] = r * g;
            }
        }
        st.pos += step;
    }
}

impl Preview {
    fn ensure_stream(&mut self, device: Option<&str>) {
        if self.stream.is_some() && self.device.as_deref() == device {
            return;
        }
        self.stream = None;
        self.device = device.map(|s| s.to_string());
        match Self::open(self.state.clone(), device) {
            Ok(s) => {
                self.stream = Some(s);
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn open(state: Arc<Mutex<PvState>>, device: Option<&str>) -> anyhow::Result<cpal::Stream> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        // Listing devices can block on some systems: look for the named one on another thread
        // and fall back to the default output after 3 s.
        let named = device.and_then(|want| {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let want = want.to_string();
            std::thread::spawn(move || {
                let d = cpal::default_host().output_devices().ok().and_then(|mut it| it.find(|d| d.name().map(|n| n == want).unwrap_or(false)));
                let _ = tx.send(d);
            });
            rx.recv_timeout(std::time::Duration::from_secs(3)).ok().flatten()
        });
        let dev = match named {
            Some(d) => d,
            None => host.default_output_device().ok_or_else(|| anyhow::anyhow!("no audio output device"))?,
        };
        let cfg = dev.default_output_config()?;
        let rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let config: cpal::StreamConfig = cfg.clone().into();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => dev.build_output_stream(&config, move |data: &mut [f32], _| render(&mut state.lock(), data, channels, rate), |e| eprintln!("preview audio error: {e}"), None)?,
            _ => {
                let mut tmp: Vec<f32> = Vec::with_capacity(8192);
                dev.build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        tmp.resize(data.len(), 0.0);
                        render(&mut state.lock(), &mut tmp, channels, rate);
                        for (d, s) in data.iter_mut().zip(tmp.iter().copied()) {
                            *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                    },
                    |e| eprintln!("preview audio error: {e}"),
                    None,
                )?
            }
        };
        stream.play()?;
        Ok(stream)
    }

    /// Play `track` from `frac` (0–1) of its length. The file is decoded on its own thread the
    /// first time; a click in the track already previewing only moves the position ("Skip").
    pub fn play(&mut self, track: i64, path: &str, frac: f64, gain: f32, device: Option<&str>, ctx: &eframe::egui::Context) {
        self.ensure_stream(device);
        let mut st = self.state.lock();
        st.error = None;
        st.gain = gain;
        if st.track == Some(track) {
            if let Some(a) = &st.audio {
                st.pos = frac.clamp(0.0, 1.0) * a.frames() as f64;
                st.playing = true;
            } else {
                st.pending = Some(frac);
            }
            return;
        }
        st.generation += 1;
        let gen = st.generation;
        st.track = Some(track);
        st.audio = None;
        st.playing = false;
        st.pos = 0.0;
        st.pending = Some(frac);
        drop(st);
        let state = self.state.clone();
        let path = path.to_string();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = crate::audio::decode_file(&path);
            let mut st = state.lock();
            if st.generation != gen {
                return;
            }
            match r {
                Ok(a) => {
                    let f = st.pending.take().unwrap_or(0.0);
                    st.pos = f.clamp(0.0, 1.0) * a.frames() as f64;
                    st.audio = Some(Arc::new(a));
                    st.playing = true;
                }
                Err(e) => {
                    st.track = None;
                    st.error = Some(e.to_string());
                }
            }
            drop(st);
            ctx.request_repaint();
        });
    }

    pub fn stop(&mut self) {
        let mut st = self.state.lock();
        st.generation += 1;
        st.playing = false;
        st.track = None;
        st.audio = None;
        st.pending = None;
    }

    /// The previewing track and its position as a fraction of the track (None while decoding).
    pub fn status(&self) -> Option<(i64, Option<f64>)> {
        let mut st = self.state.lock();
        let t = st.track?;
        match &st.audio {
            Some(a) => {
                if !st.playing {
                    // Reached the end.
                    st.track = None;
                    st.audio = None;
                    return None;
                }
                Some((t, Some(st.pos / a.frames().max(1) as f64)))
            }
            None => Some((t, None)),
        }
    }

    pub fn set_volume(&self, v: f32) {
        self.state.lock().volume = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_resampled_and_stops_at_the_end() {
        // 1000 frames of a ramp at 22050 Hz played on a 44100 Hz stereo device: 2 output
        // frames per source frame, then silence and playing = false.
        let samples: Vec<f32> = (0..1000).flat_map(|i| [i as f32 / 1000.0, -(i as f32) / 1000.0]).collect();
        let a = Arc::new(Decoded { sr: 22050, samples, raw_frames: 1000, damaged: 0, truncated: false });
        let mut st = PvState { audio: Some(a), playing: true, volume: 0.5, gain: 2.0, pos: 100.0, ..Default::default() };
        let mut out = vec![9.0f32; 400];
        render(&mut st, &mut out, 2, 44100);
        assert!((st.pos - 200.0).abs() < 1e-6);
        assert!((out[0] - 0.1).abs() < 1e-5 && (out[1] + 0.1).abs() < 1e-5);
        // Half-way between source frames 100 and 101.
        assert!((out[2] - 0.1005).abs() < 1e-5);
        let mut out = vec![9.0f32; 4000];
        render(&mut st, &mut out, 2, 44100);
        assert!(!st.playing);
        assert_eq!(*out.last().unwrap(), 0.0);
        // Not playing: silence, position unchanged.
        let p = st.pos;
        let mut out = vec![9.0f32; 16];
        render(&mut st, &mut out, 2, 44100);
        assert!(out.iter().all(|s| *s == 0.0) && st.pos == p);
    }
}
