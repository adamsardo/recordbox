//! PERFORMANCE mode, 2Deck Horizontal: the two enlarged waveforms stacked along the top, deck 1
//! on the left and deck 2 on the right with the mixer between them, and the mixer's
//! headphones / crossfader row along the bottom.
//!
//! Geometry is measured on rekordbox 7.2.19 at 1512 pt (`rekordbox-reference/performance/01`,
//! `stems/14` and `30`) and from its layout code (`checks/re-7.2.19/performance-decks.md`:
//! platter panel, button and font sizes). Positions below are in points from the top-left of
//! the player area (just under the 28 pt top bar).

use crate::app::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_player::{draw_detail_at, draw_overview, lighten, press_fired, split_time, tip, DetailMode};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, Ui};

/// Height of the stacked waveforms.
const WAVES_H: f32 = 143.0;
/// Width of a deck and x of deck 2 and of the mixer column (at 1512 pt).
const DECK_W: f32 = 700.0;
const MIXER_X: f32 = 705.0;
const MIXER_W: f32 = 104.0;
const DECK2_X: f32 = 812.0;
/// Extra height of the deck area while the stem part buttons are shown.
const STEM_ROW: f32 = 21.0;
/// Extra height while the phrase bar is shown under the overview.
pub const PHRASE_ROW: f32 = 6.0;

const PANEL: Color32 = hex(0x1E1E1E);
const PAD: Color32 = hex(0x141414);
const PART: [(&str, usize, Color32); 4] = [("VOCAL", 3, Color32::from_rgb(0x02, 0xDA, 0x0C)), ("INST", 2, Color32::from_rgb(0xDA, 0x1B, 0x02)), ("BASS", 1, Color32::from_rgb(0xD2, 0x00, 0x90)), ("DRUMS", 0, Color32::from_rgb(0x46, 0x55, 0xFF))];

pub fn performance(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    let (l, mut t) = (full.min.x, full.min.y);
    // The FX panel, when shown, sits above the waveforms.
    if app.prefs.perf_fx {
        crate::ui_fx::panel(app, ui, Rect::from_min_size(pos2(l, t), vec2(full.width(), crate::ui_fx::FX_H - 1.0)));
        t += crate::ui_fx::FX_H;
    }
    // Two decks (2Deck Horizontal) or four (4Deck Horizontal: decks 1 and 3 on the left, 2 and
    // 4 on the right, the mixer's four strips between, in rekordbox's order 3, 1, 2, 4).
    // Measured at 1512 pt: with four decks the waveforms take 198 pt (four lanes), a deck is
    // 655 pt wide and 189 pt tall, and the mixer 189 pt wide.
    let four = app.deck_count() == 4;
    let cx = full.center().x;
    let (mixer_w, gap) = if four { (189.0, 5.0) } else { (MIXER_W, 3.0) };
    let mixer_x = cx - mixer_w / 2.0 + if four { -0.5 } else { 1.0 };
    let deck_w = (mixer_x - l - if four { 6.0 } else { 5.0 }).max(480.0);
    let deck2_x = mixer_x + mixer_w + gap;
    let _ = (DECK_W, DECK2_X, MIXER_X);
    let stems_row = if app.prefs.show_stems { STEM_ROW } else { 0.0 };
    let phrase_h = if app.prefs.show_phrase_bar { PHRASE_ROW } else { 0.0 };
    // The sampler panel, when shown, is the last row, under the mixer's bottom row.
    let sampler_h = if app.prefs.perf_sampler { crate::ui_sampler::SAMPLER_H } else { 0.0 };
    app.sampler_poll();
    let deck_bottom = full.max.y - 43.0 - sampler_h;
    let waves_h = if four { 198.0 } else { WAVES_H };
    let count = app.deck_count();
    let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });

    // ---- the enlarged waveforms, one lane per deck
    let lane = waves_h / count as f32;
    for n in 0..count {
        let wr = Rect::from_min_max(pos2(l, t + lane * n as f32), pos2(full.max.x, t + lane * (n + 1) as f32));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(wr).id_salt(("perfwave", n)));
        child.set_clip_rect(wr.intersect(ui.clip_rect()));
        app.with_deck(n, |app| {
            let pos = app.pos();
            draw_detail_at(app, &mut child, wr, pos, 0.0, DetailMode::Full);
        });
        if n > 0 {
            p.rect_filled(Rect::from_min_max(pos2(l, wr.min.y - 0.5), pos2(full.max.x, wr.min.y + 0.5)), 0.0, hex(0x1E1E1E));
        }
        if four {
            p.text(pos2(full.max.x - 6.0, wr.center().y), Align2::RIGHT_CENTER, format!("{}", n + 1), bold(9.5), hex(0x505050));
        }
    }
    zoom_column(app, ui, pos2(l, t), waves_h);

    // ---- decks
    let deck_h = if four { 189.0 + stems_row + phrase_h } else { deck_bottom - (t + waves_h) };
    let mut rects = [Rect::NOTHING; 4];
    for n in 0..count {
        let right = n % 2 == 1;
        let x0 = if right { deck2_x } else { l };
        let w = if right { full.max.x - deck2_x } else { deck_w };
        let y = t + waves_h + (n / 2) as f32 * (deck_h + 1.0);
        let dr = Rect::from_min_size(pos2(x0, y), vec2(w, deck_h));
        rects[n as usize] = dr;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(dr).id_salt(("perfdeck", n)));
        child.set_clip_rect(dr.intersect(ui.clip_rect()));
        let is_active = app.px.active == n;
        app.with_deck(n, |app| deck(app, &mut child, dr, n, is_active, stems_row, four));
        app.with_deck(n, |app| crate::ui_player2::drop_to_load(app, ui, dr));
    }

    // ---- mixer
    let mr = Rect::from_min_max(pos2(mixer_x, t + waves_h), pos2(mixer_x + mixer_w, deck_bottom));
    if four { mixer4(app, ui, mr) } else { mixer(app, ui, mr) }
    let br = Rect::from_min_max(pos2(l, deck_bottom + 1.5), pos2(full.max.x, full.max.y - sampler_h));
    bottom_row(app, ui, br, cx);
    if sampler_h > 0.0 {
        crate::ui_sampler::panel(app, ui, Rect::from_min_max(pos2(l, full.max.y - sampler_h), full.max));
    }

    // The deck that was pressed becomes the active one (keyboard shortcuts follow it).
    if let Some(pp) = pressed.filter(|pp| full.contains(*pp)) {
        if let Some(n) = (0..count).find(|n| rects[*n as usize].contains(pp)) {
            app.activate_deck(n);
        } else if pp.y > t && pp.y <= t + waves_h {
            app.activate_deck((((pp.y - t) / lane) as u8).min(count - 1));
        }
    }
}

/// Zoom in / RST / zoom out over the left end of the waveforms (one zoom for both decks).
fn zoom_column(app: &mut App, ui: &mut Ui, o: Pos2, waves_h: f32) {
    let p = ui.painter().clone();
    p.rect_filled(Rect::from_min_size(o, vec2(28.0, waves_h)), 0.0, BG);
    let o = pos2(o.x, o.y + (waves_h - WAVES_H) / 2.0);
    let tile = |y: f32| Rect::from_center_size(pos2(o.x + 10.6, o.y + y), vec2(18.0, 18.0));
    let zin = ui.interact(tile(45.0), ui.id().with("pzin"), Sense::click());
    crate::icons_player::zoom(&p, tile(45.0).center(), 1.0, WHITE, true);
    if tip(zin, app, "Zoom In").clicked() && app.prefs.zoom > 0 {
        app.prefs.zoom -= 1;
    }
    let rst = ui.interact(Rect::from_center_size(pos2(o.x + 10.6, o.y + 70.0), vec2(20.0, 12.0)), ui.id().with("pzrst"), Sense::click());
    p.text(pos2(o.x + 10.6, o.y + 70.0), Align2::CENTER_CENTER, "RST", caps(7.5), WHITE);
    if tip(rst, app, "Zoom Reset").clicked() {
        app.prefs.zoom = 2;
    }
    let zout = ui.interact(tile(95.0), ui.id().with("pzout"), Sense::click());
    crate::icons_player::zoom(&p, tile(95.0).center(), 1.0, WHITE, false);
    if tip(zout, app, "Zoom Out").clicked() && app.prefs.zoom + 1 < ZOOMS.len() {
        app.prefs.zoom += 1;
    }
}

/// A flat text button of the deck (KEY SYNC, MASTER, SLIP, Q, MT, AU, MA ...).
fn flat(ui: &mut Ui, r: Rect, id: &str, label: &str, size: f32, fill: Color32, text: Color32) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(r, 2.0, if resp.hovered() && fill != BLUE { lighten(fill) } else { fill });
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, caps(size), text);
    resp
}

