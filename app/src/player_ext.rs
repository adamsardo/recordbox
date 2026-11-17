//! Player logic beyond the basics in app.rs: the second deck (2 Players layout), loop
//! adjust, cues and loops set from the waveform menu, "Export Loop As Wav", the Beat Count
//! Display, search (scan), previous / next track and layouts.

use crate::app::*;
use crate::model::*;
use eframe::egui;

/// Per-deck state added by this module (a field of `Deck`).
#[derive(Default)]
pub struct DeckExt {
    /// Loop adjust: 0 = off, 1 = the loop-in point follows ←/→ and drags on the enlarged
    /// waveform, 2 = the loop-out point.
    pub loop_adjust: u8,
    /// Position the enlarged waveform's right-click menu acts at (seconds, not yet quantized).
    pub menu_time: Option<f64>,
    /// A drag adjusting a loop edge: pointer x and the edge's time when it began.
    pub adjust_drag: Option<(f32, f64)>,
    /// The GRID panel's BPM field is being typed in (otherwise it shows the current BPM).
    pub bpm_editing: bool,
    /// The right mouse button is holding CUE on the waveform ("Click on the waveform for PLAY
    /// and CUE").
    pub wave_cue_held: bool,
}

/// Player state shared by the layouts (a field of `App`).
pub struct PlayerExt {
    /// The deck that is not active (deck B until the decks are swapped).
    pub other: Deck,
    /// `App::deck_backup` of the other deck.
    pub other_backup: Option<(Deck, f64, f32)>,
    /// Which deck `App::deck` is: 0 = A, 1 = B.
    pub active: u8,
    /// Master mute.
    pub muted: bool,
}

impl Default for PlayerExt {
    fn default() -> Self {
        PlayerExt { other: Deck::default(), other_backup: None, active: 0, muted: false }
    }
}

/// Auto beat loop lengths rekordbox offers, in beats ("1/64 Beat Loop" … "512 Beat Loop").
pub const LOOP_SIZES: [f64; 16] = [1.0 / 64.0, 1.0 / 32.0, 1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0];
/// Beat jump lengths in beats, and in bars (shown as bars; a bar is four beats).
pub const JUMP_BEATS: [f64; 9] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
pub const JUMP_BARS: [f64; 6] = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// Search (scan) speed while a SEARCH button is held.
pub const SEARCH_SPEED: f64 = 8.0;

/// "1/4", "4", "512": a beat count as the loop and jump controls show it.
pub fn beats_text(b: f64) -> String {
    if b < 1.0 {
        format!("1/{}", (1.0 / b).round())
    } else {
        format!("{b}")
    }
}

/// The beat jump button's caption: "4Beats", "1/2Beat", "2Bars".
pub fn jump_label(beats: f64, bars: bool) -> String {
    if bars && beats >= 4.0 {
        let n = beats / 4.0;
        format!("{}{}", n, if n == 1.0 { "Bar" } else { "Bars" })
    } else if beats <= 1.0 {
        format!("{}Beat", beats_text(beats))
    } else {
        format!("{}Beats", beats)
    }
}

/// Current position as bars.beats ("1.1Bars" on the first downbeat).
pub fn bars_label(beats: &[Beat], t: f64) -> String {
    if beats.is_empty() {
        return String::new();
    }
    let i = beats.partition_point(|b| b.time <= t + 1e-4);
    if i == 0 {
        return "1.1Bars".into();
    }
    let b = beats[i - 1];
    let first_down = beats.iter().position(|x| x.beat == 1).unwrap_or(0);
    let bars = beats[..i].iter().skip(first_down).filter(|x| x.beat == 1).count().max(1);
    format!("{}.{}Bars", bars, b.beat)
}

/// Beats still to come before the next memory cue after `t`: the beat being played counts,
/// so it reads 16 for the whole of the sixteenth beat before the cue and 1 during the last.
/// None when there is no memory cue ahead (or no grid).
pub fn beats_to_next_memory(beats: &[Beat], memory: &[f64], t: f64) -> Option<usize> {
    if beats.is_empty() {
        return None;
    }
    let m = memory.iter().copied().filter(|m| *m > t + 1e-4).fold(f64::NAN, f64::min);
    if m.is_nan() {
        return None;
    }
    let from = beats.partition_point(|b| b.time <= t + 1e-4);
    let to = beats.partition_point(|b| b.time < m - 1e-4);
    Some(to.saturating_sub(from) + 1)
}

