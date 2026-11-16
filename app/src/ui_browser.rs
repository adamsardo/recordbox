//! Header bar, browser (sidebar tree, track list, My Tag panel, devices) and footer.

use eframe::egui;
use crate::app::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_player::{button, star};
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui};
use std::collections::HashSet;

/// Track list row pitch, as measured in the original (15 pt).
pub const ROW_H: f32 = 15.0;

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
/// Header bar, 29 pt. Positions measured in the original (x in pt from the window's left edge;
/// right-hand items measured from the right edge of a 1512 pt window).
pub fn header_bar(app: &mut App, ui: &mut Ui) {
    let r = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, BG1);
    let cy = r.center().y;
    let tips_on = app.prefs.tooltips;
    let tipf = move |resp: egui::Response, t: &str| if tips_on { resp.on_hover_text(t) } else { resp };
    // Mode: EXPORT + chevron at 115 pt.
    p.text(pos2(r.min.x + 16.0, cy), Align2::LEFT_CENTER, "EXPORT", bold(11.0), WHITE);
    let mode_r = Rect::from_min_max(pos2(r.min.x + 8.0, r.min.y), pos2(r.min.x + 124.0, r.max.y));
    crate::icons::chevron_down(&p, Rect::from_center_size(pos2(r.min.x + 115.0, cy), vec2(14.0, 12.0)), WHITE);
    let _ = tipf(ui.interact(mode_r, ui.id().with("mode"), Sense::hover()), "This app covers EXPORT mode: library, preparation and USB export.");
    // Layout selector: icon at 145 pt, chevron at 171 pt.
    let lay_r = Rect::from_min_max(pos2(r.min.x + 134.0, r.min.y + 3.0), pos2(r.min.x + 180.0, r.max.y - 3.0));
    let lay = ui.interact(lay_r, ui.id().with("layout"), Sense::click());
    if lay.hovered() {
        p.rect_filled(lay_r, 2.0, hex(0x1E1E1E));
    }
    crate::icons::layout_icon(&p, Rect::from_center_size(pos2(r.min.x + 145.0, cy), vec2(18.0, 14.0)), WHITE);
    crate::icons::chevron_down(&p, Rect::from_center_size(pos2(r.min.x + 171.0, cy), vec2(14.0, 12.0)), WHITE);
    dropdown(ui, &lay, "layout", |ui| {
        if ui.selectable_label(!app.prefs.browser_only, "1 Player").clicked() {
            app.prefs.browser_only = false;
            app.save_prefs();
        }
        if ui.selectable_label(app.prefs.browser_only, "Full Browser").clicked() {
            app.prefs.browser_only = true;
            app.save_prefs();
        }
    });
    // Auto-analysis switch in the place of the original's circle icon (320 pt).
    let rec_r = Rect::from_center_size(pos2(r.min.x + 320.0, cy), vec2(22.0, 22.0));
    let rec = ui.interact(rec_r, ui.id().with("autoan"), Sense::click());
    crate::icons::record(&p, rec_r, if app.prefs.auto_analysis { WHITE } else { OFF });
    if tipf(rec, if app.prefs.auto_analysis { "Auto Analysis On (click to turn off)" } else { "Auto Analysis Off (click to turn on)" }).clicked() {
        app.prefs.auto_analysis = !app.prefs.auto_analysis;
        app.save_prefs();
    }
    // Right side, measured from the right edge: clock (−34), slider (−106..−72), meter (−204..−125),
    // sync mark (−223), gear (−261).
    let rx = r.max.x;
    let now = chrono::Local::now().format("%H:%M").to_string();
    p.text(pos2(rx - 20.0, cy), Align2::RIGHT_CENTER, now, FontId::proportional(11.5), WHITE);
    let gear_r = Rect::from_center_size(pos2(rx - 261.0, cy), vec2(22.0, 22.0));
    let gear = ui.interact(gear_r, ui.id().with("gear"), Sense::click());
    crate::icons::gear(&p, gear_r, WHITE);
    if tipf(gear, "Preferences").clicked() {
        app.dialogs.prefs_open = true;
    }
    // Sync / analysis activity mark (blue arc).
    let arc_c = pos2(rx - 223.0, cy + 1.0);
    let busy = app.analysis_total > 0 || app.export_job.is_some() || app.importing.is_some();
    let mut pts = vec![];
    let spin = if busy { ui.input(|i| i.time) as f32 * 4.0 } else { 0.0 };
    for i in 0..=16 {
        let a = std::f32::consts::PI * (0.9 + i as f32 * 1.2 / 16.0) + spin;
        pts.push(arc_c + vec2(a.cos(), a.sin()) * 7.0);
    }
    p.add(egui::Shape::line(pts, Stroke::new(2.4, BLUE)));
    if busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    // Level meter: level of the playing track at the play position.
    let meter = Rect::from_min_max(pos2(rx - 204.0, cy - 2.5), pos2(rx - 125.0, cy + 2.5));
    p.rect_filled(meter, 1.0, hex(0x3A3A3A));
    if app.playing() {
        if let Some(w) = &app.deck.wave {
            let f = (app.pos() * WAVE_FPS) as usize;
            if let Some(fr) = w.frames.get(f) {
                let lvl = (fr[0] as f32 / 255.0).min(1.0);
                p.rect_filled(Rect::from_min_max(meter.min, pos2(meter.min.x + meter.width() * lvl, meter.max.y)), 1.0, hex(0xB0B0B0));
            }
        }
    }
    // Master volume slider.
    let sl = Rect::from_min_max(pos2(rx - 106.0, cy - 6.0), pos2(rx - 72.0, cy + 6.0));
    let sresp = ui.interact(sl, ui.id().with("vol"), Sense::click_and_drag());
    let mut v = app.player.state.lock().volume;
    if sresp.dragged() || sresp.clicked() {
        if let Some(pp) = sresp.interact_pointer_pos() {
            v = ((pp.x - sl.min.x) / sl.width()).clamp(0.0, 1.0);
            app.player.state.lock().volume = v;
        }
    }
    p.line_segment([pos2(sl.min.x, cy), pos2(sl.max.x, cy)], Stroke::new(2.0, hex(0x3A3A3A)));
    p.line_segment([pos2(sl.min.x, cy), pos2(sl.min.x + sl.width() * v, cy)], Stroke::new(2.0, BLUE));
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
            if let Some(f) = rfd::FileDialog::new().set_file_name("PrepDeck-library-backup.db").save_file() {
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
        if ui.button("About PrepDeck").clicked() {
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
    let Ok(text) = std::fs::read_to_string(f) else { return };
    let base = f.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let paths: Vec<std::path::PathBuf> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let p = std::path::PathBuf::from(l);
            if p.is_absolute() {
                p
            } else {
                base.join(p)
            }
        })
        .filter(|p| p.exists())
        .collect();
    let name = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("Imported".into());
    let id = app.db.create_node(0, &name, NodeKind::Playlist, None).unwrap_or(0);
    app.reload_playlists();
    app.import_paths(paths, Some(id));
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
    let mc = pos2(r.min.x + 17.0, r.center().y);
    p.circle_stroke(mc, 6.0, Stroke::new(1.8, WHITE));
    p.circle_filled(mc, 2.2, WHITE);
    p.text(pos2(r.min.x + 27.0, r.center().y), Align2::LEFT_CENTER, "PrepDeck", bold(15.5), WHITE);
    // totals
    let (n, secs, bytes) = if let View::Device { .. } = app.view {
        (0usize, 0.0, 0u64)
    } else {
        app.list.iter().filter_map(|id| app.track(*id)).fold((0, 0.0, 0u64), |(n, s, b), t| (n + 1, s + t.duration, b + t.file_size))
    };
    let h = (secs / 3600.0).floor();
    let m = ((secs - h * 3600.0) / 60.0).floor();
    let total = format!("{n} Tracks, {} hours {} minutes, {:.1} GB", h, m, bytes as f64 / 1e9);
    p.text(pos2(r.max.x - 34.0, r.center().y), Align2::RIGHT_CENTER, total, FontId::proportional(11.0), DIM2);
    // status / messages
    let mid = Rect::from_min_max(pos2(r.min.x + 120.0, r.min.y + 3.0), pos2(r.max.x - 300.0, r.max.y - 3.0));
    app.toasts.retain(|t| t.until > std::time::Instant::now());
    if let Some(t) = app.toasts.last() {
        if t.error {
            p.rect_filled(mid, 0.0, CAUTION_BG);
            p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, &t.text, FontId::proportional(11.5), CAUTION_TX);
        } else {
            p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, &t.text, FontId::proportional(11.5), TEXT);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    } else if let Some(job) = &app.export_job {
        let (d, t, txt) = &job.last;
        p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, format!("Exporting {}/{} · {}", d, t, txt), FontId::proportional(11.5), TEXT);
    } else if app.analysis_total > 0 {
        let pending = app.pool.pending.load(std::sync::atomic::Ordering::Relaxed);
        p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, format!("Analyzing… {} of {}", app.analysis_total - pending, app.analysis_total), FontId::proportional(11.5), TEXT);
        let stop = ui.interact(Rect::from_min_size(pos2(mid.max.x - 120.0, mid.min.y), vec2(110.0, mid.height())), ui.id().with("stopan"), Sense::click());
        p.text(pos2(mid.max.x - 64.0, mid.center().y), Align2::CENTER_CENTER, "Stop Analyzing", FontId::proportional(11.0), BLUE);
        if stop.clicked() {
            app.pool.stop();
        }
    } else if app.importing.is_some() {
        p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, format!("Importing… {}", app.import_count), FontId::proportional(11.5), TEXT);
    }
}

