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

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    if ctx.wants_keyboard_input() {
        return;
    }
    use egui::Key;
    let fresh = |k: Key| ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if *key == k && !modifiers.command)));
    let (_s, _c, c_up, enter, up, down, cmd_a, del, _m, left, right, shift) = ctx.input(|i| {
        (
            false,
            false,
            i.key_released(Key::C),
            i.key_pressed(Key::Enter),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.modifiers.command && i.key_pressed(Key::A),
            i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
            false,
            i.key_pressed(Key::ArrowLeft),
            i.key_pressed(Key::ArrowRight),
            i.modifiers.shift,
        )
    });
    let (space, c_down, m) = (fresh(Key::Space), fresh(Key::C), fresh(Key::M));
    if space {
        app.play_pause();
    }
    if c_down {
        app.cue_down();
    }
    if c_up {
        app.cue_up();
    }
    if m {
        app.memory_set();
    }
    for (k, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8].iter().enumerate() {
        if fresh(*key) {
            app.hot_cue_down(k as i32);
        }
        if ctx.input(|i| i.key_released(*key)) {
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
            app.anchor = Some(next);
            app.selected.clear();
            app.selected.insert(app.list[next]);
            app.scroll_to_selected = true;
        }
    }
    if enter {
        if let Some(i) = app.anchor {
            if let Some(id) = app.list.get(i).copied() {
                app.load_track(id);
            }
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
