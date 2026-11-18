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
        let fxh = if self.prefs.perf_fx { crate::ui_fx::FX_H } else { 0.0 };
        let t = 28.0 + fxh;
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
            // The FX panel (shown by PREPDECK_SHOT_VIEW=perffx, 52 pt above the rest): assign FX1
            // to deck 1, switch ECHO on, step its beat down, hold RELEASE FX.
            ("FX1 assign deck 1", pos2(85.0, t - fxh + 17.5), pos2(85.0, t - fxh + 17.5)),
            ("FX1 ECHO on", pos2(193.0 + 111.0 + 40.0, t - fxh + 17.5), pos2(193.0 + 111.0 + 40.0, t - fxh + 17.5)),
            ("FX1 ECHO beat down", pos2(193.0 + 111.0 + 39.0, t - fxh + 36.5), pos2(193.0 + 111.0 + 39.0, t - fxh + 36.5)),
            ("FX1 RELEASE FX held", pos2(576.0, t - fxh + 17.5), pos2(576.0, t - fxh + 18.5)),
            // The CFX row under the Beat FX row: turn deck 1's COLOR knob to the left (drag down).
            ("CFX COLOR knob of deck 1 to the left", pos2(863.0, t - fxh + 65.0), pos2(863.0, t - fxh + 65.0 + 60.0)),
            // Quick clicks: press and release in the same frame (found by hand: sliders missed them).
            ("quick click on channel fader 2, half way", pos2(782.0, my + 194.5), pos2(782.0, my + 194.5)),
            ("quick click on the crossfader, a quarter in", pos2(733.5, by + 21.5), pos2(733.5, by + 21.5)),
            ("quick click on master volume, half way", pos2(1512.0 - 87.5, 14.0), pos2(1512.0 - 87.5, 14.0)),
            ("PLAY/PAUSE deck 1 (pause)", o + egui::vec2(20.0, 102.0), o + egui::vec2(20.0, 102.0)),
        ];
        let _ = d2;
        // The 4Deck Horizontal layout has its own positions (decks 655 pt wide, mixer from 661,
        // decks 2 and 4 from 855; the second row of decks 190 pt further down).
        let four = self.prefs.perf_four;
        // The sampler panel (PREPDECK_SHOT_VIEW=perfsampler, 2-deck layout): its own steps.
        let smp = self.prefs.perf_sampler && !four;
        let steps = if smp {
            let py = by - 1.5 + 43.0;
            let (sx, sy) = (796.0, py + 88.0);
            let s10 = 1512.0 - 4.0 - 3.0 * 179.0;
            let p = |x: f32, y: f32| (pos2(x, y), pos2(x, y));
            let st = |n: &'static str, (a, b): (egui::Pos2, egui::Pos2)| (n, a, b);
            vec![
                st("SMP PLAY slot 1 (one-shot)", p(23.0, py + 34.0)),
                st("SMP Shift-free second PLAY restarts it", p(23.0, py + 34.0)),
                st("SMP PLAY slot 2 (loop)", p(183.0 + 19.0, py + 34.0)),
                st("SMP EDIT slot 1", p(4.0 + 179.0 - 32.0, py + 34.0)),
                st("SMP gate button in EDIT", p(183.0 + 17.0 + 59.0, py + 6.0 + 15.0)),
                st("SMP EDIT slot 1 (close)", p(4.0 + 179.0 - 32.0, py + 34.0)),
                st("SMP PLAY slot 1 held (gate)", p(23.0, py + 34.0)),
                st("SMP Q", p(510.5, py + 104.0)),
                st("SMP BPM +", p(744.0, py + 103.0)),
                st("SMP BPM SYNC", p(570.0, py + 103.0)),
                ("SMP volume fader down", pos2(775.0, py + 22.0), pos2(775.0, py + 42.0)),
                st("SMP BANK 2", p(82.0, py + 104.0)),
                st("SMP BANK 1", p(54.0, py + 104.0)),
                // The sequencer strip (from x 796, the panel's last 30 pt). Slot 1 is a gate
                // sample by now, slot 2 a loop, slot 10 a one-shot; Q is on.
                st("SEQ REC (armed, nothing plays yet)", p(sx + 10.0, sy + 15.0)),
                st("SEQ pad 1 with REC armed (starts the pattern; gate: on and off)", p(23.0, py + 34.0)),
                st("SEQ pad 10 (one-shot)", p(s10 + 19.0, py + 34.0)),
                st("SEQ pad 2 (loop: plays, not recorded)", p(183.0 + 19.0, py + 34.0)),
                st("SEQ REC off (the pattern keeps playing)", p(sx + 10.0, sy + 15.0)),
                st("SEQ metronome on", p(sx + 554.0, sy + 13.0)),
                st("SEQ metronome volume (1 -> 2)", p(sx + 579.0, sy + 13.0)),
                st("SEQ length pull-down", p(sx + 237.0, sy + 15.0)),
                st("SEQ length 2Bars", p(sx + 213.0 + 25.0, sy + 24.0 + 36.0)),
                st("SEQ SAVE", p(sx + 73.0, sy + 15.0)),
                st("SEQ MUTE mode", p(sx + 468.0, sy + 15.0)),
                st("SEQ mute button of slot 10 (shown by the mode)", p(s10 + 42.0, py + 34.0)),
                st("SEQ ERASE mode", p(sx + 511.0, sy + 15.0)),
                st("SEQ erase button of slot 1", p(4.0 + 179.0 - 32.0, py + 34.0)),
                st("SEQ next pattern (waits for the bar)", p(sx + 202.5, sy + 15.0)),
                st("wait 1", p(sx + 300.0, sy - 40.0)),
                st("wait 2", p(sx + 300.0, sy - 40.0)),
                st("wait 3", p(sx + 300.0, sy - 40.0)),
                st("wait 4", p(sx + 300.0, sy - 40.0)),
                st("wait 5", p(sx + 300.0, sy - 40.0)),
                st("wait 6: pattern 2 plays (empty)", p(sx + 300.0, sy - 40.0)),
                st("SEQ PLAY (stop: REC and the modes go off)", p(sx + 35.0, sy + 15.0)),
                st("SEQ MUTE while stopped (refused)", p(sx + 468.0, sy + 15.0)),
                st("SEQ previous pattern (stopped: at once)", p(sx + 107.5, sy + 15.0)),
                st("SEQ PLAY (pattern 1 from its start)", p(sx + 35.0, sy + 15.0)),
                st("wait 7", p(sx + 300.0, sy - 40.0)),
                st("wait 8", p(sx + 300.0, sy - 40.0)),
                st("wait 9", p(sx + 300.0, sy - 40.0)),
                st("wait 10", p(sx + 300.0, sy - 40.0)),
                st("wait 11", p(sx + 300.0, sy - 40.0)),
                st("wait 12", p(sx + 300.0, sy - 40.0)),
                st("wait 13", p(sx + 300.0, sy - 40.0)),
                st("wait 14: pattern 1 has been round", p(sx + 300.0, sy - 40.0)),
                st("SMP eject slot 10", p(1512.0 - 4.0 - 3.0 * 179.0 + 62.0, py + 34.0)),
            ]
        } else if four {
            let y1 = t + 198.0;
            let y3 = y1 + 190.0 + extra;
            let ly3 = y3 + 76.0 + extra;
            let p = |x: f32, y: f32| (pos2(x, y), pos2(x, y));
            let st = |n: &'static str, (a, b): (egui::Pos2, egui::Pos2)| (n, a, b);
            vec![
                st("4D FX panel button", p(205.0, 14.0)),
                st("4D MASTER deck 3 (lower half)", p(655.0 - 42.0, y3 + 39.0)),
                st("4D loop size down deck 3", p(382.0 + 16.0, ly3 + 6.0 + 49.0)),
                st("4D PLAY deck 4", p(855.0 + 10.0 + 20.0, ly3 + 84.0)),
                st("4D hot cue pad B deck 4", p(855.0 + 285.0 + 40.0, ly3 + 6.0 + 19.6 + 7.0)),
                ("4D channel fader 4 down", pos2(661.0 + 162.5, y1 + 224.5), pos2(661.0 + 162.5, y1 + 284.0)),
                st("quick click on CUE of channel 3", p(661.0 + 30.0, y1 + 198.0)),
                st("4D BEAT SYNC deck 2", p(1512.0 - 76.0 + 20.0, y1 + 14.0)),
                st("4D PLAY deck 4 (pause)", p(855.0 + 10.0 + 20.0, ly3 + 84.0)),
            ]
        } else {
            steps
        };
        const START: u32 = 140;
        const SPAN: u32 = 40;
        if frame < START {
            return;
        }
        let i = ((frame - START) / SPAN) as usize;
        let f = (frame - START) % SPAN;
        let Some((name, a, b)) = steps.get(i).copied() else { return };
        if smp && frame == START {
            // Nothing is heard during the run (the samples and the metronome still run).
            if let Some(e) = crate::sampler::SAMPLER.lock().as_mut() {
                e.silent = true;
            }
        }
        if name.starts_with("wait") {
            // (The run is not tied to the screen's refresh: a wait step is made to last about
            // 0.65 s, so that the pattern, which runs in real time, gets somewhere.)
            std::thread::sleep(std::time::Duration::from_millis(16));
            if f == 0 {
                raw.events.push(egui::Event::PointerMoved(a));
            }
            if f == 30 {
                eprintln!("perftest: {name}: {}", self.perf_state_sampler());
            }
            return;
        }
        let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        match f {
            0 => raw.events.push(egui::Event::PointerMoved(a)),
            3 => {
                raw.events.push(button(a, true));
                if name.starts_with("quick") {
                    raw.events.push(button(a, false));
                }
            }
            10 if smp && name.contains("held") => eprintln!("perftest: {name} (while held): {}", self.perf_state_sampler()),
            10 if name.contains("held") => eprintln!("perftest: {name} (while held): {}", self.perf_state()),
            4..=11 => {
                if a != b {
                    let k = (f - 3) as f32 / 8.0;
                    raw.events.push(egui::Event::PointerMoved(a + (b - a) * k));
                }
            }
            12 => {
                if !name.starts_with("quick") {
                    raw.events.push(button(b, false));
                }
            }
            // The loop is left running for a while before its state is read.
            30 if smp => eprintln!("perftest: {name}: {}", self.perf_state_sampler()),
            30 if four => eprintln!("perftest: {name}: {}", self.perf_state4()),
            30 => eprintln!("perftest: {name}: {}", self.perf_state()),
            _ => {}
        }
    }

    fn perf_state_sampler(&self) -> String {
        let sp = &self.prefs.sampler;
        let playing: Vec<usize> = match crate::sampler::SAMPLER.lock().as_ref() {
            Some(e) => (0..16).filter(|pad| e.slot_state(*pad, sp.bank as usize).0).map(|pad| pad + 1).collect(),
            None => Vec::new(),
        };
        let loaded: Vec<usize> = sp.slots.iter().enumerate().filter(|(_, s)| s.is_some()).map(|(i, _)| i + 1).collect();
        let s1 = sp.slots[0].as_ref().map(|s| (s.looped, s.gate));
        // The sequencer: transport, the current pattern's notes as (beat, pad, on), what is
        // saved, and how much of it has been played since PLAY.
        let seq = match crate::sampler::SAMPLER.lock().as_ref() {
            Some(e) => {
                let q = &e.seq;
                let pat = q.pattern(q.current());
                let notes: Vec<String> = pat.events.iter().map(|ev| format!("{:.2}:{}{}", ev.t, ev.pad + 1, if ev.on { "" } else { "off" })).collect();
                let muted: Vec<usize> = (0..16).filter(|pad| self.px.sampler.is_muted(sp.bank as usize * 16 + pad)).map(|pad| pad + 1).collect();
                format!(
                    "playing {} rec {} pos {:.2}/{} pattern {} (reserved {:?}) bars {} notes [{}] dirty {} passes {} fired {} | muted {muted:?} mode {}",
                    q.playing,
                    q.rec,
                    q.pos(),
                    q.beats(),
                    q.current() + 1,
                    q.reserved().map(|n| n + 1),
                    pat.bars,
                    notes.join(" "),
                    q.dirty(q.current()),
                    q.passes,
                    q.fired,
                    self.px.sampler.seq_mode
                )
            }
            None => String::new(),
        };
        let saved: Vec<String> = sp.seq.patterns.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|p| format!("{}: {} bars, {} notes", i + 1, p.bars, p.events.len()))).collect();
        format!(
            "loaded {loaded:?} | playing pads {playing:?} | slot 1 (loop, gate) {s1:?} | bank {} Q {} sync {} bpm {:.2} volume {} gain {} | SEQ {seq} | saved {saved:?} current {} metronome {} vol {}",
            sp.bank + 1,
            sp.quantize,
            sp.sync,
            sp.bpm,
            sp.volume,
            sp.gain,
            sp.seq.current + 1,
            sp.seq.metronome,
            sp.seq.metronome_vol
        )
    }

    fn perf_state4(&self) -> String {
        let act = self.px.active as usize;
        let loops: Vec<f64> = (0..4).map(|n| if n == act { self.prefs.loop_beats } else { self.px.parked_loop[n].unwrap_or(self.prefs.loop_beats) }).collect();
        let d4 = self.deck_of(3);
        let hot: Vec<i32> = d4.cues.iter().filter(|c| c.kind == crate::model::CueKind::Hot).map(|c| c.slot).collect();
        let playing: Vec<bool> = self.player.decks.iter().map(|d| d.lock().playing).collect();
        format!(
            "fx panel {} | active deck {} master deck {} | loop sizes {:?} | playing {:?} | deck 4 hot cues {:?} | fader 4 {:.2} | cue 3 {} | deck 2 beat sync {}",
            self.prefs.perf_fx,
            act + 1,
            self.px.master + 1,
            loops,
            playing,
            hot,
            self.px.mix[3].fader,
            self.px.mix[2].cue,
            self.deck_of(1).x.beat_sync
        )
    }

    fn perf_state(&self) -> String {
        let act = self.px.active as usize;
        let (d1, s1) = (self.deck_of(0), &self.player.decks[0]);
        let s = s1.lock();
        let secs = s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0);
        let hot: Vec<i32> = d1.cues.iter().filter(|c| c.kind == crate::model::CueKind::Hot).map(|c| c.slot).collect();
        let m = self.px.mix[0];
        let q1 = if act == 0 { self.prefs.quantize } else { self.px.parked_quantize[0] };
        format!(
            "cfx color {:?} (deck {:.2}) | fx1 assign {:?} echo on {} value {} release {:?} | fader2 {:.2} volume {:.2} | playing {} pos {:.2}s loop {:?} slip {} bg {:?} hot {:?} range step {} Q {} MT {} key {:+} | fader {:.2} (gain {:.3}) high {:.2} (pos {:.2}) xfade {:.2} assign {} xf-gain {:.2} stem-iso {} | master deck {} beat-sync {} tempo {:.4}",
            self.px.cfx_color,
            s.cfx_color,
            self.px.fx[0].params.assign,
            self.px.fx[0].params.slots[1].on,
            self.px.fx[0].params.slots[1].value,
            self.px.fx[0].params.release.is_some(),
            self.px.mix[1].fader,
            s.volume,
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