/// The readout above the enlarged waveform for a Beat Count Display mode: 0 = Current Position
/// (Bars), 1 = Count to the next MEMORY CUE (Bars), 2 = Count to the next MEMORY CUE (Beats).
/// The bar countdown runs like a player's: "3.4Bars" sixteen beats before the cue, "0.1Bars"
/// on the last beat.
pub fn beat_count_label(beats: &[Beat], memory: &[f64], t: f64, mode: u8) -> String {
    match mode {
        1 => match beats_to_next_memory(beats, memory, t) {
            Some(n) => format!("{}.{}Bars", (n - 1) / 4, (n - 1) % 4 + 1),
            None => "-.-Bars".into(),
        },
        2 => match beats_to_next_memory(beats, memory, t) {
            Some(n) => format!("{n}Beats"),
            None => "-Beats".into(),
        },
        _ => bars_label(beats, t),
    }
}

/// Shortest loop loop adjust leaves (seconds).
pub const MIN_LOOP: f64 = 0.005;

/// A loop with one edge moved by `delta` seconds (edge 1 = in, 2 = out). The edges never
/// cross and stay inside the track.
pub fn adjust_loop(l: (f64, f64), edge: u8, delta: f64, duration: f64) -> (f64, f64) {
    let (a, b) = l;
    match edge {
        1 => ((a + delta).clamp(0.0, (b - MIN_LOOP).max(0.0)), b),
        2 => (a, (b + delta).clamp(a + MIN_LOOP, duration.max(a + MIN_LOOP))),
        _ => l,
    }
}

/// The interleaved stereo samples between two times.
pub fn loop_samples(samples: &[f32], sr: u32, a: f64, b: f64) -> &[f32] {
    let frames = samples.len() / 2;
    let fa = ((a.max(0.0) * sr as f64).round() as usize).min(frames);
    let fb = ((b.max(0.0) * sr as f64).round() as usize).clamp(fa, frames);
    &samples[fa * 2..fb * 2]
}

