//! recordbox's own feel layer (rekordbox has none of this): how controls answer the pointer
//! and how the picture moves between states. Everything here is presentation on top of state
//! that changes exactly when it did before: a click still acts on its own frame, a drag still
//! sets the value it always set; only what is drawn eases (`motion.rs`).
//!
//! With Preferences > Advanced > Others > recordbox > Interface motion set to Off, every helper
//! here returns what the call site drew before, so the app looks and behaves as it did.
//!
//! - Hover, press and release faces (`hl`, `face`, `hover`, `press`), switch / radio / check
//!   transitions (`on_amt`).
//! - Knobs and faders: Shift for fine adjustment (a tenth of the travel), the drawn value
//!   gliding to a value set by a click or a double-click reset (`shown`), value read-outs that
//!   fade in while a control is moved (`readout`), grab cursors.
//! - Lists: trackpad scrolling passed through 1:1 (egui smooths large trackpad steps as if they
//!   were wheel notches; the system already gives momentum), smooth scrolling to the selected
//!   row (`scroll_to`, `scroll_anim`), an eased row hover and selection (`row_bg`), live guides
//!   for column resizing and reordering.
//! - Drag and drop: eased drop-target highlights (`drop_glow`), a drag label with a shadow, a
//!   snap-back when a drop is refused, an insertion line that slides between rows.
//! - Panels, pages, layouts and modes: the area that changed fades in from its background
//!   (`veil`); nothing waits for it and no layout moves.
//! - Waveforms: the drawn play position advances with the frame clock between audio
//!   callbacks (`shown_pos`), so a playing deck scrolls by the same amount every frame.

use crate::app::App;
use crate::motion::{self, Kind};
use crate::theme::{tc, Tc};
use eframe::egui;
use egui::{pos2, vec2, Color32, CursorIcon, Id, Pos2, Rect, Response, Stroke, Ui};
use std::cell::RefCell;

// ---- Colours of what this layer adds (tokens with a Dark and a Light value).
/// Read-out bubble behind a value.
pub const READOUT_BG: Tc = tc(0x242424, 0xFFFFFF);
/// Its outline.
pub const READOUT_EDGE: Tc = tc(0x4A4A4A, 0xA0A0A0);
/// Its text.
pub const READOUT_TX: Tc = tc(0xFFFFFF, 0x000000);
/// A track list row under the pointer (an overlay over the row's own colour).
pub const ROW_HOVER: Tc = tc(0x232323, 0xE6E6E6);
/// Drop-target wash (the outline is the accent blue).
pub const DROP_WASH: Tc = tc(0x1473EB, 0x1473EB);
/// Shadow under a drag label.
pub const SHADOW: Tc = tc(0x000000, 0x000000);
/// Column resize / reorder guide.
pub const GUIDE: Tc = tc(0x1473EB, 0x1473EB);

// ------------------------------------------------------------------ hover, press, release

/// How much `resp` is hovered (0..1, eased: quick in, soft out).
pub fn hover(resp: &Response) -> f32 {
    hover_if(resp, true)
}

pub fn hover_if(resp: &Response, cond: bool) -> f32 {
    mark(resp.id);
    let h = (resp.hovered() && cond) as u8 as f32;
    motion::toward(resp.id.with("feel_h"), h, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade)
}

/// How much `resp` is pressed: 1 on the frame the button goes down, easing back after release.
pub fn press(resp: &Response) -> f32 {
    mark(resp.id);
    let d = resp.is_pointer_button_down_on() as u8 as f32;
    motion::toward(resp.id.with("feel_p"), d, 0.0, motion::RELEASE, Kind::Fade)
}

/// How much `resp` is "hot" (hovered or held), eased; for handles that light up.
pub fn hot(resp: &Response) -> f32 {
    mark(resp.id);
    let h = (resp.hovered() || resp.dragged()) as u8 as f32;
    motion::toward(resp.id.with("feel_hot"), h, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade)
}

/// `press` that reacts only when `cond` holds.
pub fn press_if(resp: &Response, cond: bool) -> f32 {
    mark(resp.id);
    let d = (resp.is_pointer_button_down_on() && cond) as u8 as f32;
    motion::toward(resp.id.with("feel_p"), d, 0.0, motion::RELEASE, Kind::Fade)
}

/// The colour of a hover-only fill (None when nothing is to be drawn): `c` while hovered,
/// fading in and out.
pub fn hover_fill(resp: &Response, c: impl Into<Color32>) -> Option<Color32> {
    hover_fill_if(resp, true, c)
}

pub fn hover_fill_if(resp: &Response, cond: bool, c: impl Into<Color32>) -> Option<Color32> {
    let c = c.into();
    mark(resp.id);
    if !motion::on() {
        return (resp.hovered() && cond).then_some(c);
    }
    let h = hover_if(resp, cond);
    (h > 0.0).then(|| motion::fade(c, h))
}

/// `base`, or `hot` while hovered, eased.
pub fn hl(resp: &Response, base: impl Into<Color32>, hot: impl Into<Color32>) -> Color32 {
    hl_if(resp, true, base, hot)
}

/// `base`, or `hot` while hovered or dragged (a handle), eased.
pub fn hot_col(resp: &Response, base: impl Into<Color32>, hot_c: impl Into<Color32>) -> Color32 {
    let (base, hot_c) = (base.into(), hot_c.into());
    mark(resp.id);
    if !motion::on() {
        return if resp.hovered() || resp.dragged() { hot_c } else { base };
    }
    motion::mix(base, hot_c, hot(resp))
}

/// `hl` that reacts only when `cond` holds (a disabled control does not light up).
pub fn hl_if(resp: &Response, cond: bool, base: impl Into<Color32>, hot: impl Into<Color32>) -> Color32 {
    let (base, hot) = (base.into(), hot.into());
    mark(resp.id);
    let h = resp.hovered() && cond;
    if !motion::on() {
        return if h { hot } else { base };
    }
    motion::mix(base, hot, hover_if(resp, cond))
}

/// A push face: `down` from the press frame, `hot` while hovered, `base` at rest; the press
/// shows at once and lets go softly.
pub fn face(resp: &Response, base: Color32, hot: Color32, down: Color32) -> Color32 {
    face_if(resp, true, base, hot, down)
}

pub fn face_if(resp: &Response, cond: bool, base: Color32, hot: Color32, down: Color32) -> Color32 {
    let (h, d) = (resp.hovered() && cond, resp.is_pointer_button_down_on() && cond);
    mark(resp.id);
    if !motion::on() {
        return if d { down } else if h { hot } else { base };
    }
    let p = motion::toward(resp.id.with("feel_p"), d as u8 as f32, 0.0, motion::RELEASE, Kind::Fade);
    motion::mix(motion::mix(base, hot, hover_if(resp, cond)), down, p)
}

/// A fill drawn over a control only while it is hovered (`hover_c`) or pressed (`down_c`):
/// the hover fades in and out, the press shows on its own frame and lets go softly. None when
/// nothing is to be drawn. Off: `down_c` while pressed, else `hover_c` while hovered.
pub fn overlay(resp: &Response, cond: bool, hover_c: impl Into<Color32>, down_c: impl Into<Color32>) -> Option<Color32> {
    let (hc, dc) = (hover_c.into(), down_c.into());
    mark(resp.id);
    let (h, d) = (resp.hovered() && cond, resp.is_pointer_button_down_on() && cond);
    if !motion::on() {
        return if d { Some(dc) } else if h { Some(hc) } else { None };
    }
    let p = motion::toward(resp.id.with("feel_p"), d as u8 as f32, 0.0, motion::RELEASE, Kind::Fade);
    let c = motion::mix(motion::fade(hc, hover_if(resp, cond)), dc, p);
    (c.a() > 0).then_some(c)
}

/// How much the pointer is over `r` (0..1, eased like a hover), for places that test the
/// pointer against a rectangle rather than a response. `id` keys it.
pub fn hover_rect(ui: &Ui, id: Id, r: Rect, cond: bool) -> f32 {
    let on = cond && ui.rect_contains_pointer(r);
    motion::toward(id.with("feel_hr"), on as u8 as f32, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade)
}

/// Content that is swapped while the pointer is over `rect` (a slot that shows its controls
/// instead of its waveform): the new content comes in from `bg` with the hover's timing
/// instead of in one frame. Painted over the area at the end of the frame.
pub fn swap_veil(id: Id, shown: bool, rect: Rect, bg: Color32) {
    let k = id.with("feel_swap");
    let t = motion::toward(k, shown as u8 as f32, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade);
    // The veil is strongest right after the swap and gone when the amount has arrived.
    let target = shown as u8 as f32;
    let left = (target - t).abs();
    if left > 0.0 && motion::on() && rect.is_positive() {
        SWAPS.with(|s| {
            let mut s = s.borrow_mut();
            if s.len() < 64 {
                s.push((rect, motion::fade(bg, 0.7 * left)));
            }
        });
    }
}

thread_local! {
    static SWAPS: RefCell<Vec<(Rect, Color32)>> = const { RefCell::new(Vec::new()) };
}

fn paint_swaps(ctx: &egui::Context) {
    SWAPS.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_empty() {
            return;
        }
        let lp = ctx.layer_painter(egui::LayerId::background());
        for (r, c) in s.drain(..) {
            lp.with_clip_rect(r).rect_filled(r, 0.0, c);
        }
    });
}

/// How far a row, page or tab is into its selected colour (0..1): `SELECT` in, `HOVER_OUT` out.
/// Every list, tree and Preferences page uses this one timing.
pub fn sel_amt(id: Id, on: bool) -> f32 {
    motion::toward(id.with("feel_sel"), on as u8 as f32, motion::SELECT, motion::HOVER_OUT, Kind::Fade)
}

