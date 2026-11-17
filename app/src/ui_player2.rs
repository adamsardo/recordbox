//! Player UI beyond the 1 Player panel in ui_player.rs: the enlarged waveform's right-click
//! menu, the GRID panel, loop size and beat jump lists, the 2 Players layout (decks A and B
//! side by side) and the Simple Player row.

use crate::app::*;
use crate::model::*;
use crate::player_ext::{beats_text, JUMP_BARS, JUMP_BEATS, LOOP_SIZES};
use crate::theme::*;
use crate::ui_player::{bold_button, button, close_cross, draw_detail, draw_note_icon, draw_overview, hot_cue_menu, icon_button, lighten, seg_button, split_time, tip};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui};

/// Height of the Simple Player row.
pub const SIMPLE_HEIGHT: f32 = 58.0;

// ---------------------------------------------------------------------- menus

/// Right-click menu of the enlarged waveform; everything acts at `t` (the clicked position).
pub fn wave_menu(app: &mut App, ui: &mut Ui, t: f64) {
    ui.set_min_width(250.0);
    let has_loop = app.defined_loop().is_some();
    if ui.button("Set Cue(In) Point").clicked() {
        app.set_cue_at(t);
        ui.close_menu();
    }
    if ui.button("Set Out Point").clicked() {
        app.set_out_at(t);
        ui.close_menu();
    }
    ui.separator();
    for slot in 0..4 {
        if ui.button(format!("Set Hot Cue {}", (b'A' + slot as u8) as char)).clicked() {
            app.set_hot_cue_at(slot, t);
            ui.close_menu();
        }
    }
    for slot in 0..4 {
        if ui.add_enabled(has_loop, egui::Button::new(format!("Set Hot Loop {}", (b'A' + slot as u8) as char))).clicked() {
            app.set_hot_loop(slot);
            ui.close_menu();
        }
    }
    if ui.add_enabled(has_loop, egui::Button::new("Set Memory Loop")).clicked() {
        app.set_memory_loop();
        ui.close_menu();
    }
    if ui.add_enabled(has_loop, egui::Button::new("Export Loop As Wav")).clicked() {
        ui.close_menu();
        app.export_loop_wav();
    }
    ui.separator();
    ui.menu_button("Beat Count Display", |ui| {
        if beat_count_items(app, ui) {
            ui.close_menu();
        }
    });
    let mut on = app.prefs.wave_click_play_cue;
    if ui.checkbox(&mut on, "Click on the waveform for PLAY and CUE").clicked() {
        app.prefs.wave_click_play_cue = on;
        app.save_prefs();
        ui.close_menu();
    }
    ui.menu_button("Change waveform color", |ui| {
        if wave_color_items(app, ui) {
            ui.close_menu();
        }
    });
}

/// The three Beat Count Display choices; true when one was picked.
fn beat_count_items(app: &mut App, ui: &mut Ui) -> bool {
    let mut picked = false;
    for (i, name) in ["Current Position (Bars)", "Count to the next MEMORY CUE (Bars)", "Count to the next MEMORY CUE (Beats)"].iter().enumerate() {
        if ui.radio(app.prefs.beat_count_mode == i as u8, *name).clicked() {
            app.prefs.beat_count_mode = i as u8;
            app.save_prefs();
            picked = true;
        }
    }
    picked
}

fn wave_color_items(app: &mut App, ui: &mut Ui) -> bool {
    let mut picked = false;
    for (i, name) in ["BLUE", "RGB", "3Band"].iter().enumerate() {
        if ui.radio(app.prefs.wave_color.min(2) == i as u8, *name).clicked() {
            app.prefs.wave_color = i as u8;
            app.save_prefs();
            picked = true;
        }
    }
    picked
}

/// The rest of the ≡ menu of the CUE/LOOP strip: the waveform menu's settings (reachable
/// here when right-click is CUE), loop export, mute and the Recordbox extras.
pub fn strip_menu_extra(app: &mut App, ui: &mut Ui) {
    let has_loop = app.defined_loop().is_some();
    ui.separator();
    if ui.add_enabled(has_loop, egui::Button::new("Set Memory Loop")).clicked() {
        app.set_memory_loop();
    }
    if ui.add_enabled(has_loop, egui::Button::new("Export Loop As Wav")).clicked() {
        app.export_loop_wav();
    }
    ui.separator();
    ui.label(egui::RichText::new("Beat Count Display").size(10.5).color(DIM));
    beat_count_items(app, ui);
    ui.separator();
    let mut on = app.prefs.wave_click_play_cue;
    if ui.checkbox(&mut on, "Click on the waveform for PLAY and CUE").clicked() {
        app.prefs.wave_click_play_cue = on;
        app.save_prefs();
    }
    ui.separator();
    ui.label(egui::RichText::new("Change waveform color").size(10.5).color(DIM));
    wave_color_items(app, ui);
    ui.separator();
    let mut m = app.px.muted;
    if ui.checkbox(&mut m, "Mute").clicked() {
        app.toggle_mute();
    }
    ui.separator();
    ui.label(egui::RichText::new("Recordbox extras").size(10.5).color(DIM));
    let mut changed = ui.checkbox(&mut app.prefs.show_stems, "STEMS button").clicked();
    changed |= ui.checkbox(&mut app.prefs.show_phrase_bar, "Phrase bar").clicked();
    changed |= ui.checkbox(&mut app.prefs.show_vocal, "Vocal positions").clicked();
    if changed {
        app.save_prefs();
    }
}

/// Auto beat loop sizes: picking one starts a loop of that length.
pub fn loop_size_menu(app: &mut App, ui: &mut Ui) {
    egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
        for v in LOOP_SIZES {
            if ui.selectable_label(app.prefs.loop_beats == v, format!("{} Beat Loop", beats_text(v))).clicked() {
                app.pick_auto_loop(v);
                ui.memory_mut(|m| m.close_popup());
            }
        }
    });
}