/// One deck: track information, overview, stem part buttons, pads, loop box and the platter
/// panel. `r`: the deck's rectangle (its top is the bottom of the waveforms).
fn deck(app: &mut App, ui: &mut Ui, r: Rect, n: u8, is_active: bool, stems_row: f32, c4: bool) {
    let p = ui.painter().clone();
    let (x0, y0, w) = (r.min.x, r.min.y, r.width());
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x0 + x, y0 + y), vec2(w, h));
    let pos = app.pos();
    let loaded = app.deck.track.is_some();
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    let dim = hex(0x323232);

    // Sizes of the 2-deck layout, or of the compact 4-deck one (header 43 pt instead of 57,
    // artwork 32, title 13.5 pt; measured on reference performance/03).
    let hdr = if c4 { 43.0 } else { 57.0 };
    let (art, title_y, sub_y) = if c4 { (at(25.0, 6.0, 32.0, 32.0), 14.0, 32.5) } else { (at(25.0, 10.0, 34.0, 34.0), 18.5, 37.5) };
    let (title_f, sub_f, time_f, tx0) = if c4 { (13.5, 10.5, 12.0, 60.0) } else { (15.5, 12.0, 13.5, 64.0) };
    // ---- track information
    p.rect_filled(at(0.0, 0.0, w, hdr), 0.0, PANEL);
    // ---- overview (the whole track). It is laid out before the header's buttons: in the
    // 4-deck layout its top (the hot cue letters) reaches under MASTER, and the buttons must
    // win the click there.
    // (The waveform stands on a base 88.5 pt down; the progress frame, the minute marks and,
    // when shown, the phrase bar follow under it, the phrase bar in a row of its own.)
    let phrase_h = if app.prefs.show_phrase_bar { PHRASE_ROW } else { 0.0 };
    // (In the 4-deck layout the base is 68 pt down and the lower part starts 76 pt down.)
    let lower = if c4 { 76.0 } else { 99.5 };
    p.rect_filled(at(0.0, hdr, w, lower - hdr + phrase_h + stems_row), 0.0, BG);
    draw_overview(app, ui, at(4.0, if c4 { 37.0 } else { 58.5 }, w - 8.0, 44.0), pos);

    p.text(pos2(x0 + 13.6, y0 + if c4 { 14.0 } else { 13.5 }), Align2::CENTER_CENTER, format!("{}", n + 1), bold(9.5), if is_active { WHITE } else { DIM });
    crate::ui_player2::artwork(app, ui, art, &track);
    let (title, artist) = match &track {
        Some(tr) => (tr.display_title(), tr.artist.clone()),
        None => ("Not Loaded.".to_string(), String::new()),
    };
    let key_w = 170.0;
    // The deck's place (the sequencer strip can be dropped on it), and its title as a handle:
    // dragged to a sampler slot, the deck's track loads there.
    app.px.sampler.deck_rects[(n as usize).min(3)] = r;
    if let Some(id) = app.deck.track {
        let hr = at(tx0, 4.0, (w - tx0 - key_w - 170.0).clamp(60.0, 240.0), if c4 { 20.0 } else { 26.0 });
        let hresp = ui.interact(hr, ui.id().with("deckdrag"), Sense::drag());
        if hresp.drag_started() && app.drag.is_none() {
            app.drag = Some(vec![id]);
        }
    }
    let text_clip = p.with_clip_rect(at(tx0, 0.0, (w - tx0 - key_w - 170.0).max(60.0), hdr));
    text_clip.text(pos2(x0 + tx0, y0 + title_y), Align2::LEFT_CENTER, title, bold(title_f), if loaded { WHITE } else { dim });
    let artist_clip = p.with_clip_rect(at(tx0, 0.0, if c4 { 104.0 } else { 116.0 }, hdr));
    artist_clip.text(pos2(x0 + tx0, y0 + sub_y), Align2::LEFT_CENTER, artist, FontId::proportional(sub_f), DIM);
    if loaded {
        let bpm = app.effective_bpm();
        p.text(pos2(x0 + if c4 { 169.0 } else { 185.0 }, y0 + sub_y), Align2::LEFT_CENTER, format!("{bpm:.2}"), FontId::proportional(sub_f), WHITE);
        p.text(pos2(x0 + if c4 { 207.0 } else { 228.0 }, y0 + sub_y), Align2::LEFT_CENTER, app.result_key_label(), FontId::proportional(sub_f), WHITE);
    }
    // Remaining and elapsed time, right-aligned before the key controls.
    let remain = (app.deck.duration - pos).max(0.0);
    let tx = x0 + w - 190.0;
    let (b2, s2) = split_time(if loaded { pos } else { 0.0 });
    let tcol = if loaded { WHITE } else { dim };
    p.text(pos2(tx, y0 + sub_y - 1.0), Align2::RIGHT_CENTER, format!("{b2}.{s2}"), FontId::proportional(time_f), if loaded { DIM } else { dim });
    let (b1, s1) = split_time(if loaded { remain } else { 0.0 });
    p.text(pos2(tx - if c4 { 48.0 } else { 53.0 }, y0 + sub_y - 1.0), Align2::RIGHT_CENTER, format!("-{b1}.{s1}"), FontId::proportional(time_f), tcol);

    // KEY SYNC, key shift, BEAT SYNC and MASTER at the right.
    let rx = x0 + w;
    let ks = app.deck.x.key_sync;
    if tip(flat(ui, Rect::from_min_size(pos2(rx - 161.0, y0 + if c4 { 7.5 } else { 11.0 }), vec2(66.0, if c4 { 11.5 } else { 12.5 })), "keysync", "KEY SYNC", 8.5, if ks { hex(0x007DE1) } else { BTN }, if loaded || ks { WHITE } else { OFF }), app, "KEY SYNC").clicked() {
        app.toggle_key_sync();
    }
    let ky = y0 + if c4 { 33.5 } else { 38.0 };
    let kl = ui.interact(Rect::from_center_size(pos2(rx - 158.0, ky), vec2(12.0, 14.0)), ui.id().with("keydn"), Sense::click());
    crate::icons_player::chevron_left(&p, pos2(rx - 158.0, ky), 0.9, if loaded { WHITE } else { dim });
    if tip(kl, app, "Key Shift Down").clicked() {
        app.key_shift_step(-1);
    }
    let kr = ui.interact(Rect::from_center_size(pos2(rx - 97.0, ky), vec2(12.0, 14.0)), ui.id().with("keyup"), Sense::click());
    crate::icons_player::chevron_right(&p, pos2(rx - 97.0, ky), 0.9, if loaded { WHITE } else { dim });
    if tip(kr, app, "Key Shift Up").clicked() {
        app.key_shift_step(1);
    }
    if loaded {
        let reset = ui.interact(Rect::from_min_max(pos2(rx - 150.0, ky - 7.0), pos2(rx - 104.0, ky + 7.0)), ui.id().with("keyreset"), Sense::click());
        p.text(pos2(rx - 140.0, ky), Align2::CENTER_CENTER, app.result_key_label(), bold(10.0), WHITE);
        p.text(pos2(rx - 113.0, ky), Align2::CENTER_CENTER, crate::player_ext::shift_text(app.deck.x.key_shift), bold(10.0), WHITE);
        if tip(reset, app, "Key Reset").clicked() {
            app.key_reset();
        }
    }
    let bs = app.deck.x.beat_sync;
    let bsr = if c4 { Rect::from_min_size(pos2(rx - 76.0, y0 + 4.5), vec2(41.0, 19.5)) } else { Rect::from_min_size(pos2(rx - 79.0, y0 + 8.5), vec2(44.0, 20.0)) };
    let bresp = ui.interact(bsr, ui.id().with("beatsync"), Sense::click());
    p.rect_filled(bsr, 2.0, if bs { BLUE } else if bresp.hovered() { lighten(BTN) } else { BTN });
    p.text(pos2(bsr.center().x, bsr.min.y + 5.5), Align2::CENTER_CENTER, "BEAT", caps(8.0), WHITE);
    p.text(pos2(bsr.center().x, bsr.min.y + 14.5), Align2::CENTER_CENTER, "SYNC", caps(8.0), WHITE);
    if tip(bresp, app, "BEAT SYNC").clicked() {
        app.toggle_beat_sync();
    }
    let is_master = app.px.master == n && !app.sampler_is_master();
    let mrect = if c4 { Rect::from_min_size(pos2(rx - 77.0, y0 + 29.5), vec2(69.0, 11.0)) } else { Rect::from_min_size(pos2(rx - 79.0, y0 + 33.5), vec2(70.0, 12.0)) };
    let mresp = ui.interact(mrect, ui.id().with("master"), Sense::click());
    p.rect_filled(mrect, 1.0, if mresp.hovered() { lighten(BTN) } else { BTN });
    if is_master {
        p.rect_stroke(mrect, 1.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
    }
    p.text(mrect.center(), Align2::CENTER_CENTER, "MASTER", caps(8.0), WHITE);
    if tip(mresp, app, "MASTER").clicked() {
        app.set_master();
    }

    // ---- stem part buttons (MUTE / SOLO), right-aligned under the overview
    if stems_row > 0.0 {
        let sy = y0 + lower - 2.5 + phrase_h;
        // (Only as wide as its buttons, so the phrase bar beside it stays visible.)
        p.rect_filled(Rect::from_min_max(pos2(rx - 262.0, sy), pos2(rx, sy + 19.0)), 0.0, BG);
        let on = app.player.state.lock().stem_on;
        let three = app.prefs.stem_parts == 3;
        let parts: Vec<(&str, Vec<usize>, Color32)> = PART.iter().filter(|x| !(three && x.0 == "BASS")).map(|x| (x.0, if three && x.0 == "INST" { vec![2, 1] } else { vec![x.1] }, x.2)).collect();
        let mut bx = rx - 6.0 - parts.len() as f32 * 50.0 + 6.0;
        p.text(pos2(bx - 12.0, sy + 9.5), Align2::RIGHT_CENTER, if app.prefs.stem_solo { "SOLO" } else { "MUTE" }, FontId::proportional(10.0), hex(0x8C8C8C));
        for (i, (label, srcs, col)) in parts.iter().enumerate() {
            let br = Rect::from_min_size(pos2(bx, sy + 1.0), vec2(44.0, 17.0));
            let resp = ui.interact(br, ui.id().with(("part", i)), Sense::click());
            let lit = srcs.iter().all(|s| on[*s]);
            p.rect_filled(br, 1.0, if resp.hovered() { hex(0x1E1E1E) } else { hex(0x0F0F0F) });
            if lit {
                p.rect_stroke(br, 1.0, Stroke::new(1.0_f32, *col), egui::StrokeKind::Inside);
            }
            p.text(br.center(), Align2::CENTER_CENTER, *label, FontId::proportional(10.0), if lit { *col } else { hex(0x8C8C8C) });
            if resp.on_hover_text(format!("STEM MUTE ({label}):\nThe {label} Stem is output when set to on. The {label} Stem is muted when set to off.")).clicked() && loaded {
                crate::prefs_ext::stem_part_click(app, srcs);
            }
            bx += 50.0;
        }
    }

    // ---- lower part: pads, loop box and the platter panel. Deck 1 has the platter at the
    // mixer's side (right); deck 2 has it at the left, again at the mixer's side.
    let ly = y0 + lower + phrase_h + stems_row;
    p.rect_filled(Rect::from_min_max(pos2(x0, ly), r.max), 0.0, PAD);
    let right = n % 2 == 1;
    let (pads_x, loop_x, panel_x, tabs_x) = match (right, c4) {
        (false, false) => (38.0, w - 288.0, w - 210.0, 0.0),
        (true, false) => (297.0, 220.0, 9.0, w - 29.0),
        (false, true) => (38.0, w - 273.0, w - 195.0, 0.0),
        (true, true) => (285.0, 207.0, 10.0, w - 29.0),
    };
    let pads_w = if right { (w - 40.0 - pads_x).max(200.0) } else { (loop_x - 11.0 - 38.0).max(200.0) };
    p.rect_filled(Rect::from_min_max(pos2(x0 + tabs_x, ly), pos2(x0 + tabs_x + 28.0, r.max.y)), 0.0, PANEL);
    // Tab column: the pad view mark.
    let tc = pos2(x0 + tabs_x + 13.6, ly + if c4 { 24.0 } else { 34.0 });
    for (dx, dy) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
        p.rect_filled(Rect::from_center_size(tc + vec2(dx, dy), vec2(5.0, 5.0)), 0.0, WHITE);
    }
    let top = if c4 { 6.0 } else { 6.5 };
    pads(app, ui, Rect::from_min_size(pos2(x0 + pads_x, ly + top), vec2(pads_w, 110.0)), n, c4);
    loop_box(app, ui, pos2(x0 + loop_x, ly + top), n, c4);
    platter_panel(app, ui, pos2(x0 + panel_x, ly + if c4 { 0.0 } else { -3.0 }), n, is_active, c4);
}

