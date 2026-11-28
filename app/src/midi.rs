//! MIDI controllers in PERFORMANCE mode: the functions recordbox can be told to do by a
//! controller (the registry), the MIDI ports (CoreMIDI through `midir`), the worker of each
//! port, LED feedback, and what a mapped message does to the decks, the mixer, the FX units,
//! the sampler and the browser. After rekordbox 7.2.19 (`checks/re-7.2.19/controllers-midi.md`).
//!
//! Threads: the port's callback only queues the message; a worker per port looks it up in the
//! device's map (`midi_map.rs`). Rows with the `Fast` option (transport, pads, faders) and the
//! jog wheel act on the audio engine from the worker at once; every message also goes to the
//! interface thread (`poll`), which keeps the app's own state in step and does everything
//! else. An output thread per port sends LED messages one per `MidiOutInterval`, highest
//! priority first, and blinks.
//!
//! Function names are rekordbox's (the `Name` column of its mapping files and the names its
//! MIDI Learn window shows), so that mapping files move between the two. The list below is
//! recordbox's own: what it implements. The factory mappings and rekordbox's Learn catalogue
//! are AlphaTheta's and are not part of recordbox; factory mappings are read from the
//! installed rekordbox when a controller is connected.
//!
//! Not here on purpose: any hardware authentication handshake (controllers that need one
//! stay unsupported), HID players and PRO DJ LINK.

use crate::app::*;
use crate::audio::PlayState;
use crate::midi_csv::{Entry, Kind};
use crate::midi_map::*;
use crate::model::*;
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------- settings

/// The preferences that belong to controllers (a field of `Prefs`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MidiPrefs {
    /// rekordbox's TakeoverEqPartIso ("Takeover when switching to EQ/STEM ISO"), default off.
    pub takeover_eq_iso: bool,
    /// rekordbox's DeckNeedleLock (Lock by default): the needle strip does not seek while
    /// the deck plays.
    pub needle_lock: bool,
    /// JOG Rotation Speed: 45 rpm instead of 33 1/3.
    pub jog_45rpm: bool,
    /// The device last picked in the MIDI setting window.
    pub device: String,
}

impl Default for MidiPrefs {
    fn default() -> MidiPrefs {
        MidiPrefs { takeover_eq_iso: false, needle_lock: true, jog_45rpm: false, device: String::new() }
    }
}

// ---------------------------------------------------------------------- the registry

/// The MIDI setting window's tabs (rekordbox's order). VIDEO and LIGHTING are out of
/// recordbox's scope and have no functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Deck,
    Pad,
    Fx,
    Sampler,
    Mixer,
    Browse,
    Other,
    Video,
    Lighting,
    MixPointLink,
    /// Not offered by MIDI Learn (jog wheels, Shift): used by factory mappings only.
    None,
}

pub const TABS: [(Tab, &str); 10] = [(Tab::Deck, "DECK"), (Tab::Pad, "PAD"), (Tab::Fx, "FX"), (Tab::Sampler, "SAMPLER"), (Tab::Mixer, "MIXER"), (Tab::Browse, "BROWSE"), (Tab::Other, "OTHER"), (Tab::Video, "VIDEO"), (Tab::Lighting, "LIGHTING"), (Tab::MixPointLink, "MIXPOINTLINK")];

/// The Learn window's control types (rekordbox's numbers).
pub const T_BUTTON: u8 = 1;
pub const T_ROTARY: u8 = 2;
pub const T_KNOB7: u8 = 3;
pub const T_KNOB14: u8 = 4;
pub const T_INDICATOR: u8 = 5;
pub const T_PAD: u8 = 6;
pub const T_VALUE: u8 = 7;
pub const T_BUTTON_PAD: u8 = 8;

pub fn type_text(t: u8) -> &'static str {
    match t {
        T_BUTTON => "Button",
        T_ROTARY => "Rotary",
        T_KNOB7 => "Knob/Slider(0h-7Fh)",
        T_KNOB14 => "Knob/Slider(0h-3FFFh)",
        T_INDICATOR => "Indicator",
        T_PAD => "Pad",
        T_VALUE => "Value",
        T_BUTTON_PAD => "Button(for Pad)",
        _ => "",
    }
}

/// The types the Type list offers together: Button with Button(for Pad), 7-bit with 14-bit.
pub fn type_choices(t: u8) -> &'static [u8] {
    match t {
        T_BUTTON | T_BUTTON_PAD => &[T_BUTTON, T_BUTTON_PAD],
        T_KNOB7 | T_KNOB14 => &[T_KNOB7, T_KNOB14],
        T_ROTARY => &[T_ROTARY],
        T_INDICATOR => &[T_INDICATOR],
        T_PAD => &[T_PAD],
        T_VALUE => &[T_VALUE],
        _ => &[],
    }
}

/// The mapping-file type of a Learn type.
pub fn kind_of_type(t: u8) -> Kind {
    match t {
        T_ROTARY => Kind::Rotary,
        T_KNOB7 => Kind::KnobSlider,
        T_KNOB14 => Kind::KnobSliderHiRes,
        T_INDICATOR => Kind::Indicator,
        T_PAD => Kind::Pad(crate::midi_csv::PadType::ByDevice),
        T_VALUE => Kind::Value,
        _ => Kind::Button,
    }
}

/// The Learn type a mapping entry is shown as (`addMidiLearnTableData`).
pub fn type_of_entry(e: &Entry) -> u8 {
    match e.kind {
        Kind::Button => if e.opts.min == -1 && e.opts.max == -1 { T_BUTTON } else { T_BUTTON_PAD },
        Kind::KnobSlider => T_KNOB7,
        Kind::KnobSliderHiRes | Kind::Slider | Kind::Knob => T_KNOB14,
        Kind::Rotary => T_ROTARY,
        Kind::Value => T_VALUE,
        Kind::Indicator => T_INDICATOR,
        Kind::Pad(_) => T_PAD,
        _ => T_BUTTON,
    }
}

/// What a row of a mapping-file type gives its function: a press / release, an absolute
/// position, relative steps, jog counts, or a device parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InClass {
    Press,
    Abs,
    Rel,
    Jog,
    Param,
}

/// The value class of an input row's type (None: output only).
pub fn in_class(k: Kind) -> Option<InClass> {
    Some(match k {
        Kind::Button | Kind::Toggle | Kind::Pad(_) | Kind::JogTouch => InClass::Press,
        Kind::KnobSlider | Kind::KnobSliderHiRes | Kind::Slider | Kind::Knob | Kind::Value | Kind::RawValue | Kind::RotaryAsKnob => InClass::Abs,
        Kind::Rotary => InClass::Rel,
        Kind::JogRotate | Kind::JogRotateAbsPos | Kind::Difference | Kind::JogTimestamp => InClass::Jog,
        Kind::Parameter => InClass::Param,
        Kind::Indicator | Kind::IndicatorHiRes | Kind::JogIndicator | Kind::JogIndicatorHiRes => return None,
    })
}

/// The device parameters (`Parameter` rows) recordbox uses: the output pacing and the jog
/// indicator's update interval. (`JogRatency`: rekordbox's files call it "unused", and it is
/// read but not used here either.)
pub const PARAMS: [&str; 3] = ["MidiOutInterval", "JogIndicatorInterval", "JogRatency"];

/// Outputs that follow the engine and are sent by row name (`Hub::engine_outputs`).
pub const NAMED_OUTPUTS: [&str; 26] = ["JogPlayPauseIndicator", "JogPlayPauseIndicatorShort", "ChannelLevel", "LoadedIndicator", "GetLoopActive", "SlipStateForFlashing", "PlayingState", "EndWarningIndicator", "TriggerEndWarning", "OnAirIndicator", "ChannelLevel.1", "ChannelLevel.2", "ChannelLevel.3", "ChannelLevel.4", "ChannelLevel.5", "LoadedIndicatorDdjRz", "LoadedIndicatorDdjRzWithoutFlashing", "ReverseIndicator", "JogCurrentCueIndicator", "BeatIndicatorNumerator", "BeatIndicator1", "BeatIndicator2", "BeatIndicator4", "BeatIndicator8", "BeatIndicator16", "BeatIndicator32"];

/// recordbox acts on an input row of this function and type.
pub fn input_acts(name: &str, kind: Kind) -> bool {
    let Some(c) = in_class(kind) else { return false };
    if c == InClass::Param {
        return PARAMS.contains(&name);
    }
    registry().by_name(name).map(|f| act_accepts(f.act, c)).unwrap_or(false)
}

/// recordbox drives an output row of this function (a light, a meter, the jog ring).
pub fn output_source(name: &str) -> bool {
    NAMED_OUTPUTS.contains(&name) || registry().by_name(name).map(|f| act_has_led(f.act) || echo_led(f.act)).unwrap_or(false)
}

/// Which value classes an action does something with (`apply`, `Hub::fast`).
pub fn act_accepts(a: Act, c: InClass) -> bool {
    use Act as A;
    match a {
        A::SamplerVolume | A::FxUnitBeatRot(_) => matches!(c, InClass::Abs | InClass::Rel),
        A::Needle | A::Tempo | A::FxLevel(..) | A::CfxColor(_) | A::SamplerGain | A::Trim | A::Eq(_) | A::StemIsoKnob(_) | A::ChFader | A::HpMix | A::HpLevel | A::CrossFader | A::MasterLevel => c == InClass::Abs,
        A::LoopSize | A::FxReleaseType(_) | A::Browse | A::Zoom | A::LoopMoveRot => c == InClass::Rel,
        A::FxSelect(..) => matches!(c, InClass::Press | InClass::Rel | InClass::Abs),
        A::FxReleaseOn(_) => matches!(c, InClass::Press | InClass::Abs),
        A::VinylAdjust(_) | A::XfAssignKnob => c == InClass::Abs,
        A::Jog(_) | A::JogSearch => c == InClass::Jog,
        A::TempoLed(_) => false,
        A::Stub(Stub::VinylBrake(_)) => c == InClass::Abs,
        A::More(m) => crate::midi_acts::accepts(m, c),
        _ => c == InClass::Press,
    }
}

/// Buttons whose light shows the button held (they have no state to show).
pub fn echo_led(a: Act) -> bool {
    act_accepts(a, InClass::Press) && !act_has_led(a) && !matches!(a, Act::Shift | Act::JogTouch | Act::Stub(Stub::Reverse))
}

/// The actions whose state `leds` sends to an output row.
pub fn act_has_led(a: Act) -> bool {
    use Act as A;
    use crate::sampler_ext::SmpAct;
    if let A::More(m) = a {
        return crate::midi_acts::has_led(m);
    }
    matches!(
        a,
        A::FxLevel(..) | A::PlayPause | A::Cue | A::Search(_) | A::Slip | A::Quantize | A::QuantizeAll | A::MasterTempo | A::Sync | A::Master | A::KeySync | A::TempoLed(_) | A::AutoLoop | A::Loop4 | A::Reloop | A::LoopIn | A::LoopOut | A::PitchBend(_) | A::PadMode(_) | A::HotCue(_) | A::HotCueDelete(_) | A::BeatJumpPad(_) | A::BeatLoopPad(_) | A::StemPart(_) | A::StemFx(_) | A::StemSolo | A::FxOn(..) | A::FxMode(_) | A::FxAuto(_) | A::FxTap(_) | A::FxReleaseOn(_) | A::FxAssign(..) | A::SamplerPad(..) | A::SamplerPadStop(_) | A::Smp(SmpAct::Sync | SmpAct::Master | SmpAct::Quantize | SmpAct::SeqPlay | SmpAct::SeqRec) | A::SamplerCue | A::SeqCall(..) | A::HeadCue | A::XfAssign(_) | A::StemIso | A::Panel(_) | A::Related | A::Stub(Stub::Vinyl) | A::Stub(Stub::Reverse)
    )
}

/// Features other parts of recordbox are building: the registry has their commands, and
/// `stub` is the one place to connect them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stub {
    RecPanel,
    AutoMixStartStop,
    /// MIX POINT LINK: 0 panel, 1 cue select next, 2 back, 3 set, 4 exit, 5 deck select, 6 silent.
    MixPointLink(u8),
    /// PAD FX: bank (1 or 2).
    PadFxMode(u8),
    PadFx(u8, u8),
    PadFxBeat(u8, i32),
    /// Slicer: loop variant.
    SlicerMode(bool),
    SlicerPad(bool, u8),
    SlicerShift(bool, i32),
    SlicerRoll(bool, i32),
    SlicerLength(bool, i32),
    SlicerCapture,
    Reverse,
    SlipReverse,
    /// Vinyl mode on / off (the jog's own vinyl switch follows it here already).
    Vinyl,
    VinylBrake(u8),
    AutoCue,
    LoopMove(i32),
    ActiveLoop,
    SyncRate,
}

/// What a function does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    PlayPause,
    Cue,
    TrackStart,
    Track(i32),
    Search(i32),
    Needle,
    Slip,
    Quantize,
    QuantizeAll,
    MasterTempo,
    Tempo,
    TempoRange,
    /// The takeover direction LEDs: -1 TempoDown, +1 TempoUp.
    TempoLed(i32),
    TempoReset,
    Sync,
    Master,
    TapBpm,
    PitchBend(i32),
    AutoLoop,
    Loop4,
    LoopResize(f64),
    LoopSize,
    LoopIn,
    LoopOut,
    Reloop,
    ReTrigger,
    BeatLoop(f64),
    GridHalfDouble(bool),
    KeyShift(i32),
    KeySync,
    KeyReset,
    StemSolo,
    PadMode(u8),
    HotCue(u8),
    HotCueDelete(u8),
    MemoryStep(i32),
    MemorySet,
    MemoryDelete,
    BeatJumpPad(u8),
    BeatJumpPage(i32),
    BeatLoopPad(u8),
    StemPart(u8),
    StemFx(u8),
    FxLevel(u8, u8),
    FxOn(u8, u8),
    FxSelect(u8, u8, i32),
    FxBeat(u8, u8, i32),
    FxMode(u8),
    FxUnitBeat(u8, i32),
    FxUnitBeatRot(u8),
    FxAuto(u8),
    FxTap(u8),
    FxReleaseType(u8),
    FxReleaseBeat(u8, i32),
    FxReleaseOn(u8),
    FxAssign(u8, u8),
    CfxColor(u8),
    CfxSelect(i32),
    SamplerPad(u8, bool),
    SamplerPadStop(u8),
    SamplerBank(i32),
    SamplerGain,
    SamplerVolume,
    Smp(crate::sampler_ext::SmpAct),
    SamplerCue,
    SeqCall(u8, bool),
    Trim,
    Eq(u8),
    HeadCue,
    ChFader,
    HpMix,
    HpLevel,
    CrossFader,
    XfAssign(u8),
    MasterLevel,
    StemIso,
    StemIsoKnob(u8),
    Browse,
    BrowseStep(i32),
    Forward,
    Back,
    TagList,
    Related,
    Area,
    Load,
    InstDoubles,
    Sort(Col),
    Panel(u8),
    Zoom,
    /// Jog rows: true = the platter top (scratch), false = the outer ring (bend).
    Jog(bool),
    JogSearch,
    JogTouch,
    Shift,
    /// Loop Move as an encoder: one move right or left per message (`deviceLoop(int)`).
    LoopMoveRot,
    /// The top bar's panels in turn: FX, SAMPLER, MIXER, REC, none.
    PanelStep(i32),
    /// The Browse layout and back (`MainComponent::switchActiveWindow`).
    BrowseView,
    CallStep(i32),
    /// Vinyl Speed Adjust: 0 brake, 1 start, 2 both.
    VinylAdjust(u8),
    FxTapLong(u8),
    FxAssignHold(u8, u8),
    XfAssignKnob,
    CfxCenter(u8),
    /// An FX slot's effect by its place in `fx::INFO`.
    FxSelectKind(u8, u8, u8),
    Stub(Stub),
    /// The functions of `midi_acts.rs`.
    More(crate::midi_acts::M),
}

/// One function recordbox can be mapped to.
pub struct Func {
    /// rekordbox's `Name` of its mapping files.
    pub name: String,
    /// rekordbox's name for it in the MIDI setting window.
    pub display: String,
    pub tab: Tab,
    /// The ADD menu's submenu ("" = none).
    pub category: &'static str,
    pub ty: u8,
    pub per_deck: bool,
    pub act: Act,
}

pub struct Registry {
    pub funcs: Vec<Func>,
    by_name: HashMap<String, u32>,
}

impl Registry {
    /// The tag of a name (index + 1), 0 when recordbox has no such function.
    pub fn tag(&self, name: &str) -> u32 {
        self.by_name.get(name).copied().unwrap_or(0)
    }
    pub fn get(&self, tag: u32) -> Option<&Func> {
        self.funcs.get((tag as usize).wrapping_sub(1))
    }
    pub fn by_name(&self, name: &str) -> Option<&Func> {
        self.get(self.tag(name))
    }
    /// The name is one of the MIDI setting window's commands.
    pub fn learnable(&self, name: &str) -> bool {
        self.by_name(name).map(|f| f.tab != Tab::None).unwrap_or(false)
    }
    /// The commands of a tab in ADD-menu order.
    pub fn of_tab(&self, tab: Tab) -> impl Iterator<Item = &Func> {
        self.funcs.iter().filter(move |f| f.tab == tab)
    }
}

pub fn registry() -> &'static Registry {
    static R: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        let funcs = build_registry();
        let mut by_name = HashMap::new();
        for (i, f) in funcs.iter().enumerate() {
            by_name.entry(f.name.clone()).or_insert(i as u32 + 1);
        }
        Registry { funcs, by_name }
    })
}

