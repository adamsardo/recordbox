//! recordbox's own interface motion (not rekordbox's: rekordbox snaps every state change).
//!
//! Time-based easing keyed by widget id. A widget asks for the value to show this frame
//! (`toward` for a 0..1 state such as hover, `tween` for any number that jumps, `follow` for a
//! number that moves continuously); the store remembers where each one was, eases it with
//! wall-clock time (so 60 Hz and 120 Hz look the same), and counts what has not arrived yet.
//! `end_frame` asks for the next frame only while something is still moving, so the app idles
//! at zero repaints. Settled values are exactly the target, so a screen at rest is drawn with
//! exactly the colours and positions it had before this module existed.
//!
//! Levels (Preferences > Advanced > Others > recordbox > Interface motion, stored under the
//! library setting `recordbox_motion`): Full; Reduced (colour and opacity fades stay, shorter;
//! movement, sliding and smooth scrolling go), also chosen when macOS "Reduce motion" is on;
//! Off (every value is its target: the app behaves and draws exactly as before).
//!
//! `--screenshot` runs: Full unless `PREPDECK_SHOT_MOTION=off|reduced|full` says otherwise,
//! and every value snaps to its target from five frames before the picture (`settle`).

use eframe::egui;
use egui::{Color32, Id, Rgba};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering::Relaxed};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Full = 0,
    Reduced = 1,
    Off = 2,
}

impl Level {
    pub fn from_u8(v: u8) -> Level {
        match v {
            1 => Level::Reduced,
            2 => Level::Off,
            _ => Level::Full,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Level::Full => "Full",
            Level::Reduced => "Reduced",
            Level::Off => "Off",
        }
    }
}

/// What an animation changes: a colour or an opacity (kept, shorter, in Reduced) or a
/// position or size (dropped in Reduced).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Fade,
    Move,
}

// ---- Durations, chosen once (seconds).
/// Hover coming on: quick, so the pointer never waits for it.
pub const HOVER_IN: f32 = 0.07;
/// Hover going off and a press letting go: a little slower, so leaving feels soft.
pub const HOVER_OUT: f32 = 0.16;
/// A press released.
pub const RELEASE: f32 = 0.14;
/// Selection colour, drop-target highlights, read-outs.
pub const FEEDBACK: f32 = 0.11;
/// A row, page or tab becoming selected (going off: `HOVER_OUT`).
pub const SELECT: f32 = 0.066;
/// A control's drawn value travelling to a value set by a click or a reset.
pub const GLIDE: f32 = 0.12;
/// Panels, pages, modes and layouts appearing.
pub const PANEL: f32 = 0.18;
/// Smooth scrolling to a row, eased zoom.
pub const SCROLL: f32 = 0.14;
/// A refused drag going back to where it came from.
pub const SNAP_BACK: f32 = 0.22;
/// Nothing lasts longer than this.
pub const MAX: f32 = 0.25;

static SETTING: AtomicU8 = AtomicU8::new(0);
static SETTING_LOADED: AtomicBool = AtomicBool::new(false);
static SYSTEM_REDUCE: AtomicBool = AtomicBool::new(false);
static LEVEL: AtomicU8 = AtomicU8::new(2);
static SETTLE: AtomicBool = AtomicBool::new(false);
/// Values still moving in the last finished frame (tools and tests read it).
static LAST_ACTIVE: AtomicU32 = AtomicU32::new(0);

/// The level the user chose (before macOS "Reduce motion" is applied).
pub fn setting() -> Level {
    Level::from_u8(SETTING.load(Relaxed))
}

pub fn set_setting(l: Level) {
    SETTING.store(l as u8, Relaxed);
}

/// The level in effect this frame.
#[inline]
pub fn level() -> Level {
    Level::from_u8(LEVEL.load(Relaxed))
}

/// True when anything eases this frame (not Off, not settling for a screenshot).
#[inline]
pub fn on() -> bool {
    LEVEL.load(Relaxed) != Level::Off as u8 && !SETTLE.load(Relaxed)
}

/// True when things may move (Full only).
#[inline]
pub fn moves() -> bool {
    LEVEL.load(Relaxed) == Level::Full as u8 && !SETTLE.load(Relaxed)
}

