//! The sampler deck's sequencer, after rekordbox 7.2.19's `rbxfrm::MidiSequencerUnit`
//! (checks/re-7.2.19/pads-sampler-automix.md, section B6): eight patterns of 1, 2 or 4 bars
//! holding pad note-ons and note-offs timed in quarter-note beats. A pattern plays in a loop
//! on the sampler's BPM; REC adds what is played on the pads to it (overdub), note-ons moved
//! to the record quantize grid.
//!
//! The audio thread calls `advance` once per buffer, under the sampler's lock. It gives back
//! the events of that buffer with their frame, and allocates nothing: every pattern's event
//! list has its full capacity from the start.

use serde::{Deserialize, Serialize};

pub const PATTERNS: usize = 8;
pub const PADS: usize = 16;
/// Most events one pattern holds; what is played after that is not recorded.
pub const MAX_EVENTS: usize = 2048;

/// A note-on or note-off of one pad (rekordbox: MIDI channel = pad, note number = bank).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// Beats from the start of the pattern.
    pub t: f64,
    pub pad: u8,
    pub bank: u8,
    pub on: bool,
    pub vel: f32,
    /// Recorded ahead of the play position on this pass: the pad has already sounded it, so
    /// the first time the play position reaches it, it is skipped.
    #[serde(skip)]
    pub fresh: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pattern {
    /// 1, 2 or 4.
    pub bars: u8,
    pub events: Vec<Event>,
}

impl Pattern {
    /// An empty pattern is one bar long, as in rekordbox.
    pub fn new() -> Pattern {
        Pattern { bars: 1, events: Vec::with_capacity(MAX_EVENTS) }
    }
    pub fn beats(&self) -> f64 {
        self.bars as f64 * 4.0
    }
}

/// `rbxfrm::UnitOfTime::quantizePpq`: `t` moved to the grid of `unit` beats counted from
/// `origin`; from `threshold` into a step it goes up, before that down.
pub fn quantize_ppq(unit: f64, t: f64, threshold: f64, origin: f64) -> f64 {
    if unit <= 0.0 {
        return t;
    }
    let r = (t - origin).rem_euclid(unit);
    if r >= threshold { t + (unit - r) } else { t - r }
}

/// The record quantize grid in beats for rekordbox's `SequencerQuantize` setting (1..6 =
/// 1/32, 1/16, 1/8, 1/4, 1/2, 1/1 note).
pub fn quantize_unit(setting: u8) -> f64 {
    [0.125, 0.25, 0.5, 1.0, 2.0, 4.0][(setting.clamp(1, 6) - 1) as usize]
}

#[derive(Clone, Copy)]
struct Held {
    t: f64,
    bank: u8,
}

pub struct Sequencer {
    patterns: Vec<Pattern>,
    dirty: [bool; PATTERNS],
    cur: usize,
    pub playing: bool,
    /// REC is armed; notes are recorded while it is armed and the pattern plays.
    pub rec: bool,
    pub metronome: bool,
    /// Record quantize grid in beats (0 = off).
    pub rec_unit: f64,
    // The play position is `anchor + since * bpf` beats: `since` frames have passed since
    // the position was `anchor`. Counting whole frames keeps every pass the same length.
    anchor: f64,
    since: u64,
    bpf: f64,
    /// The first event the play position has not reached on this pass, and the next beat.
    next: usize,
    next_beat: u32,
    /// Gate pads held down while recording: their note-on, waiting for the note-off.
    held: [Option<Held>; PADS],
    /// A pattern change waiting for its beat: (pattern, beat of the current pattern).
    reserve: Option<(usize, f64)>,
    /// Times the play position has gone back to the start, and events given out, since PLAY.
    pub passes: u32,
    pub fired: u32,
}

impl Sequencer {
    pub fn new() -> Sequencer {
        Sequencer {
            patterns: (0..PATTERNS).map(|_| Pattern::new()).collect(),
            dirty: [false; PATTERNS],
            cur: 0,
            playing: false,
            rec: false,
            metronome: false,
            rec_unit: 1.0,
            anchor: 0.0,
            since: 0,
            bpf: 120.0 / 60.0 / 44100.0,
            next: 0,
            next_beat: 0,
            held: [None; PADS],
            reserve: None,
            passes: 0,
            fired: 0,
        }
    }