/// `base`, or `sel` while selected, eased with `sel_amt`. Off: exactly one of the two.
pub fn sel_col(id: Id, on: bool, base: impl Into<Color32>, sel: impl Into<Color32>) -> Color32 {
    let (base, sel) = (base.into(), sel.into());
    if !motion::on() {
        return if on { sel } else { base };
    }
    motion::mix(base, sel, sel_amt(id, on))
}

thread_local! {
    static FRAME_LIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A themed colour as the frame's appearance draws it, also inside `theme::dark_scope` (a
/// selected row is drawn as in Dark; the colour it fades from is the row's own).
pub fn outside(c: Tc) -> Color32 {
    c.pick(FRAME_LIGHT.with(|l| l.get()))
}

/// A two-state amount for a switch, radio or check box (0 off, 1 on), eased.
pub fn on_amt(id: Id, on: bool) -> f32 {
    motion::toward(id.with("feel_on"), on as u8 as f32, motion::FEEDBACK, motion::FEEDBACK, Kind::Move)
}

/// A new status-line message fades in (160 ms) instead of replacing the last one in a frame.
pub fn toast_in(text: &str) -> f32 {
    thread_local! { static LAST: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
    let h = Id::new(text).value();
    let id = Id::new("feel_toast");
    if LAST.with(|l| l.replace(h)) != h && motion::on() {
        motion::set(id, 0.0);
    }
    motion::toward(id, 1.0, motion::HOVER_OUT, 0.0, Kind::Fade)
}

/// How much of something that shows only while `on` (signs under the pointer) is drawn:
/// fades in 110 ms, out 160 ms; exactly 0 or 1 at rest.
pub fn appear(id: Id, on: bool) -> f32 {
    motion::toward(id.with("feel_ap"), on as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade)
}

/// How far a control is into its lit state (0..1), eased (a colour change, not a move).
pub fn state_amt(id: Id, on: bool) -> f32 {
    motion::toward(id.with("feel_st"), on as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade)
}

/// A colour change between two states of a control (lit / unlit), eased.
pub fn state_col(id: Id, on: bool, off_col: Color32, on_col: Color32) -> Color32 {
    if !motion::on() {
        return if on { on_col } else { off_col };
    }
    let t = motion::toward(id.with("feel_st"), on as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade);
    motion::mix(off_col, on_col, t)
}

/// The round CUE (`cue`) or PLAY face of the players, lit / loaded as before, with eased
/// hover and a press that darkens the face on its own frame.
pub fn round_face(p: &egui::Painter, c: Pos2, dia: f32, lit: bool, loaded: bool, resp: &Response, cue: bool) {
    mark(resp.id);
    let (h, d) = if motion::on() { (hover(resp), press(resp)) } else { (resp.hovered() as u8 as f32, 0.0) };
    if cue {
        crate::ui_player::cue_button_face_t(p, c, dia, lit, loaded, h, d);
    } else {
        crate::ui_player::play_button_face_t(p, c, dia, lit, h, d);
    }
}

/// The kit's dark push button (`ui_kit::paint_button`) with eased hover and press.
pub fn button_face(p: &egui::Painter, r: Rect, resp: &Response) {
    mark(resp.id);
    if motion::on() {
        crate::ui_kit::paint_button_t(p, r, hover(resp), press(resp));
    } else {
        crate::ui_kit::paint_button(p, r, resp.hovered(), resp.is_pointer_button_down_on());
    }
}

// ------------------------------------------------------------------ cursors

/// A draggable control: an open hand over it, a closed one while it is held.
pub fn grab_cursor(resp: &Response) {
    if !motion::extras() {
        return;
    }
    if resp.dragged() || resp.is_pointer_button_down_on() {
        resp.ctx.set_cursor_icon(CursorIcon::Grabbing);
    } else if resp.hovered() {
        resp.ctx.set_cursor_icon(CursorIcon::Grab);
    }
}

/// A splitter or column edge (only where the call site did not set one already).
pub fn resize_cursor(resp: &Response, horizontal: bool) {
    if motion::extras() && (resp.hovered() || resp.dragged()) {
        resp.ctx.set_cursor_icon(if horizontal { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
    }
}

// ------------------------------------------------------------------ knobs and faders

/// The vertical drag of a knob this frame, a tenth of it while Shift is held (fine
/// adjustment; Shift has no other meaning on a knob).
pub fn knob_dy(ui: &Ui, resp: &Response) -> f32 {
    let dy = resp.drag_delta().y;
    if motion::extras() && ui.input(|i| i.modifiers.shift) {
        dy * 0.1
    } else {
        dy
    }
}

/// A fader's value under Shift: relative to where the drag started, at a tenth of the
/// pointer's travel, instead of the pointer's absolute place. `travel` is the length of the
/// fader in points; `along` is the pointer's coordinate along it (y up = larger value: pass
/// -y). Returns None when the plain absolute rule should be used (no Shift, or Off).
pub fn fader_fine(ui: &Ui, resp: &Response, value: f32, travel: f32, along: f32) -> Option<f32> {
    if !motion::extras() {
        return None;
    }
    let key = resp.id.with("feel_fine");
    let shift = ui.input(|i| i.modifiers.shift);
    let held = resp.is_pointer_button_down_on() || resp.dragged();
    if !held {
        ui.data_mut(|d| d.remove::<(f32, f32)>(key));
        return None;
    }
    if !shift {
        ui.data_mut(|d| d.remove::<(f32, f32)>(key));
        return None;
    }
    let (v0, a0) = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || (value, along)));
    Some((v0 + (along - a0) / travel.max(1.0) * 0.1).clamp(0.0, 1.0))
}

/// The value a control is drawn at: `value` itself while it is dragged or set by MIDI (so
/// the handle never lags the pointer), gliding to it when `jumped` (a click on the track, a
/// double-click reset, a label click).
pub fn shown(id: Id, value: f32, jumped: bool) -> f32 {
    let jump = if jumped { 0.0 } else { f32::INFINITY };
    motion::glide(id.with("feel_v"), value, jump, motion::GLIDE, Kind::Move)
}

/// The value a knob is drawn at: exact while dragged (and for small external steps such as
/// a MIDI knob's), gliding for a double-click reset or a jump from elsewhere (a label click
/// that kills the band).
pub fn knob_shown(resp: &Response, v: f32) -> f32 {
    let jump = if resp.double_clicked() { 0.0 } else if resp.dragged() { f32::INFINITY } else { 0.12 };
    motion::glide(resp.id.with("feel_v"), v, jump, motion::GLIDE, Kind::Move)
}

/// The value a fader is drawn at: gliding to where a press on its track put it and to a
/// double-click reset, exact while it is dragged.
pub fn fader_shown(ui: &Ui, resp: &Response, v: f32) -> f32 {
    let jump = if press_started(ui, resp) || resp.double_clicked() { 0.0 } else if resp.dragged() { f32::INFINITY } else { 0.12 };
    motion::glide(resp.id.with("feel_v"), v, jump, motion::GLIDE, Kind::Move)
}

thread_local! {
    static KNOB_FMT: std::cell::Cell<Option<fn(f32) -> String>> = const { std::cell::Cell::new(None) };
}

/// How the next knob drawn shows its value in its read-out (default: percent, or the offset
/// from the centre for a knob that resets to the centre).
pub fn knob_fmt(f: fn(f32) -> String) {
    KNOB_FMT.with(|c| c.set(Some(f)));
}

/// The read-out of a knob at `c` (its centre), while it is dragged.
pub fn knob_readout(ui: &Ui, resp: &Response, c: Pos2, v: f32, reset: f32) {
    let f = KNOB_FMT.with(|k| k.take());
    if !motion::extras() {
        return;
    }
    let active = resp.dragged() || resp.is_pointer_button_down_on();
    readout(ui, resp.id, pos2(c.x, c.y - 14.0), active, || match f {
        Some(f) => f(v),
        None if (reset - 0.5).abs() < 1e-6 => {
            let o = ((v - 0.5) * 200.0).round();
            if o == 0.0 { "0".to_string() } else { format!("{o:+.0}") }
        }
        None => pct(v),
    });
}

/// A channel fader's cursor and read-out (its level in dB) at the handle `h`.
pub fn fader_feel(ui: &Ui, resp: &Response, h: Pos2, v: f32, curve: u8) {
    grab_cursor(resp);
    readout(ui, resp.id, pos2(h.x + 34.0, h.y + 7.0), resp.dragged() || resp.is_pointer_button_down_on(), || {
        let g = crate::mixer::channel_fader_gain(v, curve);
        if g <= 0.0 { "-inf dB".to_string() } else { format!("{:.1} dB", 20.0 * g.log10()) }
    });
}

/// The crossfader's cursor and read-out (the two sides' shares) above the handle `h`.
pub fn xfader_feel(ui: &Ui, resp: &Response, h: Pos2, v: f32) {
    grab_cursor(resp);
    readout(ui, resp.id, h, resp.dragged() || resp.is_pointer_button_down_on(), || format!("A {:.0} · {:.0} B", (1.0 - v) * 100.0, v * 100.0));
}

/// Read-out text of the TRIM knob.
pub fn fmt_trim(v: f32) -> String {
    db(crate::mixer::trim_db(v))
}

/// Read-out text of HIGH and LOW.
pub fn fmt_shelf(v: f32) -> String {
    db(crate::mixer::shelf_db(v))
}

/// Read-out text of MID.
pub fn fmt_mid(v: f32) -> String {
    db(crate::mixer::mid_db_q(v).0)
}

/// True on the frame a press starts on `resp` (the pointer went down on it).
pub fn press_started(ui: &Ui, resp: &Response) -> bool {
    resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed())
}

/// A small value label at `at` (its bottom centre) that fades in while `active` and out after.
/// `text` is only called while the label is visible (no work and no allocation otherwise).
pub fn readout(ui: &Ui, id: Id, at: Pos2, active: bool, text: impl FnOnce() -> String) {
    if !motion::extras() {
        return;
    }
    let a = motion::toward(id.with("feel_ro"), active as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT * 1.5, Kind::Fade);
    if a <= 0.0 {
        return;
    }
    let text = text();
    let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("feel_readout")));
    let font = crate::theme::rb_bold(10.0);
    let g = lp.layout_no_wrap(text, font, motion::fade(READOUT_TX.c(), a));
    let r = Rect::from_center_size(pos2(at.x, at.y - 8.0 - 2.0 * (1.0 - a)), vec2(g.size().x + 10.0, 15.0));
    lp.rect_filled(r.translate(vec2(0.0, 1.0)), 3.0, motion::fade(SHADOW.c(), 0.35 * a));
    lp.rect_filled(r, 3.0, motion::fade(READOUT_BG.c(), 0.96 * a));
    lp.rect_stroke(r, 3.0, Stroke::new(1.0, motion::fade(READOUT_EDGE.c(), a)), egui::StrokeKind::Inside);
    lp.galley(pos2(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0), g, READOUT_TX.c());
}

