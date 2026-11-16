//! The preparation player (top of the window): track header, overview and enlarged
//! waveforms with beat grid and cue markers, transport, hot cues, memory cues, loops,
//! grid editing and the MEMORY / HOT CUE / INFO side panel.

use eframe::egui;
use crate::app::*;
use crate::model::*;
use crate::theme::*;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, Ui};

pub fn button(ui: &mut Ui, rect: Rect, id: &str, label: &str, fill: Color32, text: Color32, size: f32) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    let f = if r.is_pointer_button_down_on() { fill.gamma_multiply(0.7) } else if r.hovered() { lighten(fill) } else { fill };
    ui.painter().rect_filled(rect, 3.0, f);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(size), text);
    r
}

fn lighten(c: Color32) -> Color32 {
    Color32::from_rgb(c.r().saturating_add(20), c.g().saturating_add(20), c.b().saturating_add(20))
}

fn tip(r: Response, app: &App, t: &str) -> Response {
    if app.prefs.tooltips {
        r.on_hover_text(t)
    } else {
        r
    }
}

/// An icon button: a box (optional) with a vector icon drawn by `draw`.
pub fn icon_button(ui: &mut Ui, rect: Rect, id: &str, fill: Option<Color32>, draw: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    if let Some(f) = fill {
        let f = if r.is_pointer_button_down_on() { f.gamma_multiply(0.7) } else if r.hovered() { lighten(f) } else { f };
        ui.painter().rect_filled(rect, 2.0, f);
    } else if r.hovered() {
        ui.painter().rect_filled(rect, 2.0, hex(0x1A1A1A));
    }
    draw(ui.painter(), rect, if r.hovered() { WHITE } else { TEXT });
    r
}

/// Horizontal positions in the player are measured on a 1512 pt wide window, where the
/// waveform area spans x = 81..1300. `sx` maps such an x to the current width.
fn sx(main: Rect, x: f32) -> f32 {
    main.min.x + (x - 81.0) * (main.width() / 1219.0)
}

pub fn player_panel(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    let left = Rect::from_min_size(full.min, vec2(81.0, full.height()));
    let right = Rect::from_min_max(pos2(full.max.x - 202.0, full.min.y), full.max);
    let main = Rect::from_min_max(pos2(left.max.x, full.min.y), pos2(full.max.x - 212.0, full.max.y));
    p.rect_filled(left, 0.0, BG2);

    let pos = app.pos();
    let playing = app.playing();
    let loaded = app.deck.track.is_some();

    // ---------------- left column: jumps, beat jump, CUE, PLAY (centres measured in the original)
    let lx = left.min.x;
    let ly = left.min.y;
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_center_size(pos2(lx + x, ly + y), vec2(w, h));
    if tip(icon_button(ui, at(25.0, 34.0, 25.0, 22.0), "front", Some(BTN), crate::icons::jump_front), app, "Jump to the front").clicked() {
        app.seek(0.0);
        if !app.playing() {
            app.deck.cue_point = 0.0;
        }
    }
    if tip(icon_button(ui, at(54.5, 34.0, 25.0, 22.0), "rear", Some(BTN), crate::icons::jump_rear), app, "Jump to the rear").clicked() {
        let d = app.deck.duration;
        app.seek((d - 0.5).max(0.0));
    }
    let bj = app.prefs.beat_jump;
    if tip(icon_button(ui, at(25.0, 79.5, 25.0, 22.0), "bjl", Some(BTN), crate::icons::chevron_left), app, "Beat Jump backward").clicked() {
        app.beat_jump(-bj);
    }
    if tip(icon_button(ui, at(54.5, 79.5, 25.0, 22.0), "bjr", Some(BTN), crate::icons::chevron_right), app, "Beat Jump forward").clicked() {
        app.beat_jump(bj);
    }
    let bj_rect = at(40.0, 113.5, 54.0, 26.0);
    let bj_label = if bj < 1.0 { format!("1/{}Beat", (1.0 / bj).round()) } else { format!("{}Beats", bj) };
    let r = ui.interact(bj_rect, ui.id().with("bjsize"), Sense::click());
    p.rect_filled(bj_rect, 2.0, if r.hovered() { lighten(BTN) } else { BTN });
    p.text(bj_rect.center() - vec2(0.0, 5.0), Align2::CENTER_CENTER, bj_label, bold(9.0), WHITE);
    crate::icons::chevron_down(&p, Rect::from_center_size(bj_rect.center() + vec2(0.0, 6.0), vec2(12.0, 10.0)), WHITE);
    crate::ui_browser::dropdown(ui, &r, "bjpop", |ui: &mut Ui| {
        for v in [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0] {
            let l = if v < 1.0 { format!("1/{} Beat", (1.0 / v) as i32) } else { format!("{} Beats", v) };
            if ui.selectable_label(app.prefs.beat_jump == v, l).clicked() {
                app.prefs.beat_jump = v;
                app.save_prefs();
            }
        }
    });
    // CUE
    let cue_c = pos2(lx + 40.0, ly + 162.5);
    let cue_rect = Rect::from_center_size(cue_c, vec2(42.0, 42.0));
    let cr = ui.interact(cue_rect, ui.id().with("cuebtn"), Sense::click_and_drag());
    let cue_lit = loaded && (!playing || app.deck.cue_preview);
    // Measured (references 01/02): 40 pt ring, 2 pt stroke; idle #745D31 with grey text; with a
    // track loaded #F4BC53, white text and a #282828 face.
    let face = if cr.hovered() { hex(0x2E2E2E) } else if loaded { hex(0x282828) } else { BG2 };
    p.circle_filled(cue_c, 18.5, face);
    p.circle_stroke(cue_c, 18.5, Stroke::new(2.0, if cue_lit { hex(0xF4BC53) } else { hex(0x745D31) }));
    p.text(cue_c, Align2::CENTER_CENTER, "CUE", bold(10.5), if loaded { WHITE } else { hex(0x323232) });
    if cr.is_pointer_button_down_on() && !app.deck.cue_preview && ui.input(|i| i.pointer.any_pressed()) {
        app.cue_down();
    }
    let cue_key_held = app.prefs.key_for("cue").map(|k| ui.input(|i| i.key_down(k))).unwrap_or(false);
    if !cr.is_pointer_button_down_on() && app.deck.cue_preview && !cue_key_held {
        app.cue_up();
    }
    // PLAY
    let play_c = pos2(lx + 40.0, ly + 213.0);
    let pr = ui.interact(Rect::from_center_size(play_c, vec2(42.0, 42.0)), ui.id().with("playbtn"), Sense::click());
    p.circle_filled(play_c, 18.5, if pr.hovered() { hex(0x2A2A2A) } else { BG2 });
    // Stopped: dim green ring and a grey triangle (references 01/02).
    let ring = if playing { PLAY_GREEN } else { hex(0x38672C) };
    p.circle_stroke(play_c, 18.5, Stroke::new(2.0, ring));
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

    let my = main.min.y;
    // ---------------- enlarged waveform (drawn first; the zoom buttons sit on top of it)
    let det = Rect::from_min_max(pos2(main.min.x, my + 87.0), pos2(main.max.x, my + 204.0));
    draw_detail(app, ui, det, pos);

    // ---------------- zoom column (overlaid on the left of the enlarged waveform), on an opaque
    // black strip so the waveform never shows through the buttons.
    ui.painter().rect_filled(Rect::from_min_max(pos2(main.min.x, my + 98.0), pos2(main.min.x + 25.5, my + 176.0)), 0.0, Color32::BLACK);
    let zc = |y: f32| Rect::from_center_size(pos2(main.min.x + 12.7, my + y), vec2(22.0, 20.0));
    if tip(icon_button(ui, zc(112.0), "zin", None, |p, r, c| crate::icons::magnifier(p, r, c, 1)), app, "Zoom In").clicked() && app.prefs.zoom > 0 {
        app.prefs.zoom -= 1;
    }
    if tip(button(ui, zc(137.0), "zrst", "RST", Color32::TRANSPARENT, TEXT, 9.0), app, "Reset Zoom").clicked() {
        app.prefs.zoom = 2;
    }
    if tip(icon_button(ui, zc(162.0), "zout", None, |p, r, c| crate::icons::magnifier(p, r, c, -1)), app, "Zoom Out").clicked() && app.prefs.zoom + 1 < ZOOMS.len() {
        app.prefs.zoom += 1;
    }

    // ---------------- header row (title, artist, times, key, BPM)
    let hy = my + 10.6;
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    let dur = app.deck.duration;
    let div = |x: f32| p.line_segment([pos2(x, hy - 7.5), pos2(x, hy + 7.5)], Stroke::new(1.0, OFF));
    let x_artist = sx(main, 733.0);
    let x_time = sx(main, 1011.0);
    let x_key = sx(main, 1168.0);
    let x_bpm = sx(main, 1225.0);
    if let Some(t) = &track {
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(sx(main, 179.0), my), pos2(x_artist - 8.0, my + 22.0)));
        clip.text(pos2(sx(main, 179.0), hy), Align2::LEFT_CENTER, t.display_title(), bold(15.0), WHITE);
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(x_artist, my), pos2(x_time - 8.0, my + 22.0)));
        clip.text(pos2(x_artist + 17.0, hy), Align2::LEFT_CENTER, &t.artist, FontId::proportional(12.0), DIM);
    }
    for x in [x_artist, x_time, x_key, x_bpm] {
        div(x);
    }
    let remain = (dur - pos).max(0.0);
    let dim_t = if loaded { DIM } else { OFF };
    let bright_t = if loaded { WHITE } else { OFF };
    let (big, small) = split_time(if loaded { remain } else { 0.0 });
    let tx = x_time + 21.0;
    let (c1, c2) = if app.prefs.time_remain { (bright_t, dim_t) } else { (dim_t, bright_t) };
    let tr = p.text(pos2(tx, hy), Align2::LEFT_CENTER, format!("-{big}"), FontId::proportional(14.5), c1);
    let tr2 = p.text(pos2(tr.max.x, hy + 1.5), Align2::LEFT_CENTER, format!(".{small}"), FontId::proportional(10.5), c1);
    let (big2, small2) = split_time(if loaded { pos } else { 0.0 });
    let er = p.text(pos2(tr2.max.x + 13.0, hy), Align2::LEFT_CENTER, &big2, FontId::proportional(14.5), c2);
    p.text(pos2(er.max.x, hy + 1.5), Align2::LEFT_CENTER, format!(".{small2}"), FontId::proportional(10.5), c2);
    let time_hit = ui.interact(Rect::from_min_max(pos2(x_time, my), pos2(x_key, my + 22.0)), ui.id().with("timesw"), Sense::click());
    if tip(time_hit, app, "Switch Time/Remain").clicked() {
        app.prefs.time_remain = !app.prefs.time_remain;
    }
    if let Some(t) = &track {
        p.text(pos2((x_key + x_bpm) / 2.0, hy), Align2::CENTER_CENTER, t.key_label(app.prefs.key_alnum), FontId::proportional(13.0), WHITE);
    }
    if loaded {
        let b = app.current_bpm();
        p.text(pos2(sx(main, 1263.0), hy), Align2::CENTER_CENTER, if b > 0.0 { format!("{b:.2}") } else { "--.--".into() }, FontId::proportional(13.0), WHITE);
    }

    // ---------------- artwork
    let art = Rect::from_min_max(pos2(sx(main, 94.5), my + 16.6), pos2(sx(main, 94.5) + 49.0, my + 50.8));
    match &track {
        Some(t) => {
            if let Some(tex) = crate::ui_browser::artwork_texture(app, &t.artwork) {
                p.image(tex.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                p.rect_filled(art, 0.0, hex(0x141414));
                draw_note_icon(&p, art);
            }
        }
        None => {
            // Empty deck (reference 01): a dark 47.5 × 46 pt tile with a grey disc and hub.
            let tile = Rect::from_min_size(pos2(sx(main, 95.0), my + 11.0), vec2(47.5, 46.0));
            p.rect_filled(tile, 0.0, hex(0x0F0F0F));
            p.circle_filled(tile.center(), 19.5, hex(0x1E1E1E));
            p.circle_stroke(tile.center(), 19.5, Stroke::new(1.0, hex(0x262626)));
            p.circle_filled(tile.center(), 3.0, hex(0x0F0F0F));
            p.circle_stroke(tile.center(), 3.0, Stroke::new(1.0, hex(0x262626)));
        }
    }

    // ---------------- overview waveform
    let ov = Rect::from_min_max(pos2(sx(main, 159.0), my + 24.0), pos2(sx(main, 1293.0), my + 57.0));
    draw_overview(app, ui, ov, pos);

    // ---------------- control strip
    let strip = Rect::from_min_max(pos2(left.max.x, my + 208.0), pos2(main.max.x, full.max.y));
    control_strip(app, ui, strip, main);

    // ---------------- right sub panel
    side_panel(app, ui, right);

    if playing || app.deck.loading.is_some() {
        ui.ctx().request_repaint();
    }
}