/// A 16-bit PCM WAV file's bytes for interleaved samples.
pub fn wav16_bytes(sr: u32, channels: u16, samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn write_wav16(path: &std::path::Path, sr: u32, channels: u16, samples: &[f32]) -> std::io::Result<()> {
    std::fs::write(path, wav16_bytes(sr, channels, samples))
}

/// Row to load for Previous Track (-1) / Next Track (+1): the neighbour of the loaded track
/// in the list that passes `ok`, or the first / last usable row when the loaded track is not
/// in the list.
pub fn adjacent_row(list: &[i64], current: Option<i64>, dir: i32, ok: impl Fn(i64) -> bool) -> Option<usize> {
    let n = list.len() as i64;
    let mut i = match current.and_then(|c| list.iter().position(|x| *x == c)) {
        Some(i) => i as i64 + dir as i64,
        None if dir > 0 => 0,
        None => n - 1,
    };
    while i >= 0 && i < n {
        if ok(list[i as usize]) {
            return Some(i as usize);
        }
        i += dir.signum() as i64;
    }
    None
}

impl App {
    // ------------------------------------------------------------------ decks and layouts
    /// The 2 Players layout is showing.
    pub fn two_players(&self) -> bool {
        !self.prefs.browser_only && self.prefs.player_layout == 1
    }

    fn swap_decks(&mut self) {
        std::mem::swap(&mut self.deck, &mut self.px.other);
        std::mem::swap(&mut self.deck_backup, &mut self.px.other_backup);
        std::mem::swap(&mut self.player.state, &mut self.player.other);
        self.px.active ^= 1;
    }

    /// Run `f` with deck `n` (0 = A, 1 = B) in place as `self.deck` / `self.player.state`.
    pub fn with_deck<R>(&mut self, n: u8, f: impl FnOnce(&mut App) -> R) -> R {
        if n.min(1) == self.px.active {
            return f(self);
        }
        self.swap_decks();
        let r = f(self);
        self.swap_decks();
        r
    }

    /// Make deck `n` the one keyboard shortcuts and the browser act on.
    pub fn activate_deck(&mut self, n: u8) {
        if n.min(1) != self.px.active {
            self.swap_decks();
        }
    }

    /// Load a track into deck 0 (A) or 1 (B); B exists only in the 2 Players layout.
    pub fn load_track_to(&mut self, deck: u8, id: i64) {
        let d = if self.two_players() { deck.min(1) } else { self.px.active };
        self.with_deck(d, |a| a.load_track_here(id));
    }

    /// Layout: 0 = 1 Player, 1 = 2 Players, 2 = Simple Player, 3 = Full Browser.
    pub fn set_player_layout(&mut self, layout: u8) {
        if layout >= 3 {
            self.prefs.browser_only = true;
        } else {
            self.prefs.browser_only = false;
            self.prefs.player_layout = layout;
        }
        self.poll_decks();
        self.save_prefs();
    }

    /// 0 = 1 Player, 1 = 2 Players, 2 = Simple Player, 3 = Full Browser.
    pub fn player_layout(&self) -> u8 {
        if self.prefs.browser_only {
            3
        } else {
            self.prefs.player_layout.min(2)
        }
    }

    /// Once per frame: background results for the deck that is not active, one master volume
    /// and mute for both decks, and deck B put away outside the 2 Players layout.
    pub fn poll_decks(&mut self) {
        if !self.two_players() {
            self.activate_deck(0);
            self.load_target = 0;
            let mut o = self.player.other.lock();
            o.playing = false;
            o.search = 0.0;
        }
        let o = &self.px.other;
        if o.loading.is_some() || o.phrases_rx.is_some() || o.vocal_rx.is_some() {
            self.swap_decks();
            self.poll_load();
            self.poll_phrases();
            self.poll_vocal();
            self.swap_decks();
        }
        let v = {
            let mut s = self.player.state.lock();
            s.muted = self.px.muted;
            s.volume
        };
        let mut o = self.player.other.lock();
        o.volume = v;
        o.muted = self.px.muted;
    }

    pub fn toggle_mute(&mut self) {
        self.px.muted = !self.px.muted;
        self.poll_decks();
    }

    // ------------------------------------------------------------------ transport
    /// Previous Track (-1) / Next Track (+1): load the neighbouring row of the track list.
    pub fn load_adjacent(&mut self, dir: i32) {
        if self.list_dirty {
            self.rebuild_list();
        }
        let row = adjacent_row(&self.list, self.deck.track, dir, |id| self.track(id).map(|t| !t.missing).unwrap_or(false));
        if let Some(i) = row {
            let id = self.list[i];
            self.load_track_here(id);
            self.set_anchor(i);
            self.selected.clear();
            self.selected.insert(id);
            self.scroll_to_selected = true;
        }
    }

    /// Previous Track, as on a player: back to the start of the track when it is more than a
    /// second in, otherwise the previous row of the track list.
    pub fn previous_track(&mut self) {
        if self.deck.audio.is_some() && self.pos() > 1.0 {
            self.seek(0.0);
            if !self.playing() {
                self.deck.cue_point = 0.0;
            }
        } else {
            self.load_adjacent(-1);
        }
    }

    /// Hold a SEARCH button: -1 backward, +1 forward, 0 released.
    pub fn search(&mut self, dir: i32) {
        let mut s = self.player.state.lock();
        s.search = if s.audio.is_some() { dir.signum() as f64 * SEARCH_SPEED } else { 0.0 };
    }

    pub fn searching(&self) -> i32 {
        let s = self.player.state.lock().search;
        if s > 0.0 {
            1
        } else if s < 0.0 {
            -1
        } else {
            0
        }
    }

    /// Empty the active deck (click on the artwork).
    pub fn unload_track(&mut self) {
        {
            let mut s = self.player.state.lock();
            s.playing = false;
            s.audio = None;
            s.pos = 0.0;
            s.loop_range = None;
            s.metronome = None;
            s.stems = None;
            s.stem_on = [true; 4];
            s.search = 0.0;
        }
        let (ctrl_tab, panel_tab, metronome) = (self.deck.ctrl_tab, self.deck.panel_tab, self.deck.metronome);
        self.deck = Deck { ctrl_tab, panel_tab, metronome, ..Deck::default() };
        self.deck_backup = None;
    }

    // ------------------------------------------------------------------ cues and loops at a position
    /// The loop in play, or the last one set.
    pub fn defined_loop(&self) -> Option<(f64, f64)> {
        self.loop_active().or(self.deck.last_loop).filter(|(a, b)| b > a)
    }

    /// "Set Cue(In) Point": the cue point, which is also where a loop set with "Set Out
    /// Point" begins. A stopped deck moves there.
    pub fn set_cue_at(&mut self, t: f64) {
        if self.deck.audio.is_none() {
            return;
        }
        let q = self.quantized(t.clamp(0.0, self.deck.duration.max(0.0)));
        self.deck.cue_point = q;
        self.deck.loop_in = Some(q);
        if !self.playing() {
            self.seek(q);
        }
    }

    /// "Set Out Point": loop from the in point (the cue point if none was set) to `t`.
    pub fn set_out_at(&mut self, t: f64) {
        if self.deck.audio.is_none() {
            return;
        }
        let a = self.deck.loop_in.unwrap_or(self.deck.cue_point);
        let b = self.quantized(t.clamp(0.0, self.deck.duration.max(0.0)));
        if b > a + MIN_LOOP {
            self.player.state.lock().loop_range = Some((a, b));
            self.deck.last_loop = Some((a, b));
            self.deck.loop_in = Some(a);
        } else {
            self.toast_err("The out point must be after the cue (in) point.");
        }
    }

    /// Store a hot cue (or, with `end`, a hot loop) in a slot, replacing what was there.
    pub fn set_hot(&mut self, slot: i32, start: f64, end: Option<f64>) {
        let Some(tid) = self.deck.track else { return };
        if self.deck.audio.is_none() {
            return;
        }
        self.snapshot_cues();
        let old: Vec<i64> = self.deck.cues.iter().filter(|c| c.kind == CueKind::Hot && c.slot == slot).map(|c| c.id).collect();
        for id in old {
            let _ = self.db.delete_cue(id);
            self.deck.cues.retain(|c| c.id != id);
        }
        let mut c = Cue { id: 0, track_id: tid, kind: CueKind::Hot, slot, start, end, color: hotcue_default_color(slot, end.is_some()), comment: String::new(), active_loop: false };
        if let Ok(id) = self.db.save_cue(&c) {
            c.id = id;
            self.deck.cues.push(c);
        }
        self.refresh_cue_counts(tid);
    }

    /// "Set Hot Cue A…": at a position of the waveform (quantized when Q is on).
    pub fn set_hot_cue_at(&mut self, slot: i32, t: f64) {
        let q = self.quantized(t.clamp(0.0, self.deck.duration.max(0.0)));
        self.set_hot(slot, q, None);
    }

    /// "Set Hot Loop A…": the loop in play (or last set) as a hot cue.
    pub fn set_hot_loop(&mut self, slot: i32) {
        if let Some((a, b)) = self.defined_loop() {
            self.set_hot(slot, a, Some(b));
        }
    }

    /// "Set Memory Loop": the loop in play (or last set) as a memory loop.
    pub fn set_memory_loop(&mut self) {
        let Some(tid) = self.deck.track else { return };
        let Some((a, b)) = self.defined_loop() else { return };
        if self.deck.cues.iter().any(|c| c.kind == CueKind::Memory && (c.start - a).abs() < 0.001 && c.end.map(|e| (e - b).abs() < 0.001).unwrap_or(false)) {
            return;
        }
        if self.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).count() >= 10 {
            self.toast_err("This track already has 10 memory cues and loops, the most it can hold. Delete one to add another.");
            return;
        }
        let mut c = Cue { id: 0, track_id: tid, kind: CueKind::Memory, slot: -1, start: a, end: Some(b), color: 0, comment: String::new(), active_loop: false };
        self.snapshot_cues();
        if let Ok(id) = self.db.save_cue(&c) {
            c.id = id;
            self.deck.cues.push(c);
            self.refresh_cue_counts(tid);
        }
    }

    /// "Export Loop As Wav": the loop's audio as a 16-bit WAV file the user chooses.
    pub fn export_loop_wav(&mut self) {
        let (Some((a, b)), Some(audio)) = (self.defined_loop(), self.deck.audio.clone()) else {
            self.toast_err("Set a loop first.");
            return;
        };
        let t = self.deck.track.and_then(|id| self.track(id)).cloned();
        let title: String = t.as_ref().map(|t| t.display_title()).unwrap_or_else(|| "Loop".into()).chars().map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
        let Some(path) = rfd::FileDialog::new().set_title("Export Loop As Wav").add_filter("WAV File", &["wav"]).set_file_name(format!("{title} (Loop).wav")).save_file() else { return };
        // Never write over the track itself.
        if t.map(|t| std::path::Path::new(&t.path) == path).unwrap_or(false) {
            self.toast_err("Choose another name: that is the track's own file.");
            return;
        }
        match write_wav16(&path, audio.sr, 2, loop_samples(&audio.samples, audio.sr, a, b)) {
            Ok(()) => self.toast(format!("Loop exported: {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())),
            Err(e) => self.toast_err(format!("The loop could not be exported: {e}")),
        }
    }

    // ------------------------------------------------------------------ loop buttons
    /// Exit/Reloop: leave the loop in play, or go back to the last one.
    pub fn exit_reloop(&mut self) {
        self.deck.x.loop_adjust = 0;
        if self.loop_active().is_some() {
            self.loop_exit();
        } else {
            self.reloop();
        }
    }

    /// Loop In: sets the in point; during a loop it switches in-point adjust on and off.
    pub fn loop_in_pressed(&mut self) {
        if self.loop_active().is_some() {
            self.deck.x.loop_adjust = if self.deck.x.loop_adjust == 1 { 0 } else { 1 };
        } else {
            self.loop_in();
        }
    }

    /// Loop Out: closes the loop; during a loop it switches out-point adjust on and off.
    pub fn loop_out_pressed(&mut self) {
        if self.loop_active().is_some() {
            self.deck.x.loop_adjust = if self.deck.x.loop_adjust == 2 { 0 } else { 2 };
        } else {
            self.loop_out();
        }
    }

    /// Move the loop edge being adjusted to `t` (quantized when Q is on).
    pub fn loop_adjust_to(&mut self, t: f64) {
        let edge = self.deck.x.loop_adjust;
        let Some(l) = self.loop_active() else { return };
        let cur = if edge == 1 { l.0 } else { l.1 };
        let n = adjust_loop(l, edge, self.quantized(t) - cur, self.deck.duration);
        self.player.state.lock().loop_range = Some(n);
        self.deck.last_loop = Some(n);
    }

    /// ←/→ in loop adjust: one quantize step with Q on (5 ms without), 1 ms when `fine`.
    pub fn loop_adjust_step(&mut self, dir: i32, fine: bool) {
        let edge = self.deck.x.loop_adjust;
        let Some(l) = self.loop_active() else { return };
        let step = if fine {
            0.001
        } else if self.prefs.quantize && !self.deck.beats.is_empty() {
            self.beat_len() / self.prefs.quantize_div.max(1) as f64
        } else {
            0.005
        };
        let n = adjust_loop(l, edge, dir as f64 * step, self.deck.duration);
        self.player.state.lock().loop_range = Some(n);
        self.deck.last_loop = Some(n);
    }

    /// Start (or with the same length, stop) an auto beat loop of `beats`, remembering the size.
    pub fn pick_auto_loop(&mut self, beats: f64) {
        self.prefs.loop_beats = beats;
        self.deck.x.loop_adjust = 0;
        self.auto_loop(beats);
        self.save_prefs();
    }

    // ------------------------------------------------------------------ grid
    /// "Set a beat to the current position": slide the grid (whole track, or from the current
    /// position on) so that its nearest beat lands on the playhead.
    pub fn grid_center(&mut self) {
        let p = self.pos();
        match nearest_beat(&self.deck.beats, p) {
            Some(b) if (p - b.time).abs() > 1e-6 => self.grid_shift(p - b.time),
            Some(_) => {}
            // No grid: let grid_shift explain.
            None => self.grid_shift(0.0),
        }
    }

    pub fn set_metronome_volume(&mut self, step: u8) {
        self.prefs.metronome_vol = step.clamp(1, 3);
        self.update_metronome();
        self.save_prefs();
    }

    /// Text the GRID panel's BPM field accepted with Enter.
    pub fn grid_enter_bpm(&mut self, text: &str) {
        match text.trim().replace(',', ".").parse::<f64>() {
            Ok(v) => self.grid_set_bpm(v),
            Err(_) => self.toast_err("BPM value should be between 40 and 499."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize) -> Vec<Beat> {
        // 120 BPM from 1.0 s; the first beat is a downbeat.
        (0..n).map(|i| Beat { time: 1.0 + i as f64 * 0.5, bpm: 120.0, beat: (i % 4) as u8 + 1, index: i }).collect()
    }

    #[test]
    fn wav_header_and_samples() {
        let b = wav16_bytes(44100, 2, &[0.0, 1.0, -1.0, 0.5, 2.0, -2.0]);
        assert_eq!(b.len(), 44 + 12);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 12);
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes(b[20..22].try_into().unwrap()), 1, "PCM");
        assert_eq!(u16::from_le_bytes(b[22..24].try_into().unwrap()), 2, "channels");
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 44100);
        assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 44100 * 4, "byte rate");
        assert_eq!(u16::from_le_bytes(b[32..34].try_into().unwrap()), 4, "block align");
        assert_eq!(u16::from_le_bytes(b[34..36].try_into().unwrap()), 16, "bits");
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 12);
        let s: Vec<i16> = b[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        // Full scale, half scale, and clipping of out-of-range input.
        assert_eq!(s, vec![0, 32767, -32767, 16384, 32767, -32767]);
    }

    #[test]
    fn wav_file_decodes_back_to_the_loop() {
        // One second of a ramp at 8 kHz; export 0.25–0.75 s and read it back with the decoder.
        let sr = 8000u32;
        let samples: Vec<f32> = (0..sr as usize).flat_map(|i| { let v = i as f32 / sr as f32 - 0.5; [v, -v] }).collect();
        let part = loop_samples(&samples, sr, 0.25, 0.75);
        assert_eq!(part.len(), 4000 * 2);
        assert_eq!(part[0], samples[2000 * 2]);
        let dir = std::env::temp_dir().join(format!("recordbox-wav-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loop.wav");
        write_wav16(&path, sr, 2, part).unwrap();
        let d = crate::audio::decode_file(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(d.sr, sr);
        assert_eq!(d.frames(), 4000);
        assert!((d.samples[0] - part[0]).abs() < 1e-3 && (d.samples[1] - part[1]).abs() < 1e-3);
        assert!((d.samples[7998] - part[7998]).abs() < 1e-3);
        // Ranges outside the audio are clamped.
        assert!(loop_samples(&samples, sr, 2.0, 3.0).is_empty());
        assert_eq!(loop_samples(&samples, sr, -1.0, 9.0).len(), samples.len());
    }

    #[test]
    fn beat_count_display() {
        let b = grid(64);
        // Current Position (Bars).
        assert_eq!(beat_count_label(&b, &[], 0.0, 0), "1.1Bars");
        assert_eq!(beat_count_label(&b, &[], 1.0, 0), "1.1Bars");
        assert_eq!(beat_count_label(&b, &[], 1.6, 0), "1.2Bars");
        assert_eq!(beat_count_label(&b, &[], 3.0, 0), "2.1Bars");
        // A memory cue on beat index 16 (9.0 s), another on 32 (17.0 s).
        let mem = [9.0, 17.0];
        assert_eq!(beats_to_next_memory(&b, &mem, 1.0), Some(16));
        assert_eq!(beats_to_next_memory(&b, &mem, 1.2), Some(16));
        assert_eq!(beats_to_next_memory(&b, &mem, 1.5), Some(15));
        assert_eq!(beats_to_next_memory(&b, &mem, 8.7), Some(1));
        // On the cue itself the count turns to the one after it.
        assert_eq!(beats_to_next_memory(&b, &mem, 9.0), Some(16));
        assert_eq!(beats_to_next_memory(&b, &mem, 17.0), None);
        assert_eq!(beat_count_label(&b, &mem, 1.0, 1), "3.4Bars");
        assert_eq!(beat_count_label(&b, &mem, 7.0, 1), "0.4Bars");
        assert_eq!(beat_count_label(&b, &mem, 8.7, 1), "0.1Bars");
        assert_eq!(beat_count_label(&b, &mem, 1.0, 2), "16Beats");
        assert_eq!(beat_count_label(&b, &mem, 8.7, 2), "1Beats");
        assert_eq!(beat_count_label(&b, &mem, 20.0, 1), "-.-Bars");
        assert_eq!(beat_count_label(&b, &mem, 20.0, 2), "-Beats");
        assert_eq!(beat_count_label(&[], &mem, 1.0, 2), "-Beats");
    }

    #[test]
    fn loop_adjust_moves_one_edge_and_keeps_the_loop_valid() {
        let l = (10.0, 12.0);
        assert_eq!(adjust_loop(l, 1, 0.5, 300.0), (10.5, 12.0));
        assert_eq!(adjust_loop(l, 1, -0.5, 300.0), (9.5, 12.0));
        assert_eq!(adjust_loop(l, 2, 0.5, 300.0), (10.0, 12.5));
        assert_eq!(adjust_loop(l, 2, -0.5, 300.0), (10.0, 11.5));
        // The in point stops short of the out point and at the start of the track.
        assert_eq!(adjust_loop(l, 1, 5.0, 300.0), (12.0 - MIN_LOOP, 12.0));
        assert_eq!(adjust_loop(l, 1, -50.0, 300.0), (0.0, 12.0));
        // The out point stops short of the in point and at the end of the track.
        assert_eq!(adjust_loop(l, 2, -5.0, 300.0), (10.0, 10.0 + MIN_LOOP));
        assert_eq!(adjust_loop(l, 2, 500.0, 300.0), (10.0, 300.0));
        assert_eq!(adjust_loop(l, 0, 1.0, 300.0), l);
    }

    #[test]
    fn previous_and_next_track_rows() {
        let list = [10, 11, 12, 13];
        let all = |_| true;
        assert_eq!(adjacent_row(&list, Some(11), 1, all), Some(2));
        assert_eq!(adjacent_row(&list, Some(11), -1, all), Some(0));
        assert_eq!(adjacent_row(&list, Some(13), 1, all), None);
        assert_eq!(adjacent_row(&list, Some(10), -1, all), None);
        // Nothing loaded (or a track from another list): first / last row.
        assert_eq!(adjacent_row(&list, None, 1, all), Some(0));
        assert_eq!(adjacent_row(&list, Some(99), -1, all), Some(3));
        // Missing files are skipped.
        assert_eq!(adjacent_row(&list, Some(10), 1, |id| id != 11), Some(2));
        assert_eq!(adjacent_row(&[], None, 1, all), None);
    }

    #[test]
    fn captions() {
        assert_eq!(jump_label(4.0, false), "4Beats");
        assert_eq!(jump_label(0.5, false), "1/2Beat");
        assert_eq!(jump_label(1.0, false), "1Beat");
        assert_eq!(jump_label(4.0, true), "1Bar");
        assert_eq!(jump_label(32.0, true), "8Bars");
        assert_eq!(beats_text(1.0 / 64.0), "1/64");
        assert_eq!(beats_text(512.0), "512");
    }
}

// ---------------------------------------------------------------------- screenshot checks

/// The PREPDECK_SHOT_PLAYER switches (comma separated, with --screenshot), for looking at
/// player states: `2p` 2 Players with a second track on deck B · `simple` Simple Player ·
/// `ma` manual loop buttons · `loop` a 4-beat loop · `adjust` loop-out adjust · `count1` /
/// `count2` Beat Count Display modes · `metro` metronome on at volume 3 · `bars` beat jump
/// in bars · `menu` the waveform's right-click menu. Combine with
/// PREPDECK_SHOT_VIEW=grid for the GRID panel.
fn shot_flags() -> Vec<String> {
    std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default()
}

impl App {
    /// Wait for the active deck's audio (screenshot runs only).
    fn shot_wait_load(&mut self) {
        if let Some(rx) = self.deck.loading.take() {
            if let Ok(msg) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
                let (tx2, rx2) = crossbeam_channel::bounded(1);
                let _ = tx2.send(msg);
                self.deck.loading = Some(rx2);
                self.poll_load();
            }
        }
    }

    /// Put the player into the states PREPDECK_SHOT_PLAYER names (after the track is loaded).
    pub fn shot_player_setup(&mut self) {
        let flags = shot_flags();
        let has = |f: &str| flags.iter().any(|x| x == f);
        if flags.is_empty() {
            return;
        }
        if has("2p") {
            self.prefs.browser_only = false;
            self.prefs.player_layout = 1;
            let a = self.deck.track;
            let second = self.tracks.iter().filter(|t| t.analyzed == 1 && !t.missing && Some(t.id) != a && t.duration > 60.0).map(|t| t.id).nth(3);
            if let Some(id) = second {
                self.load_track_to(1, id);
                self.with_deck(1, |app| {
                    app.shot_wait_load();
                    let beats = app.deck.beats.clone();
                    if let Some(b) = beats.get(64) {
                        app.seek(b.time);
                        app.hot_cue_down(0);
                        app.deck.cue_point = b.time;
                        app.memory_set();
                    }
                    if let Some(b) = beats.get(96) {
                        app.seek(b.time);
                        app.hot_cue_down(2);
                        app.prefs.loop_auto = true;
                        app.auto_loop(4.0);
                    }
                });
            }
        }
        if has("simple") {
            self.prefs.browser_only = false;
            self.prefs.player_layout = 2;
        }
        if has("1p") {
            self.prefs.browser_only = false;
            self.prefs.player_layout = 0;
        }
        if has("ma") || has("adjust") {
            self.prefs.loop_auto = false;
        }
        if has("loop") || has("adjust") {
            let p = self.pos();
            self.seek(p + 4.0);
            self.auto_loop(4.0);
            let p = self.pos();
            self.seek(p + 0.6);
        }
        if has("adjust") {
            self.deck.x.loop_adjust = 2;
        }
        if has("count1") {
            self.prefs.beat_count_mode = 1;
        }
        if has("count2") {
            self.prefs.beat_count_mode = 2;
        }
        if has("metro") {
            self.deck.metronome = true;
            self.prefs.metronome_vol = 3;
            self.update_metronome();
        }
        if has("bars") {
            self.prefs.beat_jump = 8.0;
            self.prefs.beat_jump_bars = true;
        }
        if has("clickmode") {
            self.prefs.wave_click_play_cue = true;
        }
    }

    /// Pointer events for PREPDECK_SHOT_PLAYER=menu (right-click on the enlarged waveform,
    /// 150 pt right of the playhead) and =padhover (pointer over hot cue pad A's corner).
    pub fn shot_player_events(&self, frame: u32, raw: &mut egui::RawInput) {
        let flags = shot_flags();
        if flags.iter().any(|x| x == "menu") {
            let Some((r, _, _)) = self.deck.detail_map else { return };
            let at = egui::pos2(r.center().x + 150.0, r.center().y - 20.0);
            let click = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
            match frame {
                80 => raw.events.push(egui::Event::PointerMoved(at)),
                82 => raw.events.push(click(true)),
                83 => raw.events.push(click(false)),
                _ => {}
            }
        }
    }
}
