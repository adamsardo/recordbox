//! BEAT FX and RELEASE FX of the Performance mixer: two FX units (FX1, FX2), each either in
//! multi mode (three effect slots) or single mode (one effect with five parameters),
//! assignable to decks 1 to 4, the sampler or the master.
//!
//! What follows rekordbox 7.2.19 (`checks/re-7.2.19/mixer-fx-recording.md`, section 2): the
//! effect lists, their order and engine ids, the single-mode parameter names, the beat tables
//! and their defaults, the insert points (a deck's effects after its channel fader and the
//! crossfader, the master's before the master limiter), a separate effect state per assigned
//! channel, the 3.33 ms smoothing of on/off, the echo family's LEVEL/DEPTH law and feedback
//! law, MT DELAY's mix law, STRETCH's speed law, TRANS as a square gate, FLANGER's sweep
//! range, PHASER's sweep range, ROLL looping the last beat from a 15 s history, the reverb's
//! structure (five damped combs into two all-passes, feedback 0.9265 to 0.9986), the RELEASE
//! FX time rules, and Beat FX quantize (start and LFO phase on the deck's beat grid).
//! What does not: rekordbox's filter tables, reverb tunings, pitch shifter and most
//! per-effect mix laws were not traced; those parts are recordbox's own and are marked below.

use parking_lot::Mutex;

/// Every Beat FX: rekordbox's 35 multi-mode effects and PITCH ECHO (single mode only, where
/// multi mode has UP ECHO and DOWN ECHO).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Delay,
    Echo,
    Spiral,
    Reverb,
    RevDelay,
    MtDelay,
    PitchEcho,
    UpEcho,
    DownEcho,
    Trans,
    Pan,
    Filter,
    Flanger,
    Phaser,
    SlipRoll,
    Roll,
    RevRoll,
    Robot,
    Pitch,
    EnigmaJet,
    MobiusSaw,
    MobiusTri,
    LowCutEcho,
    PingPong,
    Helix,
    VinylBrake,
    Stretch,
    BpfEcho,
    Noise,
    SpiralUp,
    ReverbUp,
    HpfEcho,
    LpfEcho,
    CrushEcho,
    SpiralDown,
    ReverbDown,
}

/// Beat tables of rekordbox (spec 2.4).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Table {
    B,
    C,
    D,
    /// Signed beats.
    S,
    /// Percent (REVERB).
    P1,
    /// Percent (ROBOT, PITCH).
    P2,
    /// No beat control: `<` `>` are disabled.
    None,
}

pub struct Info {
    pub kind: Kind,
    pub name: &'static str,
    /// rekordbox's engine ids (`SINGLE_FX_INDEX_LIST`, `MULTI_FX_INDEX_LIST`); -1 = not in
    /// that mode's list.
    pub single_id: i8,
    pub multi_id: i8,
    /// Single-mode labels: knob 1, knob 2, button 1, button 2 ("" = no such control).
    pub labels: [&'static str; 4],
    pub table: Table,
    /// Default index into the table.
    pub def: usize,
}

const fn info(kind: Kind, name: &'static str, single_id: i8, multi_id: i8, labels: [&'static str; 4], table: Table, def: usize) -> Info {
    Info { kind, name, single_id, multi_id, labels, table, def }
}

const CUTS: [&str; 2] = ["LOW CUT", "HI CUT"];
const RF: [&str; 2] = ["REV", "FREEZE"];
const TP: [&str; 2] = ["TONE", "PHASE"];