fn split_time(t: f64) -> (String, String) {
    let tenths = ((t * 10.0).floor() as i64).max(0);
    let secs = tenths / 10;
    (format!("{:02}:{:02}", secs / 60, secs % 60), format!("{}", tenths % 10))
}

fn draw_note_icon(p: &egui::Painter, r: Rect) {
    let c = r.center();
    p.circle_filled(c + vec2(-4.0, 8.0), 5.0, OFF);
    p.line_segment([c + vec2(0.5, 8.0), c + vec2(0.5, -12.0)], Stroke::new(2.0, OFF));
    p.line_segment([c + vec2(0.5, -12.0), c + vec2(9.0, -8.0)], Stroke::new(2.0, OFF));
}

/// Colour + height for one waveform column according to the chosen display style.
pub fn wave_bars(mode: u8, v: [u8; 4]) -> Vec<(f32, Color32)> {
    let f = |x: u8| x as f32 / 255.0;
    match mode {
        0 => {
            let white = (f(v[3]) / (f(v[0]) + 0.05)).min(1.0);
            let c = Color32::from_rgb((30.0 + 180.0 * white) as u8, (120.0 + 110.0 * white) as u8, 255);
            vec![(f(v[0]), c)]
        }
        1 => {
            let (r, g, b) = (f(v[1]), f(v[2]), f(v[3]) * 1.3);
            let m = r.max(g).max(b).max(0.01);
            let c = Color32::from_rgb((255.0 * r / m) as u8, (255.0 * g / m) as u8, (255.0 * (b / m).min(1.0)) as u8);
            vec![(f(v[0]), c)]
        }
        // 3Band from display levels (track-list previews): blue body as the envelope, then
        // smaller orange and white layers, matching the original's balance of colours.
        _ => vec![(f(v[0]), WAVE_LOW), (f(v[2]) * 0.6, WAVE_MID), (f(v[3]) * 0.35, WAVE_HIGH)],
    }
}