/// Pad modes of `PlayerExt::pad_mode` beyond HOT CUE (0), STEMS (1), BEAT JUMP (2) and
/// BEAT LOOP (3).
pub const PAD_SAMPLER: u8 = 4;
pub const PAD_SEQ_CALL: u8 = 5;

/// The pad area: eight pads in two columns and the pad mode pull-down under them.
fn pads(app: &mut App, ui: &mut Ui, r: Rect, n: u8, c4: bool) {
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let mode = match app.px.pad_mode[n as usize] {
        1 if !app.prefs.show_stems => 0,
        m => m,
    };
    let page = app.deck.x.pad_page.min(1);
    let col_w = (r.width() - 3.0) / 2.0;
    // Pad rows: 19 pt on a 22.7 pt pitch; 15 pt on a 19.6 pt pitch in the 4-deck layout.
    let (pad_h, pitch) = if c4 { (15.0, 19.6) } else { (19.0, 22.7) };
    let pad = |c: usize, row: usize| Rect::from_min_size(pos2(r.min.x + c as f32 * (col_w + 3.0), r.min.y + row as f32 * pitch), vec2(col_w, pad_h));
    if mode == 1 {
        // STEMS: the parts on the left (sounding = outlined), their FX switches on the right.
        let on = app.player.state.lock().stem_on;
        let three = app.prefs.stem_parts == 3;
        for (row, (label, src, col)) in PART.iter().enumerate() {
            let enabled = !(three && *label == "BASS");
            let srcs: Vec<usize> = if three && *label == "INST" { vec![2, 1] } else { vec![*src] };
            let lit = srcs.iter().all(|s| on[*s]);
            for c in 0..2 {
                let pr = pad(c, row);
                let resp = ui.interact(pr, ui.id().with(("stempad", c, row)), Sense::click());
                let active = if c == 0 { lit } else { app.px.stem_fx[row] };
                p.rect_filled(pr, 0.0, if resp.hovered() && enabled { hex(0x1A1A1A) } else { hex(0x0A0A0A) });
                if active && enabled {
                    p.rect_stroke(pr, 0.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
                }
                let a = if enabled && loaded { 1.0 } else { 0.5 };
                p.rect_filled(Rect::from_min_size(pos2(pr.min.x + 3.0, pr.center().y - 4.0), vec2(8.0, 8.0)), 0.0, col.gamma_multiply(a));
                let text = if c == 0 { label.to_string() } else { format!("FX {label}") };
                p.text(pos2(pr.min.x + 15.0, pr.center().y), Align2::LEFT_CENTER, text, FontId::proportional(11.5), if !enabled || !loaded { OFF } else if active { hex(0x3C8CFF) } else { WHITE });
                if c == 0 {
                    if press_fired(ui, &resp) && enabled && loaded {
                        crate::prefs_ext::stem_part_click(app, &srcs);
                    }
                } else if resp.on_hover_text(format!("STEM FX ({label}):\nSet when applying effects to the {label} Stem.\n(recordbox has no effects yet: the switch is kept but does nothing.)")).clicked() && enabled {
                    app.px.stem_fx[row] = !app.px.stem_fx[row];
                }
            }
        }
    } else if mode == 2 {
        // BEAT JUMP: one cell per length, back on its left and forward on its right. Page 1:
        // 1 to 128 beats (the lengths rekordbox's pads default to); page 2: 1/8 to 1/2.
        let lens: [f64; 8] = if page == 0 { [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0] } else { [0.125, 0.25, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0] };
        for (i, b) in lens.iter().enumerate() {
            let pr = pad(i / 4, i % 4);
            p.rect_filled(pr, 0.0, hex(0x161616));
            if *b <= 0.0 {
                continue;
            }
            p.text(pr.center(), Align2::CENTER_CENTER, crate::player_ext::beats_text(*b), bold(10.5), if loaded { WHITE } else { OFF });
            for (dir, x) in [(-1.0, pr.min.x), (1.0, pr.max.x - 30.0)] {
                let br = Rect::from_min_size(pos2(x, pr.min.y), vec2(30.0, pr.height()));
                let resp = ui.interact(br, ui.id().with(("bjpad", i, dir as i32)), Sense::click());
                p.rect_filled(br, 0.0, if resp.hovered() && loaded { lighten(BTN) } else { BTN });
                let col = if loaded { WHITE } else { OFF };
                if dir < 0.0 {
                    crate::icons_player::chevron_left(&p, br.center(), 1.1, col);
                } else {
                    crate::icons_player::chevron_right(&p, br.center(), 1.1, col);
                }
                if press_fired(ui, &resp) && loaded {
                    app.beat_jump(dir * *b);
                }
            }
        }
    } else if mode == PAD_SAMPLER {
        // SAMPLER: the pads play the slots of the sampler's bank (1-8; the second page 9-16),
        // in the slot's colour with its sample's name. They follow the sequencer's MUTE /
        // ERASE modes, Shift stops, and an empty pad pressed while this deck loops captures
        // the loop (`Sampler.pad.csv`, `deviceSamplerPadSelectedSlot`).
        let bank = app.prefs.sampler.bank.min(3) as usize;
        let shift = ui.input(|i| i.modifiers.shift);
        let released = ui.input(|i| i.pointer.any_released());
        let pads_on = app.sampler_pads();
        for i in 0..8usize {
            let slot = i + 8 * page as usize;
            let idx = bank * 16 + slot;
            let pr = pad(i / 4, i % 4);
            let resp = ui.interact(pr, ui.id().with(("smppad", i)), Sense::click_and_drag());
            let used = slot < pads_on;
            let name = app.prefs.sampler.slots.get(idx).and_then(|s| s.as_ref()).map(|s| s.name.clone()).filter(|_| used);
            let ready = name.is_some() && app.px.sampler.ready(idx);
            let playing = ready && crate::sampler::SAMPLER.lock().as_ref().is_some_and(|e| e.slot_state(slot, bank).0);
            let muted = app.px.sampler.is_muted(idx);
            match &name {
                Some(nm) => {
                    let code = app.sampler_color_code(idx);
                    let col = if playing { hex(crate::sampler_ext::PAD_COLORS[code as usize]) } else { crate::sampler_ext::pad_color(code) };
                    let col = if muted || !ready { col.gamma_multiply(0.45) } else { col };
                    p.rect_filled(pr, 0.0, if resp.hovered() { lighten(col) } else { col });
                    p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, format!("{}", slot + 1), bold(10.5), hex(0xF4F4F4));
                    p.with_clip_rect(pr).text(pos2(pr.min.x + 26.0, pr.center().y), Align2::LEFT_CENTER, nm, FontId::proportional(10.5), hex(0xF4F4F4));
                }
                None => {
                    p.rect_filled(pr, 0.0, if resp.hovered() && used { hex(0x1E1E1E) } else { hex(0x161616) });
                    p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, format!("{}", slot + 1), FontId::proportional(10.5), if used { hex(0x8C8C8C) } else { hex(0x3C3C3C) });
                }
            }
            if press_fired(ui, &resp) && used {
                app.sampler_deck_pad_down(n, slot, shift);
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("smppad_held", n)), slot));
            }
            if released {
                if let Some(h) = ui.ctx().data_mut(|d| d.remove_temp::<usize>(egui::Id::new(("smppad_held", n)))) {
                    app.sampler_pad_up(h);
                }
            }
        }
    } else if mode == PAD_SEQ_CALL {
        // SEQ. CALL: the eight patterns ("P1".."P8", the name, "<n>Bars"). A press starts the
        // pattern, Shift + press stops it; right-click names it; the cross clears a saved one.
        let shift = ui.input(|i| i.modifiers.shift);
        let (cur, reserved, playing, bars) = match crate::sampler::SAMPLER.lock().as_ref() {
            Some(e) => (e.seq.current(), e.seq.reserved(), e.seq.playing, std::array::from_fn::<u8, 8, _>(|k| e.seq.pattern(k).bars)),
            None => (0, None, false, [1; 8]),
        };
        let blink = (ui.input(|i| i.time) / 0.3) as i64 % 2 == 0;
        for i in 0..8usize {
            let pr = pad(i / 4, i % 4);
            let resp = ui.interact(pr, ui.id().with(("seqpad", i)), Sense::click());
            let saved = app.prefs.sampler.seq.patterns.get(i).is_some_and(|x| x.is_some());
            let lit = (playing && cur == i && reserved.is_none()) || (reserved == Some(i) && blink);
            p.rect_filled(pr, 0.0, if lit { BLUE } else if resp.hovered() { hex(0x1E1E1E) } else { hex(0x161616) });
            let tc = if saved || lit { hex(0xF4F4F4) } else { hex(0x8C8C8C) };
            p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, format!("P{}", i + 1), bold(10.5), tc);
            p.with_clip_rect(Rect::from_min_max(pr.min, pos2(pr.max.x - 56.0, pr.max.y))).text(pos2(pr.min.x + 30.0, pr.center().y), Align2::LEFT_CENTER, app.prefs.sampler.seq.name(i), FontId::proportional(10.5), tc);
            p.text(pos2(pr.max.x - 20.0, pr.center().y), Align2::RIGHT_CENTER, if bars[i] == 1 { "1Bar".to_string() } else { format!("{}Bars", bars[i]) }, FontId::proportional(9.5), tc);
            if saved {
                let dr = Rect::from_center_size(pos2(pr.max.x - 9.0, pr.center().y), vec2(12.0, 12.0));
                let dresp = ui.interact(dr, ui.id().with(("seqpad_del", i)), Sense::click());
                let (c, col) = (dr.center(), if dresp.hovered() { WHITE } else { hex(0xA0A0A0) });
                p.line_segment([c + vec2(-3.0, -3.0), c + vec2(3.0, 3.0)], Stroke::new(1.2_f32, col));
                p.line_segment([c + vec2(-3.0, 3.0), c + vec2(3.0, -3.0)], Stroke::new(1.2_f32, col));
                if dresp.on_hover_text("Delete selected sequence").clicked() {
                    app.sampler_seq_delete(i);
                    continue;
                }
            }
            if press_fired(ui, &resp) {
                app.sampler_seq_call(i, shift);
            }
            resp.context_menu(|ui| {
                let mut name = app.prefs.sampler.seq.name(i);
                ui.label("Name");
                if ui.add(egui::TextEdit::singleline(&mut name).char_limit(32).desired_width(140.0)).changed() {
                    let sq = &mut app.prefs.sampler.seq;
                    sq.names.resize(8, String::new());
                    sq.names[i] = if name.trim() == format!("PATTERN {}", i + 1) { String::new() } else { name.trim().to_string() };
                    app.save_prefs();
                }
            });
        }
    } else if mode == 3 {
        // BEAT LOOP: the 16 lengths, 1/64 to 2 beats on the first page and 4 to 512 on the
        // second; the running loop's length is lit, and pressing it again ends the loop.
        let cur = app.loop_active().map(|(a, b)| (b - a) / app.beat_len().max(1e-9));
        for i in 0..8usize {
            let b = crate::player_ext::LOOP_SIZES[i + 8 * page as usize];
            let pr = pad(i / 4, i % 4);
            let resp = ui.interact(pr, ui.id().with(("blpad", i)), Sense::click());
            let lit = cur.map(|c| (c / b - 1.0).abs() < 0.02).unwrap_or(false);
            p.rect_filled(pr, 0.0, if lit { LOOP } else if resp.hovered() && loaded { hex(0x1E1E1E) } else { hex(0x161616) });
            p.text(pr.center(), Align2::CENTER_CENTER, crate::player_ext::beats_text(b), bold(10.5), if loaded { WHITE } else { OFF });
            if press_fired(ui, &resp) && loaded {
                if lit {
                    app.loop_exit();
                } else {
                    app.prefs.loop_beats = b;
                    app.auto_loop(b);
                }
            }
        }
    } else {
        // HOT CUE: A-D on the left, E-H on the right; the second page has I-P.
        for slot in (8 * page as usize)..(8 * page as usize + 8) {
            let pr = pad((slot % 8) / 4, slot % 4);
            let resp = ui.interact(pr, ui.id().with(("hotpad", slot)), Sense::click());
            let cue = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32).cloned();
            let letter = ((b'A' + slot as u8) as char).to_string();
            match &cue {
                Some(c) => {
                    let col = hotcue_color(c.color);
                    p.rect_filled(pr, 0.0, if resp.hovered() { lighten(col) } else { col });
                    p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, letter, bold(10.5), hex(0xF4F4F4));
                    let (m, f) = split_time(c.start);
                    p.with_clip_rect(pr).text(pos2(pr.min.x + 26.0, pr.center().y), Align2::LEFT_CENTER, if c.comment.is_empty() { format!("{m}.{f}") } else { c.comment.clone() }, FontId::proportional(10.5), hex(0xF4F4F4));
                }
                None => {
                    p.rect_filled(pr, 0.0, if resp.hovered() && loaded { hex(0x1E1E1E) } else { hex(0x161616) });
                    p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, letter, FontId::proportional(10.5), if loaded { hex(0x8C8C8C) } else { hex(0x3C3C3C) });
                }
            }
            if press_fired(ui, &resp) && loaded {
                app.hot_cue_down(slot as i32);
            }
            let held = app.deck.hot_preview.is_some() || app.player.state.lock().slip_hold;
            if held && !ui.input(|i| i.pointer.any_down()) {
                app.hot_cue_up();
            }
            if let Some(c) = cue {
                resp.context_menu(|ui| crate::ui_player::hot_cue_menu(app, ui, &c));
            }
        }
    }
    // Pad mode pull-down (deck 1 at the left under the pads, deck 2 at the right).
    let label = match mode {
        1 => "STEMS",
        2 => "BEAT JUMP",
        3 => "BEAT LOOP",
        PAD_SAMPLER => "SAMPLER",
        PAD_SEQ_CALL => "SEQ. CALL",
        _ => "HOT CUE",
    };
    let mw = 70.0;
    let mr = Rect::from_min_size(pos2(if n % 2 == 0 { r.min.x } else { r.max.x - mw }, r.min.y + if c4 { 81.0 } else { 91.5 }), vec2(mw, if c4 { 15.0 } else { 17.0 }));
    let resp = ui.interact(mr, ui.id().with("padmode"), Sense::click());
    p.rect_filled(mr, 2.0, if resp.hovered() { lighten(BTN) } else { PANEL });
    p.text(pos2(mr.min.x + 5.0, mr.center().y), Align2::LEFT_CENTER, label, caps(8.5), WHITE);
    crate::icons_player::chevron_down(&p, pos2(mr.max.x - 9.0, mr.center().y + 0.5), 1.0, WHITE);
    let stems_on = app.prefs.show_stems;
    crate::ui_browser::dropdown(ui, &resp, "padmodepop", |ui| {
        if ui.selectable_label(mode == 0, "HOT CUE").clicked() {
            app.px.pad_mode[n as usize] = 0;
            ui.memory_mut(|m| m.close_popup());
        }
        if ui.add_enabled(stems_on, egui::SelectableLabel::new(mode == 1, "STEMS")).on_disabled_hover_text("Enable the STEMS Function in Preferences > Extensions > STEMS.").clicked() {
            app.px.pad_mode[n as usize] = 1;
            ui.memory_mut(|m| m.close_popup());
        }
        for (m, name) in [(2u8, "BEAT JUMP"), (3, "BEAT LOOP")] {
            if ui.selectable_label(mode == m, name).clicked() {
                app.px.pad_mode[n as usize] = m;
                ui.memory_mut(|m| m.close_popup());
            }
        }
        if ui.selectable_label(mode == PAD_SEQ_CALL, "SEQ. CALL").clicked() {
            app.px.pad_mode[n as usize] = PAD_SEQ_CALL;
            ui.memory_mut(|m| m.close_popup());
        }
        for name in ["PAD FX", "SLICER", "KEYBOARD", "KEY SHIFT", "ACT. CENSR", "MEMORY CUE"] {
            ui.add_enabled(false, egui::SelectableLabel::new(false, name)).on_disabled_hover_text("Not built in recordbox yet.");
        }
        // (In rekordbox SAMPLER is a pad mode of the controllers only; recordbox has no
        // controller pads yet, so it is offered here.)
        ui.separator();
        if ui.selectable_label(mode == PAD_SAMPLER, "SAMPLER").clicked() {
            app.px.pad_mode[n as usize] = PAD_SAMPLER;
            ui.memory_mut(|m| m.close_popup());
        }
    });
    // Page buttons beside the pull-down (HOT CUE: A-H / I-P; BEAT JUMP and BEAT LOOP: their
    // second page). Where rekordbox puts its page control on screen is not known.
    if mode != 1 && mode != PAD_SEQ_CALL {
        let px = if n % 2 == 0 { mr.max.x + 6.0 } else { mr.min.x - 52.0 };
        let names = match mode {
            2 => ["1 - 128", "1/8 - 1/2"],
            3 => ["1/64 - 2", "4 - 512"],
            PAD_SAMPLER => ["1 - 8", "9 - 16"],
            _ => ["A - H", "I - P"],
        };
        let pr = Rect::from_min_size(pos2(px, mr.min.y), vec2(46.0, mr.height()));
        let resp = ui.interact(pr, ui.id().with("padpage"), Sense::click());
        p.rect_filled(pr, 2.0, if resp.hovered() { lighten(BTN) } else { PANEL });
        p.text(pr.center(), Align2::CENTER_CENTER, names[page as usize], FontId::proportional(9.0), DIM2);
        if resp.on_hover_text("Page").clicked() {
            app.deck.x.pad_page = 1 - page;
        }
    }
}