fn build_registry() -> Vec<Func> {
    use crate::midi_acts::M;
    use crate::sampler_ext::SmpAct;
    use Act as A;
    let mut v: Vec<Func> = vec![];
    let mut add = |tab: Tab, category: &'static str, display: &str, name: &str, ty: u8, per_deck: bool, act: Act| v.push(Func { name: name.to_string(), display: display.to_string(), tab, category, ty, per_deck, act });
    let (b, rot, k7, ind, pad, val) = (T_BUTTON, T_ROTARY, T_KNOB7, T_INDICATOR, T_PAD, T_VALUE);
    const DVS: &str = "DVS (no timecode vinyl in recordbox)";
    const LINK: &str = "Ableton Link (recordbox has no Ableton Link)";
    const DRUM_FX: &str = "GROOVE CIRCUIT DRUM ROLL / TRANS: the PAD FX engine (ui_padfx.rs, the effects track's) has no entry for an effect and beat of its own on the DRUMS part";
    const ADJUST_TIME: &str = "FX ADJUST TIME: the FX units of recordbox (ui_fx.rs, the effects track's) have no time encoder (rekordbox's `rotateTimeEncoder`)";
    const DISCOVER: &str = "DISCOVER (rekordbox's recommendations from its online service: streaming, out of scope)";

    // ---- DECK (the ADD menu's order)
    let t = Tab::Deck;
    add(t, "", "PlayPause", "PlayPause", b, true, A::PlayPause);
    add(t, "", "Cue", "Cue", b, true, A::Cue);
    add(t, "", "JumpToTrackStart", "JumpToTrackStart", b, true, A::TrackStart);
    add(t, "", "TrackSearch Next", "TrackNext", b, true, A::Track(1));
    add(t, "", "TrackSearch Back", "TrackPrev", b, true, A::Track(-1));
    add(t, "", "Search Forward", "SearchFwd", b, true, A::Search(1));
    add(t, "", "Search Reverse", "SearchRev", b, true, A::Search(-1));
    add(t, "", "NeedleSearch", "NeedleSearch", val, true, A::Needle);
    add(t, "", "Slip", "Slip", b, true, A::Slip);
    add(t, "", "Quantize", "Quantize", b, true, A::Quantize);
    add(t, "", "Reverse", "Reverse", b, true, A::Stub(Stub::Reverse));
    add(t, "", "SlipReverse", "SlipReverse", b, true, A::Stub(Stub::SlipReverse));
    add(t, "", "Vinyl", "Vinyl", b, true, A::Stub(Stub::Vinyl));
    add(t, "", "VinylBrakeSpeed LeftDeck", "VinylAdjustBrake.Left", k7, false, A::Stub(Stub::VinylBrake(0)));
    add(t, "", "VinylBrakeSpeed RightDeck", "VinylAdjustBrake.Right", k7, false, A::Stub(Stub::VinylBrake(1)));
    add(t, "", "AutoCue All Decks", "AutoCue4", b, false, A::More(M::AutoCueAll));
    add(t, "", "Quantize All Decks", "Quantize4", b, false, A::QuantizeAll);
    add(t, "", "AutoCue", "AutoCue", b, true, A::Stub(Stub::AutoCue));
    add(t, "Tempo", "MasterTempo", "MasterTempo", b, true, A::MasterTempo);
    add(t, "Tempo", "Tempo", "TempoSlider", k7, true, A::Tempo);
    add(t, "Tempo", "TempoRange", "TempoRange", b, true, A::TempoRange);
    add(t, "Tempo", "TempoTakeOverLED Down", "TempoDown", ind, true, A::TempoLed(-1));
    add(t, "Tempo", "TempoTakeOverLED Up", "TempoUp", ind, true, A::TempoLed(1));
    add(t, "Tempo", "TempoReset", "TempoReset", b, true, A::TempoReset);
    add(t, "Tempo", "Sync", "Sync", b, true, A::Sync);
    add(t, "Tempo", "SyncRate", "SyncRate", b, true, A::Stub(Stub::SyncRate));
    add(t, "Tempo", "Master", "Master", b, true, A::Master);
    add(t, "Tempo", "TapBpm", "TapBpm", b, true, A::TapBpm);
    add(t, "Tempo", "PitchBend Up", "PitchBendUp", b, true, A::PitchBend(1));
    add(t, "Tempo", "PitchBend Down", "PitchBendDown", b, true, A::PitchBend(-1));
    add(t, "Loop", "AutoLoop", "AutoLoop", b, true, A::AutoLoop);
    add(t, "Loop", "4BeatLoop", "4BeatLoop", b, true, A::Loop4);
    add(t, "Loop", "ActiveLoop", "ActiveLoop", b, true, A::Stub(Stub::ActiveLoop));
    add(t, "Loop", "ActiveLoop Select (HotCue)", "ActiveLoopSelectHotCue", b, true, A::More(M::ActiveLoopSel { hot: true, off: false }));
    add(t, "Loop", "ActiveLoop Off (HotCue)", "ActiveLoopOffHotCue", b, true, A::More(M::ActiveLoopSel { hot: true, off: true }));
    add(t, "Loop", "ActiveLoop Select (MemoryCue)", "ActiveLoopSelectMemCue", b, true, A::More(M::ActiveLoopSel { hot: false, off: false }));
    add(t, "Loop", "ActiveLoop Off (MemoryCue)", "ActiveLoopOffMemCue", b, true, A::More(M::ActiveLoopSel { hot: false, off: true }));
    add(t, "Loop", "Loop Half", "LoopHalf", b, true, A::LoopResize(0.5));
    add(t, "Loop", "Loop Double", "LoopDouble", b, true, A::LoopResize(2.0));
    add(t, "Loop", "Loop Size", "LoopHalfDouble", rot, true, A::LoopSize);
    add(t, "Loop", "LoopMove Right", "LoopMoveRight", b, true, A::Stub(Stub::LoopMove(1)));
    add(t, "Loop", "LoopMove Left", "LoopMoveLeft", b, true, A::Stub(Stub::LoopMove(-1)));
    add(t, "Loop", "Loop Move", "LoopMove", rot, true, A::LoopMoveRot);
    add(t, "Loop", "Loop In", "LoopIn", b, true, A::LoopIn);
    add(t, "Loop", "Loop ReTrigger", "ReTrigger", b, true, A::ReTrigger);
    add(t, "Loop", "Loop Out", "LoopOut", b, true, A::LoopOut);
    add(t, "Loop", "ReloopExit", "ReloopExit", b, true, A::Reloop);
    for (i, beats) in crate::player_ext::LOOP_SIZES.iter().enumerate() {
        let label = ["1/64", "1/32", "1/16", "1/8", "1/4", "1/2", "1", "2", "4", "8", "16", "32", "64", "128", "256", "512"][i];
        add(t, "Loop", &format!("AutoBeatLoop {label}"), &format!("BeatLoop{}", label.replace('/', "_")), b, true, A::BeatLoop(*beats));
    }
    add(t, "Grid", "GridAdjust", "GridAdjust", b, true, A::More(M::Grid(false)));
    add(t, "Grid", "GridSlide", "GridSlide", b, true, A::More(M::Grid(true)));
    add(t, "Grid", "GridAdjust Half", "GridAdjustHalf", b, true, A::GridHalfDouble(false));
    add(t, "Grid", "GridAdjust Double", "GridAdjustDouble", b, true, A::GridHalfDouble(true));
    add(t, "Capture", "SlicerCapture", "SlicerCapture", b, true, A::Stub(Stub::SlicerCapture));
    for (d, nm) in [("DVS Switch to Absolute", "DvsAbsolute"), ("DVS Switch to Relative", "DvsRelative"), ("DVS Switch to Internal", "DvsInternal"), ("DVS Switch to Through", "DvsThrough"), ("DVS Switch to Wireless(A)", "DvsWireless(A)"), ("DVS Switch to Wireless(B)", "DvsWireless(B)"), ("DVS Toggle INT and WIR/REL/ABS", "DvsWirAbsRelInt")] {
        add(t, "DVS", d, nm, b, true, A::More(M::Off(DVS)));
    }
    add(t, "KeyShift", "Semitone Up", "SemitoneUp", b, true, A::KeyShift(1));
    add(t, "KeyShift", "Semitone Down", "SemitoneDown", b, true, A::KeyShift(-1));
    add(t, "KeyShift", "KeySync", "KeySync", b, true, A::KeySync);
    add(t, "KeyShift", "KeyReset", "KeyReset", b, true, A::KeyReset);
    add(t, "STEMS", "ActiveStem Mute/Solo", "ActivePartMuteSolo", b, false, A::StemSolo);
    add(t, "STEMS", "DRUM SWAP SINGLE Mode", "DrumSwapSingle", b, true, A::More(M::DrumMulti(false)));
    add(t, "STEMS", "DRUM SWAP MULTI Mode", "DrumSwapMulti", b, true, A::More(M::DrumMulti(true)));
    add(t, "STEMS", "DRUM LOOP BANK A", "DrumSwapBankA", b, true, A::More(M::DrumBank(0)));
    add(t, "STEMS", "DRUM LOOP BANK B", "DrumSwapBankB", b, true, A::More(M::DrumBank(1)));
    add(t, "STEMS", "DRUM SWAP Gain", "DrumSwapGain", k7, true, A::More(M::DrumGain));
    for s in 0..4u8 {
        add(t, "STEMS", &format!("DRUM SWAP Slot {} Play/Stop", s + 1), &format!("DrumSwap{}", s + 1), b, true, A::More(M::DrumSlot(s)));
    }
    add(t, "STEMS", "DRUM CAPTURE On/Off", "DrumSwapCapture", b, true, A::More(M::DrumCapture));
    add(t, "STEMS", "DRUM CAPTURE Length", "DrumSwapCaptureLength", b, true, A::More(M::DrumCaptureLen));

    // ---- PAD
    let t = Tab::Pad;
    add(t, "HotCue", "HotCueMode", "HotCue", b, true, A::PadMode(0));
    for n in 1..=16u8 {
        add(t, "HotCue", &format!("HotCue{n}"), &format!("PAD{n}_HotCue"), pad, true, A::HotCue(n - 1));
    }
    for n in 1..=16u8 {
        add(t, "HotCue", &format!("HotCue{n} Delete"), &format!("PAD{n}_HotCue+Shift"), pad, true, A::HotCueDelete(n - 1));
    }
    add(t, "MemoryCue", "MemoryCueCall Next", "Parameter1Right_HotCue", b, true, A::MemoryStep(1));
    add(t, "MemoryCue", "MemoryCueCall Back", "Parameter1Left_HotCue", b, true, A::MemoryStep(-1));
    add(t, "MemoryCue", "MemoryCue Set", "Memory", b, true, A::MemorySet);
    add(t, "MemoryCue", "MemoryCue Delete", "Delete", b, true, A::MemoryDelete);
    for bank in 1..=2u8 {
        add(t, "PadFX", &format!("PadFX{bank}Mode"), &format!("PadFx{bank}"), b, true, A::Stub(Stub::PadFxMode(bank)));
        for n in 1..=16u8 {
            add(t, "PadFX", &format!("PadFX{bank} Pad{n}"), &format!("PAD{n}_PadFx{bank}"), pad, true, A::Stub(Stub::PadFx(bank, n - 1)));
        }
        add(t, "PadFX", &format!("PadFX{bank}Beat Up"), &format!("Parameter1Right_PadFx{bank}"), b, true, A::Stub(Stub::PadFxBeat(bank, 1)));
        add(t, "PadFX", &format!("PadFX{bank}Beat Down"), &format!("Parameter1Left_PadFx{bank}"), b, true, A::Stub(Stub::PadFxBeat(bank, -1)));
    }
    for (lp, mode) in [(false, "Slicer"), (true, "SlicerLoop")] {
        add(t, "Slicer", &format!("{mode}Mode"), mode, b, true, A::Stub(Stub::SlicerMode(lp)));
        for n in 1..=8u8 {
            add(t, "Slicer", &format!("{mode} Pad{n}"), &format!("PAD{n}_{mode}"), pad, true, A::Stub(Stub::SlicerPad(lp, n - 1)));
        }
    }
    for (lp, mode) in [(false, "Slicer"), (true, "SlicerLoop")] {
        add(t, "Slicer", &format!("{mode}Shift Left"), &format!("PAD5_{mode}+Shift"), pad, true, A::Stub(Stub::SlicerShift(lp, -1)));
        add(t, "Slicer", &format!("{mode}Shift Right"), &format!("PAD6_{mode}+Shift"), pad, true, A::Stub(Stub::SlicerShift(lp, 1)));
    }
    for (lp, mode) in [(false, "Slicer"), (true, "SlicerLoop")] {
        add(t, "Slicer", &format!("{mode}Roll Up"), &format!("Parameter1Right_{mode}"), b, true, A::Stub(Stub::SlicerRoll(lp, 1)));
        add(t, "Slicer", &format!("{mode}Roll Down"), &format!("Parameter1Left_{mode}"), b, true, A::Stub(Stub::SlicerRoll(lp, -1)));
    }
    for (lp, mode) in [(false, "Slicer"), (true, "SlicerLoop")] {
        add(t, "Slicer", &format!("{mode}Length Up"), &format!("Parameter2Right_{mode}"), b, true, A::Stub(Stub::SlicerLength(lp, 1)));
        add(t, "Slicer", &format!("{mode}Length Down"), &format!("Parameter2Left_{mode}"), b, true, A::Stub(Stub::SlicerLength(lp, -1)));
    }
    add(t, "BeatJump", "BeatJumpMode", "BeatJump", b, true, A::PadMode(2));
    for n in 1..=16u8 {
        add(t, "BeatJump", &format!("BeatJump Pad{n}"), &format!("PAD{n}_BeatJump"), pad, true, A::BeatJumpPad(n - 1));
    }
    add(t, "BeatJump", "BeatJump Page Right", "Parameter1Right_BeatJump", b, true, A::BeatJumpPage(1));
    add(t, "BeatJump", "BeatJump Page Left", "Parameter1Left_BeatJump", b, true, A::BeatJumpPage(-1));
    add(t, "BeatJump", "BeatJump Bank Right", "Parameter2Right_BeatJump", b, true, A::More(M::JumpPage2(1)));
    add(t, "BeatJump", "BeatJump Bank Left", "Parameter2Left_BeatJump", b, true, A::More(M::JumpPage2(-1)));
    add(t, "BeatLoop", "BeatLoopMode", "BeatLoop", b, true, A::PadMode(3));
    for n in 1..=16u8 {
        add(t, "BeatLoop", &format!("BeatLoop Pad{n}"), &format!("PAD{n}_BeatLoop"), pad, true, A::BeatLoopPad(n - 1));
    }
    let code = |m: crate::pads::PadMode| m.code();
    use crate::pads::PadMode as PM;
    add(t, "ActiveCensor", "ActiveCensorMode", "ActiveCensor", b, true, A::PadMode(code(PM::ActiveCensor)));
    for (i, d) in ["ActiveCensor FX1", "ActiveCensor FX2", "ActiveCensor FX3", "ActiveCensor FX4", "ActiveCensor In", "ActiveCensor Out", "ActiveCensor Clear", "ActiveCensor On/Off"].iter().enumerate() {
        add(t, "ActiveCensor", d, &format!("PAD{}_ActiveCensor", i + 1), pad, true, A::More(M::Censor(i as u8)));
    }
    add(t, "ActiveCensor", "ActiveCensorCall Next", "Parameter1Right_ActiveCensor", b, true, A::More(M::CensorCall(1)));
    add(t, "ActiveCensor", "ActiveCensorCall Back", "Parameter1Left_ActiveCensor", b, true, A::More(M::CensorCall(-1)));
    add(t, "ActiveCensor", "ActiveCensorBeat Up", "Parameter2Right_ActiveCensor", b, true, A::More(M::CensorBeat(1)));
    add(t, "ActiveCensor", "ActiveCensorBeat Down", "Parameter2Left_ActiveCensor", b, true, A::More(M::CensorBeat(-1)));
    add(t, "KeyShift", "KeyShiftMode", "KeyShift", b, true, A::PadMode(code(PM::KeyShift)));
    for n in 1..=16u8 {
        add(t, "KeyShift", &format!("KeyShift Pad{n}"), &format!("PAD{n}_KeyShift"), pad, true, A::More(M::KeyShiftPad(n - 1)));
    }
    add(t, "KeyShift", "KeyShift Page Right", "Parameter1Right_KeyShift", pad, true, A::More(M::KeyShiftPage(1)));
    add(t, "KeyShift", "KeyShift Page Left", "Parameter1Left_KeyShift", pad, true, A::More(M::KeyShiftPage(-1)));
    add(t, "Keyboard", "KeyboardMode", "Keyboard", b, true, A::PadMode(code(PM::Keyboard)));
    for n in 1..=16u8 {
        add(t, "Keyboard", &format!("Keyboard Pad{n}"), &format!("PAD{n}_Keyboard"), pad, true, A::More(M::KeyboardPad(n - 1)));
    }
    for n in 1..=16u8 {
        add(t, "Keyboard", &format!("Keyboard HotCue Delete Pad{n}"), &format!("PAD{n}_Keyboard+Shift"), pad, true, A::More(M::KeyboardDelete(n - 1)));
    }
    add(t, "Keyboard", "Keyboard Page Right", "Parameter1Right_Keyboard", pad, true, A::More(M::KeyboardPage(1)));
    add(t, "Keyboard", "Keyboard Page Left", "Parameter1Left_Keyboard", pad, true, A::More(M::KeyboardPage(-1)));
    add(t, "SampleScratch", "SampleScratchMode", "SampleScratch", b, true, A::PadMode(code(PM::SamplerScratch)));
    for n in 1..=8u8 {
        add(t, "SampleScratch", &format!("SampleScratch Pad{n}"), &format!("PAD{n}_SampleScratch"), pad, true, A::More(M::ScratchPad(n - 1, false)));
    }
    for n in 1..=8u8 {
        add(t, "SampleScratch", &format!("SampleScratch Eject Pad{n}"), &format!("PAD{n}_SampleScratch+Shift"), pad, true, A::More(M::ScratchPad(n - 1, true)));
    }
    add(t, "HotCue", "HotCue/SamplerMode", "HotCueSampler", b, true, A::PadMode(0));
    for n in 1..=3u8 {
        add(t, "HotCue", &format!("HotCue{n}/Sampler{n} Play"), &format!("PAD{n}_HotCueSampler"), pad, true, A::More(M::HotCueSampler(n - 1, false)));
    }
    for n in 1..=3u8 {
        add(t, "HotCue", &format!("HotCue{n} Delete/Sampler{n} Stop"), &format!("PAD{n}_HotCueSampler+Shift"), pad, true, A::More(M::HotCueSampler(n - 1, true)));
    }
    add(t, "STEMS", "StemsMode", "Stems", b, true, A::PadMode(1));
    // (Sources in the engine's order: 0 drums, 1 bass, 2 other, 3 vocals; FX rows as on screen.)
    add(t, "STEMS", "ActiveStem Inst", "ActivePartInst", pad, true, A::StemPart(2));
    add(t, "STEMS", "ActiveStem Bass", "ActivePartBass", pad, true, A::StemPart(1));
    add(t, "STEMS", "ActiveStem Vocal", "ActivePartVocal", pad, true, A::StemPart(3));
    add(t, "STEMS", "ActiveStem Drums", "ActivePartDrums", pad, true, A::StemPart(0));
    add(t, "STEMS", "ActiveStem FX Inst", "FXPartSelectInstOn", pad, false, A::StemFx(1));
    add(t, "STEMS", "ActiveStem FX Bass", "FXPartSelectBassOn", pad, false, A::StemFx(2));
    add(t, "STEMS", "ActiveStem FX Vocal", "FXPartSelectVocalOn", pad, false, A::StemFx(0));
    add(t, "STEMS", "ActiveStem FX Drums", "FXPartSelectDrumsOn", pad, false, A::StemFx(3));

    // ---- FX
    let t = Tab::Fx;
    for u in 0..2u8 {
        let cat: &'static str = if u == 0 { "FX1" } else { "FX2" };
        for s in 0..3u8 {
            let p = format!("{cat}-{}", s + 1);
            add(t, cat, &format!("{p} LevelDepth"), &p, k7, false, A::FxLevel(u, s));
            add(t, cat, &format!("{p} On"), &format!("{p}On"), b, false, A::FxOn(u, s));
            add(t, cat, &format!("{p} Select Next"), &format!("{p}Select"), b, false, A::FxSelect(u, s, 1));
            add(t, cat, &format!("{p} Select Back"), &format!("{p}SelectBack"), b, false, A::FxSelect(u, s, -1));
            add(t, cat, &format!("{p} Beat Up"), &format!("{p}BeatUp"), b, false, A::FxBeat(u, s, 1));
            add(t, cat, &format!("{p} Beat Down"), &format!("{p}BeatDown"), b, false, A::FxBeat(u, s, -1));
        }
        add(t, cat, &format!("{cat} Multi/Single Toggle"), &format!("{cat}FXMode"), b, false, A::FxMode(u));
        add(t, cat, &format!("{cat} Beat Up"), &format!("{cat}BeatUp"), b, false, A::FxUnitBeat(u, 1));
        add(t, cat, &format!("{cat} Beat Down"), &format!("{cat}BeatDown"), b, false, A::FxUnitBeat(u, -1));
        add(t, cat, &format!("{cat} BeatSize"), &format!("{cat}BeatUpDown"), rot, false, A::FxUnitBeatRot(u));
        add(t, cat, &format!("{cat} AdjustTime"), &format!("{cat}AdjustTime"), rot, false, A::More(M::Off(ADJUST_TIME)));
        add(t, cat, &format!("{cat} BPM AutoMode"), &format!("{cat}AutoBeat"), b, false, A::FxAuto(u));
        add(t, cat, &format!("{cat} BPM TapMode/TapBpm"), &format!("{cat}TapBeat"), b, false, A::FxTap(u));
        add(t, cat, &format!("{cat} ReleaseFX Select"), &format!("{cat}ReleaseFXType"), rot, false, A::FxReleaseType(u));
        add(t, cat, &format!("{cat} ReleaseFX Beat Up"), &format!("{cat}ReleaseFXBeatUp"), b, false, A::FxReleaseBeat(u, 1));
        add(t, cat, &format!("{cat} ReleaseFX Beat Down"), &format!("{cat}ReleaseFXBeatDown"), b, false, A::FxReleaseBeat(u, -1));
        add(t, cat, &format!("{cat} ReleaseFX On"), &format!("{cat}ReleaseFXOn"), b, false, A::FxReleaseOn(u));
        for d in 0..4u8 {
            add(t, cat, &format!("{cat} AssignDeck{}", d + 1), &format!("{cat}Assign{}", d + 1), b, false, A::FxAssign(u, d));
        }
    }
    for (i, ch) in ["CH1", "CH2", "CH3", "CH4"].iter().enumerate() {
        add(t, "CFX", &format!("CFX Parameter{ch}"), &format!("CFXParameter{ch}"), k7, false, A::CfxColor(i as u8));
    }
    add(t, "CFX", "CFX ParameterSampler", "CFXParameterSampler", k7, false, A::CfxColor(crate::fx::SAMPLER as u8));
    add(t, "CFX", "CFX ParameterMaster", "CFXParameterMaster", k7, false, A::CfxColor(4));
    add(t, "CFX", "CFX On", "CFXOn", b, false, A::More(M::CfxOn));
    add(t, "CFX", "CFX Select Next", "CFXSelect", b, false, A::CfxSelect(1));
    add(t, "CFX", "CFX Select Back", "CFXSelectBack", b, false, A::CfxSelect(-1));
    for u in 0..2u8 {
        let cat: &'static str = if u == 0 { "MFX1" } else { "MFX2" };
        let n = u + 1;
        add(t, cat, &format!("{cat} Parameter"), cat, rot, false, A::More(M::MfxParam(u)));
        add(t, cat, &format!("{cat} On"), &format!("{cat}On"), b, false, A::More(M::MfxOn(u)));
        add(t, cat, &format!("{cat} AssignDeck{n}"), &format!("{cat}Assign{n}"), b, false, A::More(M::MfxAssign(u, u)));
        add(t, cat, &format!("{cat} AssignDeck{}", n + 2), &format!("{cat}Assign{}", n + 2), b, false, A::More(M::MfxAssign(u, u + 2)));
        add(t, cat, &format!("{cat} Select Next"), &format!("{cat}SelectUp"), b, false, A::More(M::MfxSelect(u, 1)));
        add(t, cat, &format!("{cat} Select Back"), &format!("{cat}SelectDown"), b, false, A::More(M::MfxSelect(u, -1)));
        add(t, cat, &format!("{cat} Cancel"), &format!("{cat}Abort"), b, false, A::More(M::MfxCancel(u)));
    }
    add(t, "", "MFX Sample Volume", "MFXSampleVolume", rot, false, A::More(M::MfxSampleVolume));
    for (kind, word) in [("Roll", "ROLL"), ("Trans", "TRANS")] {
        let beats: &[&str] = if kind == "Roll" { &["1/16", "1/8", "1/4", "1/2", "3/4", "1", "2", "4", "8"] } else { &["1/16", "1/8", "1/4", "1/2", "3/4", "1", "2", "4", "8", "16"] };
        for bt in beats {
            add(t, "STEMS", &format!("DRUM {word} {bt} On"), &format!("Drum{kind}{}", bt.replace('/', "_")), b, true, A::More(M::Off(DRUM_FX)));
        }
    }
    add(t, "STEMS", "DRUM RELESE FX On", "DrumReleaseFx", b, true, A::More(M::DrumRelease));
    add(t, "STEMS", "DRUM RELESE FX Select", "DrumReleaseFxSelect", b, true, A::More(M::DrumReleaseSelect));

    // ---- SAMPLER (pads 9-16 are listed for the second eight slots; they are the same rows)
    let t = Tab::Sampler;
    add(t, "Sampler", "SamplerMode", "Sampler", b, true, A::PadMode(crate::ui_perf::PAD_SAMPLER));
    for n in 1..=16u8 {
        let p = (n - 1) % 8;
        add(t, "Sampler", &format!("Sampler{n} Play"), &format!("PAD{}_Sampler", p + 1), pad, true, A::SamplerPad(p, false));
    }
    for n in 1..=16u8 {
        let p = (n - 1) % 8;
        add(t, "Sampler", &format!("Sampler{n} Stop"), &format!("PAD{}_Sampler+Shift", p + 1), pad, true, A::SamplerPadStop(p));
    }
    add(t, "VelocitySampler", "VelocitySamplerMode", "VelocitySampler", b, true, A::PadMode(crate::ui_perf::PAD_SAMPLER));
    for n in 1..=16u8 {
        let p = (n - 1) % 8;
        add(t, "VelocitySampler", &format!("VelocitySampler{n} Play"), &format!("PAD{}_VelocitySampler", p + 1), pad, true, A::SamplerPad(p, true));
    }
    for n in 1..=16u8 {
        let p = (n - 1) % 8;
        add(t, "VelocitySampler", &format!("VelocitySampler{n} Stop"), &format!("PAD{}_VelocitySampler+Shift", p + 1), pad, true, A::SamplerPadStop(p));
    }
    for n in 1..=16u8 {
        let p = (n - 1) % 8;
        add(t, "VelocitySampler", &format!("VelocitySampler{n} Level"), &format!("PAD{}_VelocitySampler+Velocity", p + 1), val, true, A::More(M::VelocityLevel(p)));
    }
    add(t, "Bank", "SamplerBank Up/SeqEraseMode", "Parameter1Right_Sampler", b, true, A::SamplerBank(1));
    add(t, "Bank", "SamplerBank Down/SeqMuteMode", "Parameter1Left_Sampler", b, true, A::SamplerBank(-1));
    add(t, "", "SamplerGain", "SamplerGain", k7, false, A::SamplerGain);
    add(t, "", "SamplerVolume", "SamplerVolume", k7, false, A::SamplerVolume);
    add(t, "", "SamplerSync", "SamplerSync", b, false, A::Smp(SmpAct::Sync));
    add(t, "", "SamplerMaster", "SamplerMaster", b, false, A::Smp(SmpAct::Master));
    add(t, "", "SamplerCue", "SamplerCue", b, false, A::SamplerCue);
    add(t, "", "SamplerTapBpm", "SamplerTapBpm", b, false, A::More(M::SamplerTapBpm));
    add(t, "", "SamplerQuantize", "SamplerQuantize", b, false, A::Smp(SmpAct::Quantize));
    add(t, "Sequence", "SequencerRec", "SequencerRec", b, true, A::Smp(SmpAct::SeqRec));
    add(t, "Sequence", "SequencerStart", "SequencerStart", b, true, A::Smp(SmpAct::SeqPlay));
    add(t, "Sequence", "SequencerSave", "SaveSequence", b, true, A::Smp(SmpAct::SeqSave));
    add(t, "Sequence", "SequencerSlotSelect Next", "SelectSlot", b, true, A::Smp(SmpAct::SeqStep(1)));
    add(t, "Sequence", "SequenceLoad", "SequenceLoad", b, true, A::Smp(SmpAct::SeqLoad(255)));
    add(t, "Sequence", "SequenceCallMode", "SequenceCall", b, true, A::PadMode(crate::ui_perf::PAD_SEQ_CALL));
    for n in 1..=8u8 {
        add(t, "Sequence", &format!("SequenceCall{n} Play"), &format!("PAD{n}_SequenceCall"), pad, true, A::SeqCall(n - 1, false));
    }
    for n in 1..=8u8 {
        add(t, "Sequence", &format!("SequenceCall{n} Stop"), &format!("PAD{n}_SequenceCall+Shift"), pad, true, A::SeqCall(n - 1, true));
    }

    // ---- MIXER
    let t = Tab::Mixer;
    add(t, "", "Gain", "Gain", k7, true, A::Trim);
    add(t, "", "EQ High", "EQHigh", k7, true, A::Eq(2));
    add(t, "", "EQ Mid", "EQMid", k7, true, A::Eq(1));
    add(t, "", "EQ Low", "EQLow", k7, true, A::Eq(0));
    add(t, "", "HeadphoneCue", "HeadphoneCue", b, true, A::HeadCue);
    add(t, "", "MasterCue", "MasterCue", b, false, A::More(M::MasterCue));
    add(t, "", "ChannelFader", "ChannelFader", k7, true, A::ChFader);
    add(t, "", "HeadphonesMix", "HeadphonesMix", k7, false, A::HpMix);
    add(t, "", "HeadphonesVolume", "HeadphonesVolume", k7, false, A::HpLevel);
    add(t, "", "CrossFader", "CrossFader", k7, false, A::CrossFader);
    add(t, "", "CrossfaderAssignA", "CrossfaderAssign.A", b, true, A::XfAssign(1));
    add(t, "", "CrossfaderAssignB", "CrossfaderAssign.B", b, true, A::XfAssign(2));
    add(t, "", "CrossfaderAssignThru", "CrossfaderAssign.Thru", b, true, A::XfAssign(0));
    add(t, "", "CrossFaderCurve", "CrossFaderCurve", k7, false, A::More(M::XfCurve(false)));
    add(t, "", "MasterLevel", "MasterLevel", k7, false, A::MasterLevel);
    add(t, "STEMS", "StemISO Mode", "PartIsolator", b, true, A::StemIso);
    add(t, "STEMS", "StemISO Inst/Bass", "PartIsolatorInst", k7, true, A::StemIsoKnob(1));
    add(t, "STEMS", "StemISO Vocal", "PartIsolatorVocal", k7, true, A::StemIsoKnob(2));
    add(t, "STEMS", "StemISO Drums", "PartIsolatorDrums", k7, true, A::StemIsoKnob(0));

    // ---- BROWSE
    let t = Tab::Browse;
    add(t, "", "Browse", "Browse", rot, false, A::Browse);
    add(t, "", "Browse Up", "BrowseUp", b, false, A::BrowseStep(-1));
    add(t, "", "Browse Down", "BrowseDown", b, false, A::BrowseStep(1));
    add(t, "", "BrowseForward", "Forward", b, false, A::Forward);
    add(t, "", "BrowseBack", "Back", b, false, A::Back);
    add(t, "", "Browse Jump Up", "BrowseJumpUp", b, false, A::BrowseStep(-10));
    add(t, "", "Browse Jump Down", "BrowseJumpDown", b, false, A::BrowseStep(10));
    add(t, "", "AddToTagList", "AddToTagList", b, false, A::TagList);
    add(t, "", "RelatedTracks", "RelatedTracks", b, false, A::Related);
    add(t, "", "DISCOVER", "Discover", b, false, A::More(M::Off(DISCOVER)));
    add(t, "", "Area", "Area", b, false, A::Area);
    add(t, "", "Area Right", "AreaRight", b, false, A::Forward);
    add(t, "", "Area Left", "AreaLeft", b, false, A::Back);
    add(t, "", "Load", "Load", b, true, A::Load);
    add(t, "", "InstantDoubles", "InstDoubles", b, true, A::InstDoubles);
    add(t, "", "SortBPM", "SortBPM", b, false, A::Sort(Col::Bpm));
    add(t, "", "SortSONG", "SortSONG", b, false, A::Sort(Col::Title));
    add(t, "", "SortTRACK", "SortTRACK", b, false, A::Sort(Col::TrackNumber));
    add(t, "", "SortARTIST", "SortARTIST", b, false, A::Sort(Col::Artist));
    add(t, "", "SortKEY", "SortKEY", b, false, A::Sort(Col::Key));
    add(t, "", "Preview/PreviewPlayer PlayStop", "PlayPausePreview", b, false, A::More(M::Preview(false)));
    add(t, "", "Preview/PreviewPlayer Skip", "SkipPreview", b, false, A::More(M::Preview(true)));

    // ---- OTHER
    let t = Tab::Other;
    add(t, "", "FXPanel", "FxPanel", b, false, A::Panel(0));
    add(t, "", "SamplerPanel", "SamplerPanel", b, false, A::Panel(1));
    add(t, "", "MixerPanel", "MixerPanel", b, false, A::Panel(2));
    add(t, "", "RecPanel", "RecPanel", b, false, A::Stub(Stub::RecPanel));
    add(t, "", "PanelSelect Next", "PanelForward", b, false, A::PanelStep(1));
    add(t, "", "PanelSelect Back", "PanelBack", b, false, A::PanelStep(-1));
    add(t, "", "WaveZoom", "Zoom", rot, false, A::Zoom);
    add(t, "", "BrowseView", "SwitchActiveWindow", b, false, A::BrowseView);
    add(t, "", "AutoMix StartStop", "AutoMixStartStop", b, false, A::Stub(Stub::AutoMixStartStop));
    add(t, "AbletonLink", "AbletonLink TapBpm", "AbletonLinkTapBpm", b, false, A::More(M::Off(LINK)));
    add(t, "AbletonLink", "AbletonLink BPM Up", "AbletonLinkBpmUp", b, false, A::More(M::Off(LINK)));
    add(t, "AbletonLink", "AbletonLink BPM Down", "AbletonLinkBpmDown", b, false, A::More(M::Off(LINK)));
    add(t, "AbletonLink", "AbletonLink BPM", "AbletonLinkBpmUpDown", rot, false, A::More(M::Off(LINK)));

    // ---- MIXPOINTLINK
    let t = Tab::MixPointLink;
    for (i, (d, n, pd)) in [("MixPointLinkPanel", "MixPointLinkPanel", false), ("MixPointLink CueSelect Next", "MixPointSelectNext", true), ("MixPointLink CueSelect Back", "MixPointSelectPrev", true), ("MixPointLink Set", "MixPointLink", true), ("MixPointLink Exit", "MixPointLinkExit", true), ("MixPointLink Deck Select", "MixPointLinkDeck", true), ("MixPointLink Silent", "MixPointLinkSilent", true)].iter().enumerate() {
        add(t, "", d, n, b, *pd, A::Stub(Stub::MixPointLink(i as u8)));
    }

    // ---- factory mappings only: MIDI Learn does not offer these
    let t = Tab::None;
    add(t, "", "", "JogScratch", b, true, A::Jog(true));
    add(t, "", "", "JogPitchBend", b, true, A::Jog(false));
    add(t, "", "", "WheelPitchBend", b, true, A::Jog(false));
    add(t, "", "", "JogSearch", b, true, A::JogSearch);
    add(t, "", "", "WheelSearch", b, true, A::JogSearch);
    add(t, "", "", "JogTouch", b, true, A::JogTouch);
    add(t, "", "", "Shift", b, true, A::Shift);
    // (A long press of LOOP IN, sent by the hardware as its own code: the 4-beat loop.)
    add(t, "", "", "4BeatLoopLongPress", b, true, A::Loop4);
    // (A crossfader-assign switch as a knob: A at 0x00, THRU at 0x40, B at 0x7F.)
    add(t, "", "", "CrossfaderAssign", k7, true, A::XfAssignKnob);
    for ch in 0..4u8 {
        // (The CFX knob's centre click: the knob back to the middle.)
        add(t, "", "", &format!("CFXParameterCH{}Center", ch + 1), b, false, A::CfxCenter(ch));
    }
    // (CDJ-style CALL buttons: the next / previous memory cue; while a loop plays, the loop
    // doubles / halves. `UiPlayer::deviceMemoryCue` 0x1020281dc.)
    add(t, "", "", "CallNext", b, true, A::CallStep(1));
    add(t, "", "", "CallPrev", b, true, A::CallStep(-1));
    // (Vinyl Speed Adjust knobs of a deck: brake, start, or both. `deviceVinylAdjust`.)
    add(t, "", "", "VinylAdjustBrake", k7, true, A::VinylAdjust(0));
    add(t, "", "", "VinylAdjustStart", k7, true, A::VinylAdjust(1));
    add(t, "", "", "VinylAdjustTouchRelease", k7, true, A::VinylAdjust(2));
    for u in 0..2u8 {
        let cat = if u == 0 { "FX1" } else { "FX2" };
        // TAP pressed: a tap; held for a second: back to AUTO (`RbxBpmPanelDeviceComponent`).
        add(t, "", "", &format!("{cat}TapBeatWithLongPress"), b, false, A::FxTapLong(u));
        // A channel switch's positions: the assign follows the switch (on while its code is
        // held, off when released; `RbxFxDeviceComponent::eventOnChanged` $_1 with "set").
        for (i, ch) in ["CH1", "CH2", "CH3", "CH4"].iter().enumerate() {
            add(t, "", "", &format!("{cat}Assign.{ch}"), b, false, A::FxAssignHold(u, i as u8));
        }
        add(t, "", "", &format!("{cat}Assign.SAMPLER"), b, false, A::FxAssignHold(u, crate::fx::SAMPLER as u8));
        // A slot's effect chosen by name (`FX1-1Select.ECHO`): rekordbox's effect names in
        // capitals without spaces or brackets.
        for sl in 0..3u8 {
            for (i, info) in crate::fx::INFO.iter().enumerate() {
                let key: String = info.name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
                add(t, "", "", &format!("{cat}-{}Select.{key}", sl + 1), b, false, A::FxSelectKind(u, sl, i as u8));
            }
        }
        add(t, "", "", &format!("{cat}Assign.MASTER"), b, false, A::FxAssignHold(u, crate::fx::MASTER as u8));
    }
    // (rekordbox's combination pads, controller only: pads 33..48 of the deck's PAD FX.)
    for n in 1..=16u8 {
        add(t, "", "", &format!("PAD{n}_PadFxCombination"), pad, true, A::Stub(Stub::PadFx(3, n - 1)));
    }
    // Shift layers of the pads that act as the pad itself (`BeatJumpView::eventOnChanged`
    // 0x101ddd50c, `PadFxView::eventOnChanged` 0x101e2643c, `ActiveCensorView` 0x101084c40,
    // `KeyShiftView` 0x101d64adc: the +Shift component of pad n goes the same way as pad n).
    for n in 1..=16u8 {
        add(t, "", "", &format!("PAD{n}_BeatJump+Shift"), pad, true, A::BeatJumpPad(n - 1));
        for bank in 1..=2u8 {
            add(t, "", "", &format!("PAD{n}_PadFx{bank}+Shift"), pad, true, A::Stub(Stub::PadFx(bank, n - 1)));
        }
        add(t, "", "", &format!("PAD{n}_KeyShift+Shift"), pad, true, A::More(M::KeyShiftPad(n - 1)));
        add(t, "", "", &format!("PAD{n}_BeatLoop+Shift"), pad, true, A::BeatLoopPad(n - 1));
    }
    for n in 1..=8u8 {
        add(t, "", "", &format!("PAD{n}_ActiveCensor+Shift"), pad, true, A::More(M::Censor(n - 1)));
    }
    add(t, "", "", "PageLeft_BeatJump", b, true, A::More(M::JumpPage2(-1)));
    add(t, "", "", "PageRight_BeatJump", b, true, A::More(M::JumpPage2(1)));
    // (PAD FX pages: PageLeft is `onPageUp`, PageRight `onPageDown`, as rekordbox names them.)
    for bank in 1..=2u8 {
        add(t, "", "", &format!("PageLeft_PadFx{bank}"), b, true, A::More(M::PadFxPage(0)));
        add(t, "", "", &format!("PageRight_PadFx{bank}"), b, true, A::More(M::PadFxPage(1)));
    }
    // (The velocity sampler's bank buttons: `SamplerControlPanel`.)
    add(t, "", "", "Parameter1Right_VelocitySampler", b, true, A::SamplerBank(1));
    add(t, "", "", "Parameter1Left_VelocitySampler", b, true, A::SamplerBank(-1));
    add(t, "", "", "CrossFaderCurve3Pos", k7, false, A::More(M::XfCurve(true)));
    add(t, "", "", "VinylState", b, true, A::More(M::VinylState));
    for n in 1..=6u8 {
        add(t, "", "", &format!("CFX{n}On"), b, false, A::More(M::CfxButton(n - 1)));
    }
    // The Pad Editor's slots (`PadEditorData::createDevice` 0x1020f64c4).
    for m in 1..=8u8 {
        add(t, "", "", &format!("PadMode{m}"), b, true, A::More(M::EditorMode(m - 1)));
        add(t, "", "", &format!("PageLeft_PadMode{m}"), b, true, A::More(M::EditorPage(m - 1, -1)));
        add(t, "", "", &format!("PageRight_PadMode{m}"), b, true, A::More(M::EditorPage(m - 1, 1)));
        add(t, "", "", &format!("Permission_PadMode{m}"), ind, true, A::More(M::Permission(m - 1)));
        for p in 1..=16u8 {
            add(t, "", "", &format!("PAD{p}_PadMode{m}"), pad, true, A::More(M::EditorPad(m - 1, p - 1, false)));
            add(t, "", "", &format!("PAD{p}_PadMode{m}+Shift"), pad, true, A::More(M::EditorPad(m - 1, p - 1, true)));
        }
    }
    // Second batch: deck, browse, FX and mixer functions of the factory files.
    add(t, "", "", "JogTimestamp", b, true, A::Jog(true));
    add(t, "", "", "ShiftWeGO4", b, true, A::Shift);
    add(t, "", "", "Deck", b, true, A::More(M::DeckHeld));
    add(t, "", "", "AutoLoopMode", b, true, A::More(M::LoopBox(false)));
    add(t, "", "", "ManualLoopMode", b, true, A::More(M::LoopBox(true)));
    add(t, "", "", "TimeMode", b, true, A::More(M::TimeMode(false)));
    add(t, "", "", "TimeMode4", b, false, A::More(M::TimeMode(true)));
    add(t, "", "", "ForwardAndLoad", b, true, A::More(M::ForwardAndLoad));
    for u in 0..2u8 {
        for sl in 0..3u8 {
            add(t, "", "", &format!("Fx{}-{}_BeatTarget", u + 1, sl + 1), b, false, A::More(M::BeatTarget(u, sl)));
            add(t, "", "", &format!("FX{}-{}OnMomentary", u + 1, sl + 1), b, false, A::More(M::FxMomentary(u, sl, 255)));
            for ch in 0..2u8 {
                add(t, "", "", &format!("CH{}_FX{}-{}OnMomentary", ch + 1, u + 1, sl + 1), b, false, A::More(M::FxMomentary(u, sl, ch)));
            }
            add(t, "", "", &format!("Sampler_FX{}-{}OnMomentary", u + 1, sl + 1), b, false, A::More(M::FxMomentary(u, sl, crate::fx::SAMPLER as u8)));
        }
        let cat = if u == 0 { "FX1" } else { "FX2" };
        add(t, "", "", &format!("{cat}BeatMultiplier"), ind, false, A::More(M::Out(crate::midi_acts::Out::FxBeat(u))));
        add(t, "", "", &format!("ReleaseFxIlluminationFx{}", u + 1), ind, false, A::More(M::Out(crate::midi_acts::Out::ReleaseOn(u))));
        for k in 0..3u8 {
            add(t, "", "", &format!("{cat}ReleaseFXIndicator.{}", k + 1), ind, false, A::More(M::Out(crate::midi_acts::Out::ReleaseType(u, k))));
        }
        add(t, "", "", &format!("{cat}AssignMaster"), b, false, A::FxAssign(u, crate::fx::MASTER as u8));
        add(t, "", "", &format!("{cat}AssignSampler"), b, false, A::FxAssign(u, crate::fx::SAMPLER as u8));
        add(t, "", "", &format!("{cat}Assign.LEFT"), b, false, A::FxAssignHold(u, 0));
        add(t, "", "", &format!("{cat}Assign.RIGHT"), b, false, A::FxAssignHold(u, 1));
        let mf = if u == 0 { "MFX1" } else { "MFX2" };
        add(t, "", "", &format!("{mf}Illumi"), ind, false, A::More(M::Out(crate::midi_acts::Out::MfxIllumi(u, 0))));
        for p in 1..=4u8 {
            add(t, "", "", &format!("{mf}Illumi.PRESET{p}"), ind, false, A::More(M::Out(crate::midi_acts::Out::MfxIllumi(u, p))));
        }
    }
    add(t, "", "", "DeselectAllFxBeatTarget", b, false, A::More(M::BeatTarget(255, 255)));
    add(t, "", "", "ReleaseFxIlluminationPadLeft", ind, false, A::More(M::Out(crate::midi_acts::Out::ReleaseOn(0))));
    add(t, "", "", "ReleaseFxIlluminationPadRight", ind, false, A::More(M::Out(crate::midi_acts::Out::ReleaseOn(1))));
    add(t, "", "", "Capture", b, true, A::More(M::Capture));
    add(t, "", "", "CaptureValidCondition", ind, true, A::More(M::CaptureOk));
    add(t, "", "", "Bank", b, true, A::More(M::BankMode));
    for p in 1..=4u8 {
        add(t, "", "", &format!("PAD{p}_Bank"), b, true, A::More(M::BankPad(p - 1)));
    }
    for (i, nm) in ["CrossFaderCurve.STRAIGHT", "CrossFaderCurve.MID", "CrossFaderCurve.SHARP", "CrossFaderCurve.THRU"].iter().enumerate() {
        add(t, "", "", nm, b, false, A::More(M::XfCurvePos(i as u8)));
    }
    for (i, nm) in ["ChFaderCurve.STRAIGHT", "ChFaderCurve.MID", "ChFaderCurve.SHARP"].iter().enumerate() {
        add(t, "", "", nm, b, false, A::More(M::ChCurvePos(i as u8)));
    }
    add(t, "", "", "FxBpmAutoMode", b, false, A::More(M::FxBpmMode(false)));
    add(t, "", "", "FxBpmTapMode", b, false, A::More(M::FxBpmMode(true)));
    add(t, "", "", "CFXSubParameter", k7, false, A::More(M::CfxSub));
    add(t, "", "", "CustomPartLayout", ind, false, A::More(M::Out(crate::midi_acts::Out::PartLayout)));
    for beats in [4, 8, 16] {
        add(t, "", "", &format!("Jump{beats}Fwd"), b, true, A::More(M::Jump(beats)));
        add(t, "", "", &format!("Jump{beats}Prev"), b, true, A::More(M::Jump(-beats)));
    }
    add(t, "", "", "DirectionFwdRev", b, true, A::More(M::Direction(false)));
    add(t, "", "", "DirectionSlipRev", b, true, A::More(M::Direction(true)));
    add(t, "", "", "EqCurve", b, false, A::More(M::EqCurve));
    add(t, "", "", "Rec", b, false, A::More(M::Rec));
    add(t, "", "", "PadFx", b, true, A::Stub(Stub::PadFxMode(1)));
    for (nm, src) in [("Vocal", 3u8), ("Inst", 2), ("Drums", 0)] {
        add(t, "", "", &format!("ActivePartButton{nm}"), b, true, A::StemPart(src));
    }
    for (nm, row) in [("Inst", 1u8), ("Vocal", 0), ("Drums", 3)] {
        add(t, "", "", &format!("FXPartSelect{nm}"), b, false, A::StemFx(row));
    }
    add(t, "", "", "SamplerBpmUp", b, false, A::Smp(SmpAct::Bpm(0.01)));
    add(t, "", "", "SamplerBpmDown", b, false, A::Smp(SmpAct::Bpm(-0.01)));
    add(t, "", "", "TempoCenter", b, true, A::More(M::TempoStep(0)));
    for (nm, st) in [("Small", 10), ("Medium", 50), ("Large", 200)] {
        add(t, "", "", &format!("TempoInc{nm}"), b, true, A::More(M::TempoStep(st)));
        add(t, "", "", &format!("TempoDec{nm}"), b, true, A::More(M::TempoStep(-st)));
    }
    for (i, nm) in ["Deck3", "Deck4"].iter().enumerate() {
        add(t, "", "", nm, b, false, A::More(M::DeckSide(i as u8 + 3)));
        add(t, "", "", &format!("{nm}WithoutState"), b, false, A::More(M::DeckSide(i as u8 + 3)));
    }
    for (nm, src) in [("Vocal", 3u8), ("Inst", 2), ("Bass", 1), ("Drums", 0)] {
        add(t, "", "", &format!("{nm}PartInstDoubles"), b, true, A::More(M::PartDoubles(src)));
    }
    add(t, "", "", "AreaLeftEdge", b, false, A::More(M::AreaEdge(true)));
    add(t, "", "", "AreaRightEdge", b, false, A::More(M::AreaEdge(false)));
    add(t, "", "", "BrowseJumpTop", b, false, A::More(M::BrowseEdge(true)));
    add(t, "", "", "BrowseJumpBottom", b, false, A::More(M::BrowseEdge(false)));
    add(t, "", "", "BrowseLineNumber", val, false, A::More(M::BrowseLine));
    add(t, "", "", "ScrollBarControl", val, false, A::More(M::ScrollBar));
    add(t, "", "", "SortType", val, false, A::More(M::SortType));
    // (The DDJ-RZ's own names for the browse functions: the same handlers.)
    add(t, "", "", "ForwardDdjRz", b, false, A::Forward);
    add(t, "", "", "LoadDdjRz", b, true, A::Load);
    add(t, "", "", "AddToTagListDdjRz", b, false, A::TagList);
    add(t, "", "", "RelatedTracksDdjRz", b, false, A::Related);
    for p in 1..=8u8 {
        add(t, "", "", &format!("PAD{p}_VelocitySampler+VelocityRzx"), val, true, A::More(M::VelocityLevel(p - 1)));
    }
    v
}