fn draw_overview(app: &mut App, ui: &mut Ui, r: Rect, pos: f64) {
    let p = ui.painter().clone();
    let dur = app.deck.duration.max(0.001);
    // Measured from the original: baseline 29 pt below the top, bars up to 24 pt tall,
    // a double progress rule at +33/+36 pt and minute ticks under it.
    let base = r.min.y + 29.0;
    let full_h = 24.0;
    if let Some(w) = &app.deck.wave {
        let n = r.width().max(1.0) as usize;
        let cols = w.columns(n);
        // the overview fills its height at the track's loudest column
        let top = cols.iter().map(|c| { let d = Waveform::disp(c); d[0].max(d[1]) }).max().unwrap_or(255).max(1) as f32;
        let norm = (255.0 / top).min(4.0);
        for (i, c) in cols.iter().enumerate() {
            let x = r.min.x + i as f32 + 0.5;
            let played = (i as f64 / n as f64) * dur < pos;
            let mut d = Waveform::disp(c);
            for v in d.iter_mut() {
                *v = (*v as f32 * norm).min(255.0) as u8;
            }
            let layers: Vec<(f32, Color32)> = if app.prefs.wave_color >= 2 {
                // 3Band overview, as the original draws it: low (blue) from the baseline, mid
                // (orange) stacked on it, high (white) on top. Heights per band unit were
                // measured from the original's screen (0.25, 0.115, 0.085 pt) against its own
                // overview values; the values come from the model fitted to its files.
                let [lo, mi, hi] = match &app.deck.rb_wave {
                    // rekordbox's own overview values for this file, when available.
                    Some(rw) if !rw.1.is_empty() => rw.1[(i * rw.1.len() / n.max(1)).min(rw.1.len() - 1)],
                    _ => crate::export::anlz::three_band_overview(c),
                };
                let (b, o, w) = (lo as f32 * 0.25 / full_h, mi as f32 * 0.115 / full_h, hi as f32 * 0.085 / full_h);
                vec![(b + o + w, WAVE_HIGH), (b + o, WAVE_MID), (b, WAVE_LOW)]
            } else {
                wave_bars(app.prefs.wave_color, d)
            };
            for (h, col) in layers {
                let col = if played { col.gamma_multiply(0.45) } else { col };
                p.line_segment([pos2(x, base), pos2(x, base - h.min(1.0) * full_h)], Stroke::new(1.0, col));
            }
        }
    } else if app.deck.track.is_some() {
        p.text(r.center(), Align2::CENTER_CENTER, "Analyzing…", FontId::proportional(11.0), DIM);
    }
    // Vocal positions (rekordbox's vocal detection): a band along the top, brighter where the
    // vocal level is higher.
    if let (true, Some(v)) = (app.prefs.show_vocal, &app.deck.vocal) {
        let (step, levels) = (v.0, &v.1);
        let w = r.width().max(1.0) as usize;
        for i in 0..w {
            let t0 = i as f64 / w as f64 * dur;
            let t1 = (i + 1) as f64 / w as f64 * dur;
            let (a, b) = ((t0 / step) as usize, ((t1 / step) as usize).max((t0 / step) as usize + 1));
            let lvl = levels.get(a.min(levels.len())..b.min(levels.len())).and_then(|s| s.iter().max().copied()).unwrap_or(0);
            if lvl > 0 {
                let x = r.min.x + i as f32 + 0.5;
                let col = hex(0x4FC3D9).gamma_multiply(0.35 + 0.65 * (lvl.min(4) as f32 / 4.0));
                p.line_segment([pos2(x, r.min.y + 1.0), pos2(x, r.min.y + 3.5)], Stroke::new(1.0, col));
            }
        }
    }
    // Phrase bar (as rekordbox shows phrase analysis): one coloured block per phrase.
    if let (Some(st), false) = (&app.deck.phrases, app.deck.beats.is_empty()) {
        let y0 = r.min.y + 38.5;
        let beat_t = |b: u16| app.deck.beats.get((b.max(1) - 1) as usize).map(|x| x.time).unwrap_or(dur);
        for (k, ph) in st.phrases.iter().enumerate() {
            let t0 = beat_t(ph.beat);
            let t1 = st.phrases.get(k + 1).map(|n| beat_t(n.beat)).unwrap_or_else(|| beat_t(st.end_beat));
            let x0 = r.min.x + (t0 / dur) as f32 * r.width();
            let x1 = r.min.x + (t1 / dur) as f32 * r.width();
            let col = match ph.kind {
                crate::phrase::INTRO => hex(0x3E7BD6),
                crate::phrase::UP => hex(0xD6A23E),
                crate::phrase::DOWN => hex(0x5AA05A),
                crate::phrase::CHORUS => hex(0xC8464B),
                crate::phrase::OUTRO => hex(0x8A62C8),
                _ => hex(0x606060),
            };
            p.rect_filled(Rect::from_min_max(pos2(x0 + 0.5, y0), pos2((x1 - 0.5).max(x0 + 1.0), y0 + 3.0)), 0.0, col);
        }
    }
    // progress rule and minute ticks (nothing at all on an empty deck, as in reference 01)
    if app.deck.track.is_some() {
        p.line_segment([pos2(r.min.x, r.min.y + 33.0), pos2(r.max.x, r.min.y + 33.0)], Stroke::new(1.0, hex(0x5A5A5A)));
        p.line_segment([pos2(r.min.x, r.min.y + 36.0), pos2(r.max.x, r.min.y + 36.0)], Stroke::new(1.0, hex(0x3A3A3A)));
        let minute = 60.0 / dur * r.width() as f64;
        let mut k = 1.0;
        while k * minute < r.width() as f64 {
            let x = r.min.x + (k * minute) as f32;
            p.line_segment([pos2(x, r.min.y + 37.0), pos2(x, r.min.y + 39.0)], Stroke::new(1.0, DIM2));
            k += 1.0;
        }
        let x_of = |t: f64| r.min.x + (t / dur) as f32 * r.width();
        // loops
        for c in &app.deck.cues {
            if let Some(e) = c.end.filter(|e| *e > c.start) {
                p.rect_filled(Rect::from_min_max(pos2(x_of(c.start), r.min.y), pos2(x_of(e).max(x_of(c.start) + 1.0), r.max.y)), 0.0, LOOP.gamma_multiply(0.25));
            }
        }
        // memory cues (below) and hot cues (above)
        for c in &app.deck.cues {
            let x = x_of(c.start);
            if c.kind == CueKind::Memory {
                let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
                tri_up(&p, pos2(x, base + 0.5), 5.0, col);
            } else {
                let col = hotcue_color(c.color);
                // 10 pt box starting 4 pt left of the cue line (reference 02).
                let lr = Rect::from_min_size(pos2(x - 4.0, r.min.y), vec2(10.0, 10.0));
                p.rect_filled(lr, 0.0, col);
                p.text(lr.center(), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), FontId::proportional(8.0), WHITE);
                p.line_segment([pos2(x, r.min.y), pos2(x, base)], Stroke::new(1.0, col));
            }
        }
        // cue point
        tri_up(&p, pos2(x_of(app.deck.cue_point), base + 0.5), 6.0, CUE_MARK);
        // playhead
        let px = x_of(pos);
        p.line_segment([pos2(px, r.min.y + 2.0), pos2(px, base)], Stroke::new(1.5, WHITE));
    }
    let resp = ui.interact(r.expand2(vec2(0.0, 4.0)), ui.id().with("overview"), Sense::click_and_drag());
    if (resp.clicked() || resp.dragged()) && app.deck.audio.is_some() {
        if let Some(pp) = resp.interact_pointer_pos() {
            let t = ((pp.x - r.min.x) / r.width()).clamp(0.0, 1.0) as f64 * dur;
            app.seek(t);
            if !app.playing() {
                app.deck.cue_point = app.quantized(t);
                app.seek(app.deck.cue_point);
            }
        }
    }
}

fn tri_up(p: &egui::Painter, tip: Pos2, s: f32, c: Color32) {
    p.add(Shape::convex_polygon(vec![tip, tip + vec2(s, s * 1.2), tip + vec2(-s, s * 1.2)], c, Stroke::NONE));
}
fn tri_down(p: &egui::Painter, tip: Pos2, s: f32, c: Color32) {
    p.add(Shape::convex_polygon(vec![tip, tip + vec2(-s, -s * 1.2), tip + vec2(s, -s * 1.2)], c, Stroke::NONE));
}

fn bars_label(beats: &[Beat], t: f64) -> String {
    if beats.is_empty() {
        return String::new();
    }
    let i = beats.partition_point(|b| b.time <= t + 1e-4);
    if i == 0 {
        return "1.1Bars".into();
    }
    let b = beats[i - 1];
    let first_down = beats.iter().position(|x| x.beat == 1).unwrap_or(0);
    let bars = beats[..i].iter().skip(first_down).filter(|x| x.beat == 1).count().max(1);
    format!("{}.{}Bars", bars, b.beat)
}

