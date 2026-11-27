//! Small functions the menu bar calls for features that live in other modules: the
//! Performance layouts, the pad and platter switch, Automix and the sample pack imports.
//! One place to connect them keeps `menus.rs` free of the details.

use crate::app::*;

/// The Performance layouts in rekordbox's View menu order: 0 = 2 Deck Horizontal,
/// 1 = 2 Deck Vertical, 2 = 4 Deck Horizontal, 3 = 4 Deck Vertical, 4 = Browse.
pub fn perf_layout(app: &App) -> u8 {
    if app.prefs.perf_four {
        2
    } else {
        0
    }
}

pub fn perf_layout_available(k: u8) -> bool {
    matches!(k, 0 | 2)
}

pub fn set_perf_layout(app: &mut App, k: u8) {
    if !perf_layout_available(k) {
        return;
    }
    app.prefs.perf_four = k == 2;
    app.poll_decks();
    app.save_prefs();
}

/// "Show / Hide Pad and Platter" (rekordbox: the horizontal layouts only).
pub fn pad_platter_available(_app: &App) -> bool {
    false
}

pub fn toggle_pad_platter(_app: &mut App) {}

/// View > Automix (the Automix playlist under the browser).
pub fn automix_available() -> bool {
    false
}

pub fn automix_shown(_app: &App) -> bool {
    false
}

pub fn toggle_automix(_app: &mut App) {}

/// File > Additional contents > Sample Pack / GROOVE CIRCUIT FACTORY SAMPLES PACK > Import.
pub fn pack_import_available(_id: &str) -> bool {
    false
}

pub fn pack_import(_app: &mut App, _id: &str) {}
