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

pub fn player_panel(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    let left = Rect::from_min_size(full.min, vec2(81.0, full.height()));
    let right_w = 204.0;
    let right = Rect::from_min_max(pos2(full.max.x - right_w, full.min.y), full.max);
    let main = Rect::from_min_max(pos2(left.max.x, full.min.y), pos2(right.min.x - 4.0, full.max.y));
    p.rect_filled(left, 0.0, BG2);
    p.rect_filled(right, 0.0, BG1);

    let pos = app.pos();
    let playing = app.playing();
    let loaded = app.deck.track.is_some();

    // ---------------- left column: jumps, beat jump, CUE, PLAY
    let lx = left.min.x;
    let ly = left.min.y;
    if tip(button(ui, Rect::from_min_size(pos2(lx + 8.0, ly + 28.0), vec2(30.0, 26.0)), "front", "⇤", BG2, TEXT, 15.0), app, "Jump to the front").clicked() {
        app.seek(0.0);
        if !app.playing() {
            app.deck.cue_point = 0.0;
        }
    }
    if tip(button(ui, Rect::from_min_size(pos2(lx + 43.0, ly + 28.0), vec2(30.0, 26.0)), "rear", "⇥", BG2, TEXT, 15.0), app, "Jump to the rear").clicked() {
        let d = app.deck.duration;
        app.seek((d - 0.5).max(0.0));
    }
    let bj = app.prefs.beat_jump;
    if tip(button(ui, Rect::from_min_size(pos2(lx + 8.0, ly + 70.0), vec2(30.0, 26.0)), "bjl", "‹", BG2, TEXT, 18.0), app, "Beat Jump backward").clicked() {
        app.beat_jump(-bj);
    }
    if tip(button(ui, Rect::from_min_size(pos2(lx + 43.0, ly + 70.0), vec2(30.0, 26.0)), "bjr", "›", BG2, TEXT, 18.0), app, "Beat Jump forward").clicked() {
        app.beat_jump(bj);
    }
    let bj_rect = Rect::from_min_size(pos2(lx + 8.0, ly + 100.0), vec2(65.0, 30.0));
    let bj_label = if bj < 1.0 { format!("1/{}Beat", (1.0 / bj).round()) } else { format!("{}Beats", bj) };
    let r = button(ui, bj_rect, "bjsize", &format!("{bj_label}\n⌄"), BTN, TEXT, 9.5);
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
    let cue_c = pos2(lx + 40.0, ly + 152.0);
    let cue_rect = Rect::from_center_size(cue_c, vec2(42.0, 42.0));
    let cr = ui.interact(cue_rect, ui.id().with("cuebtn"), Sense::click_and_drag());
    let cue_lit = loaded && (!playing || app.deck.cue_preview);
    p.circle_filled(cue_c, 19.0, if cr.hovered() { hex(0x2E2E2E) } else { BTN });
    p.circle_stroke(cue_c, 19.0, Stroke::new(2.0, if cue_lit { CUE_ORANGE } else { CUE_ORANGE.gamma_multiply(0.45) }));
    p.text(cue_c, Align2::CENTER_CENTER, "CUE", FontId::proportional(11.0), WHITE);
    if cr.is_pointer_button_down_on() && !app.deck.cue_preview && ui.input(|i| i.pointer.any_pressed()) {
        app.cue_down();
    }
    if !cr.is_pointer_button_down_on() && app.deck.cue_preview && !ui.input(|i| i.key_down(egui::Key::C)) {
        app.cue_up();
    }
    // PLAY
    let play_c = pos2(lx + 40.0, ly + 210.0);
    let pr = ui.interact(Rect::from_center_size(play_c, vec2(42.0, 42.0)), ui.id().with("playbtn"), Sense::click());
    p.circle_filled(play_c, 19.0, if pr.hovered() { hex(0x2E2E2E) } else { BTN });
    let green = if playing { PLAY_GREEN } else { hex(0x246822) };
    p.circle_stroke(play_c, 19.0, Stroke::new(2.0, green));
    if playing {
        for dx in [-4.0, 4.0] {
            p.rect_filled(Rect::from_center_size(play_c + vec2(dx, 0.0), vec2(4.0, 12.0)), 0.0, green);
        }
    } else {
        p.add(Shape::convex_polygon(vec![play_c + vec2(-4.0, -7.0), play_c + vec2(7.0, 0.0), play_c + vec2(-4.0, 7.0)], green, Stroke::NONE));
    }
    if tip(pr, app, "Play/Pause").clicked() {
        app.play_pause();
    }

    // ---------------- zoom column
    let zx = main.min.x;
    if tip(button(ui, Rect::from_min_size(pos2(zx + 4.0, ly + 82.0), vec2(26.0, 22.0)), "zin", "⊕", BG, TEXT, 16.0), app, "Zoom In").clicked() && app.prefs.zoom > 0 {
        app.prefs.zoom -= 1;
    }
    if tip(button(ui, Rect::from_min_size(pos2(zx + 4.0, ly + 110.0), vec2(26.0, 18.0)), "zrst", "RST", BG, TEXT, 9.0), app, "Reset Zoom").clicked() {
        app.prefs.zoom = 2;
    }
    if tip(button(ui, Rect::from_min_size(pos2(zx + 4.0, ly + 134.0), vec2(26.0, 22.0)), "zout", "⊖", BG, TEXT, 16.0), app, "Zoom Out").clicked() && app.prefs.zoom + 1 < ZOOMS.len() {
        app.prefs.zoom += 1;
    }

    let wx0 = main.min.x + 34.0;
    let wx1 = main.max.x;

    // ---------------- header row
    let hy = main.min.y + 2.0;
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    let dur = app.deck.duration;
    if let Some(t) = &track {
        let title_rect = p.text(pos2(wx0 + 56.0, hy + 9.0), Align2::LEFT_CENTER, t.display_title(), FontId::proportional(15.0), WHITE);
        let ax = (title_rect.max.x + 16.0).max(wx0 + 56.0 + (wx1 - wx0) * 0.42);
        p.line_segment([pos2(ax - 8.0, hy + 1.0), pos2(ax - 8.0, hy + 17.0)], Stroke::new(1.0, OFF));
        p.text(pos2(ax, hy + 9.0), Align2::LEFT_CENTER, &t.artist, FontId::proportional(12.0), DIM);
    }
    // time / key / bpm block on the right of the header
    let bx = wx1 - 360.0;
    p.line_segment([pos2(bx - 8.0, hy + 1.0), pos2(bx - 8.0, hy + 17.0)], Stroke::new(1.0, OFF));
    let remain = (dur - pos).max(0.0);
    let (big, small) = split_time(if loaded { remain } else { 0.0 });
    let tr = p.text(pos2(bx, hy + 9.0), Align2::LEFT_CENTER, format!("-{big}"), FontId::proportional(15.0), if app.prefs.time_remain { WHITE } else { DIM });
    let tr2 = p.text(pos2(tr.max.x, hy + 11.0), Align2::LEFT_CENTER, format!(".{small}"), FontId::proportional(11.0), if app.prefs.time_remain { WHITE } else { DIM });
    let (big2, small2) = split_time(if loaded { pos } else { 0.0 });
    let er = p.text(pos2(tr2.max.x + 12.0, hy + 9.0), Align2::LEFT_CENTER, &big2, FontId::proportional(15.0), if app.prefs.time_remain { DIM } else { WHITE });
    p.text(pos2(er.max.x, hy + 11.0), Align2::LEFT_CENTER, format!(".{small2}"), FontId::proportional(11.0), if app.prefs.time_remain { DIM } else { WHITE });
    let time_hit = ui.interact(Rect::from_min_max(pos2(bx, hy), pos2(er.max.x + 20.0, hy + 18.0)), ui.id().with("timesw"), Sense::click());
    if tip(time_hit, app, "Switch Time/Remain").clicked() {
        app.prefs.time_remain = !app.prefs.time_remain;
    }
    let kx = bx + 196.0;
    p.line_segment([pos2(kx - 10.0, hy + 1.0), pos2(kx - 10.0, hy + 17.0)], Stroke::new(1.0, OFF));
    if let Some(t) = &track {
        p.text(pos2(kx + 16.0, hy + 9.0), Align2::CENTER_CENTER, key_name(t.key, app.prefs.key_alnum), FontId::proportional(13.0), WHITE);
    }
    let bpx = kx + 60.0;
    p.line_segment([pos2(bpx - 10.0, hy + 1.0), pos2(bpx - 10.0, hy + 17.0)], Stroke::new(1.0, OFF));
    if loaded {
        let b = app.current_bpm();
        p.text(pos2(bpx + 30.0, hy + 9.0), Align2::CENTER_CENTER, if b > 0.0 { format!("{b:.2}") } else { "--.--".into() }, FontId::proportional(13.0), WHITE);
    }

    // ---------------- artwork
    let art = Rect::from_min_size(pos2(wx0 + 2.0, main.min.y + 20.0), vec2(46.0, 46.0));
    p.rect_filled(art, 0.0, hex(0x141414));
    if let Some(t) = &track {
        if let Some(tex) = crate::ui_browser::artwork_texture(app, &t.artwork) {
            p.image(tex.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            draw_note_icon(&p, art);
        }
    }

    // ---------------- overview waveform
    let ov = Rect::from_min_max(pos2(wx0 + 56.0, main.min.y + 26.0), pos2(wx1 - 4.0, main.min.y + 62.0));
    draw_overview(app, ui, ov, pos);

    // ---------------- enlarged waveform
    let det = Rect::from_min_max(pos2(wx0, main.min.y + 70.0), pos2(wx1, main.max.y - 52.0));
    draw_detail(app, ui, det, pos);

    // ---------------- control strip
    let strip = Rect::from_min_max(pos2(left.max.x, main.max.y - 48.0), pos2(main.max.x, main.max.y));
    p.rect_filled(strip, 0.0, BG2);
    control_strip(app, ui, strip);

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
        _ => vec![(f(v[1]).max(f(v[0]) * 0.55), WAVE_LOW), (f(v[2]), WAVE_MID), (f(v[3]) * 0.9, WAVE_HIGH)],
    }
}

