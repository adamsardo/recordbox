//! Tools for seeing recordbox's interface motion without taking over the screen (all with
//! `--screenshot` on a throwaway library copy):
//!
//! - `PREPDECK_SHOT_FRAMES=<n>,<every>[,<start>]` (`PREPDECK_SHOT_FRAMES_RECT=x,y,w,h` in
//!   points, default the whole window): a contact strip of `n` pictures, one every `every`
//!   frames from frame `start` (default 150), saved next to the screenshot as
//!   `<name>-strip.jpg` with the time of each picture in `<name>-strip.txt`.
//! - `PREPDECK_SHOT_PLAYER=feeltest PREPDECK_FEEL=<scenario>`: a scripted interaction with
//!   real pointer, wheel and key events (`hover`, `press`, `fader`, `knob`, `xfader`, `scroll`,
//!   `wheel`, `keys`, `drag`, `refuse`, `panel`, `mode`, `play`), printing "feeltest:" lines.
//! - Every `PREPDECK_BENCH` run also prints the intervals between frames (a histogram, and how
//!   many were longer than 1.5 frame periods) and, while a deck plays, how evenly the drawn
//!   position advanced.
//! - `PREPDECK_IDLE_PROBE=<seconds>`: from frame 150 the run stops asking for frames and counts
//!   the frames drawn anyway for that long ("idle: ..."), then quits.

use crate::app::App;
use crate::motion;
use eframe::egui;
use egui::{pos2, Pos2};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;

struct StripTag(usize, f64, u32);

#[derive(Default)]
struct Strip {
    n: usize,
    every: u32,
    start: u32,
    rect: Option<egui::Rect>,
    asked: usize,
    pics: Vec<(usize, f64, u32, image::RgbaImage)>,
    done: bool,
}

#[derive(Default)]
struct Stats {
    last: Option<std::time::Instant>,
    intervals: Vec<f32>,
    /// Per interval: the previous frame's CPU time (eframe's, without the swap).
    cpu: Vec<f32>,
    /// Per frame: the display refresh it started in and how far into it (s), the period.
    refresh: Vec<(u64, f64, f64)>,
    /// Per frame while a deck plays: (drawn position step, real position step) in seconds.
    steps: Vec<(f64, f64)>,
    last_pos: Option<(f64, f64)>,
    window: Option<(u32, u32)>,
}

static STRIP: Mutex<Option<Strip>> = Mutex::new(None);
static STATS: Mutex<Option<Stats>> = Mutex::new(None);
static IDLE_COUNT: AtomicU64 = AtomicU64::new(0);
static IDLE_ON: AtomicBool = AtomicBool::new(false);

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn scenario() -> Option<&'static str> {
    static S: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let on = std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "feeltest")).unwrap_or(false);
    S.get_or_init(|| if on { env("PREPDECK_FEEL") } else { None }).as_deref()
}

/// True while the idle probe runs: main.rs then does not ask for the next frame.
pub fn idle_probe() -> bool {
    IDLE_ON.load(Relaxed)
}

