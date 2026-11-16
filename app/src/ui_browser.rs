//! Header bar, browser (sidebar tree, track list, My Tag panel, devices) and footer.

use eframe::egui;
use crate::app::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_player::{button, star};
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui};
use std::collections::HashSet;

pub const ROW_H: f32 = 17.0;

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
pub fn header_bar(app: &mut App, ui: &mut Ui) {
    let r = ui.max_rect();
    ui.painter().rect_filled(r, 0.0, BG1);
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        let mode = ui.add(egui::Label::new(RichText::new("EXPORT  ▼").strong().size(12.0).color(WHITE)).sense(Sense::click()));
        let mode = if app.prefs.tooltips { mode.on_hover_text("This app covers EXPORT mode (library, preparation and USB export).") } else { mode };
        let _ = mode;
        ui.add_space(18.0);
        let layout = ui.add(egui::Label::new(RichText::new(if app.prefs.browser_only { "▭ Full Browser ▼" } else { "▭ 1 Player ▼" }).size(11.0).color(DIM2)).sense(Sense::click()));
        dropdown(ui, &layout, "layout", |ui| {
            if ui.selectable_label(!app.prefs.browser_only, "1 Player").clicked() {
                app.prefs.browser_only = false;
                app.save_prefs();
            }
            if ui.selectable_label(app.prefs.browser_only, "Full Browser").clicked() {
                app.prefs.browser_only = true;
                app.save_prefs();
            }
        });
        ui.add_space(14.0);
        menus(app, ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let now = chrono::Local::now().format("%H:%M").to_string();
            ui.label(RichText::new(now).size(12.0).color(WHITE).strong());
            ui.add_space(14.0);
            let gear = ui.add(egui::Label::new(RichText::new("⚙").size(16.0).color(WHITE)).sense(Sense::click()));
            if gear.on_hover_text("Preferences").clicked() {
                app.dialogs.prefs_open = true;
            }
            ui.add_space(10.0);
            // master volume
            let mut v = app.player.state.lock().volume;
            let s = ui.add_sized(vec2(90.0, 14.0), egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
            if s.changed() {
                app.player.state.lock().volume = v;
            }
            ui.label(RichText::new("🔈").size(12.0).color(DIM));
        });
    });
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

