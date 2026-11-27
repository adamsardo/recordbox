//! `PREPDECK_SHOT_PLAYER=stemexporttest` (with --screenshot and PREPDECK_BENCH=20000 to keep
//! the window up; scratch library only): works the stem export end to end with real pointer
//! events and prints what happened to stderr.
//!
//! 1. A normal playlist is opened and one of its tracks loaded (PREPDECK_STEMX_KIND=mp3|flac|
//!    wav picks the source's format; default mp3), the STEMS strip shown, nothing separated.
//! 2. EXPORT > Instrumental in the strip: the explanation comes up; OK. The export separates
//!    the track on the deck, writes the file and adds it; the file, the copied grid and cues
//!    and its place in the playlist are printed.
//! 3. EXPORT > Vocals with the Collection as the view: not added to a playlist.
//! 4. The playlist setting off, EXPORT > Instrumental in the playlist again: numbered file, not
//!    in the playlist.
//! 5. Instrumentals of a FLAC and a WAV track that are not on the deck, from the track menu's
//!    command (separated in the background, one after the other, progress in the footer).
//! Pictures (scratch/shots, PREPDECK_STEMX_SHOTS=<folder>): the strip, the explanation, the
//! footer during the batch, then the strip and the explanation in Light.

use crate::app::*;
use crate::model::*;
use crate::stem_export::{self as sx, Part};
use eframe::egui;

fn on() -> bool {
    std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "stemexporttest")).unwrap_or(false)
}

#[derive(Default)]
struct Run {
    step: usize,
    wait: u32,
    pl: i64,
    src: i64,
    started: Option<std::time::Instant>,
    extra: Vec<i64>,
}

static RUN: parking_lot::Mutex<Option<Run>> = parking_lot::Mutex::new(None);

fn shots_dir() -> std::path::PathBuf {
    std::env::var("PREPDECK_STEMX_SHOTS").map(std::path::PathBuf::from).unwrap_or_else(|_| crate::db::data_dir().join("stemx-shots"))
}

fn snap(app: &App, name: &str) {
    let p = shots_dir().join(format!("{name}.png"));
    app.ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(p.to_string_lossy().to_string())));
}

fn click(raw: &mut egui::RawInput, p: egui::Pos2, phase: u32) {
    match phase {
        0 => raw.events.push(egui::Event::PointerMoved(p)),
        1 => raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }),
        _ => raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }),
    }
}

