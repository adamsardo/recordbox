//! `PREPDECK_SHOT_PLAYER=midirig` (with `--screenshot`, `PREPDECK_SHOT_VIEW=performance` and
//! `PREPDECK_BENCH=<frames>` to keep the window up): a dress rehearsal of the owner's DDJ-S1
//! rig before a real controller is plugged in.
//!
//! Three CoreMIDI virtual port pairs are made under the exact names of the rig: "PIONEER
//! DDJ-S1" (the controller itself), "DDJ-S1 Jog Helper" and "PIONEER DDJ-ERGO" (the two ports
//! of the owner's helper program). The helper's real ports may exist at the same time under
//! the same names, so the hub is told to open only these ports, by CoreMIDI's unique id
//! (`Hub::only_ids`): nothing is read from or sent to the real helper or controller. The
//! mappings are the owner's own files, read where recordbox reads them (rekordbox's folder,
//! read only; recordbox's own folder is the throwaway data folder).
//!
//! Every input row of the three mappings is sent with its own code(s), as the hardware and
//! the helper send them (the S1's 7-bit knobs, its 14-bit tempo fader, its relative encoders,
//! the helper's jog in each mode), and what changed in the app is compared with what rekordbox
//! does for that function. Then the lights: states are set and the bytes that reach the
//! port are compared. Then what only shows with hardware: start-up with the ports there,
//! unplug and replug, the helper restarting, a jog flood while both decks play, the window
//! minimised. One line per check on stderr ("midirig:") and a Markdown report
//! (`PREPDECK_MIDI_RIG_OUT`, default `<data folder>/midirig.md`); the app quits when done.
//!
//! Nothing here runs unless the switch is given.

use crate::app::*;
use crate::midi_csv::Kind;
use crate::model::*;
use midir::os::unix::{VirtualInput, VirtualOutput};
use parking_lot::Mutex;
use std::fmt::Write as _;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The S1's own jog, touch and FX CH SELECT messages, sent on the S1's port.
const RAW: &str = "(S1 raw jog / touch / FX CH SELECT, unmapped)";
pub const PORTS: [&str; 3] = ["PIONEER DDJ-S1", "DDJ-S1 Jog Helper", "PIONEER DDJ-ERGO"];
/// The stand-in ports' own names: the owner's helper program takes any port whose name holds
/// "DDJ-S1" as its controller, and rekordbox, if it starts meanwhile, opens a port named as
/// a controller it knows. The hub takes each as the device of the rig's name (`Hub::alias`).
/// (Fixed names: rekordbox, when it runs, writes a header-only mapping file into its own folder
/// for every new port name it sees, so a name per run would leave a file per run there.)
pub const PORT_NAMES: [&str; 3] = ["rbxtest-midi4 S1", "rbxtest-midi4 helper", "rbxtest-midi4 ERGO"];
const OTHER_NAME: &str = "rbxtest-midi4 other";
const S1: usize = 0;
const HELPER: usize = 1;
const ERGO: usize = 2;

pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "midirig")).unwrap_or(false))
}

// ---------------------------------------------------------------------- the virtual ports

/// One port of the rig: a source recordbox reads and a destination it writes its lights to.
pub struct VPort {
    pub name: String,
    src: Arc<Mutex<Option<midir::MidiOutputConnection>>>,
    dst: Option<midir::MidiInputConnection<()>>,
    pub src_id: String,
    pub dst_id: String,
}

/// What reached the rig's destinations: (port, ms since the rig began, message).
type Got = Arc<Mutex<Vec<(usize, f64, [u8; 3])>>>;

fn source_ids(name: &str) -> Vec<String> {
    let Ok(i) = midir::MidiInput::new("recordbox-rig-list") else { return vec![] };
    i.ports().iter().filter(|p| i.port_name(p).map(|n| n == name).unwrap_or(false)).map(|p| p.id()).collect()
}

fn dest_ids(name: &str) -> Vec<String> {
    let Ok(o) = midir::MidiOutput::new("recordbox-rig-list") else { return vec![] };
    o.ports().iter().filter(|p| o.port_name(p).map(|n| n == name).unwrap_or(false)).map(|p| p.id()).collect()
}

