//! The sampler deck's audio engine, after rekordbox 7.2.19 (checks/re-7.2.19/
//! pads-sampler-automix.md, section B): 16 voices, one per pad position, each holding up to
//! four clips (one per bank) and playing one of them at a time. A clip plays once or loops,
//! and in gate mode only sounds while its pad is held. Loop clips with SYNC follow the sampler
//! deck's BPM; with MT their pitch stays where it was.
//!
//! The interface thread owns the settings and writes them here under the lock; the audio
//! thread takes the lock for the length of one buffer (`audio::render_cue`).

use crate::audio::{ClipReader, Decoded};
use parking_lot::Mutex;
use std::sync::Arc;

pub const PADS: usize = 16;
pub const BANKS: usize = 4;

/// `rbxfrm::ClipPlayMode`: bit 0 = loop, bit 1 = gate.
pub const MODE_LOOP: u8 = 1;
pub const MODE_GATE: u8 = 2;

/// Fade used when a clip is stopped or restarted (rekordbox fades out before it moves the
/// play position; its fade length was not traced).
const FADE_SECS: f32 = 0.003;

pub struct Clip {
    pub audio: Arc<Decoded>,
    pub mode: u8,
    /// 0..2 (the slot's GAIN: 50 = 1.0, 100 = 2.0).
    pub gain: f32,
    pub sync: bool,
    pub mt: bool,
    /// The sample's own BPM (0 when unknown: it then plays at its own speed).
    pub bpm: f32,
    /// Start position in ms (the EDIT panel's shift buttons).
    pub offset_ms: f32,
    pub mute: bool,
}

struct Voice {
    clips: [Option<Clip>; BANKS],
    bank: usize,
    pos: f64,
    playing: bool,
    /// Fade envelope and where it is heading.
    level: f32,
    target: f32,
    velocity: f32,
    /// A launch waiting for its quantize point (bank, velocity, beat of the sampler clock).
    pending: Option<(usize, f32, f64)>,
    /// A launch waiting for the fade-out of what is playing.
    restart: Option<(usize, f32)>,
    reader: ClipReader,
    peak: f32,
}

impl Voice {
    fn new() -> Voice {
        Voice { clips: [None, None, None, None], bank: 0, pos: 0.0, playing: false, level: 0.0, target: 0.0, velocity: 1.0, pending: None, restart: None, reader: ClipReader::new(), peak: 0.0 }
    }

    fn start(&mut self, bank: usize, velocity: f32) {
        let Some(c) = self.clips[bank].as_ref() else { return };
        self.bank = bank;
        self.pos = (c.audio.sr as f64 / 1000.0 * c.offset_ms.max(0.0) as f64).floor().min(c.audio.frames().saturating_sub(1) as f64);
        self.playing = true;
        self.level = 1.0;
        self.target = 1.0;
        self.velocity = velocity.clamp(0.0, 1.0);
        self.restart = None;
        self.reader.reset();
    }

    /// Start now; what is playing on this voice fades out first.
    fn launch_now(&mut self, bank: usize, velocity: f32) {
        if self.playing {
            self.restart = Some((bank, velocity));
            self.target = 0.0;
        } else {
            self.start(bank, velocity);
        }
    }
}

pub struct Engine {
    voices: Vec<Voice>,
    /// Sampler GAIN and volume fader, as gains.
    pub gain: f32,
    pub volume: f32,
    /// Level of the sampler in the headphones (0 = CUE off).
    pub cue_gain: f32,
    pub bpm: f32,
    /// Q: launches wait for the next multiple of `q_beats` on the sampler's clock.
    pub quantize: bool,
    pub q_beats: f64,
    clock: f64,
    buf: Vec<f32>,
    peak: [f32; 2],
}

pub static SAMPLER: Mutex<Option<Box<Engine>>> = Mutex::new(None);

