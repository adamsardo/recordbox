//! Recordbox: local DJ library, track analysis, set preparation and Pioneer-compatible USB export.

use eframe::egui;
mod analysis;
mod app;
mod audio;
mod backup;
mod brand;
mod cfx;
mod browser_ext;
mod collection_cleanup;
mod db;
mod demo;
mod device_ext;
mod export;
mod fx;
mod icons;
mod icons_player;
mod icons_src;
mod importer;
mod itunes;
mod jobs;
mod key;
mod library_ext;
mod menus;
mod mixer;
mod perf_selftest;
mod sampler;
mod sampler_ext;
mod sequencer;
mod ui_sampler;
mod model;
mod manual;
mod phrase;
mod playlist_files;
mod preview;
mod player_ext;
mod prefs_ext;
mod rbimport;
mod stems;
mod smart;
mod theme;
mod ui_browser;
mod ui_browser_ext;
mod ui_dialogs;
mod ui_kit;
mod ui_library_ext;
mod ui_player;
mod ui_prefs;
mod ui_sync;
mod ui_fx;
mod ui_perf;
mod ui_player2;
mod xmlexport;

use app::App;

struct Gui {
    app: App,
    pending_docked_size: Option<egui::Vec2>,
}

const MAIN_MIN_SIZE: egui::Vec2 = egui::vec2(1100.0, 700.0);

struct DetachedLibrary {
    builder: egui::ViewportBuilder,
    docked_size: Option<egui::Vec2>,
    deck_window_height: f32,
}

impl DetachedLibrary {
    fn id() -> egui::ViewportId {
        egui::ViewportId::from_hash_of("recordbox_library")
    }

    fn open(ctx: &egui::Context, player_height: f32) -> Self {
        let size = ctx.screen_rect().size();
        let docked_size = windowed(ctx).then_some(size);
        let deck_window_height = player_height + 56.0; // Header and status bar.
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(MAIN_MIN_SIZE.x, deck_window_height)));
        if docked_size.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(size.x, deck_window_height)));
        }
        Self {
            builder: egui::ViewportBuilder::default().with_title("recordbox — Library")
                .with_inner_size([size.x, (size.y - deck_window_height).max(400.0)])
                .with_min_inner_size([900.0, 360.0])
                .with_fullscreen(ctx.input(|i| i.viewport().fullscreen == Some(true))),
            docked_size,
            deck_window_height,
        }
    }

    fn fit_decks(&mut self, ctx: &egui::Context, player_height: f32) {
        let height = player_height + 56.0;
        if self.deck_window_height != height {
            let size = ctx.screen_rect().size();
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(MAIN_MIN_SIZE.x, height)));
            // Follow deck layout changes only while the user has kept the compact height.
            if windowed(ctx) && (size.y - self.deck_window_height).abs() <= 2.0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(size.x, height)));
            }
            self.deck_window_height = height;
        }
    }

    fn reattach(self, ctx: &egui::Context) -> Option<egui::Vec2> {
        ctx.request_repaint();
        if self.docked_size.is_some() && !windowed(ctx) {
            // The WM must first be allowed to restore its saved, deck-height window.
            // Raising the minimum now can prevent it from leaving maximized mode on X11.
            return self.docked_size;
        }
        restore_docked_size(ctx, self.docked_size);
        None
    }
}

fn restore_docked_size(ctx: &egui::Context, size: Option<egui::Vec2>) {
    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(MAIN_MIN_SIZE));
    if let Some(size) = size {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    }
}

fn windowed(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().fullscreen != Some(true) && i.viewport().maximized != Some(true))
}

fn window_input(app: &mut App, ctx: &egui::Context) {
    menus::apply_edit_commands(app, ctx);
    window_fullscreen(&app.prefs, ctx, app.library_window.is_some());
    if ctx.input(|i| i.focused || i.events.contains(&egui::Event::WindowFocused(false))) {
        shortcuts(app, ctx);
    }
}

fn library_panel(app: &mut App, ctx: &egui::Context) {
    egui::CentralPanel::default().frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
        ui_browser::browser(app, ui);
    });
}