fn make_port(idx: usize, name: &str, got: Got, t0: Instant) -> Option<VPort> {
    let (bs, bd) = (source_ids(name), dest_ids(name));
    let src = midir::MidiOutput::new("recordbox-rig").ok()?.create_virtual(name).ok()?;
    let dst = midir::MidiInput::new("recordbox-rig").ok()?.create_virtual(
        name,
        move |_, m, _| {
            if m.len() == 3 {
                got.lock().push((idx, t0.elapsed().as_secs_f64() * 1000.0, [m[0], m[1], m[2]]));
            }
        },
        (),
    ).ok()?;
    let mut src_id = None;
    let mut dst_id = None;
    for _ in 0..50 {
        src_id = source_ids(name).into_iter().find(|i| !bs.contains(i));
        dst_id = dest_ids(name).into_iter().find(|i| !bd.contains(i));
        if src_id.is_some() && dst_id.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Some(VPort { name: name.into(), src: Arc::new(Mutex::new(Some(src))), dst: Some(dst), src_id: src_id?, dst_id: dst_id? })
}

/// The unique ids of the rig's ports, for the hub (`midi::start`). None until they exist.
pub fn ids() -> Option<Vec<String>> {
    RIG.lock().as_ref().map(|r| r.ports.iter().flat_map(|p| [p.src_id.clone(), p.dst_id.clone()]).collect())
}

// ---------------------------------------------------------------------- state snapshots

#[derive(Clone, Debug, Default)]
struct DS {
    track: Option<i64>,
    loaded: bool,
    playing: bool,
    pos: f64,
    dur: f64,
    cue: f64,
    lp: Option<(f64, f64)>,
    loop_in: Option<f64>,
    tempo_adj: f64,
    range: u8,
    bend: f64,
    e_bend: f64,
    search: f64,
    reverse: bool,
    slip: bool,
    sync: bool,
    master: bool,
    mt: bool,
    ks: bool,
    mem: Vec<(f64, Option<f64>, bool)>,
    hot: [Option<f64>; 8],
    vinyl: bool,
    jog: String,
    trim: f32,
    eq: [f32; 3],
    fader: f32,
    hp: bool,
    e_eq: [f32; 3],
}

#[derive(Clone, Debug, Default)]
struct FxS {
    single: bool,
    kind: String,
    on: bool,
    level: f32,
    idx: usize,
    slot_kind: String,
    slot_on: bool,
    slot_level: f32,
    slot_idx: [usize; 3],
    assign: Vec<bool>,
    auto: bool,
    tap_bpm: f32,
}

#[derive(Clone, Debug, Default)]
struct St {
    d: [DS; 2],
    xf: f32,
    fx: [FxS; 2],
    panels: (bool, bool, bool, bool),
    browse_layout: bool,
    focus_tree: bool,
    view: String,
    anchor: Option<usize>,
    tags: usize,
    brake: [u8; 2],
    start: [u8; 2],
    toast: Option<String>,
}

fn snap(app: &App) -> St {
    let hub = app.midi.hub.clone();
    let mut d: [DS; 2] = Default::default();
    for n in 0..2u8 {
        if n >= app.deck_count() {
            continue;
        }
        let dk = app.deck_of(n);
        let s = app.player.decks[n as usize].lock();
        let sr = s.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
        let mut hot = [None; 8];
        for c in dk.cues.iter().filter(|c| c.kind == CueKind::Hot && (0..8).contains(&c.slot)) {
            hot[c.slot as usize] = Some(c.start);
        }
        let m = app.px.mix[n as usize];
        d[n as usize] = DS {
            track: dk.track,
            loaded: s.audio.is_some(),
            playing: s.playing,
            pos: s.pos / sr,
            dur: dk.duration,
            cue: dk.cue_point,
            lp: s.loop_range,
            loop_in: dk.loop_in,
            tempo_adj: dk.x.tempo_adj,
            range: dk.x.tempo_range,
            bend: dk.x.bend as f64,
            e_bend: s.bend,
            search: s.search,
            reverse: s.dk.reverse,
            slip: dk.x.slip,
            sync: dk.x.beat_sync,
            master: app.px.master == n,
            mt: dk.x.master_tempo,
            ks: dk.x.key_sync,
            mem: dk.cues.iter().filter(|c| c.kind == CueKind::Memory).map(|c| (c.start, c.end, c.active_loop)).collect(),
            hot,
            vinyl: hub.as_ref().map(|h| h.vinyl[n as usize].load(Relaxed)).unwrap_or(true),
            jog: format!("{:?}", s.dk.jog),
            trim: m.trim,
            eq: m.eq,
            fader: m.fader,
            hp: m.cue,
            e_eq: s.eq_pos,
        };
    }
    let fx: [FxS; 2] = std::array::from_fn(|u| {
        let f = &app.px.fx[u];
        let p = &f.params;
        FxS {
            single: p.single,
            kind: format!("{:?}", p.one.kind),
            on: p.one.on,
            level: p.one.level,
            idx: f.one_idx,
            slot_kind: format!("{:?}", p.slots[0].kind),
            slot_on: p.slots[0].on,
            slot_level: p.slots[0].level,
            slot_idx: f.idx,
            assign: p.assign.to_vec(),
            auto: f.auto,
            tap_bpm: f.tap_bpm,
        }
    });
    St {
        d,
        xf: app.px.crossfader,
        fx,
        panels: (app.prefs.perf_fx, app.prefs.perf_sampler, crate::ui_perf_layouts::mixer_shown(app), app.prefs.rec.panel),
        browse_layout: app.prefs.perf_lay.browse,
        focus_tree: app.dialogs.ext.focus_tree,
        view: format!("{:?}", app.view),
        anchor: app.anchor,
        tags: app.ext.tag_list.len(),
        brake: app.prefs.rb.brake,
        start: app.prefs.rb.start,
        toast: app.toasts.last().map(|t| t.text.clone()),
    }
}

// ---------------------------------------------------------------------- cases

/// One message the rig sends, and how long to wait after it (ms) when sent in a burst.
#[derive(Clone, Copy, Debug)]
struct Msg {
    m: [u8; 3],
    gap_ms: u64,
}

fn msg(st: u8, d1: u8, d2: u8) -> Msg {
    Msg { m: [st, d1, d2], gap_ms: 0 }
}

/// One input check: a code of a mapping row.
#[derive(Clone, Debug)]
struct Case {
    port: usize,
    func: String,
    kind: Kind,
    code: u16,
    /// Mapping deck (0: none).
    deck: u8,
    /// A variant of the same row (the other direction of an encoder, a long press).
    variant: &'static str,
}

impl Case {
    fn n(&self) -> usize {
        if self.deck == 0 { 0 } else { (self.deck as usize - 1).min(1) }
    }
    fn st(&self) -> u8 {
        (self.code >> 8) as u8
    }
    fn d1(&self) -> u8 {
        (self.code & 0xFF) as u8
    }
}

struct Result_ {
    port: usize,
    code: String,
    func: String,
    deck: u8,
    sent: String,
    changed: String,
    pass: Option<bool>,
    note: String,
}

/// The messages for a case, as the hardware (or the helper) sends them.
fn plan(c: &Case) -> Vec<Msg> {
    let (st, d1) = (c.st(), c.d1());
    if c.func == RAW {
        // What the S1 itself sends for its jogs, touches and FX CH SELECT (the helper reads
        // these too and passes them on its own ports): not in the S1's mapping.
        let mut v = vec![];
        for ch in 0..2u8 {
            for (msb, lsb) in [(0x01, 0x21), (0x02, 0x22), (0x06, 0x26), (0x07, 0x27)] {
                v.push(msg(0xB0 | ch, msb, 0x00));
                v.push(msg(0xB0 | ch, lsb, 0x10));
            }
            v.push(msg(0x90 | ch, 0x36, 0x7F));
            v.push(msg(0x90 | ch, 0x36, 0x00));
            v.push(msg(0x90 | ch, 0x68, 0x7F));
            v.push(msg(0x90 | ch, 0x68, 0x00));
        }
        v.push(msg(0xB2, 0x00, 0x01));
        v.push(msg(0xB3, 0x00, 0x02));
        return v;
    }
    let press = |v: u8| vec![msg(st, d1, v), msg(st, d1, 0)];
    match (c.func.as_str(), c.kind) {
        (_, Kind::Button | Kind::Pad(_)) if c.variant == "long" => vec![msg(st, d1, 0x7F)],
        (_, Kind::Button | Kind::Pad(_)) => press(0x7F),
        ("NeedleSearch", _) => vec![msg(st, d1, 0x40), msg(st, d1, 0x00)],
        (_, Kind::KnobSlider) if c.func.starts_with("VinylAdjust") => vec![msg(st, d1, 0x40)],
        (_, Kind::KnobSlider) => vec![msg(st, d1, 0x20)],
        (_, Kind::KnobSliderHiRes) => {
            // The S1's tempo fader: its centre (8191, 3F 7F) to pick the deck up, then a
            // quarter of the way up (12288 = 60 00).
            vec![msg(st, d1, 0x3F), msg(st, d1 + 0x20, 0x7F), msg(st, d1, 0x60), msg(st, d1 + 0x20, 0x00)]
        }
        (_, Kind::Rotary) if c.variant == "minus" => vec![msg(st, d1, 0x7F)],
        (_, Kind::Rotary) if c.variant == "fast" => vec![msg(st, d1, 0x04)],
        (_, Kind::Rotary) => vec![msg(st, d1, 0x01)],
        (_, Kind::JogTouch) => press(0x7F),
        (_, Kind::JogRotate | Kind::Difference) => {
            // 30 counts of +5 every 8 ms (the helper's 7-bit form, 0x40 + delta).
            // (+10 counts every 8 ms: the platter at about its nominal speed; slower turns do
            // not bend at all, `|speed| > 0.5`.)
            (0..30).map(|_| Msg { m: [st, d1, 0x4A], gap_ms: 8 }).collect()
        }
        _ => press(0x7F),
    }
}

/// The deck put in a known state before a case: loaded, paused at 20 s, no loop, no reverse,
/// no sync. Returns frames to wait.
fn base(app: &mut App, n: u8) {
    // (Vinyl Speed Adjust back to none: the S1's TOUCH/RELEASE and BRAKE rows set it, and a
    // long brake keeps a paused deck moving for seconds.)
    // (`PREPDECK_MIDI_RIG_NO_RESET=1` leaves it, to show what it does to the cases after.)
    if (app.prefs.rb.brake != [0; 2] || app.prefs.rb.start != [0; 2]) && std::env::var_os("PREPDECK_MIDI_RIG_NO_RESET").is_none() {
        app.prefs.rb.brake = [0; 2];
        app.prefs.rb.start = [0; 2];
        app.prefs.deck.vinyl_brake = [0; 2];
        app.prefs.deck.vinyl_start = [0; 2];
        app.push_deck_controls();
    }
    // (Q as it was when the run began: the "q" cases switch it on.)
    let q = Q_START.load(Relaxed);
    if q < 4 {
        for d in 0..2u8 {
            app.with_deck(d, |a| a.prefs.quantize = q & 1 == 1);
        }
        app.prefs.rb.q_reverse.on = q & 2 == 2;
    }
    // (The other deck stands too: a playing deck takes MASTER over from a standing one.)
    play(app, 1 - n.min(1), false);
    app.with_deck(n, |a| {
        if a.playing() {
            a.play_pause();
        }
        if a.loop_active().is_some() {
            a.loop_exit();
        }
        a.deck.loop_in = None;
        if a.player.state.lock().dk.reverse {
            a.reverse_off();
        }
        if a.deck.x.beat_sync {
            a.toggle_beat_sync();
        }
        if a.deck.x.slip {
            a.toggle_slip();
        }
        a.seek(20.0);
        a.deck.cue_point = 10.0;
    });
}

fn play(app: &mut App, n: u8, on: bool) {
    app.with_deck(n, |a| {
        if a.playing() != on {
            a.play_pause();
        }
    });
}

fn prep(app: &mut App, c: &Case) {
    let n = c.n() as u8;
    let o = 1 - n;
    match c.func.as_str() {
        // Browse and FX rows act on the app as a whole.
        "Browse" | "Forward" | "Back" | "Area" | "AddToTagList" | "RelatedTracks" | "PanelForward" | "SwitchActiveWindow" | "FxPanel" => {
            if c.func == "RelatedTracks" && app.view != View::Collection {
                app.set_view(View::Collection);
            }
            if matches!(c.func.as_str(), "Browse" | "AddToTagList") {
                app.dialogs.ext.focus_tree = false;
                if app.list_dirty {
                    app.rebuild_list();
                }
                if !app.list.is_empty() {
                    app.set_anchor(3.min(app.list.len() - 1));
                    app.selected.clear();
                    app.selected.insert(app.list[3.min(app.list.len() - 1)]);
                }
                if c.func == "AddToTagList" {
                    app.ext.tag_list.clear();
                }
            }
            if c.func == "Forward" || c.func == "Back" {
                // Forward from the tree (Collection: no folder to open), Back from the list.
                app.dialogs.ext.focus_tree = c.func == "Forward";
                if c.func == "Forward" && app.view != View::Collection {
                    app.set_view(View::Collection);
                }
            }
            return;
        }
        _ => {}
    }
    if c.func.starts_with("FX") {
        return;
    }
    if c.func == RAW {
        base(app, 1);
    }
    base(app, n);
    match c.func.as_str() {
        "PlayPause" => {}
        "Cue" => {
            app.with_deck(n, |a| a.deck.cue_point = 10.0);
            play(app, n, true);
        }
        "Sync" => {
            app.with_deck(o, |a| a.set_master());
        }
        "Master" => {
            app.with_deck(o, |a| a.set_master());
        }
        "TempoReset" => {
            app.with_deck(n, |a| {
                a.deck.x.tempo_adj = 0.05;
                a.push_deck_controls();
            });
        }
        "TempoSlider" => {
            app.with_deck(n, |a| {
                a.deck.x.tempo_adj = 0.0;
                a.push_deck_controls();
            });
            if let Some(h) = app.midi.hub.clone() {
                h.arm_takeover(n as usize);
            }
        }
        "SlipReverse" | "Reverse" | "PitchBendUp" | "PitchBendDown" | "WheelPitchBend" | "JogPitchBend" | "JogScratch" | "JogTouch" => {
            if c.variant == "q" {
                app.with_deck(n, |a| a.prefs.quantize = true);
                app.prefs.rb.q_reverse.on = true;
                // (Just past a beat: the turn waits for nearly a whole beat.)
                app.with_deck(n, |a| {
                    if let Some(t) = a.deck.beats.iter().map(|b| b.time).find(|t| *t >= 20.0) {
                        a.seek(t + 0.03);
                    }
                });
            }
            play(app, n, true);
            if let Some(h) = app.midi.hub.clone() {
                h.vinyl[n as usize].store(true, Relaxed);
            }
        }
        "LoopOut" => {
            play(app, n, true);
            app.with_deck(n, |a| a.deck.loop_in = Some(a.pos() - 2.0));
        }
        "LoopIn" | "AutoLoop" => play(app, n, true),
        "ReloopExit" | "LoopHalf" | "LoopDouble" | "LoopHalfDouble" | "LoopMove" | "ReTrigger" => {
            play(app, n, true);
            app.with_deck(n, |a| a.auto_loop(4.0));
            if c.func == "ReTrigger" {
                // (Half way into the loop.)
                let l = app.with_deck(n, |a| a.loop_active());
                if let Some((s, e)) = l {
                    app.with_deck(n, |a| a.seek((s + e) / 2.0));
                }
            }
        }
        "CallNext" | "CallPrev" | "ActiveLoop" | "Delete" | "Memory" => {
            // Memory cues at 10, 30 and 50 s (a loop at 30); the deck stands at 20 s.
            app.with_deck(n, |a| {
                let ids: Vec<i64> = a.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).map(|c| c.id).collect();
                for id in ids {
                    a.delete_cue(id);
                }
                for t in [10.0, 50.0] {
                    a.deck.cue_point = t;
                    a.seek(t);
                    a.memory_set();
                }
                a.seek(30.0);
                a.deck.cue_point = 30.0;
                let b = a.beat_len();
                a.player.state.lock().loop_range = Some((30.0, 30.0 + 4.0 * b));
                a.memory_set();
                a.loop_exit();
                a.seek(20.0);
                a.deck.cue_point = 20.0;
                if c.func == "ActiveLoop" || c.func == "Delete" {
                    a.deck.cue_point = 30.0;
                    a.seek(30.0);
                }
                if c.func == "Memory" {
                    a.deck.cue_point = 40.0;
                    a.seek(40.0);
                }
            });
        }
        f if f.starts_with("PAD") && f.ends_with("_HotCue") => {
            let slot = f[3..4].parse::<i32>().unwrap_or(1) - 1;
            app.with_deck(n, |a| {
                if let Some(id) = a.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).map(|c| c.id) {
                    a.delete_cue(id);
                }
                a.seek(20.0 + slot as f64 * 4.0);
            });
        }
        f if f.starts_with("PAD") && f.ends_with("+Shift") => {
            let slot = f[3..4].parse::<i32>().unwrap_or(1) - 1;
            app.with_deck(n, |a| {
                if !a.deck.cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == slot) {
                    a.seek(60.0 + slot as f64 * 4.0);
                    a.hot_cue_down(slot);
                    a.hot_cue_up();
                }
                if a.playing() {
                    a.play_pause();
                }
            });
        }
        "Load" => {
            if app.list_dirty {
                app.rebuild_list();
            }
            let cur = app.deck_of(n).track;
            let pick = app.list.iter().position(|id| Some(*id) != cur && app.track(*id).map(|t| !t.missing).unwrap_or(false));
            if let Some(i) = pick {
                app.set_anchor(i);
                app.selected.clear();
                app.selected.insert(app.list[i]);
            }
        }
        "InstDoubles" => {
            app.with_deck(o, |a| a.seek(42.0));
            play(app, o, true);
        }
        "SearchFwd" | "SearchRev" => {}
        _ => {}
    }
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// What changed, and whether it is what rekordbox does for the function.
fn check(app: &App, c: &Case, b: &St, m: &St, a: &St) -> (Option<bool>, String) {
    let n = c.n();
    let (db, dm, da) = (&b.d[n], &m.d[n], &a.d[n]);
    let yes = |ok: bool, s: String| (Some(ok), s);
    match c.func.as_str() {
        RAW => {
            let same = |x: &DS, y: &DS| near(x.pos, y.pos, 0.001) && x.e_bend == y.e_bend && x.jog == y.jog && x.playing == y.playing;
            let ok = same(&b.d[0], &a.d[0]) && same(&b.d[1], &a.d[1]) && b.fx[0].assign == a.fx[0].assign && b.fx[1].assign == a.fx[1].assign;
            yes(ok, format!("decks 1 and 2 unchanged {ok} (pos {:.3}/{:.3} -> {:.3}/{:.3}, jog {} / {}), FX assign unchanged", b.d[0].pos, b.d[1].pos, a.d[0].pos, a.d[1].pos, a.d[0].jog, a.d[1].jog))
        }
        "PlayPause" => yes(!db.playing && da.playing, format!("deck {} playing {} -> {}", n + 1, db.playing, da.playing)),
        "Cue" => yes(db.playing && !da.playing && near(da.pos, 10.0, 0.05), format!("playing at {:.2} s -> paused {} at {:.2} s (cue 10.00)", db.pos, !da.playing, da.pos)),
        "JumpToTrackStart" => yes(da.pos < 0.05, format!("pos {:.2} -> {:.2} s", db.pos, da.pos)),
        "Sync" => yes(!db.sync && da.sync, format!("BEAT SYNC {} -> {}", db.sync, da.sync)),
        "Master" => yes(!db.master && da.master, format!("MASTER {} -> {}", db.master, da.master)),
        "MasterTempo" => yes(db.mt != da.mt, format!("MASTER TEMPO {} -> {}", db.mt, da.mt)),
        "KeySync" => yes(db.ks != da.ks, format!("KEY SYNC {} -> {}", db.ks, da.ks)),
        "TempoRange" => yes(da.range == (db.range + 1) % 4, format!("tempo range index {} -> {}", db.range, da.range)),
        "TempoReset" => yes(near(db.tempo_adj, 0.05, 1e-9) && da.tempo_adj.abs() < 1e-9, format!("tempo {:+.2} % -> {:+.2} %", db.tempo_adj * 100.0, da.tempo_adj * 100.0)),
        "TempoSlider" => {
            // 12288 is 4095 above the centre of 8192: +range x 4095 / 8191.
            let range = crate::player_ext::TEMPO_RANGES[(da.range as usize + 1) % 4] / 100.0;
            let want = 4096.0 * range / 8191.0;
            yes(near(da.tempo_adj, want, 1e-4), format!("tempo {:+.3} % -> {:+.3} % (fader 8191 then 12288 = centre + 4096; range +-{:.0} %: wants {:+.3} %)", db.tempo_adj * 100.0, da.tempo_adj * 100.0, range * 100.0, want * 100.0))
        }
        "Vinyl" => yes(db.vinyl != da.vinyl, format!("VINYL (jog mode) {} -> {}", db.vinyl, da.vinyl)),
        "Slip" => yes(db.slip != da.slip, format!("SLIP {} -> {}", db.slip, da.slip)),
        "SlipReverse" => yes(!db.reverse && dm.reverse && !da.reverse, format!("reverse while held: {} -> {} -> released {}", db.reverse, dm.reverse, da.reverse)),
        "Reverse" => yes(!db.reverse && da.reverse, format!("REVERSE {} -> {} (64 ms after the press: {}){}", db.reverse, da.reverse, dm.reverse, if c.variant == "q" { format!(" (Q on, REVERSE quantize {}: on the next REVERSE beat; deck at {:.3} s, its beat {:.3} s)", crate::prefs_rb::Q_BEATS.get(app.prefs.rb.q_reverse.beat.saturating_sub(1) as usize).copied().unwrap_or("?"), dm.pos, { let d = app.deck_of(n as u8); let b = crate::player_ext::bpm_at(&d.beats, d.cue_point); if b > 0.0 { 60.0 / b } else { 0.0 } }) } else { String::new() })),
        "Load" => {
            let want = app.anchor.and_then(|i| app.list.get(i).copied());
            yes(da.track.is_some() && da.track != db.track && da.track == want && da.loaded, format!("deck {} track {:?} -> {:?} (selected row {:?}), loaded {}", n + 1, db.track, da.track, want, da.loaded))
        }
        "InstDoubles" => {
            let o = 1 - n;
            yes(da.track == a.d[o].track && da.loaded && near(da.pos, a.d[o].pos, 0.5), format!("deck {} track {:?} -> {:?} (deck {} has {:?}), pos {:.2} vs {:.2} s", n + 1, db.track, da.track, o + 1, a.d[o].track, da.pos, a.d[o].pos))
        }
        "LoopIn" => yes(db.loop_in.is_none() && da.loop_in.is_some(), format!("loop in point {:?} -> {:?}", db.loop_in.map(|x| format!("{x:.2}")), da.loop_in.map(|x| format!("{x:.2}")))),
        "LoopOut" => yes(db.lp.is_none() && da.lp.is_some(), format!("loop {:?} -> {:?}", db.lp, da.lp)),
        "ReloopExit" => yes(db.lp.is_some() && da.lp.is_none(), format!("loop {:?} -> {:?}", db.lp.is_some(), da.lp.is_some())),
        "AutoLoop" => {
            let bl = { let d = app.deck_of(n as u8); let bpm = crate::player_ext::bpm_at(&d.beats, d.cue_point); if bpm > 0.0 { 60.0 / bpm } else { 0.5 } };
            let beats = da.lp.map(|(s, e)| (e - s) / bl);
            yes(db.lp.is_none() && da.lp.is_some(), format!("loop none -> {:?} ({:.2} beats)", da.lp.map(|(s, e)| (format!("{s:.2}"), format!("{e:.2}"))), beats.unwrap_or(0.0)))
        }
        "LoopHalf" | "LoopDouble" | "LoopHalfDouble" => {
            let len = |d: &DS| d.lp.map(|(s, e)| e - s).unwrap_or(0.0);
            let want = if c.func == "LoopHalf" || c.variant == "minus" { 0.5 } else { 2.0 };
            yes(len(db) > 0.0 && near(len(da) / len(db), want, 0.02), format!("loop length {:.3} -> {:.3} s (x{want})", len(db), len(da)))
        }
        "LoopMove" => {
            let s = |d: &DS| d.lp.map(|x| x.0).unwrap_or(-1.0);
            let ok = if c.variant == "minus" { s(da) < s(db) } else { s(da) > s(db) };
            yes(db.lp.is_some() && ok, format!("loop start {:.3} -> {:.3} s", s(db), s(da)))
        }
        "ReTrigger" => {
            // (Measured 64 ms after the press: the deck plays on from the loop start.)
            let s = db.lp.map(|x| x.0).unwrap_or(-1.0);
            yes(da.lp.is_some() && dm.pos >= s - 0.01 && dm.pos < s + 0.12, format!("pos {:.3} -> {:.3} s 64 ms after the press (loop start {s:.3})", db.pos, dm.pos))
        }
        "CallNext" => yes(near(da.cue, 30.0, 0.01), format!("cue point {:.2} -> {:.2} s (next memory cue 30.00)", db.cue, da.cue)),
        "CallPrev" => yes(near(da.cue, 10.0, 0.01), format!("cue point {:.2} -> {:.2} s (previous memory cue 10.00)", db.cue, da.cue)),
        "ActiveLoop" => {
            let act = |d: &DS| d.mem.iter().find(|x| near(x.0, 30.0, 0.01)).map(|x| x.2);
            yes(act(db).is_some() && act(db) != act(da), format!("memory loop at 30 s active {:?} -> {:?}", act(db), act(da)))
        }
        "Delete" => yes(da.mem.len() + 1 == db.mem.len(), format!("memory cues {} -> {} (the one at 30 s)", db.mem.len(), da.mem.len())),
        "Memory" => yes(da.mem.len() == db.mem.len() + 1 && da.mem.iter().any(|x| near(x.0, 40.0, 0.05)), format!("memory cues {} -> {} (new at 40 s: {})", db.mem.len(), da.mem.len(), da.mem.iter().any(|x| near(x.0, 40.0, 0.05)))),
        f if f.starts_with("PAD") && f.ends_with("_HotCue") => {
            let slot = f[3..4].parse::<usize>().unwrap_or(1) - 1;
            yes(db.hot[slot].is_none() && da.hot[slot].is_some(), format!("hot cue {} empty -> {:?}", (b'A' + slot as u8) as char, da.hot[slot].map(|x| format!("{x:.2} s"))))
        }
        f if f.starts_with("PAD") && f.ends_with("+Shift") => {
            let slot = f[3..4].parse::<usize>().unwrap_or(1) - 1;
            yes(db.hot[slot].is_some() && da.hot[slot].is_none(), format!("hot cue {} {:?} -> deleted {}", (b'A' + slot as u8) as char, db.hot[slot].map(|x| format!("{x:.2} s")), da.hot[slot].is_none()))
        }
        "NeedleSearch" => {
            // 0x40 is the middle of the strip (8192 of 16383: place 200 of 400), then the
            // finger lifts (0: place 0, the start, as in rekordbox).
            yes(near(dm.pos, da.dur * 0.5, 0.6) && da.pos < 0.05, format!("paused at {:.2}: strip 0x40 -> {:.2} s (half of {:.1}); finger up (0x00) -> {:.2} s", db.pos, dm.pos, da.dur, da.pos))
        }
        "VinylAdjustBrake" | "VinylAdjustTouchRelease" => {
            let side = n;
            let want = crate::midi::vinyl_index(8192);
            let ok = if c.func == "VinylAdjustBrake" { a.brake[side] == want } else { a.brake[side] == want && a.start[side] == want };
            yes(ok, format!("Vinyl Speed Adjust side {}: brake {} -> {}, start {} -> {} (0x40 = 8192: index {want})", side + 1, b.brake[side], a.brake[side], b.start[side], a.start[side]))
        }
        "Gain" => yes(near(da.trim as f64, 0.25, 0.002), format!("TRIM {:.3} -> {:.3} (0x20 = 4096 of 16383: 0.25)", db.trim, da.trim)),
        "EQHigh" | "EQMid" | "EQLow" => {
            let i = match c.func.as_str() { "EQLow" => 0, "EQMid" => 1, _ => 2 };
            yes(near(da.eq[i] as f64, 0.25, 0.002) && near(da.e_eq[i] as f64, 0.25, 0.002), format!("{} knob {:.3} -> {:.3}, engine {:.3}", c.func, db.eq[i], da.eq[i], da.e_eq[i]))
        }
        "ChannelFader" => yes(near(da.fader as f64, 0.25, 0.002), format!("channel fader {:.3} -> {:.3}", db.fader, da.fader)),
        "CrossFader" => yes(near(a.xf as f64, 0.25, 0.002), format!("crossfader {:.3} -> {:.3}", b.xf, a.xf)),
        "HeadphoneCue" => yes(db.hp != da.hp, format!("headphone CUE {} -> {}", db.hp, da.hp)),
        "Browse" => {
            let want = if c.variant == "minus" { -1i64 } else if c.variant == "fast" { 10 } else { 1 };
            let d = a.anchor.map(|x| x as i64).unwrap_or(-99) - b.anchor.map(|x| x as i64).unwrap_or(-99);
            yes(d == want, format!("list row {:?} -> {:?} (moved {d}, wants {want})", b.anchor, a.anchor))
        }
        "Forward" => yes(b.focus_tree && !a.focus_tree || (b.view != a.view), format!("tree focused {} -> {}; view {} -> {}", b.focus_tree, a.focus_tree, b.view, a.view)),
        "Back" => yes(a.focus_tree, format!("tree focused {} -> {}", b.focus_tree, a.focus_tree)),
        "Area" => yes(b.focus_tree != a.focus_tree, format!("tree focused {} -> {}", b.focus_tree, a.focus_tree)),
        "AddToTagList" => yes(a.tags == b.tags + 1, format!("tag list {} -> {} tracks", b.tags, a.tags)),
        "RelatedTracks" => yes(b.view != a.view, format!("view {} -> {}", b.view, a.view)),
        "PanelForward" => yes(b.panels != a.panels, format!("panels (fx, sampler, mixer, rec) {:?} -> {:?}", b.panels, a.panels)),
        "SwitchActiveWindow" => yes(b.browse_layout != a.browse_layout, format!("Browse layout {} -> {}", b.browse_layout, a.browse_layout)),
        "FxPanel" => yes(b.panels.0 != a.panels.0, format!("FX panel {} -> {}", b.panels.0, a.panels.0)),
        f if f.starts_with("FX") => {
            let u = if f.starts_with("FX2") { 1 } else { 0 };
            let (fb, fa) = (&b.fx[u], &a.fx[u]);
            let (lv_b, lv_a) = if fa.single { (fb.level, fa.level) } else { (fb.slot_level, fa.slot_level) };
            if f.ends_with("-1On") {
                let (ob, oa) = if fa.single { (fb.on, fa.on) } else { (fb.slot_on, fa.slot_on) };
                return yes(ob != oa, format!("FX{} effect on {ob} -> {oa}", u + 1));
            }
            if f.ends_with("-1Select") {
                let (kb, ka) = if fa.single { (&fb.kind, &fa.kind) } else { (&fb.slot_kind, &fa.slot_kind) };
                return yes(kb != ka, format!("FX{} effect {kb} -> {ka}", u + 1));
            }
            if f.ends_with("BeatUpDown") {
                let (ib, ia) = if fa.single { (fb.idx as i64, fa.idx as i64) } else { (fb.slot_idx[0] as i64, fa.slot_idx[0] as i64) };
                let want = if c.variant == "minus" { -1 } else { 1 };
                return yes(ia - ib == want || (want < 0 && ib == 0 && ia == 0), format!("FX{} beat index {ib} -> {ia}", u + 1));
            }
            if f.ends_with("TapBeatWithLongPress") {
                if c.variant == "long" {
                    return yes(fa.auto, format!("FX{} held 1.2 s: AUTO {} (TAP before)", u + 1, fa.auto));
                }
                return yes(!fa.auto, format!("FX{} BPM AUTO {} -> {} (a tap)", u + 1, fb.auto, fa.auto));
            }
            if f.contains("Assign.") {
                let ch = match f.split('.').nth(1).unwrap_or("") {
                    "CH1" => 0,
                    "CH2" => 1,
                    "CH3" => 2,
                    "CH4" => 3,
                    "MASTER" => crate::fx::MASTER,
                    _ => 99,
                };
                if ch == 99 {
                    return (Some(false), format!("FX{} assign: recordbox has no MIC / AUX channel; nothing changed ({:?} -> {:?})", u + 1, fb.assign, fa.assign));
                }
                let held = m.fx[u].assign[ch];
                return yes(held && !fa.assign[ch], format!("FX{} assign {}: while the switch is there {held}, after it leaves {}", u + 1, f.split('.').nth(1).unwrap_or(""), fa.assign[ch]));
            }
            if f.ends_with("-1") {
                return yes(near(lv_a as f64, 0.25, 0.002), format!("FX{} LEVEL/DEPTH {lv_b:.3} -> {lv_a:.3}", u + 1));
            }
            (None, "no check written".into())
        }
        "JogScratch" | "WheelPitchBend" | "JogPitchBend" => yes(dm.e_bend > 0.0 && da.e_bend == 0.0, format!("untouched platter, playing: bend while turning {:+.4}, after {:+.4}", dm.e_bend, da.e_bend)),
        "JogTouch" => yes(dm.jog.starts_with("Speed") && da.jog == "None", format!("vinyl on, playing: touched -> jog {} (the record held), let go -> {}", dm.jog, da.jog)),
        "WheelSearch" | "JogSearch" => yes(da.pos > db.pos + 0.5, format!("paused at {:.2} s: 30 x +10 counts -> {:.2} s", db.pos, da.pos)),
        "PitchBendUp" | "PitchBendDown" => {
            let want = if c.func == "PitchBendUp" { 1.0 } else { -1.0 };
            yes(dm.bend == want && da.bend == 0.0 && dm.e_bend * want > 0.0, format!("pitch bend held {} (engine {:+.4}) -> released {}", dm.bend, dm.e_bend, da.bend))
        }
        "SearchFwd" | "SearchRev" => {
            let want = if c.func == "SearchFwd" { 1.0 } else { -1.0 };
            yes(dm.search * want > 0.0 && da.search == 0.0, format!("search held {:+.1} -> released {:+.1}", dm.search, da.search))
        }
        _ => (None, "no check written".into()),
    }
}