fn bench() -> u32 {
    env("PREPDECK_BENCH").and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn at_env(name: &str) -> Option<Pos2> {
    let v = env(name)?;
    let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
    Some(pos2(it.next()?, it.next()?))
}

pub fn frame_start(app: &mut App, ctx: &egui::Context) {
    let Some((_, frame)) = app.screenshot.clone() else { return };
    IDLE_COUNT.fetch_add(1, Relaxed);
    if frame == 150 {
        if let Some(secs) = env("PREPDECK_IDLE_PROBE").and_then(|v| v.parse::<f64>().ok()) {
            IDLE_ON.store(true, Relaxed);
            IDLE_COUNT.store(0, Relaxed);
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs_f64(secs));
                let n = IDLE_COUNT.load(Relaxed);
                let causes = ctx2.repaint_causes();
                eprintln!("idle: {n} frames drawn in {secs:.1} s with nothing asking (last causes: {:?})", causes.iter().take(4).map(|c| c.to_string()).collect::<Vec<_>>());
                std::process::exit(0);
            });
        }
    }
    // Strip settings.
    if frame == 2 {
        if let Some(v) = env("PREPDECK_SHOT_FRAMES") {
            let p: Vec<u32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            let rect = env("PREPDECK_SHOT_FRAMES_RECT").and_then(|r| {
                let q: Vec<f32> = r.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (q.len() == 4).then(|| egui::Rect::from_min_size(pos2(q[0], q[1]), egui::vec2(q[2], q[3])))
            });
            *STRIP.lock().unwrap() = Some(Strip { n: *p.first().unwrap_or(&12) as usize, every: *p.get(1).unwrap_or(&1).max(&1), start: *p.get(2).unwrap_or(&150), rect, ..Default::default() });
        }
        if bench() > 0 {
            *STATS.lock().unwrap() = Some(Stats::default());
        }
    }
    // Frame intervals while the bench runs.
    if let Some(st) = STATS.lock().unwrap().as_mut() {
        let now = std::time::Instant::now();
        let inside = st.window.map_or(frame > 90, |(a, b)| frame >= a && frame <= b);
        if let Some(l) = st.last {
            if inside {
                st.intervals.push(now.duration_since(l).as_secs_f32() * 1000.0);
                st.cpu.push(motion::last_cpu_ms());
                if let Some(r) = motion::frame_refresh() {
                    st.refresh.push(r);
                }
            }
        }
        st.last = Some(now);
    }
    if let Some(s) = scenario() {
        scenario_state(app, s, frame);
    }
}

pub fn frame_end(app: &mut App, ctx: &egui::Context) {
    let Some((path, frame)) = app.screenshot.clone() else { return };
    // Play steadiness: the drawn and the real position of the active deck.
    if bench() > 0 && app.playing() {
        let shown = crate::ui_feel::shown_pos(app);
        let real = app.pos();
        if let Some(st) = STATS.lock().unwrap().as_mut() {
            if let Some((ps, pr)) = st.last_pos {
                let inside = st.window.map_or(frame > 90, |(a, b)| frame >= a && frame <= b);
                if inside {
                    st.steps.push((shown - ps, real - pr));
                }
            }
            st.last_pos = Some((shown, real));
        }
    }
    if let Some(s) = scenario().filter(|s| matches!(*s, "wavedrag" | "wavescroll")) {
        if (T0..T0 + span(s)).contains(&frame) {
            let shown = crate::ui_feel::shown_pos(app);
            let ask = WAVE.with(|w| w.get()).zip(WAVE_X.with(|w| w.get())).map(|((r, _, pps), x)| (x - r.center().x) as f64 / pps);
            eprintln!("feeltest: {s} frame {} t {:.4} drawn {:.4} real {:.4} pointer-offset-s {}", frame - T0, ctx.input(|i| i.time), shown, app.pos(), ask.map_or("-".into(), |a| format!("{a:.4}")));
        }
    }
    let b = bench();
    if b > 0 && frame == 119 + b {
        report_stats(app);
    }
    // Strip pictures.
    let mut g = STRIP.lock().unwrap();
    if let Some(s) = g.as_mut() {
        if !s.done && frame >= s.start && (frame - s.start) % s.every == 0 && s.asked < s.n {
            let t = ctx.input(|i| i.time);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(StripTag(s.asked, t, frame))));
            s.asked += 1;
        }
        if !s.done && s.pics.len() >= s.n {
            s.done = true;
            save_strip(s, &path);
        }
    }
}