/// Loop size with halve / double, and AU (auto beat loop) or MA (manual: IN, OUT, RELOOP/EXIT).
fn loop_box(app: &mut App, ui: &mut Ui, o: Pos2, n: u8, c4: bool) {
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let looping = app.loop_active().is_some();
    let manual = app.px.loop_manual[n as usize];
    // (size box, arrows' top and height, AU/MA's top and height): 2-deck or 4-deck layout.
    let (sh, ay, ah, by, bh) = if c4 { (30.5, 35.0, 28.5, 69.0, 16.0) } else { (31.5, 33.5, 32.0, 69.5, 17.0) };
    let size_r = Rect::from_min_size(o, vec2(68.0, sh));
    let lr = Rect::from_min_size(o + vec2(0.0, ay), vec2(33.0, ah));
    let rr = Rect::from_min_size(o + vec2(35.0, ay), vec2(33.0, ah));
    if manual {
        let re = ui.interact(size_r, ui.id().with("reloop"), Sense::click());
        p.rect_filled(size_r, 0.0, if looping { LOOP } else if re.hovered() { lighten(BTN) } else { BTN });
        p.text(size_r.center(), Align2::CENTER_CENTER, if looping { "EXIT" } else { "RELOOP" }, caps(9.0), WHITE);
        if tip(re, app, "Reloop/Exit").clicked() && loaded {
            app.exit_reloop();
        }
        let li = ui.interact(lr, ui.id().with("loopin"), Sense::click());
        p.rect_filled(lr, 0.0, if li.hovered() { lighten(BTN) } else { BTN });
        p.text(lr.center(), Align2::CENTER_CENTER, "IN", caps(9.0), WHITE);
        if tip(li, app, "Loop In").clicked() && loaded {
            app.loop_in_pressed();
        }
        let lo = ui.interact(rr, ui.id().with("loopout"), Sense::click());
        p.rect_filled(rr, 0.0, if lo.hovered() { lighten(BTN) } else { BTN });
        p.text(rr.center(), Align2::CENTER_CENTER, "OUT", caps(9.0), WHITE);
        if tip(lo, app, "Loop Out").clicked() && loaded {
            app.loop_out_pressed();
        }
    } else {
        let sz = ui.interact(size_r, ui.id().with("loopsize"), Sense::click());
        p.rect_filled(size_r, 0.0, if looping { LOOP } else if sz.hovered() { lighten(BTN) } else { BTN });
        p.text(size_r.center(), Align2::CENTER_CENTER, crate::player_ext::beats_text(app.prefs.loop_beats), bold(12.0), WHITE);
        if tip(sz, app, "Auto Beat Loop").clicked() && loaded {
            if looping {
                app.loop_exit();
            } else {
                app.auto_loop(app.prefs.loop_beats);
            }
        }
        let hl = ui.interact(lr, ui.id().with("loophalf"), Sense::click());
        p.rect_filled(lr, 0.0, if hl.hovered() { lighten(BTN) } else { BTN });
        crate::icons_player::chevron_left(&p, lr.center(), 1.35, WHITE);
        if tip(hl, app, "Loop Halve").clicked() {
            app.resize_loop(0.5);
        }
        let dl = ui.interact(rr, ui.id().with("loopdouble"), Sense::click());
        p.rect_filled(rr, 0.0, if dl.hovered() { lighten(BTN) } else { BTN });
        crate::icons_player::chevron_right(&p, rr.center(), 1.35, WHITE);
        if tip(dl, app, "Loop Double").clicked() {
            app.resize_loop(2.0);
        }
    }
    let au = Rect::from_min_size(o + vec2(0.0, by), vec2(33.0, bh));
    let ma = Rect::from_min_size(o + vec2(35.0, by), vec2(33.0, bh));
    if flat(ui, au, "loopau", "AU", 8.5, if manual { BTN } else { BLUE }, WHITE).clicked() {
        app.px.loop_manual[n as usize] = false;
    }
    let mresp = ui.interact(ma, ui.id().with("loopma"), Sense::click());
    p.rect_filled(ma, 2.0, if manual { BLUE } else { PAD });
    if !manual {
        p.rect_stroke(ma, 2.0, Stroke::new(1.0_f32, hex(0x505050)), egui::StrokeKind::Inside);
    }
    p.text(ma.center(), Align2::CENTER_CENTER, "MA", caps(8.5), WHITE);
    if mresp.clicked() {
        app.px.loop_manual[n as usize] = true;
    }
}