/// Read-out text for a 0..1 control shown as a percentage.
pub fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

/// Read-out text in dB ("-inf dB" at silence).
pub fn db(v: f32) -> String {
    if v.is_finite() { format!("{v:+.1} dB") } else { "-inf dB".to_string() }
}

// ------------------------------------------------------------------ lists

/// A list row's background: the selection colour fades in and out, and the row under the
/// pointer gets a soft lift (recordbox's own; rekordbox's list has no hover). Returns the
/// colour the row is filled with; `base` is its colour unselected, `sel` selected.
pub fn row_bg(ui: &Ui, key: Id, rect: Rect, selected: bool, base: Color32, sel: Color32) -> Color32 {
    if !motion::on() {
        let c = if selected { sel } else { base };
        if motion::extras() && !selected && hover_row(ui, rect) {
            return ROW_HOVER.c();
        }
        return c;
    }
    let s = sel_amt(key, selected);
    let h = motion::toward(key.with("feel_rh"), (!selected && hover_row(ui, rect)) as u8 as f32, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade);
    motion::mix(motion::mix(base, outside(ROW_HOVER), h), sel, s)
}

/// The pointer is over `rect` and nothing is being dragged or pressed elsewhere.
fn hover_row(ui: &Ui, rect: Rect) -> bool {
    let Some(p) = ui.ctx().pointer_hover_pos() else { return false };
    rect.contains(p) && ui.clip_rect().contains(p) && !ui.ctx().dragged_id().is_some() && ui.ctx().layer_id_at(p).map_or(true, |l| l == ui.layer_id())
}

#[derive(Clone, Copy)]
struct ScrollAnim {
    from: f32,
    to: f32,
    t0: f64,
    applied: f32,
}

thread_local! {
    static SCROLLS: RefCell<Vec<(Id, ScrollAnim)>> = const { RefCell::new(Vec::new()) };
}

/// Scroll `sa` (the scroll area whose state id is `state_id`) to `y`: at once when motion is
/// Off or Reduced, else smoothly (`scroll_anim` applies it frame by frame).
pub fn scroll_to(ui: &Ui, sa: egui::ScrollArea, state_id: Id, y: f32) -> egui::ScrollArea {
    if !motion::moves() {
        return sa.vertical_scroll_offset(y);
    }
    let cur = egui::scroll_area::State::load(ui.ctx(), state_id).map(|s| s.offset.y).unwrap_or(y);
    if (cur - y).abs() < 0.5 {
        return sa.vertical_scroll_offset(y);
    }
    // The first step is taken on this frame, so the list starts moving with the key press.
    let t0 = motion::now() - motion::frame_lead() as f64;
    let first = cur + (y - cur) * motion::ease(motion::frame_lead() / motion::SCROLL);
    SCROLLS.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|(i, _)| *i != state_id);
        s.push((state_id, ScrollAnim { from: cur, to: y, t0, applied: first }));
    });
    motion::keep_alive();
    sa.vertical_scroll_offset(first)
}

/// The in-flight smooth scroll of `state_id`, if any, applied to `sa`. A wheel, trackpad or
/// scroll-bar move by the user cancels it.
pub fn scroll_anim(ui: &Ui, sa: egui::ScrollArea, state_id: Id) -> egui::ScrollArea {
    let user = ui.input(|i| i.raw_scroll_delta != egui::Vec2::ZERO || i.smooth_scroll_delta != egui::Vec2::ZERO);
    let cur = egui::scroll_area::State::load(ui.ctx(), state_id).map(|s| s.offset.y);
    let now = motion::now();
    SCROLLS.with(|s| {
        let mut s = s.borrow_mut();
        let Some(k) = s.iter().position(|(i, _)| *i == state_id) else { return sa };
        let a = s[k].1;
        let moved_by_someone = cur.map_or(false, |c| (c - a.applied).abs() > 1.0);
        if user || moved_by_someone || !motion::moves() {
            s.remove(k);
            return if motion::moves() { sa } else { sa.vertical_scroll_offset(a.to) };
        }
        let p = ((now - a.t0) as f32 / motion::SCROLL).clamp(0.0, 1.0);
        let v = if p >= 1.0 { a.to } else { a.from + (a.to - a.from) * motion::ease(p) };
        if p >= 1.0 {
            s.remove(k);
        } else {
            s[k].1.applied = v;
            motion::keep_alive();
        }
        sa.vertical_scroll_offset(v)
    })
}

/// A vertical guide across the list while a column edge is dragged or a column is moved.
pub fn column_guide(ui: &Ui, x: f32, top: f32, bottom: f32, strength: f32) {
    if !motion::extras() || strength <= 0.0 {
        return;
    }
    let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("feel_guide")));
    lp.rect_filled(Rect::from_min_max(pos2(x - 0.5, top), pos2(x + 0.5, bottom)), 0.0, motion::fade(GUIDE.c(), 0.8 * strength));
}

/// The x of a column-move insertion mark, sliding between column edges.
pub fn slide_x(id: Id, x: f32) -> f32 {
    motion::follow(id.with("feel_slx"), x, 0.025, 0.25, Kind::Move)
}

/// The y of a list insertion line, sliding between rows.
pub fn slide_y(id: Id, y: f32) -> f32 {
    motion::follow(id.with("feel_sly"), y, 0.025, 0.25, Kind::Move)
}

// ------------------------------------------------------------------ drag and drop

thread_local! {
    static DROP: RefCell<DropState> = const { RefCell::new(DropState { targets: Vec::new(), was_dragging: false, origin: None, label: String::new(), last: None, snap: None }) };
}

struct DropState {
    /// Drop targets drawn this frame (for telling an accepted drop from a refused one).
    targets: Vec<Rect>,
    was_dragging: bool,
    origin: Option<Pos2>,
    label: String,
    last: Option<Pos2>,
    /// A refused drop's label going home: (from, to, start time).
    snap: Option<(Pos2, Pos2, f64)>,
}

/// A drop target's highlight: a blue wash and outline that fade in while a drag is over it.
/// Returns true when the old outline should be drawn instead (motion Off).
pub fn drop_glow(ui: &Ui, id: Id, rect: Rect, over: bool, rounding: f32) -> bool {
    DROP.with(|d| d.borrow_mut().targets.push(rect));
    if !motion::extras() {
        return over;
    }
    let a = motion::toward(id.with("feel_drop"), over as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade);
    if a > 0.0 {
        let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("feel_drop")));
        let r = rect.intersect(ui.clip_rect());
        lp.rect_filled(r, rounding, motion::fade(DROP_WASH.c(), 0.16 * a));
        lp.rect_stroke(r, rounding, Stroke::new(1.5, motion::fade(crate::theme::BLUE.c(), a)), egui::StrokeKind::Inside);
    }
    false
}

/// The label that follows a drag (tracks, a playlist): drawn on top, with a soft shadow.
/// Returns false when the call site should draw its old label (motion Off).
pub fn drag_label(ui: &Ui, pp: Pos2, label: &str) -> bool {
    if !motion::extras() {
        return false;
    }
    DROP.with(|d| {
        let mut d = d.borrow_mut();
        if !d.was_dragging || d.origin.is_none() {
            d.origin = ui.input(|i| i.pointer.press_origin()).or(Some(pp));
        }
        d.label.clear();
        d.label.push_str(label);
        d.last = Some(pp);
    });
    let a = motion::toward(Id::new("feel_drag_in"), 1.0, motion::FEEDBACK, 0.0, Kind::Fade);
    paint_label(ui.ctx(), pp, label, a);
    true
}

fn paint_label(ctx: &egui::Context, pp: Pos2, label: &str, a: f32) {
    let lp = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("dragl")));
    let font = crate::theme::rb(11.0);
    let g = lp.layout_no_wrap(label.to_string(), font, motion::fade(crate::theme::ON_ACCENT.c(), a));
    let rr = Rect::from_min_size(pp + vec2(12.0, 6.0), vec2(g.size().x + 12.0, 18.0));
    lp.rect_filled(rr.translate(vec2(0.0, 2.0)).expand(1.0), 4.0, motion::fade(SHADOW.c(), 0.30 * a));
    lp.rect_filled(rr, 3.0, motion::fade(crate::theme::ROW_SEL.c(), 0.97 * a));
    lp.galley(pos2(rr.min.x + 6.0, rr.center().y - g.size().y / 2.0), g, crate::theme::ON_ACCENT.c());
}