/// True when recordbox's added feedback (row hover, read-outs, drop glow, cursors) is drawn:
/// any level but Off. Unlike `on`, it stays true while settling, so a screenshot taken with
/// the pointer over something shows the settled feedback.
#[inline]
pub fn extras() -> bool {
    LEVEL.load(Relaxed) != Level::Off as u8
}

/// Whether macOS asks apps to reduce motion (System Settings > Accessibility > Display).
pub fn system_reduce_motion() -> bool {
    SYSTEM_REDUCE.load(Relaxed)
}

/// Values that had not arrived at the end of the last frame.
pub fn last_active() -> u32 {
    LAST_ACTIVE.load(Relaxed)
}

#[derive(Clone, Copy)]
struct Anim {
    from: f32,
    to: f32,
    t0: f64,
    dur: f32,
    seen: u64,
}

struct Store {
    map: egui::IdMap<Anim>,
    now: f64,
    prev: f64,
    dt: f32,
    lead: f32,
    frame: u64,
    active: u32,
    last_poll: f64,
    /// The display clock: where the frame being built will be shown (see `display_time`).
    disp: f64,
    disp_step: f64,
    period: f64,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store { map: egui::IdMap::default(), now: 0.0, prev: 0.0, dt: 1.0 / 60.0, lead: 1.0 / 60.0, frame: 0, active: 0, last_poll: -10.0, disp: 0.0, disp_step: 0.0, period: 1.0 / 120.0 });
}

/// Start of a frame (main.rs, before anything is drawn). `shot`: the frame number of a
/// `--screenshot` run and whether its picture is about to be taken.
pub fn begin_frame(ctx: &egui::Context, db: &crate::db::Db, shot: Option<(u32, bool)>) {
    let (now, dt) = ctx.input(|i| (i.time, i.stable_dt));
    if !SETTING_LOADED.swap(true, Relaxed) {
        let v = db.setting("recordbox_motion").and_then(|s| s.trim().parse::<u8>().ok()).unwrap_or(0);
        SETTING.store(v.min(2), Relaxed);
    }
    // The time `pace_frame` waited for the display is part of this frame's time.
    let now = now + PACED_US.load(Relaxed) as f64 / 1e6;
    let poll = STORE.with(|s| {
        let mut s = s.borrow_mut();
        step_clock(&mut s, now, dt);
        if s.frame % 240 == 0 {
            let f = s.frame;
            s.map.retain(|_, a| f.saturating_sub(a.seen) < 1200);
        }
        if s.map.capacity() == 0 {
            s.map.reserve(2048);
        }
        let due = now - s.last_poll > 2.0;
        if due {
            s.last_poll = now;
        }
        due
    });
    if poll && shot.is_none() {
        SYSTEM_REDUCE.store(query_reduce_motion(), Relaxed);
    }
    let mut l = setting();
    if let Some((_, settle)) = shot {
        l = shot_level();
        SETTLE.store(settle, Relaxed);
    } else {
        SETTLE.store(false, Relaxed);
        if l == Level::Full && SYSTEM_REDUCE.load(Relaxed) {
            l = Level::Reduced;
        }
    }
    LEVEL.store(l as u8, Relaxed);
}

// ---- Frame pacing.
//
// A playing deck asks for the next frame at once, every frame. eframe then starts it as soon as
// the last one has been handed to OpenGL, and on macOS that hand-over (`swap_buffers`) takes
// a second frame without waiting and then waits almost two refreshes: measured on a playing
// deck at 120 Hz, frames started 3 ms and 13.7 ms apart in turn (two per 16.7 ms), with a lone
// 8.3 ms now and then. Here a frame starts at most once per refresh of the display, timed by
// the display itself (CVDisplayLink): a frame that would start inside the refresh the last one
// started in waits for the next refresh (never longer than one period and a bit). A frame
// after a pause starts at once; nothing is drawn more often than before, only less bunched.

static PACE_WAITS: AtomicU32 = AtomicU32::new(0);
/// This frame's refresh number and how far into that refresh it started (tools read it).
static FRAME_TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
static FRAME_PHASE_US: AtomicU32 = AtomicU32::new(0);