fn draw_detail(app: &mut App, ui: &mut Ui, r: Rect, pos: f64) {
    // `r` is the beat/waveform area; the 19 pt row above it holds the bar.beat readout and the
    // hot cue letters (measured on reference 02).
    let p = ui.painter_at(Rect::from_min_max(r.min - vec2(0.0, 20.0), r.max + vec2(0.0, 8.0)));
    let span = ZOOMS[app.prefs.zoom.min(ZOOMS.len() - 1)];
    let px_per_s = r.width() as f64 / span;
    let cx = r.center().x;
    let t_of = |x: f32| pos + (x - cx) as f64 / px_per_s;
    app.deck.detail_map = Some((r, pos, px_per_s));
    let x_of = |t: f64| cx + ((t - pos) * px_per_s) as f32;
    let mid = r.center().y;
    let half = r.height() / 2.0 - 10.0;
    let dur = app.deck.duration;
    // loops
    if let Some((a, b)) = app.loop_active() {
        p.rect_filled(Rect::from_min_max(pos2(x_of(a), r.min.y), pos2(x_of(b), r.max.y)), 0.0, LOOP.gamma_multiply(0.22));
    }
    for c in &app.deck.cues {
        if let Some(e) = c.end.filter(|e| *e > c.start) {
            p.rect_filled(Rect::from_min_max(pos2(x_of(c.start), r.max.y - 6.0), pos2(x_of(e), r.max.y)), 0.0, LOOP.gamma_multiply(0.6));
        }
    }
    // Beat lines run behind the waveform in dim grey (bars get a white line on top, below).
    {
        let (tl, tr) = (t_of(r.min.x), t_of(r.max.x));
        let i0 = app.deck.beats.partition_point(|b| b.time < tl);
        for b in app.deck.beats[i0..].iter().take_while(|b| b.time <= tr) {
            let x = x_of(b.time);
            p.line_segment([pos2(x, r.min.y + 5.0), pos2(x, r.max.y - 5.0)], Stroke::new(1.0, hex(0x4C4C4C)));
        }
    }
    if let Some(w) = &app.deck.wave {
        let fps = WAVE_FPS;
        let n = w.frames.len();
        let mut x = r.min.x;
        while x < r.max.x {
            let t0 = t_of(x);
            let t1 = t_of(x + 1.0);
            if t1 >= 0.0 && t0 <= dur {
                let a = ((t0 * fps).max(0.0) as usize).min(n);
                let b = (((t1 * fps).ceil()) as usize).clamp(a + 1, n.max(a + 1)).min(n);
                if a < n {
                    let mut m = [0u8; 8];
                    for f in &w.frames[a..b.max(a + 1).min(n)] {
                        for k in 0..8 {
                            m[k] = m[k].max(f[k]);
                        }
                    }
                    let bars = if app.prefs.wave_color >= 2 {
                        // 3Band detail: each band centred at its own height, low, then mid, then
                        // high drawn over it (the original's layering), from the fitted model.
                        let [lo, mi, hi] = match &app.deck.rb_wave {
                            // rekordbox's own detail values (150 per second), when available.
                            Some(rw) if !rw.0.is_empty() => {
                                let ia = ((t0.max(0.0) * 150.0) as usize).min(rw.0.len() - 1);
                                let ib = ((t1.max(0.0) * 150.0).ceil() as usize).clamp(ia + 1, rw.0.len());
                                rw.0[ia..ib].iter().fold([0u8; 3], |acc, v| [acc[0].max(v[0]), acc[1].max(v[1]), acc[2].max(v[2])])
                            }
                            _ => crate::export::anlz::three_band_detail(&m),
                        };
                        let s = |v: u8| (v as f32 / 95.0).min(1.0);
                        vec![(s(lo), WAVE_LOW), (s(mi), WAVE_MID), (s(hi), WAVE_HIGH)]
                    } else {
                        wave_bars(app.prefs.wave_color, Waveform::disp(&m))
                    };
                    for (h, col) in bars {
                        let hh = h * half;
                        p.line_segment([pos2(x + 0.5, mid - hh), pos2(x + 0.5, mid + hh)], Stroke::new(1.0, col));
                    }
                }
            }
            x += 1.0;
        }
    } else if app.deck.track.is_some() {
        p.text(r.center() + vec2(60.0, 0.0), Align2::LEFT_CENTER, if app.deck.loading.is_some() { "Loading…" } else { "Analyzing…" }, FontId::proportional(12.0), DIM);
    }
    // beat grid
    let tl = t_of(r.min.x);
    let tr = t_of(r.max.x);
    let i0 = app.deck.beats.partition_point(|b| b.time < tl);
    for b in app.deck.beats[i0..].iter().take_while(|b| b.time <= tr) {
        let x = x_of(b.time);
        if b.beat == 1 {
            // Bar: red 5 × 4 pt triangles at both ends, a white line between them.
            p.add(Shape::convex_polygon(vec![pos2(x - 2.5, r.min.y), pos2(x + 2.5, r.min.y), pos2(x, r.min.y + 4.0)], BAR_RED, Stroke::NONE));
            p.add(Shape::convex_polygon(vec![pos2(x - 2.5, r.max.y), pos2(x, r.max.y - 4.0), pos2(x + 2.5, r.max.y)], BAR_RED, Stroke::NONE));
            p.line_segment([pos2(x, r.min.y + 4.5), pos2(x, r.max.y - 4.5)], Stroke::new(1.0, WHITE));
        } else {
            // Beat: a 3 pt grey cap and a short tick at both ends.
            for (y0, d) in [(r.min.y, 1.0f32), (r.max.y, -1.0)] {
                p.line_segment([pos2(x - 1.5, y0 + d * 0.5), pos2(x + 1.5, y0 + d * 0.5)], Stroke::new(1.0, hex(0x7C7C7C)));
                p.line_segment([pos2(x, y0), pos2(x, y0 + d * 4.0)], Stroke::new(1.0, hex(0x7C7C7C)));
            }
        }
    }
    // cues
    for c in &app.deck.cues {
        let x = x_of(c.start);
        if x < r.min.x - 20.0 || x > r.max.x + 20.0 {
            continue;
        }
        if c.kind == CueKind::Hot {
            let col = hotcue_color(c.color);
            // A 10 pt letter box centred on the cue, in the row above the beats (no comment
            // text: the original shows none there).
            p.line_segment([pos2(x, r.min.y - 2.0), pos2(x, r.max.y)], Stroke::new(1.5, col));
            let lr = Rect::from_center_size(pos2(x, r.min.y - 7.0), vec2(10.0, 10.0));
            p.rect_filled(lr, 0.0, col);
            p.text(lr.center(), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), bold(8.0), WHITE);
        } else {
            let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
            tri_up(&p, pos2(x, r.max.y - 9.0), 6.0, col);
        }
    }
    if app.deck.track.is_some() {
        tri_up(&p, pos2(x_of(app.deck.cue_point), r.max.y - 9.0), 7.0, CUE_MARK);
    }
    if app.deck.track.is_some() {
        // playhead
        p.line_segment([pos2(cx, r.min.y - 4.0), pos2(cx, r.max.y + 4.0)], Stroke::new(1.5, WHITE));
        p.rect_filled(Rect::from_center_size(pos2(cx, r.min.y + 2.0), vec2(6.0, 6.0)), 0.0, BAR_RED);
        let label = bars_label(&app.deck.beats, pos);
        p.text(pos2(cx - 6.0, r.min.y - 9.0), Align2::RIGHT_CENTER, label, FontId::proportional(15.0), BEAT_BLUE);
    }
    // interaction: drag a cue marker to move it, drag elsewhere to scrub, scroll to move,
    // ctrl/cmd + scroll to zoom
    let resp = ui.interact(Rect::from_min_max(r.min - vec2(0.0, 19.0), r.max), ui.id().with("detail"), Sense::click_and_drag());
    let dur = app.deck.duration;
    if app.deck.audio.is_some() {
        if app.deck.cue_drag.is_none() {
            if let Some(hp) = resp.hover_pos() {
                if cue_marker_hit(&app.deck.cues, hp, r, x_of).is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
        }
        if resp.drag_started() {
            // Where the button went down: a drag is only recognised after the pointer has
            // moved a few pixels, and the marker under the press is the one being dragged.
            let pp = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
            match pp.and_then(|pp| cue_marker_hit(&app.deck.cues, pp, r, x_of).map(|id| (id, pp))) {
                Some((id, pp)) => {
                    let start = app.deck.cues.iter().find(|c| c.id == id).map(|c| c.start).unwrap_or(0.0);
                    app.deck.cue_drag = Some((id, app.deck.cues.clone(), t_of(pp.x) - start));
                }
                None => app.deck.drag_origin = Some((pp.map(|p| p.x).unwrap_or(cx), pos)),
            }
        }
        if let Some((id, _, grab)) = app.deck.cue_drag.clone() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.commit_cue_drag(true);
            } else if resp.dragged() {
                if let Some(pp) = resp.interact_pointer_pos() {
                    let t = app.quantized(t_of(pp.x) - grab);
                    if let Some(c) = app.deck.cues.iter_mut().find(|c| c.id == id) {
                        *c = moved_cue(c, t, dur);
                    }
                }
            }
            if resp.drag_stopped() {
                app.commit_cue_drag(false);
            }
        } else if resp.dragged() {
            if let (Some((x0, p0)), Some(pp)) = (app.deck.drag_origin, resp.interact_pointer_pos()) {
                let t = p0 - (pp.x - x0) as f64 / px_per_s;
                app.seek(t);
            }
        }
        if resp.drag_stopped() && app.deck.drag_origin.is_some() {
            app.deck.drag_origin = None;
            if !app.playing() {
                let q = app.quantized(app.pos());
                app.deck.cue_point = q;
                app.seek(q);
            }
        }
        if resp.hovered() {
            let (scroll, zoom_mod) = ui.input(|i| (i.raw_scroll_delta, i.modifiers.command || i.modifiers.ctrl));
            if zoom_mod && scroll.y.abs() > 0.5 {
                if scroll.y > 0.0 && app.prefs.zoom > 0 {
                    app.prefs.zoom -= 1;
                } else if scroll.y < 0.0 && app.prefs.zoom + 1 < ZOOMS.len() {
                    app.prefs.zoom += 1;
                }
            } else if scroll.x.abs() > 0.5 || scroll.y.abs() > 0.5 {
                let d = if scroll.x.abs() > scroll.y.abs() { scroll.x } else { scroll.y };
                let t = pos - d as f64 / px_per_s;
                app.seek(t);
            }
        }
    }
}