fn report_stats(app: &App) {
    let Some(st) = STATS.lock().unwrap().take() else { return };
    let mut v = st.intervals.clone();
    if v.is_empty() {
        return;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = v[v.len() / 2];
    let long = v.iter().filter(|x| **x > 1.5 * med).count();
    let edges = [4.0, 6.0, 7.5, 8.5, 9.5, 12.0, 15.0, 17.5, 20.0, 25.0, 34.0, 50.0, f32::INFINITY];
    let mut counts = vec![0usize; edges.len()];
    for x in &v {
        let k = edges.iter().position(|e| x < e).unwrap_or(edges.len() - 1);
        counts[k] += 1;
    }
    let mut hist = String::new();
    let mut lo = 0.0;
    for (e, c) in edges.iter().zip(&counts) {
        if *c > 0 {
            hist.push_str(&format!(" [{lo:.1}-{}: {c}]", if e.is_finite() { format!("{e:.1}") } else { "inf".into() }));
        }
        lo = *e;
    }
    let pn = &crate::ui_feel::PASS_NS;
    eprintln!("bench: egui widget frames eased {}; widget pass mean {:.1} us, longest {:.1} us over {} frames", crate::ui_feel::EASED_FRAMES.load(Relaxed), pn[0].load(Relaxed) as f64 / 1000.0 / pn[1].load(Relaxed).max(1) as f64, pn[2].load(Relaxed) as f64 / 1000.0, pn[1].load(Relaxed));
    eprintln!("bench: frame intervals {} frames, median {med:.2} ms, p95 {:.2}, max {:.2}, longer than 1.5x median {long}, motion {} (still moving last frame {});{hist}", v.len(), v[v.len() * 95 / 100], v[v.len() - 1], motion::level().name(), motion::last_active());
    if !st.cpu.is_empty() {
        let mut c = st.cpu.clone();
        c.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let wait: Vec<f32> = st.intervals.iter().zip(&st.cpu).map(|(i, c)| (i - c).max(0.0)).collect();
        let mut w = wait.clone();
        w.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |v: &Vec<f32>, q: usize| v[(v.len() * q / 100).min(v.len() - 1)];
        eprintln!("bench: frame CPU (eframe, no swap) median {:.2} ms p95 {:.2} max {:.2}; outside it (swap + event loop) median {:.2} p5 {:.2} p95 {:.2}", pct(&c, 50), pct(&c, 95), c[c.len() - 1], pct(&w, 50), pct(&w, 5), pct(&w, 95));
        // The intervals in order (first 40), to see the rhythm.
        let seq: Vec<String> = st.intervals.iter().take(40).map(|x| format!("{x:.1}")).collect();
        eprintln!("bench: interval sequence {}", seq.join(" "));
    }
    // Presented frames, estimated from the display link: frame i started in refresh t_i, phase
    // p_i into it, and took c_i of CPU (eframe's, reported with the next frame); it is shown at
    // the first refresh after it is done: t_i + 1 + floor((p_i + c_i) / period). Steps between
    // shown frames: 1 = a new picture every refresh; 0 = two frames for one refresh (one never
    // seen); 2+ = a refresh showed the old picture again.
    if st.refresh.len() > 10 && st.cpu.len() >= st.refresh.len() {
        let shown: Vec<u64> = st.refresh.iter().enumerate().filter_map(|(i, (t, p, per))| {
            let c = *st.cpu.get(i + 1)? as f64 / 1000.0;
            (*per > 0.0).then(|| t + 1 + ((p + c) / per).floor() as u64)
        }).collect();
        let mut h = [0usize; 4];
        for w in shown.windows(2) {
            h[(w[1].saturating_sub(w[0]) as usize).min(3)] += 1;
        }
        let per = st.refresh[0].2 * 1000.0;
        eprintln!("bench: presented (estimated from the display link, period {per:.2} ms): steps between shown frames: 0 refreshes {} | 1 refresh {} | 2 refreshes {} | 3+ {}", h[0], h[1], h[2], h[3]);
    }
    if st.steps.len() > 10 {
        let stat = |f: &dyn Fn(&(f64, f64)) -> f64| {
            let xs: Vec<f64> = st.steps.iter().map(f).collect();
            let m = xs.iter().sum::<f64>() / xs.len() as f64;
            let sd = (xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / xs.len() as f64).sqrt();
            let still = xs.iter().filter(|x| x.abs() < 1e-9).count();
            (m * 1000.0, sd * 1000.0, still)
        };
        let (ms, ss, zs) = stat(&|p| p.0);
        let (mr, sr, zr) = stat(&|p| p.1);
        eprintln!("bench: play steps per frame (ms of track): drawn mean {ms:.2} sd {ss:.2} frames without a move {zs}; audio position mean {mr:.2} sd {sr:.2} frames without a move {zr} ({} frames, zoom {})", st.steps.len(), app.prefs.zoom);
    }
}

pub fn raw_input(app: &mut App, raw: &mut egui::RawInput) {
    // Strip pictures arrive as input events: take them out before main.rs sees a screenshot.
    let mut g = STRIP.lock().unwrap();
    if let Some(s) = g.as_mut() {
        let ppp = raw.viewports.get(&raw.viewport_id).and_then(|v| v.native_pixels_per_point).unwrap_or(2.0);
        raw.events.retain(|e| {
            if let egui::Event::Screenshot { user_data, image, .. } = e {
                if let Some(tag) = user_data.data.as_ref().and_then(|d| d.downcast_ref::<StripTag>()) {
                    s.pics.push((tag.0, tag.1, tag.2, crop(image, s.rect, ppp)));
                    return false;
                }
            }
            true
        });
    }
    drop(g);
    let Some((_, frame)) = app.screenshot.clone() else { return };
    if let Some(sc) = scenario() {
        scenario_events(app, sc, frame, raw);
    }
}

fn crop(img: &egui::ColorImage, rect: Option<egui::Rect>, ppp: f32) -> image::RgbaImage {
    let [w, h] = img.size;
    let (x0, y0, x1, y1) = match rect {
        Some(r) => (((r.min.x * ppp) as usize).min(w), ((r.min.y * ppp) as usize).min(h), ((r.max.x * ppp) as usize).min(w), ((r.max.y * ppp) as usize).min(h)),
        None => (0, 0, w, h),
    };
    let (cw, ch) = ((x1 - x0).max(1), (y1 - y0).max(1));
    let mut out = image::RgbaImage::new(cw as u32, ch as u32);
    for y in 0..ch {
        for x in 0..cw {
            let px = img.pixels[(y0 + y) * w + x0 + x];
            let [r, g, b, a] = px.to_srgba_unmultiplied();
            out.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, a]));
        }
    }
    // Half size: one picture pixel per point.
    image::imageops::resize(&out, (cw as u32 / 2).max(1), (ch as u32 / 2).max(1), image::imageops::FilterType::Triangle)
}

