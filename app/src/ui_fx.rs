//! The FX panel of PERFORMANCE mode: FX1 and FX2, each with its assign buttons, multi mode
//! (three effect slots: name = on/off, list, LEVEL/DEPTH, `<` value `>`) or single mode (one
//! effect with LEVEL/DEPTH, `<` value `>`, two knobs, two buttons and a snapshot), RELEASE FX
//! (press and hold) and the FX tempo (AUTO / TAP); under it the CFX row in single or multi
//! mode. Positions measured on rekordbox 7.2.19 at 1512 pt
//! (`rekordbox-reference/performance/06-fx.jpg`): the row is 52 pt tall, a unit 756 pt wide.
//! That reference shows multi mode and CFX single mode; the single-mode slot and the CFX
//! multi-mode row are laid out here without a reference.
//!
//! The panel's choices are kept between sessions (`FxSaved`, in recordbox's own settings),
//! holding what rekordbox keeps in its FxUnitSettings file.

use crate::app::*;
use crate::fx::{self, Kind, UnitParams};
use crate::theme::*;
use crate::ui_player::{lighten, tip};
use eframe::egui;
use egui::{pos2, vec2, Align2, Rect, Sense, Stroke, Ui};
use serde::{Deserialize, Serialize};

/// Height of the FX panel: the Beat FX row (52 pt) and the CFX row (26 pt).
pub const FX_H: f32 = 78.0;
const BEAT_H: f32 = 52.0;

/// A single-mode snapshot: knob 1, knob 2, button 1, button 2.
pub type Snap = (f32, f32, bool, bool);

/// One FX unit as the interface holds it.
pub struct FxUi {
    pub params: UnitParams,
    /// Index into each multi slot's beat table, and single mode's.
    pub idx: [usize; 3],
    pub one_idx: usize,
    /// The beat index each effect was left at (by its place in `fx::INFO`): the three multi
    /// slots, then single mode.
    pub beats: [[u8; 36]; 4],
    /// Single mode's snapshot of each effect.
    pub snaps: [Option<Snap>; 36],
    pub release_kind: u8,
    /// Index into the RELEASE FX beat table, kept per RELEASE FX.
    pub release_idx: [usize; 3],
    /// FX tempo: AUTO (the decks' own) or TAP.
    pub auto: bool,
    pub tap: fx::Tap,
    pub tap_bpm: f32,
}

fn pos(k: Kind) -> usize {
    fx::INFO.iter().position(|i| i.kind == k).unwrap_or(0)
}

impl FxUi {
    pub fn new() -> FxUi {
        let params = UnitParams::new();
        let defaults: [u8; 36] = std::array::from_fn(|i| fx::INFO[i].def as u8);
        let idx = [fx::beat_table(params.slots[0].kind).1, fx::beat_table(params.slots[1].kind).1, fx::beat_table(params.slots[2].kind).1];
        FxUi { params, idx, one_idx: fx::beat_table(params.one.kind).1, beats: [defaults; 4], snaps: [None; 36], release_kind: 0, release_idx: [fx::RELEASE[0].1, fx::RELEASE[1].1, fx::RELEASE[2].1], auto: true, tap: fx::Tap::new(), tap_bpm: 120.0 }
    }

    /// Put effect `k` in slot `s` (0..2 = the multi slots, 3 = single mode): switched off,
    /// at the beat value it was left at; single mode takes the effect's snapshot.
    pub fn set_kind(&mut self, s: usize, k: Kind) {
        let table = fx::beat_table(k).0;
        let idx = (self.beats[s][pos(k)] as usize).min(table.len().saturating_sub(1));
        let mut sp = fx::SlotParams::new(k);
        sp.value = table.get(idx).copied().unwrap_or(0.0);
        if s < 3 {
            sp.level = self.params.slots[s].level;
            self.params.slots[s] = sp;
            self.idx[s] = idx;
        } else {
            sp.level = self.params.one.level;
            if let Some((a, b, c, d)) = self.snaps[pos(k)] {
                sp.knob = [a, b];
                sp.btn = [c, d];
            }
            self.params.one = sp;
            self.one_idx = idx;
        }
    }

    pub fn set_index(&mut self, s: usize, idx: usize) {
        let sp = if s < 3 { &mut self.params.slots[s] } else { &mut self.params.one };
        let table = fx::beat_table(sp.kind).0;
        if table.is_empty() {
            return;
        }
        let idx = idx.min(table.len() - 1);
        sp.value = table[idx];
        self.beats[s][pos(sp.kind)] = idx as u8;
        if s < 3 {
            self.idx[s] = idx;
        } else {
            self.one_idx = idx;
        }
    }

    /// What is kept between sessions.
    pub fn saved(&self) -> UnitSaved {
        let by_id = |slot: usize, single: bool, n: usize| -> Vec<u8> { (0..n).map(|id| fx::from_id(id as i64, single).map(|k| self.beats[slot][pos(k)]).unwrap_or(0)).collect() };
        UnitSaved {
            single_shown: self.params.single,
            single_fx: fx::info_of(self.params.one.kind).single_id,
            single_beat: by_id(3, true, 34),
            multi_fx: std::array::from_fn(|s| fx::info_of(self.params.slots[s].kind).multi_id),
            multi_beat: std::array::from_fn(|s| by_id(s, false, 35)),
            release_fx: self.release_kind,
            release_beat: std::array::from_fn(|i| self.release_idx[i] as u8),
            snapshots: (0..34).map(|id| fx::from_id(id, true).and_then(|k| self.snaps[pos(k)])).collect(),
        }
    }

