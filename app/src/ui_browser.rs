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
    crate::icons_player::layout(&p, pos2(r.min.x + 145.0, r.min.y + 14.0), WHITE);
    ip(&p, pos2(r.min.x + 170.75, r.min.y + 14.5), 1.0, WHITE);
    dropdown(ui, &lay, "layout", |ui| {
        // Layouts of the player (App::set_player_layout: 0–2 player layouts, 3 = browser only).
        let cur = app.player_layout();
        for (i, name) in ["1 Player", "2 Players", "Simple Player", "Full Browser"].iter().enumerate() {
            if ui.selectable_label(cur == i as u8, *name).clicked() {
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
            if let Some(f) = rfd::FileDialog::new().set_file_name("Recordbox-library-backup.db").save_file() {
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
        if ui.button("About Recordbox").clicked() {
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
    // App mark (own design) and name, where the original shows its wordmark.
    // Sized and placed like the reference's footer mark: an 11.5 pt mark from 10 pt, the name
    // from 25 pt in a regular weight with 11 pt ascenders.
    let mc = pos2(r.min.x + 15.75, r.center().y + 0.25);
    p.circle_stroke(mc, 4.9, Stroke::new(1.6, WHITE));
    p.circle_filled(mc, 1.9, WHITE);
    p.text(pos2(r.min.x + 25.0, r.center().y - 0.5), Align2::LEFT_CENTER, "Recordbox", FontId::proportional(17.0), WHITE);
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
    // Measured in reference 01/04/05 (pt from the window's left edge): 2 pt black margin, icon
    // strip to 37, tree pane to 248, track list 253..1475 with its scroll bar to 1487, right
    // icon strip 1487..1512. The top row of the tree area is 21 pt high with a 1 pt line under it.
    let strip = Rect::from_min_size(full.min, vec2(37.0, full.height()));
    let tree = Rect::from_min_max(pos2(strip.max.x, full.min.y), pos2(full.min.x + 248.0, full.max.y));
    let right_strip = Rect::from_min_max(pos2(full.max.x - 25.0, full.min.y), full.max);
    // Information window (268 pt) at the far right, My Tag (230 pt) to its left.
    let info_w = if app.prefs.info_panel { 268.0 } else { 0.0 };
    let info = Rect::from_min_max(pos2(right_strip.min.x - info_w - 2.5, full.min.y), pos2(right_strip.min.x - 2.5, full.max.y));
    let mytag_w = if app.prefs.show_mytag { 230.0 } else { 0.0 };
    let mytag_right = if app.prefs.info_panel { info.min.x - 2.5 } else { right_strip.min.x };
    let mytag = Rect::from_min_max(pos2(mytag_right - mytag_w, full.min.y), pos2(mytag_right, full.max.y));
    let list = Rect::from_min_max(pos2(full.min.x + 253.0, full.min.y), pos2(mytag.min.x, full.max.y));
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    p.rect_filled(Rect::from_min_max(pos2(full.min.x + 2.0, full.min.y), tree.max), 0.0, BG2);
    p.rect_filled(right_strip, 0.0, BG2);
    // Top row of the tree area: collapse chevron and sub-window tile.
    crate::icons_src::chevron(&p, Rect::from_min_size(pos2(full.min.x + 12.0, full.min.y + 5.0), vec2(6.0, 10.0)), OFF);
    let wt = Rect::from_min_size(pos2(full.min.x + 50.0, full.min.y + 2.5), vec2(16.0, 16.0));
    p.rect_stroke(wt, 1.5, Stroke::new(1.0, OFF), egui::StrokeKind::Inside);
    crate::icons_src::window_split(&p, wt, WHITE, BG2);
    p.rect_filled(Rect::from_min_max(pos2(full.min.x, full.min.y + 21.0), pos2(full.min.x + 253.0, full.min.y + 22.0)), 0.0, BG);

    icon_strip(app, ui, strip);
    let mut drop_targets: Vec<(Rect, i64)> = vec![];
    // Tree rows span 37..236 pt (the Collection pane 38..245) from 22 pt below the top.
    let tree_inner = if app.side == SideMode::Collection {
        p.rect_filled(Rect::from_min_max(pos2(full.min.x + 246.0, full.min.y + 22.0), tree.max), 0.0, BG);
        Rect::from_min_max(pos2(full.min.x + 37.0, full.min.y + 22.0), pos2(full.min.x + 246.0, full.max.y - 4.0))
    } else {
        Rect::from_min_max(pos2(full.min.x + 37.0, full.min.y + 22.0), pos2(full.min.x + 236.0, full.max.y - 4.0))
    };
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(tree_inner));
    tui.spacing_mut().item_spacing.y = 0.0;
    tree_pane(app, &mut tui, &mut drop_targets);
    // A playlist drag that ends anywhere other than on a tree row is cancelled; while it runs
    // its name follows the pointer.
    if let Some(id) = app.node_drag {
        if ui.input(|i| i.pointer.any_released()) {
            app.node_drag = None;
        } else if let (Some(pp), Some(n)) = (ui.ctx().pointer_latest_pos(), app.node(id)) {
            ui.painter().text(pp + vec2(12.0, 4.0), Align2::LEFT_CENTER, &n.name, rb(11.5), WHITE);
        }
    }
    let track_view = !matches!(app.view, View::Device { .. } | View::DeviceHistory { .. } | View::Explorer(_) | View::Source { .. });
    // Playlist Palette: a 26 pt strip of four slots under the track list.
    let list = if app.prefs.palette_open && track_view {
        let pr = Rect::from_min_max(pos2(list.min.x, list.max.y - 26.0), list.max);
        crate::ui_library_ext::palette_bar(app, ui, pr, &mut drop_targets);
        Rect::from_min_max(list.min, pos2(list.max.x, pr.min.y - 2.0))
    } else {
        list
    };
    let list = if app.prefs.sub_browser && track_view {
        let split = list.min.y + (list.height() * 0.58).round();
        let sub = Rect::from_min_max(pos2(list.min.x, split + 4.0), list.max);
        sub_browser(app, ui, sub);
        let main = Rect::from_min_max(list.min, pos2(list.max.x, split));
        // Tracks dragged from the sub-browser can be dropped into the main playlist.
        if app.sub_drag {
            if let Some(pl) = app.is_manual_playlist() {
                drop_targets.push((main, pl));
            }
        }
        main
    } else {
        list
    };
    let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(list));
    match app.view.clone() {
        View::Device { root, node } => device_list(app, &mut lui, &root, node, None),
        View::DeviceHistory { root, id } => device_list(app, &mut lui, &root, None, Some(id)),
        View::Explorer(dir) => explorer_list(app, &mut lui, &dir),
        View::Source { kind, node } => crate::ui_library_ext::source_list(app, &mut lui, kind, &node),
        _ => track_list(app, &mut lui),
    }
    if app.prefs.show_mytag {
        p.rect_filled(mytag, 0.0, BG2);
        let mut mui = ui.new_child(egui::UiBuilder::new().max_rect(mytag.shrink(6.0)));
        mytag_panel(app, &mut mui);
    }
    if app.prefs.info_panel {
        p.rect_filled(info, 0.0, BG2);
        let mut iui = ui.new_child(egui::UiBuilder::new().max_rect(info.shrink(6.0)));
        crate::ui_library_ext::info_panel(app, &mut iui);
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

type IconFn = fn(&egui::Painter, Rect, Color32);

/// What a source tile opens.
#[derive(Clone, Copy, PartialEq)]
enum Tile {
    Side(SideMode),
    Matching,
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
        (src::matching, Tile::Matching, "Matching"),
        (src::tag_list, Tile::TagList, "Tag List"),
        (src::hot_cue_bank, Tile::HotCueBank, "Hot Cue Bank List"),
        (src::itunes, Tile::Side(SideMode::Itunes), "iTunes"),
        (src::xml, Tile::Side(SideMode::RekordboxXml), "rekordbox xml"),
        (src::explorer, Tile::Side(SideMode::Explorer), "Explorer"),
        (src::device, Tile::Side(SideMode::Devices), "Devices"),
        (src::history, Tile::Side(SideMode::History), "Histories"),
        (src::recording, Tile::Off, "Recordings (not available in this app)"),
    ];
    // The highlight is bright while a track row holds the selection, muted otherwise (01 / 04).
    let hi = if app.selected.is_empty() { ROW_SEL } else { TILE_ON };
    for (i, (draw, tile, name)) in items.iter().filter(|it| app.prefs.tree_item(it.2)).collect::<Vec<_>>().into_iter().enumerate() {
        let c = pos2(r.min.x + 16.0, r.min.y + 34.0 + i as f32 * 20.0);
        let rr = Rect::from_center_size(c, vec2(18.0, 18.0));
        let on = match tile {
            Tile::Side(SideMode::Related) => app.side == SideMode::Related && app.view != View::Matching,
            Tile::Side(SideMode::Playlists) => app.side == SideMode::Playlists && app.view != View::TagList,
            Tile::Side(m) => app.side == *m,
            Tile::Matching => app.view == View::Matching,
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
        let resp = if app.prefs.tooltips { resp.on_hover_text(*name) } else { resp };
        if resp.clicked() {
            match *tile {
                Tile::Off => {}
                Tile::Matching => {
                    app.side = SideMode::Related;
                    app.set_view(View::Matching);
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
                        SideMode::Playlists => {}
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

/// Right icon strip: My Tag, Related, Matching, Information, Sub-browser at 193/227/261/294/328 pt.
fn right_icons(app: &mut App, ui: &mut Ui, r: Rect) {
    use crate::icons_src as src;
    let items: [(src::SrcIcon, &str); 5] = [
        (src::mytag, "Display My Tag configuration window."),
        (src::related, "Display related track list window."),
        (src::matching, "Set a good match combination of tracks.\nSee also Related Track List window.(Matching)"),
        (src::info, "Display information window."),
        (src::sub_browser, "Sub-browser"),
    ];
    for (i, (draw, tipt)) in items.iter().enumerate() {
        // References: 18 pt tiles centred 12 pt into the strip on a 34 pt pitch from 189 pt.
        let rr = Rect::from_center_size(pos2(r.min.x + 12.0, r.min.y + 189.0 + 34.0 * i as f32), vec2(18.0, 18.0));
        let on = match i {
            0 => app.prefs.show_mytag,
            1 => app.view == View::Related,
            2 => app.view == View::Matching,
            3 => app.prefs.info_panel,
            4 => app.prefs.sub_browser,
            _ => false,
        };
        let resp = ui.interact(rr, ui.id().with(("ricon", i)), Sense::click());
        // Same tiles as the source strip (skins: mytag, relatedTrack, trackInfo, subBrowse).
        // Lit like the source tiles; otherwise a #565656 border on #212121. My Tag is dimmed
        // on the Devices list (reference 05).
        let dim = i == 0 && !on && matches!(app.view, View::Device { .. } | View::DeviceHistory { .. });
        let bg = if on {
            ui.painter().rect_filled(rr, 1.5, TILE_ON);
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
            match i {
                0 => {
                    app.prefs.show_mytag = !app.prefs.show_mytag;
                    app.save_prefs();
                }
                1 => {
                    if app.view == View::Related {
                        app.side = SideMode::Collection;
                        app.set_view(View::Collection);
                    } else {
                        app.side = SideMode::Related;
                        app.set_view(View::Related);
                    }
                }
                2 => {
                    if app.view == View::Matching {
                        app.side = SideMode::Collection;
                        app.set_view(View::Collection);
                    } else {
                        app.side = SideMode::Related;
                        app.set_view(View::Matching);
                    }
                }
                3 => {
                    app.prefs.info_panel = !app.prefs.info_panel;
                    app.save_prefs();
                }
                4 => {
                    app.prefs.sub_browser = !app.prefs.sub_browser;
                    app.save_prefs();
                }
                _ => {}
            }
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
    }
    let right = if count.is_some() { rr.max.x - 62.0 } else { rr.max.x - 4.0 };
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

fn tree_pane(app: &mut App, ui: &mut Ui, drop: &mut Vec<(Rect, i64)>) {
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
            let sresp = ui.interact(st, ui.id().with("tree_search_tile"), Sense::click());
            if app.ext.tree_search_open {
                ui.painter().rect_filled(st, 1.5, TILE_ON);
                ui.painter().rect_stroke(st, 1.5, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
            } else {
                ui.painter().rect_stroke(st, 1.5, Stroke::new(1.0, if sresp.hovered() { hex(0x8C8C8C) } else { OFF }), egui::StrokeKind::Inside);
            }
            crate::icons_src::playlist_search(ui.painter(), st, if app.ext.tree_search_open { WHITE } else { hex(0xC7C7C7) }, if app.ext.tree_search_open { TILE_ON } else { BG2 });
            if sresp.clicked() {
                app.ext.tree_search_open = !app.ext.tree_search_open;
                if app.ext.tree_search_open {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("tree_search_focus"), true));
                } else {
                    app.ext.tree_search.clear();
                }
            }
            let tv = Rect::from_min_max(pos2(row.min.x + 26.0, row.min.y), pos2(row.min.x + 126.0, row.min.y + 23.0));
            ui.painter().rect_filled(tv, 0.0, TILE_ON);
            ui.painter().text(tv.center(), Align2::CENTER_CENTER, "Tree View", rb(12.0), WHITE);
            let cv = Rect::from_min_max(pos2(tv.max.x, tv.min.y), pos2(row.max.x + 12.0, tv.max.y));
            let wide = ui.painter().with_clip_rect(cv);
            wide.rect_filled(cv, 0.0, hex(0x323232));
            crate::icons_src::tree_lock(&wide, pos2(cv.min.x + 11.0, cv.center().y - 0.5), DIM, hex(0x323232));
            wide.text(pos2(cv.min.x + 19.5, cv.center().y), Align2::LEFT_CENTER, "Column View", rb(12.0), DIM);
            ui.painter().with_clip_rect(Rect::from_min_max(row.min, pos2(cv.max.x, row.max.y))).rect_filled(Rect::from_min_max(pos2(row.min.x - 37.0, row.min.y + 23.0), pos2(cv.max.x, row.max.y)), 0.0, BG);
            if app.ext.tree_search_open {
                crate::ui_library_ext::tree_search_field(app, ui);
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let all_sel = app.view == View::Collection;
                let all = tree_row(ui, 0, "All Tracks", Some(app.tracks.len()), all_sel, TreeIcon::Note, 15.0);
                if all.clicked() {
                    app.set_view(View::Collection);
                }
                all.context_menu(|ui| crate::ui_library_ext::collection_menu(app, ui));
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
    let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == parent).cloned().collect();
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
        let resp = tree_row(ui, depth, &n.name, count, sel, icon, 15.0);
        if n.kind == NodeKind::Playlist {
            drop.push((resp.rect, n.id));
        }
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
                        app.toast_err(format!("{name} also has rekordbox's OneLibrary database, which Recordbox can't read yet, so it can't tell which files are unused."));
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
            if ui.button("Import Cue Changes from Players…").on_hover_text("Hot cues and memory cues set or moved on a DJ player since the last export from Recordbox.").clicked() {
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
            ui.label(RichText::new("Has a OneLibrary database that Recordbox does not update yet.").size(10.5).color(DIM)).on_hover_text("Written by rekordbox 7.2.11 or later. Players that read OneLibrary will not show changes made by Recordbox.");
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
        View::Related => format!("Related Tracks — {}", app.related_source().and_then(|id| app.track(id)).map(|t| t.display_title()).unwrap_or_else(|| "load or select a track".into())),
        View::Category { field, value } => format!("{} › {}", ["Date Added", "Genre", "Artist", "Album"][*field as usize % 4], if value.is_empty() { "(none)" } else { value }),
    }
}

fn track_list(app: &mut App, ui: &mut Ui) {
    if matches!(app.view, View::Related | View::Matching) && app.related_built_for != app.related_source() {
        app.list_dirty = true;
    }
    let had_edit = app.ext.edit.is_some();
    app.ext.edit_drawn = false;
    if app.list_dirty {
        app.rebuild_list();
    }
    let r = ui.max_rect();
    // Title bar: the first 26 pt of the browser, through to the window's right edge; a divider
    // 340 pt in; on the right the PLAYER A selector, four view-mode tiles and the search box
    // (measured from the list's right edge, 1487 pt in the references).
    let bar = Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 26.0));
    let p = ui.painter().clone();
    p.rect_filled(bar, 0.0, BG2);
    p.text(pos2(bar.min.x + 10.0, bar.center().y), Align2::LEFT_CENTER, format!("{} ({} Tracks)", view_title(app), app.list.len()), rb(TITLE_FONT), WHITE);
    let rx = bar.max.x;
    p.rect_filled(Rect::from_min_max(pos2(bar.min.x + 340.0, bar.min.y), pos2(bar.min.x + 341.0, bar.max.y)), 0.0, BG);
    // PLAYER A (load target; this app has one player)
    // Reference 04: a 16 pt high #282828 box 505..425 pt from the right, its last 16 pt a
    // separate chevron part.
    let pa = Rect::from_min_max(pos2(rx - 505.0, bar.min.y + 5.0), pos2(rx - 425.0, bar.min.y + 21.0));
    let two = app.two_players();
    p.rect_filled(Rect::from_min_max(pa.min, pos2(pa.max.x - 16.5, pa.max.y)), egui::CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, BTN);
    p.rect_filled(Rect::from_min_max(pos2(pa.max.x - 16.0, pa.min.y), pa.max), egui::CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, BTN);
    p.text(pos2(pa.min.x + 7.0, pa.center().y), Align2::LEFT_CENTER, if two && app.load_target == 1 { "PLAYER B" } else { "PLAYER A" }, rb_bold(9.75), WHITE);
    {
        let c = pos2(pa.max.x - 8.0, pa.center().y + 0.5);
        p.add(egui::Shape::line(vec![c + vec2(-3.6, -1.8), c + vec2(0.0, 1.8), c + vec2(3.6, -1.8)], Stroke::new(1.0, WHITE)));
    }
    let par = ui.interact(pa, ui.id().with("playera"), Sense::click());
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
    // Playlist Palette switch, left of PLAYER A.
    {
        let pr = Rect::from_center_size(pos2(rx - 522.0, bar.center().y), vec2(18.0, 18.0));
        let presp = ui.interact(pr, ui.id().with("palette_toggle"), Sense::click());
        if app.prefs.palette_open {
            p.rect_filled(pr, 2.0, BLUE);
        } else {
            p.rect_filled(pr, 2.0, if presp.hovered() { hex(0x3A3A3A) } else { BTN });
        }
        crate::icons_src::bullet_rows(&p, pr, WHITE);
        let presp = if app.prefs.tooltips { presp.on_hover_text("Show/Hide Playlist Palette") } else { presp };
        if presp.clicked() {
            app.prefs.palette_open = !app.prefs.palette_open;
            app.save_prefs();
        }
    }
    // View-mode icons: track list with artwork, track list without artwork, column browser and
    // sub-browser (the last two are not part of this version and say so).
    let with_art = app.prefs.columns.contains(&Col::Artwork);
    let vic: [(f32, IconFn, Option<bool>, &str); 4] = [
        (396.0, crate::icons_src::bullet_rows4, Some(with_art && !app.prefs.artwork_full), "Show the upper part of the artwork."),
        (358.0, crate::icons_src::bullet_rows, Some(with_art && app.prefs.artwork_full), "Show the entire artwork."),
        (299.0, crate::icons_src::columns3, Some(app.prefs.column_browser), "Column browser"),
        (261.0, crate::icons_src::side_pane, Some(app.prefs.sub_browser), "Sub-browser"),
    ];
    for (i, (dx, draw, on, tipt)) in vic.into_iter().enumerate() {
        // Skins artworkPart / artworkAll / subBrowse: 18 pt tiles, 1.5 pt corners, a 1 pt
        // border that is #646464, or blue when lit; the glyph stays white.
        let vr = Rect::from_center_size(pos2(rx - dx, bar.center().y), vec2(18.0, 18.0));
        let resp = ui.interact(vr, ui.id().with(("viewmode", i)), if on.is_some() { Sense::click() } else { Sense::hover() });
        p.rect_stroke(vr, 1.5, Stroke::new(1.0, if on == Some(true) { TILE_ON } else if resp.hovered() { hex(0x8C8C8C) } else { OFF }), egui::StrokeKind::Inside);
        draw(&p, vr, WHITE);
        let resp = if app.prefs.tooltips || on.is_none() { resp.on_hover_text(tipt) } else { resp };
        if resp.clicked() && i == 3 {
            app.prefs.sub_browser = !app.prefs.sub_browser;
            app.save_prefs();
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
    let sr = Rect::from_min_max(pos2(rx - 232.0, bar.min.y + 1.0), pos2(rx - 20.0, bar.max.y - 1.0));
    p.rect_filled(sr, 0.0, BG);
    // Skin searchSelect: a 25 x 14 pt magnifier with its scope triangle, #646464.
    crate::icons_src::search_select(&p, Rect::from_min_size(pos2(sr.min.x + 9.0, sr.min.y + 5.5), vec2(25.0, 14.0)), OFF);
    let scope_btn = ui.interact(Rect::from_min_max(sr.min, pos2(sr.min.x + 35.0, sr.max.y)), ui.id().with("search_scope"), Sense::click());
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
    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
        te.request_focus();
    }
    // Reference 01: the column menu sits in the icon strip's column, 12.75 pt from the window edge.
    let clip = ui.clip_rect();
    ui.set_clip_rect(ui.ctx().screen_rect());
    // Skin brows_menu tile: 18 pt, 2 pt corners, #282828, centred 12.5 pt into the right strip.
    let cmr = Rect::from_center_size(pos2(ui.ctx().screen_rect().max.x - 12.5, bar.min.y + 14.0), vec2(18.0, 18.0));
    let cm = ui.interact(cmr, ui.id().with("colmenu"), Sense::click());
    ui.painter().rect_filled(cmr, 2.0, if cm.hovered() { hex(0x3A3A3A) } else { BTN });
    crate::icons_src::bullet_rows(ui.painter(), cmr, WHITE);
    ui.set_clip_rect(clip);
    dropdown(ui, &cm, "cols", |ui| {
        ui.label(RichText::new("Columns").strong());
        crate::ui_library_ext::column_checklist(app, ui);
        if ui.button("Auto-size all columns").clicked() {
            for col in app.prefs.columns.clone() {
                crate::ui_library_ext::autosize_column(app, ui, col);
            }
            app.save_prefs();
        }
    });
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
                crate::icons_src::funnel(ui.painter(), pos2(cr.min.x + 15.0, cr.center().y - 1.0), if app.filter_open { TILE_ON } else { DIM });
                ui.painter().rect_filled(Rect::from_min_max(pos2(cr.min.x + 29.0, cr.min.y), pos2(cr.min.x + 30.0, cr.max.y)), 0.0, BG);
            }
            let label = if *c == Col::Status { "" } else { c.label() };
            ui.painter().with_clip_rect(cr).text(pos2(cr.min.x + 7.0, cr.center().y), Align2::LEFT_CENTER, label, rb(TITLE_FONT), if sorted { TILE_ON } else { WHITE });
            if sorted {
                // Skins sortUp / sortDown: the arrow sits 14 pt from the column's right edge.
                let ac = if label.is_empty() { pos2(cr.min.x + 44.0, cr.center().y) } else { pos2(cr.max.x - 14.0, cr.center().y) };
                crate::icons_src::sort_arrow(&ui.painter().with_clip_rect(cr), ac, app.prefs.sort_asc, TILE_ON);
            }
            ui.painter().rect_filled(Rect::from_min_max(pos2(cr.max.x - 1.0, cr.min.y), cr.max), 0.0, BG);
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
    if !ui.ctx().wants_keyboard_input() {
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
    let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if !dropped.is_empty() {
        app.import_paths(dropped, app.is_manual_playlist());
    }
}

/// Sub-browser: a second track list (Collection or any playlist) under the main one. Rows drag
/// onto playlists in the tree or into the main list's playlist; double-click loads.
fn sub_browser(app: &mut App, ui: &mut Ui, r: Rect) {
    ui.painter().rect_filled(r, 0.0, BG1);
    let view = app.sub_view.clone();
    let name = match &view {
        View::Playlist(id) => app.node(*id).map(|n| n.name.clone()).unwrap_or_else(|| "Collection".into()),
        _ => "Collection".into(),
    };
    // The list is re-read at most once a second (playlists come from the database).
    let cache_id = egui::Id::new("sub_cache");
    let cached: Option<(String, f64, std::sync::Arc<Vec<i64>>)> = ui.ctx().data(|d| d.get_temp(cache_id));
    let now = ui.input(|i| i.time);
    let key = format!("{view:?}");
    let ids = match cached.filter(|c| c.0 == key && now - c.1 < 1.0) {
        Some(c) => c.2,
        None => {
            let v = std::sync::Arc::new(app.view_tracks_of(&view));
            ui.ctx().data_mut(|d| d.insert_temp(cache_id, (key, now, v.clone())));
            v
        }
    };
    let bar = Rect::from_min_size(r.min, vec2(r.width(), 22.0));
    ui.painter().rect_filled(bar, 0.0, BG2);
    ui.painter().text(pos2(bar.min.x + 10.5, bar.center().y), Align2::LEFT_CENTER, format!("{name} ({} Tracks)", ids.len()), rb(11.5), WHITE);
    let pick = Rect::from_min_max(pos2(bar.max.x - 190.0, bar.min.y + 3.0), pos2(bar.max.x - 8.0, bar.max.y - 3.0));
    ui.painter().rect_stroke(pick, 2.0, Stroke::new(1.0, DIM), egui::StrokeKind::Inside);
    ui.painter().text(pos2(pick.min.x + 6.0, pick.center().y), Align2::LEFT_CENTER, "Show list…", rb(11.0), TEXT);
    crate::icons::chevron_down(ui.painter(), Rect::from_center_size(pos2(pick.max.x - 8.0, pick.center().y), vec2(10.0, 8.0)), TEXT);
    let presp = ui.interact(pick, ui.id().with("sub_pick"), Sense::click());
    dropdown(ui, &presp, "sub_pick_menu", |ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            if ui.selectable_label(view == View::Collection, "Collection").clicked() {
                app.sub_view = View::Collection;
                app.sub_sel.clear();
            }
            fn add(app: &mut App, ui: &mut Ui, parent: i64, depth: usize) {
                let kids: Vec<(i64, String, NodeKind)> = app.nodes.iter().filter(|n| n.parent == parent).map(|n| (n.id, n.name.clone(), n.kind.clone())).collect();
                for (id, name, kind) in kids {
                    let label = format!("{}{}{name}", "    ".repeat(depth), if kind == NodeKind::Folder { "▸ " } else { "" });
                    if ui.selectable_label(app.sub_view == View::Playlist(id), label).clicked() {
                        app.sub_view = View::Playlist(id);
                        app.sub_sel.clear();
                    }
                    if kind == NodeKind::Folder {
                        add(app, ui, id, depth + 1);
                    }
                }
            }
            add(app, ui, 0, 0);
        });
    });
    // Header and rows: Track Title, Artist, BPM, Key, Time.
    let body = Rect::from_min_max(pos2(r.min.x, bar.max.y), r.max);
    let fixed = 60.0 + 50.0 + 56.0;
    let tw = ((body.width() - fixed - 12.0) * 0.55).max(80.0);
    let aw = (body.width() - fixed - 12.0 - tw).max(60.0);
    let cols: [(&str, f32, bool); 5] = [("Track Title", tw, false), ("Artist", aw, false), ("BPM", 60.0, true), ("Key", 50.0, false), ("Time", 56.0, true)];
    let hr = Rect::from_min_size(body.min, vec2(body.width(), 13.5));
    ui.painter().rect_filled(hr, 0.0, BG2);
    let mut x = hr.min.x + 8.0;
    for (label, w, _) in cols {
        ui.painter().text(pos2(x, hr.center().y), Align2::LEFT_CENTER, label, rb(11.0), WHITE);
        x += w;
    }
    let rows = Rect::from_min_max(pos2(body.min.x, hr.max.y), body.max);
    let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(rows));
    rui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let alnum = app.prefs.key_alnum;
    egui::ScrollArea::vertical().id_salt("sub_rows").auto_shrink([false, false]).show_rows(&mut rui, 19.0, ids.len(), |ui, range| {
        for i in range {
            let id = ids[i];
            let Some(t) = app.track(id).cloned() else { continue };
            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 19.0), Sense::click_and_drag());
            let sel = app.sub_sel.contains(&id);
            if sel {
                ui.painter().rect_filled(rr, 0.0, ROW_SEL);
            } else if i % 2 == 1 {
                ui.painter().rect_filled(rr, 0.0, BG2.gamma_multiply(0.5));
            }
            let mut x = rr.min.x + 8.0;
            let texts = [t.display_title(), t.artist.clone(), if t.bpm > 0.0 { format!("{:.2}", t.bpm) } else { String::new() }, t.key_label(alnum), if t.duration > 0.0 { fmt_time(t.duration, false) } else { String::new() }];
            for ((_, w, right), txt) in cols.iter().zip(texts) {
                let cell = Rect::from_min_size(pos2(x, rr.min.y), vec2(*w - 6.0, rr.height()));
                let p = ui.painter().with_clip_rect(cell);
                if *right {
                    p.text(pos2(cell.max.x - 4.0, cell.center().y), Align2::RIGHT_CENTER, txt, rb(11.5), TEXT);
                } else {
                    p.text(pos2(cell.min.x, cell.center().y), Align2::LEFT_CENTER, txt, rb(11.5), if t.missing { DIM } else { TEXT });
                }
                x += w;
            }
            if resp.clicked() {
                let cmd = ui.input(|i| i.modifiers.command);
                if cmd {
                    if !app.sub_sel.remove(&id) {
                        app.sub_sel.insert(id);
                    }
                } else {
                    app.sub_sel.clear();
                    app.sub_sel.insert(id);
                }
            }
            if resp.double_clicked() {
                app.load_track(id);
            }
            if resp.drag_started() && app.drag.is_none() {
                let ids: Vec<i64> = if sel { ids.iter().copied().filter(|x| app.sub_sel.contains(x)).collect() } else { vec![id] };
                app.drag = Some(ids);
                app.sub_drag = true;
            }
        }
    });
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
                // Stored preview: 400 display-level columns of 4 bytes; resample to the cell width.
                let src: Vec<[u8; 4]> = pv.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                let len = src.len().max(1);
                let cols: Vec<[u8; 4]> = (0..n)
                    .map(|i| {
                        let a = i * len / n;
                        let b = ((i + 1) * len / n).max(a + 1).min(len);
                        let mut m = [0u8; 4];
                        for f in src.get(a..b).unwrap_or(&[]) {
                            for k in 0..4 {
                                m[k] = m[k].max(f[k]);
                            }
                        }
                        m
                    })
                    .collect();
                let base = r.max.y - 2.5;
                let hh = (r.height() - 6.0).max(4.0);
                for (k, v) in cols.iter().enumerate() {
                    let x = x0 + k as f32 + 0.5;
                    for (h, cc) in crate::ui_player::wave_bars(app.prefs.wave_color, *v) {
                        p.line_segment([pos2(x, base), pos2(x, base - h * hh)], Stroke::new(1.0, cc));
                    }
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

fn mytag_panel(app: &mut App, ui: &mut Ui) {
    let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new("My Tag").strong().color(WHITE));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+ Column").on_hover_text("Create a new My Tag category").clicked() {
                let id = app.db.add_mytag(0, "Untitled Column").unwrap_or(0);
                app.reload_tags();
                app.dialogs.rename_tag = Some((id, "Untitled Column".into()));
            }
        });
    });
    ui.label(RichText::new(if sel.is_empty() { "Select tracks, then click tags to set them. Alt-click a tag to filter the list.".to_string() } else { format!("{} track(s) selected", sel.len()) }).size(10.5).color(DIM));
    ui.add_space(4.0);
    let cats: Vec<MyTag> = app.mytags.iter().filter(|t| t.parent == 0).cloned().collect();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for c in cats {
            ui.add_space(4.0);
            if let Some((rid, name)) = app.dialogs.rename_tag.clone() {
                if rid == c.id {
                    let mut n = name;
                    let r = ui.text_edit_singleline(&mut n);
                    // Whether the field just gave up focus must be read before focus is asked for again.
                    let gave_up = r.lost_focus();
                    if !gave_up {
                        r.request_focus();
                    }
                    app.dialogs.rename_tag = Some((rid, n.clone()));
                    if gave_up {
                        let _ = app.db.rename_mytag(rid, n.trim());
                        app.dialogs.rename_tag = None;
                        app.reload_tags();
                    }
                    continue;
                }
            }
            let hr = ui.label(RichText::new(&c.name).size(11.5).color(DIM2).strong());
            let cc = c.clone();
            hr.context_menu(|ui| {
                if ui.button("Add tag").clicked() {
                    app.dialogs.new_tag = Some((cc.id, String::new()));
                    ui.close_menu();
                }
                if ui.button("Rename").clicked() {
                    app.dialogs.rename_tag = Some((cc.id, cc.name.clone()));
                    ui.close_menu();
                }
                if ui.button("Delete category").clicked() {
                    let _ = app.db.delete_mytag(cc.id);
                    app.reload_tags();
                    ui.close_menu();
                }
            });
            let tags: Vec<MyTag> = app.mytags.iter().filter(|t| t.parent == c.id).cloned().collect();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for tg in tags {
                    if let Some((rid, name)) = app.dialogs.rename_tag.clone() {
                        if rid == tg.id {
                            let mut n = name;
                            let r = ui.add(egui::TextEdit::singleline(&mut n).desired_width(90.0));
                            // Whether the field just gave up focus must be read before focus is asked for again.
                            let gave_up = r.lost_focus();
                            if !gave_up {
                                r.request_focus();
                            }
                            app.dialogs.rename_tag = Some((rid, n.clone()));
                            if gave_up {
                                let _ = app.db.rename_mytag(rid, n.trim());
                                app.dialogs.rename_tag = None;
                                app.reload_tags();
                            }
                            continue;
                        }
                    }
                    let count = sel.iter().filter(|t| app.track_tags.get(t).map(|s| s.contains(&tg.id)).unwrap_or(false)).count();
                    let on = !sel.is_empty() && count == sel.len();
                    let partial = count > 0 && !on;
                    let filtering = app.filter_tags.contains(&tg.id);
                    let fill = if filtering { hex(0x814B00) } else if on { BLUE } else if partial { hex(0x0F4274) } else { BTN };
                    let b = ui.add(egui::Button::new(RichText::new(&tg.name).size(11.0).color(WHITE)).fill(fill));
                    if b.clicked() {
                        if ui.input(|i| i.modifiers.alt) {
                            if !app.filter_tags.insert(tg.id) {
                                app.filter_tags.remove(&tg.id);
                            }
                            app.filter_open = true;
                            app.list_dirty = true;
                        } else if !sel.is_empty() {
                            app.set_tag(&sel, tg.id, !on);
                        }
                    }
                    let t2 = tg.clone();
                    b.context_menu(|ui| {
                        if ui.button("Filter the list by this tag").clicked() {
                            app.filter_tags.insert(t2.id);
                            app.filter_open = true;
                            app.list_dirty = true;
                            ui.close_menu();
                        }
                        if ui.button("Rename").clicked() {
                            app.dialogs.rename_tag = Some((t2.id, t2.name.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Delete").clicked() {
                            let _ = app.db.delete_mytag(t2.id);
                            app.reload_tags();
                            ui.close_menu();
                        }
                    });
                }
                if let Some((cid, name)) = app.dialogs.new_tag.clone() {
                    if cid == c.id {
                        let mut n = name;
                        let r = ui.add(egui::TextEdit::singleline(&mut n).desired_width(90.0).hint_text("New tag"));
                        // Whether the field just gave up focus must be read before focus is asked for again.
                        let gave_up = r.lost_focus();
                        if !gave_up {
                            r.request_focus();
                        }
                        app.dialogs.new_tag = Some((cid, n.clone()));
                        if gave_up {
                            if !n.trim().is_empty() {
                                let _ = app.db.add_mytag(cid, n.trim());
                                app.reload_tags();
                            }
                            app.dialogs.new_tag = None;
                        }
                        return;
                    }
                }
                if ui.add(egui::Button::new(RichText::new("+").size(11.0)).fill(BG1)).on_hover_text("Create a new My Tag.").clicked() {
                    app.dialogs.new_tag = Some((c.id, String::new()));
                }
            });
        }
        ui.add_space(8.0);
        if !sel.is_empty() && ui.small_button("Add \"My Tag\" to the \"Comments\"").clicked() {
            for id in &sel {
                let tags = app.tag_text(*id);
                if tags.is_empty() {
                    continue;
                }
                if let Some(t) = app.track_mut(*id) {
                    let add = format!("/* {tags} */");
                    if !t.comment.contains(&add) {
                        t.comment = if t.comment.is_empty() { add } else { format!("{} {}", t.comment, add) };
                    }
                }
                app.save_track(*id);
            }
        }
    });
}

pub fn track_menu_items(app: &mut App, ui: &mut Ui, sel: &[i64]) {
    crate::ui_library_ext::track_menu(app, ui, sel);
}
