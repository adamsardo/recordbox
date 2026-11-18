//! The sampler panel of PERFORMANCE mode, after rekordbox 7.2.19's `SamplerControlPanel`
//! (118 pt high under the mixer's bottom row). Positions are those of its `resized`
//! functions (`SamplerControlPanel::resized` 0x100b07668, `SamplerControlComp::resized`
//! 0x100aefd64, `SequencerComponent::resized` 0x1003c0380) and of reference
//! performance/08-sampler.
//!
//! 16 slots per bank, four banks. A slot is loaded by dropping a track on it. The sequencer
//! strip is drawn but not built yet.

use crate::app::*;
use crate::sampler::{self, Clip, MODE_GATE, MODE_LOOP, PADS};
use crate::theme::*;
use crate::ui_player::{lighten, tip};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const SAMPLER_H: f32 = 118.0;
const SLOT_H: f32 = 44.0;
const PANEL: Color32 = hex(0x1E1E1E);
const OFF: Color32 = hex(0x323232);

/// One loaded slot, as it is kept between sessions (rekordbox keeps the same values in its
/// `SamplerSettings` file: loop, gate, mt, sync, gain 0..100, offset in ms).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SlotPrefs {
    pub track: i64,
    pub path: String,
    pub name: String,
    pub looped: bool,
    pub gate: bool,
    pub mt: bool,
    pub sync: bool,
    pub gain: f32,
    pub offset_ms: f32,
    pub bpm: f32,
}

impl Default for SlotPrefs {
    fn default() -> SlotPrefs {
        SlotPrefs { track: 0, path: String::new(), name: String::new(), looped: false, gate: false, mt: true, sync: true, gain: 50.0, offset_ms: 0.0, bpm: 0.0 }
    }
}

impl SlotPrefs {
    fn mode(&self) -> u8 {
        (if self.looped { MODE_LOOP } else { 0 }) | (if self.gate { MODE_GATE } else { 0 })
    }
}

/// rekordbox's defaults: GAIN 50, volume 100, 120 BPM, bank 1, Q and BPM SYNC off.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SamplerPrefs {
    /// 64 slots: bank * 16 + pad.
    pub slots: Vec<Option<SlotPrefs>>,
    pub gain: f32,
    pub volume: f32,
    pub bpm: f32,
    pub bank: u8,
    pub quantize: bool,
    pub sync: bool,
    pub cue: bool,
}

impl Default for SamplerPrefs {
    fn default() -> SamplerPrefs {
        SamplerPrefs { slots: vec![None; 64], gain: 50.0, volume: 100.0, bpm: 120.0, bank: 0, quantize: false, sync: false, cue: false }
    }
}

type Loaded = Result<Arc<crate::audio::Decoded>, String>;

/// What the panel keeps while the app runs.
pub struct SamplerUi {
    restored: bool,
    loading: Vec<(usize, std::sync::mpsc::Receiver<Loaded>)>,
    /// Mini waveform of each loaded slot: peak per column (150 columns).
    wave: Vec<Option<Vec<u8>>>,
    /// The slot whose EDIT panel is open.
    edit: Option<usize>,
    mute: Vec<bool>,
    pad_db: [f32; PADS],
    /// BPM + / - held: direction, when it was pressed and how many repeats have fired.
    bpm_hold: Option<(f32, f64, u32)>,
    /// The pad whose PLAY button the pointer holds.
    held: Option<usize>,
}

impl SamplerUi {
    pub fn new() -> SamplerUi {
        SamplerUi { restored: false, loading: Vec::new(), wave: vec![None; 64], edit: None, mute: vec![false; 64], pad_db: [-120.0; PADS], bpm_hold: None, held: None }
    }
}

const WAVE_COLS: usize = 150;

fn mini_wave(a: &crate::audio::Decoded) -> Vec<u8> {
    let frames = a.frames().max(1);
    (0..WAVE_COLS)
        .map(|c| {
            let (s, e) = (c * frames / WAVE_COLS, ((c + 1) * frames / WAVE_COLS).max(c * frames / WAVE_COLS + 1).min(frames));
            let stride = ((e - s) / 256).max(1);
            let peak = (s..e).step_by(stride).fold(0.0f32, |m, i| m.max(a.samples[i * 2].abs()).max(a.samples[i * 2 + 1].abs()));
            (peak.min(1.0) * 255.0) as u8
        })
        .collect()
}

fn decode_in_background(ui: &mut SamplerUi, idx: usize, path: String) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::audio::decode_file(&path).map(Arc::new).map_err(|e| e.to_string()));
    });
    ui.loading.retain(|(i, _)| *i != idx);
    ui.loading.push((idx, rx));
}