/// Beat jump lengths, in beats and in bars.
pub fn jump_size_menu(app: &mut App, ui: &mut Ui) {
    let pick = |app: &mut App, beats: f64, bars: bool| {
        app.prefs.beat_jump = beats;
        app.prefs.beat_jump_bars = bars;
        app.save_prefs();
    };
    let cur = (app.prefs.beat_jump, app.prefs.beat_jump_bars);
    for v in JUMP_BEATS {
        let l = if v <= 1.0 { format!("{} Beat", beats_text(v)) } else { format!("{v} Beats") };
        if ui.selectable_label(cur == (v, false), l).clicked() {
            pick(app, v, false);
        }
    }
    ui.separator();
    for v in JUMP_BARS {
        if ui.selectable_label(cur == (v * 4.0, true), format!("{} {}", v, if v == 1.0 { "Bar" } else { "Bars" })).clicked() {
            pick(app, v * 4.0, true);
        }
    }
}

/// Manual loop buttons IN, OUT and RELOOP/EXIT. `at(x, w)` places a button `w` wide centred
/// on the 1 Player strip's design x (IN 968, OUT 1005, RELOOP/EXIT 1060). During a loop IN
/// and OUT switch loop adjust for their edge on and off (the button then blinks).
pub fn manual_loop_buttons(app: &mut App, ui: &mut Ui, at: impl Fn(f32, f32) -> Rect) {
    let active = app.loop_active().is_some();
    let adj = app.deck.x.loop_adjust;
    let blink = (ui.input(|i| i.time) * 2.5) as i64 % 2 == 0;
    let fill = |on: bool, adjusting: bool| {
        if adjusting {
            if blink {
                LOOP
            } else {
                LOOP.gamma_multiply(0.4)
            }
        } else if on {
            LOOP.gamma_multiply(0.7)
        } else {
            BTN
        }
    };
    let in_on = active || app.deck.loop_in.is_some();
    if tip(button(ui, at(968.0, 34.0), "lin", "IN", fill(in_on, adj == 1), WHITE, 9.5), app, "Manual Loop:\nSet a Loop In point.").clicked() {
        app.loop_in_pressed();
    }
    if tip(button(ui, at(1005.0, 34.0), "lout", "OUT", fill(active, adj == 2), WHITE, 9.5), app, "Manual Loop:\nSet a Loop Out point.").clicked() {
        app.loop_out_pressed();
    }
    if tip(button(ui, at(1060.0, 70.0), "lre", "RELOOP/EXIT", fill(false, false), if active || app.deck.last_loop.is_some() { WHITE } else { OFF }, 9.5), app, "Back to Loop playback (Reloop), or cancel Loop playback (Loop Exit).").clicked() {
        app.exit_reloop();
    }
    if adj != 0 {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// A track dragged from a list and released over `rect` is loaded into the active deck.
pub fn drop_to_load(app: &mut App, ui: &mut Ui, rect: Rect) {
    let Some(ids) = app.drag.clone() else { return };
    let Some(pp) = ui.ctx().pointer_latest_pos() else { return };
    if !rect.contains(pp) {
        return;
    }
    ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, BLUE), egui::StrokeKind::Inside);
    if ui.input(|i| i.pointer.any_released()) {
        if let Some(id) = ids.first() {
            app.load_track_here(*id);
        }
    }
}

// ---------------------------------------------------------------------- GRID panel

/// Icons of the GRID panel, drawn after the original skin's 30 × 30 pt buttons
/// (Grid_Export_Button_*): `m` maps a point of that 30 pt square into the button.
#[derive(Clone, Copy, PartialEq)]
enum GridIcon {
    MoveLeft,
    MoveRight,
    Narrow,
    Wide,
    Double,
    Half,
    Center,
    Whole,
    FromCurrent,
    Undo,
    Redo,
    Metronome(bool),
    Volume(u8),
    Lock(bool),
}

const SKIN_BLUE: Color32 = hex(0x0096FF);