fn import_m3u(app: &mut App, f: &std::path::Path) {
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
    p.text(pos2(r.min.x + 14.0, r.center().y), Align2::LEFT_CENTER, "◉ PrepDeck", FontId::proportional(14.0), WHITE);
    // totals
    let (n, secs, bytes) = if let View::Device { .. } = app.view {
        (0usize, 0.0, 0u64)
    } else {
        app.list.iter().filter_map(|id| app.track(*id)).fold((0, 0.0, 0u64), |(n, s, b), t| (n + 1, s + t.duration, b + t.file_size))
    };
    let h = (secs / 3600.0).floor();
    let m = ((secs - h * 3600.0) / 60.0).floor();
    let total = format!("{n} Tracks, {} hours {} minutes, {:.1} GB", h, m, bytes as f64 / 1e9);
    p.text(pos2(r.max.x - 14.0, r.center().y), Align2::RIGHT_CENTER, total, FontId::proportional(11.0), DIM);
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
pub fn browser(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let strip = Rect::from_min_size(full.min, vec2(30.0, full.height()));
    let tree = Rect::from_min_max(pos2(strip.max.x, full.min.y), pos2(strip.max.x + 219.0, full.max.y));
    let right_strip = Rect::from_min_max(pos2(full.max.x - 26.0, full.min.y), full.max);
    let mytag_w = if app.prefs.show_mytag { 230.0 } else { 0.0 };
    let mytag = Rect::from_min_max(pos2(right_strip.min.x - mytag_w, full.min.y), pos2(right_strip.min.x, full.max.y));
    let list = Rect::from_min_max(pos2(tree.max.x + 4.0, full.min.y), pos2(mytag.min.x - 2.0, full.max.y));
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    p.rect_filled(strip, 0.0, BG2);
    p.rect_filled(tree, 0.0, BG2);
    p.rect_filled(right_strip, 0.0, BG2);

    icon_strip(app, ui, strip);
    let mut drop_targets: Vec<(Rect, i64)> = vec![];
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(tree.shrink2(vec2(4.0, 4.0))));
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

fn icon_strip(app: &mut App, ui: &mut Ui, r: Rect) {
    let items: [(&str, SideMode, &str); 3] = [("♫", SideMode::Collection, "Collection"), ("☰", SideMode::Playlists, "Playlists"), ("⏏", SideMode::Devices, "Devices")];
    for (i, (icon, mode, name)) in items.iter().enumerate() {
        let rr = Rect::from_min_size(pos2(r.min.x + 3.0, r.min.y + 26.0 + i as f32 * 28.0), vec2(24.0, 24.0));
        let on = app.side == *mode;
        if on {
            ui.painter().rect_filled(rr, 2.0, BLUE);
        }
        let resp = ui.interact(rr, ui.id().with(("side", i)), Sense::click());
        ui.painter().text(rr.center(), Align2::CENTER_CENTER, *icon, FontId::proportional(14.0), if on { WHITE } else if resp.hovered() { DIM2 } else { OFF });
        if resp.on_hover_text(*name).clicked() {
            app.side = *mode;
            match mode {
                SideMode::Collection => app.set_view(View::Collection),
                SideMode::Devices => {
                    app.scan_devices();
                }
                SideMode::Playlists => {}
            }
        }
    }
}

fn right_icons(app: &mut App, ui: &mut Ui, r: Rect) {
    let items = [("🏷", "Display My Tag configuration window."), ("⏷", "Filter"), ("ⓘ", "Display information window.")];
    for (i, (icon, tipt)) in items.iter().enumerate() {
        let rr = Rect::from_min_size(pos2(r.min.x + 2.0, r.min.y + 160.0 + i as f32 * 34.0), vec2(22.0, 22.0));
        let on = match i {
            0 => app.prefs.show_mytag,
            1 => app.filter_open,
            _ => false,
        };
        if on {
            ui.painter().rect_filled(rr, 2.0, BLUE);
        }
        let resp = ui.interact(rr, ui.id().with(("ricon", i)), Sense::click());
        ui.painter().text(rr.center(), Align2::CENTER_CENTER, *icon, FontId::proportional(13.0), if on || resp.hovered() { WHITE } else { DIM });
        if resp.on_hover_text(*tipt).clicked() {
            match i {
                0 => {
                    app.prefs.show_mytag = !app.prefs.show_mytag;
                    app.save_prefs();
                }
                1 => {
                    app.filter_open = !app.filter_open;
                    app.list_dirty = true;
                }
                _ => {
                    if let Some(id) = app.list.iter().find(|t| app.selected.contains(t)).copied() {
                        app.dialogs.info_track = app.track(id).cloned();
                    }
                }
            }
        }
    }
}

fn tree_row(ui: &mut Ui, depth: usize, text: &str, count: Option<usize>, selected: bool, icon: &str, id: egui::Id) -> egui::Response {
    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rr, 0.0, SELECT);
    } else if resp.hovered() {
        p.rect_filled(rr, 0.0, hex(0x262626));
    }
    let x = rr.min.x + 6.0 + depth as f32 * 16.0;
    p.text(pos2(x, rr.center().y), Align2::LEFT_CENTER, icon, FontId::proportional(11.0), DIM2);
    p.text(pos2(x + 16.0, rr.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), if selected { WHITE } else { TEXT });
    if let Some(c) = count {
        p.text(pos2(rr.max.x - 8.0, rr.center().y), Align2::RIGHT_CENTER, c.to_string(), FontId::proportional(11.0), WHITE);
    }
    let _ = id;
    resp
}