// ---------------------------------------------------------------------- shared state

/// A light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Led {
    Off,
    On,
    /// Blinking in the row's `Blink=` cycle (500 ms without one).
    Blink,
    Value(u8),
    /// Blinking in a cycle the function sets itself, whatever the row says
    /// (`operateBlinkChange` with an interval: PLAY 1000 ms, CUE 500 ms).
    BlinkMs(u16),
}

/// PLAY and CUE lights as `UiPlayer::deviceUpdateLed` 0x10202a7f4 sets them: playing, both lit;
/// standing (or a CUE / hot cue preview held), PLAY blinks every 1000 ms; CUE is lit at the cue
/// point, dark at the end of the track, else blinks every 500 ms; no track, both dark.
pub fn transport_lights(loaded: bool, playing: bool, preview: bool, at_cue: bool, at_end: bool) -> (Led, Led) {
    if !loaded {
        return (Led::Off, Led::Off);
    }
    let play = if playing && !preview { Led::On } else { Led::BlinkMs(1000) };
    let cue = if at_end { Led::Off } else if (playing && !preview) || at_cue || preview { Led::On } else { Led::BlinkMs(500) };
    (play, cue)
}

/// What the workers need to know about a deck, refreshed by the interface every frame.
#[derive(Clone, Copy, Debug)]
pub struct DeckSnap {
    pub loaded: bool,
    pub cue_point: f64,
    /// Hot cues A to P: start and, for a loop, its end (seconds).
    pub hot: [Option<(f64, Option<f64>)>; 16],
    /// The tempo range as a fraction (0.10 = +-10 %).
    pub range: f64,
    pub follower: bool,
    /// Bend of a held pitch bend button, as the engine has it.
    pub button_bend: f64,
    pub stem_iso_on: bool,
    /// Crossfader side: 0 THRU, 1 A, 2 B.
    pub assign: u8,
    /// The on-screen position of the knob a pick-up compares with (LOW, MID, HIGH).
    pub eq: [f32; 3],
    pub stem_iso: [f32; 3],
    /// The auto loop's size in beats, and a beat's length (seconds).
    pub loop_beats: f64,
    pub beat_len: f64,
}

impl Default for DeckSnap {
    fn default() -> DeckSnap {
        DeckSnap { loaded: false, cue_point: 0.0, hot: [None; 16], range: 0.10, follower: false, button_bend: 0.0, stem_iso_on: false, assign: 0, eq: [0.5; 3], stem_iso: [0.5; 3], loop_beats: 4.0, beat_len: 0.5 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub perf: bool,
    pub count: u8,
    pub active: u8,
    pub decks: [DeckSnap; 4],
    pub curve: u8,
    /// The sampler: the bank, how many pads it has, which are loaded, and whether pads play
    /// (the sequencer's MUTE / ERASE modes give them another meaning).
    pub bank: u8,
    pub smp_pads: u8,
    pub smp_loaded: u16,
    pub smp_plain: bool,
    pub velocity_curve: u8,
    pub takeover_eq_iso: bool,
    pub rpm45: bool,
}

impl Default for Snapshot {
    fn default() -> Snapshot {
        Snapshot { perf: true, count: 2, active: 0, decks: [DeckSnap::default(); 4], curve: 1, bank: 0, smp_pads: 16, smp_loaded: 0, smp_plain: true, velocity_curve: 3, takeover_eq_iso: false, rpm45: false }
    }
}

/// An engine value a worker set ahead of the interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Param {
    ChFader,
    Trim,
    Eq(u8),
    CrossFader,
    Tempo,
}

/// What reaches the interface thread.
pub enum UiEvent {
    Control { device: String, tag: u32, deck: u8, val: Val, fast_done: bool, seq: u64 },
    /// A message seen while LEARN is armed.
    Learn { device: String, msg: [u8; 3] },
    /// A device came or went.
    Ports,
    /// A device was opened: its name and where its mapping came from.
    Connected { device: String, summary: String },
    /// A device's port went away.
    Gone { device: String },
}

struct OutState {
    queue: OutQueue,
    blink: Blinker,
    /// Blinking codes: the entry to light and its priority.
    blinking: HashMap<u16, (Entry, i32)>,
    conn: Option<midir::MidiOutputConnection>,
}

/// One connected controller.
pub struct Device {
    pub name: String,
    pub map: Mutex<MidiMap>,
    /// The rekordbox installed here has a factory mapping for it.
    pub factory: bool,
    /// rekordbox runs a hardware handshake with this model before it treats it as started.
    /// recordbox does not have one: the device is listed and its messages are read, but it
    /// is not supported and may not send anything.
    pub needs_auth: bool,
    pub has_output: bool,
    pub learning: AtomicBool,
    pub alive: AtomicBool,
    out_index: Mutex<HashMap<(u32, u8), Vec<Entry>>>,
    leds: Mutex<HashMap<(u32, u8), Led>>,
    /// Last values sent to outputs the registry does not name (jog ring, level meters).
    named: Mutex<HashMap<u16, u16>>,
    /// The names of the output rows (so that `set_named` passes over names the map lacks).
    out_names: Mutex<std::collections::HashSet<String>>,
    out: Mutex<OutState>,
    out_cv: Condvar,
    interval_ms: AtomicU64,
    jog: Mutex<[Jog; 4]>,
    scan: Mutex<[Scan; 4]>,
    /// Last Load press per deck (the DDJ-400 family's double press = Instant Doubles).
    load_at: Mutex<[Option<Instant>; 4]>,
    in_conn: Mutex<Option<midir::MidiInputConnection<()>>>,
    pub received: AtomicU64,
    pub sent: AtomicU64,
    /// CoreMIDI's unique id of the input port (empty for devices made without a port): a
    /// port that comes back under the same name with another id (a helper program
    /// restarted between two scans) is a new connection.
    pub port_id: String,
}

impl Device {
    /// After the map changed: mark the functions recordbox knows, index the outputs and
    /// forget what the LEDs were sent, so that they are all sent again.
    pub fn reindex(&self) {
        let reg = registry();
        let mut m = self.map.lock();
        m.tag_inputs(|n| reg.tag(n));
        let mut idx: HashMap<(u32, u8), Vec<Entry>> = HashMap::new();
        for o in &m.outputs {
            let tag = reg.tag(&o.name);
            if tag != 0 {
                idx.entry((tag, o.deck)).or_default().push(o.clone());
            }
        }
        self.interval_ms.store(m.out_interval_ms(), Relaxed);
        *self.out_names.lock() = m.outputs.iter().map(|o| o.name.clone()).collect();
        drop(m);
        *self.out_index.lock() = idx;
        self.leds.lock().clear();
    }

    pub fn out_keys(&self) -> Vec<(u32, u8)> {
        self.out_index.lock().keys().copied().collect()
    }

    /// Set the light of a function (for a deck). Sends only when it changed.
    pub fn set_led(&self, tag: u32, deck: u8, led: Led) {
        {
            let mut l = self.leds.lock();
            if l.get(&(tag, deck)) == Some(&led) {
                return;
            }
            l.insert((tag, deck), led);
        }
        let entries = self.out_index.lock().get(&(tag, deck)).cloned().unwrap_or_default();
        if entries.is_empty() {
            return;
        }
        let map = self.map.lock();
        let mut out = self.out.lock();
        for e in entries {
            out.blink.stop(e.code);
            out.blinking.remove(&e.code);
            if crate::midi_log::on() && out.conn.is_some() {
                let m = |v: u8| map.led_message(&e, v);
                match led {
                    Led::Blink | Led::BlinkMs(_) => {
                        let ms = if let Led::BlinkMs(ms) = led { ms as i32 } else if e.opts.blink != 0 { e.opts.blink } else { 500 };
                        crate::midi_log::tx(&self.name, &[m(127)[0], m(127)[1]], &format!("{} deck {}: blinking every {ms} ms ({:02X} / {:02X})", e.name, deck, m(127)[2], m(0)[2]));
                    }
                    Led::On => crate::midi_log::tx(&self.name, &m(127), &format!("{} deck {}: on", e.name, deck)),
                    Led::Off => crate::midi_log::tx(&self.name, &m(0), &format!("{} deck {}: off", e.name, deck)),
                    Led::Value(v) => crate::midi_log::tx(&self.name, &m(v), &format!("{} deck {}: value {v}", e.name, deck)),
                }
            }
            match led {
                Led::Blink | Led::BlinkMs(_) => {
                    out.blink.start(e.code, e.opts.blink, if let Led::BlinkMs(ms) = led { ms as i32 } else { 0 });
                    out.blinking.insert(e.code, (e.clone(), e.opts.priority));
                }
                Led::On => out.queue.push(map.led_message(&e, 127), e.opts.priority),
                Led::Off => out.queue.push(map.led_message(&e, 0), e.opts.priority),
                Led::Value(v) => out.queue.push(map.led_message(&e, v), e.opts.priority),
            }
        }
        drop(out);
        self.out_cv.notify_one();
    }

    /// Send a value to the outputs of a row by its name (rows the function registry does
    /// not have: the jog ring, the channel level meters, the loaded indicator). `value` gets
    /// the row and returns the value to send; a value equal to the last one sent is not sent.
    /// 14-bit rows (IndicatorHiRes, JogIndicatorHiRes) are sent as rekordbox's
    /// `MidiData::getLongMessage` 0x10132107c builds them: the upper seven bits on the code,
    /// the lower seven on the code + 0x10.
    pub fn set_named(&self, name: &str, deck: u8, value: &dyn Fn(&Entry) -> u16) {
        if !self.out_names.lock().contains(name) {
            return;
        }
        let entries: Vec<Entry> = self.map.lock().outputs.iter().filter(|e| e.name == name && e.deck == deck).cloned().collect();
        if entries.is_empty() {
            return;
        }
        let mut sent = false;
        {
            let mut last = self.named.lock();
            let mut out = self.out.lock();
            for e in entries {
                let hires = matches!(e.kind, Kind::IndicatorHiRes | Kind::JogIndicatorHiRes);
                let v = value(&e) & if hires { 0x3FFF } else { 0x7F };
                if last.get(&e.code) == Some(&v) {
                    continue;
                }
                last.insert(e.code, v);
                let (st, d1) = ((e.code >> 8) as u8, (e.code & 0xFF) as u8);
                if hires {
                    out.queue.push([st, d1, (v >> 7) as u8 & 0x7F], e.opts.priority);
                    out.queue.push([st, d1.wrapping_add(0x10), v as u8 & 0x7F], e.opts.priority);
                } else {
                    out.queue.push([st, d1, v as u8], e.opts.priority);
                }
                sent = true;
            }
        }
        if sent {
            self.out_cv.notify_one();
        }
    }

    /// The output thread: one message per `MidiOutInterval`, and the 50 ms blink tick.
    fn run_output(self: Arc<Self>) {
        let mut next_tick = Instant::now() + Duration::from_millis(50);
        while self.alive.load(Relaxed) {
            let mut out = self.out.lock();
            if Instant::now() >= next_tick {
                next_tick += Duration::from_millis(50);
                let changes = out.blink.tick();
                if !changes.is_empty() {
                    if let Some(map) = self.map.try_lock() {
                        for (code, on) in changes {
                            if let Some((e, prio)) = out.blinking.get(&code).cloned() {
                                out.queue.push(map.led_message(&e, if on { 127 } else { 0 }), prio);
                            }
                        }
                    }
                }
            }
            match out.queue.pop() {
                Some(msg) => {
                    if let Some(c) = out.conn.as_mut() {
                        let _ = c.send(&msg);
                    }
                    self.sent.fetch_add(1, Relaxed);
                    drop(out);
                    std::thread::sleep(Duration::from_millis(self.interval_ms.load(Relaxed).clamp(1, 16)));
                }
                None => {
                    let wait = next_tick.saturating_duration_since(Instant::now()).min(Duration::from_millis(50));
                    self.out_cv.wait_for(&mut out, wait);
                }
            }
        }
    }
}

/// The value a hot cue pad's light gets (`UiPlayer::deviceUpdateHotCueLed` 0x10202af74): the
/// cue's colour code when it has one, else the colour of its slot in the HOT CUE colour set
/// (`rekordbox::hotCueColorIndex*`; 36 for a loop). Pad rows then map it for their family
/// (`midi_map::pad_led`), button rows light for it. Codes above 64 are not sent (0).
pub fn hot_cue_led(color: u8, slot: i32, is_loop: bool, set: u8) -> u8 {
    let v = if color != 0 { color } else { crate::pads::hot_cue_set_color(set, slot, is_loop) };
    if v <= 64 { v } else { 0 }
}

/// `PREPDECK_MIDI_PROBE=<port name>` (with `PREPDECK_SHOT_PLAYER=midiprobe`): a real
/// controller's input port is opened for `secs` seconds and every message is logged
/// ("midiprobe:" lines on stderr). Its output port is never opened, so nothing at all is sent
/// to the device. Afterwards each message goes through MIDI Learn's filter
/// (`LearnFilter`): the code Learn would write is checked against an empty mapping (it binds
/// to a learnable command) and looked up, read-only, in the user mapping rekordbox keeps for
/// the port (`~/Library/Application Support/Pioneer/rekordbox6/MidiMappings/<port>.midi.csv`),
/// to name the function rekordbox would run.
pub fn probe(port: &str, secs: f64) {
    let say = |s: String| eprintln!("midiprobe: {s}");
    let Ok(mut input) = midir::MidiInput::new("recordbox-probe") else { return say("CoreMIDI client could not be made".into()) };
    input.ignore(midir::Ignore::None);
    let names: Vec<String> = input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect();
    say(format!("input ports: {names:?}"));
    if let Ok(o) = midir::MidiOutput::new("recordbox-probe-list") {
        let outs: Vec<String> = o.ports().iter().filter_map(|p| o.port_name(p).ok()).collect();
        say(format!("output ports (listed only, none is opened): {outs:?}"));
    }
    let Some(p) = input.ports().into_iter().find(|p| input.port_name(p).map(|n| n == port).unwrap_or(false)) else { return say(format!("{port}: no such input port")) };
    let log: Arc<Mutex<Vec<(u64, Vec<u8>)>>> = Arc::new(Mutex::new(vec![]));
    let l2 = log.clone();
    let t0 = Instant::now();
    let conn = input.connect(&p, "recordbox-probe-in", move |_, m, _| l2.lock().push((t0.elapsed().as_millis() as u64, m.to_vec())), ());
    let Ok(conn) = conn else { return say(format!("{port}: could not be opened")) };
    say(format!("{port}: listening for {secs:.1} s (input only; nothing is sent to the device)"));
    std::thread::sleep(Duration::from_secs_f64(secs));
    drop(conn);
    let msgs = log.lock().clone();
    for (t, m) in &msgs {
        say(format!("{t:>6} ms  {}", m.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")));
    }
    say(format!("{} messages in {secs:.1} s", msgs.len()));
    // rekordbox's own mapping for the port (read only).
    let rb_user = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Pioneer/rekordbox6/MidiMappings").join(format!("{port}.midi.csv")));
    let rb_map = rb_user.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| MidiMap::load(port, None, Some(&b)).ok());
    say(format!("rekordbox's user mapping for the port: {}", match (&rb_user, &rb_map) { (Some(f), Some(m)) => format!("{} ({} output rows)", f.display(), m.outputs.len()), _ => "none".into() }));
    let factory = crate::midi_csv::factory_file(&crate::midi_csv::factory_dir(), port);
    say(format!("factory mapping in the installed rekordbox: {}", factory.map(|f| f.display().to_string()).unwrap_or_else(|| "none".into())));
    let raw: Vec<Vec<u8>> = msgs.iter().map(|m| m.1.clone()).collect();
    let learned = probe_learn(port, &raw, rb_map.as_ref());
    for (c, ok, in_rb) in &learned {
        say(format!("Learn code {c:04X}: binds {ok} | rekordbox's mapping: {in_rb}"));
    }
    let codes = learned;
    say(format!("{} distinct codes Learn would take; {}", codes.len(), if codes.is_empty() { "nothing was touched on the controller during the run" } else { "PASS (every received control can be learned)" }));
}

/// `PREPDECK_MIDI_LOG=1`: every message a port receives is logged ("midilog:" on stderr) with
/// the function its row names and the value it gave.
pub fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("PREPDECK_MIDI_LOG").map(|v| v == "1").unwrap_or(false))
}

/// `recordbox --midi-probe "<port>" [secs]`: the read-only probe without a window.
/// `recordbox --midi-run "<port>|<port>" [secs]`: the named real ports are opened as
/// controllers on four engine decks (silence of 5 minutes on decks 1 and 2, no audio
/// output), with every message logged; the function each one ran and what it did to the
/// decks is printed. `PREPDECK_MIDI_NO_OUT=1` keeps the output ports closed. The mappings
/// are read from a throwaway folder (`PREPDECK_DATA`) or rekordbox's folder (read only).
pub fn cli(args: &[String]) -> bool {
    let at = |flag: &str| args.iter().position(|a| a == flag);
    if let Some(i) = at("--midi-probe") {
        let port = args.get(i + 1).cloned().unwrap_or_default();
        let secs = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(10.0);
        probe(&port, secs);
        return true;
    }
    // `recordbox --midi-virtual "<name>" [secs]`: a stand-in controller in a process of its
    // own (as a USB device or a helper program is): a virtual source and destination of that
    // name for `secs` seconds; the source sends a PLAY press and release (90 0B 7F / 00)
    // every second; what the destination receives is counted. For checking that another
    // process's port coming and going is seen.
    if let Some(i) = at("--midi-virtual") {
        use midir::os::unix::{VirtualInput, VirtualOutput};
        let name = args.get(i + 1).cloned().unwrap_or_else(|| "recordbox stand-in".into());
        let secs: f64 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(5.0);
        let got = Arc::new(AtomicU64::new(0));
        let g2 = got.clone();
        let out = midir::MidiOutput::new("recordbox-virtual").ok().and_then(|o| o.create_virtual(&name).ok());
        let inp = midir::MidiInput::new("recordbox-virtual").ok().and_then(|i| i.create_virtual(&name, move |_, _, _| { g2.fetch_add(1, Relaxed); }, ()).ok());
        let (Some(mut out), Some(_inp)) = (out, inp) else {
            eprintln!("midivirtual: {name}: could not be made");
            return true;
        };
        eprintln!("midivirtual: {name}: up for {secs:.1} s");
        let t0 = Instant::now();
        while t0.elapsed().as_secs_f64() < secs {
            let _ = out.send(&[0x90, 0x0B, 0x7F]);
            std::thread::sleep(Duration::from_millis(50));
            let _ = out.send(&[0x90, 0x0B, 0x00]);
            std::thread::sleep(Duration::from_millis(950));
        }
        eprintln!("midivirtual: {name}: closing; {} messages received", got.load(Relaxed));
        return true;
    }
    let Some(i) = at("--midi-run") else { return false };
    let ports: Vec<String> = args.get(i + 1).map(|s| s.split('|').map(|p| p.to_string()).collect()).unwrap_or_default();
    let secs: f64 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
    for d in &decks[..2] {
        let n = 44100 * 300;
        d.lock().audio = Some(Arc::new(crate::audio::Decoded { sr: 44100, samples: vec![0.0; n * 2], raw_frames: n, damaged: 0, truncated: false }));
    }
    let (hub, rx) = Hub::new(decks.clone(), crate::db::data_dir().join("MidiMappings"));
    {
        let mut s = hub.snap.lock();
        s.decks[0].loaded = true;
        s.decks[1].loaded = true;
    }
    *hub.only.lock() = Some(ports.clone());
    hub.scan();
    let devs: Vec<String> = hub.devices.lock().iter().map(|d| format!("{} (mapping: {}, output {})", d.name, hub.user_source(&d.name).map(|p| p.display().to_string()).unwrap_or_else(|| "none".into()), if d.has_output { "open" } else { "closed" })).collect();
    eprintln!("midirun: opened {devs:?} for {secs:.1} s");
    let t0 = Instant::now();
    let mut handled = 0;
    let mut by_fn: std::collections::BTreeMap<String, usize> = Default::default();
    while t0.elapsed().as_secs_f64() < secs {
        // The outputs that follow the engine (PLAY / CUE, jog ring, meters, loaded), as the
        // app's light thread sends them; the app's own lights need the interface.
        hub.engine_outputs();
        let Ok(ev) = rx.recv_timeout(Duration::from_millis(50)) else { continue };
        if let UiEvent::Control { device, tag, deck, val, fast_done, .. } = ev {
            let Some(f) = registry().get(tag) else { continue };
            handled += 1;
            *by_fn.entry(f.name.clone()).or_default() += 1;
            let n = hub.deck_ix(deck).unwrap_or(0);
            let s = hub.decks[n].lock();
            eprintln!("midirun: [{device}] {} deck {deck} {val:?} -> {}; deck {}: playing {} pos {:.3} s tempo {:+.3} % bend {:+.4} jog {:?} ch_fader {:.3}", f.name, if fast_done { "done on the port's thread" } else { "for the interface thread" }, n + 1, s.playing, s.pos / 44100.0, (s.tempo - 1.0) * 100.0, s.bend, s.dk.jog, s.ch_fader);
        }
    }
    let rec: u64 = hub.devices.lock().iter().map(|d| d.received.load(Relaxed)).sum();
    let sent: u64 = hub.devices.lock().iter().map(|d| d.sent.load(Relaxed)).sum();
    eprintln!("midirun: {rec} messages received, {handled} ran a function, {sent} sent; by function: {by_fn:?}");
    for d in hub.devices.lock().iter() {
        d.alive.store(false, Relaxed);
        d.in_conn.lock().take();
    }
    true
}

/// The mapping rekordbox keeps for a port the user set up there
/// (`~/Library/Application Support/Pioneer/rekordbox6/MidiMappings/<port>.midi.csv`). Only
/// ever read. (Not in unit tests, which must not depend on this computer's rekordbox.)
pub fn rekordbox_user_file(port: &str) -> Option<PathBuf> {
    if cfg!(test) && std::env::var_os("PREPDECK_MIDI_RB_USER").is_none() && std::env::var_os("PREPDECK_MIDI_RB_DIR").is_none() {
        return None;
    }
    let dir = match std::env::var_os("PREPDECK_MIDI_RB_DIR") {
        // (A stand-in folder for tests and scripted runs.)
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Pioneer/rekordbox6/MidiMappings"),
    };
    let f = dir.join(format!("{port}.midi.csv"));
    f.is_file().then_some(f)
}

/// MIDI Learn on received messages: the codes its filter passes, whether each binds to a
/// learnable command (PlayPause, deck 1) of an empty mapping, and what `rb` (rekordbox's
/// mapping for the port, when there is one) runs for it.
pub fn probe_learn(port: &str, msgs: &[Vec<u8>], rb: Option<&MidiMap>) -> Vec<(u16, bool, String)> {
    let mut filter = LearnFilter::default();
    let mut codes: Vec<u16> = vec![];
    for m in msgs {
        if let Some(c) = filter.feed(m) {
            if !codes.contains(&c) {
                codes.push(c);
            }
        }
    }
    let reg = registry();
    codes
        .into_iter()
        .map(|c| {
            let mut map = MidiMap::new(port);
            let row = LearnEntry { name: "PlayPause".into(), deck: 1, kind: Kind::Button, min: -1, max: -1, comment: String::new() };
            let ok = map.change_input(&row, None, Some(c), &|n: &str| reg.learnable(n)).is_ok() && map.input_at(c).is_some();
            let in_rb = rb.and_then(|m| m.input_at(c)).map(|e| format!("{} (deck {})", e.name, e.deck)).unwrap_or_else(|| "-".into());
            (c, ok, in_rb)
        })
        .collect()
}

/// The models whose MIDI side rekordbox authenticates (`getDeviceAuth` overrides).
pub fn needs_auth(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["DDJ-1000", "DDJ-800", "DDJ-400", "DDJ-200", "DDJ-RB", "DDJ-RR", "DDJ-RZ", "DDJ-WEGO4", "DDJ-FLX6", "DDJ-XP1", "DDJ-XP2", "DJM-250MK2", "DJM-450", "DJM-900NXS2", "DJM-TOUR1", "DJM-A9", "DJM-V10", "DJM-V5", "XDJ-RX", "XDJ-RR", "XDJ-XZ"].iter().any(|d| n.contains(d)) && !n.contains("XDJ-RX3")
}

/// Everything the port threads and the interface share.
pub struct Hub {
    pub devices: Mutex<Vec<Arc<Device>>>,
    pub decks: [Arc<Mutex<PlayState>>; 4],
    pub snap: Mutex<Snapshot>,
    overlay: Mutex<Vec<(u8, Param, f32, u64)>>,
    seq: AtomicU64,
    tx: crossbeam_channel::Sender<UiEvent>,
    pub ctx: Mutex<Option<eframe::egui::Context>>,
    /// Where the user's mappings are kept: `<data folder>/MidiMappings`.
    pub user_dir: PathBuf,
    pub factory_dir: PathBuf,
    /// A CUE held on a stopped deck is playing from the cue point.
    cue_preview: [AtomicBool; 4],
    /// A hot cue pad held on a stopped deck: where it started (bits of the seconds; NaN = none).
    hot_preview: [AtomicU64; 4],
    /// Pads held, per deck (for the on-screen pads, which end a hold when no pointer is down).
    pub pads_down: [AtomicU8; 4],
    jog_bend: [AtomicU64; 4],
    takeover: Mutex<[TempoTakeover; 4]>,
    pickup: Mutex<[[Pickup; 3]; 4]>,
    /// Jog vinyl mode per deck (on by default, as in rekordbox).
    pub vinyl: [AtomicBool; 4],
    pub shift: [AtomicBool; 4],
    /// GRID ADJUST (1) / GRID SLIDE (2) held per deck, and the platter's travel waiting to
    /// be applied (milliseconds of the track).
    pub grid: [AtomicU8; 4],
    pub grid_acc: Mutex<[f64; 4]>,
    /// Ports recordbox opened itself, by name: never taken as controllers.
    pub ignore: Mutex<Vec<String>>,
    /// When set, only these ports are opened (tests and the hardware scripted run: a real
    /// controller on the machine is then neither opened nor sent anything).
    pub only: Mutex<Option<Vec<String>>>,
    /// When the interface last ran `poll` (ms since the hub was made): while it is not
    /// called (the window is hidden or minimised, so no frames are drawn), the hub's own
    /// thread keeps the lights that follow the engine going.
    pub ui_seen_ms: AtomicU64,
    pub born: Instant,
    /// Where rekordbox's user mappings are read from, when not rekordbox's own folder (tests).
    pub rb_dir: Mutex<Option<PathBuf>>,
    /// When set, only the ports with these CoreMIDI unique ids are opened, inputs and outputs
    /// (the rehearsal's virtual ports, whose names are those of real ports on the machine).
    pub only_ids: Mutex<Option<Vec<String>>>,
    /// Ports taken as the device of another name (the rehearsal's stand-in ports: a port
    /// whose name holds "DDJ-S1" would be taken by the owner's helper program as its
    /// controller, so the stand-ins have names of their own).
    pub alias: Mutex<HashMap<String, String>>,
}

fn nan_bits() -> u64 {
    f64::NAN.to_bits()
}

impl Hub {
    pub fn new(decks: [Arc<Mutex<PlayState>>; 4], user_dir: PathBuf) -> (Arc<Hub>, crossbeam_channel::Receiver<UiEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let hub = Hub {
            devices: Mutex::new(vec![]),
            decks,
            snap: Mutex::new(Snapshot::default()),
            overlay: Mutex::new(vec![]),
            seq: AtomicU64::new(1),
            tx,
            ctx: Mutex::new(None),
            user_dir,
            factory_dir: crate::midi_csv::factory_dir(),
            cue_preview: std::array::from_fn(|_| AtomicBool::new(false)),
            hot_preview: std::array::from_fn(|_| AtomicU64::new(nan_bits())),
            pads_down: std::array::from_fn(|_| AtomicU8::new(0)),
            jog_bend: std::array::from_fn(|_| AtomicU64::new(0f64.to_bits())),
            takeover: Mutex::new([TempoTakeover::default(); 4]),
            pickup: Mutex::new([[Pickup::default(); 3]; 4]),
            vinyl: std::array::from_fn(|_| AtomicBool::new(true)),
            shift: std::array::from_fn(|_| AtomicBool::new(false)),
            grid: std::array::from_fn(|_| AtomicU8::new(0)),
            grid_acc: Mutex::new([0.0; 4]),
            ignore: Mutex::new(vec![]),
            only: Mutex::new(None),
            ui_seen_ms: AtomicU64::new(0),
            born: Instant::now(),
            rb_dir: Mutex::new(None),
            only_ids: Mutex::new(None),
            alias: Mutex::new(HashMap::new()),
        };
        (Arc::new(hub), rx)
    }

    fn wake(&self) {
        if let Some(c) = self.ctx.lock().as_ref() {
            c.request_repaint();
        }
    }

