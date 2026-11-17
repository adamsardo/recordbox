//! Header bar, browser (sidebar tree, track list, My Tag panel, devices) and footer.

use eframe::egui;
use crate::app::*;
use crate::model::*;
use crate::theme::*;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui};
use std::collections::{HashMap, HashSet};

/// Track list row pitch, as measured in the original (15 pt).
pub const ROW_H: f32 = 15.0;
/// Text size of the track list's title bar and column headers.
pub const TITLE_FONT: f32 = 10.5;

/// An unboxed icon button.
pub fn icon_button_ui(ui: &mut Ui, rect: Rect, id: &str, draw: fn(&egui::Painter, Rect, Color32)) -> egui::Response {
    crate::ui_player::icon_button(ui, rect, id, None, draw)
}

/// A click-to-open menu under a custom drawn button.
pub fn dropdown(ui: &mut Ui, resp: &egui::Response, id: &str, add: impl FnOnce(&mut Ui)) {
    let pid = ui.id().with(("dropdown", id));
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(pid));
    }
    egui::popup::popup_below_widget(ui, pid, resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(160.0);
        add(ui);
    });
}

pub fn artwork_texture(app: &mut App, path: &str) -> Option<egui::TextureHandle> {
    if path.is_empty() {
        return None;
    }
    if let Some(t) = app.textures.get(path) {
        return t.clone();
    }
    let tex = image::open(path).ok().map(|img| {
        let small = img.thumbnail(96, 96).to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], &small);
        app.ctx.load_texture(path, ci, egui::TextureOptions::LINEAR)
    });
    app.textures.insert(path.to_string(), tex.clone());
    tex
}

// ------------------------------------------------------------------------------------ header
/// Header bar, 28 pt. Positions measured in the original (x in pt from the window's left edge;
/// right-hand items measured from the right edge of a 1512 pt window).
pub fn header_bar(app: &mut App, ui: &mut Ui) {
    let r = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, BG1);
    let tips_on = app.prefs.tooltips;
    let tipf = move |resp: egui::Response, t: &str| if tips_on { resp.on_hover_text(t) } else { resp };
    // Measured on rekordbox (28 pt header): EXPORT in 10.3 pt Arial Bold from x = 16.5 (caps
    // 9.5–17), pull-down chevrons (8 × 4, 1 pt) centred at 115.75 and 170.75, y 14.5.
    let ip = crate::icons_player::chevron_down;
    p.text(pos2(r.min.x + 16.0, r.min.y + 14.0), Align2::LEFT_CENTER, "EXPORT", caps(10.3), WHITE);
    let mode_r = Rect::from_min_max(pos2(r.min.x + 8.0, r.min.y), pos2(r.min.x + 124.0, r.max.y));
    ip(&p, pos2(r.min.x + 115.75, r.min.y + 14.5), 1.0, WHITE);
    let _ = tipf(ui.interact(mode_r, ui.id().with("mode"), Sense::hover()), "This app covers EXPORT mode: library, preparation and USB export.");
    // Layout selector: an 18 × 9 frame at x 136–154, y 9.5–18.5.
    let lay_r = Rect::from_min_max(pos2(r.min.x + 130.0, r.min.y + 3.0), pos2(r.min.x + 180.0, r.max.y - 3.0));
    let lay = ui.interact(lay_r, ui.id().with("layout"), Sense::click());
    if lay.hovered() {
        p.rect_filled(lay_r, 2.0, hex(0x1E1E1E));
    }
    crate::icons_player::layout_mark(&p, pos2(r.min.x + 145.0, r.min.y + 14.0), app.player_layout(), WHITE);
    ip(&p, pos2(r.min.x + 170.75, r.min.y + 14.5), 1.0, WHITE);
    dropdown(ui, &lay, "layout", |ui| {
        // Layouts of the player, in rekordbox's order, each with its mark
        // (App::set_player_layout: 0–3 player layouts, 4 = browser only).
        let cur = app.player_layout();
        for (i, name) in ["1 Player", "2 Players", "Dual Player", "Simple Player", "Full Browser"].iter().enumerate() {
            let resp = ui.selectable_label(cur == i as u8, format!("        {name}"));
            crate::icons_player::layout_mark(ui.painter(), pos2(resp.rect.min.x + 15.0, resp.rect.center().y), i as u8, WHITE);
            if resp.clicked() {
                app.set_player_layout(i as u8);
            }
        }
    });
    // Auto-analysis switch in the place of the original's circle mark: a 20 pt #282828 tile
    // at x 310–330, y 4–24 with a 13 pt ring and a 7 pt dot.
    let rec_r = Rect::from_min_size(pos2(r.min.x + 310.0, r.min.y + 4.0), vec2(20.0, 20.0));
    let rec = ui.interact(rec_r, ui.id().with("autoan"), Sense::click());
    p.rect_filled(rec_r, 2.0, if rec.hovered() { hex(0x323232) } else { BTN });
    crate::icons_player::record(&p, rec_r.center(), if app.prefs.auto_analysis { WHITE } else { OFF });
    if tipf(rec, if app.prefs.auto_analysis { "Auto Analysis On (click to turn off)" } else { "Auto Analysis Off (click to turn on)" }).clicked() {
        app.prefs.auto_analysis = !app.prefs.auto_analysis;
        app.save_prefs();
    }
    // Right side, measured from the right edge (1512 pt window): gear 14 pt centred at −261,
    // sync mark 18 pt centred at −223, level meter −204..−124, volume −102..−70, clock ends −21.5.
    let rx = r.max.x;
    let ty = r.min.y;
    let now = chrono::Local::now().format("%H:%M").to_string();
    p.text(pos2(rx - 21.4, ty + 13.8), Align2::RIGHT_CENTER, now, FontId::proportional(10.7), WHITE);
    let gear_r = Rect::from_center_size(pos2(rx - 261.0, ty + 13.0), vec2(14.0, 14.0));
    let gear = ui.interact(gear_r.expand(4.0), ui.id().with("gear"), Sense::click());
    crate::icons_player::gear(&p, gear_r.center(), 1.0, WHITE, BG1);
    if tipf(gear, "Preferences").clicked() {
        app.dialogs.prefs_open = true;
    }
    // Sync / analysis activity mark (turns while work is running).
    let busy = app.analysis_total > 0 || app.export_job.is_some() || app.importing.is_some();
    let spin = if busy { ui.input(|i| i.time) as f32 * 4.0 } else { 0.0 };
    crate::icons_player::sync_mark(&p, pos2(rx - 223.0, ty + 14.0), spin, BLUE);
    if busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    // Level meter (left and right): two 80 × 3 pt bars at y 9–12 and 15–18.
    let bars = [Rect::from_min_max(pos2(rx - 204.0, ty + 9.0), pos2(rx - 124.0, ty + 12.0)), Rect::from_min_max(pos2(rx - 204.0, ty + 15.0), pos2(rx - 124.0, ty + 18.0))];
    for b in bars {
        p.rect_filled(b, 0.0, hex(0x323232));
    }
    if app.playing() {
        if let Some(w) = &app.deck.wave {
            let f = (app.pos() * WAVE_FPS) as usize;
            if let Some(fr) = w.frames.get(f) {
                let lvl = (fr[0] as f32 / 255.0).min(1.0);
                for b in bars {
                    p.rect_filled(Rect::from_min_max(b.min, pos2(b.min.x + b.width() * lvl, b.max.y)), 0.0, hex(0xB0B0B0));
                }
            }
        }
    }
    // Master volume slider: a 32 × 3 pt bar at y 13–16.
    let sl = Rect::from_min_max(pos2(rx - 102.0, ty + 8.0), pos2(rx - 70.0, ty + 21.0));
    let sresp = ui.interact(sl.expand2(vec2(3.0, 0.0)), ui.id().with("vol"), Sense::click_and_drag());
    let mut v = app.player.state.lock().volume;
    if sresp.dragged() || sresp.clicked() {
        if let Some(pp) = sresp.interact_pointer_pos() {
            v = ((pp.x - sl.min.x) / sl.width()).clamp(0.0, 1.0);
            app.player.state.lock().volume = v;
        }
    }
    p.rect_filled(Rect::from_min_max(pos2(sl.min.x, ty + 13.0), pos2(sl.max.x, ty + 16.0)), 0.0, hex(0x323232));
    p.rect_filled(Rect::from_min_max(pos2(sl.min.x, ty + 13.0), pos2(sl.min.x + (sl.width() * v).max(2.0), ty + 16.0)), 0.0, BLUE);
    let _ = tipf(sresp, "Master volume (preview)");
}

fn menus(app: &mut App, ui: &mut Ui) {
    let small = |s: &str| RichText::new(s).size(11.5).color(DIM2);
    ui.menu_button(small("File"), |ui| {
        if ui.button("Import Track…").clicked() {
            ui.close_menu();
            if let Some(files) = rfd::FileDialog::new().add_filter("Audio", &["mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "alac", "mp4"]).pick_files() {
                app.import_paths(files, app.is_manual_playlist());
            }
        }
        if ui.button("Import Folder…").clicked() {
            ui.close_menu();
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                app.import_paths(vec![d], app.is_manual_playlist());
            }
        }
        if ui.button("Import Playlist (m3u)…").clicked() {
            ui.close_menu();
            if let Some(f) = rfd::FileDialog::new().add_filter("Playlist", &["m3u", "m3u8"]).pick_file() {
                import_m3u(app, &f);
            }
        }
        ui.separator();
        if ui.button("Import collection from rekordbox xml…").on_hover_text("Reads a collection exported by rekordbox (File > Export Collection in xml format). Nothing is written to rekordbox or your music.").clicked() {
            ui.close_menu();
            if let Some(f) = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file() {
                app.import_xml(&f);
            }
        }
        if ui.button("Export Collection in xml format…").clicked() {
            ui.close_menu();
            if let Some(f) = rfd::FileDialog::new().set_file_name("collection.xml").save_file() {
                app.export_collection_xml(&f);
            }
        }
        ui.separator();
        if ui.button("Display All Missing Files").clicked() {
            ui.close_menu();
            app.set_view(View::Collection);
            app.side = SideMode::Collection;
            app.search.clear();
            let missing: Vec<i64> = app.tracks.iter().filter(|t| t.missing).map(|t| t.id).collect();
            app.toast(format!("{} missing file(s) in the collection", missing.len()));
            app.selected = missing.into_iter().collect();
        }
        if ui.button("Show library folder in Finder").clicked() {
            ui.close_menu();
            let _ = std::process::Command::new("/usr/bin/open").arg(crate::db::data_dir()).spawn();
        }
        if ui.button("Backup Library…").clicked() {
            ui.close_menu();
            if let Some(f) = rfd::FileDialog::new().set_file_name("recordbox-library-backup.db").save_file() {
                let _ = app.db.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                match std::fs::copy(crate::db::data_dir().join("library.db"), &f) {
                    Ok(_) => app.toast("Library backed up."),
                    Err(e) => app.toast_err(format!("Backup failed: {e}")),
                }
            }
        }
    });
    ui.menu_button(small("View"), |ui| {
        if ui.checkbox(&mut app.prefs.browser_only, "Full Browser").clicked() {
            app.save_prefs();
            ui.close_menu();
        }
        if ui.checkbox(&mut app.prefs.show_mytag, "My Tag Configuration Window").clicked() {
            app.save_prefs();
            ui.close_menu();
        }
        if ui.checkbox(&mut app.filter_open, "Filter").clicked() {
            app.list_dirty = true;
            ui.close_menu();
        }
        ui.separator();
        ui.label("Waveform color");
        for (i, n) in ["BLUE", "RGB", "3Band"].iter().enumerate() {
            if ui.radio(app.prefs.wave_color == i as u8, *n).clicked() {
                app.prefs.wave_color = i as u8;
                app.save_prefs();
            }
        }
    });
    ui.menu_button(small("Track"), |ui| {
        let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
        track_menu_items(app, ui, &sel);
    });
    ui.menu_button(small("Playlist"), |ui| {
        if ui.button("Create New Playlist").clicked() {
            ui.close_menu();
            app.side = SideMode::Playlists;
            app.create_node(0, NodeKind::Playlist);
        }
        if ui.button("Create New Intelligent Playlist").clicked() {
            ui.close_menu();
            app.side = SideMode::Playlists;
            app.create_node(0, NodeKind::Smart);
        }
        if ui.button("Create New Folder").clicked() {
            ui.close_menu();
            app.side = SideMode::Playlists;
            app.create_node(0, NodeKind::Folder);
        }
        ui.separator();
        if ui.button("Export to USB device…").clicked() {
            ui.close_menu();
            open_export(app, vec![], vec![]);
        }
    });
    ui.menu_button(small("Help"), |ui| {
        if ui.button("About recordbox").clicked() {
            app.dialogs.about = true;
            ui.close_menu();
        }
        ui.label(RichText::new("Shortcuts: Space play/pause · C cue · 1–8 hot cues · M memory · ↑/↓ select · Return load").size(10.5).color(DIM));
    });
    let auto = app.prefs.auto_analysis;
    let a = ui.add(egui::Label::new(RichText::new(if auto { "Auto Analysis On" } else { "Auto Analysis Off" }).size(10.5).color(if auto { BLUE } else { DIM })).sense(Sense::click()));
    if a.on_hover_text("Analyse tracks automatically when they are imported").clicked() {
        app.prefs.auto_analysis = !auto;
        app.save_prefs();
    }
}

pub fn import_m3u(app: &mut App, f: &std::path::Path) {
    app.import_playlist_file(f, 0);
}

pub fn open_export(app: &mut App, playlists: Vec<i64>, tracks: Vec<i64>) {
    app.scan_devices();
    app.dialogs.export_open = true;
    app.dialogs.export_sel = playlists.into_iter().collect();
    app.dialogs.export_tracks = tracks;
    if app.dialogs.export_root.as_ref().map(|r| !app.devices.iter().any(|d| &d.root == r)).unwrap_or(true) {
        app.dialogs.export_root = app.devices.first().map(|d| d.root.clone());
    }
}

// ------------------------------------------------------------------------------------ footer
pub fn footer(app: &mut App, ui: &mut Ui) {
    let r = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, BG);
    // The recordbox logo where the original shows its wordmark, at the same size and place
    // (references 01/02): the mark starts 10 pt from the left, the letters' x-height is 8.5 pt
    // and their baseline 8 pt above the window's foot (0.169 pt per logo unit).
    crate::brand::paint_logo(ui, Rect::from_min_size(pos2(r.min.x + 9.66, r.max.y - 19.7), vec2(104.1, 14.0)), WHITE);
    // totals
    let (n, secs, bytes) = if let View::Device { .. } = app.view {
        (0usize, 0.0, 0u64)
    } else {
        app.list.iter().filter_map(|id| app.track(*id)).fold((0, 0.0, 0u64), |(n, s, b), t| (n + 1, s + t.duration, b + t.file_size))
    };
    let h = (secs / 3600.0).floor();
    let m = ((secs - h * 3600.0) / 60.0).floor();
    let total = format!("{n} Tracks, {} hours {} minutes, {:.1} GB", h, m, bytes as f64 / 1e9);
    p.text(pos2(r.max.x - 34.5, r.center().y), Align2::RIGHT_CENTER, total, rb_bold(11.5), DIM);
    // status / messages
    let mid = Rect::from_min_max(pos2(r.min.x + 120.0, r.min.y + 3.0), pos2(r.max.x - 300.0, r.max.y - 3.0));
    app.toasts.retain(|t| t.until > std::time::Instant::now());
    if let Some(t) = app.toasts.last() {
        if t.error {
            p.rect_filled(mid, 0.0, CAUTION_BG);
            p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, &t.text, rb(11.5), CAUTION_TX);
        } else {
            p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, &t.text, rb(11.5), TEXT);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    } else if let Some(job) = &app.export_job {
        let (d, t, txt) = &job.last;
        p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, format!("Exporting {}/{} · {}", d, t, txt), rb(11.5), TEXT);
    } else if app.analysis_total > 0 {
        let pending = app.pool.pending.load(std::sync::atomic::Ordering::Relaxed);
        p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, format!("Analyzing… {} of {}", app.analysis_total - pending, app.analysis_total), rb(11.5), TEXT);
        let stop = ui.interact(Rect::from_min_size(pos2(mid.max.x - 120.0, mid.min.y), vec2(110.0, mid.height())), ui.id().with("stopan"), Sense::click());
        p.text(pos2(mid.max.x - 64.0, mid.center().y), Align2::CENTER_CENTER, "Stop Analyzing", rb(11.0), BLUE);
        if stop.clicked() {
            app.pool.stop();
        }
    } else if app.importing.is_some() {
        p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, format!("Importing… {}", app.import_count), rb(11.5), TEXT);
    }
}

