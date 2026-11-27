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
//! Traced from rekordbox's effect classes (the `Multi*::updateParameter` functions and the
//! cores): every multi-mode effect's LEVEL/DEPTH law, the reverb (line times, decay, input
//! gain, cross-fed combs), Beat FX FILTER's structure and cutoff curves, SPIRAL's feedback
//! law, LOW CUT ECHO's band split, PING PONG's times, UP / DOWN ECHO, HELIX, ROBOT's gains,
//! PITCH's percent law, the FX+ echoes' filters, the brake and back-spin curves.
//! What is still recordbox's own: the pitch shifter, ENIGMA JET, MOBIUS, NOISE, SPIRAL UP /
//! DOWN and REVERB UP / DOWN bodies, ROBOT's modulation, MT DELAY's taps, STRETCH's and
//! Beat FX VINYL BRAKE's knobs, and most single-mode knob laws (marked below).

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


// ---------------------------------------------------------------------------------------
// Laws read from rekordbox 7.2.19's effect classes (arm64, Hex-Rays). `l` is LEVEL/DEPTH.
// ---------------------------------------------------------------------------------------

/// rekordbox's feedback law for the echo family's FEEDBACK knob: the left half runs from 0
/// to the centre value, the right half from there to 1.
pub fn fb_law(k: f32, centre: f32) -> f32 {
    let k = k.clamp(0.0, 1.0);
    if k < 0.5 { 2.0 * centre * k } else { centre + (1.0 - centre) * 2.0 * (k - 0.5) }
}

/// The echo family's LEVEL/DEPTH law (`MultiBeatFxEcho::calcMixRatio`; DELAY, ECHO, REVERB,
/// LOW CUT ECHO and PING PONG use it too): (dry, wet).
pub fn echo_mix(level: f32) -> (f32, f32) {
    if level < 0.75 { (1.0, 1.3333 * level) } else { (4.0 - 4.0 * level, 1.0) }
}

/// MT DELAY's LEVEL/DEPTH law (`GroupMultiTapDelay::calculateMixRatio`): (dry, wet).
pub fn mt_mix(level: f32) -> (f32, f32) {
    if level < 0.5 { (1.0, 1.4 * level) } else { (1.3 - 0.6 * level, 0.7) }
}

/// TRANS and PAN (`MultiBeatFxTrans` / `MultiBeatFxPan::updateParameter`): the effect takes
/// over in the lower half of the knob and is alone from the centre.
pub fn half_mix(l: f32) -> (f32, f32) {
    if l < 0.5 { (1.0 - 2.0 * l, 2.0 * l) } else { (0.0, 1.0) }
}

/// FLANGER and PHASER (`MultiBeatFxFlanger::updateParameter`): both at 0.75 from a quarter up.
pub fn quarter_mix(l: f32) -> (f32, f32) {
    if l >= 0.25 { (0.75, 0.75) } else { (1.0 - l, 3.0 * l) }
}

/// FLANGER's and PHASER's feedback in multi mode: none up to a quarter, then up to 0.8.
pub fn mod_fb(l: f32) -> f32 {
    if l < 0.25 { 0.0 } else { 1.0667 * l - 0.26667 }
}

/// TRANS's open share of each turn in multi mode: half up to the centre, 1/16 fully up.
pub fn trans_duty(l: f32) -> f32 {
    if l >= 0.5 { 0.9375 - 0.875 * l } else { 0.5 }
}

/// Beat FX FILTER (`MultiBeatFxFilter::updateParameter`): the filter is alone above 0.15625,
/// below that a quarter-sine cross-fade; (dry, wet).
pub fn filter_mix(l: f32) -> (f32, f32) {
    if l > 0.15625 {
        return (0.0, 1.0);
    }
    let x = l.max(0.0) * 12.8;
    let t = (1.0 - x) - x * (1.0 - x);
    let dry = ((0.5 - 0.5 * x) * ((0.071862 * t - 0.64211) * t + 1.5703) + 0.5).clamp(0.0, 1.0);
    (dry, 1.0 - dry)
}

/// Beat FX FILTER's resonance gain in multi mode (0 .. 0.96).
pub fn filter_reso(l: f32) -> f32 {
    let v = if l < 0.1523 { 1.0 } else if l >= 0.3984 { 1.0 - l } else { 1.0 - 0.000048631 * l };
    (0.96 - 0.96 * v * v).min(0.96)
}

/// The ROLL family (`MultiBeatFxRollBase::calcMixRatio`): rekordbox's 129-step table is
/// 1 - (i / 127.5)^2 exactly; (dry, wet).
pub fn roll_mix(l: f32) -> (f32, f32) {
    let i = (l.clamp(0.0, 1.0) * 256.0) as i32;
    if i > 128 {
        return (0.0, 1.0);
    }
    let t = |i: i32| (1.0 - (i as f32 / 127.5).powi(2)).max(0.0);
    (t(i), t(128 - i))
}

/// UP ECHO / DOWN ECHO (`MultiBeatFxUpEcho::updateParameter`): (wet, pitch percent of each
/// repeat). Feedback is 0.85; the dry sound stays whole.
pub fn up_echo(l: f32) -> (f32, f32) {
    if l < 0.5 { (1.4 * l, 0.0) } else { (0.7, 33.333 * l - 16.667) }
}

/// rekordbox's pitch shifter percent law (`rbxfrm::PitchShifter::setPitchPercentage`):
/// -100 % is an octave down, +100 % an octave up, +200 % two.
pub fn pitch_ratio(pct: f32) -> f32 {
    let p = pct.clamp(-200.0, 200.0);
    1.0 + if p < 0.0 { 0.005 * p } else if p < 100.0 { 0.01 * p } else { 0.02 * p - 1.0 }
}

/// SPIRAL (`BeatFxSpiral::updateParameter`): (wet, feedback, output gain). While it is on the
/// feedback passes 1 above the centre (the core limits the loop); once switched off it is
/// held under 1 so the tail dies away.
pub fn spiral_law(l: f32, on: bool) -> (f32, f32, f32) {
    let wet = if l < 0.75 { 1.333 * l } else { 1.0 };
    let fb = if l < 0.1 {
        8.0 * l
    } else if l < 0.5 {
        0.8
    } else if l < 1.0 {
        0.8 + if on { 0.8 } else { 0.386 } * (l - 0.5)
    } else if on {
        1.2
    } else {
        0.993
    };
    (wet, fb, 1.0 - 0.501 * l)
}

/// HELIX (`MultiBeatFxHelix::updateParameter`): (how much of the loop is kept each turn, how
/// much of the input enters). Fully up the loop is held and the input is shut out.
pub fn helix_law(l: f32) -> (f32, f32) {
    if l >= 0.5 {
        (0.7 + 0.3 * l, 0.9 - 0.9 * l)
    } else {
        let t = (1.0 - 2.0 * l) * (1.0 - 2.0 * l);
        (0.85 * (1.0 - t), 0.45 + 0.55 * t)
    }
}

/// ROBOT (`MultiFxRobot::updateParameter`): (the plain sound's gain, the robot voice's gain,
/// modulation). The knob is read in 8-bit steps.
pub fn robot_law(l: f32) -> (f32, f32, f32) {
    let v = (l.clamp(0.0, 1.0) * 255.0) as i32;
    if v > 24 { (0.75, 0.75, (v - 25) as f32 * 0.0034783) } else { (1.0 - 0.01 * v as f32, 0.03 * v as f32, 0.0) }
}

/// STRETCH's playing speed from LEVEL/DEPTH (`StretchCore::processEffectInternal`).
pub fn stretch_speed(level: f32) -> f32 {
    if level >= 0.5 { 0.75 - 0.5 * level } else { 1.0 - level }
}

/// PHASER's number of all-pass stages for the STAGE knob: 2 at the left end, 6 at the centre
/// (what multi mode uses), 24 at the right end (rekordbox's range is 2..60; its knob law was
/// not traced).
pub fn phaser_stages(k: f32) -> usize {
    if k <= 0.5 { 2 + (k.max(0.0) * 8.0).round() as usize } else { 6 + ((k.min(1.0) - 0.5) * 36.0).round() as usize }
}

/// REVERB (`MultiBeatFxReverb::CoreParameterCalculator`), for the percent value `p` (1..100):
/// the size factor the comb times are multiplied by.
pub fn reverb_size(p: f32) -> f32 {
    0.2 + 0.17101 * (0.02306 * p + 1.01).powi(2)
}

/// The reverb's input gain.
pub fn reverb_in_gain(p: f32) -> f32 {
    0.2 - 6.6116e-5 * (p - 55.0).powi(2) + if p >= 91.0 { ((p - 90.0) * 0.1).powi(4) * 0.016529 } else { 0.0 }
}

/// The reverb's decay time (ms to -60 dB). rekordbox keeps ten 100-step tables of comb gains;
/// every entry is 10^(-3 * comb delay / this time), and this polynomial fits the time within
/// 4 % (0.77 s at 1 %, 9.1 s at 50 %, 250 s at 100 %).
pub fn reverb_rt60_ms(p: f32) -> f32 {
    let p = p.clamp(1.0, 100.0);
    (((((2.27830833e-9 * p - 5.30411715e-7) * p + 3.42242528e-5) * p + 6.5895372e-5) * p + 1.18306299e-2) * p + 6.6592768).exp()
}

/// The reverb's comb delays at size 1 (ms; left and right of each of the five), and the two
/// all-passes' (left, right).
pub const REVERB_COMB_MS: [f32; 10] = [22.0938, 38.6667, 39.9167, 23.2292, 23.8542, 40.8646, 41.5, 24.802, 21.5, 23.885];
pub const REVERB_AP_MS: [f32; 4] = [6.97917, 8.23958, 22.7396, 21.4792];

/// VINYL BRAKE's speed step (`VinylBrakeCore`): the speed falls in a straight line that would
/// reach zero after the decay time, and below 1/8 at a fixed 0.20559 per second.
#[inline]
pub fn brake_step(speed: f32, decay_secs: f32, rate: f32) -> f32 {
    (speed - if speed >= 0.125 { 1.0 / (decay_secs * rate) } else { 0.20559 / rate }).max(0.0)
}