fn grid_icon(p: &Painter, r: Rect, icon: GridIcon, c: Color32) {
    let k = r.height() / 30.0;
    let m = |x: f32, y: f32| pos2(r.center().x + (x - 15.0) * k, r.center().y + (y - 15.0) * k);
    let tri = |a: Pos2, b: Pos2, d: Pos2, col: Color32| {
        p.add(Shape::convex_polygon(vec![a, b, d], col, Stroke::NONE));
    };
    // Three thin beat lines starting at x (3 pt apart).
    let bars = |x: f32, y0: f32, y1: f32, col: Color32| {
        for i in 0..3 {
            let xx = x + 3.0 * i as f32 + 0.5;
            p.line_segment([m(xx, y0), m(xx, y1)], Stroke::new(1.0, col));
        }
    };
    let grey = hex(0x646464);
    match icon {
        GridIcon::MoveLeft => {
            tri(m(5.0, 15.0), m(10.0, 12.0), m(10.0, 18.0), c);
            bars(15.0, 9.0, 21.0, grey);
        }
        GridIcon::MoveRight => {
            tri(m(25.0, 15.0), m(20.0, 18.0), m(20.0, 12.0), c);
            bars(8.0, 9.0, 21.0, grey);
        }
        GridIcon::Narrow => {
            // Arrows pointing in at the beat lines.
            tri(m(9.0, 15.0), m(4.0, 18.0), m(4.0, 12.0), c);
            tri(m(22.0, 15.0), m(27.0, 12.0), m(27.0, 18.0), c);
            bars(12.0, 9.0, 21.0, grey);
        }
        GridIcon::Wide => {
            tri(m(4.0, 15.0), m(9.0, 12.0), m(9.0, 18.0), c);
            tri(m(27.0, 15.0), m(22.0, 18.0), m(22.0, 12.0), c);
            bars(12.0, 9.0, 21.0, grey);
        }
        GridIcon::Double | GridIcon::Half => {
            bars(4.0, 9.0, 21.0, grey);
            p.text(m(20.5, 15.0), Align2::CENTER_CENTER, if icon == GridIcon::Double { "x2" } else { "/2" }, bold(10.5 * k / 0.8), c);
        }
        GridIcon::Center => {
            // A beat line, white above and red below, as beats are marked on the waveform.
            p.rect_filled(Rect::from_min_max(m(14.0, 7.0), m(16.0, 15.0)), 0.0, c);
            p.rect_filled(Rect::from_min_max(m(14.0, 15.0), m(16.0, 23.0)), 0.0, hex(0xFA0000));
        }
        GridIcon::Whole | GridIcon::FromCurrent => {
            bars(5.0, 10.0, 20.0, if icon == GridIcon::Whole { c } else { hex(0x4B4B4B) });
            bars(18.0, 10.0, 20.0, c);
            tri(m(15.0, 9.0), m(13.0, 6.0), m(17.0, 6.0), grey);
            tri(m(15.0, 21.0), m(17.0, 24.0), m(13.0, 24.0), grey);
            p.rect_filled(Rect::from_min_max(m(14.0, 10.0), m(16.0, 20.0)), 0.0, if icon == GridIcon::Whole { grey } else { hex(0xFA0000) });
        }
        GridIcon::Undo | GridIcon::Redo => {
            // A three-quarter turn with an arrow head at its open end (mirrored for Redo).
            let sgn = if icon == GridIcon::Undo { 1.0 } else { -1.0 };
            let mm = |x: f32, y: f32| m(15.0 + sgn * (x - 15.0), y);
            let mut pts = vec![];
            for i in 0..=16 {
                let a = -std::f32::consts::FRAC_PI_2 + i as f32 / 16.0 * std::f32::consts::PI;
                pts.push(mm(13.6 + a.cos() * 4.6, 15.0 + a.sin() * 4.2));
            }
            p.add(Shape::line(pts, Stroke::new(1.7 * k / 0.8, c)));
            p.add(Shape::convex_polygon(vec![mm(9.0, 10.8), mm(13.6, 7.9), mm(13.6, 13.7)], c, Stroke::NONE));
        }
        GridIcon::Metronome(on) => {
            let col = if on { SKIN_BLUE } else { hex(0x787878) };
            let s = Stroke::new(1.6 * k / 0.8, col);
            // Body, base and pendulum.
            p.add(Shape::closed_line(vec![m(12.5, 7.5), m(17.5, 7.5), m(20.5, 22.0), m(9.5, 22.0)], s));
            p.line_segment([m(10.5, 18.3), m(19.5, 18.3)], s);
            p.line_segment([m(15.0, 18.3), m(19.5, 9.5)], s);
        }
        GridIcon::Volume(v) => {
            for (i, (x, y0, y1)) in [(9.0, 12.0, 17.0), (13.0, 10.0, 19.0), (17.0, 8.0, 22.0)].iter().enumerate() {
                p.rect_filled(Rect::from_min_max(m(*x, *y0), m(*x + 2.0, *y1)), 0.0, if (i as u8) < v { SKIN_BLUE } else { hex(0x787878) });
            }
        }
        GridIcon::Lock(on) => {
            let col = if on { hex(0x007DE1) } else { hex(0x969696) };
            let body = Rect::from_min_max(m(9.0, 14.0), m(21.0, 23.0));
            p.rect_filled(body, 1.0, col);
            let mut pts = vec![];
            // Closed shackle when locked, swung open when not.
            let (cx, lift) = if on { (15.0, 0.0) } else { (15.0, -2.5) };
            for i in 0..=12 {
                let a = std::f32::consts::PI + i as f32 * std::f32::consts::PI / 12.0;
                pts.push(m(cx + a.cos() * 4.0, 11.0 + lift + a.sin() * 4.5));
            }
            let mut line = vec![m(cx - 4.0, 14.0)];
            line.extend(pts);
            if on {
                line.push(m(cx + 4.0, 14.0));
            }
            p.add(Shape::line(line, Stroke::new(1.8 * k / 0.8, col)));
            p.circle_filled(m(15.0, 18.0), 1.4 * k / 0.8, Color32::BLACK);
        }
    }
}

