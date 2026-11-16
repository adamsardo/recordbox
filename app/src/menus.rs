//! Native macOS menu bar (the original keeps File / Edit / View / Track / Playlist / Help in the
//! system menu bar, not inside the window).

use crate::app::*;
use crate::model::*;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

pub fn build() -> Option<Menu> {
    let menu = Menu::new();
    let item = |id: &str, text: &str, acc: Option<(Modifiers, Code)>| MenuItem::with_id(id, text, true, acc.map(|(m, c)| Accelerator::new(m, c)));
    let cmd = Modifiers::META;
    let app_menu = Submenu::new("PrepDeck", true);
    app_menu
        .append_items(&[
            &item("about", "About PrepDeck", None),
            &PredefinedMenuItem::separator(),
            &item("prefs", "Preferences…", Some((cmd, Code::Comma))),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])
        .ok()?;
    let file = Submenu::new("File", true);
    file.append_items(&[
        &item("import_track", "Import Track…", Some((cmd, Code::KeyO))),
        &item("import_folder", "Import Folder…", Some((cmd | Modifiers::SHIFT, Code::KeyO))),
        &item("import_playlist", "Import Playlist…", None),
        &PredefinedMenuItem::separator(),
        &item("sync_rekordbox", "Sync rekordbox Library Now", None),
        &item("import_xml", "Import Collection from rekordbox xml…", None),
        &item("export_xml", "Export Collection in xml format…", None),
        &PredefinedMenuItem::separator(),
        &item("missing", "Display All Missing Files", None),
        &item("backup", "Backup Library…", None),
        &item("library_folder", "Show Library Folder in Finder", None),
    ])
    .ok()?;
    let edit = Submenu::new("Edit", true);
    edit.append_items(&[
        &item("undo", "Undo", None),
        &item("redo", "Redo", None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
    ])
    .ok()?;
    let view = Submenu::new("View", true);
    view.append_items(&[
        &item("layout_player", "1 Player", None),
        &item("layout_browser", "Full Browser", None),
        &PredefinedMenuItem::separator(),
        &item("mytag_window", "My Tag Configuration Window", None),
        &item("filter", "Filter", None),
        &PredefinedMenuItem::separator(),
        &item("wave_blue", "Waveform: BLUE", None),
        &item("wave_rgb", "Waveform: RGB", None),
        &item("wave_3band", "Waveform: 3Band", None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::fullscreen(None),
    ])
    .ok()?;
    let track = Submenu::new("Track", true);
    track
        .append_items(&[
            &item("load", "Load to Player", None),
            &PredefinedMenuItem::separator(),
            &item("analyze", "Analyze Track", None),
            &item("analyze_key", "Analyze Key", None),
            &item("analysis_lock", "Analysis Lock / Unlock", None),
            &item("auto_analysis", "Auto Analysis On / Off", None),
            &item("stop_analysis", "Stop Analyzing", None),
            &PredefinedMenuItem::separator(),
            &item("export_tracks", "Export Track…", None),
            &item("property", "Property…", Some((cmd, Code::KeyI))),
            &item("reload_tags", "Reload Tag", None),
            &item("show_finder", "Show in Finder", None),
            &PredefinedMenuItem::separator(),
            &item("remove_collection", "Remove from Collection", None),
            &item("trash", "Restore Removed Tracks…", None),
        ])
        .ok()?;
    let playlist = Submenu::new("Playlist", true);
    playlist
        .append_items(&[
            &item("new_playlist", "Create New Playlist", Some((cmd, Code::KeyN))),
            &item("new_smart", "Create New Intelligent Playlist", None),
            &item("new_folder", "Create New Folder", None),
            &PredefinedMenuItem::separator(),
            &item("export_device", "Export to USB Device…", Some((cmd, Code::KeyE))),
        ])
        .ok()?;
    let help = Submenu::new("Help", true);
    help.append_items(&[&item("about", "About PrepDeck", None), &item("shortcuts", "Keyboard Shortcuts", None)]).ok()?;
    menu.append_items(&[&app_menu, &file, &edit, &view, &track, &playlist, &help]).ok()?;
    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();
    Some(menu)
}

pub fn poll(app: &mut App) {
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        let id: &str = ev.id().as_ref();
        handle(app, id);
    }
}

fn selected(app: &App) -> Vec<i64> {
    app.list.iter().copied().filter(|t| app.selected.contains(t)).collect()
}

pub fn handle(app: &mut App, id: &str) {
    let sel = selected(app);
    match id {
        "about" => app.dialogs.about = true,
        "undo" => app.undo_edit(),
        "trash" => app.dialogs.trash = Some(Default::default()),
        "redo" => app.redo_edit(),
        "prefs" => app.dialogs.prefs_open = true,
        "import_track" => {
            if let Some(files) = rfd::FileDialog::new().add_filter("Audio", &["mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "alac", "mp4"]).pick_files() {
                let pl = app.is_manual_playlist();
                app.import_paths(files, pl);
            }
        }
        "import_folder" => {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                let pl = app.is_manual_playlist();
                app.import_paths(vec![d], pl);
            }
        }
        "import_playlist" => {
            if let Some(f) = rfd::FileDialog::new().add_filter("Playlist", &["m3u", "m3u8"]).pick_file() {
                crate::ui_browser::import_m3u(app, &f);
            }
        }
        "sync_rekordbox" => {
            if crate::rbimport::library_dir().is_some() {
                app.start_rekordbox_sync();
            } else {
                app.toast_err("No rekordbox library was found on this computer.");
            }
        }
        "import_xml" => {
            if let Some(f) = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file() {
                app.import_xml(&f);
            }
        }
        "export_xml" => {
            if let Some(f) = rfd::FileDialog::new().set_file_name("collection.xml").save_file() {
                app.export_collection_xml(&f);
            }
        }
        "missing" => {
            app.side = SideMode::Collection;
            app.set_view(View::Collection);
            let missing: Vec<i64> = app.tracks.iter().filter(|t| t.missing).map(|t| t.id).collect();
            app.toast(format!("{} missing file(s) in the collection", missing.len()));
            app.selected = missing.into_iter().collect();
        }
        "backup" => {
            if let Some(f) = rfd::FileDialog::new().set_file_name("PrepDeck-library-backup.db").save_file() {
                let _ = app.db.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                match std::fs::copy(crate::db::data_dir().join("library.db"), &f) {
                    Ok(_) => app.toast("Library backed up."),
                    Err(e) => app.toast_err(format!("Backup failed: {e}")),
                }
            }
        }
        "library_folder" => {
            let _ = std::process::Command::new("/usr/bin/open").arg(crate::db::data_dir()).spawn();
        }
        "layout_player" => {
            app.prefs.browser_only = false;
            app.save_prefs();
        }
        "layout_browser" => {
            app.prefs.browser_only = true;
            app.save_prefs();
        }
        "mytag_window" => {
            app.prefs.show_mytag = !app.prefs.show_mytag;
            app.save_prefs();
        }
        "filter" => {
            app.filter_open = !app.filter_open;
            app.list_dirty = true;
        }
        "wave_blue" | "wave_rgb" | "wave_3band" => {
            app.prefs.wave_color = match id {
                "wave_blue" => 0,
                "wave_rgb" => 1,
                _ => 2,
            };
            app.save_prefs();
        }
        "load" => {
            if let Some(t) = sel.first() {
                app.load_track(*t);
            }
        }
        "analyze" => app.reanalyze(&sel),
        "analyze_key" => app.analyze_key_tracks(&sel),
        "analysis_lock" => {
            let locked = sel.iter().all(|id| app.track(*id).map(|t| t.grid_locked).unwrap_or(false));
            for id in &sel {
                if let Some(t) = app.track_mut(*id) {
                    t.grid_locked = !locked;
                }
                app.save_track(*id);
            }
        }
        "auto_analysis" => {
            app.prefs.auto_analysis = !app.prefs.auto_analysis;
            app.save_prefs();
            let s = if app.prefs.auto_analysis { "Auto Analysis On" } else { "Auto Analysis Off" };
            app.toast(s);
        }
        "stop_analysis" => app.pool.stop(),
        "export_tracks" => crate::ui_browser::open_export(app, vec![], sel),
        "property" => {
            if let Some(t) = sel.first() {
                app.dialogs.info_track = app.track(*t).cloned();
            }
        }
        "reload_tags" => app.reload_tags_from_files(&sel),
        "show_finder" => {
            if let Some(t) = sel.first().and_then(|t| app.track(*t)) {
                let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(&t.path).spawn();
            }
        }
        "remove_collection" => {
            if !sel.is_empty() {
                app.dialogs.confirm_remove = Some(sel);
            }
        }
        "new_playlist" => {
            app.side = SideMode::Playlists;
            app.create_node(0, NodeKind::Playlist);
        }
        "new_smart" => {
            app.side = SideMode::Playlists;
            app.create_node(0, NodeKind::Smart);
        }
        "new_folder" => {
            app.side = SideMode::Playlists;
            app.create_node(0, NodeKind::Folder);
        }
        "export_device" => crate::ui_browser::open_export(app, vec![], vec![]),
        "shortcuts" => app.toast("Space play/pause · C cue · 1–8 hot cues · M memory · ←/→ beat jump · ↑/↓ select · Return load · ⌘F search"),
        _ => {}
    }
}