// ------------------------------------------------------------------------------------ browser
/// Browser geometry measured in the original (pt): icon strip 0..26.5, tree pane 26.5..247,
/// track list 253..1485.5, right icon strip 1488..1512; top row of the tree area 20 pt high.
pub fn browser(app: &mut App, ui: &mut Ui) {
    app.poll_library_ext();
    // rekordbox's browser starts 283 pt below the window top: one point above this panel (the
    // player's last point is empty), so the browser is drawn from there.
    let mut full = ui.max_rect();
    if !app.prefs.browser_only {
        full.min.y -= 1.0;
        let mut clip = ui.clip_rect();
        clip.min.y = clip.min.y.min(full.min.y);
        ui.set_clip_rect(clip);
    }
    // Measured in the references (pt from the window's left edge): 2 pt black margin, icon
    // strip to 37, tree pane to `tree_w` (248 in references 01/04/05, 364 in the 7.2.19 set),
    // track list from 5 pt further with its scroll bar ending at the right-hand window (or at
    // the right icon strip, 1487..1512). The top row of the tree area is 21 pt high with a
    // 1 pt line under it.
    crate::ui_browser_ext::sync_right_panels(app);
    let whole = full;
    let track_view = !matches!(app.view, View::Device { .. } | View::DeviceHistory { .. } | View::Explorer(_) | View::Source { .. });
    // Sub-browser under the browser (reference 15): the browser keeps the upper 298 of 608 pt.
    let sub_below = if app.prefs.bx.sub_horizontal {
        let split = full.min.y + (full.height() * 0.49).round();
        let sub = Rect::from_min_max(pos2(full.min.x, split + 8.0), full.max);
        full.max.y = split;
        Some(sub)
    } else {
        None
    };
    let tw = app.prefs.bx.tree_w.clamp(170.0, (full.width() - 420.0).max(170.0));
    let strip = Rect::from_min_size(full.min, vec2(37.0, full.height()));
    let tree = Rect::from_min_max(pos2(strip.max.x, full.min.y), pos2(full.min.x + tw, full.max.y));
    let right_strip = Rect::from_min_max(pos2(full.max.x - 25.0, full.min.y), full.max);
    // Which side was clicked last decides how bright the lit source tile is.
    if let Some(pp) = ui.input(|i| if i.pointer.primary_pressed() { i.pointer.interact_pos() } else { None }) {
        if strip.contains(pp) || tree.contains(pp) {
            app.ext.tree_focus = true;
        } else if full.contains(pp) && !right_strip.contains(pp) {
            app.ext.tree_focus = false;
        }
    }
    // One window at a time left of the right strip (references 10-14): My Tag 223 pt, Related
    // Tracks / Track Suggestion 334 pt, Information 445 pt, the sub-browser 717 of 1512 pt;
    // the track list ends 5 pt before it.
    let rp = crate::ui_browser_ext::right_panel(app);
    let avail = right_strip.min.x - (full.min.x + tw + 5.0);
    let pw = match rp {
        1 => 223.0,
        2 | 3 => 334.0,
        4 => 445.0,
        5 => (full.width() * 0.4742).round(),
        _ => 0.0,
    };
    let pw = if rp == 0 { 0.0 } else { f32::min(pw, (avail - 200.0).max(120.0)) };
    let panel = Rect::from_min_max(pos2(right_strip.min.x - pw, full.min.y), pos2(right_strip.min.x, full.max.y));
    let list = Rect::from_min_max(pos2(full.min.x + tw + 5.0, full.min.y), pos2(if rp == 0 { right_strip.min.x } else { panel.min.x - 5.0 }, full.max.y));
    let p = ui.painter().clone();
    p.rect_filled(whole, 0.0, BG);
    p.rect_filled(Rect::from_min_max(pos2(full.min.x + 2.0, full.min.y), tree.max), 0.0, BG2);
    p.rect_filled(right_strip, 0.0, BG2);
    // Top row of the tree area: collapse chevron and the tile that puts the sub-browser under
    // the browser (skin subBrowse_vertical; lit: blue with a white frame).
    crate::icons_src::chevron(&p, Rect::from_min_size(pos2(full.min.x + 12.0, full.min.y + 5.0), vec2(6.0, 10.0)), OFF);
    let wt = Rect::from_min_size(pos2(full.min.x + 50.0, full.min.y + 2.5), vec2(16.0, 16.0));
    let wresp = ui.interact(wt, ui.id().with("sub_below_tile"), Sense::click());
    if app.prefs.bx.sub_horizontal {
        p.rect_filled(wt.expand(1.0), 0.0, TILE_ON);
        p.rect_stroke(wt, 1.5, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
        crate::icons_src::window_split(&p, wt, WHITE, TILE_ON);
    } else {
        p.rect_stroke(wt, 1.5, Stroke::new(1.0, if wresp.hovered() { hex(0x8C8C8C) } else { OFF }), egui::StrokeKind::Inside);
        crate::icons_src::window_split(&p, wt, WHITE, BG2);
    }
    let wresp = if app.prefs.tooltips { wresp.on_hover_text("Display sub-browser window.") } else { wresp };
    if wresp.clicked() {
        app.prefs.bx.sub_horizontal = !app.prefs.bx.sub_horizontal;
        if app.prefs.bx.sub_horizontal {
            app.prefs.sub_browser = false;
        }
        app.save_prefs();
    }
    p.rect_filled(Rect::from_min_max(pos2(full.min.x, full.min.y + 21.0), pos2(full.min.x + tw + 5.0, full.min.y + 22.0)), 0.0, BG);

    {
        let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(strip));
        sui.set_clip_rect(strip.intersect(ui.clip_rect()));
        icon_strip(app, &mut sui, strip);
    }
    let mut drop_targets: Vec<(Rect, i64)> = vec![];
    // Tree rows span 37 pt to 12 pt before the pane's edge (the Collection pane to 2 pt before
    // it) from 22 pt below the top.
    let tree_inner = if app.side == SideMode::Collection {
        p.rect_filled(Rect::from_min_max(pos2(tree.max.x - 2.0, full.min.y + 22.0), tree.max), 0.0, BG);
        Rect::from_min_max(pos2(full.min.x + 37.0, full.min.y + 22.0), pos2(tree.max.x - 2.0, full.max.y - 4.0))
    } else {
        Rect::from_min_max(pos2(full.min.x + 37.0, full.min.y + 22.0), pos2(tree.max.x - 12.0, full.max.y - 4.0))
    };
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(tree_inner));
    tui.set_clip_rect(Rect::from_min_max(tree_inner.min, pos2(tree.max.x, tree_inner.max.y)).intersect(ui.clip_rect()));
    tui.spacing_mut().item_spacing.y = 0.0;
    tree_pane(app, &mut tui, &mut drop_targets);
    // The divider between the tree and the track list can be dragged.
    {
        let dr = Rect::from_min_max(pos2(tree.max.x, full.min.y), pos2(tree.max.x + 5.0, full.max.y));
        let d = ui.interact(dr, ui.id().with("tree_divider"), Sense::drag());
        if d.hovered() || d.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if d.dragged() {
            app.prefs.bx.tree_w = (tw + d.drag_delta().x).clamp(170.0, (full.width() - 420.0).max(170.0));
        }
        if d.drag_stopped() {
            app.save_prefs();
        }
    }
    // A playlist drag that ends anywhere other than on a tree row is cancelled; while it runs
    // its name follows the pointer.
    if let Some(id) = app.node_drag {
        if ui.input(|i| i.pointer.any_released()) {
            app.node_drag = None;
        } else if let (Some(pp), Some(n)) = (ui.ctx().pointer_latest_pos(), app.node(id)) {
            ui.painter().text(pp + vec2(12.0, 4.0), Align2::LEFT_CENTER, &n.name, rb(11.5), WHITE);
        }
    }
    // Playlist Palette: a 26 pt strip of four slots under the track list.
    let list = if app.prefs.palette_open && track_view {
        let pr = Rect::from_min_max(pos2(list.min.x, list.max.y - 26.0), list.max);
        crate::ui_library_ext::palette_bar(app, ui, pr, &mut drop_targets);
        Rect::from_min_max(list.min, pos2(list.max.x, pr.min.y - 2.0))
    } else {
        list
    };
    // The sub-browser: beside the track list (right strip) or under the browser.
    let sub_rect = if rp == 5 { Some(panel) } else { sub_below };
    if let Some(sr) = sub_rect {
        crate::ui_browser_ext::sub_browser(app, ui, sr, &mut drop_targets);
        // Tracks dragged from the sub-browser can be dropped into the main playlist.
        if app.sub_drag {
            if let Some(pl) = app.is_manual_playlist() {
                drop_targets.push((list, pl));
            }
        }
    }
    let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(list));
    lui.set_clip_rect(list.intersect(ui.clip_rect()));
    list_for_view(app, &mut lui);
    match rp {
        1 => crate::ui_browser_ext::mytag_panel(app, ui, panel),
        2 => crate::ui_browser_ext::related_window(app, ui, panel, 0),
        3 => crate::ui_browser_ext::related_window(app, ui, panel, 1),
        4 => crate::ui_library_ext::info_panel(app, ui, panel),
        _ => {}
    }
    right_icons(app, ui, right_strip);
    let ctx = ui.ctx().clone();
    crate::ui_library_ext::windows(app, &ctx);

    // drag & drop of tracks onto playlists
    if let Some(ids) = app.drag.clone() {
        let ptr = ui.ctx().pointer_latest_pos();
        if let Some(pp) = ptr {
            let label = if ids.len() == 1 { app.track(ids[0]).map(|t| t.display_title()).unwrap_or_default() } else { format!("{} tracks", ids.len()) };
            let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dragl")));
            let rr = Rect::from_min_size(pp + vec2(12.0, 6.0), vec2(8.0 + label.len() as f32 * 6.5, 18.0));
            lp.rect_filled(rr, 3.0, ROW_SEL);
            lp.text(rr.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, label, rb(11.0), WHITE);
            for (r, _id) in &drop_targets {
                if r.contains(pp) {
                    lp.rect_stroke(*r, 2.0, Stroke::new(1.5, BLUE), egui::StrokeKind::Inside);
                }
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(pp) = ptr {
                if let Some((_, id)) = drop_targets.iter().find(|(r, _)| r.contains(pp)) {
                    match *id {
                        // The Tag List row, and the Matching panel (link with its source track).
                        -1 => app.tag_list_add(&ids),
                        -2 => {
                            if let Some(src) = app.related_source() {
                                for t in &ids {
                                    app.matching_set(src, *t, true);
                                }
                            }
                        }
                        pl => app.add_to_playlist(pl, &ids),
                    }
                }
            }
            app.drag = None;
            app.sub_drag = false;
        }
    }
}

/// The list of the view shown: tracks, a device's contents, a folder or an outside library.
pub(crate) fn list_for_view(app: &mut App, lui: &mut Ui) {
    match app.view.clone() {
        View::Device { root, node } => device_list(app, lui, &root, node, None),
        View::DeviceHistory { root, id } => device_list(app, lui, &root, None, Some(id)),
        View::Explorer(dir) => explorer_list(app, lui, &dir),
        View::Source { kind, node } => crate::ui_library_ext::source_list(app, lui, kind, &node),
        _ => track_list(app, lui),
    }
}

type IconFn = fn(&egui::Painter, Rect, Color32);

/// What a source tile opens.
#[derive(Clone, Copy, PartialEq)]
enum Tile {
    Side(SideMode),
    Suggest,
    TagList,
    HotCueBank,
    Off,
}