/// GRID panel (the strip under the waveform while the GRID tab is selected): beat grid
/// moves, tempo changes, first beat, tap, BPM entry, range, Undo/Redo, metronome and lock.
/// Buttons are 28 × 24 pt on the skin's #191919 with 2 pt between partners and 10 pt
/// between groups, starting at `x0`.
pub fn grid_strip(app: &mut App, ui: &mut Ui, s: Rect, x0: f32) {
    let y = s.center().y;
    let face = hex(0x191919);
    let locked = app.deck.track.and_then(|t| app.track(t)).map(|t| t.grid_locked).unwrap_or(false);
    let loaded = app.deck.track.is_some();
    let mut x = x0;
    // One icon button; returns its response.
    let ib = |ui: &mut Ui, app: &App, x: &mut f32, id: &str, icon: GridIcon, tipt: &str, selected: bool, gap: f32| -> egui::Response {
        let r = Rect::from_min_size(pos2(*x, y - 12.0), vec2(28.0, 24.0));
        *x += 28.0 + gap;
        let resp = icon_button(ui, r, id, Some(face), |p, r, c| grid_icon(p, r, icon, if loaded { c } else { OFF }));
        if selected {
            ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, SKIN_BLUE), egui::StrokeKind::Inside);
        }
        tip(resp, app, tipt)
    };
    let fine = ui.input(|i| i.modifiers.shift);
    let fc = app.prefs.grid_from_current;
    // Beat grid left / right: 1 ms a click (Shift: 5 ms).
    let step = if fine { 0.005 } else { 0.001 };
    if ib(ui, app, &mut x, "gl", GridIcon::MoveLeft, if fc { "Shift Beatgrid left" } else { "Shift the whole Beatgrid left" }, false, 2.0).clicked() {
        app.grid_shift(-step);
    }
    if ib(ui, app, &mut x, "gr", GridIcon::MoveRight, if fc { "Shift Beatgrid right" } else { "Shift the whole Beatgrid right" }, false, 10.0).clicked() {
        app.grid_shift(step);
    }
    // Beat intervals: 0.01 BPM a click.
    if ib(ui, app, &mut x, "gn", GridIcon::Narrow, "Shrink beat intervals", false, 2.0).clicked() {
        app.grid_scale_bpm(|v| ((v + 0.01) * 100.0).round() / 100.0);
    }
    if ib(ui, app, &mut x, "gw", GridIcon::Wide, "Expand beat intervals", false, 10.0).clicked() {
        app.grid_scale_bpm(|v| ((v - 0.01) * 100.0).round() / 100.0);
    }
    if ib(ui, app, &mut x, "g2", GridIcon::Double, "Double the BPM value", false, 2.0).clicked() {
        app.grid_scale_bpm(|v| v * 2.0);
    }
    if ib(ui, app, &mut x, "gh", GridIcon::Half, "Halve the BPM value", false, 10.0).clicked() {
        app.grid_scale_bpm(|v| v / 2.0);
    }
    // First beat of a bar, and a beat, at the current position.
    let r = Rect::from_min_size(pos2(x, y - 12.0), vec2(50.0, 24.0));
    x += 52.0;
    if tip(button(ui, r, "gdown", "1.1Bars", face, if loaded { hex(0xC8C8C8) } else { OFF }, 10.5), app, "Set the first beat of a bar to the current position.").clicked() {
        app.grid_set_downbeat();
    }
    if ib(ui, app, &mut x, "gcenter", GridIcon::Center, "Set a beat to the current position", false, 10.0).clicked() {
        app.grid_center();
    }
    let r = Rect::from_min_size(pos2(x, y - 12.0), vec2(44.0, 24.0));
    x += 46.0;
    if tip(button(ui, r, "gtap", "TAP", face, hex(0x969696), 11.0), app, "Set BPM with the tapping interval").clicked() {
        app.grid_tap();
    }
    // BPM: shows the tempo at the playhead; type a value (40–499) and press Enter to set it.
    let r = Rect::from_min_size(pos2(x, y - 12.0), vec2(62.0, 24.0));
    x += 72.0;
    if !app.deck.x.bpm_editing {
        let b = app.current_bpm();
        app.deck.bpm_text = if loaded && b > 0.0 { format!("{b:.2}") } else { String::new() };
    }
    let mut txt = app.deck.bpm_text.clone();
    let resp = ui.put(r, egui::TextEdit::singleline(&mut txt).horizontal_align(egui::Align::Center).hint_text("BPM").interactive(loaded));
    app.deck.bpm_text = txt;
    if resp.gained_focus() || resp.has_focus() {
        app.deck.x.bpm_editing = true;
    }
    if resp.lost_focus() {
        app.deck.x.bpm_editing = false;
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let t = app.deck.bpm_text.clone();
            app.grid_enter_bpm(&t);
        }
    }
    let _ = tip(resp, app, "Enter a BPM value");
    // Range of the changes.
    if ib(ui, app, &mut x, "gwhole", GridIcon::Whole, "Make an adjustment on the whole track", !fc, 2.0).clicked() {
        app.prefs.grid_from_current = false;
        app.save_prefs();
    }
    if ib(ui, app, &mut x, "gcur", GridIcon::FromCurrent, "Make an adjustment from the current position", fc, 10.0).clicked() {
        app.prefs.grid_from_current = true;
        app.save_prefs();
    }
    if ib(ui, app, &mut x, "gundo", GridIcon::Undo, "Undo", false, 2.0).clicked() {
        app.grid_undo();
    }
    if ib(ui, app, &mut x, "gredo", GridIcon::Redo, "Redo", false, 10.0).clicked() {
        app.grid_redo();
    }
    // Metronome and its volume (three steps), a joined pair as in the skin.
    let met = app.deck.metronome;
    if ib(ui, app, &mut x, "gmet", GridIcon::Metronome(met), "Metronome On/Off", false, 0.0).clicked() {
        app.deck.metronome = !met;
        app.update_metronome();
    }
    let vol = app.prefs.metronome_vol.clamp(1, 3);
    if ib(ui, app, &mut x, "gmetvol", GridIcon::Volume(vol), "Metronome Volume", false, 10.0).clicked() {
        app.set_metronome_volume(vol % 3 + 1);
    }
    let lock_tip = if locked { "Track Analysis, Key Analysis, and Beatgrid Adjustment are locked." } else { "Enable/Disable Track Analysis, Key Analysis, and Beatgrid Adjustment." };
    if ib(ui, app, &mut x, "glock", GridIcon::Lock(locked), lock_tip, false, 0.0).clicked() {
        if let Some(id) = app.deck.track {
            if let Some(t) = app.track_mut(id) {
                t.grid_locked = !t.grid_locked;
            }
            app.save_track(id);
        }
    }
}

// ---------------------------------------------------------------------- transport pieces

/// Previous / Next Track icons after the skin's export_prevTrack_btn / export_nextTrack_btn:
/// an arrow with a shaft pointing at (or away from) a bar.
fn track_icon(p: &Painter, r: Rect, c: Color32, dir: f32) {
    let m = r.center();
    let s = Stroke::new(1.5, c);
    p.line_segment([m + vec2(-dir * 6.5, -4.0), m + vec2(-dir * 6.5, 4.0)], s);
    p.line_segment([m + vec2(-dir * 3.0, 0.0), m + vec2(dir * 3.5, 0.0)], s);
    p.add(Shape::convex_polygon(vec![m + vec2(dir * 7.0, 0.0), m + vec2(dir * 2.5, -3.5), m + vec2(dir * 2.5, 3.5)], c, Stroke::NONE));
}

/// Search icons: two triangles (◀◀ / ▶▶).
fn search_icon(p: &Painter, r: Rect, c: Color32, dir: f32) {
    let m = r.center();
    for off in [-3.0f32, 3.0] {
        let o = m + vec2(off, 0.0);
        p.add(Shape::convex_polygon(vec![o + vec2(dir * 3.0, 0.0), o + vec2(-dir * 3.0, -4.0), o + vec2(-dir * 3.0, 4.0)], c, Stroke::NONE));
    }
}

fn speaker_icon(p: &Painter, r: Rect, c: Color32, muted: bool) {
    let m = r.center();
    p.add(Shape::convex_polygon(vec![m + vec2(-6.0, -2.0), m + vec2(-3.0, -2.0), m + vec2(-3.0, 2.0), m + vec2(-6.0, 2.0)], c, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![m + vec2(-3.0, -2.0), m + vec2(1.0, -5.5), m + vec2(1.0, 5.5), m + vec2(-3.0, 2.0)], c, Stroke::NONE));
    if muted {
        let s = Stroke::new(1.5, hex(0xE04848));
        p.line_segment([m + vec2(3.0, -3.0), m + vec2(8.0, 3.0)], s);
        p.line_segment([m + vec2(8.0, -3.0), m + vec2(3.0, 3.0)], s);
    } else {
        for rad in [3.0f32, 6.0] {
            let pts: Vec<Pos2> = (0..=8).map(|i| { let a = -0.9 + i as f32 * 0.225; m + vec2(1.5 + a.cos() * rad, a.sin() * rad) }).collect();
            p.add(Shape::line(pts, Stroke::new(1.3, c)));
        }
    }
}

/// Mute button (shared by both decks).
fn mute_button(app: &mut App, ui: &mut Ui, r: Rect) {
    let muted = app.px.muted;
    if tip(icon_button(ui, r, "mute", None, |p, r, c| speaker_icon(p, r, c, muted)), app, "Mute").clicked() {
        app.toggle_mute();
    }
}

