//! Split Screen (View > Show Split Screen): the browser in a window of its own, the main
//! window keeping the decks. rekordbox's tooltip: "Split Screen: Open a browser screen as an
//! independent window." Its command (`MainComponent::perform`, 45132) asks the browser view to
//! detach, and going full screen attaches it again (`browseAttach`).
//!
//! Not traced: the sizes of the two windows after the split. Here the main window shrinks to
//! the header, the decks and the footer, the browser window opens under it with the size the
//! browser had, and both return when the browser window is closed or the command is chosen
//! again.

use crate::app::*;
use eframe::egui;

#[derive(Default)]
pub struct Split {
    pub open: bool,
    /// The main window's size before the split, to put back.
    restore: Option<egui::Vec2>,
    /// The main window has been resized for the split.
    applied: bool,
}

impl App {
    pub fn split_toggle(&mut self) {
        self.split.open = !self.split.open;
        self.split.applied = false;
        self.ctx.request_repaint();
    }
}

/// `PREPDECK_SHOT_SPLIT=1` (with --screenshot and `PREPDECK_BENCH=300`): a scripted run with
/// two real windows. It opens Split Screen at frame 20, logs both windows' sizes, puts the
/// main window into full screen (which must take the browser back), leaves full screen,
/// opens the split again and leaves it open for the main window's picture.
fn shot_run() -> bool {
    std::env::var("PREPDECK_SHOT_SPLIT").is_ok()
}

fn shot_step(app: &mut App, ctx: &egui::Context, frame: u32, browser: Option<egui::Rect>) {
    let main = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
    let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    let say = |what: &str, app: &App| eprintln!("splittest: frame {frame}: {what}: split open {} | main window {:?} full screen {fs} | browser window {:?} | decks height {:.0}", app.split.open, main, browser.map(|r| r.size()), decks_height(app));
    match frame {
        20 => {
            app.split_toggle();
            say("View > Show Split Screen", app);
        }
        60 => say("after the split", app),
        70 => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            say("main window to full screen", app);
        }
        170 => say("in full screen (the browser must be back in the main window)", app),
        180 => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false)),
        270 => {
            say("full screen left", app);
            app.split_toggle();
            say("Show Split Screen again", app);
        }
        330 | 380 => say("before the picture", app),
        _ => {}
    }
    if frame > 20 && frame % 10 == 0 {
        ctx.request_repaint();
    }
}

/// Height of the main window's content without the browser.
fn decks_height(app: &App) -> f32 {
    28.0 + 28.0 + if !app.prefs.browser_only || app.performance() { app.player_height() } else { 0.0 }
}

/// Called every frame from the main window, before its panels. Returns true while the browser
/// is in its own window (the main window then leaves the browser out).
pub fn frame(app: &mut App, ctx: &egui::Context) -> bool {
    // Screenshots are of the main window, and a full-screen window takes the browser back.
    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    let run = shot_run() && app.screenshot.is_some();
    let frame = app.screenshot.as_ref().map(|s| s.1).unwrap_or(0);
    if app.split.open && ((app.screenshot.is_some() && !run) || fullscreen) {
        app.split.open = false;
        app.split.applied = false;
    }
    let main_size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
    if !app.split.open {
        // (In full screen the size goes back once full screen is left: macOS ignores it before.)
        if let Some(size) = if fullscreen { None } else { app.split.restore.take() } {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(1100.0, 700.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        if run {
            shot_step(app, ctx, frame, None);
        }
        return false;
    }
    let dh = decks_height(app);
    if !app.split.applied {
        app.split.applied = true;
        if app.split.restore.is_none() {
            app.split.restore = main_size;
        }
        if let Some(size) = main_size {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(1100.0, dh)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(size.x, dh)));
        }
    }
    let was = app.split.restore.unwrap_or(egui::vec2(1512.0, 921.0));
    let builder = egui::ViewportBuilder::default().with_title("recordbox").with_inner_size([was.x, (was.y - dh).max(300.0)]).with_min_inner_size([900.0, 240.0]);
    let mut close = false;
    let mut browser_rect = None;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("recordbox_browser"), builder, |vctx, class| {
        if class == egui::ViewportClass::Embedded {
            // No second window on this backend: stay attached.
            close = true;
            return;
        }
        egui::CentralPanel::default().frame(egui::Frame::default().fill(crate::theme::BG.c())).show(vctx, |ui| {
            crate::ui_browser::browser(app, ui);
        });
        if vctx.input(|i| i.viewport().close_requested()) {
            close = true;
        }
        browser_rect = vctx.input(|i| i.viewport().inner_rect);
    });
    if close {
        app.split.open = false;
        app.split.applied = false;
        if let Some(size) = app.split.restore.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(1100.0, 700.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        return false;
    }
    if run {
        shot_step(app, ctx, frame, browser_rect);
    }
    true
}
