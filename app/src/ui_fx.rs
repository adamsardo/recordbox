//! The FX panel of PERFORMANCE mode: FX1 and FX2, each with its assign buttons, three effect
//! slots (name = on/off, list, LEVEL/DEPTH, `<` value `>`), RELEASE FX (press and hold) and
//! the FX tempo (AUTO / TAP). Positions measured on rekordbox 7.2.19 at 1512 pt
//! (`rekordbox-reference/performance/06-fx.jpg`): the row is 52 pt tall, a unit 756 pt wide.

use crate::app::*;
use crate::fx::{self, Kind, UnitParams};
use crate::theme::*;
use crate::ui_player::{lighten, tip};
use eframe::egui;
use egui::{pos2, vec2, Align2, Rect, Sense, Stroke, Ui};

/// Height of the FX panel: the Beat FX row (52 pt) and the CFX row (26 pt).
pub const FX_H: f32 = 78.0;
const BEAT_H: f32 = 52.0;

/// One FX unit as the interface holds it.
pub struct FxUi {
    pub params: UnitParams,
    /// Index into each slot's beat table.
    pub idx: [usize; 3],
    pub release_kind: u8,
    /// Index into the RELEASE FX beat table, kept per RELEASE FX.
    pub release_idx: [usize; 3],
    /// FX tempo: AUTO (the decks' own) or TAP.
    pub auto: bool,
    pub tap: fx::Tap,
    pub tap_bpm: f32,
}

impl FxUi {
    pub fn new() -> FxUi {
        let params = UnitParams::new();
        let idx = [fx::beat_table(params.slots[0].kind).1, fx::beat_table(params.slots[1].kind).1, fx::beat_table(params.slots[2].kind).1];
        FxUi { params, idx, release_kind: 0, release_idx: [fx::RELEASE[0].1, fx::RELEASE[1].1, fx::RELEASE[2].1], auto: true, tap: fx::Tap::new(), tap_bpm: 120.0 }
    }
}