    pub fn current(&self) -> usize {
        self.cur
    }
    pub fn pattern(&self, slot: usize) -> &Pattern {
        &self.patterns[slot % PATTERNS]
    }
    /// The pattern has been changed since it was saved or loaded.
    pub fn dirty(&self, slot: usize) -> bool {
        self.dirty[slot % PATTERNS]
    }
    pub fn mark_saved(&mut self, slot: usize) {
        self.dirty[slot % PATTERNS] = false;
    }
    /// The pattern a change is waiting to switch to.
    pub fn reserved(&self) -> Option<usize> {
        self.reserve.map(|r| r.0)
    }
    /// Length of the current pattern in beats.
    pub fn beats(&self) -> f64 {
        self.patterns[self.cur].beats()
    }
    /// Play position in beats.
    pub fn pos(&self) -> f64 {
        let len = self.beats();
        (self.anchor + self.since as f64 * self.bpf).clamp(0.0, len - 1e-9)
    }

    /// The frame, counted like `since`, on which the play position reaches beat `t`.
    #[inline]
    fn frame_of(&self, t: f64) -> u64 {
        let x = (t - self.anchor) / self.bpf;
        if x <= 0.0 { 0 } else { (x - 1e-7).ceil().max(0.0) as u64 }
    }

    /// Has the play position still to reach beat `t` on this pass? (Right after the position
    /// was set anew, only what lies less than a frame behind it is still to come.)
    #[inline]
    fn unplayed(&self, t: f64) -> bool {
        self.frame_of(t) >= self.since && t > self.anchor - self.bpf
    }

    /// After the events or the position changed: find again what has not been played yet.
    fn resync(&mut self) {
        let evs = &self.patterns[self.cur].events;
        let mut i = 0;
        while i < evs.len() && !self.unplayed(evs[i].t) {
            i += 1;
        }
        self.next = i;
        let mut k = (self.anchor + self.since as f64 * self.bpf).floor().max(0.0) as u32;
        while !self.unplayed(k as f64) {
            k += 1;
        }
        self.next_beat = k;
    }

    fn rewind(&mut self) {
        self.anchor = 0.0;
        self.since = 0;
        self.next = 0;
        self.next_beat = 0;
        self.held = [None; PADS];
        self.reserve = None;
        for e in self.patterns[self.cur].events.iter_mut() {
            e.fresh = false;
        }
    }

    /// PLAY: from the start of the pattern.
    pub fn play(&mut self) {
        if self.playing {
            return;
        }
        self.rewind();
        self.playing = true;
        self.passes = 0;
        self.fired = 0;
    }

    /// Stop: back to the start; REC goes off (rekordbox's `doStop` clears the rec mode).
    pub fn stop(&mut self) {
        self.playing = false;
        self.rec = false;
        self.rewind();
    }

    pub fn set_rec(&mut self, on: bool) {
        self.rec = on;
        if !on {
            self.held = [None; PADS];
        }
    }

    fn recording(&self) -> bool {
        self.rec && self.playing
    }

    /// Put an event in its place by time; an exact duplicate is not added. Returns the index
    /// of the event.
    fn insert(&mut self, ev: Event) -> Option<usize> {
        let evs = &mut self.patterns[self.cur].events;
        if let Some(i) = evs.iter().position(|e| e.t == ev.t && e.pad == ev.pad && e.bank == ev.bank && e.on == ev.on) {
            return Some(i);
        }
        if evs.len() >= MAX_EVENTS.min(evs.capacity()) {
            return None;
        }
        let at = evs.iter().position(|e| e.t > ev.t).unwrap_or(evs.len());
        evs.insert(at, ev);
        self.dirty[self.cur] = true;
        Some(at)
    }

