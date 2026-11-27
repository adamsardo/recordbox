//! Recordbox: local DJ library, track analysis, set preparation and Pioneer-compatible USB export.

use eframe::egui;
mod analysis;
mod app;
mod app_hooks;
mod audio;
mod automix;
mod backup;
mod brand;
mod cfx;
mod cfx_space;
mod browser_ext;
mod db;
mod cue_tools;
mod deck;
mod demo;
mod dupes;
mod device_ext;
mod edit_audio;
mod edit_model;
mod edit_project;
mod edit_render;
mod edit_selftest;
mod export;
mod fx;
mod fx_core;
mod fx_lfo;
mod fx_roll;
mod fx_sweep;
mod fx_rev;
mod fx_echo;
mod mfx;
mod icons;
mod icons_player;
mod icons_src;
mod importer;
mod itunes;
mod jobs;
mod keycmd_data;
mod keycmd;
mod keytest_selftest;
mod key;
mod library_ext;
mod menus;
mod midi;
mod midi_csv;
mod midi_map;
mod midi_cover;
mod midi_acts;
mod mix_selftest;
mod mixer;
mod groove;
mod ui_groove;
mod pad_audio;
mod pads;
mod pads_ext;
mod pads_selftest;
mod mpl;
mod perf_selftest;
mod sampler;
mod sampler_ext;
mod sampler_lib;
mod sequencer;
mod ui_sampler;
mod model;
mod manual;
mod phrase;
mod phrase_rb;
mod phrase_rb_b;
mod phrase_rb_c;
mod phrase_rb_d;
mod phrase_rb_d1;
mod phrase_rb_d2;
mod phrase_rb_d3;
mod phrase_rb_d4;
mod phrase_rb_d5;
mod phrase_rb_d6;
mod phrase_rb_d7;
mod phrase_rb_d8;
mod phrase_rb_d9;
mod phrase_rb_d10;
mod phrase_rb_d11;
mod phrase_rb_d12;
mod phrase_rb_d13;
mod phrase_rb_d14;
mod phrase_rb_d15;
#[cfg(test)]
mod phrase_rb_var;
mod phrase_rb_dsp;
mod playlist_files;
mod preview;
mod player_ext;
mod prefs_bridge;
mod prefs_ext;
mod prefs_font;
mod prefs_rb;
mod prefs_selftest;
mod prefs_shell;
mod rbimport;
mod recorder;
mod stems;
mod smart;
mod split;
mod playlist_art;
mod theme;
mod traffic;
mod ui_automix;
mod ui_browser;
mod ui_browser_ext;
mod ui_dialogs;
mod ui_edit;
mod ui_edit_view;
mod ui_kit;
mod ui_library_ext;
mod ui_midi;
mod ui_mpl;
mod ui_sampler_lib;
mod ui_player;
mod ui_prefs;
mod ui_rec;
mod ui_prefs_adv;
mod ui_prefs_item;
mod ui_prefs_keys;
mod ui_prefs_perf;
mod ui_prefs_view;
mod ui_fx;
mod ui_pads;
mod ui_padfx;
mod ui_perf;
mod ui_perf_layouts;
mod ui_player2;
mod xmlexport;
mod vocal_rb;
mod vocal_rb_mp3;
mod vocal_rb_net;

use app::App;

struct Gui {
    app: App,
}