// ---------------------------------------------------------------------- the run

#[derive(PartialEq, Clone, Copy, Debug)]
enum Stage {
    Setup,
    Connect,
    Startup,
    Inputs,
    Jogs,
    Outputs,
    Hardware,
    Done,
}

struct Rig {
    ports: Vec<VPort>,
    got: Got,
    t0: Instant,
    stage: Stage,
    cases: Vec<Case>,
    i: usize,
    at: u32,
    sub: u32,
    snaps: Vec<St>,
    results: Vec<Result_>,
    outs: Vec<Result_>,
    hw: Vec<Result_>,
    jog: Vec<Result_>,
    sending: Option<std::thread::JoinHandle<()>>,
    mark: usize,
    report: String,
    frame_ms: Vec<f32>,
    last_frame: Option<Instant>,
    note: String,
    /// Ticks of 16 ms since the stage (or case) began: each runs once, at most one a frame.
    t_stage: Instant,
    vt: u32,
    restarted: bool,
    t_mark: Option<Instant>,
    child: Option<std::process::Child>,
    wait: u32,
}

/// The stage (or the next case) begins now.
fn restart(r: &mut Rig) {
    r.t_stage = Instant::now();
    r.vt = 0;
    r.restarted = true;
}

static RIG: Mutex<Option<Rig>> = Mutex::new(None);