/// rekordbox's `VirtualDeckPanel` (2Deck Horizontal): the play mode, CUE and PLAY/PAUSE, the
/// platter with BPM, tempo and tempo range, and SLIP / Q / MT. `o`: the panel's top-left.
fn platter_panel(app: &mut App, ui: &mut Ui, o: Pos2, n: u8, is_active: bool, c4: bool) {
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(o + vec2(x, y), vec2(w, h));
    // Play mode: INT (internal) only.
    // rekordbox's panel numbers for the 2-deck layout (platter 120 at (45, 5), CUE and PLAY
    // 36 at (2, 38) and (2, 84), SLIP / Q / MT at x 170, y 10 / 38 / 102) and for the 4-deck
    // one (platter 108 at (45, 2), buttons 30 at (5, 33) and (5, 69), column at x 158,
    // y 7 / 35 / 87).
    let (int_y, cue_c, play_c, rad, pc, d, col_x, ys, sc) = if c4 { (9.0, vec2(20.0, 48.0), vec2(20.0, 84.0), 13.5, vec2(99.0, 56.0), 98.0, 158.0, [7.0, 35.0, 87.0], 0.89) } else { (11.0, vec2(20.0, 56.0), vec2(20.0, 102.0), 16.5, vec2(105.0, 65.0), 110.0, 170.0, [10.0, 38.0, 102.0], 1.0) };
    let int = ui.interact(at(0.0, int_y, 40.0, 17.0), ui.id().with("playmode"), Sense::hover());
    p.rect_filled(at(0.0, int_y, 40.0, 17.0), 2.0, PANEL);
    p.text(o + vec2(5.0, int_y + 8.5), Align2::LEFT_CENTER, "INT", caps(8.5), WHITE);
    crate::icons_player::chevron_down(&p, o + vec2(31.0, int_y + 9.0), 1.0, WHITE);
    let _ = int.on_hover_text("INTERNAL: the deck plays from the software. (DVS modes are not built in recordbox.)");
    crate::ui_player2::cue_play(app, ui, o + cue_c, o + play_c, rad, is_active);

    // Platter: 120 pt square at (45, 5); rings as fractions of d = 110.
    let c = o + pc;
    let playing = app.playing();
    let pos = app.pos();
    p.circle_filled(c, d / 2.0, if loaded { WHITE } else { hex(0x2A2A2A) });
    p.circle_filled(c, 0.93636 * d / 2.0, PANEL);
    p.circle_filled(c, 0.8 * d / 2.0, if loaded { BLUE } else { hex(0x141E2A) });
    p.circle_filled(c, 0.76364 * d / 2.0, BG);
    if loaded {
        // The position marker turns at 33 1/3 rpm.
        let a = (pos * (100.0 / 3.0 / 60.0) * std::f64::consts::TAU) as f32 - std::f32::consts::FRAC_PI_2;
        let m = c + vec2(a.cos(), a.sin()) * (0.968 * d / 2.0);
        p.circle_filled(m, 3.2, hex(0xE61E1E));
    }
    let plat = ui.interact(Rect::from_center_size(c, vec2(84.0 * sc, 84.0 * sc)), ui.id().with("platter"), Sense::hover());
    let bpm = if loaded { app.effective_bpm() } else { 0.0 };
    let tcol = if loaded { hex(0xE0E0E0) } else { hex(0x3C3C3C) };
    let whole = format!("{}", bpm.floor() as i64);
    let frac = format!(".{:02}", ((bpm - bpm.floor()) * 100.0).round().min(99.0) as i64);
    let g = p.layout_no_wrap(if loaded { whole } else { "00".into() }, bold(18.0 * sc), tcol);
    let g2 = p.layout_no_wrap(if loaded { frac } else { ".00".into() }, bold(12.5 * sc), tcol);
    let tw = g.size().x + g2.size().x;
    let bx = c.x - tw / 2.0;
    let by = c.y - 8.0 * sc;
    let (h1, h2) = (g.size().y, g2.size().y);
    p.galley(pos2(bx, by - h1 / 2.0), g.clone(), tcol);
    p.galley(pos2(bx + g.size().x, by - h1 / 2.0 + (h1 - h2) - 2.0), g2, tcol);
    // Tempo in percent (one decimal, half away from zero) and the range; a click on the
    // range steps it (±6, ±10, ±16, WIDE).
    let pct = (app.deck_speed() - 1.0) * 100.0;
    let shown = ((pct.abs() + 0.005) * 10.0).floor() / 10.0;
    let sign = if pct < -0.05 { "-" } else if pct > 0.05 { "+" } else { "" };
    p.text(pos2(c.x - 2.0, c.y + 12.5 * sc), Align2::RIGHT_CENTER, format!("{sign}{shown:.1}%"), bold(8.5 * sc), if loaded { hex(0x969696) } else { hex(0x3C3C3C) });
    let range = app.tempo_range() * 100.0;
    let rl = if range >= 100.0 { "WIDE".to_string() } else { format!("±{range:.0}") };
    let rr = Rect::from_min_size(pos2(c.x + 1.0, c.y + 6.0 * sc), vec2(30.0, 13.0));
    let rresp = ui.interact(rr, ui.id().with("temporange"), Sense::click());
    p.text(pos2(c.x + 3.0, c.y + 12.5 * sc), Align2::LEFT_CENTER, rl, bold(8.5 * sc), if loaded { hex(0xE6461E) } else { hex(0x3C3C3C) });
    if tip(rresp, app, "Tempo Range").clicked() {
        app.tempo_range_next();
    }
    // Tempo − / + and RESET appear inside the platter under the pointer.
    if loaded && (plat.hovered() || ui.rect_contains_pointer(Rect::from_center_size(c, vec2(96.0, 96.0)))) {
        let hb = |dx: f32, w: f32| Rect::from_min_size(c + vec2(dx * sc, 18.0 * sc), vec2(w, 13.0));
        let minus = hb(-30.0, 13.0);
        let plus = hb(17.0, 13.0);
        let reset = hb(-15.0, 30.0);
        let mr = ui.interact(minus, ui.id().with("tminus"), Sense::click());
        crate::icons_player::tempo_sign(&p, minus.center(), false, WHITE);
        let held = mr.is_pointer_button_down_on() && repeat(ui, &mr);
        if tip(mr, app, "Tempo -").clicked() || held {
            app.tempo_step(-1);
        }
        let pr = ui.interact(plus, ui.id().with("tplus"), Sense::click());
        crate::icons_player::tempo_sign(&p, plus.center(), true, WHITE);
        let held = pr.is_pointer_button_down_on() && repeat(ui, &pr);
        if tip(pr, app, "Tempo +").clicked() || held {
            app.tempo_step(1);
        }
        let rs = ui.interact(reset, ui.id().with("treset"), Sense::click());
        p.text(reset.center(), Align2::CENTER_CENTER, "RESET", caps(7.0), WHITE);
        if tip(rs, app, "Tempo Reset").clicked() {
            app.tempo_reset();
        }
    }
    let _ = playing;

    // SLIP (not built), Q, MT.
    // SLIP: lit when on; blinks (500 ms) while the track runs on in the background.
    let slip_on = app.deck.x.slip;
    let blink = slip_on && app.slip_running() && (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
    if app.slip_running() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    if tip(flat(ui, at(col_x, ys[0], 30.0, 18.0), "slip", "SLIP", 8.0, if slip_on && !blink { BLUE } else { PANEL }, WHITE), app, "Slip").clicked() {
        app.toggle_slip();
    }
    let q = app.prefs.quantize;
    if tip(flat(ui, at(col_x, ys[1], 30.0, 18.0), "quant", "Q", 8.5, if q { BLUE } else { BTN }, WHITE), app, "Quantize").clicked() {
        app.prefs.quantize = !q;
        app.save_prefs();
    }
    let mt = app.deck.x.master_tempo;
    if tip(flat(ui, at(col_x, ys[2], 30.0, 18.0), "mt", "MT", 8.5, if mt { BLUE } else { PANEL }, WHITE), app, "Master Tempo").clicked() {
        app.toggle_master_tempo();
    }
    let _ = n;
}

/// A held button repeats every 50 ms after 0.4 s.
fn repeat(ui: &Ui, resp: &egui::Response) -> bool {
    ui.ctx().request_repaint();
    let key = resp.id.with("repeat");
    let now = ui.input(|i| i.time);
    let held = ui.input(|i| i.pointer.press_start_time()).map(|s| now - s).unwrap_or(0.0);
    if held > 0.4 {
        let last = ui.ctx().data(|d| d.get_temp::<f64>(key)).unwrap_or(0.0);
        if now - last >= 0.05 {
            ui.ctx().data_mut(|d| d.insert_temp(key, now));
            return true;
        }
    }
    false
}

/// A rotary knob 24 pt across: drag up or down to turn (a full sweep over 150 pt), double
/// click for `reset`. Returns the new value when it changed.
pub(crate) fn turn_knob(ui: &mut Ui, c: Pos2, id: impl std::hash::Hash, value: f32, reset: f32) -> (egui::Response, Option<f32>) {
    let r = Rect::from_center_size(c, vec2(24.0, 24.0));
    let resp = ui.interact(r, ui.id().with(id), Sense::click_and_drag());
    let mut out = None;
    if resp.double_clicked() {
        out = Some(reset);
    } else if resp.dragged() {
        let dy = resp.drag_delta().y;
        if dy != 0.0 {
            out = Some((value - dy / 150.0).clamp(0.0, 1.0));
        }
    }
    (resp, out)
}

pub(crate) fn knob(ui: &mut Ui, c: Pos2, id: impl std::hash::Hash, value: f32, reset: f32, colour: Color32) -> Option<f32> {
    let (resp, out) = turn_knob(ui, c, id, value, reset);
    let p = ui.painter();
    let v = out.unwrap_or(value);
    // The track: a 270 degree arc open at the bottom; the pointer: a short line.
    let ang = |x: f32| (-225.0 + 270.0 * x).to_radians();
    let pts: Vec<Pos2> = (0..=36).map(|i| c + vec2(ang(i as f32 / 36.0).cos(), ang(i as f32 / 36.0).sin()) * 9.5).collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.5_f32, if resp.hovered() || resp.dragged() { hex(0x464646) } else { hex(0x323232) })));
    let a = ang(v);
    p.line_segment([c + vec2(a.cos(), a.sin()) * 6.5, c + vec2(a.cos(), a.sin()) * 11.0], Stroke::new(2.0_f32, colour));
    out
}

