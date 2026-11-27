//! The controller functions added by the third MIDI pass: what they do in recordbox, which
//! value classes they take, and their lights. `midi.rs` keeps the registry (`Act::More`) and
//! sends these here. Traced handlers of rekordbox 7.2.19 are named where they apply; the rest
//! is marked as a decision.

use crate::app::*;
use crate::midi::{Hub, InClass, Led};
use crate::midi_map::Val;
use crate::model::*;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What a function of this file does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum M {
    /// MASTER CUE (`MasterCueDeviceComponent::eventOnChanged` 0x1000dccf0): the master in the
    /// headphones on or off. Not on the DDJ-SX / SX2, which do it in the hardware.
    MasterCue,
    /// BEAT JUMP page with Shift (Parameter2Left/Right_BeatJump, PageLeft/Right_BeatJump:
    /// `BeatJumpView::eventOnChanged` 0x101ddd50c, `PageSelector::rightButtonClick(true)`).
    JumpPage2(i32),
    /// ActiveLoop Select / Off for the hot cue loops or the memory loops.
    ActiveLoopSel { hot: bool, off: bool },
    /// AutoCue All Decks.
    AutoCueAll,
    /// GRID ADJUST (false) / GRID SLIDE (true) held: the jog changes the grid
    /// (`UiPlayer::deviceGrid` 0x102028acc, `deviceJogValue` 0x102029eb8).
    Grid(bool),
    /// A function of a feature recordbox does not have; the reason.
    Off(&'static str),
    /// ACTIVE CENSOR pads (`ActiveCensorView::eventOnChanged` 0x101084c40): 0..3 the effect,
    /// 4 IN, 5 OUT, 6 CLEAR, 7 ON/OFF.
    Censor(u8),
    CensorCall(i32),
    CensorBeat(i32),
    /// KEY SHIFT and KEYBOARD pads and pages (`KeyShiftView` / `KeyboardView`).
    KeyShiftPad(u8),
    KeyShiftPage(i32),
    KeyboardPad(u8),
    KeyboardDelete(u8),
    KeyboardPage(i32),
    /// SAMPLE SCRATCH: load slot n onto the deck / unload it.
    ScratchPad(u8, bool),
    /// XDJ-R1 HOT CUE / SAMPLER pads (`MidiMapXDJR1`): a hot cue when the slot has one, else
    /// the sampler slot; with Shift delete / stop.
    HotCueSampler(u8, bool),
    /// CFX ON (the shown effect on / off) and a controller's CFX buttons 1..6.
    CfxOn,
    CfxButton(u8),
    /// MERGE FX: parameter knob, ON, deck assign, effect select, cancel; sample volume.
    MfxParam(u8),
    MfxOn(u8),
    MfxAssign(u8, u8),
    MfxSelect(u8, i32),
    MfxCancel(u8),
    MfxSampleVolume,
    /// GROOVE CIRCUIT DRUM SWAP (`GrooveCircuit::eventOnChanged` 0x1000fe60c).
    DrumMulti(bool),
    DrumBank(u8),
    DrumGain,
    DrumSlot(u8),
    DrumCapture,
    DrumCaptureLen,
    DrumRelease,
    DrumReleaseSelect,
    /// A velocity pad's level, sent ahead of its note (PADn_VelocitySampler+Velocity).
    VelocityLevel(u8),
    SamplerTapBpm,
    /// The crossfader curve knob (`CrossFaderCurveComp::eventAbsoluteValueChanged`
    /// 0x1000dc828); the three-position switch acts at its middle and top positions only.
    XfCurve(bool),
    /// The preview player: play / stop, skip.
    Preview(bool),
    /// The pad editor's eight pad mode buttons of a controller (PadMode1..8), their pads
    /// (PADp_PadModem, +Shift), page buttons, and the permission lights.
    EditorMode(u8),
    EditorPad(u8, u8, bool),
    EditorPage(u8, i32),
    Permission(u8),
    /// VinylState: the controller tells which state its VINYL button shows.
    VinylState,
    /// TempoCenter (0: tempo reset) and TempoInc / Dec Small, Medium, Large: the slider by
    /// that many hundredths of a percent (10, 50, 200: `WidgetPlayerDJ::operate` 92 / 93).
    TempoStep(i32),
    /// Deck (DeckSelect): rekordbox notes the button held (`UiPlayer::isDeckPressed`).
    DeckHeld,
    /// AutoLoopMode / ManualLoopMode: the deck's loop box shows AUTO or MANUAL loop.
    LoopBox(bool),
    /// TimeMode (the deck) / TimeMode4 (all decks): elapsed / remaining.
    TimeMode(bool),
    /// The browse knob's press (ForwardAndLoad): into the folder from the tree, else load the
    /// selected track to the deck; pressed twice within 300 ms: Instant Doubles.
    ForwardAndLoad,
    /// Fx1-1_BeatTarget ..: the FX unit's beat buttons step that slot only; 255: all again
    /// (DeselectAllFxBeatTarget, both units).
    BeatTarget(u8, u8),
    /// Capture (loop / slicer capture to the sampler) and its light.
    Capture,
    CaptureOk,
    /// BANK pad mode and its pads 1..4 (the sampler bank).
    BankMode,
    BankPad(u8),
    /// Crossfader curve switch positions (STRAIGHT, MID, SHARP, THRU) and channel fader curve
    /// switch positions.
    XfCurvePos(u8),
    ChCurvePos(u8),
    /// FX BPM AUTO / TAP for both units.
    FxBpmMode(bool),
    /// The CFX sub parameter knob: the effect's PARAM.
    CfxSub,
    /// An FX slot on while held, for a channel (CHn_FXa-bOnMomentary, Sampler_..) or alone.
    FxMomentary(u8, u8, u8),
    /// An FX slot's effect from a knob's position across the list.
    FxSelectKnob(u8, u8),
    /// Beat jump by a number of beats (Jump4Fwd ...).
    Jump(i32),
    /// The direction lever: reverse (or slip reverse) while it is down.
    Direction(bool),
    /// The EQ / ISOLATOR switch.
    EqCurve,
    /// Recording start / stop.
    Rec,
    /// Outputs: lights that follow a state (no input).
    Out(Out),
    /// Deck3 / Deck4 (+WithoutState): the controller's side now plays deck 3 / 4 (its rows
    /// marked Dual follow; pressed again, back to deck 1 / 2). A decision: the handler's
    /// lambdas (`deviceDeckSelect` $_19 / $_20) were not followed.
    DeckSide(u8),
    /// A STEMS part's Instant Doubles: the other deck's track comes here with only that part
    /// playing (source in the engine's order). A decision; not traced.
    PartDoubles(u8),
    /// Browse: the tree (true) or the track list; the first (true) or last row; a row by
    /// number; the list's scroll position; the sort column by number.
    AreaEdge(bool),
    BrowseEdge(bool),
    BrowseLine,
    ScrollBar,
    SortType,
    /// PAD FX page (rekordbox's PageLeft = `onPageUp`, PageRight = `onPageDown`): the pads
    /// show their first or second eight.
    PadFxPage(u8),
}

/// Lights a controller has that only follow recordbox's state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    /// FXn BeatMultiplier: the unit's beat value (its place in the beat table).
    FxBeat(u8),
    /// The release FX: lit while it is on; for a release type 0..2 while it is the chosen one.
    ReleaseOn(u8),
    ReleaseType(u8, u8),
    /// MERGE FX unit on; with a preset, while that MERGE FX is the chosen one.
    MfxIllumi(u8, u8),
    /// The STEMS layout of three parts (CustomPartLayout).
    PartLayout,
}