/// BACK SPIN (`ReleaseFxBackSpin`, `BackSpinCore::calculatePlaybackSpeedArray`): the backward
/// speed `t` seconds after the press, for a stop time and the length of two beats (seconds).
/// It climbs to 8 times speed over 0.16 of two beats, falls in a straight line over the rest
/// of the stop time down to 0.28235, and from there stops within 0.1861 s.
pub fn back_spin_speed(t: f32, stop: f32, two_beats: f32) -> f32 {
    let (mut up, mut down) = (0.16 * two_beats, stop - 0.16 * two_beats);
    if down < up {
        up = stop * 0.5;
        down = stop * 0.5;
    }
    let (up, down) = (up.max(0.001), down.max(0.001));
    if t < up {
        return 8.0 * t / up;
    }
    let knee = down * (8.0 - 0.28235) / 8.0;
    let u = t - up;
    if u < knee { 8.0 - 8.0 * u / down } else { (0.28235 * (1.0 - (u - knee) / 0.1861)).max(0.0) }
}

/// A curve through evenly spaced points, interpolated in the logarithm (fits of rekordbox's
/// frequency tables).
pub fn knots(k: &[f32], x: f32) -> f32 {
    let f = x.clamp(0.0, 1.0) * (k.len() - 1) as f32;
    let i = (f as usize).min(k.len() - 2);
    (k[i].ln() + (k[i + 1].ln() - k[i].ln()) * (f - i as f32)).exp()
}

/// Beat FX FILTER's cutoffs over the LFO's travel (`LFOFilterCore`'s 129-step tables, every
/// eighth step): the low-pass and the lower edge of its resonance band, and the high-pass
/// (whose resonance band ends at four times its cutoff).
const LFO_LPF_MAIN: [f32; 17] = [23718.6, 16080.8, 12517.8, 9916.9, 7908.7, 6316.7, 5034.8, 3992.9, 3142.3, 2448.0, 1884.6, 1433.3, 1080.0, 814.7, 629.9, 521.0, 485.0];
const LFO_LPF_RESO: [f32; 17] = [16845.1, 6351.0, 3601.2, 2201.0, 1399.3, 914.6, 612.7, 421.3, 298.3, 218.1, 165.0, 129.2, 104.8, 88.2, 77.3, 71.2, 69.1];
const LFO_HPF_MAIN: [f32; 17] = [51.8, 112.7, 157.3, 210.3, 285.4, 390.9, 533.6, 722.2, 969.9, 1296.3, 1728.4, 2300.9, 3055.7, 4040.5, 5306.9, 6907.4, 8892.1];

/// The FX+ pack's filter cutoffs for the main knob in 8-bit steps `v` (`FxPlus*Echo::
/// updateFilterCoef`): LPF ECHO 20 kHz down to 96 Hz, HPF ECHO 60 Hz up to 8.5 kHz, BPF ECHO
/// 150 Hz up to 8.7 kHz, CRUSH ECHO's high-pass 60 Hz up to 4 kHz.
pub fn fxplus_fc(kind: Kind, v: f32) -> f32 {
    let n = (v - 4.0).max(0.0);
    match kind {
        Kind::LpfEcho => 20000.0 * 0.97895318f32.powf(251.0 * (n / 251.0).powf(0.78)),
        Kind::HpfEcho => 60.0 * 1.0201633f32.powf(n),
        Kind::BpfEcho => 150.0 * 1.0163087f32.powf(251.0 * (n / 251.0).powf(1.1)),
        _ => 60.0 * 1.0168726f32.powf(n),
    }
}

/// The FX+ echoes' share of the plain and the filtered sound for the main knob `v` (0..255).
pub fn fxplus_mix(kind: Kind, v: f32) -> (f32, f32) {
    let g = if v < 4.0 { 1.0 } else { 1.0159 - 0.0039841 * v };
    let g3 = g * g * g;
    match kind {
        Kind::LpfEcho => {
            if v <= 3.0 {
                (1.0, 0.0)
            } else if v < 32.0 {
                let x = (v - 4.0) * 0.035714;
                (1.0 - x, x)
            } else if v < 224.0 {
                (0.0, 1.0)
            } else {
                (0.0, (255.0 - v) * 0.015625 + 0.5)
            }
        }
        Kind::HpfEcho => (g3, (1.0 - g3) * if v >= 231.0 { (255.0 - v) * 0.012 + 0.7 } else { 1.0 }),
        Kind::BpfEcho => (1.0, 1.0 - g3),
        _ => (g3, 1.0 - g3),
    }
}

const MAX_RATE: usize = 48000;
/// Longest delay (4 s) and the history kept for ROLL and RELEASE FX (15 s), in frames.
const DELAY_LEN: usize = 4 * MAX_RATE;
const RING_LEN: usize = 15 * MAX_RATE;
/// The FX+ echoes share the delay line: four parts of a second each.
const PART_LEN: usize = DELAY_LEN / 4;
/// The pitch shifter's line and window (frames).
const PS_LEN: usize = 4096;
const PS_WIN: f32 = 2048.0;
/// The reverb's lines: combs up to 41.5 ms x 2.08, all-passes up to 22.8 ms (at 48 kHz).
const COMB_LEN: usize = 4400;
const AP_LEN: usize = 1200;
/// On/off and gate smoothing, as rekordbox's 3.3333 ms.
const SMOOTH_SECS: f32 = 0.0033333;
/// The band split of LOW CUT / HI CUT (`ThreeBandSeparate`: the isolator's crossovers).
const CUT_LOW_HZ: f32 = 284.0;
const CUT_HIGH_HZ: f32 = 4649.0;
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

/// The spiral's loop limiter: through up to 1, never past 1.25 (rekordbox works out a
/// threshold from the input level; this fixed one is recordbox's own).
#[inline]
fn limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 1.0 { x } else { x.signum() * (1.0 + (a - 1.0) / (1.0 + 4.0 * (a - 1.0))) }
}

#[inline]
fn svf_g(fc: f32, rate: f32) -> f32 {
    (PI * (fc / rate).clamp(0.0002, 0.45)).tan()
}