    pub fn load(&mut self, s: &UnitSaved) {
        for (slot, single, list) in [(0, false, &s.multi_beat[0]), (1, false, &s.multi_beat[1]), (2, false, &s.multi_beat[2]), (3, true, &s.single_beat)] {
            for (id, b) in list.iter().enumerate() {
                if let Some(k) = fx::from_id(id as i64, single) {
                    let n = fx::beat_table(k).0.len();
                    self.beats[slot][pos(k)] = (*b as usize).min(n.saturating_sub(1)) as u8;
                }
            }
        }
        for (id, snap) in s.snapshots.iter().enumerate() {
            if let Some(k) = fx::from_id(id as i64, true) {
                self.snaps[pos(k)] = snap.map(|(a, b, c, d)| (a.clamp(0.0, 1.0), b.clamp(0.0, 1.0), c, d));
            }
        }
        for slot in 0..3 {
            if let Some(k) = fx::from_id(s.multi_fx[slot] as i64, false) {
                self.set_kind(slot, k);
            }
        }
        if let Some(k) = fx::from_id(s.single_fx as i64, true) {
            self.set_kind(3, k);
        }
        self.params.single = s.single_shown;
        self.release_kind = s.release_fx.min(2);
        for i in 0..3 {
            self.release_idx[i] = (s.release_beat[i] as usize).min(fx::RELEASE_BEATS.len() - 1);
        }
    }
}

/// One unit's kept choices, as rekordbox's FxUnitSettings holds them: the mode, the effects
/// by engine id, the beat-table index of every effect by engine id (per multi slot and for
/// single mode), the RELEASE FX and its beat indices, and single mode's snapshots. Not kept,
/// as in rekordbox: on/off, LEVEL/DEPTH, the assign buttons, AUTO / TAP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UnitSaved {
    pub single_shown: bool,
    pub single_fx: i8,
    pub single_beat: Vec<u8>,
    pub multi_fx: [i8; 3],
    pub multi_beat: [Vec<u8>; 3],
    pub release_fx: u8,
    pub release_beat: [u8; 3],
    pub snapshots: Vec<Option<Snap>>,
}

impl Default for UnitSaved {
    fn default() -> Self {
        FxUi::new().saved()
    }
}

/// The CFX row's kept choices: the mode, DEFAULT / USER, the USER effect, PARAM per effect,
/// and multi mode's effect per channel (decks 1 to 4, master, sampler).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CfxSaved {
    pub multi_shown: bool,
    pub user: bool,
    pub user_fx: u8,
    pub params: [f32; 9],
    pub multi_fx: [u8; 6],
}