/// Once per frame after everything is drawn: a drag that ended over nothing that takes drops
/// sends its label back to where it started.
fn drops_end_frame(app: &App, ctx: &egui::Context) {
    let dragging = app.drag.is_some() || app.node_drag.is_some();
    DROP.with(|d| {
        let mut d = d.borrow_mut();
        if d.was_dragging && !dragging && motion::moves() {
            let at = ctx.input(|i| i.pointer.latest_pos()).or(d.last);
            let over_target = at.map_or(false, |p| d.targets.iter().any(|r| r.contains(p)));
            if let (Some(from), Some(to), false) = (at, d.origin, over_target) {
                if from.distance(to) > 8.0 {
                    d.snap = Some((from, to, motion::now()));
                }
            }
        }
        if dragging {
            if !d.was_dragging {
                motion::set(Id::new("feel_drag_in"), 0.0);
            }
        } else {
            d.origin = None;
        }
        d.was_dragging = dragging;
        if let Some((from, to, t0)) = d.snap {
            let p = ((motion::now() - t0) as f32 / motion::SNAP_BACK).clamp(0.0, 1.0);
            if p >= 1.0 {
                d.snap = None;
            } else {
                let e = motion::ease(p);
                let label = d.label.clone();
                paint_label(ctx, from + (to - from) * e, &label, 1.0 - e);
                motion::keep_alive();
            }
        }
        d.targets.clear();
    });
}

// ------------------------------------------------------------------ veils (panel, page, mode changes)

#[derive(Clone, Copy)]
struct Veil {
    rect: Rect,
    t0: f64,
    from: f32,
    col: Color32,
}

thread_local! {
    static WATCH: RefCell<Vec<(&'static str, u64)>> = const { RefCell::new(Vec::new()) };
    static VEILS: RefCell<Vec<(&'static str, Veil)>> = const { RefCell::new(Vec::new()) };
}

/// Watch a piece of state that changes what an area shows (a panel shown, a page, a mode):
/// when it changes, the area fades in from `bg` (starting `strength` opaque). The new content
/// is there and live at once; the veil only covers it for the first `PANEL` seconds.
pub fn watch(key: &'static str, state: u64, rect: Rect, bg: Color32, strength: f32) {
    let changed = WATCH.with(|w| {
        let mut w = w.borrow_mut();
        match w.iter_mut().find(|(k, _)| *k == key) {
            Some((_, s)) if *s != state => {
                *s = state;
                true
            }
            Some(_) => false,
            None => {
                w.push((key, state));
                false
            }
        }
    });
    if changed && motion::on() && rect.is_positive() {
        let v = Veil { rect, t0: motion::now(), from: strength, col: bg };
        VEILS.with(|vs| {
            let mut vs = vs.borrow_mut();
            vs.retain(|(k, _)| *k != key);
            vs.push((key, v));
        });
    }
}

fn paint_veils(ctx: &egui::Context) {
    VEILS.with(|vs| {
        let mut vs = vs.borrow_mut();
        if vs.is_empty() {
            return;
        }
        if !motion::on() {
            vs.clear();
            return;
        }
        let now = motion::now();
        let lp = ctx.layer_painter(egui::LayerId::background());
        vs.retain(|(_, v)| {
            let dur = if motion::moves() { motion::PANEL } else { motion::PANEL * 0.7 };
            let p = ((now - v.t0) as f32 / dur).clamp(0.0, 1.0);
            if p >= 1.0 {
                return false;
            }
            let a = v.from * (1.0 - motion::ease(p));
            lp.with_clip_rect(v.rect).rect_filled(v.rect, 0.0, motion::fade(v.col, a));
            motion::keep_alive();
            true
        });
    });
}

/// The areas recordbox watches for changes: modes, layouts, panels, Preferences pages.
fn watch_app(app: &App, ctx: &egui::Context) {
    let screen = ctx.screen_rect();
    let bg = crate::theme::BG.c();
    let below_header = Rect::from_min_max(pos2(screen.min.x, screen.min.y + 28.0), screen.max);
    let perf = app.performance();
    let edit = crate::ui_edit::is_edit(app);
    let mode = if edit { 2 } else if perf { 1 } else { 0 };
    watch("mode", mode, below_header, bg, 0.85);
    let h = if app.prefs.browser_only && !perf { 0.0 } else { app.player_height() };
    let player = Rect::from_min_size(below_header.min, vec2(screen.width(), h));
    let browser = Rect::from_min_max(pos2(screen.min.x, player.max.y), pos2(screen.max.x, screen.max.y - 26.0));
    if perf {
        let lay = crate::ui_perf_layouts::layout(app) as u64;
        let panels = (app.prefs.perf_fx as u64) | (app.prefs.rec.panel as u64) << 1;
        watch("perf_layout", lay << 8 | mode, player, bg, 0.8);
        watch("perf_panels", panels << 8 | lay, player, bg, 0.6);
    } else if !edit {
        watch("player_layout", (app.prefs.player_layout as u64) << 1 | app.prefs.browser_only as u64, below_header, bg, 0.8);
    }
    if !edit {
        let rp = crate::ui_browser_ext::right_panel(app) as u64;
        let sub = (app.prefs.bx.sub_horizontal as u64) << 8 | (app.prefs.column_browser as u64) << 9;
        watch("browser_panels", rp | sub | (mode << 16), browser, bg, 0.5);
        // A new sort order: the rows fade in rather than jump.
        let sort = (app.prefs.sort_col as u64) << 1 | app.prefs.sort_asc as u64;
        watch("sort", sort, Rect::from_min_max(pos2(browser.min.x + app.prefs.bx.tree_w + 5.0, browser.min.y + 44.0), pos2(browser.max.x - 25.0, browser.max.y)), crate::theme::BG1.c(), 0.55);
    }
}

// ------------------------------------------------------------------ keyboard focus

/// The keyboard focus ring (lists, the tree, dialogs, Preferences).
pub const FOCUS_RING: Tc = tc(0x3C8CFF, 0x1473EB);
/// The text caret of every text field.
pub const CARET: Tc = tc(0xC0DEFF, 0x00537D);

thread_local! {
    /// The keyboard is being used to move around (a navigation key since the last click).
    static KB: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The main window's keyboard focus is on the tree (else on the track list).
    static KB_TREE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Where the ring around a focused egui widget was last drawn, per viewport.
    static RING: RefCell<Vec<(egui::ViewportId, egui::LayerId, Rect)>> = const { RefCell::new(Vec::new()) };
    /// Dialog buttons seen this frame that answer Return / Escape: (viewport, layer, id, role).
    static KEYS: RefCell<Vec<(egui::ViewportId, egui::LayerId, Id, KeyRole)>> = const { RefCell::new(Vec::new()) };
    static KEYS_LAST: RefCell<Vec<(egui::ViewportId, egui::LayerId, Id, KeyRole)>> = const { RefCell::new(Vec::new()) };
    /// The front dialog and its default button, as seen at the start of this frame.
    static TAB_TOP: std::cell::Cell<Option<(egui::LayerId, Option<Id>)>> = const { std::cell::Cell::new(None) };
    static TAB_LEFT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True while the keyboard is being used to move around: a navigation key (Tab, the arrows,
/// Home, End, Page Up / Down) was pressed since the last mouse click. Off: never.
pub fn keyboard() -> bool {
    motion::extras() && KB.with(|k| k.get())
}

fn kb_input(events: &[egui::Event]) {
    use egui::Key::*;
    for e in events {
        match e {
            egui::Event::Key { key: Tab | ArrowUp | ArrowDown | ArrowLeft | ArrowRight | Home | End | PageUp | PageDown, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl => KB.with(|k| k.set(true)),
            egui::Event::PointerButton { pressed: true, .. } => KB.with(|k| k.set(false)),
            _ => {}
        }
    }
}

fn ring(p: &egui::Painter, r: Rect, a: f32) {
    p.rect_stroke(r, 3.0, Stroke::new(2.0, motion::fade(FOCUS_RING.c(), 0.9 * a)), egui::StrokeKind::Inside);
}

/// The ring on the row that has the main window's keyboard focus: the track list's current
/// row (`tree` false) or the tree's selected row (`tree` true), drawn only while the keyboard
/// is being used and the focus is on that side (View > Change Focus, Tab).
pub fn focus_row(ui: &Ui, r: Rect, tree: bool) {
    if !motion::extras() {
        return;
    }
    let on = keyboard() && KB_TREE.with(|t| t.get()) == tree && !text_focused(ui.ctx());
    let a = appear(Id::new(("feel_kbrow", tree)), on);
    if a > 0.0 {
        ring(&ui.painter().with_clip_rect(ui.clip_rect()), r.shrink(1.0), a);
    }
}

/// End of a viewport's frame: the ring around the egui widget that has the keyboard focus in a
/// dialog, menu or Preferences (the main window's own panels use `focus_row`).
fn focus_end_frame(ctx: &egui::Context) {
    let vp = ctx.viewport_id();
    if !motion::extras() {
        RING.with(|r| r.borrow_mut().retain(|x| x.0 != vp));
        return;
    }
    let target = ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id)).filter(|r| keyboard() && (r.layer_id != egui::LayerId::background() || vp != egui::ViewportId::ROOT) && r.rect.is_positive());
    // (Not a window's title bar, which egui's Tab order also visits.)
    let target = target.filter(|r| !ctx.memory(|m| m.area_rect(r.layer_id.id)).is_some_and(|a| r.layer_id.order == egui::Order::Middle && r.rect.min.y <= a.min.y + 2.0 && r.rect.width() >= 0.8 * a.width()));
    let a = motion::toward(Id::new(("feel_ring_a", vp)), target.is_some() as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade);
    let shown = RING.with(|rg| {
        let mut rg = rg.borrow_mut();
        let k = rg.iter().position(|x| x.0 == vp);
        if let Some(t) = &target {
            let want = t.rect.expand(2.0);
            let fresh = k.is_none() || a < 0.3 || rg[k.unwrap()].1 != t.layer_id;
            // The ring glides from the widget it was on to the next one (Tab).
            let c = |n: i32, v: f32| {
                let id = Id::new(("feel_ring", vp, n));
                if fresh {
                    motion::set(id, v);
                }
                motion::tween(id, v, motion::GLIDE, Kind::Move)
            };
            let r = Rect::from_min_max(pos2(c(0, want.min.x), c(1, want.min.y)), pos2(c(2, want.max.x), c(3, want.max.y)));
            match k {
                Some(k) => rg[k] = (vp, t.layer_id, r),
                None => rg.push((vp, t.layer_id, r)),
            }
            Some((t.layer_id, r))
        } else {
            k.map(|k| (rg[k].1, rg[k].2))
        }
    });
    if let Some((layer, r)) = shown.filter(|_| a > 0.0) {
        ring(&ctx.layer_painter(layer), r, a);
    }
}

/// What a dialog button answers: Return (the confirming one), Escape (Cancel, No, Close) or
/// both (a dialog's only button).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyRole {
    Return,
    Escape,
    Both,
}

/// The role of a dialog button from its label (None: neither key).
pub fn key_role(label: &str) -> Option<KeyRole> {
    match label.trim() {
        "Cancel" | "No" | "Close" | "Don't Save" => Some(KeyRole::Escape),
        "OK" | "Ok" | "Yes" | "Done" | "Save" | "Apply" | "Export" | "Start" | "Continue" => Some(KeyRole::Return),
        _ => None,
    }
}

/// A button of a dialog (a window, not the main window's panels) that Return or Escape
/// presses: the frontmost dialog's button is clicked on the next key press.
pub fn dialog_key(resp: &Response, role: Option<KeyRole>) {
    let Some(role) = role else { return };
    if resp.layer_id == egui::LayerId::background() || !resp.enabled() {
        return;
    }
    let vp = resp.ctx.viewport_id();
    KEYS.with(|k| {
        let mut k = k.borrow_mut();
        if let Some(x) = k.iter_mut().find(|x| x.2 == resp.id) {
            x.3 = role;
        } else if k.len() < 64 {
            k.push((vp, resp.layer_id, resp.id, role));
        }
    });
}

/// An egui button with a standard label that Return / Escape answer in a dialog.
pub fn key_button(ui: &mut Ui, label: &str) -> Response {
    let r = ui.button(label);
    dialog_key(&r, key_role(label));
    r
}

/// Return or Escape pressed (no text field taking it): the frontmost dialog's matching button
/// is clicked (through egui's accessibility click, so nothing else sees the key).
fn dialog_keys_input(ctx: &egui::Context, raw: &mut egui::RawInput) {
    if !motion::extras() || text_focused(ctx) {
        return;
    }
    let vp = ctx.viewport_id();
    if std::env::var("PREPDECK_FEEL_DEBUG").is_ok() && raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape | egui::Key::Enter, pressed: true, .. })) {
        eprintln!("feeldebug: keys seen, {} buttons known", KEYS_LAST.with(|k| k.borrow().len()));
    }
    let cands = KEYS_LAST.with(|k| k.borrow().iter().filter(|x| x.0 == vp).copied().collect::<Vec<_>>());
    if cands.is_empty() {
        return;
    }
    // The frontmost layer that has such buttons.
    let order: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
    let rank = |l: egui::LayerId| (l.order as usize, order.iter().position(|x| *x == l).unwrap_or(0));
    let Some(top) = cands.iter().map(|c| c.1).max_by_key(|l| rank(*l)) else { return };
    // A widget of that dialog reached with Tab takes Return itself (egui clicks it).
    let focused = ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id));
    // (The window's title bar takes focus in egui's Tab order too: it counts as outside.)
    let area = ctx.memory(|m| m.area_rect(top.id));
    let title = |r: &Response| area.is_some_and(|a| r.rect.min.y <= a.min.y + 2.0 && r.rect.width() >= 0.8 * a.width());
    let focus_in_top = focused.as_ref().is_some_and(|r| r.layer_id == top && !title(r));
    // Tab stays in the front dialog: from outside it, the first Tab goes to its default button.
    let first = cands.iter().filter(|c| c.1 == top).find(|c| c.3 != KeyRole::Escape).or_else(|| cands.iter().find(|c| c.1 == top)).map(|c| c.2);
    TAB_TOP.with(|t| t.set(Some((top, first))));
    let wrapped = TAB_LEFT.with(|t| t.replace(false));
    if !focus_in_top {
        if let Some(f) = first {
            if wrapped {
                // Last frame's Tab took the focus out of the dialog (egui moves it at the end of
                // the frame): round to its first button.
                ctx.memory_mut(|m| m.request_focus(f));
            } else if let Some(k) = raw.events.iter().position(|e| matches!(e, egui::Event::Key { key: egui::Key::Tab, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl)) {
                ctx.memory_mut(|m| m.request_focus(f));
                raw.events.remove(k);
                KB.with(|k| k.set(true));
            }
        }
    }
    let mut i = 0;
    while i < raw.events.len() {
        let hit = match &raw.events[i] {
            egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, repeat: false, .. } if modifiers.is_none() && !focus_in_top => Some(KeyRole::Return),
            egui::Event::Key { key: egui::Key::Escape, pressed: true, modifiers, repeat: false, .. } if modifiers.is_none() => Some(KeyRole::Escape),
            _ => None,
        };
        // A dialog's only button answers both keys.
        let alone = cands.iter().filter(|c| c.1 == top).count() == 1;
        let target = hit.and_then(|want| {
            let mut m = cands.iter().filter(|c| c.1 == top && (alone || c.3 == want || c.3 == KeyRole::Both));
            match (m.next(), m.next()) {
                (Some(c), None) => Some(c.2),
                _ => None,
            }
        });
        if hit.is_some() && std::env::var("PREPDECK_FEEL_DEBUG").is_ok() {
            eprintln!("feeldebug: key {hit:?} top {top:?} cands {:?} target {target:?}", cands.iter().map(|c| (c.1, c.2.short_debug_format(), c.3)).collect::<Vec<_>>());
        }
        if let Some(id) = target {
            raw.events[i] = egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest { action: egui::accesskit::Action::Click, target: egui::accesskit::NodeId(id.value()), data: None });
        }
        i += 1;
    }
}