impl eframe::App for Gui {
    /// PREPDECK_SHOT_DRAG (with --screenshot): drag hot cue A 100 px to the right with real
    /// pointer events, to check marker dragging end to end; the result goes to stderr.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Edit menu commands (and their shortcuts, which the menu bar takes before the view sees
        // the keys) become the input events egui's text fields and the track list act on.
        for c in std::mem::take(&mut self.app.edit_cmds) {
            match c {
                0 => raw.events.push(egui::Event::Cut),
                1 => raw.events.push(egui::Event::Copy),
                2 => {
                    if let Some(t) = arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok()) {
                        raw.events.push(egui::Event::Paste(t));
                    }
                }
                _ => {
                    let m = egui::Modifiers { command: true, mac_cmd: true, ..Default::default() };
                    raw.events.push(egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: m });
                    raw.events.push(egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: false, repeat: false, modifiers: m });
                }
            }
        }
        ui_groove::frame_hook(&mut self.app, raw);
        app_hooks::raw_input(&mut self.app, raw);
        let Some((_, frame)) = self.app.screenshot.clone() else { return };
        self.app.shot_player_events(frame, raw);
        self.app.shot_perf_events(frame, raw);
        self.app.shot_midi_events(frame, raw);
        self.app.shot_rec_events(frame, raw);
        self.app.shot_edit_events(frame, raw);
        self.app.shot_pads_events(frame, raw);
        crate::ui_perf_layouts::shot_layout_events(&mut self.app, frame, raw);
        self.app.shot_mix_events(frame, raw);
        self.app.shot_prefs_events(frame, raw);
        self.app.shot_key_events(frame, raw);
        crate::dupes::shot_events(&mut self.app, frame, raw);
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
        prefs_bridge::frame_limit(&mut self.app, ctx);
        let bench_t0 = std::time::Instant::now();
        let app = &mut self.app;
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
        step!(midi::poll(app));
        step!(prefs_bridge::frame(app));
        step!(app.poll_decks());
        step!(automix::poll(app, ctx));
        step!(app.poll_export());
        step!(app.poll_rekordbox_sync());
        step!(app.poll_stems());
        step!(app.poll_stem_waves());
        step!(app.poll_phrases());
        step!(app.poll_vocal());
        step!(app.poll_key_job());
        step!(app.poll_audio_device());
        step!(app.poll_relocate());
        step!(app.poll_rec());
        step!(backup::poll(app));
        step!(menus::poll(app));
        step!(menus::sync(app));
        shortcuts(app, ctx);
        // Preferences > View > Tooltips off: no hover text anywhere, including the ones drawn
        // with egui's own `on_hover_text` (a delay that never runs out; infinity would panic
        // in egui's repaint timer).
        let tip_delay = if app.prefs.tooltips { 0.5 } else { 1.0e9 };
        if ctx.style().interaction.tooltip_delay != tip_delay {
            ctx.all_styles_mut(|s| s.interaction.tooltip_delay = tip_delay);
        }
        // Full screen (View menu, keyboard): the window follows; the menu tick follows the window.
        let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if std::mem::take(&mut app.dialogs.ext.toggle_fullscreen) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
        }
        app.dialogs.ext.fullscreen = fs;

        egui::TopBottomPanel::top("header").exact_height(28.0).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG1)).show(ctx, |ui| {
            step!(ui_browser::header_bar(app, ui));
        });
        egui::TopBottomPanel::bottom("footer").exact_height(28.0).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
            ui_browser::footer(app, ui);
        });
        // EXPORT mode's recording panel, a strip under the top bar.
        if !app.performance() && app.prefs.rec.panel_export {
            egui::TopBottomPanel::top("recstrip").exact_height(ui_rec::EXPORT_H).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG2)).show(ctx, |ui| ui_rec::export_strip(app, ui));
        }
        if ui_edit::is_edit(app) { step!(ui_edit_view::top_panel(app, ctx)); } else
        if !app.prefs.browser_only || app.performance() {
            // The header is 28 pt and 1 Player / 2 Players 256 pt tall with no rule between them
            // (measured on rekordbox: the player ends 284 pt below the window top); the Simple
            // Player is one 58 pt row.
            let h = app.player_height();
            let perf = app.performance();
            egui::TopBottomPanel::top(egui::Id::new(("player", if perf { 9 } else { app.prefs.player_layout.min(3) }))).exact_height(h).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| match app.prefs.player_layout {
                _ if perf => step!(ui_perf::performance(app, ui)),
                1 | 3 => step!(ui_player2::two_players(app, ui)),
                2 => step!(ui_player2::simple_player(app, ui)),
                _ => step!(ui_player::player_panel(app, ui)),
            });
        }
        // Split Screen: the browser is in a window of its own (split.rs).
        let split = split::frame(app, ctx);
        egui::CentralPanel::default().frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
            if ui_edit_view::hides_browser(app) { return; }
            if !split {
                step!(ui_browser::browser(app, ui));
            }
        });
        step!(ui_dialogs::dialogs(app, ctx));
        // Preferences rows of other files' features take effect (prefs_bridge.rs).
        if app.dialogs.prefs_open || app.ctx.cumulative_pass_nr() < 3 {
            app.prefs_bridge();
        }
        step!(dupes::windows(app, ctx));
        step!(cue_tools::windows(app, ctx));
        step!(ui_midi::window(app, ctx));
        step!(ui_rec::windows(app, ctx));

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
                    eprintln!("bench: stems ready {}, separating {:?}, stem error {:?}", app.player.state.lock().stems.is_some(), app.stem.separating, app.stem.error);
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
        prefs_bridge::on_exit(&mut self.app);
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

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    keycmd::frame(app, ctx);
    if ui_edit::frame(app, ctx) { return; }
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
        if app.searching() != 0 && !ctx.input(|i| i.pointer.any_down()) && !app.midi.search_held[app.px.active.min(3) as usize] {
            app.search(0);
        }
    }
    if app.deck.hot_preview.is_some() {
        let hot_held = (1..=8).any(|n| app.prefs.key_for(&format!("hot{n}")).map(|k| ctx.input(|i| i.key_down(k))).unwrap_or(false));
        if !hot_held && !ctx.input(|i| i.pointer.any_down()) && !keycmd::holding(app.px.active) {
            app.hot_cue_up();
        }
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
    let shortcuts: std::collections::HashMap<&str, app::Shortcut> = recordbox_actions().iter().filter(|a| keycmd::old_loop_runs(&app.prefs, a)).filter_map(|a| app.prefs.shortcut_for(a).map(|k| (*a, k))).collect();
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


fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if midi::cli(&args) {
        return Ok(());
    }
    let screenshot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1).cloned());
    let shot_mode = screenshot.is_some();
    let mut viewport = egui::ViewportBuilder::default().with_title("recordbox").with_inner_size([1512.0, 921.0]).with_min_inner_size([1100.0, 700.0]);
    // The recordbox mark as the window and Dock icon.
    if let Some(icon) = brand::app_icon(brand::system_dark()) {
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
        let app = App::new(cc, screenshot);
        if bench_on() {
            eprintln!("bench: ready to draw {:.0} ms after launch", t0.elapsed().as_secs_f32() * 1000.0);
        }
        Ok(Box::new(Gui { app }))
    }))
}