    pub fn device(&self, name: &str) -> Option<Arc<Device>> {
        self.devices.lock().iter().find(|d| d.name == name).cloned()
    }

    pub fn user_file(&self, device: &str) -> PathBuf {
        self.user_dir.join(format!("{device}.midi.csv"))
    }

    /// The device's mapping: the installed rekordbox's factory file, when there is one, with
    /// the user's file on top. A user file that cannot be read as this device's is left out.
    pub fn load_map(&self, device: &str) -> (MidiMap, bool) {
        let factory = crate::midi_csv::factory_file(&self.factory_dir, device).and_then(|p| std::fs::read(p).ok());
        // recordbox's own file for the device first. Without one, the mapping the user made
        // for it in rekordbox is used as it is (read only, re-read at every connect), so a
        // controller set up there works here at once; a change made in recordbox's MIDI
        // window is saved to recordbox's folder and takes over from then on.
        let user = self.user_source(device).and_then(|f| std::fs::read(f).ok());
        let map = MidiMap::load(device, factory.as_deref(), user.as_deref()).or_else(|_| MidiMap::load(device, factory.as_deref(), None)).unwrap_or_else(|_| MidiMap::new(device));
        (map, factory.is_some())
    }

    /// The user mapping a device is read from: recordbox's own file when it has a row, else
    /// the one the user made in rekordbox (read only). recordbox writes its own file with the
    /// header alone the first time it sees a port, as rekordbox does, so a file without a
    /// single row counts as none.
    pub fn user_source(&self, device: &str) -> Option<PathBuf> {
        let own = self.user_file(device);
        let has_rows = std::fs::read(&own).ok().map(|b| crate::midi_csv::CsvFile::parse(&b).map(|f| f.rows().next().is_some()).unwrap_or(true)).unwrap_or(false);
        if has_rows {
            return Some(own);
        }
        let rb = match self.rb_dir.lock().as_ref() {
            Some(d) => Some(d.join(format!("{device}.midi.csv"))).filter(|f| f.is_file()),
            None => rekordbox_user_file(device),
        };
        rb.or_else(|| own.is_file().then_some(own))
    }

    /// Where a device's mapping came from and how many rows were taken, for the log and the
    /// status line: "factory mapping 160 rows, mapping from rekordbox (<file>) 74 rows, 0
    /// rejected".
    pub fn mapping_summary(&self, dev: &Device) -> String {
        let mut parts = vec![];
        if let Some(f) = crate::midi_csv::factory_file(&self.factory_dir, &dev.name) {
            let n = std::fs::read(&f).ok().and_then(|b| crate::midi_csv::CsvFile::parse(&b).ok()).map(|c| c.rows().count()).unwrap_or(0);
            parts.push(format!("factory mapping {} ({n} rows)", f.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()));
        }
        match self.user_source(&dev.name) {
            Some(f) => {
                let n = std::fs::read(&f).ok().and_then(|b| crate::midi_csv::CsvFile::parse(&b).ok()).map(|c| c.rows().count()).unwrap_or(0);
                let whose = if f == self.user_file(&dev.name) { "recordbox's own mapping" } else { "mapping from rekordbox" };
                if n > 0 || parts.is_empty() {
                    parts.push(format!("{whose} {} ({n} rows)", f.display()));
                }
            }
            None if parts.is_empty() => parts.push("no mapping (MIDI Learn only)".into()),
            None => {}
        }
        let m = dev.map.lock();
        format!("{}; {} input codes, {} output rows, {} rows rejected", parts.join(" + "), m.live_inputs().count(), m.outputs.len(), m.rejected)
    }

    /// The mapping of the device comes from rekordbox's folder (recordbox has none of its own).
    pub fn from_rekordbox(&self, device: &str) -> bool {
        self.user_source(device).map(|p| p != self.user_file(device)).unwrap_or(false)
    }

    /// The device's mapping to recordbox's own folder (never rekordbox's). A mapping read
    /// from rekordbox's folder and not changed here is not written, so that recordbox keeps
    /// following what the user sets up in rekordbox until a change is made in recordbox.
    pub fn save_map(&self, dev: &Device) -> std::io::Result<()> {
        let text = dev.map.lock().save_text();
        if self.from_rekordbox(&dev.name) && self.load_map(&dev.name).0.save_text() == text {
            return Ok(());
        }
        std::fs::create_dir_all(&self.user_dir)?;
        std::fs::write(self.user_file(&dev.name), text)
    }

    /// Look at the MIDI ports: connect controllers that appeared, drop those that left.
    pub fn scan(self: &Arc<Self>) {
        let Ok(mut input) = midir::MidiInput::new("recordbox") else { return };
        input.ignore(midir::Ignore::None);
        let ignore = self.ignore.lock().clone();
        let only = self.only.lock().clone();
        let only_ids = self.only_ids.lock().clone();
        // One list of (port, name, id), made in one pass: a port whose name cannot be read
        // (CoreMIDI can list a port that is half removed while a device goes) is left out
        // without shifting the others.
        let ports: Vec<(midir::MidiInputPort, String, String)> = input
            .ports()
            .into_iter()
            .filter_map(|p| {
                let name = input.port_name(&p).ok()?;
                let id = p.id();
                Some((p, name, id))
            })
            // (With both lists set, a port in either is taken: the rehearsal's own ports by
            // id, a stand-in of another process by name.)
            .filter(|(_, n, id)| match (&only_ids, &only) {
                (Some(ids), Some(names)) => ids.contains(id) || names.contains(n),
                (Some(ids), None) => ids.contains(id),
                // (A test run's stand-in ports, named "rbxtest-...", are that run's alone: a
                // copy of the app that did not ask for them leaves them closed.)
                (None, Some(_)) => true,
                (None, None) => !n.starts_with("rbxtest-"),
            })
            .collect();
        // Gone: their threads end and their ports close. A device is its port (the unique
        // id), not its name: a helper program that restarts makes new ports of the same names.
        let mut changed = false;
        let mut gone: Vec<String> = vec![];
        let mut gone_devs: Vec<Arc<Device>> = vec![];
        {
            let mut devs = self.devices.lock();
            devs.retain(|d| {
                let there = if d.port_id.is_empty() { ports.iter().any(|p| p.1 == d.name) } else { ports.iter().any(|p| p.2 == d.port_id) };
                if !there {
                    d.alive.store(false, Relaxed);
                    d.in_conn.lock().take();
                    d.out_cv.notify_all();
                    changed = true;
                    gone.push(d.name.clone());
                    gone_devs.push(d.clone());
                }
                there
            });
        }
        for d in gone_devs {
            self.release_device(&d);
        }
        for name in gone {
            crate::midi_log::port(&name, "disconnected", "");
            let _ = self.tx.send(UiEvent::Gone { device: name });
        }
        let alias = self.alias.lock().clone();
        for (port, port_name, _) in ports.iter() {
            let name = alias.get(port_name).cloned().unwrap_or_else(|| port_name.clone());
            // rekordbox passes over its own port, "<error>" and "Arduino Leonardo".
            if only_ids.is_none() && only.as_ref().is_some_and(|o| !o.contains(&name)) {
                continue;
            }
            if ignore.contains(&name) || name == "<error>" || name == "Arduino Leonardo" || name.is_empty() || self.device(&name).is_some() {
                continue;
            }
            if self.connect(&name, port, port_name).is_some() {
                changed = true;
            }
        }
        if changed {
            let _ = self.tx.send(UiEvent::Ports);
            self.wake();
        }
    }

    fn connect(self: &Arc<Self>, name: &str, port: &midir::MidiInputPort, port_name: &str) -> Option<Arc<Device>> {
        // The user's file is made with its header the first time a device is seen.
        let uf = self.user_file(name);
        if !uf.exists() {
            let _ = std::fs::create_dir_all(&self.user_dir);
            let _ = std::fs::write(&uf, crate::midi_csv::empty_user_file(name));
        }
        let (map, factory) = self.load_map(name);
        // The output port of the same name, when the device has one. `PREPDECK_MIDI_NO_OUT=1`:
        // inputs only, nothing is ever sent to a device.
        let no_out = std::env::var("PREPDECK_MIDI_NO_OUT").map(|v| v == "1").unwrap_or(false);
        let only_ids = self.only_ids.lock().clone();
        let only = self.only.lock().clone();
        let conn = (!no_out).then(|| midir::MidiOutput::new("recordbox").ok()).flatten().and_then(|out| {
            let p = out.ports().into_iter().find(|p| out.port_name(p).map(|n| n == port_name).unwrap_or(false) && only_ids.as_ref().map(|o| o.contains(&p.id()) || only.as_ref().is_some_and(|n| n.iter().any(|x| x == port_name))).unwrap_or(true))?;
            out.connect(&p, "recordbox-out").ok()
        });
        let cpr = jog_counts(name);
        let dev = Arc::new(Device {
            name: name.to_string(),
            map: Mutex::new(map),
            factory,
            needs_auth: needs_auth(name),
            has_output: conn.is_some(),
            learning: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            out_index: Mutex::new(HashMap::new()),
            leds: Mutex::new(HashMap::new()),
            named: Mutex::new(HashMap::new()),
            out_names: Mutex::new(Default::default()),
            out: Mutex::new(OutState { queue: OutQueue::default(), blink: Blinker::default(), blinking: HashMap::new(), conn }),
            out_cv: Condvar::new(),
            interval_ms: AtomicU64::new(1),
            jog: Mutex::new(std::array::from_fn(|_| Jog::new(cpr))),
            scan: Mutex::new(Default::default()),
            load_at: Mutex::new([None; 4]),
            in_conn: Mutex::new(None),
            received: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            port_id: port.id(),
        });
        dev.reindex();
        // Lights that only show a button held start dark (the controller may still show
        // what another program left lit).
        if dev.has_output {
            for (tag, deck) in dev.out_keys() {
                if registry().get(tag).map(|f| echo_led(f.act)).unwrap_or(false) {
                    dev.set_led(tag, deck, Led::Off);
                }
            }
        }
        // The callback only queues; the worker does the rest.
        let (tx, rx) = crossbeam_channel::unbounded::<(u64, Vec<u8>)>();
        let mut input = midir::MidiInput::new("recordbox").ok()?;
        input.ignore(midir::Ignore::None);
        let conn = input.connect(port, "recordbox-in", move |t, msg, _| {
            let _ = tx.send((t, msg.to_vec()));
        }, ()).ok()?;
        *dev.in_conn.lock() = Some(conn);
        let (h, d) = (self.clone(), dev.clone());
        let _ = std::thread::Builder::new().name(format!("midi-in {name}")).spawn(move || h.run_input(d, rx));
        let d = dev.clone();
        let _ = std::thread::Builder::new().name(format!("midi-out {name}")).spawn(move || d.run_output());
        let summary = self.mapping_summary(&dev);
        eprintln!("### MIDI:open success [{name}] {summary}");
        crate::midi_log::port(name, "connected", &format!("{summary}; output {}", if dev.has_output { "open" } else if no_out { "closed (PREPDECK_MIDI_NO_OUT=1)" } else { "none" }));
        self.devices.lock().push(dev.clone());
        let _ = self.tx.send(UiEvent::Connected { device: name.to_string(), summary });
        Some(dev)
    }