fn save_strip(s: &mut Strip, shot: &str) {
    s.pics.sort_by_key(|p| p.0);
    let (w, h) = (s.pics[0].3.width(), s.pics[0].3.height());
    let cols = if w < 400 { 8 } else { 4 }.min(s.pics.len() as u32);
    let rows = (s.pics.len() as u32).div_ceil(cols);
    let gap = 4;
    let mut sheet = image::RgbImage::from_pixel(cols * (w + gap) + gap, rows * (h + gap) + gap, image::Rgb([60, 60, 60]));
    for (k, p) in s.pics.iter().enumerate() {
        let (cx, cy) = (gap + (k as u32 % cols) * (w + gap), gap + (k as u32 / cols) * (h + gap));
        for (x, y, px) in p.3.enumerate_pixels() {
            if x < w && y < h {
                sheet.put_pixel(cx + x, cy + y, image::Rgb([px[0], px[1], px[2]]));
            }
        }
    }
    let base = shot.trim_end_matches(".png");
    let jpg = format!("{base}-strip.jpg");
    let mut f = std::fs::File::create(&jpg).ok();
    if let Some(f) = f.as_mut() {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(f, 82);
        let _ = enc.encode_image(&sheet);
    }
    let t0 = s.pics[0].1;
    let txt: String = s.pics.iter().map(|p| format!("{}\tframe {}\t{:+.1} ms\n", p.0, p.2, (p.1 - t0) * 1000.0)).collect();
    let _ = std::fs::write(format!("{base}-strip.txt"), txt);
    eprintln!("feeltest: strip of {} pictures ({}x{} each, {} per row) saved to {jpg}", s.pics.len(), w, h, cols);
}