/// `PREPDECK_MIDI_RIG_STAGES=inputs,jogs,outputs,hardware` (default all): the parts to run.
fn want(stage: &str) -> bool {
    std::env::var("PREPDECK_MIDI_RIG_STAGES").map(|v| v.split(',').any(|x| x.trim() == stage)).unwrap_or(true)
}

fn say(s: &str) {
    eprintln!("midirig: {s}");
}

fn send(r: &Rig, port: usize, msgs: &[Msg]) {
    if let Some(c) = r.ports[port].src.lock().as_mut() {
        for m in msgs {
            let _ = c.send(&m.m);
        }
    }
}

/// A burst sent on a thread of its own with its gaps.
fn send_timed(r: &mut Rig, port: usize, msgs: Vec<Msg>) {
    let src = r.ports[port].src.clone();
    r.sending = Some(std::thread::spawn(move || {
        for m in msgs {
            if let Some(c) = src.lock().as_mut() {
                let _ = c.send(&m.m);
            }
            if m.gap_ms > 0 {
                std::thread::sleep(Duration::from_millis(m.gap_ms));
            }
        }
    }));
}

fn hex(c: u16) -> String {
    format!("{c:04X}")
}

/// The input cases: every input code of the three ports' mappings as the hub loaded them.
fn cases(app: &App) -> Vec<Case> {
    let Some(hub) = app.midi.hub.clone() else { return vec![] };
    let mut v = vec![];
    for (pi, name) in PORTS.iter().enumerate() {
        let Some(dev) = hub.device(name) else { continue };
        let map = dev.map.lock();
        let mut ins: Vec<(u16, String, Kind, u8, bool)> = map.live_inputs().map(|i| (i.e.code, i.e.name.clone(), i.e.kind, i.e.deck, i.e.user)).collect();
        // The ERGO's factory mapping: only the rows the helper sends on that port.
        if pi == ERGO {
            ins.retain(|i| matches!(i.0 & 0xF0FF, 0xB021 | 0xB022 | 0xB023 | 0xB027 | 0x9036 | 0x9067) && i.0 & 0x0F00 <= 0x0100);
        }
        ins.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        for (code, func, kind, deck, _) in ins {
            if pi != ERGO && matches!(kind, Kind::JogRotate | Kind::JogTouch | Kind::Difference) {
                // (The helper never sends its port's jog rows; they are sent anyway, below.)
            }
            v.push(Case { port: pi, func: func.clone(), kind, code, deck, variant: "" });
            if kind == Kind::Rotary {
                v.push(Case { port: pi, func: func.clone(), kind, code, deck, variant: "minus" });
                if func == "Browse" {
                    v.push(Case { port: pi, func: func.clone(), kind, code, deck, variant: "fast" });
                }
            }
            if func.ends_with("TapBeatWithLongPress") {
                v.push(Case { port: pi, func, kind, code, deck, variant: "long" });
            } else if func == "Reverse" {
                // (With Q on, rekordbox turns on the next REVERSE beat.)
                v.push(Case { port: pi, func, kind, code, deck, variant: "q" });
            }
        }
    }
    v.push(Case { port: S1, func: RAW.into(), kind: Kind::Button, code: 0xB001, deck: 1, variant: "" });
    v
}

