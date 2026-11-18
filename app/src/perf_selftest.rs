//! PREPDECK_SHOT_PLAYER=perftest (with --screenshot, PREPDECK_SHOT_VIEW=performance and
//! PREPDECK_BENCH=<frames> to keep the window up): operates the Performance screen with real
//! pointer events, one control after another, and prints what each did. A stand-in for a hand
//! test where the screen cannot be taken over. The positions are those of `ui_perf.rs` in the
//! 1512 pt screenshot window.

use crate::app::*;
use eframe::egui;
use egui::pos2;

impl App {
    pub fn shot_perf_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        if !std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "perftest")).unwrap_or(false) {
            return;
        }
        let t = 28.0;
        let extra = if self.prefs.show_stems { 21.0 } else { 0.0 } + if self.prefs.show_phrase_bar { crate::ui_perf::PHRASE_ROW } else { 0.0 };
        let ly = t + 143.0 + 99.5 + extra;
        let o = pos2(490.0, ly - 3.0);
        let my = t + 143.0;
        let by = t + 368.0 + extra + 1.5;
        let d2 = 812.0;
        // (name, start, end of a drag or the same point for a click)
        let steps: Vec<(&str, egui::Pos2, egui::Pos2)> = vec![
            // (PREPDECK_BENCH has the deck playing from frame 60, so the loop and SLIP are tried
            // on a playing deck; the last step pauses it.)
            ("SLIP", o + egui::vec2(185.0, 19.0), o + egui::vec2(185.0, 19.0)),
            ("loop size (start loop)", pos2(446.0, ly + 22.0), pos2(446.0, ly + 22.0)),
            ("loop size (end loop)", pos2(446.0, ly + 22.0), pos2(446.0, ly + 22.0)),
            ("hot cue pad B", pos2(128.0, ly + 38.7), pos2(128.0, ly + 38.7)),
            ("tempo range", o + egui::vec2(121.0, 77.0), o + egui::vec2(121.0, 77.0)),
            ("Q", o + egui::vec2(185.0, 47.0), o + egui::vec2(185.0, 47.0)),
            ("MT", o + egui::vec2(185.0, 111.0), o + egui::vec2(185.0, 111.0)),
            ("key shift up", pos2(700.0 - 97.0, my + 38.0), pos2(700.0 - 97.0, my + 38.0)),
            ("channel fader 1 down", pos2(729.5, my + 169.5), pos2(729.5, my + 209.5)),
            ("HIGH knob 1 to the left", pos2(729.5, my + 51.0), pos2(729.5, my + 51.0 + 80.0)),
            ("crossfader to the right", pos2(756.0, by + 21.5), pos2(801.0, by + 21.5)),
            ("assign deck 1 off side A", pos2(629.0 + 15.0, by + 15.0), pos2(629.0 + 15.0, by + 15.0)),
            ("STEM (STEM ISO) 1", pos2(729.5, my + 234.5), pos2(729.5, my + 234.5)),
            ("MASTER deck 2", pos2(1512.0 - 79.0 + 35.0, my + 39.5), pos2(1512.0 - 79.0 + 35.0, my + 39.5)),
            ("BEAT SYNC deck 1", pos2(700.0 - 79.0 + 22.0, my + 18.5), pos2(700.0 - 79.0 + 22.0, my + 18.5)),
            ("PLAY/PAUSE deck 1 (pause)", o + egui::vec2(20.0, 102.0), o + egui::vec2(20.0, 102.0)),
        ];
        let _ = d2;
        const START: u32 = 140;
        const SPAN: u32 = 40;
        if frame < START {
            return;
        }
        let i = ((frame - START) / SPAN) as usize;
        let f = (frame - START) % SPAN;
        let Some((name, a, b)) = steps.get(i).copied() else { return };
        let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        match f {
            0 => raw.events.push(egui::Event::PointerMoved(a)),
            3 => raw.events.push(button(a, true)),
            4..=11 => {
                if a != b {
                    let k = (f - 3) as f32 / 8.0;
                    raw.events.push(egui::Event::PointerMoved(a + (b - a) * k));
                }
            }
            12 => raw.events.push(button(b, false)),
            // The loop is left running for a while before its state is read.
            30 => eprintln!("perftest: {name}: {}", self.perf_state()),
            _ => {}
        }
    }

    fn perf_state(&self) -> String {
        let act = self.px.active as usize;
        let (d1, s1) = if act == 0 { (&self.deck, &self.player.state) } else { (&self.px.other, &self.player.other) };
        let s = s1.lock();
        let secs = s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0);
        let hot: Vec<i32> = d1.cues.iter().filter(|c| c.kind == crate::model::CueKind::Hot).map(|c| c.slot).collect();
        let m = self.px.mix[0];
        let q1 = if act == 0 { self.prefs.quantize } else { self.px.other_quantize };
        format!(
            "playing {} pos {:.2}s loop {:?} slip {} bg {:?} hot {:?} range step {} Q {} MT {} key {:+} | fader {:.2} (gain {:.3}) high {:.2} (pos {:.2}) xfade {:.2} assign {} xf-gain {:.2} stem-iso {} | master deck {} beat-sync {} tempo {:.4}",
            s.playing,
            secs,
            s.loop_range.map(|l| ((l.0 * 100.0).round() / 100.0, (l.1 * 100.0).round() / 100.0)),
            s.slip,
            s.slip_pos.map(|p| (p / s.audio.as_ref().map(|a| a.sr as f64).unwrap_or(1.0) * 100.0).round() / 100.0),
            hot,
            d1.x.tempo_range,
            q1,
            d1.x.master_tempo,
            d1.x.key_shift,
            m.fader,
            s.ch_fader,
            m.eq[2],
            s.eq_pos[2],
            self.px.crossfader,
            m.assign,
            s.fader,
            m.stem_iso_on,
            self.px.master + 1,
            d1.x.beat_sync,
            s.tempo
        )
    }
}