/// Left icon strip (skins/MediaSelect): 18 pt tiles on a 20 pt pitch, centred at x = 16 pt,
/// the first one centred 34 pt below the browser top. The selected tile has a white border on
/// the selection colour, a 21 pt band behind it and a 2 pt bar at the window edge.
fn icon_strip(app: &mut App, ui: &mut Ui, r: Rect) {
    use crate::icons_src as src;
    let items: [(src::SrcIcon, Tile, &str); 12] = [
        (src::collection, Tile::Side(SideMode::Collection), "Collection"),
        (src::playlist, Tile::Side(SideMode::Playlists), "Playlists"),
        (src::related, Tile::Side(SideMode::Related), "Related Tracks"),
        (src::matching, Tile::Suggest, "Track Suggestion"),
        (src::tag_list, Tile::TagList, "Tag List"),
        (src::hot_cue_bank, Tile::HotCueBank, "Hot Cue Bank List"),
        (src::itunes, Tile::Side(SideMode::Itunes), "iTunes"),
        (src::xml, Tile::Side(SideMode::RekordboxXml), "rekordbox xml"),
        (src::explorer, Tile::Side(SideMode::Explorer), "Explorer"),
        (src::device, Tile::Side(SideMode::Devices), "Devices"),
        (src::history, Tile::Side(SideMode::History), "Histories"),
        (src::recording, Tile::Off, "Recordings (not available in this app)"),
    ];
    // The highlight is bright while the tree side was clicked last (reference 04) and muted
    // while the track list was, or nothing yet (references 01 and 02).
    let hi = if app.ext.tree_focus { TILE_ON } else { ROW_SEL };
    for (i, (draw, tile, name)) in items.iter().filter(|it| app.prefs.tree_item(it.2)).collect::<Vec<_>>().into_iter().enumerate() {
        let c = pos2(r.min.x + 16.0, r.min.y + 34.0 + i as f32 * 20.0);
        let rr = Rect::from_center_size(c, vec2(18.0, 18.0));
        let on = match tile {
            Tile::Side(SideMode::Related) => app.side == SideMode::Related,
            Tile::Side(SideMode::Playlists) => app.side == SideMode::Playlists && app.view != View::TagList,
            Tile::Side(m) => app.side == *m,
            Tile::Suggest => app.side == SideMode::Suggest,
            Tile::TagList => app.view == View::TagList,
            Tile::HotCueBank | Tile::Off => false,
        };
        let resp = ui.interact(rr.expand(1.0), ui.id().with(("side", i)), Sense::click());
        let p = ui.painter();
        let bg = if on {
            p.rect_filled(Rect::from_min_max(pos2(r.min.x + 2.0, rr.min.y), pos2(r.min.x + 4.0, rr.max.y)), 0.0, hi);
            p.rect_filled(Rect::from_min_max(pos2(r.min.x + 6.0, rr.min.y), pos2(r.min.x + 27.0, rr.max.y)), 0.0, hi);
            p.rect_stroke(rr, 1.5, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
            hi
        } else {
            src::tile(p, rr, false, resp.hovered(), *tile != Tile::Off, BG2)
        };
        let col = if *tile == Tile::Off { hex(0x4A4A4A) } else { WHITE };
        draw(p, rr, col, bg);
        let tip = match tile {
            Tile::Suggest => crate::browser_ext::SUGGEST_TIPS[0],
            _ => name,
        };
        let resp = if app.prefs.tooltips { resp.on_hover_text(tip) } else { resp };
        if resp.clicked() {
            match *tile {
                Tile::Off => {}
                Tile::Suggest => {
                    app.side = SideMode::Suggest;
                    if !matches!(app.view, View::Suggest(_)) {
                        app.set_view(View::Suggest(0));
                    }
                }
                Tile::TagList => {
                    app.side = SideMode::Playlists;
                    app.set_view(View::TagList);
                }
                Tile::HotCueBank => app.side = SideMode::Playlists,
                Tile::Side(m) => {
                    app.side = m;
                    match m {
                        SideMode::Collection => app.set_view(View::Collection),
                        SideMode::Devices => {
                            app.scan_devices();
                            app.set_view(View::Device { root: String::new(), node: None });
                        }
                        SideMode::Related => app.set_view(View::Related),
                        SideMode::Explorer => {}
                        SideMode::History => {
                            app.scan_devices();
                            app.set_view(View::History(0));
                        }
                        SideMode::Itunes => app.set_view(View::Source { kind: crate::library_ext::SRC_ITUNES, node: String::new() }),
                        SideMode::RekordboxXml => app.set_view(View::Source { kind: crate::library_ext::SRC_RBXML, node: String::new() }),
                        SideMode::Playlists | SideMode::Suggest => {}
                    }
                }
            }
        }
    }
    // Bottom of the strip: the library sync mark ("Sync rekordbox Library Now"), lit while a
    // sync is running. (rekordbox's link-status mark above it has no counterpart here.)
    let sr = Rect::from_center_size(pos2(r.min.x + 20.0, r.max.y - 20.0), vec2(18.0, 12.0));
    let busy = app.rb_job.is_some();
    let resp = ui.interact(sr.expand(3.0), ui.id().with("strip_sync"), Sense::click());
    src::sync_arrows(ui.painter(), sr, if busy { BLUE } else if resp.hovered() { WHITE } else { hex(0xDCDCDC) });
    let resp = if app.prefs.tooltips { resp.on_hover_text("Sync rekordbox Library Now") } else { resp };
    if resp.clicked() && !busy {
        app.start_rekordbox_sync();
    }
}

/// Right icon strip (references 10-15): My Tag, Related Tracks, Track Suggestion, Information
/// and Sub-browser, 18 pt tiles centred 12 pt into the strip on a 34 pt pitch; the first one
/// 189 pt down a 608 pt browser and 50 pt down the 298 pt one above a sub-browser.
fn right_icons(app: &mut App, ui: &mut Ui, r: Rect) {
    use crate::icons_src as src;
    let items: [(src::SrcIcon, &str); 5] = [
        (src::mytag, "Display My Tag configuration window."),
        (src::related, "Display related track list window."),
        (src::matching, "Display Track Suggestion window."),
        (src::info, "Display information window."),
        (src::sub_browser, "Display sub-browser window."),
    ];
    let y0 = (50.0 + (r.height() - 298.0) * 0.4548).round().max(30.0);
    let now = crate::ui_browser_ext::right_panel(app);
    for (i, (draw, tipt)) in items.iter().enumerate() {
        let rr = Rect::from_center_size(pos2(r.min.x + 12.0, r.min.y + y0 + 34.0 * i as f32), vec2(18.0, 18.0));
        let on = now == i as u8 + 1;
        let resp = ui.interact(rr, ui.id().with(("ricon", i)), Sense::click());
        // Same tiles as the source strip (skins: mytag, relatedTrack, trackInfo, subBrowse).
        // Lit like the source tiles; otherwise a #565656 border on #212121. My Tag is dimmed
        // on the Devices list (reference 05).
        let dim = i == 0 && !on && matches!(app.view, View::Device { .. } | View::DeviceHistory { .. });
        let bg = if on {
            ui.painter().rect_filled(rr.expand(1.0), 0.0, TILE_ON);
            ui.painter().rect_stroke(rr, 1.5, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
            TILE_ON
        } else {
            let fill = hex(0x212121);
            ui.painter().rect_filled(rr, 1.5, fill);
            ui.painter().rect_stroke(rr, 1.5, Stroke::new(1.0, if dim { hex(0x2E2E2E) } else if resp.hovered() { hex(0x8C8C8C) } else { hex(0x565656) }), egui::StrokeKind::Inside);
            fill
        };
        draw(ui.painter(), rr, if dim { hex(0x4A4A4A) } else { WHITE }, bg);
        let resp = if app.prefs.tooltips { resp.on_hover_text(*tipt) } else { resp };
        if resp.clicked() {
            crate::ui_browser_ext::set_right_panel(app, if on { 0 } else { i as u8 + 1 });
        }
    }
}

/// Kind of glyph drawn before a tree row's text.
#[derive(Clone, Copy, PartialEq)]
pub enum TreeIcon {
    None,
    Lock,
    Note,
    Open,
    Closed,
    FolderOpen,
    FolderClosed,
    Playlist,
    Smart,
    Device,
    /// The CUE Analysis Playlist: its name stops before the [Start] button.
    CuePlaylist,
}

/// One tree row, measured in reference 04 (pt from the row's left edge, 37 pt from the window's):
/// rows 15 pt high, 20 pt of indentation per level; a root row has its triangle at 15 and its
/// text at 28; a row with a glyph has it centred at 31 and its text at 43, a folder's triangle
/// 23.5 pt left of its glyph; counts end 30.5 pt from the right; selection #314B67.
pub(crate) fn tree_row(ui: &mut Ui, depth: usize, text: &str, count: Option<usize>, selected: bool, icon: TreeIcon, pitch: f32) -> egui::Response {
    use crate::icons_src as src;
    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), pitch), Sense::click_and_drag());
    let p = ui.painter();
    let bg = if selected { ROW_SEL } else if resp.hovered() { hex(0x262626) } else { BG2 };
    if selected || resp.hovered() {
        p.rect_filled(rr, 0.0, bg);
    }
    let d = depth as f32 * 20.0;
    let cy = rr.center().y;
    let ic = pos2(rr.min.x + 31.0 + d, cy);
    let col = if selected { WHITE } else { TREE_ICON };
    let mut tx = rr.min.x + 43.0 + d;
    match icon {
        TreeIcon::None => tx = rr.min.x + 28.0 + d,
        TreeIcon::Open | TreeIcon::Closed => {
            src::tree_arrow(p, pos2(rr.min.x + 15.0 + d, cy), icon == TreeIcon::Open, WHITE);
            tx = rr.min.x + 28.0 + d;
        }
        TreeIcon::FolderOpen | TreeIcon::FolderClosed => {
            src::tree_arrow(p, pos2(ic.x - 23.5, cy), icon == TreeIcon::FolderOpen, WHITE);
            src::tree_folder(p, ic, col);
        }
        TreeIcon::Lock => src::tree_lock(p, ic, col, bg),
        TreeIcon::Note => src::tree_note(p, ic, col),
        TreeIcon::Playlist => src::tree_list(p, ic, col, bg),
        TreeIcon::Smart => src::tree_smart(p, ic + vec2(2.5, 2.5), col, bg),
        TreeIcon::Device => src::tree_device(p, ic, col, bg),
        TreeIcon::CuePlaylist => src::tree_cue_list(p, ic, col),
    }
    let right = if icon == TreeIcon::CuePlaylist { rr.max.x - 114.0 } else if count.is_some() { rr.max.x - 62.0 } else { rr.max.x - 4.0 };
    let clip = p.with_clip_rect(Rect::from_min_max(pos2(tx, rr.min.y), pos2(right.max(tx + 20.0), rr.max.y)));
    clip.text(pos2(tx, cy), Align2::LEFT_CENTER, text, rb(TREE_FONT), if icon == TreeIcon::Lock && !selected { DIM } else { WHITE });
    if let Some(c) = count {
        p.text(pos2(rr.max.x - 30.5, cy), Align2::RIGHT_CENTER, c.to_string(), rb(TREE_FONT), WHITE);
    }
    resp
}

/// Text size of tree rows.
pub const TREE_FONT: f32 = 10.5;

/// A row of the Collection pane (reference 01): the selection spans 1 pt in from the left to
/// the pane's edge, the triangle is centred 9 pt in and the text starts 20 pt in (+`indent`).
fn collection_row(ui: &mut Ui, text: &str, arrow: Option<bool>, indent: f32, count: Option<usize>, selected: bool, pitch: f32) -> egui::Response {
    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), pitch), Sense::click());
    let p = ui.painter();
    let bar = Rect::from_min_max(pos2(rr.min.x + 1.0, rr.min.y), pos2(rr.max.x - 1.0, rr.max.y));
    if selected {
        p.rect_filled(bar, 0.0, ROW_SEL);
    } else if resp.hovered() {
        p.rect_filled(bar, 0.0, hex(0x262626));
    }
    if let Some(open) = arrow {
        crate::icons_src::tree_arrow(p, pos2(rr.min.x + 9.0 + indent, rr.center().y), open, WHITE);
    }
    let tx = rr.min.x + 20.0 + indent;
    let right = if count.is_some() { rr.max.x - 40.0 } else { rr.max.x - 4.0 };
    p.with_clip_rect(Rect::from_min_max(pos2(tx, rr.min.y), pos2(right, rr.max.y))).text(pos2(tx, rr.center().y), Align2::LEFT_CENTER, text, rb(TREE_FONT), WHITE);
    if let Some(c) = count {
        p.text(pos2(rr.max.x - 10.0, rr.center().y), Align2::RIGHT_CENTER, c.to_string(), rb(TREE_FONT), WHITE);
    }
    resp
}

pub(crate) fn tree_pane(app: &mut App, ui: &mut Ui, drop: &mut Vec<(Rect, i64)>) {
    match app.side {
        SideMode::Collection => {
            // Reference 01: "Collection" heading in a 12 pt band, a 1 pt line under it, "All"
            // 17 pt high, then the categories on an 18 pt pitch.
            let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
            ui.painter().text(pos2(hr.min.x + 5.0, hr.center().y), Align2::LEFT_CENTER, "Collection", rb(TREE_FONT), WHITE);
            let (lr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(Rect::from_min_max(lr.min, pos2(lr.max.x + 7.0, lr.max.y)), 0.0, BG);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let sel = app.view == View::Collection;
                let all = collection_row(ui, "All", None, 0.0, None, sel, 17.0);
                if all.clicked() {
                    app.set_view(View::Collection);
                }
                all.context_menu(|ui| crate::ui_library_ext::collection_menu(app, ui));
                for (fi, f) in ["Date Added", "Genre", "Artist", "Album"].iter().enumerate() {
                    let key = egui::Id::new(("cat_open", fi));
                    let open = ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
                    let r = collection_row(ui, f, Some(open), 0.0, None, false, 18.0);
                    if r.clicked() {
                        ui.ctx().data_mut(|d| d.insert_temp(key, !open));
                    }
                    if open {
                        let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
                        for t in &app.tracks {
                            *counts.entry(App::category_value(t, fi as u8)).or_default() += 1;
                        }
                        let mut items: Vec<(String, usize)> = counts.into_iter().collect();
                        if fi == 0 {
                            items.reverse(); // newest first
                        }
                        for (v, n) in items {
                            let sel = app.view == View::Category { field: fi as u8, value: v.clone() };
                            let label = if v.is_empty() { "(none)".to_string() } else { v.clone() };
                            if collection_row(ui, &label, None, 14.0, Some(n), sel, 16.0).clicked() {
                                app.set_view(View::Category { field: fi as u8, value: v });
                            }
                        }
                    }
                }
            });
        }
        SideMode::Playlists => {
            // Reference 04: a 22 x 23 pt search tile, then the Tree View / Column View toggle
            // (100 pt and the rest, 23 pt high), a 1 pt line, and the tree's rows.
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            let st = Rect::from_min_max(pos2(row.min.x + 0.5, row.min.y), pos2(row.min.x + 22.5, row.min.y + 23.0));
            // The search tile opens a field that narrows the tree to matching names.
            // Preferences > View > Layout: "Playlist search filter" (off: no tile, no field).
            let filter_on = app.prefs.playlist_search_filter;
            if !filter_on && app.ext.tree_search_open {
                app.ext.tree_search_open = false;
                app.ext.tree_search.clear();
            }
            let sresp = ui.interact(st, ui.id().with("tree_search_tile"), if filter_on { Sense::click() } else { Sense::hover() });
            if !filter_on {
            } else if app.ext.tree_search_open {
                ui.painter().rect_filled(st, 1.5, TILE_ON);
                ui.painter().rect_stroke(st, 1.5, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
            } else {
                ui.painter().rect_stroke(st, 1.5, Stroke::new(1.0, if sresp.hovered() { hex(0x8C8C8C) } else { OFF }), egui::StrokeKind::Inside);
            }
            if filter_on {
                crate::icons_src::playlist_search(ui.painter(), st, if app.ext.tree_search_open { WHITE } else { hex(0xC7C7C7) }, if app.ext.tree_search_open { TILE_ON } else { BG2 });
            }
            if sresp.clicked() {
                app.ext.tree_search_open = !app.ext.tree_search_open;
                if app.ext.tree_search_open {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("tree_search_focus"), true));
                } else {
                    app.ext.tree_search.clear();
                }
            }
            // Tree View / Column View: two 100 pt tabs (blue with white text when chosen,
            // #323232 with grey text otherwise).
            let cv_on = app.prefs.bx.column_view;
            for (i, name) in ["Tree View", "Column View"].iter().enumerate() {
                let tr = Rect::from_min_max(pos2(row.min.x + 26.0 + 100.0 * i as f32, row.min.y), pos2(row.min.x + 126.0 + 100.0 * i as f32, row.min.y + 23.0));
                let on = cv_on == (i == 1);
                let resp = ui.interact(tr, ui.id().with(("tree_tab", i)), Sense::click());
                let wide = ui.painter().with_clip_rect(Rect::from_min_max(row.min, pos2(row.max.x + 12.0, row.max.y)));
                wide.rect_filled(tr, 0.0, if on { TILE_ON } else if resp.hovered() { hex(0x3A3A3A) } else { hex(0x323232) });
                wide.text(tr.center(), Align2::CENTER_CENTER, *name, rb(12.0), if on { WHITE } else { DIM });
                if resp.clicked() && !on {
                    app.prefs.bx.column_view = i == 1;
                    app.save_prefs();
                }
            }
            ui.painter().with_clip_rect(Rect::from_min_max(row.min, pos2(row.max.x + 12.0, row.max.y))).rect_filled(Rect::from_min_max(pos2(row.min.x - 37.0, row.min.y + 23.0), pos2(row.max.x + 12.0, row.max.y)), 0.0, BG);
            if app.prefs.bx.column_view {
                crate::ui_browser_ext::column_view(app, ui, drop);
                return;
            }
            if app.ext.tree_search_open {
                crate::ui_library_ext::tree_search_field(app, ui);
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let all_sel = app.view == View::Collection;
                // Preferences > View > Layout: "Display All Tracks in the Playlists".
                if app.prefs.all_tracks_in_playlists {
                    let all = tree_row(ui, 0, "All Tracks", Some(app.tracks.len()), all_sel, TreeIcon::Note, 15.0);
                    if all.clicked() {
                        app.set_view(View::Collection);
                    }
                    all.context_menu(|ui| crate::ui_library_ext::collection_menu(app, ui));
                }
                crate::ui_library_ext::tree_top_rows(app, ui, drop);
                let root = tree_row(ui, 0, "Playlists", None, false, TreeIcon::Open, 15.0);
                root.context_menu(|ui| {
                    if ui.button("Create New Playlist").clicked() {
                        app.create_node(0, NodeKind::Playlist);
                        ui.close_menu();
                    }
                    if ui.button("Create New Intelligent Playlist").clicked() {
                        app.create_node(0, NodeKind::Smart);
                        ui.close_menu();
                    }
                    if ui.button("Create New Folder").clicked() {
                        app.create_node(0, NodeKind::Folder);
                        ui.close_menu();
                    }
                    crate::ui_library_ext::root_menu_extra(app, ui);
                });
                node_rows(app, ui, 0, 1, drop);
                crate::export::hotcue_bank::hot_cue_bank_tree(app, ui);
            });
        }
        SideMode::Devices => devices_tree(app, ui),
        SideMode::History => crate::ui_library_ext::history_tree(app, ui),
        SideMode::Suggest => crate::ui_browser_ext::suggest_tree(app, ui),
        SideMode::Related => {
            // Tracks dropped on the Matching panel are linked with its source track.
            if app.view == View::Matching {
                drop.push((ui.max_rect(), -2));
            }
            crate::ui_library_ext::related_panel(app, ui)
        }
        SideMode::Itunes => crate::ui_library_ext::source_tree(app, ui, crate::library_ext::SRC_ITUNES),
        SideMode::RekordboxXml => crate::ui_library_ext::source_tree(app, ui, crate::library_ext::SRC_RBXML),
        SideMode::Explorer => explorer_tree(app, ui),
    }
}