/// Once per frame: hand the units' settings and each channel's tempo to the audio thread.
/// The rack (its delay lines) is made the first time the FX panel is used.
pub fn sync(app: &mut App) {
    let on = app.performance() && app.prefs.perf_fx;
    if !on {
        if let Some(mut g) = fx::RACK.try_lock() {
            if let Some(r) = g.as_mut() {
                r.enabled = false;
            }
        }
        return;
    }
    // Tempo per channel: each deck's own, and the master deck's for the master.
    let mut bpm = [120.0f32; fx::CHANNELS];
    for n in 0..app.deck_count() {
        let b = app.with_deck(n, |a| if a.deck.track.is_some() { a.effective_bpm() } else { 0.0 });
        if b > 0.0 {
            bpm[n as usize] = b as f32;
        }
    }
    bpm[fx::MASTER] = bpm[app.px.master.min(3) as usize];
    let mut g = fx::RACK.lock();
    let rack = g.get_or_insert_with(|| Box::new(fx::Rack::new()));
    rack.enabled = true;
    rack.master_cfx_set = (if app.px.cfx_user { app.px.cfx_kind } else { 0 }, app.px.cfx_color[4], app.px.cfx_param);
    for (u, ui) in rack.units.iter_mut().zip(app.px.fx.iter()) {
        let mut p = ui.params;
        p.bpm = if ui.auto { bpm } else { [ui.tap_bpm; fx::CHANNELS] };
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
        let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x0 + x, y0 + y), vec2(w, h));
        p.text(pos2(x0 + 11.0, y0 + 14.0), Align2::LEFT_CENTER, format!("FX{}", u + 1), bold(11.5), hex(0x8C8C8C));
        // Assign: decks 1 and 2 and the master (M alone, as in rekordbox: lighting M clears
        // the decks, lighting a deck clears M). Decks 3 and 4 and the sampler do not exist.
        let four = app.deck_count() == 4;
        let labels = [("1", Some(0usize)), ("2", Some(1)), ("S", None), ("3", if four { Some(2) } else { None }), ("4", if four { Some(3) } else { None }), ("M", Some(fx::MASTER))];
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
                            for d in a.iter_mut().take(4) {
                                *d = false;
                            }
                        } else {
                            a[fx::MASTER] = false;
                        }
                    }
                }
            } else {
                let _ = resp.on_hover_text("Not built in recordbox yet.");
            }
        }
        // Mode: multi (three effects) is what exists; single mode is not built.
        let multi = at(158.0, 8.0, 22.0, 17.0);
        p.rect_filled(multi, 2.0, BLUE);
        for k in -1..=1 {
            p.circle_filled(multi.center() + vec2(k as f32 * 5.0, 0.0), 1.5, WHITE);
        }
        let _ = ui.interact(multi, ui.id().with(("fxmulti", u)), Sense::hover()).on_hover_text("Beat FX multi-mode:\nThis mode allows to use up to three effects simultaneously.");
        let single = at(158.0, 29.0, 22.0, 17.0);
        p.rect_stroke(single, 2.0, Stroke::new(1.0_f32, hex(0x646464)), egui::StrokeKind::Inside);
        p.circle_filled(single.center(), 1.5, hex(0x8C8C8C));
        let _ = ui.interact(single, ui.id().with(("fxsingle", u)), Sense::hover()).on_hover_text("Beat FX single-mode:\nThis mode allows to use one effect with five parameters.\n(Not built in recordbox yet.)");

        // The three slots.
        for s in 0..3usize {
            let sx = 193.0 + s as f32 * 111.0;
            let sp = app.px.fx[u].params.slots[s];
            let name_r = at(sx, 10.0, 80.0, 15.0);
            let nresp = ui.interact(name_r, ui.id().with(("fxname", u, s)), Sense::click());
            p.rect_filled(name_r, 2.0, if sp.on { BLUE } else if nresp.hovered() { lighten(BTN) } else { BTN });
            p.text(name_r.center(), Align2::CENTER_CENTER, fx::name(sp.kind), caps(8.5), WHITE);
            if tip(nresp, app, "Turns the effect on/off.").clicked() {
                app.px.fx[u].params.slots[s].on = !sp.on;
            }
            let list_r = at(sx + 83.0, 10.0, 17.0, 15.0);
            let lresp = ui.interact(list_r, ui.id().with(("fxlist", u, s)), Sense::click());
            p.rect_filled(list_r, 2.0, if lresp.hovered() { lighten(BTN) } else { BTN });
            crate::icons_player::chevron_down(&p, list_r.center() + vec2(0.0, 0.5), 1.0, WHITE);
            crate::ui_browser::dropdown(ui, &lresp, &format!("fxpop{u}{s}"), |ui| {
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    for (i, (name, kind)) in fx::LIST.iter().enumerate() {
                        if i == 26 {
                            ui.separator();
                        }
                        match kind {
                            Some(k) => {
                                if ui.selectable_label(sp.kind == *k, *name).clicked() {
                                    set_kind(app, u, s, *k);
                                    ui.memory_mut(|m| m.close_popup());
                                }
                            }
                            None => {
                                ui.add_enabled(false, egui::SelectableLabel::new(false, *name)).on_disabled_hover_text("Not built in recordbox yet.");
                            }
                        }
                    }
                });
            });
            // LEVEL/DEPTH, then `<` value `>`.
            let kc = pos2(x0 + sx + 13.0, y0 + 36.5);
            if let Some(v) = crate::ui_perf::knob(ui, kc, ("fxlevel", u, s), sp.level, 0.5, BLUE) {
                app.px.fx[u].params.slots[s].level = v;
            }
            let (table, _, percent) = fx::beat_table(sp.kind);
            let idx = app.px.fx[u].idx[s].min(table.len() - 1);
            let down = ui.interact(at(sx + 31.0, 28.0, 16.0, 17.0), ui.id().with(("fxdown", u, s)), Sense::click());
            crate::icons_player::chevron_left(&p, down.rect.center(), 1.0, if idx > 0 { WHITE } else { OFF });
            if down.clicked() && idx > 0 {
                set_index(app, u, s, idx - 1);
            }
            p.text(pos2(x0 + sx + 66.0, y0 + 36.5), Align2::CENTER_CENTER, fx::beat_text(table[idx], percent), bold(9.0), WHITE);
            let up = ui.interact(at(sx + 84.0, 28.0, 16.0, 17.0), ui.id().with(("fxup", u, s)), Sense::click());
            crate::icons_player::chevron_right(&p, up.rect.center(), 1.0, if idx + 1 < table.len() { WHITE } else { OFF });
            if up.clicked() && idx + 1 < table.len() {
                set_index(app, u, s, idx + 1);
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

fn set_kind(app: &mut App, u: usize, s: usize, k: Kind) {
    let (table, def, _) = fx::beat_table(k);
    let f = &mut app.px.fx[u];
    f.params.slots[s].kind = k;
    f.params.slots[s].on = false;
    f.idx[s] = def;
    f.params.slots[s].value = table[def];
}

fn set_index(app: &mut App, u: usize, s: usize, idx: usize) {
    let f = &mut app.px.fx[u];
    let table = fx::beat_table(f.params.slots[s].kind).0;
    f.idx[s] = idx.min(table.len() - 1);
    f.params.slots[s].value = table[f.idx[s]];
}

/// The CFX row (single mode): DEFAULT / USER, the effect, PARAM, and a COLOR knob per channel
/// in rekordbox's order S, 3, 1, 2, 4, M. Measured on reference performance/06 at 1512 pt.
fn cfx_row(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, hex(0x1E1E1E));
    let (x0, cy) = (r.min.x, r.center().y);
    let k = r.width() / 1512.0;
    let at = |x: f32, w: f32, h: f32| Rect::from_center_size(pos2(x0 + x * k, cy), vec2(w, h));
    p.text(pos2(x0 + 11.0, cy), Align2::LEFT_CENTER, "CFX", bold(11.5), hex(0x8C8C8C));
    // Mode: single (one effect for all channels) is what exists.
    let multi = at(92.0, 22.0, 15.0);
    p.rect_stroke(multi, 2.0, Stroke::new(1.0_f32, hex(0x646464)), egui::StrokeKind::Inside);
    for d in -1..=1 {
        p.circle_filled(multi.center() + vec2(d as f32 * 5.0, 0.0), 1.5, hex(0x8C8C8C));
    }
    let _ = ui.interact(multi, ui.id().with("cfxmulti"), Sense::hover()).on_hover_text("CFX multi-mode: an effect per channel.\n(Not built in recordbox yet.)");
    let single = at(116.0, 22.0, 15.0);
    p.rect_filled(single, 2.0, BLUE);
    p.circle_filled(single.center(), 1.5, WHITE);
    // DEFAULT (FILTER) / USER (the chosen effect).
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
    }
    // The effect: FILTER in DEFAULT; in USER a click lists the nine types.
    let kind = if user { app.px.cfx_kind } else { 0 };
    let er = at(414.0, 68.0, 15.0);
    let eresp = ui.interact(er, ui.id().with("cfxeffect"), Sense::click());
    p.rect_filled(er, 2.0, hex(0x141414));
    p.rect_stroke(er, 2.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
    p.text(er.center(), Align2::CENTER_CENTER, crate::cfx::name(kind), caps(8.0), WHITE);
    if user {
        crate::ui_browser::dropdown(ui, &eresp, "cfxpop", |ui| {
            for (name, k) in crate::cfx::LIST {
                match k {
                    Some(k) => {
                        if ui.selectable_label(kind == k, name).clicked() {
                            app.px.cfx_kind = k;
                            ui.memory_mut(|m| m.close_popup());
                        }
                    }
                    None => {
                        ui.add_enabled(false, egui::SelectableLabel::new(false, name)).on_disabled_hover_text("Not built in recordbox yet.");
                    }
                }
            }
        });
    }
    p.text(pos2(x0 + 543.0 * k, cy), Align2::CENTER_CENTER, "PARAM", caps(8.0), WHITE);
    if let Some(v) = crate::ui_perf::knob(ui, pos2(x0 + 580.0 * k, cy), "cfxparam", app.px.cfx_param, 0.5, BLUE) {
        app.px.cfx_param = v;
    }
    // COLOR knobs. Decks 1 and 2 and the master are live.
    let four = app.deck_count() == 4;
    for (label, x, ch) in [("S", 647.0, None), ("3", 742.0, if four { Some(2usize) } else { None }), ("1", 836.0, Some(0)), ("2", 929.0, Some(1)), ("4", 1024.0, if four { Some(3) } else { None }), ("M", 1119.0, Some(4))] {
        p.text(pos2(x0 + x * k, cy), Align2::CENTER_CENTER, label, caps(8.5), if ch.is_some() { WHITE } else { OFF });
        let kc = pos2(x0 + (x + 27.0) * k, cy);
        match ch {
            Some(c) => {
                let v = app.px.cfx_color[c];
                if let Some(nv) = crate::ui_perf::knob(ui, kc, ("cfxcolor", c), v, 0.5, if (v - 0.5).abs() > 0.012 { BLUE } else { WHITE }) {
                    // A small catch at the centre, so the knob comes to rest on "off".
                    app.px.cfx_color[c] = if (nv - 0.5).abs() < 0.015 { 0.5 } else { nv };
                }
            }
            None => {
                p.circle_stroke(kc, 9.5, Stroke::new(2.5_f32, hex(0x282828)));
            }
        }
    }
}