/// The cue whose marker is under `p` on the zoomed waveform `r`: a hot cue by its letter box
/// in the row above the beats, a memory cue by its triangle along the bottom (the waveform body scrubs).
pub fn cue_marker_hit(cues: &[Cue], p: Pos2, r: Rect, x_of: impl Fn(f64) -> f32) -> Option<i64> {
    let mut best: Option<(f32, i64)> = None;
    for c in cues {
        let x = x_of(c.start);
        let (hit, d) = match c.kind {
            CueKind::Hot => (p.y >= r.min.y - 13.0 && p.y <= r.min.y && (p.x - x).abs() <= 6.0, (p.x - x).abs()),
            _ => (p.y >= r.max.y - 18.0 && p.y <= r.max.y && (p.x - x).abs() <= 7.0, (p.x - x).abs()),
        };
        if hit && best.map(|b| d < b.0).unwrap_or(true) {
            best = Some((d, c.id));
        }
    }
    best.map(|b| b.1)
}

/// `c` moved to start at `t`: a loop keeps its length, and the cue stays inside the track.
pub fn moved_cue(c: &Cue, t: f64, duration: f64) -> Cue {
    let len = c.end.map(|e| e - c.start).filter(|l| *l > 0.0);
    let max = if duration > 0.0 { (duration - len.unwrap_or(0.0)).max(0.0) } else { f64::MAX };
    let start = t.clamp(0.0, max);
    Cue { start, end: match len { Some(l) => Some(start + l), None => c.end.map(|_| start) }, ..c.clone() }
}

fn control_strip(app: &mut App, ui: &mut Ui, s: Rect, main: Rect) {
    let p = ui.painter().clone();
    // The strip behind the pads is dark grey in the original, the pads darker still.
    p.rect_filled(Rect::from_min_max(pos2(s.min.x + 81.5, s.min.y), pos2(main.max.x, s.max.y - 2.0)), 0.0, BG2);
    // Tabs CUE/LOOP and GRID (stacked, as measured: x 83..162.5).
    let tab1 = Rect::from_min_max(pos2(s.min.x + 2.0, s.min.y), pos2(s.min.x + 81.5, s.min.y + 23.0));
    let tab2 = Rect::from_min_max(pos2(s.min.x + 2.0, s.min.y + 23.0), pos2(s.min.x + 81.5, s.max.y));
    if tab_button(ui, tab1, "tab_cue", "CUE/LOOP", app.deck.ctrl_tab == 0).clicked() {
        app.deck.ctrl_tab = 0;
    }
    if tab_button(ui, tab2, "tab_grid", "GRID", app.deck.ctrl_tab == 1).clicked() {
        app.deck.ctrl_tab = 1;
    }
    let y = s.min.y + 22.6;
    let x0 = sx(main, 407.9);
    let at = |x: f32, w: f32| Rect::from_center_size(pos2(sx(main, x), y), vec2(w, 23.0));
    if app.deck.ctrl_tab == 0 {
        // hot cue pads: 28 pt dark squares on a 32.9 pt pitch starting at x = 420.4;
        // a set cue shows as a 15 pt square of its colour behind the letter.
        let mut x = x0;
        for slot in 0..8 {
            let r = Rect::from_center_size(pos2(sx(main, 420.4 + slot as f32 * 32.9), y), vec2(28.0, 28.0));
            let _ = x;
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
            p.text(r.center(), Align2::CENTER_CENTER, ((b'A' + slot as u8) as char).to_string(), FontId::proportional(15.0), tc);
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
            if let Some(c) = cue {
                let resp = tip(resp, app, "Click: call hot cue · Shift+click: delete · Right-click: options");
                resp.context_menu(|ui| hot_cue_menu(app, ui, &c));
            }
            x += 32.9;
        }
        // memory: MEMORY x 695.6..756, then ◀ ▶ ✕ at 769.6 / 796 / 821
        if tip(bold_button(ui, at(725.8, 60.5), "mem", "MEMORY"), app, "Set Memory Cue(Loop)").clicked() {
            app.memory_set();
        }
        if tip(icon_button(ui, at(769.6, 22.0), "memprev", Some(BG2), crate::icons::tri_left), app, "Call Previous Memory Cue").clicked() {
            app.memory_jump(-1);
        }
        if tip(icon_button(ui, at(796.0, 22.0), "memnext", Some(BG2), crate::icons::tri_right), app, "Call Next Memory Cue").clicked() {
            app.memory_jump(1);
        }
        if tip(icon_button(ui, at(821.0, 22.0), "memdel", Some(BG2), |p, r, c| crate::icons::cross(p, r, c, 1.6)), app, "Delete Memory Cue(Loop)").clicked() {
            app.memory_delete_current();
        }
        // loop group: AU 861..897.4, MA 900.4..936, ‹ 951.8, field 975..1032.7, › 1045.6
        let au = app.prefs.loop_auto;
        if tip(seg_button(ui, at(879.2, 36.4), "au", "AU", au), app, "Auto Beat Loop").clicked() {
            app.prefs.loop_auto = true;
        }
        if tip(seg_button(ui, at(918.2, 35.6), "ma", "MA", !au), app, "Manual Loop").clicked() {
            app.prefs.loop_auto = false;
        }
        if au {
            if tip(icon_button(ui, at(951.8, 18.0), "lhalf", None, crate::icons::chevron_left), app, "Halve the loop length").clicked() {
                app.resize_loop(0.5);
            }
            let lb = app.prefs.loop_beats;
            let label = if lb < 1.0 { format!("1/{}", (1.0 / lb).round()) } else { format!("{}", lb) };
            let active = app.loop_active().is_some();
            if tip(button(ui, at(1003.9, 57.7), "lbeat", &label, if active { LOOP.gamma_multiply(0.7) } else { BTN }, WHITE, 11.5), app, "Auto Beat Loop on/off").clicked() {
                if active {
                    app.loop_exit();
                } else {
                    app.auto_loop(lb);
                }
            }
            if tip(icon_button(ui, at(1045.6, 18.0), "ldouble", None, crate::icons::chevron_right), app, "Double the loop length").clicked() {
                app.resize_loop(2.0);
            }
        } else {
            let mut xx = 951.0;
            for (lbl, id) in [("IN", "lin"), ("OUT", "lout"), ("RELOOP", "lre"), ("EXIT", "lex")] {
                let w = if lbl.len() > 3 { 50.0 } else { 34.0 };
                let on = match lbl {
                    "IN" => app.deck.loop_in.is_some(),
                    "OUT" => app.loop_active().is_some(),
                    _ => false,
                };
                if button(ui, at(xx + w / 2.0, w), id, lbl, if on { LOOP.gamma_multiply(0.7) } else { BTN }, WHITE, 9.5).clicked() {
                    match lbl {
                        "IN" => app.loop_in(),
                        "OUT" => app.loop_out(),
                        "RELOOP" => app.reloop(),
                        _ => app.loop_exit(),
                    }
                }
                xx += w + 3.0;
            }
        }
        // Q 1206.6..1238.4, menu 1259.5
        let q = app.prefs.quantize;
        if tip(seg_button(ui, at(1165.0, 46.0), "stems_btn", "STEMS", false), app, "Show the stem (part) controls").clicked() {
            app.deck.ctrl_tab = 2;
        }
        if tip(seg_button(ui, at(1222.5, 31.8), "quant", "Q", q), app, "Quantize ON/OFF").clicked() {
            app.prefs.quantize = !q;
            app.save_prefs();
        }
        let menu_r = tip(icon_button(ui, at(1259.5, 22.0), "stripmenu", None, crate::icons::hamburger), app, "Show Menu items");
        crate::ui_browser::dropdown(ui, &menu_r, "strippop", |ui: &mut Ui| {
            if ui.button("Clear all hot cues").clicked() {
                let ids: Vec<i64> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Hot).map(|c| c.id).collect();
                for i in ids {
                    app.delete_cue(i);
                }
            }
            if ui.button("Clear all memory cues").clicked() {
                let ids: Vec<i64> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).map(|c| c.id).collect();
                for i in ids {
                    app.delete_cue(i);
                }
            }
            if ui.button("Memory cues from hot cues").clicked() {
                let hot: Vec<Cue> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Hot).cloned().collect();
                for h in hot {
                    if !app.deck.cues.iter().any(|c| c.kind == CueKind::Memory && (c.start - h.start).abs() < 0.001) {
                        let mut c = Cue { id: 0, kind: CueKind::Memory, slot: -1, color: 0, ..h };
                        app.snapshot_cues();
                        if let Ok(id) = app.db.save_cue(&c) {
                            c.id = id;
                            app.deck.cues.push(c);
                        }
                    }
                }
                if let Some(t) = app.deck.track {
                    app.refresh_cue_counts(t);
                }
            }
        });
    } else if app.deck.ctrl_tab == 2 {
        stems_strip(app, ui, s, main);
    } else {
        grid_strip(app, ui, s, x0);
    }
}