fn draw_overview(app: &mut App, ui: &mut Ui, r: Rect, pos: f64) {
    let p = ui.painter().clone();
    let dur = app.deck.duration.max(0.001);
    let mid = r.center().y + 2.0;
    let half = r.height() / 2.0 - 2.0;
    if let Some(w) = &app.deck.wave {
        let n = r.width().max(1.0) as usize;
        let cols = w.columns(n);
        for (i, c) in cols.iter().enumerate() {
            let x = r.min.x + i as f32 + 0.5;
            let played = (i as f64 / n as f64) * dur < pos;
            for (h, col) in wave_bars(app.prefs.wave_color, *c) {
                let col = if played { col.gamma_multiply(0.45) } else { col };
                // overview is drawn from the baseline upwards like the original
                p.line_segment([pos2(x, mid + half * 0.15), pos2(x, mid + half * 0.15 - h * half * 1.15)], Stroke::new(1.0, col));
            }
        }
    } else if app.deck.track.is_some() {
        p.text(r.center(), Align2::CENTER_CENTER, "Analyzing…", FontId::proportional(11.0), DIM);
    }
    // time scale ticks
    p.line_segment([pos2(r.min.x, r.max.y - 1.0), pos2(r.max.x, r.max.y - 1.0)], Stroke::new(1.0, hex(0x3A3A3A)));
    if app.deck.track.is_some() {
        let minute = 60.0 / dur * r.width() as f64;
        let mut k = 1.0;
        while k * minute < r.width() as f64 {
            let x = r.min.x + (k * minute) as f32;
            p.line_segment([pos2(x, r.max.y - 3.0), pos2(x, r.max.y)], Stroke::new(1.0, DIM));
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
                tri_up(&p, pos2(x, r.max.y + 1.0), 5.0, col);
            } else {
                let col = hotcue_color(c.color);
                let lr = Rect::from_min_size(pos2(x - 0.5, r.min.y - 10.0), vec2(10.0, 10.0));
                p.rect_filled(lr, 0.0, col);
                p.text(lr.center(), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), FontId::proportional(8.0), WHITE);
                p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.0, col));
            }
        }
        // cue point
        tri_up(&p, pos2(x_of(app.deck.cue_point), r.max.y + 1.0), 6.0, CUE_MARK);
        // playhead
        let px = x_of(pos);
        p.line_segment([pos2(px, r.min.y - 2.0), pos2(px, r.max.y)], Stroke::new(1.5, WHITE));
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
    let p = ui.painter_at(r.expand2(vec2(0.0, 8.0)));
    let span = ZOOMS[app.prefs.zoom.min(ZOOMS.len() - 1)];
    let px_per_s = r.width() as f64 / span;
    let cx = r.center().x;
    let t_of = |x: f32| pos + (x - cx) as f64 / px_per_s;
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
                    let mut m = [0u8; 4];
                    for f in &w.frames[a..b.max(a + 1).min(n)] {
                        for k in 0..4 {
                            m[k] = m[k].max(f[k]);
                        }
                    }
                    for (h, col) in wave_bars(app.prefs.wave_color, m) {
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
            tri_down(&p, pos2(x, r.min.y + 6.0), 3.5, BAR_RED);
            tri_up(&p, pos2(x, r.max.y - 6.0), 3.5, BAR_RED);
            p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 46)));
        } else {
            p.line_segment([pos2(x, r.min.y), pos2(x, r.min.y + 5.0)], Stroke::new(1.0, hex(0xA0A0A0)));
            p.line_segment([pos2(x, r.max.y - 5.0), pos2(x, r.max.y)], Stroke::new(1.0, hex(0xA0A0A0)));
            p.line_segment([pos2(x, r.min.y + 5.0), pos2(x, r.max.y - 5.0)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(160, 160, 160, 26)));
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
            p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.5, col));
            let lr = Rect::from_min_size(pos2(x, r.min.y), vec2(13.0, 13.0));
            p.rect_filled(lr, 0.0, col);
            p.text(lr.center(), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), FontId::proportional(10.0), WHITE);
            if !c.comment.is_empty() {
                p.text(pos2(x + 16.0, r.min.y + 6.0), Align2::LEFT_CENTER, &c.comment, FontId::proportional(10.0), col);
            }
        } else {
            let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
            tri_up(&p, pos2(x, r.max.y - 9.0), 6.0, col);
        }
    }
    tri_up(&p, pos2(x_of(app.deck.cue_point), r.max.y - 9.0), 7.0, CUE_MARK);
    if app.deck.track.is_some() {
        // playhead
        p.line_segment([pos2(cx, r.min.y - 4.0), pos2(cx, r.max.y + 4.0)], Stroke::new(1.5, WHITE));
        p.rect_filled(Rect::from_center_size(pos2(cx, r.min.y + 2.0), vec2(6.0, 6.0)), 0.0, BAR_RED);
        let label = bars_label(&app.deck.beats, pos);
        p.text(pos2(cx - 6.0, r.min.y + 10.0), Align2::RIGHT_CENTER, label, FontId::proportional(15.0), BEAT_BLUE);
    }
    // interaction: drag to scrub, scroll to move, ctrl/cmd + scroll to zoom
    let resp = ui.interact(r, ui.id().with("detail"), Sense::click_and_drag());
    if app.deck.audio.is_some() {
        if resp.drag_started() {
            app.deck.drag_origin = Some((resp.interact_pointer_pos().map(|p| p.x).unwrap_or(cx), pos));
        }
        if resp.dragged() {
            if let (Some((x0, p0)), Some(pp)) = (app.deck.drag_origin, resp.interact_pointer_pos()) {
                let t = p0 - (pp.x - x0) as f64 / px_per_s;
                app.seek(t);
            }
        }
        if resp.drag_stopped() {
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

fn control_strip(app: &mut App, ui: &mut Ui, s: Rect) {
    let p = ui.painter().clone();
    // Tabs CUE/LOOP and GRID (stacked).
    let tab1 = Rect::from_min_size(pos2(s.min.x + 4.0, s.min.y + 1.0), vec2(79.0, 22.0));
    let tab2 = Rect::from_min_size(pos2(s.min.x + 4.0, s.min.y + 24.0), vec2(79.0, 22.0));
    if button(ui, tab1, "tab_cue", "CUE/LOOP", if app.deck.ctrl_tab == 0 { BG2 } else { BG }, WHITE, 10.0).clicked() {
        app.deck.ctrl_tab = 0;
    }
    if button(ui, tab2, "tab_grid", "GRID", if app.deck.ctrl_tab == 1 { BG2 } else { BG }, WHITE, 10.0).clicked() {
        app.deck.ctrl_tab = 1;
    }
    let y = s.center().y;
    let x0 = s.min.x + 110.0;
    if app.deck.ctrl_tab == 0 {
        // hot cue pads
        let mut x = x0 + (s.width() - 900.0).max(0.0) * 0.25;
        for slot in 0..8 {
            let r = Rect::from_center_size(pos2(x + 13.0, y), vec2(26.0, 26.0));
            let cue = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).cloned();
            let (fill, tc) = match &cue {
                Some(c) => (hotcue_color(c.color), WHITE),
                None => (BG1, OFF),
            };
            let resp = ui.interact(r, ui.id().with(("pad", slot)), Sense::click_and_drag());
            p.rect_filled(r, 2.0, if resp.hovered() && cue.is_none() { hex(0x1A1A1A) } else { fill });
            p.text(r.center(), Align2::CENTER_CENTER, ((b'A' + slot as u8) as char).to_string(), FontId::proportional(14.0), tc);
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
            x += 33.0;
        }
        x += 22.0;
        // memory
        let mr = Rect::from_min_size(pos2(x, y - 12.0), vec2(72.0, 24.0));
        if tip(button(ui, mr, "mem", "MEMORY", BTN, WHITE, 10.5), app, "Set Memory Cue(Loop)").clicked() {
            app.memory_set();
        }
        x += 78.0;
        if tip(button(ui, Rect::from_min_size(pos2(x, y - 12.0), vec2(24.0, 24.0)), "memprev", "◀", BG2, WHITE, 11.0), app, "Call Previous Memory Cue").clicked() {
            app.memory_jump(-1);
        }
        x += 28.0;
        if tip(button(ui, Rect::from_min_size(pos2(x, y - 12.0), vec2(24.0, 24.0)), "memnext", "▶", BG2, WHITE, 11.0), app, "Call Next Memory Cue").clicked() {
            app.memory_jump(1);
        }
        x += 28.0;
        if tip(button(ui, Rect::from_min_size(pos2(x, y - 12.0), vec2(24.0, 24.0)), "memdel", "✕", BG2, WHITE, 13.0), app, "Delete Memory Cue(Loop)").clicked() {
            app.memory_delete_current();
        }
        x += 52.0;
        // loop group
        let au = app.prefs.loop_auto;
        if tip(button(ui, Rect::from_min_size(pos2(x, y - 12.0), vec2(48.0, 24.0)), "au", "AU", if au { BLUE } else { BG2 }, WHITE, 11.0), app, "Auto Beat Loop").clicked() {
            app.prefs.loop_auto = true;
        }
        x += 52.0;
        if tip(button(ui, Rect::from_min_size(pos2(x, y - 12.0), vec2(48.0, 24.0)), "ma", "MA", if !au { BLUE } else { BG2 }, WHITE, 11.0), app, "Manual Loop").clicked() {
            app.prefs.loop_auto = false;
        }
        x += 60.0;
        if au {
            if button(ui, Rect::from_min_size(pos2(x, y - 12.0), vec2(22.0, 24.0)), "lhalf", "‹", BG2, WHITE, 16.0).clicked() {
                app.resize_loop(0.5);
            }
            x += 26.0;
            let lb = app.prefs.loop_beats;
            let label = if lb < 1.0 { format!("1/{}", (1.0 / lb).round()) } else { format!("{}", lb) };
            let active = app.loop_active().is_some();
            let lr = Rect::from_min_size(pos2(x, y - 12.0), vec2(88.0, 24.0));
            if tip(button(ui, lr, "lbeat", &label, if active { LOOP.gamma_multiply(0.7) } else { BTN }, WHITE, 12.0), app, "Auto Beat Loop on/off").clicked() {
                if active {
                    app.loop_exit();
                } else {
                    app.auto_loop(lb);
                }
            }
            x += 92.0;
            if button(ui, Rect::from_min_size(pos2(x, y - 12.0), vec2(22.0, 24.0)), "ldouble", "›", BG2, WHITE, 16.0).clicked() {
                app.resize_loop(2.0);
            }
            x += 34.0;
        } else {
            for (lbl, id) in [("IN", "lin"), ("OUT", "lout"), ("RELOOP", "lre"), ("EXIT", "lex")] {
                let w = if lbl.len() > 3 { 56.0 } else { 38.0 };
                let on = match lbl {
                    "IN" => app.deck.loop_in.is_some(),
                    "EXIT" => false,
                    _ => app.loop_active().is_some() && lbl == "OUT",
                };
                if button(ui, Rect::from_min_size(pos2(x, y - 12.0), vec2(w, 24.0)), id, lbl, if on { LOOP.gamma_multiply(0.7) } else { BTN }, WHITE, 10.5).clicked() {
                    match lbl {
                        "IN" => app.loop_in(),
                        "OUT" => app.loop_out(),
                        "RELOOP" => app.reloop(),
                        _ => app.loop_exit(),
                    }
                }
                x += w + 4.0;
            }
            x += 8.0;
        }
        let qx = s.max.x - 70.0;
        let q = app.prefs.quantize;
        if tip(button(ui, Rect::from_min_size(pos2(qx.max(x), y - 12.0), vec2(40.0, 24.0)), "quant", "Q", if q { BLUE } else { BG2 }, WHITE, 12.0), app, "Quantize ON/OFF").clicked() {
            app.prefs.quantize = !q;
            app.save_prefs();
        }
        let menu_r = button(ui, Rect::from_min_size(pos2(qx.max(x) + 46.0, y - 12.0), vec2(22.0, 24.0)), "stripmenu", "≡", BG2, WHITE, 16.0);
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
    } else {
        grid_strip(app, ui, s, x0);
    }
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
    if b(ui, app, "gundo", "↶", 28.0, "Undo", false) {
        app.grid_undo();
    }
    if b(ui, app, "gredo", "↷", 28.0, "Redo", false) {
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
    let tabs_y = r.max.y - 30.0;
    let list = Rect::from_min_max(pos2(r.min.x + 4.0, r.min.y + 4.0), pos2(r.max.x - 4.0, tabs_y - 6.0));
    p.rect_filled(list, 0.0, BG1);
    let tw = (r.width() - 16.0) / 3.0;
    for (i, name) in ["MEMORY", "HOT CUE", "INFO"].iter().enumerate() {
        let tr = Rect::from_min_size(pos2(r.min.x + 4.0 + i as f32 * (tw + 4.0), tabs_y), vec2(tw, 24.0));
        let tipt = ["Show MEMORY Panel", "Show HOT CUE Panel", "Show INFO Panel"][i];
        if tip(button(ui, tr, &format!("ptab{i}"), name, if app.deck.panel_tab == i as u8 { BLUE } else { BG2 }, WHITE, 10.5), app, tipt).clicked() {
            app.deck.panel_tab = i as u8;
        }
    }
    let row_h = (list.height() / 10.0).floor();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    match app.deck.panel_tab {
        0 => {
            let mems = app.memory_cues();
            for i in 0..10 {
                let rr = Rect::from_min_size(pos2(list.min.x, list.min.y + i as f32 * row_h), vec2(list.width(), row_h));
                p.line_segment([pos2(rr.min.x, rr.max.y), pos2(rr.max.x, rr.max.y)], Stroke::new(1.0, BG2));
                let Some(c) = mems.get(i).cloned() else {
                    p.text(pos2(rr.max.x - 10.0, rr.center().y), Align2::CENTER_CENTER, "✕", FontId::proportional(10.0), hex(0x3A3A3A));
                    continue;
                };
                let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
                tri_up(&p, pos2(rr.min.x + 9.0, rr.center().y - 4.0), 4.0, col);
                p.text(pos2(rr.min.x + 18.0, rr.center().y), Align2::LEFT_CENTER, cue_time(c.start), FontId::proportional(10.5), WHITE);
                if c.is_loop() {
                    p.text(pos2(rr.min.x + 82.0, rr.center().y), Align2::LEFT_CENTER, "⟲", FontId::proportional(11.0), if c.active_loop { LOOP } else { DIM });
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
                p.text(del.center(), Align2::CENTER_CENTER, "✕", FontId::proportional(10.0), DIM);
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
                let rr = Rect::from_min_size(pos2(list.min.x, list.min.y + slot as f32 * row_h * 1.25), vec2(list.width(), row_h * 1.25));
                p.line_segment([pos2(rr.min.x, rr.max.y), pos2(rr.max.x, rr.max.y)], Stroke::new(1.0, BG2));
                let c = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).cloned();
                let chip = Rect::from_center_size(pos2(rr.min.x + 12.0, rr.center().y), vec2(15.0, 15.0));
                let letter = ((b'A' + slot as u8) as char).to_string();
                match &c {
                    Some(c) => {
                        p.rect_filled(chip, 1.0, hotcue_color(c.color));
                        p.text(chip.center(), Align2::CENTER_CENTER, &letter, FontId::proportional(10.0), WHITE);
                        p.text(pos2(rr.min.x + 26.0, rr.center().y), Align2::LEFT_CENTER, fmt_time(c.start, false), FontId::proportional(10.5), WHITE);
                        p.text(pos2(rr.min.x + 64.0, rr.center().y), Align2::LEFT_CENTER, bars_label(&app.deck.beats, c.start), FontId::proportional(10.5), BEAT_BLUE);
                        let del = Rect::from_center_size(pos2(rr.max.x - 10.0, rr.center().y), vec2(16.0, rr.height()));
                        if app.deck.editing_comment == Some(c.id) {
                            let mut txt = c.comment.clone();
                            let er = Rect::from_min_max(pos2(rr.min.x + 116.0, rr.min.y + 2.0), pos2(rr.max.x - 20.0, rr.max.y - 2.0));
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
                            p.text(pos2(rr.min.x + 116.0, rr.center().y), Align2::LEFT_CENTER, &c.comment, FontId::proportional(10.5), DIM2);
                        }
                        p.text(del.center(), Align2::CENTER_CENTER, "✕", FontId::proportional(10.0), DIM);
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
                        p.rect_filled(chip, 1.0, BG2);
                        p.text(chip.center(), Align2::CENTER_CENTER, &letter, FontId::proportional(10.0), OFF);
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