/// A text field has the keyboard focus (egui's `wants_keyboard_input` is true for any focused
/// widget, a button reached with Tab too).
fn text_focused(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.focused()).is_some_and(|id| egui::text_edit::TextEditState::load(ctx, id).is_some())
}

fn keys_end_frame(ctx: &egui::Context) {
    let vp = ctx.viewport_id();
    // A Tab inside the front dialog: if egui moves the focus out of it, the next frame brings
    // it back to the dialog's first button (the ring wraps).
    if let Some((top, Some(_))) = TAB_TOP.with(|t| t.take()).filter(|_| vp == egui::ViewportId::ROOT) {
        let tab = ctx.input(|i| i.key_pressed(egui::Key::Tab));
        let inside = ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id)).is_some_and(|r| r.layer_id == top);
        TAB_LEFT.with(|t| t.set(tab && inside));
        let _ = top;
    }
    KEYS.with(|k| {
        let mut k = k.borrow_mut();
        KEYS_LAST.with(|l| {
            let mut l = l.borrow_mut();
            l.retain(|x| x.0 != vp);
            l.extend(k.iter().filter(|x| x.0 == vp).copied());
        });
        k.retain(|x| x.0 != vp);
    });
}

/// The caret and text selection of every text field: one look in every window and dialog.
pub fn text_look(v: &mut egui::Visuals) {
    v.text_cursor.stroke.color = CARET.c();
    v.selection.bg_fill = crate::theme::BLUE.c();
    v.selection.stroke = Stroke::new(1.0, crate::theme::ON_ACCENT.c());
}

// ------------------------------------------------------------------ Preferences pages

thread_local! {
    static PAGES: RefCell<Vec<(Id, u64, f64)>> = const { RefCell::new(Vec::new()) };
}

/// Preferences is a window of its own, which the main window's veils do not reach: when the
/// page or tab shown in `rect` changes (`state`), the new page fades in from `bg` the way
/// the main window's panels do (`PANEL`, 70 % at the start). Painted on `ui`'s painter, so
/// call it after the page is drawn. Nothing moves; the page is live at once.
pub fn page_veil(ui: &Ui, key: Id, state: u64, rect: Rect, bg: Color32) {
    let now = motion::now();
    let start = PAGES.with(|p| {
        let mut p = p.borrow_mut();
        match p.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, s, t0)) => {
                if *s != state {
                    *s = state;
                    *t0 = now - motion::frame_lead() as f64;
                }
                *t0
            }
            None => {
                p.push((key, state, f64::NEG_INFINITY));
                f64::NEG_INFINITY
            }
        }
    });
    if !motion::on() || !rect.is_positive() {
        return;
    }
    let dur = if motion::moves() { motion::PANEL } else { motion::PANEL * 0.7 };
    let p = ((now - start) as f32 / dur).clamp(0.0, 1.0);
    if p >= 1.0 {
        return;
    }
    let a = 0.7 * (1.0 - motion::ease(p));
    ui.painter().with_clip_rect(rect).rect_filled(rect, 0.0, motion::fade(bg, a));
    motion::keep_alive();
}

// ------------------------------------------------------------------ waveforms: zoom

/// The time span a zoomed waveform shows, easing (in log scale, 140 ms) to a new zoom step
/// instead of jumping; exactly `span` once there. Keyed by the waveform's place `r` (two
/// players can share one `ui`).
pub fn zoom_span(ui: &Ui, r: Rect, span: f64) -> f64 {
    let target = (span as f32).ln();
    let v = motion::tween(ui.id().with(("feel_zoom", r.min.x as i32, r.min.y as i32)), target, motion::SCROLL, Kind::Move);
    if v == target { span } else { (v as f64).exp() }
}