/// Previous Track / Next Track and Search Backward / Search Forward: four 25 × 22 pt buttons
/// centred on `c[0..4]`. Search works while its button is held.
fn track_and_search(app: &mut App, ui: &mut Ui, c: [Pos2; 4]) {
    let rect = |i: usize| Rect::from_center_size(c[i], vec2(25.0, 22.0));
    if tip(icon_button(ui, rect(0), "prevtrk", Some(BTN), |p, r, c| track_icon(p, r, c, -1.0)), app, "Previous Track").clicked() {
        app.previous_track();
    }
    if tip(icon_button(ui, rect(1), "nexttrk", Some(BTN), |p, r, c| track_icon(p, r, c, 1.0)), app, "Next Track").clicked() {
        app.load_adjacent(1);
    }
    let back = tip(icon_button(ui, rect(2), "srchb", Some(BTN), |p, r, c| search_icon(p, r, c, -1.0)), app, "Search Backward");
    let fwd = tip(icon_button(ui, rect(3), "srchf", Some(BTN), |p, r, c| search_icon(p, r, c, 1.0)), app, "Search Forward");
    let dir = if back.is_pointer_button_down_on() { -1 } else if fwd.is_pointer_button_down_on() { 1 } else { 0 };
    if dir != app.searching() {
        app.search(dir);
        // Released while stopped: the deck stays where the search ended, as its new cue point.
        if dir == 0 && !app.playing() {
            app.deck.cue_point = app.pos();
        }
    }
}

/// CUE and PLAY/PAUSE as round buttons of radius `rad` (18.5 pt in the 1 Player layout).
/// `keys`: this deck is the one the keyboard operates.
fn cue_play(app: &mut App, ui: &mut Ui, cue_c: Pos2, play_c: Pos2, rad: f32, keys: bool) {
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let playing = app.playing();
    let size = vec2(rad * 2.0 + 5.0, rad * 2.0 + 5.0);
    let cr = ui.interact(Rect::from_center_size(cue_c, size), ui.id().with("cuebtn"), Sense::click_and_drag());
    let cue_lit = loaded && (!playing || app.deck.cue_preview);
    let face = if cr.hovered() { hex(0x2E2E2E) } else if loaded { hex(0x282828) } else { BG2 };
    p.circle_filled(cue_c, rad, face);
    p.circle_stroke(cue_c, rad, Stroke::new(2.0, if cue_lit { hex(0xF4BC53) } else { hex(0x745D31) }));
    p.text(cue_c, Align2::CENTER_CENTER, "CUE", bold(10.5 * (rad / 18.5).max(0.85)), if loaded { WHITE } else { hex(0x323232) });
    if cr.is_pointer_button_down_on() && !app.deck.cue_preview && ui.input(|i| i.pointer.any_pressed()) {
        app.cue_down();
    }
    let cue_key_held = keys && app.prefs.key_for("cue").map(|k| ui.input(|i| i.key_down(k))).unwrap_or(false);
    if !cr.is_pointer_button_down_on() && app.deck.cue_preview && !cue_key_held && !app.deck.x.wave_cue_held {
        app.cue_up();
    }
    let _ = tip(cr, app, "Set a Cue point if you click during pause. \nMove to the Cue point if you click during playback.");
    let pr = ui.interact(Rect::from_center_size(play_c, size), ui.id().with("playbtn"), Sense::click());
    p.circle_filled(play_c, rad, if pr.hovered() { hex(0x2A2A2A) } else { BG2 });
    p.circle_stroke(play_c, rad, Stroke::new(2.0, if playing { PLAY_GREEN } else { hex(0x38672C) }));
    if playing {
        for dx in [-3.5, 3.5] {
            p.rect_filled(Rect::from_center_size(play_c + vec2(dx, 0.0), vec2(3.5, 11.0)), 0.0, PLAY_GREEN);
        }
    } else {
        p.add(Shape::convex_polygon(vec![play_c + vec2(-3.0, -5.5), play_c + vec2(5.0, 0.0), play_c + vec2(-3.0, 5.5)], hex(0x646464), Stroke::NONE));
    }
    if tip(pr, app, "Play/Pause").clicked() {
        app.play_pause();
    }
}