fn tree_pane(app: &mut App, ui: &mut Ui, drop: &mut Vec<(Rect, i64)>) {
    match app.side {
        SideMode::Collection => {
            ui.label(RichText::new("Collection").size(12.5).color(WHITE));
            let sel = app.view == View::Collection;
            let n = app.tracks.len();
            if tree_row(ui, 1, "All", Some(n), sel, "", ui.id().with("all")).clicked() {
                app.set_view(View::Collection);
            }
            ui.add_space(4.0);
            for f in ["Date Added", "Genre", "Artist", "Album"] {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(RichText::new(format!("🔒 {f}")).size(11.5).color(DIM));
                });
            }
        }
        SideMode::Playlists => {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let _ = button(ui, Rect::from_min_size(ui.cursor().min, vec2(92.0, 20.0)), "treeview", "Tree View", BLUE, WHITE, 11.0);
                ui.add_space(96.0);
                ui.label(RichText::new("Column View 🔒").size(11.0).color(DIM));
            });
            ui.add_space(26.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let all_sel = app.view == View::Collection;
                if tree_row(ui, 0, "All Tracks", Some(app.tracks.len()), all_sel, "♫", ui.id().with("alltracks")).clicked() {
                    app.set_view(View::Collection);
                }
                let root = tree_row(ui, 0, "Playlists", None, false, "▼", ui.id().with("plroot"));
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
            NodeKind::Folder => (if open { "▼" } else { "▶" }, None),
            NodeKind::Smart => ("⚙", n.smart.as_ref().map(|r| app.smart_tracks(r).len())),
            NodeKind::Playlist => ("☰", Some(app.node_counts.get(&n.id).copied().unwrap_or(0))),
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
        let resp = tree_row(ui, depth, &n.name, count, sel, icon, ui.id().with(("node", n.id)));
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
    tree_row(ui, 0, "Devices", None, !sel_dev, "▼", ui.id().with("devroot"));
    let devs: Vec<(String, String, String, Option<Vec<crate::export::usb::DevNode>>, Option<String>, usize)> = app
        .devices
        .iter()
        .map(|d| (d.root.clone(), d.name.clone(), d.fs.clone(), d.lib.as_ref().map(|l| l.nodes.clone()), d.error.clone(), d.lib.as_ref().map(|l| l.tracks.len()).unwrap_or(0)))
        .collect();
    for (root, name, fs, nodes, err, ntracks) in devs {
        let sel = app.view == View::Device { root: root.clone(), node: None };
        let resp = tree_row(ui, 1, &format!("{name}  ({})", App::fs_label(&fs)), Some(ntracks), sel, "⏏", ui.id().with(("dev", &root)));
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
                    let resp = tree_row(ui, depth, &n.name, if n.folder { None } else { Some(n.entries.len()) }, sel, if n.folder { "▶" } else { "☰" }, ui.id().with(("dn", root, n.id)));
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
    // Title bar
    let bar = Rect::from_min_size(r.min, vec2(r.width(), 26.0));
    ui.painter().rect_filled(bar, 0.0, BG2);
    ui.painter().text(pos2(bar.min.x + 12.0, bar.center().y), Align2::LEFT_CENTER, format!("{} ({} Tracks)", view_title(app), app.list.len()), FontId::proportional(12.5), WHITE);
    // search box
    let sr = Rect::from_min_max(pos2(bar.max.x - 310.0, bar.min.y + 3.0), pos2(bar.max.x - 34.0, bar.max.y - 3.0));
    ui.painter().rect_filled(sr, 2.0, BG);
    ui.painter().text(pos2(sr.min.x + 8.0, sr.center().y), Align2::LEFT_CENTER, "🔍", FontId::proportional(11.0), DIM);
    let mut s = app.search.clone();
    let te = ui.put(Rect::from_min_max(pos2(sr.min.x + 24.0, sr.min.y), sr.max), egui::TextEdit::singleline(&mut s).frame(false).hint_text("Search within this track list").vertical_align(egui::Align::Center));
    if te.changed() {
        app.search = s;
        app.list_dirty = true;
    }
    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
        te.request_focus();
    }
    let cm = button(ui, Rect::from_min_size(pos2(bar.max.x - 28.0, bar.min.y + 3.0), vec2(22.0, 20.0)), "colmenu", "≡", BG2, WHITE, 15.0);
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
    let area = Rect::from_min_max(pos2(r.min.x, top), r.max);
    let mut aui = ui.new_child(egui::UiBuilder::new().max_rect(area));
    let cols = app.prefs.columns.clone();
    let mut all_cols = vec![Col::Status];
    all_cols.extend(cols.iter().copied().filter(|c| *c != Col::Status));
    let total_w: f32 = all_cols.iter().map(|c| col_width(app, *c)).sum::<f32>() + 12.0;
    let manual = app.is_manual_playlist();
    egui::ScrollArea::horizontal().id_salt("hscroll").auto_shrink([false, false]).show(&mut aui, |ui| {
        let w = total_w.max(ui.available_width());
        // header
        let (hr, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
        ui.painter().rect_filled(hr, 0.0, BG2);
        let mut x = hr.min.x;
        for c in &all_cols {
            let cw = col_width(app, *c);
            let cr = Rect::from_min_size(pos2(x, hr.min.y), vec2(cw, hr.height()));
            let resp = ui.interact(cr, ui.id().with(("hdr", *c)), Sense::click());
            let sorted = app.prefs.sort_col == *c;
            let label = if *c == Col::Status { "" } else { c.label() };
            let txt = if sorted { format!("{label} {}", if app.prefs.sort_asc { "↑" } else { "↓" }) } else { label.to_string() };
            ui.painter().with_clip_rect(cr).text(pos2(cr.min.x + 6.0, cr.center().y), Align2::LEFT_CENTER, txt, FontId::proportional(11.5), if sorted { BLUE } else { WHITE });
            ui.painter().line_segment([pos2(cr.max.x - 1.0, cr.min.y + 3.0), pos2(cr.max.x - 1.0, cr.max.y - 3.0)], Stroke::new(1.0, hex(0x3A3A3A)));
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
                                let s = (((pp.x - x - 4.0) / 14.0).floor() as i32 + 1).clamp(0, 5) as u8;
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
            if t.color > 0 {
                p.rect_filled(Rect::from_min_size(r.min + vec2(2.0, 2.0), vec2(3.0, r.height() - 4.0)), 0.0, label_color(t.color));
            }
            match t.analyzed {
                3 => {
                    p.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, "···", FontId::proportional(10.0), BLUE);
                }
                2 => {
                    p.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, "!", FontId::proportional(11.0), CAUTION_TX);
                }
                0 => {
                    p.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, "0%", FontId::proportional(9.5), DIM);
                }
                _ => {}
            }
            if let Some((h, m)) = app.cue_counts.get(&t.id) {
                if h + m > 0 {
                    p.text(pos2(r.max.x - 4.0, r.center().y), Align2::RIGHT_CENTER, "CUE", FontId::proportional(8.5), WHITE);
                }
            }
            if t.missing {
                p.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, "⚠", FontId::proportional(10.0), CAUTION_TX);
            }
        }
        Col::Preview => {
            if let Some(pv) = app.previews.get(&t.id) {
                let n = (r.width() - 8.0).max(1.0) as usize;
                let frames: Vec<[u8; 4]> = pv.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                let w = Waveform { frames };
                let cols = w.columns(n);
                let base = r.max.y - 2.0;
                let hh = r.height() - 4.0;
                for (k, v) in cols.iter().enumerate() {
                    let x = r.min.x + 4.0 + k as f32 + 0.5;
                    for (h, cc) in crate::ui_player::wave_bars(app.prefs.wave_color, *v) {
                        p.line_segment([pos2(x, base), pos2(x, base - h * hh)], Stroke::new(1.0, cc));
                    }
                }
            }
        }
        Col::Artwork => {
            let sq = Rect::from_center_size(r.center(), vec2(r.height() - 2.0, r.height() - 2.0));
            if let Some(tex) = artwork_texture(app, &t.artwork) {
                p.image(tex.id(), sq, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        Col::Title => txt(t.display_title()),
        Col::Artist => txt(t.artist.clone()),
        Col::Album => txt(t.album.clone()),
        Col::Genre => txt(t.genre.clone()),
        Col::Bpm => txt(if t.bpm > 0.0 { format!("{:.2}", t.bpm) } else { String::new() }),
        Col::Rating => {
            for s in 1..=5u8 {
                star(&p, pos2(r.min.x + 10.0 + (s - 1) as f32 * 14.0, r.center().y), 5.5, if t.rating >= s { WHITE } else { DIM }, t.rating >= s);
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