/// The mixer column: per deck TRIM, HIGH / MID / LOW (or VOCAL / INST / DRUMS in STEM ISO
/// mode), headphones CUE, the channel fader and the STEM button; the level meters between.
fn mixer(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    let (x0, y0) = (r.min.x, r.min.y);
    let stems = app.prefs.show_stems;
    let mut changed = false;
    for n in 0..2usize {
        let cx = x0 + if n == 0 { 24.5 } else { 77.0 };
        let mut m = app.px.mix[n];
        let lab = |p: &Painter, y: f32, s: &str, col: Color32| {
            p.text(pos2(cx, y0 + y), Align2::CENTER_CENTER, s, caps(8.0), col);
        };
        // TRIM
        if let Some(v) = knob(ui, pos2(cx, y0 + 18.5), ("trim", n), m.trim, 0.5, WHITE) {
            m.trim = v;
            changed = true;
        }
        let tl = ui.interact(Rect::from_center_size(pos2(cx, y0 + 32.5), vec2(36.0, 10.0)), ui.id().with(("triml", n)), Sense::hover());
        lab(&p, 32.5, "TRIM", WHITE);
        let tdb = crate::mixer::trim_db(m.trim);
        let _ = tl.on_hover_text(if tdb.is_finite() { format!("{tdb:+.1}dB") } else { "-inf dB".to_string() });
        // HIGH / MID / LOW, or the parts in STEM ISO mode. A click on a label sets it to 0
        // (and back to the centre).
        let iso = m.stem_iso_on && stems;
        let names: [(&str, usize); 3] = if iso { [("VOCAL", 2), (if app.prefs.stem_parts == 3 { "INST" } else { "INST/BA." }, 1), ("DRUMS", 0)] } else { [("HIGH", 2), ("MID", 1), ("LOW", 0)] };
        for (i, (name, idx)) in names.iter().enumerate() {
            let ky = 51.0 + 33.3 * i as f32;
            let val = if iso { m.stem_iso[*idx] } else { m.eq[*idx] };
            let col = if iso { PART[[3, 1, 0][*idx]].2 } else { WHITE };
            let new = knob(ui, pos2(cx, y0 + ky), ("eq", n, i), val, 0.5, col);
            let lr = ui.interact(Rect::from_center_size(pos2(cx, y0 + ky + 14.0), vec2(40.0, 10.0)), ui.id().with(("eql", n, i)), Sense::click());
            lab(&p, ky + 14.0, name, if val < 0.01 { hex(0xE61E1E) } else { WHITE });
            let killed = lr.on_hover_text("Click to change the value to [0]").clicked();
            let v = if killed { Some(if val < 0.01 { 0.5 } else { 0.0 }) } else { new };
            if let Some(v) = v {
                if iso {
                    m.stem_iso[*idx] = v;
                } else {
                    m.eq[*idx] = v;
                }
                changed = true;
            }
        }
        // Headphones CUE.
        let cr = Rect::from_center_size(pos2(cx, y0 + 155.0), vec2(33.0, 15.0));
        let cresp = ui.interact(cr, ui.id().with(("cue", n)), Sense::click());
        p.rect_filled(cr, 2.0, if m.cue { hex(0xFFA500) } else if cresp.hovered() { lighten(BTN) } else { BTN });
        p.text(cr.center(), Align2::CENTER_CENTER, "CUE", caps(8.5), if m.cue { BG } else { WHITE });
        let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
        if cresp.on_hover_text(if hp_on { "Headphones CUE" } else { "Headphones CUE.\nNo headphones output is set: choose one in Preferences > Audio > Input/Output (it needs a device with four or more outputs)." }).clicked() {
            m.cue = !m.cue;
            changed = true;
        }
        // Channel fader: 50 pt of travel, the level shown in blue under the handle.
        let (ft, fb) = (y0 + 169.5, y0 + 219.5);
        let fr = Rect::from_min_max(pos2(cx - 12.0, ft - 4.0), pos2(cx + 12.0, fb + 4.0));
        let fresp = ui.interact(fr, ui.id().with(("fader", n)), Sense::click_and_drag());
        // (Also on `clicked`: a quick click's press and release can fall into one frame.)
        if let Some(pp) = fresp.interact_pointer_pos().filter(|_| fresp.dragged() || fresp.is_pointer_button_down_on() || fresp.clicked()) {
            m.fader = ((fb - pp.y) / (fb - ft)).clamp(0.0, 1.0);
            changed = true;
        }
        if fresp.double_clicked() {
            m.fader = 1.0;
            changed = true;
        }
        let hy = fb - m.fader * (fb - ft);
        p.rect_filled(Rect::from_min_max(pos2(cx - 1.0, ft), pos2(cx + 1.0, fb)), 0.0, hex(0x323232));
        p.rect_filled(Rect::from_min_max(pos2(cx - 1.0, hy), pos2(cx + 1.0, fb)), 0.0, BLUE);
        p.rect_filled(Rect::from_center_size(pos2(cx, hy), vec2(24.0, 2.0)), 0.0, if fresp.hovered() || fresp.dragged() { WHITE } else { hex(0x8C8C8C) });
        // STEM (STEM ISO mode), in the extra row the stem part buttons add.
        if stems {
            let sr = Rect::from_center_size(pos2(cx, y0 + 234.5), vec2(33.0, 15.0));
            let sresp = ui.interact(sr, ui.id().with(("stemiso", n)), Sense::click());
            p.rect_filled(sr, 2.0, if m.stem_iso_on { BLUE } else if sresp.hovered() { lighten(BTN) } else { BTN });
            p.text(sr.center(), Align2::CENTER_CENTER, "STEM", caps(8.5), WHITE);
            if sresp.on_hover_text("STEM ISO Mode:\nSet to STEM ISO mode when set to on, enabling volume adjustment of each Stem. Set to EQ mode when set to off, enabling frequency band adjustment for HIGH/MID/LOW.").clicked() {
                m.stem_iso_on = !m.stem_iso_on;
                changed = true;
            }
        } else {
            m.stem_iso_on = false;
        }
        // Level meter: 2 pt wide, between the strips, from LOW up to TRIM.
        let mx = x0 + if n == 0 { 44.5 } else { 57.5 };
        let (mt, mb) = (y0 + 6.0, y0 + 135.0);
        p.rect_filled(Rect::from_min_max(pos2(mx, mt), pos2(mx + 2.0, mb)), 0.0, hex(0x323232));
        let dbv = m.shown_db[0].max(m.shown_db[1]);
        meter_bar(&p, Rect::from_min_max(pos2(mx, mt), pos2(mx + 2.0, mb)), dbv, true);
        app.px.mix[n] = m;
    }
    if changed {
        app.poll_decks();
    }
}