// ------------------------------------------------------------------------------------ browser
/// Browser geometry measured in the original (pt): icon strip 0..26.5, tree pane 26.5..247,
/// track list 253..1485.5, right icon strip 1488..1512; top row of the tree area 20 pt high.
pub fn browser(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let strip = Rect::from_min_size(full.min, vec2(26.5, full.height()));
    let tree = Rect::from_min_max(pos2(strip.max.x, full.min.y), pos2(full.min.x + 247.0, full.max.y));
    let right_strip = Rect::from_min_max(pos2(full.max.x - 24.0, full.min.y), full.max);
    let mytag_w = if app.prefs.show_mytag { 230.0 } else { 0.0 };
    let mytag = Rect::from_min_max(pos2(right_strip.min.x - mytag_w - 2.5, full.min.y), pos2(right_strip.min.x - 2.5, full.max.y));
    let list = Rect::from_min_max(pos2(full.min.x + 253.0, full.min.y), pos2(mytag.min.x, full.max.y));
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    p.rect_filled(Rect::from_min_max(full.min, tree.max), 0.0, BG2);
    p.rect_filled(right_strip, 0.0, BG2);
    // Top row of the tree area: collapse chevron and sub-window icon.
    crate::icons::chevron_right(&p, Rect::from_center_size(pos2(full.min.x + 13.6, full.min.y + 12.0), vec2(14.0, 14.0)), OFF);
    crate::icons::window_icon(&p, Rect::from_center_size(pos2(full.min.x + 57.5, full.min.y + 12.0), vec2(18.0, 16.0)), WHITE);
    p.line_segment([pos2(full.min.x, full.min.y + 22.0), pos2(tree.max.x, full.min.y + 22.0)], Stroke::new(1.0, BG));

    icon_strip(app, ui, strip);
    let mut drop_targets: Vec<(Rect, i64)> = vec![];
    let tree_inner = Rect::from_min_max(pos2(tree.min.x + 11.0, full.min.y + 24.0), pos2(tree.max.x - 3.0, full.max.y - 4.0));
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(tree_inner));
    tui.spacing_mut().item_spacing.y = 0.0;
    tree_pane(app, &mut tui, &mut drop_targets);
    let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(list));
    match app.view.clone() {
        View::Device { root, node } => device_list(app, &mut lui, &root, node),
        _ => track_list(app, &mut lui),
    }
    if app.prefs.show_mytag {
        p.rect_filled(mytag, 0.0, BG2);
        let mut mui = ui.new_child(egui::UiBuilder::new().max_rect(mytag.shrink(6.0)));
        mytag_panel(app, &mut mui);
    }
    right_icons(app, ui, right_strip);

    // drag & drop of tracks onto playlists
    if let Some(ids) = app.drag.clone() {
        let ptr = ui.ctx().pointer_latest_pos();
        if let Some(pp) = ptr {
            let label = if ids.len() == 1 { app.track(ids[0]).map(|t| t.display_title()).unwrap_or_default() } else { format!("{} tracks", ids.len()) };
            let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dragl")));
            let rr = Rect::from_min_size(pp + vec2(12.0, 6.0), vec2(8.0 + label.len() as f32 * 6.5, 18.0));
            lp.rect_filled(rr, 3.0, SELECT);
            lp.text(rr.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(11.0), WHITE);
            for (r, _id) in &drop_targets {
                if r.contains(pp) {
                    lp.rect_stroke(*r, 2.0, Stroke::new(1.5, BLUE), egui::StrokeKind::Inside);
                }
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(pp) = ptr {
                if let Some((_, id)) = drop_targets.iter().find(|(r, _)| r.contains(pp)) {
                    app.add_to_playlist(*id, &ids);
                }
            }
            app.drag = None;
        }
    }
}

type IconFn = fn(&egui::Painter, Rect, Color32);