/// The display refresh this frame started in, how far into it (seconds) and the refresh period;
/// None without a display link.
pub fn frame_refresh() -> Option<(u64, f64, f64)> {
    let t = FRAME_TICK.load(Relaxed);
    (t != u64::MAX).then(|| (t, FRAME_PHASE_US.load(Relaxed) as f64 / 1e6, link_period()))
}

#[cfg(target_os = "macos")]
fn link_period() -> f64 {
    vsync::period()
}

#[cfg(not(target_os = "macos"))]
fn link_period() -> f64 {
    0.0
}
static PACED_US: AtomicU32 = AtomicU32::new(0);
static PACED_PREV_US: AtomicU32 = AtomicU32::new(0);

/// Whether OpenGL's swap waits for the display (eframe's `vsync`). On macOS it does not: that
/// wait is what bunched the frames, and `pace_frame` gives one frame per refresh instead
/// (`PREPDECK_VSYNC=on` brings the old swap back). Elsewhere it does, as before.
pub fn swap_waits() -> bool {
    !cfg!(target_os = "macos") || std::env::var("PREPDECK_VSYNC").as_deref() == Ok("on") || pace_off()
}

/// `PREPDECK_PACE=off` (measurements): the old way, the swap waits and frames are not paced;
/// the display link only records which refresh each frame started in.
fn pace_off() -> bool {
    static P: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *P.get_or_init(|| std::env::var("PREPDECK_PACE").as_deref() == Ok("off"))
}

/// main.rs, at the start of `update` (before its frame timer): one frame per display refresh.
/// This is the frame scheduler, not interface motion: it runs at every motion level (with the
/// swap no longer waiting, nothing else would hold frames to the display's rate).
pub fn pace_frame(ctx: &egui::Context, _shot: bool) {
    let w = pace(ctx);
    PACED_PREV_US.store(PACED_US.load(Relaxed), Relaxed);
    PACED_US.store((w * 1e6) as u32, Relaxed);
}

/// Frames that waited for the display so far (tools read it).
pub fn pace_waits() -> u32 {
    PACE_WAITS.load(Relaxed)
}