/// Stem controls: VOCAL / INST / BASS / DRUMS (mute a part by switching it off), the model in
/// use and separation progress.
fn stems_strip(app: &mut App, ui: &mut Ui, s: Rect, main: Rect) {
    let p = ui.painter().clone();
    let y = s.min.y + 22.6;
    let at = |x: f32, w: f32| Rect::from_center_size(pos2(sx(main, x), y), vec2(w, 26.0));
    let have = app.player.state.lock().stems.is_some();
    let on = app.player.state.lock().stem_on;
    let loaded = app.deck.track.is_some();
    // (label, source indices, colour). Three parts put bass with the instruments.
    let parts: Vec<(&str, Vec<usize>, Color32)> = if app.prefs.stem_parts == 3 {
        vec![("VOCAL", vec![3], hex(0x3CC85A)), ("INST", vec![2, 1], hex(0xA05AE6)), ("DRUMS", vec![0], hex(0x2E7DF0))]
    } else {
        vec![("VOCAL", vec![3], hex(0x3CC85A)), ("INST", vec![2], hex(0xA05AE6)), ("BASS", vec![1], hex(0xF08C28)), ("DRUMS", vec![0], hex(0x2E7DF0))]
    };
    for (i, (label, srcs, col)) in parts.iter().enumerate() {
        let r = at(452.0 + i as f32 * 78.0, 70.0);
        let resp = ui.interact(r, ui.id().with(("stem", i)), Sense::click());
        let lit = have && srcs.iter().all(|s| on[*s]);
        p.rect_filled(r, 3.0, if lit { col.gamma_multiply(0.85) } else if resp.hovered() { hex(0x2A2A2A) } else { hex(0x161616) });
        p.rect_stroke(r, 3.0, Stroke::new(1.0, if have { *col } else { hex(0x3C3C3C) }), egui::StrokeKind::Inside);
        p.text(r.center(), Align2::CENTER_CENTER, *label, bold(10.5), if have { WHITE } else { DIM });
        let resp = resp.on_hover_text(if have { "Switch this part on or off" } else { "Separate this track into parts first" });
        if resp.clicked() && loaded {
            // Every source of the part goes to the same new state.
            let target = !srcs.iter().all(|s| on[*s]);
            for s in srcs {
                if on[*s] != target {
                    app.toggle_stem(*s);
                }
            }
        }
    }
    // Status and actions.
    let model = app.stem.model.as_deref().and_then(crate::stems::spec).map(|m| m.name).unwrap_or("checking this computer…");
    let status = if let Some((d, t)) = app.stem.downloading {
        if t > 0 { format!("Downloading {model}: {:.0}%", d as f64 / t as f64 * 100.0) } else { format!("Downloading {model}…") }
    } else if let Some((_, pr)) = app.stem.separating {
        format!("Separating with {model}: {:.0}%", pr * 100.0)
    } else if app.stem.loading.is_some() {
        "Loading stems…".to_string()
    } else if let Some(e) = &app.stem.error {
        e.clone()
    } else if have {
        format!("Stems ready ({model}). Click a part to mute it.")
    } else if !loaded {
        "Load a track to separate it into parts.".to_string()
    } else {
        format!("Not separated yet. Model: {model}.")
    };
    p.text(pos2(sx(main, 770.0), y), Align2::LEFT_CENTER, status, FontId::proportional(11.5), if app.stem.error.is_some() { CAUTION_TX } else { DIM2 });
    // Where the model comes from and its licence, on hover over the status line.
    if let Some(m) = app.stem.model.as_deref().and_then(crate::stems::spec) {
        let sr = Rect::from_min_size(pos2(sx(main, 770.0), y - 9.0), vec2(360.0, 18.0));
        ui.interact(sr, ui.id().with("stem_model_info"), Sense::hover()).on_hover_text(format!("{} — {}. Licence: {}. Checked against its published checksum after download.", m.name, m.source, m.licence));
    }
    let busy = app.stem.separating.is_some() || app.stem.downloading.is_some();
    if busy {
        if tip(bold_button(ui, at(1165.0, 54.0), "stem_stop", "STOP"), app, "Stop").clicked() {
            app.stop_separation();
        }
    } else if !have && loaded {
        if tip(bold_button(ui, at(1150.0, 84.0), "stem_go", "SEPARATE"), app, "Split this track into vocal, instrument, bass and drum parts").clicked() {
            app.separate_loaded();
        }
    }
    if tip(seg_button(ui, at(1222.5, 40.0), "stems_back", "BACK", false), app, "Back to the cue and loop controls").clicked() {
        app.deck.ctrl_tab = 0;
    }
}

/// MEMORY-style raised button with a bold caps label.
fn bold_button(ui: &mut Ui, rect: Rect, id: &str, label: &str) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(rect, 2.0, if r.is_pointer_button_down_on() { BG2 } else if r.hovered() { lighten(BTN) } else { BTN });
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, bold(9.0), WHITE);
    r
}

/// Stacked CUE/LOOP and GRID tabs: active #1E1E1E, inactive black, bold caps.
fn tab_button(ui: &mut Ui, rect: Rect, id: &str, label: &str, active: bool) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(rect, 0.0, if active { BG2 } else { BG });
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, bold(9.5), WHITE);
    r
}

/// Toggle buttons (AU / MA / Q): blue when on, #1E1E1E with a hairline when off.
fn seg_button(ui: &mut Ui, rect: Rect, id: &str, label: &str, on: bool) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    let fill = if on { BLUE } else if r.hovered() { hex(0x2A2A2A) } else { BG2 };
    ui.painter().rect_filled(rect, 2.0, fill);
    if !on {
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, hex(0x3C3C3C)), egui::StrokeKind::Inside);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, bold(9.0), WHITE);
    r
}

fn hot_cue_menu(app: &mut App, ui: &mut Ui, c: &Cue) {
    ui.label(format!("Hot Cue {}", (b'A' + c.slot as u8) as char));
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(180.0);
        for idx in HOTCUE_PICKER {
            let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            ui.painter().rect_filled(r, 2.0, hotcue_color(idx));
            if c.color == idx {
                ui.painter().rect_stroke(r, 2.0, Stroke::new(2.0, WHITE), egui::StrokeKind::Inside);
            }
            if resp.clicked() {
                let mut n = c.clone();
                n.color = idx;
                app.update_cue(n);
                ui.close_menu();
            }
        }
    });
    if ui.button("Add comments").clicked() {
        app.deck.panel_tab = 1;
        app.deck.editing_comment = Some(c.id);
        ui.close_menu();
    }
    if ui.button("Delete").clicked() {
        app.delete_cue(c.id);
        ui.close_menu();
    }
}