/// Left icon strip: icons on a 19.9 pt pitch from 37.6 pt below the browser top, centred at
/// x = 16 pt, each in a thin outlined square; the selected one filled blue.
fn icon_strip(app: &mut App, ui: &mut Ui, r: Rect) {
    let bm: IconFn = |p, r, c| crate::icons::bookmark(p, r, c, true);
    let items: [(IconFn, Option<SideMode>, &str); 7] = [
        (bm, Some(SideMode::Collection), "Collection"),
        (crate::icons::list, Some(SideMode::Playlists), "Playlists"),
        (crate::icons::link, None, "Related Tracks (not available in this app)"),
        (crate::icons::monitor, None, "Explorer (not available in this app)"),
        (crate::icons::usb, Some(SideMode::Devices), "Devices"),
        (crate::icons::history, None, "Histories (shown under each device)"),
        (crate::icons::record, None, "Recordings (not available in this app)"),
    ];
    for (i, (draw, mode, name)) in items.iter().enumerate() {
        let c = pos2(r.min.x + 16.0, r.min.y + 37.6 + i as f32 * 19.9);
        let rr = Rect::from_center_size(c, vec2(16.5, 16.5));
        let on = mode.map(|m| app.side == m).unwrap_or(false);
        let resp = ui.interact(rr.expand(1.5), ui.id().with(("side", i)), Sense::click());
        let p = ui.painter();
        if on {
            p.rect_filled(rr, 2.0, BLUE);
        } else {
            p.rect_stroke(rr, 2.0, Stroke::new(1.0, if mode.is_some() { DIM } else { hex(0x4A4A4A) }), egui::StrokeKind::Inside);
        }
        let col = if on {
            WHITE
        } else if mode.is_none() {
            hex(0x4A4A4A)
        } else if resp.hovered() {
            WHITE
        } else {
            DIM2
        };
        draw(p, rr.shrink(1.0), col);
        let resp = if app.prefs.tooltips { resp.on_hover_text(*name) } else { resp };
        if let (true, Some(m)) = (resp.clicked(), mode) {
            app.side = *m;
            match m {
                SideMode::Collection => app.set_view(View::Collection),
                SideMode::Devices => app.scan_devices(),
                SideMode::Playlists => {}
            }
        }
    }
    // Bottom: sync icon (status of background work).
    let sr = Rect::from_center_size(pos2(r.min.x + 19.0, r.max.y - 20.0), vec2(18.0, 18.0));
    crate::icons::sync(ui.painter(), sr, DIM2);
}

/// Right icon strip: My Tag, Related, Filter, Information, Sub-browser at 193/227/261/294/328 pt.
fn right_icons(app: &mut App, ui: &mut Ui, r: Rect) {
    let items: [(IconFn, &str); 5] = [
        (crate::icons::tag, "Display My Tag configuration window."),
        (crate::icons::link, "Related Tracks (not available in this app)"),
        (crate::icons::funnel, "Filter"),
        (crate::icons::info, "Display information window."),
        (crate::icons::split_panes, "Sub-browser (not available in this app)"),
    ];
    for (i, (draw, tipt)) in items.iter().enumerate() {
        let y = [193.0, 227.0, 261.0, 294.0, 328.0][i];
        let rr = Rect::from_center_size(pos2(r.center().x, r.min.y + y), vec2(19.0, 19.0));
        let enabled = matches!(i, 0 | 2 | 3);
        let on = match i {
            0 => app.prefs.show_mytag,
            2 => app.filter_open,
            _ => false,
        };
        let resp = ui.interact(rr, ui.id().with(("ricon", i)), Sense::click());
        if on {
            ui.painter().rect_filled(rr, 2.0, BLUE);
        } else {
            ui.painter().rect_stroke(rr, 2.0, Stroke::new(1.0, if enabled { DIM } else { hex(0x4A4A4A) }), egui::StrokeKind::Inside);
        }
        draw(ui.painter(), rr.shrink(1.5), if !enabled { hex(0x4A4A4A) } else if on || resp.hovered() { WHITE } else { TEXT });
        let resp = if app.prefs.tooltips { resp.on_hover_text(*tipt) } else { resp };
        if resp.clicked() {
            match i {
                0 => {
                    app.prefs.show_mytag = !app.prefs.show_mytag;
                    app.save_prefs();
                }
                2 => {
                    app.filter_open = !app.filter_open;
                    app.list_dirty = true;
                }
                3 => {
                    if let Some(id) = app.list.iter().find(|t| app.selected.contains(t)).copied() {
                        app.dialogs.info_track = app.track(id).cloned();
                    }
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

/// One tree row: 15 pt high (rows sit on an 18 pt pitch in the Collection list, 15 pt in the
/// playlist tree), 15 pt indentation per level, selection #294C6A.
fn tree_row(ui: &mut Ui, depth: usize, text: &str, count: Option<usize>, selected: bool, icon: TreeIcon, pitch: f32) -> egui::Response {
    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), pitch), Sense::click());
    let p = ui.painter();
    let bar = Rect::from_center_size(rr.center(), vec2(rr.width(), 15.0));
    if selected {
        p.rect_filled(bar, 0.0, SELECT);
    } else if resp.hovered() {
        p.rect_filled(bar, 0.0, hex(0x262626));
    }
    let x = rr.min.x + 4.0 + depth as f32 * 15.0;
    let cy = rr.center().y;
    let ir = Rect::from_center_size(pos2(x + 5.0, cy), vec2(11.0, 11.0));
    let mut tx = x + 16.0;
    match icon {
        TreeIcon::None => tx = x + 2.0,
        TreeIcon::Lock => crate::icons::lock(p, ir, DIM),
        TreeIcon::Note => {
            p.circle_filled(pos2(ir.center().x - 2.0, ir.center().y + 3.0), 2.6, DIM2);
            p.line_segment([pos2(ir.center().x + 0.5, ir.center().y + 3.0), pos2(ir.center().x + 0.5, ir.min.y)], Stroke::new(1.3, DIM2));
            p.line_segment([pos2(ir.center().x + 0.5, ir.min.y), pos2(ir.center().x + 4.5, ir.min.y + 2.5)], Stroke::new(1.3, DIM2));
        }
        TreeIcon::Open | TreeIcon::FolderOpen => {
            p.add(egui::Shape::convex_polygon(vec![pos2(ir.min.x + 1.0, cy - 3.0), pos2(ir.max.x - 1.0, cy - 3.0), pos2(ir.center().x, cy + 3.5)], WHITE, Stroke::NONE));
        }
        TreeIcon::Closed | TreeIcon::FolderClosed => {
            p.add(egui::Shape::convex_polygon(vec![pos2(ir.center().x - 2.5, cy - 4.5), pos2(ir.center().x + 3.5, cy), pos2(ir.center().x - 2.5, cy + 4.5)], WHITE, Stroke::NONE));
        }
        TreeIcon::Playlist => crate::icons::list(p, ir.expand(2.0), DIM2),
        TreeIcon::Smart => crate::icons::gear(p, ir.expand(1.0), DIM2),
        TreeIcon::Device => crate::icons::usb(p, ir.expand(2.0), DIM2),
    }
    if matches!(icon, TreeIcon::FolderOpen | TreeIcon::FolderClosed) {
        // folder glyph after the disclosure triangle
        let fr = Rect::from_center_size(pos2(x + 23.0, cy), vec2(13.0, 10.0));
        p.rect_filled(fr, 1.0, DIM2);
        p.rect_filled(Rect::from_min_size(fr.min - vec2(0.0, 2.0), vec2(6.0, 3.0)), 1.0, DIM2);
        tx = x + 34.0;
    }
    let clip = p.with_clip_rect(Rect::from_min_max(pos2(tx, rr.min.y), pos2(rr.max.x - 30.0, rr.max.y)));
    clip.text(pos2(tx, cy), Align2::LEFT_CENTER, text, FontId::proportional(11.5), if selected { WHITE } else if icon == TreeIcon::Lock { DIM } else { TEXT });
    if let Some(c) = count {
        p.text(pos2(rr.max.x - 10.0, cy), Align2::RIGHT_CENTER, c.to_string(), FontId::proportional(11.0), WHITE);
    }
    resp
}

fn tree_pane(app: &mut App, ui: &mut Ui, drop: &mut Vec<(Rect, i64)>) {
    match app.side {
        SideMode::Collection => {
            // "Collection" heading at 31 pt, "All" at 47 pt, locked filters on an 18 pt pitch.
            let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
            ui.painter().text(pos2(hr.min.x + 4.0, hr.center().y), Align2::LEFT_CENTER, "Collection", FontId::proportional(12.0), WHITE);
            let sel = app.view == View::Collection;
            if tree_row(ui, 0, "All", None, sel, TreeIcon::None, 17.0).clicked() {
                app.set_view(View::Collection);
            }
            for f in ["Date Added", "Genre", "Artist", "Album"] {
                let r = tree_row(ui, 1, f, None, false, TreeIcon::Lock, 18.0);
                if app.prefs.tooltips {
                    r.on_hover_text("Browsing by category is not available in this app; use the search box or the Filter.");
                }
            }
        }
        SideMode::Playlists => {
            // Tree View / Column View segmented toggle at 27..48 pt.
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 25.0), Sense::hover());
            crate::icons::magnifier(ui.painter(), Rect::from_center_size(pos2(row.min.x + 11.0, row.center().y), vec2(16.0, 16.0)), DIM2, 0);
            let tv = Rect::from_min_max(pos2(row.min.x + 26.0, row.min.y + 2.0), pos2(row.min.x + 124.0, row.max.y - 2.0));
            ui.painter().rect_filled(tv, 0.0, BLUE);
            ui.painter().text(tv.center(), Align2::CENTER_CENTER, "Tree View", FontId::proportional(12.5), WHITE);
            let cv = Rect::from_min_max(pos2(tv.max.x + 4.0, tv.min.y), pos2(row.max.x, tv.max.y));
            ui.painter().rect_filled(cv, 0.0, hex(0x323232));
            crate::icons::lock(ui.painter(), Rect::from_center_size(pos2(cv.min.x + 9.0, cv.center().y), vec2(10.0, 10.0)), DIM);
            ui.painter().with_clip_rect(cv).text(pos2(cv.min.x + 18.0, cv.center().y), Align2::LEFT_CENTER, "Column View", FontId::proportional(12.0), DIM);
            ui.add_space(4.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let all_sel = app.view == View::Collection;
                if tree_row(ui, 1, "All Tracks", Some(app.tracks.len()), all_sel, TreeIcon::Note, 15.0).clicked() {
                    app.set_view(View::Collection);
                }
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
                });
                node_rows(app, ui, 0, 1, drop);
            });
        }
        SideMode::Devices => devices_tree(app, ui),
    }
}