#[cfg(target_os = "macos")]
mod vsync {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering::*};
    use std::sync::{Condvar, Mutex};

    type Link = *mut c_void;
    type Callback = extern "C" fn(Link, *const c_void, *const c_void, u64, *mut u64, *mut c_void) -> i32;
    #[link(name = "CoreVideo", kind = "framework")]
    extern "C" {
        fn CVDisplayLinkCreateWithActiveCGDisplays(out: *mut Link) -> i32;
        fn CVDisplayLinkSetOutputCallback(link: Link, cb: Callback, user: *mut c_void) -> i32;
        fn CVDisplayLinkSetCurrentCGDisplay(link: Link, display: u32) -> i32;
        fn CVDisplayLinkStart(link: Link) -> i32;
        fn CVDisplayLinkStop(link: Link) -> i32;
        fn CVDisplayLinkGetActualOutputVideoRefreshPeriod(link: Link) -> f64;
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGGetDisplaysWithPoint(p: CGPoint, max: u32, out: *mut u32, count: *mut u32) -> i32;
    }

    static LINK: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static RUNNING: AtomicBool = AtomicBool::new(false);
    static DISPLAY: AtomicU64 = AtomicU64::new(u64::MAX);
    /// Refreshes seen, and when the link was last wanted (ms since `T0`).
    static TICK: (Mutex<u64>, Condvar) = (Mutex::new(0), Condvar::new());
    /// When the last refresh callback came (ns since `T0`).
    static TICK_AT_NS: AtomicU64 = AtomicU64::new(0);
    static WANTED_MS: AtomicU64 = AtomicU64::new(0);
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

    fn ms() -> u64 {
        T0.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
    }

    extern "C" fn on_refresh(_: Link, _: *const c_void, _: *const c_void, _: u64, _: *mut u64, _: *mut c_void) -> i32 {
        TICK_AT_NS.store(T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64, Relaxed);
        let (m, c) = &TICK;
        if let Ok(mut t) = m.lock() {
            *t += 1;
        }
        c.notify_all();
        0
    }

    /// The link, running (made on first use; stopped by its watcher after a second unused).
    fn ensure() -> Option<Link> {
        WANTED_MS.store(ms(), Relaxed);
        let mut l = LINK.load(Acquire);
        if l.is_null() {
            let mut out: Link = std::ptr::null_mut();
            if unsafe { CVDisplayLinkCreateWithActiveCGDisplays(&mut out) } != 0 || out.is_null() {
                return None;
            }
            unsafe { CVDisplayLinkSetOutputCallback(out, on_refresh, std::ptr::null_mut()) };
            LINK.store(out, Release);
            l = out;
        }
        if !RUNNING.swap(true, Relaxed) {
            unsafe { CVDisplayLinkStart(l) };
            // Stops the link once frames have not wanted it for a second, then ends.
            let _ = std::thread::Builder::new().name("vsync-watch".into()).spawn(|| loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if ms().saturating_sub(WANTED_MS.load(Relaxed)) > 1000 {
                    unsafe { CVDisplayLinkStop(LINK.load(Acquire)) };
                    RUNNING.store(false, Relaxed);
                    return;
                }
            });
        }
        Some(l)
    }

    /// Follow the display the window is on (`p`: a point of the window, global points).
    pub fn follow(p: (f64, f64)) {
        let Some(l) = ensure() else { return };
        let (mut id, mut n) = (0u32, 0u32);
        if unsafe { CGGetDisplaysWithPoint(CGPoint { x: p.0, y: p.1 }, 1, &mut id, &mut n) } == 0 && n == 1 && DISPLAY.swap(id as u64, Relaxed) != id as u64 {
            unsafe { CVDisplayLinkSetCurrentCGDisplay(l, id) };
        }
    }

    /// The refresh count now (starting the link if needed).
    pub fn tick() -> Option<u64> {
        ensure()?;
        TICK.0.lock().ok().map(|t| *t)
    }

    /// Wait until the count passes `seen`, at most `max` seconds. Returns the seconds waited.
    pub fn wait_past(seen: u64, max: f64) -> f64 {
        let t = std::time::Instant::now();
        let (m, c) = &TICK;
        let Ok(g) = m.lock() else { return 0.0 };
        let _ = c.wait_timeout_while(g, std::time::Duration::from_secs_f64(max), |n| *n <= seen);
        t.elapsed().as_secs_f64()
    }

    /// Seconds since the last refresh callback (None before the first).
    pub fn since_tick() -> Option<f64> {
        let at = TICK_AT_NS.load(Relaxed);
        (at > 0).then(|| (T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64).saturating_sub(at) as f64 / 1e9)
    }

    pub fn period() -> f64 {
        let l = LINK.load(Acquire);
        if l.is_null() { 0.0 } else { unsafe { CVDisplayLinkGetActualOutputVideoRefreshPeriod(l) } }
    }
}

thread_local! {
    static LAST_TICK: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static LAST_START: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    static FOLLOW_AT: std::cell::Cell<f64> = const { std::cell::Cell::new(-10.0) };
}

/// Start of a frame: wait for the next refresh if this frame would start inside the one the
/// last frame started in. Returns the seconds waited.
#[cfg(target_os = "macos")]
fn pace(ctx: &egui::Context) -> f64 {
    let t = ctx.input(|i| i.time);
    if t - FOLLOW_AT.with(|f| f.get()) > 2.0 {
        FOLLOW_AT.with(|f| f.set(t));
        if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
            vsync::follow(((r.min.x + 40.0) as f64, (r.min.y + 40.0) as f64));
        }
    }
    let Some(now) = vsync::tick() else { return cap_rate() };
    let last = LAST_TICK.with(|l| l.get());
    let p = vsync::period();
    let p = if p > 0.002 && p < 0.05 { p } else { 1.0 / 60.0 };
    // A frame after a pause (the link may have been stopped meanwhile) starts at once.
    let recent = LAST_START.with(|l| l.get()).is_some_and(|t| t.elapsed().as_secs_f64() < 2.0 * p);
    let mut waited = 0.0;
    if last == Some(now) && recent && !pace_off() {
        waited = vsync::wait_past(now, p * 1.25);
        PACE_WAITS.fetch_add(1, Relaxed);
    }
    let t = vsync::tick();
    LAST_TICK.with(|l| l.set(t));
    LAST_START.with(|l| l.set(Some(std::time::Instant::now())));
    FRAME_TICK.store(t.unwrap_or(u64::MAX), Relaxed);
    FRAME_PHASE_US.store(vsync::since_tick().map_or(0, |s| (s * 1e6) as u32), Relaxed);
    waited
}