impl App {
    /// Load a library track into a sampler slot (bank * 16 + pad). The slot keeps its mode
    /// buttons when it was loaded before, as in rekordbox.
    pub fn sampler_load(&mut self, idx: usize, id: i64) {
        let Some(t) = self.track(id).cloned() else { return };
        if t.missing {
            self.toast_err(format!("The file for \"{}\" cannot be found.", t.display_title()));
            return;
        }
        let mut s = self.prefs.sampler.slots.get(idx).cloned().flatten().unwrap_or_default();
        s.track = id;
        s.path = t.path.clone();
        s.name = t.display_title();
        s.bpm = t.bpm as f32;
        s.offset_ms = 0.0;
        if self.prefs.sampler.slots.len() < 64 {
            self.prefs.sampler.slots.resize(64, None);
        }
        self.prefs.sampler.slots[idx] = Some(s);
        self.px.sampler.wave[idx] = None;
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            e.set_clip(idx % PADS, idx / PADS, None);
        }
        decode_in_background(&mut self.px.sampler, idx, t.path);
        self.save_prefs();
    }

    pub fn sampler_eject(&mut self, idx: usize) {
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            e.set_clip(idx % PADS, idx / PADS, None);
        }
        if let Some(s) = self.prefs.sampler.slots.get_mut(idx) {
            *s = None;
        }
        self.px.sampler.wave[idx] = None;
        self.px.sampler.mute[idx] = false;
        self.px.sampler.loading.retain(|(i, _)| *i != idx);
        if self.px.sampler.edit == Some(idx) {
            self.px.sampler.edit = None;
        }
        self.save_prefs();
    }

    /// Once per frame in PERFORMANCE mode: bring the saved slots back the first time, take in
    /// finished decodes, and hand the deck's settings to the audio engine.
    pub fn sampler_poll(&mut self) {
        if !self.px.sampler.restored {
            self.px.sampler.restored = true;
            if self.prefs.sampler.slots.len() != 64 {
                self.prefs.sampler.slots.resize(64, None);
            }
            sampler::SAMPLER.lock().get_or_insert_with(|| Box::new(sampler::Engine::new()));
            let saved: Vec<(usize, String)> = self.prefs.sampler.slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, s.path.clone()))).collect();
            for (i, path) in saved {
                // A file that is gone leaves its slot empty (rekordbox removes it the same way).
                if std::path::Path::new(&path).exists() {
                    decode_in_background(&mut self.px.sampler, i, path);
                } else {
                    self.prefs.sampler.slots[i] = None;
                }
            }
        }
        let mut done = Vec::new();
        self.px.sampler.loading.retain(|(i, rx)| match rx.try_recv() {
            Ok(r) => {
                done.push((*i, r));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (i, r) in done {
            let Some(s) = self.prefs.sampler.slots[i].clone() else { continue };
            match r {
                Ok(a) => {
                    self.px.sampler.wave[i] = Some(mini_wave(&a));
                    let clip = Clip { audio: a, mode: s.mode(), gain: (s.gain / 50.0).clamp(0.0, 2.0), sync: s.sync, mt: s.mt, bpm: s.bpm, offset_ms: s.offset_ms, mute: self.px.sampler.mute[i] };
                    if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                        e.set_clip(i % PADS, i / PADS, Some(clip));
                    }
                }
                Err(e) => {
                    self.toast_err(format!("\"{}\" could not be loaded into the sampler: {e}", s.name));
                    self.prefs.sampler.slots[i] = None;
                }
            }
        }
        // BPM SYNC: the sampler follows the master deck's tempo.
        let sp = &self.prefs.sampler;
        let mut bpm = sp.bpm;
        if sp.sync {
            let m = self.px.master.min(self.deck_count().saturating_sub(1));
            let b = self.with_deck(m, |a| if a.deck.track.is_some() { a.effective_bpm() } else { 0.0 });
            if b > 0.0 {
                bpm = b as f32;
            }
        }
        let sp = &self.prefs.sampler;
        let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
        let cue_gain = if sp.cue && hp_on { crate::mixer::cue_master_balance(1.0 - self.px.hp_mix) * crate::mixer::headphones_level(self.px.hp_level) } else { 0.0 };
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            // GAIN: rekordbox's trim table, unity at the centre; the fader: the channel fader's law.
            e.gain = crate::mixer::trim_gain(sp.gain / 100.0);
            e.volume = crate::mixer::channel_fader_gain(sp.volume / 100.0, self.prefs.ch_fader_curve);
            e.cue_gain = cue_gain;
            e.bpm = bpm.clamp(40.0, 499.0);
            e.quantize = sp.quantize;
        }
    }

    /// The BPM the sampler deck shows: its own, or the master deck's while BPM SYNC is on.
    fn sampler_shown_bpm(&mut self) -> f32 {
        if self.prefs.sampler.sync {
            let m = self.px.master.min(self.deck_count().saturating_sub(1));
            let b = self.with_deck(m, |a| if a.deck.track.is_some() { a.effective_bpm() } else { 0.0 });
            if b > 0.0 {
                return b as f32;
            }
        }
        self.prefs.sampler.bpm
    }

    fn sampler_push_slot(&mut self, idx: usize) {
        let Some(s) = self.prefs.sampler.slots[idx].clone() else { return };
        let mute = self.px.sampler.mute[idx];
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            if let Some(c) = e.clip_mut(idx % PADS, idx / PADS) {
                c.mode = s.mode();
                c.gain = (s.gain / 50.0).clamp(0.0, 2.0);
                c.sync = s.sync;
                c.mt = s.mt;
                c.bpm = s.bpm;
                c.offset_ms = s.offset_ms;
                c.mute = mute;
            }
        }
        self.save_prefs();
    }
}