fn ext_of(t: &Track) -> String {
    std::path::Path::new(&t.path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn report(app: &mut App, o: &sx::Outcome, label: &str) {
    let (id, w) = match &o.result {
        Ok(x) => x,
        Err(e) => return eprintln!("stemexporttest: {label}: FAILED: {e}"),
    };
    let src = app.track(o.req.track).cloned();
    let new = app.track(*id).cloned();
    let size = std::fs::metadata(&w.path).map(|m| m.len()).unwrap_or(0);
    let src_size = src.as_ref().and_then(|s| std::fs::metadata(&s.path).ok()).map(|m| m.len()).unwrap_or(0);
    let info = sx::probe_source(&w.path.to_string_lossy(), new.as_ref().map(|t| t.bitrate).unwrap_or(0));
    let src_info = src.as_ref().and_then(|s| sx::probe_source(&s.path, s.bitrate));
    eprintln!("stemexporttest: {label}: file {}", w.path.display());
    eprintln!("stemexporttest: {label}: written as {} ({}), {} Hz, {} ch, read back {} frames ({:.3} s) against the source's {} frames at {} Hz, peak {:.4} ({:.2} dBFS), size {} bytes vs source {} bytes ({:.2}x)", w.plan.describe(), w.plan.fallback.clone().unwrap_or_else(|| "same format family as the source".into()), w.plan.rate, w.plan.ch, w.frames, w.frames as f64 / w.plan.rate as f64, w.src_frames, w.src_rate, w.peak, 20.0 * w.peak.max(1e-9).log10(), size, src_size, size as f64 / src_size.max(1) as f64);
    eprintln!("stemexporttest: {label}: source {:?}, export probed {:?}", src_info, info);
    eprintln!("stemexporttest: {label}: measured timeline offset {:?} frames, leading frames dropped {}, copied timeline moved {:.6} s", w.offset, w.dropped, w.shift);
    if let (Some(s), Some(n)) = (src, new) {
        let (gs, gn) = (app.db.grid(s.id), app.db.grid(n.id));
        let (cs, cn) = (app.db.cues(s.id).unwrap_or_default(), app.db.cues(n.id).unwrap_or_default());
        let same_cues = cs.len() == cn.len() && cs.iter().zip(&cn).all(|(a, b)| a.kind == b.kind && a.slot == b.slot && (a.start + w.shift - b.start).abs() < 1e-9);
        eprintln!(
            "stemexporttest: {label}: in collection id {} \"{}\" by \"{}\" | bpm {:.2} vs {:.2}, key {} vs {} | grid anchors {} vs {}, first beat {:?} vs {:?} | cues {} vs {} ({}) | link {:?} | attribute set {}",
            n.id,
            n.title,
            n.artist,
            n.bpm,
            s.bpm,
            n.key,
            s.key,
            gn.as_ref().map(|g| g.anchors.len()).unwrap_or(0),
            gs.as_ref().map(|g| g.anchors.len()).unwrap_or(0),
            gn.as_ref().and_then(|g| g.anchors.first()).map(|a| a.time),
            gs.as_ref().and_then(|g| g.anchors.first()).map(|a| a.time),
            cn.len(),
            cs.len(),
            if same_cues { "same positions" } else { "DIFFERENT" },
            app.db.setting(&sx::link_key(n.id)),
            crate::ui_edit::edited_set(app).contains(&n.id),
        );
        let pls = app.db.playlists_containing(n.id).unwrap_or_default();
        let mut place = String::new();
        for pl in &pls {
            let list = app.db.playlist_tracks(*pl).unwrap_or_default();
            let at = list.iter().position(|t| *t == n.id);
            let src_at = list.iter().position(|t| *t == s.id);
            place.push_str(&format!("playlist {pl} at row {:?} (source at row {:?}{}) ", at, src_at, if at.is_some() && src_at.is_some() && at.unwrap() == src_at.unwrap() + 1 { ", directly after it" } else { "" }));
        }
        eprintln!("stemexporttest: {label}: playlists {pls:?} {place}");
    }
}

/// Mid-run pictures arrive as Screenshot events carrying their file name.
fn take_pictures(raw: &mut egui::RawInput) {
    raw.events.retain(|e| {
        if let egui::Event::Screenshot { image, user_data, .. } = e {
            if let Some(path) = user_data.data.as_ref().and_then(|d| d.downcast_ref::<String>()) {
                let [w, h] = image.size;
                let mut buf = image::RgbaImage::new(w as u32, h as u32);
                for (i, px) in image.pixels.iter().enumerate() {
                    let [r, g, b, a] = px.to_srgba_unmultiplied();
                    buf.put_pixel((i % w) as u32, (i / w) as u32, image::Rgba([r, g, b, a]));
                }
                if let Some(p) = std::path::Path::new(path).parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                let _ = buf.save(path);
                eprintln!("stemexporttest: picture {path}");
                return false;
            }
        }
        true
    });
}

pub fn shot_events(app: &mut App, frame: u32, raw: &mut egui::RawInput) {
    if !on() {
        return;
    }
    take_pictures(raw);
    if frame < 40 {
        return;
    }
    let mut g = RUN.lock();
    let r = g.get_or_insert_with(Run::default);
    if r.wait > 0 {
        r.wait -= 1;
        return;
    }
    let rect = |app: &App, k: &str| app.stemx.rects.get(k).map(|r| r.center());
    let step = r.step;
    r.step += 1;
    match step {
        0 => {
            let kind = std::env::var("PREPDECK_STEMX_KIND").unwrap_or_else(|_| "mp3".into());
            let good = |t: &Track| t.analyzed == 1 && !t.missing && t.duration > 60.0 && t.duration < 420.0 && ext_of(t) == kind;
            let mut pick = None;
            for n in app.nodes.iter().filter(|n| n.kind == NodeKind::Playlist) {
                let list = app.db.playlist_tracks(n.id).unwrap_or_default();
                if list.len() < 3 {
                    continue;
                }
                if let Some(t) = list.iter().take(list.len() - 1).find(|t| app.track(**t).is_some_and(good)) {
                    pick = Some((n.id, *t));
                    break;
                }
            }
            let Some((pl, src)) = pick else {
                eprintln!("stemexporttest: no playlist with an analysed {kind} track");
                std::process::exit(2);
            };
            r.pl = pl;
            r.src = src;
            app.prefs.app_mode = 0;
            app.prefs.browser_only = false;
            app.prefs.player_layout = 0;
            app.prefs.show_stems = true;
            app.prefs.stemx = Default::default();
            app.prefs.stems_auto_separate = false;
            // The export folder is inside the throwaway data folder, never the user's Music.
            let dir = std::env::var("PREPDECK_STEMX_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| crate::db::data_dir().join("stem-exports-test"));
            let data = std::env::var("PREPDECK_DATA").map(std::path::PathBuf::from).ok();
            let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(|p| p.join("scratch"));
            if !(data.as_ref().is_some_and(|d| dir.starts_with(d)) || scratch.as_ref().is_some_and(|s| dir.starts_with(s))) {
                eprintln!("stemexporttest: the export folder {} is not inside PREPDECK_DATA or the scratch folder; not running", dir.display());
                std::process::exit(2);
            }
            app.prefs.stemx.dir = dir.to_string_lossy().to_string();
            eprintln!("stemexporttest: export folder {}", sx::export_dir(&app.prefs.stemx).display());
            app.side = SideMode::Playlists;
            app.set_view(View::Playlist(pl));
            app.rebuild_list();
            app.load_track(src);
            app.shot_wait_load();
            app.deck.ctrl_tab = 2;
            if app.playing() {
                app.play_pause();
            }
            let t = app.track(src).cloned().unwrap();
            eprintln!("stemexporttest: playlist {pl} \"{}\", source {} \"{} - {}\" ({}, {:.1} s), stems on deck {}", app.node(pl).map(|n| n.name.clone()).unwrap_or_default(), src, t.artist, t.title, ext_of(&t), t.duration, app.player.state.lock().stems.is_some());
            // FLAC and WAV tracks for the batch (not on the deck).
            for k in ["flac", "wav"] {
                if let Some(t) = app.tracks.iter().find(|t| t.analyzed == 1 && !t.missing && t.duration > 60.0 && t.duration < 420.0 && ext_of(t) == k && t.id != src) {
                    r.extra.push(t.id);
                }
            }
            r.wait = 30;
        }
        1 => {
            snap(app, "strip-dark");
            r.wait = 10;
        }
        2..=4 => {
            if let Some(p) = rect(app, "strip_export") {
                click(raw, p, (step - 2) as u32);
            }
        }
        5..=7 => match rect(app, "strip_instrumental") {
            Some(p) => click(raw, p, (step - 5) as u32),
            None => eprintln!("stemexporttest: the EXPORT menu did not open"),
        },
        8 => {
            r.wait = 10;
        }
        9 => {
            match &app.stemx.ask {
                Some(a) => eprintln!("stemexporttest: explanation shown: playlist {:?}, add ticked {}, don't show ticked {}, exports waiting {}", a.playlist, a.add, a.dont_show, a.reqs.len()),
                None => eprintln!("stemexporttest: NO explanation shown"),
            }
            snap(app, "popup-dark");
            r.wait = 10;
        }
        10..=12 => {
            if let Some(p) = rect(app, "ok") {
                click(raw, p, (step - 10) as u32);
            }
        }
        13 => {
            eprintln!("stemexporttest: after OK: explanation {} | busy {} | separating {:?} | prefs to_playlist {} explain {}", app.stemx.ask.is_some(), sx::busy(app), app.stem.separating, app.prefs.stemx.to_playlist, app.prefs.stemx.explain);
            r.started = Some(std::time::Instant::now());
            r.wait = 45;
        }
        14 => {
            // The strip while it runs.
            snap(app, "strip-exporting-dark");
            eprintln!("stemexporttest: strip status: {:?}", crate::ui_stem_export::strip_status(app));
        }
        15 => {
            if app.stemx.outcomes.is_empty() {
                r.step = 15;
                r.wait = 30;
                if r.started.is_some_and(|t| t.elapsed().as_secs() > 240) {
                    eprintln!("stemexporttest: timed out: {:?}", sx::status(app));
                    std::process::exit(3);
                }
                return;
            }
            eprintln!("stemexporttest: first export took {:.1} s", r.started.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0));
            let o = app.stemx.outcomes[0].clone();
            drop(g);
            report(app, &o, "1 Instrumental from the playlist");
            let mut g = RUN.lock();
            let r = g.as_mut().unwrap();
            // 3: Vocals from the Collection view.
            app.side = SideMode::Collection;
            app.set_view(View::Collection);
            app.rebuild_list();
            r.wait = 10;
        }
        16..=18 => {
            if let Some(p) = rect(app, "strip_export") {
                click(raw, p, (step - 16) as u32);
            }
        }
        19..=21 => {
            if let Some(p) = rect(app, "strip_vocals") {
                click(raw, p, (step - 19) as u32);
            }
        }
        22 => {
            eprintln!("stemexporttest: Vocals from the Collection: explanation {} | busy {}", app.stemx.ask.is_some(), sx::busy(app));
            r.started = Some(std::time::Instant::now());
        }
        23 => {
            if app.stemx.outcomes.len() < 2 {
                r.step = 23;
                r.wait = 30;
                return;
            }
            eprintln!("stemexporttest: second export took {:.1} s", r.started.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0));
            let o = app.stemx.outcomes[1].clone();
            let pl = r.pl;
            drop(g);
            report(app, &o, "2 Vocals from the Collection");
            let mut g = RUN.lock();
            let r = g.as_mut().unwrap();
            // 4: the setting off, from the playlist again.
            app.prefs.stemx.to_playlist = false;
            app.side = SideMode::Playlists;
            app.set_view(View::Playlist(pl));
            app.rebuild_list();
            r.wait = 10;
        }
        24..=26 => {
            if let Some(p) = rect(app, "strip_export") {
                click(raw, p, (step - 24) as u32);
            }
        }
        27..=29 => {
            if let Some(p) = rect(app, "strip_instrumental") {
                click(raw, p, (step - 27) as u32);
            }
        }
        30 => {
            if app.stemx.outcomes.len() < 3 {
                r.step = 30;
                r.wait = 30;
                return;
            }
            let o = app.stemx.outcomes[2].clone();
            let extra = r.extra.clone();
            drop(g);
            report(app, &o, "3 Instrumental from the playlist, setting off");
            // 5: FLAC and WAV tracks off the deck, as the track menu does it.
            eprintln!("stemexporttest: batch of {} tracks off the deck: {:?}", extra.len(), extra.iter().map(|t| app.track(*t).map(|t| format!("{} ({})", t.title, ext_of(t))).unwrap_or_default()).collect::<Vec<_>>());
            app.prefs.stemx.to_playlist = true;
            sx::request(app, &extra, Part::Instrumental, false);
            let mut g = RUN.lock();
            let r = g.as_mut().unwrap();
            r.started = Some(std::time::Instant::now());
            r.wait = 90;
        }
        31 => {
            eprintln!("stemexporttest: footer: {:?}", sx::footer_text(app));
            snap(app, "footer-batch-dark");
        }
        32 => {
            let want = 3 + r.extra.len();
            if app.stemx.outcomes.len() < want {
                r.step = 32;
                r.wait = 30;
                if r.started.is_some_and(|t| t.elapsed().as_secs() > 400) {
                    eprintln!("stemexporttest: batch timed out: {:?}", sx::footer_text(app));
                    std::process::exit(3);
                }
                return;
            }
            eprintln!("stemexporttest: batch took {:.1} s", r.started.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0));
            let outs: Vec<sx::Outcome> = app.stemx.outcomes[3..].to_vec();
            drop(g);
            for (i, o) in outs.iter().enumerate() {
                report(app, o, &format!("{} batch", 4 + i));
            }
            // Light: the strip and the explanation.
            app.prefs.rb.color_theme = 2;
            let pl = RUN.lock().as_ref().unwrap().pl;
            app.stemx.ask = Some(sx::Ask { reqs: vec![], playlist: Some((pl, app.node(pl).map(|n| n.name.clone()).unwrap_or_default())), add: true, dont_show: true });
            RUN.lock().as_mut().unwrap().wait = 20;
        }
        33 => {
            snap(app, "popup-light");
            r.wait = 10;
        }
        34 => {
            app.stemx.ask = None;
            r.wait = 10;
        }
        35 => {
            snap(app, "strip-light");
            r.wait = 10;
        }
        36 => {
            use std::sync::atomic::Ordering::Relaxed;
            let left: Vec<String> = std::fs::read_dir(sx::export_dir(&app.prefs.stemx)).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
            eprintln!("stemexporttest: export folder {}: {left:?}", sx::export_dir(&app.prefs.stemx).display());
            eprintln!("stemexporttest: audio callback gave a buffer up {} times (found a deck held {} times)", crate::audio::AUDIO_DROPPED.load(Relaxed), crate::audio::AUDIO_CONTENDED.load(Relaxed));
            eprintln!("stemexporttest: done");
            app.prefs.rb.color_theme = 0;
            std::process::exit(0);
        }
        _ => {}
    }
}