// ------------------------------------------------------------------ scenarios

/// Frame the scripted interaction starts at.
const T0: u32 = 160;

fn click(raw: &mut egui::RawInput, p: Pos2, down: bool) {
    raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: down, modifiers: Default::default() });
}

fn mv(raw: &mut egui::RawInput, p: Pos2) {
    raw.events.push(egui::Event::PointerMoved(p));
}

fn key(raw: &mut egui::RawInput, k: egui::Key, down: bool) {
    raw.events.push(egui::Event::Key { key: k, physical_key: None, pressed: down, repeat: false, modifiers: Default::default() });
}

fn wheel(raw: &mut egui::RawInput, unit: egui::MouseWheelUnit, dy: f32) {
    raw.events.push(egui::Event::MouseWheel { unit, delta: egui::vec2(0.0, dy), modifiers: Default::default() });
}

fn list_offset(app: &App) -> f32 {
    // The track list's scroll state (ui_browser.rs `vscroll`).
    let _ = app;
    LIST_OFF.with(|c| c.get())
}

thread_local! {
    static LIST_OFF: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
    /// The enlarged waveform when a wave scenario starts: its rect, the position, pixels per s.
    static WAVE: std::cell::Cell<Option<(egui::Rect, f64, f64)>> = const { std::cell::Cell::new(None) };
    /// The pointer's x the wave drag asked for this frame.
    static WAVE_X: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
}

/// The x of the wave drag at scenario frame `f` (a press at the centre, then back and forth).
fn wave_x(c: f32, f: u32) -> f32 {
    let k = f.saturating_sub(10) as f32;
    c + 160.0 * (k * std::f32::consts::TAU / 120.0).sin()
}

/// The track list reports its scroll offset (ui_browser.rs) for the scroll scenarios.
pub fn list_offset_seen(y: f32) {
    LIST_OFF.with(|c| c.set(y));
}

fn scenario_state(app: &mut App, s: &str, frame: u32) {
    if frame == T0 - 10 {
        match s {
            "play" => {
                app.prefs.zoom = 0;
            }
            _ => {}
        }
    }
    if frame == T0 && bench() > 0 {
        if let Some(st) = STATS.lock().unwrap().as_mut() {
            // The histogram covers the interaction only.
            st.window = Some((T0, T0 + span(s)));
            st.intervals.clear();
            st.cpu.clear();
            st.refresh.clear();
            st.steps.clear();
        }
    }
    if frame == T0 && matches!(s, "wavedrag" | "wavescroll") {
        WAVE.with(|w| w.set(app.deck.detail_map.map(|(r, _, pps)| (r, app.pos(), pps))));
    }
    if (T0..T0 + span(s)).contains(&frame) && matches!(s, "scroll" | "wheel" | "keys") {
        eprintln!("feeltest: {s} frame {} list offset {:.2} selected {:?}", frame - T0, list_offset(app), app.anchor);
    }
}

fn span(s: &str) -> u32 {
    match s {
        "scroll" | "wheel" => 120,
        "keys" => 140,
        "play" => 600,
        "drag" | "refuse" | "menu" => 80,
        "wavedrag" | "wavescroll" => 240,
        _ => 60,
    }
}