/// What the rows of a value class give a function of this file.
pub fn accepts(m: M, c: InClass) -> bool {
    match m {
        M::Off(_) => false,
        M::DrumGain | M::MfxSampleVolume | M::XfCurve(_) => matches!(c, InClass::Abs | InClass::Rel),
        M::MfxParam(_) => matches!(c, InClass::Abs | InClass::Rel),
        M::VelocityLevel(_) => c == InClass::Abs,
        M::Permission(_) | M::Out(_) | M::CaptureOk => false,
        M::CfxSub | M::FxSelectKnob(..) | M::BrowseLine | M::ScrollBar | M::SortType => c == InClass::Abs,
        _ => c == InClass::Press,
    }
}

/// The functions here whose light shows a state (the others light while held).
pub fn has_led(m: M) -> bool {
    matches!(m, M::MasterCue | M::CfxOn | M::CfxButton(_) | M::MfxOn(_) | M::MfxAssign(..) | M::DrumMulti(_) | M::DrumBank(_) | M::DrumSlot(_) | M::DrumCapture | M::Grid(_) | M::EditorMode(_) | M::EditorPad(..) | M::Permission(_) | M::Censor(_) | M::KeyShiftPad(_) | M::KeyboardPad(_) | M::ScratchPad(..) | M::HotCueSampler(..) | M::Preview(false) | M::LoopBox(_) | M::Out(_) | M::CaptureOk | M::BankMode | M::BankPad(_) | M::FxMomentary(..) | M::ScrollBar | M::SortType)
}

/// Per-app state of this file (a field of `MidiState`).
pub struct MoreState {
    /// MASTER CUE: rekordbox's mixer has it on for the computer's own audio
    /// (`SettingIF::updateHeadPhoneCueSetting` 0x1015ca2f4: on unless the audio device's
    /// record says otherwise).
    pub master_cue: bool,
    /// A velocity pad's level and when it came (per deck and pad).
    pub level: [[Option<(u8, Instant)>; 8]; 4],
    /// SamplerTapBpm taps.
    pub taps: Vec<Instant>,
    /// When the grid jog was last applied.
    pub grid_at: Option<Instant>,
    /// Deck buttons held (DeckSelect), the BANK pad mode per deck, the FX beat target per unit.
    pub deck_held: [bool; 4],
    pub bank_mode: [bool; 4],
    pub beat_target: [Option<u8>; 2],
    /// ForwardAndLoad's last press per deck.
    pub load_at: [Option<Instant>; 4],
    /// A part's Instant Doubles waiting for the track: (deck, part).
    pub part_doubles: Option<(u8, u8)>,
}

impl Default for MoreState {
    fn default() -> MoreState {
        MoreState { master_cue: true, level: [[None; 8]; 4], taps: vec![], grid_at: None, deck_held: [false; 4], bank_mode: [false; 4], beat_target: [None; 2], load_at: [None; 4], part_doubles: None }
    }
}

/// The level a velocity pad of deck `n` takes: the PADn_VelocitySampler+Velocity value sent
/// in the last 100 ms, if any.
pub fn pad_level(app: &App, n: usize, pad: usize) -> Option<u8> {
    app.midi.more.level.get(n.min(3)).and_then(|l| l.get(pad.min(7)).copied().flatten()).filter(|(_, t)| t.elapsed() < Duration::from_millis(100)).map(|(v, _)| v)
}