fn node_rows(app: &mut App, ui: &mut Ui, parent: i64, depth: usize, drop: &mut Vec<(Rect, i64)>) {
    if depth == 1 && app.node_drag.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.node_drag = None;
    }
    let show_cue_pl = app.prefs.tree_item("CUE Analysis Playlist");
    let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == parent && (show_cue_pl || !(n.kind == NodeKind::Playlist && n.name == crate::browser_ext::CUE_PLAYLIST_NAME))).cloned().collect();
    // Tree search: only names that match (and the folders leading to them) are listed.
    let query = if app.ext.tree_search_open { crate::smart::fold(app.ext.tree_search.trim()) } else { String::new() };
    for n in kids {
        if !query.is_empty() && !crate::ui_library_ext::node_matches(app, n.id, &query, 0) {
            continue;
        }
        let sel = app.view == View::Playlist(n.id);
        let open = app.expanded.contains(&n.id) || !query.is_empty();
        let (icon, count) = match n.kind {
            NodeKind::Folder => (if open { TreeIcon::FolderOpen } else { TreeIcon::FolderClosed }, None),
            NodeKind::Smart => (TreeIcon::Smart, n.smart.as_ref().map(|r| app.smart_tracks(r).len())),
            NodeKind::Playlist => (TreeIcon::Playlist, Some(app.node_counts.get(&n.id).copied().unwrap_or(0))),
        };
        if let Some((rid, _)) = &app.dialogs.rename {
            if *rid == n.id {
                let mut name = app.dialogs.rename.as_ref().unwrap().1.clone();
                let resp = ui.horizontal(|ui| {
                    ui.add_space(6.0 + depth as f32 * 16.0);
                    ui.add(egui::TextEdit::singleline(&mut name).desired_width(150.0))
                });
                let resp = resp.inner;
                // Whether the field just gave up focus must be read before focus is asked for again.
                let gave_up = resp.lost_focus();
                if !gave_up {
                    resp.request_focus();
                }
                app.dialogs.rename.as_mut().unwrap().1 = name.clone();
                if gave_up {
                    if !name.trim().is_empty() {
                        let _ = app.db.rename_node(n.id, name.trim());
                    }
                    app.dialogs.rename = None;
                    app.reload_playlists();
                }
                continue;
            }
        }
        // Preferences > View > Layout: "Display the number of tracks in a playlist on the Tree View".
        let is_cue = n.kind == NodeKind::Playlist && app.cue_playlist() == Some(n.id);
        let resp = tree_row(ui, depth, &n.name, count.filter(|_| app.prefs.playlist_track_counts), sel, if is_cue { TreeIcon::CuePlaylist } else { icon }, 15.0);
        if n.kind == NodeKind::Playlist {
            drop.push((resp.rect, n.id));
        }
        crate::ui_browser_ext::node_row_extras(app, ui, &n, resp.rect, sel, is_cue);
        // Dragging playlists and folders: top third = before, bottom third = after, middle of a
        // folder = into it.
        if resp.drag_started() && app.drag.is_none() {
            app.node_drag = Some(n.id);
        }
        if let Some(dragged) = app.node_drag {
            if dragged != n.id {
                if let Some(pp) = ui.ctx().pointer_latest_pos().filter(|pp| resp.rect.contains(*pp)) {
                    let h = resp.rect.height();
                    let rel = (pp.y - resp.rect.min.y) / h;
                    let zone: i8 = if n.kind == NodeKind::Folder && (0.3..0.7).contains(&rel) { 0 } else if rel < 0.5 { -1 } else { 1 };
                    let painter = ui.painter();
                    match zone {
                        0 => {
                            painter.rect_stroke(resp.rect.shrink(1.0), 2.0, Stroke::new(1.5, BLUE), egui::StrokeKind::Inside);
                        }
                        z => {
                            let y = if z < 0 { resp.rect.min.y } else { resp.rect.max.y };
                            painter.line_segment([pos2(resp.rect.min.x + depth as f32 * 15.0, y), pos2(resp.rect.max.x, y)], Stroke::new(2.0, BLUE));
                        }
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        app.node_drag = None;
                        app.drop_node(dragged, n.id, zone);
                        return;
                    }
                }
            }
        }
        if resp.clicked() {
            if n.kind == NodeKind::Folder {
                if open {
                    app.expanded.remove(&n.id);
                } else {
                    app.expanded.insert(n.id);
                }
            }
            app.set_view(View::Playlist(n.id));
        }
        if resp.double_clicked() {
            if n.kind == NodeKind::Smart {
                app.dialogs.smart_edit = Some((Some(n.id), n.parent, n.name.clone(), n.smart.clone().unwrap_or_default()));
            } else {
                app.dialogs.rename = Some((n.id, n.name.clone()));
            }
        }
        let nn = n.clone();
        resp.context_menu(|ui| {
            let parent_for_new = if nn.kind == NodeKind::Folder { nn.id } else { nn.parent };
            if ui.button("Create New Playlist").clicked() {
                app.create_node(parent_for_new, NodeKind::Playlist);
                ui.close_menu();
            }
            if ui.button("Create New Intelligent Playlist").clicked() {
                app.create_node(parent_for_new, NodeKind::Smart);
                ui.close_menu();
            }
            if ui.button("Create New Folder").clicked() {
                app.create_node(parent_for_new, NodeKind::Folder);
                ui.close_menu();
            }
            ui.separator();
            if nn.kind == NodeKind::Smart && ui.button("Edit the Intelligent Playlist").clicked() {
                app.dialogs.smart_edit = Some((Some(nn.id), nn.parent, nn.name.clone(), nn.smart.clone().unwrap_or_default()));
                ui.close_menu();
            }
            if nn.kind == NodeKind::Smart && ui.button("Convert to a normal playlist").clicked() {
                let tracks = nn.smart.as_ref().map(|r| app.smart_tracks(r)).unwrap_or_default();
                let id = app.db.create_node(nn.parent, &format!("{} (copy)", nn.name), NodeKind::Playlist, None).unwrap_or(0);
                let _ = app.db.set_playlist_tracks(id, &tracks);
                app.reload_playlists();
                ui.close_menu();
            }
            let what = if nn.kind == NodeKind::Folder { "Folder" } else { "Playlist" };
            if ui.button(format!("Rename {what}")).clicked() {
                app.dialogs.rename = Some((nn.id, nn.name.clone()));
                ui.close_menu();
            }
            if nn.kind == NodeKind::Playlist && ui.button("Sort Items by Track Title").clicked() {
                let mut v = app.db.playlist_tracks(nn.id).unwrap_or_default();
                v.sort_by_key(|t| app.track(*t).map(|t| crate::smart::fold(&t.display_title())).unwrap_or_default());
                let _ = app.db.set_playlist_tracks(nn.id, &v);
                app.reload_playlists();
                ui.close_menu();
            }
            if ui.button(format!("Export {what}")).clicked() {
                open_export(app, vec![nn.id], vec![]);
                ui.close_menu();
            }
            crate::ui_library_ext::node_menu_extra(app, ui, &nn);
            if ui.button("Move to top level").clicked() {
                let _ = app.db.move_node(nn.id, 0, 0);
                app.reload_playlists();
                ui.close_menu();
            }
            ui.menu_button("Move into folder", |ui| {
                let folders: Vec<PlaylistNode> = app.nodes.iter().filter(|f| f.kind == NodeKind::Folder && f.id != nn.id).cloned().collect();
                for f in folders {
                    if ui.button(&f.name).clicked() {
                        let _ = app.db.move_node(nn.id, f.id, 0);
                        app.reload_playlists();
                        ui.close_menu();
                    }
                }
            });
            ui.separator();
            if ui.button(format!("Delete {what}")).clicked() {
                app.dialogs.confirm_delete_node = Some((nn.id, nn.name.clone(), nn.kind == NodeKind::Folder));
                ui.close_menu();
            }
        });
        if n.kind == NodeKind::Folder && open {
            node_rows(app, ui, n.id, depth + 1, drop);
        }
    }
}

fn devices_tree(app: &mut App, ui: &mut Ui) {
    if app.devices_scanned.map(|t| t.elapsed().as_secs() > 4).unwrap_or(true) && app.export_job.is_none() {
        app.scan_devices();
    }
    let on_root = matches!(&app.view, View::Device { root, .. } if root.is_empty());
    let dr = tree_row(ui, 0, "Devices", None, on_root, TreeIcon::Open, 15.0);
    if dr.clicked() {
        app.set_view(View::Device { root: String::new(), node: None });
    }
    // Reference 05 shows only the "Devices" row; Recordbox's extras live in its context menu.
    let dr = if app.prefs.tooltips && app.devices.is_empty() { dr.on_hover_text("Connect a USB drive or SD card (FAT32, exFAT or HFS+). Right-click for more.") } else { dr };
    dr.context_menu(|ui| {
        if ui.button("Add a folder as a device…").on_hover_text("Export into any folder, for example to prepare a drive image or test an export.").clicked() {
            ui.close_menu();
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                let s = d.to_string_lossy().to_string();
                if !app.prefs.devices_extra.contains(&s) {
                    app.prefs.devices_extra.push(s);
                    app.save_prefs();
                }
                app.scan_devices();
            }
        }
        if ui.button("Export to device…").clicked() {
            ui.close_menu();
            open_export(app, vec![], vec![]);
        }
    });
    let devs: Vec<(String, String, String, Option<Vec<crate::export::usb::DevNode>>, Option<String>, usize)> = app
        .devices
        .iter()
        .map(|d| (d.root.clone(), d.name.clone(), d.fs.clone(), d.lib.as_ref().map(|l| l.nodes.clone()), d.error.clone(), d.lib.as_ref().map(|l| l.tracks.len()).unwrap_or(0)))
        .collect();
    for (root, name, fs, nodes, err, ntracks) in devs {
        let sel = app.view == View::Device { root: root.clone(), node: None };
        let resp = tree_row(ui, 1, &format!("{name}  ({})", App::fs_label(&fs)), if fs == "folder" { Some(ntracks) } else { None }, sel, TreeIcon::Device, 15.0);
        // Eject button at the right end of a removable device's row (as in the original).
        let mut ejected = false;
        if fs != "folder" {
            let er = Rect::from_center_size(pos2(resp.rect.max.x - 12.0, resp.rect.center().y), vec2(14.0, 14.0));
            let eb = ui.interact(er, ui.id().with(("eject", &root)), Sense::click());
            let col = if eb.hovered() { WHITE } else { DIM2 };
            let p = ui.painter();
            let c = er.center();
            p.add(egui::Shape::convex_polygon(vec![pos2(c.x, c.y - 5.0), pos2(c.x + 5.0, c.y + 0.5), pos2(c.x - 5.0, c.y + 0.5)], col, Stroke::NONE));
            p.rect_filled(Rect::from_min_max(pos2(c.x - 5.0, c.y + 2.5), pos2(c.x + 5.0, c.y + 4.5)), 0.0, col);
            let eb = eb.on_hover_text("Disconnect USB Device");
            if eb.clicked() {
                ejected = true;
                if app.export_job.is_some() {
                    app.toast_err("Wait for the export to finish before disconnecting the device.");
                } else {
                    match std::process::Command::new("/usr/sbin/diskutil").arg("eject").arg(&root).output() {
                        Ok(o) if o.status.success() => {
                            app.toast(format!("{name} can now be removed safely."));
                            app.set_view(View::Device { root: String::new(), node: None });
                            app.scan_devices();
                        }
                        Ok(o) => app.toast_err(format!("Could not disconnect {name}: {}", String::from_utf8_lossy(&o.stderr).trim())),
                        Err(e) => app.toast_err(format!("Could not disconnect {name}: {e}")),
                    }
                }
            }
        }
        if resp.clicked() && !ejected {
            app.set_view(View::Device { root: root.clone(), node: None });
        }
        // Background Color of the device (Property window), as a marker on its row.
        if let Some(bg) = app.device_ui.bg.get(&root).copied().filter(|b| (1..=8).contains(b)) {
            ui.painter().rect_filled(Rect::from_min_size(pos2(resp.rect.min.x + 2.0, resp.rect.min.y + 3.0), vec2(3.0, resp.rect.height() - 6.0)), 1.0, label_color(bg));
        }
        // Tracks or a playlist dragged from the library and dropped on the device open the
        // export for them.
        if app.drag.is_some() || app.node_drag.is_some() {
            if let Some(pp) = ui.ctx().pointer_latest_pos().filter(|p| resp.rect.contains(*p)) {
                ui.painter().rect_stroke(resp.rect, 2.0, Stroke::new(1.5, BLUE), egui::StrokeKind::Inside);
                if ui.input(|i| i.pointer.any_released()) {
                    let _ = pp;
                    let (lists, tracks) = (app.node_drag.take().into_iter().collect::<Vec<i64>>(), app.drag.take().unwrap_or_default());
                    open_export(app, lists, tracks);
                    app.dialogs.export_root = Some(root.clone());
                }
            }
        }
        let r2 = root.clone();
        resp.context_menu(|ui| {
            if ui.button("Export to this device…").clicked() {
                app.dialogs.export_root = Some(r2.clone());
                open_export(app, vec![], vec![]);
                app.dialogs.export_root = Some(r2.clone());
                ui.close_menu();
            }
            if ui.button("Update Playlists on This Device").on_hover_text("Export again the playlists that are both on this device and in your library (same folder and name), so the device has your latest edits.").clicked() {
                app.update_device_playlists(&r2);
                ui.close_menu();
            }
            crate::device_ext::device_row_menu(app, ui, &r2);
            if ui.button("Clean Up Unused Files…").on_hover_text("Music, analysis and artwork files on the device that no track in its library uses.").clicked() {
                let dev = app.devices.iter().find(|d| d.root == r2).map(|d| (d.name.clone(), d.lib.clone()));
                match dev {
                    Some((name, _)) if crate::export::usb::has_onelibrary(std::path::Path::new(&r2)) => {
                        app.toast_err(format!("{name} also has rekordbox's OneLibrary database, which recordbox can't read yet, so it can't tell which files are unused."));
                    }
                    Some((name, Some(lib))) => {
                        let files = crate::export::usb::unused_files(std::path::Path::new(&r2), &lib);
                        if files.is_empty() {
                            app.toast(format!("{name} has no unused files."));
                        } else {
                            app.dialogs.device_cleanup = Some((r2.clone(), name, files));
                        }
                    }
                    _ => app.toast_err("This device has no library to compare with."),
                }
                ui.close_menu();
            }
            if ui.button("Import Cue Changes from Players…").on_hover_text("Hot cues and memory cues set or moved on a DJ player since the last export from recordbox.").clicked() {
                app.check_player_cue_edits(&r2, true);
                ui.close_menu();
            }
            if ui.button("Show in Finder").clicked() {
                let _ = std::process::Command::new("/usr/bin/open").arg(&r2).spawn();
                ui.close_menu();
            }
            if fs != "folder" && ui.button("Disconnect USB Device").clicked() {
                if app.export_job.is_some() {
                    app.toast_err("Wait for the export to finish before disconnecting the device.");
                } else {
                    match std::process::Command::new("/usr/sbin/diskutil").arg("eject").arg(&r2).output() {
                        Ok(o) if o.status.success() => {
                            app.toast("The device can now be removed safely.");
                            app.set_view(View::Device { root: String::new(), node: None });
                            app.scan_devices();
                        }
                        Ok(o) => app.toast_err(format!("Could not disconnect the device: {}", String::from_utf8_lossy(&o.stderr).trim())),
                        Err(e) => app.toast_err(format!("Could not disconnect the device: {e}")),
                    }
                }
                ui.close_menu();
            }
            if fs == "folder" && ui.button("Remove USB Device").on_hover_text("Take this folder off the device list. Nothing in the folder is deleted.").clicked() {
                app.prefs.devices_extra.retain(|d| d != &r2);
                app.save_prefs();
                app.scan_devices();
                ui.close_menu();
            }
        });
        if let Some(e) = err {
            ui.label(RichText::new(e).size(10.5).color(CAUTION_TX));
        }
        if fs != "folder" && crate::export::usb::has_onelibrary(std::path::Path::new(&root)) {
            ui.label(RichText::new("Has a OneLibrary database that recordbox does not update yet.").size(10.5).color(DIM)).on_hover_text("Written by rekordbox 7.2.11 or later. Players that read OneLibrary will not show changes made by recordbox.");
        }
        if let Some(nodes) = nodes {
            fn rows(app: &mut App, ui: &mut Ui, root: &str, nodes: &[crate::export::usb::DevNode], parent: u32, depth: usize) {
                let mut kids: Vec<&crate::export::usb::DevNode> = nodes.iter().filter(|n| n.parent == parent).collect();
                kids.sort_by_key(|n| n.seq);
                for n in kids {
                    let sel = app.view == View::Device { root: root.to_string(), node: Some(n.id) };
                    let resp = tree_row(ui, depth + 1, &n.name, if n.folder { None } else { Some(n.entries.len()) }, sel, if n.folder { TreeIcon::FolderOpen } else { TreeIcon::Playlist }, 15.0);
                    if resp.clicked() {
                        app.set_view(View::Device { root: root.to_string(), node: Some(n.id) });
                    }
                    resp.context_menu(|ui| crate::device_ext::device_node_menu(app, ui, root, n));
                    if n.folder {
                        rows(app, ui, root, nodes, n.id, depth + 1);
                    }
                }
            }
            rows(app, ui, &root, &nodes, 0, 1);
        }
    }
}