/// A second-order Butterworth section (low-pass, or high-pass with `hp`): b0, b1, b2, a1, a2.
fn butter(fc: f32, rate: f32, hp: bool) -> [f32; 5] {
    let w = TAU * (fc / rate).min(0.45);
    let (s, c) = w.sin_cos();
    let alpha = s / std::f32::consts::SQRT_2;
    let a0 = 1.0 + alpha;
    let b = if hp { [(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0] } else { [(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0] };
    [b[0], b[1], b[2], -2.0 * c / a0, (1.0 - alpha) / a0]
}

#[inline]
fn biquad(z: &mut [f32], c: &[f32; 5], x: f32) -> f32 {
    let y = c[0] * x + z[0];
    z[0] = c[1] * x + z[1] - c[3] * y;
    z[1] = c[2] * x - c[4] * y;
    y
}

/// First-order sections as rekordbox's `IirFilter::calcCoef*1stOrder` (bilinear): b0, b1, a1.
#[inline]
fn one_pole(fc: f32, rate: f32, hp: bool) -> [f32; 3] {
    let k = (PI * (fc / rate).min(0.319)).tan();
    if hp { [1.0 / (1.0 + k), -1.0 / (1.0 + k), (k - 1.0) / (k + 1.0)] } else { [k / (1.0 + k), k / (1.0 + k), (k - 1.0) / (k + 1.0)] }
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
    /// More than 0: that many taps, an eighth of the time apart, and no feedback (MT DELAY).
    taps: u8,
    /// The loop goes through the spiral's limiter instead of the plain one.
    limit: bool,
}

/// The reverb's settings for a block.
#[derive(Clone, Copy)]
struct Rv {
    gin: f32,
    d: [usize; 10],
    g: [f32; 10],
    ap: [usize; 4],
    /// One-pole steps: the loop's low-pass, the input's low-pass and high-pass.
    loop_k: f32,
    in_lp: f32,
    in_hp: f32,
}

impl Rv {
    /// `p`: the percent value; `size` and `decay` scale the lines and the decay time (1 in
    /// multi mode).
    fn new(p: f32, size: f32, decay: f32, rate: f32) -> Rv {
        let p = p.clamp(1.0, 100.0);
        let s = (reverb_size(p) * size).clamp(0.2, 2.08);
        let rt = reverb_rt60_ms(p) * decay;
        let step = |fc: f32| 1.0 - (-TAU * fc / rate).exp();
        let mut v = Rv { gin: reverb_in_gain(p), d: [1; 10], g: [0.0; 10], ap: [1; 4], loop_k: step(if rate >= 47000.0 { 20500.0f32.min(0.45 * rate) } else { 18000.0 }), in_lp: step(14000.0), in_hp: step(80.0) };
        for i in 0..10 {
            let ms = REVERB_COMB_MS[i] * s;
            v.d[i] = ((ms * 0.001 * rate) as usize).clamp(1, COMB_LEN - 1);
            v.g[i] = 10f32.powf(-3.0 * ms / rt);
        }
        for i in 0..4 {
            v.ap[i] = ((REVERB_AP_MS[i] * 0.001 * rate) as usize).clamp(1, AP_LEN - 1);
        }
        v
    }
}

/// One effect's state for one channel.
pub(crate) struct Slot {
    kind: Kind,
    /// Stereo delay line (interleaved) and its write position (frames).
    delay: Vec<f32>,
    w: usize,
    /// Delay time in use (frames), moving toward the wanted time.
    time: f32,
    /// Wet gain in use (0 off .. 1 on).
    gate: f32,
    lfo: f32,
    /// LOW CUT / HI CUT band split (per channel: the cut bands, then the rest; two
    /// Butterworth sections each, as rekordbox's `ThreeBandSeparate`).
    bands: [[f32; 16]; 2],
    f1: [[f32; 2]; 2],
    f2: [[f32; 2]; 2],
    f3: [[f32; 2]; 2],
    lp: [f32; 2],
    ap: [[f32; 24]; 2],
    fb: [f32; 2],
    /// Beat FX FILTER: per channel, the six first-order sections' memories and the resonance
    /// path's last output.
    lf: [[f32; 13]; 2],
    /// Reverb: ten comb lines and four all-pass lines (flat), their write positions, the
    /// loop filters and the input filters.
    combs: Vec<f32>,
    comb_w: usize,
    comb_lp: [f32; 10],
    aps: Vec<f32>,
    ap_w: usize,
    rin: [f32; 4],
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
    /// VINYL BRAKE / STRETCH: how far behind the live sound playback is (frames), frames
    /// into the current turn, and the brake's speed.
    lag: f64,
    t: f32,
    speed: f32,
    /// MOBIUS: oscillator phases and the input's envelope. The FX+ echoes' second LFO.
    osc: [f32; 6],
    env: f32,
    lfo2: f32,
}

impl Slot {
    pub(crate) fn new() -> Slot {
        Slot { delay: vec![0.0; DELAY_LEN * 2], combs: vec![0.0; COMB_LEN * 10], aps: vec![0.0; AP_LEN * 4], ps: vec![0.0; PS_LEN * 2], ..Slot::empty() }
    }

    /// A slot without its lines, for a place that is never switched on.
    pub(crate) fn empty() -> Slot {
        Slot { kind: Kind::Delay, delay: Vec::new(), w: 0, time: 0.0, gate: 0.0, lfo: 0.0, bands: [[0.0; 16]; 2], f1: [[0.0; 2]; 2], f2: [[0.0; 2]; 2], f3: [[0.0; 2]; 2], lp: [0.0; 2], ap: [[0.0; 24]; 2], fb: [0.0; 2], lf: [[0.0; 13]; 2], combs: Vec::new(), comb_w: 0, comb_lp: [0.0; 10], aps: Vec::new(), ap_w: 0, rin: [0.0; 4], roll: None, was_on: false, odd: false, ps: Vec::new(), ps_w: 0, ps_ph: 0.0, rev_k: 0.0, seed: 0x2468_ACE1, hold: [0.0; 2], hold_n: 0.0, lag: 0.0, t: 0.0, speed: 1.0, osc: [0.0; 6], env: 0.0, lfo2: 0.0 }
    }

    fn clear(&mut self) {
        self.delay.iter_mut().for_each(|v| *v = 0.0);
        self.ps.iter_mut().for_each(|v| *v = 0.0);
        self.combs.iter_mut().for_each(|v| *v = 0.0);
        self.aps.iter_mut().for_each(|v| *v = 0.0);
        self.bands = [[0.0; 16]; 2];
        self.f1 = [[0.0; 2]; 2];
        self.f2 = [[0.0; 2]; 2];
        self.f3 = [[0.0; 2]; 2];
        self.lp = [0.0; 2];
        self.ap = [[0.0; 24]; 2];
        self.fb = [0.0; 2];
        self.lf = [[0.0; 13]; 2];
        self.comb_lp = [0.0; 10];
        self.rin = [0.0; 4];
        self.time = 0.0;
        self.lfo = 0.0;
        self.lfo2 = 0.0;
        self.roll = None;
        self.odd = false;
        self.rev_k = 0.0;
        self.hold = [0.0; 2];
        self.hold_n = 0.0;
        self.lag = 0.0;
        self.t = 0.0;
        self.speed = 1.0;
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

    /// One of the four parts the FX+ echoes share the line in: read `frames` back.
    #[inline]
    fn part_tap(&self, part: usize, frames: f32, c: usize) -> f32 {
        let i = (frames as usize).clamp(1, PART_LEN - 1);
        let w = self.w % PART_LEN;
        self.delay[(part * PART_LEN + (w + PART_LEN - i) % PART_LEN) * 2 + c]
    }

    #[inline]
    fn part_put(&mut self, part: usize, l: f32, r: f32) {
        let k = (part * PART_LEN + self.w % PART_LEN) * 2;
        self.delay[k] = sat(l);
        self.delay[k + 1] = sat(r);
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

    /// The echo core (`EchoCore::processEffect`): read the repeats, colour them, write the
    /// input and the feedback. The output is the repeats alone.
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
            er = self.tap((self.time * e.mr).max(0.010 * r), 1);
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
        if e.limit {
            self.push(limit(il + el * e.fb), limit(ir + er * e.fb));
        } else {
            self.push(il + el * e.fb, ir + er * e.fb);
        }
        (el, er)
    }

    /// rekordbox's reverb (`NewReverbCore`): the input through a 14 kHz low-pass and an 80 Hz
    /// high-pass at the input gain, five combs in parallel whose left and right lines feed
    /// each other through a low-pass, then two all-passes of gain 0.7 in series, at 0.7081.
    #[inline]
    fn reverb(&mut self, l: f32, r: f32, v: &Rv) -> (f32, f32) {
        let mut x = [l, r];
        for c in 0..2 {
            self.rin[c] += (x[c] - self.rin[c]) * v.in_lp;
            self.rin[2 + c] += (self.rin[c] - self.rin[2 + c]) * v.in_hp;
            x[c] = (self.rin[c] - self.rin[2 + c]) * v.gin;
        }
        let w = self.comb_w;
        let mut acc = [0.0f32; 2];
        for k in 0..5 {
            let (il, ir) = (2 * k, 2 * k + 1);
            let fl = self.combs[il * COMB_LEN + (w + COMB_LEN - v.d[il]) % COMB_LEN] * v.g[il];
            let fr = self.combs[ir * COMB_LEN + (w + COMB_LEN - v.d[ir]) % COMB_LEN] * v.g[ir];
            acc[0] += fl;
            acc[1] += fr;
            self.comb_lp[il] += (fl - self.comb_lp[il]) * v.loop_k;
            self.comb_lp[ir] += (fr - self.comb_lp[ir]) * v.loop_k;
            self.combs[il * COMB_LEN + w] = x[0] + self.comb_lp[ir];
            self.combs[ir * COMB_LEN + w] = x[1] + self.comb_lp[il];
        }
        self.comb_w = (w + 1) % COMB_LEN;
        let aw = self.ap_w;
        for c in 0..2 {
            for k in 0..2 {
                let i = k * 2 + c;
                let d = self.aps[i * AP_LEN + (aw + AP_LEN - v.ap[i]) % AP_LEN];
                let y = d - 0.7 * acc[c];
                self.aps[i * AP_LEN + aw] = acc[c] + 0.7 * y;
                acc[c] = y;
            }
        }
        self.ap_w = (aw + 1) % AP_LEN;
        (acc[0] * 0.7081, acc[1] * 0.7081)
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// `multi`: a multi-mode slot (or a PAD FX pad), whose inner settings rekordbox derives
    /// from LEVEL/DEPTH alone; otherwise single mode, where the two knobs and two buttons set
    /// them. `beat`: where the channel's deck is on its beat grid at the start of `buf`
    /// (beats), when Beat FX quantize applies.
    pub(crate) fn process(&mut self, p: &SlotParams, bpm: f32, buf: &mut [f32], rate: u32, ring: &Ring, beat: Option<f64>, multi: bool) {
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
        // The effect's time: its beat value, or (no beat control) a beat.
        let beats = if table.is_empty() || percent { 1.0 } else { p.value.abs().max(0.0625) };
        let down = !percent && p.value < 0.0;
        let secs = (beats * beat_secs).clamp(0.010, 32.0);
        let gate_k = 1.0 / (SMOOTH_SECS * r);
        let level = p.level.clamp(0.0, 1.0);
        let (k1, k2) = (p.knob[0].clamp(0.0, 1.0), p.knob[1].clamp(0.0, 1.0));
        let (b1, b2) = (p.btn[0], p.btn[1]);
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
            self.speed = 1.0;
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
        // Time the delay line runs at (rekordbox: 1 ms .. 4 s; SPIRAL 10 ms .. 3.99 s).
        let echo_secs = match kind {
            MobiusSaw | MobiusTri | VinylBrake | Noise => 0.75 * beat_secs,
            Helix => secs * if multi { 1.0 } else { 0.5 + k1 },
            Spiral => secs.clamp(0.010, 3.99),
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
        let mut e = EchoCfg { ml: 1.0, mr: 1.0, fb: 0.0, rev: false, freeze: false, ratio: 1.0, filt: 0, g: 0.0, taps: 0, limit: false };
        // FILTER knob of the echoes: centre = through, left a low-pass falling from 12 kHz to
        // 93 Hz, right a high-pass rising from 61 Hz to 8.95 kHz (the ends of rekordbox's
        // `InstFxLPF` / `InstFxHPF` tables; a plain second-order filter stands in for theirs).
        let echo_filter = |e: &mut EchoCfg, k: f32| {
            if k < 0.48 {
                e.filt = 1;
                e.g = svf_g(crate::cfx::inst_fc(false, (0.5 - k) * 2.0), r);
            } else if k > 0.52 {
                e.filt = 2;
                e.g = svf_g(crate::cfx::inst_fc(true, (k - 0.5) * 2.0), r);
            }
        };
        // (dry, wet) of the effects that mix a wet sound with the dry one.
        let mut mix = echo_mix(level);
        let mut spiral_out = 1.0;
        match kind {
            Delay | RevDelay => {
                // Multi mode: one repeat, both sides at the beat value. Single mode: LR RATIO
                // in rekordbox's nine steps (1:3, 1:2, 1:1.5, 1:1.25, 1:1, ... 3:1).
                if !multi {
                    const STEP: [(f32, f32); 9] = [(1.0, 3.0), (1.0, 2.0), (1.0, 1.5), (1.0, 1.25), (1.0, 1.0), (1.25, 1.0), (1.5, 1.0), (2.0, 1.0), (3.0, 1.0)];
                    let (a, b) = STEP[((k2 * 9.0) as usize).min(8)];
                    let m = a.max(b);
                    e.ml = swing(k1) * a / m;
                    e.mr = swing(k1) * b / m;
                }
                e.rev = kind == RevDelay;
            }
            Echo => {
                e.fb = if multi { 0.7 } else { fb_law(k1, 0.7) };
                if !multi {
                    echo_filter(&mut e, k2);
                    e.rev = b1;
                    e.freeze = b2;
                }
            }
            UpEcho | DownEcho => {
                // Feedback 0.85; the repeats start to climb (or fall) from the centre up.
                let (wet, pct) = up_echo(level);
                mix = (1.0, wet);
                e.fb = 0.85;
                e.ratio = pitch_ratio(if kind == UpEcho { pct } else { -pct });
            }
            PitchEcho => {
                e.fb = fb_law(k1, 0.7);
                // PITCH: the knob's ends are an octave down and up.
                let pct = (k2 - 0.5) * 200.0;
                e.ratio = pitch_ratio(if pct.abs() < 2.5 { 0.0 } else { pct });
                e.rev = b1;
                e.freeze = b2;
            }
            LowCutEcho => {
                // An echo (feedback 0.7) of the sound without its lows (under 284 Hz).
                e.fb = if multi { 0.7 } else { fb_law(k1, 0.7) };
                if !multi {
                    e.ml = swing(k2);
                    e.mr = swing(k2);
                    e.rev = b1;
                    e.freeze = b2;
                }
            }
            PingPong => {
                // The right side at three quarters of the left's time (at least 10 ms).
                e.fb = if multi { 0.7 } else { fb_law(k1, 0.7) };
                e.mr = 0.75;
                if !multi {
                    echo_filter(&mut e, k2);
                    e.rev = b1;
                    e.freeze = b2;
                }
            }
            Spiral => {
                let (wet, fb, out) = spiral_law(level, p.on);
                mix = (1.0, wet);
                spiral_out = out;
                // Single mode: FEEDBACK 0 .. 0.8 .. 1.2 and PITCH (the knob's ends an octave).
                e.fb = if multi { fb } else if k1 < 0.5 { 1.6 * k1 } else { 0.8 + 0.8 * (k1 - 0.5) };
                if !p.on {
                    e.fb = e.fb.min(0.993);
                }
                e.limit = true;
                if !multi {
                    let pct = k2 * 200.0 - 100.0;
                    e.ratio = pitch_ratio(if pct.abs() < 2.5 { 0.0 } else { pct });
                }
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
                mix = mt_mix(level);
                e.taps = if multi { 7 } else { (1.0 + k2 * 12.0).round().clamp(1.0, 7.0) as u8 };
                if !multi {
                    e.ml = swing(k1);
                    e.mr = swing(k1);
                }
            }
            Reverb => {}
            Trans | Pan => mix = half_mix(level),
            Filter => mix = filter_mix(level),
            Flanger | Phaser => mix = quarter_mix(level),
            Roll | RevRoll | SlipRoll => mix = if self.roll.is_some() { roll_mix(level) } else { (1.0, 0.0) },
            Robot | Helix => mix = (0.0, 1.0),
            Pitch => mix = if level <= 0.0 || p.value == 0.0 && (multi || (k1 - 0.5).abs() <= 0.02) { (1.0, 0.0) } else { (0.0, 1.0) },
            Noise => {
                e.fb = 0.5;
                e.freeze = b2;
            }
            MobiusSaw | MobiusTri => e.fb = 0.5,
            VinylBrake => e.fb = 0.6,
            _ => {}
        }
        let (dry, wet) = mix;
        // LOW CUT / HI CUT: the effect is not given those bands (LOW CUT ECHO never gets the
        // lows). The echoes simply do not repeat them; the others pass them through.
        let has_cuts = info_of(kind).labels[2] == "LOW CUT";
        let (cut_lo, cut_hi) = (kind == LowCutEcho || has_cuts && !multi && b1, has_cuts && !multi && b2);
        let cuts = cut_lo || cut_hi;
        let (lo_c, hi_c) = (butter(CUT_LOW_HZ, r, false), butter(CUT_HIGH_HZ, r, true));
        let (lo_hp, hi_lp) = (butter(CUT_LOW_HZ, r, true), butter(CUT_HIGH_HZ, r, false));
        let echo_type = matches!(kind, Delay | RevDelay | Echo | PitchEcho | UpEcho | DownEcho | LowCutEcho | PingPong | Spiral | SpiralUp | SpiralDown | MtDelay | Reverb | ReverbUp | ReverbDown);
        // Per-effect settings that do not change inside the block.
        let dir = if down != (b1 && matches!(kind, EnigmaJet | MobiusSaw | MobiusTri)) { -1.0f32 } else { 1.0 };
        let hold_lfo = b2 && matches!(kind, EnigmaJet | MobiusSaw | MobiusTri);
        // ROBOT / PITCH: the percent value, or (single mode) the PITCH knob once it leaves its
        // centre. Multi-mode PITCH scales the shift with LEVEL/DEPTH (twice it at the top).
        let pct = if !multi && (k1 - 0.5).abs() > 0.02 { (k1 - 0.5) * 200.0 } else { p.value };
        let robot_ratio = pitch_ratio(pct.clamp(-100.0, 100.0));
        let pitch_r = pitch_ratio(if multi { pct * 2.0 * level } else { pct });
        let (rob_a, rob_b, rob_mod) = robot_law(level);
        let rob_mod = if multi { rob_mod } else { 0.8 * k2 };
        let speed = stretch_speed(level);
        let stages = if multi { 6 } else { phaser_stages(k2) };
        let (helix_keep, helix_in) = helix_law(level);
        let env_k = (1.0 - (-1.0 / (0.002 * r)).exp(), 1.0 - (-1.0 / (0.12 * r)).exp());
        let base = (ring.w + 2 * RING_LEN - frames) as f64;
        // REVERB: the percent value; single mode's ROOM SIZE scales the lines (half to one and
        // a half) and DECAY the decay time (a quarter to four times).
        let rv = match kind {
            Reverb => Rv::new(p.value, if multi { 1.0 } else { 0.5 + k1 }, if multi { 1.0 } else { 4f32.powf(2.0 * k2 - 1.0) }, r),
            ReverbUp | ReverbDown => Rv::new(35.0 + 30.0 * k1, 1.0, 1.0, r),
            _ => Rv::new(50.0, 1.0, 1.0, r),
        };
        // TRANS: the open share of a turn, never under 5 ms.
        let duty = (if multi { trans_duty(level) } else { k1 }).clamp((0.005 / secs).min(0.5), 1.0);
        // FLANGER / PHASER feedback: multi mode from LEVEL/DEPTH, single mode from FEEDBACK.
        let fl_fb = if multi { mod_fb(level) } else { fb_law(k1, 0.7).min(0.9999) * if b1 { -1.0 } else { 1.0 } };
        let ph_fb = if multi { mod_fb(level) } else { fb_law(k1, 0.5).min(0.9999) * if b1 { -1.0 } else { 1.0 } };
        // Beat FX FILTER: resonance gain, the LFO's reach and the type.
        let reso = if multi { filter_reso(level) } else { (0.96 * k1 * 2.0).min(0.96) };
        let (flt_span, flt_hp, flt_inv) = if multi { (1.0, false, false) } else { ((k2 * 2.0).min(1.0), b1, b2) };
        // The FX+ echoes: the main knob in 8-bit steps, ECHO MIX and MOD.
        let fxp_v = (level * 255.9).floor().min(255.0);
        let (fxp_echo, fxp_mod) = if multi { (0.5, 0.0) } else { (k1, k2) };
        let fxp_depth = if fxp_mod < 0.015625 { 0.0 } else if fxp_mod >= 0.15625 { 50.0 } else { 1.3943 * fxp_mod * 255.0 - 5.5556 };
        let fxp_lfo_step = 1.0 / ((0.200 - 0.00070588 * fxp_mod * 255.0).max(0.02) * r);
        let fxp_lvl: [f32; 4] = std::array::from_fn(|i| (4.0 * fxp_echo - i as f32).clamp(0.0, 1.0));
        for n in 0..frames {
            let target = if p.on { 1.0 } else { 0.0 };
            self.gate += (target - self.gate).clamp(-gate_k, gate_k);
            let g = self.gate;
            if g <= 0.0 && !p.on {
                // Off and faded out: the rest of the buffer is the dry sound, untouched.
                break;
            }
            let (xl, xr) = (buf[n * 2], buf[n * 2 + 1]);
            // What the effect is given (`dl`, `dr`) and the bands kept from it (`tl`, `tr`).
            let (mut dl, mut dr, mut tl, mut tr) = (xl, xr, 0.0, 0.0);
            if cuts {
                for (c, x) in [xl, xr].into_iter().enumerate() {
                    let z = &mut self.bands[c];
                    let (mut thru, mut y) = (0.0, x);
                    if cut_lo {
                        let a = biquad(&mut z[0..2], &lo_c, x);
                        thru += biquad(&mut z[2..4], &lo_c, a);
                        let b = biquad(&mut z[8..10], &lo_hp, y);
                        y = biquad(&mut z[10..12], &lo_hp, b);
                    }
                    if cut_hi {
                        let a = biquad(&mut z[4..6], &hi_c, x);
                        thru += biquad(&mut z[6..8], &hi_c, a);
                        let b = biquad(&mut z[12..14], &hi_lp, y);
                        y = biquad(&mut z[14..16], &hi_lp, b);
                    }
                    if c == 0 {
                        tl = thru;
                        dl = y;
                    } else {
                        tr = thru;
                        dr = y;
                    }
                }
                if echo_type {
                    // (The dry side of an echo is the whole sound.)
                    tl = 0.0;
                    tr = 0.0;
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
            // The dry sound (whole for the echoes) and the wet sound, mixed by the effect's
            // law; the on/off gate moves both gains over 3.33 ms.
            let (fdl, fdr) = if echo_type { (xl, xr) } else { (dl, dr) };
            let mixed = |wl: f32, wr: f32| (fdl * (1.0 + (dry - 1.0) * g) + wl * wet * g, fdr * (1.0 + (dry - 1.0) * g) + wr * wet * g);
            let (ol, or) = match kind {
                Delay | RevDelay | Echo | PitchEcho | UpEcho | DownEcho | LowCutEcho | PingPong | MtDelay => {
                    let (el, er) = self.echo(il, ir, &e, r);
                    mixed(el, er)
                }
                Spiral => {
                    // The loop: a 12 kHz low-pass going in, the repeats through a 150 Hz
                    // high-pass and a 12.1 kHz low-pass, at the output gain.
                    let gp = svf_g(12000.0, r);
                    let (pl, pr) = (svf(&mut self.f2[0], il, gp, 1.4).0, svf(&mut self.f2[1], ir, gp, 1.4).0);
                    let (el, er) = self.echo(pl, pr, &e, r);
                    let gh = svf_g(150.0, r);
                    let (hl, hr) = (svf(&mut self.f3[0], el, gh, 1.4).2, svf(&mut self.f3[1], er, gh, 1.4).2);
                    mixed(hl * spiral_out, hr * spiral_out)
                }
                SpiralUp | SpiralDown => {
                    // The spiral with its pitch climbing (or falling) further on every repeat;
                    // the main knob brings in the echo, its feedback and the bend.
                    let (el, er) = self.echo(il, ir, &e, r);
                    let wet = (level * 2.0).min(1.0) * (0.4 + 1.2 * k1).min(1.0) * g;
                    (xl + el * wet, xr + er * wet)
                }
                Reverb => {
                    let (rl, rr) = self.reverb(il, ir, &rv);
                    mixed(rl, rr)
                }
                ReverbUp | ReverbDown => {
                    // The sound is filtered (high-pass rising / low-pass falling with the main
                    // knob) and a reverb of it comes in; TIME is the decay.
                    let amt = (level * (0.5 + k2)).min(1.0);
                    let fc = if kind == ReverbUp { 20.0 * (6000.0f32 / 20.0).powf(amt) } else { 20000.0 * (300.0f32 / 20000.0).powf(amt) };
                    let gf = svf_g(fc, r);
                    let a = svf(&mut self.f2[0], xl, gf, 1.2);
                    let b = svf(&mut self.f2[1], xr, gf, 1.2);
                    let (fl, fr) = if kind == ReverbUp { (a.2, b.2) } else { (a.0, b.0) };
                    let x = (level * 8.0).min(1.0) * g;
                    let (ml, mr) = (xl + (fl - xl) * x, xr + (fr - xr) * x);
                    let (rl, rr) = self.reverb(ml * target, mr * target, &rv);
                    let wet = (level * 2.0).min(1.0) * 2.0 * g;
                    (ml + rl * wet, mr + rr * wet)
                }
                Trans => {
                    // A square gate once per beat value, open for its duty (never under 5 ms),
                    // its edges smoothed over 3.33 ms; SWING moves every second opening.
                    let sw = if multi { 0.0 } else if self.odd { (k2 - 0.5).max(0.0) * 0.5 } else { (0.5 - k2).max(0.0) * 0.5 };
                    let open = if self.lfo >= sw && self.lfo < sw + duty { 1.0 } else { 0.0 };
                    self.fb[0] += (open - self.fb[0]).clamp(-gate_k, gate_k);
                    mixed(dl * self.fb[0], dr * self.fb[0])
                }
                Pan => {
                    // The sound moves left and right once per beat value, each side up to
                    // 1.4125 (+3 dB, rekordbox's LFO range) as the other falls to nothing.
                    // Single mode: LFO AMP is how far, LFO its shape.
                    let s = (TAU * self.lfo).sin();
                    let s = if multi { s } else if k2 > 0.5 { (s * (1.0 + (k2 - 0.5) * 20.0)).clamp(-1.0, 1.0) } else { s * (1.0 - (0.5 - k2) * 2.0 * (1.0 - s.abs())) };
                    let pan = s * if multi { 1.0 } else { (k1 * 2.0).min(1.0) };
                    let a = (pan + 1.0) * 0.25 * PI;
                    mixed(dl * 1.4125 * a.cos(), dr * 1.4125 * a.sin())
                }
                Filter => {
                    // rekordbox's LFO filter: three first-order sections at the swept cutoff,
                    // and a resonance path (three more, band-limiting the result) fed back
                    // and added at the resonance gain. A triangle LFO turns once per beat
                    // value. TONE: the high-pass; PHASE: the LFO turned over.
                    let t = (if flt_inv { 1.0 - tri } else { tri }) * flt_span;
                    let (main, res) = if flt_hp {
                        let fc = knots(&LFO_HPF_MAIN, t);
                        (one_pole(fc, r, true), one_pole(fc * 4.0, r, false))
                    } else {
                        (one_pole(knots(&LFO_LPF_MAIN, t), r, false), one_pole(knots(&LFO_LPF_RESO, t), r, true))
                    };
                    let rg = reso * if t < 0.1 { 0.0001 } else if t < 0.2 { (1.0 - 0.9999 * (2.0 - 10.0 * t).powi(3)).clamp(0.0001, 1.0) } else { 1.0 };
                    let mut o = [0.0f32; 2];
                    for (c, x) in [dl, dr].into_iter().enumerate() {
                        let z = &mut self.lf[c];
                        let mut y = x + rg * z[12];
                        for k in 0..3 {
                            let out = main[0] * y + main[1] * z[k * 2] - main[2] * z[k * 2 + 1];
                            z[k * 2] = y;
                            z[k * 2 + 1] = out;
                            y = out;
                        }
                        let mut q = y;
                        for k in 3..6 {
                            let out = res[0] * q + res[1] * z[k * 2] - res[2] * z[k * 2 + 1];
                            z[k * 2] = q;
                            z[k * 2 + 1] = out;
                            q = out;
                        }
                        z[12] = sat(q);
                        o[c] = y + rg * q;
                    }
                    mixed(o[0], o[1])
                }
                Flanger => {
                    // A feedback comb whose delay a triangle LFO sweeps between one sample
                    // and 2.6667 ms once per beat value.
                    let t = if !multi && b2 { 1.0 - tri } else { tri };
                    let d = 1.0 + t * if multi { 1.0 } else { (k2 * 2.0).min(1.0) } * (0.0026667 * r - 1.0);
                    let (el, er) = (self.tap(d, 0), self.tap(d, 1));
                    self.push(dl + el * fl_fb, dr + er * fl_fb);
                    mixed(el, er)
                }
                Phaser => {
                    // First-order all-pass stages (six in multi mode), their centre swept by
                    // a triangle LFO: 300 Hz .. 8.06 kHz in multi mode (rekordbox's bottom and
                    // peak), 100 Hz .. 15 kHz in single mode (its limits).
                    let t = if !multi && b2 { 1.0 - tri } else { tri };
                    let fc = if multi { 299.96 * (8058.92f32 / 299.96).powf(t) } else { 100.0 * (15000.0f32 / 100.0).powf(t) };
                    let tn = (PI * (fc / r).min(0.49)).tan();
                    let a = (tn - 1.0) / (tn + 1.0);
                    let mut o = [0.0f32; 2];
                    for (c, x) in [dl, dr].into_iter().enumerate() {
                        let mut y = x + self.fb[c] * ph_fb;
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
                    mixed(o[0], o[1])
                }
                Roll | RevRoll | SlipRoll => {
                    let (mut ol, mut or) = (dl, dr);
                    if let Some((start, len, pos)) = self.roll.as_mut() {
                        let i = *pos as usize % *len;
                        let j = if kind == RevRoll { *len - 1 - i } else { i };
                        let k = (*start + j) % RING_LEN;
                        // A few frames of fade at the loop's ends, so the splice is silent;
                        // single mode's DUTY under its centre cuts the end of every turn short.
                        let edge = (i.min(*len - 1 - i) as f32 / (0.002 * r)).min(1.0);
                        let duty = if !multi && k1 < 0.5 && (i as f32) > *len as f32 * (k1 * 2.0).max(0.05) { 0.0 } else { 1.0 };
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
                    mixed(ol, or)
                }
                Robot => {
                    // The sound shifted in pitch, with a short comb for the metallic voice at
                    // the modulation amount (the comb is recordbox's own; rekordbox's
                    // `RobotCore` modulation was not traced). The plain sound and the voice
                    // are at rekordbox's gains.
                    let (pl, pr) = self.shift(dl, dr, robot_ratio);
                    let d = 0.004 * r;
                    let (cl, cr) = (self.tap(d, 0), self.tap(d, 1));
                    let fbk = 0.9 * rob_mod;
                    self.push(pl + cl * fbk, pr + cr * fbk);
                    mixed(dl * rob_a + (pl + cl * fbk) * rob_b, dr * rob_a + (pr + cr * fbk) * rob_b)
                }
                Pitch => {
                    let (pl, pr) = self.shift(dl, dr, pitch_r);
                    mixed(pl, pr)
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
                    // The beat value held as a loop the input is laid over: each turn keeps
                    // rekordbox's share of the loop and lets in its share of the input, so
                    // fully up the loop is held and the input is shut out. Single mode: SPEED
                    // plays the loop higher or lower (rekordbox's 1.38 and 1.78 as the ends).
                    let (el, er) = (self.tap(self.time, 0), self.tap(self.time, 1));
                    let (yl, yr) = (il * helix_in + el * helix_keep, ir * helix_in + er * helix_keep);
                    self.push(yl, yr);
                    let ratio = if multi { 1.0 } else if k2 >= 0.5 { 1.0 + (k2 - 0.5) * 2.0 * 0.78 } else { 1.0 - (0.5 - k2) * 2.0 * (1.0 - 1.0 / 1.38) };
                    let (sl, sr) = if (ratio - 1.0).abs() > 0.004 { self.shift(el, er, ratio) } else { (el, er) };
                    mixed(dl * helix_in + sl * helix_keep, dr * helix_in + sr * helix_keep)
                }
                VinylBrake => {
                    // The sound slows to a stop like a record with the motor off: rekordbox's
                    // brake curve over the beat value. ON: it starts again and brakes again,
                    // REPEAT sets how often. ECHO: an echo of the braking sound. BPF: a
                    // band-pass on it.
                    let total = (secs.min(12.0) * r).max(1.0);
                    self.speed = brake_step(self.speed, secs.min(12.0), r);
                    let sp = self.speed;
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
                        self.speed = 1.0;
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
                    // rekordbox's FX+ echoes: the main knob (read in 8-bit steps) moves a
                    // filter (or the crusher and its high-pass) and the share of the filtered
                    // sound; MOD wobbles the knob with a fast LFO; ECHO MIX brings in up to
                    // four echoes of the result (a quarter, a half, three quarters of a beat
                    // and a beat; feedback 0.75).
                    self.lfo2 = (self.lfo2 + fxp_lfo_step).fract();
                    let v = (fxp_v - fxp_depth * (TAU * self.lfo2).sin().max(0.0)).clamp(0.0, 255.0);
                    let (pd, pw) = fxplus_mix(kind, v);
                    let fc = fxplus_fc(kind, v);
                    let gf = svf_g(fc, r);
                    let (mut sl, mut sr) = (dl, dr);
                    if kind == CrushEcho {
                        let hold = (1.0 + v * 0.0019608 * 0.010 * r).max(1.0);
                        self.hold_n += 1.0;
                        if self.hold_n >= hold {
                            self.hold_n -= hold;
                            self.hold = [dl, dr];
                        }
                        (sl, sr) = (self.hold[0], self.hold[1]);
                    }
                    let q = match kind {
                        LpfEcho => (1.5 * (1.0 - ((v - 3.0).max(0.0) / 252.0).powi(16))).max(0.5),
                        BpfEcho => 0.65,
                        _ => (1.4 * (1.0 - ((255.0 - v) / 252.0).powi(16))).max(0.5),
                    };
                    let a = svf(&mut self.f2[0], sl, gf, 1.0 / q);
                    let b = svf(&mut self.f2[1], sr, gf, 1.0 / q);
                    let (fl, fr) = match kind {
                        LpfEcho => (a.0, b.0),
                        BpfEcho => (a.1 / q, b.1 / q),
                        _ => (a.2, b.2),
                    };
                    let (ml, mr) = (dl * (1.0 + (pd - 1.0) * g) + fl * pw * g, dr * (1.0 + (pd - 1.0) * g) + fr * pw * g);
                    let (mut el, mut er) = (0.0, 0.0);
                    for i in 0..4 {
                        let d = ((i + 1) as f32 * 0.25 * beat_secs * r).clamp(0.010 * r, (PART_LEN - 2) as f32);
                        let (tl2, tr2) = (self.part_tap(i, d, 0), self.part_tap(i, d, 1));
                        let (fl2, fr2) = if b1 && !multi { (tr2, tl2) } else { (tl2, tr2) };
                        let (inl, inr) = if b2 && !multi { (0.0, 0.0) } else { (ml * 0.7 * target, mr * 0.7 * target) };
                        self.part_put(i, inl + fl2 * 0.75, inr + fr2 * 0.75);
                        el += tl2 * fxp_lvl[i];
                        er += tr2 * fxp_lvl[i];
                    }
                    self.w = (self.w + 1) % DELAY_LEN;
                    (ml + el * g, mr + er * g)
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
pub(crate) struct Ring {
    pub(crate) buf: Vec<f32>,
    pub(crate) w: usize,
}

impl Ring {
    pub(crate) fn new() -> Ring {
        Ring { buf: vec![0.0; RING_LEN * 2], w: 0 }
    }

    /// The history at a fractional frame index (wrapping).
    #[inline]
    pub(crate) fn at(&self, idx: f64) -> (f32, f32) {
        let f = idx.rem_euclid(RING_LEN as f64);
        let i = f as usize % RING_LEN;
        let j = (i + 1) % RING_LEN;
        let t = (f - f.floor()) as f32;
        (self.buf[i * 2] + (self.buf[j * 2] - self.buf[i * 2]) * t, self.buf[i * 2 + 1] + (self.buf[j * 2 + 1] - self.buf[i * 2 + 1]) * t)
    }

    pub(crate) fn write(&mut self, buf: &[f32]) {
        for f in buf.chunks_exact(2) {
            self.buf[self.w * 2] = f[0];
            self.buf[self.w * 2 + 1] = f[1];
            self.w = (self.w + 1) % RING_LEN;
        }
    }
}

/// RELEASE FX state for one channel.
struct Release {
    kind: u8,
    /// Frames since it was pressed, where in the history playback stands (frames from the
    /// press) and the brake's speed.
    t: f32,
    pos: f64,
    speed: f32,
    start: usize,
    gate: f32,
    held: bool,
}

pub(crate) struct Channel {
    /// The three multi-mode slots, then single mode's effect.
    slots: [Slot; 4],
    ring: Ring,
    release: Release,
}

impl Channel {
    fn new() -> Channel {
        Channel { slots: [Slot::new(), Slot::new(), Slot::new(), Slot::new()], ring: Ring::new(), release: Release { kind: 0, t: 0.0, pos: 0.0, speed: 1.0, start: 0, gate: 0.0, held: false } }
    }

    /// A channel that only ever runs its first slot (PAD FX).
    fn one_slot() -> Channel {
        Channel { slots: [Slot::new(), Slot::empty(), Slot::empty(), Slot::empty()], ..Channel::new_without_slots() }
    }

    fn new_without_slots() -> Channel {
        Channel { slots: [Slot::empty(), Slot::empty(), Slot::empty(), Slot::empty()], ring: Ring::new(), release: Release { kind: 0, t: 0.0, pos: 0.0, speed: 1.0, start: 0, gate: 0.0, held: false } }
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
                s.process(&sp, p.bpm[ch], buf, rate, &self.ring, beat, i < 3);
            }
        }
        // RELEASE FX: press and hold. The time is the beat value (rekordbox: VINYL BRAKE
        // 10 ms .. 12 s, ECHO 10 ms .. 4 s, BACK SPIN's stop time, with two beats as the
        // length it spins back over).
        if let Some((kind, beats)) = p.release {
            let beat_secs = 60.0 / p.bpm[ch].clamp(20.0, 1000.0);
            if !self.release.held {
                // (The history already has this buffer: the brake starts at its first frame.)
                let start = (self.ring.w + RING_LEN - frames % RING_LEN) % RING_LEN;
                self.release = Release { kind, t: 0.0, pos: 0.0, speed: 1.0, start, gate: self.release.gate, held: true };
            }
            let secs = (beats * beat_secs).clamp(0.010, 12.0);
            let total = secs * r;
            let two_beats = (2.0 * beat_secs).clamp(0.010, 4.0);
            let gk = 1.0 / (SMOOTH_SECS * r);
            for n in 0..frames {
                let rel = &mut self.release;
                rel.gate = (rel.gate + gk).min(1.0);
                let (wl, wr) = match rel.kind {
                    1 => {
                        // ECHO: the input is cut and the last beat repeats, each time at 0.7.
                        let len = (total.min(4.0 * r) as usize).clamp(1, RING_LEN - 1);
                        let i = rel.t as usize;
                        let turn = i / len;
                        let k = (rel.start + RING_LEN - len + i % len) % RING_LEN;
                        let a = 0.7f32.powi(turn as i32 + 1);
                        (self.ring.buf[k * 2] * a, self.ring.buf[k * 2 + 1] * a)
                    }
                    2 => {
                        // BACK SPIN: backwards over the last two beats at rekordbox's speed curve.
                        let sp = back_spin_speed(rel.t / r, secs, two_beats);
                        rel.pos -= sp as f64;
                        let span = (two_beats * r) as f64;
                        if rel.pos < -span {
                            rel.pos += span;
                        }
                        let (l, rr) = self.ring.at(rel.start as f64 + rel.pos);
                        let a = if rel.t < 4.0 { 1.0 } else { (sp * 4.0).min(1.0) };
                        (l * a, rr * a)
                    }
                    _ => {
                        // VINYL BRAKE: the sound goes on, ever slower, to a stop.
                        rel.speed = brake_step(rel.speed, secs, r);
                        rel.pos += rel.speed as f64;
                        let (l, rr) = self.ring.at(rel.start as f64 + rel.pos);
                        let a = (rel.speed * 8.0).min(1.0);
                        (l * a, rr * a)
                    }
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
        self.ring.write(buf);
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

/// What a deck's PAD FX pads ask for at the moment (set by the interface): a Beat FX or
/// Scene FX (as a multi-mode slot), a Sound Color FX (effect, COLOR, PARAM) and a RELEASE FX
/// (which one and its beats).
#[derive(Clone, Copy)]
pub struct PadSet {
    pub beat: Option<SlotParams>,
    pub color: Option<(u8, f32, f32)>,
    pub release: Option<(u8, f32)>,
}

impl PadSet {
    pub const NONE: PadSet = PadSet { beat: None, color: None, release: None };

    pub fn any(&self) -> bool {
        self.beat.is_some() || self.color.is_some() || self.release.is_some()
    }
}

/// A deck's PAD FX unit: one effect, one Sound Color FX and the release effects.
struct PadChan {
    ch: Channel,
    cfx: crate::cfx::Cfx,
    /// The last Beat FX, kept so it can fade out, and the colour effect in use.
    last: SlotParams,
    color: (u8, f32),
    /// Frames since anything was asked of it (it rests once its tails are over).
    idle: u32,
}

/// Both FX units and the scratch buffers the decks are rendered into on their way through them.
pub struct Rack {
    pub units: [Unit; 2],
    pub scratch: [Vec<f32>; 4],
    /// The interface has the FX panel in use (else the audio path skips the units).
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
    /// PAD FX: a unit per deck (made by the interface when a deck first shows PAD FX), what
    /// its pads ask for, the deck's tempo and its Beat FX quantize.
    pads: [Option<Box<PadChan>>; 4],
    pub pad_set: [PadSet; 4],
    pub pad_bpm: [f32; 4],
    pub pad_quantize: [bool; 4],
    /// MERGE FX (made by the interface when it is first enabled).
    pub merge: Option<Box<crate::mfx::Engine>>,
    /// The master's RELEASE FX (Preferences > Controller > Effect > Unit Number 1): which one
    /// and its beats while held, the tempo, and its state (made by the interface).
    pub master_release: Option<(u8, f32)>,
    pub master_bpm: f32,
    master_rel: Option<Box<Channel>>,
}

impl Rack {
    pub fn new() -> Rack {
        let unit = || Unit { params: UnitParams::new(), channels: [None, None, None, None, None, None] };
        Rack { units: [unit(), unit()], scratch: std::array::from_fn(|_| vec![0.0; 16384]), enabled: false, master_cfx: crate::cfx::Cfx::new(), master_cfx_set: (0, 0.5, 0.5), sampler_cfx: crate::cfx::Cfx::new(), sampler_cfx_set: (0, 0.5, 0.5), beat: [None; CHANNELS], pads: [None, None, None, None], pad_set: [PadSet::NONE; 4], pad_bpm: [120.0; 4], pad_quantize: [false; 4], merge: None, master_release: None, master_bpm: 120.0, master_rel: None }
    }

    /// Make deck `ch`'s PAD FX unit (about 8 MB; called by the interface, never the audio thread).
    pub fn prepare_pad(&mut self, ch: usize) {
        if ch < 4 && self.pads[ch].is_none() {
            self.pads[ch] = Some(Box::new(PadChan { ch: Channel::one_slot(), cfx: crate::cfx::Cfx::new(), last: SlotParams::new(Kind::Delay), color: (0, 0.5), idle: u32::MAX }));
        }
    }

    /// Make the master's RELEASE FX state (about 6 MB; called by the interface).
    pub fn prepare_master_release(&mut self) {
        if self.master_rel.is_none() {
            self.master_rel = Some(Box::new(Channel::new_without_slots()));
        }
    }

    pub fn has_pad(&self, ch: usize) -> bool {
        ch < 4 && self.pads[ch].is_some()
    }

    fn pads_busy(&self) -> bool {
        self.pads.iter().zip(&self.pad_set).any(|(p, s)| p.as_ref().is_some_and(|p| s.any() || p.idle < 20 * 48000))
    }

    /// Something is assigned and switched on (or held, or still fading): the effects must run.
    pub fn active(&self) -> bool {
        self.enabled && (self.units.iter().any(|u| u.params.assign.iter().any(|a| *a)) || (self.master_cfx_set.1 - 0.5).abs() > 0.011 || (self.sampler_cfx_set.1 - 0.5).abs() > 0.011) || self.pads_busy() || self.merge.as_ref().is_some_and(|m| m.busy()) || self.master_rel.as_ref().is_some_and(|c| self.master_release.is_some() || c.release.gate > 0.0)
    }

    /// The sampler's mix has to come through here (a unit is assigned to it, or its CFX is on).
    pub fn sampler_in_use(&self) -> bool {
        self.enabled && (self.units.iter().any(|u| u.params.assign[SAMPLER]) || (self.sampler_cfx_set.1 - 0.5).abs() > 0.011)
    }

    /// Run one channel's audio (stereo interleaved) through what is assigned to it.
    /// `ch`: 0..3 = decks 1 to 4, 4 = the master, 5 = the sampler. A deck's order: MERGE FX,
    /// its PAD FX unit, then FX1 and FX2.
    pub fn process(&mut self, ch: usize, buf: &mut [f32], rate: u32) {
        if ch == MASTER {
            if let Some(m) = self.merge.as_deref_mut() {
                m.master(buf, rate);
            }
            if self.enabled {
                let (k, c, p) = self.master_cfx_set;
                self.master_cfx.process(k, c, p, buf, rate);
            }
            if let Some(c) = self.master_rel.as_deref_mut() {
                c.remember(buf);
                let mut up = UnitParams::new();
                up.slots.iter_mut().for_each(|s| s.on = false);
                up.release = self.master_release;
                up.bpm = [self.master_bpm; CHANNELS];
                c.process(&up, MASTER, buf, rate, None);
            }
        }
        if ch == SAMPLER && self.enabled && (self.sampler_cfx_set.1 - 0.5).abs() > 0.011 {
            let (k, c, p) = self.sampler_cfx_set;
            self.sampler_cfx.process(k, c, p, buf, rate);
        }
        if ch < 4 {
            if let Some(m) = self.merge.as_deref_mut() {
                m.deck(ch, buf, rate, self.beat[ch]);
            }
            let set = self.pad_set[ch];
            if let Some(pc) = self.pads[ch].as_deref_mut() {
                if set.any() {
                    pc.idle = 0;
                } else {
                    pc.idle = pc.idle.saturating_add((buf.len() / 2) as u32);
                }
                if pc.idle < 20 * 48000 {
                    pc.ch.remember(buf);
                    if let Some((k, c, p)) = set.color {
                        pc.color = (k, p);
                        pc.cfx.process(k, c, p, buf, rate);
                    } else {
                        // (Centre: the effect glides out and leaves the path.)
                        pc.cfx.process(pc.color.0, 0.5, pc.color.1, buf, rate);
                    }
                    let mut up = UnitParams::new();
                    match set.beat {
                        Some(sp) => {
                            pc.last = sp;
                            up.slots[0] = sp;
                        }
                        None => up.slots[0] = SlotParams { on: false, ..pc.last },
                    }
                    up.slots[1].on = false;
                    up.slots[2].on = false;
                    up.release = set.release;
                    up.bpm = [self.pad_bpm[ch]; CHANNELS];
                    let beat = if self.pad_quantize[ch] { self.beat[ch] } else { None };
                    pc.ch.process(&up, ch, buf, rate, beat);
                }
            }
        }
        if !self.enabled {
            return;
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
        rack.units[0].params = unit(Kind::Trans, 1.0, 0.5);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(48000, 440.0);
        rack.process(0, &mut x, 48000);
        // (LEVEL/DEPTH at the centre: the gate alone, open half of the turn.)
        // 1 beat at 120 BPM = 24,000 frames: open for the first half, closed for the second.
        assert!(rms(&x[2000 * 2..10000 * 2]) > 0.3);
        assert!(rms(&x[14000 * 2..22000 * 2]) < 0.01);
        assert!(rms(&x[26000 * 2..34000 * 2]) > 0.3);
        // Fully up it is open for a sixteenth of the turn (1,500 frames).
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Trans, 1.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(48000, 440.0);
        rack.process(0, &mut x, 48000);
        assert!(rms(&x[300 * 2..1200 * 2]) > 0.3 && rms(&x[2000 * 2..23000 * 2]) < 0.01);
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
        // 440 Hz at +100 % is 880 Hz, at -50 % 330 Hz (rekordbox's law: -100 % is the octave down).
        // LEVEL/DEPTH at the centre gives the value as labelled; fully up doubles it.
        for (pct, want) in [(100.0, 880.0), (-50.0, 330.0), (26.0, 554.4)] {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params = unit(Kind::Pitch, pct, 0.5);
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
        // DELAY, LR RATIO at the left end (1:3): the left repeat comes at a third of the time.
        let x = run([0.5, 0.0], [false, false], Kind::Delay);
        assert!(x[8100 * 2] > 0.3 && x[8100 * 2 + 1].abs() < 0.01 && x[24100 * 2 + 1] > 0.3);
        // At the centre (1:1) both come together.
        let x = run([0.5, 0.5], [false, false], Kind::Delay);
        assert!(x[24100 * 2] > 0.3 && x[24100 * 2 + 1] > 0.3 && x[8100 * 2].abs() < 0.01);
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
            let mut p = unit(Kind::Trans, 1.0, 0.5);
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
        // VINYL BRAKE, 1 beat, LEVEL/DEPTH fully up, ECHO at zero: slowing over the beat and
        // (rekordbox's tail below 1/8 speed) stopped 0.6 s after it.
        let mut rack = Rack::new();
        rack.enabled = true;
        p.one = SlotParams { on: true, level: 1.0, value: 1.0, knob: [0.5, 0.0], ..SlotParams::new(Kind::VinylBrake) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(96000, 440.0);
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        assert!(rms(&x[2000 * 2..8000 * 2]) > 0.2 && rms(&x[56000 * 2..]) < 0.01);
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
            let mut x = tone(96000, 440.0);
            rack.process(0, &mut x, 48000);
            let peak = x.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!(peak.is_finite() && peak < 2.0);
            assert!(rms(&x[2000 * 2..6000 * 2]) > 0.05, "release {kind} is heard");
            if kind != 1 {
                // The brake (1 beat = 0.5 s, then its 0.6 s tail) and the back spin (0.67 s)
                // have stopped.
                assert!(rms(&x[56000 * 2..]) < 0.01, "release {kind} ends in silence");
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

    #[test]
    fn traced_laws() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        // TRANS / PAN, FLANGER / PHASER, FILTER, ROLL mixes.
        assert_eq!((half_mix(0.25), half_mix(0.5), half_mix(1.0)), ((0.5, 0.5), (0.0, 1.0), (0.0, 1.0)));
        assert_eq!((quarter_mix(0.1), quarter_mix(0.25), quarter_mix(1.0)), ((0.9, 0.3), (0.75, 0.75), (0.75, 0.75)));
        assert!(near(mod_fb(0.25), 0.0) && near(mod_fb(1.0), 0.8) && mod_fb(0.1) == 0.0);
        assert!(near(trans_duty(0.3), 0.5) && near(trans_duty(1.0), 0.0625) && near(trans_duty(0.75), 0.28125));
        assert!(near(filter_mix(0.0).0, 1.0) && filter_mix(0.2) == (0.0, 1.0) && near(filter_mix(0.078125).0, 0.5));
        assert!(near(filter_reso(1.0), 0.96) && near(filter_reso(0.5), 0.72) && filter_reso(0.2) < 0.001);
        assert!(roll_mix(0.0) == (1.0, 0.0) && roll_mix(0.6) == (0.0, 1.0) && near(roll_mix(0.25).0, 0.748039) && near(roll_mix(0.25).1, 0.748039));
        assert!(near(roll_mix(0.03125).0, 0.996063) && near(roll_mix(0.46875).0, 0.1142));
        // UP ECHO: nothing bent up to the centre, 16.667 % a repeat fully up; the percent law.
        assert!(up_echo(0.25) == (0.35, 0.0) && near(up_echo(1.0).1, 16.666) && up_echo(1.0).0 == 0.7);
        assert!(near(pitch_ratio(-100.0), 0.5) && near(pitch_ratio(100.0), 2.0) && near(pitch_ratio(200.0), 4.0) && near(pitch_ratio(-50.0), 0.75) && pitch_ratio(0.0) == 1.0);
        // SPIRAL: feedback 0.8 over the middle, past 1 fully up while on, under 1 once off.
        assert!(near(spiral_law(0.05, true).1, 0.4) && near(spiral_law(0.3, true).1, 0.8) && near(spiral_law(0.75, true).1, 1.0) && near(spiral_law(1.0, true).1, 1.2));
        assert!(near(spiral_law(1.0, false).1, 0.993) && near(spiral_law(0.75, false).1, 0.8965) && near(spiral_law(1.0, true).2, 0.499) && near(spiral_law(0.6, true).0, 0.7998));
        // HELIX: through at 0, the loop held and the input shut fully up.
        assert!(helix_law(0.0) == (0.0, 1.0) && near(helix_law(0.5).0, 0.85) && near(helix_law(0.5).1, 0.45) && near(helix_law(1.0).0, 1.0) && near(helix_law(1.0).1, 0.0));
        assert!(robot_law(0.0) == (1.0, 0.0, 0.0) && robot_law(0.5).0 == 0.75 && near(robot_law(1.0).2, 0.8));
        // REVERB: the size and input gain laws, and the decay time at a few values (rekordbox's
        // tables give 765 ms at 1 %, 9.08 s at 50 %, 249.6 s at 100 %).
        assert!(near(reverb_size(50.0), 1.0) && near(reverb_size(1.0), 0.3826) && near(reverb_size(100.0), 2.0805));
        assert!(near(reverb_in_gain(55.0), 0.2) && near(reverb_in_gain(1.0), 0.0072) && near(reverb_in_gain(100.0), 0.0826));
        for (p, want) in [(1.0, 765.0), (22.0, 1329.0), (50.0, 9080.0), (78.0, 69970.0), (100.0, 249600.0)] {
            assert!((reverb_rt60_ms(p) / want - 1.0).abs() < 0.05, "{p}: {}", reverb_rt60_ms(p));
        }
        // The brake: a straight line to 1/8 speed, then 0.20559 a second.
        let mut sp = 1.0f32;
        for _ in 0..24000 {
            sp = brake_step(sp, 1.0, 48000.0);
        }
        assert!(near(sp, 0.5));
        for _ in 0..24000 {
            sp = brake_step(sp, 1.0, 48000.0);
        }
        assert!(near(sp, 0.125 - 0.20559 * 0.125), "{sp}");
        // The back spin: 8 times speed after 0.16 of two beats, 0.28235 at the knee, then stopped.
        assert!(near(back_spin_speed(0.16, 2.0, 1.0), 8.0) && near(back_spin_speed(0.08, 2.0, 1.0), 4.0));
        assert!(near(back_spin_speed(0.16 + 1.84 * (8.0 - 0.28235) / 8.0, 2.0, 1.0), 0.28235) && back_spin_speed(2.2, 2.0, 1.0) == 0.0);
        assert!(near(back_spin_speed(0.25, 0.5, 4.0), 8.0), "a stop time under twice the climb splits in half");
        // The FX+ filters' ends and mixes.
        assert!(near(fxplus_fc(Kind::LpfEcho, 4.0), 20000.0) && (fxplus_fc(Kind::LpfEcho, 255.0) - 96.0).abs() < 2.0);
        assert!((fxplus_fc(Kind::HpfEcho, 255.0) - 9000.0).abs() < 60.0 && (fxplus_fc(Kind::BpfEcho, 255.0) - 8700.0).abs() < 60.0 && (fxplus_fc(Kind::CrushEcho, 255.0) - 4000.0).abs() < 30.0);
        assert!(fxplus_mix(Kind::LpfEcho, 0.0) == (1.0, 0.0) && fxplus_mix(Kind::LpfEcho, 100.0) == (0.0, 1.0) && near(fxplus_mix(Kind::LpfEcho, 255.0).1, 0.5));
        assert!(fxplus_mix(Kind::BpfEcho, 0.0) == (1.0, 0.0) && fxplus_mix(Kind::BpfEcho, 255.0).0 == 1.0 && near(fxplus_mix(Kind::HpfEcho, 255.0).0, 0.0));
        // The filter tables' ends.
        assert!((knots(&LFO_LPF_MAIN, 0.0) - 23718.6).abs() < 1.0 && (knots(&LFO_LPF_MAIN, 1.0) - 485.0).abs() < 0.1 && (knots(&LFO_HPF_MAIN, 0.5) - 969.9).abs() < 0.1);
    }

    fn burst(n: usize) -> Vec<f32> {
        (0..n).flat_map(|i| { let v = if i < 480 { 0.5 } else { 0.0 }; [v, v] }).collect()
    }

    #[test]
    fn ping_pong_up_echo_and_low_cut_echo() {
        let run = |kind: Kind, level: f32, x: &mut Vec<f32>| {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params = unit(kind, 1.0, level);
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            rack.process(0, x, 48000);
        };
        // PING PONG, 1 beat: the left repeat a beat later, the right one at three quarters.
        let mut x = burst(48000 * 2);
        run(Kind::PingPong, 0.75, &mut x);
        assert!(x[24100 * 2] > 0.3 && x[24100 * 2 + 1].abs() < 0.01 && x[18100 * 2 + 1] > 0.3 && x[18100 * 2].abs() < 0.01);
        // Its second repeats at 0.7.
        assert!((x[48100 * 2] / x[24100 * 2] - 0.7).abs() < 0.03 && (x[36100 * 2 + 1] / x[18100 * 2 + 1] - 0.7).abs() < 0.03);
        // UP ECHO at the centre: repeats at 0.85 of each other, at 0.7 of the sound, unbent.
        let mut x = burst(48000 * 2);
        run(Kind::UpEcho, 0.5, &mut x);
        assert!((x[24100 * 2] - 0.35).abs() < 0.02 && (x[48100 * 2] / x[24100 * 2] - 0.85).abs() < 0.03);
        // LOW CUT ECHO: a 100 Hz tone is not repeated, a 2 kHz one is.
        for (freq, repeated) in [(100.0, false), (2000.0, true)] {
            let mut x: Vec<f32> = tone(12000, freq);
            x.extend(std::iter::repeat(0.0).take(60000 * 2));
            run(Kind::LowCutEcho, 0.75, &mut x);
            let echo = rms(&x[26000 * 2..34000 * 2]);
            assert_eq!(echo > 0.1, repeated, "{freq} Hz: {echo}");
        }
    }

    #[test]
    fn reverb_decays_at_its_traced_rate() {
        // REVERB at 25 %: a decay time of about 1.5 s, so half a second later the tail is
        // about 20 dB down, and it has not died within the first second.
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Reverb, 25.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x: Vec<f32> = (0..48000 * 3).flat_map(|i| { let v = if i < 2400 { ((i * 7919 % 1000) as f32 / 500.0 - 1.0) * 0.5 } else { 0.0 }; [v, v] }).collect();
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        let db = |a: usize| 20.0 * rms(&x[a * 2..(a + 9600) * 2]).log10();
        let fall = db(12000) - db(36000);
        assert!(db(12000) > -60.0, "{}", db(12000));
        assert!((fall - 20.0).abs() < 7.0, "fell {fall} dB in 0.5 s");
    }

    #[test]
    fn beat_fx_filter_sweeps_down_to_485_hz() {
        // FILTER, 1 beat, LEVEL/DEPTH at 0.3 (the filter alone, almost no resonance): at the
        // start of a turn the low-pass is wide open, half a beat in it is at 485 Hz, so a
        // 5 kHz tone is passed and then cut by about 60 dB (three first-order sections).
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Filter, 1.0, 0.3);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(24000, 5000.0);
        for chunk in x.chunks_mut(64 * 2) {
            rack.process(0, chunk, 48000);
        }
        let open = rms(&x[300 * 2..900 * 2]);
        let shut = rms(&x[11700 * 2..12300 * 2]);
        assert!(open > 0.25, "{open}");
        assert!(20.0 * (shut / open).log10() < -45.0, "{}", 20.0 * (shut / open).log10());
    }

    #[test]
    fn pad_fx_unit_runs_what_its_pads_hold() {
        let mut rack = Rack::new();
        rack.prepare_pad(0);
        assert!(rack.has_pad(0) && !rack.active());
        // A held ECHO pad (1/2 beat, LEVEL/DEPTH 30): the rack is in use and the echo is heard.
        rack.pad_set[0].beat = Some(SlotParams { on: true, level: 0.3, value: 0.5, ..SlotParams::new(Kind::Echo) });
        assert!(rack.active());
        let mut x = burst(48000);
        rack.process(0, &mut x, 48000);
        assert!((x[12100 * 2] - 0.5 * 1.3333 * 0.3).abs() < 0.02, "{}", x[12100 * 2]);
        // Let go: the pad's effect fades out and the sound is the dry one again.
        rack.pad_set[0] = PadSet::NONE;
        let dry = tone(9600, 440.0);
        let mut y = dry.clone();
        rack.process(0, &mut y, 48000);
        assert!(y[2000..] == dry[2000..]);
        // A release pad takes the sound over; a colour pad (LPF fully turned) dulls it.
        rack.pad_set[0].release = Some((1, 0.5));
        let mut y = vec![0.0f32; 9600 * 2];
        rack.process(0, &mut y, 48000);
        assert!(rms(&y[2000..]) > 0.1, "the release echo repeats what was playing");
        rack.pad_set[0] = PadSet { color: Some((crate::cfx::LPF, 1.0, 0.5)), ..PadSet::NONE };
        let mut y = tone(48000, 5000.0);
        rack.process(0, &mut y, 48000);
        assert!(rms(&y[40000 * 2..]) < 0.01);
        // Nothing asked for 20 s: the unit rests and the rack is no longer in use.
        rack.pad_set[0] = PadSet::NONE;
        let mut y = tone(48000, 440.0);
        for _ in 0..21 {
            rack.process(0, &mut y, 48000);
        }
        assert!(!rack.active());
    }

    #[test]
    fn release_vinyl_brake_starts_at_the_sound_being_played() {
        // (Found by another track: the brake used to play 15-second-old audio.) The history is
        // a ramp and the sound goes on as the same ramp while the brake is held: its first
        // output frames are the dry signal at the press, falling behind as it slows.
        let ramp = |from: usize, n: usize| -> Vec<f32> { (from..from + n).flat_map(|i| { let v = i as f32 / 100000.0; [v, v] }).collect() };
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params.assign = [true, false, false, false, false, false];
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut hist = ramp(0, 48000);
        rack.process(0, &mut hist, 48000);
        rack.units[0].params.release = Some((0, 4.0));
        let mut x = ramp(48000, 4800);
        let dry = x.clone();
        rack.process(0, &mut x, 48000);
        // After the 3.33 ms fade-in the output is the dry sound within a frame or two (4 beats
        // at 120 BPM = 2 s: 300 frames in the playback is 0.5 frame behind).
        for f in [200usize, 300, 600] {
            assert!((x[f * 2] - dry[f * 2]).abs() < 3.0 / 100000.0, "frame {f}: {} vs {}", x[f * 2], dry[f * 2]);
        }
        // Later it is clearly behind the live sound (slower), never ahead, never the old history.
        assert!(x[4700 * 2] < dry[4700 * 2] && x[4700 * 2] > dry[4000 * 2]);
    }
}
