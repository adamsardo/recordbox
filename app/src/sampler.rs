//! The sampler deck's audio engine, after rekordbox 7.2.19 (checks/re-7.2.19/
//! pads-sampler-automix.md, section B): 16 voices, one per pad position, each holding up to
//! four clips (one per bank) and playing one of them at a time. A clip plays once or loops,
//! and in gate mode only sounds while its pad is held. Loop clips with SYNC follow the sampler
//! deck's BPM; with MT their pitch stays where it was.
//!
//! The sequencer (`sequencer.rs`) runs on the same clock. Each buffer it hands over the pad
//! events that fall inside it, and they start or stop their clip on their exact frame.
//!
//! The interface thread owns the settings and writes them here under the lock; the audio
//! thread takes the lock for the length of one buffer (`audio::render_cue`).

use crate::audio::{ClipReader, Decoded};
use crate::sequencer::Sequencer;
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

/// rekordbox's metronome gains for its volume settings 0, 1, 2 (table at 0x10375D144).
pub const METRONOME_GAINS: [f32; 3] = [0.17741, 0.26611, 0.35481];

/// A start or stop on a given frame of the buffer being made.
#[derive(Clone, Copy)]
struct Sched {
    frame: u32,
    bank: u8,
    on: bool,
    vel: f32,
}

const SCHED_MAX: usize = 8;

#[derive(Clone)]
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
    /// Starts and stops inside the buffer being made, in frame order.
    sched: [Sched; SCHED_MAX],
    nsched: usize,
    reader: ClipReader,
    peak: f32,
}