fn node_rows(app: &mut App, ui: &mut Ui, parent: i64, depth: usize, drop: &mut Vec<(Rect, i64)>) {
    let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == parent).cloned().collect();
    for n in kids {
        let sel = app.view == View::Playlist(n.id);
        let open = app.expanded.contains(&n.id);
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
                resp.request_focus();
                app.dialogs.rename.as_mut().unwrap().1 = name.clone();
                if resp.lost_focus() {
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
            if ui.button(format!("Export {what}…")).clicked() {
                open_export(app, vec![nn.id], vec![]);
                ui.close_menu();
            }
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
                let _ = app.db.delete_node(nn.id);
                if app.view == View::Playlist(nn.id) {
                    app.set_view(View::Collection);
                }
                app.reload_playlists();
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
    let sel_dev = matches!(app.view, View::Device { .. });
    ui.add_space(1.0);
    tree_row(ui, 1, "Devices", None, !sel_dev, TreeIcon::Open, 17.0);
    let devs: Vec<(String, String, String, Option<Vec<crate::export::usb::DevNode>>, Option<String>, usize)> = app
        .devices
        .iter()
        .map(|d| (d.root.clone(), d.name.clone(), d.fs.clone(), d.lib.as_ref().map(|l| l.nodes.clone()), d.error.clone(), d.lib.as_ref().map(|l| l.tracks.len()).unwrap_or(0)))
        .collect();
    for (root, name, fs, nodes, err, ntracks) in devs {
        let sel = app.view == View::Device { root: root.clone(), node: None };
        let resp = tree_row(ui, 2, &format!("{name}  ({})", App::fs_label(&fs)), Some(ntracks), sel, TreeIcon::Device, 17.0);
        if resp.clicked() {
            app.set_view(View::Device { root: root.clone(), node: None });
        }
        let r2 = root.clone();
        resp.context_menu(|ui| {
            if ui.button("Export to this device…").clicked() {
                app.dialogs.export_root = Some(r2.clone());
                open_export(app, vec![], vec![]);
                app.dialogs.export_root = Some(r2.clone());
                ui.close_menu();
            }
            if ui.button("Show in Finder").clicked() {
                let _ = std::process::Command::new("/usr/bin/open").arg(&r2).spawn();
                ui.close_menu();
            }
            if fs != "folder" && ui.button("Disconnect USB Device").clicked() {
                let _ = std::process::Command::new("/usr/sbin/diskutil").arg("eject").arg(&r2).spawn();
                ui.close_menu();
            }
            if fs == "folder" && ui.button("Remove from list").clicked() {
                app.prefs.devices_extra.retain(|d| d != &r2);
                app.save_prefs();
                app.scan_devices();
                ui.close_menu();
            }
        });
        if let Some(e) = err {
            ui.label(RichText::new(e).size(10.5).color(CAUTION_TX));
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
                    let (r, id, name) = (root.to_string(), n.id, n.name.clone());
                    resp.context_menu(|ui| {
                        if ui.button(if n.folder { "Delete Folder from device" } else { "Delete Playlist from device" }).clicked() {
                            app.dialogs.device_remove = Some((r.clone(), id, name.clone()));
                            ui.close_menu();
                        }
                    });
                    if n.folder {
                        rows(app, ui, root, nodes, n.id, depth + 1);
                    }
                }
            }
            rows(app, ui, &root, &nodes, 0, 2);
        }
    }
    if app.devices.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Connect a USB drive or SD card (FAT32, exFAT or HFS+).").size(11.0).color(DIM));
    }
    ui.add_space(10.0);
    if ui.button("Add a folder as a device…").on_hover_text("Export into any folder, for example to prepare a drive image or test an export.").clicked() {
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
        open_export(app, vec![], vec![]);
    }
}