    /// A pad was played (not a loop sample). While recording, its note-on goes into the
    /// pattern, moved to the record quantize grid. `gate`: the pad's sample sounds only while
    /// the pad is held, so a note-off will follow.
    pub fn note_on(&mut self, pad: usize, bank: usize, vel: f32, gate: bool) -> bool {
        if !self.recording() {
            return false;
        }
        let (len, pos) = (self.beats(), self.pos());
        let mut t = quantize_ppq(self.rec_unit, pos, self.rec_unit / 2.0, 0.0);
        // At or past the end: it belongs to the start of the next pass.
        let wrapped = t >= len;
        if wrapped {
            t -= len;
        }
        // Still ahead of the play position on this pass? Then the pad has sounded it already.
        let fresh = wrapped || self.unplayed(t);
        let ev = Event { t, pad: (pad % PADS) as u8, bank: bank as u8, on: true, vel: vel.clamp(0.0, 1.0), fresh };
        let Some(i) = self.insert(ev) else { return false };
        self.patterns[self.cur].events[i].fresh = fresh;
        self.held[pad % PADS] = if gate { Some(Held { t, bank: bank as u8 }) } else { None };
        self.resync();
        true
    }

    /// A gate pad was released: its note-off is recorded where the play position is (note-offs
    /// are not quantized). A note-off that would land on or before its own note-on goes one
    /// quantize step after it.
    pub fn note_off(&mut self, pad: usize, bank: usize) {
        if !self.recording() {
            return;
        }
        let Some(h) = self.held[pad % PADS].take() else { return };
        if h.bank as usize != bank {
            return;
        }
        let (len, pos) = (self.beats(), self.pos());
        let on_ahead = self.patterns[self.cur].events.iter().any(|e| e.on && e.fresh && e.pad as usize == pad % PADS && e.bank == h.bank && e.t == h.t);
        let step = if self.rec_unit > 0.0 { self.rec_unit } else { 0.125 };
        let t = if on_ahead || pos == h.t { h.t + step } else { pos };
        let ev = Event { t: t.rem_euclid(len), pad: (pad % PADS) as u8, bank: h.bank, on: false, vel: 0.0, fresh: false };
        self.insert(ev);
        self.resync();
    }

    /// ERASE + pad: take every note of that pad out of the current pattern.
    pub fn erase_track(&mut self, pad: usize) {
        let evs = &mut self.patterns[self.cur].events;
        let before = evs.len();
        evs.retain(|e| e.pad as usize != pad % PADS);
        if evs.len() != before {
            self.dirty[self.cur] = true;
        }
        self.held[pad % PADS] = None;
        self.resync();
    }

    /// DELETE on a pattern: empty, one bar.
    pub fn clear(&mut self, slot: usize) {
        let slot = slot % PATTERNS;
        self.patterns[slot].events.clear();
        self.patterns[slot].bars = 1;
        self.dirty[slot] = false;
        if slot == self.cur {
            self.set_position_after_change();
        }
    }

    /// Length of the current pattern: 1, 2 or 4 bars. Notes past a shorter end stay where they
    /// are and are not played.
    pub fn set_bars(&mut self, bars: u8) {
        let bars = if bars >= 4 { 4 } else if bars >= 2 { 2 } else { 1 };
        if self.patterns[self.cur].bars == bars {
            return;
        }
        self.patterns[self.cur].bars = bars;
        self.dirty[self.cur] = true;
        self.set_position_after_change();
    }

    fn set_position_after_change(&mut self) {
        let len = self.beats();
        self.anchor = (self.anchor + self.since as f64 * self.bpf).rem_euclid(len);
        self.since = 0;
        self.held = [None; PADS];
        if let Some((_, at)) = self.reserve.as_mut() {
            *at = at.min(len);
        }
        self.resync();
    }

    /// Choose a pattern. REC goes off. Stopped: at once. Playing: on the next multiple of
    /// `unit` beats of the current pattern, its end at the latest.
    pub fn select(&mut self, slot: usize, unit: f64) {
        let slot = slot % PATTERNS;
        self.set_rec(false);
        if slot == self.cur {
            self.reserve = None;
            return;
        }
        if self.playing {
            let (len, pos) = (self.beats(), self.pos());
            let at = if unit > 0.0 { ((pos / unit).floor() + 1.0) * unit } else { pos };
            self.reserve = Some((slot, at.min(len)));
        } else {
            self.cur = slot;
            self.rewind();
        }
    }

