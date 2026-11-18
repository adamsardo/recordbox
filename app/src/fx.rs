//! BEAT FX and RELEASE FX of the Performance mixer: two FX units (FX1, FX2), each with three
//! effect slots (rekordbox's "multi mode"), assignable to deck 1, deck 2 or the master.
//!
//! What follows rekordbox 7.2.19 (`checks/re-7.2.19/mixer-fx-recording.md`, section 2): the
//! effect list and its order, the beat tables and their defaults, the insert points (a deck's
//! effects after its channel fader and the crossfader, the master's before the master
//! limiter), a separate effect state per assigned channel, the 3.33 ms smoothing of on/off,
//! the echo family's LEVEL/DEPTH law and feedback, TRANS as a square gate, FLANGER's sweep
//! range, PHASER's sweep range, ROLL looping the last beat from a 15 s history, the reverb's
//! structure (five damped combs into two all-passes, feedback 0.9265 to 0.9986) and the
//! RELEASE FX time rules. What does not: the exact filter tables, reverb tunings, mix laws
//! other than the echo's and SPIRAL's pitch-shifting feedback were not traced; those parts
//! are recordbox's own and are marked below.

use parking_lot::Mutex;

/// Effects that exist here, in rekordbox's multi-mode list order (the others of its 35 are
/// listed in the menu but not built).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Delay,
    Echo,
    Spiral,
    Reverb,
    Trans,
    Pan,
    Filter,
    Flanger,
    Phaser,
    SlipRoll,
    Roll,
    RevRoll,
    LowCutEcho,
    PingPong,
}

/// rekordbox's multi-mode effect list: (name, the effect if recordbox has it).
pub const LIST: [(&str, Option<Kind>); 35] = [
    ("DELAY", Some(Kind::Delay)),
    ("ECHO", Some(Kind::Echo)),
    ("SPIRAL", Some(Kind::Spiral)),
    ("REVERB", Some(Kind::Reverb)),
    ("REV DELAY", None),
    ("MT DELAY", None),
    ("UP ECHO", None),
    ("DOWN ECHO", None),
    ("TRANS", Some(Kind::Trans)),
    ("PAN", Some(Kind::Pan)),
    ("FILTER", Some(Kind::Filter)),
    ("FLANGER", Some(Kind::Flanger)),
    ("PHASER", Some(Kind::Phaser)),
    ("SLIP ROLL", Some(Kind::SlipRoll)),
    ("ROLL", Some(Kind::Roll)),
    ("REV ROLL", Some(Kind::RevRoll)),
    ("ROBOT", None),
    ("PITCH", None),
    ("ENIGMA JET", None),
    ("MOBIUS SAW", None),
    ("MOBIUS TRI", None),
    ("LOW CUT ECHO", Some(Kind::LowCutEcho)),
    ("PING PONG", Some(Kind::PingPong)),
    ("HELIX", None),
    ("VINYL BRAKE", None),
    ("STRETCH", None),
    ("BPF ECHO", None),
    ("NOISE", None),
    ("SPIRAL UP", None),
    ("REVERB UP", None),
    ("HPF ECHO", None),
    ("LPF ECHO", None),
    ("CRUSH ECHO", None),
    ("SPIRAL DOWN", None),
    ("REVERB DOWN", None),
];

pub fn name(k: Kind) -> &'static str {
    LIST.iter().find(|x| x.1 == Some(k)).map(|x| x.0).unwrap_or("")
}