// ------------------------------------------------------------------ waveforms: the drawn play position

#[derive(Clone, Copy, Default)]
struct PosTrack {
    shown: f64,
    real: f64,
    real_at: f64,
    rate: f64,
    frame: u64,
    /// Start of the window the rate is measured over (position, display time).
    anchor: (f64, f64),
}

thread_local! {
    static POS: RefCell<Vec<(usize, PosTrack)>> = const { RefCell::new(Vec::new()) };
    static FRAME_NO: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The play position (seconds) to draw the active deck's waveforms, platter and times at.
/// The audio thread moves the real position once per audio buffer (about 11 ms), which a
/// 120 Hz display samples unevenly (about a third of the frames see no move, others two
/// buffers' worth). The drawn position advances each frame by the playback rate measured over
/// the last 0.12 s or more, times one step of the display clock (`motion::display_time`), and
/// is pulled slowly onto the real one. A seek, a loop wrap, a scratch or a stop is followed at
/// once; a paused deck is drawn at exactly its position.
pub fn shown_pos(app: &App) -> f64 {
    let real = app.pos();
    let key = std::sync::Arc::as_ptr(&app.player.state) as usize;
    // A waveform being dragged is drawn where the pointer has it (`wave_drag.rs`).
    if let Some(t) = held_pos(app) {
        POS.with(|p| p.borrow_mut().retain(|(k, _)| *k != key));
        return t;
    }
    if !motion::on() {
        return real;
    }
    let frame = FRAME_NO.with(|f| f.get());
    let (now, step) = (motion::display_time(), motion::display_step());
    let fresh = |real: f64| PosTrack { shown: real, real, real_at: now, rate: 0.0, frame, anchor: (real, now) };
    POS.with(|p| {
        let mut p = p.borrow_mut();
        let k = match p.iter().position(|(k, _)| *k == key) {
            Some(k) => k,
            None => {
                p.push((key, fresh(real)));
                return real;
            }
        };
        let t = &mut p[k].1;
        if t.frame == frame {
            return t.shown;
        }
        let gap = frame.saturating_sub(t.frame);
        t.frame = frame;
        if gap > 2 || step <= 0.0 || step > 0.1 {
            *t = fresh(real);
            return real;
        }
        if real != t.real {
            // A move against the expected one is a seek, a loop wrap or a scratch.
            let expect = t.real + t.rate * (now - t.real_at);
            let jump = (real - t.real).abs() > 0.25 || (t.rate != 0.0 && (real - expect).abs() > 0.06);
            t.real = real;
            t.real_at = now;
            if jump {
                *t = fresh(real);
                return real;
            }
            let span = now - t.anchor.1;
            if span >= 0.12 {
                let r = (real - t.anchor.0) / span;
                t.rate = if t.rate == 0.0 { r } else { t.rate + (r - t.rate) * 0.5 };
                t.anchor = (real, now);
            }
        } else if now - t.real_at > 0.035 {
            // No callback for longer than three buffers: stopped (or held).
            *t = fresh(real);
            return real;
        }
        if t.rate == 0.0 {
            t.shown = real;
            return real;
        }
        // Where the audio is now (the last callback's position carried on at the rate), and
        // where the drawing would be after one more display step; the drawing is pulled onto
        // the audio slowly, so the frame-to-frame step stays even.
        let extrap = t.real + t.rate * (now - t.real_at);
        let predicted = t.shown + t.rate * step;
        let err = extrap - predicted;
        if err.abs() > 0.06 {
            t.shown = real;
        } else {
            t.shown = predicted + err * (1.0 - (-step / 0.3).exp());
        }
        motion::keep_alive();
        t.shown
    })
}

thread_local! {
    /// Decks whose waveform is being dragged: (deck, the position under the pointer).
    static HELD: RefCell<Vec<(usize, f64)>> = const { RefCell::new(Vec::new()) };
}

fn deck_key(app: &App) -> usize {
    std::sync::Arc::as_ptr(&app.player.state) as usize
}

/// The active deck's waveform is being dragged and the pointer has it at `t` seconds: it is
/// drawn there until `release_pos`, whatever the audio does meanwhile.
pub fn hold_pos(app: &App, t: f64) {
    let key = deck_key(app);
    HELD.with(|h| {
        let mut h = h.borrow_mut();
        match h.iter_mut().find(|(k, _)| *k == key) {
            Some(e) => e.1 = t,
            None => h.push((key, t)),
        }
    });
}

/// The position a dragged waveform is held at, if the active deck's is.
pub fn held_pos(app: &App) -> Option<f64> {
    let key = deck_key(app);
    HELD.with(|h| h.borrow().iter().find(|(k, _)| *k == key).map(|e| e.1))
}

/// The deck was just moved by `by` seconds by hand (a scroll over its waveform): the drawn
/// position moves by as much on this frame and keeps its own clock, so each frame of a scroll
/// over a playing deck moves by the scroll plus one refresh of play (restarting from the
/// audio's position instead, which only moves once per audio buffer, left one frame in three
/// about 10 ms short).
pub fn shift_pos(app: &App, by: f64) {
    let key = deck_key(app);
    POS.with(|p| {
        if let Some((_, t)) = p.borrow_mut().iter_mut().find(|(k, _)| *k == key) {
            t.shown += by;
            t.real += by;
            t.anchor.0 += by;
        }
    });
}

pub fn release_pos(app: &App) {
    let key = deck_key(app);
    HELD.with(|h| h.borrow_mut().retain(|(k, _)| *k != key));
}

// ------------------------------------------------------------------ the track list's totals

static TRACKS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A track may have changed (app.rs `track_mut`): the list totals are counted again.
pub fn tracks_touched() {
    TRACKS_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

thread_local! {
    static TOTALS: std::cell::Cell<Option<((u64, usize, usize, usize, u64), (usize, f64, u64))>> = const { std::cell::Cell::new(None) };
}

/// The footer's totals of the list showing (tracks, seconds, bytes). Counting them looked up
/// every track of the list on every frame; they are counted again only when the list's tracks,
/// the library or a track changes (the key: an order-free sum of the list's ids, the list's
/// length, the library's place and size, and `tracks_touched`).
pub fn list_totals(app: &App) -> (usize, f64, u64) {
    let ids = app.list.iter().fold(0u64, |a, id| a.wrapping_add((*id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5851_F42D));
    let key = (ids, app.list.len(), app.tracks.as_ptr() as usize, app.tracks.len(), TRACKS_GEN.load(std::sync::atomic::Ordering::Relaxed));
    if let Some((k, v)) = TOTALS.with(|t| t.get()) {
        if k == key {
            return v;
        }
    }
    let v = app.list.iter().filter_map(|id| app.track(*id)).fold((0, 0.0, 0u64), |(n, s, b), t| (n + 1, s + t.duration, b + t.file_size));
    TOTALS.with(|t| t.set(Some((key, v))));
    v
}

// ------------------------------------------------------------------ track list artwork off the frame

struct ArtLoader {
    tx: crossbeam_channel::Sender<String>,
    rx: crossbeam_channel::Receiver<(String, Option<egui::ColorImage>)>,
    pending: std::collections::HashSet<String>,
}

thread_local! {
    static ART: RefCell<Option<ArtLoader>> = const { RefCell::new(None) };
}

/// A track list cell's artwork. Decoding a cover took 2.5-6.7 ms per row on the frame that
/// first showed it (a list's first frame: 30-90 ms). Here the file is decoded on a background
/// thread and the picture fades in when it is ready (the same 96 px thumbnail as before).
/// Motion Off: loaded on the frame, as before.
pub fn list_artwork(app: &mut App, path: &str) -> Option<egui::TextureHandle> {
    if !motion::on() || path.is_empty() || app.textures.contains_key(path) {
        return crate::ui_browser::artwork_texture(app, path);
    }
    let ctx = app.ctx.clone();
    ART.with(|a| {
        let mut a = a.borrow_mut();
        let l = a.get_or_insert_with(|| {
            let (tx, jobs) = crossbeam_channel::unbounded::<String>();
            let (done, rx) = crossbeam_channel::unbounded();
            std::thread::Builder::new()
                .name("artwork".into())
                .spawn(move || {
                    for p in jobs {
                        let img = image::open(&p).ok().map(|img| {
                            let small = img.thumbnail(96, 96).to_rgba8();
                            egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], &small)
                        });
                        if done.send((p, img)).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                })
                .ok();
            ArtLoader { tx, rx, pending: Default::default() }
        });
        if l.pending.insert(path.to_string()) {
            let _ = l.tx.send(path.to_string());
        }
    });
    None
}

/// Textures of covers decoded since the last frame.
fn poll_artwork(app: &mut App) {
    ART.with(|a| {
        let mut a = a.borrow_mut();
        let Some(l) = a.as_mut() else { return };
        while let Ok((path, img)) = l.rx.try_recv() {
            l.pending.remove(&path);
            if app.textures.contains_key(&path) {
                continue;
            }
            let tex = img.map(|ci| app.ctx.load_texture(path.as_str(), ci, egui::TextureOptions::LINEAR));
            motion::set(Id::new(("feel_art", path.as_str())), 0.0);
            app.textures.insert(path, tex);
        }
    });
}

/// The tint a track list cover is drawn with: white, fading in when it has just arrived.
pub fn art_tint(path: &str) -> Color32 {
    let a = motion::toward(Id::new(("feel_art", path)), 1.0, motion::PANEL, 0.0, Kind::Fade);
    motion::fade(Color32::WHITE, a)
}

// ------------------------------------------------------------------ egui's own widgets

// egui's built-in widgets (buttons, menu and context-menu items, combo-box lists, selectable
// labels, text fields: about 430 call sites) pick one of their style's looks (inactive, hovered,
// pressed) per frame, so hover and release snap. Instead of wrapping every call site, the
// frame rectangle such a widget drew is found in its layer at the end of the frame and redrawn
// part way between the look it had before and the one it has now. The looks are the ones the
// widget itself drew (seen on earlier frames), so any style a dialog or menu sets is kept, and
// once a change has settled nothing is touched: the frame is exactly what egui drew.

/// A widget's frame rectangle in one state (None in a `looks` slot: nothing drawn).
#[derive(Clone, Copy, PartialEq)]
struct Look {
    rect: Rect,
    fill: Color32,
    stroke: Stroke,
    radius: egui::CornerRadius,
}

#[derive(Clone, Copy)]
struct WTrack {
    id: Id,
    vp: egui::ViewportId,
    /// The look seen in each state: inactive, hovered, pressed (outer None: not seen yet),
    /// relative to the widget's top left (a list may scroll while a hover fades).
    looks: [Option<Option<Look>>; 3],
    state: u8,
    /// Where the frame is: the widget's rect, or the box inside it (a check box's) as it was
    /// before the pointer came, relative to the widget's top left.
    anchor: Rect,
    /// Last frame's drawn look, rect and state (a widget whose look changes on its own is left
    /// alone: it animates itself).
    last: Option<(Option<Look>, Rect, u8)>,
}

thread_local! {
    static WTRACKS: RefCell<Vec<WTrack>> = const { RefCell::new(Vec::new()) };
    /// Frame rectangles near the pointer at the end of the last frame that moved it, per
    /// viewport and layer: the look a widget had before the pointer reached it.
    static NEAR: RefCell<Vec<(egui::ViewportId, egui::LayerId, Look)>> = const { RefCell::new(Vec::new()) };
    /// Widgets this file eases itself this frame (the pass leaves them alone).
    static HANDLED: RefCell<Vec<Id>> = const { RefCell::new(Vec::new()) };
    /// Widgets that animate themselves (egui's scroll bars, for one).
    static OWN: RefCell<Vec<Id>> = const { RefCell::new(Vec::new()) };
    static SCRATCH_IDS: RefCell<Vec<Id>> = const { RefCell::new(Vec::new()) };
}

/// Frames of egui widgets redrawn between looks so far (tools read it).
pub static EASED_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Note that `id` is eased by a helper of this file this frame.
#[inline]
fn mark(id: Id) {
    HANDLED.with(|h| {
        let mut h = h.borrow_mut();
        if h.len() < 4096 {
            h.push(id);
        }
    });
}

/// The frame rectangle of a widget at `r`: the first plain rectangle with the same centre
/// that is `r` grown or shrunk by the same amount (up to 8 pt) on every side.
fn find_frame(list: &egui::layers::PaintList, r: Rect) -> Option<(usize, Look)> {
    for (i, cs) in list.all_entries().enumerate() {
        if let egui::Shape::Rect(s) = &cs.shape {
            if s.brush.is_some() || s.blur_width > 0.0 {
                continue;
            }
            let (dw, dh) = (s.rect.width() - r.width(), s.rect.height() - r.height());
            if (dw - dh).abs() < 0.02 && dw.abs() <= 8.02 && (s.rect.center() - r.center()).length_sq() < 0.0004 {
                return Some((i, Look { rect: s.rect, fill: s.fill, stroke: s.stroke, radius: s.corner_radius }));
            }
        }
    }
    None
}

fn same_frame(a: Rect, r: Rect) -> bool {
    let (dw, dh) = (a.width() - r.width(), a.height() - r.height());
    (dw - dh).abs() < 0.02 && dw.abs() <= 8.02 && (a.center() - r.center()).length_sq() < 0.0004
}

fn fade_look(l: Look, t: f32) -> Look {
    Look { fill: motion::fade(l.fill, t), stroke: Stroke::new(l.stroke.width, motion::fade(l.stroke.color, t)), ..l }
}

fn lerp_look(a: Option<Look>, b: Option<Look>, t: f32) -> Option<Look> {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    match (a, b) {
        (None, None) => None,
        (Some(a), None) => Some(fade_look(a, 1.0 - t)),
        (None, Some(b)) => Some(fade_look(b, t)),
        (Some(a), Some(b)) => {
            let none = |s: Stroke| s.width <= 0.0 || s.color.a() == 0;
            let stroke = match (none(a.stroke), none(b.stroke)) {
                (true, true) => b.stroke,
                (false, true) => Stroke::new(a.stroke.width, motion::fade(a.stroke.color, 1.0 - t)),
                (true, false) => Stroke::new(b.stroke.width, motion::fade(b.stroke.color, t)),
                (false, false) => Stroke::new(a.stroke.width + (b.stroke.width - a.stroke.width) * t, motion::mix(a.stroke.color, b.stroke.color, t)),
            };
            let lr = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
            let (ra, rb) = (a.radius, b.radius);
            Some(Look {
                rect: Rect::from_min_max(a.rect.min.lerp(b.rect.min, t), a.rect.max.lerp(b.rect.max, t)),
                fill: motion::mix(a.fill, b.fill, t),
                stroke,
                radius: egui::CornerRadius { nw: lr(ra.nw, rb.nw), ne: lr(ra.ne, rb.ne), sw: lr(ra.sw, rb.sw), se: lr(ra.se, rb.se) },
            })
        }
    }
}

/// Time the pass took: total ns, frames, longest ns (tools read it).
pub static PASS_NS: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

/// End of a viewport's frame: ease egui's own widgets between their looks.
fn widgets_end_frame(ctx: &egui::Context) {
    use std::sync::atomic::Ordering::Relaxed;
    let t0 = std::time::Instant::now();
    widgets_pass(ctx);
    let ns = t0.elapsed().as_nanos() as u64;
    PASS_NS[0].fetch_add(ns, Relaxed);
    PASS_NS[1].fetch_add(1, Relaxed);
    PASS_NS[2].fetch_max(ns, Relaxed);
}

fn widgets_pass(ctx: &egui::Context) {
    let vp = ctx.viewport_id();
    if !motion::on() {
        WTRACKS.with(|w| w.borrow_mut().retain(|t| t.vp != vp));
        NEAR.with(|n| n.borrow_mut().retain(|x| x.0 != vp));
        return;
    }
    let (pp, delta) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.delta().length() + i.raw_scroll_delta.length() + i.smooth_scroll_delta.length()));
    // Widgets that became hovered start being followed, with the look they had last frame.
    SCRATCH_IDS.with(|s| {
        let mut ids = s.borrow_mut();
        ids.clear();
        ctx.interaction_snapshot(|snap| ids.extend(snap.hovered.iter().copied()));
        for &id in ids.iter() {
            let skip = HANDLED.with(|h| h.borrow().contains(&id)) || OWN.with(|o| o.borrow().contains(&id)) || WTRACKS.with(|w| w.borrow().iter().any(|t| t.id == id));
            if skip {
                continue;
            }
            let Some(r) = ctx.read_response(id) else { continue };
            // (Buttons, menu items, list entries of pop-ups; not whole list rows or panels.)
            if !r.sense.senses_click() || !r.enabled() || r.rect.width() > 700.0 || r.rect.height() > 80.0 {
                continue;
            }
            // Its frame as it was: centred on the widget, else (a check box) the first box inside it.
            let (before, anchor) = NEAR.with(|n| {
                let n = n.borrow();
                let mine = n.iter().filter(|(v, l, _)| *v == vp && *l == r.layer_id);
                match mine.clone().find(|(_, _, k)| same_frame(k.rect, r.rect)) {
                    Some(x) => (Some(x.2), r.rect),
                    None => match mine.clone().find(|(_, _, k)| r.rect.expand(1.0).contains_rect(k.rect) && k.rect.width() < 40.0 && k.rect.height() < 40.0) {
                        Some(x) => (Some(x.2), x.2.rect),
                        None => (None, r.rect),
                    },
                }
            });
            motion::set(id.with("feel_eh"), 0.0);
            motion::set(id.with("feel_ep"), 0.0);
            let anchor = anchor.translate(-r.rect.min.to_vec2());
            let before = before.map(|l| Look { rect: l.rect.translate(-r.rect.min.to_vec2()), ..l });
            WTRACKS.with(|w| w.borrow_mut().push(WTrack { id, vp, looks: [Some(before), None, None], state: 0, anchor, last: None }));
        }
    });
    // The rectangles near the pointer now, for the next frame (taken before any is redrawn).
    if let Some(p) = pp.filter(|_| delta > 0.0 || NEAR.with(|n| !n.borrow().iter().any(|x| x.0 == vp))) {
        let reach = (8.0 + 2.5 * delta).min(300.0);
        let mut layers = [Some(egui::LayerId::background()), ctx.layer_id_at(p)];
        if layers[1] == layers[0] {
            layers[1] = None;
        }
        NEAR.with(|n| {
            let mut n = n.borrow_mut();
            n.retain(|x| x.0 != vp);
            ctx.graphics(|g| {
                for layer in layers.into_iter().flatten() {
                    let Some(list) = g.get(layer) else { continue };
                    for cs in list.all_entries() {
                        if let egui::Shape::Rect(s) = &cs.shape {
                            if n.len() < 512 && s.brush.is_none() && s.rect.expand(reach).contains(p) && s.rect.width() < 1200.0 {
                                n.push((vp, layer, Look { rect: s.rect, fill: s.fill, stroke: s.stroke, radius: s.corner_radius }));
                            }
                        }
                    }
                }
            });
        });
    }
    // Each followed widget: its state, the eased amounts, and its frame redrawn between looks.
    WTRACKS.with(|w| {
        let mut w = w.borrow_mut();
        w.retain_mut(|t| {
            if t.vp != vp {
                return true;
            }
            let Some(r) = ctx.read_response(t.id) else { return false };
            let pressed = r.is_pointer_button_down_on() || r.clicked() || r.has_focus();
            let state: u8 = if pressed { 2 } else if r.hovered() { 1 } else { 0 };
            let (ht, pt) = (r.hovered() as u8 as f32, pressed as u8 as f32);
            let h = motion::toward(t.id.with("feel_eh"), ht, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade);
            let p = motion::toward(t.id.with("feel_ep"), pt, 0.0, motion::RELEASE, Kind::Fade);
            let settled = h == ht && p == pt;
            if settled && state == t.state && t.looks[state as usize].is_some() {
                return state != 0;
            }
            let mut keep = true;
            ctx.graphics_mut(|g| {
                let Some(list) = g.get_mut(r.layer_id) else {
                    keep = false;
                    return;
                };
                let drawn = find_frame(list, t.anchor.translate(r.rect.min.to_vec2()));
                let dl = drawn.map(|d| d.1);
                if let Some((pl, pr, ps)) = t.last {
                    if ps == state && pr == r.rect && pl != dl {
                        // The widget's own look moved without a change of state.
                        OWN.with(|o| {
                            let mut o = o.borrow_mut();
                            if o.len() > 256 {
                                o.clear();
                            }
                            o.push(t.id);
                        });
                        keep = false;
                        return;
                    }
                }
                let o = r.rect.min.to_vec2();
                t.looks[state as usize] = Some(dl.map(|l| Look { rect: l.rect.translate(-o), ..l }));
                t.state = state;
                t.last = Some((dl, r.rect, state));
                if settled {
                    keep = state != 0;
                    return;
                }
                let look = |k: usize| t.looks[k].map(|x| x.map(|l| Look { rect: l.rect.translate(o), ..l })).unwrap_or(dl);
                let shown = lerp_look(lerp_look(look(0), look(1), h), look(2), p);
                if shown == dl {
                    return;
                }
                EASED_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                match (drawn, shown) {
                    (Some((i, _)), s) => {
                        let s = s.unwrap_or(Look { fill: Color32::TRANSPARENT, stroke: Stroke::NONE, ..dl.unwrap() });
                        list.mutate_shape(egui::layers::ShapeIdx(i), |cs| {
                            if let egui::Shape::Rect(rs) = &mut cs.shape {
                                rs.rect = s.rect;
                                rs.fill = s.fill;
                                rs.stroke = s.stroke;
                                rs.corner_radius = s.radius;
                            }
                        });
                    }
                    (None, Some(s)) => {
                        // Nothing drawn in this state (a menu item or a selectable label at
                        // rest): the fading frame goes in just behind the widget's first shape.
                        let inside = r.rect.expand(2.0);
                        let found = list.all_entries().enumerate().find(|(_, cs)| {
                            let b = cs.shape.visual_bounding_rect();
                            b.is_positive() && inside.contains_rect(b)
                        });
                        match found {
                            Some((j, cs)) => {
                                let cs = cs.clone();
                                let rs = egui::epaint::RectShape::new(s.rect, s.radius, s.fill, s.stroke, egui::StrokeKind::Inside);
                                list.set(egui::layers::ShapeIdx(j), cs.clip_rect, egui::Shape::Vec(vec![egui::Shape::Rect(rs), cs.shape]));
                            }
                            None => keep = false,
                        }
                    }
                    (None, None) => {}
                }
            });
            keep
        });
    });
}