impl eframe::App for Gui {
    /// PREPDECK_SHOT_DRAG (with --screenshot): drag hot cue A 100 px to the right with real
    /// pointer events, to check marker dragging end to end; the result goes to stderr.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some((_, frame)) = self.app.screenshot.clone() else { return };
        self.app.shot_player_events(frame, raw);
        self.app.shot_perf_events(frame, raw);
        if std::env::var("PREPDECK_SHOT_DRAG").is_err() {
            return;
        }
        let Some((r, pos, pps)) = self.app.deck.detail_map else { return };
        let Some(a) = self.app.deck.cues.iter().find(|c| c.kind == model::CueKind::Hot && c.slot == 0).cloned() else { return };
        let ax = r.center().x + ((a.start - pos) * pps) as f32 + 2.0;
        let y = r.min.y - 7.0;
        static START: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
        let press = |down: bool, x: f32| egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Primary, pressed: down, modifiers: Default::default() };
        let mv = |x: f32| egui::Event::PointerMoved(egui::pos2(x, y));
        match frame {
            80 => {
                let x0 = *START.get_or_init(|| ax);
                eprintln!("drag: hot cue A at {:.3} s, grabbing at x {x0:.1}", a.start);
                raw.events.push(mv(x0));
            }
            81 => raw.events.push(press(true, *START.get().unwrap_or(&ax))),
            82..=91 => {
                let x0 = *START.get().unwrap_or(&ax);
                raw.events.push(mv(x0 + 10.0 * (frame - 81) as f32));
            }
            92 => raw.events.push(press(false, START.get().copied().unwrap_or(ax) + 100.0)),
            100 => {
                let saved = self.app.db.cues(a.track_id).unwrap_or_default().into_iter().find(|c| c.kind == model::CueKind::Hot && c.slot == 0);
                eprintln!("drag: after: deck {:.3} s, saved {:?}, undo steps {}, 100 px = {:.3} s", a.start, saved.map(|c| c.start), self.app.undo_len(), 100.0 / pps);
            }
            101 => {
                self.app.undo_edit();
                let saved = self.app.db.cues(a.track_id).unwrap_or_default().into_iter().find(|c| c.kind == model::CueKind::Hot && c.slot == 0);
                eprintln!("drag: after Undo: deck {:.3} s, saved {:?}", self.app.deck.cues.iter().find(|c| c.id == a.id).map(|c| c.start).unwrap_or(-1.0), saved.map(|c| c.start));
            }
            // Second drag, cancelled with Esc while the button is still down.
            102 => raw.events.push(mv(ax)),
            103 => raw.events.push(press(true, ax)),
            104..=108 => raw.events.push(mv(ax + 15.0 * (frame - 103) as f32)),
            109 => eprintln!("drag: during second drag: deck {:.3} s", a.start),
            110 => raw.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }),
            111 => raw.events.push(mv(ax + 120.0)),
            112 => raw.events.push(press(false, ax + 120.0)),
            114 => {
                let saved = self.app.db.cues(a.track_id).unwrap_or_default().into_iter().find(|c| c.kind == model::CueKind::Hot && c.slot == 0);
                eprintln!("drag: after Esc: deck {:.3} s, saved {:?}, undo steps {}", a.start, saved.map(|c| c.start), self.app.undo_len());
            }
            _ => {}
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let bench_t0 = std::time::Instant::now();
        let app = &mut self.app;
        if self.pending_docked_size.is_some() && windowed(ctx) {
            restore_docked_size(ctx, self.pending_docked_size.take());
        }
        // PREPDECK_BENCH: name any step that holds a frame up for more than 4 ms.
        macro_rules! step {
            ($e:expr) => {{
                let t = std::time::Instant::now();
                $e;
                if bench_on() && t.elapsed().as_millis() >= 4 {
                    eprintln!("bench: slow step {} {:.1} ms", stringify!($e), t.elapsed().as_secs_f32() * 1000.0);
                }
            }};
        }
        step!(app.poll_import());
        step!(app.poll_analysis());
        step!(app.poll_load());
        step!(app.poll_decks());
        step!(app.poll_export());
        step!(app.poll_rekordbox_sync());
        step!(app.poll_stems());
        step!(app.poll_phrases());
        step!(app.poll_vocal());
        step!(app.poll_key_job());
        step!(app.poll_audio_device());
        step!(app.poll_relocate());
        step!(backup::poll(app));
        step!(menus::poll(app));
        step!(menus::sync(app));
        window_input(app, ctx);

        let toggle_library = egui::TopBottomPanel::top("header").exact_height(28.0).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG1)).show(ctx, |ui| {
            ui_browser::header_bar(app, ui)
        }).inner;
        if toggle_library {
            if let Some(window) = app.library_window.take() {
                self.pending_docked_size = window.reattach(ctx);
            } else if !ctx.embed_viewports() {
                let mut window = DetachedLibrary::open(ctx, app.player_height());
                window.docked_size = window.docked_size.or(self.pending_docked_size.take());
                app.library_window = Some(window);
            }
        }
        egui::TopBottomPanel::bottom("footer").exact_height(28.0).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
            ui_browser::footer(app, ui);
        });
        let decks_visible = !app.full_browser() || app.performance();
        if decks_visible {
            // The header is 28 pt and 1 Player / 2 Players 256 pt tall with no rule between them
            // (measured on rekordbox: the player ends 284 pt below the window top); the Simple
            // Player is one 58 pt row.
            let h = app.player_height();
            let perf = app.performance();
            let draw_players = |app: &mut App| {
                egui::TopBottomPanel::top(egui::Id::new(("player", if perf { 9 } else { app.prefs.player_layout.min(3) }))).exact_height(h).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| match app.prefs.player_layout {
                    _ if perf => step!(ui_perf::performance(app, ui)),
                    1 | 3 => step!(ui_player2::two_players(app, ui)),
                    2 => step!(ui_player2::simple_player(app, ui)),
                    _ => step!(ui_player::player_panel(app, ui)),
                });
            };
            if app.library_window.is_some() {
                app.with_detached_player_drag(ctx, draw_players);
            } else {
                draw_players(app);
            }
        }
        let library_id = DetachedLibrary::id();
        let deck_height = app.player_height();
        if let Some(window) = &mut app.library_window {
            window.fit_decks(ctx, deck_height);
            let reattach = ctx.show_viewport_immediate(library_id, window.builder.clone(), |child, class| {
                if class == egui::ViewportClass::Embedded || child.input(|i| i.viewport().close_requested()) {
                    return true;
                }
                window_input(app, child);
                step!(library_panel(app, child));
                step!(ui_dialogs::library_dialogs(app, child));
                false
            });
            if reattach {
                if let Some(window) = app.library_window.take() {
                    self.pending_docked_size = window.reattach(ctx);
                }
            }
        }
        if app.library_window.is_none() {
            step!(library_panel(app, ctx));
        }
        step!(ui_prefs::prefs_window(app, ctx));
        if app.library_window.is_none() {
            step!(ui_dialogs::library_dialogs(app, ctx));
        } else if app.ext.cleanup.is_some() {
            egui::Modal::new("detached_collection_cleanup".into()).show(ctx, |ui| {
                ui.label("Collection cleanup is open in the library browser.");
                if ui.button("Show library").clicked() {
                    ctx.send_viewport_cmd_to(library_id, egui::ViewportCommand::Focus);
                }
            });
        }
        collection_cleanup::ui::guard_app_close(app, ctx);
        app.update_transport(ctx, decks_visible);

        if app.analysis_total > 0 || app.importing.is_some() || app.export_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        // PREPDECK_BENCH=<frames> (with --screenshot): play the loaded track muted for that many
        // frames before the picture and print how long building each frame took.
        let bench: u32 = std::env::var("PREPDECK_BENCH").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        if bench > 0 {
            if let Some((_, frames)) = app.screenshot.clone() {
                static TIMES: std::sync::Mutex<Vec<f32>> = std::sync::Mutex::new(Vec::new());
                // PREPDECK_BENCH_MUTE=<frame>: mute the vocals at that frame (the stem waveform).
                if std::env::var("PREPDECK_BENCH_MUTE").ok().and_then(|v| v.parse::<u32>().ok()) == Some(frames) {
                    eprintln!("bench: stems ready at the mute {}", app.player.state.lock().stems.is_some());
                    app.player.state.lock().stem_on[3] = false;
                }
                if frames == 60 {
                    if !app.px.muted {
                        app.toggle_mute();
                    }
                    if !app.playing() {
                        app.play_pause();
                    }
                }
                let mut t = TIMES.lock().unwrap();
                if frames > 90 {
                    t.push(bench_t0.elapsed().as_secs_f32() * 1000.0);
                }
                if frames == 119 + bench && !t.is_empty() {
                    let mut v = t.clone();
                    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let avg = v.iter().sum::<f32>() / v.len() as f32;
                    eprintln!("bench: {} frames, update avg {:.2} ms, median {:.2}, p95 {:.2}, max {:.2}", v.len(), avg, v[v.len() / 2], v[v.len() * 95 / 100], v[v.len() - 1]);
                    use std::sync::atomic::Ordering::Relaxed;
                    eprintln!("bench: stems ready {}, separating {:?}, stem error {:?}", app.player.state.lock().stems.is_some(), app.stem.separating, app.stem.error_for(app.deck.track));
                    eprintln!("bench: mixer meters deck 1 {:.1} dB, master {:.1} dB, limiter on {}", app.px.mix[0].shown_db[0], app.px.master_db[0], audio::LIMITER_ON.load(Relaxed));
                    eprintln!("bench: audio callback found a deck held {} times, gave a buffer up {} times", audio::AUDIO_CONTENDED.load(Relaxed), audio::AUDIO_DROPPED.load(Relaxed));
                }
            }
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
            // Taken once the launch-time rekordbox sync has finished, so the picture shows the
            // synced library (up to ~20 s).
            static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if frames >= 120 + bench && (app.rb_job.is_none() || frames >= 1200 + bench) && !ASKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            if frames > 1500 + bench {
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

fn bench_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("PREPDECK_BENCH").is_ok())
}