/// The 4-deck mixer column: four strips in rekordbox's order 3, 1, 2, 4 (measured on
/// reference performance/03: strip centres 30, 74, 118.5 and 162.5 pt into the 189 pt column;
/// deck number 14 pt down, TRIM 40, HIGH 80, MID 120, LOW 160 with their labels 15 pt under,
/// CUE 198, the fader from 224.5 to 344).
fn mixer4(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    let (x0, y0) = (r.min.x, r.min.y);
    let stems = app.prefs.show_stems;
    let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
    let mut changed = false;
    for (col, n) in [2usize, 0, 1, 3].into_iter().enumerate() {
        let cx = x0 + [30.0, 74.0, 118.5, 162.5][col];
        let mut m = app.px.mix[n];
        p.text(pos2(cx, y0 + 14.0), Align2::CENTER_CENTER, format!("{}", n + 1), FontId::proportional(11.5), WHITE);
        if let Some(v) = knob(ui, pos2(cx, y0 + 40.0), ("trim4", n), m.trim, 0.5, WHITE) {
            m.trim = v;
            changed = true;
        }
        let tl = ui.interact(Rect::from_center_size(pos2(cx, y0 + 55.0), vec2(36.0, 10.0)), ui.id().with(("triml4", n)), Sense::hover());
        p.text(pos2(cx, y0 + 55.0), Align2::CENTER_CENTER, "TRIM", caps(8.0), WHITE);
        let tdb = crate::mixer::trim_db(m.trim);
        let _ = tl.on_hover_text(if tdb.is_finite() { format!("{tdb:+.1}dB") } else { "-inf dB".to_string() });
        let iso = m.stem_iso_on && stems;
        let names: [(&str, usize); 3] = if iso { [("VOCAL", 2), (if app.prefs.stem_parts == 3 { "INST" } else { "INST/BA." }, 1), ("DRUMS", 0)] } else { [("HIGH", 2), ("MID", 1), ("LOW", 0)] };
        for (i, (name, idx)) in names.iter().enumerate() {
            let ky = 80.0 + 40.0 * i as f32;
            let val = if iso { m.stem_iso[*idx] } else { m.eq[*idx] };
            let col_k = if iso { PART[[3, 1, 0][*idx]].2 } else { WHITE };
            let new = knob(ui, pos2(cx, y0 + ky), ("eq4", n, i), val, 0.5, col_k);
            let lr = ui.interact(Rect::from_center_size(pos2(cx, y0 + ky + 15.0), vec2(40.0, 10.0)), ui.id().with(("eql4", n, i)), Sense::click());
            p.text(pos2(cx, y0 + ky + 15.0), Align2::CENTER_CENTER, *name, caps(if iso { 7.0 } else { 8.0 }), if val < 0.01 { hex(0xE61E1E) } else { WHITE });
            let killed = lr.on_hover_text("Click to change the value to [0]").clicked();
            let v = if killed { Some(if val < 0.01 { 0.5 } else { 0.0 }) } else { new };
            if let Some(v) = v {
                if iso {
                    m.stem_iso[*idx] = v;
                } else {
                    m.eq[*idx] = v;
                }
                changed = true;
            }
        }
        let cr = Rect::from_center_size(pos2(cx, y0 + 198.0), vec2(33.0, 15.0));
        let cresp = ui.interact(cr, ui.id().with(("cue4", n)), Sense::click());
        p.rect_filled(cr, 2.0, if m.cue { hex(0xFFA500) } else if cresp.hovered() { lighten(BTN) } else { BTN });
        p.text(cr.center(), Align2::CENTER_CENTER, "CUE", caps(8.5), if m.cue { BG } else { WHITE });
        if cresp.on_hover_text(if hp_on { "Headphones CUE" } else { "Headphones CUE.\nNo headphones output is set: choose one in Preferences > Audio > Input/Output (it needs a device with four or more outputs)." }).clicked() {
            m.cue = !m.cue;
            changed = true;
        }
        let (ft, fb) = (y0 + 224.5, y0 + 344.0);
        let fr = Rect::from_min_max(pos2(cx - 12.0, ft - 4.0), pos2(cx + 12.0, fb + 4.0));
        let fresp = ui.interact(fr, ui.id().with(("fader4", n)), Sense::click_and_drag());
        if let Some(pp) = fresp.interact_pointer_pos().filter(|_| fresp.dragged() || fresp.is_pointer_button_down_on() || fresp.clicked()) {
            m.fader = ((fb - pp.y) / (fb - ft)).clamp(0.0, 1.0);
            changed = true;
        }
        if fresp.double_clicked() {
            m.fader = 1.0;
            changed = true;
        }
        let hy = fb - m.fader * (fb - ft);
        p.rect_filled(Rect::from_min_max(pos2(cx - 1.0, ft), pos2(cx + 1.0, fb)), 0.0, hex(0x323232));
        p.rect_filled(Rect::from_min_max(pos2(cx - 1.0, hy), pos2(cx + 1.0, fb)), 0.0, BLUE);
        p.rect_filled(Rect::from_center_size(pos2(cx, hy), vec2(24.0, 2.0)), 0.0, if fresp.hovered() || fresp.dragged() { WHITE } else { hex(0x8C8C8C) });
        // Level meter: 2 pt wide, left of the fader.
        let mrect = Rect::from_min_max(pos2(cx - 17.0, ft), pos2(cx - 15.0, fb));
        p.rect_filled(mrect, 0.0, hex(0x282828));
        meter_bar(&p, mrect, m.shown_db[0].max(m.shown_db[1]), true);
        if stems {
            let sr = Rect::from_center_size(pos2(cx, y0 + 362.0), vec2(33.0, 15.0));
            let sresp = ui.interact(sr, ui.id().with(("stemiso4", n)), Sense::click());
            p.rect_filled(sr, 2.0, if m.stem_iso_on { BLUE } else if sresp.hovered() { lighten(BTN) } else { BTN });
            p.text(sr.center(), Align2::CENTER_CENTER, "STEM", caps(8.5), WHITE);
            if sresp.on_hover_text("STEM ISO Mode:\nSet to STEM ISO mode when set to on, enabling volume adjustment of each Stem. Set to EQ mode when set to off, enabling frequency band adjustment for HIGH/MID/LOW.").clicked() {
                m.stem_iso_on = !m.stem_iso_on;
                changed = true;
            }
        } else {
            m.stem_iso_on = false;
        }
        app.px.mix[n] = m;
    }
    if changed {
        app.poll_decks();
    }
}

