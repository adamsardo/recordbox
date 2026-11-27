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

/// Height of the main window's content without the browser.
fn decks_height(app: &App) -> f32 {
    28.0 + 28.0 + if !app.prefs.browser_only || app.performance() { app.player_height() } else { 0.0 }
}

/// Called every frame from the main window, before its panels. Returns true while the browser
/// is in its own window (the main window then leaves the browser out).
pub fn frame(app: &mut App, ctx: &egui::Context) -> bool {
    // Screenshots are of the main window, and a full-screen window takes the browser back.
    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    if app.split.open && (app.screenshot.is_some() || fullscreen) {
        app.split.open = false;
        app.split.applied = false;
    }
    let main_size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
    if !app.split.open {
        if let Some(size) = app.split.restore.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(1100.0, 700.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
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
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("recordbox_browser"), builder, |vctx, class| {
        if class == egui::ViewportClass::Embedded {
            // No second window on this backend: stay attached.
            close = true;
            return;
        }
        egui::CentralPanel::default().frame(egui::Frame::default().fill(crate::theme::BG)).show(vctx, |ui| {
            crate::ui_browser::browser(app, ui);
        });
        if vctx.input(|i| i.viewport().close_requested()) {
            close = true;
        }
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
    true
}