#[cfg(not(target_os = "macos"))]
fn pace(_: &egui::Context) -> f64 {
    0.0
}

/// No display link (it could not be made): frames at most 120 a second, by the clock.
#[allow(dead_code)]
fn cap_rate() -> f64 {
    thread_local! { static LAST: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) }; }
    let min = std::time::Duration::from_secs_f64(1.0 / 120.0);
    let mut waited = 0.0;
    if let Some(l) = LAST.with(|l| l.get()) {
        let d = l.elapsed();
        if d < min {
            std::thread::sleep(min - d);
            waited = (min - d).as_secs_f64();
        }
    }
    LAST.with(|l| l.set(Some(std::time::Instant::now())));
    waited
}

/// The frame clock: the measured interval since the last frame, or (after idling, when there
/// was no last frame to measure from) egui's predicted one, never more than a 60 Hz frame. The
/// first frame of a change shows that much progress.
fn step_clock(s: &mut Store, now: f64, stable_dt: f32) {
    let gap = (now - s.now) as f32;
    s.prev = s.now;
    s.now = now;
    s.dt = if gap > 0.0 && gap <= 0.1 { gap } else { stable_dt.clamp(0.0, 1.0 / 60.0) };
    s.lead = s.dt.min(1.0 / 60.0);
    s.frame += 1;
    s.active = 0;
    // The display clock. Frames are started unevenly (measured with the bench: about a third
    // 0-4 ms apart, a third 8 ms, a third 12-15 ms at 120 Hz), while the display shows them one
    // refresh apart. Each frame advances this clock by whole refresh periods (at least one),
    // so something that moves with it moves by the same step every frame; it is pulled back
    // onto real time when it drifts by more than two periods.
    let g = gap as f64;
    if gap > 0.0 && gap <= 0.1 {
        s.period += (g.min(1.0 / 24.0) - s.period) * 0.02;
        // One period per frame; more only when the frame is late by over 1.4 periods (a frame
        // really missed), back onto real time when the clock got ahead of it.
        let mut next = s.disp + s.period;
        let e = now - next;
        if e > 1.4 * s.period {
            next += (e / s.period).round() * s.period;
        }
        if (next - now).abs() > 2.5 * s.period {
            next = now;
        }
        s.disp_step = next - s.disp;
        s.disp = next;
    } else {
        s.disp_step = 0.0;
        s.disp = now;
    }
}

static LAST_CPU_US: AtomicU32 = AtomicU32::new(0);

/// main.rs, first thing in `update`: how long the last frame took on the CPU without waiting
/// for the display (eframe's `cpu_usage`: our update, tessellation and painting).
pub fn frame_cpu(secs: Option<f32>) {
    LAST_CPU_US.store(secs.map_or(0, |s| (s * 1e6) as u32), Relaxed);
}

/// The last frame's CPU time in ms, without the time it waited for the display (tools read it).
pub fn last_cpu_ms() -> f32 {
    LAST_CPU_US.load(Relaxed).saturating_sub(PACED_PREV_US.load(Relaxed)) as f32 / 1000.0
}

/// The display clock (seconds, egui's time base): advances by whole refresh periods per frame.
pub fn display_time() -> f64 {
    STORE.with(|s| s.borrow().disp)
}

/// How far the display clock moved this frame (0 on the first frame after idling).
pub fn display_step() -> f64 {
    STORE.with(|s| s.borrow().disp_step)
}

/// The estimated refresh period (seconds).
pub fn period() -> f64 {
    STORE.with(|s| s.borrow().period)
}

/// End of a frame: the one place that asks for another frame while something moves.
pub fn end_frame(ctx: &egui::Context) {
    let active = STORE.with(|s| s.borrow().active);
    LAST_ACTIVE.store(active, Relaxed);
    if active > 0 {
        ctx.request_repaint();
    }
}

fn shot_level() -> Level {
    static L: std::sync::OnceLock<Level> = std::sync::OnceLock::new();
    *L.get_or_init(|| match std::env::var("PREPDECK_SHOT_MOTION").as_deref() {
        Ok("off") => Level::Off,
        Ok("reduced") => Level::Reduced,
        _ => Level::Full,
    })
}