/// End of the frame of a viewport of its own (Preferences, the split-screen browser).
pub fn viewport_end(ctx: &egui::Context) {
    widgets_end_frame(ctx);
    focus_end_frame(ctx);
    keys_end_frame(ctx);
}

// ------------------------------------------------------------------ frame hooks

/// main.rs `raw_input_hook`: trackpad scrolling 1:1, film-strip pictures, the feel test.
pub fn raw_input(app: &mut App, raw: &mut egui::RawInput) {
    if motion::extras() {
        // egui treats a trackpad step of 8 pt or more as a wheel notch and spreads it over
        // 0.1 s, which smears fast swipes and lags them behind the fingers; macOS already
        // gives trackpad scrolling its momentum. Split such steps so each is passed on as it is.
        let mut i = 0;
        while i < raw.events.len() {
            if let egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, modifiers } = raw.events[i] {
                let len = delta.length();
                if len >= 8.0 && !(modifiers.ctrl || modifiers.mac_cmd || modifiers.command) {
                    let n = (len / 7.5).ceil() as usize;
                    let part = delta / n as f32;
                    raw.events[i] = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: part, modifiers };
                    for _ in 1..n {
                        raw.events.insert(i + 1, egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: part, modifiers });
                    }
                    i += n;
                    continue;
                }
            }
            i += 1;
        }
    }
    crate::feel_selftest::raw_input(app, raw);
    dialog_keys_input(&app.ctx, raw);
}