impl Voice {
    fn new() -> Voice {
        Voice { clips: [None, None, None, None], bank: 0, pos: 0.0, playing: false, level: 0.0, target: 0.0, velocity: 1.0, pending: None, restart: None, sched: [Sched { frame: 0, bank: 0, on: false, vel: 0.0 }; SCHED_MAX], nsched: 0, reader: ClipReader::new(), peak: 0.0 }
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

    /// Fade out (Shift + PLAY, a gate release, the sequencer's stop).
    fn fade_out(&mut self) {
        if self.playing {
            self.target = 0.0;
            self.restart = None;
        }
    }

    /// Keep a start or stop for frame `frame` of the buffer being made (more than eight in
    /// one buffer on one pad: the rest are dropped).
    fn schedule(&mut self, frame: usize, bank: usize, on: bool, vel: f32) {
        if self.nsched >= SCHED_MAX {
            return;
        }
        let mut i = self.nsched;
        while i > 0 && self.sched[i - 1].frame as usize > frame {
            self.sched[i] = self.sched[i - 1];
            i -= 1;
        }
        self.sched[i] = Sched { frame: frame as u32, bank: (bank % BANKS) as u8, on, vel };
        self.nsched += 1;
    }

    fn run_sched(&mut self, s: Sched) {
        let bank = s.bank as usize;
        if s.on {
            if self.clips[bank].is_some() {
                self.launch_now(bank, s.vel);
            }
        } else if self.playing && self.bank == bank && self.clips[bank].as_ref().is_some_and(|c| c.mode & MODE_GATE != 0) {
            // A note-off only ends gate clips.
            self.fade_out();
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
    pub seq: Sequencer,
    /// The metronome's sound, its level, and where its playback is (source frames).
    click: Arc<Decoded>,
    pub click_gain: f32,
    click_pos: Option<f64>,
    click_at: [u32; 4],
    nclick: usize,
    /// The last buffer had sound from the slots (`mix` holds it).
    pub sounding: bool,
    /// For the scripted test run: everything works, nothing is heard.
    pub silent: bool,
    /// BPM SYNC is on and the sampler is not the sync master: its beats are held on the
    /// master deck's (`FOLLOW_PHASE`).
    pub follow: bool,
    /// The sampler's own mix of the last buffer goes to `out` (false: the caller takes it
    /// from `take_mix`, to send it through effects first).
    pub direct: bool,
    /// 0 = master output, 1 = headphones output only.
    pub route: u8,
}

/// The metronome's sound when rekordbox's own click file is not there to read: 42 ms of a
/// 1 kHz tone that dies away quickly.
fn default_click() -> Decoded {
    let (sr, frames) = (44100u32, 1856usize);
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f32 / sr as f32;
        let v = (std::f32::consts::TAU * 1000.0 * t).sin() * (-t * 140.0).exp() * (1.0 - i as f32 / frames as f32);
        samples.push(v);
        samples.push(v);
    }
    Decoded { sr, samples, raw_frames: frames, damaged: 0, truncated: false }
}

pub static SAMPLER: Mutex<Option<Box<Engine>>> = Mutex::new(None);

/// The sampler's beat clock at the start of the next buffer (f64 bits). The decks' BEAT SYNC
/// reads it while the sampler is the sync master.
pub static CLOCK_BITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The master deck's place in its beat (0..1, f64 bits; NaN: it is not playing or has no
/// grid), written by the audio callback before the sampler runs. BPM SYNC holds the
/// sampler's clock on it.
pub static FOLLOW_PHASE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(f64::NAN.to_bits());

/// The sampler's beat phase minus the master's, wrapped to -0.5..0.5 beats.
pub fn phase_diff(own: f64, master: f64) -> f64 {
    (own - master + 0.5).rem_euclid(1.0) - 0.5
}

impl Engine {
    pub fn new() -> Engine {
        Engine { voices: (0..PADS).map(|_| Voice::new()).collect(), gain: 1.0, volume: 1.0, cue_gain: 0.0, bpm: 120.0, quantize: false, q_beats: 1.0, clock: 0.0, buf: vec![0.0; 8192], peak: [0.0; 2], seq: Sequencer::new(), click: Arc::new(default_click()), click_gain: METRONOME_GAINS[1], click_pos: None, click_at: [0; 4], nclick: 0, sounding: false, silent: false, follow: false, direct: true, route: 0 }
    }

    /// The metronome's sound (stereo).
    pub fn set_click(&mut self, click: Arc<Decoded>) {
        if click.frames() >= 2 {
            self.click = click;
            self.click_pos = None;
        }
    }

    /// Sequencer PLAY: the pattern starts from its beginning, and the sampler's beat clock
    /// with it, so that Q launches and the pattern share one grid.
    pub fn seq_play(&mut self) {
        if self.seq.playing {
            return;
        }
        // While the sampler follows the master deck, the pattern starts at the master's place
        // in its beat, so its beats fall on the master's from the first one.
        let m = f64::from_bits(FOLLOW_PHASE.load(std::sync::atomic::Ordering::Relaxed));
        let start = if self.follow && m.is_finite() { m.clamp(0.0, 0.999) } else { 0.0 };
        let shift = self.clock - start;
        for v in self.voices.iter_mut() {
            if let Some(p) = v.pending.as_mut() {
                p.2 -= shift;
            }
        }
        self.clock = start;
        self.seq.play_from(start);
    }

    /// The sampler's beat clock (beats since it was last set).
    pub fn clock(&self) -> f64 {
        self.clock
    }

    /// Every loaded slot's clip (pad, bank), for a copy of the engine.
    pub fn clip(&self, pad: usize, bank: usize) -> Option<&Clip> {
        self.voices[pad % PADS].clips[bank % BANKS].as_ref()
    }

    /// One pass of the current pattern as audio (interleaved stereo at `rate`), made away
    /// from the audio thread on a copy of the engine: what Sequence Load puts on a deck. The
    /// second of two passes is taken, so a sample that rings over the end of the pattern is
    /// there at its start, as it is when the pattern goes round. The slots' own gains count;
    /// the sampler's GAIN and fader do not.
    pub fn render_pattern(&self, rate: u32) -> Option<Vec<f32>> {
        let pat = self.seq.pattern(self.seq.current()).clone();
        if !pat.events.iter().any(|e| e.on) {
            return None;
        }
        let mut e = Engine::new();
        for pad in 0..PADS {
            for bank in 0..BANKS {
                if let Some(c) = self.clip(pad, bank) {
                    e.set_clip(pad, bank, Some(c.clone()));
                }
            }
        }
        e.bpm = self.bpm;
        let mut p = pat.clone();
        p.events.iter_mut().for_each(|ev| ev.fresh = false);
        e.seq.set_pattern(0, &p);
        e.seq.select(0, 0.0);
        e.seq_play();
        let frames = (pat.beats() * 60.0 / e.bpm.clamp(40.0, 499.0) as f64 * rate as f64).round() as usize;
        let mut out = vec![0.0f32; frames * 4];
        for chunk in out.chunks_mut(1024) {
            e.process(chunk, &mut [], rate);
        }
        Some(out.split_off(frames * 2))
    }

    /// The sampler's mix of the last buffer when it was not sent out directly: (master part,
    /// headphones part), both already at their levels.
    pub fn mix_mut(&mut self, n: usize) -> &mut [f32] {
        if self.buf.len() < n * 2 {
            self.buf.resize(n * 2, 0.0);
        }
        if !self.sounding {
            self.buf[..n * 2].iter_mut().for_each(|v| *v = 0.0);
        }
        &mut self.buf[..n * 2]
    }
    /// The level of the sampler's output for the meter, when the caller mixed it.
    pub fn note_peak(&mut self, l: f32, r: f32) {
        self.peak = [self.peak[0].max(l), self.peak[1].max(r)];
    }
    pub fn out_gains(&self) -> (f32, f32) {
        if self.silent { (0.0, 0.0) } else if self.route == 1 { (0.0, self.gain * self.volume) } else { (self.gain * self.volume, self.gain * self.cue_gain) }
    }

    /// Sequencer stop: back to the start, REC off, and every pad stops (rekordbox sends
    /// all-notes-off to the 16 tracks).
    pub fn seq_stop(&mut self) {
        self.seq.stop();
        for v in self.voices.iter_mut() {
            v.pending = None;
            v.fade_out();
        }
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
    /// With the sequencer's REC armed, the first press starts the pattern, and the press is
    /// recorded. Loop samples are not recorded: the answer is then `true` (rekordbox shows
    /// "Loop record error").
    pub fn launch(&mut self, pad: usize, bank: usize, velocity: f32) -> bool {
        let Some(mode) = self.voices[pad % PADS].clips[bank % BANKS].as_ref().map(|c| c.mode) else { return false };
        let mut loop_refused = false;
        if self.seq.rec {
            if !self.seq.playing {
                self.seq_play();
            }
            if mode & MODE_LOOP != 0 {
                loop_refused = true;
            } else {
                self.seq.note_on(pad % PADS, bank % BANKS, velocity, mode & MODE_GATE != 0);
            }
        }
        let (quantize, q, clock) = (self.quantize, self.q_beats.max(1.0 / 32.0), self.clock);
        let v = &mut self.voices[pad % PADS];
        if quantize {
            let at = (clock / q).ceil() * q;
            v.pending = Some((bank % BANKS, velocity, at));
        } else {
            v.launch_now(bank % BANKS, velocity);
        }
        loop_refused
    }

    /// PLAY released, or Shift + PLAY (`force`): only gate clips stop on a release; a launch
    /// still waiting for its quantize point is dropped with them.
    pub fn stop(&mut self, pad: usize, bank: usize, force: bool) {
        let v = &mut self.voices[pad % PADS];
        let gate = v.clips[bank % BANKS].as_ref().is_some_and(|c| c.mode & MODE_GATE != 0);
        if !(force || gate) {
            return;
        }
        if gate && !force {
            self.seq.note_off(pad % PADS, bank % BANKS);
        }
        let v = &mut self.voices[pad % PADS];
        if matches!(v.pending, Some((b, _, _)) if b == bank % BANKS) {
            v.pending = None;
        }
        if v.playing && v.bank == bank % BANKS {
            v.fade_out();
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
        let mut beats_per_frame = self.bpm.clamp(40.0, 499.0) as f64 / 60.0 / rate.max(1) as f64;
        if self.follow {
            // BPM SYNC: the clock runs a little faster or slower until its beats are on the
            // master deck's (the same correction the decks' BEAT SYNC uses).
            let m = f64::from_bits(FOLLOW_PHASE.load(std::sync::atomic::Ordering::Relaxed));
            if m.is_finite() {
                beats_per_frame *= 1.0 + crate::audio::sync_correction(phase_diff(self.clock.rem_euclid(1.0), m));
            }
        }
        let (c0, c1) = (self.clock, self.clock + n as f64 * beats_per_frame);
        self.clock = c1;
        CLOCK_BITS.store(c1.to_bits(), std::sync::atomic::Ordering::Relaxed);
        // The sequencer: its events of this buffer go to their pads, each with its frame.
        self.nclick = 0;
        {
            let Engine { seq, voices, click_at, nclick, .. } = self;
            seq.advance(
                n,
                beats_per_frame,
                |frame, e| voices[e.pad as usize % PADS].schedule(frame, e.bank as usize, e.on, e.vel),
                |frame| {
                    if *nclick < click_at.len() {
                        click_at[*nclick] = frame as u32;
                        *nclick += 1;
                    }
                },
            );
        }
        // A quantized launch that falls inside this buffer starts at its frame.
        for v in self.voices.iter_mut() {
            if let Some((bank, vel, at)) = v.pending {
                if at < c1 {
                    v.schedule((((at - c0) / beats_per_frame).max(0.0) as usize).min(n - 1), bank, true, vel);
                    v.pending = None;
                }
            }
        }
        self.sounding = self.voices.iter().any(|v| v.playing || v.nsched > 0);
        if self.sounding {
            self.render_voices(out, cue, rate, n);
        }
        self.render_click(out, rate, n);
    }

    fn render_voices(&mut self, out: &mut [f32], cue: &mut [f32], rate: u32, n: usize) {
        if self.buf.len() < n * 2 {
            // (Only when the device asks for a larger buffer than was foreseen.)
            self.buf.resize(n * 2, 0.0);
        }
        let (g, cue_g) = self.out_gains();
        let buf = &mut self.buf[..n * 2];
        buf.iter_mut().for_each(|v| *v = 0.0);
        let fade_k = 1.0 / (FADE_SECS * rate as f32);
        let hop = ((rate as usize) / 50).clamp(64, 4096);
        let bpm = self.bpm;
        for v in self.voices.iter_mut() {
            let (mut k, mut i) = (0usize, 0usize);
            while i < n {
                while k < v.nsched && v.sched[k].frame as usize <= i {
                    let s = v.sched[k];
                    k += 1;
                    v.run_sched(s);
                }
                if !v.playing {
                    // Nothing sounds until the next start, if there is one in this buffer.
                    if k < v.nsched {
                        i = (v.sched[k].frame as usize).max(i + 1).min(n);
                        if i >= n {
                            break;
                        }
                        continue;
                    }
                    break;
                }
                let Some(c) = v.clips[v.bank].as_ref() else {
                    v.playing = false;
                    continue;
                };
                let frames = c.audio.frames();
                if frames < 2 {
                    v.playing = false;
                    continue;
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
                i += 1;
            }
            // Starts and stops the loop did not reach (the pad was silent to the end).
            while k < v.nsched {
                let s = v.sched[k];
                k += 1;
                v.run_sched(s);
            }
            v.nsched = 0;
        }
        if self.silent || !self.direct {
            return;
        }
        let cue_g = if cue.len() >= out.len() { cue_g } else { 0.0 };
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

    /// The metronome: its click starts on the frames the sequencer named. It goes to the
    /// master output at its own level (not through the sampler's GAIN and volume).
    fn render_click(&mut self, out: &mut [f32], rate: u32, n: usize) {
        if self.nclick == 0 && self.click_pos.is_none() {
            return;
        }
        let frames = self.click.frames();
        let step = self.click.sr as f64 / rate.max(1) as f64;
        let g = if self.silent { 0.0 } else { self.click_gain };
        let mut k = 0;
        for i in 0..n {
            while k < self.nclick && self.click_at[k] as usize <= i {
                self.click_pos = Some(0.0);
                k += 1;
            }
            let Some(pos) = self.click_pos else {
                if k >= self.nclick {
                    break;
                }
                continue;
            };
            let j = pos as usize;
            if j + 1 >= frames {
                self.click_pos = None;
                continue;
            }
            let f = (pos - j as f64) as f32;
            let s = &self.click.samples;
            out[i * 2] += (s[j * 2] + (s[j * 2 + 2] - s[j * 2]) * f) * g;
            out[i * 2 + 1] += (s[j * 2 + 1] + (s[j * 2 + 3] - s[j * 2 + 1]) * f) * g;
            self.click_pos = Some(pos + step);
        }
        self.nclick = 0;
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

    fn ev(t: f64, pad: u8, on: bool) -> crate::sequencer::Event {
        crate::sequencer::Event { t, pad, bank: 0, on, vel: 1.0, fresh: false }
    }

    /// Run in buffers of `block` frames (as the audio callback does).
    fn run_blocks(e: &mut Engine, frames: usize, block: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
            e.process(&mut out[done * 2..(done + n) * 2], &mut [], 44100);
            done += n;
        }
        out
    }

    #[test]
    fn a_pattern_starts_its_pads_on_the_exact_frame_every_pass() {
        for block in [64usize, 512, 4096] {
            let mut e = Engine::new();
            e.bpm = 120.0;
            e.set_clip(0, 0, Some(clip(1000, 0)));
            e.set_clip(5, 0, Some(clip(1000, 0)));
            e.seq.set_pattern(0, &crate::sequencer::Pattern { bars: 1, events: vec![ev(0.0, 0, true), ev(2.5, 5, true)] });
            e.seq_play();
            let out = run_blocks(&mut e, 88200 * 3, block);
            let starts: Vec<usize> = (0..out.len() / 2).filter(|i| out[i * 2] != 0.0 && (*i == 0 || out[i * 2 - 2] == 0.0)).collect();
            // One bar at 120 BPM is 88200 frames; beat 2.5 is 55125 frames in.
            assert_eq!(starts, vec![0, 55125, 88200, 88200 + 55125, 176400, 176400 + 55125], "block {block}");
            assert_eq!(e.seq.passes, 2);
        }
    }

    #[test]
    fn pressing_a_pad_with_rec_armed_starts_the_pattern_and_records_the_press() {
        let mut e = Engine::new();
        e.bpm = 120.0;
        e.quantize = true;
        e.set_clip(0, 0, Some(clip(1000, 0)));
        e.set_clip(1, 0, Some(clip(1000, MODE_LOOP)));
        run(&mut e, 5000);
        e.seq.set_rec(true);
        assert!(!e.seq.playing);
        assert!(!e.launch(0, 0, 1.0));
        assert!(e.seq.playing && e.seq.rec);
        // The pattern and the clock start together, so the quantized press sounds at once.
        let out = run(&mut e, 22050 + 6000);
        assert!(out[0] != 0.0);
        // A second press a little after beat 1: recorded on beat 1, heard on beat 2.
        e.launch(0, 0, 1.0);
        // A loop sample is refused (and still plays).
        assert!(e.launch(1, 0, 1.0));
        let p = e.seq.pattern(0);
        assert_eq!(p.events.iter().map(|x| (x.t, x.pad, x.on)).collect::<Vec<_>>(), vec![(0.0, 0, true), (1.0, 0, true)]);
        let out = run(&mut e, 22050);
        let first = (0..22050).find(|i| out[i * 2] != 0.0).unwrap();
        assert!((first as i64 - (22050 - 6000)).abs() <= 2, "{first}");
        // Stop: REC off, the position back at the start, the loop pad silent after its fade.
        e.seq_stop();
        assert!(!e.seq.playing && !e.seq.rec && e.seq.pos() == 0.0);
        let out = run(&mut e, 2000);
        assert_eq!(out[2 * 1500], 0.0);
        assert!(!e.slot_state(1, 0).0);
        // Played back: beat 0 and beat 1.
        e.quantize = false;
        e.seq_play();
        let out = run(&mut e, 88200);
        let starts: Vec<usize> = (0..88200).filter(|i| out[i * 2] != 0.0 && (*i == 0 || out[i * 2 - 2] == 0.0)).collect();
        assert_eq!(starts, vec![0, 22050]);
    }

    #[test]
    fn a_recorded_gate_note_ends_where_the_pad_was_released() {
        let mut e = Engine::new();
        e.bpm = 120.0;
        e.seq.rec_unit = 0.0;
        e.set_clip(2, 0, Some(clip(100_000, MODE_GATE)));
        e.seq.set_rec(true);
        e.launch(2, 0, 1.0);
        run(&mut e, 10000);
        e.stop(2, 0, false);
        run(&mut e, 78200);
        e.seq.set_rec(false);
        // The next pass plays it without the pad: about 10000 frames of sound, then silence.
        let out = run(&mut e, 88200);
        assert!(out[2 * 5000] != 0.0 && out[2 * 9000] != 0.0);
        assert_eq!(out[2 * 12000], 0.0);
        assert_eq!(e.seq.pattern(0).events.len(), 2);
    }

    #[test]
    fn the_metronome_clicks_on_the_beats_at_its_own_level() {
        let mut e = Engine::new();
        e.bpm = 120.0;
        e.gain = 0.0;
        e.volume = 0.0;
        // A click that is easy to find: 100 frames of 1.0.
        e.set_click(Arc::new(Decoded { sr: 44100, samples: vec![1.0; 200], raw_frames: 100, damaged: 0, truncated: false }));
        e.click_gain = METRONOME_GAINS[2];
        e.seq.metronome = true;
        let out = run_blocks(&mut e, 44100, 512);
        assert!(out.iter().all(|v| *v == 0.0), "no clicks while the sequencer is stopped");
        e.seq_play();
        let out = run_blocks(&mut e, 88200 + 22050, 512);
        let starts: Vec<usize> = (0..out.len() / 2).filter(|i| out[i * 2] != 0.0 && (*i == 0 || out[i * 2 - 2] == 0.0)).collect();
        assert_eq!(starts, vec![0, 22050, 44100, 66150, 88200]);
        assert!((out[2 * 22060] - METRONOME_GAINS[2]).abs() < 1e-6 && (out[2 * 22060 + 1] - METRONOME_GAINS[2]).abs() < 1e-6);
        assert_eq!(out[2 * 22200], 0.0);
        e.seq.metronome = false;
        let out = run_blocks(&mut e, 44100, 512);
        assert!(out.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_muted_slot_is_silent_in_a_pattern_and_silent_mode_gives_no_output() {
        let mut e = Engine::new();
        e.set_clip(0, 0, Some(clip(1000, 0)));
        e.seq.set_pattern(0, &crate::sequencer::Pattern { bars: 1, events: vec![ev(0.0, 0, true)] });
        e.clip_mut(0, 0).unwrap().mute = true;
        e.seq_play();
        let out = run(&mut e, 2000);
        assert!(out.iter().all(|v| *v == 0.0));
        assert_eq!(e.seq.fired, 1);
        e.clip_mut(0, 0).unwrap().mute = false;
        e.silent = true;
        e.seq.metronome = true;
        let out = run(&mut e, 88200);
        assert!(out.iter().all(|v| *v == 0.0));
        assert_eq!(e.seq.fired, 2);
    }
}