/// The values the `<` `>` buttons step through for an effect, and where they start:
/// (values, default index, percent instead of beats). rekordbox's tables B, C, D and P1.
pub fn beat_table(k: Kind) -> (&'static [f32], usize, bool) {
    const B: [f32; 10] = [0.0625, 0.125, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
    const C: [f32; 9] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
    const D: [f32; 11] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    const P1: [f32; 7] = [1.0, 10.0, 25.0, 50.0, 75.0, 90.0, 100.0];
    match k {
        Kind::Delay | Kind::Echo | Kind::Spiral | Kind::LowCutEcho | Kind::PingPong => (&B, 5, false),
        Kind::Reverb => (&P1, 3, true),
        Kind::Trans | Kind::Pan | Kind::SlipRoll | Kind::Roll | Kind::RevRoll => (&C, 4, false),
        Kind::Filter | Kind::Flanger | Kind::Phaser => (&D, 6, false),
    }
}

/// A beat value as rekordbox prints it: whole beats as `N`, fractions as `n/d`, percent `N%`.
pub fn beat_text(v: f32, percent: bool) -> String {
    if percent {
        return format!("{v:.0}%");
    }
    if v >= 1.0 {
        return format!("{v:.0}");
    }
    if (v - 0.75).abs() < 1e-6 {
        return "3/4".into();
    }
    format!("1/{:.0}", 1.0 / v)
}

/// RELEASE FX: (name, default index into table B). VINYL BRAKE 3/4, ECHO 1, BACK SPIN 4.
pub const RELEASE: [(&str, usize); 3] = [("VINYL BRAKE", 4), ("ECHO", 5), ("BACK SPIN", 7)];
pub const RELEASE_BEATS: [f32; 10] = [0.0625, 0.125, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];

/// What the interface sets for one slot.
#[derive(Clone, Copy)]
pub struct SlotParams {
    pub kind: Kind,
    pub on: bool,
    /// LEVEL/DEPTH, 0..1.
    pub level: f32,
    /// Beats (or percent for REVERB).
    pub value: f32,
}

#[derive(Clone, Copy)]
pub struct UnitParams {
    /// Decks 1 to 4, then the master.
    pub assign: [bool; 5],
    pub slots: [SlotParams; 3],
    /// RELEASE FX held: which one and its beats.
    pub release: Option<(u8, f32)>,
    /// Tempo each channel's effects run at (BPM).
    pub bpm: [f32; 5],
}

impl UnitParams {
    pub fn new() -> UnitParams {
        let s = |kind| SlotParams { kind, on: false, level: 0.5, value: beat_table(kind).0[beat_table(kind).1] };
        // rekordbox's default slots: DELAY, ECHO, SPIRAL.
        UnitParams { assign: [false; 5], slots: [s(Kind::Delay), s(Kind::Echo), s(Kind::Spiral)], release: None, bpm: [120.0; 5] }
    }
}

const MAX_RATE: usize = 48000;
/// Longest delay (4 s) and the history kept for ROLL and RELEASE FX (15 s), in frames.
const DELAY_LEN: usize = 4 * MAX_RATE;
const RING_LEN: usize = 15 * MAX_RATE;
/// On/off and gate smoothing, as rekordbox's 3.3333 ms.
const SMOOTH_SECS: f32 = 0.0033333;

/// One effect's state for one channel.
struct Slot {
    kind: Kind,
    /// Stereo delay line (interleaved) and its write position (frames).
    delay: Vec<f32>,
    w: usize,
    /// Delay time in use (frames), moving toward the wanted time.
    time: f32,
    /// Wet gain in use (0 off .. 1 on).
    gate: f32,
    lfo: f32,
    /// One-pole and state-variable filter states, all-pass states.
    lp: [f32; 2],
    svf: [[f32; 2]; 2],
    ap: [[f32; 8]; 2],
    fb: [f32; 2],
    /// Reverb: comb and all-pass lines with their positions.
    combs: Vec<Vec<f32>>,
    comb_pos: [usize; 10],
    comb_lp: [f32; 10],
    aps: Vec<Vec<f32>>,
    ap_pos: [usize; 4],
    /// ROLL: where the captured loop starts in the history, its length and the read position.
    roll: Option<(usize, usize, f32)>,
    was_on: bool,
    ping: bool,
}

impl Slot {
    fn new() -> Slot {
        // Reverb line lengths in frames at 48 kHz (recordbox's own tunings: mutually prime,
        // 25 to 37 ms combs, 5 and 1.7 ms all-passes; the right channel 23 frames longer).
        let comb_len = [1215usize, 1293, 1390, 1476, 1548];
        let ap_len = [240usize, 82];
        let mut combs = Vec::new();
        for ch in 0..2 {
            for l in comb_len {
                combs.push(vec![0.0; l + ch * 23]);
            }
        }
        let mut aps = Vec::new();
        for ch in 0..2 {
            for l in ap_len {
                aps.push(vec![0.0; l + ch * 23]);
            }
        }
        Slot { kind: Kind::Delay, delay: vec![0.0; DELAY_LEN * 2], w: 0, time: 0.0, gate: 0.0, lfo: 0.0, lp: [0.0; 2], svf: [[0.0; 2]; 2], ap: [[0.0; 8]; 2], fb: [0.0; 2], combs, comb_pos: [0; 10], comb_lp: [0.0; 10], aps, ap_pos: [0; 4], roll: None, was_on: false, ping: false }
    }

    fn clear(&mut self) {
        self.delay.iter_mut().for_each(|v| *v = 0.0);
        self.combs.iter_mut().for_each(|c| c.iter_mut().for_each(|v| *v = 0.0));
        self.aps.iter_mut().for_each(|c| c.iter_mut().for_each(|v| *v = 0.0));
        self.lp = [0.0; 2];
        self.svf = [[0.0; 2]; 2];
        self.ap = [[0.0; 8]; 2];
        self.fb = [0.0; 2];
        self.comb_lp = [0.0; 10];
        self.time = 0.0;
        self.lfo = 0.0;
        self.roll = None;
    }

    /// Read the delay line `frames` back (fractional), channel `c`.
    #[inline]
    fn tap(&self, frames: f32, c: usize) -> f32 {
        let f = frames.clamp(1.0, (DELAY_LEN - 2) as f32);
        let i = f as usize;
        let t = f - i as f32;
        let a = self.delay[((self.w + DELAY_LEN - i) % DELAY_LEN) * 2 + c];
        let b = self.delay[((self.w + DELAY_LEN - i - 1) % DELAY_LEN) * 2 + c];
        a + (b - a) * t
    }

    fn process(&mut self, p: &SlotParams, bpm: f32, buf: &mut [f32], rate: u32, ring: &Ring) {
        if p.kind != self.kind {
            self.kind = p.kind;
            self.clear();
            self.gate = 0.0;
            self.was_on = false;
        }
        if !p.on && self.gate <= 0.0 {
            self.was_on = false;
            return;
        }
        let r = rate as f32;
        let beat = 60.0 / bpm.clamp(20.0, 1000.0);
        let secs = (p.value * beat).clamp(0.010, 32.0);
        let gate_k = 1.0 / (SMOOTH_SECS * r);
        let level = p.level.clamp(0.0, 1.0);
        // The echo family's LEVEL/DEPTH law (rekordbox): the dry signal stays whole up to
        // three quarters, the echo rises to full there, then the dry signal falls.
        let (echo_dry, echo_wet) = if level < 0.75 { (1.0, 1.3333 * level) } else { (4.0 - 4.0 * level, 1.0) };
        let frames = buf.len() / 2;
        let starting = p.on && !self.was_on;
        self.was_on = p.on;
        if starting {
            self.lfo = 0.0;
            if matches!(self.kind, Kind::Roll | Kind::RevRoll | Kind::SlipRoll) {
                // Catch the last beat (10 ms to 4 s) from the history and loop it.
                let len = ((secs.clamp(0.010, 4.0)) * r) as usize;
                self.roll = Some(((ring.w + RING_LEN - len.min(RING_LEN - 1)) % RING_LEN, len.max(1), 0.0));
            }
        }
        let want_time = (secs.min(4.0) * r).min((DELAY_LEN - 2) as f32);
        if self.time <= 0.0 {
            self.time = want_time;
        }
        let lfo_step = 1.0 / (secs * r);
        for n in 0..frames {
            let target = if p.on { 1.0 } else { 0.0 };
            self.gate += (target - self.gate).clamp(-gate_k, gate_k);
            let g = self.gate;
            let (dl, dr) = (buf[n * 2], buf[n * 2 + 1]);
            // The delay time glides to a new value instead of jumping (pitch slides, as a
            // tape echo's would, for the few milliseconds it takes).
            self.time += (want_time - self.time) * (1.0 / (0.05 * r));
            self.lfo += lfo_step;
            if self.lfo >= 1.0 {
                self.lfo -= 1.0;
            }
            let tri = 1.0 - (2.0 * self.lfo - 1.0).abs(); // 0..1..0
            let (ol, or) = match self.kind {
                Kind::Delay | Kind::Echo | Kind::LowCutEcho | Kind::PingPong | Kind::Spiral => {
                    // DELAY: one repeat. ECHO: feedback 0.7 (rekordbox's centre value).
                    let fbk = match self.kind {
                        Kind::Delay => 0.0,
                        Kind::Spiral => 0.82,
                        _ => 0.7,
                    };
                    let (mut el, mut er) = (self.tap(self.time, 0), self.tap(self.time, 1));
                    if self.kind == Kind::LowCutEcho {
                        // Low cut on the echo path: a first-order high-pass at 400 Hz
                        // (recordbox's own corner).
                        let k = 1.0 - (-std::f32::consts::TAU * 400.0 / r).exp();
                        self.lp[0] += (el - self.lp[0]) * k;
                        self.lp[1] += (er - self.lp[1]) * k;
                        el -= self.lp[0];
                        er -= self.lp[1];
                    }
                    if self.kind == Kind::Spiral {
                        // SPIRAL: a long-feedback echo whose repeats darken. (rekordbox also
                        // shifts the feedback's pitch; that part is not built.)
                        let k = 1.0 - (-std::f32::consts::TAU * 3500.0 / r).exp();
                        self.lp[0] += (el - self.lp[0]) * k;
                        self.lp[1] += (er - self.lp[1]) * k;
                        el = self.lp[0];
                        er = self.lp[1];
                    }
                    // The echo keeps ringing after OFF only as long as the gate takes to
                    // close; the input is fed while the effect is on.
                    let (il, ir) = (dl * target, dr * target);
                    let (wl, wr) = if self.kind == Kind::PingPong {
                        // The repeats alternate sides: left's echo feeds right and back.
                        (il + ir + er * fbk, el * fbk)
                    } else {
                        (il + el * fbk, ir + er * fbk)
                    };
                    self.delay[self.w * 2] = wl;
                    self.delay[self.w * 2 + 1] = wr;
                    self.w = (self.w + 1) % DELAY_LEN;
                    (dl * (1.0 + (echo_dry - 1.0) * g) + el * echo_wet * g, dr * (1.0 + (echo_dry - 1.0) * g) + er * echo_wet * g)
                }
                Kind::Reverb => {
                    // Five damped combs in parallel into two all-passes in series; the comb
                    // feedback runs over rekordbox's 0.9265 .. 0.9986 with the percent value.
                    let fbk = 0.9265 + (0.9986 - 0.9265) * (p.value / 100.0).clamp(0.0, 1.0) * 0.6;
                    let mut out = [0.0f32; 2];
                    for c in 0..2 {
                        let x = if c == 0 { dl } else { dr } * target * 0.2;
                        let mut acc = 0.0;
                        for k in 0..5 {
                            let i = c * 5 + k;
                            let line = &mut self.combs[i];
                            let len = line.len();
                            let y = line[self.comb_pos[i]];
                            self.comb_lp[i] = y * 0.75 + self.comb_lp[i] * 0.25;
                            line[self.comb_pos[i]] = x + self.comb_lp[i] * fbk;
                            self.comb_pos[i] = (self.comb_pos[i] + 1) % len;
                            acc += y;
                        }
                        for k in 0..2 {
                            let i = c * 2 + k;
                            let line = &mut self.aps[i];
                            let len = line.len();
                            let d = line[self.ap_pos[i]];
                            let y = d - acc * 0.5;
                            line[self.ap_pos[i]] = acc + d * 0.5;
                            self.ap_pos[i] = (self.ap_pos[i] + 1) % len;
                            acc = y;
                        }
                        out[c] = acc;
                    }
                    (dl * (1.0 + (echo_dry - 1.0) * g) + out[0] * echo_wet * g, dr * (1.0 + (echo_dry - 1.0) * g) + out[1] * echo_wet * g)
                }
                Kind::Trans => {
                    // A square gate once per beat value, open for the first half, its edges
                    // smoothed over 3.33 ms; LEVEL/DEPTH sets how far it closes.
                    let open = if self.lfo < 0.5 { 1.0 } else { 0.0 };
                    self.fb[0] += (open - self.fb[0]).clamp(-gate_k, gate_k);
                    let a = 1.0 - level * g * (1.0 - self.fb[0]);
                    (dl * a, dr * a)
                }
                Kind::Pan => {
                    // The sound moves left and right once per beat value.
                    let pan = (std::f32::consts::TAU * self.lfo).sin() * level * g;
                    (dl * (1.0 - pan.max(0.0)), dr * (1.0 + pan.min(0.0)))
                }
                Kind::Filter => {
                    // A resonant low-pass whose cutoff a triangle LFO sweeps between 120 Hz
                    // and 14 kHz once per beat value (recordbox's own range and resonance;
                    // rekordbox's tables were not traced).
                    let fc = 120.0 * (14000.0f32 / 120.0).powf(tri);
                    let f = 2.0 * (std::f32::consts::PI * (fc / r).min(0.24)).sin();
                    let q = 0.35;
                    let mut o = [0.0f32; 2];
                    for (c, x) in [dl, dr].into_iter().enumerate() {
                        let s = &mut self.svf[c];
                        let hp = x - s[0] - q * s[1];
                        s[1] += f * hp;
                        s[0] += f * s[1];
                        o[c] = s[0];
                    }
                    let m = level * g;
                    (dl * (1.0 - m) + o[0] * m, dr * (1.0 - m) + o[1] * m)
                }
                Kind::Flanger => {
                    // A feedback comb whose delay a triangle LFO sweeps between one sample
                    // and 2.6667 ms once per beat value.
                    let d = 1.0 + tri * (0.0026667 * r - 1.0);
                    let (el, er) = (self.tap(d, 0), self.tap(d, 1));
                    self.delay[self.w * 2] = dl + el * 0.7;
                    self.delay[self.w * 2 + 1] = dr + er * 0.7;
                    self.w = (self.w + 1) % DELAY_LEN;
                    let m = level * g;
                    (dl + el * m, dr + er * m)
                }
                Kind::Phaser => {
                    // Six first-order all-pass stages, their centre swept 100 Hz .. 15 kHz.
                    let fc = 100.0 * (15000.0f32 / 100.0).powf(tri);
                    let t = (std::f32::consts::PI * (fc / r).min(0.49)).tan();
                    let a = (t - 1.0) / (t + 1.0);
                    let mut o = [0.0f32; 2];
                    for (c, x) in [dl, dr].into_iter().enumerate() {
                        let mut y = x + self.fb[c] * 0.5;
                        for k in 0..6 {
                            let z = self.ap[c][k];
                            let out = a * y + z;
                            self.ap[c][k] = y - a * out;
                            y = out;
                        }
                        self.fb[c] = y;
                        o[c] = y;
                    }
                    let m = level * g;
                    (dl + o[0] * m, dr + o[1] * m)
                }
                Kind::Roll | Kind::RevRoll | Kind::SlipRoll => {
                    let (mut ol, mut or) = (dl, dr);
                    if let Some((start, len, pos)) = self.roll.as_mut() {
                        let i = *pos as usize % *len;
                        let j = if self.kind == Kind::RevRoll { *len - 1 - i } else { i };
                        let k = (*start + j) % RING_LEN;
                        // A few frames of fade at the loop's ends, so the splice is silent.
                        let edge = (i.min(*len - 1 - i) as f32 / (0.002 * r)).min(1.0);
                        ol = ring.buf[k * 2] * edge;
                        or = ring.buf[k * 2 + 1] * edge;
                        *pos += 1.0;
                        if *pos as usize >= *len {
                            *pos = 0.0;
                            if self.kind == Kind::SlipRoll && p.on {
                                // SLIP ROLL catches the newest beat again on every turn.
                                *start = (ring.w + RING_LEN - (*len).min(RING_LEN - 1)) % RING_LEN;
                            }
                        }
                    }
                    let m = level * g;
                    (dl * (1.0 - m) + ol * m, dr * (1.0 - m) + or * m)
                }
            };
            buf[n * 2] = ol;
            buf[n * 2 + 1] = or;
        }
        if !p.on && self.gate <= 0.0 {
            // Off and faded out: the next ON starts clean.
            self.clear();
        }
    }
}

/// The last 15 s of a channel (interleaved stereo), for ROLL and RELEASE FX.
struct Ring {
    buf: Vec<f32>,
    w: usize,
}

/// RELEASE FX state for one channel.
struct Release {
    kind: u8,
    /// Frames since it was pressed, and where in the history playback stands (frames back
    /// from the press).
    t: f32,
    pos: f32,
    start: usize,
    gate: f32,
    held: bool,
}

struct Channel {
    slots: [Slot; 3],
    ring: Ring,
    release: Release,
}

impl Channel {
    fn new() -> Channel {
        Channel { slots: [Slot::new(), Slot::new(), Slot::new()], ring: Ring { buf: vec![0.0; RING_LEN * 2], w: 0 }, release: Release { kind: 0, t: 0.0, pos: 0.0, start: 0, gate: 0.0, held: false } }
    }

    fn process(&mut self, p: &UnitParams, ch: usize, buf: &mut [f32], rate: u32) {
        let frames = buf.len() / 2;
        let r = rate as f32;
        let holding = p.release.is_some();
        // While a RELEASE FX is held the unit's Beat FX are out (rekordbox cancels them).
        if !holding && self.release.gate <= 0.0 {
            for (s, sp) in self.slots.iter_mut().zip(p.slots.iter()) {
                s.process(sp, p.bpm[ch], buf, rate, &self.ring);
            }
        }
        // RELEASE FX: press and hold. The time is the beat value (rekordbox: VINYL BRAKE
        // 10 ms .. 12 s, ECHO 10 ms .. 4 s, BACK SPIN's stop time).
        if let Some((kind, beats)) = p.release {
            if !self.release.held {
                self.release = Release { kind, t: 0.0, pos: 0.0, start: self.ring.w, gate: self.release.gate, held: true };
            }
            let secs = (beats * 60.0 / p.bpm[ch].clamp(20.0, 1000.0)).clamp(0.010, 12.0);
            let total = secs * r;
            let gk = 1.0 / (SMOOTH_SECS * r);
            for n in 0..frames {
                let rel = &mut self.release;
                rel.gate = (rel.gate + gk).min(1.0);
                let x = (rel.t / total).min(1.0);
                // Playback speed over the history: the brake slows from 1 to 0; the back spin
                // runs backwards, up to four times speed a fifth of the way in, then to a stop.
                let speed = match rel.kind {
                    0 => 1.0 - x,
                    2 => {
                        if x < 0.2 { -4.0 * (x / 0.2) } else { -4.0 * (1.0 - (x - 0.2) / 0.8) }
                    }
                    _ => 0.0,
                };
                let (wl, wr) = if rel.kind == 1 {
                    // ECHO: the input is cut and the last beat repeats, each time at 0.7.
                    let len = (total as usize).clamp(1, RING_LEN - 1);
                    let i = rel.t as usize;
                    let turn = i / len;
                    let k = (rel.start + RING_LEN - len + i % len) % RING_LEN;
                    let a = 0.7f32.powi(turn as i32 + 1);
                    (self.ring.buf[k * 2] * a, self.ring.buf[k * 2 + 1] * a)
                } else {
                    rel.pos += speed;
                    let f = rel.start as f32 + rel.pos;
                    let k = (f.rem_euclid(RING_LEN as f32)) as usize % RING_LEN;
                    let a = if x >= 1.0 { 0.0 } else { (1.0 - x).min(1.0).sqrt() };
                    (self.ring.buf[k * 2] * a, self.ring.buf[k * 2 + 1] * a)
                };
                rel.t += 1.0;
                let g = rel.gate;
                buf[n * 2] = buf[n * 2] * (1.0 - g) + wl * g;
                buf[n * 2 + 1] = buf[n * 2 + 1] * (1.0 - g) + wr * g;
            }
        } else {
            self.release.held = false;
            if self.release.gate > 0.0 {
                // Let go: the track comes back over 3.33 ms.
                let gk = 1.0 / (SMOOTH_SECS * r);
                for n in 0..frames {
                    self.release.gate = (self.release.gate - gk).max(0.0);
                    let g = 1.0 - self.release.gate;
                    buf[n * 2] *= g;
                    buf[n * 2 + 1] *= g;
                }
            }
        }
    }

    /// Keep the channel's dry history (before the effects) for ROLL and RELEASE FX.
    fn remember(&mut self, buf: &[f32]) {
        if self.release.held {
            return;
        }
        for f in buf.chunks_exact(2) {
            self.ring.buf[self.ring.w * 2] = f[0];
            self.ring.buf[self.ring.w * 2 + 1] = f[1];
            self.ring.w = (self.ring.w + 1) % RING_LEN;
        }
    }
}

/// The master's channel index in `UnitParams::assign`.
pub const MASTER: usize = 4;

pub struct Unit {
    pub params: UnitParams,
    /// One effect state per channel, made (by the interface, not the audio thread) the first
    /// time the unit is assigned to that channel: each holds about 10 MB of delay lines.
    channels: [Option<Box<Channel>>; 5],
}

impl Unit {
    /// Make the effect states of the channels this unit is assigned to.
    pub fn prepare(&mut self, assign: &[bool; 5]) {
        for (c, a) in self.channels.iter_mut().zip(assign) {
            if *a && c.is_none() {
                *c = Some(Box::new(Channel::new()));
            }
        }
    }
}

/// Both FX units and the scratch buffers the decks are rendered into on their way through them.
pub struct Rack {
    pub units: [Unit; 2],
    pub scratch: [Vec<f32>; 4],
    /// The interface has the FX panel in use (else the audio path skips the rack entirely).
    pub enabled: bool,
    /// The master's Sound Color FX: effect, COLOR (0.5 = off), PARAM.
    pub master_cfx: crate::cfx::Cfx,
    pub master_cfx_set: (u8, f32, f32),
}

impl Rack {
    pub fn new() -> Rack {
        let unit = || Unit { params: UnitParams::new(), channels: [None, None, None, None, None] };
        Rack { units: [unit(), unit()], scratch: std::array::from_fn(|_| vec![0.0; 16384]), enabled: false, master_cfx: crate::cfx::Cfx::new(), master_cfx_set: (0, 0.5, 0.5) }
    }

    /// Something is assigned and switched on (or held, or still fading): the effects must run.
    pub fn active(&self) -> bool {
        self.enabled && (self.units.iter().any(|u| u.params.assign.iter().any(|a| *a)) || (self.master_cfx_set.1 - 0.5).abs() > 0.011)
    }

    /// Run one channel's audio (stereo interleaved) through the units assigned to it.
    /// `ch`: 0..3 = decks 1 to 4, 4 = the master.
    pub fn process(&mut self, ch: usize, buf: &mut [f32], rate: u32) {
        if ch == MASTER {
            let (k, c, p) = self.master_cfx_set;
            self.master_cfx.process(k, c, p, buf, rate);
        }
        for u in self.units.iter_mut() {
            if u.params.assign[ch] {
                let p = u.params;
                if let Some(c) = u.channels[ch].as_deref_mut() {
                    c.remember(buf);
                    c.process(&p, ch, buf, rate);
                }
            }
        }
    }
}

/// The rack, made when Performance mode first needs it (about 30 MB of delay lines).
pub static RACK: Mutex<Option<Box<Rack>>> = Mutex::new(None);

/// Tap tempo, as rekordbox's: gaps under 60 ms are ignored, over 1.5 s start again; the mean
/// of up to 11 intervals; a tap more than 255 ms or 16.7 % off the mean starts again.
pub struct Tap {
    last: Option<f64>,
    gaps: Vec<f64>,
}

impl Tap {
    pub fn new() -> Tap {
        Tap { last: None, gaps: Vec::new() }
    }

    /// A tap at `now` seconds; the tempo so far, if there is one.
    pub fn tap(&mut self, now: f64) -> Option<f32> {
        if let Some(l) = self.last {
            let gap = now - l;
            if gap < 0.060 {
                return self.bpm();
            }
            if gap > 1.5 {
                self.gaps.clear();
            } else {
                if let Some(mean) = self.mean() {
                    if (gap - mean).abs() > 0.255 || (gap - mean).abs() / mean > 0.167 {
                        self.gaps.clear();
                    }
                }
                self.gaps.push(gap);
                if self.gaps.len() > 11 {
                    self.gaps.remove(0);
                }
            }
        }
        self.last = Some(now);
        self.bpm()
    }

    fn mean(&self) -> Option<f64> {
        if self.gaps.is_empty() { None } else { Some(self.gaps.iter().sum::<f64>() / self.gaps.len() as f64) }
    }

    fn bpm(&self) -> Option<f32> {
        self.mean().map(|m| (60.0 / m).clamp(20.0, 1000.0) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, freq: f32) -> Vec<f32> {
        (0..frames).flat_map(|i| { let v = (i as f32 * freq * std::f32::consts::TAU / 48000.0).sin() * 0.5; [v, v] }).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn unit(kind: Kind, value: f32, level: f32) -> UnitParams {
        let mut p = UnitParams::new();
        p.assign = [true, false, false, false, false];
        p.slots[0] = SlotParams { kind, on: true, level, value };
        p.bpm = [120.0; 5];
        p
    }

    #[test]
    fn nothing_on_leaves_the_sound_untouched() {
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params.assign = [true, true, false, false, true];
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let x = tone(4800, 440.0);
        let mut y = x.clone();
        rack.process(0, &mut y, 48000);
        rack.process(MASTER, &mut y, 48000);
        assert_eq!(x, y);
    }

    #[test]
    fn delay_repeats_once_one_beat_later_and_echo_keeps_repeating() {
        // A 10 ms burst, then silence; 120 BPM, 1 beat = 0.5 s = 24,000 frames.
        let burst = |n: usize| -> Vec<f32> { (0..n).flat_map(|i| { let v = if i < 480 { 0.5 } else { 0.0 }; [v, v] }).collect() };
        for (kind, third) in [(Kind::Delay, false), (Kind::Echo, true)] {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params = unit(kind, 1.0, 0.75);
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            let mut x = burst(48000 * 2);
            rack.process(0, &mut x, 48000);
            let at = |f: usize| rms(&x[f * 2..(f + 480) * 2]);
            assert!(at(0) > 0.45, "{kind:?}: the dry burst");
            assert!(at(12000) < 0.01, "{kind:?}: silence between");
            assert!(at(24000) > 0.4, "{kind:?}: the repeat a beat later, {}", at(24000));
            if third {
                // ECHO: the second repeat at 0.7 of the first.
                assert!((at(48000) / at(24000) - 0.7).abs() < 0.05, "{}", at(48000) / at(24000));
            } else {
                assert!(at(48000) < 0.01, "DELAY repeats once");
            }
        }
    }

    #[test]
    fn level_depth_law_of_the_echo_family() {
        // LEVEL/DEPTH at 0: no echo at all. Fully right: the echo only.
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Echo, 1.0, 0.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let x = tone(48000, 440.0);
        let mut y = x.clone();
        rack.process(0, &mut y, 48000);
        assert!(x.iter().zip(&y).all(|(a, b)| (a - b).abs() < 1e-6));
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Echo, 1.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut y = x.clone();
        rack.process(0, &mut y, 48000);
        // Before the first repeat arrives there is only the (removed) dry signal.
        assert!(rms(&y[2000..40000]) < 0.01);
    }

    #[test]
    fn trans_gates_half_of_each_beat_value() {
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Trans, 1.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(48000, 440.0);
        rack.process(0, &mut x, 48000);
        // 1 beat at 120 BPM = 24,000 frames: open for the first half, closed for the second.
        assert!(rms(&x[2000 * 2..10000 * 2]) > 0.3);
        assert!(rms(&x[14000 * 2..22000 * 2]) < 0.01);
        assert!(rms(&x[26000 * 2..34000 * 2]) > 0.3);
    }

    #[test]
    fn roll_loops_the_last_beat_value() {
        let mut rack = Rack::new();
        rack.enabled = true;
        // History: a rising ramp, so a frame's value tells where it came from.
        rack.units[0].params.assign = [true, false, false, false, false];
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
        let mut hist: Vec<f32> = (0..48000).flat_map(|i| { let v = i as f32 / 48000.0; [v, v] }).collect();
        rack.process(0, &mut hist, 48000);
        // ROLL on, 1/4 beat = 6,000 frames. It catches what was heard up to the buffer in
        // which it is switched on (here one of 64 frames, silent), and repeats that.
        rack.units[0].params = unit(Kind::Roll, 0.25, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut first = vec![0.0f32; 64 * 2];
        rack.process(0, &mut first, 48000);
        let mut x = vec![0.0f32; 20000 * 2];
        rack.process(0, &mut x, 48000);
        // The loop is frames 42,064..48,064 of the history; the first buffer played 64 of it.
        let v = |f: usize| x[f * 2];
        let want = (42064.0 + 64.0 + 3000.0) / 48000.0;
        assert!((v(3000) - want).abs() < 0.01, "{} vs {want}", v(3000));
        assert!((v(9000) - want).abs() < 0.01, "second turn {}", v(9000));
        assert!((v(15000) - want).abs() < 0.01);
    }

    #[test]
    fn modulation_effects_and_reverb_stay_bounded_and_change_the_sound() {
        for kind in [Kind::Filter, Kind::Flanger, Kind::Phaser, Kind::Pan, Kind::Reverb, Kind::Spiral, Kind::LowCutEcho, Kind::PingPong, Kind::SlipRoll, Kind::RevRoll] {
            let mut rack = Rack::new();
            rack.enabled = true;
            let (t, i, _) = beat_table(kind);
            rack.units[0].params = unit(kind, t[i], 0.8);
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            let x = tone(48000 * 3, 330.0);
            let mut y = x.clone();
            rack.process(0, &mut y, 48000);
            let peak = y.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!(peak.is_finite() && peak < 4.0, "{kind:?} peak {peak}");
            let diff = rms(&x.iter().zip(&y).map(|(a, b)| a - b).collect::<Vec<f32>>());
            assert!(diff > 0.005, "{kind:?} does something ({diff})");
        }
    }

    #[test]
    fn off_fades_out_and_the_dry_signal_returns() {
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Trans, 1.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(20000, 440.0);
        rack.process(0, &mut x, 48000);
        rack.units[0].params.slots[0].on = false;
        let dry = tone(4800, 440.0);
        let mut y = dry.clone();
        rack.process(0, &mut y, 48000);
        assert!(y[2000..].iter().zip(&dry[2000..]).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn release_fx_take_over_while_held_and_let_go() {
        for kind in 0..3u8 {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params.assign = [true, false, false, false, false];
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            let mut hist = tone(48000, 440.0);
            rack.process(0, &mut hist, 48000);
            rack.units[0].params.release = Some((kind, 1.0));
            let mut x = tone(48000, 440.0);
            rack.process(0, &mut x, 48000);
            let peak = x.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!(peak.is_finite() && peak < 2.0);
            if kind != 1 {
                // The brake and the back spin have stopped after their time (1 beat = 0.5 s).
                assert!(rms(&x[30000 * 2..]) < 0.01, "release {kind} ends in silence");
            }
            rack.units[0].params.release = None;
            let dry = tone(4800, 440.0);
            let mut y = dry.clone();
            rack.process(0, &mut y, 48000);
            assert!(y[2000..].iter().zip(&dry[2000..]).all(|(a, b)| (a - b).abs() < 1e-5), "release {kind}: the track is back");
        }
    }

    #[test]
    fn tap_tempo_and_beat_text() {
        let mut t = Tap::new();
        assert_eq!(t.tap(0.0), None);
        let mut bpm = None;
        for i in 1..6 {
            bpm = t.tap(i as f64 * 0.5);
        }
        assert!((bpm.unwrap() - 120.0).abs() < 0.01);
        // A pause of over 1.5 s starts again.
        assert_eq!(t.tap(10.0), None);
        assert!((t.tap(10.4).unwrap() - 150.0).abs() < 0.01);
        assert_eq!(beat_text(0.75, false), "3/4");
        assert_eq!(beat_text(0.0625, false), "1/16");
        assert_eq!(beat_text(4.0, false), "4");
        assert_eq!(beat_text(50.0, true), "50%");
        assert_eq!(beat_table(Kind::Filter).0[beat_table(Kind::Filter).1], 4.0);
    }
}