/// The mode function a controller's pad mode button `slot` (0..7) has in the Pad Editor
/// (rekordbox's `Mode.pad.csv` `select` column unless the user changed it).
pub fn editor_function(app: &App, slot: usize) -> String {
    let assign = (app.prefs.pads.editor_assign as usize).min(1);
    if let Some(f) = app.prefs.pads.editor_modes.get(assign).and_then(|v| v.get(slot)).filter(|f| !f.is_empty()) {
        return f.clone();
    }
    static MODES: std::sync::OnceLock<Vec<crate::pads::ModeEntry>> = std::sync::OnceLock::new();
    let modes = MODES.get_or_init(crate::pads::read_mode_csv);
    modes.iter().find(|m| m.select == Some(slot as u8 + 1)).map(|m| m.function.clone()).unwrap_or_else(|| ["HotCue", "PadFx1", "BeatJump", "Sampler", "Keyboard", "PadFx2", "BeatLoop", "KeyShift"][slot.min(7)].to_string())
}

/// The pad mode a Pad Editor mode function selects.
pub fn editor_pad_mode(f: &str) -> Option<crate::pads::PadMode> {
    use crate::pads::PadMode as P;
    Some(match f {
        "HotCue" => P::HotCue,
        "PadFx1" | "PadFx2" => P::PadFx,
        "BeatJump" | "UserBeatJump" => P::BeatJump,
        "Sampler" => P::Sampler,
        "Keyboard" => P::Keyboard,
        "KeyShift" | "UserKeyShift" => P::KeyShift,
        "BeatLoop" => P::BeatLoop,
        "Slicers" => P::Slicer,
        "ActiveCensor" => P::ActiveCensor,
        "SequenceCall" => P::SeqCall,
        "SampleScratch" => P::SamplerScratch,
        "Stems" => P::Stems,
        f => P::User(f.strip_prefix("User").and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=8).contains(n))? - 1),
    })
}

/// The registry name a pad of a Pad Editor mode acts as (`PadEditorData::eventOnChanged`
/// 0x1020f7f44 hands the pad to the mode's own pad): None for modes whose pads recordbox
/// handles here (USER modes).
pub fn editor_pad_name(f: &str, pad: u8, shift: bool) -> Option<String> {
    let p = pad + 1;
    let s = if shift { "+Shift" } else { "" };
    Some(match f {
        "HotCue" => format!("PAD{p}_HotCue{s}"),
        "PadFx1" | "PadFx2" => format!("PAD{p}_{f}"),
        "BeatJump" | "UserBeatJump" => format!("PAD{p}_BeatJump"),
        "Sampler" => format!("PAD{p}_Sampler{s}"),
        "Keyboard" => format!("PAD{p}_Keyboard{s}"),
        "KeyShift" | "UserKeyShift" => format!("PAD{p}_KeyShift"),
        "BeatLoop" => format!("PAD{p}_BeatLoop"),
        "Slicers" => format!("PAD{p}_Slicer{s}"),
        "ActiveCensor" => format!("PAD{p}_ActiveCensor"),
        "SequenceCall" => format!("PAD{p}_SequenceCall{s}"),
        "SampleScratch" => format!("PAD{p}_SampleScratch{s}"),
        _ => return None,
    })
}

/// The registry names of a Pad Editor mode's page buttons (left, right).
fn editor_page_name(f: &str, right: bool) -> Option<String> {
    let side = if right { "Right" } else { "Left" };
    Some(match f {
        "HotCue" => format!("Parameter1{side}_HotCue"),
        "PadFx1" | "PadFx2" => format!("Parameter1{side}_{f}"),
        "BeatJump" | "UserBeatJump" => format!("Parameter1{side}_BeatJump"),
        "Sampler" => format!("Parameter1{side}_Sampler"),
        "Keyboard" => format!("Parameter1{side}_Keyboard"),
        "KeyShift" | "UserKeyShift" => format!("Parameter1{side}_KeyShift"),
        "Slicers" => format!("Parameter1{side}_Slicer"),
        "ActiveCensor" => format!("Parameter1{side}_ActiveCensor"),
        _ => return None,
    })
}

/// Run another registry function by name (a Pad Editor pad, an XDJ-R1 pad).
fn run(app: &mut App, hub: &Arc<Hub>, device: &str, name: &str, deck: u8, val: Val) {
    if let Some(f) = crate::midi::registry().by_name(name) {
        crate::midi::apply(app, hub, device, f, deck, val, false, 0);
    }
}

/// The CFX effect a controller's CFX button stands for: read from the button's row in the
/// device's mapping ("CFX NOISE On/Off"). A decision: rekordbox has a table per device class
/// (`RbxCfxControlBehavior::selectFx`), which was not traced; the factory files name the
/// effect of each button.
fn cfx_button_kind(hub: &Hub, device: &str, i: u8) -> Option<u8> {
    let dev = hub.device(device)?;
    let map = dev.map.lock();
    let name = format!("CFX{}On", i + 1);
    let comment = map.inputs_named(&name).into_iter().find(|c| !c.is_empty())?;
    let up = comment.to_ascii_uppercase().replace("D-ECHO", "DUB ECHO").replace("DUBECHO", "DUB ECHO");
    crate::cfx::LIST.iter().filter_map(|(n, k)| k.map(|k| (*n, k))).find(|(n, _)| up.contains(n)).map(|(_, k)| k)
}