fn scenario_events(app: &mut App, s: &str, frame: u32, raw: &mut egui::RawInput) {
    if frame < T0 {
        return;
    }
    let f = frame - T0;
    let at = at_env("PREPDECK_FEEL_AT");
    match s {
        "hover" => {
            let p = at.unwrap_or(pos2(700.0, 14.0));
            match f {
                0 => mv(raw, p + egui::vec2(0.0, 60.0)),
                4 => mv(raw, p),
                30 => mv(raw, p + egui::vec2(0.0, 60.0)),
                _ => {}
            }
        }
        "press" => {
            let p = at.unwrap_or(pos2(700.0, 14.0));
            match f {
                0 => mv(raw, p),
                10 => click(raw, p, true),
                24 => click(raw, p, false),
                40 => mv(raw, p + egui::vec2(0.0, 60.0)),
                _ => {}
            }
        }
        // Performance: channel fader 1 dragged down, then a double-click puts it back to the top.
        "fader" | "knob" | "xfader" => {
            let (a, b) = match s {
                "fader" => (at.unwrap_or(pos2(729.5, 28.0 + 143.0 + 169.5)), egui::vec2(0.0, 36.0)),
                "knob" => (at.unwrap_or(pos2(729.5, 28.0 + 143.0 + 51.0)), egui::vec2(0.0, 60.0)),
                _ => (at.unwrap_or(pos2(756.0, 28.0 + 368.0 + 1.5 + 21.5)), egui::vec2(40.0, 0.0)),
            };
            match f {
                0 => mv(raw, a),
                4 => click(raw, a, true),
                5..=16 => mv(raw, a + b * ((f - 4) as f32 / 12.0)),
                17 => click(raw, a + b, false),
                30 => {
                    eprintln!("feeltest: {s} after the drag: {}", mixer_line(app));
                }
                34 | 36 => click(raw, a + b, true),
                35 | 37 => click(raw, a + b, false),
                50 => eprintln!("feeltest: {s} after the double-click: {}", mixer_line(app)),
                56 => mv(raw, a + b + egui::vec2(120.0, 0.0)),
                _ => {}
            }
        }
        // A trackpad swipe: steps of 40 pt falling off like the system's momentum.
        "scroll" => {
            let p = at.unwrap_or(pos2(800.0, 650.0));
            if f == 0 {
                mv(raw, p);
            }
            if (2..62).contains(&f) {
                let k = (f - 2) as f32;
                let d = -40.0 * (-k / 14.0).exp();
                if d.abs() > 0.3 {
                    wheel(raw, egui::MouseWheelUnit::Point, d);
                }
            }
            if (70..100).contains(&f) {
                let k = (f - 70) as f32;
                wheel(raw, egui::MouseWheelUnit::Point, 30.0 * (-k / 10.0).exp());
            }
        }
        // A mouse wheel: three notches down, a pause, two up.
        "wheel" => {
            let p = at.unwrap_or(pos2(800.0, 650.0));
            match f {
                0 => mv(raw, p),
                4 | 12 | 20 => wheel(raw, egui::MouseWheelUnit::Line, -1.0),
                60 | 64 => wheel(raw, egui::MouseWheelUnit::Line, 1.0),
                _ => {}
            }
        }
        // Click a row, then the down arrow 30 times (selection moves, the list follows).
        "keys" => {
            let p = at.unwrap_or(pos2(800.0, 500.0));
            match f {
                0 => mv(raw, p),
                2 => click(raw, p, true),
                3 => click(raw, p, false),
                10..=130 if (f - 10) % 4 == 0 => key(raw, egui::Key::ArrowDown, true),
                11..=131 if (f - 11) % 4 == 0 => key(raw, egui::Key::ArrowDown, false),
                _ => {}
            }
        }
        // Drag the row under `PREPDECK_FEEL_AT` to `PREPDECK_FEEL_TO` (a playlist in the tree, or
        // for "refuse" a place that takes no drop) and let go there.
        "drag" | "refuse" => {
            let a = at.unwrap_or(pos2(800.0, 500.0));
            let b = at_env("PREPDECK_FEEL_TO").unwrap_or(if s == "drag" { pos2(150.0, 420.0) } else { pos2(1000.0, 14.0) });
            match f {
                0 => mv(raw, a),
                2 => click(raw, a, true),
                3..=40 => mv(raw, a + (b - a) * motion::ease((f - 3) as f32 / 37.0)),
                50 => click(raw, b, false),
                52 => eprintln!("feeltest: {s} released over {:?}: drag {:?}", b, app.drag.as_ref().map(|d| d.len())),
                _ => {}
            }
        }
        // Right-click at `PREPDECK_FEEL_AT` (a track row: its context menu), then the pointer
        // goes down the menu one item at a time and leaves it.
        "menu" => {
            let p = at.unwrap_or(pos2(800.0, 500.0));
            let sec = |raw: &mut egui::RawInput, down: bool| raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Secondary, pressed: down, modifiers: Default::default() });
            match f {
                0 => mv(raw, p),
                2 => sec(raw, true),
                3 => sec(raw, false),
                10..=58 if (f - 10) % 8 == 0 => mv(raw, p + egui::vec2(40.0, 12.0 + 22.0 * ((f - 10) / 8) as f32)),
                66 => mv(raw, p + egui::vec2(-200.0, 0.0)),
                _ => {}
            }
        }
        // A dialog open (PREPDECK_SHOT_VIEW=...,confirmdelete): Tab three times (the focus ring
        // goes from button to button), then PREPDECK_FEEL_KEY (Escape, Enter) and the answer.
        "dialog" => {
            let k = match env("PREPDECK_FEEL_KEY").as_deref() {
                Some("Enter") => egui::Key::Enter,
                _ => egui::Key::Escape,
            };
            match f {
                4 | 16 | 28 => key(raw, egui::Key::Tab, true),
                5 | 17 | 29 => key(raw, egui::Key::Tab, false),
                44 => key(raw, k, true),
                45 => key(raw, k, false),
                10 | 22 | 34 => eprintln!("feeltest: dialog focus {:?} keyboard {}", app.ctx.memory(|m| m.focused()).map(|i| (i.short_debug_format(), app.ctx.read_response(i).map(|r| (r.layer_id, r.rect, r.sense)))), crate::ui_feel::keyboard()),
                40 => eprintln!("feeltest: dialog before the key: open {}", app.dialogs.confirm_delete_node.is_some()),
                50 => eprintln!("feeltest: dialog after {:?}: open {}, playlists view {:?}", k, app.dialogs.confirm_delete_node.is_some(), app.view),
                _ => {}
            }
        }
        // Drag a playing deck's enlarged waveform back and forth (Export: the centre of the
        // waveform, then 160 pt each way over two seconds), and let go.
        "wavedrag" => {
            let Some((r, _, _)) = WAVE.with(|w| w.get()) else { return };
            let y = r.center().y;
            match f {
                1 | 2 => mv(raw, pos2(r.center().x, y)),
                8 => click(raw, pos2(r.center().x, y), true),
                9..=209 => {
                    let x = wave_x(r.center().x, f);
                    WAVE_X.with(|w| w.set(Some(x)));
                    mv(raw, pos2(x, y));
                }
                210 => {
                    WAVE_X.with(|w| w.set(None));
                    click(raw, pos2(wave_x(r.center().x, 209), y), false);
                }
                _ => {}
            }
        }
        // A trackpad swipe over the playing waveform: point steps falling off, then back.
        "wavescroll" => {
            let Some((r, _, _)) = WAVE.with(|w| w.get()) else { return };
            // (The waveform is known from the scenario's first frame on: move there on the next.)
            if f <= 2 {
                mv(raw, r.center());
            }
            if (4..64).contains(&f) {
                let d = 24.0 * (-((f - 4) as f32) / 20.0).exp();
                raw.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(d, 0.0), modifiers: Default::default() });
            }
            if (100..160).contains(&f) {
                let d = -24.0 * (-((f - 100) as f32) / 20.0).exp();
                raw.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(d, 0.0), modifiers: Default::default() });
            }
        }
        "panel" | "mode" => {
            let p = at.unwrap_or(pos2(700.0, 14.0));
            match f {
                0 => mv(raw, p),
                4 => click(raw, p, true),
                5 => click(raw, p, false),
                12 => eprintln!("feeltest: {s} after the click: performance {} perf_fx {} layout {}", app.performance(), app.prefs.perf_fx, app.prefs.player_layout),
                _ => {}
            }
        }
        _ => {}
    }
}

fn mixer_line(app: &App) -> String {
    let m = &app.px.mix[0];
    format!("fader 1 {:.3} eq {:?} crossfader {:.3}", m.fader, m.eq, app.px.crossfader)
}
