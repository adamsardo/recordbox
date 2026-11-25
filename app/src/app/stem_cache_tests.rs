use super::{App, StemMsg, Track};
use crate::stems::{self, Stems};
use anyhow::{ensure, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const MODEL: &str = "htdemucs_ft";
const FRAMES: usize = 4_410;

// The cache and library both use PREPDECK_DATA. Set it only in a child
// process so parallel tests and the developer's library are unaffected.
fn isolated(test: &str, run: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    const CHILD: &str = "RECORDBOX_STEM_CACHE_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return run(&PathBuf::from(std::env::var_os("PREPDECK_DATA").unwrap()));
    }
    let profile = tempfile::tempdir()?;
    let module = module_path!().split_once("::").unwrap().1;
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", &format!("{module}::{test}"), "--nocapture"])
        .env(CHILD, test)
        .env("PREPDECK_DATA", profile.path())
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn open_app() -> App {
    let options = crate::DeckOptions {
        disable_play: true,
        disable_load: false,
    };
    let mut app = App::open_library(&Default::default(), None, options);
    app.stem.model = Some(MODEL.into());
    app.prefs.stems_auto_separate = false;
    app.prefs.vocal_analysis = false;
    app
}

fn track(app: &App, dir: &Path, name: &str) -> Result<(i64, PathBuf)> {
    let path = dir.join(name);
    crate::demo::write_wav(&path, 44_100, &[0.1; 2 * FRAMES])?;
    let t = Track {
        path: path.to_str().unwrap().into(),
        analyzed: 1,
        ..Default::default()
    };
    Ok((app.db.insert_track(&t)?, stems::cache_path(&t.path, MODEL)))
}

fn save_parts(path: &Path, sample: f32) -> Result<()> {
    stems::save(
        path,
        &Stems {
            len: FRAMES,
            data: vec![sample; 8 * FRAMES],
        },
    )
}

fn finish_loading(app: &mut App) {
    let start = Instant::now();
    loop {
        app.poll_load();
        app.poll_stems();
        assert!(
            app.stem.separating.is_none(),
            "cache restoration started inference"
        );
        if app.deck.loading.is_none() && !app.loading_stems() {
            assert!(app.deck.audio.is_some(), "the generated WAV did not load");
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "deck/cache load timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn assert_parts(app: &App, sample: f32) {
    let player = app.player.state.lock();
    let parts = player
        .stems
        .as_ref()
        .expect("the deck should have its cached parts");
    assert_eq!(parts.len, FRAMES);
    assert!(parts
        .data
        .iter()
        .all(|&v| (v - sample).abs() < 1.0 / 32767.0));
}

#[test]
fn deck_reuses_parts_after_switching_tracks_and_reopening_the_library() -> Result<()> {
    isolated(
        "deck_reuses_parts_after_switching_tracks_and_reopening_the_library",
        |dir| {
            let mut app = open_app();
            let (a, cache_a) = track(&app, dir, "A.wav")?;
            let (b, cache_b) = track(&app, dir, "B.wav")?;
            save_parts(&cache_a, 0.25)?;
            save_parts(&cache_b, -0.5)?;
            app.reload_tracks();

            for (id, sample) in [(a, 0.25), (b, -0.5)] {
                app.load_track(id);
                finish_loading(&mut app);
                assert_parts(&app, sample);
            }
            app.load_track(a);
            assert!(app.deck.audio.is_none());
            // No models installed. Hold downloads so a regression can't contact the network.
            app.stem.downloading = Some((0, 1));
            app.separate_loaded();
            assert!(
                app.stem.pending.is_none(),
                "manual request queued inference for a cached track"
            );
            finish_loading(&mut app);
            assert_parts(&app, 0.25);
            drop(app);

            let mut reopened = open_app();
            reopened.reload_tracks();
            reopened.load_track(a);
            finish_loading(&mut reopened);
            assert_parts(&reopened, 0.25);
            Ok(())
        },
    )
}

#[test]
fn late_speed_result_preserves_the_selected_quality_parts() -> Result<()> {
    isolated(
        "late_speed_result_preserves_the_selected_quality_parts",
        |dir| {
            let mut app = open_app();
            let (id, quality_path) = track(&app, dir, "two-models.wav")?;
            app.reload_tracks();
            let speed = stems::spec("htdemucs").unwrap();
            let speed_path = stems::cache_path(&app.track(id).unwrap().path, speed.id);
            save_parts(&quality_path, 0.5)?;
            save_parts(&speed_path, -0.25)?;
            app.select_stem_model(speed);
            app.load_track(id);
            app.stem.downloading = Some((0, 1));
            app.separate_loaded();

            // Deliver the real speed read last, independently of OS thread scheduling.
            let delayed = app.stem.rx.recv_timeout(Duration::from_secs(10))?;
            assert!(matches!(&delayed, StemMsg::Cached(key, Ok(_)) if key.model == speed.id));
            app.select_stem_model(stems::spec(MODEL).unwrap());
            finish_loading(&mut app);
            assert_parts(&app, 0.5);
            app.stem.tx.send(delayed).unwrap();
            app.poll_stems();
            assert_parts(&app, 0.5);

            app.select_stem_model(speed);
            finish_loading(&mut app);
            assert_parts(&app, -0.25);
            Ok(())
        },
    )
}

#[test]
fn damaged_cache_allows_an_explicit_separation_retry() -> Result<()> {
    isolated("damaged_cache_allows_an_explicit_separation_retry", |dir| {
        let mut app = open_app();
        let (id, cache) = track(&app, dir, "damaged.wav")?;
        std::fs::write(cache, b"PDSTEMS1")?;
        app.reload_tracks();
        app.load_track(id);
        finish_loading(&mut app);
        assert!(app.stem.error_for(Some(id)).is_some());
        assert!(app.player.state.lock().stems.is_none());

        // An explicit retry may wait for weights, but must get past the damaged entry.
        app.stem.downloading = Some((0, 1));
        app.separate_loaded();
        finish_loading(&mut app);
        assert_eq!(
            app.stem.pending.as_ref().map(|(key, _)| key.track),
            Some(id)
        );
        Ok(())
    })
}