const AUDIO_EXT: [&str; 10] = ["mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "alac", "mp4", "ogg"];

fn is_audio(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| AUDIO_EXT.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

/// Explorer: folders on this computer (Music, home, mounted drives), opened one level at a time.
fn explorer_tree(app: &mut App, ui: &mut Ui) {
    tree_row(ui, 0, "Explorer", None, false, TreeIcon::Open, 15.0);
    let mut roots: Vec<(String, std::path::PathBuf)> = vec![];
    if let Some(m) = dirs::audio_dir() {
        roots.push(("Music".into(), m));
    }
    if let Some(h) = dirs::home_dir() {
        roots.push((h.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Home".into()), h));
    }
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        let mut v: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        v.sort();
        for p in v {
            roots.push((p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), p));
        }
    }
    egui::ScrollArea::vertical().id_salt("explorer_tree").auto_shrink([false, false]).show(ui, |ui| {
        for (name, path) in roots {
            explorer_node(app, ui, &name, &path, 1);
        }
    });
}

fn explorer_node(app: &mut App, ui: &mut Ui, name: &str, path: &std::path::Path, depth: usize) {
    let key = egui::Id::new(("exp_open", path.to_string_lossy().to_string()));
    let open = ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
    let ps = path.to_string_lossy().to_string();
    let sel = app.view == View::Explorer(ps.clone());
    let r = tree_row(ui, depth, name, None, sel, if open { TreeIcon::FolderOpen } else { TreeIcon::FolderClosed }, 15.0);
    if r.clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(key, !open || !sel));
        app.set_view(View::Explorer(ps));
    }
    if open && depth < 14 {
        // Subfolder listings are cached for five seconds instead of read every frame.
        let ck = egui::Id::new(("exp_dirs", path.to_string_lossy().to_string()));
        let now = ui.input(|i| i.time);
        let cached: Option<(f64, Vec<std::path::PathBuf>)> = ui.ctx().data(|d| d.get_temp(ck));
        let dirs_ = match cached {
            Some((t, v)) if now - t < 5.0 => v,
            _ => {
                let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(path)
                    .map(|rd| rd.flatten().filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).map(|e| e.path()).filter(|p| !p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(true)).collect())
                    .unwrap_or_default();
                v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
                ui.ctx().data_mut(|d| d.insert_temp(ck, (now, v.clone())));
                v
            }
        };
        {
            for d in dirs_.iter().take(500) {
                let n = d.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                explorer_node(app, ui, &n, d, depth + 1);
            }
        }
    }
}

/// Files in an Explorer folder. Double-click loads a file (importing it first when it is not
/// in the collection yet); "Import" adds the selection or the whole folder.
fn explorer_list(app: &mut App, ui: &mut Ui, dir: &str) {
    let r = ui.max_rect();
    // The folder listing is cached for two seconds rather than read on every frame.
    let cache_key = egui::Id::new(("explorer_files", dir.to_string()));
    let cached: Option<(f64, Vec<(std::path::PathBuf, u64)>)> = ui.ctx().data(|d| d.get_temp(cache_key));
    let now = ui.input(|i| i.time);
    let files_sz: Vec<(std::path::PathBuf, u64)> = match cached {
        Some((t, v)) if now - t < 2.0 => v,
        _ => {
            let mut v: Vec<(std::path::PathBuf, u64)> = std::fs::read_dir(dir)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_audio(p)).map(|p| { let n = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0); (p, n) }).collect())
                .unwrap_or_default();
            v.sort_by_key(|p| p.0.file_name().map(|n| n.to_string_lossy().to_lowercase()));
            ui.ctx().data_mut(|d| d.insert_temp(cache_key, (now, v.clone())));
            v
        }
    };
    let files: Vec<std::path::PathBuf> = files_sz.iter().map(|x| x.0.clone()).collect();
    let in_collection: HashMap<&str, i64> = app.tracks.iter().map(|t| (t.path.as_str(), t.id)).collect();
    let in_collection: HashMap<String, i64> = files.iter().filter_map(|f| { let s = f.to_string_lossy().to_string(); in_collection.get(s.as_str()).map(|id| (s, *id)) }).collect();
    let bar = Rect::from_min_size(r.min, vec2(r.width(), 24.0));
    ui.painter().rect_filled(bar, 0.0, BG2);
    ui.painter().text(pos2(bar.min.x + 12.0, bar.center().y), Align2::LEFT_CENTER, format!("{} ({} files)", dir, files.len()), rb(12.0), WHITE);
    let ib = Rect::from_min_max(pos2(bar.max.x - 150.0, bar.min.y + 2.0), pos2(bar.max.x - 8.0, bar.max.y - 2.0));
    if !files.is_empty() && ui.put(ib, egui::Button::new(RichText::new("Import this folder").size(11.5))).clicked() {
        let pl = app.is_manual_playlist();
        app.import_paths(files.clone(), pl);
    }
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(r.min.x, r.min.y + 26.0), r.max)));
    let cols = [("Name", 420.0), ("Type", 60.0), ("Size", 80.0), ("In Collection", 100.0)];
    let hdr = body.allocate_exact_size(vec2(body.available_width(), 14.0), Sense::hover()).0;
    body.painter().rect_filled(hdr, 0.0, BG2);
    let mut x = hdr.min.x + 8.0;
    for (c, w) in cols {
        body.painter().text(pos2(x, hdr.center().y), Align2::LEFT_CENTER, c, rb(11.0), WHITE);
        x += w;
    }
    if files.is_empty() {
        body.add_space(20.0);
        body.label(RichText::new("No audio files in this folder.").color(DIM));
        return;
    }
    let sel_key = egui::Id::new("explorer_sel");
    let mut sel: Option<String> = body.ctx().data(|d| d.get_temp(sel_key));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(&mut body, ROW_H, files.len(), |ui, range| {
        for i in range {
            let f = &files[i];
            let fs = f.to_string_lossy().to_string();
            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            let in_col = in_collection.get(&fs).copied();
            if sel.as_deref() == Some(fs.as_str()) {
                ui.painter().rect_filled(rr, 0.0, ROW_SEL);
            } else if i % 2 == 1 {
                ui.painter().rect_filled(rr, 0.0, hex(0x0E0E0E));
            }
            let size = files_sz[i].1;
            let vals = [
                f.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                f.extension().map(|s| s.to_string_lossy().to_uppercase()).unwrap_or_default(),
                format!("{:.1} MB", size as f64 / 1e6),
                if in_col.is_some() { "✓".into() } else { String::new() },
            ];
            let mut x = rr.min.x + 8.0;
            for (v, (_, w)) in vals.iter().zip(cols) {
                ui.painter().with_clip_rect(Rect::from_min_max(pos2(x, rr.min.y), pos2(x + w - 6.0, rr.max.y))).text(pos2(x, rr.center().y), Align2::LEFT_CENTER, v, rb(11.5), if in_col.is_some() { WHITE } else { TEXT });
                x += w;
            }
            if resp.clicked() {
                sel = Some(fs.clone());
            }
            if resp.double_clicked() {
                match in_col {
                    Some(id) => app.load_track(id),
                    None => {
                        app.load_after_import = Some(fs.clone());
                        app.import_paths(vec![f.clone()], None);
                    }
                }
            }
            resp.context_menu(|ui| {
                if in_col.is_none() && ui.button("Import to Collection").clicked() {
                    app.import_paths(vec![f.clone()], None);
                    ui.close_menu();
                }
                if ui.button("Show in Finder").clicked() {
                    let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(f).spawn();
                    ui.close_menu();
                }
            });
        }
    });
    body.ctx().data_mut(|d| d.insert_temp(sel_key, sel));
}

fn device_list(app: &mut App, ui: &mut Ui, root: &str, node: Option<u32>, history: Option<u32>) {
    if root.is_empty() {
        // The "Devices" / "Histories" item itself: an empty list, as in the original.
        // Reference 05: the usual title bar (title, divider, the view-mode icons greyed out).
        let r = ui.max_rect();
        // The bar is the first 26 pt; the four tiles keep their places with #2E2E2E borders
        // (the lit one a dim blue) and #616161 glyphs; the column-menu tile stays in the right
        // strip; an idle horizontal scroll bar lies along the bottom.
        let bar = Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 26.0));
        let p = ui.painter().clone();
        p.rect_filled(bar, 0.0, BG2);
        p.text(pos2(bar.min.x + 10.0, bar.center().y), Align2::LEFT_CENTER, if history.is_some() { "Histories" } else { "Devices" }, rb(TITLE_FONT), WHITE);
        p.rect_filled(Rect::from_min_max(pos2(bar.min.x + 340.0, bar.min.y), pos2(bar.min.x + 341.0, bar.max.y)), 0.0, BG);
        let with_art = app.prefs.columns.contains(&Col::Artwork);
        let icons: [(f32, IconFn, bool); 4] = [
            (396.0, crate::icons_src::bullet_rows4, with_art && !app.prefs.artwork_full),
            (358.0, crate::icons_src::bullet_rows, with_art && app.prefs.artwork_full),
            (299.0, crate::icons_src::columns3, app.prefs.column_browser),
            (261.0, crate::icons_src::side_pane, app.prefs.sub_browser),
        ];
        for (dx, draw, on) in icons {
            let vr = Rect::from_center_size(pos2(bar.max.x - dx, bar.center().y), vec2(18.0, 18.0));
            p.rect_stroke(vr, 1.5, Stroke::new(1.0, if on { hex(0x1B385B) } else { hex(0x2E2E2E) }), egui::StrokeKind::Inside);
            draw(&p, vr, hex(0x616161));
        }
        let wide = p.with_clip_rect(ui.ctx().screen_rect());
        let cmr = Rect::from_center_size(pos2(ui.ctx().screen_rect().max.x - 12.5, bar.min.y + 14.0), vec2(18.0, 18.0));
        wide.rect_filled(cmr, 2.0, BTN);
        crate::icons_src::bullet_rows(&wide, cmr, WHITE);
        p.rect_filled(Rect::from_min_max(pos2(r.min.x, r.max.y - 10.0), pos2(r.min.x + 1121.0_f32.min(r.width() - 113.0), r.max.y - 2.0)), 0.0, SCROLL);
        return;
    }
    let Some(dev) = app.devices.iter().find(|d| d.root == root) else {
        ui.label("The device is no longer connected.");
        return;
    };
    let title = match (node, history, &dev.lib) {
        (_, Some(h), Some(l)) => l.histories.iter().find(|x| x.id == h).map(|x| x.name.clone()).unwrap_or_default(),
        (Some(n), _, Some(l)) => l.nodes.iter().find(|x| x.id == n).map(|x| x.name.clone()).unwrap_or_default(),
        _ => format!("{} — All Tracks", dev.name),
    };
    let tracks: Vec<crate::export::usb::DevTrack> = match &dev.lib {
        None => vec![],
        Some(l) => match (node, history) {
            (_, Some(h)) => l.histories.iter().find(|x| x.id == h).map(|x| x.entries.iter().filter_map(|t| l.tracks.get(t).cloned()).collect()).unwrap_or_default(),
            (None, None) => l.tracks.values().cloned().collect(),
            (Some(n), None) => l.nodes.iter().find(|x| x.id == n).map(|x| x.entries.iter().filter_map(|t| l.tracks.get(t).cloned()).collect()).unwrap_or_default(),
        },
    };
    let r = ui.max_rect();
    let bar = Rect::from_min_size(r.min, vec2(r.width(), 26.0));
    ui.painter().rect_filled(bar, 0.0, BG2);
    ui.painter().text(pos2(bar.min.x + 10.0, bar.center().y), Align2::LEFT_CENTER, format!("{title} ({} Tracks)", tracks.len()), rb(TITLE_FONT), WHITE);
    let space = match (dev.free, dev.total) {
        (Some(f), Some(t)) => format!("Available Space {:.1} GB / Total Space {:.1} GB", f as f64 / 1e9, t as f64 / 1e9),
        _ => String::new(),
    };
    ui.painter().text(pos2(bar.max.x - 12.0, bar.center().y), Align2::RIGHT_CENTER, space, rb(11.0), DIM);
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(r.min.x, r.min.y + 27.0), r.max)));
    if dev.lib.is_none() {
        body.add_space(30.0);
        body.vertical_centered(|ui| {
            ui.label(RichText::new("This device has no DJ library yet.").color(DIM));
            ui.add_space(6.0);
            if ui.button("Export playlists to this device…").clicked() {
                let r = root.to_string();
                open_export(app, vec![], vec![]);
                app.dialogs.export_root = Some(r);
            }
        });
        return;
    }
    let cols = ["Track Title", "Artist", "Album", "Genre", "BPM", "Key", "Time", "Rating", "File"];
    let widths = [200.0, 150.0, 150.0, 100.0, 56.0, 46.0, 50.0, 60.0, 360.0];
    let hdr = body.allocate_exact_size(vec2(body.available_width(), 14.0), Sense::hover()).0;
    body.painter().rect_filled(hdr, 0.0, BG2);
    let mut x = hdr.min.x + 8.0;
    for (c, w) in cols.iter().zip(widths) {
        body.painter().text(pos2(x, hdr.center().y), Align2::LEFT_CENTER, *c, rb(TITLE_FONT), WHITE);
        x += w;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(&mut body, ROW_H, tracks.len(), |ui, range| {
        for i in range {
            let t = &tracks[i];
            let (rr, row) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            if app.device_ui.sel_root != root {
                app.device_ui.sel_root = root.to_string();
                app.device_ui.sel.clear();
                app.device_ui.anchor = None;
            }
            // Click selects; ⌘ adds or removes one; ⇧ extends from the last clicked row.
            if row.clicked() || (row.secondary_clicked() && !app.device_ui.sel.contains(&t.id)) {
                let m = ui.input(|i| i.modifiers);
                if m.shift && app.device_ui.anchor.is_some() && !row.secondary_clicked() {
                    let a = app.device_ui.anchor.unwrap().min(tracks.len() - 1);
                    app.device_ui.sel = tracks[a.min(i)..=a.max(i)].iter().map(|x| x.id).collect();
                } else if m.command && !row.secondary_clicked() {
                    if !app.device_ui.sel.remove(&t.id) {
                        app.device_ui.sel.insert(t.id);
                    }
                    app.device_ui.anchor = Some(i);
                } else {
                    app.device_ui.sel = [t.id].into();
                    app.device_ui.anchor = Some(i);
                }
            }
            let selected = app.device_ui.sel.contains(&t.id);
            ui.painter().rect_filled(rr, 0.0, if selected { ROW_SEL } else if i % 2 == 0 { BG1 } else { BG2 });
            let r3 = root.to_string();
            row.context_menu(|ui| {
                let ids: Vec<u32> = app.device_ui.sel.iter().copied().collect();
                crate::device_ext::device_track_menu(app, ui, &r3, &ids);
            });
            let vals = [
                t.title.clone(),
                t.artist.clone(),
                t.album.clone(),
                t.genre.clone(),
                format!("{:.2}", t.bpm100 as f64 / 100.0),
                if app.prefs.key_alnum { key_alnum(parse_key(&t.key_name)) } else { t.key_name.clone() },
                fmt_time(t.duration as f64, false),
                "★".repeat(t.rating as usize),
                t.file_path.clone(),
            ];
            let mut x = rr.min.x + 8.0;
            for (v, w) in vals.iter().zip(widths) {
                let clip = ui.painter().with_clip_rect(Rect::from_min_max(pos2(x, rr.min.y), pos2(x + w - 6.0, rr.max.y)));
                clip.text(pos2(x, rr.center().y), Align2::LEFT_CENTER, v, rb(10.5), TEXT);
                x += w;
            }
        }
    });
}