/// Artwork tile; a click unloads the track.
fn artwork(app: &mut App, ui: &mut Ui, art: Rect, track: &Option<Track>) {
    let p = ui.painter().clone();
    match track {
        Some(t) => {
            if let Some(tex) = crate::ui_browser::artwork_texture(app, &t.artwork) {
                p.image(tex.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                p.rect_filled(art, 0.0, hex(0x141414));
                draw_note_icon(&p.with_clip_rect(art), art);
            }
            let ar = ui.interact(art, ui.id().with("artwork"), Sense::click());
            if tip(ar, app, "Artwork:\nClick it to unload the track.").clicked() {
                app.unload_track();
            }
        }
        None => {
            p.rect_filled(art, 0.0, hex(0x0F0F0F));
            let rad = art.height().min(art.width()) * 0.42;
            p.circle_filled(art.center(), rad, hex(0x1E1E1E));
            p.circle_stroke(art.center(), rad, Stroke::new(1.0, hex(0x262626)));
            p.circle_filled(art.center(), 2.5, hex(0x0F0F0F));
        }
    }
}

/// Remaining and elapsed time starting at `x` (centred on `y`); a click switches which one is
/// bright. Returns the right end.
fn times(app: &mut App, ui: &mut Ui, x: f32, y: f32, pos: f64) -> f32 {
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let remain = (app.deck.duration - pos).max(0.0);
    let (dim_t, bright_t) = if loaded { (DIM, WHITE) } else { (OFF, OFF) };
    let (c1, c2) = if app.prefs.time_remain { (bright_t, dim_t) } else { (dim_t, bright_t) };
    let (big, small) = split_time(if loaded { remain } else { 0.0 });
    let tr = p.text(pos2(x, y), Align2::LEFT_CENTER, format!("-{big}"), FontId::proportional(13.5), c1);
    let tr2 = p.text(pos2(tr.max.x, y + 1.5), Align2::LEFT_CENTER, format!(".{small}"), FontId::proportional(10.0), c1);
    let (big2, small2) = split_time(if loaded { pos } else { 0.0 });
    let er = p.text(pos2(tr2.max.x + 9.0, y), Align2::LEFT_CENTER, &big2, FontId::proportional(13.5), c2);
    let er2 = p.text(pos2(er.max.x, y + 1.5), Align2::LEFT_CENTER, format!(".{small2}"), FontId::proportional(10.0), c2);
    let hit = ui.interact(Rect::from_min_max(pos2(x, y - 10.0), pos2(er2.max.x, y + 10.0)), ui.id().with("timesw"), Sense::click());
    if tip(hit, app, "Switch Time/Remain").clicked() {
        app.prefs.time_remain = !app.prefs.time_remain;
    }
    er2.max.x
}

// ---------------------------------------------------------------------- 2 Players

/// Decks A and B side by side. Each has its own track, waveforms, cue/play, hot cues,
/// memory cues and loops, and both sound together. The deck last clicked is the active
/// one (blue letter): keyboard shortcuts and the GRID / side panels of the 1 Player
/// layout follow it. The browser's PLAYER A/B selector (`App::load_target`) chooses where
/// double-click and Return load; dragging a track onto a deck loads it there.
pub fn two_players(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, BG);
    let half = (full.width() / 2.0).floor();
    let mut activate = None;
    let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    for n in 0..2u8 {
        let d = Rect::from_min_max(pos2(full.min.x + n as f32 * half + n as f32, full.min.y), pos2(full.min.x + (n as f32 + 1.0) * half - 1.0 + n as f32, full.max.y));
        let is_active = app.px.active == n;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(d).id_salt(("deck", n)));
        child.set_clip_rect(d.intersect(ui.clip_rect()));
        app.with_deck(n, |app| small_deck(app, &mut child, d, n, is_active));
        if pressed.map(|pp| d.contains(pp)).unwrap_or(false) {
            activate = Some(n);
        }
    }
    ui.painter().line_segment([pos2(full.min.x + half, full.min.y), pos2(full.min.x + half, full.max.y)], Stroke::new(1.0, hex(0x323232)));
    if let Some(n) = activate {
        app.activate_deck(n);
    }
    let any = app.playing() || app.player.other.lock().playing || app.px.other.loading.is_some() || app.deck.loading.is_some();
    if any {
        ui.ctx().request_repaint();
    }
}