    /// The worker of a port (rekordbox's `DeviceMidi::run`).
    fn run_input(self: Arc<Self>, dev: Arc<Device>, rx: crossbeam_channel::Receiver<(u64, Vec<u8>)>) {
        let mut last: Option<(f64, Instant)> = None;
        // The controller log: a turning jog is written as one line per 250 ms (the messages,
        // their counts, what the deck did), not a line per message.
        // (One gathering per jog code: two platters turning at once interleave.)
        let mut jog_agg: Vec<JogAgg> = vec![];
        let flush = |hub: &Hub, all: &mut Vec<JogAgg>, only_old: bool| {
            let mut keep = vec![];
            for a in all.drain(..) {
                if only_old && a.since.elapsed() < Duration::from_millis(250) {
                    keep.push(a);
                    continue;
                }
                let state = hub.deck_ix(a.deck).map(|n| hub.engine_state(n)).unwrap_or_default();
                crate::midi_log::rx(&dev.name, &a.first, &format!("x{} ({:+} counts in {:.0} ms) -> {} deck {} => {state}", a.count, a.sum, a.last_ms - a.first_ms, a.name, a.deck));
            }
            *all = keep;
        };
        while dev.alive.load(Relaxed) {
            if !jog_agg.is_empty() {
                flush(&self, &mut jog_agg, true);
            }
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok((t, msg)) => {
                    let t_ms = t as f64 / 1000.0;
                    last = Some((t_ms, Instant::now()));
                    dev.received.fetch_add(1, Relaxed);
                    if msg.first().map(|b| *b == 0xF0).unwrap_or(true) {
                        continue;
                    }
                    // Learn sees the message, and it still does what it is mapped to.
                    if dev.learning.load(Relaxed) && msg.len() == 3 {
                        let _ = self.tx.send(UiEvent::Learn { device: dev.name.clone(), msg: [msg[0], msg[1], msg[2]] });
                        self.wake();
                    }
                    let ev = dev.map.lock().handle(&msg, t_ms);
                    let logging = log_on() || crate::midi_log::on();
                    let what = if logging {
                        match &ev {
                            Some(e) => {
                                let name = dev.map.lock().inputs.get(e.entry).and_then(|i| i.as_ref()).map(|i| i.e.name.clone()).unwrap_or_default();
                                format!("{name} deck {} {:?}{}{}", e.deck, e.val, if e.fast { " fast" } else { "" }, if e.tag == 0 { " (no action in recordbox)" } else { "" })
                            }
                            None => match dev.map.lock().input_at(((msg[0] as u16) << 8) | msg.get(1).copied().unwrap_or(0) as u16) {
                                Some(e) => format!("{} (half of a 14-bit pair, or no value)", e.name),
                                None => "unmapped".into(),
                            },
                        }
                    } else {
                        String::new()
                    };
                    if log_on() {
                        let hex = msg.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
                        eprintln!("midilog: {:>8.1} ms [{}] {hex} -> {what}", t_ms, dev.name);
                    }
                    let jog_msg = ev.as_ref().map(|e| matches!(e.val, Val::Jog { .. })).unwrap_or(false);
                    if crate::midi_log::on() && !jog_msg && !jog_agg.is_empty() {
                        flush(&self, &mut jog_agg, false);
                    }
                    if crate::midi_log::on() && jog_msg {
                        if let Some(Event { val: Val::Jog { delta, .. }, deck, entry, .. }) = ev {
                            let i = match jog_agg.iter().position(|a| a.first[..2] == msg[..2]) {
                                Some(i) => i,
                                None => {
                                    jog_agg.push(JogAgg {
                                first: [msg[0], msg[1], msg.get(2).copied().unwrap_or(0)],
                                name: dev.map.lock().inputs.get(entry).and_then(|i| i.as_ref()).map(|i| i.e.name.clone()).unwrap_or_default(),
                                deck,
                                count: 0,
                                sum: 0,
                                first_ms: t_ms,
                                last_ms: t_ms,
                                since: Instant::now(),
                                    });
                                    jog_agg.len() - 1
                                }
                            };
                            let a = &mut jog_agg[i];
                            a.count += 1;
                            a.sum += delta;
                            a.last_ms = t_ms;
                        }
                    } else if crate::midi_log::on() && ev.as_ref().map(|e| !e.fast || e.tag == 0).unwrap_or(true) {
                        crate::midi_log::rx(&dev.name, &msg, &what);
                    }
                    if let Some(ev) = ev {
                        let peer = dev.map.lock().dual_message(&msg, &ev);
                        let fast_deck = (ev.fast && ev.tag != 0).then(|| self.deck_ix(ev.deck)).flatten();
                        self.dispatch(&dev, ev);
                        // A row done on this thread: the deck as the engine has it now.
                        if crate::midi_log::on() && ev.fast && ev.tag != 0 && !jog_msg {
                            let state = fast_deck.map(|n| self.engine_state(n)).unwrap_or_else(|| "no such deck in the layout".into());
                            crate::midi_log::rx(&dev.name, &msg, &format!("{what} => {state}"));
                        }
                        if let Some(p) = peer {
                            let pev = dev.map.lock().handle(&p, t_ms);
                            if let Some(pev) = pev {
                                self.dispatch(&dev, pev);
                            }
                        }
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // A bend ends 60 ms after its last count; a held platter that stopped
                    // holds the record.
                    if let Some((t, at)) = last {
                        let now = t + at.elapsed().as_secs_f64() * 1000.0;
                        for n in 0..4 {
                            let out = dev.jog.lock()[n].idle(now);
                            if out != JogOut::Nothing {
                                self.jog_out(n, out);
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
        flush(&self, &mut jog_agg, false);
    }

    fn dispatch(&self, dev: &Device, ev: Event) {
        let mut done = false;
        let mut seq = 0;
        if ev.tag != 0 {
            if let Some(f) = registry().get(ev.tag) {
                if ev.fast {
                    seq = self.seq.fetch_add(1, Relaxed);
                    done = self.fast(dev, f, ev.deck, ev.val, seq);
                }
            }
        }
        let _ = self.tx.send(UiEvent::Control { device: dev.name.clone(), tag: ev.tag, deck: ev.deck, val: ev.val, fast_done: done, seq });
        self.wake();
    }

    /// The deck a mapping's deck number means: 1..4, or the active deck for a row without one.
    pub fn deck_ix(&self, deck: u8) -> Option<usize> {
        let s = self.snap.lock();
        let n = if deck == 0 { s.active as usize } else { deck as usize - 1 };
        (n < s.count as usize && n < 4).then_some(n)
    }

    // ------------------------------------------------------------------ transport

    /// PLAY/PAUSE. A cue preview that is running becomes normal play.
    pub fn play_pause(&self, n: usize) {
        let mut s = self.decks[n].lock();
        if s.audio.is_none() {
            return;
        }
        if self.cue_preview[n].swap(false, Relaxed) {
            return;
        }
        s.playing = !s.playing;
    }

    /// CUE pressed or released. False: the deck stands away from the cue point, so the press
    /// sets the cue point there (the interface does that, with quantize).
    pub fn cue(&self, n: usize, down: bool) -> bool {
        let cue = self.snap.lock().decks[n].cue_point;
        let mut s = self.decks[n].lock();
        let Some(sr) = s.audio.as_ref().map(|a| a.sr as f64) else { return true };
        if !down {
            if self.cue_preview[n].swap(false, Relaxed) {
                s.playing = false;
                s.pos = cue * sr;
            }
            return true;
        }
        if s.playing {
            if !self.cue_preview[n].load(Relaxed) {
                s.playing = false;
                s.pos = cue * sr;
                s.loop_range = None;
            }
            true
        } else if (s.pos / sr - cue).abs() < 0.01 {
            self.cue_preview[n].store(true, Relaxed);
            s.playing = true;
            true
        } else {
            false
        }
    }

    /// A hot cue pad pressed or released. False: the slot is empty, so the press stores a
    /// hot cue (the interface does that).
    pub fn hot_cue(&self, n: usize, slot: usize, down: bool) -> bool {
        let hot = self.snap.lock().decks[n].hot.get(slot).copied().flatten();
        let mut s = self.decks[n].lock();
        let Some(sr) = s.audio.as_ref().map(|a| a.sr as f64) else { return true };
        if !down {
            self.pads_down[n].store(0, Relaxed);
            s.slip_hold = false;
            let start = f64::from_bits(self.hot_preview[n].swap(nan_bits(), Relaxed));
            if !start.is_nan() {
                s.playing = false;
                s.loop_range = None;
                s.pos = start * sr;
            }
            return true;
        }
        let Some((start, end)) = hot else { return false };
        self.pads_down[n].store(1, Relaxed);
        // SLIP on a playing deck: the cue sounds while the pad is held.
        if s.slip && s.playing && s.slip_pos.is_none() {
            s.slip_pos = Some(s.pos);
        }
        s.slip_hold = s.slip && s.playing;
        s.pos = start * sr;
        if let Some(e) = end.filter(|e| *e > start) {
            s.loop_range = Some((start, e));
        }
        if !s.playing {
            // (Gate Cue: held, it plays and returns; off, it plays on.)
            if crate::prefs_bridge::GATE_CUE.load(Relaxed) {
                self.hot_preview[n].store(start.to_bits(), Relaxed);
            }
            s.playing = true;
        }
        true
    }

    /// A pad is being held on deck `n` by a controller.
    pub fn pad_held(&self, n: usize) -> bool {
        self.pads_down[n.min(3)].load(Relaxed) != 0 || !f64::from_bits(self.hot_preview[n.min(3)].load(Relaxed)).is_nan()
    }

    // ------------------------------------------------------------------ absolute controls

    /// Put a mixer or tempo value into the engine. Returns false when it was not applied (a
    /// pick-up is waiting, or the control does not exist in the layout showing).
    pub fn set_param(&self, n: usize, p: Param, pos: f32) -> bool {
        let snap = *self.snap.lock();
        if !snap.perf {
            return false;
        }
        match p {
            Param::ChFader => self.decks[n].lock().ch_fader = crate::mixer::channel_fader_gain(pos, snap.curve),
            Param::Trim => self.decks[n].lock().trim = crate::mixer::trim_gain(pos),
            Param::Eq(b) => {
                let b = (b as usize).min(2);
                let d = &snap.decks[n];
                // Pick-up after the knobs changed their meaning (EQ / STEM ISO).
                let sw = if d.stem_iso_on { d.stem_iso[b] } else { d.eq[b] };
                if !self.pickup.lock()[n][b].feed(pos, sw) {
                    return false;
                }
                let mut s = self.decks[n].lock();
                if d.stem_iso_on {
                    let g = crate::audio::stem_iso_gain(pos);
                    match b {
                        0 => s.stem_iso[0] = g,
                        1 => {
                            s.stem_iso[1] = g;
                            s.stem_iso[2] = g;
                        }
                        _ => s.stem_iso[3] = g,
                    }
                } else {
                    s.eq_pos[b] = pos;
                }
            }
            Param::CrossFader => {
                let g = crate::audio::crossfader_gains(pos);
                for i in 0..(snap.count as usize).min(4) {
                    self.decks[i].lock().fader = match snap.decks[i].assign {
                        1 => g.0,
                        2 => g.1,
                        _ => 1.0,
                    };
                }
            }
            Param::Tempo => self.decks[n].lock().tempo = 1.0 + pos as f64,
        }
        true
    }

    /// The tempo slider at `raw` (0..16383): through the soft takeover. The tempo change to
    /// set (a fraction), when the slider has the deck.
    pub fn tempo_slider(&self, n: usize, raw: i32) -> Option<f64> {
        let d = self.snap.lock().decks[n];
        let deck_pct = (self.decks[n].lock().tempo - 1.0) * 100.0;
        let slider = tempo_slider_percent(raw, d.range * 100.0);
        self.takeover.lock()[n].feed(slider, deck_pct, d.loaded, d.follower).map(|p| p / 100.0)
    }

    pub fn takeover_state(&self, n: usize) -> TempoTakeover {
        self.takeover.lock()[n.min(3)]
    }

    /// The takeover of deck `n` has just started waiting (read once).
    pub fn takeover_notice(&self, n: usize) -> bool {
        self.takeover.lock()[n.min(3)].take_notice()
    }

    /// A controller holds the platter of deck `n` (rekordbox lets the needle strip seek
    /// then, Needle Lock or not).
    pub fn jog_touched(&self, n: usize) -> bool {
        self.devices.lock().iter().any(|d| d.jog.lock()[n.min(3)].touched())
    }

    pub fn arm_takeover(&self, n: usize) {
        self.takeover.lock()[n.min(3)].arm();
    }

    pub fn start_pickup(&self, n: usize) {
        for p in self.pickup.lock()[n.min(3)].iter_mut() {
            p.start();
        }
    }

    // ------------------------------------------------------------------ jog

    fn jog_out(&self, n: usize, out: JogOut) {
        match out {
            JogOut::Nothing => {}
            JogOut::Bend(b) => {
                self.jog_bend[n].store(b.to_bits(), Relaxed);
                let base = self.snap.lock().decks[n].button_bend;
                self.decks[n].lock().bend = base + b;
            }
            other => jog_scratch(self, n, other),
        }
    }

    // ------------------------------------------------------------------ the fast path

    /// What a `Fast` row (or the jog) does from the worker. True when the engine side is done
    /// and the interface only has to follow.
    fn fast(&self, dev: &Device, f: &Func, deck: u8, val: Val, seq: u64) -> bool {
        let Some(n) = self.deck_ix(deck) else { return false };
        let press = matches!(val, Val::On(true, _));
        let release = matches!(val, Val::On(false, _));
        let over = |p: Param, v: f32| self.overlay.lock().push((n as u8, p, v, seq));
        match f.act {
            Act::PlayPause => {
                if press {
                    self.play_pause(n);
                    self.transport_leds(n);
                }
                true
            }
            Act::Cue => {
                let done = if press || release { self.cue(n, press) } else { true };
                self.transport_leds(n);
                done
            }
            Act::HotCue(slot) => {
                if press || release {
                    self.hot_cue(n, slot as usize, press)
                } else {
                    true
                }
            }
            Act::SamplerPad(pad, velocity) => {
                let snap = *self.snap.lock();
                if !snap.perf || !snap.smp_plain {
                    return false;
                }
                // Decks 2 and 4 play the second eight when the sampler has sixteen slots.
                let pad = pad as usize + if n % 2 == 1 && snap.smp_pads > 8 { 8 } else { 0 };
                if pad >= snap.smp_pads as usize || snap.smp_loaded & (1 << pad) == 0 {
                    return press;
                }
                let Val::On(on, v) = val else { return true };
                if let Some(e) = crate::sampler::SAMPLER.lock().as_mut() {
                    if on {
                        let gain = if velocity { crate::sampler_ext::velocity_gain(snap.velocity_curve, v) } else { 1.0 };
                        // (A loop sample pressed while the sequencer records is refused; the
                        // interface says so.)
                        if e.launch(pad, snap.bank as usize, gain) {
                            return false;
                        }
                    } else {
                        e.stop(pad, snap.bank as usize, false);
                    }
                }
                true
            }
            Act::ChFader | Act::Trim | Act::Eq(_) | Act::CrossFader => {
                let Some(pos) = val.unit() else { return false };
                let p = match f.act {
                    Act::ChFader => Param::ChFader,
                    Act::Trim => Param::Trim,
                    Act::Eq(b) => Param::Eq(b),
                    _ => Param::CrossFader,
                };
                if self.set_param(n, p, pos) {
                    over(p, pos);
                    true
                } else {
                    // A pick-up is waiting: the interface has nothing to do either.
                    matches!(p, Param::Eq(_))
                }
            }
            Act::Tempo => {
                let Val::Abs { .. } = val else { return false };
                let raw = (val.unit().unwrap_or(0.5) * 16383.0).round() as i32;
                let raw = if val.unit() == Some(0.5) { 8192 } else { raw };
                match self.tempo_slider(n, raw) {
                    Some(adj) => {
                        self.set_param(n, Param::Tempo, adj as f32);
                        over(Param::Tempo, adj as f32);
                        true
                    }
                    // Not picked up yet (the direction LEDs show which way to go).
                    None => true,
                }
            }
            Act::MasterLevel => {
                let Some(pos) = val.unit() else { return false };
                for d in &self.decks {
                    d.lock().volume = pos;
                }
                true
            }
            Act::JogTouch => {
                let Val::On(on, _) = val else { return true };
                if self.grid[n].load(Relaxed) != 0 {
                    return true;
                }
                let playing = self.decks[n].lock().playing;
                let snap = *self.snap.lock();
                let out = {
                    let mut j = dev.jog.lock();
                    j[n].vinyl = self.vinyl[n].load(Relaxed);
                    j[n].rpm = if snap.rpm45 { 45.0 } else { 100.0 / 3.0 };
                    j[n].touch(on, playing)
                };
                self.jog_out(n, out);
                true
            }
            Act::Jog(scratch_row) => {
                let Val::Jog { delta, t_ms } = val else { return true };
                // GRID ADJUST / SLIDE held: the platter's travel goes to the grid
                // (`deviceJogValue`: jog x 60000 / rpm, in ms of the track).
                if self.grid[n].load(Relaxed) != 0 {
                    let k = dev.jog.lock()[n].k();
                    self.grid_acc.lock()[n] += delta as f64 * k;
                    return true;
                }
                let out = dev.jog.lock()[n].rotate(delta, t_ms, scratch_row);
                self.jog_out(n, out);
                true
            }
            Act::JogSearch => {
                let Val::Jog { delta, t_ms } = val else { return true };
                let k = dev.jog.lock()[n].k();
                let Some(secs) = dev.scan.lock()[n].value(delta, t_ms, k) else { return true };
                let mut s = self.decks[n].lock();
                if let Some(a) = s.audio.clone() {
                    s.pos = (s.pos + secs * a.sr as f64).clamp(0.0, a.frames() as f64);
                }
                true
            }
            _ => false,
        }
    }

    /// The outputs that follow the engine alone, sent from the interface every frame and,
    /// while it draws no frames, from the hub's own thread: PLAY / CUE, the jog ring
    /// (`JogPlayPauseIndicator(Short)`, a `JogIndicator` row: `Min + Max * angle / 360`,
    /// `MidiJogIndicator::setLongValue` 0x101324334; the platter turns at 33 1/3 rpm), the
    /// channel level meter (`ChannelLevel`, an `Indicator` row: `max(L, R) * 100`,
    /// `ChannelLevelDeviceUnit::timerCallback` 0x1000d5ca0, here the deck's level before the
    /// faders as the engine meters it) and `LoadedIndicator` (127 loaded, 0 empty: not traced).
    pub fn engine_outputs(&self) {
        let devs: Vec<Arc<Device>> = self.devices.lock().iter().filter(|d| d.has_output).cloned().collect();
        if devs.is_empty() {
            return;
        }
        // (All four decks, as rekordbox's four players: a two-deck layout's decks 3 and 4
        // report themselves too, stopped.)
        for n in 0..4 {
            self.transport_leds(n);
            let snap_d = self.snap.lock().decks[n];
            // (The platter's turn for a time on the record at 33 1/3 rpm.)
            let turn_of = |secs: f64| (secs * (100.0 / 3.0) / 60.0 * 360.0).rem_euclid(360.0) as i32;
            let (loaded, angle, level, looping, slipping, playing, remain, reverse, on_air, loop_beats) = {
                let s = self.decks[n].lock();
                let (looping, slipping, playing) = (s.loop_range.is_some(), s.slip && s.slip_pos.is_some(), s.playing);
                match s.audio.as_ref() {
                    Some(a) => {
                        let secs = s.pos / a.sr.max(1) as f64;
                        let dur = a.frames() as f64 / a.sr.max(1) as f64;
                        // (On air: the deck is heard at the master: playing, its channel fader
                        // and its crossfader side open. A decision; `MixerControlPanel` was
                        // not traced.)
                        let air = playing && s.ch_fader > 0.0 && s.fader > 0.0;
                        let lb = s.loop_range.map(|(a, b)| (b - a) / snap_d.beat_len.max(1e-9)).unwrap_or(snap_d.loop_beats);
                        (true, turn_of(secs), s.peak[0].max(s.peak[1]), looping, slipping, playing, dur - secs, s.dk.reverse, air, lb)
                    }
                    None => (false, 360, 0.0, false, false, false, f64::INFINITY, false, false, snap_d.loop_beats),
                }
            };
            let cue_angle = turn_of(snap_d.cue_point);
            let blink_on = |e: &Entry| (self.born.elapsed().as_millis() as u64 / (if e.opts.blink > 0 { e.opts.blink as u64 } else { 500 } / 2).max(1)) % 2 == 0;
            let lvl = ((level.clamp(0.0, 1.0) * 100.0) as i32).clamp(0, 127) as u8;
            for d in &devs {
                for deck in [n as u8 + 1, if n as u8 == self.snap.lock().active { 0 } else { 255 }] {
                    if deck == 255 {
                        continue;
                    }
                    for name in ["JogPlayPauseIndicator", "JogPlayPauseIndicatorShort"] {
                        d.set_named(name, deck, &|e: &Entry| crate::midi_map::jog_ring(angle, &e.opts) as u16);
                    }
                    d.set_named("ChannelLevel", deck, &|_| lvl as u16);
                    d.set_named("LoadedIndicator", deck, &|_| if loaded { 127 } else { 0 });
                    // (Not traced: lit while a loop plays / while SLIP runs underneath / while
                    // the deck plays.)
                    d.set_named("GetLoopActive", deck, &|_| if looping { 127 } else { 0 });
                    d.set_named("SlipStateForFlashing", deck, &|_| if slipping { 127 } else { 0 });
                    d.set_named("PlayingState", deck, &|_| if playing { 127 } else { 0 });
                    // The end of the track coming (the last 30 s while it plays: the waveform's
                    // end warning; the time is a decision), on air, the level meter's segments
                    // (WeGO: ChannelLevel.1..5, lit from 20 % steps of the meter), the loaded
                    // lights of the DDJ-RZ, REVERSE blinking, the cue point on the jog ring.
                    let warn = playing && remain < 30.0;
                    d.set_named("EndWarningIndicator", deck, &|_| if warn { 127 } else { 0 });
                    d.set_named("TriggerEndWarning", deck, &|_| if warn { 127 } else { 0 });
                    d.set_named("OnAirIndicator", deck, &|_| if on_air { 127 } else { 0 });
                    for seg in 1..=5u16 {
                        d.set_named(&format!("ChannelLevel.{seg}"), deck, &|_| if lvl as u16 >= seg * 20 - 10 { 127 } else { 0 });
                    }
                    d.set_named("LoadedIndicatorDdjRz", deck, &|_| if loaded { 127 } else { 0 });
                    d.set_named("LoadedIndicatorDdjRzWithoutFlashing", deck, &|_| if loaded { 127 } else { 0 });
                    d.set_named("ReverseIndicator", deck, &|e| if reverse && blink_on(e) { 127 } else { 0 });
                    if loaded {
                        d.set_named("JogCurrentCueIndicator", deck, &|e: &Entry| crate::midi_map::jog_ring(cue_angle, &e.opts) as u16);
                    }
                    // The DDJ-SR's loop size lights (`1/Beat`, `1Beat` ... `32Beat or
                    // 1/32Beat`): the size of the loop playing, else the auto loop's.
                    let (frac, whole) = if loop_beats < 1.0 { (true, (1.0 / loop_beats.max(1e-9)).round() as i32) } else { (false, loop_beats.round() as i32) };
                    d.set_named("BeatIndicatorNumerator", deck, &|_| if frac { 127 } else { 0 });
                    for k in [1, 2, 4, 8, 16, 32] {
                        d.set_named(&format!("BeatIndicator{k}"), deck, &|_| if whole == k { 127 } else { 0 });
                    }
                }
            }
        }
    }

    /// A device's port went away: what it held is let go, as if its releases had come. The
    /// platter it touched or bent, and a CUE / hot cue preview or pad a controller held.
    pub fn release_device(&self, dev: &Device) {
        for n in 0..4 {
            let out = dev.jog.lock()[n].let_go();
            if out != JogOut::Nothing {
                self.jog_out(n, out);
                self.jog_out(n, JogOut::Bend(0.0));
            }
            if self.cue_preview[n].load(Relaxed) {
                self.cue(n, false);
            }
            if self.pads_down[n].load(Relaxed) != 0 || !f64::from_bits(self.hot_preview[n].load(Relaxed)).is_nan() {
                self.hot_cue(n, 0, false);
            }
        }
    }

    /// A deck as the engine has it, for the log.
    pub fn engine_state(&self, n: usize) -> String {
        let s = self.decks[n.min(3)].lock();
        let sr = s.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
        format!(
            "deck {} {} pos {:.3} s tempo {:+.3} % bend {:+.4} jog {:?} search {:+.1} trim {:.3} eq {:.2}/{:.2}/{:.2} ch_fader {:.3} xf_gain {:.3}{}",
            n + 1,
            if s.audio.is_none() { "empty" } else if s.playing { "playing" } else { "paused" },
            s.pos / sr,
            (s.tempo - 1.0) * 100.0,
            s.bend,
            s.dk.jog,
            s.search,
            s.trim,
            s.eq_pos[0],
            s.eq_pos[1],
            s.eq_pos[2],
            s.ch_fader,
            s.fader,
            s.loop_range.map(|(a, b)| format!(" loop {a:.3}-{b:.3}")).unwrap_or_default()
        )
    }

    /// PLAY and CUE lights straight from the engine (so they follow a press without waiting
    /// for a frame): PLAY lit while playing and blinking while a loaded deck stands; CUE lit
    /// while playing or standing on the cue point, blinking elsewhere.
    pub fn transport_leds(&self, n: usize) {
        let reg = registry();
        let (play, cue) = (reg.tag("PlayPause"), reg.tag("Cue"));
        let cue_point = self.snap.lock().decks[n].cue_point;
        let preview = self.cue_preview[n].load(Relaxed) || !f64::from_bits(self.hot_preview[n].load(Relaxed)).is_nan();
        let (loaded, playing, at_cue, at_end) = {
            let s = self.decks[n].lock();
            match s.audio.as_ref() {
                Some(a) => (true, s.playing, (s.pos / a.sr as f64 - cue_point).abs() < 0.01, s.pos >= a.frames() as f64 - 1.0),
                None => (false, false, false, false),
            }
        };
        let (pl, cl) = transport_lights(loaded, playing, preview, at_cue, at_end);
        let active = self.snap.lock().active as usize;
        for d in self.devices.lock().iter() {
            d.set_led(play, n as u8 + 1, pl);
            d.set_led(cue, n as u8 + 1, cl);
            if n == active {
                d.set_led(play, 0, pl);
                d.set_led(cue, 0, cl);
            }
        }
    }
}

/// A jog's messages gathered for one log line.
struct JogAgg {
    first: [u8; 3],
    name: String,
    deck: u8,
    count: u32,
    sum: i32,
    first_ms: f64,
    last_ms: f64,
    since: Instant,
}

/// The platter has the record: scratch, let go, or spin on.
///
/// The deck engine's jog (deck.rs) does the work: `Scratch(speed)` holds the record at the
/// platter's speed (negative = backwards), `Release` gives it back to normal play. A platter
/// let go while spinning (`Spin`) is treated as a release: the worker gets no further speeds
/// to follow the spin down with.
pub fn jog_scratch(hub: &Hub, n: usize, out: JogOut) {
    let jog = match out {
        JogOut::Scratch(speed) => crate::deck::Jog::Speed(speed),
        _ => crate::deck::Jog::None,
    };
    hub.jog_bend[n].store(0f64.to_bits(), Relaxed);
    hub.decks[n].lock().dk.jog = jog;
}

// ---------------------------------------------------------------------- the app's side

/// MIDI state of the app (a field of `App`).
pub struct MidiState {
    pub hub: Option<Arc<Hub>>,
    rx: Option<crossbeam_channel::Receiver<UiEvent>>,
    /// The CoreMIDI client made on the main thread, kept so that port changes are noticed.
    _keeper: Option<midir::MidiInput>,
    pub win: crate::ui_midi::Win,
    /// Device names, as the MIDI setting window lists them.
    pub devices: Vec<String>,
    /// The beat jump pads' page per deck (1..9; 5 puts 1 beat on the first pad).
    pub jump_page: [u8; 4],
    /// Instant Doubles asked for: (deck, source deck), waiting for the track to be read.
    pending_double: Option<(u8, u8)>,
    /// What the stubs were last asked (for the features other parts are building).
    pub stub_log: Vec<String>,
    /// The tempo each deck had after the last frame, to arm the takeover when it is changed
    /// on screen.
    last_tempo: [f64; 4],
    last_iso: [bool; 4],
    last_follow: [bool; 4],
    started: bool,
    /// Messages that were mapped, for the scripted check.
    pub handled: u64,
    /// FX1 / FX2 TapBeatWithLongPress held since (a second makes it AUTO).
    pub tap_held: [Option<Instant>; 2],
    /// A controller holds SEARCH FWD / REV on the deck (the window may have no focus then).
    pub search_held: [bool; 4],
    /// The state of the functions of `midi_acts.rs` (MASTER CUE, velocity levels ...).
    pub more: crate::midi_acts::MoreState,
    /// The Pad Editor slot (0..7) a controller's pad mode button last chose, per deck.
    pub editor_slot: [u8; 4],
}

impl Default for MidiState {
    fn default() -> MidiState {
        MidiState { hub: None, rx: None, _keeper: None, win: Default::default(), devices: vec![], jump_page: [5; 4], pending_double: None, stub_log: vec![], last_tempo: [0.0; 4], last_iso: [false; 4], last_follow: [false; 4], started: false, handled: 0, tap_held: [None; 2], search_held: [false; 4], more: Default::default(), editor_slot: [0; 4] }
    }
}

impl MidiState {
    /// A controller holds a pad of deck `n` (0 = deck 1).
    pub fn pad_held(&self, n: u8) -> bool {
        self.hub.as_ref().map(|h| h.pad_held(n as usize)).unwrap_or(false)
    }
    pub fn connected(&self) -> bool {
        !self.devices.is_empty()
    }
}

fn start(app: &mut App) {
    app.midi.started = true;
    if std::env::var("PREPDECK_MIDI").map(|v| v == "off").unwrap_or(false) {
        return;
    }
    let (hub, rx) = Hub::new(app.player.decks.clone(), crate::db::data_dir().join("MidiMappings"));
    *hub.ctx.lock() = Some(app.ctx.clone());
    // The hardware probe (`probe`) has the port to itself: the hub opens nothing.
    if std::env::var("PREPDECK_MIDI_PROBE").is_ok() {
        *hub.only.lock() = Some(vec![]);
    }
    // The window's scripted runs use their own virtual controller only: a real controller on
    // the machine is not opened by them.
    if std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| matches!(x.trim(), "miditest" | "mids1test"))).unwrap_or(false) || std::env::var("PREPDECK_SHOT_MIDI").is_ok() {
        *hub.only.lock() = Some(vec![crate::ui_midi::TEST_NAME.to_string()]);
    }
    // The rehearsal's virtual ports only (their names are those of the real rig): by id.
    if crate::midi_rig::on() {
        *hub.only_ids.lock() = Some(crate::midi_rig::ids().unwrap_or_default());
        for i in 0..3 {
            hub.alias.lock().insert(crate::midi_rig::PORT_NAMES[i].to_string(), crate::midi_rig::PORTS[i].to_string());
        }
    }
    // The first client is made here, on the main thread, whose run loop delivers CoreMIDI's
    // notices of devices coming and going.
    app.midi._keeper = midir::MidiInput::new("recordbox").ok();
    let h = hub.clone();
    let _ = std::thread::Builder::new().name("midi-ports".into()).spawn(move || loop {
        h.scan();
        std::thread::sleep(Duration::from_millis(1000));
    });
    // While the window draws no frames (hidden, minimised: macOS sends it no redraws), the
    // lights that follow the engine are kept going from here, every 50 ms.
    let w = Arc::downgrade(&hub);
    if crate::midi_log::on() {
        crate::midi_log::note(&format!("MIDI started; mappings: recordbox's {}, rekordbox's {} (read only); outputs {}", crate::db::data_dir().join("MidiMappings").display(), rekordbox_user_file("x").map(|f| f.parent().map(|p| p.display().to_string()).unwrap_or_default()).unwrap_or_else(|| "~/Library/Application Support/Pioneer/rekordbox6/MidiMappings".into()), if std::env::var("PREPDECK_MIDI_NO_OUT").map(|v| v == "1").unwrap_or(false) { "closed (PREPDECK_MIDI_NO_OUT=1)" } else { "open" }));
    }
    let _ = std::thread::Builder::new().name("midi-leds".into()).spawn(move || {
        let mut tick = 0u64;
        let mut last: HashMap<String, (u64, u64)> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(50));
            let Some(h) = w.upgrade() else { return };
            let now = h.born.elapsed().as_millis() as u64;
            if now.saturating_sub(h.ui_seen_ms.load(Relaxed)) > 150 {
                h.engine_outputs();
            }
            // The controller log: each port's message counts every 10 s while they change.
            tick += 1;
            if tick % 200 == 0 && crate::midi_log::on() {
                for d in h.devices.lock().iter() {
                    let c = (d.received.load(Relaxed), d.sent.load(Relaxed));
                    if last.get(&d.name) != Some(&c) {
                        crate::midi_log::sent_summary(&d.name, c.0, c.1);
                        last.insert(d.name.clone(), c);
                    }
                }
            }
        }
    });
    app.midi.hub = Some(hub);
    app.midi.rx = Some(rx);
}

fn snapshot(app: &App) -> Snapshot {
    let count = app.deck_count();
    let mut decks = [DeckSnap::default(); 4];
    for n in 0..count.min(4) {
        let d = app.deck_of(n);
        let mut hot = [None; 16];
        for c in d.cues.iter().filter(|c| c.kind == CueKind::Hot && (0..16).contains(&c.slot)) {
            hot[c.slot as usize] = Some((c.start, c.end));
        }
        let m = app.px.mix[n as usize];
        let follower = app.two_players() && d.x.beat_sync && (n != app.px.master || app.sampler_is_master());
        let range = if app.performance() { crate::player_ext::TEMPO_RANGES[(d.x.tempo_range as usize + 1) % 4] / 100.0 } else { crate::player_ext::TEMPO_RANGE };
        let auto = crate::prefs_bridge::auto_beat_loop_beats(&app.prefs);
        let loop_beats = if n == app.px.active { auto } else { app.px.parked_loop[n as usize].unwrap_or(auto) };
        let bpm = crate::player_ext::bpm_at(&d.beats, d.cue_point);
        let beat_len = if bpm > 0.0 { 60.0 / bpm } else { 0.5 };
        decks[n as usize] = DeckSnap { loaded: d.audio.is_some(), cue_point: d.cue_point, hot, range, follower, button_bend: d.x.bend as f64 * crate::player_ext::BEND_AMOUNT, stem_iso_on: m.stem_iso_on, assign: m.assign, eq: m.eq, stem_iso: m.stem_iso, loop_beats, beat_len };
    }
    let bank = app.prefs.sampler.bank.min(3);
    let pads = app.sampler_pads().min(16);
    let mut loaded = 0u16;
    for p in 0..pads {
        if app.sampler_loaded(bank as usize * 16 + p) && app.px.sampler.ready(bank as usize * 16 + p) {
            loaded |= 1 << p;
        }
    }
    Snapshot { perf: app.performance(), count, active: app.px.active, decks, curve: app.prefs.ch_fader_curve, bank, smp_pads: pads as u8, smp_loaded: loaded, smp_plain: app.px.sampler.seq_mode == 0, velocity_curve: app.prefs.sampler.velocity_curve, takeover_eq_iso: app.prefs.midi.takeover_eq_iso, rpm45: app.prefs.midi.jog_45rpm }
}

/// Once per frame, before the decks are handed to the audio thread: what the controllers
/// sent, and the lights.
/// The first connection to the system's MIDI server takes about half a second. It is made
/// on a helper thread first, so the main thread's own client (needed there for port-change
/// notices) is created quickly once the server is up, without holding up the first frames.
fn midi_server_ready() -> bool {
    use std::sync::atomic::AtomicU8;
    static STATE: AtomicU8 = AtomicU8::new(0);
    match STATE.load(Relaxed) {
        0 => {
            STATE.store(1, Relaxed);
            let spawned = std::thread::Builder::new().name("midi-warmup".into()).spawn(|| {
                let _ = midir::MidiInput::new("recordbox warm-up").map(|c| c.port_count());
                STATE.store(2, Relaxed);
            });
            if spawned.is_err() {
                STATE.store(2, Relaxed);
            }
            false
        }
        1 => false,
        _ => true,
    }
}

pub fn poll(app: &mut App) {
    if !app.midi.started {
        if !midi_server_ready() {
            return;
        }
        start(app);
    }
    let Some(hub) = app.midi.hub.clone() else { return };
    hub.ui_seen_ms.store(hub.born.elapsed().as_millis() as u64, Relaxed);
    let have = !hub.devices.lock().is_empty();
    let pending = app.midi.rx.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
    if !have && !pending {
        if !app.midi.devices.is_empty() {
            app.midi.devices.clear();
        }
        return;
    }
    *hub.snap.lock() = snapshot(app);
    // The takeover is armed again when the tempo was changed on screen or sync changed, and
    // the EQ knobs wait for a pick-up when they switch between EQ and STEM ISO.
    for n in 0..app.deck_count().min(4) as usize {
        let d = app.deck_of(n as u8);
        let (tempo, iso, follow) = (d.x.tempo_adj, app.px.mix[n].stem_iso_on, d.x.beat_sync);
        if (tempo - app.midi.last_tempo[n]).abs() > 1e-9 || follow != app.midi.last_follow[n] {
            hub.arm_takeover(n);
        }
        if iso != app.midi.last_iso[n] && app.prefs.midi.takeover_eq_iso {
            hub.start_pickup(n);
        }
        app.midi.last_iso[n] = iso;
        app.midi.last_follow[n] = follow;
    }
    let events: Vec<UiEvent> = app.midi.rx.as_ref().map(|r| r.try_iter().collect()).unwrap_or_default();
    for ev in events {
        match ev {
            UiEvent::Ports => {
                app.midi.devices = hub.devices.lock().iter().map(|d| d.name.clone()).collect();
                crate::ui_midi::devices_changed(app);
            }
            UiEvent::Learn { device, msg } => crate::ui_midi::learn_message(app, &device, msg),
            UiEvent::Connected { device, summary } => connected_notice(app, &device, &summary, true),
            UiEvent::Gone { device } => connected_notice(app, &device, "", false),
            UiEvent::Control { device, tag, deck, val, fast_done, seq } => {
                if let Some(f) = registry().get(tag) {
                    app.midi.handled += 1;
                    apply(app, &hub, &device, f, deck, val, fast_done, seq);
                    if crate::midi_log::on() && !matches!(f.act, Act::Jog(_) | Act::JogSearch) {
                        crate::midi_log::did(&device, &f.name, deck, &ui_state(app, deck));
                    }
                }
            }
        }
    }
    for n in 0..app.deck_count().min(4) as usize {
        app.midi.last_tempo[n] = app.deck_of(n as u8).x.tempo_adj;
    }
    finish_double(app);
    crate::midi_acts::part_doubles_tick(app);
    crate::midi_acts::grid_tick(app, &hub);
    for u in 0..2 {
        if app.midi.tap_held[u].map(|t| t.elapsed() >= Duration::from_millis(1000)).unwrap_or(false) {
            app.midi.tap_held[u] = None;
            app.px.fx[u].auto = true;
        }
    }
    if have {
        *hub.snap.lock() = snapshot(app);
        leds(app, &hub);
        hub.engine_outputs();
    }
}

/// rekordbox's status line when a controller comes or goes (`MainComponent::initializeDevice`
/// 0x1009896b4 / `finalizeDevice` 0x10098ad88: "xxxxx is connected." / "xxxxx is
/// disconnected." with the device's display name). Where its mapping came from goes to the
/// controller log (`midi_log`), not to the status line, which keeps rekordbox's words.
fn connected_notice(app: &mut App, device: &str, summary: &str, connected: bool) {
    let name = display_name(device);
    if connected {
        app.toast(format!("{name} is connected."));
        let _ = summary;
    } else {
        app.toast(format!("{name} is disconnected."));
        // What the device held is let go (its releases will never come): a pitch bend or
        // search button held here; the platter and the pads in `Hub::release_device`.
        for n in 0..4u8 {
            if app.midi.search_held[n as usize] {
                app.midi.search_held[n as usize] = false;
                if n < app.deck_count() {
                    app.with_deck(n, |a| a.scan(0));
                }
            }
            if n < app.deck_count() && app.deck_of(n).x.bend != 0 {
                app.with_deck(n, |a| a.pitch_bend(0));
            }
        }
    }
}

/// A status line message that a stream of controller messages may repeat: shown again (its
/// time renewed) rather than stacked.
pub fn status(app: &mut App, text: &str, error: bool) {
    if let Some(t) = app.toasts.iter_mut().rev().find(|t| t.text == text) {
        t.until = Instant::now() + Duration::from_secs(if error { 8 } else { 5 });
        return;
    }
    if error {
        app.toast_err(text);
    } else {
        app.toast(text);
    }
}

/// The name rekordbox shows for a device (`DeviceDefs::getDisplayName` 0x1001a78d8).
pub fn display_name(device: &str) -> String {
    device.to_string()
}

/// The state a mapped message left behind, for the controller log: the deck, the mixer
/// channel, the FX units and the browser.
pub fn ui_state(app: &App, deck: u8) -> String {
    let n = if deck == 0 { app.px.active } else { deck - 1 };
    if n >= app.deck_count() {
        return format!("deck {} is not in the layout showing", n + 1);
    }
    let d = app.deck_of(n);
    let (playing, pos) = {
        let s = app.player.decks[n as usize].lock();
        (s.playing, s.audio.as_ref().map(|a| s.pos / a.sr as f64))
    };
    let m = app.px.mix[n as usize];
    let fx: Vec<String> = (0..2)
        .map(|u| {
            let f = &app.px.fx[u];
            let p = &f.params;
            if p.single {
                format!("FX{} {:?} on {} level {:.2} beat {} assign {:?} {}", u + 1, p.one.kind, p.one.on, p.one.level, f.one_idx, p.assign, if f.auto { "AUTO" } else { "TAP" })
            } else {
                format!("FX{} multi {:?} on {:?} level {:.2} assign {:?}", u + 1, p.slots.iter().map(|s| s.kind).collect::<Vec<_>>(), p.slots.iter().map(|s| s.on).collect::<Vec<_>>(), p.slots[0].level, p.assign)
            }
        })
        .collect();
    format!(
        "deck {} {} {} cue {:.3} tempo {:+.2} % range {} sync {} master {} mt {} slip {} | ch trim {:.2} eq {:.2}/{:.2}/{:.2} fader {:.2} cue {} | xfader {:.2} | {} | panels fx {} sampler {} | list row {:?} tree {}",
        n + 1,
        if d.audio.is_none() { "empty".to_string() } else if playing { "playing".to_string() } else { "paused".to_string() },
        pos.map(|p| format!("pos {p:.3} s")).unwrap_or_default(),
        d.cue_point,
        d.x.tempo_adj * 100.0,
        d.x.tempo_range,
        d.x.beat_sync,
        app.px.master == n,
        d.x.master_tempo,
        d.x.slip,
        m.trim,
        m.eq[0],
        m.eq[1],
        m.eq[2],
        m.fader,
        m.cue,
        app.px.crossfader,
        fx.join(" / "),
        app.prefs.perf_fx,
        app.prefs.perf_sampler,
        app.anchor,
        app.dialogs.ext.focus_tree
    )
}

/// Called at the end of `push_deck_controls`: the values a worker set since the interface
/// last looked are put back over what the interface just wrote, and the jog's bend is added.
pub fn after_push(app: &mut App) {
    let Some(hub) = app.midi.hub.as_ref() else { return };
    let pending: Vec<(u8, Param, f32, u64)> = hub.overlay.lock().clone();
    for (n, p, v, _) in pending {
        hub.set_param(n as usize, p, v);
    }
    for n in 0..4 {
        let b = f64::from_bits(hub.jog_bend[n].load(Relaxed));
        if b != 0.0 {
            hub.decks[n].lock().bend += b;
        }
    }
}

/// The app's deck for a mapping's deck number.
fn deck_of(app: &App, deck: u8) -> Option<u8> {
    let n = if deck == 0 { app.px.active } else { deck - 1 };
    (n < app.deck_count()).then_some(n)
}

fn browse_step(app: &mut App, n: i32) {
    if n == 0 {
        return;
    }
    if app.dialogs.ext.focus_tree {
        for _ in 0..n.abs().min(50) {
            crate::prefs_ext::tree_step(app, n.signum());
        }
        return;
    }
    if app.list_dirty {
        app.rebuild_list();
    }
    if app.list.is_empty() {
        return;
    }
    let next = match app.anchor {
        Some(cur) => (cur as i64 + n as i64).clamp(0, app.list.len() as i64 - 1) as usize,
        None => 0,
    };
    app.set_anchor(next);
    app.selected.clear();
    app.selected.insert(app.list[next]);
    app.scroll_to_selected = true;
}

/// How a browse encoder's value moves the track list (`BrowseBasicView::
/// eventRelativeValueChanged` 0x100330b50): a group size by the value's size (1 below 4, 2
/// below 12, 3 below 24, 8 below 64, else 12), then value / group rows that many times.
pub fn browse_rows(v: i32) -> (i32, i32) {
    let a = v.unsigned_abs();
    let group = if a < 4 { 1 } else if a < 12 { 2 } else if a < 24 { 3 } else if a < 64 { 8 } else { 12 };
    (v / group, group)
}

/// Forward on the tree: a closed folder opens and its first child is chosen. False when the
/// row is not a closed folder (the focus then moves on).
fn tree_forward(app: &mut App) -> bool {
    let View::Playlist(id) = app.view else { return false };
    if !app.node(id).map(|n| n.kind == NodeKind::Folder).unwrap_or(false) || app.expanded.contains(&id) {
        return false;
    }
    app.expanded.insert(id);
    let mut kids: Vec<&PlaylistNode> = app.nodes.iter().filter(|n| n.parent == id).collect();
    kids.sort_by_key(|n| n.seq);
    if let Some(first) = kids.first().map(|n| n.id) {
        app.set_view(View::Playlist(first));
    }
    true
}

/// Back on the tree: an open folder closes; else the folder holding the row is chosen and
/// closed.
fn tree_back(app: &mut App) {
    let View::Playlist(id) = app.view else { return };
    if app.node(id).map(|n| n.kind == NodeKind::Folder).unwrap_or(false) && app.expanded.contains(&id) {
        app.expanded.remove(&id);
        return;
    }
    let parent = app.node(id).map(|n| n.parent).unwrap_or(0);
    if parent != 0 {
        app.expanded.remove(&parent);
        app.set_view(View::Playlist(parent));
    }
}

/// Instant Doubles: the track of another deck comes onto this one at the same place.
fn inst_doubles(app: &mut App, n: u8) {
    let count = app.deck_count();
    let playing = |app: &App, k: u8| app.player.decks[k as usize].lock().playing;
    let src = (0..count).filter(|k| *k != n && app.deck_of(*k).track.is_some()).max_by_key(|k| playing(app, *k));
    let Some(src) = src else { return };
    let Some(id) = app.deck_of(src).track else { return };
    app.with_deck(n, |a| a.load_track_here(id));
    app.midi.pending_double = Some((n, src));
}

fn finish_double(app: &mut App) {
    let Some((n, src)) = app.midi.pending_double else { return };
    if app.deck_of(n).loading.is_some() {
        return;
    }
    app.midi.pending_double = None;
    if app.deck_of(n).audio.is_none() || app.deck_of(n).track != app.deck_of(src).track {
        return;
    }
    let (pos, playing) = {
        let o = app.player.decks[src as usize].lock();
        (o.pos, o.playing)
    };
    let (adj, mt) = (app.deck_of(src).x.tempo_adj, app.deck_of(src).x.master_tempo);
    let d = app.deck_of_mut(n);
    d.x.tempo_adj = adj;
    d.x.master_tempo = mt;
    {
        let mut s = app.player.decks[n as usize].lock();
        s.pos = pos;
        s.playing = playing;
    }
    app.push_deck_controls();
}

/// What a mapped message does, on the interface thread. `fast_done`: the worker already did
/// the engine side.
#[allow(clippy::too_many_arguments)]
pub fn apply(app: &mut App, hub: &Arc<Hub>, device: &str, f: &Func, deck: u8, val: Val, fast_done: bool, seq: u64) {
    let press = matches!(val, Val::On(true, _));
    let release = matches!(val, Val::On(false, _));
    let rel = if let Val::Rel(n) = val { n } else { 0 };
    let unit = val.unit();
    // Controllers act in PERFORMANCE mode; browsing works in both.
    let browse = matches!(f.act, Act::Browse | Act::BrowseStep(_) | Act::Forward | Act::Back | Act::TagList | Act::Related | Act::Area | Act::Sort(_));
    if !app.performance() && !browse {
        return;
    }
    let Some(n) = deck_of(app, deck) else { return };
    let ni = n as usize;
    // A button without a state of its own is lit while it is held: rekordbox's handlers end
    // with `operateOnChange(pressed)` on the function's own component (deviceMemoryCue,
    // devicePitchBend, deviceActiveLoop, deviceLoop ...).
    if (press || release) && echo_led(f.act) {
        let tag = registry().tag(&f.name);
        for d in hub.devices.lock().iter() {
            d.set_led(tag, deck, if press { Led::On } else { Led::Off });
        }
    }
    // An absolute value a worker already put into the engine: the app's own value follows.
    let settle = |app: &mut App| {
        hub.overlay.lock().retain(|o| o.3 > seq);
        let _ = app;
    };
    match f.act {
        Act::PlayPause => {
            if press {
                if fast_done {
                    app.with_deck(n, |a| {
                        a.deck.cue_preview = false;
                        if a.playing() {
                            a.sync_align();
                        }
                    });
                } else {
                    hub.play_pause(ni);
                    app.with_deck(n, |a| if a.playing() { a.sync_align() });
                }
            }
        }
        Act::Cue => {
            if (press || release) && (fast_done || hub.cue(ni, press)) {
                return;
            }
            if press {
                // The deck stands away from the cue point: the cue point is set here.
                app.with_deck(n, |a| {
                    if a.deck.audio.is_some() && !a.playing() {
                        let q = a.quantized(a.pos());
                        a.deck.cue_point = q;
                        a.seek(q);
                    }
                });
            }
        }
        Act::HotCue(slot) => {
            if (press || release) && !fast_done && !hub.hot_cue(ni, slot as usize, press) && press {
                // An empty slot: the hot cue (or the running loop) is stored there.
                app.with_deck(n, |a| a.hot_cue_down(slot as i32));
            } else if press {
                // (The loop a hot loop started is the deck's last loop.)
                if let Some(c) = app.deck_of(n).cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32).cloned() {
                    if let Some(e) = c.end.filter(|e| *e > c.start) {
                        app.deck_of_mut(n).last_loop = Some((c.start, e));
                    }
                }
            }
        }
        Act::HotCueDelete(slot) => {
            if press {
                app.with_deck(n, |a| {
                    if let Some(id) = a.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32).map(|c| c.id) {
                        a.delete_cue(id);
                    }
                });
            }
        }
        Act::TrackStart => {
            if press {
                app.with_deck(n, |a| a.seek(0.0));
            }
        }
        Act::Track(dir) => {
            if press {
                app.with_deck(n, |a| if dir > 0 { a.load_adjacent(1) } else { a.previous_track() });
            }
        }
        Act::Search(dir) => {
            // (FF / REW held: rekordbox's scan, also on a paused deck, `UiPlayer::eventFFOn`.)
            if press {
                app.midi.search_held[ni] = true;
                app.with_deck(n, |a| a.scan(dir));
            } else if release {
                app.midi.search_held[ni] = false;
                app.with_deck(n, |a| if a.searching() == dir { a.scan(0) });
            }
        }
        Act::Needle => {
            // `UiPlayer::eventAbsoluteValueChanged` 0x102029150: with Needle Lock on, only a
            // standing deck (or one whose platter is held) seeks; the value (0..16383) x
            // 0.024416 is one of 401 places, `eventLoudView` 0x102212e04 seeks to place / 400
            // of the track (leaving a loop it lands outside). A strip that sends 0 when the
            // finger lifts (the DDJ-S1) lands on the track's start, as in rekordbox.
            if let Val::Abs { v, .. } = val {
                let lock = app.prefs.midi.needle_lock;
                let held = hub.jog_touched(ni);
                app.with_deck(n, |a| {
                    if lock && a.playing() && !held {
                        status(a, "Needle Lock. Not available during playback.", true);
                    } else if a.deck.audio.is_some() {
                        let place = ((v.max(0) as f32) * 0.024416) as i32;
                        let t = (place as f32 * 0.0025) as f64 * a.deck.duration;
                        if a.loop_active().map(|(s, e)| t < s || t > e).unwrap_or(false) {
                            a.loop_exit();
                        }
                        a.seek(t);
                    }
                });
            }
        }
        Act::Slip => {
            if press {
                app.with_deck(n, |a| a.toggle_slip());
            }
        }
        Act::Quantize => {
            if press {
                app.with_deck(n, |a| a.prefs.quantize = !a.prefs.quantize);
                app.save_prefs();
            }
        }
        Act::QuantizeAll => {
            if press {
                let on = !app.prefs.quantize;
                app.prefs.quantize = on;
                app.px.parked_quantize = [on; 4];
                app.save_prefs();
            }
        }
        Act::MasterTempo => {
            if press {
                app.with_deck(n, |a| a.toggle_master_tempo());
            }
        }
        Act::Tempo => {
            let Val::Abs { .. } = val else { return };
            let adj = if fast_done {
                let a = hub.decks[ni].lock().tempo - 1.0;
                (hub.takeover_state(ni).state == 5 || !app.deck_of(n).audio.is_some()).then_some(a)
            } else {
                let raw = if unit == Some(0.5) { 8192 } else { (unit.unwrap_or(0.5) * 16383.0).round() as i32 };
                hub.tempo_slider(ni, raw)
            };
            if let Some(adj) = adj {
                if !app.with_deck(n, |a| a.sync_follower()) {
                    app.deck_of_mut(n).x.tempo_adj = adj;
                    app.midi.last_tempo[ni] = adj;
                }
            }
            // rekordbox's status line: the slider moved on a deck that follows a sync master
            // (`eventAbsoluteValueChanged` 0x102029150), or the takeover started waiting.
            if app.with_deck(n, |a| a.sync_follower()) && app.deck_of(n).audio.is_some() {
                status(app, "Invalid operation. In Sync mode, Tempo Slider is only available for Master Deck.", true);
            } else if hub.takeover_notice(ni) {
                status(app, "Move Tempo Slider to the current BPM value.", false);
            }
            settle(app);
        }
        Act::TempoRange => {
            if press {
                app.with_deck(n, |a| a.tempo_range_next());
            }
        }
        Act::TempoReset => {
            if press {
                app.with_deck(n, |a| a.tempo_reset());
            }
        }
        Act::TempoLed(_) => {}
        Act::Sync => {
            if press {
                app.with_deck(n, |a| a.toggle_beat_sync());
            }
        }
        Act::Master => {
            if press {
                app.with_deck(n, |a| a.set_master());
            }
        }
        Act::TapBpm => {
            if press {
                app.with_deck(n, |a| a.grid_tap());
            }
        }
        Act::PitchBend(dir) => {
            if press {
                app.with_deck(n, |a| a.pitch_bend(dir));
            } else if release {
                app.with_deck(n, |a| a.pitch_bend(0));
            }
        }
        Act::AutoLoop | Act::Loop4 => {
            if press {
                app.with_deck(n, |a| {
                    if a.loop_active().is_some() {
                        a.loop_exit();
                    } else {
                        let b = if f.act == Act::Loop4 { 4.0 } else { crate::prefs_bridge::auto_beat_loop_beats(&a.prefs) };
                        a.auto_loop(b);
                    }
                });
            }
        }
        Act::LoopResize(x) => {
            if press {
                app.with_deck(n, |a| a.resize_loop(x));
            }
        }
        Act::LoopSize => {
            if rel != 0 {
                app.with_deck(n, |a| a.resize_loop(if rel > 0 { 2.0 } else { 0.5 }));
            }
        }
        Act::LoopIn => {
            if press {
                app.with_deck(n, |a| a.loop_in_pressed());
            }
        }
        Act::LoopOut => {
            if press {
                app.with_deck(n, |a| a.loop_out_pressed());
            }
        }
        Act::Reloop => {
            if press {
                app.with_deck(n, |a| a.exit_reloop());
            }
        }
        Act::ReTrigger => {
            if press {
                app.with_deck(n, |a| match a.loop_active() {
                    Some((s, _)) => a.seek(s),
                    None => a.reloop(),
                });
            }
        }
        Act::BeatLoop(beats) => {
            if press {
                app.with_deck(n, |a| beat_loop(a, beats));
            }
        }
        Act::BeatLoopPad(i) => {
            if press {
                let beats = crate::player_ext::LOOP_SIZES[(i as usize).min(15)];
                app.with_deck(n, |a| beat_loop(a, beats));
            }
        }
        Act::GridHalfDouble(double) => {
            if press {
                app.with_deck(n, |a| a.grid_double_half(double));
            }
        }
        Act::KeyShift(dir) => {
            if press {
                app.with_deck(n, |a| a.key_shift_step(dir));
            }
        }
        Act::KeySync => {
            if press {
                app.with_deck(n, |a| a.toggle_key_sync());
            }
        }
        Act::KeyReset => {
            if press {
                app.with_deck(n, |a| a.key_reset());
            }
        }
        Act::StemSolo => {
            if press {
                app.prefs.stem_solo = !app.prefs.stem_solo;
                app.save_prefs();
            }
        }
        Act::PadMode(m) => {
            if press {
                // (Through the deck's own choice, so that rekordbox's refusals apply.)
                if !app.with_deck(n, |a| a.set_pad_mode(crate::pads::PadMode::from_code(m))) && m < 32 {
                    app.px.pad_mode[ni] = m;
                }
            }
        }
        Act::MemoryStep(dir) => {
            if press {
                app.with_deck(n, |a| a.memory_jump(dir));
            }
        }
        Act::MemorySet => {
            if press {
                app.with_deck(n, |a| a.memory_set());
            }
        }
        Act::MemoryDelete => {
            if press {
                app.with_deck(n, |a| a.memory_delete_current());
            }
        }
        Act::BeatJumpPad(i) => {
            if press {
                // rekordbox's cells FINE, 1/8 ... 128, each back and forward; the deck's page
                // (BeatJumpPage<n>, as the pads on screen) chooses the first cell on the pads
                // (page 5: 1 beat on pads 1 and 2). `BeatJumpView::onPadPressed` 0x101dda6c0.
                let page = app.prefs.pads.beat_jump_page[ni];
                let id = crate::pads::jump_page_first(page) + i;
                if id < 24 {
                    app.with_deck(n, |a| a.pad_beat_jump(id));
                }
            }
        }
        Act::BeatJumpPage(dir) => {
            if press {
                let pg = app.prefs.pads.beat_jump_page[ni];
                app.prefs.pads.beat_jump_page[ni] = crate::pads::jump_page_step(pg, dir > 0, false);
                app.save_prefs();
            }
        }
        Act::StemPart(src) => {
            if press {
                // (With three parts shown, INST stands for INST and BASS, as on screen.)
                let srcs: Vec<usize> = if app.prefs.stem_parts == 3 && src == 2 { vec![2, 1] } else { vec![src as usize] };
                app.with_deck(n, |a| {
                    if a.deck.track.is_some() {
                        crate::prefs_ext::stem_part_click(a, &srcs);
                    }
                });
            }
        }
        Act::StemFx(row) => {
            if press {
                let r = (row as usize).min(3);
                app.px.stem_fx[r] = !app.px.stem_fx[r];
            }
        }
        Act::FxLevel(u, s) => {
            if let Some(p) = unit {
                let fx = &mut app.px.fx[u as usize % 2];
                if fx.params.single {
                    if s == 0 {
                        fx.params.one.level = p;
                    }
                } else {
                    fx.params.slots[s as usize % 3].level = p;
                }
            }
        }
        Act::FxOn(u, s) => {
            if press {
                let fx = &mut app.px.fx[u as usize % 2];
                if fx.params.single {
                    if s == 0 {
                        fx.params.one.on = !fx.params.one.on;
                    }
                } else {
                    let sl = &mut fx.params.slots[s as usize % 3];
                    sl.on = !sl.on;
                }
            }
        }
        Act::FxSelect(u, s, _) if unit.is_some() => {
            // (A knob row: the effect at the knob's place across the list.)
            crate::midi_acts::apply(app, hub, device, crate::midi_acts::M::FxSelectKnob(u, s), deck, n, val);
        }
        Act::FxSelect(u, s, dir) => {
            // (A Rotary row steps the list by its direction, one effect per message.)
            let dir = if rel != 0 { rel.signum() } else { dir };
            if press || rel != 0 {
                let fx = &mut app.px.fx[u as usize % 2];
                let single = fx.params.single;
                if single && s != 0 {
                    return;
                }
                let slot = if single { 3 } else { s as usize % 3 };
                let cur = if single { fx.params.one.kind } else { fx.params.slots[slot].kind };
                let list: Vec<crate::fx::Kind> = crate::fx::list(single).map(|i| i.kind).collect();
                if let Some(i) = list.iter().position(|k| *k == cur) {
                    let k = list[(i as i32 + dir).rem_euclid(list.len() as i32) as usize];
                    fx.set_kind(slot, k);
                }
            }
        }
        Act::FxBeat(u, s, dir) => {
            if press {
                fx_beat(&mut app.px.fx[u as usize % 2], Some(s as usize), dir);
            }
        }
        Act::FxUnitBeat(u, dir) => {
            if press {
                // (A beat target chosen on the controller: that slot only.)
                let target = app.midi.more.beat_target[u as usize % 2].map(|s| s as usize);
                fx_beat(&mut app.px.fx[u as usize % 2], target, dir);
            }
        }
        Act::FxUnitBeatRot(u) => {
            if rel != 0 {
                fx_beat(&mut app.px.fx[u as usize % 2], None, rel.signum());
            } else if let Some(p) = unit {
                // A knob (`RbxFxDeviceComponent::eventAbsoluteValueChanged` 0x1005fe09c): its
                // position across the beat values (`setEffectBeatArea(v x 0.000061039)`), the
                // unit's effect in single mode, all three slots in multi mode.
                let fx = &mut app.px.fx[u as usize % 2];
                let slots: Vec<usize> = if fx.params.single { vec![3] } else { vec![0, 1, 2] };
                for sl in slots {
                    let kind = if sl < 3 { fx.params.slots[sl].kind } else { fx.params.one.kind };
                    let len = crate::fx::beat_table(kind).0.len();
                    if len > 0 {
                        fx.set_index(sl, ((p as f64 * len as f64) as usize).min(len - 1));
                    }
                }
            }
        }
        Act::FxMode(u) => {
            if press {
                let p = &mut app.px.fx[u as usize % 2].params;
                p.single = !p.single;
            }
        }
        Act::FxAuto(u) => {
            if press {
                app.px.fx[u as usize % 2].auto = true;
            }
        }
        Act::FxTap(u) => {
            if press {
                let now = app.ctx.input(|i| i.time);
                let fx = &mut app.px.fx[u as usize % 2];
                fx.auto = false;
                if let Some(bpm) = fx.tap.tap(now) {
                    fx.tap_bpm = bpm;
                }
            }
        }
        Act::FxReleaseType(u) => {
            if rel != 0 {
                let fx = &mut app.px.fx[u as usize % 2];
                fx.release_kind = (fx.release_kind as i32 + rel.signum()).rem_euclid(3) as u8;
            }
        }
        Act::FxReleaseBeat(u, dir) => {
            if press {
                let fx = &mut app.px.fx[u as usize % 2];
                let k = fx.release_kind as usize % 3;
                fx.release_idx[k] = (fx.release_idx[k] as i32 + dir).clamp(0, crate::fx::RELEASE_BEATS.len() as i32 - 1) as usize;
            }
        }
        Act::FxReleaseOn(u) => {
            let (press, release) = match val {
                Val::Abs { v, .. } => (v > 0, v == 0),
                _ => (press, release),
            };
            if press || release {
                let fx = &mut app.px.fx[u as usize % 2];
                let k = fx.release_kind as usize % 3;
                fx.params.release = press.then_some((k as u8, crate::fx::RELEASE_BEATS[fx.release_idx[k].min(crate::fx::RELEASE_BEATS.len() - 1)]));
            }
        }
        Act::FxAssign(u, ch) => {
            if press {
                let a = &mut app.px.fx[u as usize % 2].params.assign[(ch as usize).min(crate::fx::CHANNELS - 1)];
                *a = !*a;
            }
        }
        Act::CfxColor(ch) => {
            if let Some(p) = unit {
                if ch as usize == crate::fx::SAMPLER {
                    app.px.sampler.cfx_color = p;
                } else {
                    app.px.cfx_color[(ch as usize).min(4)] = p;
                }
            }
        }
        Act::CfxSelect(dir) => {
            if press {
                let list: Vec<u8> = crate::cfx::LIST.iter().filter_map(|x| x.1).collect();
                let cur = if app.px.cfx_user { app.px.cfx_kind } else { 0 };
                let i = list.iter().position(|k| *k == cur).unwrap_or(0) as i32;
                let k = list[(i + dir).rem_euclid(list.len() as i32) as usize];
                app.px.cfx_user = true;
                app.px.cfx_kind = k;
                app.px.cfx_param = app.px.fxx.cfx_params[k as usize % 9];
            }
        }
        Act::SamplerPad(pad, velocity) => {
            let Val::On(on, v) = val else { return };
            if !fast_done {
                let pad = pad as usize + if n % 2 == 1 && app.sampler_pads() > 8 { 8 } else { 0 };
                if on {
                    let gain = if velocity { crate::sampler_ext::velocity_gain(app.prefs.sampler.velocity_curve, v) } else { 1.0 };
                    app.sampler_pad_down(pad, gain, false, false);
                } else {
                    app.sampler_pad_up(pad);
                }
            }
        }
        Act::SamplerPadStop(pad) => {
            if press {
                let pad = pad as usize + if n % 2 == 1 && app.sampler_pads() > 8 { 8 } else { 0 };
                app.sampler_pad_down(pad, 1.0, true, false);
            }
        }
        Act::SamplerBank(dir) => {
            if press {
                let b = (app.prefs.sampler.bank as i32 + dir).clamp(0, 3) as u8;
                app.sampler_action(crate::sampler_ext::SmpAct::Bank(b));
            }
        }
        Act::SamplerGain => {
            if let Some(p) = unit {
                app.prefs.sampler.gain = p * 100.0;
            }
        }
        Act::SamplerVolume => {
            if let Some(p) = unit {
                app.prefs.sampler.volume = p * 100.0;
            } else if rel != 0 {
                // (An encoder, as the DDJ-ERGO has: one percent a step; not traced.)
                app.prefs.sampler.volume = (app.prefs.sampler.volume + rel as f32).clamp(0.0, 100.0);
            }
        }
        Act::SamplerCue => {
            if press {
                app.prefs.sampler.cue = !app.prefs.sampler.cue;
            }
        }
        Act::Smp(a) => {
            if press {
                let a = match a {
                    crate::sampler_ext::SmpAct::SeqLoad(_) => crate::sampler_ext::SmpAct::SeqLoad(n),
                    a => a,
                };
                app.sampler_action(a);
            }
        }
        Act::SeqCall(slot, stop) => {
            if press {
                app.sampler_seq_call(slot as usize, stop);
            }
        }
        Act::Trim => {
            if let Some(p) = unit {
                app.px.mix[ni].trim = p;
            }
            settle(app);
        }
        Act::Eq(b) => {
            if let Some(p) = unit {
                let b = (b as usize).min(2);
                // (A waiting pick-up leaves the on-screen knob where it is.)
                let applied = if fast_done { !hub.pickup.lock()[ni][b].waiting } else { hub.set_param(ni, Param::Eq(b as u8), p) };
                if applied {
                    let m = &mut app.px.mix[ni];
                    if m.stem_iso_on { m.stem_iso[b] = p } else { m.eq[b] = p }
                }
            }
            settle(app);
        }
        Act::StemIsoKnob(i) => {
            if let Some(p) = unit {
                app.px.mix[ni].stem_iso[(i as usize).min(2)] = p;
            }
        }
        Act::HeadCue => {
            if press {
                app.px.mix[ni].cue = !app.px.mix[ni].cue;
            }
        }
        Act::ChFader => {
            if let Some(p) = unit {
                app.px.mix[ni].fader = p;
            }
            settle(app);
        }
        Act::HpMix => {
            if let Some(p) = unit {
                app.px.hp_mix = p;
            }
        }
        Act::HpLevel => {
            if let Some(p) = unit {
                app.px.hp_level = p;
            }
        }
        Act::CrossFader => {
            if let Some(p) = unit {
                app.px.crossfader = p;
            }
            settle(app);
        }
        Act::XfAssign(side) => {
            if press {
                app.px.mix[ni].assign = side;
            }
        }
        Act::MasterLevel => {
            if let (Some(p), false) = (unit, fast_done) {
                app.player.state.lock().volume = p;
            }
        }
        Act::StemIso => {
            if press {
                app.px.mix[ni].stem_iso_on = !app.px.mix[ni].stem_iso_on;
            }
        }
        Act::Browse => {
            // `BrowseBasicView::eventRelativeValueChanged` 0x100330b50: 0 puts the focus on the
            // track list; otherwise the list moves `steps` rows `times` times
            // (`ListViewer::deviceKeyPress` 0x100412bc8), the tree by the value
            // (`CdjDispTree::eventDial`).
            if rel == 0 {
                if matches!(val, Val::Rel(0)) {
                    app.dialogs.ext.focus_tree = false;
                }
            } else if app.dialogs.ext.focus_tree {
                browse_step(app, rel);
            } else {
                let (steps, times) = browse_rows(rel);
                for _ in 0..times {
                    browse_step(app, steps);
                }
            }
        }
        Act::BrowseStep(d) => {
            if press {
                browse_step(app, d);
            }
        }
        Act::Forward => {
            if press {
                // `BrowseBasicView::eventForward` 0x10032edac with `CdjDispTree::
                // eventForward_TreeView` 0x1014bc700: on a closed folder of the tree, the folder
                // opens and its first child is chosen (the focus stays); otherwise the focus
                // goes on to the track list.
                if !(app.dialogs.ext.focus_tree && tree_forward(app)) {
                    app.dialogs.ext.focus_tree = false;
                }
            }
        }
        Act::Back => {
            if press {
                // `eventBack` 0x10032f1c8 / `eventBack_TreeView` 0x1014bca64: in the tree, an
                // open folder closes, else the folder holding the row is chosen and closed;
                // from the track list the focus goes back to the tree.
                if app.dialogs.ext.focus_tree {
                    tree_back(app);
                }
                app.dialogs.ext.focus_tree = true;
            }
        }
        Act::Area => {
            if press {
                app.dialogs.ext.focus_tree = !app.dialogs.ext.focus_tree;
            }
        }
        Act::TagList => {
            if press {
                if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
                    app.tag_list_add(&[id]);
                }
            }
        }
        Act::Related => {
            if press {
                let v = if app.view == View::Related { View::Collection } else { View::Related };
                app.set_view(v);
            }
        }
        Act::Sort(col) => {
            if press {
                if app.prefs.sort_col == col {
                    app.prefs.sort_asc = !app.prefs.sort_asc;
                } else {
                    app.prefs.sort_col = col;
                    app.prefs.sort_asc = true;
                }
                app.list_dirty = true;
                app.save_prefs();
            }
        }
        Act::Load => {
            if press {
                // The DDJ-400 and DDJ-FLX4: a second press within 300 ms is Instant Doubles.
                let double = (device.contains("DDJ-400") || device.contains("DDJ-FLX4"))
                    && hub.device(device).map(|d| {
                        let mut at = d.load_at.lock();
                        let hit = at[ni].map(|t| t.elapsed() < Duration::from_millis(300)).unwrap_or(false);
                        at[ni] = if hit { None } else { Some(Instant::now()) };
                        hit
                    }).unwrap_or(false);
                if double {
                    inst_doubles(app, n);
                } else {
                    if app.list_dirty {
                        app.rebuild_list();
                    }
                    if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
                        app.with_deck(n, |a| a.load_track_here(id));
                    }
                }
            }
        }
        Act::InstDoubles => {
            if press {
                inst_doubles(app, n);
            }
        }
        Act::Panel(p) => {
            if press {
                match p {
                    0 => app.prefs.perf_fx = !app.prefs.perf_fx,
                    1 => app.prefs.perf_sampler = !app.prefs.perf_sampler,
                    // (MixerPanel: the mixer panel shown or hidden, where the layout has one.)
                    _ => {
                        if crate::ui_perf_layouts::panel_enabled(app, 3) {
                            crate::ui_perf_layouts::toggle_mixer(app);
                        }
                    }
                }
                app.save_prefs();
            }
        }
        Act::Zoom => {
            if rel != 0 {
                let z = (app.prefs.zoom as i32 - rel.signum()).clamp(0, ZOOMS.len() as i32 - 1);
                app.prefs.zoom = z as usize;
            }
        }
        Act::Shift => {
            if press || release {
                hub.shift[ni].store(press, Relaxed);
            }
        }
        // The jog is the worker's alone.
        Act::Jog(_) | Act::JogSearch | Act::JogTouch => {}
        Act::LoopMoveRot => {
            if rel != 0 {
                app.with_deck(n, |a| a.loop_move(rel.signum()));
            }
        }
        Act::PanelStep(dir) => {
            if press {
                panel_step(app, dir);
            }
        }
        Act::BrowseView => {
            if press && app.performance() {
                if app.prefs.perf_lay.browse {
                    app.prefs.perf_lay.browse = false;
                    app.poll_decks();
                    app.save_prefs();
                } else {
                    crate::ui_perf_layouts::set_row(app, 4);
                }
            }
        }
        Act::CallStep(dir) => {
            if press {
                app.with_deck(n, |a| {
                    if a.loop_active().is_some() {
                        a.resize_loop(if dir > 0 { 2.0 } else { 0.5 });
                    } else {
                        a.memory_jump(dir);
                    }
                });
            }
        }
        Act::VinylAdjust(which) => {
            if let Val::Abs { v, .. } = val {
                vinyl_adjust(app, ni % 2, which, v);
            }
        }
        Act::FxTapLong(u) => {
            let u = u as usize % 2;
            if press {
                let now = app.ctx.input(|i| i.time);
                let fx = &mut app.px.fx[u];
                fx.auto = false;
                if let Some(bpm) = fx.tap.tap(now) {
                    fx.tap_bpm = bpm;
                }
                app.midi.tap_held[u] = Some(Instant::now());
            } else if release {
                app.midi.tap_held[u] = None;
            }
        }
        Act::FxSelectKind(u, sl, i) => {
            if press {
                let fx = &mut app.px.fx[u as usize % 2];
                let single = fx.params.single;
                let k = crate::fx::INFO[(i as usize).min(crate::fx::INFO.len() - 1)].kind;
                // (Only an effect the mode's list has; in single mode the unit's one effect.)
                if crate::fx::list(single).any(|x| x.kind == k) && (!single || sl == 0) {
                    fx.set_kind(if single { 3 } else { sl as usize % 3 }, k);
                }
            }
        }
        Act::XfAssignKnob => {
            if let Some(p) = unit {
                app.px.mix[ni].assign = if p < 0.25 { 1 } else if p > 0.75 { 2 } else { 0 };
            }
        }
        Act::CfxCenter(ch) => {
            if press {
                app.px.cfx_color[(ch as usize).min(4)] = 0.5;
            }
        }
        Act::FxAssignHold(u, ch) => {
            if press || release {
                fx_assign_set(&mut app.px.fx[u as usize % 2].params.assign, ch as usize, press);
            }
        }
        Act::Stub(s) => stub(app, hub, s, n, val),
        Act::More(m) => crate::midi_acts::apply(app, hub, device, m, deck, n, val),
    }
}