// ------------------------------------------------------------------------------------ track list
pub fn col_width(app: &App, c: Col) -> f32 {
    if c == Col::Status {
        // Fixed: three 30 pt cells (reference 10).
        return 90.0;
    }
    app.prefs.widths.get(&c.key()).copied().unwrap_or(c.width())
}

fn view_title(app: &App) -> String {
    match &app.view {
        View::Collection => "Collection".into(),
        View::Playlist(id) => app.node(*id).map(|n| n.name.clone()).unwrap_or_default(),
        View::Device { .. } | View::DeviceHistory { .. } | View::Explorer(_) | View::Source { .. } => String::new(),
        View::History(id) => app.ext.histories.iter().find(|h| h.id == *id).map(|h| h.name.clone()).unwrap_or_else(|| "Histories".into()),
        View::TagList => "Tag List".into(),
        View::Matching => format!("Matching — {}", app.related_source().and_then(|id| app.track(id)).map(|t| t.display_title()).unwrap_or_else(|| "load or select a track".into())),
        View::Suggest(k) => format!("{} — {}", if *k == 0 { "Track Suggestion" } else { crate::browser_ext::SUGGEST_KINDS[(*k as usize).min(3)] }, app.related_source().and_then(|id| app.track(id)).map(|t| t.display_title()).unwrap_or_else(|| "load or select a track".into())),
        View::Related => format!("Related Tracks — {}", app.related_source().and_then(|id| app.track(id)).map(|t| t.display_title()).unwrap_or_else(|| "load or select a track".into())),
        View::Category { field, value } => format!("{} › {}", ["Date Added", "Genre", "Artist", "Album"][*field as usize % 4], if value.is_empty() { "(none)" } else { value }),
    }
}