impl Default for CfxSaved {
    fn default() -> Self {
        CfxSaved { multi_shown: false, user: false, user_fx: 0, params: [0.5; 9], multi_fx: [0; 6] }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxSaved {
    pub units: [UnitSaved; 2],
    pub cfx: CfxSaved,
}

/// The FX panel's state outside the two units.
pub struct FxExtra {
    /// The kept choices have been put in place (once, at the first frame).
    pub loaded: bool,
    /// A change waits to be written.
    pub pending: Option<std::time::Instant>,
    /// CFX multi mode: an effect and an on/off per channel (decks 1 to 4, master, sampler).
    pub cfx_multi: bool,
    pub cfx_multi_kind: [u8; 6],
    pub cfx_multi_on: [bool; 6],
    /// PARAM of each CFX effect.
    pub cfx_params: [f32; 9],
}

impl Default for FxExtra {
    fn default() -> Self {
        FxExtra { loaded: false, pending: None, cfx_multi: false, cfx_multi_kind: [0; 6], cfx_multi_on: [true; 6], cfx_params: [0.5; 9] }
    }
}

fn cfx_ok(k: u8) -> u8 {
    if crate::cfx::LIST.iter().any(|x| x.1 == Some(k)) { k } else { 0 }
}

/// The kept choices as they stand now.
pub fn snapshot(app: &App) -> FxSaved {
    let x = &app.px.fxx;
    FxSaved { units: [app.px.fx[0].saved(), app.px.fx[1].saved()], cfx: CfxSaved { multi_shown: x.cfx_multi, user: app.px.cfx_user, user_fx: app.px.cfx_kind, params: x.cfx_params, multi_fx: x.cfx_multi_kind } }
}

/// Put kept choices in place.
pub fn restore(app: &mut App, s: &FxSaved) {
    for (u, saved) in app.px.fx.iter_mut().zip(&s.units) {
        u.load(saved);
    }
    let x = &mut app.px.fxx;
    x.cfx_multi = s.cfx.multi_shown;
    x.cfx_multi_kind = s.cfx.multi_fx.map(cfx_ok);
    x.cfx_params = s.cfx.params.map(|p| if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.5 });
    app.px.cfx_user = s.cfx.user;
    app.px.cfx_kind = cfx_ok(s.cfx.user_fx);
    app.px.cfx_param = x.cfx_params[(if app.px.cfx_user { app.px.cfx_kind } else { 0 }) as usize % 9];
}

/// The Sound Color FX of a channel (0..3 = decks 1 to 4, 4 = master, 5 = sampler): effect,
/// COLOR, PARAM. Single mode: one effect for every channel; multi mode: the channel's own,
/// out of the path while its button is off.
pub fn cfx_for(app: &App, ch: usize) -> (u8, f32, f32) {
    let x = &app.px.fxx;
    let color = if ch == fx::SAMPLER { app.px.sampler.cfx_color } else { app.px.cfx_color[ch.min(4)] };
    if x.cfx_multi {
        let k = x.cfx_multi_kind[ch.min(5)];
        (k, if x.cfx_multi_on[ch.min(5)] { color } else { 0.5 }, x.cfx_params[k as usize % 9])
    } else {
        (if app.px.cfx_user { app.px.cfx_kind } else { 0 }, color, app.px.cfx_param)
    }
}

/// Once per frame: hand the units' settings and each channel's tempo to the audio thread.
/// The rack (its delay lines) is made the first time the FX panel is used.
pub fn sync(app: &mut App) {
    if !app.px.fxx.loaded {
        app.px.fxx.loaded = true;
        let s = app.prefs.fx.clone();
        restore(app, &s);
    }
    // Keep the choices: written half a second after the last change.
    let now = snapshot(app);
    if now != app.prefs.fx {
        app.prefs.fx = now;
        app.px.fxx.pending = Some(std::time::Instant::now());
    }
    if app.px.fxx.pending.map(|t| t.elapsed().as_millis() > 500).unwrap_or(false) {
        app.px.fxx.pending = None;
        app.save_prefs();
    }
    let on = app.performance() && app.prefs.perf_fx;
    if !on {
        if let Some(mut g) = fx::RACK.try_lock() {
            if let Some(r) = g.as_mut() {
                r.enabled = false;
            }
        }
        return;
    }
    // Tempo per channel: each deck's own, and the master deck's for the master. Beat FX
    // quantize follows each deck's Q.
    let mut bpm = [120.0f32; fx::CHANNELS];
    let mut quantize = [false; fx::CHANNELS];
    for n in 0..app.deck_count() {
        let b = app.with_deck(n, |a| if a.deck.track.is_some() { a.effective_bpm() } else { 0.0 });
        if b > 0.0 {
            bpm[n as usize] = b as f32;
        }
        quantize[n as usize] = if n == app.px.active { app.prefs.quantize } else { app.px.parked_quantize[n as usize] };
    }
    let m = app.px.master.min(3) as usize;
    bpm[fx::MASTER] = bpm[m];
    quantize[fx::MASTER] = quantize[m];
    bpm[fx::SAMPLER] = app.prefs.sampler.bpm.clamp(40.0, 499.0);
    let master_cfx = cfx_for(app, fx::MASTER);
    let mut g = fx::RACK.lock();
    let rack = g.get_or_insert_with(|| Box::new(fx::Rack::new()));
    rack.enabled = true;
    rack.master_cfx_set = master_cfx;
    for (u, ui) in rack.units.iter_mut().zip(app.px.fx.iter()) {
        let mut p = ui.params;
        p.bpm = if ui.auto { bpm } else { [ui.tap_bpm; fx::CHANNELS] };
        p.quantize = quantize;
        u.prepare(&p.assign);
        u.params = p;
    }
}

fn small(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, lit: bool, enabled: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let p = ui.painter();
    p.rect_filled(r, 2.0, if lit { BLUE } else if resp.hovered() && enabled { lighten(BTN) } else { hex(0x141414) });
    p.text(r.center(), Align2::CENTER_CENTER, label, caps(8.0), if enabled { WHITE } else { OFF });
    resp
}

/// A mode button: three dots (multi) or one (single), lit for the mode in use.
fn mode_button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, dots: i32, lit: bool, tooltip: &str) -> bool {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let p = ui.painter();
    if lit {
        p.rect_filled(r, 2.0, BLUE);
    } else {
        p.rect_filled(r, 2.0, if resp.hovered() { lighten(BTN) } else { hex(0x1E1E1E) });
        p.rect_stroke(r, 2.0, Stroke::new(1.0_f32, hex(0x646464)), egui::StrokeKind::Inside);
    }
    let c = if lit { WHITE } else { hex(0x8C8C8C) };
    for k in 0..dots {
        p.circle_filled(r.center() + vec2((k as f32 - (dots - 1) as f32 / 2.0) * 5.0, 0.0), 1.5, c);
    }
    let clicked = resp.clicked();
    let _ = resp.on_hover_text(tooltip);
    clicked
}

/// The read-out of a single-mode knob (rekordbox: most are the position as 0..100; PITCH
/// runs -100..+100 with a dead zone around 0; STAGE is the number of stages).
pub fn knob_text(label: &str, v: f32) -> String {
    match label {
        "PITCH" => {
            let x = ((v - 0.5) * 200.0).round();
            if x.abs() < 2.5 { "0".into() } else { format!("{x:+.0}") }
        }
        "STAGE" => format!("{}", fx::phaser_stages(v)),
        _ => format!("{:.0}", v * 100.0),
    }
}