/// One deck of the 2 Players layout in `d` (half the window wide, 255 pt tall):
/// a 64 pt transport column (Previous/Next Track 28, Search 56, beat jump 84 and its length
/// 112, CUE 162.5, PLAY 213 pt below the top), then the header row (0–22), artwork and
/// overview (24–58), the enlarged waveform (84–170) and two control rows (centred 192, 228).
fn small_deck(app: &mut App, ui: &mut Ui, d: Rect, n: u8, is_active: bool) {
    let p = ui.painter().clone();
    let pos = app.pos();
    let loaded = app.deck.track.is_some();
    let left = Rect::from_min_size(d.min, vec2(64.0, d.height()));
    p.rect_filled(left, 0.0, BG2);
    let (lx, ly) = (left.min.x, left.min.y);
    track_and_search(app, ui, [pos2(lx + 18.5, ly + 28.0), pos2(lx + 45.5, ly + 28.0), pos2(lx + 18.5, ly + 56.0), pos2(lx + 45.5, ly + 56.0)]);
    let bj = app.prefs.beat_jump;
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_center_size(pos2(lx + x, ly + y), vec2(w, h));
    if tip(icon_button(ui, at(18.5, 84.0, 25.0, 22.0), "bjl", Some(BTN), crate::icons::chevron_left), app, "Jump Reverse").clicked() {
        app.beat_jump(-bj);
    }
    if tip(icon_button(ui, at(45.5, 84.0, 25.0, 22.0), "bjr", Some(BTN), crate::icons::chevron_right), app, "Jump Forward").clicked() {
        app.beat_jump(bj);
    }
    let bj_rect = at(32.0, 112.0, 52.0, 24.0);
    let r = tip(ui.interact(bj_rect, ui.id().with("bjsize"), Sense::click()), app, "Select the beat/bar length jumping from the current position.");
    p.rect_filled(bj_rect, 2.0, if r.hovered() { lighten(BTN) } else { BTN });
    p.text(bj_rect.center() - vec2(0.0, 5.0), Align2::CENTER_CENTER, crate::player_ext::jump_label(bj, app.prefs.beat_jump_bars), bold(9.0), WHITE);
    crate::icons::chevron_down(&p, Rect::from_center_size(bj_rect.center() + vec2(0.0, 6.0), vec2(12.0, 10.0)), WHITE);
    crate::ui_browser::dropdown(ui, &r, "bjpop", |ui: &mut Ui| jump_size_menu(app, ui));
    cue_play(app, ui, pos2(lx + 32.0, ly + 162.5), pos2(lx + 32.0, ly + 213.0), 18.5, is_active);

    let main = Rect::from_min_max(pos2(left.max.x + 2.0, d.min.y), pos2(d.max.x - 4.0, d.max.y));
    let (mx, my, w) = (main.min.x, main.min.y, main.width());
    // ---- header: deck letter, title, artist, times, key, BPM
    let hy = my + 11.0;
    let badge = Rect::from_min_size(pos2(mx + 2.0, my + 3.0), vec2(18.0, 16.0));
    p.rect_filled(badge, 2.0, if is_active { BLUE } else { BTN });
    p.text(badge.center(), Align2::CENTER_CENTER, if n == 0 { "A" } else { "B" }, bold(11.0), WHITE);
    let _ = tip(ui.interact(badge, ui.id().with("badge"), Sense::hover()), app, if is_active { "Keyboard shortcuts operate this deck." } else { "Click the deck to operate it with the keyboard." });
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    let (x_artist, x_time, x_key, x_bpm) = (mx + w * 0.40, mx + w * 0.60, mx + w * 0.835, mx + w * 0.915);
    if let Some(t) = &track {
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(mx + 26.0, my), pos2(x_artist - 6.0, my + 22.0)));
        clip.text(pos2(mx + 26.0, hy), Align2::LEFT_CENTER, t.display_title(), bold(13.5), WHITE);
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(x_artist, my), pos2(x_time - 6.0, my + 22.0)));
        clip.text(pos2(x_artist + 8.0, hy), Align2::LEFT_CENTER, &t.artist, FontId::proportional(11.5), DIM);
        p.text(pos2((x_key + x_bpm) / 2.0, hy), Align2::CENTER_CENTER, t.key_label(app.prefs.key_alnum), FontId::proportional(12.5), WHITE);
    }
    for x in [x_artist, x_time, x_key, x_bpm] {
        p.line_segment([pos2(x, hy - 7.5), pos2(x, hy + 7.5)], Stroke::new(1.0, OFF));
    }
    times(app, ui, x_time + 8.0, hy, pos);
    if loaded {
        let b = app.current_bpm();
        p.text(pos2((x_bpm + main.max.x) / 2.0, hy), Align2::CENTER_CENTER, if b > 0.0 { format!("{b:.2}") } else { "--.--".into() }, FontId::proportional(12.5), WHITE);
    }
    // ---- artwork and overview
    artwork(app, ui, Rect::from_min_size(pos2(mx + 2.0, my + 24.0), vec2(34.0, 34.0)), &track);
    draw_overview(app, ui, Rect::from_min_max(pos2(mx + 44.0, my + 24.0), pos2(main.max.x - 2.0, my + 57.0)), pos);
    // ---- enlarged waveform (Ctrl/Cmd + scroll zooms)
    draw_detail(app, ui, Rect::from_min_max(pos2(mx, my + 84.0), pos2(main.max.x, my + 170.0)), pos);
    // ---- control row 1: hot cue pads (26 pt on a 30 pt pitch), MEMORY ◀ ▶ ✕, Q
    p.rect_filled(Rect::from_min_max(pos2(mx - 2.0, my + 175.0), pos2(d.max.x, d.max.y)), 0.0, BG2);
    let y1 = my + 192.0;
    for slot in 0..8 {
        let r = Rect::from_center_size(pos2(mx + 16.0 + slot as f32 * 30.0, y1), vec2(26.0, 26.0));
        hot_pad(app, ui, r, slot);
    }
    let b1 = |x: f32, w: f32| Rect::from_center_size(pos2(mx + x, y1), vec2(w, 23.0));
    if tip(bold_button(ui, b1(282.0, 56.0), "mem", "MEMORY"), app, "Set Memory Cue(Loop)").clicked() {
        app.memory_set();
    }
    if tip(icon_button(ui, b1(324.0, 22.0), "memprev", Some(BG2), crate::icons::tri_left), app, "Call Previous Memory Cue").clicked() {
        app.memory_jump(-1);
    }
    if tip(icon_button(ui, b1(348.0, 22.0), "memnext", Some(BG2), crate::icons::tri_right), app, "Call Next Memory Cue").clicked() {
        app.memory_jump(1);
    }
    if tip(icon_button(ui, b1(372.0, 22.0), "memdel", Some(BG2), |p, r, c| crate::icons::cross(p, r, c, 1.6)), app, "Delete Memory Cue(Loop)").clicked() {
        app.memory_delete_current();
    }
    // Memory cues of this deck as small numbered chips (click: call).
    let mems = app.memory_cues();
    let mut cx = mx + 394.0;
    for (i, c) in mems.iter().enumerate() {
        let r = Rect::from_min_size(pos2(cx, y1 - 9.0), vec2(17.0, 18.0));
        if r.max.x > main.max.x - 44.0 {
            break;
        }
        let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
        let resp = ui.interact(r, ui.id().with(("memchip", c.id)), Sense::click());
        p.rect_filled(r, 2.0, if resp.hovered() { hex(0x2A2A2A) } else { hex(0x121212) });
        p.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 2.0)), 0.0, col);
        p.text(r.center() + vec2(0.0, 1.0), Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(9.5), if c.is_loop() { LOOP } else { WHITE });
        let label = format!("Call Memory Cue(Loop)  {}{}", fmt_time(c.start, false), if c.comment.is_empty() { String::new() } else { format!("  {}", c.comment) });
        if tip(resp, app, &label).clicked() {
            app.call_memory(c);
        }
        cx += 19.0;
    }
    let q = app.prefs.quantize;
    if tip(seg_button(ui, Rect::from_center_size(pos2(main.max.x - 20.0, y1), vec2(30.0, 23.0)), "quant", "Q", q), app, "Quantize ON/OFF").clicked() {
        app.prefs.quantize = !q;
        app.save_prefs();
    }
    // ---- control row 2: AU / MA, loop buttons, mute, menu
    let y2 = my + 228.0;
    let b2 = |x: f32, w: f32| Rect::from_center_size(pos2(mx + x, y2), vec2(w, 23.0));
    let au = app.prefs.loop_auto;
    if tip(seg_button(ui, b2(20.0, 34.0), "au", "AU", au), app, "Auto Beat Loop").clicked() {
        app.prefs.loop_auto = true;
    }
    if tip(seg_button(ui, b2(57.0, 34.0), "ma", "MA", !au), app, "Manual Loop").clicked() {
        app.prefs.loop_auto = false;
    }
    if au {
        if tip(icon_button(ui, b2(90.0, 18.0), "lhalf", None, crate::icons::chevron_left), app, "Auto Beat Loop:\nMake Loop length shorter.").clicked() {
            app.resize_loop(0.5);
        }
        let lb = app.prefs.loop_beats;
        let active = app.loop_active().is_some();
        let field = b2(131.0, 56.0);
        if tip(button(ui, field, "lbeat", &beats_text(lb), if active { LOOP.gamma_multiply(0.7) } else { BTN }, WHITE, 11.5), app, "Auto Beat Loop:\nSet/Reset Loop of displayed beat length.").clicked() {
            if active {
                app.loop_exit();
            } else {
                app.auto_loop(lb);
            }
        }
        let dr = Rect::from_min_max(pos2(field.max.x - 14.0, field.min.y), field.max);
        let dresp = tip(ui.interact(dr, ui.id().with("lsize"), Sense::click()), app, "Beat Loop");
        crate::icons::chevron_down(&p, Rect::from_center_size(dr.center(), vec2(10.0, 9.0)), if dresp.hovered() { WHITE } else { DIM });
        crate::ui_browser::dropdown(ui, &dresp, "lsizepop", |ui: &mut Ui| loop_size_menu(app, ui));
        if tip(icon_button(ui, b2(172.0, 18.0), "ldouble", None, crate::icons::chevron_right), app, "Auto Beat Loop:\nMake Loop length longer.").clicked() {
            app.resize_loop(2.0);
        }
    } else {
        // IN 78..112, OUT 115..149, RELOOP/EXIT 152..222 from the row's left.
        manual_loop_buttons(app, ui, |x, w| b2(x - 951.0 + 78.0, w));
    }
    mute_button(app, ui, Rect::from_center_size(pos2(main.max.x - 52.0, y2), vec2(24.0, 22.0)));
    let menu_r = tip(icon_button(ui, Rect::from_center_size(pos2(main.max.x - 20.0, y2), vec2(22.0, 22.0)), "stripmenu", None, crate::icons::hamburger), app, "Show Menu items");
    crate::ui_browser::dropdown(ui, &menu_r, "strippop", |ui: &mut Ui| {
        let t = app.pos();
        ui.label(egui::RichText::new("At the current position").size(10.5).color(DIM));
        if ui.button("Set Cue(In) Point").clicked() {
            app.set_cue_at(t);
        }
        if ui.button("Set Out Point").clicked() {
            app.set_out_at(t);
        }
        strip_menu_extra(app, ui);
    });
    drop_to_load(app, ui, d);
}