fn grid_strip(app: &mut App, ui: &mut Ui, s: Rect, x0: f32) {
    let y = s.center().y;
    let mut x = x0;
    let mut b = |ui: &mut Ui, app: &App, id: &str, label: &str, w: f32, tipt: &str, on: bool| -> bool {
        let r = Rect::from_min_size(pos2(x, y - 12.0), vec2(w, 24.0));
        x += w + 4.0;
        tip(button(ui, r, id, label, if on { BLUE } else { BTN }, WHITE, 10.5), app, tipt).clicked()
    };
    let locked = app.deck.track.and_then(|t| app.track(t)).map(|t| t.grid_locked).unwrap_or(false);
    let fine = ui.input(|i| i.modifiers.shift);
    let step = if fine { 0.001 } else { 0.005 };
    if b(ui, app, "gl", "◀◀", 34.0, "Shift the whole Beatgrid left (Shift: fine)", false) {
        app.grid_shift(-step);
    }
    if b(ui, app, "gr", "▶▶", 34.0, "Shift the whole Beatgrid right (Shift: fine)", false) {
        app.grid_shift(step);
    }
    if b(ui, app, "gw", "WIDE", 44.0, "Expand beat intervals", false) {
        app.grid_scale_bpm(|v| ((v - 0.01) * 100.0).round() / 100.0);
    }
    if b(ui, app, "gn", "NARROW", 54.0, "Shrink beat intervals", false) {
        app.grid_scale_bpm(|v| ((v + 0.01) * 100.0).round() / 100.0);
    }
    if b(ui, app, "g2", "x2", 32.0, "Double the BPM value", false) {
        app.grid_scale_bpm(|v| v * 2.0);
    }
    if b(ui, app, "gh", "/2", 32.0, "Halve the BPM value", false) {
        app.grid_scale_bpm(|v| v / 2.0);
    }
    if b(ui, app, "gdown", "1.1", 36.0, "Set the first beat of a bar to the current position.", false) {
        app.grid_set_downbeat();
    }
    if b(ui, app, "gtap", "TAP", 40.0, "Set BPM with the tapping interval", false) {
        app.grid_tap();
    }
    // BPM entry
    let r = Rect::from_min_size(pos2(x, y - 12.0), vec2(64.0, 24.0));
    x += 70.0;
    let mut txt = app.deck.bpm_text.clone();
    let resp = ui.put(r, egui::TextEdit::singleline(&mut txt).horizontal_align(egui::Align::Center).hint_text("BPM"));
    app.deck.bpm_text = txt;
    let resp = tip(resp, app, "Enter a BPM value");
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Ok(v) = app.deck.bpm_text.trim().parse::<f64>() {
            app.grid_set_bpm(v);
        }
    }
    x += 6.0;
    let mut b = |ui: &mut Ui, app: &App, id: &str, label: &str, w: f32, tipt: &str, on: bool| -> bool {
        let r = Rect::from_min_size(pos2(x, y - 12.0), vec2(w, 24.0));
        x += w + 4.0;
        tip(button(ui, r, id, label, if on { BLUE } else { BTN }, WHITE, 10.5), app, tipt).clicked()
    };
    if b(ui, app, "gundo", "UNDO", 44.0, "Undo", false) {
        app.grid_undo();
    }
    if b(ui, app, "gredo", "REDO", 44.0, "Redo", false) {
        app.grid_redo();
    }
    let fc = app.prefs.grid_from_current;
    if b(ui, app, "gwhole", "WHOLE", 50.0, "Make an adjustment on the whole track", !fc) {
        app.prefs.grid_from_current = false;
    }
    if b(ui, app, "gcur", "FROM HERE", 70.0, "Make an adjustment from the current position", fc) {
        app.prefs.grid_from_current = true;
    }
    let met = app.deck.metronome;
    if b(ui, app, "gmet", "METRO", 54.0, "Metronome On/Off", met) {
        app.deck.metronome = !met;
        app.update_metronome();
    }
    if b(ui, app, "glock", if locked { "LOCKED" } else { "LOCK" }, 58.0, "Analysis Lock: keep this grid when re-analysing", locked) {
        if let Some(id) = app.deck.track {
            if let Some(t) = app.track_mut(id) {
                t.grid_locked = !t.grid_locked;
            }
            app.save_track(id);
        }
    }
}

fn side_panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    // Measured: list x 1312..1504, y 5..222 below the player top; tabs y 229..247, 60.5 wide.
    // Reference 01/03: a #1E1E1E panel (x 1303..1509, 259 pt tall) holding the list (x 1311..1501,
    // from 5 pt) as separate rounded rows: MEMORY 10 × 21 pt on a 22 pt pitch, HOT CUE 8 × 26 pt
    // on a 27 pt pitch.
    let tabs_y = r.min.y + 229.0;
    p.rect_filled(Rect::from_min_max(pos2(r.min.x - 7.0, r.min.y), pos2(r.max.x - 3.0, r.min.y + 259.0)), 0.0, hex(0x1E1E1E));
    let list = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 5.0), pos2(r.max.x - 11.0, r.min.y + 224.0));
    for (i, name) in ["MEMORY", "HOT CUE", "INFO"].iter().enumerate() {
        let tr = Rect::from_min_size(pos2(r.min.x + 1.0 + i as f32 * 64.0, tabs_y), vec2(60.5, 18.0));
        let tipt = ["Show MEMORY Panel", "Show HOT CUE Panel", "Show INFO Panel"][i];
        if tip(seg_button(ui, tr, &format!("ptab{i}"), name, app.deck.panel_tab == i as u8), app, tipt).clicked() {
            app.deck.panel_tab = i as u8;
        }
    }
    let row_h = 22.0;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    match app.deck.panel_tab {
        0 => {
            let mems = app.memory_cues();
            for i in 0..10 {
                let rr = Rect::from_min_size(pos2(list.min.x, list.min.y + i as f32 * row_h), vec2(list.width(), row_h - 1.0));
                p.rect_filled(rr, 2.0, hex(0x0F0F0F));
                let Some(c) = mems.get(i).cloned() else {
                    close_cross(&p, pos2(rr.max.x - 10.25, rr.center().y), 7.0, hex(0x454545));
                    continue;
                };
                let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
                tri_up(&p, pos2(rr.min.x + 9.0, rr.center().y - 4.0), 4.0, col);
                p.text(pos2(rr.min.x + 18.0, rr.center().y), Align2::LEFT_CENTER, cue_time(c.start), FontId::proportional(10.5), WHITE);
                if c.is_loop() {
                    p.text(pos2(rr.min.x + 82.0, rr.center().y), Align2::LEFT_CENTER, "∞", FontId::proportional(11.0), if c.active_loop { LOOP } else { DIM });
                }
                let del = Rect::from_center_size(pos2(rr.max.x - 10.0, rr.center().y), vec2(16.0, row_h));
                if app.deck.editing_comment == Some(c.id) {
                    let mut txt = c.comment.clone();
                    let er = Rect::from_min_max(pos2(rr.min.x + 96.0, rr.min.y + 1.0), pos2(rr.max.x - 20.0, rr.max.y - 1.0));
                    let resp = child.put(er, egui::TextEdit::singleline(&mut txt).font(FontId::proportional(10.5)));
                    resp.request_focus();
                    if txt != c.comment {
                        let mut n = c.clone();
                        n.comment = txt;
                        app.update_cue(n);
                    }
                    if resp.lost_focus() {
                        app.deck.editing_comment = None;
                    }
                } else {
                    p.text(pos2(rr.min.x + 96.0, rr.center().y), Align2::LEFT_CENTER, &c.comment, FontId::proportional(10.5), DIM2);
                }
                close_cross(&p, pos2(rr.max.x - 10.25, rr.center().y), 7.0, WHITE);
                let dr = child.interact(del, child.id().with(("memdel", c.id)), Sense::click());
                if dr.clicked() {
                    app.delete_cue(c.id);
                    continue;
                }
                let resp = child.interact(Rect::from_min_max(rr.min, pos2(del.min.x, rr.max.y)), child.id().with(("mem", c.id)), Sense::click());
                if resp.clicked() {
                    app.call_memory(&c);
                }
                if resp.double_clicked() {
                    app.deck.editing_comment = Some(c.id);
                }
                resp.context_menu(|ui| {
                    if ui.button("Call Memory Cue(Loop)").clicked() {
                        app.call_memory(&c);
                        ui.close_menu();
                    }
                    if ui.button("Add comments to the memory cue (loop)").clicked() {
                        app.deck.editing_comment = Some(c.id);
                        ui.close_menu();
                    }
                    if c.is_loop() && ui.checkbox(&mut c.active_loop.clone(), "Set Active Loop").clicked() {
                        let mut n = c.clone();
                        n.active_loop = !n.active_loop;
                        app.update_cue(n);
                        ui.close_menu();
                    }
                    ui.menu_button("Color", |ui| {
                        for i in 0..9u8 {
                            if ui.selectable_label(c.color == i, egui::RichText::new(format!("■ {}", COLOR_NAMES[i as usize])).color(if i == 0 { DIM } else { label_color(i) })).clicked() {
                                let mut n = c.clone();
                                n.color = i;
                                app.update_cue(n);
                                ui.close_menu();
                            }
                        }
                    });
                    if ui.button("Delete Memory Cue(Loop)").clicked() {
                        app.delete_cue(c.id);
                        ui.close_menu();
                    }
                });
            }
        }
        1 => {
            for slot in 0..8 {
                let rr = Rect::from_min_size(pos2(list.min.x, list.min.y + slot as f32 * 27.0), vec2(list.width(), 26.0));
                p.rect_filled(rr, 2.0, hex(0x0F0F0F));
                let c = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).cloned();
                let chip = Rect::from_center_size(pos2(rr.min.x + 15.0, rr.center().y), vec2(11.0, 11.0));
                let letter = ((b'A' + slot as u8) as char).to_string();
                match &c {
                    Some(c) => {
                        p.rect_filled(chip, 0.0, hotcue_color(c.color));
                        p.text(chip.center(), Align2::CENTER_CENTER, &letter, FontId::proportional(10.0), WHITE);
                        p.text(pos2(rr.min.x + 31.5, rr.center().y), Align2::LEFT_CENTER, fmt_time(c.start, false), FontId::proportional(11.0), WHITE);
                        let del = Rect::from_center_size(pos2(rr.max.x - 17.0, rr.center().y), vec2(18.0, rr.height()));
                        if app.deck.editing_comment == Some(c.id) {
                            let mut txt = c.comment.clone();
                            let er = Rect::from_min_max(pos2(rr.min.x + 66.5, rr.min.y + 3.0), pos2(rr.max.x - 26.0, rr.max.y - 3.0));
                            let resp = child.put(er, egui::TextEdit::singleline(&mut txt).font(FontId::proportional(10.5)));
                            resp.request_focus();
                            if txt != c.comment {
                                let mut n = c.clone();
                                n.comment = txt;
                                app.update_cue(n);
                            }
                            if resp.lost_focus() {
                                app.deck.editing_comment = None;
                            }
                        } else {
                            p.text(pos2(rr.min.x + 66.5, rr.center().y), Align2::LEFT_CENTER, &c.comment, FontId::proportional(11.0), WHITE);
                        }
                        close_cross(&p, pos2(rr.max.x - 17.0, rr.center().y), 8.0, WHITE);
                        if child.interact(del, child.id().with(("hcdel", slot)), Sense::click()).clicked() {
                            app.delete_cue(c.id);
                            continue;
                        }
                        let resp = child.interact(Rect::from_min_max(rr.min, pos2(del.min.x, rr.max.y)), child.id().with(("hc", slot)), Sense::click());
                        if resp.clicked() {
                            app.seek(c.start);
                            if !app.playing() {
                                app.deck.cue_point = c.start;
                            }
                        }
                        if resp.double_clicked() {
                            app.deck.editing_comment = Some(c.id);
                        }
                        let cc = c.clone();
                        resp.context_menu(|ui| hot_cue_menu(app, ui, &cc));
                    }
                    None => {
                        // An empty slot shows only its letter, in grey.
                        p.text(chip.center(), Align2::CENTER_CENTER, &letter, FontId::proportional(11.0), hex(0x5A5A5A));
                    }
                }
            }
        }
        _ => info_panel(app, &mut child, list),
    }
}