/// PanelSelect Next / Back (`MainComponent::eventOnChanged`, PanelForward / PanelBack): the
/// panels in rekordbox's order FX, SAMPLER, MIXER, REC (VIDEO is not in recordbox), then
/// none; the one after (or before) the last one showing is shown alone. Panels the layout
/// does not have are left as they are and passed over.
fn panel_step(app: &mut App, dir: i32) {
    use crate::ui_perf_layouts::{mixer_shown, panel_enabled};
    // (Tile numbers of `panel_enabled`: 0 FX, 2 Sampler, 3 Mixer, 4 Rec.)
    let tiles = [0usize, 2, 3, 4];
    let on = [app.prefs.perf_fx, app.prefs.perf_sampler, mixer_shown(app), app.prefs.rec.panel];
    let enabled: [bool; 4] = std::array::from_fn(|i| panel_enabled(app, tiles[i]));
    let pick = panel_next(on, enabled, dir);
    let want = |i: usize| pick == Some(i);
    if enabled[0] {
        app.prefs.perf_fx = want(0);
    }
    if enabled[1] {
        app.prefs.perf_sampler = want(1);
    }
    if enabled[2] && mixer_shown(app) != want(2) {
        crate::ui_perf_layouts::toggle_mixer(app);
    }
    if enabled[3] {
        app.prefs.rec.panel = want(3);
    }
    app.save_prefs();
}

/// The panel PanelForward (`dir` 1) or PanelBack (-1) shows next, of FX, SAMPLER, MIXER,
/// REC (None: none). The current one is the last one lit.
pub fn panel_next(on: [bool; 4], enabled: [bool; 4], dir: i32) -> Option<usize> {
    let cur = (0..4).rev().find(|i| on[*i] && enabled[*i]);
    let mut next: i32 = match (dir > 0, cur) {
        (true, Some(c)) => c as i32 + 1,
        (true, None) => 0,
        (false, Some(c)) => c as i32 - 1,
        (false, None) => 3,
    };
    while (0..4).contains(&next) && !enabled[next as usize] {
        next += dir.signum();
    }
    (0..4).contains(&next).then_some(next as usize)
}

/// The speed table index a Vinyl Speed Adjust knob position (0..16383) gives.
pub fn vinyl_index(v: i32) -> u8 {
    (((v.max(0) as f64) * 0.0155649149) as i32).clamp(0, 255) as u8
}

/// A Vinyl Speed Adjust knob (`UiPlayer::deviceVinylAdjust` 0x102029430): the knob's 14-bit
/// position times 0.0155649149 (0..255) is the speed table index; 0 sets Touch/Brake, 1
/// Release/Start, 2 both. Kept per side, as Preferences keeps them.
fn vinyl_adjust(app: &mut App, side: usize, which: u8, v: i32) {
    let idx = vinyl_index(v);
    let side = side.min(1);
    if which != 1 {
        app.prefs.rb.brake[side] = idx;
        app.prefs.deck.vinyl_brake[side] = idx;
    }
    if which != 0 {
        app.prefs.rb.start[side] = idx;
        app.prefs.deck.vinyl_start[side] = idx;
    }
    app.push_deck_controls();
}

/// An FX assign set to on or off as the on-screen button does it when it is clicked: the
/// master alone, or channels without the master.
pub fn fx_assign_set(a: &mut [bool; crate::fx::CHANNELS], c: usize, on: bool) {
    let c = c.min(crate::fx::CHANNELS - 1);
    if a[c] == on {
        return;
    }
    a[c] = on;
    if on {
        if c == crate::fx::MASTER {
            for (i, d) in a.iter_mut().enumerate() {
                if i != crate::fx::MASTER {
                    *d = false;
                }
            }
        } else {
            a[crate::fx::MASTER] = false;
        }
    }
}

fn beat_loop(a: &mut App, beats: f64) {
    let cur = a.loop_active().map(|(s, e)| (e - s) / a.beat_len().max(1e-9));
    if cur.map(|c| (c / beats - 1.0).abs() < 0.02).unwrap_or(false) {
        a.loop_exit();
    } else {
        a.prefs.loop_beats = beats;
        a.auto_loop(beats);
    }
}

/// Beat value one step up or down: of a slot, or of the unit (single mode: its effect; multi
/// mode: all three slots).
fn fx_beat(fx: &mut crate::ui_fx::FxUi, slot: Option<usize>, dir: i32) {
    let step = |fx: &mut crate::ui_fx::FxUi, s: usize| {
        let cur = if s < 3 { fx.idx[s] } else { fx.one_idx } as i32;
        fx.set_index(s, (cur + dir).max(0) as usize);
    };
    match (fx.params.single, slot) {
        (true, Some(0)) | (true, None) => step(fx, 3),
        (true, _) => {}
        (false, Some(s)) => step(fx, s % 3),
        (false, None) => {
            for s in 0..3 {
                step(fx, s);
            }
        }
    }
}

/// The one place where the commands of features built elsewhere arrive.
///
/// TO WIRE AFTER THE MERGE: each arm names what it should call. Until then a command is
/// noted in `app.midi.stub_log` and does the nearest thing recordbox has, if any.
pub fn stub(app: &mut App, hub: &Arc<Hub>, s: Stub, deck: u8, val: Val) {
    let press = matches!(val, Val::On(true, _));
    let release = matches!(val, Val::On(false, _));
    if !(press || release || val.unit().is_some() || matches!(val, Val::Rel(_))) {
        return;
    }
    let note = |app: &mut App, what: String| {
        app.midi.stub_log.push(what);
        let n = app.midi.stub_log.len();
        if n > 64 {
            app.midi.stub_log.drain(..n - 64);
        }
    };
    match s {
        // Recording: show / hide the REC panel.
        Stub::RecPanel => {
            if press {
                app.rec_toggle_panel();
                note(app, "RecPanel".into());
            }
        }
        // Automix: start / stop.
        Stub::AutoMixStartStop => {
            if press {
                app.automix_start_stop();
                note(app, "AutoMixStartStop".into());
            }
        }
        // MIX POINT LINK: 0 panel, 1 cue select next, 2 back, 3 set, 4 exit, 5 deck select, 6 silent.
        Stub::MixPointLink(cmd) => {
            if press {
                match cmd {
                    0 => app.mpl_toggle_panel(),
                    1 => app.mpl_select_cue(1),
                    2 => app.mpl_select_cue(-1),
                    3 => app.mpl_request_link(),
                    4 => {
                        for d in 0..4 {
                            app.mpl_cancel(d, 3);
                        }
                    }
                    5 => {
                        // Deck select: the deck's playhead becomes the point of the current mode.
                        let t = app.with_deck(deck, |a| a.pos());
                        app.mpl_wave_click(deck as usize, t, false);
                    }
                    _ => app.mpl_toggle_silent(),
                }
                note(app, format!("MixPointLink {cmd} deck {}", deck + 1));
            }
        }
        // PAD FX: pad mode of bank 1 / 2; a pad held; beat value up / down.
        Stub::PadFxMode(bank) => {
            if press {
                app.with_deck(deck, |a| a.set_pad_mode(crate::pads::PadMode::PadFx));
                note(app, format!("PadFxMode {bank} deck {}", deck + 1));
            }
        }
        Stub::PadFx(bank, pad) => {
            // (Pads 1..16 of a bank; the second bank's pads follow the first's.)
            let idx = ((bank.max(1) - 1) as u8) * 16 + pad as u8;
            if press {
                crate::ui_padfx::pad_down(app, deck as usize, idx);
            } else if release {
                crate::ui_padfx::pad_up(app, deck as usize, idx);
            }
            if press || release {
                note(app, format!("PadFx {bank} pad {} {} deck {}", pad + 1, if press { "on" } else { "off" }, deck + 1));
            }
        }
        Stub::PadFxBeat(bank, dir) => {
            if press {
                crate::ui_padfx::beat_shift(app, deck as usize, dir < 0);
                note(app, format!("PadFxBeat {bank} {dir:+} deck {}", deck + 1));
            }
        }
        // Slicer / Slicer Loop: pad mode, a section pad, shift, roll, length, capture.
        Stub::SlicerMode(lp) => {
            if press {
                app.with_deck(deck, |a| {
                    if a.set_pad_mode(crate::pads::PadMode::Slicer) && a.slicer_now().1 != lp {
                        a.slicer_toggle_loop();
                    }
                });
                note(app, format!("SlicerMode loop={lp} deck {}", deck + 1));
            }
        }
        Stub::SlicerPad(lp, pad) => {
            if press || release {
                app.with_deck(deck, |a| a.slicer_pad(pad as u8, press));
                note(app, format!("SlicerPad loop={lp} pad {} {} deck {}", pad + 1, if press { "on" } else { "off" }, deck + 1));
            }
        }
        Stub::SlicerShift(lp, dir) | Stub::SlicerRoll(lp, dir) | Stub::SlicerLength(lp, dir) => {
            if press {
                let n = (deck as usize).min(3);
                match s {
                    Stub::SlicerShift(..) => app.with_deck(deck, |a| a.slicer_shift(dir > 0)),
                    Stub::SlicerRoll(..) => {
                        let ri = (app.prefs.pads.slicer_roll[n] as i32 + dir.signum() as i32).max(0) as usize;
                        app.with_deck(deck, |a| a.slicer_set_roll(ri));
                    }
                    _ => {
                        let li = (app.prefs.pads.slicer_length[n] as i32 + dir.signum() as i32).clamp(0, 3) as usize;
                        app.with_deck(deck, |a| a.slicer_set_length(li));
                    }
                }
                note(app, format!("{s:?} loop={lp} {dir:+} deck {}", deck + 1));
            }
        }
        Stub::SlicerCapture => {
            if press {
                app.with_deck(deck, |a| {
                    a.slicer_capture();
                });
                note(app, format!("SlicerCapture deck {}", deck + 1));
            }
        }
        // Reverse (toggle) and Slip Reverse (while held).
        Stub::Reverse => {
            if press {
                app.with_deck(deck, |a| a.reverse_toggle(false));
                note(app, format!("Reverse deck {}", deck + 1));
            }
        }
        Stub::SlipReverse => {
            if press {
                app.with_deck(deck, |a| a.reverse_toggle(true));
            } else if release {
                app.with_deck(deck, |a| a.reverse_off());
            }
            if press || release {
                note(app, format!("SlipReverse {} deck {}", if press { "on" } else { "off" }, deck + 1));
            }
        }
        // Vinyl mode: the jog's switch follows already; the deck's VINYL button should too.
        Stub::Vinyl => {
            if press {
                let v = &hub.vinyl[(deck as usize).min(3)];
                v.store(!v.load(Relaxed), Relaxed);
                note(app, format!("Vinyl {} deck {}", if v.load(Relaxed) { "on" } else { "off" }, deck + 1));
            }
        }
        // Vinyl Speed Adjust (Touch/Brake) of the left / right decks: 0..1.
        Stub::VinylBrake(side) => {
            if let Val::Abs { v, .. } = val {
                vinyl_adjust(app, side as usize, 0, v);
                note(app, format!("VinylBrake side {side} {v}"));
            }
        }
        Stub::AutoCue => {
            if press {
                crate::prefs_bridge::toggle_auto_cue(app, deck);
                note(app, format!("AutoCue deck {}", deck + 1));
            }
        }
        // Move the running loop one beat.
        Stub::LoopMove(dir) => {
            if press {
                let moved = app.with_deck(deck, |a| {
                    let had = a.loop_active().is_some();
                    a.loop_move(dir as i32);
                    had
                });
                note(app, format!("LoopMove {dir:+} deck {} ({})", deck + 1, if moved { "moved" } else { "no loop" }));
            }
        }
        // The memory loop at the cue point is made an active loop or not (`eventActiveLoop`).
        Stub::ActiveLoop => {
            if press {
                app.with_deck(deck, |a| {
                    let cp = a.deck.cue_point;
                    if let Some(mut c) = a.deck.cues.iter().find(|c| c.kind == CueKind::Memory && c.end.is_some() && (c.start - cp).abs() < 0.005).cloned() {
                        c.active_loop = !c.active_loop;
                        a.update_cue(c);
                    }
                });
                note(app, format!("ActiveLoop deck {}", deck + 1));
            }
        }
        Stub::SyncRate => {
            if press {
                app.with_deck(deck, |a| a.sync_rate_toggle());
                note(app, format!("SyncRate deck {}", deck + 1));
            }
        }
    }
}