impl Engine {
    pub fn new() -> Engine {
        Engine { voices: (0..PADS).map(|_| Voice::new()).collect(), gain: 1.0, volume: 1.0, cue_gain: 0.0, bpm: 120.0, quantize: false, q_beats: 1.0, clock: 0.0, buf: vec![0.0; 8192], peak: [0.0; 2] }
    }

    /// Put a clip in a slot (or take it out); whatever the slot was playing stops at once.
    pub fn set_clip(&mut self, pad: usize, bank: usize, clip: Option<Clip>) {
        let v = &mut self.voices[pad % PADS];
        if v.bank == bank % BANKS {
            v.playing = false;
            v.restart = None;
        }
        if matches!(v.pending, Some((b, _, _)) if b == bank % BANKS) {
            v.pending = None;
        }
        v.clips[bank % BANKS] = clip;
    }

    pub fn clip_mut(&mut self, pad: usize, bank: usize) -> Option<&mut Clip> {
        self.voices[pad % PADS].clips[bank % BANKS].as_mut()
    }

    /// PLAY pressed (a note-on in rekordbox): start the clip, from its start if it is playing.
    pub fn launch(&mut self, pad: usize, bank: usize, velocity: f32) {
        let (quantize, q, clock) = (self.quantize, self.q_beats.max(1.0 / 32.0), self.clock);
        let v = &mut self.voices[pad % PADS];
        if v.clips[bank % BANKS].is_none() {
            return;
        }
        if quantize {
            let at = (clock / q).ceil() * q;
            v.pending = Some((bank % BANKS, velocity, at));
        } else {
            v.launch_now(bank % BANKS, velocity);
        }
    }

    /// PLAY released, or Shift + PLAY (`force`): only gate clips stop on a release; a launch
    /// still waiting for its quantize point is dropped with them.
    pub fn stop(&mut self, pad: usize, bank: usize, force: bool) {
        let v = &mut self.voices[pad % PADS];
        let gate = v.clips[bank % BANKS].as_ref().is_some_and(|c| c.mode & MODE_GATE != 0);
        if !(force || gate) {
            return;
        }
        if matches!(v.pending, Some((b, _, _)) if b == bank % BANKS) {
            v.pending = None;
        }
        if v.playing && v.bank == bank % BANKS {
            v.target = 0.0;
            v.restart = None;
        }
    }

    /// For the panel: is this slot sounding, and how far through its clip (0..1).
    pub fn slot_state(&self, pad: usize, bank: usize) -> (bool, f32) {
        let v = &self.voices[pad % PADS];
        if !(v.playing && v.bank == bank % BANKS) {
            return (false, 0.0);
        }
        let frames = v.clips[v.bank].as_ref().map(|c| c.audio.frames()).unwrap_or(1).max(1);
        (true, (v.pos / frames as f64) as f32)
    }

    /// Peak of a pad since the last call (for its level meter), and of the sampler's output.
    pub fn take_pad_peak(&mut self, pad: usize) -> f32 {
        std::mem::take(&mut self.voices[pad % PADS].peak)
    }
    pub fn take_peak(&mut self) -> [f32; 2] {
        std::mem::take(&mut self.peak)
    }