/// main.rs, at the start of `update`.
pub fn begin_frame(app: &mut App, ctx: &egui::Context) {
    let shot = app.screenshot.as_ref().map(|(_, f)| {
        let bench: u32 = bench_frames();
        let pic = 120 + bench;
        // From five frames before the picture is asked for, everything snaps to its target.
        (*f, *f + 5 >= pic && (app.rb_job.is_none() || *f + 5 >= 1200 + bench))
    });
    motion::begin_frame(ctx, &app.db, shot);
    HANDLED.with(|h| h.borrow_mut().clear());
    // The recorder's engine (buffers of a few MB) was made on the first frames, on this thread
    // (8 ms); it is made on a thread of its own at launch instead, as live as before.
    static WARM: std::sync::Once = std::sync::Once::new();
    WARM.call_once(|| {
        let _ = std::thread::Builder::new().name("rec-warm".into()).spawn(|| {
            crate::recorder::engine();
        });
    });
    KB_TREE.with(|t| t.set(app.dialogs.ext.focus_tree));
    FRAME_LIGHT.with(|l| l.set(crate::theme::is_light()));
    ctx.input(|i| kb_input(&i.events));
    FRAME_NO.with(|f| f.set(f.get() + 1));
    poll_artwork(app);
    crate::feel_selftest::frame_start(app, ctx);
}

/// main.rs, at the end of `update`.
pub fn end_frame(app: &mut App, ctx: &egui::Context) {
    drops_end_frame(app, ctx);
    watch_app(app, ctx);
    paint_veils(ctx);
    paint_swaps(ctx);
    widgets_end_frame(ctx);
    focus_end_frame(ctx);
    keys_end_frame(ctx);
    crate::feel_selftest::frame_end(app, ctx);
    motion::end_frame(ctx);
}

fn bench_frames() -> u32 {
    static B: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *B.get_or_init(|| std::env::var("PREPDECK_BENCH").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

// ------------------------------------------------------------------ Preferences

/// Preferences > Advanced > Others, under the "recordbox" heading: the interface motion level.
pub fn prefs_item(app: &mut App, ui: &mut Ui) {
    let cur = motion::setting();
    let mut pick = None;
    crate::ui_prefs_item::item(ui, "RecordboxMotion", 170.0, |it| {
        it.title("Interface motion");
        it.hint("recordbox's own: how buttons, faders, lists, drags and panels respond and move. rekordbox has no such setting; Off draws everything exactly as rekordbox does.");
        for (i, l) in [motion::Level::Full, motion::Level::Reduced, motion::Level::Off].into_iter().enumerate() {
            if it.radio(52.0, 56.0 + 26.0 * i as f32, cur == l, l.name()) {
                pick = Some(l);
            }
        }
        it.no_hint();
        let note = if motion::system_reduce_motion() { "macOS asks for reduced motion: Full is shown as Reduced." } else { "Reduced keeps fades and drops movement (also chosen by macOS Reduce motion)." };
        it.text(52.0, 136.0, 556.0, 18.0, note, 14.0, crate::ui_kit::Just::Left);
    });
    if let Some(l) = pick {
        if l != cur {
            motion::save_setting(&app.db, l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(c: u32, e: f32) -> Look {
        Look { rect: Rect::from_min_size(pos2(10.0, 10.0), vec2(80.0, 20.0)).expand(e), fill: crate::theme::hex(c), stroke: Stroke::NONE, radius: egui::CornerRadius::same(3) }
    }

    #[test]
    fn looks_mix_exactly_at_the_ends() {
        let (a, b) = (Some(look(0x282828, 0.0)), Some(look(0x3A3A3A, 1.0)));
        assert!(lerp_look(a, b, 0.0) == a);
        assert!(lerp_look(a, b, 1.0) == b);
        let m = lerp_look(a, b, 0.5).unwrap();
        assert!(m.rect.width() > 80.0 && m.rect.width() < 82.0);
        // From nothing drawn (a menu item at rest): the hovered look faded.
        let f = lerp_look(None, b, 0.5).unwrap();
        assert!(f.fill.a() < 255 && f.fill.a() > 0);
        assert!(lerp_look(None, b, 0.0).is_none());
    }

    #[test]
    fn frame_rects_are_found_by_centre_and_even_growth() {
        let r = Rect::from_min_size(pos2(10.0, 10.0), vec2(80.0, 20.0));
        assert!(same_frame(r.expand(1.0), r));
        assert!(same_frame(r, r));
        assert!(!same_frame(r.translate(vec2(1.0, 0.0)), r));
        assert!(!same_frame(Rect::from_min_size(r.min, vec2(81.0, 20.0)), r));
    }

    #[test]
    fn dialog_key_roles() {
        assert_eq!(key_role("OK"), Some(KeyRole::Return));
        assert_eq!(key_role("Cancel"), Some(KeyRole::Escape));
        assert_eq!(key_role("No"), Some(KeyRole::Escape));
        assert_eq!(key_role("Delete"), None);
    }
}