/// The effect's name (on/off), its list, LEVEL/DEPTH and `<` value `>`: a multi slot
/// (`s` 0..2) or single mode's effect (`s` 3), drawn at `sx` in the unit.
fn slot(app: &mut App, ui: &mut Ui, u: usize, s: usize, o: egui::Pos2, sx: f32) {
    let p = ui.painter().clone();
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(o.x + x, o.y + y), vec2(w, h));
    let single = s == 3;
    let sp = if single { app.px.fx[u].params.one } else { app.px.fx[u].params.slots[s] };
    let name_r = at(sx, 10.0, 80.0, 15.0);
    let nresp = ui.interact(name_r, ui.id().with(("fxname", u, s)), Sense::click());
    p.rect_filled(name_r, 2.0, if sp.on { BLUE } else if nresp.hovered() { lighten(BTN) } else { BTN });
    p.text(name_r.center(), Align2::CENTER_CENTER, fx::name(sp.kind), caps(8.5), WHITE);
    if tip(nresp, app, "Turns the effect on/off.").clicked() {
        let f = &mut app.px.fx[u].params;
        if single { f.one.on = !sp.on } else { f.slots[s].on = !sp.on }
    }
    let list_r = at(sx + 83.0, 10.0, 17.0, 15.0);
    let lresp = ui.interact(list_r, ui.id().with(("fxlist", u, s)), Sense::click());
    p.rect_filled(list_r, 2.0, if lresp.hovered() { lighten(BTN) } else { BTN });
    crate::icons_player::chevron_down(&p, list_r.center() + vec2(0.0, 0.5), 1.0, WHITE);
    crate::ui_browser::dropdown(ui, &lresp, &format!("fxpop{u}{s}"), |ui| {
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            // (rekordbox draws a line before the last nine, its "FX+ pack".)
            let n = fx::list(single).count();
            for (i, info) in fx::list(single).enumerate() {
                if i + 9 == n {
                    ui.separator();
                }
                if ui.selectable_label(sp.kind == info.kind, info.name).clicked() {
                    app.px.fx[u].set_kind(s, info.kind);
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
    });
    // LEVEL/DEPTH, then `<` value `>`.
    let kc = pos2(o.x + sx + 13.0, o.y + 36.5);
    if let Some(v) = crate::ui_perf::knob(ui, kc, ("fxlevel", u, s), sp.level, 0.5, BLUE) {
        let f = &mut app.px.fx[u].params;
        if single { f.one.level = v } else { f.slots[s].level = v }
    }
    let table = fx::beat_table(sp.kind).0;
    let idx = if single { app.px.fx[u].one_idx } else { app.px.fx[u].idx[s] }.min(table.len().saturating_sub(1));
    let (can_down, can_up) = (idx > 0, idx + 1 < table.len());
    let down = ui.interact(at(sx + 31.0, 28.0, 16.0, 17.0), ui.id().with(("fxdown", u, s)), Sense::click());
    crate::icons_player::chevron_left(&p, down.rect.center(), 1.0, if can_down { WHITE } else { OFF });
    if down.clicked() && can_down {
        app.px.fx[u].set_index(s, idx - 1);
    }
    p.text(pos2(o.x + sx + 66.0, o.y + 36.5), Align2::CENTER_CENTER, fx::value_text(sp.kind, sp.value), bold(9.0), if table.is_empty() { OFF } else { WHITE });
    let up = ui.interact(at(sx + 84.0, 28.0, 16.0, 17.0), ui.id().with(("fxup", u, s)), Sense::click());
    crate::icons_player::chevron_right(&p, up.rect.center(), 1.0, if can_up { WHITE } else { OFF });
    if up.clicked() && can_up {
        app.px.fx[u].set_index(s, idx + 1);
    }
}

/// Single mode's own controls, right of the effect: knob 1, knob 2, button 1, button 2 and
/// the snapshot button. (Laid out without a reference picture.)
fn single_controls(app: &mut App, ui: &mut Ui, u: usize, o: egui::Pos2) {
    let p = ui.painter().clone();
    let sp = app.px.fx[u].params.one;
    let labels = fx::info_of(sp.kind).labels;
    for k in 0..2usize {
        let x = o.x + 193.0 + 112.0 + k as f32 * 68.0;
        let label = labels[k];
        if label.is_empty() {
            p.circle_stroke(pos2(x + 13.0, o.y + 36.5), 9.5, Stroke::new(2.5_f32, hex(0x282828)));
            continue;
        }
        p.text(pos2(x + 30.0, o.y + 17.5), Align2::CENTER_CENTER, label, caps(7.5), WHITE);
        if let Some(v) = crate::ui_perf::knob(ui, pos2(x + 13.0, o.y + 36.5), ("fxknob", u, k), sp.knob[k], 0.5, BLUE) {
            app.px.fx[u].params.one.knob[k] = v;
        }
        p.text(pos2(x + 44.0, o.y + 36.5), Align2::CENTER_CENTER, knob_text(label, sp.knob[k]), bold(9.0), WHITE);
    }
    for k in 0..2usize {
        let r = Rect::from_min_size(pos2(o.x + 193.0 + 250.0, o.y + 10.0 + k as f32 * 19.0), vec2(56.0, 15.0));
        let label = labels[2 + k];
        if label.is_empty() {
            continue;
        }
        let resp = ui.interact(r, ui.id().with(("fxbtn", u, k)), Sense::click());
        p.rect_filled(r, 2.0, if sp.btn[k] { BLUE } else if resp.hovered() { lighten(BTN) } else { BTN });
        p.text(r.center(), Align2::CENTER_CENTER, label, caps(7.5), WHITE);
        if resp.clicked() {
            app.px.fx[u].params.one.btn[k] = !sp.btn[k];
        }
    }
    // Snapshot: keeps the two knobs and the two buttons for this effect (LEVEL/DEPTH is not
    // kept, as in rekordbox). Lit while the controls are as saved.
    let r = Rect::from_min_size(pos2(o.x + 193.0 + 311.0, o.y + 10.0), vec2(24.0, 15.0));
    let now: Snap = (sp.knob[0], sp.knob[1], sp.btn[0], sp.btn[1]);
    let saved = app.px.fx[u].snaps[pos(sp.kind)] == Some(now);
    let resp = ui.interact(r, ui.id().with(("fxsnap", u)), Sense::click());
    p.rect_filled(r, 2.0, if resp.hovered() { lighten(BTN) } else { BTN });
    let c = if saved { hex(0x3C8CFF) } else { WHITE };
    let body = Rect::from_center_size(r.center() + vec2(0.0, 0.5), vec2(12.0, 8.0));
    p.rect_stroke(body, 1.5, Stroke::new(1.0_f32, c), egui::StrokeKind::Inside);
    p.circle_stroke(body.center(), 2.0, Stroke::new(1.0_f32, c));
    p.rect_filled(Rect::from_min_size(pos2(body.min.x + 2.0, body.min.y - 1.5), vec2(4.0, 1.5)), 0.0, c);
    if resp.clicked() {
        app.px.fx[u].snaps[pos(sp.kind)] = Some(now);
    }
    let _ = resp.on_hover_text("Snapshot: saves the positions of the knobs and buttons for this effect.");
}

/// The FX row: FX1 in the left half of `r`, FX2 in the right half.
pub fn panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    let full = r;
    let r = Rect::from_min_size(r.min, vec2(r.width(), BEAT_H - 1.0));
    cfx_row(app, ui, Rect::from_min_max(pos2(full.min.x, full.min.y + BEAT_H), full.max));
    p.rect_filled(r, 0.0, hex(0x1E1E1E));
    p.rect_filled(Rect::from_min_max(pos2(r.center().x - 0.5, r.min.y), pos2(r.center().x + 0.5, r.max.y)), 0.0, BG);
    for u in 0..2usize {
        let x0 = if u == 0 { r.min.x } else { r.center().x };
        let y0 = r.min.y;
        let o = pos2(x0, y0);
        let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x0 + x, y0 + y), vec2(w, h));
        p.text(pos2(x0 + 11.0, y0 + 14.0), Align2::LEFT_CENTER, format!("FX{}", u + 1), bold(11.5), hex(0x8C8C8C));
        // Assign: decks, the sampler (S) and the master (M alone, as in rekordbox: lighting M
        // clears the others, lighting another clears M). Decks 3 and 4 exist in the 4-deck
        // layout.
        let four = app.deck_count() == 4;
        let labels = [("1", Some(0usize)), ("2", Some(1)), ("S", Some(fx::SAMPLER)), ("3", if four { Some(2) } else { None }), ("4", if four { Some(3) } else { None }), ("M", Some(fx::MASTER))];
        for (i, (label, ch)) in labels.iter().enumerate() {
            let br = at(76.0 + (i % 3) as f32 * 21.5, 10.0 + (i / 3) as f32 * 18.0, 18.0, 15.0);
            let lit = ch.map(|c| app.px.fx[u].params.assign[c]).unwrap_or(false);
            let resp = small(ui, br, ("fxassign", u, i), label, lit, ch.is_some());
            if let Some(c) = ch {
                if resp.clicked() {
                    let a = &mut app.px.fx[u].params.assign;
                    a[*c] = !a[*c];
                    if a[*c] {
                        if *c == fx::MASTER {
                            for (i, d) in a.iter_mut().enumerate() {
                                if i != fx::MASTER {
                                    *d = false;
                                }
                            }
                        } else {
                            a[fx::MASTER] = false;
                        }
                    }
                }
            } else {
                let _ = resp.on_hover_text("Decks 3 and 4 are in the 4Deck layout.");
            }
        }
        // Mode: multi (three effects) or single (one effect with five parameters).
        let single = app.px.fx[u].params.single;
        if mode_button(ui, at(158.0, 8.0, 22.0, 17.0), ("fxmulti", u), 3, !single, "Beat FX multi-mode:\nThis mode allows to use up to three effects simultaneously.") {
            app.px.fx[u].params.single = false;
        }
        if mode_button(ui, at(158.0, 29.0, 22.0, 17.0), ("fxsingle", u), 1, single, "Beat FX single-mode:\nThis mode allows to use one effect with five parameters.") {
            app.px.fx[u].params.single = true;
        }
        if single {
            slot(app, ui, u, 3, o, 193.0);
            single_controls(app, ui, u, o);
        } else {
            for s in 0..3usize {
                slot(app, ui, u, s, o, 193.0 + s as f32 * 111.0);
            }
        }

        // RELEASE FX: held while the name is pressed.
        let rk = app.px.fx[u].release_kind.min(2) as usize;
        let rel_r = at(538.0, 10.0, 76.0, 15.0);
        let rresp = ui.interact(rel_r, ui.id().with(("fxrel", u)), Sense::click_and_drag());
        let held = rresp.is_pointer_button_down_on();
        p.rect_filled(rel_r, 2.0, hex(0x141414));
        p.rect_stroke(rel_r, 2.0, Stroke::new(1.0_f32, if held { BLUE } else { hex(0x8C8C8C) }), egui::StrokeKind::Inside);
        p.text(rel_r.center(), Align2::CENTER_CENTER, fx::RELEASE[rk].0, caps(8.0), if held { hex(0x3C8CFF) } else { WHITE });
        let ridx = app.px.fx[u].release_idx[rk].min(fx::RELEASE_BEATS.len() - 1);
        app.px.fx[u].params.release = if held { Some((rk as u8, fx::RELEASE_BEATS[ridx])) } else { None };
        if held {
            ui.ctx().request_repaint();
        }
        let _ = rresp.on_hover_text("RELEASE FX: on while the button is held.");
        let rl = at(617.0, 10.0, 17.0, 15.0);
        let rlresp = ui.interact(rl, ui.id().with(("fxrellist", u)), Sense::click());
        p.rect_filled(rl, 2.0, if rlresp.hovered() { lighten(BTN) } else { BTN });
        crate::icons_player::chevron_down(&p, rl.center() + vec2(0.0, 0.5), 1.0, WHITE);
        crate::ui_browser::dropdown(ui, &rlresp, &format!("fxrelpop{u}"), |ui| {
            for (i, (name, _)) in fx::RELEASE.iter().enumerate() {
                if ui.selectable_label(rk == i, *name).clicked() {
                    app.px.fx[u].release_kind = i as u8;
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
        let rd = ui.interact(at(544.0, 28.0, 16.0, 17.0), ui.id().with(("fxreldown", u)), Sense::click());
        crate::icons_player::chevron_left(&p, rd.rect.center(), 1.0, if ridx > 0 { WHITE } else { OFF });
        if rd.clicked() && ridx > 0 {
            app.px.fx[u].release_idx[rk] = ridx - 1;
        }
        p.text(pos2(x0 + 580.0, y0 + 36.5), Align2::CENTER_CENTER, fx::beat_text(fx::RELEASE_BEATS[ridx], false), bold(9.0), WHITE);
        let ru = ui.interact(at(595.0, 28.0, 16.0, 17.0), ui.id().with(("fxrelup", u)), Sense::click());
        crate::icons_player::chevron_right(&p, ru.rect.center(), 1.0, if ridx + 1 < fx::RELEASE_BEATS.len() { WHITE } else { OFF });
        if ru.clicked() && ridx + 1 < fx::RELEASE_BEATS.len() {
            app.px.fx[u].release_idx[rk] = ridx + 1;
        }

        // FX tempo: AUTO (each channel's own tempo) or TAP.
        let auto = app.px.fx[u].auto;
        let ar = at(645.0, 7.0, 104.0, 17.0);
        let aresp = ui.interact(ar, ui.id().with(("fxauto", u)), Sense::click());
        p.rect_filled(ar, 2.0, if auto { BLUE } else if aresp.hovered() { lighten(BTN) } else { hex(0x141414) });
        if !auto {
            p.rect_stroke(ar, 2.0, Stroke::new(1.0_f32, hex(0x8C8C8C)), egui::StrokeKind::Inside);
        }
        p.text(ar.center(), Align2::CENTER_CENTER, "AUTO", caps(8.0), WHITE);
        if aresp.clicked() {
            app.px.fx[u].auto = true;
        }
        let tr = at(645.0, 28.0, 104.0, 17.0);
        let tresp = ui.interact(tr, ui.id().with(("fxtap", u)), Sense::click());
        p.rect_filled(tr, 2.0, if !auto { BLUE } else if tresp.hovered() { lighten(BTN) } else { hex(0x141414) });
        if auto {
            p.rect_stroke(tr, 2.0, Stroke::new(1.0_f32, hex(0x8C8C8C)), egui::StrokeKind::Inside);
        }
        let label = if auto { "TAP".to_string() } else { format!("TAP  {:.1}", app.px.fx[u].tap_bpm) };
        p.text(tr.center(), Align2::CENTER_CENTER, label, caps(8.0), WHITE);
        if crate::ui_player::press_fired(ui, &tresp) {
            let now = ui.input(|i| i.time);
            let f = &mut app.px.fx[u];
            if f.auto {
                f.auto = false;
            }
            if let Some(b) = f.tap.tap(now) {
                f.tap_bpm = b;
            }
        }
    }
}

/// A COLOR knob (centre = off, with a small catch so it comes to rest there).
fn color_knob(app: &mut App, ui: &mut Ui, kc: egui::Pos2, c: usize) {
    let v = if c == fx::SAMPLER { app.px.sampler.cfx_color } else { app.px.cfx_color[c] };
    if let Some(nv) = crate::ui_perf::knob(ui, kc, ("cfxcolor", c), v, 0.5, if (v - 0.5).abs() > 0.012 { BLUE } else { WHITE }) {
        let nv = if (nv - 0.5).abs() < 0.015 { 0.5 } else { nv };
        if c == fx::SAMPLER {
            app.px.sampler.cfx_color = nv;
        } else {
            app.px.cfx_color[c] = nv;
        }
    }
}

/// The CFX row. Single mode: DEFAULT / USER, the effect, PARAM, and a COLOR knob per channel
/// in rekordbox's order S, 3, 1, 2, 4, M (measured on reference performance/06 at 1512 pt).
/// Multi mode: per channel an effect (name = on/off, list) and its COLOR knob.
fn cfx_row(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, hex(0x1E1E1E));
    let (x0, cy) = (r.min.x, r.center().y);
    let k = r.width() / 1512.0;
    let at = |x: f32, w: f32, h: f32| Rect::from_center_size(pos2(x0 + x * k, cy), vec2(w, h));
    p.text(pos2(x0 + 11.0, cy), Align2::LEFT_CENTER, "CFX", bold(11.5), hex(0x8C8C8C));
    let multi = app.px.fxx.cfx_multi;
    if mode_button(ui, at(92.0, 22.0, 15.0), "cfxmulti", 3, multi, "CFX multi-mode: an effect for each channel.") {
        app.px.fxx.cfx_multi = true;
    }
    if mode_button(ui, at(116.0, 22.0, 15.0), "cfxsingle", 1, !multi, "CFX single-mode: one effect for all channels.") {
        app.px.fxx.cfx_multi = false;
    }
    // S (the sampler deck) is live when "Apply CFX to Sampler" is ticked in Preferences >
    // Controller > Effect; decks 3 and 4 in the 4-deck layout.
    let four = app.deck_count() == 4;
    let smp = if app.prefs.sampler.cfx_on_sampler { Some(fx::SAMPLER) } else { None };
    let channels = [("S", 647.0, smp), ("3", 742.0, if four { Some(2usize) } else { None }), ("1", 836.0, Some(0)), ("2", 929.0, Some(1)), ("4", 1024.0, if four { Some(3) } else { None }), ("M", 1119.0, Some(4))];
    if multi {
        // Six cells across the row (laid out without a reference picture).
        for (i, (label, _, ch)) in channels.into_iter().enumerate() {
            let cx = 150.0 + i as f32 * 224.0;
            p.text(pos2(x0 + (cx + 8.0) * k, cy), Align2::CENTER_CENTER, label, caps(8.5), if ch.is_some() { WHITE } else { OFF });
            let kc = pos2(x0 + (cx + 150.0) * k, cy);
            let Some(c) = ch else {
                p.circle_stroke(kc, 9.5, Stroke::new(2.5_f32, hex(0x282828)));
                continue;
            };
            let kind = app.px.fxx.cfx_multi_kind[c];
            let on = app.px.fxx.cfx_multi_on[c];
            let er = at(cx + 62.0, 80.0, 15.0);
            let eresp = ui.interact(er, ui.id().with(("cfxmname", c)), Sense::click());
            p.rect_filled(er, 2.0, if on { BLUE } else if eresp.hovered() { lighten(BTN) } else { BTN });
            p.text(er.center(), Align2::CENTER_CENTER, crate::cfx::name(kind), caps(8.0), WHITE);
            if eresp.clicked() {
                app.px.fxx.cfx_multi_on[c] = !on;
            }
            let _ = eresp.on_hover_text("Turns the effect on/off.");
            let lr = at(cx + 113.0, 17.0, 15.0);
            let lresp = ui.interact(lr, ui.id().with(("cfxmlist", c)), Sense::click());
            p.rect_filled(lr, 2.0, if lresp.hovered() { lighten(BTN) } else { BTN });
            crate::icons_player::chevron_down(&p, lr.center() + vec2(0.0, 0.5), 1.0, WHITE);
            crate::ui_browser::dropdown(ui, &lresp, &format!("cfxmpop{c}"), |ui| {
                for (name, kd) in crate::cfx::LIST {
                    let Some(kd) = kd else { continue };
                    if ui.selectable_label(kind == kd, name).clicked() {
                        app.px.fxx.cfx_multi_kind[c] = kd;
                        ui.memory_mut(|m| m.close_popup());
                    }
                }
            });
            color_knob(app, ui, kc, c);
        }
        return;
    }
    // DEFAULT (the mixer's effects: FILTER without hardware) / USER (the chosen effect).
    let user = app.px.cfx_user;
    let dr = at(218.5, 78.0, 15.0);
    let dresp = ui.interact(dr, ui.id().with("cfxdefault"), Sense::click());
    p.rect_filled(dr, 2.0, if !user { BLUE } else { hex(0x141414) });
    if user {
        p.rect_stroke(dr, 2.0, Stroke::new(1.0_f32, hex(0x8C8C8C)), egui::StrokeKind::Inside);
    }
    p.text(dr.center(), Align2::CENTER_CENTER, "DEFAULT", caps(8.0), WHITE);
    if dresp.clicked() {
        app.px.cfx_user = false;
        app.px.cfx_param = app.px.fxx.cfx_params[crate::cfx::DEFAULT_LIST[0] as usize];
    }
    let ur = at(298.0, 78.0, 15.0);
    let uresp = ui.interact(ur, ui.id().with("cfxuser"), Sense::click());
    p.rect_filled(ur, 2.0, if user { BLUE } else { hex(0x141414) });
    if !user {
        p.rect_stroke(ur, 2.0, Stroke::new(1.0_f32, hex(0x8C8C8C)), egui::StrokeKind::Inside);
    }
    p.text(ur.center(), Align2::CENTER_CENTER, "USER", caps(8.0), WHITE);
    if uresp.clicked() {
        app.px.cfx_user = true;
        app.px.cfx_param = app.px.fxx.cfx_params[app.px.cfx_kind as usize % 9];
    }
    // The effect: FILTER in DEFAULT; in USER a click lists the nine types.
    let kind = if user { app.px.cfx_kind } else { crate::cfx::DEFAULT_LIST[0] };
    let er = at(414.0, 68.0, 15.0);
    let eresp = ui.interact(er, ui.id().with("cfxeffect"), Sense::click());
    p.rect_filled(er, 2.0, hex(0x141414));
    p.rect_stroke(er, 2.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
    p.text(er.center(), Align2::CENTER_CENTER, crate::cfx::name(kind), caps(8.0), WHITE);
    if user {
        crate::ui_browser::dropdown(ui, &eresp, "cfxpop", |ui| {
            for (name, kd) in crate::cfx::LIST {
                let Some(kd) = kd else { continue };
                if ui.selectable_label(kind == kd, name).clicked() {
                    app.px.cfx_kind = kd;
                    // (Each effect keeps its own PARAM.)
                    app.px.cfx_param = app.px.fxx.cfx_params[kd as usize % 9];
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
    }
    p.text(pos2(x0 + 543.0 * k, cy), Align2::CENTER_CENTER, "PARAM", caps(8.0), WHITE);
    if let Some(v) = crate::ui_perf::knob(ui, pos2(x0 + 580.0 * k, cy), "cfxparam", app.px.cfx_param, 0.5, BLUE) {
        app.px.cfx_param = v;
        app.px.fxx.cfx_params[kind as usize % 9] = v;
    }
    for (label, x, ch) in channels {
        p.text(pos2(x0 + x * k, cy), Align2::CENTER_CENTER, label, caps(8.5), if ch.is_some() { WHITE } else { OFF });
        let kc = pos2(x0 + (x + 27.0) * k, cy);
        match ch {
            Some(c) => color_knob(app, ui, kc, c),
            None => {
                p.circle_stroke(kc, 9.5, Stroke::new(2.5_f32, hex(0x282828)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panels_choices_survive_a_round_trip() {
        // Defaults as rekordbox's file: DELAY / ECHO / SPIRAL by engine id 0 / 1 / 3, every
        // beat index at its table default, RELEASE FX 0 with 4 / 5 / 7.
        let d = UnitSaved::default();
        assert_eq!(d.multi_fx, [0, 1, 3]);
        assert_eq!((d.release_fx, d.release_beat), (0, [4, 5, 7]));
        assert_eq!(d.single_beat.len(), 34);
        assert_eq!(d.multi_beat[0].len(), 35);
        // Single ids: 0 DELAY (B, 5), 4 REVERB (P1, 3), 6 FILTER (D, 6), 26 ENIGMA JET (S, 17),
        // 32 VINYL BRAKE (C, 6), 33 STRETCH (C, 5), 12 ROBOT (P2, 3), 16 PITCH (P2, 0).
        for (id, want) in [(0, 5), (4, 3), (6, 6), (26, 17), (32, 6), (33, 5), (12, 3), (16, 0)] {
            assert_eq!(d.single_beat[id], want, "single id {id}");
        }
        // Change everything that is kept, write it as JSON, read it back into a new unit.
        let mut u = FxUi::new();
        u.set_kind(0, Kind::Helix);
        u.set_index(0, 2);
        u.set_kind(1, Kind::EnigmaJet);
        u.set_index(1, 3);
        u.set_kind(2, Kind::ReverbUp);
        u.set_kind(3, Kind::PitchEcho);
        u.set_index(3, 8);
        u.params.one.knob = [0.2, 0.9];
        u.params.one.btn = [true, false];
        u.snaps[pos(Kind::PitchEcho)] = Some((0.2, 0.9, true, false));
        u.params.single = true;
        u.release_kind = 2;
        u.release_idx = [1, 2, 9];
        // Going to another effect and back keeps the first one's beat value.
        u.set_kind(0, Kind::Trans);
        u.set_kind(0, Kind::Helix);
        assert_eq!(u.idx[0], 2);
        let all = FxSaved { units: [u.saved(), UnitSaved::default()], cfx: CfxSaved { multi_shown: true, user: true, user_fx: 6, params: [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9], multi_fx: [0, 1, 2, 3, 4, 8] } };
        let json = serde_json::to_string(&all).unwrap();
        let back: FxSaved = serde_json::from_str(&json).unwrap();
        assert_eq!(back, all);
        let mut v = FxUi::new();
        v.load(&back.units[0]);
        assert_eq!(v.params.slots.map(|s| s.kind), [Kind::Helix, Kind::EnigmaJet, Kind::ReverbUp]);
        assert_eq!(v.idx, [2, 3, 0]);
        assert_eq!(v.params.slots[1].value, -8.0);
        assert!(v.params.single);
        assert_eq!((v.params.one.kind, v.one_idx, v.params.one.value), (Kind::PitchEcho, 8, 8.0));
        assert_eq!((v.params.one.knob, v.params.one.btn), ([0.2, 0.9], [true, false]));
        assert_eq!((v.release_kind, v.release_idx), (2, [1, 2, 9]));
        assert_eq!(v.saved(), all.units[0]);
        // Nothing switched on by loading.
        assert!(!v.params.one.on && v.params.slots.iter().all(|s| !s.on));
        // An older settings file without the FX part, and one with a broken part, load.
        let old: FxSaved = serde_json::from_str("{}").unwrap();
        assert_eq!(old, FxSaved::default());
        let odd: FxSaved = serde_json::from_str(r#"{"units":[{"single_fx":99,"multi_fx":[-1,77,5],"multi_beat":[[200],[],[]]},{}]}"#).unwrap();
        let mut w = FxUi::new();
        w.load(&odd.units[0]);
        assert_eq!(w.params.slots.map(|s| s.kind), [Kind::Delay, Kind::Echo, Kind::Trans]);
        assert_eq!(w.idx[0], 9);
    }

    #[test]
    fn single_mode_read_outs() {
        assert_eq!(knob_text("FEEDBACK", 0.5), "50");
        assert_eq!(knob_text("PITCH", 0.5), "0");
        assert_eq!(knob_text("PITCH", 0.505), "0");
        assert_eq!(knob_text("PITCH", 1.0), "+100");
        assert_eq!(knob_text("PITCH", 0.0), "-100");
        assert_eq!(knob_text("STAGE", 0.5), "6");
        assert_eq!(knob_text("STAGE", 0.0), "2");
    }
}