/// A small text button without a box (Q, BPM SYNC, MASTER, MUTE, ERASE): grey, blue when on.
fn text_button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, on: bool, enabled: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let col = if !enabled { hex(0x424242) } else if on { BLUE } else if resp.hovered() { WHITE } else { hex(0x8C8C8C) };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, caps(9.5), col);
    resp
}

fn box_button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, on: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(r, 2.0, if on { BLUE } else if resp.hovered() { lighten(BTN) } else { BTN });
    if !label.is_empty() {
        ui.painter().text(r.center(), Align2::CENTER_CENTER, label, caps(8.0), WHITE);
    }
    resp
}

/// The panel. `r`: 118 pt high, the full width of the window.
pub fn panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    p.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 1.0)), 0.0, Color32::BLACK);
    let (x0, y0, w, h) = (r.min.x, r.min.y, r.width(), r.height());
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x0 + x, y0 + y), vec2(w, h));
    let bank = app.prefs.sampler.bank.min(3) as usize;
    let slot_w = ((w - 80.0) / 8.0).floor().max(146.0);
    let dt = ui.input(|i| i.stable_dt).min(0.1);

    // Levels and play positions of the visible bank, read once.
    let mut states = [(false, 0.0f32); PADS];
    if let Some(e) = sampler::SAMPLER.lock().as_mut() {
        for (pad, st) in states.iter_mut().enumerate() {
            *st = e.slot_state(pad, bank);
            let pk = e.take_pad_peak(pad);
            let db = if pk > 0.0 { 20.0 * pk.log10() } else { -120.0 };
            app.px.sampler.pad_db[pad] = crate::mixer::meter_fall(app.px.sampler.pad_db[pad], db, dt);
        }
    }
    if states.iter().any(|s| s.0) || app.px.sampler.pad_db.iter().any(|d| *d > -100.0) || !app.px.sampler.loading.is_empty() {
        ui.ctx().request_repaint();
    }

    // ---- the 16 slots: pads 1-4 and 5-8 on the left, 9-12 and 13-16 on the right
    let slot_rect = |pad: usize| {
        let (col, row) = (pad & 3, (pad & 7) >> 2);
        let x = if pad < 8 { col as f32 * slot_w + 4.0 } else { w + (col as f32 - 4.0) * slot_w - 4.0 };
        at(x, SLOT_H * row as f32 + 4.0, slot_w, SLOT_H)
    };
    let shift = ui.input(|i| i.modifiers.shift);
    let released = ui.input(|i| i.pointer.any_released());
    let pointer = ui.ctx().pointer_latest_pos();
    for pad in 0..PADS {
        let idx = bank * PADS + pad;
        let sr = slot_rect(pad);
        let (sx, sy, sw) = (sr.min.x, sr.min.y, sr.width());
        let s_at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(sx + x, sy + y), vec2(w, h));
        p.rect_filled(sr.shrink(4.0), 0.0, Color32::BLACK);
        let slot = app.prefs.sampler.slots.get(idx).cloned().flatten();
        let loading = app.px.sampler.loading.iter().any(|(i, _)| *i == idx);
        let (playing, frac) = states[pad];
        let hovered = pointer.is_some_and(|pp| sr.contains(pp));

        // Level meter (4 x 32) at the right edge.
        let mr = s_at(sw - 11.0, 6.0, 4.0, 32.0);
        p.rect_filled(mr, 0.0, OFF);
        let lv = ((app.px.sampler.pad_db[pad] + 40.0) / 40.0).clamp(0.0, 1.0);
        if lv > 0.0 {
            p.rect_filled(Rect::from_min_max(pos2(mr.min.x, mr.max.y - mr.height() * lv), mr.max), 0.0, hex(0x1EC81E));
        }

        // Mini waveform (w - 29 by 16) with the play position.
        let wr = s_at(6.0, 6.0, sw - 29.0, 16.0);
        if let Some(cols) = app.px.sampler.wave[idx].as_ref() {
            let mut bars = crate::ui_kit::Bars::new();
            let n = wr.width().max(1.0) as usize;
            for i in 0..n {
                let v = cols[(i * cols.len() / n).min(cols.len() - 1)] as f32 / 255.0;
                let hh = (v * wr.height() / 2.0).max(0.5);
                let x = wr.min.x + i as f32;
                let done = playing && (i as f32 / n as f32) < frac;
                bars.rect(Rect::from_min_max(pos2(x, wr.center().y - hh), pos2(x + 1.0, wr.center().y + hh)), if done { hex(0x5A5A5A) } else { hex(0xC8C8C8) });
            }
            bars.paint(&p);
            if playing {
                let x = wr.min.x + wr.width() * frac.clamp(0.0, 1.0);
                p.rect_filled(Rect::from_min_max(pos2(x - 0.5, wr.min.y - 1.0), pos2(x + 0.5, wr.max.y + 1.0)), 0.0, hex(0xE61E1E));
            }
        } else if loading {
            p.text(wr.left_center(), Align2::LEFT_CENTER, "Loading…", FontId::proportional(9.5), DIM);
        }

        // PLAY (26 x 14): fires on mouse-down; Shift + PLAY stops; the release stops gate modes.
        let pr = s_at(6.0, 23.0, 26.0, 14.0);
        let presp = ui.interact(pr, ui.id().with(("smp_play", idx)), Sense::click_and_drag());
        let loaded = slot.is_some() && app.px.sampler.wave[idx].is_some();
        p.rect_filled(pr, 2.0, if !loaded { OFF } else if playing { BLUE } else if presp.hovered() { lighten(hex(0x0F4FA8)) } else { hex(0x0F4FA8) });
        if loaded {
            let c = pr.center();
            p.add(egui::Shape::convex_polygon(vec![c + vec2(-3.0, -4.0), c + vec2(4.0, 0.0), c + vec2(-3.0, 4.0)], WHITE, Stroke::NONE));
            if crate::ui_player::press_fired(ui, &presp) {
                if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                    if shift {
                        e.stop(pad, bank, true);
                    } else {
                        e.launch(pad, bank, 1.0);
                    }
                }
                app.px.sampler.held = Some(pad);
            }
        }
        if app.px.sampler.held == Some(pad) && released {
            if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                e.stop(pad, bank, false);
            }
            app.px.sampler.held = None;
        }
        let _ = tip(presp, app, "Play the sample. Shift + click stops it.");

        if let Some(s) = &slot {
            let editing = app.px.sampler.edit == Some(idx);
            let tools = hovered || editing;
            // Title (from x 38), shortened while the slot's buttons show.
            let tx = if tools { 74.0 } else { 38.0 };
            let tw = sw - 53.0 - (tx - 38.0) - if tools { 34.0 } else { 0.0 };
            let clip = p.with_clip_rect(s_at(tx, 24.0, tw.max(10.0), 14.0));
            clip.text(pos2(sx + tx, sy + 31.0), Align2::LEFT_CENTER, &s.name, FontId::proportional(10.5), WHITE);
            if tools {
                // Mute (16 x 14), Eject (16 x 14) and EDIT (30 x 14).
                let muted = app.px.sampler.mute[idx];
                let mresp = box_button(ui, s_at(34.0, 23.0, 16.0, 14.0), ("smp_mute", idx), "M", muted);
                if tip(mresp, app, "Mute").clicked() {
                    app.px.sampler.mute[idx] = !muted;
                    app.sampler_push_slot(idx);
                }
                let er = s_at(54.0, 23.0, 16.0, 14.0);
                let eresp = box_button(ui, er, ("smp_eject", idx), "", false);
                let c = er.center();
                p.add(egui::Shape::convex_polygon(vec![c + vec2(-4.0, 0.5), c + vec2(0.0, -4.0), c + vec2(4.0, 0.5)], WHITE, Stroke::NONE));
                p.rect_filled(Rect::from_center_size(c + vec2(0.0, 3.0), vec2(8.0, 1.5)), 0.0, WHITE);
                if tip(eresp, app, "Eject").clicked() {
                    app.sampler_eject(idx);
                    continue;
                }
                let dresp = box_button(ui, s_at(sw - 47.0, 23.0, 30.0, 14.0), ("smp_edit", idx), "EDIT", editing);
                if tip(dresp, app, "EDIT").clicked() {
                    app.px.sampler.edit = if editing { None } else { Some(idx) };
                }
            }
        }

        // A track dragged from the browser loads here.
        if let Some(ids) = app.drag.clone() {
            if hovered {
                p.rect_stroke(sr.shrink(3.0), 0.0, Stroke::new(2.0_f32, BLUE), egui::StrokeKind::Inside);
                if released {
                    if let Some(id) = ids.first() {
                        app.sampler_load(idx, *id);
                    }
                }
            }
        }
    }

    // ---- the sampler deck's strip between the two groups of slots
    let c = ((w - 80.0) / 2.0).floor();
    let mut sp = app.prefs.sampler.clone();
    let mut changed = false;
    if let Some(v) = crate::ui_perf::knob(ui, pos2(x0 + c + 22.0, y0 + 24.0), "smp_gain", sp.gain / 100.0, 0.5, BLUE) {
        sp.gain = (v * 100.0).round();
        changed = true;
    }
    p.text(pos2(x0 + c + 22.0, y0 + 39.0), Align2::CENTER_CENTER, "GAIN", caps(8.0), hex(0x8C8C8C));
    let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
    let cr = at(c + 7.0, 50.0, 30.0, 14.0);
    let cresp = ui.interact(cr, ui.id().with("smp_cue"), Sense::click());
    p.rect_filled(cr, 2.0, if sp.cue { hex(0xFFA500) } else if cresp.hovered() { lighten(BTN) } else { BTN });
    p.text(cr.center(), Align2::CENTER_CENTER, "CUE", caps(8.0), if sp.cue { BG } else { hex(0x8C8C8C) });
    if cresp.on_hover_text(if hp_on { "Headphones CUE of the sampler" } else { "Headphones CUE of the sampler.\nNo headphones output is set: choose one in Preferences > Audio > Input/Output." }).clicked() {
        sp.cue = !sp.cue;
        changed = true;
    }
    // Volume fader (26 x 47).
    let fr = at(c + 46.0, 17.0, 26.0, 47.0);
    let (ft, fb, fx) = (fr.min.y + 3.5, fr.max.y - 2.0, fr.center().x);
    let fresp = ui.interact(fr, ui.id().with("smp_vol"), Sense::click_and_drag());
    if let Some(pp) = fresp.interact_pointer_pos().filter(|_| fresp.dragged() || fresp.is_pointer_button_down_on() || fresp.clicked()) {
        sp.volume = (((fb - pp.y) / (fb - ft)).clamp(0.0, 1.0) * 100.0).round();
        changed = true;
    }
    let hy = fb - sp.volume / 100.0 * (fb - ft);
    p.rect_filled(Rect::from_min_max(pos2(fx - 1.0, ft), pos2(fx + 1.0, fb)), 0.0, OFF);
    p.rect_filled(Rect::from_min_max(pos2(fx - 1.0, hy), pos2(fx + 1.0, fb)), 0.0, BLUE);
    p.rect_filled(Rect::from_center_size(pos2(fx, hy), vec2(24.0, 2.0)), 0.0, if fresp.hovered() || fresp.dragged() { WHITE } else { hex(0x646464) });

    // BPM (bold 18, 52 x 20): drag up or down, or the wheel; + and - under it.
    let synced = sp.sync;
    let shown = app.sampler_shown_bpm();
    let br = at(w / 2.0 - 26.0, 68.0, 52.0, 20.0);
    let bresp = ui.interact(br, ui.id().with("smp_bpm"), Sense::click_and_drag());
    let whole = format!("{}", shown.floor() as i32);
    let frac = format!(".{:02}", ((shown - shown.floor()) * 100.0).round().min(99.0) as i32);
    let g1 = p.layout_no_wrap(whole, bold(16.5), WHITE);
    let g2 = p.layout_no_wrap(frac, bold(12.5), WHITE);
    let tx = br.center().x - (g1.size().x + g2.size().x) / 2.0;
    let base = br.center().y + g1.size().y / 2.0;
    p.galley(pos2(tx, base - g1.size().y), g1.clone(), WHITE);
    p.galley(pos2(tx + g1.size().x, base - g2.size().y - 1.0), g2, WHITE);
    if !synced {
        if bresp.dragged() {
            sp.bpm = (sp.bpm - bresp.drag_delta().y * 0.2).clamp(40.0, 499.0);
            changed = true;
        }
        let wheel = if bresp.hovered() { ui.input(|i| i.raw_scroll_delta.y) } else { 0.0 };
        if wheel != 0.0 {
            sp.bpm = (sp.bpm + 0.01 * wheel.signum()).clamp(40.0, 499.0);
            changed = true;
        }
    }
    let _ = bresp.on_hover_text("Sampler BPM");
    // + / -: 0.01 on the press; held, it repeats every 100 ms after 500 ms: nine steps of
    // 0.01, twenty of 0.1, then 0.2.
    let now = ui.input(|i| i.time);
    for (dir, x, label) in [(1.0f32, w / 2.0 - 22.0, "+"), (-1.0, w / 2.0 + 2.0, "-")] {
        let rr = at(x, h - 25.0, 20.0, 20.0);
        let resp = ui.interact(rr, ui.id().with(("smp_bpm_step", label)), Sense::click_and_drag());
        let col = if synced { hex(0x424242) } else if resp.hovered() { WHITE } else { hex(0xC8C8C8) };
        p.rect_filled(Rect::from_center_size(rr.center(), vec2(9.0, 1.2)), 0.0, col);
        if dir > 0.0 {
            p.rect_filled(Rect::from_center_size(rr.center(), vec2(1.2, 9.0)), 0.0, col);
        }
        if synced {
            continue;
        }
        if crate::ui_player::press_fired(ui, &resp) {
            sp.bpm = (((sp.bpm + 0.01 * dir) * 100.0).round() / 100.0).clamp(40.0, 499.0);
            changed = true;
            app.px.sampler.bpm_hold = Some((dir, now, 0));
        }
        if let Some((d, t0, ticks)) = app.px.sampler.bpm_hold.filter(|hold| hold.0 == dir) {
            if resp.is_pointer_button_down_on() {
                let due = ((now - t0 - 0.5) / 0.1).floor();
                if due >= ticks as f64 {
                    let t = ticks + 1;
                    let step = if t < 10 { 0.01 } else if t < 30 { 0.1 } else { 0.2 };
                    let snapped = if t < 10 { sp.bpm } else { (sp.bpm / step).round() * step };
                    sp.bpm = (((snapped + step * d) * 100.0).round() / 100.0).clamp(40.0, 499.0);
                    changed = true;
                    app.px.sampler.bpm_hold = Some((d, t0, t));
                }
                ui.ctx().request_repaint();
            } else {
                app.px.sampler.bpm_hold = None;
            }
        }
    }

    // ---- the bottom line: BANK, Q, BPM SYNC, MASTER
    p.text(pos2(x0 + 6.0, y0 + h - 13.0), Align2::LEFT_CENTER, "BANK", caps(9.5), hex(0xC8C8C8));
    for b in 0..4usize {
        let rr = at(43.0 + 28.0 * b as f32, h - 19.0, 22.0, 10.0);
        let resp = ui.interact(rr, ui.id().with(("smp_bank", b)), Sense::click());
        p.rect_filled(rr, 1.0, if bank == b { BLUE } else if resp.hovered() { lighten(hex(0x282828)) } else { hex(0x282828) });
        if crate::ui_player::press_fired(ui, &resp) && bank != b {
            sp.bank = b as u8;
            // Changing the bank closes an open EDIT panel.
            app.px.sampler.edit = None;
            changed = true;
        }
        let _ = resp.on_hover_text(format!("BANK {}", b + 1));
    }
    let master_r = at(4.0 * slot_w - 92.0, h - 20.0, 51.0, 10.0);
    let sync_r = Rect::from_min_size(pos2(master_r.min.x - 88.0, master_r.min.y), vec2(68.0, 10.0));
    let q_r = Rect::from_min_size(pos2(sync_r.min.x - 30.0, y0 + h - 19.0), vec2(9.0, 10.0));
    if text_button(ui, q_r.expand(3.0), "smp_q", "Q", sp.quantize, true).on_hover_text("QUANTIZE: samples start on the sampler's next beat").clicked() {
        sp.quantize = !sp.quantize;
        changed = true;
    }
    if text_button(ui, sync_r, "smp_sync", "BPM SYNC", sp.sync, true).on_hover_text("BPM SYNC: the sampler follows the master deck's BPM").clicked() {
        sp.sync = !sp.sync;
        changed = true;
    }
    let _ = text_button(ui, master_r, "smp_master", "MASTER", false, false).on_hover_text("MASTER (the sampler as sync master): not built in recordbox yet.");

    // ---- the sequencer strip (590 x 30 from the centre + 40): drawn, not built yet
    let so = pos2(x0 + (w / 2.0).floor() + 40.0, y0 + h - 30.0);
    let q_at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(so + vec2(x, y), vec2(w, h));
    let not_yet = "Sequencer: not built in recordbox yet.";
    let rec = q_at(0.0, 5.0, 20.0, 20.0);
    p.rect_filled(rec, 2.0, hex(0x141414));
    p.circle_filled(rec.center(), 5.5, hex(0xC81E1E));
    p.circle_stroke(rec.center(), 7.0, Stroke::new(1.0_f32, hex(0x8C1414)));
    let play = q_at(25.0, 5.0, 20.0, 20.0);
    p.rect_filled(play, 2.0, hex(0x141414));
    let pc = play.center();
    p.add(egui::Shape::convex_polygon(vec![pc + vec2(-3.5, -5.0), pc + vec2(5.0, 0.0), pc + vec2(-3.5, 5.0)], WHITE, Stroke::NONE));
    let save = q_at(50.0, 5.0, 46.0, 20.0);
    p.rect_filled(save, 2.0, hex(0x0A0A0A));
    p.text(save.center(), Align2::CENTER_CENTER, "SAVE", caps(9.5), hex(0x424242));
    let dimc = hex(0x646464);
    let al = q_at(105.0, 11.0, 5.0, 8.0);
    p.add(egui::Shape::line(vec![al.right_top(), al.left_center(), al.right_bottom()], Stroke::new(1.0_f32, dimc)));
    p.text(q_at(100.0, 5.0, 110.0, 20.0).center(), Align2::CENTER_CENTER, "PATTERN 1", caps(9.5), hex(0x424242));
    let ar = q_at(200.0, 11.0, 5.0, 8.0);
    p.add(egui::Shape::line(vec![ar.left_top(), ar.right_center(), ar.left_bottom()], Stroke::new(1.0_f32, dimc)));
    let len = q_at(213.0, 6.0, 48.0, 18.0);
    p.text(len.left_center() + vec2(2.0, 0.0), Align2::LEFT_CENTER, "1Bar", FontId::proportional(11.0), WHITE);
    crate::icons_player::chevron_down(&p, len.right_center() - vec2(6.0, 0.0), 1.0, WHITE);
    let beat = q_at(270.0, 10.0, 163.0, 12.0);
    p.rect_filled(Rect::from_min_max(pos2(beat.min.x, beat.center().y - 2.5), pos2(beat.max.x, beat.center().y + 2.5)), 0.0, Color32::BLACK);
    for k in 1..4 {
        let x = beat.min.x + beat.width() * k as f32 / 4.0;
        p.rect_filled(Rect::from_min_max(pos2(x, beat.center().y - 3.5), pos2(x + 1.0, beat.center().y - 2.5)), 0.0, hex(0x323232));
    }
    p.rect_filled(Rect::from_min_size(pos2(beat.min.x, beat.min.y), vec2(4.0, 12.0)), 1.0, WHITE);
    p.text(q_at(445.0, 5.0, 46.0, 20.0).center(), Align2::CENTER_CENTER, "MUTE", caps(9.5), hex(0x424242));
    p.text(q_at(488.0, 5.0, 46.0, 20.0).center(), Align2::CENTER_CENTER, "ERASE", caps(9.5), hex(0x424242));
    // Metronome and its volume.
    let mc = q_at(545.0, 4.0, 20.0, 20.0).center();
    p.add(egui::Shape::closed_line(vec![mc + vec2(-3.0, 5.0), mc + vec2(-1.5, -5.0), mc + vec2(1.5, -5.0), mc + vec2(3.0, 5.0)], Stroke::new(1.0_f32, WHITE)));
    p.line_segment([mc + vec2(0.0, 3.0), mc + vec2(3.5, -4.0)], Stroke::new(1.0_f32, WHITE));
    let vc = q_at(570.0, 4.0, 20.0, 20.0).center();
    for (k, hh) in [6.0f32, 10.0].iter().enumerate() {
        p.rect_filled(Rect::from_center_size(vc + vec2(-2.0 + 4.0 * k as f32, 0.0), vec2(2.5, *hh)), 0.0, BLUE);
    }
    let _ = ui.interact(q_at(0.0, 0.0, 590.0, 30.0), ui.id().with("smp_seq"), Sense::hover()).on_hover_text(not_yet);

    if changed {
        app.prefs.sampler = sp;
        app.save_prefs();
    }

    // ---- the EDIT panel of one slot: beside it, over both rows (slot width - 2 by 84)
    if let Some(idx) = app.px.sampler.edit.filter(|i| i / PADS == bank && app.prefs.sampler.slots[*i].is_some()) {
        let pad = idx % PADS;
        let sr = slot_rect(pad);
        // To the right of the slot; for the last column of the right group, to its left.
        let ex = if (pad & !4) == 11 { sr.min.x - sr.width() + 2.0 } else { sr.max.x };
        let er = Rect::from_min_size(pos2(ex, y0 + 6.0), vec2(slot_w - 2.0, 2.0 * SLOT_H - 4.0));
        edit_panel(app, ui, er, idx, slot_w);
    }
}