/// What a function of this file does, on the interface thread. `n`: the app's deck.
pub fn apply(app: &mut App, hub: &Arc<Hub>, device: &str, m: M, deck: u8, n: u8, val: Val) {
    let press = matches!(val, Val::On(true, _));
    let release = matches!(val, Val::On(false, _));
    let rel = if let Val::Rel(r) = val { r } else { 0 };
    let unit = val.unit();
    let ni = (n as usize).min(3);
    match m {
        M::Off(_) | M::Permission(_) => {}
        M::MasterCue => {
            if press && !device.contains("PIONEER DDJ-SX") {
                app.midi.more.master_cue = !app.midi.more.master_cue;
            }
        }
        M::JumpPage2(dir) => {
            if press {
                let pg = app.prefs.pads.beat_jump_page[ni];
                app.prefs.pads.beat_jump_page[ni] = crate::pads::jump_page_step(pg, dir > 0, true);
                app.save_prefs();
            }
        }
        M::ActiveLoopSel { hot, off } => {
            // A decision (the handler's lambdas `deviceActiveLoop` $_30 / $_31 were not
            // followed): Select makes the next loop of that kind after the active one (by
            // position) the active loop, Off makes none of them active.
            if press {
                app.with_deck(n, |a| {
                    let kind = if hot { CueKind::Hot } else { CueKind::Memory };
                    let mut loops: Vec<Cue> = a.deck.cues.iter().filter(|c| c.kind == kind && c.end.map(|e| e > c.start).unwrap_or(false)).cloned().collect();
                    loops.sort_by(|x, y| x.start.partial_cmp(&y.start).unwrap_or(std::cmp::Ordering::Equal));
                    let cur = loops.iter().position(|c| c.active_loop);
                    let next = if off || loops.is_empty() { None } else { Some(cur.map(|i| (i + 1) % loops.len()).unwrap_or(0)) };
                    for (i, c) in loops.into_iter().enumerate() {
                        let want = Some(i) == next;
                        if c.active_loop != want {
                            let mut c = c;
                            c.active_loop = want;
                            a.update_cue(c);
                        }
                    }
                });
            }
        }
        M::AutoCueAll => {
            if press {
                let on = !crate::prefs_bridge::auto_cue_on(app, app.px.active);
                for d in 0..app.deck_count() {
                    if crate::prefs_bridge::auto_cue_on(app, d) != on {
                        crate::prefs_bridge::toggle_auto_cue(app, d);
                    }
                }
            }
        }
        M::Grid(slide) => {
            if press || release {
                // (Refused while a loop is being adjusted: `kGridOperateErrorLoopAdjust`;
                // recordbox has no loop adjust from the jog.)
                let mode = if press { if slide { 2 } else { 1 } } else { 0 };
                let cur = hub.grid[ni].load(Relaxed);
                if press && cur != 0 {
                    return;
                }
                if release && cur != if slide { 2 } else { 1 } {
                    return;
                }
                if release {
                    grid_flush(app, hub, ni);
                }
                hub.grid[ni].store(mode, Relaxed);
            }
        }
        M::Censor(i) => {
            if press {
                app.with_deck(n, |a| match i {
                    0..=3 => a.censor_set_kind(i),
                    4 => a.censor_in(),
                    5 => {
                        a.censor_out();
                    }
                    6 => {
                        a.censor_clear();
                    }
                    _ => a.censor_onoff(),
                });
            }
        }
        M::CensorCall(dir) => {
            if press {
                app.with_deck(n, |a| a.censor_call(dir > 0));
            }
        }
        M::CensorBeat(dir) => {
            if press {
                app.with_deck(n, |a| a.censor_beat(dir > 0));
            }
        }
        M::KeyShiftPad(i) => {
            if press {
                let page = app.prefs.pads.key_shift_page[ni];
                app.with_deck(n, |a| a.pad_key(crate::pads::key_pad_pos(page, i as usize)));
            }
        }
        M::KeyboardPad(i) => {
            let page = app.prefs.pads.keyboard_page[ni];
            if press {
                app.with_deck(n, |a| a.pad_keyboard_down(crate::pads::key_pad_pos(page, i as usize)));
            } else if release {
                app.with_deck(n, |a| a.pad_keyboard_up());
            }
        }
        M::KeyboardDelete(i) => {
            // (KEYBOARD's hot cue pads with Shift: the hot cue of that pad is deleted.)
            if press {
                app.with_deck(n, |a| {
                    if let Some(id) = a.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == i as i32).map(|c| c.id) {
                        a.delete_cue(id);
                    }
                });
            }
        }
        M::KeyShiftPage(dir) | M::KeyboardPage(dir) => {
            if press {
                let pg = if matches!(m, M::KeyShiftPage(_)) { &mut app.prefs.pads.key_shift_page[ni] } else { &mut app.prefs.pads.keyboard_page[ni] };
                *pg = (*pg as i32 + dir).clamp(1, 5) as u8;
                app.save_prefs();
            }
        }
        M::ScratchPad(i, eject) => {
            if press {
                if eject {
                    // (Unload the scratch sample: the deck is emptied when it holds that slot's
                    // track.)
                    let idx = app.prefs.sampler.bank.min(3) as usize * 16 + i as usize;
                    let track = app.prefs.sampler.slots.get(idx).cloned().flatten().map(|s| s.track);
                    app.with_deck(n, |a| {
                        if track.is_some() && a.deck.track == track && !a.playing() {
                            a.unload_track();
                        }
                    });
                } else {
                    app.with_deck(n, |a| a.sample_scratch_pad(i as usize));
                }
            }
        }
        M::HotCueSampler(i, shift) => {
            let has_hot = app.deck_of(n).cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == i as i32);
            let name = if has_hot { format!("PAD{}_HotCue{}", i + 1, if shift { "+Shift" } else { "" }) } else { format!("PAD{}_Sampler{}", i + 1, if shift { "+Shift" } else { "" }) };
            run(app, hub, device, &name, deck, val);
        }
        M::CfxOn => {
            if press {
                // The single CFX's effect on / off: off is the default FILTER at rest.
                app.px.cfx_user = !app.px.cfx_user;
            }
        }
        M::CfxButton(i) => {
            if press {
                if let Some(k) = cfx_button_kind(hub, device, i) {
                    if app.px.cfx_user && app.px.cfx_kind == k {
                        app.px.cfx_user = false;
                    } else {
                        app.px.cfx_user = true;
                        app.px.cfx_kind = k;
                        app.px.cfx_param = app.px.fxx.cfx_params[k as usize % 9];
                    }
                }
            }
        }
        M::MfxOn(u) => {
            if press {
                crate::mfx::toggle(app, (u as usize).min(1), false);
            }
        }
        M::MfxCancel(u) => {
            if press {
                crate::mfx::toggle(app, (u as usize).min(1), true);
            }
        }
        M::MfxParam(u) => {
            let s = &mut app.px.fxx.mfx.units[(u as usize).min(1)];
            if let Some(p) = unit {
                s.knob = (p * 2.0 - 1.0).clamp(-1.0, 1.0);
            } else if rel != 0 {
                // (An encoder: 1/64 of the knob's half travel a step; a decision.)
                s.knob = (s.knob + rel as f32 / 64.0).clamp(-1.0, 1.0);
            }
        }
        M::MfxAssign(u, d) => {
            if press {
                let u = (u as usize).min(1);
                app.prefs.fx.mfx.upper[u] = d >= 2;
                app.save_prefs();
            }
        }
        M::MfxSelect(u, dir) => {
            if press {
                let u = (u as usize).min(1);
                app.prefs.fx.mfx.select[u] = (app.prefs.fx.mfx.select[u] as i32 + dir).rem_euclid(4) as u8;
                app.save_prefs();
            }
        }
        M::MfxSampleVolume => {
            if let Some(p) = unit {
                app.prefs.fx.mfx.volume = (p * 24.0 - 12.0).clamp(-12.0, 12.0);
            } else if rel != 0 {
                app.prefs.fx.mfx.volume = (app.prefs.fx.mfx.volume + rel as f32 * 0.5).clamp(-12.0, 12.0);
            }
        }
        M::DrumMulti(multi) => {
            if press {
                app.groove_set_multi(ni, multi);
            }
        }
        M::DrumBank(b) => {
            if press {
                app.groove_set_bank(ni, b);
            }
        }
        M::DrumGain => {
            if let Some(p) = unit {
                for i in 0..crate::groove::SLOTS {
                    app.groove_set_slot_gain(ni, i, p);
                }
            }
        }
        M::DrumSlot(s) => {
            if press {
                let bank = app.gc.set.decks[ni].bank as usize;
                let idx = bank * 4 + s as usize;
                if !app.groove_capture(ni, idx) {
                    app.groove_slot_play(ni, idx);
                }
            }
        }
        M::DrumCapture => {
            if press {
                app.groove_capture_toggle(n);
            }
        }
        M::DrumCaptureLen => {
            if press {
                app.groove_capture_length_next();
            }
        }
        M::DrumRelease => {
            if press || release {
                app.groove_release_fx(ni, press);
            }
        }
        M::DrumReleaseSelect => {
            if press {
                let k = app.gc.set.decks[ni].fx_type;
                app.groove_select_fx(ni, (k + 1) % 3);
            }
        }
        M::VelocityLevel(pad) => {
            if let Val::Abs { v, .. } = val {
                let lv = (v.clamp(0, 127)) as u8;
                if let Some(l) = app.midi.more.level.get_mut(ni).and_then(|l| l.get_mut((pad as usize).min(7))) {
                    *l = Some((lv, Instant::now()));
                }
            }
        }
        M::SamplerTapBpm => {
            if press {
                // TAP: the average of the taps of the last 2 s (a decision; the sampler's
                // own tap was not traced).
                let now = Instant::now();
                let taps = &mut app.midi.more.taps;
                taps.retain(|t| now.duration_since(*t) < Duration::from_secs(2));
                taps.push(now);
                if taps.len() >= 2 {
                    let span = now.duration_since(taps[0]).as_secs_f64() / (taps.len() - 1) as f64;
                    if span > 0.0 {
                        let bpm = (60.0 / span) as f32;
                        let cur = app.prefs.sampler.bpm;
                        app.sampler_action(crate::sampler_ext::SmpAct::Bpm(bpm - cur));
                    }
                }
            }
        }
        M::XfCurve(three) => {
            if let Val::Abs { v, .. } = val {
                let c = if three {
                    match v {
                        0x2000 => Some(16),
                        0x3FFF => Some(32),
                        _ => None,
                    }
                } else {
                    Some(((v.clamp(0, 16383) as f64) * 32.0 / 16383.0).round() as u8)
                };
                if let Some(c) = c {
                    app.prefs.deck.crossfader_curve = c;
                    app.prefs.rb.crossfader_curve = c;
                }
            }
        }
        M::Preview(skip) => {
            if press {
                if skip {
                    // (15 s further into the track, as the keyboard's Skip; not traced.)
                    if let Some((id, Some(frac))) = app.preview_status() {
                        let dur = app.track(id).map(|t| t.duration).unwrap_or(0.0).max(1.0);
                        app.preview_play(id, (frac + 15.0 / dur).min(0.99));
                    }
                } else if app.preview_status().is_some() {
                    app.preview_stop();
                } else {
                    if app.list_dirty {
                        app.rebuild_list();
                    }
                    if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
                        app.preview_play(id, 0.0);
                    }
                }
            }
        }
        M::EditorMode(slot) => {
            if press {
                let f = editor_function(app, slot as usize);
                if let Some(mode) = editor_pad_mode(&f) {
                    app.with_deck(n, |a| {
                        a.set_pad_mode(mode);
                    });
                    app.midi.editor_slot[ni] = slot;
                }
            }
        }
        M::EditorPad(slot, pad, shift) => {
            let f = editor_function(app, slot as usize);
            match editor_pad_name(&f, pad, shift) {
                Some(name) => run(app, hub, device, &name, deck, val),
                None => {
                    // USER modes: the pad's item.
                    if let Some(crate::pads::PadMode::User(u)) = editor_pad_mode(&f) {
                        app.with_deck(n, |a| {
                            if press {
                                if let Some(item) = a.user_items(u as usize).get(pad as usize).cloned() {
                                    a.user_pad_down(&item, shift);
                                }
                            } else if release {
                                a.user_pad_up();
                            }
                        });
                    }
                }
            }
        }
        M::EditorPage(slot, dir) => {
            let f = editor_function(app, slot as usize);
            if let Some(name) = editor_page_name(&f, dir > 0) {
                run(app, hub, device, &name, deck, val);
            }
        }
        M::VinylState => {
            // The VINYL button's state as the controller has it: the jog follows.
            if press || release {
                hub.vinyl[ni].store(press, Relaxed);
            }
        }
        M::PadFxPage(p) => {
            if press {
                app.with_deck(n, |a| a.deck.x.pad_page = p.min(1));
            }
        }
        M::TempoStep(step) => {
            if press {
                app.with_deck(n, |a| {
                    if step == 0 {
                        a.tempo_reset();
                    } else if a.sync_follower() {
                        a.toast_err("Invalid operation. In Sync mode, Tempo Slider is only available for Master Deck.");
                    } else {
                        let range = a.tempo_range();
                        a.deck.x.tempo_adj = (a.deck.x.tempo_adj + step as f64 * 0.0001).clamp(-range, range);
                        a.push_deck_controls();
                    }
                });
            }
        }
        M::DeckHeld => {
            if press || release {
                app.midi.more.deck_held[ni] = press;
            }
        }
        M::LoopBox(manual) => {
            if press {
                app.px.loop_manual[ni] = manual;
            }
        }
        M::TimeMode(all) => {
            if press {
                if all {
                    let v = if app.prefs.deck.player_time_mode[ni] == 0 { 1 } else { 0 };
                    app.prefs.deck.player_time_mode = [v; 4];
                    app.save_prefs();
                } else {
                    app.with_deck(n, |a| a.time_mode_toggle());
                }
            }
        }
        M::ForwardAndLoad => {
            if press {
                if app.dialogs.ext.focus_tree {
                    run(app, hub, device, "Forward", 0, val);
                } else {
                    let hit = app.midi.more.load_at[ni].map(|t| t.elapsed() < Duration::from_millis(300)).unwrap_or(false);
                    app.midi.more.load_at[ni] = if hit { None } else { Some(Instant::now()) };
                    run(app, hub, device, if hit { "InstDoubles" } else { "Load" }, deck, val);
                }
            }
        }
        M::BeatTarget(u, slot) => {
            if press {
                if u > 1 {
                    app.midi.more.beat_target = [None; 2];
                } else {
                    let t = &mut app.midi.more.beat_target[u as usize];
                    *t = if *t == Some(slot) { None } else { Some(slot) };
                }
            }
        }
        M::Capture => {
            if press {
                app.with_deck(n, |a| {
                    a.slicer_capture();
                });
            }
        }
        M::BankMode => {
            if press {
                app.midi.more.bank_mode[ni] = !app.midi.more.bank_mode[ni];
            }
        }
        M::BankPad(b) => {
            if press {
                app.sampler_action(crate::sampler_ext::SmpAct::Bank(b.min(3)));
            }
        }
        M::XfCurvePos(p) => {
            // (STRAIGHT, MID, SHARP: the curve setting's ends and middle; THRU: no crossfader,
            // both sides at full level, as the sharpest curve. A decision.)
            if press {
                let c = [0u8, 16, 32, 32][(p as usize).min(3)];
                app.prefs.deck.crossfader_curve = c;
                app.prefs.rb.crossfader_curve = c;
            }
        }
        M::ChCurvePos(p) => {
            if press {
                app.prefs.ch_fader_curve = p.min(2);
                app.save_prefs();
            }
        }
        M::FxBpmMode(tap) => {
            if press {
                for u in 0..2 {
                    app.px.fx[u].auto = !tap;
                }
            }
        }
        M::CfxSub => {
            if let Some(p) = unit {
                app.px.cfx_param = p;
                let k = app.px.cfx_kind as usize % 9;
                app.px.fxx.cfx_params[k] = p;
            }
        }
        M::FxMomentary(u, s, ch) => {
            if press || release {
                let fx = &mut app.px.fx[(u as usize).min(1)];
                if ch != 255 {
                    crate::midi::fx_assign_set(&mut fx.params.assign, ch as usize, press);
                }
                if fx.params.single {
                    if s == 0 {
                        fx.params.one.on = press;
                    }
                } else {
                    fx.params.slots[(s as usize).min(2)].on = press;
                }
            }
        }
        M::FxSelectKnob(u, s) => {
            if let Some(p) = unit {
                let fx = &mut app.px.fx[(u as usize).min(1)];
                let single = fx.params.single;
                if !(single && s != 0) {
                    let list: Vec<crate::fx::Kind> = crate::fx::list(single).map(|i| i.kind).collect();
                    if !list.is_empty() {
                        let k = list[((p as f64 * list.len() as f64) as usize).min(list.len() - 1)];
                        let slot = if single { 3 } else { (s as usize).min(2) };
                        let cur = if single { fx.params.one.kind } else { fx.params.slots[slot].kind };
                        if cur != k {
                            fx.set_kind(slot, k);
                        }
                    }
                }
            }
        }
        M::Jump(beats) => {
            if press {
                app.with_deck(n, |a| a.beat_jump(beats as f64));
            }
        }
        M::Direction(slip) => {
            if press {
                app.with_deck(n, |a| a.reverse_toggle(slip));
            } else if release {
                app.with_deck(n, |a| a.reverse_off());
            }
        }
        M::EqCurve => {
            if press || release {
                app.prefs.eq_isolator = press;
                app.save_prefs();
            }
        }
        M::Rec => {
            if press {
                app.rec_button();
            }
        }
        M::Out(_) | M::CaptureOk => {}
        M::DeckSide(dk) => {
            if press {
                if let Some(dev) = hub.device(device) {
                    let mut map = dev.map.lock();
                    let side = ((dk + 1) % 2) as usize;
                    let base = side as u8 + 1;
                    map.host[side] = if map.host[side] == dk { base } else { dk };
                    map.dual_on[side] = map.host[side] == dk;
                }
            }
        }
        M::PartDoubles(src) => {
            if press {
                app.midi.more.part_doubles = Some((n, src));
                run(app, hub, device, "InstDoubles", deck, val);
            }
        }
        M::AreaEdge(tree) => {
            if press {
                app.dialogs.ext.focus_tree = tree;
            }
        }
        M::BrowseEdge(top) => {
            if press {
                if app.list_dirty {
                    app.rebuild_list();
                }
                if !app.list.is_empty() {
                    let i = if top { 0 } else { app.list.len() - 1 };
                    app.set_anchor(i);
                    app.selected.clear();
                    app.selected.insert(app.list[i]);
                    app.scroll_to_selected = true;
                }
            }
        }
        M::BrowseLine | M::ScrollBar => {
            // (BrowseLineNumber: the row at that number, 0 = the first; SCROLL BAR: the row
            // at that share of the list. Decisions: neither handler was traced.)
            if let Val::Abs { v, .. } = val {
                if app.list_dirty {
                    app.rebuild_list();
                }
                let len = app.list.len();
                if len > 0 {
                    let i = if m == M::BrowseLine { (v.max(0) as usize).min(len - 1) } else { ((unit.unwrap_or(0.0) as f64 * (len - 1) as f64).round() as usize).min(len - 1) };
                    app.set_anchor(i);
                    app.selected.clear();
                    app.selected.insert(app.list[i]);
                    app.scroll_to_selected = true;
                }
            }
        }
        M::SortType => {
            // (The sort keys of the browse functions in their order: BPM, title, track
            // number, artist, key. A decision.)
            if let Val::Abs { v, .. } = val {
                let cols = [Col::Bpm, Col::Title, Col::TrackNumber, Col::Artist, Col::Key];
                if let Some(c) = cols.get(v.max(0) as usize) {
                    app.prefs.sort_col = *c;
                    app.list_dirty = true;
                    app.save_prefs();
                }
            }
        }
    }
}