fn device_list(app: &mut App, ui: &mut Ui, root: &str, node: Option<u32>) {
    let Some(dev) = app.devices.iter().find(|d| d.root == root) else {
        ui.label("The device is no longer connected.");
        return;
    };
    let title = match (node, &dev.lib) {
        (Some(n), Some(l)) => l.nodes.iter().find(|x| x.id == n).map(|x| x.name.clone()).unwrap_or_default(),
        _ => format!("{} — All Tracks", dev.name),
    };
    let tracks: Vec<crate::export::usb::DevTrack> = match &dev.lib {
        None => vec![],
        Some(l) => match node {
            None => l.tracks.values().cloned().collect(),
            Some(n) => l.nodes.iter().find(|x| x.id == n).map(|x| x.entries.iter().filter_map(|t| l.tracks.get(t).cloned()).collect()).unwrap_or_default(),
        },
    };
    let r = ui.max_rect();
    let bar = Rect::from_min_size(r.min, vec2(r.width(), 24.0));
    ui.painter().rect_filled(bar, 0.0, BG2);
    ui.painter().text(pos2(bar.min.x + 12.0, bar.center().y), Align2::LEFT_CENTER, format!("{title} ({} Tracks)", tracks.len()), FontId::proportional(12.5), WHITE);
    let space = match (dev.free, dev.total) {
        (Some(f), Some(t)) => format!("Available Space {:.1} GB / Total Space {:.1} GB", f as f64 / 1e9, t as f64 / 1e9),
        _ => String::new(),
    };
    ui.painter().text(pos2(bar.max.x - 12.0, bar.center().y), Align2::RIGHT_CENTER, space, FontId::proportional(11.0), DIM);
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(r.min.x, r.min.y + 26.0), r.max)));
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
    let hdr = body.allocate_exact_size(vec2(body.available_width(), 16.0), Sense::hover()).0;
    body.painter().rect_filled(hdr, 0.0, BG2);
    let mut x = hdr.min.x + 8.0;
    for (c, w) in cols.iter().zip(widths) {
        body.painter().text(pos2(x, hdr.center().y), Align2::LEFT_CENTER, *c, FontId::proportional(11.0), WHITE);
        x += w;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(&mut body, ROW_H, tracks.len(), |ui, range| {
        for i in range {
            let t = &tracks[i];
            let (rr, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            ui.painter().rect_filled(rr, 0.0, if i % 2 == 0 { BG1 } else { BG2 });
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
                clip.text(pos2(x, rr.center().y), Align2::LEFT_CENTER, v, FontId::proportional(11.5), TEXT);
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
        View::Device { .. } => String::new(),
    }
}

fn track_list(app: &mut App, ui: &mut Ui) {
    if app.list_dirty {
        app.rebuild_list();
    }
    let r = ui.max_rect();
    // Title bar: 5..27 pt below the browser top; divider after the title area; on the right
    // PLAYER A selector, four view-mode icons and the search box (measured from the right edge).
    let bar = Rect::from_min_max(pos2(r.min.x, r.min.y + 5.0), pos2(r.max.x, r.min.y + 27.0));
    let p = ui.painter().clone();
    p.rect_filled(bar, 0.0, BG2);
    p.text(pos2(bar.min.x + 10.5, bar.center().y), Align2::LEFT_CENTER, format!("{} ({} Tracks)", view_title(app), app.list.len()), FontId::proportional(11.5), WHITE);
    let rx = bar.max.x;
    p.line_segment([pos2(bar.min.x + 340.0, bar.min.y), pos2(bar.min.x + 340.0, bar.max.y)], Stroke::new(1.0, BG));
    // PLAYER A (load target; this app has one player)
    let pa = Rect::from_min_max(pos2(rx - 496.5, bar.min.y + 3.0), pos2(rx - 427.5, bar.max.y - 3.0));
    p.text(pos2(pa.min.x + 4.0, pa.center().y), Align2::LEFT_CENTER, "PLAYER A", bold(10.0), WHITE);
    crate::icons::chevron_down(&p, Rect::from_center_size(pos2(pa.max.x - 6.0, pa.center().y), vec2(12.0, 10.0)), WHITE);
    let par = ui.interact(pa, ui.id().with("playera"), Sense::click());
    if app.prefs.tooltips {
        par.on_hover_text("Double-click a track (or press Return) to load it into the player.");
    }
    // View-mode icons (list with artwork is the active one).
    let vic: [(f32, IconFn, bool); 4] = [(394.5, crate::icons::view_art_list, true), (356.5, crate::icons::view_list, false), (297.5, crate::icons::view_columns, false), (260.0, crate::icons::split_panes, false)];
    for (dx, draw, on) in vic {
        let vr = Rect::from_center_size(pos2(rx - dx, bar.center().y), vec2(18.0, 17.0));
        if on {
            p.rect_filled(vr, 2.0, BLUE);
        } else {
            p.rect_stroke(vr, 2.0, Stroke::new(1.0, DIM), egui::StrokeKind::Inside);
        }
        draw(&p, vr.shrink(1.0), WHITE);
    }
    // search box
    let sr = Rect::from_min_max(pos2(rx - 230.5, bar.min.y + 1.0), pos2(rx - 18.5, bar.max.y - 1.0));
    p.rect_filled(sr, 0.0, BG);
    crate::icons::magnifier(&p, Rect::from_center_size(pos2(sr.min.x + 11.0, sr.center().y), vec2(16.0, 16.0)), DIM2, 0);
    crate::icons::chevron_down(&p, Rect::from_center_size(pos2(sr.min.x + 22.0, sr.center().y + 1.0), vec2(8.0, 7.0)), DIM2);
    let mut s = app.search.clone();
    let te = ui.put(Rect::from_min_max(pos2(sr.min.x + 30.0, sr.min.y), sr.max), egui::TextEdit::singleline(&mut s).frame(false).hint_text(RichText::new("Search within this track list").color(DIM)).vertical_align(egui::Align::Center).font(FontId::proportional(11.5)));
    if te.changed() {
        app.search = s;
        app.list_dirty = true;
    }
    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
        te.request_focus();
    }
    let cm = icon_button_ui(ui, Rect::from_center_size(pos2(rx - 8.0, bar.center().y), vec2(14.0, 18.0)), "colmenu", crate::icons::hamburger);
    dropdown(ui, &cm, "cols", |ui| {
        ui.label(RichText::new("Columns").strong());
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for c in ALL_COLS {
                if c == Col::Status {
                    continue;
                }
                let mut on = app.prefs.columns.contains(&c);
                if ui.checkbox(&mut on, c.label()).changed() {
                    if on {
                        app.prefs.columns.push(c);
                    } else {
                        app.prefs.columns.retain(|x| *x != c);
                    }
                    app.save_prefs();
                }
            }
        });
        if ui.button("Auto-size all columns").clicked() {
            app.prefs.widths.clear();
            app.save_prefs();
        }
    });
    let mut top = bar.max.y;
    if app.filter_open {
        let fr = Rect::from_min_size(pos2(r.min.x, top), vec2(r.width(), 28.0));
        ui.painter().rect_filled(fr, 0.0, BG1);
        let mut fui = ui.new_child(egui::UiBuilder::new().max_rect(fr.shrink2(vec2(10.0, 3.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        filter_bar(app, &mut fui);
        top = fr.max.y;
    }
    let top = top.max(r.min.y + 31.0) - 4.0;
    let area = Rect::from_min_max(pos2(r.min.x, top + 4.0), r.max);
    let mut aui = ui.new_child(egui::UiBuilder::new().max_rect(area));
    aui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let cols = app.prefs.columns.clone();
    let mut all_cols = vec![Col::Status];
    all_cols.extend(cols.iter().copied().filter(|c| *c != Col::Status));
    let total_w: f32 = all_cols.iter().map(|c| col_width(app, *c)).sum::<f32>() + 12.0;
    let manual = app.is_manual_playlist();
    egui::ScrollArea::horizontal().id_salt("hscroll").auto_shrink([false, false]).show(&mut aui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let w = total_w.max(ui.available_width());
        // header: 13.5 pt, #1E1E1E, white 11 pt labels, sort column in blue with an arrow
        let (hr, _) = ui.allocate_exact_size(vec2(w, 13.5), Sense::hover());
        ui.painter().rect_filled(hr, 0.0, BG2);
        let mut x = hr.min.x;
        for c in &all_cols {
            let cw = col_width(app, *c);
            let cr = Rect::from_min_size(pos2(x, hr.min.y), vec2(cw, hr.height()));
            let resp = ui.interact(cr, ui.id().with(("hdr", *c)), Sense::click());
            let sorted = app.prefs.sort_col == *c;
            if *c == Col::Status {
                crate::icons::funnel(ui.painter(), Rect::from_center_size(pos2(cr.min.x + 15.0, cr.center().y), vec2(11.0, 11.0)), if app.filter_open { BLUE } else { DIM2 });
            }
            let label = if *c == Col::Status { "" } else { c.label() };
            let tr = ui.painter().with_clip_rect(cr).text(pos2(cr.min.x + 5.0, cr.center().y), Align2::LEFT_CENTER, label, FontId::proportional(11.0), if sorted { BLUE } else { WHITE });
            if sorted {
                let ax = if label.is_empty() { cr.min.x + 22.0 } else { tr.max.x + 8.0 };
                let ay = cr.center().y;
                let s = Stroke::new(1.2, BLUE);
                ui.painter().line_segment([pos2(ax, ay - 5.0), pos2(ax, ay + 5.0)], s);
                if app.prefs.sort_asc {
                    ui.painter().line_segment([pos2(ax - 3.0, ay - 2.0), pos2(ax, ay - 5.0)], s);
                    ui.painter().line_segment([pos2(ax + 3.0, ay - 2.0), pos2(ax, ay - 5.0)], s);
                } else {
                    ui.painter().line_segment([pos2(ax - 3.0, ay + 2.0), pos2(ax, ay + 5.0)], s);
                    ui.painter().line_segment([pos2(ax + 3.0, ay + 2.0), pos2(ax, ay + 5.0)], s);
                }
            }
            ui.painter().line_segment([pos2(cr.max.x - 0.5, cr.min.y + 1.0), pos2(cr.max.x - 0.5, cr.max.y - 1.0)], Stroke::new(1.0, BG));
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
                if ui.button("Auto-size this column").clicked() {
                    app.prefs.widths.remove(&c2.key());
                    app.save_prefs();
                    ui.close_menu();
                }
                if c2 != Col::Status && ui.button("Hide this column").clicked() {
                    app.prefs.columns.retain(|x| *x != c2);
                    app.save_prefs();
                    ui.close_menu();
                }
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
        let mut sa = egui::ScrollArea::vertical().id_salt("vscroll").auto_shrink([false, false]);
        if app.scroll_to_selected {
            if let Some(i) = app.anchor {
                let h = ui.available_height();
                sa = sa.vertical_scroll_offset(((i as f32) * ROW_H - h * 0.4).max(0.0));
            }
            app.scroll_to_selected = false;
        }
        let mut drop_line: Option<usize> = None;
        sa.show_rows(ui, ROW_H, n, |ui, range| {
            for i in range {
                let id = app.list[i];
                let Some(t) = app.track(id).cloned() else { continue };
                let (rr, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click_and_drag());
                let sel = app.selected.contains(&id);
                let loaded = app.deck.track == Some(id);
                let bg = if sel { SELECT } else if i % 2 == 0 { BG1 } else { BG2 };
                ui.painter().rect_filled(rr, 0.0, bg);
                if loaded && !sel {
                    ui.painter().rect_stroke(rr, 0.0, Stroke::new(1.0, hex(0x335577)), egui::StrokeKind::Inside);
                }
                let mut x = rr.min.x;
                for c in &all_cols {
                    let cw = col_width(app, *c);
                    let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(cw, rr.height()));
                    draw_cell(app, ui, *c, &t, cr, i);
                    x += cw;
                }
                // Rating column clicks
                if resp.clicked() {
                    if let Some(pp) = resp.interact_pointer_pos() {
                        let mut x = rr.min.x;
                        for c in &all_cols {
                            let cw = col_width(app, *c);
                            if *c == Col::Rating && pp.x >= x && pp.x < x + cw {
                                let s = (((pp.x - x - 3.5) / 15.5).floor() as i32 + 1).clamp(0, 5) as u8;
                                if let Some(tm) = app.track_mut(id) {
                                    tm.rating = if tm.rating == s { 0 } else { s };
                                }
                                app.save_track(id);
                            }
                            x += cw;
                        }
                    }
                    let (cmd, shift) = ui.input(|inp| (inp.modifiers.command, inp.modifiers.shift));
                    if shift {
                        let a = app.anchor.unwrap_or(i);
                        let (lo, hi) = (a.min(i), a.max(i));
                        app.selected = app.list[lo..=hi].iter().copied().collect();
                    } else if cmd {
                        if !app.selected.insert(id) {
                            app.selected.remove(&id);
                        }
                        app.anchor = Some(i);
                    } else {
                        app.selected.clear();
                        app.selected.insert(id);
                        app.anchor = Some(i);
                    }
                }
                if resp.double_clicked() {
                    app.load_track(id);
                }
                if resp.drag_started() {
                    if !app.selected.contains(&id) {
                        app.selected.clear();
                        app.selected.insert(id);
                        app.anchor = Some(i);
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
                    app.anchor = Some(i);
                }
                resp.context_menu(|ui| {
                    let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
                    track_menu_items(app, ui, &sel);
                });
            }
        });
        // Reorder inside a playlist (only when sorted by #)
        if let (Some(pl), Some(to), Some(ids)) = (manual, drop_line, app.drag.clone()) {
            if ui.input(|i| i.pointer.any_released()) && app.prefs.sort_col == Col::Num && app.prefs.sort_asc && app.search.is_empty() {
                app.move_in_playlist(pl, &ids, to);
                app.drag = None;
            }
        }
    });
    // Files dropped from Finder
    let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if !dropped.is_empty() {
        app.import_paths(dropped, app.is_manual_playlist());
    }
}

fn draw_cell(app: &mut App, ui: &mut Ui, c: Col, t: &Track, r: Rect, row: usize) {
    let p = ui.painter().with_clip_rect(r.shrink2(vec2(3.0, 0.0)));
    let text_pos = |right: bool| if right { pos2(r.max.x - 6.0, r.center().y) } else { pos2(r.min.x + 6.0, r.center().y) };
    let font = FontId::proportional(11.5);
    let col = if t.missing { CAUTION_TX } else { TEXT };
    let txt = |s: String| {
        let right = c.right_aligned();
        p.text(text_pos(right), if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }, s, font.clone(), col);
    };
    match c {
        Col::Num => {
            let n = if app.is_manual_playlist().is_some() && app.prefs.sort_col == Col::Num && app.search.is_empty() { row + 1 } else { row + 1 };
            txt(n.to_string());
        }
        Col::Status => {
            // Two 30 pt sub-columns as in the original: source/colour, then analysis state / ★CUE.
            let a = Rect::from_min_size(r.min, vec2(30.0, r.height()));
            let b = Rect::from_min_max(pos2(r.min.x + 30.0, r.min.y), r.max);
            p.line_segment([pos2(a.max.x - 0.5, r.min.y), pos2(a.max.x - 0.5, r.max.y)], Stroke::new(1.0, BG));
            if t.color > 0 {
                p.rect_filled(Rect::from_center_size(a.center(), vec2(9.0, 9.0)), 1.0, label_color(t.color));
            }
            if t.missing {
                p.text(a.center(), Align2::CENTER_CENTER, "!", FontId::proportional(11.0), CAUTION_TX);
            }
            let cued = app.cue_counts.get(&t.id).map(|(h, m)| h + m > 0).unwrap_or(false);
            match t.analyzed {
                3 => {
                    let tt = app.ctx.input(|i| i.time) as f32;
                    for k in 0..3 {
                        let on = ((tt * 3.0) as i32 % 3) == k;
                        p.circle_filled(pos2(b.min.x + 8.0 + k as f32 * 5.0, b.center().y), 1.6, if on { WHITE } else { DIM });
                    }
                    app.ctx.request_repaint_after(std::time::Duration::from_millis(300));
                }
                2 => {
                    p.text(pos2(b.min.x + 6.0, b.center().y), Align2::LEFT_CENTER, "ERR", FontId::proportional(8.5), CAUTION_TX);
                }
                0 => {
                    p.text(pos2(b.min.x + 5.0, b.center().y), Align2::LEFT_CENTER, "0%", FontId::proportional(9.5), DIM2);
                }
                _ => {
                    star(&p, pos2(b.min.x + 8.0, b.center().y), 4.8, WHITE, true);
                    if cued {
                        p.text(pos2(b.min.x + 13.0, b.center().y), Align2::LEFT_CENTER, "CUE", FontId::proportional(9.5), WHITE);
                    }
                }
            }
        }
        Col::Preview => {
            if let Some(pv) = app.previews.get(&t.id) {
                let n = (r.width() - 8.0).max(1.0) as usize;
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
                let base = r.max.y - 1.5;
                let hh = r.height() - 3.0;
                for (k, v) in cols.iter().enumerate() {
                    let x = r.min.x + 4.0 + k as f32 + 0.5;
                    for (h, cc) in crate::ui_player::wave_bars(app.prefs.wave_color, *v) {
                        p.line_segment([pos2(x, base), pos2(x, base - h * hh)], Stroke::new(1.0, cc));
                    }
                }
                // Hot cue letters at their positions (as the original's preview shows them).
                if let Some(hc) = app.hot_positions.get(&t.id) {
                    let dur = t.duration.max(0.001);
                    for (start, slot, color) in hc {
                        let x = r.min.x + 4.0 + (*start / dur) as f32 * n as f32;
                        let cr = Rect::from_min_size(pos2(x, r.min.y + 1.0), vec2(8.0, 8.0));
                        p.rect_filled(cr, 0.0, hotcue_color(*color));
                        p.text(cr.center(), Align2::CENTER_CENTER, ((b'A' + *slot as u8) as char).to_string(), FontId::proportional(7.0), WHITE);
                    }
                }
            }
        }
        Col::Artwork => {
            // The artwork fills the cell, cropped around its middle band.
            if let Some(tex) = artwork_texture(app, &t.artwork) {
                let cell = r.shrink2(vec2(0.0, 0.5));
                let band = (cell.height() / cell.width()).min(1.0) / 2.0;
                p.image(tex.id(), cell, Rect::from_min_max(pos2(0.0, 0.5 - band), pos2(1.0, 0.5 + band)), Color32::WHITE);
            }
        }
        Col::Title => txt(t.display_title()),
        Col::Artist => txt(t.artist.clone()),
        Col::Album => txt(t.album.clone()),
        Col::Genre => txt(t.genre.clone()),
        Col::Bpm => txt(if t.bpm > 0.0 { format!("{:.2}", t.bpm) } else { String::new() }),
        Col::Rating => {
            for s in 1..=5u8 {
                star(&p, pos2(r.min.x + 11.0 + (s - 1) as f32 * 15.5, r.center().y), 6.8, if t.rating >= s { WHITE } else { DIM2 }, t.rating >= s);
            }
        }
        Col::Time => txt(if t.duration > 0.0 { fmt_time(t.duration, false) } else { String::new() }),
        Col::Key => txt(key_name(t.key, app.prefs.key_alnum)),
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
    }
}

fn filter_bar(app: &mut App, ui: &mut Ui) {
    ui.label(RichText::new("BPM").size(11.0).color(DIM2));
    let (mut lo, mut hi) = app.filter_bpm;
    let a = ui.add(egui::DragValue::new(&mut lo).range(0.0..=300.0).speed(0.5).prefix("≥ "));
    let b = ui.add(egui::DragValue::new(&mut hi).range(0.0..=300.0).speed(0.5).prefix("≤ "));
    if a.changed() || b.changed() {
        app.filter_bpm = (lo, hi);
        app.list_dirty = true;
    }
    if let Some(t) = app.deck.track.and_then(|id| app.track(id)) {
        let bpm = t.bpm;
        let key = t.key;
        if ui.small_button("±6% of loaded").clicked() && bpm > 0.0 {
            app.filter_bpm = ((bpm * 0.94 * 100.0).round() / 100.0, (bpm * 1.06 * 100.0).round() / 100.0);
            app.list_dirty = true;
        }
        if ui.small_button("Matching key").on_hover_text("Same key, ±1 on the key wheel, or relative major/minor").clicked() && key >= 0 {
            app.filter_key = key;
            app.list_dirty = true;
        }
    }
    ui.separator();
    ui.label(RichText::new("Key").size(11.0).color(DIM2));
    let alnum = app.prefs.key_alnum;
    egui::ComboBox::from_id_salt("fkey").width(64.0).selected_text(if app.filter_key < 0 { "Any".to_string() } else { key_name(app.filter_key, alnum) }).show_ui(ui, |ui| {
        if ui.selectable_label(app.filter_key < 0, "Any").clicked() {
            app.filter_key = -1;
            app.list_dirty = true;
        }
        for k in 0..24 {
            if ui.selectable_label(app.filter_key == k, format!("{}  ({})", key_name(k, alnum), key_name(k, !alnum))).clicked() {
                app.filter_key = k;
                app.list_dirty = true;
            }
        }
    });
    ui.separator();
    ui.label(RichText::new("Rating ≥").size(11.0).color(DIM2));
    let mut r = app.filter_rating as i32;
    if ui.add(egui::DragValue::new(&mut r).range(0..=5)).changed() {
        app.filter_rating = r as u8;
        app.list_dirty = true;
    }
    ui.separator();
    egui::ComboBox::from_id_salt("fcolor").width(80.0).selected_text(if app.filter_color == 0 { "Any color".to_string() } else { COLOR_NAMES[app.filter_color as usize].to_string() }).show_ui(ui, |ui| {
        for i in 0..9u8 {
            if ui.selectable_label(app.filter_color == i, if i == 0 { "Any color" } else { COLOR_NAMES[i as usize] }).clicked() {
                app.filter_color = i;
                app.list_dirty = true;
            }
        }
    });
    if !app.filter_tags.is_empty() {
        ui.separator();
        if ui.selectable_label(app.filter_tags_all, if app.filter_tags_all { "My Tag: all" } else { "My Tag: any" }).clicked() {
            app.filter_tags_all = !app.filter_tags_all;
            app.list_dirty = true;
        }
    }
    if ui.small_button("Clear").clicked() {
        app.filter_bpm = (0.0, 0.0);
        app.filter_key = -1;
        app.filter_rating = 0;
        app.filter_color = 0;
        app.filter_tags.clear();
        app.list_dirty = true;
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
                    r.request_focus();
                    app.dialogs.rename_tag = Some((rid, n.clone()));
                    if r.lost_focus() {
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
                            r.request_focus();
                            app.dialogs.rename_tag = Some((rid, n.clone()));
                            if r.lost_focus() {
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
                    let fill = if filtering { hex(0x7A4E00) } else if on { BLUE } else if partial { hex(0x1F4170) } else { BTN };
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
                        r.request_focus();
                        app.dialogs.new_tag = Some((cid, n.clone()));
                        if r.lost_focus() {
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
    if sel.is_empty() {
        ui.label(RichText::new("No track selected").color(DIM));
        return;
    }
    let one = sel.len() == 1;
    if one && ui.button("Load to Player").clicked() {
        app.load_track(sel[0]);
        ui.close_menu();
    }
    ui.menu_button("Add To Playlist", |ui| {
        let pls: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.kind == NodeKind::Playlist).cloned().collect();
        if pls.is_empty() {
            ui.label(RichText::new("No playlists yet").color(DIM));
        }
        for n in pls {
            if ui.button(&n.name).clicked() {
                app.add_to_playlist(n.id, sel);
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.button("Create New Playlist").clicked() {
            let id = app.create_node(0, NodeKind::Playlist);
            app.add_to_playlist(id, sel);
            app.side = SideMode::Playlists;
            ui.close_menu();
        }
    });
    if ui.button(if one { "Export Track…" } else { "Export Tracks…" }).clicked() {
        open_export(app, vec![], sel.to_vec());
        ui.close_menu();
    }
    ui.separator();
    if ui.button(if one { "Analyze Track" } else { "Analyze Tracks" }).clicked() {
        app.reanalyze(sel);
        ui.close_menu();
    }
    if ui.button(if one { "Analyze Key" } else { "Analyze Keys" }).clicked() {
        for id in sel {
            if let Some(t) = app.track_mut(*id) {
                t.key = -1;
            }
        }
        app.analyze(sel, true);
        ui.close_menu();
    }
    let locked = sel.iter().all(|id| app.track(*id).map(|t| t.grid_locked).unwrap_or(false));
    if ui.button(if locked { "Analysis Unlock" } else { "Analysis Lock" }).clicked() {
        for id in sel {
            if let Some(t) = app.track_mut(*id) {
                t.grid_locked = !locked;
            }
            app.save_track(*id);
        }
        ui.close_menu();
    }
    if app.analysis_total > 0 && ui.button("Stop Analyzing").clicked() {
        app.pool.stop();
        ui.close_menu();
    }
    ui.separator();
    ui.menu_button("Rating", |ui| {
        for s in 0..=5u8 {
            if ui.button(if s == 0 { "No rating".to_string() } else { "★".repeat(s as usize) }).clicked() {
                for id in sel {
                    if let Some(t) = app.track_mut(*id) {
                        t.rating = s;
                    }
                    app.save_track(*id);
                }
                ui.close_menu();
            }
        }
    });
    ui.menu_button("Color", |ui| {
        for i in 0..9u8 {
            if ui.button(RichText::new(format!("■ {}", COLOR_NAMES[i as usize])).color(if i == 0 { DIM } else { label_color(i) })).clicked() {
                for id in sel {
                    if let Some(t) = app.track_mut(*id) {
                        t.color = i;
                    }
                    app.save_track(*id);
                }
                ui.close_menu();
            }
        }
    });
    let auto = sel.iter().all(|id| app.track(*id).map(|t| t.autoload_hotcue).unwrap_or(false));
    if ui.button(if auto { "Disable Auto Load Hot Cue" } else { "Enable Auto Load Hot Cue" }).clicked() {
        for id in sel {
            if let Some(t) = app.track_mut(*id) {
                t.autoload_hotcue = !auto;
            }
            app.save_track(*id);
        }
        ui.close_menu();
    }
    if ui.button(if one { "Reload Tag" } else { "Reload Tags" }).clicked() {
        app.reload_tags_from_files(sel);
        ui.close_menu();
    }
    if ui.button("Reset DJ Play Count").clicked() {
        for id in sel {
            if let Some(t) = app.track_mut(*id) {
                t.play_count = 0;
            }
            app.save_track(*id);
        }
        ui.close_menu();
    }
    if one && ui.button("Change Artwork…").clicked() {
        if let Some(f) = rfd::FileDialog::new().add_filter("Image", &["jpg", "jpeg", "png"]).pick_file() {
            let dest = crate::db::artwork_dir().join(format!("custom_{}.{}", sel[0], f.extension().and_then(|e| e.to_str()).unwrap_or("jpg")));
            if std::fs::copy(&f, &dest).is_ok() {
                app.textures.retain(|k, _| !k.contains(&format!("custom_{}", sel[0])));
                if let Some(t) = app.track_mut(sel[0]) {
                    t.artwork = dest.to_string_lossy().to_string();
                }
                app.save_track(sel[0]);
            }
        }
        ui.close_menu();
    }
    if one && ui.button("Relocate…").on_hover_text("Point this track to a moved file").clicked() {
        if let Some(f) = rfd::FileDialog::new().pick_file() {
            let p = f.to_string_lossy().to_string();
            if let Some(t) = app.track_mut(sel[0]) {
                t.path = p;
                t.missing = false;
            }
            app.save_track(sel[0]);
        }
        ui.close_menu();
    }
    if one && ui.button("Playlists including this track").clicked() {
        let pls = app.db.playlists_containing(sel[0]).unwrap_or_default();
        let names: Vec<String> = pls.iter().filter_map(|p| app.node(*p)).map(|n| n.name.clone()).collect();
        app.toast(if names.is_empty() { "This track is not in any playlist.".to_string() } else { format!("In: {}", names.join(", ")) });
        ui.close_menu();
    }
    if ui.button("Show in Finder").clicked() {
        if let Some(t) = app.track(sel[0]) {
            let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(&t.path).spawn();
        }
        ui.close_menu();
    }
    if one && ui.button("Property…").clicked() {
        app.dialogs.info_track = app.track(sel[0]).cloned();
        ui.close_menu();
    }
    ui.separator();
    if let Some(pl) = app.is_manual_playlist() {
        if ui.button("Remove from Playlist").clicked() {
            let s: HashSet<i64> = sel.iter().copied().collect();
            app.remove_from_playlist(pl, &s);
            ui.close_menu();
        }
    }
    if ui.button("Remove from Collection").clicked() {
        app.dialogs.confirm_remove = Some(sel.to_vec());
        ui.close_menu();
    }
}