/// Called every frame from the scripted-run hook.
pub fn frame(app: &mut App, frame: u32) {
    if !on() {
        return;
    }
    FRAME.store(frame, Relaxed);
    let mut g = RIG.lock();
    if g.is_none() {
        // Silent from the start: the decks play during the run.
        if !app.px.muted {
            app.toggle_mute();
        }
        let got: Got = Arc::new(Mutex::new(vec![]));
        let t0 = Instant::now();
        let mut ports = vec![];
        for (i, n) in PORT_NAMES.iter().enumerate() {
            match make_port(i, n, got.clone(), t0) {
                Some(p) => ports.push(p),
                None => {
                    say(&format!("could not make the virtual port {n}"));
                    std::process::exit(2);
                }
            }
        }
        for p in &ports {
            say(&format!("virtual port \"{}\": source id {}, destination id {} (real ports of that name: sources {:?}, destinations {:?})", p.name, p.src_id, p.dst_id, source_ids(&p.name).into_iter().filter(|i| *i != p.src_id).collect::<Vec<_>>(), dest_ids(&p.name).into_iter().filter(|i| *i != p.dst_id).collect::<Vec<_>>()));
        }
        *g = Some(Rig { ports, got, t0, stage: Stage::Setup, cases: vec![], i: 0, at: frame, sub: 0, snaps: vec![], results: vec![], outs: vec![], hw: vec![], jog: vec![], sending: None, mark: 0, report: String::new(), frame_ms: vec![], last_frame: None, note: String::new(), t_stage: Instant::now(), vt: 0, restarted: false, t_mark: None, child: None, wait: 0 });
        return;
    }
    let r = g.as_mut().unwrap();
    let now = Instant::now();
    if let Some(l) = r.last_frame.replace(now) {
        r.frame_ms.push(l.elapsed().as_secs_f32() * 1000.0);
    }
    // One tick a frame at most, and not ahead of the clock.
    let due = (r.t_stage.elapsed().as_millis() / 16) as u32;
    if r.vt > due {
        return;
    }
    let since = r.vt;
    r.vt += 1;
    r.restarted = false;
    let frame = since;
    match r.stage {
        Stage::Setup => {
            // The hub opens only the rig's ports (midi::start reads `ids`); once they are open,
            // deck 2 gets a track too.
            if let Some(h) = app.midi.hub.clone() {
                let ids = r.ports.iter().flat_map(|p| [p.src_id.clone(), p.dst_id.clone()]).collect();
                *h.only_ids.lock() = Some(ids);
                let mut al = h.alias.lock();
                for i in 0..3 {
                    al.insert(PORT_NAMES[i].to_string(), PORTS[i].to_string());
                }
            }
            if !app.performance() {
                return;
            }
            Q_START.store(app.prefs.quantize as u8 | (app.prefs.rb.q_reverse.on as u8) << 1, Relaxed);
            // (A known start: no panels, not the Browse layout; earlier runs saved theirs.)
            app.prefs.perf_lay.browse = false;
            app.prefs.perf_fx = false;
            app.prefs.perf_sampler = false;
            app.prefs.rec.panel = false;
            r.stage = Stage::Connect;
            restart(r);
        }
        Stage::Connect => {
            let hub = app.midi.hub.clone();
            let open: Vec<String> = hub.as_ref().map(|h| h.devices.lock().iter().map(|d| d.name.clone()).collect()).unwrap_or_default();
            if since == 1 {
                // Deck 2: the second analysed track.
                let first = app.deck_of(0).track;
                if let Some(id) = app.tracks.iter().find(|t| t.analyzed == 1 && !t.missing && Some(t.id) != first).map(|t| t.id) {
                    app.with_deck(1, |a| a.load_track_here(id));
                }
            }
            let loaded = (0..2u8).all(|n| app.deck_of(n).audio.is_some() && app.deck_of(n).loading.is_none());
            if open.len() == 3 && loaded && since > 30 {
                say(&format!("connected after {since} frames: {open:?}; decks loaded: {:?}", (0..2u8).map(|n| app.deck_of(n).track).collect::<Vec<_>>()));
                for name in PORTS {
                    if let Some(d) = hub.as_ref().and_then(|h| h.device(name)) {
                        say(&format!("{name}: {}", hub.as_ref().unwrap().mapping_summary(&d)));
                    }
                }
                r.stage = Stage::Startup;
                restart(r);
            } else if since > 900 {
                say(&format!("TIMEOUT: open {open:?}, decks loaded {loaded}"));
                r.stage = Stage::Done;
            }
        }
        Stage::Startup => {
            // The lights sent on connect, before anything was touched (rekordbox sends every
            // light's state after `DeviceMidi::enableDevice`).
            if since < 60 {
                return;
            }
            let got = r.got.lock().clone();
            let hub = app.midi.hub.clone().unwrap();
            for (pi, name) in PORTS.iter().enumerate() {
                let Some(d) = hub.device(name) else { continue };
                let outs: Vec<(u16, String, u8)> = d.map.lock().outputs.iter().map(|e| (e.code, e.name.clone(), e.deck)).collect();
                let mut codes: Vec<u16> = outs.iter().map(|o| o.0).collect();
                codes.sort();
                codes.dedup();
                let seen: Vec<u16> = codes.iter().copied().filter(|c| got.iter().any(|g| g.0 == pi && (((g.2[0] as u16) << 8) | g.2[1] as u16) == *c)).collect();
                let missing: Vec<String> = codes.iter().filter(|c| !seen.contains(c)).map(|c| hex(*c)).collect();
                r.hw.push(Result_ { port: pi, code: "-".into(), func: "lights on connect".into(), deck: 0, sent: "(nothing)".into(), changed: format!("{} of {} output codes sent within 1 s of the connect; not sent: {:?}", seen.len(), codes.len(), missing), pass: Some(missing.is_empty()), note: String::new() });
            }
            // The helper's Vinyl light on its ERGO port: 9015 / 9115 lit (vinyl mode on).
            let vin: Vec<String> = got.iter().filter(|g| g.0 == ERGO && g.2[1] == 0x15 && g.2[0] & 0xF0 == 0x90).map(|g| format!("{:02X} {:02X} {:02X}", g.2[0], g.2[1], g.2[2])).collect();
            let ok = vin.iter().any(|v| v.starts_with("90 15 7F")) && vin.iter().any(|v| v.starts_with("91 15 7F"));
            r.hw.push(Result_ { port: ERGO, code: "9015/9115".into(), func: "Vinyl light to the helper on connect".into(), deck: 0, sent: "(nothing)".into(), changed: format!("sent {vin:?}"), pass: Some(ok), note: "the helper scratches only while it has seen 9n 15 7F".into() });
            r.cases = cases(app);
            if let Ok(only) = std::env::var("PREPDECK_MIDI_RIG_ONLY") {
                r.cases.retain(|c| only.split(',').any(|o| c.func.contains(o.trim())));
            }
            say(&format!("{} input cases", r.cases.len()));
            r.stage = Stage::Inputs;
            restart(r);
            r.i = 0;
            r.sub = 0;
        }
        Stage::Inputs => {
            if !want("inputs") {
                r.i = r.cases.len();
            }
            let Some(c) = r.cases.get(r.i).cloned() else {
                r.stage = Stage::Jogs;
                restart(r);
                r.sub = 0;
                return;
            };
            let plan_v = plan(&c);
            let long = c.variant == "long";
            // Frames: 0 prep, 4 before + first message, 8 mid + the rest, 14 after (Load and
            // Instant Doubles wait for the track: 60).
            let jog = matches!(c.kind, Kind::JogRotate | Kind::Difference);
            let settle = if matches!(c.func.as_str(), "Load" | "InstDoubles") { 150 } else if long { 90 } else if jog { 40 } else { 14 };
            match since {
                0 => {
                            prep(app, &c);
                    r.mark = r.got.lock().len();
                }
                4 => {
                    r.snaps = vec![snap(app)];
                    send(r, c.port, &plan_v[..1]);
                }
                8 => {
                    r.snaps.push(snap(app));
                    if plan_v.len() > 1 {
                        if matches!(c.kind, Kind::JogRotate | Kind::Difference) {
                            send_timed(r, c.port, plan_v[1..].to_vec());
                        } else {
                            send(r, c.port, &plan_v[1..]);
                        }
                    }
                }
                16 if jog => {
                    r.snaps[1] = snap(app);
                }
                s if s >= settle => {
                    if long && s == settle {
                        send(r, c.port, &[msg(c.st(), c.d1(), 0)]);
                    }
                    let a = snap(app);
                    let (b, m) = (r.snaps[0].clone(), r.snaps.get(1).cloned().unwrap_or_default());
                    let (pass, changed) = check(app, &c, &b, &m, &a);
                    // Judged on the state, not at a fixed time: a change that comes later (a
                    // quantized REVERSE waits for its beat, a busy machine draws fewer frames)
                    // is waited for, up to 2.4 s more; one that never comes fails then.
                    if pass == Some(false) && s < settle + 150 {
                        return;
                    }
                    let changed = if s > settle { format!("{changed} (after {} ms more)", (s - settle) * 16) } else { changed };
                    let sent = plan_v.iter().map(|m| format!("{:02X} {:02X} {:02X}", m.m[0], m.m[1], m.m[2])).collect::<Vec<_>>();
                    let sent = if sent.len() > 4 { format!("{} x{}", sent[0], sent.len()) } else { sent.join(", ") };
                    let lights: Vec<String> = r.got.lock()[r.mark..].iter().filter(|g| g.0 == c.port).map(|g| format!("{:02X}{:02X}={:02X}", g.2[0], g.2[1], g.2[2])).collect();
                    let note = if lights.is_empty() { String::new() } else { format!("lights sent meanwhile: {}", lights.iter().take(6).cloned().collect::<Vec<_>>().join(" ")) };
                    say(&format!("{} [{}] {} deck {} {}: {} -> {}", match pass { Some(true) => "PASS", Some(false) => "FAIL", None => "----" }, PORTS[c.port], hex(c.code), c.deck, c.func, sent, changed));
                    r.results.push(Result_ { port: c.port, code: hex(c.code), func: format!("{}{}", c.func, if c.variant.is_empty() { String::new() } else { format!(" ({})", c.variant) }), deck: c.deck, sent, changed, pass, note });
                    r.i += 1;
                    restart(r);
                }
                _ => {}
            }
        }
        Stage::Jogs if !want("jogs") => {
            r.stage = Stage::Outputs;
            restart(r);
        }
        Stage::Outputs if !want("outputs") => {
            r.stage = Stage::Hardware;
            restart(r);
        }
        Stage::Hardware if !want("hardware") => {
            r.stage = Stage::Done;
            restart(r);
        }
        Stage::Jogs => jogs(app, r, frame),
        Stage::Outputs => outputs(app, r, frame),
        Stage::Hardware => hardware(app, r, frame),
        Stage::Done => {
            if since == 2 {
                let text = report(r, app);
                let path = std::env::var("PREPDECK_MIDI_RIG_OUT").map(std::path::PathBuf::from).unwrap_or_else(|_| crate::db::data_dir().join("midirig.md"));
                let _ = std::fs::write(&path, text);
                let fails = r.results.iter().chain(r.outs.iter()).chain(r.hw.iter()).chain(r.jog.iter()).filter(|x| x.pass == Some(false)).count();
                let total = r.results.len() + r.outs.len() + r.hw.len() + r.jog.len();
                say(&format!("done: {total} checks, {fails} fail; report {}", path.display()));
            }
            if since == 6 {
                std::process::exit(0);
            }
        }
    }
}

/// Wait at this tick until `ready` (at most 150 ticks, 2.4 s): true while waiting.
fn hold(r: &mut Rig, ready: bool) -> bool {
    if !ready && r.wait < 150 {
        r.wait += 1;
        r.vt -= 1;
        return true;
    }
    r.wait = 0;
    false
}

// ---------------------------------------------------------------------- the platter