/// After Instant Doubles of a part: once the track is on the deck, only that part plays.
pub fn part_doubles_tick(app: &mut App) {
    let Some((n, src)) = app.midi.more.part_doubles else { return };
    if app.deck_of(n).loading.is_some() {
        return;
    }
    app.midi.more.part_doubles = None;
    if app.deck_of(n).audio.is_none() {
        return;
    }
    let mut s = app.player.decks[(n as usize).min(3)].lock();
    for (i, on) in s.stem_on.iter_mut().enumerate() {
        *on = i == src as usize;
    }
}

/// The grid change the jog asked for while GRID ADJUST / SLIDE is held, at most every 100 ms
/// as rekordbox's timers 18 / 19 (`UiPlayer::timerCallback` 0x101ab9d40): the platter's
/// travel in milliseconds of the track (`jog * 60000 / rpm`), whole milliseconds. SLIDE moves
/// the grid by that time; ADJUST changes the beat interval by that many 0.01 BPM steps
/// (a decision: the switch events 81 / 82 it sends were not followed).
pub fn grid_tick(app: &mut App, hub: &Arc<Hub>) {
    if app.midi.more.grid_at.map(|t| t.elapsed() < Duration::from_millis(100)).unwrap_or(false) {
        return;
    }
    app.midi.more.grid_at = Some(Instant::now());
    for n in 0..4 {
        grid_flush(app, hub, n);
    }
}