/// A hot cue pad: press to set or play, right-click for colour / comment / delete, and a
/// cross in the corner to clear.
fn hot_pad(app: &mut App, ui: &mut Ui, r: Rect, slot: i32) {
    let p = ui.painter().clone();
    let cue = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).cloned();
    let resp = ui.interact(r, ui.id().with(("pad", slot)), Sense::click_and_drag());
    p.rect_filled(r, 2.0, if resp.hovered() { hex(0x1A1A1A) } else { hex(0x121212) });
    let tc = match &cue {
        Some(c) => {
            p.rect_filled(Rect::from_center_size(r.center(), vec2(15.0, 15.0)), 0.0, hotcue_color(c.color));
            WHITE
        }
        None => hex(0x5F5F5F),
    };
    let letter = (b'A' + slot as u8) as char;
    p.text(r.center(), Align2::CENTER_CENTER, letter.to_string(), FontId::proportional(14.0), tc);
    if resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
        if ui.input(|i| i.modifiers.shift) {
            if let Some(c) = &cue {
                app.delete_cue(c.id);
            }
        } else {
            app.hot_cue_down(slot);
        }
    }
    if !resp.is_pointer_button_down_on() && app.deck.hot_preview.is_some() && ui.input(|i| i.pointer.primary_released()) {
        app.hot_cue_up();
    }
    let resp = tip(resp, app, &format!("Set/Play Hot Cue(Loop) {letter}"));
    if let Some(c) = cue {
        resp.context_menu(|ui| hot_cue_menu(app, ui, &c));
        let xr = Rect::from_center_size(pos2(r.max.x - 4.5, r.min.y + 4.5), vec2(10.0, 10.0));
        let xresp = ui.interact(xr, ui.id().with(("padclr", slot)), Sense::click());
        if ui.rect_contains_pointer(r.expand(2.0)) {
            p.rect_filled(xr, 2.0, if xresp.hovered() { hex(0x505050) } else { hex(0x323232) });
            close_cross(&p, xr.center(), 5.0, WHITE);
        }
        if tip(xresp, app, &format!("Clear Hot Cue(Loop) {letter}")).clicked() {
            app.delete_cue(c.id);
        }
    }
}

// ---------------------------------------------------------------------- Simple Player

/// The Simple Player: one 58 pt row with Previous/Next Track, Search, CUE, PLAY, artwork,
/// title and artist, the overview waveform, times, key and BPM, and mute. No enlarged
/// waveform. x positions from the left: track 22/49, search 80/107, CUE 142, PLAY 180,
/// artwork 204..246, text 254..(254 + 17 %), waveform up to 250 pt from the right.
pub fn simple_player(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    let pos = app.pos();
    let cy = full.center().y;
    let x0 = full.min.x;
    p.rect_filled(Rect::from_min_max(full.min, pos2(x0 + 200.0, full.max.y)), 0.0, BG2);
    track_and_search(app, ui, [pos2(x0 + 22.0, cy), pos2(x0 + 49.0, cy), pos2(x0 + 80.0, cy), pos2(x0 + 107.0, cy)]);
    cue_play(app, ui, pos2(x0 + 142.0, cy), pos2(x0 + 180.0, cy), 15.5, true);
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    artwork(app, ui, Rect::from_min_size(pos2(x0 + 206.0, cy - 21.0), vec2(42.0, 42.0)), &track);
    let tx = x0 + 256.0;
    let tw = (full.width() * 0.17).max(150.0);
    if let Some(t) = &track {
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(tx, full.min.y), pos2(tx + tw - 8.0, full.max.y)));
        clip.text(pos2(tx, cy - 9.0), Align2::LEFT_CENTER, t.display_title(), bold(13.5), WHITE);
        clip.text(pos2(tx, cy + 9.0), Align2::LEFT_CENTER, &t.artist, FontId::proportional(11.5), DIM);
    } else {
        p.text(pos2(tx, cy), Align2::LEFT_CENTER, "Not loaded.", FontId::proportional(11.5), OFF);
    }
    let right = full.max.x;
    // Overview (its own geometry: bars end 29 pt and the phrase bar 41 pt below its top).
    let ov = Rect::from_min_max(pos2(tx + tw, full.min.y + 8.0), pos2(right - 252.0, full.min.y + 41.0));
    draw_overview(app, ui, ov, pos);
    // Times, then key over BPM, then mute.
    times(app, ui, right - 240.0, cy, pos);
    if let Some(t) = &track {
        p.text(pos2(right - 68.0, cy - 9.0), Align2::CENTER_CENTER, t.key_label(app.prefs.key_alnum), FontId::proportional(12.5), WHITE);
        let b = app.current_bpm();
        p.text(pos2(right - 68.0, cy + 9.0), Align2::CENTER_CENTER, if b > 0.0 { format!("{b:.2}") } else { "--.--".into() }, FontId::proportional(12.5), WHITE);
    }
    p.line_segment([pos2(right - 100.0, cy - 16.0), pos2(right - 100.0, cy + 16.0)], Stroke::new(1.0, OFF));
    mute_button(app, ui, Rect::from_center_size(pos2(right - 22.0, cy), vec2(26.0, 24.0)));
    drop_to_load(app, ui, full);
    if app.playing() || app.searching() != 0 || app.deck.loading.is_some() {
        ui.ctx().request_repaint();
    }
}