/// The jog as the helper sends it in platter mode, measured: scratch distance, bend, touch
/// release, vinyl off, search, and the nudge mode's buttons.
fn jogs(app: &mut App, r: &mut Rig, frame: u32) {
    let since = frame;
    // Steps of 40 frames each.
    let step = (since / 40) as usize;
    let ph = since % 40;
    let hub = app.midi.hub.clone().unwrap();
    let pos = |app: &App, n: usize| {
        let s = app.player.decks[n].lock();
        s.pos / s.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0)
    };
    let push = |r: &mut Rig, code: &str, func: &str, sent: String, changed: String, pass: bool, note: &str| {
        say(&format!("{} [jog] {code} {func}: {sent} -> {changed}", if pass { "PASS" } else { "FAIL" }));
        r.jog.push(Result_ { port: ERGO, code: code.into(), func: func.into(), deck: 1, sent, changed, pass: Some(pass), note: note.into() });
    };
    match (step, ph) {
        // 1. Scratch: vinyl on, paused deck 1, touch, 2300 counts forward (one ERGO platter
        // turn: 100 messages of +23 every 5 ms), let go. rekordbox's ERGO: 2300 counts a turn,
        // a turn at 33 1/3 rpm is 1.8 s of the record.
        (0, 0) => {
            base(app, 0);
            hub.vinyl[0].store(true, Relaxed);
            r.snaps = vec![snap(app)];
            let mut v = vec![Msg { m: [0x90, 0x36, 0x7F], gap_ms: 5 }];
            v.extend((0..100).map(|_| Msg { m: [0xB0, 0x22, 0x40 + 23], gap_ms: 5 }));
            send_timed(r, ERGO, v);
        }
        (0, 39) => {
            // (Judged once the burst is sent and the record stands, up to 2.4 s more: a busy
            // machine sends the 100 messages more slowly than 5 ms apart.)
            let standing = {
                let d = app.player.decks[0].lock();
                d.dk.jog == crate::deck::Jog::Speed(0.0) || d.dk.jog_left.abs() < 0.5
            };
            if hold(r, r.sending.as_ref().map(|h| h.is_finished()).unwrap_or(true) && standing) {
                return;
            }
            let b = r.snaps[0].clone();
            let p = pos(app, 0);
            let moved = p - b.d[0].pos;
            let jog = snap(app).d[0].jog.clone();
            // (Distance = counts x 60000 / (counts a turn x rpm) ms: 2300 x 60000 / (2300 x 33 1/3)
            // = 1800 ms; within 2 ms, the engine's frame rounding.)
            push(r, "9036 + B022", "JogTouch + JogScratch (one turn)", "90 36 7F, B0 22 57 x100 every 5 ms".into(), format!("deck 1 moved {moved:+.4} s while held (wants +1.8000: 2300 counts = one turn at 33 1/3 rpm); jog {jog}"), near(moved, 1.8, 0.002), "the platter stops: the record stops");
            send(r, ERGO, &[msg(0x90, 0x36, 0x00)]);
        }
        // 2. Scratch backwards, half a turn.
        (1, 0) => {
            r.snaps = vec![snap(app)];
            let mut v = vec![Msg { m: [0x90, 0x36, 0x7F], gap_ms: 5 }];
            v.extend((0..50).map(|_| Msg { m: [0xB0, 0x22, 0x40 - 23], gap_ms: 5 }));
            // (The hand stops, then lifts: 100 ms later.)
            if let Some(l) = v.last_mut() {
                l.gap_ms = 100;
            }
            v.push(Msg { m: [0x90, 0x36, 0x00], gap_ms: 0 });
            send_timed(r, ERGO, v);
        }
        (1, 39) => {
            let let_go = app.player.decks[0].lock().dk.jog == crate::deck::Jog::None;
            if hold(r, r.sending.as_ref().map(|h| h.is_finished()).unwrap_or(true) && let_go) {
                return;
            }
            let moved = pos(app, 0) - r.snaps[0].d[0].pos;
            let d = snap(app).d[0].clone();
            push(r, "9036 + B022", "JogScratch backwards, let go", "B0 22 29 x50, 100 ms still, 90 36 00".into(), format!("moved {moved:+.4} s (wants -0.9000: 1150 counts); after letting go: playing {} (it was paused) jog {}", d.playing, d.jog), near(moved, -0.9, 0.002) && d.jog == "None" && !d.playing, "");
        }
        // 3. Bend: deck 1 playing, the rim (B021) and the untouched top (B023) bend.
        (2, 0) => {
            play(app, 0, true);
            send_timed(r, ERGO, (0..40).map(|_| Msg { m: [0xB0, 0x21, 0x48], gap_ms: 8 }).collect());
        }
        (2, 20) => {
            let b = app.player.decks[0].lock().bend;
            r.note = format!("{b:+.4}");
        }
        (2, 39) => {
            let after = app.player.decks[0].lock().bend;
            let during: f64 = r.note.parse().unwrap_or(0.0);
            push(r, "B021", "WheelPitchBend (rim)", "B0 21 48 x40 every 8 ms".into(), format!("bend while turning {during:+.4}, 60 ms after the last count {after:+.4}"), during > 0.0 && after == 0.0, "");
        }
        (3, 0) => {
            send_timed(r, ERGO, (0..40).map(|_| Msg { m: [0xB0, 0x23, 0x38], gap_ms: 8 }).collect());
        }
        (3, 20) => {
            r.note = format!("{:+.4}", app.player.decks[0].lock().bend);
        }
        (3, 39) => {
            let during: f64 = r.note.parse().unwrap_or(0.0);
            let after = app.player.decks[0].lock().bend;
            push(r, "B023", "JogPitchBend (untouched top, backwards)", "B0 23 38 x40 every 8 ms".into(), format!("bend while turning {during:+.4}, after {after:+.4}"), during < 0.0 && after == 0.0, "");
        }
        // 4. Vinyl off: the helper sends JogPitchBend for the touched top, the touch itself
        // bends nothing and holds nothing.
        (4, 0) => {
            hub.vinyl[0].store(false, Relaxed);
            let mut v = vec![Msg { m: [0x90, 0x36, 0x7F], gap_ms: 8 }];
            v.extend((0..30).map(|_| Msg { m: [0xB0, 0x23, 0x48], gap_ms: 8 }));
            send_timed(r, ERGO, v);
        }
        (4, 20) => {
            let s = app.player.decks[0].lock();
            r.note = format!("{}|{:+.4}|{:?}", s.playing, s.bend, s.dk.jog);
        }
        (4, 39) => {
            send(r, ERGO, &[msg(0x90, 0x36, 0x00)]);
            hub.vinyl[0].store(true, Relaxed);
            let n = r.note.clone();
            let p: Vec<&str> = n.split('|').collect();
            let ok = p.first() == Some(&"true") && p.get(1).and_then(|b| b.parse::<f64>().ok()).unwrap_or(0.0) > 0.0 && p.get(2) == Some(&"None");
            push(r, "9036 + B023", "Vinyl off: touch + top", "90 36 7F, B0 23 48 x30".into(), format!("playing {}, bend {}, jog {}", p.first().unwrap_or(&"?"), p.get(1).unwrap_or(&"?"), p.get(2).unwrap_or(&"?")), ok, "");
        }
        // 5. JogSearch as the helper sends it: a running 7-bit position (Shift + jog). rekordbox
        // reads each byte as position - 64 (`MidiDifference` 0x101322c68, no differencing).
        (5, 0) => {
            base(app, 0);
            app.with_deck(0, |a| a.seek(120.0));
            r.snaps = vec![snap(app)];
            let mut p = 0u8;
            let v: Vec<Msg> = (0..60)
                .map(|_| {
                    p = (p + 4) & 0x7F;
                    Msg { m: [0xB0, 0x27, p], gap_ms: 8 }
                })
                .collect();
            send_timed(r, ERGO, v);
        }
        (5, 39) => {
            let moved = pos(app, 0) - r.snaps[0].d[0].pos;
            push(r, "B027", "JogSearch (helper's running position, forward turn)", "B0 27 04, 08, 0C ... x60 every 8 ms".into(), format!("deck 1 moved {moved:+.2} s"), true, "recorded, not judged: rekordbox reads each byte as byte - 64, so the direction follows where the helper's counter stands, not the turn (see the topology note)");
        }
        // 6. The helper's nudge mode (helper port): PitchBendUp held while the wheel turns.
        (6, 0) => {
            play(app, 0, true);
            send(r, HELPER, &[msg(0x90, 0x71, 0x7F)]);
        }
        (6, 10) => {
            r.note = format!("{:+.4}", app.player.decks[0].lock().bend);
            send(r, HELPER, &[msg(0x90, 0x71, 0x00)]);
        }
        (6, 30) => {
            let during: f64 = r.note.parse().unwrap_or(0.0);
            let after = app.player.decks[0].lock().bend;
            push(r, "9071", "nudge mode: PitchBendUp held / let go", "90 71 7F, 90 71 00".into(), format!("bend held {during:+.4}, released {after:+.4}"), during > 0.0 && after == 0.0, "");
        }
        // 7. Tempo range widening: deck 1 at +12 % with the range at 10 %, the fader not
        // taken over; moving it widens the range to 16 % (rekordbox's AsyncTempoUpdater).
        (7, 0) => {
            base(app, 0);
            app.with_deck(0, |a| {
                a.deck.x.tempo_range = 0;
                a.deck.x.tempo_adj = 0.12;
                a.push_deck_controls();
            });
            hub.arm_takeover(0);
        }
        (7, 8) => {
            send(r, S1, &[msg(0xB0, 0x00, 0x3F), msg(0xB0, 0x20, 0x7F)]);
        }
        (7, 30) => {
            let d = app.deck_of(0);
            let range = crate::player_ext::TEMPO_RANGES[(d.x.tempo_range as usize + 1) % 4];
            let toast = app.toasts.last().map(|t| t.text.clone());
            let ok = range == 16.0 && (d.x.tempo_adj - 0.12).abs() < 1e-9 && toast.as_deref() == Some("Tempo Range has been changed. Move Tempo Slider to the current BPM value.");
            push(r, "B000", "TempoSlider: range widening (deck +12 %, range 10 %)", "B0 00 3F, B0 20 7F".into(), format!("range {range} %, tempo {:+.2} %, status line {toast:?}", d.x.tempo_adj * 100.0), ok, "");
            app.with_deck(0, |a| {
                a.deck.x.tempo_adj = 0.0;
                a.push_deck_controls();
            });
        }
        (8, 0) => {
            r.stage = Stage::Outputs;
            restart(r);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------- the lights

fn outputs(app: &mut App, r: &mut Rig, frame: u32) {
    let since = frame;
    let step = (since / 130) as usize;
    let ph = since % 130;
    // (name, port, code, setup, expected bytes in the window)
    struct O {
        port: usize,
        code: u16,
        func: &'static str,
        state: &'static str,
        want: &'static str,
    }
    let list = [
        O { port: S1, code: 0x900B, func: "PlayPause", state: "deck 1 playing", want: "7F" },
        O { port: S1, code: 0x900B, func: "PlayPause", state: "deck 1 paused", want: "blink 1000" },
        O { port: S1, code: 0x900C, func: "Cue", state: "deck 1 paused away from the cue point", want: "blink 500" },
        O { port: S1, code: 0x900C, func: "Cue", state: "deck 1 paused on the cue point", want: "7F" },
        O { port: S1, code: 0x901B, func: "Sync", state: "deck 1 BEAT SYNC on", want: "7F" },
        O { port: S1, code: 0x9057, func: "Master", state: "deck 1 master", want: "7F" },
        O { port: S1, code: 0x9040, func: "Slip", state: "deck 1 SLIP on", want: "7F" },
        O { port: S1, code: 0x9035, func: "Vinyl", state: "deck 1 vinyl off", want: "00" },
        O { port: ERGO, code: 0x9015, func: "Vinyl (ERGO)", state: "deck 1 vinyl off", want: "00" },
        O { port: S1, code: 0x906A, func: "Reverse", state: "deck 1 reverse", want: "blink" },
        O { port: S1, code: 0x9010, func: "LoopIn", state: "deck 1 looping", want: "7F" },
        O { port: S1, code: 0x902A, func: "PAD1_HotCue", state: "deck 1 hot cue A set", want: "7F" },
        O { port: S1, code: 0x902A, func: "PAD1_HotCue", state: "deck 1 hot cue A empty", want: "00" },
        O { port: S1, code: 0x901C, func: "TempoUp", state: "deck 1 takeover: slider below the tempo", want: "7F" },
        O { port: S1, code: 0x9454, func: "HeadphoneCue", state: "deck 1 headphone CUE on", want: "7F" },
        O { port: S1, code: 0x9240, func: "FX1-1On", state: "FX1 effect on", want: "7F" },
        O { port: S1, code: 0xB039, func: "JogPlayPauseIndicatorShort", state: "deck 1 playing", want: "ring" },
        O { port: S1, code: 0xB469, func: "ChannelLevel", state: "deck 1 playing", want: "level" },
        O { port: S1, code: 0x900B, func: "PlayPause", state: "deck 1 unloaded", want: "00" },
    ];
    let Some(o) = list.get(step) else {
        r.stage = Stage::Hardware;
        restart(r);
        return;
    };
    let hub = app.midi.hub.clone().unwrap();
    match ph {
        // A plain state first (the light then has something to change from).
        0 => {
            if app.deck_of(0).audio.is_none() {
                if let Some(id) = app.tracks.iter().find(|t| t.analyzed == 1 && !t.missing).map(|t| t.id) {
                    app.with_deck(0, |a| a.load_track_here(id));
                }
            }
            hub.vinyl[0].store(true, Relaxed);
            app.px.mix[0].cue = false;
            {
                let p = &mut app.px.fx[0].params;
                if p.single { p.one.on = false } else { p.slots[0].on = false }
            }
            if app.deck_of(0).audio.is_some() {
                base(app, 0);
            }
            app.with_deck(1, |a| a.set_master());
            match o.state {
                "deck 1 hot cue A empty" => app.with_deck(0, |a| {
                    if !a.deck.cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == 0) {
                        a.hot_cue_down(0);
                        a.hot_cue_up();
                    }
                }),
                "deck 1 hot cue A set" => app.with_deck(0, |a| {
                    if let Some(id) = a.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == 0).map(|c| c.id) {
                        a.delete_cue(id);
                    }
                }),
                _ => {}
            }
            if app.deck_of(0).audio.is_some() && o.state != "deck 1 paused on the cue point" {
                app.with_deck(0, |a| a.seek(20.0));
            }
        }
        // Then the state under test.
        20 => {
            r.mark = r.got.lock().len();
            match o.state {
                "deck 1 playing" => play(app, 0, true),
                "deck 1 paused" | "deck 1 paused away from the cue point" => app.with_deck(0, |a| a.seek(25.0)),
                "deck 1 paused on the cue point" => app.with_deck(0, |a| a.seek(a.deck.cue_point)),
                "deck 1 BEAT SYNC on" => app.with_deck(0, |a| a.toggle_beat_sync()),
                "deck 1 master" => app.with_deck(0, |a| a.set_master()),
                "deck 1 SLIP on" => app.with_deck(0, |a| a.toggle_slip()),
                "deck 1 vinyl off" => hub.vinyl[0].store(false, Relaxed),
                "deck 1 reverse" => {
                    play(app, 0, true);
                    app.with_deck(0, |a| a.reverse_toggle(false));
                }
                "deck 1 looping" => {
                    play(app, 0, true);
                    app.with_deck(0, |a| a.auto_loop(4.0));
                }
                "deck 1 hot cue A set" => app.with_deck(0, |a| {
                    a.hot_cue_down(0);
                    a.hot_cue_up();
                }),
                "deck 1 hot cue A empty" => app.with_deck(0, |a| {
                    if let Some(id) = a.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == 0).map(|c| c.id) {
                        a.delete_cue(id);
                    }
                }),
                "deck 1 takeover: slider below the tempo" => {
                    app.with_deck(0, |a| {
                        a.deck.x.tempo_adj = 0.03;
                        a.push_deck_controls();
                    });
                    hub.arm_takeover(0);
                    // The fader at its centre: 3 % below the deck.
                    send(r, S1, &[msg(0xB0, 0x00, 0x3F), msg(0xB0, 0x20, 0x7F)]);
                }
                "deck 1 headphone CUE on" => app.px.mix[0].cue = true,
                "FX1 effect on" => {
                    let p = &mut app.px.fx[0].params;
                    if p.single { p.one.on = true } else { p.slots[0].on = true }
                }
                "deck 1 unloaded" => {
                    play(app, 0, false);
                    app.with_deck(0, |a| a.unload_track());
                }
                _ => {}
            }
        }
        125 => {
            let got: Vec<(f64, [u8; 3])> = r.got.lock()[r.mark..].iter().filter(|x| x.0 == o.port && (((x.2[0] as u16) << 8) | x.2[1] as u16) == o.code).map(|x| (x.1, x.2)).collect();
            let vals: Vec<u8> = got.iter().map(|x| x.1[2]).collect();
            let times: Vec<f64> = got.iter().map(|x| x.0).collect();
            let (pass, what) = match o.want {
                "7F" => (vals.last() == Some(&0x7F), format!("last value {:?}", vals.last())),
                "00" => (vals.last() == Some(&0x00), format!("last value {:?}", vals.last())),
                "blink" | "blink 1000" | "blink 500" => {
                    // On / off in turn; the cycle from the times of two "on" messages.
                    let ons: Vec<f64> = got.iter().filter(|x| x.1[2] == 0x7F).map(|x| x.0).collect();
                    let cycle = if ons.len() >= 2 { ons[ons.len() - 1] - ons[ons.len() - 2] } else { 0.0 };
                    let want = match o.want { "blink 1000" => 1000.0, "blink 500" => 500.0, _ => cycle };
                    (vals.contains(&0x7F) && vals.contains(&0x00) && (want == cycle || near(cycle, want, 60.0)), format!("{} messages, values {:?}, cycle {cycle:.0} ms", vals.len(), vals.iter().take(8).collect::<Vec<_>>()))
                }
                "ring" => {
                    let ok = !vals.is_empty() && vals.iter().all(|v| (1..=48).contains(v)) && vals.windows(2).any(|w| w[0] != w[1]);
                    (ok, format!("{} values {:?}... (1..48, turning)", vals.len(), vals.iter().take(8).collect::<Vec<_>>()))
                }
                "level" => (!vals.is_empty() && vals.iter().any(|v| *v > 0), format!("{} values, max {:?} (0..127)", vals.len(), vals.iter().max())),
                _ => (false, String::new()),
            };
            let _ = times;
            say(&format!("{} [out] {} {} ({}): {}", if pass { "PASS" } else { "FAIL" }, hex(o.code), o.func, o.state, what));
            r.outs.push(Result_ { port: o.port, code: hex(o.code), func: o.func.into(), deck: 1, sent: o.state.into(), changed: format!("wants {}; got {what}", o.want), pass: Some(pass), note: String::new() });
            // Back to a plain state.
            hub.vinyl[0].store(true, Relaxed);
            app.px.mix[0].cue = false;
            {
                let p = &mut app.px.fx[0].params;
                if p.single { p.one.on = false } else { p.slots[0].on = false }
            }
            if app.deck_of(0).audio.is_some() {
                base(app, 0);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------- hardware events

fn hardware(app: &mut App, r: &mut Rig, frame: u32) {
    let t = frame;
    let hub = app.midi.hub.clone().unwrap();
    let names = |h: &crate::midi::Hub| h.devices.lock().iter().map(|d| d.name.clone()).collect::<Vec<_>>();
    let push = |r: &mut Rig, func: &str, sent: String, changed: String, pass: bool, note: &str| {
        say(&format!("{} [hw] {func}: {sent} -> {changed}", if pass { "PASS" } else { "FAIL" }));
        r.hw.push(Result_ { port: S1, code: "-".into(), func: func.into(), deck: 0, sent, changed, pass: Some(pass), note: note.into() });
    };
    // r.i: the phase; r.sub: the tick it began.
    if t == 0 {
        r.i = 0;
        r.sub = 0;
    }
    let el = t.saturating_sub(r.sub);
    let next = |r: &mut Rig| {
        r.i += 1;
        r.sub = t;
    };
    match r.i {
        // 1. Unplug the S1: its ports go. How long until recordbox lets it go, and the status
        // line. (A port of this process.)
        0 => {
            if app.deck_of(0).audio.is_none() {
                if let Some(id) = app.tracks.iter().find(|t| t.analyzed == 1 && !t.missing).map(|t| t.id) {
                    app.with_deck(0, |a| a.load_track_here(id));
                }
            }
            r.ports[S1].src.lock().take();
            r.ports[S1].dst.take();
            r.t_mark = Some(Instant::now());
            next(r);
        }
        1 => {
            let gone = !names(&hub).contains(&PORTS[0].to_string());
            if gone || el > 400 {
                let ms = r.t_mark.map(|x| x.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0);
                r.note = format!("{ms:.0}");
                if !gone {
                    push(r, "unplug: the S1's ports go (this process's ports)", "(virtual source and destination closed)".into(), format!("still open after {ms:.0} ms"), false, "");
                }
                next(r);
            }
        }
        2 if el >= 12 => {
            let toast = app.toasts.iter().rev().find(|t| t.text.contains("DDJ-S1")).map(|t| t.text.clone());
            let open = names(&hub);
            if !open.contains(&PORTS[0].to_string()) {
                push(r, "unplug: the S1's ports go (this process's ports)", "(virtual source and destination closed)".into(), format!("let go after {} ms; devices {open:?}; status line {toast:?}", r.note), toast.as_deref() == Some("PIONEER DDJ-S1 is disconnected."), "");
            }
            // 2. Plug it back: a new port of the same name.
            match make_port(S1, PORT_NAMES[0], r.got.clone(), r.t0) {
                Some(p) => {
                    if let Some(ids) = hub.only_ids.lock().as_mut() {
                        ids.push(p.src_id.clone());
                        ids.push(p.dst_id.clone());
                    }
                    r.ports[S1] = p;
                }
                None => say("could not make the S1 port again"),
            }
            r.mark = r.got.lock().len();
            r.t_mark = Some(Instant::now());
            r.note.clear();
            next(r);
        }
        3 => {
            let back = names(&hub).contains(&PORTS[0].to_string());
            if back && r.note.is_empty() {
                r.note = format!("{t}");
            }
            let since_back = r.note.parse::<u32>().map(|b| t.saturating_sub(b)).unwrap_or(0);
            if (back && since_back > 40) || el > 400 {
                r.note.clear();
                let open = names(&hub);
                let toast = app.toasts.iter().rev().find(|t| t.text.contains("DDJ-S1")).map(|t| t.text.clone());
                let lights = r.got.lock()[r.mark..].iter().filter(|x| x.0 == S1).count();
                push(r, "replug: a new port of the same name", "(new virtual source and destination)".into(), format!("devices {open:?}; status line {toast:?}; {lights} light messages sent to it again"), back && toast.as_deref() == Some("PIONEER DDJ-S1 is connected.") && lights > 0, "");
                // 3. The helper restarts quickly (its two ports go and come back within one
                // scan), with the ERGO platter held: the held record is let go and the new
                // ports are opened.
                base(app, 0);
                play(app, 0, true);
                send(r, ERGO, &[msg(0x90, 0x36, 0x7F), msg(0xB0, 0x22, 0x40)]);
                next(r);
            }
        }
        4 if el >= 20 => {
            r.note = format!("{:?}", app.player.decks[0].lock().dk.jog);
            for p in [HELPER, ERGO] {
                r.ports[p].src.lock().take();
                r.ports[p].dst.take();
            }
            std::thread::sleep(Duration::from_millis(100));
            for p in [HELPER, ERGO] {
                if let Some(np) = make_port(p, PORT_NAMES[p], r.got.clone(), r.t0) {
                    if let Some(ids) = hub.only_ids.lock().as_mut() {
                        ids.push(np.src_id.clone());
                        ids.push(np.dst_id.clone());
                    }
                    r.ports[p] = np;
                }
            }
            r.mark = r.got.lock().len();
            next(r);
        }
        5 if el >= 200 => {
            let jog = format!("{:?}", app.player.decks[0].lock().dk.jog);
            let devs: Vec<(String, String)> = hub.devices.lock().iter().map(|d| (d.name.clone(), d.port_id.clone())).collect();
            let new_ids: Vec<&String> = [HELPER, ERGO].iter().map(|p| &r.ports[*p].src_id).collect();
            let reopened = new_ids.iter().all(|id| devs.iter().any(|d| &d.1 == *id));
            let vinyl = r.got.lock()[r.mark..].iter().any(|x| x.0 == ERGO && x.2 == [0x90, 0x15, 0x7F]);
            push(r, "helper restart within one scan, platter held", "(both helper ports closed and made again 0.1 s later)".into(), format!("held record before {}, after {jog}; reopened on the new ports {reopened}; Vinyl light sent to the new ERGO port {vinyl}", r.note), jog == "None" && reopened && vinyl, "");
            // 4. A port of another process (a USB device, the helper): appears, then goes.
            let exe = std::env::current_exe().ok();
            let name = OTHER_NAME.to_string();
            *hub.only.lock() = Some(vec![name.clone()]);
            r.child = exe.and_then(|e| std::process::Command::new(e).args(["--midi-virtual", &name, "4"]).stderr(std::process::Stdio::null()).spawn().ok());
            r.t_mark = Some(Instant::now());
            r.note.clear();
            next(r);
        }
        6 => {
            let name = OTHER_NAME;
            let there = names(&hub).iter().any(|n| n == name);
            let ms = r.t_mark.map(|x| x.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0);
            if there && r.note.is_empty() {
                r.note = format!("{ms:.0}");
            }
            if !r.note.is_empty() && !there {
                let handled = app.midi.handled;
                push(r, "another process's port comes and goes", "(a second recordbox process: --midi-virtual for 4 s, PLAY every second)".into(), format!("opened {} ms after the process started, let go {ms:.0} ms after it started (it closes at 4000); messages handled so far {handled}", r.note), true, "unmapped device: no mapping, its PLAY presses are not mapped");
                *hub.only.lock() = None;
                if let Some(mut c) = r.child.take() {
                    let _ = c.wait();
                }
                next(r);
            } else if el > 900 {
                push(r, "another process's port comes and goes", "(a second recordbox process: --midi-virtual for 4 s)".into(), format!("seen after {} ms; still listed after {ms:.0} ms", if r.note.is_empty() { "never".to_string() } else { r.note.clone() }), false, "");
                *hub.only.lock() = None;
                if let Some(mut c) = r.child.take() {
                    let _ = c.kill();
                }
                next(r);
            }
        }
        // 5. A flood: both decks play, the platter is turned at full speed on deck 1 and the
        // rim on deck 2 (a message every 1 ms on each) for 3 s.
        7 => {
            play(app, 0, true);
            play(app, 1, true);
            r.frame_ms.clear();
            r.note = format!("{}|{}", crate::audio::AUDIO_DROPPED.load(Relaxed), hub.devices.lock().iter().map(|d| d.received.load(Relaxed)).sum::<u64>());
            let mut v = vec![Msg { m: [0x90, 0x36, 0x7F], gap_ms: 0 }];
            for i in 0..3000 {
                v.push(Msg { m: [0xB0, 0x22, if (i / 50) % 2 == 0 { 0x40 + 20 } else { 0x40 - 20 }], gap_ms: 0 });
                v.push(Msg { m: [0xB1, 0x21, 0x4A], gap_ms: 1 });
            }
            v.push(Msg { m: [0x90, 0x36, 0x00], gap_ms: 0 });
            // (Sent on a thread; then how long the last message, the touch let go, takes to
            // reach the deck: the worker's backlog.)
            let src = r.ports[ERGO].src.clone();
            let decks = app.player.decks.clone();
            *FLOOD.lock() = None;
            r.sending = Some(std::thread::spawn(move || {
                let t0 = Instant::now();
                for m in v {
                    if let Some(c) = src.lock().as_mut() {
                        let _ = c.send(&m.m);
                    }
                    if m.gap_ms > 0 {
                        std::thread::sleep(Duration::from_millis(m.gap_ms));
                    }
                }
                let sent_ms = t0.elapsed().as_secs_f64() * 1000.0;
                let t1 = Instant::now();
                let mut lag = -1.0;
                for _ in 0..20000 {
                    if decks[0].lock().dk.jog == crate::deck::Jog::None {
                        lag = t1.elapsed().as_secs_f64() * 1000.0;
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(100));
                }
                FLOOD.lock().replace((sent_ms, lag));
            }));
            next(r);
        }
        8 if FLOOD.lock().is_some() && el > 10 => {
            let p: Vec<u64> = r.note.split('|').filter_map(|x| x.parse().ok()).collect();
            let drops = crate::audio::AUDIO_DROPPED.load(Relaxed).saturating_sub(*p.first().unwrap_or(&0));
            let rec = hub.devices.lock().iter().map(|d| d.received.load(Relaxed)).sum::<u64>() - p.get(1).copied().unwrap_or(0);
            let mut f = r.frame_ms.clone();
            f.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let (avg, p95, max) = if f.is_empty() { (0.0, 0.0, 0.0) } else { (f.iter().sum::<f32>() / f.len() as f32, f[f.len() * 95 / 100], f[f.len() - 1]) };
            let (sent_ms, lag) = FLOOD.lock().unwrap_or((0.0, -1.0));
            push(r, "jog flood while both decks play (6002 messages)", "B0 22 +-20 and B1 21 4A, a pair every 1 ms".into(), format!("sent in {sent_ms:.0} ms; {rec} received; the last one (touch let go) reached the deck {lag:.1} ms after it was sent; audio buffers given up {drops}; frame interval avg {avg:.1} ms p95 {p95:.1} max {max:.1}"), rec >= 6002 && drops == 0 && (0.0..20.0).contains(&lag), "");
            // 6. The window minimised (from a thread: while minimised no frames may come):
            // PLAY, a row done on the port's thread, and SYNC, a row the interface does.
            base(app, 0);
            base(app, 1);
            let ctx = app.ctx.clone();
            let src = r.ports[S1].src.clone();
            let decks = app.player.decks.clone();
            r.sending = Some(std::thread::spawn(move || {
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(true));
                std::thread::sleep(Duration::from_millis(800));
                let f0 = FRAME.load(Relaxed);
                let t0 = Instant::now();
                if let Some(c) = src.lock().as_mut() {
                    for m in [[0x90, 0x0B, 0x7F], [0x90, 0x0B, 0x00], [0x90, 0x1B, 0x7F], [0x90, 0x1B, 0x00]] {
                        let _ = c.send(&m);
                    }
                }
                let mut play_ms = -1.0;
                for _ in 0..200 {
                    if decks[0].lock().playing {
                        play_ms = t0.elapsed().as_secs_f64() * 1000.0;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                MIN_PLAY_MS.store((play_ms * 10.0) as i64, Relaxed);
                std::thread::sleep(Duration::from_millis(1500));
                MIN_FRAMES.store(FRAME.load(Relaxed).wrapping_sub(f0) as i64, Relaxed);
                MIN_SYNC.store(SYNC_SEEN.load(Relaxed), Relaxed);
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }));
            next(r);
        }
        9 => {
            SYNC_SEEN.store(app.deck_of(0).x.beat_sync, Relaxed);
            if r.sending.as_ref().map(|h| h.is_finished()).unwrap_or(true) {
                next(r);
            }
        }
        10 if el >= 40 => {
            let play_ms = MIN_PLAY_MS.load(Relaxed) as f64 / 10.0;
            let frames_while = MIN_FRAMES.load(Relaxed);
            let sync_while = MIN_SYNC.load(Relaxed);
            let sync_after = app.deck_of(0).x.beat_sync;
            push(r, "window minimised: PLAY (port thread) and SYNC (interface)", "90 0B 7F/00, 90 1B 7F/00 while minimised".into(), format!("PLAY reached the engine after {play_ms:.1} ms; frames drawn in the 1.5 s after: {frames_while}; SYNC on while minimised {sync_while}, after the window came back {sync_after}"), (0.0..100.0).contains(&play_ms) && sync_after, "rows the interface does (SYNC, LOAD, browse, FX) wait for a drawn frame");
            base(app, 0);
            r.stage = Stage::Done;
            restart(r);
        }
        _ => {}
    }
}

static FLOOD: Mutex<Option<(f64, f64)>> = Mutex::new(None);
static Q_START: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(9);
static MIN_PLAY_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-10);
static MIN_FRAMES: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
static MIN_SYNC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SYNC_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn report(r: &Rig, app: &App) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# DDJ-S1 rig rehearsal (virtual ports)\n");
    let _ = writeln!(s, "Run: `PREPDECK_SHOT_PLAYER=midirig` at {}. Mappings as the hub loaded them:\n", chrono::Local::now().format("%Y-%m-%d %H:%M"));
    if let Some(h) = app.midi.hub.as_ref() {
        for n in PORTS {
            if let Some(d) = h.device(n) {
                let _ = writeln!(s, "- **{n}**: {}", h.mapping_summary(&d));
            }
        }
    }
    let table = |s: &mut String, title: &str, rows: &[Result_]| {
        let pass = rows.iter().filter(|x| x.pass == Some(true)).count();
        let fail = rows.iter().filter(|x| x.pass == Some(false)).count();
        let _ = writeln!(s, "\n## {title}: {pass} pass, {fail} fail, {} not judged\n", rows.len() - pass - fail);
        let _ = writeln!(s, "| port | code | function | deck | sent / state | what changed | result | note |\n|---|---|---|---|---|---|---|---|");
        for x in rows {
            let _ = writeln!(s, "| {} | {} | {} | {} | {} | {} | {} | {} |", PORTS[x.port], x.code, x.func, x.deck, x.sent.replace('|', "/"), x.changed.replace('|', "/"), match x.pass { Some(true) => "PASS", Some(false) => "**FAIL**", None => "-" }, x.note.replace('|', "/"));
        }
    };
    table(&mut s, "Inputs (every input code of the three mappings)", &r.results);
    table(&mut s, "Platter (the helper's jog messages)", &r.jog);
    table(&mut s, "Outputs (lights and displays)", &r.outs);
    table(&mut s, "Connect, unplug, restart, flood, minimised", &r.hw);
    s
}
