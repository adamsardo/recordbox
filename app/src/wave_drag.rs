//! Dragging an enlarged waveform with the pointer (the horizontal one in `ui_player.rs`, the
//! vertical one in `ui_perf_layouts.rs`).
//!
//! A paused deck is moved by seeking, exactly under the pointer. A playing deck in PERFORMANCE
//! mode is held like a touched platter and follows the pointer as a scratch (the platter's own
//! path in the engine, `jog_touch` / `jog_speed`): before, the deck kept playing forward while
//! every frame seeked it back under the pointer, which made the waveform jitter and the sound
//! stutter. While the drag lasts the waveform is drawn at the pointer's position
//! (`ui_feel::hold_pos`), not at the audio's, so the picture follows the hand exactly.

use crate::app::App;

/// The button went down on the waveform body.
pub fn start(app: &mut App) {
    if scratching(app) {
        app.deck.x.dk.wave_held = true;
        app.jog_touch(true);
    }
}

fn scratching(app: &App) -> bool {
    app.performance() && app.playing() && app.deck.audio.is_some()
}

/// The pointer is at track time `t` (seconds): move the deck there. `dt`: the frame's length.
/// Returns how far the target moved since the last call (for the waveform link).
pub fn to(app: &mut App, t: f64, dt: f64) -> f64 {
    let t = t.clamp(0.0, app.deck.duration.max(0.0));
    let before = crate::ui_feel::held_pos(app).unwrap_or_else(|| app.pos());
    crate::ui_feel::hold_pos(app, t);
    if app.deck.x.dk.wave_held {
        // The scratch speed: the pointer's own speed over this frame, plus a quarter of what
        // the audio is still behind by (it applies a speed one buffer late; correcting all of
        // it at once makes the speed swing back and forth). A pointer at rest with the audio
        // within 3 ms of it holds the record still.
        let dt = dt.clamp(1.0 / 500.0, 1.0 / 20.0);
        let behind = before - app.pos();
        let v = if t == before && behind.abs() < 0.003 { 0.0 } else { ((t - before) / dt + (0.25 * behind / dt).clamp(-2.0, 2.0)).clamp(-48.0, 48.0) };
        app.jog_speed(v, true);
    } else {
        app.seek(t);
    }
    t - before
}

/// The button was let go.
pub fn end(app: &mut App) {
    let at = crate::ui_feel::held_pos(app);
    crate::ui_feel::release_pos(app);
    if app.deck.x.dk.wave_held {
        app.deck.x.dk.wave_held = false;
        app.jog_touch(false);
        // Land exactly where the pointer left it.
        if let Some(t) = at.filter(|t| (t - app.pos()).abs() > 0.004) {
            app.seek(t);
        }
    }
}

/// How much more of the track an enlarged waveform shows at the deck's tempo: rekordbox scales
/// the waveform by current BPM / original BPM, so two decks playing at the same BPM have their
/// beats the same distance apart and scroll at the same speed, whatever their tracks' own
/// tempos (`djplay::WidgetWaveView::updateBpmWaveScale` 0x102110ebc: the ratio of
/// `DjEngineIF::getLowerBpm(deck, 1)` to `getLowerBpm(deck, 0)`, not below 0.1, and 1.0 for a
/// track without a BPM; passed to `setWaveScaleRatio`). rekordbox applies it 100 ms after a
/// tempo change (`operateWaveViewUpdateBpmScaleWithWait`); here it follows at once.
pub fn tempo_scale(app: &App) -> f64 {
    scale_for(app.current_bpm(), app.deck_speed())
}

fn scale_for(bpm: f64, speed: f64) -> f64 {
    let now = bpm * speed;
    if bpm * now == 0.0 {
        return if bpm > 1.19e-7 { 0.1 } else { 1.0 };
    }
    (now / bpm).max(0.1)
}

#[cfg(test)]
mod tests {
    use super::scale_for;

    #[test]
    fn two_decks_at_the_same_bpm_have_the_same_beat_spacing() {
        // 131 BPM as it is, and 105.39 BPM sped up to 131: the beats are as far apart in
        // pixels (seconds per beat / seconds across the waveform).
        let across = 4.0;
        let a = (60.0 / 131.0) / (across * scale_for(131.0, 1.0));
        let b = (60.0 / 105.39) / (across * scale_for(105.39, 131.0 / 105.39));
        assert!((a - b).abs() < 1e-12, "{a} {b}");
        // rekordbox's limits: never below a tenth, and no scaling without a BPM.
        assert_eq!(scale_for(120.0, 0.0), 0.1);
        assert_eq!(scale_for(120.0, 0.05), 0.1);
        assert_eq!(scale_for(0.0, 1.3), 1.0);
        assert_eq!(scale_for(120.0, 1.0), 1.0);
    }
}