/// Seconds an animation of `kind` takes at the current level (0 = jump).
fn scaled(dur: f32, kind: Kind) -> f32 {
    if !on() {
        return 0.0;
    }
    match (level(), kind) {
        (Level::Full, _) => dur.min(MAX),
        (Level::Reduced, Kind::Fade) => (dur * 0.7).min(MAX),
        _ => 0.0,
    }
}

#[inline]
fn ease_out(p: f32) -> f32 {
    let q = 1.0 - p;
    1.0 - q * q * q
}

#[inline]
fn ease_in_out(p: f32) -> f32 {
    if p < 0.5 {
        4.0 * p * p * p
    } else {
        let q = -2.0 * p + 2.0;
        1.0 - q * q * q / 2.0
    }
}

fn sample(a: &Anim, now: f64) -> (f32, bool) {
    if a.dur <= 0.0 {
        return (a.to, true);
    }
    let p = ((now - a.t0) as f32 / a.dur).clamp(0.0, 1.0);
    if p >= 1.0 {
        (a.to, true)
    } else {
        (a.from + (a.to - a.from) * ease_out(p), false)
    }
}

/// A 0..1 state shown eased: `rise` seconds going up, `fall` coming down (a part of the way
/// takes that part of the time). Unseen ids start at their target.
pub fn toward(id: Id, target: f32, rise: f32, fall: f32, kind: Kind) -> f32 {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let (now, lead, frame) = (s.now, s.lead, s.frame);
        let a = s.map.entry(id).or_insert(Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame });
        a.seen = frame;
        if a.to != target {
            let (cur, _) = sample(a, now);
            let d = if target > cur { scaled(rise, kind) } else { scaled(fall, kind) };
            *a = Anim { from: cur, to: target, t0: now - lead as f64, dur: d * (target - cur).abs().min(1.0), seen: frame };
        } else if !on() {
            a.dur = 0.0;
        }
        let (v, done) = sample(a, now);
        if !done {
            s.active += 1;
        }
        v
    })
}

/// A value that jumps (a click, a reset, a new scroll target) shown travelling to it over
/// `dur` seconds; a new target mid-way starts from where the value is drawn.
pub fn tween(id: Id, target: f32, dur: f32, kind: Kind) -> f32 {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let (now, lead, frame) = (s.now, s.lead, s.frame);
        let a = s.map.entry(id).or_insert(Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame });
        a.seen = frame;
        if a.to != target {
            let (cur, _) = sample(a, now);
            *a = Anim { from: cur, to: target, t0: now - lead as f64, dur: scaled(dur, kind), seen: frame };
        } else if !on() {
            a.dur = 0.0;
        }
        let (v, done) = sample(a, now);
        if !done {
            s.active += 1;
        }
        v
    })
}

/// Like `tween`, but a change smaller than `jump` is followed at once (a drag tracks the
/// pointer exactly; only a jump, such as a click on the track, glides).
pub fn glide(id: Id, target: f32, jump: f32, dur: f32, kind: Kind) -> f32 {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let (now, lead, frame) = (s.now, s.lead, s.frame);
        let a = s.map.entry(id).or_insert(Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame });
        a.seen = frame;
        if a.to != target {
            let (cur, done) = sample(a, now);
            if (target - a.to).abs() >= jump {
                *a = Anim { from: cur, to: target, t0: now - lead as f64, dur: scaled(dur, kind), seen: frame };
            } else if done {
                *a = Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame };
            } else {
                // A small move while gliding: keep gliding, to the new place.
                a.to = target;
            }
        } else if !on() {
            a.dur = 0.0;
        }
        let (v, done) = sample(a, now);
        if !done {
            s.active += 1;
        }
        v
    })
}

/// A value that moves on its own every frame, shown with frame-rate independent exponential
/// smoothing (`tau` seconds to cover 63 % of the way); it arrives exactly once it is within
/// `eps`.
pub fn follow(id: Id, target: f32, tau: f32, eps: f32, kind: Kind) -> f32 {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let (now, dt, frame) = (s.now, s.dt, s.frame);
        let a = s.map.entry(id).or_insert(Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame });
        let tau = scaled(tau, kind);
        let gap = (frame - a.seen).max(1);
        a.seen = frame;
        if tau <= 0.0 || gap > 2 {
            a.from = target;
        } else {
            let k = 1.0 - (-dt / tau).exp();
            a.from += (target - a.from) * k;
            if (target - a.from).abs() <= eps {
                a.from = target;
            }
        }
        a.to = target;
        let v = a.from;
        if v != target {
            s.active += 1;
        }
        v
    })
}