pub(crate) fn track_list(app: &mut App, ui: &mut Ui) {
    if matches!(app.view, View::Related | View::Matching | View::Suggest(_)) && app.related_built_for != app.related_source() {
        app.list_dirty = true;
    }
    let had_edit = app.ext.edit.is_some();
    app.ext.edit_drawn = false;
    if app.list_dirty {
        app.rebuild_list();
    }
    let r = ui.max_rect();
    // Title bar: the first 26 pt of the list. References 08/10/11/13/14: the title area is
    // 340 pt wide with a 1 pt divider after it; PLAYER A starts 49 pt further, the four
    // view-mode tiles follow (the group is 253 pt wide), then 20 pt on the search box, which
    // ends 20 pt from the list's edge and is up to 212 pt wide. With less room the group moves
    // left, then the search box narrows to 98 pt, then the title area narrows to 125 pt.
    // The sub-browser's bar has two tiles and a 183 pt search box ending 24 pt from the edge.
    let bar = Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 26.0));
    let p = ui.painter().with_clip_rect(bar.intersect(ui.clip_rect()));
    p.rect_filled(bar, 0.0, BG2);
    let in_sub = app.ext.in_sub;
    let rx = bar.max.x;
    let (lead, group, tail, smin, smax) = if in_sub { (36.0, 55.5, 24.0, 183.0, 216.0) } else { (49.0, 253.0, 20.0, 98.0, 212.0) };
    let title_w = (bar.width() - (lead + group + 20.0 + smin + tail)).clamp(125.0, 340.0);
    let search_w = (bar.width() - title_w - lead - group - 20.0 - tail).clamp(smin, smax);
    let gx = if search_w >= smax { rx - tail - smax - 20.0 - group } else { bar.min.x + title_w + lead };
    let path_menu = !in_sub && matches!(app.view, View::Playlist(_));
    let title_clip = Rect::from_min_max(bar.min, pos2(bar.min.x + title_w - if path_menu { 71.0 } else if in_sub { 22.0 } else { 6.0 }, bar.max.y));
    p.with_clip_rect(title_clip.intersect(ui.clip_rect())).text(pos2(bar.min.x + 10.0, bar.center().y), Align2::LEFT_CENTER, format!("{} ({} Tracks)", view_title(app), app.list.len()), rb(TITLE_FONT), WHITE);
    p.rect_filled(Rect::from_min_max(pos2(bar.min.x + title_w, bar.min.y), pos2(bar.min.x + title_w + 1.0, bar.max.y)), 0.0, BG);
    if path_menu {
        // Skin brws_list_pathsystem_pulldown (10 x 16, #646464): the folders the playlist is in.
        let ar = Rect::from_center_size(pos2(bar.min.x + title_w - 51.0, bar.center().y), vec2(10.0, 16.0));
        let aresp = ui.interact(ar.expand(4.0), ui.id().with("path_pulldown"), Sense::click());
        crate::icons_src::path_pulldown(&p, ar, if aresp.hovered() { DIM } else { OFF });
        dropdown(ui, &aresp, "path_pulldown_menu", |ui| crate::ui_browser_ext::path_menu(app, ui));
    }
    let two = app.two_players();
    if !in_sub {
        // PLAYER A (load target): a 16 pt high #282828 box 80 pt wide, its last 16 pt a
        // separate chevron part.
        let pa = Rect::from_min_max(pos2(gx, bar.min.y + 5.0), pos2(gx + 80.0, bar.min.y + 21.0));
        p.rect_filled(Rect::from_min_max(pa.min, pos2(pa.max.x - 16.5, pa.max.y)), egui::CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, BTN);
        p.rect_filled(Rect::from_min_max(pos2(pa.max.x - 16.0, pa.min.y), pa.max), egui::CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, BTN);
        p.text(pos2(pa.min.x + 7.0, pa.center().y), Align2::LEFT_CENTER, if two && app.load_target == 1 { "PLAYER B" } else { "PLAYER A" }, rb_bold(9.75), WHITE);
        {
            let c = pos2(pa.max.x - 8.0, pa.center().y + 0.5);
            p.add(egui::Shape::line(vec![c + vec2(-3.6, -1.8), c + vec2(0.0, 1.8), c + vec2(3.6, -1.8)], Stroke::new(1.0, WHITE)));
        }
        let par = ui.interact(pa.intersect(bar), ui.id().with("playera"), Sense::click());
        let par = if app.prefs.tooltips { par.on_hover_text("Player that double-click or Return loads tracks into.") } else { par };
        dropdown(ui, &par, "playera_menu", |ui| {
            // Load target: deck B exists in the 2 Players layout.
            if ui.selectable_label(app.load_target == 0 || !two, "PLAYER A").clicked() {
                app.load_target = 0;
            }
            if ui.add_enabled(two, egui::SelectableLabel::new(two && app.load_target == 1, "PLAYER B")).clicked() {
                app.load_target = 1;
            }
            if !two {
                ui.label(RichText::new("PLAYER B is available in the 2 Players layout.").size(11.0).color(DIM));
            }
        });
    }
    // View-mode icons: track list with artwork, track list without artwork, column browser and
    // sub-browser.
    let with_art = app.prefs.columns.contains(&Col::Artwork);
    let vic: [(f32, IconFn, Option<bool>, &str); 4] = [
        (if in_sub { 9.0 } else { 109.0 }, crate::icons_src::bullet_rows4, Some(with_art && !app.prefs.artwork_full), "Show the upper part of the artwork."),
        (if in_sub { 46.5 } else { 147.0 }, crate::icons_src::bullet_rows, Some(with_art && app.prefs.artwork_full), "Show the entire artwork."),
        (206.0, crate::icons_src::columns3, Some(app.prefs.column_browser), "Column browser"),
        (244.0, crate::icons_src::side_pane, Some(app.prefs.sub_browser), "Display sub-browser window."),
    ];
    for (i, (dx, draw, on, tipt)) in vic.into_iter().enumerate().take(if in_sub { 2 } else { 4 }) {
        // Skins artworkPart / artworkAll / subBrowse: 18 pt tiles, 1.5 pt corners, a 1 pt
        // border that is #646464, or blue when lit; the glyph stays white.
        let vr = Rect::from_center_size(pos2(gx + dx, bar.center().y), vec2(18.0, 18.0));
        if vr.max.x > bar.max.x {
            continue;
        }
        let resp = ui.interact(vr, ui.id().with(("viewmode", i)), if on.is_some() { Sense::click() } else { Sense::hover() });
        p.rect_stroke(vr, 1.5, Stroke::new(1.0, if on == Some(true) { TILE_ON } else if resp.hovered() { hex(0x8C8C8C) } else { OFF }), egui::StrokeKind::Inside);
        draw(&p, vr, WHITE);
        let resp = if app.prefs.tooltips || on.is_none() { resp.on_hover_text(tipt) } else { resp };
        if resp.clicked() && i == 3 {
            crate::ui_browser_ext::set_right_panel(app, if app.prefs.sub_browser { 0 } else { 5 });
        } else if resp.clicked() && i == 2 {
            app.prefs.column_browser = !app.prefs.column_browser;
            app.colbrowse = Default::default();
            app.list_dirty = true;
            app.save_prefs();
        } else if resp.clicked() && i < 2 {
            // The lit mode again hides the Artwork column; the other one shows it that way.
            if on == Some(true) {
                app.prefs.columns.retain(|c| *c != Col::Artwork);
            } else {
                if !with_art {
                    let at = app.prefs.columns.iter().position(|c| *c == Col::Title).unwrap_or(0);
                    app.prefs.columns.insert(at, Col::Artwork);
                }
                app.prefs.artwork_full = i == 1;
            }
            app.save_prefs();
        }
    }
    // search box
    let sx0 = gx + group + 20.0;
    let sr = Rect::from_min_max(pos2(sx0, bar.min.y + 1.0), pos2((sx0 + search_w).min(bar.max.x), bar.max.y - 1.0));
    let sr = if sr.width() < 40.0 { Rect::from_min_size(pos2(bar.max.x + 50.0, bar.min.y + 1.0), vec2(60.0, 24.0)) } else { sr };
    p.rect_filled(sr, 0.0, BG);
    // Skin searchSelect: a 25 x 14 pt magnifier with its scope triangle, #646464.
    crate::icons_src::search_select(&p, Rect::from_min_size(pos2(sr.min.x + 9.0, sr.min.y + 5.5), vec2(25.0, 14.0)), OFF);
    let scope_btn = ui.interact(Rect::from_min_max(sr.min, pos2(sr.min.x + 35.0, sr.max.y)).intersect(bar), ui.id().with("search_scope"), Sense::click());
    dropdown(ui, &scope_btn, "search_scope_menu", |ui| {
        for (i, name) in crate::app::SEARCH_SCOPES.iter().enumerate() {
            if ui.selectable_label(app.prefs.search_scope as usize == i, *name).clicked() {
                app.prefs.search_scope = i as u8;
                app.list_dirty = true;
                app.save_prefs();
            }
        }
    });
    let hint = if app.prefs.search_scope == 0 { "Search within this track list".to_string() } else { format!("Search {}", crate::app::SEARCH_SCOPES[app.prefs.search_scope as usize % 8]) };
    let mut s = app.search.clone();
    let te = ui.put(Rect::from_min_max(pos2(sr.min.x + 36.0, sr.min.y), sr.max), egui::TextEdit::singleline(&mut s).frame(false).margin(egui::Margin::ZERO).hint_text(RichText::new(hint).color(OFF).size(12.0)).vertical_align(egui::Align::Center).font(rb(12.0)));
    if te.changed() {
        app.search = s;
        app.list_dirty = true;
    }
    if app.ext.sub_focus == in_sub && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
        te.request_focus();
    }
    // The tile above the right icon strip (skin brows_tagListPalette / playListPalette: 18 pt,
    // 2 pt corners, #282828, centred 12.5 pt into the strip) shows and hides the Playlist
    // Palette. The columns are chosen from a column header's menu.
    if !in_sub {
        let clip = ui.clip_rect();
        ui.set_clip_rect(ui.ctx().screen_rect());
        let cmr = Rect::from_center_size(pos2(ui.ctx().screen_rect().max.x - 12.5, bar.min.y + 14.0), vec2(18.0, 18.0));
        let cm = ui.interact(cmr, ui.id().with("palette_toggle"), Sense::click());
        ui.painter().rect_filled(cmr, 2.0, if app.prefs.palette_open { BLUE } else if cm.hovered() { hex(0x3A3A3A) } else { BTN });
        crate::icons_src::bullet_rows(ui.painter(), cmr, WHITE);
        ui.set_clip_rect(clip);
        let cm = if app.prefs.tooltips { cm.on_hover_text("Show/Hide Playlist Palette") } else { cm };
        if cm.clicked() {
            app.prefs.palette_open = !app.prefs.palette_open;
            app.save_prefs();
        }
    }
    let mut top = bar.max.y;
    if app.filter_open {
        let fr = Rect::from_min_size(pos2(r.min.x, top), vec2(r.width(), 28.0));
        ui.painter().rect_filled(fr, 0.0, BG1);
        let mut fui = ui.new_child(egui::UiBuilder::new().max_rect(fr.shrink2(vec2(10.0, 3.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        crate::ui_library_ext::filter_bar(app, &mut fui);
        top = fr.max.y;
    }
    if app.prefs.column_browser {
        let cr = Rect::from_min_size(pos2(r.min.x, top.max(r.min.y + 27.0)), vec2(r.width(), 150.0));
        column_browser(app, ui, cr);
        top = cr.max.y + 4.0;
    }
    let top = top.max(r.min.y + 26.0) - 3.0;
    // Reference 04: the list keeps an 11 pt column on the right for its vertical scroll bar
    // (black track, 6.5 pt #323232 thumb) and a solid horizontal bar along the bottom.
    let vbar_x = r.max.x - 12.0;
    let area = Rect::from_min_max(pos2(r.min.x, top + 4.0), pos2(vbar_x, r.max.y));
    let mut aui = ui.new_child(egui::UiBuilder::new().max_rect(area));
    aui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    aui.spacing_mut().scroll = egui::style::ScrollStyle {
        floating: false,
        bar_width: 8.0,
        bar_inner_margin: 2.0,
        bar_outer_margin: 2.0,
        foreground_color: false,
        dormant_background_opacity: 1.0,
        active_background_opacity: 1.0,
        interact_background_opacity: 1.0,
        dormant_handle_opacity: 1.0,
        active_handle_opacity: 1.0,
        interact_handle_opacity: 1.0,
        ..egui::style::ScrollStyle::solid()
    };
    aui.visuals_mut().extreme_bg_color = Color32::BLACK;
    aui.visuals_mut().widgets.inactive.bg_fill = hex(0x323232);
    aui.visuals_mut().widgets.hovered.bg_fill = hex(0x4A4A4A);
    aui.visuals_mut().widgets.active.bg_fill = hex(0x5A5A5A);
    let mut vstate: Option<(Rect, f32, f32)> = None;
    let cols = app.prefs.columns.clone();
    let mut all_cols = vec![Col::Status];
    all_cols.extend(cols.iter().copied().filter(|c| *c != Col::Status));
    let total_w: f32 = all_cols.iter().map(|c| col_width(app, *c)).sum::<f32>() + 12.0;
    let manual = app.is_manual_playlist();
    egui::ScrollArea::horizontal().id_salt("hscroll").auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(&mut aui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let w = total_w.max(ui.available_width());
        // header: 13.5 pt, #1E1E1E, white 11 pt labels, sort column in blue with an arrow
        let (hr, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
        ui.painter().rect_filled(hr, 0.0, BG2);
        let mut x = hr.min.x;
        for c in &all_cols {
            let cw = col_width(app, *c);
            let cr = Rect::from_min_size(pos2(x, hr.min.y), vec2(cw, hr.height()));
            let resp = ui.interact(cr, ui.id().with(("hdr", *c)), if *c == Col::Status { Sense::click() } else { Sense::click_and_drag() });
            // Drag a column header to move the column (drop before/after the header under the
            // pointer).
            let drag_key = egui::Id::new("col_drag");
            if resp.drag_started() && *c != Col::Status {
                ui.ctx().data_mut(|d| d.insert_temp(drag_key, Some(*c)));
            }
            let dragging: Option<Col> = ui.ctx().data(|d| d.get_temp(drag_key)).flatten();
            if let (Some(dc), Some(pp)) = (dragging, ui.ctx().pointer_latest_pos()) {
                if dc != *c && *c != Col::Status && cr.contains(pp) {
                    let after = pp.x > cr.center().x;
                    let lx = if after { cr.max.x } else { cr.min.x };
                    ui.painter().line_segment([pos2(lx, cr.min.y), pos2(lx, cr.max.y + 4.0)], Stroke::new(2.0, BLUE));
                    if ui.input(|i| i.pointer.any_released()) {
                        let cols = &mut app.prefs.columns;
                        cols.retain(|x| *x != dc);
                        let at = cols.iter().position(|x| x == c).map(|i| if after { i + 1 } else { i }).unwrap_or(cols.len());
                        cols.insert(at.min(cols.len()), dc);
                        app.save_prefs();
                        ui.ctx().data_mut(|d| d.insert_temp::<Option<Col>>(drag_key, None));
                    }
                }
            }
            if dragging.is_some() && ui.input(|i| i.pointer.any_released()) {
                ui.ctx().data_mut(|d| d.insert_temp::<Option<Col>>(drag_key, None));
            }
            let sorted = app.prefs.sort_col == *c;
            if *c == Col::Status {
                // Skin brows_filter: 10 pt, #969696; the column is two 30 pt halves.
                // Three 30 pt cells: the Track Filter funnel, the My Tag filter funnel, and the
                // attribute icons' cell.
                if !app.ext.in_sub {
                    crate::icons_src::funnel(ui.painter(), pos2(cr.min.x + 15.0, cr.center().y - 1.0), if app.filter_open { TILE_ON } else { DIM });
                    crate::icons_src::funnel(ui.painter(), pos2(cr.min.x + 45.0, cr.center().y - 1.0), if !app.filter_tags.is_empty() { TILE_ON } else { DIM });
                }
                ui.painter().rect_filled(Rect::from_min_max(pos2(cr.min.x + 29.0, cr.min.y), pos2(cr.min.x + 30.0, cr.max.y)), 0.0, BG);
                ui.painter().rect_filled(Rect::from_min_max(pos2(cr.min.x + 59.0, cr.min.y), pos2(cr.min.x + 60.0, cr.max.y)), 0.0, BG);
            }
            let label = if *c == Col::Status { "" } else { c.label() };
            ui.painter().with_clip_rect(cr).text(pos2(cr.min.x + 7.0, cr.center().y), Align2::LEFT_CENTER, label, rb(TITLE_FONT), if sorted { TILE_ON } else { WHITE });
            if sorted {
                // Skins sortUp / sortDown: the arrow sits 14 pt from the column's right edge.
                let ac = if label.is_empty() { pos2(cr.min.x + 74.0, cr.center().y) } else { pos2(cr.max.x - 14.0, cr.center().y) };
                crate::icons_src::sort_arrow(&ui.painter().with_clip_rect(cr), ac, app.prefs.sort_asc, TILE_ON);
            }
            ui.painter().rect_filled(Rect::from_min_max(pos2(cr.max.x - 1.0, cr.min.y), cr.max), 0.0, BG);
            if *c == Col::Status && app.ext.in_sub {
                x += cw;
                continue;
            }
            if *c == Col::Status && resp.clicked() && resp.interact_pointer_pos().map(|pp| pp.x >= cr.min.x + 30.0 && pp.x < cr.min.x + 60.0).unwrap_or(false) {
                // "Show/Hide My Tag filter": the My Tag window, where a tag is Alt-clicked to
                // filter by it; with tags already chosen the funnel clears them.
                if app.filter_tags.is_empty() {
                    crate::ui_browser_ext::set_right_panel(app, if app.prefs.show_mytag { 0 } else { 1 });
                } else {
                    app.filter_tags.clear();
                }
                app.list_dirty = true;
                x += cw;
                continue;
            }
            if *c == Col::Status && resp.clicked() {
                app.filter_open = !app.filter_open;
                app.list_dirty = true;
                x += cw;
                continue;
            }
            if resp.clicked() {
                if app.prefs.sort_col == *c {
                    app.prefs.sort_asc = !app.prefs.sort_asc;
                } else {
                    app.prefs.sort_col = *c;
                    app.prefs.sort_asc = !matches!(c, Col::DateAdded | Col::Rating);
                }
                app.list_dirty = true;
                app.save_prefs();
            }
            let c2 = *c;
            resp.context_menu(|ui| {
                crate::ui_library_ext::header_menu(app, ui, c2);
            });
            // resize handle
            let hr2 = Rect::from_min_max(pos2(cr.max.x - 3.0, cr.min.y), pos2(cr.max.x + 3.0, cr.max.y));
            let h = ui.interact(hr2, ui.id().with(("resize", *c)), Sense::drag());
            if h.hovered() || h.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if h.dragged() {
                let nw = (cw + h.drag_delta().x).clamp(24.0, 800.0);
                app.prefs.widths.insert(c.key(), nw);
            }
            if h.drag_stopped() {
                app.save_prefs();
            }
            x += cw;
        }
        if app.list_dirty {
            app.rebuild_list();
        }
        let n = app.list.len();
        let rh = row_height(app);
        let mut sa = egui::ScrollArea::vertical().id_salt("vscroll").auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        if let Some(y) = app.list_scroll_to.take() {
            sa = sa.vertical_scroll_offset(y);
        }
        if app.scroll_to_selected {
            if let Some(i) = app.anchor {
                let h = ui.available_height();
                let y = i as f32 * rh;
                if app.prefs.center_selected {
                    sa = sa.vertical_scroll_offset((y - (h - rh) * 0.5).max(0.0));
                } else {
                    // Scroll only as far as needed to show the row.
                    let id = ui.make_persistent_id("vscroll");
                    let cur = egui::scroll_area::State::load(ui.ctx(), id).map(|s| s.offset.y).unwrap_or(0.0);
                    let view_h = (h - rh).max(rh);
                    if y < cur {
                        sa = sa.vertical_scroll_offset(y);
                    } else if y + rh > cur + view_h {
                        sa = sa.vertical_scroll_offset((y + rh - view_h).max(0.0));
                    }
                }
            }
            app.scroll_to_selected = false;
        }
        let mut drop_line: Option<usize> = None;
        let vout = sa.show_rows(ui, rh, n, |ui, range| {
            for i in range {
                let id = app.list[i];
                let Some(t) = app.track(id).cloned() else { continue };
                let (rr, resp) = ui.allocate_exact_size(vec2(w, rh), Sense::click_and_drag());
                let sel = app.selected.contains(&id);
                let loaded = app.deck.track == Some(id);
                let bg = if sel { ROW_SEL } else if i % 2 == 0 { BG1 } else { BG2 };
                ui.painter().rect_filled(rr, 0.0, bg);
                if loaded && !sel {
                    ui.painter().rect_stroke(rr, 0.0, Stroke::new(1.0, hex(0x27567A)), egui::StrokeKind::Inside);
                }
                let mut x = rr.min.x;
                for c in &all_cols {
                    let cw = col_width(app, *c);
                    let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(cw, rr.height()));
                    draw_cell(app, ui, *c, &t, cr, i);
                    x += cw;
                    // Column lines run through the rows: the last point of each column, #323232.
                    ui.painter().rect_filled(Rect::from_min_max(pos2(x - 1.0, rr.min.y), pos2(x, rr.max.y)), 0.0, SCROLL);
                }
                // The cell under the pointer.
                let cell_at = |app: &App, px: f32| -> Option<(Col, Rect)> {
                    let mut x = rr.min.x;
                    for c in &all_cols {
                        let cw = col_width(app, *c);
                        if px >= x && px < x + cw {
                            return Some((*c, Rect::from_min_size(pos2(x, rr.min.y), vec2(cw, rr.height()))));
                        }
                        x += cw;
                    }
                    None
                };
                let cell_of = |app: &App, col: Col| -> Option<Rect> {
                    let mut x = rr.min.x;
                    for c in &all_cols {
                        let cw = col_width(app, *c);
                        if *c == col {
                            return Some(Rect::from_min_size(pos2(x, rr.min.y), vec2(cw, rr.height())));
                        }
                        x += cw;
                    }
                    None
                };
                // Quick preview button in the Artwork cell ("Preview : Play/Stop"), shown while the
                // pointer is over the cell or this track is previewing.
                let previewing = app.preview_status().map(|s| s.0) == Some(id);
                let art_btn = cell_of(app, Col::Artwork).filter(|cr| previewing || ui.rect_contains_pointer(*cr)).map(|cr| Rect::from_center_size(pos2(cr.min.x + 10.0, cr.center().y), vec2(13.0, 13.0)));
                if let Some(b) = art_btn {
                    let pp = ui.painter();
                    pp.circle_filled(b.center(), 6.5, Color32::from_black_alpha(190));
                    pp.circle_stroke(b.center(), 5.8, Stroke::new(1.0, WHITE));
                    if previewing {
                        pp.rect_filled(Rect::from_center_size(b.center(), vec2(4.2, 4.2)), 0.0, WHITE);
                    } else {
                        pp.add(egui::Shape::convex_polygon(vec![b.center() + vec2(-1.8, -3.0), b.center() + vec2(3.2, 0.0), b.center() + vec2(-1.8, 3.0)], WHITE, Stroke::NONE));
                    }
                    if app.prefs.tooltips && ui.rect_contains_pointer(b) {
                        ui.interact(b, ui.id().with(("pv_tip", i)), Sense::hover()).on_hover_text("Preview : Play/Stop");
                    }
                }
                // In-place editor of this row's cell.
                if !app.ext.edit_drawn {
                    if let Some(col) = app.ext.edit.as_ref().filter(|e| e.track == id).map(|e| e.col) {
                        match cell_of(app, col) {
                            Some(cr) => crate::ui_library_ext::inline_editor(app, ui, cr),
                            None => app.commit_cell_edit(),
                        }
                    }
                }
                let hit = resp.interact_pointer_pos().and_then(|pp| cell_at(app, pp.x).map(|(c, cr)| (c, cr, pp)));
                if resp.clicked() {
                    let (cmd, shift) = ui.input(|inp| (inp.modifiers.command, inp.modifiers.shift));
                    let now = ui.input(|inp| inp.time);
                    let was_sole = sel && app.selected.len() == 1;
                    let mut keep_preview = false;
                    if let Some((c, cr, pp)) = hit {
                        match c {
                            Col::Rating => {
                                let s = (((pp.x - cr.min.x - 6.0) / 16.0).floor() as i32 + 1).clamp(0, 5) as u8;
                                if let Some(tm) = app.track_mut(id) {
                                    tm.rating = if tm.rating == s { 0 } else { s };
                                }
                                app.save_track(id);
                            }
                            // "Waveform Preview": play from the clicked position; a click in the
                            // track already previewing skips to it ("Preview : Skip").
                            Col::Preview => {
                                let frac = ((pp.x - cr.min.x - 30.0) / (cr.width() - 35.0).max(1.0)).clamp(0.0, 1.0) as f64;
                                app.preview_play(id, frac);
                                keep_preview = true;
                            }
                            Col::Artwork if art_btn.map(|b| b.expand(2.0).contains(pp)).unwrap_or(false) => {
                                keep_preview = true;
                                if previewing {
                                    app.preview_stop();
                                } else {
                                    app.preview_play(id, 0.0);
                                }
                            }
                            Col::Color => app.ext.color_pick = Some((id, pos2(cr.min.x, rr.max.y), now)),
                            // A click on a cell of the row already selected edits it in place
                            // (after the double-click time, so a double-click still loads).
                            c if crate::library_ext::editable(c) && was_sole && !cmd && !shift => {
                                app.ext.edit_pending = Some((id, c, now));
                                ui.ctx().request_repaint_after(std::time::Duration::from_millis(360));
                            }
                            _ => {}
                        }
                    }
                    if !keep_preview {
                        app.preview_stop();
                    }
                    if shift {
                        let a = app.anchor.unwrap_or(i);
                        let (lo, hi) = (a.min(i), a.max(i));
                        app.selected = app.list[lo..=hi].iter().copied().collect();
                    } else if cmd {
                        if !app.selected.insert(id) {
                            app.selected.remove(&id);
                        }
                        app.set_anchor(i);
                    } else {
                        app.selected.clear();
                        app.selected.insert(id);
                        app.set_anchor(i);
                    }
                }
                if resp.double_clicked() {
                    app.ext.edit_pending = None;
                    if !matches!(hit.map(|h| h.0), Some(Col::Preview | Col::Rating | Col::Color)) {
                        app.preview_stop();
                        app.load_track(id);
                    }
                }
                if resp.drag_started() {
                    app.ext.edit_pending = None;
                    if !app.selected.contains(&id) {
                        app.selected.clear();
                        app.selected.insert(id);
                        app.set_anchor(i);
                    }
                    let ids: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
                    app.drag = Some(ids);
                }
                if app.drag.is_some() && manual.is_some() {
                    if let Some(pp) = ui.ctx().pointer_latest_pos() {
                        if rr.contains(pp) {
                            let idx = if pp.y < rr.center().y { i } else { i + 1 };
                            drop_line = Some(idx);
                            let y = if pp.y < rr.center().y { rr.min.y } else { rr.max.y };
                            ui.painter().line_segment([pos2(rr.min.x, y), pos2(rr.max.x, y)], Stroke::new(2.0, BLUE));
                        }
                    }
                }
                if resp.secondary_clicked() && !app.selected.contains(&id) {
                    app.selected.clear();
                    app.selected.insert(id);
                    app.set_anchor(i);
                }
                resp.context_menu(|ui| {
                    let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
                    track_menu_items(app, ui, &sel);
                });
            }
        });
        vstate = Some((vout.inner_rect, vout.state.offset.y, vout.content_size.y));
        // Reorder inside a playlist (only when sorted by #)
        if let (Some(pl), Some(to), Some(ids)) = (manual, drop_line, app.drag.clone()) {
            if ui.input(|i| i.pointer.any_released()) && app.prefs.sort_col == Col::Num && app.prefs.sort_asc && app.search.is_empty() {
                app.move_in_playlist(pl, &ids, to);
                app.drag = None;
            }
        }
    });
    // An edit whose row is no longer drawn (scrolled away, list changed) is committed.
    if had_edit && app.ext.edit.is_some() && !app.ext.edit_drawn {
        app.commit_cell_edit();
    }
    // A single click on a selected row's cell becomes an edit once it was not a double-click.
    if let Some((id, c, t0)) = app.ext.edit_pending {
        let now = ui.input(|i| i.time);
        if now - t0 > 0.33 {
            app.ext.edit_pending = None;
            if app.selected.len() == 1 && app.selected.contains(&id) && app.drag.is_none() {
                if let Some(t) = app.track(id) {
                    let text = crate::library_ext::edit_text(t, c, app.prefs.key_alnum);
                    app.ext.edit = Some(crate::library_ext::CellEdit { track: id, col: c, text, focused: false });
                    // The editor is drawn by the row on the next frame: ask for that frame now.
                    ui.ctx().request_repaint();
                }
            }
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        }
    }
    // Keys of the list (not while a text field is being typed in): Esc ends a preview, Delete
    // removes from the Tag List or history shown, and Cut / Copy / Paste move tracks between
    // playlists.
    if !ui.ctx().wants_keyboard_input() && app.ext.sub_focus == app.ext.in_sub {
        // Select All (Edit menu / Cmd+A) selects every track of the list shown.
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            app.select_all();
        }
        let (esc, del, copy, cut, paste) = ui.input(|i| {
            let ev = |f: &dyn Fn(&egui::Event) -> bool| i.events.iter().any(|e| f(e));
            (i.key_pressed(egui::Key::Escape), (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) && i.modifiers.is_none(), ev(&|e| matches!(e, egui::Event::Copy)), ev(&|e| matches!(e, egui::Event::Cut)), ev(&|e| matches!(e, egui::Event::Paste(_))))
        });
        if esc {
            app.preview_stop();
        }
        if del && !app.selected.is_empty() {
            let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
            match app.view {
                View::TagList => app.tag_list_remove(&sel),
                View::History(h) => app.history_remove_tracks(h, &sel),
                _ => {}
            }
        }
        if copy || cut {
            app.copy_tracks(cut);
        }
        if paste {
            app.paste_tracks();
        }
    }
    // The fixed vertical scroll bar (stays put when the columns scroll sideways).
    if let Some((inner, off, content_h)) = vstate {
        let track = Rect::from_min_max(pos2(vbar_x, area.min.y + 14.0), pos2(r.max.x, inner.max.y));
        let p = ui.painter();
        // The header band runs on over the scroll bar's column.
        p.rect_filled(Rect::from_min_max(pos2(vbar_x, area.min.y), pos2(r.max.x, area.min.y + 14.0)), 0.0, BG2);
        p.rect_filled(track, 0.0, Color32::BLACK);
        // The square where the two scroll bars meet (10 pt, #323232).
        p.rect_filled(Rect::from_min_max(pos2(vbar_x + 1.0, r.max.y - 11.0), pos2(vbar_x + 11.0, r.max.y - 1.0)), 0.0, SCROLL);
        let view_h = inner.height().max(1.0);
        if content_h > view_h + 0.5 {
            let len = (track.height() * view_h / content_h).max(16.0).min(track.height());
            let max_off = (content_h - view_h).max(1.0);
            let y0 = track.min.y + (track.height() - len) * (off / max_off).clamp(0.0, 1.0);
            let thumb = Rect::from_min_size(pos2(track.min.x + 2.0, y0), vec2(8.0, len));
            let resp = ui.interact(track, ui.id().with("list_vbar"), Sense::click_and_drag());
            p.rect_filled(thumb, 2.0, if resp.dragged() { hex(0x5A5A5A) } else if resp.hovered() { hex(0x4A4A4A) } else { hex(0x323232) });
            if resp.dragged() || resp.clicked() {
                if let Some(pp) = resp.interact_pointer_pos() {
                    let f = ((pp.y - track.min.y - len / 2.0) / (track.height() - len).max(1.0)).clamp(0.0, 1.0);
                    app.list_scroll_to = Some(f * max_off);
                    ui.ctx().request_repaint();
                }
            }
        }
    }
    // Files dropped from Finder
    let dropped: Vec<std::path::PathBuf> = if app.ext.in_sub { vec![] } else { ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect()) };
    if !dropped.is_empty() {
        app.import_paths(dropped, app.is_manual_playlist());
    }
}