    /// Bring a saved pattern in (at launch, or when a pattern is loaded again).
    pub fn set_pattern(&mut self, slot: usize, p: &Pattern) {
        let slot = slot % PATTERNS;
        let dst = &mut self.patterns[slot];
        dst.bars = if p.bars >= 4 { 4 } else if p.bars >= 2 { 2 } else { 1 };
        dst.events.clear();
        let room = MAX_EVENTS.min(dst.events.capacity());
        dst.events.extend(p.events.iter().filter(|e| e.t.is_finite() && e.t >= 0.0).take(room).map(|e| Event { fresh: false, pad: e.pad % PADS as u8, ..*e }));
        dst.events.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.dirty[slot] = false;
        if slot == self.cur {
            self.set_position_after_change();
        }
    }

    /// Move the play position on by `n` frames at `bpf` beats per frame. `emit` gets every
    /// event the position reaches, with its frame in this buffer; `click` the frame of every
    /// beat while the metronome is on.
    pub fn advance(&mut self, n: usize, bpf: f64, mut emit: impl FnMut(usize, &Event), mut click: impl FnMut(usize)) {
        if !(bpf > 0.0) {
            return;
        }
        if bpf != self.bpf {
            // A new tempo: carry on from where the position is.
            self.anchor += self.since as f64 * self.bpf;
            self.since = 0;
            self.bpf = bpf;
        }
        if !self.playing {
            return;
        }
        let mut base = 0usize;
        loop {
            let len = self.beats();
            let boundary = self.reserve.map(|r| r.1.min(len)).unwrap_or(len);
            let end_frame = self.since + (n - base) as u64;
            let wrap_frame = self.frame_of(boundary);
            while self.next < self.patterns[self.cur].events.len() {
                let e = self.patterns[self.cur].events[self.next];
                if e.t >= boundary {
                    break;
                }
                let f = self.frame_of(e.t);
                if f >= end_frame {
                    break;
                }
                self.next += 1;
                if e.fresh {
                    self.patterns[self.cur].events[self.next - 1].fresh = false;
                    continue;
                }
                self.fired = self.fired.wrapping_add(1);
                emit(base + f.saturating_sub(self.since) as usize, &e);
            }
            while (self.next_beat as f64) < boundary {
                let f = self.frame_of(self.next_beat as f64);
                if f >= end_frame {
                    break;
                }
                self.next_beat += 1;
                if self.metronome {
                    click(base + f.saturating_sub(self.since) as usize);
                }
            }
            if wrap_frame >= end_frame {
                self.since = end_frame;
                break;
            }
            // The end of the pattern (or the beat of a pattern change) is inside this buffer:
            // carry on from the start, keeping the fraction of a frame that went past the end.
            base += wrap_frame.saturating_sub(self.since) as usize;
            self.anchor = (self.anchor + wrap_frame as f64 * self.bpf - boundary).max(0.0);
            self.since = 0;
            self.next = 0;
            self.next_beat = 0;
            self.passes = self.passes.wrapping_add(1);
            if let Some((slot, _)) = self.reserve.take() {
                self.cur = slot;
                self.held = [None; PADS];
                for e in self.patterns[self.cur].events.iter_mut() {
                    e.fresh = false;
                }
            } else {
                // A gate pad still held after a whole pass: its note ends just before it
                // starts again (rekordbox adds the note-off at on - 0.01).
                for pad in 0..PADS {
                    if let Some(h) = self.held[pad].take() {
                        let t = (h.t - 0.01).rem_euclid(len);
                        self.insert(Event { t, pad: pad as u8, bank: h.bank, on: false, vel: 0.0, fresh: false });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BPF: f64 = 120.0 / 60.0 / 44100.0;

    /// Run `frames` frames in buffers of `block`; returns (absolute frame, event) and the
    /// absolute frames of the metronome.
    fn run(s: &mut Sequencer, start: usize, frames: usize, block: usize, bpf: f64) -> (Vec<(usize, Event)>, Vec<usize>) {
        let (mut evs, mut clicks) = (Vec::new(), Vec::new());
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
            s.advance(n, bpf, |f, e| evs.push((start + done + f, *e)), |f| clicks.push(start + done + f));
            done += n;
        }
        (evs, clicks)
    }

    fn recording() -> Sequencer {
        let mut s = Sequencer::new();
        s.set_rec(true);
        s.play();
        s
    }

    #[test]
    fn quantize_ppq_rounds_at_the_threshold() {
        assert_eq!(quantize_ppq(1.0, 1.49, 0.5, 0.0), 1.0);
        assert_eq!(quantize_ppq(1.0, 1.5, 0.5, 0.0), 2.0);
        assert_eq!(quantize_ppq(0.25, 3.9, 0.125, 0.0), 4.0);
        assert_eq!(quantize_ppq(0.0, 1.37, 0.0, 0.0), 1.37);
        assert_eq!(quantize_unit(4), 1.0);
        assert_eq!(quantize_unit(1), 0.125);
        assert_eq!(quantize_unit(6), 4.0);
    }

    #[test]
    fn recorded_note_ons_land_on_the_quantize_grid() {
        for (unit, at_frames, want) in [(1.0, 22050 + 5000, 1.0), (1.0, 22050 + 12000, 2.0), (0.25, 30000, 1.25), (0.5, 80000, 3.5)] {
            let mut s = recording();
            s.rec_unit = unit;
            run(&mut s, 0, at_frames, 512, BPF);
            assert!(s.note_on(3, 1, 0.7, false));
            let p = s.pattern(0);
            assert_eq!(p.events.len(), 1);
            assert_eq!(p.events[0].t, want, "unit {unit}");
            assert_eq!((p.events[0].pad, p.events[0].bank, p.events[0].on, p.events[0].vel), (3, 1, true, 0.7));
            assert!(s.dirty(0));
        }
        // Without quantize the note stays where it was played.
        let mut s = recording();
        s.rec_unit = 0.0;
        run(&mut s, 0, 30000, 512, BPF);
        s.note_on(0, 0, 1.0, false);
        assert!((s.pattern(0).events[0].t - 30000.0 * BPF).abs() < 1e-9);
    }

    #[test]
    fn a_note_played_near_the_end_goes_to_the_start_of_the_pattern() {
        let mut s = recording();
        run(&mut s, 0, 88200 - 4000, 512, BPF);
        s.note_on(0, 0, 1.0, false);
        assert_eq!(s.pattern(0).events[0].t, 0.0);
        // The pad sounded it itself, so the pass that follows does not play it again...
        let (evs, _) = run(&mut s, 0, 8000, 512, BPF);
        assert!(evs.is_empty(), "{evs:?}");
        // ...and every pass after that does.
        let (evs, _) = run(&mut s, 0, 88200, 512, BPF);
        assert_eq!(evs.len(), 1);
    }

    #[test]
    fn nothing_is_recorded_unless_rec_is_armed_and_the_pattern_plays() {
        let mut s = Sequencer::new();
        assert!(!s.note_on(0, 0, 1.0, false));
        s.set_rec(true);
        assert!(!s.note_on(0, 0, 1.0, false));
        s.set_rec(false);
        s.play();
        assert!(!s.note_on(0, 0, 1.0, false));
        assert!(s.pattern(0).events.is_empty());
        // Stop takes REC off.
        s.set_rec(true);
        s.stop();
        assert!(!s.rec && !s.playing);
    }

    #[test]
    fn a_pattern_loops_with_an_exact_sample_period() {
        // 120 BPM at 44.1 kHz: one bar is 88200 frames; 125 BPM: 84672.
        for (bpm, period) in [(120.0, 88200usize), (125.0, 84672)] {
            let bpf = bpm / 60.0 / 44100.0;
            for block in [64, 441, 512, 4096] {
                let mut s = Sequencer::new();
                s.set_pattern(0, &Pattern { bars: 1, events: vec![Event { t: 0.0, pad: 0, bank: 0, on: true, vel: 1.0, fresh: false }, Event { t: 1.5, pad: 1, bank: 0, on: true, vel: 1.0, fresh: false }] });
                s.play();
                let (evs, _) = run(&mut s, 0, period * 5 + 10, block, bpf);
                let a: Vec<usize> = evs.iter().filter(|e| e.1.pad == 0).map(|e| e.0).collect();
                let b: Vec<usize> = evs.iter().filter(|e| e.1.pad == 1).map(|e| e.0).collect();
                assert_eq!(a, (0..6).map(|k| k * period).collect::<Vec<_>>(), "bpm {bpm} block {block}");
                assert_eq!(b, (0..5).map(|k| k * period + period * 3 / 8).collect::<Vec<_>>(), "bpm {bpm} block {block}");
            }
        }
    }

    #[test]
    fn a_period_that_is_not_a_whole_number_of_frames_does_not_drift() {
        // 128 BPM: one bar is 82687.5 frames. After 64 bars the start is on frame 5292000.
        let bpf = 128.0 / 60.0 / 44100.0;
        let mut s = Sequencer::new();
        s.set_pattern(0, &Pattern { bars: 1, events: vec![Event { t: 0.0, pad: 0, bank: 0, on: true, vel: 1.0, fresh: false }] });
        s.play();
        let (evs, _) = run(&mut s, 0, 5292000 + 100, 480, bpf);
        assert_eq!(evs.len(), 65);
        assert_eq!(evs[64].0, 5292000);
        for w in evs.windows(2) {
            let d = w[1].0 - w[0].0;
            assert!(d == 82687 || d == 82688, "{d}");
        }
    }

    #[test]
    fn overdub_adds_without_erasing() {
        let mut s = recording();
        run(&mut s, 0, 22050, 512, BPF);
        s.note_on(0, 0, 1.0, false);
        run(&mut s, 0, 88200, 512, BPF);
        // Second pass: another pad, and the first pad again on the same beat (a duplicate).
        s.note_on(0, 0, 1.0, false);
        run(&mut s, 0, 44100, 512, BPF);
        s.note_on(5, 0, 0.5, false);
        let p = s.pattern(0);
        assert_eq!(p.events.iter().map(|e| (e.t, e.pad)).collect::<Vec<_>>(), vec![(1.0, 0), (3.0, 5)]);
        // REC off and on again keeps what is there.
        s.set_rec(false);
        s.set_rec(true);
        run(&mut s, 0, 22050, 512, BPF);
        s.note_on(7, 2, 1.0, false);
        assert_eq!(s.pattern(0).events.len(), 3);
        // The events come back in time order (the pass that follows the recording skips the
        // note the pad has just sounded itself).
        s.set_rec(false);
        run(&mut s, 0, 88200, 512, BPF);
        let (evs, _) = run(&mut s, 0, 88200, 512, BPF);
        assert_eq!(evs.iter().map(|e| (e.0, e.1.pad)).collect::<Vec<_>>(), vec![(0, 7), (22050, 0), (66150, 5)]);
    }

    #[test]
    fn a_note_recorded_ahead_is_not_played_twice_on_its_pass() {
        let mut s = recording();
        // Played a little before beat 2: recorded on beat 2, which is still to come.
        run(&mut s, 0, 44100 - 3000, 512, BPF);
        s.note_on(0, 0, 1.0, false);
        let (evs, _) = run(&mut s, 0, 6000, 512, BPF);
        assert!(evs.is_empty());
        let (evs, _) = run(&mut s, 44100 + 3000, 88200, 512, BPF);
        assert_eq!(evs.iter().map(|e| e.0).collect::<Vec<_>>(), vec![88200 + 44100]);
        // Played a little after beat 3: recorded on beat 3, behind the position; plays next pass.
        let mut s = recording();
        run(&mut s, 0, 66150 + 2000, 512, BPF);
        s.note_on(0, 0, 1.0, false);
        let (evs, _) = run(&mut s, 66150 + 2000, 88200, 512, BPF);
        assert_eq!(evs.iter().map(|e| e.0).collect::<Vec<_>>(), vec![88200 + 66150]);
    }

    #[test]
    fn erase_clears_one_pad_and_clear_empties_the_pattern() {
        let mut s = recording();
        s.rec_unit = 0.0;
        for (k, pad) in [0usize, 1, 0, 2].into_iter().enumerate() {
            run(&mut s, 0, 10000, 512, BPF);
            s.note_on(pad, 0, 1.0, false);
            let _ = k;
        }
        assert_eq!(s.pattern(0).events.len(), 4);
        s.erase_track(0);
        assert_eq!(s.pattern(0).events.iter().map(|e| e.pad).collect::<Vec<_>>(), vec![1, 2]);
        // What is left still plays, each once per pass.
        s.set_rec(false);
        let (evs, _) = run(&mut s, 0, 88200 * 2, 512, BPF);
        assert_eq!(evs.iter().filter(|e| e.1.pad == 1).count(), 2);
        assert_eq!(evs.iter().filter(|e| e.1.pad == 0).count(), 0);
        s.set_bars(4);
        s.clear(0);
        assert!(s.pattern(0).events.is_empty() && s.pattern(0).bars == 1 && !s.dirty(0));
        let (evs, _) = run(&mut s, 0, 88200 * 2, 512, BPF);
        assert!(evs.is_empty());
    }

    #[test]
    fn the_pattern_length_can_be_one_two_or_four_bars() {
        let mut s = Sequencer::new();
        let ev = |t: f64, pad: u8| Event { t, pad, bank: 0, on: true, vel: 1.0, fresh: false };
        s.set_pattern(0, &Pattern { bars: 4, events: vec![ev(0.0, 0), ev(6.0, 1), ev(13.0, 2)] });
        assert_eq!(s.beats(), 16.0);
        s.play();
        let (evs, _) = run(&mut s, 0, 88200 * 4, 512, BPF);
        assert_eq!(evs.iter().map(|e| (e.0, e.1.pad)).collect::<Vec<_>>(), vec![(0, 0), (6 * 22050, 1), (13 * 22050, 2)]);
        // Two bars: the note in bar 4 is kept but not played; the loop is 176400 frames.
        s.stop();
        s.set_bars(2);
        assert!(s.dirty(0));
        s.play();
        let (evs, _) = run(&mut s, 0, 88200 * 4, 512, BPF);
        assert_eq!(evs.iter().map(|e| (e.0, e.1.pad)).collect::<Vec<_>>(), vec![(0, 0), (6 * 22050, 1), (176400, 0), (176400 + 6 * 22050, 1)]);
        assert_eq!(s.pattern(0).events.len(), 3);
        // One bar while playing: the position stays inside the pattern.
        s.set_bars(1);
        assert!(s.pos() < 4.0);
        let (evs, _) = run(&mut s, 0, 88200 * 2, 512, BPF);
        assert_eq!(evs.iter().filter(|e| e.1.pad == 0).count(), 2);
        assert_eq!(evs.iter().filter(|e| e.1.pad != 0).count(), 0);
        // Anything else than 1, 2, 4 is brought to one of them.
        s.set_bars(3);
        assert_eq!(s.pattern(0).bars, 2);
    }

    #[test]
    fn the_metronome_clicks_on_every_beat() {
        let mut s = Sequencer::new();
        s.metronome = true;
        s.set_bars(2);
        s.play();
        let (_, clicks) = run(&mut s, 0, 22050 * 17 + 5, 512, BPF);
        assert_eq!(clicks, (0..18).map(|k| k * 22050).collect::<Vec<_>>());
        // Off: no clicks, the beats still pass.
        s.metronome = false;
        let (_, clicks) = run(&mut s, 0, 88200, 512, BPF);
        assert!(clicks.is_empty());
        // Stopped: none.
        s.metronome = true;
        s.stop();
        let (_, clicks) = run(&mut s, 0, 88200, 512, BPF);
        assert!(clicks.is_empty());
    }

    #[test]
    fn a_tempo_change_keeps_the_position() {
        let mut s = Sequencer::new();
        s.metronome = true;
        s.play();
        run(&mut s, 0, 11025, 512, BPF);
        assert!((s.pos() - 0.5).abs() < 1e-9);
        // Twice as fast from here: beat 1 comes after another quarter of a second.
        let (_, clicks) = run(&mut s, 0, 11025, 512, BPF * 2.0);
        assert_eq!(clicks, vec![5513]);
        assert!((s.pos() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn a_gate_pad_records_a_note_on_and_a_note_off() {
        let mut s = recording();
        run(&mut s, 0, 22050 + 1000, 512, BPF);
        s.note_on(2, 0, 1.0, true);
        run(&mut s, 0, 30000, 512, BPF);
        s.note_off(2, 0);
        let p = s.pattern(0);
        assert_eq!(p.events.len(), 2);
        assert_eq!((p.events[0].t, p.events[0].on), (1.0, true));
        assert!(!p.events[1].on && (p.events[1].t - (22050.0 + 31000.0) * BPF).abs() < 1e-9);
        // Released before the quantized note-on has come: the note-off goes one step after it.
        let mut s = recording();
        run(&mut s, 0, 44100 - 4000, 512, BPF);
        s.note_on(2, 0, 1.0, true);
        run(&mut s, 0, 1000, 512, BPF);
        s.note_off(2, 0);
        let p = s.pattern(0);
        assert_eq!(p.events.iter().map(|e| (e.t, e.on)).collect::<Vec<_>>(), vec![(2.0, true), (3.0, false)]);
        // Held for a whole pass: the note-off is added just before the note-on.
        let mut s = recording();
        run(&mut s, 0, 22050 + 1000, 512, BPF);
        s.note_on(2, 0, 1.0, true);
        run(&mut s, 0, 88200, 512, BPF);
        let p = s.pattern(0);
        assert_eq!(p.events.len(), 2);
        assert!(!p.events[0].on && (p.events[0].t - 0.99).abs() < 1e-9, "{:?}", p.events);
    }

    #[test]
    fn a_pattern_change_while_playing_waits_for_the_next_bar() {
        let mut s = Sequencer::new();
        let ev = |t: f64, pad: u8| Event { t, pad, bank: 0, on: true, vel: 1.0, fresh: false };
        s.set_pattern(0, &Pattern { bars: 2, events: vec![ev(0.0, 0), ev(5.0, 0)] });
        s.set_pattern(1, &Pattern { bars: 1, events: vec![ev(0.0, 9)] });
        // Stopped: at once.
        s.select(1, 4.0);
        assert_eq!(s.current(), 1);
        s.select(0, 4.0);
        s.set_rec(true);
        s.play();
        run(&mut s, 0, 30000, 512, BPF);
        s.select(1, 4.0);
        assert!(!s.rec, "choosing a pattern takes REC off");
        assert_eq!((s.current(), s.reserved()), (0, Some(1)));
        // The change comes on beat 4 (frame 88200), not at the end of the two bars.
        let (evs, _) = run(&mut s, 30000, 88200 * 2, 512, BPF);
        assert_eq!(evs.iter().map(|e| (e.0, e.1.pad)).collect::<Vec<_>>(), vec![(88200, 9), (176400, 9)]);
        assert_eq!((s.current(), s.reserved()), (1, None));
    }

    #[test]
    fn patterns_survive_a_save_and_restore_round_trip() {
        let mut s = recording();
        s.rec_unit = 0.25;
        s.set_bars(2);
        for (pad, vel) in [(0usize, 1.0f32), (4, 0.5), (15, 0.25)] {
            run(&mut s, 0, 30000, 512, BPF);
            s.note_on(pad, pad % 4, vel, pad == 15);
        }
        run(&mut s, 0, 5000, 512, BPF);
        s.note_off(15, 3);
        let saved = s.pattern(0).clone();
        let json = serde_json::to_string(&saved).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        let mut t = Sequencer::new();
        t.set_pattern(3, &back);
        assert!(!t.dirty(3));
        t.select(3, 4.0);
        assert_eq!(t.pattern(3).bars, 2);
        let strip = |p: &Pattern| p.events.iter().map(|e| (e.t, e.pad, e.bank, e.on, e.vel)).collect::<Vec<_>>();
        assert_eq!(strip(t.pattern(3)), strip(&saved));
        assert_eq!(saved.events.len(), 4);
        // Both play the same thing.
        s.stop();
        s.play();
        t.play();
        let (a, _) = run(&mut s, 0, 176400, 512, BPF);
        let (b, _) = run(&mut t, 0, 176400, 512, BPF);
        assert_eq!(a.iter().map(|e| (e.0, e.1.pad, e.1.on)).collect::<Vec<_>>(), b.iter().map(|e| (e.0, e.1.pad, e.1.on)).collect::<Vec<_>>());
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn a_full_pattern_takes_no_more_notes_and_never_grows() {
        let mut s = recording();
        s.rec_unit = 0.0;
        let cap = s.pattern(0).events.capacity();
        for _ in 0..MAX_EVENTS + 50 {
            run(&mut s, 0, 37, 37, BPF);
            s.note_on(0, 0, 1.0, false);
        }
        assert_eq!(s.pattern(0).events.len(), MAX_EVENTS);
        assert_eq!(s.pattern(0).events.capacity(), cap);
    }
}