/// Set a value at once (a press shows on its own frame).
pub fn set(id: Id, v: f32) {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let (now, frame) = (s.now, s.frame);
        s.map.insert(id, Anim { from: v, to: v, t0: now, dur: 0.0, seen: frame });
    })
}

/// A one-off pulse: 1 at once, back to 0 over `dur` seconds (a drop accepted, a reset).
pub fn pulse(id: Id, dur: f32) {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let (now, lead, frame) = (s.now, s.lead, s.frame);
        let d = scaled(dur, Kind::Fade);
        s.map.insert(id, Anim { from: 1.0, to: 0.0, t0: now - lead as f64, dur: d, seen: frame });
    })
}

/// The current value of a pulse (0 when none is running).
pub fn pulse_value(id: Id) -> f32 {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let (now, frame) = (s.now, s.frame);
        let Some(a) = s.map.get_mut(&id) else { return 0.0 };
        a.seen = frame;
        let (v, done) = sample(a, now);
        if !done {
            s.active += 1;
        }
        v
    })
}

/// The frame's time (egui's input clock, seconds).
pub fn now() -> f64 {
    STORE.with(|s| s.borrow().now)
}

/// One frame's worth of time for the first step of a change (see `step_clock`).
pub fn frame_lead() -> f32 {
    STORE.with(|s| s.borrow().lead)
}

/// The previous frame's time.
pub fn now_prev() -> f64 {
    STORE.with(|s| s.borrow().prev)
}

/// Count something drawn this frame that is still moving outside the store (keeps frames
/// coming until it ends).
pub fn keep_alive() {
    STORE.with(|s| s.borrow_mut().active += 1);
}

/// Ease-in-out of a 0..1 progress, for callers that draw their own timeline.
pub fn ease(p: f32) -> f32 {
    ease_in_out(p.clamp(0.0, 1.0))
}

/// `a` to `b` by `t` in linear light (the endpoints are returned exactly).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    if t <= 0.0 || a == b {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let (x, y) = (Rgba::from(a).to_array(), Rgba::from(b).to_array());
    let l = |i: usize| x[i] + (y[i] - x[i]) * t;
    Color32::from(Rgba::from_rgba_premultiplied(l(0), l(1), l(2), l(3)))
}

/// A colour with its opacity multiplied by `t` (linear, premultiplied; exact at 0 and 1).
pub fn fade(c: Color32, t: f32) -> Color32 {
    if t >= 1.0 {
        c
    } else if t <= 0.0 {
        Color32::TRANSPARENT
    } else {
        c.linear_multiply(t)
    }
}

/// `NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion`.
#[cfg(target_os = "macos")]
fn query_reduce_motion() -> bool {
    use objc2::runtime::{AnyClass, AnyObject};
    let Some(cls) = AnyClass::get(c"NSWorkspace") else { return false };
    unsafe {
        let ws: *mut AnyObject = objc2::msg_send![cls, sharedWorkspace];
        if ws.is_null() {
            return false;
        }
        let on: bool = objc2::msg_send![ws, accessibilityDisplayShouldReduceMotion];
        on
    }
}

#[cfg(not(target_os = "macos"))]
fn query_reduce_motion() -> bool {
    false
}