/// Column browser: Genre, Artist and Album lists; each choice narrows the columns to its
/// right and the track list.
fn column_browser(app: &mut App, ui: &mut Ui, r: Rect) {
    if app.list_dirty {
        app.rebuild_list();
    }
    ui.painter().rect_filled(r, 0.0, BG1);
    let w = r.width() / 3.0;
    for (i, name) in ["Genre", "Artist", "Album"].iter().enumerate() {
        let cr = Rect::from_min_size(pos2(r.min.x + w * i as f32, r.min.y), vec2(w, r.height()));
        if i > 0 {
            ui.painter().line_segment([pos2(cr.min.x, cr.min.y), pos2(cr.min.x, cr.max.y)], Stroke::new(1.0, BG));
        }
        let hr = Rect::from_min_size(cr.min, vec2(w, 14.0));
        ui.painter().rect_filled(hr, 0.0, BG2);
        ui.painter().text(pos2(hr.min.x + 6.0, hr.center().y), Align2::LEFT_CENTER, *name, rb(11.0), WHITE);
        let body = Rect::from_min_max(pos2(cr.min.x + 1.0, hr.max.y), pos2(cr.max.x - 1.0, cr.max.y));
        let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body));
        bui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let values = app.colbrowse_values[i].clone();
        let total: usize = values.iter().map(|v| v.1).sum();
        let mut pick: Option<Option<String>> = None;
        egui::ScrollArea::vertical().id_salt(("colbrowse", i)).auto_shrink([false, false]).show(&mut bui, |ui| {
            let mut row = |ui: &mut Ui, label: String, val: Option<String>| {
                let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click());
                let on = app.colbrowse[i] == val;
                if on {
                    ui.painter().rect_filled(rr, 0.0, BLUE);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rr, 0.0, BG2);
                }
                ui.painter().text(pos2(rr.min.x + 6.0, rr.center().y), Align2::LEFT_CENTER, label, rb(11.0), if on { WHITE } else { TEXT });
                if resp.clicked() {
                    pick = Some(val);
                }
            };
            row(ui, format!("All ({total})"), None);
            for (v, n) in values {
                let label = if v.is_empty() { format!("(No {}) ({n})", name.to_lowercase()) } else { format!("{v} ({n})") };
                row(ui, label, Some(v));
            }
        });
        if let Some(val) = pick {
            app.colbrowse[i] = val;
            for j in i + 1..3 {
                app.colbrowse[j] = None;
            }
            app.list_dirty = true;
        }
    }
}

/// Track list row height: Preferences > View > Browse font size and line space.
pub fn row_height(app: &App) -> f32 {
    let h = ROW_H + app.prefs.browse_font.min(4) as f32 * 1.5 + app.prefs.browse_line.min(4) as f32 * 2.5;
    // "Show the entire artwork.": rows tall enough for a square picture.
    if app.prefs.artwork_full && app.prefs.columns.contains(&Col::Artwork) {
        h.max(34.0)
    } else {
        h
    }
}

/// Track list text font (Preferences > View > Browse).
pub fn list_font(app: &App) -> FontId {
    let size = 10.5 + app.prefs.browse_font.min(4) as f32 * 1.0;
    if app.prefs.browse_bold {
        rb_bold(size)
    } else {
        rb(size)
    }
}

thread_local! {
    /// Preview-cell bars by (track, columns, height, colour mode), each with a hash of the
    /// preview data it was built from.
    static PREVIEW_MESHES: std::cell::RefCell<HashMap<(i64, usize, u32, u8), (u64, egui::Mesh)>> = std::cell::RefCell::new(HashMap::new());
}

fn draw_cell(app: &mut App, ui: &mut Ui, c: Col, t: &Track, r: Rect, row: usize) {
    let p = ui.painter().with_clip_rect(r.shrink2(vec2(3.0, 0.0)));
    let text_pos = |right: bool| if right { pos2(r.max.x - 4.5, r.center().y) } else { pos2(r.min.x + 5.0, r.center().y) };
    let font = list_font(app);
    let col = if t.missing { CAUTION_TX } else { TEXT };
    let txt = |s: String| {
        let right = c.right_aligned();
        p.text(text_pos(right), if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }, s, font.clone(), col);
    };
    match c {
        Col::Num => {
            // Track Order: the track's stored position in the playlist, whatever the sort.
            // Left-aligned 5.5 pt in, as in reference 04.
            let n = app.ext.list_num.get(row).copied().unwrap_or(row + 1);
            p.text(pos2(r.min.x + 5.5, r.center().y), Align2::LEFT_CENTER, n.to_string(), font.clone(), col);
        }
        Col::Status => crate::ui_library_ext::status_cell(app, ui, t, r),
        Col::Preview => {
            // Reference 01: the cell is a darker box (black on the dark rows, #191919 on the
            // light ones) with 1 pt of the row above and below it; the waveform starts 30 pt
            // in, ends 5 pt before the column's line and stands on a base 2.5 pt above the
            // row's foot, at most 9 pt high.
            let (x0, x1) = (r.min.x + 30.0, r.max.x - 5.0);
            if r.width() > 40.0 {
                ui.painter().with_clip_rect(r).rect_filled(Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 1.0), pos2(r.max.x - 1.0, r.max.y - 1.0)), 0.0, if row % 2 == 0 { BG } else { hex(0x191919) });
            }
            if let Some(pv) = app.previews.get(&t.id) {
                let n = (x1 - x0).max(1.0) as usize;
                // The bars of one cell depend only on the stored preview, the cell's size and the
                // colour mode, so they are built once (as a mesh at the origin) and moved into
                // place each frame: rebuilding them for every visible row was most of the cost
                // of drawing the track list.
                let hh = (r.height() - 6.0).max(4.0);
                let wave_color = app.prefs.wave_color;
                let stamp = pv.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
                let key = (t.id, n, hh.to_bits(), wave_color);
                let mut mesh = PREVIEW_MESHES.with(|c| {
                    let mut c = c.borrow_mut();
                    if let Some((st, m)) = c.get(&key) {
                        if *st == stamp {
                            return m.clone();
                        }
                    }
                    // Stored preview: 400 display-level columns of 4 bytes; resample to the cell width.
                    let src: Vec<[u8; 4]> = pv.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                    let len = src.len().max(1);
                    let cols: Vec<[u8; 4]> = (0..n)
                        .map(|i| {
                            let a = i * len / n;
                            let b = ((i + 1) * len / n).max(a + 1).min(len);
                            // Mostly the mean of the columns under each pixel, which keeps the
                            // track's shape (the maximum alone flattens loud music into a solid
                            // block), with some of the maximum so single hits still stand out.
                            let part = src.get(a..b).unwrap_or(&[]);
                            let mut m = [0u8; 4];
                            for k in 0..4 {
                                let mean = part.iter().map(|f| f[k] as f32).sum::<f32>() / part.len().max(1) as f32;
                                let peak = part.iter().map(|f| f[k]).max().unwrap_or(0) as f32;
                                m[k] = (0.6 * mean + 0.4 * peak) as u8;
                            }
                            m
                        })
                        .collect();
                    let base = 0.0_f32;
                    let mut bars = crate::ui_kit::Bars::new();
                    for (k, v) in cols.iter().enumerate() {
                        let x = k as f32 + 0.5;
                        if wave_color == 2 {
                            // 3Band, as rekordbox's preview stacks it (reference 02): the low band
                            // stands on the base in blue, the mid band sits on top of it in orange
                            // and the highs on top of that in white. Display levels carry the
                            // gains 1 / 1.5 / 2.6, taken out again here.
                            let f = |b: u8, g: f32| b as f32 / 255.0 / g;
                            let mut y = base;
                            for (h, cc) in [(f(v[1], 1.0) * 0.46, WAVE_LOW), (f(v[2], 1.5) * 0.40, WAVE_MID), (f(v[3], 2.6) * 0.75, WAVE_HIGH)] {
                                let top = y - h * hh;
                                if h > 0.0 {
                                    bars.column(x, top, y, cc);
                                }
                                y = top;
                            }
                        } else {
                            for (h, cc) in crate::ui_player::wave_bars(wave_color, *v) {
                                bars.column(x, base, base - h * hh, cc);
                            }
                        }
                    }
                    let m = bars.into_mesh();
                    if c.len() > 4096 {
                        c.clear();
                    }
                    c.insert(key, (stamp, m.clone()));
                    m
                });
                if !mesh.is_empty() {
                    mesh.translate(vec2(x0, r.max.y - 2.5));
                    p.add(egui::Shape::mesh(mesh));
                }
                // Hot cue letters at their positions (as the original's preview shows them).
                if let Some(hc) = app.hot_positions.get(&t.id).filter(|_| app.prefs.cue_markers_on_preview) {
                    let dur = t.duration.max(0.001);
                    for (start, slot, color) in hc {
                        // A 7 pt square straddling the cue's position, its letter dark.
                        let x = x0 + (*start / dur) as f32 * n as f32;
                        let cr = Rect::from_min_size(pos2(x - 3.0, r.min.y + 4.0), vec2(7.0, 7.0));
                        p.rect_filled(cr, 0.0, hotcue_color(*color));
                        p.text(cr.center() + vec2(0.0, 0.3), Align2::CENTER_CENTER, ((b'A' + *slot as u8) as char).to_string(), rb_bold(7.5), hex(0x1E1E1E));
                    }
                }
            }
            // Preview playback: the playhead moves across the cell (a pulsing bar while the
            // file is being read).
            if let Some((pid, frac)) = app.preview_status() {
                if pid == t.id {
                    match frac {
                        Some(f) => {
                            let x = x0 + f as f32 * (x1 - x0);
                            p.rect_filled(Rect::from_min_max(pos2(x0, r.min.y + 1.0), pos2(x, r.max.y - 1.0)), 0.0, Color32::from_black_alpha(110));
                            p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.5, WHITE));
                        }
                        None => {
                            let a = (app.ctx.input(|i| i.time) * 6.0).sin() as f32 * 0.5 + 0.5;
                            p.rect_filled(Rect::from_min_size(pos2(x0, r.max.y - 3.0), vec2(x1 - x0, 2.0)), 0.0, WHITE.gamma_multiply(0.3 + 0.6 * a));
                        }
                    }
                }
            }
        }
        Col::Artwork => {
            if t.artwork.is_empty() {
                // No artwork: the top of the original's dim 49 pt disc.
                ui.painter().with_clip_rect(Rect::from_min_max(r.min, pos2(r.max.x - 1.0, r.max.y))).circle_filled(pos2(r.center().x - 0.5, r.min.y + 26.5), 24.5, if row % 2 == 0 { hex(0x1E1E1E) } else { hex(0x242424) });
            }
            if let Some(tex) = artwork_texture(app, &t.artwork) {
                let cell = r.shrink2(vec2(0.0, 0.5));
                if app.prefs.artwork_full {
                    // "Show the entire artwork.": the whole picture, square, in a taller row.
                    let s = cell.height().min(cell.width());
                    p.image(tex.id(), Rect::from_center_size(cell.center(), vec2(s, s)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    // "Show the upper part of the artwork.": the cell's width of the picture, from
                    // its top edge down.
                    let band = (cell.height() / cell.width()).min(1.0);
                    p.image(tex.id(), cell, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, band)), Color32::WHITE);
                }
            }
        }
        Col::Title => txt(t.display_title()),
        Col::Artist => txt(t.artist.clone()),
        Col::Album => txt(t.album.clone()),
        Col::Genre => txt(t.genre.clone()),
        Col::Bpm => txt(if t.bpm > 0.0 { format!("{:.2}", t.bpm) } else { String::new() }),
        Col::Rating => {
            for s in 1..=5u8 {
                // Skins rating_on / rating_off: 12 x 11 pt stars on a 16 pt pitch, the first centred 14 pt in.
                crate::icons_src::star(&p, pos2(r.min.x + 14.0 + (s - 1) as f32 * 16.0, r.center().y), t.rating >= s, TEXT);
            }
        }
        Col::Time => txt(if t.duration > 0.0 { fmt_time(t.duration, false) } else { String::new() }),
        Col::Key => txt(t.key_label(app.prefs.key_alnum)),
        Col::DateAdded => txt(t.date_added.get(..10).unwrap_or(&t.date_added).to_string()),
        Col::Color => {
            if t.color > 0 {
                p.rect_filled(Rect::from_center_size(r.center(), vec2(12.0, 8.0)), 1.0, label_color(t.color));
            }
        }
        Col::Comments => txt(t.comment.clone()),
        Col::Label => txt(t.label.clone()),
        Col::Remixer => txt(t.remixer.clone()),
        Col::OriginalArtist => txt(t.original_artist.clone()),
        Col::Composer => txt(t.composer.clone()),
        Col::Year => txt(if t.year > 0 { t.year.to_string() } else { String::new() }),
        Col::Bitrate => txt(if t.bitrate > 0 { format!("{} Kbps", t.bitrate) } else { String::new() }),
        Col::SampleRate => txt(if t.sample_rate > 0 { format!("{:.1} kHz", t.sample_rate as f64 / 1000.0) } else { String::new() }),
        Col::FileType => txt(t.file_type.to_uppercase()),
        Col::Size => txt(format!("{:.1} MB", t.file_size as f64 / 1e6)),
        Col::Location => txt(t.path.clone()),
        Col::FileName => txt(t.file_name()),
        Col::PlayCount => txt(t.play_count.to_string()),
        Col::MixName => txt(t.mix_name.clone()),
        Col::TrackNumber => txt(if t.track_no > 0 { t.track_no.to_string() } else { String::new() }),
        Col::MyTag => txt(app.tag_text(t.id)),
        Col::ReleaseDate => txt(t.release_date.clone()),
        Col::AlbumArtist => txt(t.album_artist.clone()),
        Col::DiscNumber => txt(if t.disc_no > 0 { t.disc_no.to_string() } else { String::new() }),
        Col::Bitdepth => txt(if t.bit_depth > 0 { format!("{} bit", t.bit_depth) } else { String::new() }),
        Col::Grouping => txt(t.grouping.clone()),
        Col::Lyricist => txt(t.lyricist.clone()),
        Col::Message => txt(t.message.clone()),
        Col::HotCue => {
            let n = app.cue_counts.get(&t.id).map(|c| c.0).unwrap_or(0);
            txt(if n > 0 { format!("{n}") } else { String::new() });
        }
        Col::DateCreated => txt(t.date_created.get(..10).unwrap_or(&t.date_created).to_string()),
    }
}

pub fn track_menu_items(app: &mut App, ui: &mut Ui, sel: &[i64]) {
    crate::ui_library_ext::track_menu(app, ui, sel);
}