/// Everything in rekordbox's list order (PITCH ECHO sits where single mode has it).
pub const INFO: [Info; 36] = [
    info(Kind::Delay, "DELAY", 0, 0, ["SWING", "LR RATIO", CUTS[0], CUTS[1]], Table::B, 5),
    info(Kind::Echo, "ECHO", 1, 1, ["FEEDBACK", "FILTER", RF[0], RF[1]], Table::B, 5),
    info(Kind::Spiral, "SPIRAL", 3, 3, ["FEEDBACK", "PITCH", CUTS[0], CUTS[1]], Table::B, 5),
    info(Kind::Reverb, "REVERB", 4, 4, ["ROOM SIZE", "DECAY", CUTS[0], CUTS[1]], Table::P1, 3),
    info(Kind::RevDelay, "REV DELAY", 2, 2, ["SWING", "LR RATIO", CUTS[0], CUTS[1]], Table::B, 5),
    info(Kind::MtDelay, "MT DELAY", 13, 13, ["SWING", "TAP NUM", CUTS[0], CUTS[1]], Table::B, 5),
    info(Kind::PitchEcho, "PITCH ECHO", 14, -1, ["FEEDBACK", "PITCH", RF[0], RF[1]], Table::B, 5),
    info(Kind::UpEcho, "UP ECHO", -1, 14, ["", "", "", ""], Table::B, 5),
    info(Kind::DownEcho, "DOWN ECHO", -1, 15, ["", "", "", ""], Table::B, 5),
    info(Kind::Trans, "TRANS", 5, 5, ["DUTY", "SWING", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::Pan, "PAN", 15, 16, ["LFO AMP", "LFO", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::Filter, "FILTER", 6, 6, ["RESONANCE", "LFO AMP", TP[0], TP[1]], Table::D, 6),
    info(Kind::Flanger, "FLANGER", 7, 7, ["FEEDBACK", "LFO AMP", TP[0], TP[1]], Table::D, 6),
    info(Kind::Phaser, "PHASER", 8, 8, ["FEEDBACK", "STAGE", TP[0], TP[1]], Table::D, 6),
    info(Kind::SlipRoll, "SLIP ROLL", 9, 9, ["DUTY", "SWING", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::Roll, "ROLL", 10, 10, ["DUTY", "SWING", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::RevRoll, "REV ROLL", 11, 12, ["DUTY", "SWING", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::Robot, "ROBOT", 12, 11, ["PITCH", "MOD", CUTS[0], CUTS[1]], Table::P2, 3),
    info(Kind::Pitch, "PITCH", 16, 17, ["PITCH", "", CUTS[0], CUTS[1]], Table::P2, 0),
    info(Kind::EnigmaJet, "ENIGMA JET", 26, 27, ["FEEDBACK", "MOD", RF[0], RF[1]], Table::S, 17),
    info(Kind::MobiusSaw, "MOBIUS SAW", 27, 28, ["ECHO LEVEL", "MOD", RF[0], RF[1]], Table::S, 17),
    info(Kind::MobiusTri, "MOBIUS TRI", 28, 29, ["ECHO LEVEL", "MOD", RF[0], RF[1]], Table::S, 17),
    info(Kind::LowCutEcho, "LOW CUT ECHO", 29, 30, ["FEEDBACK", "SWING", RF[0], RF[1]], Table::B, 5),
    info(Kind::PingPong, "PING PONG", 30, 31, ["FEEDBACK", "FILTER", RF[0], RF[1]], Table::B, 5),
    info(Kind::Helix, "HELIX", 31, 32, ["TIME", "SPEED", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::VinylBrake, "VINYL BRAKE", 32, 33, ["REPEAT", "ECHO", "ON", "BPF"], Table::C, 6),
    info(Kind::Stretch, "STRETCH", 33, 34, ["LOW / HI CUT", "", "REPITCH", "BPF"], Table::C, 5),
    info(Kind::BpfEcho, "BPF ECHO", 24, 25, ["ECHO MIX", "MOD", RF[0], RF[1]], Table::None, 0),
    info(Kind::Noise, "NOISE", 17, 18, ["VOL", "MOD", "CRUSH", "FREEZE"], Table::None, 0),
    info(Kind::SpiralUp, "SPIRAL UP", 22, 23, ["ECHO MIX", "PITCH", CUTS[0], CUTS[1]], Table::None, 0),
    info(Kind::ReverbUp, "REVERB UP", 18, 19, ["TIME", "HPF", CUTS[0], CUTS[1]], Table::None, 0),
    info(Kind::HpfEcho, "HPF ECHO", 20, 21, ["ECHO MIX", "MOD", RF[0], RF[1]], Table::None, 0),
    info(Kind::LpfEcho, "LPF ECHO", 21, 22, ["ECHO MIX", "MOD", RF[0], RF[1]], Table::None, 0),
    info(Kind::CrushEcho, "CRUSH ECHO", 25, 26, ["ECHO MIX", "MOD", RF[0], RF[1]], Table::None, 0),
    info(Kind::SpiralDown, "SPIRAL DOWN", 23, 24, ["ECHO MIX", "PITCH", CUTS[0], CUTS[1]], Table::None, 0),
    info(Kind::ReverbDown, "REVERB DOWN", 19, 20, ["TIME", "LPF", CUTS[0], CUTS[1]], Table::None, 0),
];

pub fn info_of(k: Kind) -> &'static Info {
    INFO.iter().find(|i| i.kind == k).unwrap()
}

pub fn name(k: Kind) -> &'static str {
    info_of(k).name
}

/// The pop-up list of a mode, in rekordbox's order (35 in multi mode, 34 in single mode).
pub fn list(single: bool) -> impl Iterator<Item = &'static Info> {
    INFO.iter().filter(move |i| if single { i.single_id >= 0 } else { i.multi_id >= 0 })
}

/// An effect from rekordbox's engine id (what `FxUnitSettings` stores).
pub fn from_id(id: i64, single: bool) -> Option<Kind> {
    list(single).find(|i| (if single { i.single_id } else { i.multi_id }) as i64 == id).map(|i| i.kind)
}

/// The values the `<` `>` buttons step through for an effect, and where they start:
/// (values, default index, percent instead of beats). Empty = no beat control.
pub fn beat_table(k: Kind) -> (&'static [f32], usize, bool) {
    const B: [f32; 10] = [0.0625, 0.125, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
    const C: [f32; 9] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
    const D: [f32; 11] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    const S: [f32; 22] = [-64.0, -32.0, -16.0, -8.0, -4.0, -2.0, -1.0, -0.5, -0.25, -0.125, -0.0625, 0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    const P1: [f32; 7] = [1.0, 10.0, 25.0, 50.0, 75.0, 90.0, 100.0];
    const P2: [f32; 7] = [-50.0, -37.0, -25.0, 0.0, 26.0, 50.0, 100.0];
    let i = info_of(k);
    match i.table {
        Table::B => (&B, i.def, false),
        Table::C => (&C, i.def, false),
        Table::D => (&D, i.def, false),
        Table::S => (&S, i.def, false),
        Table::P1 => (&P1, i.def, true),
        Table::P2 => (&P2, i.def, true),
        Table::None => (&[], 0, false),
    }
}

/// The value an effect starts with (0 for the effects without a beat control).
pub fn default_value(k: Kind) -> f32 {
    let (t, d, _) = beat_table(k);
    t.get(d).copied().unwrap_or(0.0)
}

/// A beat value as rekordbox prints it: whole beats as `N`, fractions as `n/d`, percent `N%`.
pub fn beat_text(v: f32, percent: bool) -> String {
    if percent {
        return format!("{v:.0}%");
    }
    let a = v.abs();
    let s = if a >= 1.0 {
        format!("{a:.0}")
    } else if (a - 0.75).abs() < 1e-6 {
        "3/4".into()
    } else {
        format!("1/{:.0}", 1.0 / a)
    };
    if v < 0.0 { format!("-{s}") } else { s }
}

/// The text of an effect's value: the signed table prints `+` too.
pub fn value_text(k: Kind, v: f32) -> String {
    let i = info_of(k);
    match i.table {
        Table::None => "-".into(),
        Table::S if v > 0.0 => format!("+{}", beat_text(v, false)),
        t => beat_text(v, matches!(t, Table::P1 | Table::P2)),
    }
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
    /// Beats (or percent for REVERB, ROBOT and PITCH).
    pub value: f32,
    /// Single mode's knob 1 and knob 2 (0..1, 0.5 in multi mode).
    pub knob: [f32; 2],
    /// Single mode's button 1 and button 2 (off in multi mode).
    pub btn: [bool; 2],
}

impl SlotParams {
    pub fn new(kind: Kind) -> SlotParams {
        SlotParams { kind, on: false, level: 0.5, value: default_value(kind), knob: [0.5; 2], btn: [false; 2] }
    }
}

#[derive(Clone, Copy)]
pub struct UnitParams {
    /// Decks 1 to 4, the master, the sampler.
    pub assign: [bool; CHANNELS],
    pub slots: [SlotParams; 3],
    /// Single mode: `one` is the unit's effect and `slots` are out.
    pub single: bool,
    pub one: SlotParams,
    /// RELEASE FX held: which one and its beats.
    pub release: Option<(u8, f32)>,
    /// Tempo each channel's effects run at (BPM).
    pub bpm: [f32; CHANNELS],
    /// Beat FX quantize per channel (the deck's Q): effects and their LFOs start on the grid.
    pub quantize: [bool; CHANNELS],
}

impl UnitParams {
    pub fn new() -> UnitParams {
        // rekordbox's default slots: DELAY, ECHO, SPIRAL; single mode starts on DELAY.
        UnitParams { assign: [false; CHANNELS], slots: [SlotParams::new(Kind::Delay), SlotParams::new(Kind::Echo), SlotParams::new(Kind::Spiral)], single: false, one: SlotParams::new(Kind::Delay), release: None, bpm: [120.0; CHANNELS], quantize: [true; CHANNELS] }
    }
}

/// rekordbox's feedback law for the echo family's FEEDBACK knob: the left half runs from 0
/// to the centre value, the right half from there to 1.
pub fn fb_law(k: f32, centre: f32) -> f32 {
    let k = k.clamp(0.0, 1.0);
    if k < 0.5 { 2.0 * centre * k } else { centre + (1.0 - centre) * 2.0 * (k - 0.5) }
}

/// The echo family's LEVEL/DEPTH law (rekordbox): (dry, wet).
pub fn echo_mix(level: f32) -> (f32, f32) {
    if level < 0.75 { (1.0, 1.3333 * level) } else { (4.0 - 4.0 * level, 1.0) }
}

/// MT DELAY's LEVEL/DEPTH law (rekordbox, `GroupMultiTapDelay::calculateMixRatio`): (dry, wet).
pub fn mt_mix(level: f32) -> (f32, f32) {
    if level < 0.5 { (1.0, 1.4 * level) } else { (1.3 - 0.6 * level, 0.7) }
}

/// STRETCH's playing speed from LEVEL/DEPTH (rekordbox, `StretchCore::processEffectInternal`).
pub fn stretch_speed(level: f32) -> f32 {
    if level >= 0.5 { 0.75 - 0.5 * level } else { 1.0 - level }
}

/// PHASER's number of all-pass stages for the STAGE knob: 2 at the left end, 6 at the centre,
/// 24 at the right end (rekordbox's range is 2..60; its knob law was not traced).
pub fn phaser_stages(k: f32) -> usize {
    if k <= 0.5 { 2 + (k.max(0.0) * 8.0).round() as usize } else { 6 + ((k.min(1.0) - 0.5) * 36.0).round() as usize }
}

const MAX_RATE: usize = 48000;
/// Longest delay (4 s) and the history kept for ROLL and RELEASE FX (15 s), in frames.
const DELAY_LEN: usize = 4 * MAX_RATE;
const RING_LEN: usize = 15 * MAX_RATE;
/// The pitch shifter's line and window (frames).
const PS_LEN: usize = 4096;
const PS_WIN: f32 = 2048.0;
/// On/off and gate smoothing, as rekordbox's 3.3333 ms.
const SMOOTH_SECS: f32 = 0.0033333;
const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::TAU;

/// A state-variable filter step: (low, band, high). `g` = tan(pi fc / rate), `k` = 1/Q.
#[inline]
fn svf(s: &mut [f32; 2], x: f32, g: f32, k: f32) -> (f32, f32, f32) {
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    let v3 = x - s[1];
    let v1 = a1 * s[0] + a2 * v3;
    let v2 = s[1] + a2 * s[0] + g * a2 * v3;
    s[0] = 2.0 * v1 - s[0];
    s[1] = 2.0 * v2 - s[1];
    (v2, v1, x - k * v1 - v2)
}

/// Through up to 1.5, then rounded off toward 2.5.
#[inline]
fn sat(x: f32) -> f32 {
    let a = x.abs();
    if a <= 1.5 { x } else { x.signum() * (1.5 + (a - 1.5) / (1.0 + (a - 1.5))) }
}

#[inline]
fn svf_g(fc: f32, rate: f32) -> f32 {
    (PI * (fc / rate).clamp(0.0002, 0.45)).tan()
}

/// What the echo core does this block.
#[derive(Clone, Copy)]
struct EchoCfg {
    /// Left and right delay as multiples of the slot's time.
    ml: f32,
    mr: f32,
    fb: f32,
    rev: bool,
    freeze: bool,
    /// Pitch ratio of each repeat (1 = none).
    ratio: f32,
    /// Filter on the echo path: 0 none, 1 low-pass, 2 high-pass.
    filt: u8,
    g: f32,
    /// The right channel's echo feeds the left and back (PING PONG).
    cross: bool,
    /// More than 0: that many taps, an eighth of the time apart, and no feedback (MT DELAY).
    taps: u8,
}

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
    /// Band split (LOW CUT / HI CUT), filter states, all-pass states.
    split: [[f32; 2]; 2],
    f1: [[f32; 2]; 2],
    f2: [[f32; 2]; 2],
    lp: [f32; 2],
    ap: [[f32; 24]; 2],
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
    /// Odd turn of the LFO (SWING).
    odd: bool,
    /// Pitch shifter: its line, write position and window phase.
    ps: Vec<f32>,
    ps_w: usize,
    ps_ph: f32,
    /// Reverse echo: frames into the current turn.
    rev_k: f32,
    seed: u32,
    hold: [f32; 2],
    hold_n: f32,
    /// VINYL BRAKE / STRETCH: how far behind the live sound playback is (frames), and frames
    /// into the current turn.
    lag: f64,
    t: f32,
    /// MOBIUS: oscillator phases and the input's envelope.
    osc: [f32; 6],
    env: f32,
}

impl Slot {
    fn new() -> Slot {
        // Reverb line lengths in frames at 48 kHz (recordbox's own tunings: mutually prime,
        // 25 to 37 ms combs, 5 and 1.7 ms all-passes; the right channel 23 frames longer).
        // The combs are made half as long again, for ROOM SIZE.
        let comb_len = [1215usize, 1293, 1390, 1476, 1548];
        let ap_len = [240usize, 82];
        let mut combs = Vec::new();
        for ch in 0..2 {
            for l in comb_len {
                combs.push(vec![0.0; (l + ch * 23) * 3 / 2 + 1]);
            }
        }
        let mut aps = Vec::new();
        for ch in 0..2 {
            for l in ap_len {
                aps.push(vec![0.0; l + ch * 23]);
            }
        }
        Slot { kind: Kind::Delay, delay: vec![0.0; DELAY_LEN * 2], w: 0, time: 0.0, gate: 0.0, lfo: 0.0, split: [[0.0; 2]; 2], f1: [[0.0; 2]; 2], f2: [[0.0; 2]; 2], lp: [0.0; 2], ap: [[0.0; 24]; 2], fb: [0.0; 2], combs, comb_pos: [0; 10], comb_lp: [0.0; 10], aps, ap_pos: [0; 4], roll: None, was_on: false, odd: false, ps: vec![0.0; PS_LEN * 2], ps_w: 0, ps_ph: 0.0, rev_k: 0.0, seed: 0x2468_ACE1, hold: [0.0; 2], hold_n: 0.0, lag: 0.0, t: 0.0, osc: [0.0; 6], env: 0.0 }
    }

    fn clear(&mut self) {
        self.delay.iter_mut().for_each(|v| *v = 0.0);
        self.ps.iter_mut().for_each(|v| *v = 0.0);
        self.combs.iter_mut().for_each(|c| c.iter_mut().for_each(|v| *v = 0.0));
        self.aps.iter_mut().for_each(|c| c.iter_mut().for_each(|v| *v = 0.0));
        self.split = [[0.0; 2]; 2];
        self.f1 = [[0.0; 2]; 2];
        self.f2 = [[0.0; 2]; 2];
        self.lp = [0.0; 2];
        self.ap = [[0.0; 24]; 2];
        self.fb = [0.0; 2];
        self.comb_lp = [0.0; 10];
        self.comb_pos = [0; 10];
        self.time = 0.0;
        self.lfo = 0.0;
        self.roll = None;
        self.odd = false;
        self.rev_k = 0.0;
        self.hold = [0.0; 2];
        self.hold_n = 0.0;
        self.lag = 0.0;
        self.t = 0.0;
        self.osc = [0.0; 6];
        self.env = 0.0;
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

    #[inline]
    fn push(&mut self, l: f32, r: f32) {
        // (Rounded off above 1.5, never past 2.5, so a feedback of 1 cannot run away.)
        self.delay[self.w * 2] = sat(l);
        self.delay[self.w * 2 + 1] = sat(r);
        self.w = (self.w + 1) % DELAY_LEN;
    }

    /// Pitch shift by `ratio`: two windows over a short delay line, half a window apart
    /// (recordbox's own shifter; rekordbox's `rbxfrm::PitchShifter` was not traced).
    #[inline]
    fn shift(&mut self, l: f32, r: f32, ratio: f32) -> (f32, f32) {
        self.ps[self.ps_w * 2] = l;
        self.ps[self.ps_w * 2 + 1] = r;
        self.ps_ph += (1.0 - ratio.clamp(0.25, 4.0)) / PS_WIN;
        self.ps_ph -= self.ps_ph.floor();
        let mut out = (0.0, 0.0);
        for k in 0..2 {
            let ph = (self.ps_ph + k as f32 * 0.5).fract();
            let d = 1.0 + ph * PS_WIN;
            let i = d as usize;
            let t = d - i as f32;
            let g = 0.5 - 0.5 * (TAU * ph).cos();
            let (a, b) = ((self.ps_w + PS_LEN - i) % PS_LEN, (self.ps_w + PS_LEN - i - 1) % PS_LEN);
            out.0 += (self.ps[a * 2] + (self.ps[b * 2] - self.ps[a * 2]) * t) * g;
            out.1 += (self.ps[a * 2 + 1] + (self.ps[b * 2 + 1] - self.ps[a * 2 + 1]) * t) * g;
        }
        self.ps_w = (self.ps_w + 1) % PS_LEN;
        out
    }

    /// The echo core: read the repeats, colour them, write the input and the feedback.
    #[inline]
    fn echo(&mut self, il: f32, ir: f32, e: &EchoCfg, r: f32) -> (f32, f32) {
        let (mut el, mut er);
        if e.taps > 0 {
            el = 0.0;
            er = 0.0;
            for k in 1..=e.taps {
                let d = self.time * k as f32 / 8.0;
                el += self.tap(d * e.ml, 0);
                er += self.tap(d * e.mr, 1);
            }
            let g = 1.0 / (e.taps as f32).sqrt();
            el *= g;
            er *= g;
        } else if e.rev {
            // Reverse: each turn of the delay time is read backwards from where it began.
            let len = self.time.clamp(0.010 * r, (DELAY_LEN / 2 - 2) as f32);
            if self.rev_k >= len {
                self.rev_k = 0.0;
            }
            let edge = (self.rev_k.min(len - self.rev_k) / (SMOOTH_SECS * r)).min(1.0);
            el = self.tap(2.0 * self.rev_k + 1.0, 0) * edge;
            er = self.tap(2.0 * self.rev_k + 1.0, 1) * edge;
            self.rev_k += 1.0;
        } else {
            el = self.tap(self.time * e.ml, 0);
            er = self.tap(self.time * e.mr, 1);
        }
        if e.filt != 0 {
            let a = svf(&mut self.f1[0], el, e.g, 1.4);
            let b = svf(&mut self.f1[1], er, e.g, 1.4);
            (el, er) = if e.filt == 1 { (a.0, b.0) } else { (a.2, b.2) };
        }
        if (e.ratio - 1.0).abs() > 1e-4 {
            (el, er) = self.shift(el, er, e.ratio);
        }
        let (il, ir) = if e.freeze { (0.0, 0.0) } else { (il, ir) };
        if e.cross {
            self.push(il + ir + er * e.fb, el * e.fb);
        } else {
            self.push(il + el * e.fb, ir + er * e.fb);
        }
        (el, er)
    }

    /// Five damped combs in parallel into two all-passes in series (rekordbox's structure).
    #[inline]
    fn reverb(&mut self, l: f32, r: f32, fbk: f32, size: f32) -> (f32, f32) {
        let mut out = [0.0f32; 2];
        for c in 0..2 {
            let x = if c == 0 { l } else { r } * 0.2;
            let mut acc = 0.0;
            for k in 0..5 {
                let i = c * 5 + k;
                let line = &mut self.combs[i];
                let len = ((line.len() as f32 * size * 0.6667) as usize).clamp(16, line.len());
                let pos = self.comb_pos[i] % len;
                let y = line[pos];
                self.comb_lp[i] = y * 0.75 + self.comb_lp[i] * 0.25;
                line[pos] = x + self.comb_lp[i] * fbk;
                self.comb_pos[i] = (pos + 1) % len;
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
        (out[0], out[1])
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// `beat`: where the channel's deck is on its beat grid at the start of `buf` (beats),
    /// when Beat FX quantize applies.
    fn process(&mut self, p: &SlotParams, bpm: f32, buf: &mut [f32], rate: u32, ring: &Ring, beat: Option<f64>) {
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
        use Kind::*;
        let kind = self.kind;
        let r = rate as f32;
        let bpm = bpm.clamp(20.0, 1000.0);
        let beat_secs = 60.0 / bpm;
        let (table, _, percent) = beat_table(kind);
        // The effect's time: its beat value, or (no beat control) three quarters of a beat.
        let beats = if table.is_empty() || percent { 0.75 } else { p.value.abs().max(0.0625) };
        let down = !percent && p.value < 0.0;
        let secs = (beats * beat_secs).clamp(0.010, 32.0);
        let gate_k = 1.0 / (SMOOTH_SECS * r);
        let level = p.level.clamp(0.0, 1.0);
        let (k1, k2) = (p.knob[0].clamp(0.0, 1.0), p.knob[1].clamp(0.0, 1.0));
        let (b1, b2) = (p.btn[0], p.btn[1]);
        let (echo_dry, echo_wet) = echo_mix(level);
        let frames = buf.len() / 2;
        let starting = p.on && !self.was_on;
        self.was_on = p.on;
        let lfo_step = 1.0 / (secs * r);
        // Beat FX quantize: the LFO's phase is the deck's place in the beat value.
        let grid = beat.map(|b| (b / beats as f64).rem_euclid(1.0) as f32);
        if let Some(want) = grid {
            let err = (want - self.lfo + 1.5).rem_euclid(1.0) - 0.5;
            if starting || err.abs() > 0.02 {
                self.lfo = want;
                self.odd = ((beat.unwrap() / beats as f64).floor() as i64) & 1 == 1;
            } else {
                self.lfo = (self.lfo + err * 0.2).rem_euclid(1.0);
            }
        }
        if starting {
            if grid.is_none() {
                self.lfo = 0.0;
                self.odd = false;
            }
            self.lag = 0.0;
            self.t = 0.0;
            self.rev_k = 0.0;
            if matches!(kind, Roll | RevRoll | SlipRoll) {
                // Catch the last beat value (10 ms to 4 s) from the history and loop it. With
                // quantize the loop ends on the grid's last step (the beat value, or a beat
                // for longer ones) and playback joins it where the deck is now.
                let len = (((secs.clamp(0.010, 4.0)) * r) as usize).clamp(1, RING_LEN / 2);
                let since = match beat {
                    Some(b) => {
                        let q = beats.min(1.0) as f64;
                        ((b / q).rem_euclid(1.0) * q * beat_secs as f64 * r as f64) as usize
                    }
                    None => 0,
                };
                let back = if beat.is_some() { since.min(RING_LEN / 4) + frames } else { 0 };
                self.roll = Some(((ring.w + 2 * RING_LEN - len - back) % RING_LEN, len, (since % len) as f32));
            }
        }
        // Time the delay line runs at.
        let echo_secs = match kind {
            MobiusSaw | MobiusTri | VinylBrake | Noise => 0.75 * beat_secs,
            Helix => secs * (0.5 + k1),
            _ => secs,
        };
        let want_time = (echo_secs.min(4.0) * r).min((DELAY_LEN - 2) as f32);
        if self.time <= 0.0 {
            self.time = want_time;
        }
        // (SPIRAL's time slides slowly, so a new beat value bends the repeats' pitch.)
        let glide = 1.0 / (if matches!(kind, Spiral | SpiralUp | SpiralDown) { 0.3 } else { 0.05 } * r);

        // The echo core's settings.
        let swing = |k: f32| 1.0 + (k - 0.5) * 0.6667;
        let mut e = EchoCfg { ml: 1.0, mr: 1.0, fb: 0.0, rev: false, freeze: false, ratio: 1.0, filt: 0, g: 0.0, cross: false, taps: 0 };
        // FILTER knob of the echoes: centre = through, left low-pass, right high-pass
        // (recordbox's own curves; rekordbox's `InstFxLPF/HPF` were not traced).
        let echo_filter = |e: &mut EchoCfg, k: f32| {
            if k < 0.48 {
                e.filt = 1;
                e.g = svf_g(20000.0 * (200.0f32 / 20000.0).powf((0.5 - k) * 2.0), r);
            } else if k > 0.52 {
                e.filt = 2;
                e.g = svf_g(20.0 * (8000.0f32 / 20.0).powf((k - 0.5) * 2.0), r);
            }
        };
        match kind {
            Delay | RevDelay => {
                e.ml = swing(k1) * (1.0 - (0.5 - k2).max(0.0));
                e.mr = swing(k1) * (1.0 - (k2 - 0.5).max(0.0));
                e.rev = kind == RevDelay;
            }
            Echo => {
                e.fb = fb_law(k1, 0.7);
                echo_filter(&mut e, k2);
                e.rev = b1;
                e.freeze = b2;
            }
            PitchEcho | UpEcho | DownEcho => {
                e.fb = fb_law(k1, 0.7);
                // PITCH: an octave either way at the knob's ends; UP / DOWN ECHO two semitones
                // a repeat (recordbox's own amounts).
                e.ratio = match kind {
                    UpEcho => 2f32.powf(2.0 / 12.0),
                    DownEcho => 2f32.powf(-2.0 / 12.0),
                    _ => 2f32.powf((k2 - 0.5) * 2.0),
                };
                e.rev = b1;
                e.freeze = b2;
            }
            LowCutEcho => {
                e.fb = fb_law(k1, 0.7);
                // (A 400 Hz corner: recordbox's own.)
                e.filt = 2;
                e.g = svf_g(400.0, r);
                e.ml = swing(k2);
                e.mr = swing(k2);
                e.rev = b1;
                e.freeze = b2;
            }
            PingPong => {
                e.fb = fb_law(k1, 0.7);
                e.cross = true;
                echo_filter(&mut e, k2);
                e.rev = b1;
                e.freeze = b2;
            }
            Spiral => {
                // A long-feedback echo whose repeats darken; PITCH bends each repeat by up
                // to a semitone.
                e.fb = fb_law(k1, 0.82);
                e.filt = 1;
                e.g = svf_g(3500.0, r);
                e.ratio = 2f32.powf((k2 - 0.5) / 6.0);
            }
            SpiralUp | SpiralDown => {
                e.fb = 0.55 + 0.4 * level;
                e.filt = 1;
                e.g = svf_g(4500.0, r);
                let semis = (0.25 + 1.5 * k2) * level;
                e.ratio = 2f32.powf(if kind == SpiralUp { semis } else { -semis } / 12.0);
            }
            MtDelay => {
                // Up to seven repeats an eighth of the beat value apart.
                e.taps = (1.0 + k2 * 12.0).round().clamp(1.0, 7.0) as u8;
                e.ml = swing(k1);
                e.mr = swing(k1);
            }
            BpfEcho | HpfEcho | LpfEcho | CrushEcho => {
                e.fb = 0.6;
                e.rev = b1;
                e.freeze = b2;
            }
            Noise => {
                e.fb = 0.5;
                e.freeze = b2;
            }
            MobiusSaw | MobiusTri => e.fb = 0.5,
            VinylBrake => e.fb = 0.6,
            _ => {}
        }
        // LOW CUT / HI CUT: those bands pass by the effect.
        let cuts = info_of(kind).labels[2] == "LOW CUT" && (b1 || b2);
        let (cut_lo, cut_hi) = (1.0 - (-TAU * 200.0 / r).exp(), 1.0 - (-TAU * 5000.0 / r).exp());
        // Per-effect settings that do not change inside the block.
        let dir = if down != (b1 && matches!(kind, EnigmaJet | MobiusSaw | MobiusTri)) { -1.0f32 } else { 1.0 };
        let hold_lfo = b2 && matches!(kind, EnigmaJet | MobiusSaw | MobiusTri);
        let pct_ratio = {
            // ROBOT / PITCH: the percent value, or the PITCH knob once it leaves its centre.
            let pct = if (k1 - 0.5).abs() > 0.02 { ((k1 - 0.5) * 200.0).clamp(-50.0, 100.0) } else { p.value };
            (1.0 + pct / 100.0).clamp(0.5, 2.0)
        };
        let speed = stretch_speed(level);
        let stages = phaser_stages(k2);
        let env_k = (1.0 - (-1.0 / (0.002 * r)).exp(), 1.0 - (-1.0 / (0.12 * r)).exp());
        let base = (ring.w + 2 * RING_LEN - frames) as f64;
        for n in 0..frames {
            let target = if p.on { 1.0 } else { 0.0 };
            self.gate += (target - self.gate).clamp(-gate_k, gate_k);
            let g = self.gate;
            if g <= 0.0 && !p.on {
                // Off and faded out: the rest of the buffer is the dry sound, untouched.
                break;
            }
            let (xl, xr) = (buf[n * 2], buf[n * 2 + 1]);
            // Split off the bands the effect leaves alone.
            let (mut dl, mut dr, mut tl, mut tr) = (xl, xr, 0.0, 0.0);
            if cuts {
                for (c, x) in [xl, xr].into_iter().enumerate() {
                    self.split[c][0] += (x - self.split[c][0]) * cut_lo;
                    self.split[c][1] += (x - self.split[c][1]) * cut_hi;
                    let thru = if b1 { self.split[c][0] } else { 0.0 } + if b2 { x - self.split[c][1] } else { 0.0 };
                    if c == 0 {
                        tl = thru;
                        dl = x - thru;
                    } else {
                        tr = thru;
                        dr = x - thru;
                    }
                }
            }
            self.time += (want_time - self.time) * glide;
            if !hold_lfo {
                self.lfo += lfo_step;
                if self.lfo >= 1.0 {
                    self.lfo -= 1.0;
                    self.odd = !self.odd;
                }
            }
            let tri = 1.0 - (2.0 * self.lfo - 1.0).abs(); // 0..1..0
            let m = level * g;
            let (il, ir) = (dl * target, dr * target);
            let (ol, or) = match kind {
                Delay | RevDelay | Echo | PitchEcho | UpEcho | DownEcho | LowCutEcho | PingPong | Spiral => {
                    // DELAY: one repeat. ECHO: feedback 0.7 at the knob's centre (rekordbox).
                    // The echo rings on after OFF only as long as the gate takes to close.
                    let (el, er) = self.echo(il, ir, &e, r);
                    (dl * (1.0 + (echo_dry - 1.0) * g) + el * echo_wet * g, dr * (1.0 + (echo_dry - 1.0) * g) + er * echo_wet * g)
                }
                MtDelay => {
                    let (el, er) = self.echo(il, ir, &e, r);
                    let (dry, wet) = mt_mix(level);
                    (dl * (1.0 + (dry - 1.0) * g) + el * wet * g, dr * (1.0 + (dry - 1.0) * g) + er * wet * g)
                }
                SpiralUp | SpiralDown => {
                    // The spiral with its pitch climbing (or falling) further on every repeat;
                    // the main knob brings in the echo, its feedback and the bend.
                    let (el, er) = self.echo(il, ir, &e, r);
                    let wet = (level * 2.0).min(1.0) * (0.4 + 1.2 * k1).min(1.0) * g;
                    (dl + el * wet, dr + er * wet)
                }
                Reverb => {
                    // The comb feedback runs over rekordbox's 0.9265 .. 0.9986 with the percent
                    // value; DECAY moves it, ROOM SIZE scales the comb lengths.
                    let fbk = (0.9265 + (0.9986 - 0.9265) * (p.value / 100.0).clamp(0.0, 1.0) * 0.6 + (k2 - 0.5) * 0.06).clamp(0.85, 0.9986);
                    let (rl, rr) = self.reverb(il, ir, fbk, 0.5 + k1);
                    (dl * (1.0 + (echo_dry - 1.0) * g) + rl * echo_wet * g, dr * (1.0 + (echo_dry - 1.0) * g) + rr * echo_wet * g)
                }
                ReverbUp | ReverbDown => {
                    // The sound is filtered (high-pass rising / low-pass falling with the main
                    // knob) and a reverb of it comes in; TIME is the decay.
                    let amt = (level * (0.5 + k2)).min(1.0);
                    let fc = if kind == ReverbUp { 20.0 * (6000.0f32 / 20.0).powf(amt) } else { 20000.0 * (300.0f32 / 20000.0).powf(amt) };
                    let gf = svf_g(fc, r);
                    let a = svf(&mut self.f2[0], dl, gf, 1.2);
                    let b = svf(&mut self.f2[1], dr, gf, 1.2);
                    let (fl, fr) = if kind == ReverbUp { (a.2, b.2) } else { (a.0, b.0) };
                    let x = (level * 8.0).min(1.0) * g;
                    let (ml, mr) = (dl + (fl - dl) * x, dr + (fr - dr) * x);
                    let (rl, rr) = self.reverb(ml * target, mr * target, 0.93 + 0.065 * k1, 1.0);
                    let wet = (level * 2.0).min(1.0) * 0.8 * g;
                    (ml + rl * wet, mr + rr * wet)
                }
                Trans => {
                    // A square gate once per beat value, open for DUTY of it (half at the
                    // centre, never under 5 ms), its edges smoothed over 3.33 ms; SWING moves
                    // every second opening; LEVEL/DEPTH sets how far it closes.
                    let duty = k1.clamp((0.005 / secs).min(0.5), 1.0);
                    let sw = if self.odd { (k2 - 0.5).max(0.0) * 0.5 } else { (0.5 - k2).max(0.0) * 0.5 };
                    let open = if self.lfo >= sw && self.lfo < sw + duty { 1.0 } else { 0.0 };
                    self.fb[0] += (open - self.fb[0]).clamp(-gate_k, gate_k);
                    let a = 1.0 - m * (1.0 - self.fb[0]);
                    (dl * a, dr * a)
                }
                Pan => {
                    // The sound moves left and right once per beat value. LFO AMP is how far,
                    // LFO its shape (centre a sine, right toward a square, left softer).
                    let s = (TAU * self.lfo).sin();
                    let s = if k2 > 0.5 { (s * (1.0 + (k2 - 0.5) * 20.0)).clamp(-1.0, 1.0) } else { s * (1.0 - (0.5 - k2) * 2.0 * (1.0 - s.abs())) };
                    let pan = s * m * (k1 * 2.0).min(1.0);
                    (dl * (1.0 - pan.max(0.0)), dr * (1.0 + pan.min(0.0)))
                }
                Filter => {
                    // A resonant filter whose cutoff a triangle LFO sweeps between 120 Hz and
                    // 14 kHz once per beat value (recordbox's own range and resonance;
                    // rekordbox's tables were not traced). TONE: high-pass; PHASE: the LFO
                    // turned over.
                    let t = if b2 { 1.0 - tri } else { tri };
                    let span = (k2 * 2.0).min(1.0);
                    let fc = 14000.0 * (120.0f32 / 14000.0).powf(span * (1.0 - t));
                    let gf = svf_g(fc, r);
                    let k = 1.0 / (0.7 * 12f32.powf(k1));
                    let a = svf(&mut self.f2[0], dl, gf, k);
                    let b = svf(&mut self.f2[1], dr, gf, k);
                    let (fl, fr) = if b1 { (a.2, b.2) } else { (a.0, b.0) };
                    (dl * (1.0 - m) + fl * m, dr * (1.0 - m) + fr * m)
                }
                Flanger => {
                    // A feedback comb whose delay a triangle LFO sweeps between one sample
                    // and 2.6667 ms once per beat value.
                    let t = if b2 { 1.0 - tri } else { tri };
                    let d = 1.0 + t * (k2 * 2.0).min(1.0) * (0.0026667 * r - 1.0);
                    let fbk = fb_law(k1, 0.7).min(0.9999) * if b1 { -1.0 } else { 1.0 };
                    let (el, er) = (self.tap(d, 0), self.tap(d, 1));
                    self.push(dl + el * fbk, dr + er * fbk);
                    (dl + el * m, dr + er * m)
                }
                Phaser => {
                    // First-order all-pass stages (six at the centre of STAGE), their centre
                    // swept 100 Hz .. 15 kHz.
                    let t = if b2 { 1.0 - tri } else { tri };
                    let fc = 100.0 * (15000.0f32 / 100.0).powf(t);
                    let tn = (PI * (fc / r).min(0.49)).tan();
                    let a = (tn - 1.0) / (tn + 1.0);
                    let fbk = fb_law(k1, 0.5).min(0.9999) * if b1 { -1.0 } else { 1.0 };
                    let mut o = [0.0f32; 2];
                    for (c, x) in [dl, dr].into_iter().enumerate() {
                        let mut y = x + self.fb[c] * fbk;
                        for k in 0..stages {
                            let z = self.ap[c][k];
                            let out = a * y + z;
                            self.ap[c][k] = y - a * out;
                            y = out;
                        }
                        let y = sat(y);
                        self.fb[c] = y;
                        o[c] = y;
                    }
                    (dl + o[0] * m, dr + o[1] * m)
                }
                Roll | RevRoll | SlipRoll => {
                    let (mut ol, mut or) = (dl, dr);
                    if let Some((start, len, pos)) = self.roll.as_mut() {
                        let i = *pos as usize % *len;
                        let j = if kind == RevRoll { *len - 1 - i } else { i };
                        let k = (*start + j) % RING_LEN;
                        // A few frames of fade at the loop's ends, so the splice is silent;
                        // DUTY under its centre cuts the end of every turn short.
                        let edge = (i.min(*len - 1 - i) as f32 / (0.002 * r)).min(1.0);
                        let duty = if k1 < 0.5 && (i as f32) > *len as f32 * (k1 * 2.0).max(0.05) { 0.0 } else { 1.0 };
                        self.fb[0] += (duty - self.fb[0]).clamp(-gate_k, gate_k);
                        ol = ring.buf[k * 2] * edge * self.fb[0];
                        or = ring.buf[k * 2 + 1] * edge * self.fb[0];
                        *pos += 1.0;
                        if *pos as usize >= *len {
                            *pos = 0.0;
                            if kind == SlipRoll && p.on {
                                // SLIP ROLL catches the newest beat again on every turn.
                                *start = (base as usize + n + 1 + RING_LEN - (*len).min(RING_LEN - 1)) % RING_LEN;
                            }
                        }
                    }
                    (dl * (1.0 - m) + ol * m, dr * (1.0 - m) + or * m)
                }
                Robot => {
                    // The sound shifted in pitch and run through a short comb, which gives the
                    // metallic voice (recordbox's own; rekordbox's `RobotCore` was not traced).
                    // Mix as rekordbox: the dry sound is gone from half way up.
                    let (pl, pr) = self.shift(dl, dr, pct_ratio);
                    let d = 0.004 * r;
                    let (cl, cr) = (self.tap(d, 0), self.tap(d, 1));
                    let fbk = 0.9 * k2;
                    self.push(pl + cl * fbk, pr + cr * fbk);
                    let (wl, wr) = ((pl + cl * fbk) * 0.7, (pr + cr * fbk) * 0.7);
                    let (dry, wet) = ((1.0 - 2.0 * level).max(0.0), (2.0 * level).min(1.0));
                    (dl * (1.0 + (dry - 1.0) * g) + wl * wet * g, dr * (1.0 + (dry - 1.0) * g) + wr * wet * g)
                }
                Pitch => {
                    let (pl, pr) = self.shift(dl, dr, pct_ratio);
                    (dl * (1.0 - m) + pl * m, dr * (1.0 - m) + pr * m)
                }
                EnigmaJet => {
                    // A flanger that seems to rise (or fall) without end: three combs, their
                    // delays moving the same way a third of a turn apart, each faded in and
                    // out at the ends of its travel (recordbox's own).
                    let fbk = 0.9 * fb_law(k1, 0.66);
                    let (mut el, mut er) = (0.0, 0.0);
                    for v in 0..3 {
                        let ph = (self.lfo * dir + v as f32 / 3.0).rem_euclid(1.0);
                        let d = 0.0003 * r * (0.008f32 / 0.0003).powf(1.0 - ph);
                        let w = 0.5 - 0.5 * (TAU * ph).cos();
                        el += self.tap(d, 0) * w;
                        er += self.tap(d, 1) * w;
                    }
                    self.push(dl + el * fbk * 0.6, dr + er * fbk * 0.6);
                    let depth = (0.5 + k2).min(1.0);
                    (dl + el * m * depth, dr + er * m * depth)
                }
                MobiusSaw | MobiusTri => {
                    // A tone that seems to rise (or fall) without end: six oscillators an
                    // octave apart moving one octave per beat value, each loudest in the
                    // middle of the range. It follows the input's level, so silence stays
                    // silent (recordbox's own).
                    let pos = (self.lfo * dir).rem_euclid(1.0);
                    let step = pos - self.lp[0];
                    if step < -0.5 {
                        self.osc.rotate_right(1);
                    } else if step > 0.5 {
                        self.osc.rotate_left(1);
                    }
                    self.lp[0] = pos;
                    let mut s = 0.0;
                    for v in 0..6 {
                        let x = v as f32 + pos;
                        self.osc[v] = (self.osc[v] + 55.0 * 2f32.powf(x) / r).fract();
                        let w = 0.5 - 0.5 * (TAU * x / 6.0).cos();
                        let ph = self.osc[v];
                        s += w * if kind == MobiusSaw { 2.0 * ph - 1.0 } else { 1.0 - 4.0 * (ph - 0.5).abs() };
                    }
                    let a = (dl.abs() + dr.abs()) * 0.5;
                    self.env += (a - self.env) * if a > self.env { env_k.0 } else { env_k.1 };
                    // MOD: how bright the tone is.
                    let gf = svf_g(500.0 * 2f32.powf(k2 * 5.0), r);
                    let s = svf(&mut self.f2[0], s * 0.5 * self.env * target, gf, 1.4).0;
                    let (el, er) = self.echo(s * k1, s * k1, &e, r);
                    (dl + (s + el) * m * 2.0, dr + (s + er) * m * 2.0)
                }
                Helix => {
                    // A loop of the beat value that the input is laid over: LEVEL/DEPTH is how
                    // long it lasts, and fully up the loop is held and the input no longer
                    // enters it. SPEED plays the loop higher or lower (recordbox's own, with
                    // rekordbox's 1.38 and 1.78 as the ends).
                    let el = self.tap(self.time, 0);
                    let er = self.tap(self.time, 1);
                    let fbk = (level * 1.25).min(1.0);
                    let ing = 1.0 - ((level - 0.8) * 5.0).clamp(0.0, 1.0);
                    self.push(il * ing + el * fbk, ir * ing + er * fbk);
                    let ratio = if k2 >= 0.5 { 1.0 + (k2 - 0.5) * 2.0 * 0.78 } else { 1.0 - (0.5 - k2) * 2.0 * (1.0 - 1.0 / 1.38) };
                    let (el, er) = if (ratio - 1.0).abs() > 0.004 { self.shift(el, er, ratio) } else { (el, er) };
                    (dl + el * echo_wet * g, dr + er * echo_wet * g)
                }
                VinylBrake => {
                    // The sound slows to a stop over the beat value, like a record with the
                    // motor off. ON: it starts again and brakes again, REPEAT sets how often.
                    // ECHO: an echo of the braking sound. BPF: a band-pass on it.
                    let total = (secs.min(12.0) * r).max(1.0);
                    let x = (self.t / total).min(1.0);
                    let sp = 1.0 - x;
                    self.lag += (1.0 - sp) as f64;
                    let (mut wl, mut wr) = ring.at(base + n as f64 - self.lag);
                    let a = (sp * 8.0).min(1.0) * (self.t / (SMOOTH_SECS * r)).min(1.0);
                    wl *= a;
                    wr *= a;
                    if b2 {
                        let gf = svf_g(1000.0, r);
                        wl = svf(&mut self.f2[0], wl, gf, 1.4).1 * 1.4;
                        wr = svf(&mut self.f2[1], wr, gf, 1.4).1 * 1.4;
                    }
                    self.t += 1.0;
                    if b1 && self.t >= total * (1.0 + 2.0 * k1) {
                        self.t = 0.0;
                        self.lag = 0.0;
                    }
                    let (el, er) = self.echo(wl * k2 * target, wr * k2 * target, &e, r);
                    (dl * (1.0 - m) + (wl + el) * m, dr * (1.0 - m) + (wr + er) * m)
                }
                Stretch => {
                    // Every beat value playback starts again at the live sound and runs at
                    // rekordbox's speed for LEVEL/DEPTH (1 .. 1/2 .. 1/4), so it falls behind.
                    // REPITCH: the pitch drops with the speed; otherwise it is shifted back up.
                    // The knob cuts lows (left) or highs (right); BPF is a band-pass.
                    let total = (secs.min(16.0) * r).max(1.0);
                    if self.lfo < lfo_step && self.t > 0.0 {
                        // (The LFO's turn is the grid's with quantize.)
                        self.t = 0.0;
                        self.lag = 0.0;
                    }
                    self.lag += (1.0 - speed) as f64;
                    let (mut wl, mut wr) = ring.at(base + n as f64 - self.lag);
                    if !b1 && speed < 0.999 {
                        (wl, wr) = self.shift(wl, wr, 1.0 / speed);
                    }
                    let edge = (self.t.min(total - self.t) / (SMOOTH_SECS * r)).clamp(0.0, 1.0);
                    self.t += 1.0;
                    if (k1 - 0.5).abs() > 0.02 {
                        let fc = if k1 < 0.5 { 20.0 * (2000.0f32 / 20.0).powf((0.5 - k1) * 2.0) } else { 20000.0 * (500.0f32 / 20000.0).powf((k1 - 0.5) * 2.0) };
                        let gf = svf_g(fc, r);
                        let a = svf(&mut self.f1[0], wl, gf, 1.4);
                        let b = svf(&mut self.f1[1], wr, gf, 1.4);
                        (wl, wr) = if k1 < 0.5 { (a.2, b.2) } else { (a.0, b.0) };
                    }
                    if b2 {
                        let gf = svf_g(1000.0, r);
                        wl = svf(&mut self.f2[0], wl, gf, 1.4).1 * 1.4;
                        wr = svf(&mut self.f2[1], wr, gf, 1.4).1 * 1.4;
                    }
                    (dl * (1.0 - g) + wl * edge * g, dr * (1.0 - g) + wr * edge * g)
                }
                BpfEcho | HpfEcho | LpfEcho | CrushEcho => {
                    // The main knob filters (or crushes) the sound more and more and an echo
                    // of the result comes in; MOD moves the filter with the beat (recordbox's
                    // own takes on the effects of these names).
                    let wob = 2f32.powf((TAU * self.lfo).sin() * k2 * 0.5 * level);
                    let (fl, fr) = match kind {
                        CrushEcho => {
                            let hold = (1.0 + level * level * 0.004 * r * wob).max(1.0);
                            self.hold_n += 1.0;
                            if self.hold_n >= hold {
                                self.hold_n -= hold;
                                self.hold = [dl, dr];
                            }
                            (self.hold[0], self.hold[1])
                        }
                        _ => {
                            let (fc, k) = match kind {
                                HpfEcho => (20.0 * (8000.0f32 / 20.0).powf(level), 1.2),
                                LpfEcho => (20000.0 * (200.0f32 / 20000.0).powf(level), 1.2),
                                _ => (300.0 * 10f32.powf(level), 1.0 / (0.5 + 3.0 * level)),
                            };
                            let gf = svf_g(fc * wob, r);
                            let a = svf(&mut self.f2[0], dl, gf, k);
                            let b = svf(&mut self.f2[1], dr, gf, k);
                            match kind {
                                HpfEcho => (a.2, b.2),
                                LpfEcho => (a.0, b.0),
                                _ => (a.1 * k, b.1 * k),
                            }
                        }
                    };
                    let x = (level * 8.0).min(1.0) * g;
                    let (ml, mr) = (dl + (fl - dl) * x, dr + (fr - dr) * x);
                    let (el, er) = self.echo(ml * target, mr * target, &e, r);
                    let wet = (level * 2.0).min(1.0) * k1 * g;
                    (ml + el * wet, mr + er * wet)
                }
                Noise => {
                    // White noise through a band the main knob moves up, at VOL, with an echo
                    // of the sound and the noise; CRUSH roughens the noise, MOD moves its band
                    // with the beat (recordbox's own).
                    let mut x = self.noise();
                    if b1 {
                        self.hold_n += 1.0;
                        if self.hold_n >= 12.0 {
                            self.hold_n = 0.0;
                            self.hold[0] = x;
                        }
                        x = self.hold[0];
                    }
                    let wob = 2f32.powf((TAU * self.lfo).sin() * k2);
                    let gf = svf_g(200.0 * (0.4 * r / 200.0).powf(level) * wob, r);
                    let nz = svf(&mut self.f2[0], x, gf, 0.6).1 * 0.6 * k1 * (level * 4.0).min(1.0) * 0.6 * target;
                    let (el, er) = self.echo((dl + nz) * target, (dr + nz) * target, &e, r);
                    let wet = (level * 2.0).min(1.0) * 0.4 * g;
                    (dl + nz * g + el * wet, dr + nz * g + er * wet)
                }
            };
            buf[n * 2] = ol + tl;
            buf[n * 2 + 1] = or + tr;
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

impl Ring {
    /// The history at a fractional frame index (wrapping).
    #[inline]
    fn at(&self, idx: f64) -> (f32, f32) {
        let f = idx.rem_euclid(RING_LEN as f64);
        let i = f as usize % RING_LEN;
        let j = (i + 1) % RING_LEN;
        let t = (f - f.floor()) as f32;
        (self.buf[i * 2] + (self.buf[j * 2] - self.buf[i * 2]) * t, self.buf[i * 2 + 1] + (self.buf[j * 2 + 1] - self.buf[i * 2 + 1]) * t)
    }
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
    /// The three multi-mode slots, then single mode's effect.
    slots: [Slot; 4],
    ring: Ring,
    release: Release,
}

impl Channel {
    fn new() -> Channel {
        Channel { slots: [Slot::new(), Slot::new(), Slot::new(), Slot::new()], ring: Ring { buf: vec![0.0; RING_LEN * 2], w: 0 }, release: Release { kind: 0, t: 0.0, pos: 0.0, start: 0, gate: 0.0, held: false } }
    }

    fn process(&mut self, p: &UnitParams, ch: usize, buf: &mut [f32], rate: u32, beat: Option<f64>) {
        let frames = buf.len() / 2;
        let r = rate as f32;
        let holding = p.release.is_some();
        // While a RELEASE FX is held the unit's Beat FX are out (rekordbox cancels them).
        if !holding && self.release.gate <= 0.0 {
            for (i, s) in self.slots.iter_mut().enumerate() {
                // The mode that is not shown is switched off (and fades out).
                let mut sp = if i < 3 { p.slots[i] } else { p.one };
                if (i < 3) == p.single {
                    sp.on = false;
                }
                s.process(&sp, p.bpm[ch], buf, rate, &self.ring, beat);
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
/// The sampler deck's channel index (FX assign "S").
pub const SAMPLER: usize = 5;
pub const CHANNELS: usize = 6;

pub struct Unit {
    pub params: UnitParams,
    /// One effect state per channel, made (by the interface, not the audio thread) the first
    /// time the unit is assigned to that channel: each holds about 12 MB of delay lines.
    channels: [Option<Box<Channel>>; CHANNELS],
}

impl Unit {
    /// Make the effect states of the channels this unit is assigned to.
    pub fn prepare(&mut self, assign: &[bool; CHANNELS]) {
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
    /// CFX on the sampler deck (kind, COLOR, PARAM); COLOR 0.5 = off.
    pub sampler_cfx: crate::cfx::Cfx,
    pub sampler_cfx_set: (u8, f32, f32),
    /// Where each channel's deck is on its beat grid at the start of the buffer being
    /// processed (beats; set by the audio thread, `None` without a grid or while stopped).
    pub beat: [Option<f64>; CHANNELS],
}

impl Rack {
    pub fn new() -> Rack {
        let unit = || Unit { params: UnitParams::new(), channels: [None, None, None, None, None, None] };
        Rack { units: [unit(), unit()], scratch: std::array::from_fn(|_| vec![0.0; 16384]), enabled: false, master_cfx: crate::cfx::Cfx::new(), master_cfx_set: (0, 0.5, 0.5), sampler_cfx: crate::cfx::Cfx::new(), sampler_cfx_set: (0, 0.5, 0.5), beat: [None; CHANNELS] }
    }

    /// Something is assigned and switched on (or held, or still fading): the effects must run.
    pub fn active(&self) -> bool {
        self.enabled && (self.units.iter().any(|u| u.params.assign.iter().any(|a| *a)) || (self.master_cfx_set.1 - 0.5).abs() > 0.011 || (self.sampler_cfx_set.1 - 0.5).abs() > 0.011)
    }

    /// The sampler's mix has to come through here (a unit is assigned to it, or its CFX is on).
    pub fn sampler_in_use(&self) -> bool {
        self.enabled && (self.units.iter().any(|u| u.params.assign[SAMPLER]) || (self.sampler_cfx_set.1 - 0.5).abs() > 0.011)
    }

    /// Run one channel's audio (stereo interleaved) through the units assigned to it.
    /// `ch`: 0..3 = decks 1 to 4, 4 = the master, 5 = the sampler.
    pub fn process(&mut self, ch: usize, buf: &mut [f32], rate: u32) {
        if ch == MASTER {
            let (k, c, p) = self.master_cfx_set;
            self.master_cfx.process(k, c, p, buf, rate);
        }
        if ch == SAMPLER && (self.sampler_cfx_set.1 - 0.5).abs() > 0.011 {
            let (k, c, p) = self.sampler_cfx_set;
            self.sampler_cfx.process(k, c, p, buf, rate);
        }
        for u in self.units.iter_mut() {
            if u.params.assign[ch] {
                let p = u.params;
                if let Some(c) = u.channels[ch].as_deref_mut() {
                    c.remember(buf);
                    let beat = if p.quantize[ch] { self.beat[ch] } else { None };
                    c.process(&p, ch, buf, rate, beat);
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
        p.assign = [true, false, false, false, false, false];
        p.slots[0] = SlotParams { on: true, level, value, ..SlotParams::new(kind) };
        p.bpm = [120.0; CHANNELS];
        p
    }

    #[test]
    fn nothing_on_leaves_the_sound_untouched() {
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params.assign = [true, true, false, false, true, false];
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
        rack.units[0].params.assign = [true, false, false, false, false, false];
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
        // Every effect, in multi mode (knobs centred) and in single mode with its knobs and
        // buttons at each extreme: bounded, audible, and after OFF the dry sound exactly.
        for info in INFO.iter() {
            let kind = info.kind;
            for (single, knob, btn) in [(false, [0.5, 0.5], [false, false]), (true, [0.0, 0.0], [true, false]), (true, [1.0, 1.0], [false, true]), (true, [1.0, 0.0], [true, true])] {
                if single && info.single_id < 0 {
                    continue;
                }
                let mut rack = Rack::new();
                rack.enabled = true;
                let mut p = unit(kind, default_value(kind), 0.8);
                if single {
                    p.single = true;
                    p.one = SlotParams { knob, btn, ..p.slots[0] };
                    p.slots[0].on = false;
                }
                rack.units[0].params = p;
                rack.units[0].prepare(&p.assign);
                // (A second of history first, as a playing deck would have.)
                rack.units[0].params.slots[0].on = false;
                rack.units[0].params.one.on = false;
                let mut pre = tone(48000, 330.0);
                rack.process(0, &mut pre, 48000);
                assert_eq!(pre, tone(48000, 330.0), "{kind:?} off is the dry sound");
                rack.units[0].params = p;
                let x = tone(48000 * 3, 330.0);
                let mut y = x.clone();
                for (i, chunk) in y.chunks_mut(512 * 2).enumerate() {
                    let _ = i;
                    rack.process(0, chunk, 48000);
                }
                let peak = y.iter().fold(0.0f32, |a, b| a.max(b.abs()));
                assert!(y.iter().all(|v| v.is_finite()) && peak < 4.0, "{kind:?} {knob:?} {btn:?} peak {peak}");
                let diff = rms(&x.iter().zip(&y).map(|(a, b)| a - b).collect::<Vec<f32>>());
                // (HELIX's low cut + high cut leave it only a band of the 330 Hz tone; the
                // echoes with FEEDBACK at zero are quiet. All must still be heard.)
                // (PAN with LFO AMP at zero does not move the sound.)
                assert!(diff > 0.002 || (kind == Kind::Pan && knob[0] == 0.0), "{kind:?} {knob:?} {btn:?} does something ({diff})");
                rack.units[0].params.slots[0].on = false;
                rack.units[0].params.one.on = false;
                let dry = tone(4800, 330.0);
                let mut z = dry.clone();
                rack.process(0, &mut z, 48000);
                assert!(z[2000..] == dry[2000..], "{kind:?} {knob:?} {btn:?}: the dry sound is back after OFF");
            }
        }
    }

    #[test]
    fn lists_ids_tables_and_laws_match_rekordbox() {
        // 35 effects in multi mode, 34 in single; engine ids are a permutation of 0..n.
        for (single, n) in [(false, 35usize), (true, 34)] {
            assert_eq!(list(single).count(), n);
            let mut ids: Vec<i8> = list(single).map(|i| if single { i.single_id } else { i.multi_id }).collect();
            ids.sort();
            assert_eq!(ids, (0..n as i8).collect::<Vec<i8>>());
            for id in 0..n as i64 {
                assert!(from_id(id, single).is_some());
            }
        }
        assert_eq!(list(false).map(|i| i.name).take(9).collect::<Vec<_>>(), ["DELAY", "ECHO", "SPIRAL", "REVERB", "REV DELAY", "MT DELAY", "UP ECHO", "DOWN ECHO", "TRANS"]);
        assert_eq!(list(true).nth(6).unwrap().name, "PITCH ECHO");
        assert_eq!(list(true).nth(24).unwrap().name, "STRETCH");
        assert_eq!(list(false).nth(26).unwrap().name, "BPF ECHO");
        assert_eq!((from_id(3, true), from_id(2, false), from_id(12, false), from_id(11, false), from_id(34, false)), (Some(Kind::Spiral), Some(Kind::RevDelay), Some(Kind::RevRoll), Some(Kind::Robot), Some(Kind::Stretch)));
        // Defaults: VINYL BRAKE 4 beats, STRETCH 2, the signed table +4, ROBOT 0 %, REVERB 50 %.
        assert_eq!(default_value(Kind::VinylBrake), 4.0);
        assert_eq!(default_value(Kind::Stretch), 2.0);
        assert_eq!((default_value(Kind::EnigmaJet), beat_table(Kind::MobiusSaw).0.len()), (4.0, 22));
        assert_eq!((default_value(Kind::Robot), default_value(Kind::Pitch), default_value(Kind::Reverb)), (0.0, -50.0, 50.0));
        assert!(beat_table(Kind::BpfEcho).0.is_empty());
        assert_eq!(value_text(Kind::EnigmaJet, 4.0), "+4");
        assert_eq!(value_text(Kind::EnigmaJet, -0.0625), "-1/16");
        assert_eq!(value_text(Kind::Robot, -37.0), "-37%");
        assert_eq!(value_text(Kind::Noise, 0.0), "-");
        assert_eq!(info_of(Kind::VinylBrake).labels, ["REPEAT", "ECHO", "ON", "BPF"]);
        // Laws: the echo's feedback (0 .. 0.7 .. 1), MT DELAY's mix, STRETCH's speed.
        assert!((fb_law(0.0, 0.7), fb_law(0.5, 0.7), fb_law(1.0, 0.7)) == (0.0, 0.7, 1.0));
        assert!((fb_law(0.25, 0.7) - 0.35).abs() < 1e-6 && (fb_law(0.75, 0.7) - 0.85).abs() < 1e-6);
        assert_eq!(mt_mix(0.0), (1.0, 0.0));
        assert!((mt_mix(0.25).1 - 0.35).abs() < 1e-6 && (mt_mix(0.5).0 - 1.0).abs() < 1e-6 && (mt_mix(1.0).0 - 0.7).abs() < 1e-6 && mt_mix(1.0).1 == 0.7);
        assert_eq!((stretch_speed(0.0), stretch_speed(0.5), stretch_speed(1.0)), (1.0, 0.5, 0.25));
        assert_eq!((echo_mix(0.75), echo_mix(1.0)), ((1.0, 1.0), (0.0, 1.0)));
    }

    /// Cycles of the strongest tone, counted from zero crossings of the left channel.
    fn crossings(x: &[f32]) -> usize {
        x.chunks_exact(2).zip(x.chunks_exact(2).skip(1)).filter(|(a, b)| a[0] <= 0.0 && b[0] > 0.0).count()
    }

    #[test]
    fn pitch_shifts_by_its_percent_value() {
        // 440 Hz at +100 % is 880 Hz, at -50 % 220 Hz (LEVEL/DEPTH fully up: the shifted sound only).
        for (pct, want) in [(100.0, 880.0), (-50.0, 220.0), (26.0, 554.4)] {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params = unit(Kind::Pitch, pct, 1.0);
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            let mut x = tone(48000 * 2, 440.0);
            rack.process(0, &mut x, 48000);
            let hz = crossings(&x[48000 * 2..]) as f32;
            assert!((hz - want).abs() < want * 0.06, "{pct}%: {hz} Hz, wanted {want}");
        }
    }

    #[test]
    fn single_mode_runs_its_one_effect_and_its_knobs_act() {
        let burst = |n: usize| -> Vec<f32> { (0..n).flat_map(|i| { let v = if i < 480 { 0.5 } else { 0.0 }; [v, v] }).collect() };
        let run = |knob: [f32; 2], btn: [bool; 2], kind: Kind| -> Vec<f32> {
            let mut rack = Rack::new();
            rack.enabled = true;
            let mut p = unit(Kind::Trans, 1.0, 1.0);
            // (The multi slot is on too: in single mode it must not act.)
            p.single = true;
            p.one = SlotParams { on: true, level: 0.75, value: 1.0, knob, btn, ..SlotParams::new(kind) };
            rack.units[0].params = p;
            rack.units[0].prepare(&p.assign);
            let mut x = burst(48000 * 2);
            rack.process(0, &mut x, 48000);
            x
        };
        let at = |x: &[f32], f: usize| rms(&x[f * 2..(f + 480) * 2]);
        // ECHO, FEEDBACK at the centre: repeats at 0.7; at the left end one repeat only; at
        // the right end they do not fade.
        let x = run([0.5, 0.5], [false, false], Kind::Echo);
        assert!(at(&x, 24000) > 0.4 && (at(&x, 48000) / at(&x, 24000) - 0.7).abs() < 0.05);
        let x = run([0.0, 0.5], [false, false], Kind::Echo);
        assert!(at(&x, 24000) > 0.4 && at(&x, 48000) < 0.01);
        let x = run([1.0, 0.5], [false, false], Kind::Echo);
        assert!((at(&x, 72000) / at(&x, 24000) - 1.0).abs() < 0.05);
        // FREEZE: nothing new enters the echo.
        let x = run([0.5, 0.5], [false, true], Kind::Echo);
        assert!(at(&x, 24000) < 0.01);
        // FILTER to the left: the repeat is duller (a 10 ms step keeps less of its edge).
        let dull = run([0.5, 0.0], [false, false], Kind::Echo);
        let bright = run([0.5, 0.5], [false, false], Kind::Echo);
        let edge = |x: &[f32]| (x[24001 * 2] - x[23999 * 2]).abs();
        assert!(edge(&dull) < edge(&bright) * 0.5, "{} vs {}", edge(&dull), edge(&bright));
        // DELAY, LR RATIO to the left end: the left repeat comes at half the time.
        let x = run([0.5, 0.0], [false, false], Kind::Delay);
        assert!(x[12100 * 2] > 0.3 && x[12100 * 2 + 1].abs() < 0.01 && x[24100 * 2 + 1] > 0.3);
        // MT DELAY at the centre: seven repeats an eighth of a beat apart (3,000 frames).
        let x = run([0.5, 0.5], [false, false], Kind::MtDelay);
        for k in 1..=7 {
            assert!(at(&x, k * 3000) > 0.05, "tap {k}");
        }
        assert!(at(&x, 8 * 3000 + 600) < 0.001);
        // TRANS in single mode with DUTY at a quarter: open for a quarter of the beat.
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(Kind::Echo, 1.0, 1.0);
        p.slots[0].on = false;
        p.single = true;
        p.one = SlotParams { on: true, level: 1.0, value: 1.0, knob: [0.25, 0.5], ..SlotParams::new(Kind::Trans) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(48000, 440.0);
        rack.process(0, &mut x, 48000);
        assert!(rms(&x[1000 * 2..5000 * 2]) > 0.3 && rms(&x[7000 * 2..23000 * 2]) < 0.01 && rms(&x[25000 * 2..29000 * 2]) > 0.3);
    }

    #[test]
    fn quantize_puts_the_gate_and_the_roll_on_the_beat_grid() {
        // TRANS, 1 beat, switched on half a beat into a beat: with quantize the gate is in
        // its closed half and opens at the next beat (12,000 frames on at 120 BPM); without,
        // it opens at once.
        for (quantize, open_first) in [(true, false), (false, true)] {
            let mut rack = Rack::new();
            rack.enabled = true;
            let mut p = unit(Kind::Trans, 1.0, 1.0);
            p.quantize = [quantize; CHANNELS];
            rack.units[0].params = p;
            rack.units[0].prepare(&p.assign);
            let mut x = tone(36000, 440.0);
            let mut beat = 8.5f64;
            for chunk in x.chunks_mut(480 * 2) {
                rack.beat[0] = Some(beat);
                rack.process(0, chunk, 48000);
                beat += 480.0 / 24000.0;
            }
            let (a, b) = (rms(&x[2000 * 2..10000 * 2]), rms(&x[14000 * 2..22000 * 2]));
            assert_eq!(a > 0.3, open_first, "quantize {quantize}: {a}");
            assert_eq!(b > 0.3, !open_first, "quantize {quantize}: {b}");
        }
        // ROLL, 1/2 beat, switched on a quarter beat after a beat: the loop is the half beat
        // that ended on that beat, joined a quarter beat in.
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params.assign = [true, false, false, false, false, false];
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut hist: Vec<f32> = (0..48000).flat_map(|i| { let v = i as f32 / 48000.0; [v, v] }).collect();
        rack.process(0, &mut hist, 48000);
        rack.units[0].params = unit(Kind::Roll, 0.5, 1.0);
        rack.beat[0] = Some(4.25);
        let mut x = vec![0.0f32; 480 * 2];
        rack.process(0, &mut x, 48000);
        // Beat 4.0 was 6,000 frames before this buffer: the loop is history 30,000..42,000,
        // and the first frame out is 6,000 into it.
        assert!((x[400 * 2] - (36000.0 + 400.0) / 48000.0).abs() < 0.001, "{}", x[400 * 2]);
    }

    #[test]
    fn stretch_plays_slower_and_vinyl_brake_stops() {
        // STRETCH at LEVEL/DEPTH 0.5 with REPITCH: half speed, so 440 Hz is heard as 220 Hz.
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(Kind::Trans, 4.0, 0.5);
        p.slots[0].on = false;
        p.single = true;
        p.one = SlotParams { on: true, level: 0.5, value: 4.0, btn: [true, false], ..SlotParams::new(Kind::Stretch) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(48000, 440.0);
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        let hz = crossings(&x[24000 * 2..]) as f32 * 2.0;
        assert!((hz - 220.0).abs() < 8.0, "{hz}");
        // Without REPITCH the pitch is put back.
        rack = Rack::new();
        rack.enabled = true;
        p.one.btn = [false, false];
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(48000, 440.0);
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        let hz = crossings(&x[24000 * 2..]) as f32 * 2.0;
        assert!((hz - 440.0).abs() < 20.0, "{hz}");
        // VINYL BRAKE, 1 beat, LEVEL/DEPTH fully up, ECHO at zero: silent once the beat is over.
        let mut rack = Rack::new();
        rack.enabled = true;
        p.one = SlotParams { on: true, level: 1.0, value: 1.0, knob: [0.5, 0.0], ..SlotParams::new(Kind::VinylBrake) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(48000, 440.0);
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        assert!(rms(&x[2000 * 2..8000 * 2]) > 0.2 && rms(&x[30000 * 2..]) < 0.01);
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
            rack.units[0].params.assign = [true, false, false, false, false, false];
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

    /// `cargo test --release fx_cost -- --ignored --nocapture`: what the effects cost on the
    /// audio thread, as a share of real time (512-frame buffers at 48 kHz, one channel).
    #[test]
    #[ignore]
    fn fx_cost() {
        let run = |label: &str, set: &dyn Fn(&mut UnitParams)| {
            let mut rack = Rack::new();
            rack.enabled = true;
            let mut p = unit(Kind::Delay, 1.0, 0.7);
            p.slots[0].on = false;
            set(&mut p);
            rack.units[0].params = p;
            rack.units[0].prepare(&p.assign);
            let mut x = tone(48000 * 20, 330.0);
            let t = std::time::Instant::now();
            for chunk in x.chunks_mut(512 * 2) {
                rack.process(0, chunk, 48000);
            }
            let secs = t.elapsed().as_secs_f64();
            println!("fx cost: {label:<34} {:6.2} % of real time ({:.1} us per 512-frame buffer)", secs / 20.0 * 100.0, secs / (48000.0 * 20.0 / 512.0) * 1e6);
        };
        run("assigned, nothing on", &|_| {});
        run("DELAY + ECHO + SPIRAL", &|p| p.slots.iter_mut().for_each(|s| s.on = true));
        for info in INFO.iter() {
            let label = format!("{} alone", info.name);
            run(&label, &|p| p.slots[0] = SlotParams { on: true, level: 0.7, ..SlotParams::new(info.kind) });
        }
        run("REVERB + PHASER + PITCH ECHO-like", &|p| {
            p.slots = [SlotParams { on: true, ..SlotParams::new(Kind::Reverb) }, SlotParams { on: true, ..SlotParams::new(Kind::Phaser) }, SlotParams { on: true, ..SlotParams::new(Kind::UpEcho) }];
        });
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