/// Store the chosen level in the library database.
pub fn save_setting(db: &crate::db::Db, l: Level) {
    set_setting(l);
    db.set_setting("recordbox_motion", &(l as u8).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The level is one global: the tests that set it run one at a time.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn frame(ctx: &egui::Context, t: f64, f: impl FnOnce()) {
        let raw = egui::RawInput { time: Some(t), predicted_dt: 1.0 / 120.0, ..Default::default() };
        let mut f = Some(f);
        let _ = ctx.run(raw, |ctx| {
            let (now, dt) = ctx.input(|i| (i.time, i.stable_dt));
            STORE.with(|s| step_clock(&mut s.borrow_mut(), now, dt));
            if let Some(f) = f.take() {
                f();
            }
        });
    }

    #[test]
    fn hover_eases_and_settles_exactly() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        LEVEL.store(Level::Full as u8, Relaxed);
        SETTLE.store(false, Relaxed);
        let ctx = egui::Context::default();
        let id = Id::new("t_hover");
        let mut v = 0.0;
        frame(&ctx, 0.0, || v = toward(id, 0.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert_eq!(v, 0.0);
        frame(&ctx, 1.0, || v = toward(id, 1.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert!(v > 0.0 && v < 1.0, "first frame shows some progress: {v}");
        frame(&ctx, 1.03, || v = toward(id, 1.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert!(v > 0.5 && v < 1.0, "{v}");
        frame(&ctx, 1.2, || v = toward(id, 1.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert_eq!(v, 1.0);
        // Off: always the target.
        LEVEL.store(Level::Off as u8, Relaxed);
        frame(&ctx, 1.21, || v = toward(id, 0.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert_eq!(v, 0.0);
        LEVEL.store(Level::Full as u8, Relaxed);
    }

    #[test]
    fn reduced_keeps_fades_and_drops_moves() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        LEVEL.store(Level::Reduced as u8, Relaxed);
        SETTLE.store(false, Relaxed);
        let ctx = egui::Context::default();
        let (a, b) = (Id::new("t_fade"), Id::new("t_move"));
        let (mut f, mut m) = (0.0, 0.0);
        frame(&ctx, 0.0, || {
            toward(a, 0.0, 0.1, 0.1, Kind::Fade);
            tween(b, 0.0, 0.1, Kind::Move);
        });
        frame(&ctx, 0.5, || {
            f = toward(a, 1.0, 0.1, 0.1, Kind::Fade);
            m = tween(b, 50.0, 0.1, Kind::Move);
        });
        assert!(f < 1.0);
        assert_eq!(m, 50.0);
        LEVEL.store(Level::Full as u8, Relaxed);
    }

    #[test]
    fn glide_tracks_small_moves_exactly() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        LEVEL.store(Level::Full as u8, Relaxed);
        SETTLE.store(false, Relaxed);
        let ctx = egui::Context::default();
        let id = Id::new("t_glide");
        let mut v = 0.0;
        frame(&ctx, 0.0, || v = glide(id, 10.0, 4.0, GLIDE, Kind::Move));
        frame(&ctx, 0.01, || v = glide(id, 11.0, 4.0, GLIDE, Kind::Move));
        assert_eq!(v, 11.0, "a drag step is followed at once");
        frame(&ctx, 0.02, || v = glide(id, 40.0, 4.0, GLIDE, Kind::Move));
        assert!(v > 11.0 && v < 40.0, "a jump glides: {v}");
        frame(&ctx, 0.5, || v = glide(id, 40.0, 4.0, GLIDE, Kind::Move));
        assert_eq!(v, 40.0);
    }

    #[test]
    fn follow_is_frame_rate_independent() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        LEVEL.store(Level::Full as u8, Relaxed);
        SETTLE.store(false, Relaxed);
        let run = |hz: f64| {
            let ctx = egui::Context::default();
            let id = Id::new(("t_follow", hz as i32));
            let mut v = 0.0;
            frame(&ctx, 0.0, || v = follow(id, 0.0, 0.03, 0.01, Kind::Move));
            let n = (0.05 * hz) as i32;
            for k in 1..=n {
                frame(&ctx, k as f64 / hz, || v = follow(id, 100.0, 0.03, 0.01, Kind::Move));
            }
            v
        };
        let (a, b) = (run(60.0), run(120.0));
        assert!((a - b).abs() < 3.0, "60 Hz {a} vs 120 Hz {b}");
    }

    #[test]
    fn mix_is_exact_at_the_ends_and_linear_between() {
        let (a, b) = (Color32::from_rgb(0x28, 0x28, 0x28), Color32::from_rgb(0x3C, 0x3C, 0x3C));
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        let m = mix(Color32::BLACK, Color32::WHITE, 0.5);
        // Half way in linear light is #BCBCBC in sRGB, not #808080.
        assert!((m.r() as i32 - 0xBC).abs() <= 1, "{m:?}");
    }
}