fn info_panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let Some(id) = app.deck.track else {
        return;
    };
    let Some(t) = app.track(id).cloned() else { return };
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        for s in 1..=5u8 {
            let (rr, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            star(ui.painter(), rr.center(), 7.0, if t.rating >= s { WHITE } else { DIM }, t.rating >= s);
            if resp.clicked() {
                let nr = if t.rating == s { 0 } else { s };
                if let Some(tm) = app.track_mut(id) {
                    tm.rating = nr;
                }
                app.save_track(id);
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("💬").color(DIM));
        let mut c = t.comment.clone();
        let resp = ui.add_sized(vec2(r.width() - 44.0, 20.0), egui::TextEdit::singleline(&mut c).hint_text("Comments"));
        if resp.changed() {
            if let Some(tm) = app.track_mut(id) {
                tm.comment = c;
            }
            app.save_track(id);
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        for i in 0..9u8 {
            let (rr, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
            let col = if i == 0 { BG2 } else { label_color(i) };
            ui.painter().rect_filled(rr, 2.0, col);
            if t.color == i {
                ui.painter().rect_stroke(rr, 2.0, Stroke::new(1.5, WHITE), egui::StrokeKind::Inside);
            }
            if resp.on_hover_text(COLOR_NAMES[i as usize]).clicked() {
                if let Some(tm) = app.track_mut(id) {
                    tm.color = i;
                }
                app.save_track(id);
            }
        }
    });
    ui.add_space(6.0);
    let tags = app.tag_text(id);
    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(if tags.is_empty() { "No My Tag".to_string() } else { tags }).color(DIM2).size(10.5));
    });
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(format!("{} · {} kbps · {} Hz", t.file_type.to_uppercase(), t.bitrate, t.sample_rate)).color(DIM).size(10.0));
    });
}

pub fn star(p: &egui::Painter, c: Pos2, r: f32, col: Color32, filled: bool) {
    let mut pts = vec![];
    for i in 0..10 {
        let a = std::f32::consts::PI / 5.0 * i as f32 - std::f32::consts::FRAC_PI_2;
        let rr = if i % 2 == 0 { r } else { r * 0.45 };
        pts.push(c + vec2(a.cos() * rr, a.sin() * rr));
    }
    if filled {
        // convex fan from centre (star isn't convex)
        for i in 0..10 {
            p.add(Shape::convex_polygon(vec![c, pts[i], pts[(i + 1) % 10]], col, Stroke::NONE));
        }
    } else {
        let mut closed = pts.clone();
        closed.push(pts[0]);
        p.add(Shape::line(closed, Stroke::new(1.0, col)));
    }
}

fn cue_time(t: f64) -> String {
    let ms = (t * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:03}", ms / 60000, (ms / 1000) % 60, ms % 1000)
}

#[cfg(test)]
mod marker_drag_tests {
    use super::*;

    fn cue(id: i64, kind: CueKind, start: f64, end: Option<f64>) -> Cue {
        Cue { id, track_id: 1, kind, slot: if kind == CueKind::Hot { 0 } else { -1 }, start, end, color: 0, comment: String::new(), active_loop: false }
    }

    #[test]
    fn markers_are_grabbed_by_their_handles_only() {
        let r = Rect::from_min_max(pos2(0.0, 100.0), pos2(1000.0, 200.0));
        // 100 px per second, 0 s at x = 0.
        let x_of = |t: f64| (t * 100.0) as f32;
        let cues = vec![cue(1, CueKind::Hot, 2.0, None), cue(2, CueKind::Memory, 5.0, None), cue(3, CueKind::Hot, 2.1, None)];
        // Hot cue letter box (the row above the beats): the nearer box wins.
        assert_eq!(cue_marker_hit(&cues, pos2(203.0, 93.0), r, x_of), Some(1));
        assert_eq!(cue_marker_hit(&cues, pos2(209.0, 93.0), r, x_of), Some(3));
        // Memory cue triangle (bottom strip).
        assert_eq!(cue_marker_hit(&cues, pos2(503.0, 195.0), r, x_of), Some(2));
        // The waveform body scrubs, it never grabs a marker.
        assert_eq!(cue_marker_hit(&cues, pos2(205.0, 150.0), r, x_of), None);
        assert_eq!(cue_marker_hit(&cues, pos2(503.0, 150.0), r, x_of), None);
        assert_eq!(cue_marker_hit(&cues, pos2(800.0, 93.0), r, x_of), None);
        assert_eq!(cue_marker_hit(&cues, pos2(203.0, 105.0), r, x_of), None);
    }

    #[test]
    fn moved_cues_keep_loop_length_and_stay_in_the_track() {
        let l = cue(1, CueKind::Hot, 10.0, Some(14.0));
        let m = moved_cue(&l, 20.0, 100.0);
        assert_eq!((m.start, m.end), (20.0, Some(24.0)));
        // A loop can't run past the end, and nothing goes before 0.
        assert_eq!(moved_cue(&l, 99.0, 100.0).start, 96.0);
        assert_eq!(moved_cue(&l, -3.0, 100.0).start, 0.0);
        let p = cue(2, CueKind::Memory, 5.0, None);
        assert_eq!(moved_cue(&p, 7.5, 100.0).end, None);
        let z = cue(3, CueKind::Hot, 5.0, Some(5.0));
        assert_eq!(moved_cue(&z, 8.0, 100.0).end, Some(8.0));
    }
}

/// The list rows' close button: a cross of `size` pt with a 1.5 pt stroke.
fn close_cross(p: &egui::Painter, c: Pos2, size: f32, col: Color32) {
    let h = size / 2.0;
    p.line_segment([c + vec2(-h, -h), c + vec2(h, h)], Stroke::new(1.5, col));
    p.line_segment([c + vec2(-h, h), c + vec2(h, -h)], Stroke::new(1.5, col));
}