fn recordbox_actions() -> Vec<&'static str> {
    app::ACTIONS.iter().map(|a| a.0).collect()
}

/// F11 also works inside a search field. An explicit key binding for another action wins.
fn window_fullscreen(prefs: &app::Prefs, ctx: &egui::Context, library_detached: bool) {
    let fullscreen = ctx.input_mut(|i| {
        if !i.focused {
            return None;
        }
        let mut toggle = false;
        i.events.retain(|event| {
            if let egui::Event::Key { key: egui::Key::F11, pressed: true, repeat, modifiers, .. } = event {
                if modifiers.is_none() && !prefs.keymap.iter().any(|(action, key)| action != "fullscreen" && key == "F11") {
                    toggle |= !repeat;
                    return false;
                }
            }
            true
        });
        toggle.then(|| !i.viewport().fullscreen.unwrap_or(false))
    });
    if let Some(fullscreen) = fullscreen {
        set_window_fullscreen(ctx, ctx.viewport_id(), library_detached, fullscreen);
    }
}

fn set_window_fullscreen(ctx: &egui::Context, origin: egui::ViewportId, library_detached: bool, fullscreen: bool) {
    let root = egui::ViewportId::ROOT;
    let library = DetachedLibrary::id();
    if library_detached && (origin == root || origin == library) {
        ctx.send_viewport_cmd_to(root, egui::ViewportCommand::Fullscreen(fullscreen));
        ctx.send_viewport_cmd_to(library, egui::ViewportCommand::Fullscreen(fullscreen));
    } else {
        ctx.send_viewport_cmd_to(origin, egui::ViewportCommand::Fullscreen(fullscreen));
    }
}

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    // Previews never outlive what holds them: losing the window's focus ends them (key-up
    // events are not delivered then), and a hot-cue preview ends once neither a hot-cue key nor
    // a mouse button is held.
    if !ctx.input(|i| i.focused) {
        if app.deck.cue_preview {
            app.cue_up();
        }
        if app.deck.hot_preview.is_some() {
            app.hot_cue_up();
        }
        // A Search key held when the window lost focus is never seen released.
        if app.searching() != 0 && !ctx.input(|i| i.pointer.any_down()) {
            app.search(0);
        }
        return;
    }
    if app.deck.hot_preview.is_some() {
        let hot_held = (1..=8).any(|n| app.prefs.key_for(&format!("hot{n}")).map(|k| ctx.input(|i| i.key_down(k))).unwrap_or(false));
        if !hot_held && !ctx.input(|i| i.pointer.any_down()) {
            app.hot_cue_up();
        }
    }
    if app.ext.cleanup.is_some() {
        return;
    }
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
    let shortcuts: std::collections::HashMap<&str, app::Shortcut> = recordbox_actions().iter().filter_map(|a| app.prefs.shortcut_for(a).map(|k| (*a, k))).collect();
    let key = |a: &str| shortcuts.get(a).map(|s| s.key);
    // A press of the action's key with exactly its modifiers (beat jumps also take Shift).
    // The sampler deck's keys work in PERFORMANCE mode only, and there they come before
    // another action on the same key (they are rekordbox's Performance preset).
    let perf = app.performance();
    let smp_keys: Vec<app::Shortcut> = if perf { shortcuts.iter().filter(|(a, _)| sampler_ext::is_sampler_action(a)).map(|(_, s)| *s).collect() } else { vec![] };
    let hit = |a: &str, repeats: bool| {
        let Some(sc) = shortcuts.get(a) else { return false };
        if sampler_ext::is_sampler_action(a) { if !perf { return false; } } else if smp_keys.contains(sc) { return false; }
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
    // One-shot actions (prefs_ext.rs). A shortcut the menu bar holds as an accelerator is
    // the menu's to answer.
    let mut acts = vec![];
    for (id, _, _, _) in app::ACTIONS {
        if prefs_ext::HELD_ACTIONS.contains(&id) || menus::owns_shortcut(app, id) {
            continue;
        }
        if let Some(act) = prefs_ext::Act::parse(id) {
            if hit(id, act.repeats()) {
                acts.push(act);
            }
        }
    }
    for act in acts {
        prefs_ext::run_action(app, act);
    }
    // A sampler pad's key let go: gate samples stop.
    if perf {
        for n in 0..16usize {
            if released(&format!("smp_play{}", n + 1)) {
                app.sampler_pad_up(n);
            }
        }
    }
    // Search Backward / Forward run while their key is held.
    for (id, dir) in [("search_back", -1), ("search_fwd", 1)] {
        if first(id) {
            app.search(dir);
        } else if released(id) && app.searching() == dir {
            app.search(0);
        }
    }
    // Change Focus: with the keyboard focus on the Tree View the arrows move there.
    let tree = app.dialogs.ext.focus_tree;
    if tree && (left || right) {
        prefs_ext::tree_expand(app, right);
    }
    if tree && (up || down) {
        prefs_ext::tree_step(app, if up { -1 } else { 1 });
    }
    let (left, right, up, down) = (left && !tree, right && !tree, up && !tree, down && !tree);
    // In loop adjust the arrows move the loop edge (Shift: 1 ms) instead of jumping.
    if (left || right) && app.deck.x.loop_adjust != 0 && app.loop_active().is_some() {
        app.loop_adjust_step(if left { -1 } else { 1 }, shift);
    } else if left {
        app.beat_jump(-if shift { 1.0 } else { app.prefs.beat_jump });
    } else if right {
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


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeckOptions {
    pub disable_load: bool,
    pub disable_play: bool,
}

impl DeckOptions {
    fn from_args(args: &[String]) -> Self {
        Self {
            disable_load: args.iter().any(|arg| arg == "--disable-load"),
            disable_play: args.iter().any(|arg| arg == "--disable-play"),
        }
    }
}

#[cfg(test)]
mod launch_tests {
    use super::DeckOptions;

    #[test]
    fn redocking_defers_size_constraints_until_the_window_manager_restores_the_player() {
        use super::{egui, restore_docked_size, DetachedLibrary, MAIN_MIN_SIZE};
        let ctx = egui::Context::default();
        let original = egui::vec2(1440.0, 840.0);
        let mut raw = egui::RawInput::default();
        raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().maximized = Some(true);
        let mut pending = None;
        let output = ctx.run(raw, |ctx| {
            let window = DetachedLibrary { builder: Default::default(), docked_size: Some(original), deck_window_height: 312.0 };
            pending = window.reattach(ctx);
        });
        assert_eq!(pending, Some(original));
        assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.iter().all(|cmd|
            !matches!(cmd, egui::ViewportCommand::MinInnerSize(_) | egui::ViewportCommand::InnerSize(_))
        ));
        let output = ctx.run(Default::default(), |ctx| restore_docked_size(ctx, pending.take()));
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::MinInnerSize(size) if *size == MAIN_MIN_SIZE)));
        assert!(commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::InnerSize(size) if *size == original)));
    }

    #[test]
    fn library_opens_in_the_main_windows_fullscreen_mode() {
        use super::{egui, DetachedLibrary};
        for (fullscreen, maximized) in [(true, false), (false, true), (false, false)] {
            let mut raw = egui::RawInput::default();
            let main = raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
            main.fullscreen = Some(fullscreen);
            main.maximized = Some(maximized);
            let _ = egui::Context::default().run(raw, |ctx| {
                let library = DetachedLibrary::open(ctx, 256.0);
                assert_eq!(library.builder.fullscreen, Some(fullscreen));
                assert_eq!(library.builder.maximized, None);
            });
        }
    }

    #[test]
    fn deck_switches_are_independent_and_session_only() {
        for (args, load, play) in [
            (vec![], false, false), (vec!["--disable-load"], true, false),
            (vec!["--disable-play"], false, true), (vec!["--disable-play", "--disable-load"], true, true)
        ] {
            let args: Vec<_> = args.into_iter().map(String::from).collect();
            assert_eq!(DeckOptions::from_args(&args), DeckOptions { disable_load: load, disable_play: play });
        }
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().skip(1).any(|arg| arg == "--help" || arg == "-h") {
        println!("Usage: recordbox [--disable-load] [--disable-play] [--screenshot FILE] [--stems-cpu] [--stems-status | --stems-device | --compare-stems FILE]\n\n  --disable-load  Disable loading tracks into decks for this session.\n  --disable-play  Disable audio playback and previews for this session.\n  --screenshot    Save a picture of the window to FILE and exit.\n  --stems-cpu     Use CPU separation for this session.\n  --stems-status  Show build, system memory and local model status without starting inference.\n  --stems-device  Check the selected separation device and exit.\n  --compare-stems FILE  Compare GPU and CPU separation of one local track and exit.");
        return Ok(());
    }
    if args.iter().any(|a| a == "--stems-status") {
        println!("{}", stems::status_report());
        return Ok(());
    }
    if args.iter().any(|a| a == "--stems-cpu") {
        stems::force_cpu();
    }
    if let Some(i) = args.iter().position(|a| a == "--compare-stems") {
        let result = args.get(i + 1).ok_or_else(|| anyhow::anyhow!("Supply a local audio file after --compare-stems"))
            .and_then(|path| stems::compare_track(path));
        if let Err(error) = result {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--stems-device") {
        println!("STEMS backend: {}", stems::backend_label());
        return Ok(());
    }
    let deck_options = DeckOptions::from_args(&args[1..]);
    let screenshot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1).cloned());
    let shot_mode = screenshot.is_some();
    let mut viewport = egui::ViewportBuilder::default().with_title("recordbox").with_inner_size([1512.0, 921.0]).with_min_inner_size(MAIN_MIN_SIZE);
    // The recordbox mark as the window and Dock icon.
    if let Some(icon) = brand::mark_icon(512) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
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
    let t0 = std::time::Instant::now();
    eframe::run_native("recordbox", options, Box::new(move |cc| {
        let app = App::new(cc, screenshot, deck_options);
        if bench_on() {
            eprintln!("bench: ready to draw {:.0} ms after launch", t0.elapsed().as_secs_f32() * 1000.0);
        }
        Ok(Box::new(Gui { app, pending_docked_size: None }))
    }))
}