fn edit_panel(app: &mut App, ui: &mut Ui, er: Rect, idx: usize, slot_w: f32) {
    let p = ui.painter().clone();
    // Swallow clicks so the slots underneath do not react.
    let _ = ui.interact(er, ui.id().with(("smp_editbg", idx)), Sense::click());
    p.rect_filled(er, 2.0, hex(0x141414));
    p.rect_stroke(er, 2.0, Stroke::new(1.0_f32, hex(0x3C3C3C)), egui::StrokeKind::Inside);
    let Some(mut s) = app.prefs.sampler.slots[idx].clone() else { return };
    let o = er.min;
    let v = 2.0 + ((slot_w - 148.0) / 2.0).max(0.0).floor();
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(o.x + v + x, o.y + y), vec2(w, h));
    let mut changed = false;

    // One-shot / loop (20 and 21 x 14) and gate (18 x 14).
    let one = box_button(ui, at(4.0, 8.0, 20.0, 14.0), ("smp_one", idx), "", !s.looped);
    let c = one.rect.center();
    p.line_segment([c + vec2(-5.0, 0.0), c + vec2(3.0, 0.0)], Stroke::new(1.2_f32, WHITE));
    p.line_segment([c + vec2(5.0, -4.0), c + vec2(5.0, 4.0)], Stroke::new(1.2_f32, WHITE));
    p.add(egui::Shape::convex_polygon(vec![c + vec2(0.5, -3.0), c + vec2(4.0, 0.0), c + vec2(0.5, 3.0)], WHITE, Stroke::NONE));
    if tip(one, app, "One-shot: the sample plays once").clicked() && s.looped {
        s.looped = false;
        changed = true;
    }
    let lp = box_button(ui, at(25.0, 8.0, 21.0, 14.0), ("smp_loop", idx), "", s.looped);
    let c = lp.rect.center();
    let ring: Vec<Pos2> = (0..=10).map(|i| { let a = (40.0 + 28.0 * i as f32).to_radians(); c + vec2(a.cos() * 5.5, a.sin() * 3.5) }).collect();
    let tipp = *ring.last().unwrap();
    p.add(egui::Shape::line(ring, Stroke::new(1.2_f32, WHITE)));
    p.add(egui::Shape::convex_polygon(vec![tipp + vec2(-3.0, -1.0), tipp + vec2(1.5, -2.5), tipp + vec2(0.5, 2.5)], WHITE, Stroke::NONE));
    if tip(lp, app, "Loop: the sample repeats; only loop samples follow the sampler's BPM").clicked() && !s.looped {
        s.looped = true;
        changed = true;
    }
    let gate = box_button(ui, at(50.0, 8.0, 18.0, 14.0), ("smp_gate", idx), "", s.gate);
    let c = gate.rect.center();
    p.add(egui::Shape::line(vec![c + vec2(-6.0, 3.0), c + vec2(-3.0, 3.0), c + vec2(-3.0, -3.0), c + vec2(3.0, -3.0), c + vec2(3.0, 3.0), c + vec2(6.0, 3.0)], Stroke::new(1.2_f32, WHITE)));
    if tip(gate, app, "Gate: the sample sounds only while its PLAY button is held").clicked() {
        s.gate = !s.gate;
        changed = true;
    }
    // MT and SYNC (30 x 14): for loop samples.
    for (x, label, id) in [(74.0, "MT", "smp_mt"), (108.0, "SYNC", "smp_sync1")] {
        let on = if label == "MT" { s.mt } else { s.sync };
        let rr = at(x, 8.0, 30.0, 14.0);
        let resp = ui.interact(rr, ui.id().with((id, idx)), Sense::click());
        p.rect_filled(rr, 2.0, if on && s.looped { BLUE } else if resp.hovered() && s.looped { lighten(BTN) } else { BTN });
        p.text(rr.center(), Align2::CENTER_CENTER, label, caps(8.0), if s.looped { WHITE } else { hex(0x424242) });
        let hint = if label == "MT" { "MASTER TEMPO: the sample keeps its pitch when its tempo changes" } else { "SYNC: the loop sample follows the sampler's BPM" };
        if tip(resp, app, hint).clicked() && s.looped {
            if label == "MT" {
                s.mt = !s.mt;
            } else {
                s.sync = !s.sync;
            }
            changed = true;
        }
    }
    // GAIN knob (24 x 24) with its label, and the sample's BPM (64 x 20).
    if let Some(g) = crate::ui_perf::knob(ui, pos2(o.x + v + 37.0, o.y + 41.0), ("smp_slot_gain", idx), s.gain / 100.0, 0.5, WHITE) {
        s.gain = (g * 100.0).round();
        changed = true;
    }
    p.text(pos2(o.x + v + 38.0, o.y + 55.5), Align2::CENTER_CENTER, "GAIN", caps(8.0), hex(0x8C8C8C));
    let bpm_r = at(75.0, 34.0, 64.0, 20.0);
    p.text(bpm_r.center(), Align2::CENTER_CENTER, if s.bpm > 0.0 { format!("{:.2}", s.bpm) } else { "---.--".to_string() }, bold(13.0), if s.bpm > 0.0 { WHITE } else { hex(0x646464) });
    // Shift the start 1 ms earlier / later; halve / double the sample's BPM (40..499).
    let sl = box_button(ui, at(4.0, 64.0, 30.0, 14.0), ("smp_shl", idx), "", false);
    crate::icons_player::chevron_left(&p, sl.rect.center(), 0.9, WHITE);
    if tip(sl, app, "Move the start point 1 ms earlier").clicked() {
        s.offset_ms = (s.offset_ms - 1.0).max(0.0);
        changed = true;
    }
    let sr = box_button(ui, at(38.0, 64.0, 30.0, 14.0), ("smp_shr", idx), "", false);
    crate::icons_player::chevron_right(&p, sr.rect.center(), 0.9, WHITE);
    if tip(sr, app, "Move the start point 1 ms later").clicked() {
        s.offset_ms += 1.0;
        changed = true;
    }
    for (x, label, k) in [(74.0, "/2", 0.5f32), (108.0, "x2", 2.0)] {
        let resp = box_button(ui, at(x, 64.0, 30.0, 14.0), ("smp_bpmk", idx, label), label, false);
        if tip(resp, app, if k < 1.0 { "Halve the sample's BPM" } else { "Double the sample's BPM" }).clicked() && s.bpm > 0.0 {
            let nb = s.bpm * k;
            if (40.0..=499.0).contains(&nb) {
                s.bpm = nb;
                changed = true;
            } else {
                app.toast_err("The BPM must be between 40 and 499.");
            }
        }
    }
    if changed {
        app.prefs.sampler.slots[idx] = Some(s);
        app.sampler_push_slot(idx);
    }
}