/// A level meter bar filling `r` from the bottom (vertical) or the left: -40 dB empty, 0 dB
/// full; green, orange above -6 dB, red at full scale.
fn meter_bar(p: &Painter, r: Rect, db: f32, vertical: bool) {
    let f = ((db + 40.0) / 40.0).clamp(0.0, 1.0);
    if f <= 0.0 {
        return;
    }
    let col = if db >= -0.1 { hex(0xF00000) } else if db > -6.0 { hex(0xFF8C19) } else { hex(0x2DD728) };
    let fill = if vertical { Rect::from_min_max(pos2(r.min.x, r.max.y - r.height() * f), r.max) } else { Rect::from_min_max(r.min, pos2(r.min.x + r.width() * f, r.max.y)) };
    p.rect_filled(fill, 0.0, col);
}

/// Read the level meters from the audio thread about every 36 ms: a higher peak shows at
/// once, otherwise the level falls 2 dB per reading.
pub(crate) fn read_meters(app: &mut App, ui: &Ui) {
    let now = ui.input(|i| i.time);
    let key = egui::Id::new("master_meter_time");
    let last = ui.ctx().data(|d| d.get_temp::<f64>(key)).unwrap_or(now - 0.036);
    let elapsed = (now - last) as f32;
    let mut next = 0.036;
    if elapsed >= next {
        ui.ctx().data_mut(|d| d.insert_temp(key, now));
        if app.performance() {
            for (n, deck) in app.player.decks.iter().enumerate() {
                let peaks = std::mem::take(&mut deck.lock().peak);
                for (db, peak) in app.px.mix[n].shown_db.iter_mut().zip(peaks) {
                    *db = crate::mixer::meter_fall(*db, peak, elapsed);
                }
            }
        }
        for (db, peak) in app.px.master_db.iter_mut().zip(crate::audio::take_master_peak()) {
            *db = crate::mixer::meter_fall(*db, peak, elapsed);
        }
    } else {
        next -= elapsed;
    }
    // Playback schedules its own frames. Only wake a paused window while a
    // visible meter is falling or the sampler is still producing sound.
    let visible = app.px.master_db.iter().any(|db| *db > -40.0)
        || (app.performance() && app.px.mix.iter().any(|strip| strip.shown_db.iter().any(|db| *db > -100.0)));
    if visible && !app.player.transport_running() {
        ui.ctx().request_repaint_after_secs(next + ui.input(|i| i.predicted_dt));
    }
}

/// The mixer's bottom row: headphones MIX and LEVEL, the crossfader with its assign buttons.
fn bottom_row(app: &mut App, ui: &mut Ui, r: Rect, cx: f32) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    let cy = r.min.y + 21.5;
    let x = |v: f32| cx + (v - 756.0);
    // Headphones.
    let hc = pos2(x(450.0), cy);
    p.add(egui::Shape::line((0..=16).map(|i| { let a = (180.0 + 180.0 * i as f32 / 16.0).to_radians(); hc + vec2(a.cos() * 7.5, a.sin() * 7.5 + 2.0) }).collect(), Stroke::new(1.8_f32, WHITE)));
    p.rect_filled(Rect::from_min_size(hc + vec2(-9.0, 1.5), vec2(3.5, 7.0)), 1.0, WHITE);
    p.rect_filled(Rect::from_min_size(hc + vec2(5.5, 1.5), vec2(3.5, 7.0)), 1.0, WHITE);
    let note = if crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) { "Headphones: MIX balances the cued decks (left) and the master (right); LEVEL is the headphones volume." } else { "Headphones.\nNo headphones output is set: choose one in Preferences > Audio > Input/Output (it needs a device with four or more outputs)." };
    if let Some(v) = knob(ui, pos2(x(490.0), cy), "hpmix", app.px.hp_mix, 0.5, WHITE) {
        app.px.hp_mix = v;
    }
    p.text(pos2(x(513.0), cy), Align2::CENTER_CENTER, "MIX", caps(8.0), DIM);
    if let Some(v) = knob(ui, pos2(x(553.0), cy), "hplevel", app.px.hp_level, 0.5, BLUE) {
        app.px.hp_level = v;
    }
    p.text(pos2(x(583.0), cy), Align2::CENTER_CENTER, "LEVEL", caps(8.0), DIM);
    let _ = ui.interact(Rect::from_min_max(pos2(x(440.0), r.min.y), pos2(x(600.0), r.max.y)), ui.id().with("hpnote"), Sense::hover()).on_hover_text(note);

    // Crossfader assign: the left block puts a deck on side A, the right block on side B;
    // neither = THRU. Decks 3 and 4 do not exist yet.
    let mut changed = false;
    for side in 1..=2u8 {
        let bx = if side == 1 { x(629.0) } else { x(821.0) };
        for d in 0..4usize {
            let br = Rect::from_min_size(pos2(bx + (d % 2) as f32 * 32.0, r.min.y + 8.0 + (d / 2) as f32 * 16.5), vec2(30.0, 14.0));
            let exists = d < app.deck_count() as usize;
            let lit = exists && app.px.mix[d].assign == side;
            let resp = ui.interact(br, ui.id().with(("assign", side, d)), Sense::click());
            p.rect_filled(br, 1.0, if lit { BLUE } else if resp.hovered() && exists { lighten(BTN) } else { hex(0x141414) });
            p.text(br.center(), Align2::CENTER_CENTER, format!("{}", d + 1), caps(8.0), if exists { WHITE } else { OFF });
            if resp.clicked() && exists {
                app.px.mix[d].assign = if lit { 0 } else { side };
                changed = true;
            }
        }
    }
    // Crossfader: 96 x 24 pt; a modifier-click or double click centres it.
    let (fl, fr) = (x(711.0), x(801.0));
    let rr = Rect::from_min_max(pos2(fl - 6.0, cy - 12.0), pos2(fr + 6.0, cy + 12.0));
    let resp = ui.interact(rr, ui.id().with("xfader"), Sense::click_and_drag());
    let mut xf = app.px.crossfader;
    if resp.double_clicked() || (resp.clicked() && ui.input(|i| i.modifiers.any())) {
        xf = 0.5;
        changed = true;
    } else if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.is_pointer_button_down_on() || resp.clicked()) {
        xf = ((pp.x - fl) / (fr - fl)).clamp(0.0, 1.0);
        changed = true;
    }
    p.rect_filled(Rect::from_min_max(pos2(fl, cy - 0.5), pos2(fr, cy + 0.5)), 0.0, hex(0x323232));
    p.rect_filled(Rect::from_min_max(pos2(fl, cy - 10.0), pos2(fl + 1.0, cy + 10.0)), 0.0, hex(0x323232));
    p.rect_filled(Rect::from_min_max(pos2(fr - 1.0, cy - 10.0), pos2(fr, cy + 10.0)), 0.0, hex(0x323232));
    let hx = fl + xf * (fr - fl);
    p.rect_filled(Rect::from_center_size(pos2(hx, cy), vec2(2.0, 22.0)), 0.0, if resp.hovered() || resp.dragged() { WHITE } else { hex(0x8C8C8C) });
    if changed {
        app.px.crossfader = xf;
        app.poll_decks();
    }
}