fn grid_flush(app: &mut App, hub: &Arc<Hub>, n: usize) {
    let mode = hub.grid[n].load(Relaxed);
    if mode == 0 {
        return;
    }
    let ms = {
        let mut acc = hub.grid_acc.lock();
        let whole = acc[n].trunc();
        acc[n] -= whole;
        whole
    };
    if ms == 0.0 || n as u8 >= app.deck_count() {
        return;
    }
    if mode == 2 {
        app.with_deck(n as u8, |a| a.grid_shift(ms / 1000.0));
    } else {
        app.with_deck(n as u8, |a| a.grid_scale_bpm(|v| ((v - ms * 0.01) * 100.0).round() / 100.0));
    }
}

/// The light of a function of this file for deck `n` (None: it lights while held).
pub fn led(app: &App, hub: &Hub, device: &str, m: M, n: u8) -> Option<Led> {
    let on = |b: bool| Some(if b { Led::On } else { Led::Off });
    let ni = (n as usize).min(3);
    let d = app.deck_of(n);
    match m {
        M::MasterCue => on(app.midi.more.master_cue),
        M::CfxOn => on(app.px.cfx_user),
        M::CfxButton(i) => on(app.px.cfx_user && cfx_button_kind(hub, device, i) == Some(app.px.cfx_kind)),
        M::MfxOn(u) => on(app.px.fxx.mfx.units[(u as usize).min(1)].on),
        M::MfxAssign(u, dk) => on(app.prefs.fx.mfx.upper[(u as usize).min(1)] == (dk >= 2)),
        M::DrumMulti(multi) => on(app.gc.set.decks[ni].multi == multi),
        M::DrumBank(b) => on(app.gc.set.decks[ni].bank == b),
        M::DrumSlot(s) => {
            let idx = app.gc.set.decks[ni].bank as usize * 4 + s as usize;
            on(app.groove_is_played(ni, idx))
        }
        M::DrumCapture => on(app.gc.capture_on == Some(n)),
        M::Grid(slide) => on(hub.grid[ni].load(Relaxed) == if slide { 2 } else { 1 }),
        M::EditorMode(slot) => on(app.midi.editor_slot[ni] == slot && editor_pad_mode(&editor_function(app, slot as usize)) == Some(crate::pads::PadMode::from_code(app.px.pad_mode[ni]))),
        // (`PadEditorData::sendPermission` 0x1020f6c38: whether the mode's page can change;
        // lit for the modes with pages.)
        M::Permission(slot) => on(editor_page_name(&editor_function(app, slot as usize), true).is_some()),
        M::EditorPad(slot, pad, false) => {
            let f = editor_function(app, slot as usize);
            match f.as_str() {
                "HotCue" => match d.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == pad as i32) {
                    Some(c) => Some(Led::Value(crate::midi::hot_cue_led(c.color, pad as i32, c.end.is_some(), app.prefs.pads.hot_cue_color))),
                    None => Some(Led::Off),
                },
                _ => on(d.audio.is_some()),
            }
        }
        M::Censor(_) | M::KeyShiftPad(_) | M::KeyboardPad(_) => on(d.audio.is_some()),
        M::ScratchPad(i, _) => {
            let idx = app.prefs.sampler.bank.min(3) as usize * 16 + i as usize;
            on(app.prefs.sampler.slots.get(idx).cloned().flatten().map(|s| s.track > 0).unwrap_or(false))
        }
        M::HotCueSampler(i, _) => on(d.cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == i as i32) || app.sampler_loaded(app.prefs.sampler.bank.min(3) as usize * 16 + i as usize)),
        M::Preview(false) => on(app.preview_status().is_some()),
        M::LoopBox(manual) => on(app.px.loop_manual[ni] == manual),
        // (The list's position and the sort key, sent back as values.)
        M::ScrollBar => {
            let len = app.list.len();
            let at = app.anchor.unwrap_or(0);
            Some(Led::Value(if len > 1 { (at as f64 * 127.0 / (len - 1) as f64).round().min(127.0) as u8 } else { 0 }))
        }
        M::SortType => Some(Led::Value([Col::Bpm, Col::Title, Col::TrackNumber, Col::Artist, Col::Key].iter().position(|c| *c == app.prefs.sort_col).unwrap_or(0) as u8)),
        M::BankMode => on(app.midi.more.bank_mode[ni]),
        M::BankPad(b) => on(app.prefs.sampler.bank == b),
        M::FxMomentary(u, s, _) => {
            let p = &app.px.fx[(u as usize).min(1)].params;
            on(if p.single { s == 0 && p.one.on } else { p.slots[(s as usize).min(2)].on })
        }
        M::CaptureOk => on(d.audio.is_some() && (app.player.decks[ni].try_lock().map(|s| s.loop_range.is_some()).unwrap_or(false) || app.pads.deck[ni].slicer.active)),
        M::Out(o) => match o {
            Out::FxBeat(u) => {
                let fx = &app.px.fx[(u as usize).min(1)];
                Some(Led::Value(if fx.params.single { fx.one_idx } else { fx.idx[0] }.min(127) as u8))
            }
            Out::ReleaseOn(u) => on(app.px.fx[(u as usize).min(1)].params.release.is_some()),
            Out::ReleaseType(u, k) => on(app.px.fx[(u as usize).min(1)].release_kind == k),
            Out::MfxIllumi(u, p) => {
                let u = (u as usize).min(1);
                if p == 0 {
                    on(app.px.fxx.mfx.units[u].on)
                } else {
                    on(app.prefs.fx.mfx.select[u] == p - 1)
                }
            }
            Out::PartLayout => on(app.prefs.stem_parts == 3),
        },
        _ => None,
    }
}