    /// Add the sampler to `out` (interleaved stereo) and, with CUE on, to the headphones.
    pub fn process(&mut self, out: &mut [f32], cue: &mut [f32], rate: u32) {
        let n = out.len() / 2;
        if n == 0 {
            return;
        }
        let beats_per_frame = self.bpm.clamp(40.0, 499.0) as f64 / 60.0 / rate.max(1) as f64;
        let (c0, c1) = (self.clock, self.clock + n as f64 * beats_per_frame);
        self.clock = c1;
        if !self.voices.iter().any(|v| v.playing || v.pending.is_some()) {
            return;
        }
        if self.buf.len() < n * 2 {
            // (Only when the device asks for a larger buffer than was foreseen.)
            self.buf.resize(n * 2, 0.0);
        }
        let buf = &mut self.buf[..n * 2];
        buf.iter_mut().for_each(|v| *v = 0.0);
        let fade_k = 1.0 / (FADE_SECS * rate as f32);
        let hop = ((rate as usize) / 50).clamp(64, 4096);
        let bpm = self.bpm;
        for v in self.voices.iter_mut() {
            // A quantized launch that falls inside this buffer starts at its frame.
            let mut start_at = None;
            if let Some((bank, vel, at)) = v.pending {
                if at < c1 {
                    start_at = Some((((at - c0) / beats_per_frame).max(0.0) as usize).min(n - 1));
                    v.pending = None;
                    v.restart = Some((bank, vel));
                }
            }
            for i in 0..n {
                if start_at == Some(i) {
                    if let Some((bank, vel)) = v.restart.take() {
                        if v.playing {
                            v.restart = Some((bank, vel));
                            v.target = 0.0;
                        } else {
                            v.start(bank, vel);
                        }
                    }
                }
                if !v.playing {
                    if start_at.is_some_and(|s| s > i) {
                        continue;
                    }
                    break;
                }
                let Some(c) = v.clips[v.bank].as_ref() else {
                    v.playing = false;
                    break;
                };
                let frames = c.audio.frames();
                if frames < 2 {
                    v.playing = false;
                    break;
                }
                let ratio = c.audio.sr as f64 / rate.max(1) as f64;
                let looped = c.mode & MODE_LOOP != 0;
                let speed = if looped && c.sync && c.bpm > 0.0 { (bpm / c.bpm).clamp(0.25, 4.0) as f64 } else { 1.0 };
                let step = ratio * speed;
                let pitch_step = if c.mt { ratio } else { step };
                let (l, r) = v.reader.frame(&c.audio, v.pos, step, pitch_step, hop);
                let g = if c.mute { 0.0 } else { c.gain * v.velocity * v.level };
                let (l, r) = (l * g, r * g);
                buf[i * 2] += l;
                buf[i * 2 + 1] += r;
                v.peak = v.peak.max(l.abs()).max(r.abs());
                v.pos += step;
                if v.pos >= (frames - 1) as f64 {
                    if looped {
                        v.pos -= (frames - 1) as f64;
                    } else {
                        // One-shot end: back to the start and stop.
                        v.playing = false;
                        v.pos = 0.0;
                    }
                }
                if v.level != v.target {
                    v.level += (v.target - v.level).clamp(-fade_k, fade_k);
                    if v.level <= 0.0 && v.target <= 0.0 {
                        v.playing = false;
                        v.pos = 0.0;
                        if let Some((bank, vel)) = v.restart.take() {
                            v.start(bank, vel);
                        }
                    }
                }
            }
        }
        let g = self.gain * self.volume;
        let cue_g = if cue.len() >= out.len() { self.gain * self.cue_gain } else { 0.0 };
        let mut peak = self.peak;
        for i in 0..n {
            let (l, r) = (buf[i * 2], buf[i * 2 + 1]);
            out[i * 2] += l * g;
            out[i * 2 + 1] += r * g;
            peak[0] = peak[0].max((l * g).abs());
            peak[1] = peak[1].max((r * g).abs());
            if cue_g > 0.0 {
                cue[i * 2] += l * cue_g;
                cue[i * 2 + 1] += r * cue_g;
            }
        }
        self.peak = peak;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(frames: usize, mode: u8) -> Clip {
        let samples = (0..frames * 2).map(|_| 0.5f32).collect();
        Clip { audio: Arc::new(Decoded { sr: 44100, samples, raw_frames: frames, damaged: 0, truncated: false }), mode, gain: 1.0, sync: true, mt: true, bpm: 0.0, offset_ms: 0.0, mute: false }
    }

    fn run(e: &mut Engine, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        e.process(&mut out, &mut [], 44100);
        out
    }

    #[test]
    fn a_one_shot_plays_once_and_stops() {
        let mut e = Engine::new();
        e.set_clip(0, 0, Some(clip(1000, 0)));
        e.launch(0, 0, 1.0);
        let out = run(&mut e, 2000);
        assert!((out[2 * 500] - 0.5).abs() < 1e-3, "{}", out[2 * 500]);
        assert_eq!(out[2 * 1500], 0.0);
        assert!(!e.slot_state(0, 0).0);
        // A release does not stop a one-shot (only gate modes stop on mouse-up).
        e.launch(0, 0, 1.0);
        e.stop(0, 0, false);
        let out = run(&mut e, 600);
        assert!((out[2 * 500] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_loop_keeps_going_and_a_gate_stops_on_release() {
        let mut e = Engine::new();
        e.set_clip(1, 0, Some(clip(1000, MODE_LOOP)));
        e.set_clip(2, 0, Some(clip(100_000, MODE_GATE)));
        e.launch(1, 0, 1.0);
        e.launch(2, 0, 1.0);
        let out = run(&mut e, 5000);
        assert!((out[2 * 4000] - 1.0).abs() < 1e-3, "loop and gate together: {}", out[2 * 4000]);
        e.stop(1, 0, false);
        e.stop(2, 0, false);
        let out = run(&mut e, 5000);
        assert!((out[2 * 4000] - 0.5).abs() < 1e-3, "only the loop is left: {}", out[2 * 4000]);
        assert!(e.slot_state(1, 0).0 && !e.slot_state(2, 0).0);
        // Shift + PLAY stops the loop too.
        e.stop(1, 0, true);
        let out = run(&mut e, 5000);
        assert_eq!(out[2 * 4000], 0.0);
    }

    #[test]
    fn one_pad_position_plays_one_bank_at_a_time() {
        let mut e = Engine::new();
        e.set_clip(0, 0, Some(clip(100_000, MODE_LOOP)));
        e.set_clip(0, 1, Some(clip(100_000, MODE_LOOP)));
        e.launch(0, 0, 1.0);
        run(&mut e, 1000);
        e.launch(0, 1, 1.0);
        let out = run(&mut e, 4000);
        assert!((out[2 * 3000] - 0.5).abs() < 1e-3, "{}", out[2 * 3000]);
        assert!(!e.slot_state(0, 0).0 && e.slot_state(0, 1).0);
    }

    #[test]
    fn a_quantized_launch_waits_for_the_next_beat() {
        let mut e = Engine::new();
        e.quantize = true;
        e.bpm = 120.0;
        e.set_clip(0, 0, Some(clip(100_000, 0)));
        // Move the clock a quarter of a beat on (a beat is 22050 frames at 120 BPM).
        run(&mut e, 5512);
        e.launch(0, 0, 1.0);
        let out = run(&mut e, 22050);
        let first = (0..22050).find(|i| out[i * 2] != 0.0).unwrap();
        assert!((first as i64 - (22050 - 5512)).abs() <= 2, "{first}");
    }

    #[test]
    fn a_synced_loop_follows_the_sampler_bpm() {
        let mut e = Engine::new();
        let mut c = clip(44100, MODE_LOOP);
        c.bpm = 120.0;
        c.mt = false;
        e.set_clip(0, 0, Some(c));
        e.bpm = 60.0;
        e.launch(0, 0, 1.0);
        run(&mut e, 22050);
        // Half speed: after half a second it is a quarter of the way through.
        assert!((e.slot_state(0, 0).1 - 0.25).abs() < 0.01, "{}", e.slot_state(0, 0).1);
    }

    #[test]
    fn gain_volume_and_cue() {
        let mut e = Engine::new();
        e.set_clip(0, 0, Some(clip(100_000, 0)));
        e.gain = 0.5;
        e.volume = 0.5;
        e.cue_gain = 1.0;
        e.launch(0, 0, 0.5);
        let mut out = vec![0.0; 2000];
        let mut cue = vec![0.0; 2000];
        e.process(&mut out, &mut cue, 44100);
        assert!((out[1000] - 0.5 * 0.5 * 0.25).abs() < 1e-4, "{}", out[1000]);
        assert!((cue[1000] - 0.5 * 0.5 * 0.5).abs() < 1e-4, "{}", cue[1000]);
    }
}