/// The lights of every connected controller, from the app's state.
fn leds(app: &mut App, hub: &Arc<Hub>) {
    let devs: Vec<Arc<Device>> = hub.devices.lock().iter().filter(|d| d.has_output).cloned().collect();
    if devs.is_empty() {
        return;
    }
    let reg = registry();
    let count = app.deck_count();
    // Each deck once.
    struct D {
        loaded: bool,
        playing: bool,
        at_cue: bool,
        looping: bool,
        slip: bool,
        searching: i32,
        reverse: bool,
    }
    let ds: Vec<D> = (0..4u8)
        .map(|n| {
            if n >= count {
                return D { loaded: false, playing: false, at_cue: false, looping: false, slip: false, searching: 0, reverse: false };
            }
            let cue = app.deck_of(n).cue_point;
            let s = app.player.decks[n as usize].lock();
            let pos = s.audio.as_ref().map(|a| s.pos / a.sr as f64);
            D { loaded: pos.is_some(), playing: s.playing, at_cue: pos.map(|p| (p - cue).abs() < 0.01).unwrap_or(false), looping: s.loop_range.is_some(), slip: s.slip, searching: if s.search > 0.0 { 1 } else if s.search < 0.0 { -1 } else { 0 }, reverse: s.dk.reverse }
        })
        .collect();
    let on = |b: bool| if b { Led::On } else { Led::Off };
    let (seq_playing, seq_rec, seq_cur) = crate::sampler::SAMPLER.try_lock().and_then(|g| g.as_ref().map(|e| (e.seq.playing, e.seq.rec, e.seq.current()))).unwrap_or((false, false, 0));
    for dev in devs {
        for (tag, deck) in dev.out_keys() {
            let Some(f) = reg.get(tag) else { continue };
            let Some(n) = deck_of(app, deck) else {
                dev.set_led(tag, deck, Led::Off);
                continue;
            };
            let ni = n as usize;
            let d = &ds[ni];
            let dk = app.deck_of(n);
            let m = app.px.mix[ni];
            let pad_mode = app.px.pad_mode[ni];
            let led = match f.act {
                // (The engine's own lights, `Hub::transport_leds`, send these.)
                Act::PlayPause | Act::Cue => continue,
                Act::Search(dir) => on(d.searching == dir),
                Act::Slip => on(d.slip),
                Act::Quantize => on(if n == app.px.active { app.prefs.quantize } else { app.px.parked_quantize[ni] }),
                Act::QuantizeAll => on(app.prefs.quantize),
                Act::MasterTempo => on(dk.x.master_tempo),
                Act::Sync => on(dk.x.beat_sync),
                Act::Master => on(app.px.master == n && !app.sampler_is_master()),
                Act::KeySync => on(dk.x.key_sync),
                Act::TempoLed(dir) => {
                    let (down, up) = hub.takeover_state(ni).leds();
                    on(if dir < 0 { down } else { up })
                }
                Act::AutoLoop | Act::Loop4 | Act::Reloop | Act::LoopIn | Act::LoopOut => on(d.looping),
                Act::PitchBend(dir) => on(dk.x.bend == dir),
                Act::PadMode(mode) => on(pad_mode == mode),
                Act::HotCue(slot) | Act::HotCueDelete(slot) => match dk.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32) {
                    Some(c) => Led::Value(hot_cue_led(c.color, slot as i32, c.end.is_some(), app.prefs.pads.hot_cue_color)),
                    None => Led::Off,
                },
                Act::BeatJumpPad(_) | Act::BeatLoopPad(_) => on(d.loaded),
                Act::StemPart(src) => on(app.player.decks[ni].try_lock().map(|s| s.stem_on[(src as usize).min(3)]).unwrap_or(true)),
                Act::StemFx(row) => on(app.px.stem_fx[(row as usize).min(3)]),
                Act::StemSolo => on(app.prefs.stem_solo),
                Act::FxOn(u, s) => {
                    let p = &app.px.fx[u as usize % 2].params;
                    on(if p.single { s == 0 && p.one.on } else { p.slots[s as usize % 3].on })
                }
                Act::FxMode(u) => on(app.px.fx[u as usize % 2].params.single),
                // (A knob's ring: the level as a value, `operateValueChange`.)
                Act::FxLevel(u, s) => {
                    let p = &app.px.fx[u as usize % 2].params;
                    let lv = if p.single { if s == 0 { p.one.level } else { 0.0 } } else { p.slots[s as usize % 3].level };
                    Led::Value((lv.clamp(0.0, 1.0) * 127.0).round() as u8)
                }
                Act::FxAuto(u) => on(app.px.fx[u as usize % 2].auto),
                Act::FxTap(u) => on(!app.px.fx[u as usize % 2].auto),
                Act::FxReleaseOn(u) => on(app.px.fx[u as usize % 2].params.release.is_some()),
                Act::FxAssign(u, ch) => on(app.px.fx[u as usize % 2].params.assign[(ch as usize).min(crate::fx::CHANNELS - 1)]),
                Act::SamplerPad(pad, _) | Act::SamplerPadStop(pad) => {
                    let p = pad as usize + if n % 2 == 1 && app.sampler_pads() > 8 { 8 } else { 0 };
                    on(app.sampler_loaded(app.prefs.sampler.bank.min(3) as usize * 16 + p))
                }
                Act::Smp(crate::sampler_ext::SmpAct::Sync) => on(app.prefs.sampler.sync),
                Act::Smp(crate::sampler_ext::SmpAct::Master) => on(app.px.sampler.master),
                Act::Smp(crate::sampler_ext::SmpAct::Quantize) => on(app.prefs.sampler.quantize),
                Act::Smp(crate::sampler_ext::SmpAct::SeqPlay) => on(seq_playing),
                Act::Smp(crate::sampler_ext::SmpAct::SeqRec) => if seq_rec && seq_playing { Led::On } else if seq_rec { Led::Blink } else { Led::Off },
                Act::SamplerCue => on(app.prefs.sampler.cue),
                Act::SeqCall(slot, _) => on(seq_playing && seq_cur == slot as usize),
                Act::HeadCue => on(m.cue),
                Act::XfAssign(side) => on(m.assign == side),
                Act::StemIso => on(m.stem_iso_on),
                Act::Panel(p) => on(match p {
                    0 => app.prefs.perf_fx,
                    1 => app.prefs.perf_sampler,
                    _ => crate::ui_perf_layouts::mixer_shown(app),
                }),
                Act::Related => on(app.view == View::Related),
                Act::Stub(Stub::Vinyl) => on(hub.vinyl[ni].load(Relaxed)),
                // (Blinking while the deck plays backwards: `deviceUpdateLed` 0x10202a7f4.)
                Act::Stub(Stub::Reverse) => if d.reverse { Led::Blink } else { Led::Off },
                Act::More(mo) => match crate::midi_acts::led(app, hub, &dev.name, mo, n) {
                    Some(l) => l,
                    None => continue,
                },
                _ => continue,
            };
            dev.set_led(tag, deck, led);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midir::os::unix::{VirtualInput, VirtualOutput};

    /// Opt-in (reads this computer's rekordbox folder, read only): the mappings the user made
    /// in rekordbox for the named ports, and which of their rows recordbox can act on.
    /// PREPDECK_MIDI_RB_USER="PIONEER DDJ-S1|DDJ-S1 Jog Helper"
    #[test]
    #[ignore]
    fn rekordbox_user_mappings_are_usable() {
        let Ok(ports) = std::env::var("PREPDECK_MIDI_RB_USER") else { return };
        for port in ports.split('|') {
            let Some(f) = rekordbox_user_file(port) else {
                eprintln!("RBUSER {port}: no file");
                continue;
            };
            let bytes = std::fs::read(&f).unwrap();
            // (With the installed rekordbox's factory mapping under it, when it has one.)
            let factory = crate::midi_csv::factory_file(&crate::midi_csv::factory_dir(), port).and_then(|p| std::fs::read(p).ok());
            let map = match MidiMap::load(port, factory.as_deref(), Some(&bytes)) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("RBUSER {port}: not readable: {e:?}");
                    continue;
                }
            };
            let ins: Vec<&Input> = map.inputs.iter().flatten().collect();
            let mut names: Vec<(String, Kind)> = ins.iter().map(|i| (i.e.name.clone(), i.e.kind)).collect();
            names.sort_by(|a, b| a.0.cmp(&b.0));
            names.dedup();
            // Each row's function with its own type: does recordbox act on it?
            let missing: Vec<String> = names.iter().filter(|(n, k)| !input_acts(n, *k)).map(|(n, k)| format!("{n} ({})", k.text())).collect();
            let mut outs: Vec<String> = map.outputs.iter().map(|e| e.name.clone()).collect();
            outs.sort();
            outs.dedup();
            let out_missing: Vec<&String> = outs.iter().filter(|n| !output_source(n)).collect();
            eprintln!("RBUSER {port}: {} input codes, {} functions, {} rejected rows; functions recordbox has no action for: {missing:?}", ins.len(), names.len(), map.rejected);
            eprintln!("RBUSER {port}: {} output rows, {} functions; without an output in recordbox: {out_missing:?}", map.outputs.len(), outs.len());
        }
    }

    #[test]
    fn learn_binds_what_a_controller_sends_and_names_rekordboxs_function() {
        // A button pressed and let go (Note On, then velocity 0), and a 14-bit fader (MSB, LSB).
        let msgs = vec![vec![0x90, 0x0B, 0x7F], vec![0x90, 0x0B, 0x00], vec![0xB0, 0x13, 0x40], vec![0xB0, 0x33, 0x00]];
        let rb = MidiMap::load("TEST", None, Some(b"@file,1,TEST\r\nPlayPause,,Button,,900B,910B,,,,,,,,,Play\r\n")).ok();
        let got = probe_learn("TEST", &msgs, rb.as_ref());
        // The release and the LSB are passed over, as rekordbox's Learn does.
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), vec![0x900B, 0xB013]);
        assert!(got.iter().all(|g| g.1), "both bind");
        assert_eq!(got[0].2, "PlayPause (deck 1)");
        assert_eq!(got[1].2, "-");
    }

    #[test]
    fn hot_cue_pad_lights_take_the_cue_colour_or_the_colour_set() {
        // A cue with a colour: that code.
        assert_eq!(hot_cue_led(42, 0, false, 4), 42);
        // Without one: the slot's colour in the set (COLORFUL A = 43, C = 23; CDJ I = 26).
        assert_eq!(hot_cue_led(0, 0, false, 4), 43);
        assert_eq!(hot_cue_led(0, 2, false, 4), 23);
        assert_eq!(hot_cue_led(0, 8, false, 1), 26);
        // A loop without a colour: rekordbox's loop entry (index 16 of the tables, 36).
        assert_eq!(hot_cue_led(0, 3, true, 2), crate::model::HOTCUE_LOOP_DEFAULT);
        // Codes above 64 are not sent.
        assert_eq!(hot_cue_led(70, 0, false, 4), 0);
    }

    #[test]
    fn registry_names_are_unique_per_tab_and_typed() {
        let r = registry();
        assert!(r.funcs.len() > 300, "{}", r.funcs.len());
        let mut seen: std::collections::HashMap<String, (Tab, Act)> = std::collections::HashMap::new();
        for f in &r.funcs {
            // (A name is listed twice only as an ADD-menu entry of the same tab and action:
            // Sampler9 Play is PAD1_Sampler again.)
            if let Some((t, a)) = seen.insert(f.name.clone(), (f.tab, f.act)) {
                assert!(t == f.tab && a == f.act && f.tab != Tab::None, "{} twice", f.name);
            }
            if f.tab != Tab::None {
                assert!(!f.display.is_empty(), "{}", f.name);
                assert!((1..=8).contains(&f.ty));
                // Pad rows are pads; pads of the catalogue are named PAD<n>_ or are stem parts.
                if is_pad_name(&f.name) && !f.name.ends_with("+Velocity") {
                    assert_eq!(f.ty, T_PAD, "{}", f.name);
                }
            }
        }
        // rekordbox's names for a few of them.
        assert_eq!(r.by_name("TrackNext").unwrap().display, "TrackSearch Next");
        assert_eq!(r.by_name("PAD1_HotCue+Shift").unwrap().display, "HotCue1 Delete");
        assert_eq!(r.by_name("BeatLoop1_64").unwrap().display, "AutoBeatLoop 1/64");
        assert_eq!(r.by_name("FX2-3SelectBack").unwrap().display, "FX2-3 Select Back");
        assert_eq!(r.by_name("CrossfaderAssign.Thru").unwrap().act, Act::XfAssign(0));
        assert!(r.by_name("TempoSlider").unwrap().per_deck && !r.by_name("CrossFader").unwrap().per_deck);
        // Jog wheels and Shift are not offered by MIDI Learn.
        assert!(!r.learnable("JogScratch") && !r.learnable("Shift") && !r.learnable("Nonsense"));
        assert!(r.learnable("PlayPause") && r.learnable("MixPointLink"));
        // The tabs that have commands.
        for (tab, name) in TABS {
            let n = r.of_tab(tab).count();
            assert_eq!(n == 0, matches!(tab, Tab::Video | Tab::Lighting), "{name}: {n}");
        }
        // Types and their families.
        assert_eq!(type_choices(T_BUTTON), &[T_BUTTON, T_BUTTON_PAD]);
        assert_eq!(type_choices(T_KNOB14), &[T_KNOB7, T_KNOB14]);
        assert_eq!(type_choices(T_PAD), &[T_PAD]);
        assert_eq!(type_text(T_KNOB14), "Knob/Slider(0h-3FFFh)");
        assert_eq!(kind_of_type(T_KNOB14), Kind::KnobSliderHiRes);
    }

    /// `cargo test --release midi_catalogue -- --nocapture`: every tab of the MIDI setting
    /// window against rekordbox's ADD menus (the traced catalogue, research material outside
    /// `app/`, read when it is on this computer): lines `CATALOGUE <tab> missing|extra|differs`
    /// and the totals line `CATALOGUE TOTAL`.
    #[test]
    fn midi_catalogue_matches_rekordboxs_add_menus() {
        let path = std::path::Path::new("/Users/adamsardo/Developer/rekordbox-3/checks/re-7.2.19/controllers-midi-learn-catalogue.tsv");
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("CATALOGUE: no traced catalogue on this computer; skipped");
            return;
        };
        let tab_of = |s: &str| TABS.iter().find(|t| t.1.eq_ignore_ascii_case(s)).map(|t| t.0);
        let r = registry();
        let (mut missing, mut extra, mut differs, mut same) = (0, 0, 0, 0);
        let mut want: Vec<(Tab, String, String, String, u8, bool)> = vec![];
        for l in text.lines().skip(2) {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 6 {
                continue;
            }
            let Some(tab) = tab_of(c[0]) else { continue };
            if matches!(tab, Tab::Video | Tab::Lighting) {
                continue;
            }
            want.push((tab, c[1].to_string(), c[2].to_string(), c[4].to_string(), c[3].parse().unwrap_or(0), c[5] == "1"));
        }
        for (tab, disp, csv, cat, ty, pd) in &want {
            match r.of_tab(*tab).find(|f| f.name == *csv && f.display == *disp).or_else(|| r.of_tab(*tab).find(|f| f.name == *csv)) {
                None => {
                    missing += 1;
                    eprintln!("CATALOGUE {:?} missing {csv} ({disp}, {cat}, type {ty})", tab);
                }
                Some(f) if f.display != *disp || f.category != cat || f.ty != *ty || f.per_deck != *pd => {
                    differs += 1;
                    eprintln!("CATALOGUE {:?} differs {csv}: has ({}, {}, {}, {}) wants ({disp}, {cat}, {ty}, {pd})", tab, f.display, f.category, f.ty, f.per_deck);
                }
                Some(_) => same += 1,
            }
        }
        for f in r.funcs.iter().filter(|f| f.tab != Tab::None) {
            if !want.iter().any(|w| w.0 == f.tab && w.2 == f.name) {
                extra += 1;
                eprintln!("CATALOGUE {:?} extra {} ({})", f.tab, f.name, f.display);
            }
        }
        // ADD-menu order: the registry's order of each tab is the catalogue's.
        let mut order_ok = true;
        for (tab, _) in TABS {
            let a: Vec<&str> = r.of_tab(tab).map(|f| f.name.as_str()).collect();
            let b: Vec<&str> = want.iter().filter(|w| w.0 == tab).map(|w| w.2.as_str()).filter(|n| a.contains(n)).collect();
            let a2: Vec<&str> = a.iter().copied().filter(|n| b.contains(n)).collect();
            if a2 != b {
                order_ok = false;
                eprintln!("CATALOGUE {tab:?} order differs");
            }
        }
        eprintln!("CATALOGUE TOTAL commands {} | same {same} | differs {differs} | missing {missing} | extra {extra} | order {}", want.len(), if order_ok { "same" } else { "differs" });
    }

    #[test]
    fn devices_that_need_the_hardware_handshake_are_known() {
        for d in ["DDJ-400", "DDJ-1000", "DDJ-FLX6", "DJM-A9", "XDJ-XZ (1)", "Pioneer DJ XDJ-RX2", "PIONEER XDJ-RX"] {
            assert!(needs_auth(d), "{d}");
        }
        for d in ["DDJ-FLX4", "DDJ-FLX2", "DDJ-GRV6", "DDJ-REV5", "DDJ-FLX10", "XDJ-RX3", "PIONEER DDJ-SX2", "Launchpad Mini"] {
            assert!(!needs_auth(d), "{d}");
        }
    }

    // ------------------------------------------------------------------ the user's DDJ-S1
    // The rows below are the codes, types and options of the two mappings the user made in
    // rekordbox for the DDJ-S1 and its jog helper (comments left out), and of the factory
    // DDJ-ERGO port the helper sends the platter on.

    const S1: &str = "@file,1,PIONEER DDJ-S1\r\n\
        ActiveLoop,,Button,,9050,9150,,,,,,,,,\r\n\
        CallNext,,Button,,9015,9115,,,,,,,,,\r\n\
        CallPrev,,Button,,9051,9151,,,,,,,,,\r\n\
        FX1TapBeatWithLongPress,,Button,9241,,,,,,,,,,,\r\n\
        FX2TapBeatWithLongPress,,Button,9341,,,,,,,,,,,\r\n\
        LoopMove,,Rotary,,B04F,B14F,,,,,,,,,\r\n\
        PanelForward,,Button,9446,,,,,,,,,,,\r\n\
        SwitchActiveWindow,,Button,9447,,,,,,,,,,,\r\n\
        VinylAdjustBrake,,KnobSlider,,B009,B109,,,,,,,,,\r\n\
        VinylAdjustTouchRelease,,KnobSlider,,B004,B104,,,,,,,,,\r\n\
        PAD1_HotCue,,DdjSxPad,,902A,912A,,,,902A,912A,,,Fast;,\r\n\
        PlayPause,,Button,,900B,910B,,,,900B,910B,,,Fast;,\r\n\
        PlayPause,,Button,,9047,9147,,,,9047,9147,,,Fast;,\r\n\
        TempoSlider,,KnobSliderHiRes,,B000,B100,,,,,,,,Fast;,\r\n\
        TempoSlider,,KnobSliderHiRes,,B005,B105,,,,,,,,Fast;,\r\n\
        NeedleSearch,,Value,,B003,B103,,,,,,,,Max=16383;,\r\n\
        Reverse,,Button,,906A,916A,,,,906A,916A,,,,\r\n\
        Load,,Button,,903D,913D,,,,903D,913D,,,,\r\n\
        Area,,Button,9443,,,,,9443,,,,,,\r\n\
        ChannelLevel,,Indicator,,,,,,,B469,B46B,,,,\r\n\
        JogPlayPauseIndicatorShort,,JogIndicator,,,,,,,B039,B139,,,Min=1;Max=48;Priority=100;,\r\n";

    const HELPER: &str = "@file,1,DDJ-S1 Jog Helper\r\n\
        FX1Assign.CH1,,Button,9210,,,,,,,,,,,\r\n\
        FX1Assign.CH2,,Button,9211,,,,,,,,,,,\r\n\
        FX1Assign.MASTER,,Button,9212,,,,,,,,,,,\r\n\
        FX2Assign.CH1,,Button,9310,,,,,,,,,,,\r\n\
        JogScratch,,JogRotate,,B022,B122,,,,,,,,RO;,\r\n\
        JogTouch,,JogTouch,,9036,9136,,,,,,,,RO;,\r\n\
        JogTouch,,JogTouch,,9068,9168,,,,,,,,RO;,\r\n\
        PitchBendDown,,Button,,9072,9172,,,,,,,,,\r\n\
        PitchBendUp,,Button,,9071,9171,,,,,,,,,\r\n\
        SearchFwd,,Button,,9073,9173,,,,,,,,,\r\n\
        SearchRev,,Button,,9074,9174,,,,,,,,,\r\n\
        WheelPitchBend,,JogRotate,,B021,B121,,,,,,,,RO;,\r\n\
        WheelSearch,,JogRotate,,B026,B126,,,,,,,,RO;,\r\n";

    /// A device with a mapping and no ports, on a hub of four decks (1 and 2 loaded).
    fn test_device(name: &str, csv: &str) -> (Arc<Hub>, Arc<Device>, crossbeam_channel::Receiver<UiEvent>) {
        let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        for d in &decks[..2] {
            d.lock().audio = Some(tone(120.0));
        }
        let (hub, rx) = Hub::new(decks, std::env::temp_dir());
        {
            let mut s = hub.snap.lock();
            s.decks[0].loaded = true;
            s.decks[1].loaded = true;
        }
        let map = MidiMap::load(name, None, Some(csv.as_bytes())).expect("the mapping reads");
        let dev = Arc::new(Device {
            name: name.into(),
            map: Mutex::new(map),
            factory: false,
            needs_auth: false,
            has_output: false,
            learning: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            out_index: Mutex::new(HashMap::new()),
            leds: Mutex::new(HashMap::new()),
            named: Mutex::new(HashMap::new()),
            out_names: Mutex::new(Default::default()),
            out: Mutex::new(OutState { queue: OutQueue::default(), blink: Blinker::default(), blinking: HashMap::new(), conn: None }),
            out_cv: Condvar::new(),
            interval_ms: AtomicU64::new(1),
            jog: Mutex::new(std::array::from_fn(|_| Jog::new(jog_counts(name)))),
            scan: Mutex::new(Default::default()),
            load_at: Mutex::new([None; 4]),
            in_conn: Mutex::new(None),
            received: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            port_id: String::new(),
        });
        dev.reindex();
        hub.devices.lock().push(dev.clone());
        (hub, dev, rx)
    }

    /// A message through the map and the port's worker side: the function it named, its
    /// deck and value.
    fn feed(hub: &Hub, dev: &Device, msg: &[u8], t_ms: f64) -> Option<(String, u8, Val)> {
        let ev = dev.map.lock().handle(msg, t_ms)?;
        let name = registry().get(ev.tag).map(|f| f.name.clone()).unwrap_or_else(|| format!("<no action: {}>", dev.map.lock().inputs[ev.entry].as_ref().unwrap().e.name));
        let (deck, val) = (ev.deck, ev.val);
        hub.dispatch(dev, ev);
        Some((name, deck, val))
    }

    #[test]
    fn every_function_of_the_users_ddj_s1_mappings_has_an_action() {
        let (hub, dev, _rx) = test_device("PIONEER DDJ-S1", S1);
        // Each code names its function, for its deck, with its row type's value.
        let cases: &[(&[u8], &str, u8)] = &[
            (&[0x90, 0x50, 0x7F], "ActiveLoop", 1),
            (&[0x91, 0x15, 0x7F], "CallNext", 2),
            (&[0x90, 0x51, 0x7F], "CallPrev", 1),
            (&[0x92, 0x41, 0x7F], "FX1TapBeatWithLongPress", 0),
            (&[0x93, 0x41, 0x7F], "FX2TapBeatWithLongPress", 0),
            (&[0xB1, 0x4F, 0x01], "LoopMove", 2),
            (&[0x94, 0x46, 0x7F], "PanelForward", 0),
            (&[0x94, 0x47, 0x7F], "SwitchActiveWindow", 0),
            (&[0xB0, 0x09, 0x40], "VinylAdjustBrake", 1),
            (&[0xB1, 0x04, 0x7F], "VinylAdjustTouchRelease", 2),
            (&[0x91, 0x2A, 0x7F], "PAD1_HotCue", 2),
            (&[0x90, 0x47, 0x7F], "PlayPause", 1),
            (&[0x90, 0x0B, 0x00], "PlayPause", 1),
        ];
        for (msg, name, deck) in cases {
            let (n, d, v) = feed(&hub, &dev, msg, 0.0).unwrap_or_else(|| panic!("{msg:02X?} found nothing"));
            assert_eq!((n.as_str(), d), (*name, *deck), "{msg:02X?}");
            let e = dev.map.lock().input_at(((msg[0] as u16) << 8) | msg[1] as u16).cloned().unwrap();
            assert!(input_acts(name, e.kind), "{name} ({:?}) acts", e.kind);
            match e.kind {
                Kind::Rotary => assert_eq!(v, Val::Rel(1)),
                Kind::KnobSlider => assert!(matches!(v, Val::Abs { .. })),
                _ => assert!(matches!(v, Val::On(..))),
            }
        }
        // The pad row's own type: the SX pad family, whatever the device.
        assert!(matches!(dev.map.lock().input_at(0x902A).unwrap().kind, Kind::Pad(crate::midi_csv::PadType::Sx)));
        // The Shift tempo fader is the same tempo slider, 14-bit.
        assert_eq!(feed(&hub, &dev, &[0xB0, 0x05, 0x40], 0.0), None);
        assert_eq!(feed(&hub, &dev, &[0xB0, 0x25, 0x00], 0.0).map(|x| x.0), Some("TempoSlider".into()));
        // The needle strip: 0..16383.
        assert_eq!(feed(&hub, &dev, &[0xB0, 0x03, 0x7F], 0.0).map(|x| x.2.unit().unwrap()), Some(1.0));
        // Every output row has a source: lights that follow a state, a held button, the meter
        // and the jog ring.
        for o in dev.map.lock().outputs.iter() {
            assert!(output_source(&o.name), "{} has no source", o.name);
        }
    }

    #[test]
    fn every_function_of_the_users_jog_helper_mapping_has_an_action() {
        let (hub, dev, _rx) = test_device("DDJ-S1 Jog Helper", HELPER);
        for (msg, name, deck) in [([0x92u8, 0x10, 0x7F], "FX1Assign.CH1", 0u8), ([0x92, 0x12, 0x7F], "FX1Assign.MASTER", 0), ([0x93, 0x10, 0x7F], "FX2Assign.CH1", 0), ([0x91, 0x71, 0x7F], "PitchBendUp", 2), ([0x90, 0x72, 0x7F], "PitchBendDown", 1), ([0x90, 0x73, 0x7F], "SearchFwd", 1), ([0x91, 0x74, 0x7F], "SearchRev", 2)] {
            let (n, d, _) = feed(&hub, &dev, &msg, 0.0).unwrap();
            assert_eq!((n.as_str(), d), (name, deck));
        }
        // Both touch codes are the platter top of their deck; the jog rows give counts.
        assert_eq!(feed(&hub, &dev, &[0x90, 0x68, 0x7F], 0.0).map(|x| (x.0, x.1)), Some(("JogTouch".into(), 1)));
        assert_eq!(feed(&hub, &dev, &[0x90, 0x68, 0x00], 1.0).map(|x| x.2), Some(Val::On(false, 0)));
        for (cc, name) in [(0x22u8, "JogScratch"), (0x21, "WheelPitchBend"), (0x26, "WheelSearch")] {
            let (n, _, v) = feed(&hub, &dev, &[0xB1, cc, 0x41], 2.0).unwrap();
            assert_eq!(n, name);
            assert_eq!(v, Val::Jog { delta: 1, t_ms: 2.0 });
        }
    }

    #[test]
    fn the_platter_scratches_the_distance_it_turns_and_the_rim_bends() {
        // The helper sends the platter on the "PIONEER DDJ-ERGO" port: 2300 counts a turn.
        let (hub, dev, _rx) = test_device("PIONEER DDJ-ERGO", "@file,1,PIONEER DDJ-ERGO\r\nJogScratch,,JogRotate,,B022,B122,,,,,,,,RO;,\r\nJogPitchBend,,JogRotate,,B023,B123,,,,,,,,RO;,\r\nJogTouch,,JogTouch,,9036,9136,,,,,,,,RO;,\r\nJogSearch,,Difference,,B027,B127,,,,,,,,RO;,\r\n");
        // Touched (vinyl on): the record is held.
        feed(&hub, &dev, &[0x90, 0x36, 0x7F], 0.0);
        assert_eq!(hub.decks[0].lock().dk.jog, crate::deck::Jog::Speed(0.0));
        // 23 counts every 10 ms = 2300 counts a second, one turn: at 33 1/3 rpm that is
        // 1.8 turns' worth of playing speed, i.e. speed 1.8.
        let mut speed = 0.0;
        for i in 1..=10 {
            feed(&hub, &dev, &[0xB0, 0x22, 0x40 + 23], i as f64 * 10.0);
            if let crate::deck::Jog::Speed(s) = hub.decks[0].lock().dk.jog {
                speed = s;
            }
        }
        assert!((speed - 1.8).abs() < 1e-9, "{speed}");
        // Distance: one message's speed stands for its own 10 ms, then the record stops.
        let mut j = dev.jog.lock()[0].clone();
        assert_eq!(j.idle(105.0), JogOut::Nothing);
        assert_eq!(j.idle(111.0), JogOut::Scratch(0.0));
        // Above the cap of 8 the rest is carried, not lost: 25 counts in 1 ms is speed 19.6.
        let mut j = Jog::new(2300.0);
        j.touch(true, true);
        j.rotate(1, 0.0, true);
        let first = j.rotate(25, 1.0, true);
        assert_eq!(first, JogOut::Scratch(8.0));
        // The rest is played at 8 for as long as it stands for.
        let JogOut::Scratch(s) = j.idle(2.0) else { panic!() };
        assert_eq!(s, 8.0);
        // Let go: back to the record's own play.
        feed(&hub, &dev, &[0x90, 0x36, 0x00], 200.0);
        // The untouched platter bends: positive counts, a positive bend while it turns.
        for i in 0..6 {
            feed(&hub, &dev, &[0xB0, 0x23, 0x50], 300.0 + i as f64 * 8.0);
        }
        assert!(hub.decks[0].lock().bend > 0.0);
        // Vinyl off and playing: touching bends instead of scratching.
        hub.vinyl[0].store(false, Relaxed);
        hub.decks[0].lock().playing = true;
        hub.decks[0].lock().dk.jog = crate::deck::Jog::None;
        feed(&hub, &dev, &[0x90, 0x36, 0x7F], 500.0);
        assert_eq!(hub.decks[0].lock().dk.jog, crate::deck::Jog::None);
        assert_eq!(dev.jog.lock()[0].state, 4);
        // JogSearch: a jump by the scan step at most every 200 ms (a slow turn: 666 ms).
        hub.decks[1].lock().pos = 44100.0 * 10.0;
        feed(&hub, &dev, &[0xB1, 0x27, 0x41], 1000.0);
        assert!((hub.decks[1].lock().pos / 44100.0 - 10.666).abs() < 1e-6);
        feed(&hub, &dev, &[0xB1, 0x27, 0x41], 1100.0);
        assert!((hub.decks[1].lock().pos / 44100.0 - 10.666).abs() < 1e-6, "not again within 200 ms");
        feed(&hub, &dev, &[0xB1, 0x27, 0x3F], 1300.0);
        assert!((hub.decks[1].lock().pos / 44100.0 - 10.0).abs() < 1e-6);
    }

    #[test]
    fn a_mapping_from_rekordboxs_folder_is_shown_and_changes_are_saved_to_recordboxs() {
        let base = std::env::temp_dir().join(format!("recordbox-midi-rbdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (rb, own) = (base.join("rekordbox"), base.join("recordbox"));
        std::fs::create_dir_all(&rb).unwrap();
        std::fs::create_dir_all(&own).unwrap();
        let name = "PIONEER DDJ-S1";
        let rb_file = rb.join(format!("{name}.midi.csv"));
        let with_comments = S1.replace("CallNext,,Button,,9015,9115,,,,,,,,,", "CallNext,,Button,,9015,9115,,,,,,,,,Loop Select: next").replace("ActiveLoop,,Button,,9050,9150,,,,,,,,,", "ActiveLoop,,Button,,9050,9150,,,,,,,,,Shift + press Auto Loop");
        std::fs::write(&rb_file, &with_comments).unwrap();
        // recordbox's own file as it is written at first sight of the port: the header only.
        std::fs::write(own.join(format!("{name}.midi.csv")), crate::midi_csv::empty_user_file(name)).unwrap();
        let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let (hub, _rx) = Hub::new(decks, own.clone());
        *hub.rb_dir.lock() = Some(rb.clone());
        assert!(hub.from_rekordbox(name));
        let (map, _) = hub.load_map(name);
        // The window's tabs: the learnable rows, with their codes and comments.
        let rows = crate::ui_midi::rows_of(&map);
        let deck: Vec<(String, u8, Option<u16>, Option<u16>, String)> = rows[0].iter().map(|r| (r.name.clone(), r.deck, r.input, r.output, r.comment.clone())).collect();
        assert!(deck.contains(&("ActiveLoop".into(), 1, Some(0x9050), None, "Shift + press Auto Loop".into())), "{deck:?}");
        assert!(deck.contains(&("PlayPause".into(), 2, Some(0x9147), Some(0x9147), String::new())));
        assert!(deck.iter().any(|r| r.0 == "LoopMove" && r.2 == Some(0xB04F)));
        // Not learnable in rekordbox either: CALL, the Vinyl knobs, the jog ring, the meter.
        for n in ["CallNext", "VinylAdjustBrake", "JogPlayPauseIndicatorShort", "ChannelLevel", "FX1TapBeatWithLongPress"] {
            assert!(rows.iter().flatten().all(|r| r.name != n), "{n} shown");
        }
        let other: Vec<&str> = rows[6].iter().map(|r| r.name.as_str()).collect();
        assert_eq!(other, vec!["PanelForward", "SwitchActiveWindow"]);
        assert_eq!(rows[1].iter().filter(|r| r.name == "PAD1_HotCue").map(|r| type_text(r.ty)).collect::<Vec<_>>(), vec!["Pad", "Pad"]);
        // Closing the window without a change writes nothing.
        let (_, dev, _r) = test_device(name, &with_comments);
        *dev.map.lock() = hub.load_map(name).0;
        hub.save_map(&dev).unwrap();
        assert_eq!(std::fs::read_to_string(own.join(format!("{name}.midi.csv"))).unwrap(), crate::midi_csv::empty_user_file(name));
        // A change goes to recordbox's folder, with the rest of the mapping; rekordbox's file
        // is untouched.
        let reg = registry();
        {
            let mut m = dev.map.lock();
            let row = LearnEntry { name: "Sync".into(), deck: 1, kind: Kind::Button, min: -1, max: -1, comment: String::new() };
            m.change_input(&row, None, Some(0x9058), &|n: &str| reg.learnable(n)).unwrap();
        }
        hub.save_map(&dev).unwrap();
        let saved = std::fs::read_to_string(own.join(format!("{name}.midi.csv"))).unwrap();
        assert!(saved.starts_with("@file,1,PIONEER DDJ-S1\r\n") && saved.contains("Sync,Sync,Button,,9058,") && saved.contains("CallNext,,Button,,9015,9115,") && saved.contains("Loop Select: next"), "{saved}");
        assert_eq!(std::fs::read_to_string(&rb_file).unwrap(), with_comments);
        // From now on recordbox's own file is the one read.
        assert!(!hub.from_rekordbox(name));
        assert!(hub.load_map(name).0.input_at(0x9058).is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fourteen_bit_outputs_send_the_upper_and_lower_seven_bits() {
        let (_hub, dev, _rx) = test_device("DDJ-RZX", "@file,1,DDJ-RZX\r\nJogPlayPauseIndicator,,JogIndicatorHiRes,,,,,,,BB00,BB01,,,RO;Min=1;Max=135;Priority=100;,\r\n");
        // Half a turn: 1 + 135 x 180 / 360 = 68.
        dev.set_named("JogPlayPauseIndicator", 1, &|e: &Entry| crate::midi_map::jog_ring(180, &e.opts) as u16);
        let mut out = dev.out.lock();
        assert_eq!(out.queue.pop(), Some([0xBB, 0x00, 0]));
        assert_eq!(out.queue.pop(), Some([0xBB, 0x10, 68]));
        drop(out);
        dev.set_named("JogPlayPauseIndicator", 1, &|_| 300);
        let mut out = dev.out.lock();
        assert_eq!((out.queue.pop(), out.queue.pop()), (Some([0xBB, 0x00, 2]), Some([0xBB, 0x10, 44])));
    }

    #[test]
    fn the_new_functions_follow_rekordboxs_rules() {
        // Vinyl Speed Adjust: knob position x 0.0155649149.
        assert_eq!((vinyl_index(0), vinyl_index(8192), vinyl_index(16383)), (0, 127, 255));
        // FX assign following a switch: master alone.
        let mut a = [true, false, false, false, false, false];
        fx_assign_set(&mut a, crate::fx::MASTER, true);
        assert_eq!(a, [false, false, false, false, true, false]);
        fx_assign_set(&mut a, 1, true);
        assert_eq!(a, [false, true, false, false, false, false]);
        fx_assign_set(&mut a, 1, false);
        assert_eq!(a, [false; 6]);
        // PanelSelect: FX, SAMPLER, MIXER, REC, none, in turn; back the other way.
        let all = [true; 4];
        assert_eq!(panel_next([false; 4], all, 1), Some(0));
        assert_eq!(panel_next([true, false, false, false], all, 1), Some(1));
        assert_eq!(panel_next([true, true, true, false], all, 1), Some(3));
        assert_eq!(panel_next([false, false, false, true], all, 1), None);
        assert_eq!(panel_next([false; 4], all, -1), Some(3));
        assert_eq!(panel_next([true, false, false, false], all, -1), None);
        // A panel the layout does not have is passed over.
        assert_eq!(panel_next([true, false, false, false], [true, false, true, true], 1), Some(2));
        // Lights: a button with no state of its own is lit while held; REVERSE blinks.
        assert!(echo_led(Act::Load) && echo_led(Act::CallStep(1)) && echo_led(Act::Stub(Stub::SlipReverse)));
        assert!(!echo_led(Act::Sync) && act_has_led(Act::Stub(Stub::Reverse)));
    }


    // ------------------------------------------------------------------ the third pass

    /// `cargo test --release third_pass -- --nocapture`: every input row of rekordbox's
    /// factory mappings whose function this pass built (`Act::More`, the Shift pad layers, the
    /// Pad Editor slots, the factory-only names) is fed its own code: the map must hand it to
    /// that function (or to the function rekordbox's first row with that code has). Line
    /// `THIRD PASS ROWS`.
    #[test]
    fn third_pass_factory_rows_reach_their_functions() {
        let dir = crate::midi_csv::factory_dir();
        let Ok(rd) = std::fs::read_dir(&dir) else {
            eprintln!("THIRD PASS ROWS: no rekordbox here; skipped");
            return;
        };
        let reg = registry();
        let (mut checked, mut wrong, mut files) = (0, 0, 0);
        let mut by_name: std::collections::BTreeMap<String, usize> = Default::default();
        for f in rd.flatten() {
            let p = f.path();
            let Some(fname) = p.file_name().map(|s| s.to_string_lossy().to_string()) else { continue };
            let Some(dev) = fname.strip_suffix(".midi.csv") else { continue };
            let Ok(bytes) = std::fs::read(&p) else { continue };
            let Ok(mut map) = MidiMap::load(dev, Some(&bytes), None) else { continue };
            files += 1;
            map.tag_inputs(|n| reg.tag(n));
            let rows: Vec<(String, u16, Kind, u32)> = map.inputs.iter().flatten().map(|i| (i.e.name.clone(), i.e.code, i.e.kind, i.tag)).collect();
            for (name, code, kind, tag) in rows {
                let Some(func) = reg.get(tag) else { continue };
                let new = matches!(func.act, Act::More(m) if !matches!(m, crate::midi_acts::M::Off(_))) || name.contains("+Shift") || name.contains("PadMode") || name.starts_with("Page");
                if !new || code == 0 || matches!(kind, Kind::KnobSliderHiRes | Kind::Slider | Kind::Knob | Kind::JogTimestamp) {
                    continue;
                }
                let st = (code >> 8) as u8;
                if st < 0x80 || st >= 0xF0 {
                    continue;
                }
                let v = match in_class(kind) {
                    Some(InClass::Rel) | Some(InClass::Jog) => 0x41,
                    Some(InClass::Abs) => 0x40,
                    _ => 0x7F,
                };
                let Some(ev) = map.handle(&[st, code as u8, v], 1000.0) else { continue };
                // (Release a held button so the next row starts clean.)
                if in_class(kind) == Some(InClass::Press) {
                    let _ = map.handle(&[st, code as u8, 0], 1001.0);
                }
                checked += 1;
                *by_name.entry(name.clone()).or_default() += 1;
                let got = reg.get(ev.tag).map(|g| (g.name.clone(), g.act));
                if got.as_ref().map(|g| g.0 != name && g.1 != func.act).unwrap_or(true) {
                    wrong += 1;
                    eprintln!("THIRD PASS ROW {fname}: {name} {code:04X} went to {got:?}");
                }
                assert!(input_acts(&name, kind), "{name} {kind:?} does not act");
            }
        }
        eprintln!("THIRD PASS ROWS files {files} | rows fed {checked} | names {} | went elsewhere {wrong}", by_name.len());
        assert!(files == 0 || (checked > 500 && wrong == 0));
    }

    fn output_device(name: &str, csv: &str) -> (Arc<Hub>, Arc<Device>) {
        let (hub, dev, _rx) = test_device(name, csv);
        // (A device with an output port: the same, but its lights are queued.)
        let d2 = Arc::new(Device {
            name: dev.name.clone(),
            map: Mutex::new(MidiMap::load(name, None, Some(csv.as_bytes())).unwrap()),
            factory: false,
            needs_auth: false,
            has_output: true,
            learning: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            out_index: Mutex::new(HashMap::new()),
            leds: Mutex::new(HashMap::new()),
            named: Mutex::new(HashMap::new()),
            out_names: Mutex::new(Default::default()),
            out: Mutex::new(OutState { queue: OutQueue::default(), blink: Blinker::default(), blinking: HashMap::new(), conn: None }),
            out_cv: Condvar::new(),
            interval_ms: AtomicU64::new(1),
            jog: Mutex::new(std::array::from_fn(|_| Jog::new(jog_counts(name)))),
            scan: Mutex::new(Default::default()),
            load_at: Mutex::new([None; 4]),
            in_conn: Mutex::new(None),
            received: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            port_id: String::new(),
        });
        d2.reindex();
        hub.devices.lock().clear();
        hub.devices.lock().push(d2.clone());
        (hub, d2)
    }

    fn sent(dev: &Device) -> Vec<[u8; 3]> {
        let mut v = vec![];
        let mut o = dev.out.lock();
        while let Some(m) = o.queue.pop() {
            v.push(m);
        }
        v
    }

    /// The lights that follow the engine: end warning, on air, a meter's segments, the
    /// DDJ-SR's loop size lights, the cue point on the jog ring, REVERSE (rows as the factory
    /// files have them).
    #[test]
    fn third_pass_engine_lights_follow_the_deck() {
        let csv = "@file,1,t\r\nEndWarningIndicator,EndWarningIndicator,Indicator,,,,,,,9B04,9B05,9B06,9B07,RO;Priority=100,\r\nOnAirIndicator,OnAirIndicator,Indicator,,,,,,,BB04,BB05,BB06,BB07,RO;Priority=100,\r\nChannelLevel.1,ChannelLevel,Indicator,,,,,,,9C28,9C2D,9C32,9C37,RO;Priority=50,\r\nChannelLevel.5,ChannelLevel,Indicator,,,,,,,9C2C,9C31,9C36,9C3B,RO;Priority=50,\r\nBeatIndicatorNumerator,BeatIndicatorNumerator,Indicator,,,,,,9056,0,1,2,3,,\r\nBeatIndicator4,BeatIndicator4,Indicator,,,,,,905B,0,1,2,3,,\r\nBeatIndicator2,BeatIndicator2,Indicator,,,,,,905A,0,1,2,3,,\r\nJogCurrentCueIndicator,JogCurrentCueIndicator,JogIndicator,,,,,,,BB24,BB25,BB26,BB27,RO;Min=1;Max=72;Priority=100,\r\n";
        let (hub, dev) = output_device("t", csv);
        {
            let mut s = hub.snap.lock();
            s.decks[0].loop_beats = 4.0;
            s.decks[0].cue_point = 0.9;
        }
        {
            let mut s = hub.decks[0].lock();
            let frames = s.audio.as_ref().unwrap().frames() as f64;
            s.pos = frames - 44100.0 * 10.0;
            s.playing = true;
            s.ch_fader = 1.0;
            s.fader = 1.0;
            s.peak = [0.95, 0.9];
        }
        hub.engine_outputs();
        let m = sent(&dev);
        let has = |a: u8, b: u8, v: u8| m.contains(&[a, b, v]);
        assert!(has(0x9B, 0x04, 127), "end warning in the last 30 s: {m:02X?}");
        assert!(has(0xBB, 0x04, 127), "on air");
        assert!(has(0x9C, 0x28, 127) && has(0x9C, 0x2C, 127), "meter segments 1 and 5 at 95 %");
        assert!(has(0x90, 0x5B, 127) && has(0x90, 0x56, 0) && has(0x90, 0x5A, 0), "4 beats: 4Beat lit");
        // The cue point 0.9 s in: (0.9 x 33 1/3 / 60 x 360) = 180 degrees: half way round.
        assert!(has(0xBB, 0x24, 1 + 36), "cue on the ring: {m:02X?}");
        // A loop of half a beat: 1/Beat and 2Beat lit.
        hub.snap.lock().decks[0].loop_beats = 0.5;
        hub.engine_outputs();
        let m = sent(&dev);
        assert!(m.contains(&[0x90, 0x56, 127]) && m.contains(&[0x90, 0x5A, 127]) && m.contains(&[0x90, 0x5B, 0]), "{m:02X?}");
        // Stopped: no end warning, off air.
        hub.decks[0].lock().playing = false;
        hub.engine_outputs();
        let m = sent(&dev);
        assert!(m.contains(&[0x9B, 0x04, 0]) && m.contains(&[0xBB, 0x04, 0]));
    }

    /// GRID SLIDE / ADJUST held: the platter's turns go to the grid (milliseconds of the
    /// track, `jog x 60000 / rpm`), and the deck is not scratched.
    #[test]
    fn third_pass_grid_takes_the_platter() {
        let csv = "@file,1,PIONEER DDJ-ERGO\r\nJogScratch,Jog,JogRotate,B022,0,,,,,,,,,RO;Fast,\r\nJogTouch,JogTouch,JogTouch,9036,0,,,,,,,,,RO;Fast,\r\n";
        let (hub, dev, _rx) = test_device("PIONEER DDJ-ERGO", csv);
        hub.grid[0].store(2, Relaxed);
        let _ = feed(&hub, &dev, &[0x90, 0x36, 0x7F], 0.0);
        let _ = feed(&hub, &dev, &[0xB0, 0x22, 0x45], 10.0);
        let k = dev.jog.lock()[0].k();
        let acc = hub.grid_acc.lock()[0];
        assert!((acc - 5.0 * k).abs() < 1e-9, "{acc} vs {}", 5.0 * k);
        assert!(matches!(hub.decks[0].lock().dk.jog, crate::deck::Jog::None), "no scratch while the grid has the platter");
        // Let go of GRID SLIDE: the platter, touched again, scratches.
        hub.grid[0].store(0, Relaxed);
        let _ = feed(&hub, &dev, &[0x90, 0x36, 0x7F], 15.0);
        let _ = feed(&hub, &dev, &[0xB0, 0x22, 0x45], 20.0);
        assert!(!matches!(hub.decks[0].lock().dk.jog, crate::deck::Jog::None));
    }

    /// The Pad Editor's slots: rekordbox's `Mode.pad.csv` gives button n its mode, and the
    /// pads of a slot are the mode's own pads.
    #[test]
    fn third_pass_pad_editor_slots() {
        use crate::midi_acts::{editor_pad_mode, editor_pad_name};
        let modes = crate::pads::read_mode_csv();
        if !modes.is_empty() {
            let sel: Vec<String> = (1..=8u8).map(|k| modes.iter().find(|m| m.select == Some(k)).map(|m| m.function.clone()).unwrap_or_default()).collect();
            assert_eq!(sel, ["HotCue", "PadFx1", "BeatJump", "Sampler", "Keyboard", "PadFx2", "BeatLoop", "KeyShift"]);
        }
        assert_eq!(editor_pad_name("HotCue", 0, true).as_deref(), Some("PAD1_HotCue+Shift"));
        assert_eq!(editor_pad_name("BeatJump", 7, true).as_deref(), Some("PAD8_BeatJump"));
        assert_eq!(editor_pad_name("Slicers", 4, true).as_deref(), Some("PAD5_Slicer+Shift"));
        assert_eq!(editor_pad_name("User3", 0, false), None);
        assert_eq!(editor_pad_mode("User3"), Some(crate::pads::PadMode::User(2)));
        assert_eq!(editor_pad_mode("KeyShift"), Some(crate::pads::PadMode::KeyShift));
        for name in ["PAD1_HotCue+Shift", "PAD8_BeatJump", "PAD5_Slicer+Shift", "PAD3_Keyboard+Shift", "PAD2_PadFx2"] {
            assert!(registry().by_name(name).is_some(), "{name}");
        }
    }

    /// Names no part of rekordbox registers are not functions here either, and every name of
    /// the factory files that does not act has a reason or is listed as not built.
    #[test]
    fn third_pass_reasons() {
        for n in crate::midi_cover::NOT_IN_REKORDBOX {
            assert!(registry().by_name(n).is_none(), "{n}");
            assert!(crate::midi_cover::reason(n).is_some());
        }
        assert!(crate::midi_cover::reason("DvsAbsolute").unwrap().contains("DVS"));
        assert!(crate::midi_cover::reason("PAD1_Slicer+Shift").is_some() && crate::midi_cover::reason("PAD5_Slicer+Shift").is_none());
        // The jog's grid mode and the master cue are functions with lights.
        assert!(act_has_led(Act::More(crate::midi_acts::M::MasterCue)) && act_has_led(Act::More(crate::midi_acts::M::Grid(true))));
        assert!(!input_acts("Discover", Kind::Button) && input_acts("MasterCue", Kind::Button) && input_acts("PAD3_PadMode5+Shift", Kind::Pad(crate::midi_csv::PadType::Rx)));
    }

    fn tone(secs: f64) -> Arc<crate::audio::Decoded> {
        let n = (44100.0 * secs) as usize;
        Arc::new(crate::audio::Decoded { sr: 44100, samples: vec![0.0; n * 2], raw_frames: n, damaged: 0, truncated: false })
    }

    fn wait(what: &str, mut ok: impl FnMut() -> bool) {
        let t0 = Instant::now();
        while !ok() {
            assert!(t0.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// End to end over CoreMIDI: a virtual controller (a source that sends, a destination
    /// that receives the LEDs) is found by name, its user mapping is loaded, notes and CCs
    /// change the decks and the mixer in the engine without any interface frame, and the
    /// LED messages come back out.
    #[test]
    fn a_virtual_controller_plays_a_deck_moves_the_mixer_and_gets_its_leds() {
        // (A fixed name: rekordbox, when it runs, writes a header file for every new port name.)
        let name = "rbxtest-midi4 test ctl".to_string();
        let dir = std::env::temp_dir().join(format!("recordbox-midi-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The controller's mapping, as a user file in rekordbox's format.
        let csv = format!(
            "@file,1,{name}\r\n\
             MidiOutInterval,MidiOutInterval,Parameter,FFF3,,,,,,,,,,Value=2;,\r\n\
             PlayPause,PlayPause,Button,,900B,910B,,,,900B,910B,,,Fast;Blink=1000;Priority=50;,\r\n\
             Cue,Cue,Button,,900C,910C,,,,900C,910C,,,Fast;Blink=500;Priority=50;,\r\n\
             PAD1_HotCue,PAD1_HotCue,Pad,,9700,9900,,,,9700,9900,,,Fast;,\r\n\
             ChannelFader,ChannelFader,KnobSliderHiRes,,B013,B113,,,,,,,,Fast;,\r\n\
             CrossFader,CrossFader,KnobSlider,B61F,,,,,,,,,,Fast;,\r\n\
             EQLow,EQLow,KnobSlider,,B00F,B10F,,,,,,,,Fast;,\r\n\
             TempoSlider,TempoSlider,KnobSliderHiRes,,B000,B100,,,,,,,,Fast;,\r\n\
             JogPitchBend,JogPitchBend,JogRotate,,B023,B123,,,,,,,,RO;,\r\n\
             Sync,Sync,Button,,9058,9158,,,,9058,9158,,,,\r\n\
             JogPlayPauseIndicatorShort,JogPlayPauseIndicatorShort,JogIndicator,,,,,,,BB00,BB01,,,RO;Min=1;Max=72;Priority=100;,\r\n\
             ChannelLevel,ChannelLevel,Indicator,,,,,,B002,0,1,,,RO;Priority=50;,\r\n"
        );
        std::fs::write(dir.join(format!("{name}.midi.csv")), csv).unwrap();

        // The virtual controller: its source (what it sends) and its destination (its LEDs).
        let Some(mut ctl_out) = midir::MidiOutput::new("test-controller").ok().and_then(|o| o.create_virtual(&name).ok()) else {
            eprintln!("a_virtual_controller...: skipped, CoreMIDI virtual ports are not available here");
            return;
        };
        let leds: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(vec![]));
        let l2 = leds.clone();
        let _ctl_in = midir::MidiInput::new("test-controller").unwrap().create_virtual(&name, move |_, m, _| l2.lock().push(m.to_vec()), ()).unwrap();

        let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        for d in &decks[..2] {
            d.lock().audio = Some(tone(60.0));
        }
        let (hub, rx) = Hub::new(decks.clone(), dir.clone());
        // Only this test's own port: a real controller on the machine (a DDJ-S1 was) is not
        // opened, so nothing is sent to it.
        *hub.only.lock() = Some(vec![name.clone()]);
        {
            let mut s = hub.snap.lock();
            s.decks[0].loaded = true;
            s.decks[1].loaded = true;
            s.decks[0].cue_point = 1.0;
            s.decks[0].hot[0] = Some((12.0, None));
            s.decks[0].assign = 1;
            s.decks[1].assign = 2;
        }
        hub.scan();
        let dev = hub.device(&name).expect("the virtual controller is found by its port name");
        assert!(dev.has_output && !dev.factory && !dev.needs_auth);
        assert_eq!(dev.interval_ms.load(Relaxed), 2);
        // The interface hears of it: the status line's notice, then the device list.
        let evs: Vec<UiEvent> = rx.try_iter().collect();
        assert!(evs.iter().any(|e| matches!(e, UiEvent::Connected { device, .. } if *device == name)));
        assert!(evs.iter().any(|e| matches!(e, UiEvent::Ports)));
        // A second scan does not connect it twice.
        hub.scan();
        assert_eq!(hub.devices.lock().iter().filter(|d| d.name == name).count(), 1);
        assert_eq!(hub.devices.lock().len(), 1, "no other port was opened");

        // The outputs that follow the engine: deck 1 half a turn in (0.9 s at 33 1/3 rpm =
        // 180 degrees: ring step 1 + 72 * 180 / 360 = 37), its level at 0.5 (50).
        {
            let mut d = decks[0].lock();
            d.pos = 0.9 * 44100.0;
            d.peak = [0.5, 0.4];
        }
        hub.engine_outputs();
        wait("the jog ring and the level meter", || {
            let l = leds.lock();
            l.iter().any(|m| m == &[0xBB, 0x00, 37]) && l.iter().any(|m| m == &[0xB0, 0x02, 50])
        });
        {
            let mut d = decks[0].lock();
            d.pos = 0.0;
            d.peak = [0.0, 0.0];
        }
        // PLAY on deck 1: the engine plays, and the PLAY LED goes on (it was not lit).
        ctl_out.send(&[0x90, 0x0B, 0x7F]).unwrap();
        wait("deck 1 playing", || decks[0].lock().playing);
        ctl_out.send(&[0x90, 0x0B, 0x00]).unwrap();
        wait("the PLAY LED", || leds.lock().iter().any(|m| m == &[0x90, 0x0B, 0x7F]));
        // The interface is told, and that the engine side is done.
        let ev = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        match ev {
            UiEvent::Control { tag, deck, val, fast_done, .. } => {
                assert_eq!((registry().get(tag).unwrap().name.as_str(), deck, val, fast_done), ("PlayPause", 1, Val::On(true, 127), true));
            }
            _ => panic!("not a control event"),
        }
        // CUE while playing: back to the cue point, stopped; its LED lights (on the cue point).
        ctl_out.send(&[0x90, 0x0C, 0x7F]).unwrap();
        wait("deck 1 back on its cue point", || {
            let s = decks[0].lock();
            !s.playing && (s.pos - 44100.0).abs() < 1.0
        });
        wait("the CUE LED", || leds.lock().iter().any(|m| m == &[0x90, 0x0C, 0x7F]));
        // The stopped deck's PLAY LED blinks in its 1000 ms cycle: off and on arrive.
        leds.lock().clear();
        wait("PLAY blinking", || {
            let l = leds.lock();
            l.iter().any(|m| m == &[0x90, 0x0B, 0x00]) && l.iter().any(|m| m == &[0x90, 0x0B, 0x7F])
        });
        ctl_out.send(&[0x90, 0x0C, 0x00]).unwrap();
        // Hot cue A held on the stopped deck with Gate Cue on (Preferences > Controller > Deck
        // > HOT CUE): plays from 12 s; released: back there, stopped.
        crate::prefs_bridge::GATE_CUE.store(true, std::sync::atomic::Ordering::Relaxed);
        ctl_out.send(&[0x97, 0x00, 0x7F]).unwrap();
        wait("hot cue A playing", || {
            let s = decks[0].lock();
            s.playing && s.pos >= 12.0 * 44100.0
        });
        assert!(hub.pad_held(0));
        ctl_out.send(&[0x97, 0x00, 0x00]).unwrap();
        wait("hot cue A released", || {
            let s = decks[0].lock();
            !s.playing && (s.pos - 12.0 * 44100.0).abs() < 1.0
        });
        assert!(!hub.pad_held(0));
        // Deck 2's channel fader, 14-bit: half way.
        ctl_out.send(&[0xB1, 0x13, 0x40]).unwrap();
        ctl_out.send(&[0xB1, 0x33, 0x00]).unwrap();
        let half = crate::mixer::channel_fader_gain(0.5, 1);
        wait("deck 2's fader", || (decks[1].lock().ch_fader - half).abs() < 1e-6);
        // The crossfader hard to B: deck 1 (side A) is silent, deck 2 at full level.
        ctl_out.send(&[0xB6, 0x1F, 0x7F]).unwrap();
        wait("the crossfader", || decks[0].lock().fader.abs() < 1e-6 && (decks[1].lock().fader - 1.0).abs() < 1e-6);
        // EQ LOW of deck 1 fully down.
        ctl_out.send(&[0xB0, 0x0F, 0x00]).unwrap();
        wait("EQ LOW", || decks[0].lock().eq_pos[0] == 0.0);
        // The tempo slider: armed and 6 % off the deck's tempo, it waits; once brought to
        // the deck's tempo it has the deck.
        ctl_out.send(&[0xB0, 0x00, 0x66]).unwrap();
        ctl_out.send(&[0xB0, 0x20, 0x00]).unwrap();
        wait("the takeover to notice", || hub.takeover_state(0).state == 3);
        assert_eq!(decks[0].lock().tempo, 1.0);
        ctl_out.send(&[0xB0, 0x00, 0x40]).unwrap();
        ctl_out.send(&[0xB0, 0x20, 0x00]).unwrap();
        wait("the takeover to match", || hub.takeover_state(0).state == 5);
        ctl_out.send(&[0xB0, 0x00, 0x60]).unwrap();
        ctl_out.send(&[0xB0, 0x20, 0x00]).unwrap();
        wait("the tempo", || (decks[0].lock().tempo - (1.0 + 0.10 * 4096.0 / 8191.0)).abs() < 1e-6);
        // The outer jog ring: a bend while it turns, gone 60 ms after it stops.
        for _ in 0..8 {
            ctl_out.send(&[0xB0, 0x23, 0x50]).unwrap();
            std::thread::sleep(Duration::from_millis(8));
        }
        wait("the bend", || decks[0].lock().bend > 0.0);
        wait("the bend to end", || decks[0].lock().bend == 0.0);
        // A row without Fast (SYNC) is left to the interface.
        while rx.try_recv().is_ok() {}
        ctl_out.send(&[0x90, 0x58, 0x7F]).unwrap();
        loop {
            match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                UiEvent::Control { tag, fast_done, .. } if registry().get(tag).map(|f| f.name == "Sync").unwrap_or(false) => {
                    assert!(!fast_done);
                    break;
                }
                _ => {}
            }
        }
        // LEDs set from the app's side arrive in priority order, the newest value per code.
        leds.lock().clear();
        let sync = registry().tag("Sync");
        dev.set_led(sync, 1, Led::On);
        dev.set_led(sync, 2, Led::On);
        dev.set_led(sync, 2, Led::Off);
        wait("the SYNC LEDs", || {
            let l = leds.lock();
            l.iter().any(|m| m == &[0x90, 0x58, 0x7F]) && l.iter().any(|m| m == &[0x91, 0x58, 0x00])
        });
        assert!(dev.received.load(Relaxed) >= 20 && dev.sent.load(Relaxed) >= 6);

        // LEARN: the worker shows the message to the window and still acts on it.
        dev.learning.store(true, Relaxed);
        while rx.try_recv().is_ok() {}
        ctl_out.send(&[0x90, 0x0B, 0x7F]).unwrap();
        let mut learned = false;
        for _ in 0..4 {
            if let Ok(UiEvent::Learn { msg, .. }) = rx.recv_timeout(Duration::from_secs(2)) {
                assert_eq!(msg, [0x90, 0x0B, 0x7F]);
                learned = true;
                break;
            }
        }
        assert!(learned);
        wait("deck 1 playing again", || decks[0].lock().playing);

        // Unplugged: the device goes at the next scan.
        drop(ctl_out);
        drop(_ctl_in);
        wait("the device to go", || {
            hub.scan();
            hub.devices.lock().is_empty()
        });
        assert!(!dev.alive.load(Relaxed));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
