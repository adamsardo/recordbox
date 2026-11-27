//! Preferences rows that belong to features in other files: the Preferences pages keep every
//! rekordbox setting in `prefs.rb` (prefs_rb.rs), and the deck engine, the pads, Groove
//! Circuit and Traffic Light keep the ones they act on in their own settings. This copies the
//! values across so that a row changed in Preferences takes effect.

use crate::app::*;

impl App {
    /// Called at start-up and on every frame the Preferences window is open.
    pub fn prefs_bridge(&mut self) {
        let p = &mut self.prefs;
        let rb = &p.rb;
        // ---- the deck engine and the mixer (deck.rs)
        let d = &mut p.deck;
        d.needle_lock = rb.needle_lock;
        d.vinyl_brake = rb.brake;
        d.vinyl_start = rb.start;
        d.jog_45 = rb.jog_speed == 2;
        d.beat_sync = rb.beat_sync_type != 2;
        d.multi_bpm_sync = rb.multi_bpm_sync;
        d.show_sync_rate = rb.show_sync_rate;
        d.quantize_one_go = rb.quantize_in_one_go;
        d.crossfader_curve = rb.crossfader_curve.min(32);
        d.headroom = rb.headroom.clamp(1, 8);
        d.eq_nxs = rb.eq_type == 0;
        d.hot_cue_countdown = rb.jog_indicator == 2;
        // ---- pads (pads.rs): PadModeLayoutType 1 Auto, 2 LIST Display, 3 PAD Display
        p.pads.layout = if rb.pad_mode_custom { rb.pad_mode_layout.clamp(1, 2) + 1 } else { 1 };
        p.pads.hot_cue_color = rb.hot_cue_color.clamp(1, 4);
        // ---- Groove Circuit (groove.rs)
        let g = &mut p.groove;
        g.enable = rb.groove_circuit;
        g.load_lock = rb.drum_swap_lock;
        g.show_title = rb.drum_swap_title;
        g.capture_stem = rb.drum_capture_stem;
        g.capture_dir = rb.drum_capture_dir.clone();
        g.auto_import = rb.drum_auto_import;
        // ---- Traffic Light (traffic.rs)
        p.traffic.range = rb.traffic_light.clamp(1, 4);
    }
}
