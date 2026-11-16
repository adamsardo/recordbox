//! PrepDeck: local DJ library, track analysis, set preparation and Pioneer-compatible USB export.

use eframe::egui;
mod analysis;
mod app;
mod audio;
mod db;
mod demo;
mod export;
mod icons;
mod importer;
mod jobs;
mod key;
mod menus;
mod model;
mod phrase;
mod rbimport;
mod stems;
mod smart;
mod theme;
mod ui_browser;
mod ui_dialogs;
mod ui_player;
mod xmlexport;

use app::App;

struct Gui {
    app: App,
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let app = &mut self.app;
        app.poll_import();
        app.poll_analysis();
        app.poll_load();
        app.poll_export();
        app.poll_rekordbox_sync();
        app.poll_stems();
        app.poll_phrases();
        menus::poll(app);
        shortcuts(app, ctx);

        egui::TopBottomPanel::top("header").exact_height(29.0).frame(egui::Frame::default().fill(theme::BG1)).show(ctx, |ui| {
            ui_browser::header_bar(app, ui);
        });
        egui::TopBottomPanel::bottom("footer").exact_height(28.0).frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
            ui_browser::footer(app, ui);
        });
        if !app.prefs.browser_only {
            egui::TopBottomPanel::top("player").exact_height(255.0).frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
                ui_player::player_panel(app, ui);
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
            ui_browser::browser(app, ui);
        });
        ui_dialogs::dialogs(app, ctx);

        if app.analysis_total > 0 || app.importing.is_some() || app.export_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        // --screenshot: draw ~120 frames, then save the window picture and quit.
        if let Some((path, frames)) = app.screenshot.clone() {
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
            });
            if let Some(img) = shot {
                let [w, h] = img.size;
                let mut buf = image::RgbaImage::new(w as u32, h as u32);
                for (i, px) in img.pixels.iter().enumerate() {
                    let [r, g, b, a] = px.to_srgba_unmultiplied();
                    buf.put_pixel((i % w) as u32, (i / w) as u32, image::Rgba([r, g, b, a]));
                }
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = buf.save(&path);
                std::process::exit(0);
            }
            if frames == 120 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            if frames > 400 {
                std::process::exit(1);
            }
            app.screenshot = Some((path, frames + 1));
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.save_prefs();
        let _ = self.app.db.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
}

fn prepdeck_actions() -> Vec<&'static str> {
    app::ACTIONS.iter().map(|a| a.0).collect()
}

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    // Library edit Undo/Redo (text fields keep their own undo while they are being edited).
    if !ctx.wants_keyboard_input() {
        let (z, shift) = ctx.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::Z), i.modifiers.shift));
        if z {
            if shift {
                app.redo_edit();
            } else {
                app.undo_edit();
            }
        }
    }
    // Key releases end cue/hot-cue previews even while a text field has focus, so a preview
    // can never be left playing.
    if ctx.wants_keyboard_input() {
        let rel = |p: &app::Prefs, a: &str| p.key_for(a).map(|k| ctx.input(|i| i.key_released(k))).unwrap_or(false);
        let cue_rel = rel(&app.prefs, "cue");
        let hot_rel = (1..=8).any(|n| rel(&app.prefs, &format!("hot{n}")));
        if app.deck.cue_preview && cue_rel {
            app.cue_up();
        }
        if app.deck.hot_preview.is_some() && hot_rel {
            app.hot_cue_up();
        }
        return;
    }
    use egui::Key;
    let shortcuts: std::collections::HashMap<&str, app::Shortcut> = prepdeck_actions().iter().filter_map(|a| app.prefs.shortcut_for(a).map(|k| (*a, k))).collect();
    let key = |a: &str| shortcuts.get(a).map(|s| s.key);
    // A press of the action's key with exactly its modifiers (beat jumps also take Shift).
    let hit = |a: &str, repeats: bool| {
        let Some(sc) = shortcuts.get(a) else { return false };
        let free_shift = a == "jump_back" || a == "jump_fwd";
        ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if *key == sc.key && (repeats || !*repeat) && sc.mods_match(modifiers, free_shift))))
    };
    let pressed = |a: &str| hit(a, true);
    let released = |a: &str| key(a).map(|k| ctx.input(|i| i.key_released(k))).unwrap_or(false);
    let first = |a: &str| hit(a, false);
    let shift = ctx.input(|i| i.modifiers.shift);
    let cmd_a = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::A));
    let (up, down, enter, left, right) = (pressed("list_up"), pressed("list_down"), pressed("load"), pressed("jump_back"), pressed("jump_fwd"));
    // Delete also answers to Backspace when it is on its default key.
    let del = pressed("remove") || (shortcuts.get("remove").map(|s| s.name()) == Some("Delete".into()) && ctx.input(|i| i.key_pressed(Key::Backspace) && i.modifiers.is_none()));
    if first("play") {
        app.play_pause();
    }
    if first("cue") {
        app.cue_down();
    }
    if released("cue") {
        app.cue_up();
    }
    if first("memory") {
        app.memory_set();
    }
    for k in 0..8 {
        let id = format!("hot{}", k + 1);
        if first(&id) {
            app.hot_cue_down(k as i32);
        }
        if released(&id) {
            app.hot_cue_up();
        }
    }
    if left {
        app.beat_jump(-if shift { 1.0 } else { app.prefs.beat_jump });
    }
    if right {
        app.beat_jump(if shift { 1.0 } else { app.prefs.beat_jump });
    }
    if up || down {
        if app.list_dirty {
            app.rebuild_list();
        }
        if !app.list.is_empty() {
            let cur = app.anchor.unwrap_or(0);
            let next = if up { cur.saturating_sub(1) } else { (cur + 1).min(app.list.len() - 1) };
            let next = if app.anchor.is_none() { 0 } else { next };
            app.set_anchor(next);
            app.selected.clear();
            app.selected.insert(app.list[next]);
            app.scroll_to_selected = true;
        }
    }
    if enter {
        if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
            app.load_track(id);
        }
    }
    if cmd_a {
        app.selected = app.list.iter().copied().collect();
    }
    if del && !app.selected.is_empty() {
        if let Some(pl) = app.is_manual_playlist() {
            let s = app.selected.clone();
            app.remove_from_playlist(pl, &s);
        }
    }
}


fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let screenshot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1).cloned());
    let shot_mode = screenshot.is_some();
    let mut viewport = egui::ViewportBuilder::default().with_title("PrepDeck").with_inner_size([1512.0, 921.0]).with_min_inner_size([1100.0, 700.0]);
    if shot_mode {
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    if shot_mode {
        options.event_loop_builder = Some(Box::new(|b| {
            #[cfg(target_os = "macos")]
            {
                use winit::platform::macos::EventLoopBuilderExtMacOS;
                b.with_activate_ignoring_other_apps(false);
            }
            let _ = b;
        }));
        options.run_and_return = false;
    }
    eframe::run_native("PrepDeck", options, Box::new(move |cc| Ok(Box::new(Gui { app: App::new(cc, screenshot) }))))
}
