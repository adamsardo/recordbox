//! The Preferences window, laid out as rekordbox's: categories on the left (View, Audio,
//! Analysis, DJ System, Keyboard, Advanced), tabs across the top, cards of settings. Also the
//! Keyboard Shortcuts window and the Backup / Restore Library windows.

use crate::analysis::BPM_RANGES;
use crate::app::*;
use crate::export::settings::{SettingFile, DEV_OVERVIEW_TYPE, DEV_WAVEFORM_COLOR, DEV_WAVEFORM_POSITION};
use crate::prefs_ext::{ACTIONS, ACTION_CATEGORIES};
use crate::theme::*;
use eframe::egui;
use crate::ui_kit::{self as kit, btn, card, inter, inter_bold, note, radio, separator, sub, toggle, PBLUE};
use egui::{pos2, vec2, Color32, Context, Rect, RichText, Ui};

// Settings another part of the app has to act on. While one is false its control is shown
// switched off with the reason, so nothing in the window pretends to work.
/// The device writer deletes an imported history (reads `prefs.delete_history_after_import`).
const HISTORY_DELETE_WIRED: bool = true;
/// The device writer stores `prefs.device_bg_color`.
const BG_COLOR_WRITTEN: bool = false;
// The player (ui_player.rs) is to act on these; set each to true once it does.
/// The enlarged waveform draws the phrases when `prefs.phrase_enlarged`.
const PHRASE_ENLARGED_WIRED: bool = true;
/// The phrase display shows phrase types always (`prefs.phrase_types_always`) or on hover.
const PHRASE_TYPES_WIRED: bool = true;
/// The enlarged waveform draws the vocal positions when `prefs.vocal_enlarged`.
const VOCAL_ENLARGED_WIRED: bool = true;
/// The stem strip's part buttons call `prefs_ext::stem_part_click` (MUTE / SOLO mode).
const STEM_SOLO_WIRED: bool = true;
/// The waveforms show only the parts that sound when `prefs.stem_waveform`.
const STEM_WAVEFORM_WIRED: bool = true;

/// Categories the original has and Recordbox does not (no subscription, no cloud): shown
/// dimmed at their places so the list reads the same.
const ABSENT: [(u8, &str, &str); 2] = [(100, "PLAN", "recordbox has no subscription plans."), (101, "CLOUD", "recordbox has no cloud service.")];
// Extensions is where rekordbox keeps its STEMS page (in Performance mode).
// Controller holds the Performance mixer's settings, as in rekordbox.
const PAGES: [(u8, &str); 8] = [(0, "View"), (5, "Audio"), (1, "Analysis"), (7, "Controller"), (2, "DJ System"), (3, "Keyboard"), (4, "Advanced"), (6, "Extensions")];

fn tabs_of(page: u8) -> &'static [&'static str] {
    match page {
        0 => &["Display Type", "Layout", "Color", "recordbox"],
        1 => &["Track Analysis", "CUE Analysis"],
        2 => &["General", "Category", "Sort", "Column", "My Settings", "Device", "Others"],
        3 => &ACTION_CATEGORIES,
        5 => &["Configuration", "Input/Output"],
        6 => &["STEMS"],
        7 => &["Deck", "Mixer", "Effect", "Sampler", "Others"],
        _ => &["Database", "Browse", "Others"],
    }
}

/// Every setting by name, for the search field: (page, tab, name).
const SETTINGS: [(u8, usize, &str); 116] = [
    (7, 2, "Apply CFX to Sampler using MIC COLOR control knob."),
    (7, 3, "SLOT"),
    (7, 3, "Do not overwrite the loaded Slot."),
    (7, 3, "Capture the loop in an empty slot"),
    (7, 3, "Auto Panel Mode"),
    (7, 3, "Sequence Load"),
    (7, 3, "Velocity Curve"),
    (7, 4, "SEQUENCER"),
    (0, 0, "Language"),
    (0, 0, "Show Tooltips"),
    (0, 0, "Font Size"),
    (0, 0, "Bold"),
    (0, 0, "Line Space"),
    (0, 0, "Show the selected track to the center"),
    (0, 0, "Key display format"),
    (0, 0, "Full/Preview Waveform"),
    (0, 0, "Full Waveform"),
    (0, 0, "Half Waveform"),
    (0, 0, "Beat Count Display"),
    (0, 1, "Media Browser"),
    (0, 1, "Browser panel"),
    (0, 1, "CUE Analysis Playlist"),
    (0, 1, "Playlist Palette"),
    (0, 1, "Display Cue Markers on Preview"),
    (0, 1, "Playlist search filter"),
    (0, 1, "Display All Tracks in the Playlists"),
    (0, 1, "Display the number of tracks in a playlist on the Tree View"),
    (0, 1, "Display SYNC RATE"),
    (0, 1, "Phrase (Enlarged Waveform)"),
    (0, 1, "Phrase (Full Waveform)"),
    (0, 1, "Always show types of phrases"),
    (0, 1, "Vocal (Enlarged Waveform)"),
    (0, 1, "Vocal (Full Waveform)"),
    (0, 2, "Appearance"),
    (0, 2, "Waveform color"),
    (0, 2, "HOT CUE color"),
    (0, 3, "Time Mode"),
    (0, 3, "Quantize"),
    (0, 3, "Tree View"),
    (0, 3, "My Tag Configuration Window"),
    (0, 3, "Column browser"),
    (0, 3, "Sub-Browser Window"),
    (5, 0, "Audio"),
    (5, 0, "Driver"),
    (5, 0, "Sample Rate"),
    (5, 0, "Buffer size"),
    (5, 0, "Metronome"),
    (5, 0, "Click Sound"),
    (5, 0, "Metronome Volume"),
    (5, 0, "Preview Volume"),
    (5, 1, "Output channels"),
    (5, 1, "Master Output"),
    (1, 0, "Use high precision BeatGrid analysis"),
    (1, 0, "Analysis Mode"),
    (1, 0, "BPM Range"),
    (1, 0, "BPM / Grid"),
    (1, 0, "KEY"),
    (1, 0, "Phrase"),
    (1, 0, "Vocal"),
    (1, 0, "Auto Analysis"),
    (1, 0, "Auto Gain"),
    (1, 1, "Set Memory Cue at the first beat of track when analyzing"),
    (1, 1, "Set Hot Cue A on the first downbeat"),
    (2, 0, "Background Color"),
    (2, 0, "Waveform color displayed on CDJ/XDJ"),
    (2, 0, "Waveform Current Position"),
    (2, 0, "Waveform Divisions"),
    (2, 0, "Type of the Overview Waveform"),
    (2, 1, "Category"),
    (2, 2, "Sort"),
    (2, 3, "Column"),
    (2, 4, "My Settings"),
    (2, 4, "QUANTIZE"),
    (2, 4, "AUTO CUE"),
    (2, 4, "TIME MODE"),
    (2, 4, "JOG MODE"),
    (2, 4, "TEMPO RANGE"),
    (2, 4, "MASTER TEMPO"),
    (2, 4, "HOT CUE AUTO LOAD"),
    (2, 4, "DJM mixer settings"),
    (2, 5, "Delete playlist tracks from device if playlist is deleted"),
    (2, 5, "Import the play history automatically"),
    (2, 5, "Delete from the device after importing the play history"),
    (2, 6, "Prepare Hot Cue Auto Load setting for CDJ/XDJ use (on import)"),
    (3, 0, "Keyboard shortcuts"),
    (4, 0, "Database"),
    (4, 0, "Backup Library"),
    (4, 0, "Restore Library"),
    (4, 1, "iTunes Library File"),
    (4, 1, "Imported Library"),
    (4, 1, "rekordbox xml"),
    (4, 1, "Auto Relocate Search Folders"),
    (4, 1, "Add \"My Tag\" to the \"Comments\""),
    (4, 1, "Color comments"),
    (4, 1, "Customize Color Comments"),
    (4, 2, "rekordbox Library"),
    (7, 0, "Eject/Load Lock"),
    (7, 0, "Instant Doubles"),
    (7, 1, "EQ"),
    (7, 1, "EQ/ISOLATOR"),
    (7, 1, "Channel Fader Curve"),
    (6, 0, "Enable the STEMS Function"),
    (6, 0, "STEMS mode"),
    (6, 0, "STEMS Analysis Process"),
    (6, 0, "Prioritize sound quality"),
    (6, 0, "Prioritize speed"),
    (6, 0, "STEM Waveform Display"),
    (6, 0, "ACTIVE STEM Setting"),
    (6, 0, "MUTE / SOLO"),
    (6, 0, "Reset the MUTE/SOLO state while loading"),
    (6, 0, "Stem separation model"),
    (6, 0, "Allow automatic separation on CPU"),
    (0, 0, "Playback animation"),
    (1, 0, "Analysis workers"),
    (2, 6, "Supported devices"),
    (0, 0, "Tooltips"),
];

/// Settings (and keyboard actions) whose name contains the search text: (page, tab, name).
pub fn search_settings(q: &str) -> Vec<(u8, usize, String)> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let mut v: Vec<(u8, usize, String)> = SETTINGS.iter().filter(|s| s.2.to_lowercase().contains(&q)).map(|s| (s.0, s.1, s.2.to_string())).collect();
    for (_, label, _, cat) in ACTIONS {
        if label.to_lowercase().contains(&q) {
            let tab = ACTION_CATEGORIES.iter().position(|c| *c == cat).unwrap_or(0);
            v.push((3, tab, label.to_string()));
        }
    }
    v
}

// ------------------------------------------------------------------ widgets

/// A switch that only works once `wired`; until then it is shown off, with the reason.
fn toggle_if(app: &mut App, ui: &mut Ui, wired: bool, pick: fn(&mut Prefs) -> &mut bool, label: &str, why: &str) {
    if wired {
        toggle(ui, pick(&mut app.prefs), label);
    } else if kit::toggle_off(ui, label, why) {
        app.toast(why);
    }
}

/// A shortcut as the window shows it. The app's font has no ⌃ or ⌥ glyph, so those two are
/// spelled out.
fn key_text(sc: &Shortcut) -> String {
    sc.label().replace('⌃', "Ctrl ").replace('⌥', "Opt ").replace('⇧', "Shift ")
}

fn path_row(ui: &mut Ui, value: &Option<String>, empty: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(500.0), 22.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 3.0, BG);
    let text = value.clone().unwrap_or_else(|| empty.to_string());
    let col = if value.is_some() { WHITE } else { DIM };
    ui.painter().with_clip_rect(r).text(pos2(r.min.x + 6.5, r.center().y), egui::Align2::LEFT_CENTER, text, inter(12.5), col);
}

/// The category icons of the sidebar, in a 30 pt box at `o` (shapes as the original's).
fn side_icon(p: &egui::Painter, o: egui::Pos2, page: u8, c: Color32, bg: Color32) {
    let at = |x: f32, y: f32| pos2(o.x + x, o.y + y);
    let bar = |x: f32, y: f32, w: f32, h: f32, col: Color32| p.rect_filled(Rect::from_min_size(at(x, y), vec2(w, h)), 0.0, col);
    match page {
        // View: an eye.
        0 => {
            let n = 28;
            let half = |sign: f32| (0..=n).map(move |i| {
                let u = -1.0 + 2.0 * i as f32 / n as f32;
                (15.005 + u * 8.27 * sign, 15.005 - sign * 5.79 * (u * std::f32::consts::FRAC_PI_2).cos().powf(0.92))
            });
            let pts: Vec<egui::Pos2> = half(1.0).chain(half(-1.0)).map(|(x, y)| at(x, y)).collect();
            p.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.29, c)));
            p.circle_filled(at(15.005, 15.005), 3.86, c);
        }
        // Audio: a speaker.
        5 => {
            p.add(egui::Shape::convex_polygon(vec![at(8.45, 8.68), at(16.17, 12.54), at(16.17, 17.69), at(8.45, 21.55)], c, egui::Stroke::NONE));
            bar(17.45, 12.54, 3.86, 5.15, c);
        }
        // Analysis: seven bars, the middle one tall.
        1 => {
            bar(14.36, 7.29, 1.29, 15.44, c);
            for x in [6.64, 9.22, 11.79, 16.93, 19.51, 22.08] {
                bar(x, 11.15, 1.29, 7.72, c);
            }
        }
        // DJ System: a player seen from above.
        2 => {
            bar(6.0, 8.57, 18.01, 12.87, c);
            p.circle_filled(at(11.147, 15.005), 3.86, bg);
            p.circle_filled(at(11.146, 15.005), 1.29, c);
            for y in [10.50, 13.08, 15.65, 18.22] {
                bar(16.29, y, 2.57, 1.29, bg);
                bar(20.15, y, 2.57, 1.29, bg);
            }
        }
        // Keyboard: a key cap with an A.
        3 => {
            p.rect_filled(Rect::from_min_max(at(7.66, 7.40), at(23.10, 22.83)), 1.29, c);
            p.text(at(15.38, 15.3), egui::Align2::CENTER_CENTER, "A", inter_bold(10.6), bg);
        }
        // PLAN: a disc with two quarter arcs cut out.
        100 => {
            p.circle_filled(at(15.007, 15.005), 7.72, c);
            let arc = |a0: f32| (0..=10).map(|i| { let a = a0 + i as f32 / 10.0 * std::f32::consts::FRAC_PI_2; at(15.007 + 5.145 * a.cos(), 15.005 + 5.145 * a.sin()) }).collect::<Vec<_>>();
            p.add(egui::Shape::line(arc(std::f32::consts::PI), egui::Stroke::new(2.57, bg)));
            p.add(egui::Shape::line(arc(0.0), egui::Stroke::new(2.57, bg)));
            p.circle_filled(at(15.007, 15.005), 2.57, bg);
        }
        // CLOUD: a cloud with the two sync arrows.
        101 => {
            p.circle_filled(at(14.36, 14.2), 5.63, c);
            p.circle_filled(at(9.86, 17.58), 3.86, c);
            p.circle_filled(at(20.0, 16.6), 4.2, c);
            p.rect_filled(Rect::from_min_max(at(9.86, 16.5), at(20.15, 21.44)), 1.0, c);
            let arc = |a0: f32| (0..=8).map(|i| { let a = a0 + i as f32 / 8.0 * 2.4; at(15.004 + 3.43 * a.cos(), 15.97 + 3.43 * a.sin()) }).collect::<Vec<_>>();
            p.add(egui::Shape::line(arc(std::f32::consts::PI), egui::Stroke::new(0.86, bg)));
            p.add(egui::Shape::line(arc(0.0), egui::Stroke::new(0.86, bg)));
            p.add(egui::Shape::line(vec![at(18.43, 12.11), at(18.43, 14.68), at(15.86, 14.68)], egui::Stroke::new(0.86, bg)));
            p.add(egui::Shape::line(vec![at(11.57, 19.83), at(11.57, 17.26), at(14.15, 17.26)], egui::Stroke::new(0.86, bg)));
        }
        // Controller: a box with two rings (skins/preferece_controller_off.svg).
        7 => {
            p.rect_filled(Rect::from_min_max(at(6.0, 8.57), at(24.01, 21.44)), 0.0, c);
            for cx in [11.145, 18.864] {
                p.circle_filled(at(cx, 15.005), 2.573, bg);
                p.circle_filled(at(cx, 15.005), 1.286, c);
            }
        }
        // Extensions: two gears (skins/preferece_extensions_off.svg: 9 pt across, eight teeth,
        // a 3.86 pt hole; centres 19.51, 11.79 and 10.50, 18.22).
        6 => {
            for (cx, cy) in [(19.508, 11.789), (10.504, 18.221)] {
                let c0 = at(cx, cy);
                p.circle_filled(c0, 3.5, c);
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4;
                    let (dir, side) = (vec2(a.sin(), -a.cos()), vec2(a.cos(), a.sin()));
                    p.add(egui::Shape::convex_polygon(vec![c0 + dir * 2.8 - side * 1.04, c0 + dir * 4.5 - side * 0.9, c0 + dir * 4.5 + side * 0.9, c0 + dir * 2.8 + side * 1.04], c, egui::Stroke::NONE));
                }
                p.circle_filled(c0, 1.93, bg);
            }
        }
        // Advanced: three lines.
        _ => {
            for y in [10.61, 14.47, 18.33] {
                bar(8.82, y, 12.86, 1.29, c);
            }
        }
    }
}

// ------------------------------------------------------------------ window

/// Width and height of the window's content (the original's 800 x 828 window less its
/// 28 pt title bar).
const WIN: egui::Vec2 = vec2(800.0, 800.0);

pub fn prefs_window(app: &mut App, ctx: &Context) {
    if !app.dialogs.prefs_open {
        return;
    }
    // Preferences opens in its own window, like the original. Screenshot mode (and backends
    // without multiple windows) draw it inside the main window instead.
    if app.screenshot.is_some() {
        prefs_embedded(app, ctx);
        return;
    }
    let vid = egui::ViewportId::from_hash_of("recordbox_preferences");
    let builder = egui::ViewportBuilder::default().with_title("Preferences").with_inner_size([WIN.x, WIN.y]).with_min_inner_size([WIN.x, 520.0]).with_max_inner_size([WIN.x, 4000.0]);
    ctx.show_viewport_immediate(vid, builder, |vctx, class| {
        if class == egui::ViewportClass::Embedded {
            prefs_embedded(app, vctx);
            return;
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(vctx, |ui| {
            let h = ui.available_height();
            prefs_body(app, ui, h)
        });
        if vctx.input(|i| i.viewport().close_requested()) {
            app.dialogs.prefs_open = false;
        }
    });
}

fn prefs_embedded(app: &mut App, ctx: &Context) {
    // Screenshots: the window's content alone at the top left of the picture, at the
    // original's size, so it can be laid over the reference.
    if app.screenshot.is_some() {
        egui::Area::new(egui::Id::new("prefs_flat")).fixed_pos(pos2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            let r = Rect::from_min_size(pos2(0.0, 0.0), WIN);
            ui.set_min_size(WIN);
            ui.painter().rect_filled(r, 0.0, BG);
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| prefs_body(app, ui, WIN.y));
        });
        return;
    }
    let mut open = true;
    let frame = egui::Frame::window(&ctx.style()).fill(BG).inner_margin(0.0);
    let h = (ctx.screen_rect().height() - 90.0).clamp(480.0, WIN.y);
    egui::Window::new("Preferences").open(&mut open).default_size(vec2(WIN.x, h)).collapsible(false).resizable(false).frame(frame).show(ctx, |ui| prefs_body(app, ui, h));
    app.dialogs.prefs_open = open;
}

fn prefs_body(app: &mut App, ui: &mut Ui, height: f32) {
    kit::apply_style(ui);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let o = ui.cursor().min;
    let (full, _) = ui.allocate_exact_size(vec2(WIN.x, height), egui::Sense::hover());
    let at = |x: f32, y: f32| pos2(o.x + x, o.y + y);
    ui.painter().rect_filled(full, 0.0, BG);
    // Sidebar 160 pt, #1E1E1E: search field, then the categories 33 pt apart.
    ui.painter().rect_filled(Rect::from_min_max(o, at(160.0, height)), 0.0, BG2);
    let sr = Rect::from_min_max(at(8.0, 16.0), at(152.0, 40.0));
    ui.painter().rect_filled(sr, 0.0, BG);
    let lens = egui::Stroke::new(1.0, hex(0x555555));
    ui.painter().circle_stroke(at(12.0 + 6.857, 20.0 + 6.857), 5.214, lens);
    ui.painter().line_segment([at(12.0 + 10.64, 20.0 + 11.075), at(12.0 + 14.07, 20.0 + 14.5)], lens);
    let field = Rect::from_min_max(pos2(sr.min.x + 24.0, sr.min.y + 3.0), pos2(sr.max.x - 2.0, sr.max.y));
    ui.put(field, egui::TextEdit::singleline(&mut app.dialogs.ext.prefs_search).frame(false).font(inter(13.0)).text_color(WHITE));
    for (n, (i, name, why)) in ABSENT.iter().enumerate() {
        let r = Rect::from_min_size(at(8.0, 56.0 + n as f32 * 33.0), vec2(144.0, 30.0));
        side_icon(ui.painter(), r.min, *i, OFF, BG2);
        ui.painter().text(pos2(r.min.x + 40.0, r.center().y), egui::Align2::LEFT_CENTER, *name, inter(13.0), OFF);
        ui.interact(r, ui.id().with(("prefs_absent", *i)), egui::Sense::hover()).on_hover_text(*why);
    }
    for (n, (i, name)) in PAGES.iter().enumerate() {
        let sel = app.dialogs.prefs_tab == *i && app.dialogs.ext.prefs_search.trim().is_empty();
        let r = Rect::from_min_size(at(8.0, 56.0 + (n + ABSENT.len()) as f32 * 33.0), vec2(144.0, 30.0));
        let resp = ui.interact(r, ui.id().with(("prefs_page", *i)), egui::Sense::click());
        let bg = if sel { PBLUE } else if resp.hovered() { hex(0x2A2A2A) } else { BG2 };
        if bg != BG2 {
            ui.painter().rect_filled(r, 0.0, bg);
        }
        let c = if sel { WHITE } else { DIM2 };
        side_icon(ui.painter(), r.min, *i, c, bg);
        kit::paint_text(ui, pos2(r.min.x + 40.0, r.center().y), egui::Align2::LEFT_CENTER, name, inter(13.0), c, 0.13);
        if resp.clicked() {
            app.dialogs.prefs_tab = *i;
            app.dialogs.ext.prefs_search.clear();
        }
    }
    let page = app.dialogs.prefs_tab;
    let tabs = tabs_of(page);
    let key = format!("prefs_subtab_{page}");
    let mut sub_tab: usize = ui.ctx().data_mut(|d| *d.get_temp_mut_or(egui::Id::new(&key), 0usize)).min(tabs.len() - 1);
    let searching = !app.dialogs.ext.prefs_search.trim().is_empty();
    // Tab strip on black, 30 pt tall: the active tab blue, the others #1E1E1E with #AAAAAA text.
    let mut x = 164.0;
    let names: Vec<&str> = if searching { vec!["Search results"] } else { tabs.to_vec() };
    for (i, t) in names.iter().enumerate() {
        let sel = searching || sub_tab == i;
        let g = kit::galley(ui, t, inter(12.0), if sel { WHITE } else { DIM2 }, 0.0);
        let w = (g.size().x + 4.45).round();
        if x + w > WIN.x - 2.0 {
            break;
        }
        let r = Rect::from_min_size(at(x, 4.0), vec2(w, 30.0));
        let resp = ui.interact(r, ui.id().with(("prefs_tab", page, i)), egui::Sense::click());
        ui.painter().rect_filled(r, 0.0, if sel { PBLUE } else if resp.hovered() { hex(0x2A2A2A) } else { BG2 });
        ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0), g, WHITE);
        if resp.clicked() && !searching {
            sub_tab = i;
        }
        x += w + 4.0;
    }
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(&key), sub_tab));
    let before = format!("{:?}", app.prefs);
    // The cards, from 38 pt down to 40 pt above the bottom edge. DJ System > General has no
    // "Reset to defaults" in the original: its cards run to 4 pt above the bottom edge.
    let no_reset = page == 2 && sub_tab == 0 && !searching;
    let area = Rect::from_min_max(at(164.0, 38.0), at(WIN.x, height - if no_reset { 4.0 } else { 40.0 }));
    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt(("prefs_scroll", page, sub_tab, searching)).max_height(area.height()).auto_shrink([false, false]).show(&mut content, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        if searching {
            search_results(app, ui);
        } else {
            match page {
                0 => page_view(app, ui, sub_tab),
                1 => page_analysis(app, ui, sub_tab),
                2 => page_dj_system(app, ui, sub_tab),
                3 => page_keyboard(app, ui, sub_tab),
                5 => page_audio(app, ui, sub_tab),
                6 => page_stems(app, ui),
                7 if sub_tab == 0 => page_deck(app, ui),
                7 if sub_tab == 1 => page_mixer(app, ui),
                7 if sub_tab == 2 => page_effect(app, ui),
                7 if sub_tab == 3 => page_sampler(app, ui),
                7 => page_controller_others(app, ui),
                _ => page_advanced(app, ui, sub_tab),
            }
        }
    });
    if format!("{:?}", app.prefs) != before {
        app.save_prefs();
    }
    let foot = Rect::from_min_max(at(164.5, height - 35.75), at(WIN.x - 8.0, height - 4.75));
    if !no_reset || app.dialogs.ext.reset_ask.is_some() {
        let mut foot_ui = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)));
        reset_row(app, &mut foot_ui);
    }
}

fn search_results(app: &mut App, ui: &mut Ui) {
    let hits = search_settings(&app.dialogs.ext.prefs_search);
    card(ui, "Search results", |ui| {
        if hits.is_empty() {
            kit::line(ui, "No setting has that in its name.");
        }
        kit::block(ui, |ui| {
            for (page, tab, name) in hits {
                let place = format!("{} > {}", PAGES.iter().find(|p| p.0 == page).map(|p| p.1).unwrap_or(""), tabs_of(page).get(tab).copied().unwrap_or(""));
                ui.horizontal(|ui| {
                    if ui.add(btn(name.as_str()).min_size(vec2(300.0, 22.0))).clicked() {
                        app.dialogs.prefs_tab = page;
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(format!("prefs_subtab_{page}")), tab));
                        app.dialogs.ext.prefs_search.clear();
                    }
                    ui.label(kit::text(place, 12.0).color(DIM));
                });
            }
        });
    });
}

/// "Reset to defaults": asks first, in the original's words; keyboard shortcuts, columns and
/// this computer's devices and folders are kept.
fn reset_row(app: &mut App, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.x = 8.0;
    match app.dialogs.ext.reset_ask {
        None => {
            if ui.add(btn("Reset to defaults").exact(vec2(249.0, 31.0))).clicked() {
                app.dialogs.ext.reset_ask = Some(None);
            }
        }
        Some(scope) => {
            let text = if scope == Some(3) { "Are you sure you want to reset all the key-mappings to their default state?" } else { "Do you want to reset the settings to defaults ?" };
            ui.label(kit::text(text, 13.0).color(WHITE)).on_hover_text("Keyboard shortcuts, columns and this computer's devices and folders are kept.");
            if ui.add(btn("OK").exact(vec2(80.0, 29.0))).clicked() {
                if scope == Some(3) {
                    app.prefs.keymap.clear();
                } else {
                    app.prefs = reset_prefs(&app.prefs);
                    crate::prefs_ext::set_stream_opts(&app.prefs);
                    let _ = app.player.switch_device(app.prefs.audio_device.as_deref());
                    app.list_dirty = true;
                }
                app.save_prefs();
                app.dialogs.ext.reset_ask = None;
            }
            if ui.add(btn("Cancel").exact(vec2(80.0, 29.0))).clicked() {
                app.dialogs.ext.reset_ask = None;
            }
        }
    }
}

/// Default settings, keeping what is not a "setting" in the window's sense: the keyboard
/// profile, the track list's columns and sort, and this computer's devices and folders.
pub fn reset_prefs(old: &Prefs) -> Prefs {
    Prefs {
        keymap: old.keymap.clone(),
        columns: old.columns.clone(),
        columns_other: old.columns_other.clone(),
        columns_for_playlist: old.columns_for_playlist,
        sort_col: old.sort_col,
        sort_asc: old.sort_asc,
        sort_other: old.sort_other,
        widths: old.widths.clone(),
        devices_extra: old.devices_extra.clone(),
        audio_device: old.audio_device.clone(),
        stem_model: old.stem_model.clone(),
        relocate_folders: old.relocate_folders.clone(),
        itunes_xml: old.itunes_xml.clone(),
        rekordbox_xml_path: old.rekordbox_xml_path.clone(),
        other: old.other.clone(),
        ..Prefs::default()
    }
}

// ------------------------------------------------------------------ View

fn page_view(app: &mut App, ui: &mut Ui, tab: usize) {
    match tab {
        0 => {
            card(ui, "Language", |ui| {
                kit::combo_row(ui, 22.0, |ui| {
                    kit::combo("lang", 520.0, "English", 14.0).show_ui(ui, |ui| {
                        let _ = ui.selectable_label(true, "English");
                    });
                });
            });
            card(ui, "Tooltips", |ui| {
                toggle(ui, &mut app.prefs.tooltips, "Show Tooltips");
            });
            card(ui, "Browse", |ui| {
                sub(ui, "Font Size");
                let mut f = app.prefs.browse_font as u32;
                if kit::step_slider(ui, &mut f, 4, "Small", "Large") {
                    app.prefs.browse_font = f as u8;
                }
                kit::indent(ui, 84.0);
                ui.add_space(-0.5);
                toggle(ui, &mut app.prefs.browse_bold, "Bold");
                separator(ui);
                sub(ui, "Line Space");
                let mut l = app.prefs.browse_line as u32;
                if kit::step_slider(ui, &mut l, 4, "Small", "Large") {
                    app.prefs.browse_line = l as u8;
                }
                separator(ui);
                ui.add_space(-1.0);
                toggle(ui, &mut app.prefs.center_selected, "Show the selected track to the center");
                ui.add_space(0.5);
                separator(ui);
                // LINK EXPORT is not part of Recordbox: the section is shown switched off.
                let why = "recordbox has no LINK EXPORT, so no tracks are coloured as played.";
                sub(ui, "Coloring of played tracks  (LINK EXPORT)");
                let mut refused = kit::toggle_off(ui, "Reset color when exiting rekordbox", why);
                ui.add_space(6.0);
                refused |= kit::row(ui, 22.0, |ui| ui.add(btn("RESET").exact(vec2(264.0, 22.0)).font_size(12.0)).on_hover_text(why).clicked());
                if refused {
                    app.toast(why);
                }
            });
            card(ui, "Key display format", |ui| {
                if radio(ui, !app.prefs.key_alnum, "Classic") {
                    app.prefs.key_alnum = false;
                    app.list_dirty = true;
                }
                if radio(ui, app.prefs.key_alnum, "Alphanumeric") {
                    app.prefs.key_alnum = true;
                    app.list_dirty = true;
                }
            });
            card(ui, "Waveform", |ui| {
                kit::labeled(ui, "Playback animation", |ui| {
                    let choices = [(15, "15 frames/s"), (30, "30 frames/s"), (0, "Display rate")];
                    let current = choices.iter().find(|(n, _)| *n == app.prefs.playback_fps).map_or("Display rate", |v| v.1);
                    kit::combo("playback_fps", 170.0, current, 14.0).show_ui(ui, |ui| {
                        for (fps, name) in choices {
                            ui.selectable_value(&mut app.prefs.playback_fps, fps, name);
                        }
                    });
                });
                sub(ui, "Full/Preview Waveform");
                kit::indent(ui, 88.0);
                if radio(ui, !app.prefs.wave_half, "Full Waveform") {
                    app.prefs.wave_half = false;
                }
                if radio(ui, app.prefs.wave_half, "Half Waveform") {
                    app.prefs.wave_half = true;
                }
            });
            card(ui, "Beat Count Display", |ui| {
                for (i, n) in ["Current Position (Bars)", "Count to the next MEMORY CUE (Bars)", "Count to the next MEMORY CUE (Beats)"].iter().enumerate() {
                    if radio(ui, app.prefs.beat_count_mode == i as u8, n) {
                        app.prefs.beat_count_mode = i as u8;
                    }
                }
            });
        }
        1 => page_layout(app, ui),
        2 => {
            card(ui, "Appearance", |ui| {
                radio(ui, true, "Dark");
                kit::radio_off(ui, false, "Light", "recordbox has the dark appearance only.");
            });
            card(ui, "Waveform color", |ui| {
                for (i, n) in ["BLUE", "RGB", "3Band"].iter().enumerate() {
                    if radio(ui, app.prefs.wave_color == i as u8, n) {
                        app.prefs.wave_color = i as u8;
                    }
                }
            });
            card(ui, "HOT CUE color", |ui| {
                // Recordbox gives new hot cues rekordbox's COLORFUL colours; the other three
                // colour sets are listed as rekordbox lists them, and cannot be chosen.
                let why = "recordbox colours hot cues as rekordbox's COLORFUL setting does; it has no other colour set yet.";
                kit::combo_row(ui, 22.0, |ui| {
                    kit::combo("hotcue_color", 520.0, "COLORFUL", 14.0).show_ui(ui, |ui| {
                        for n in ["CDJ", "COLD1", "COLD2"] {
                            ui.add_enabled(false, egui::SelectableLabel::new(false, n)).on_disabled_hover_text(why);
                        }
                        let _ = ui.selectable_label(true, "COLORFUL");
                    });
                });
            });
        }
        _ => {
            card(ui, "recordbox", |ui| {
                kit::line(ui, "Settings of recordbox that rekordbox keeps elsewhere or does not have.");
            });
            card(ui, "Time Mode", |ui| {
                if radio(ui, !app.prefs.time_remain, "Elapsed") {
                    app.prefs.time_remain = false;
                }
                if radio(ui, app.prefs.time_remain, "Remaining") {
                    app.prefs.time_remain = true;
                }
            });
            card(ui, "Quantize", |ui| {
                toggle(ui, &mut app.prefs.quantize, "Cues and loops snap to the beat grid");
                kit::row(ui, 16.0, |ui| {
                    ui.label(kit::text("Resolution", 14.0).color(WHITE));
                    for (d, n) in [(1u8, "1 beat"), (2, "1/2"), (4, "1/4"), (8, "1/8")] {
                        ui.add_space(16.0);
                        if kit::radio_raw(ui, app.prefs.quantize_div.max(1) == d, n).clicked() {
                            app.prefs.quantize_div = d;
                        }
                    }
                });
            });
            card(ui, "Tree View", |ui| {
                note(ui, "Sources rekordbox always shows. The others are in Layout > Media Browser.");
                for name in crate::prefs_ext::TREE_ITEMS_OWN {
                    let mut on = app.prefs.tree_item(name);
                    if kit::checkbox(ui, &mut on, name).changed() {
                        app.prefs.tree_items.insert(name.to_string(), on);
                    }
                }
            });
            card(ui, "Browser", |ui| {
                toggle(ui, &mut app.prefs.show_mytag, "My Tag Configuration Window");
                if toggle(ui, &mut app.prefs.column_browser, "Column browser (Genre / Artist / Album)") {
                    app.list_dirty = true;
                }
                toggle(ui, &mut app.prefs.sub_browser, "Sub-Browser Window");
            });
        }
    }
}

/// View > Layout, as rekordbox 7.2.19 lays it out: one card with the sections Media Browser,
/// Browser panel, Deck, Phrases and Vocal.
fn page_layout(app: &mut App, ui: &mut Ui) {
    card(ui, "Layout", |ui| {
        sub(ui, "Media Browser");
        ui.add_space(2.0);
        let [left, right] = crate::prefs_ext::MEDIA_BROWSER;
        for i in 0..left.len() {
            kit::row(ui, 16.0, |ui| {
                let x0 = ui.cursor().min.x;
                for (col, name) in [left[i], right[i]].into_iter().enumerate() {
                    if col == 1 {
                        // The right column starts 287 pt after the left one.
                        ui.add_space((x0 + 287.0 - ui.cursor().min.x).max(8.0));
                    }
                    if crate::prefs_ext::tree_item_available(name) {
                        let mut on = app.prefs.tree_item(name);
                        if kit::checkbox_raw(ui, &mut on, name).changed() {
                            app.prefs.tree_items.insert(name.to_string(), on);
                        }
                    } else {
                        kit::checkbox_off_raw(ui, name, &crate::prefs_ext::media_absent_reason(name));
                    }
                }
            });
        }
        separator(ui);
        sub(ui, "Browser panel");
        // Shows or hides the CUE Analysis Playlist row (and its [Start] button) in the tree.
        let mut cue_pl = app.prefs.tree_item("CUE Analysis Playlist");
        if kit::checkbox(ui, &mut cue_pl, "CUE Analysis Playlist").changed() {
            app.prefs.tree_items.insert("CUE Analysis Playlist".to_string(), cue_pl);
            app.save_prefs();
        }
        kit::checkbox(ui, &mut app.prefs.palette_enabled, "Playlist Palette");
        kit::checkbox(ui, &mut app.prefs.cue_markers_on_preview, "Display Cue Markers on Preview");
        kit::checkbox(ui, &mut app.prefs.playlist_search_filter, "Playlist search filter");
        kit::checkbox(ui, &mut app.prefs.all_tracks_in_playlists, "Display All Tracks in the Playlists");
        kit::checkbox(ui, &mut app.prefs.playlist_track_counts, "Display the number of tracks in a playlist on the Tree View");
        ui.add_space(3.0);
        separator(ui);
        sub(ui, "Deck");
        kit::checkbox_off(ui, "Display SYNC RATE", "recordbox does not show a sync rate next to BEAT SYNC yet.");
        ui.add_space(3.0);
        separator(ui);
        sub(ui, "Phrases");
        ui.add_space(-4.0);
        if PHRASE_ENLARGED_WIRED {
            kit::checkbox(ui, &mut app.prefs.phrase_enlarged, "Phrase (Enlarged Waveform)");
        } else {
            kit::checkbox_off(ui, "Phrase (Enlarged Waveform)", "recordbox shows phrases under the full waveform only, so far.");
        }
        kit::checkbox(ui, &mut app.prefs.show_phrase_bar, "Phrase (Full Waveform)");
        ui.add_space(3.0);
        toggle_if(app, ui, PHRASE_TYPES_WIRED, |p| &mut p.phrase_types_always, "Always show types of phrases", "recordbox shows phrases as coloured blocks without their type names, so far.");
        ui.add_space(3.0);
        separator(ui);
        sub(ui, "Vocal");
        ui.add_space(-4.0);
        if VOCAL_ENLARGED_WIRED {
            kit::checkbox(ui, &mut app.prefs.vocal_enlarged, "Vocal (Enlarged Waveform)");
        } else {
            kit::checkbox_off(ui, "Vocal (Enlarged Waveform)", "recordbox shows vocal positions above the full waveform only, so far.");
        }
        kit::checkbox(ui, &mut app.prefs.show_vocal, "Vocal (Full Waveform)");
    });
}

// ------------------------------------------------------------------ Audio

fn apply_audio(app: &mut App) {
    crate::prefs_ext::set_stream_opts(&app.prefs);
    if let Err(e) = app.player.switch_device(app.prefs.audio_device.as_deref()) {
        // The device refused: go back to its own defaults.
        app.prefs.audio_rate = 0;
        app.prefs.audio_buffer = 0;
        app.prefs.audio_channel = 0;
        crate::prefs_ext::set_stream_opts(&app.prefs);
        let _ = app.player.switch_device(app.prefs.audio_device.as_deref());
        app.toast_err(format!("The audio device did not accept that setting: {e}"));
    }
}

/// Audio devices and what the one in use offers. Both questions can block, so they are asked
/// in the background (once per run, or again after Refresh).
struct AudioInfo {
    /// None while waiting, Some(None) if macOS gave no answer.
    state: Option<Option<Vec<String>>>,
    devices: Vec<String>,
    caps: Option<crate::prefs_ext::AudioCaps>,
    list_id: egui::Id,
    caps_id: egui::Id,
}

type DevSlot = std::sync::Arc<parking_lot::Mutex<Option<Option<Vec<String>>>>>;
type CapSlot = std::sync::Arc<parking_lot::Mutex<Option<Option<crate::prefs_ext::AudioCaps>>>>;

fn audio_info(app: &App, ui: &Ui) -> AudioInfo {
    let list_id = egui::Id::new("audio_devices");
    let slot: DevSlot = match ui.ctx().data(|d| d.get_temp::<DevSlot>(list_id)) {
        Some(s) => s,
        None => {
            let s: DevSlot = Default::default();
            let s2 = s.clone();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                *s2.lock() = Some(crate::audio::Player::output_devices_within(std::time::Duration::from_secs(5)));
                ctx.request_repaint();
            });
            ui.ctx().data_mut(|m| m.insert_temp(list_id, s.clone()));
            s
        }
    };
    let state = slot.lock().clone();
    let devices: Vec<String> = match &state {
        Some(Some(d)) => d.clone(),
        _ => vec![],
    };
    let current = app.prefs.audio_device.clone();
    let caps_id = egui::Id::new(("audio_caps", current.clone()));
    let cap_slot: CapSlot = match ui.ctx().data(|d| d.get_temp::<CapSlot>(caps_id)) {
        Some(s) => s,
        None => {
            let s: CapSlot = Default::default();
            let s2 = s.clone();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                *s2.lock() = Some(crate::prefs_ext::audio_caps_within(current, std::time::Duration::from_secs(5)));
                ctx.request_repaint();
            });
            ui.ctx().data_mut(|m| m.insert_temp(caps_id, s.clone()));
            s
        }
    };
    let caps = cap_slot.lock().clone().flatten();
    AudioInfo { state, devices, caps, list_id, caps_id }
}

fn page_audio(app: &mut App, ui: &mut Ui, tab: usize) {
    let info = audio_info(app, ui);
    if tab == 1 {
        audio_io(app, ui, &info);
        return;
    }
    let AudioInfo { state, devices, caps, list_id, caps_id } = info;
    let current = app.prefs.audio_device.clone();
    card(ui, "Audio", |ui| {
        let shown = current.clone().or_else(|| caps.as_ref().map(|c| c.name.clone()).filter(|n| !n.is_empty())).unwrap_or_else(|| "System default".to_string());
        let mut pick: Option<Option<String>> = None;
        kit::combo_row(ui, 22.0, |ui| {
            kit::combo("audio_dev", 386.0, shown, 14.0).show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), "System default").clicked() {
                    pick = Some(None);
                }
                for d in &devices {
                    if ui.selectable_label(current.as_deref() == Some(d.as_str()), d).clicked() {
                        pick = Some(Some(d.clone()));
                    }
                }
                if devices.is_empty() && state.is_some() {
                    ui.label(RichText::new("No audio device").color(DIM));
                }
            });
            ui.add_space(4.0);
            // rekordbox opens the device driver's own settings here; on a Mac those are in
            // Audio MIDI Setup.
            if ui.add(btn("Driver").exact(vec2(148.0, 22.0)).font_size(11.5)).on_hover_text("Open Audio MIDI Setup, where macOS keeps the settings of audio devices.").clicked() {
                let _ = std::process::Command::new("/usr/bin/open").args(["-a", "Audio MIDI Setup"]).spawn();
            }
        });
        if let Some(p) = pick {
            // Another device has its own rates, buffers and channels: start from its defaults.
            let keep = (app.prefs.audio_rate, app.prefs.audio_buffer, app.prefs.audio_channel);
            app.prefs.audio_rate = 0;
            app.prefs.audio_buffer = 0;
            app.prefs.audio_channel = 0;
            crate::prefs_ext::set_stream_opts(&app.prefs);
            match app.player.switch_device(p.as_deref()) {
                Ok(()) => app.prefs.audio_device = p,
                Err(e) => {
                    (app.prefs.audio_rate, app.prefs.audio_buffer, app.prefs.audio_channel) = keep;
                    crate::prefs_ext::set_stream_opts(&app.prefs);
                    app.toast_err(format!("Could not open that audio device: {e}"));
                }
            }
        }
        // The original leaves 32 pt free under the dropdown; Recordbox says there what is
        // running, and anything that went wrong.
        let dev = caps.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        let (status, warn) = match (&state, &current) {
            (None, _) => ("Looking for audio devices…".to_string(), false),
            (Some(None), _) => ("macOS did not answer with the list of audio devices; the system's default output is used.".to_string(), true),
            (Some(Some(_)), Some(d)) if !devices.contains(d) => (format!("\"{d}\" is not connected; the system's default output is used."), true),
            _ => (format!("Running at {} Hz{}.", app.player.device_rate, if dev.is_empty() { String::new() } else { format!(" on {dev}") }), false),
        };
        kit::row_at(ui, kit::Kind::Note, 18.0, 52.0, |ui| {
            ui.label(kit::text(status, 11.0).color(if warn { CAUTION_TX } else { DIM }));
            ui.add_space(10.0);
            if ui.add(btn("Refresh").small()).on_hover_text("Look for audio devices again").clicked() {
                ui.ctx().data_mut(|m| {
                    m.remove::<DevSlot>(list_id);
                    m.remove::<CapSlot>(caps_id);
                });
            }
        });
        ui.add_space(8.0);
    });
    card(ui, "Sample Rate", |ui| {
        let rate_text = |r: u32| format!("{} Hz", r);
        let sel = rate_text(if app.prefs.audio_rate == 0 { app.player.device_rate } else { app.prefs.audio_rate });
        let mut new_rate = None;
        kit::combo_row(ui, 22.0, |ui| {
            ui.add_enabled_ui(caps.is_some(), |ui| {
                kit::combo("audio_rate", 536.0, sel, 14.0).show_ui(ui, |ui| {
                    if ui.selectable_label(app.prefs.audio_rate == 0, "Device setting").clicked() {
                        new_rate = Some(0);
                    }
                    for r in caps.as_ref().map(|c| c.rates.clone()).unwrap_or_default() {
                        if ui.selectable_label(app.prefs.audio_rate == r, rate_text(r)).clicked() {
                            new_rate = Some(r);
                        }
                    }
                });
            });
        });
        if let Some(r) = new_rate.filter(|r| *r != app.prefs.audio_rate) {
            app.prefs.audio_rate = r;
            apply_audio(app);
        }
    });
    card(ui, "Buffer size", |ui| {
        let range = caps.as_ref().and_then(|c| c.buffer);
        let b = app.prefs.audio_buffer;
        let label = if b == 0 {
            "Device setting".to_string()
        } else {
            format!("{b} samples ({:.1} ms)", b as f64 / app.player.device_rate.max(1) as f64 * 1000.0)
        };
        ui.add_space(1.0);
        kit::row_at(ui, kit::Kind::Ctl, 16.0, 57.5, |ui| {
            let r = ui.max_rect();
            kit::paint_text(ui, pos2(r.min.x, r.center().y), egui::Align2::LEFT_CENTER, &label, inter(14.0), WHITE, 0.1);
        });
        ui.add_space(5.0);
        let mut t = if b == 0 { 0.0 } else { crate::prefs_ext::buffer_to_slider(b) };
        let resp = ui.add_enabled_ui(caps.is_some(), |ui| kit::slider(ui, &mut t)).inner;
        // The stream is reopened when the handle is let go, not at every step on the way.
        let drag_id = egui::Id::new("audio_buffer_drag");
        let mut pending: Option<u32> = ui.ctx().data(|d| d.get_temp(drag_id)).flatten();
        if resp.changed() {
            pending = Some(crate::prefs_ext::slider_to_buffer(t, range));
        }
        if let Some(nb) = pending {
            app.prefs.audio_buffer = nb;
            if !resp.is_pointer_button_down_on() {
                pending = None;
                if nb != b || resp.drag_stopped() || resp.clicked() {
                    apply_audio(app);
                }
            }
        }
        ui.ctx().data_mut(|d| d.insert_temp(drag_id, pending));
        resp.on_hover_text("64 to 2048 samples. A larger buffer avoids drop-outs; a smaller one reacts sooner. \"Reset to defaults\" goes back to the device's own setting.");
    });
    kit::card_x(ui, "Metronome", vec2(-8.0, -5.0), None, |ui| {
        for (i, n) in ["Click Sound 01", "Click Sound 02", "Click Sound 03"].iter().enumerate() {
            if radio(ui, app.prefs.metronome_sound == i as u8, n) {
                app.prefs.metronome_sound = i as u8;
                crate::prefs_ext::set_stream_opts(&app.prefs);
            }
        }
        kit::separator_col(ui, hex(0xFEFEFE));
        sub(ui, "Volume");
        kit::indent(ui, 88.0);
        let vol = app.prefs.metronome_vol.clamp(1, 3);
        for (i, n) in ["Small", "Middle", "Large"].iter().enumerate() {
            if radio(ui, vol == i as u8 + 1, n) {
                app.set_metronome_volume(i as u8 + 1);
            }
        }
    });
    card(ui, "Preview Volume", |ui| {
        let mut t = app.prefs.preview_volume.clamp(0.0, 1.0);
        if kit::slider(ui, &mut t).on_hover_text("How loud tracks play from the Preview column. In the middle they play at the master volume.").changed() {
            app.prefs.preview_volume = t;
        }
    });
}

/// Audio > Input/Output. rekordbox's EXPORT mode has the output channels here (Mixer Mode
/// and the input channels belong to PERFORMANCE mode and recording).
fn audio_io(app: &mut App, ui: &mut Ui, info: &AudioInfo) {
    let chans = info.caps.as_ref().map(|c| c.channels).unwrap_or(2);
    let dev = info.caps.as_ref().map(|c| c.name.clone()).unwrap_or_default();
    let cur = if app.prefs.audio_channel + 1 < chans { app.prefs.audio_channel } else { 0 };
    let name = |ch: u16| if dev.is_empty() { format!("{}", ch + 1) } else { format!("{dev} : {}", ch + 1) };
    let mut new_ch = None;
    let mut new_hp: Option<Option<u16>> = None;
    card(ui, "Output channels", |ui| {
        sub(ui, "Master Output");
        // The pair moves together: the right channel is always the one after the left.
        for (side, label) in ["L", "R"].iter().enumerate() {
            kit::row_at(ui, kit::Kind::Ctl, 22.0, 89.5, |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(40.5, 22.0), egui::Sense::hover());
                kit::paint_text(ui, pos2(r.min.x, r.center().y), egui::Align2::LEFT_CENTER, label, inter(14.0), WHITE, 0.0);
                let shown = if chans >= 2 { name(cur + side as u16) } else if side == 0 { name(0) } else { "-".to_string() };
                kit::combo(("audio_ch", side), 340.0, shown, 14.0).show_ui(ui, |ui| {
                    let mut first = 0;
                    while first + 1 < chans {
                        if ui.selectable_label(cur == first, name(first + side as u16)).clicked() {
                            new_ch = Some(first);
                        }
                        first += 2;
                    }
                    if chans <= 2 {
                        ui.label(kit::text("This device has one stereo output.", 11.0).color(DIM));
                    }
                });
            });
        }
        // Headphones Output: another pair of the same device (an interface or controller
        // with four or more outputs), or none.
        sub(ui, "Headphones Output");
        let hp = app.prefs.hp_channel.filter(|h| h + 1 < chans && *h != cur);
        for (side, label) in ["L", "R"].iter().enumerate() {
            kit::row_at(ui, kit::Kind::Ctl, 22.0, 89.5, |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(40.5, 22.0), egui::Sense::hover());
                kit::paint_text(ui, pos2(r.min.x, r.center().y), egui::Align2::LEFT_CENTER, label, inter(14.0), WHITE, 0.0);
                let shown = hp.map(|h| name(h + side as u16)).unwrap_or_else(|| "-".to_string());
                kit::combo(("audio_hp", side), 340.0, shown, 14.0).show_ui(ui, |ui| {
                    if ui.selectable_label(hp.is_none(), "-").clicked() {
                        new_hp = Some(None);
                    }
                    let mut first = 0;
                    while first + 1 < chans {
                        if first != cur && ui.selectable_label(hp == Some(first), name(first + side as u16)).clicked() {
                            new_hp = Some(Some(first));
                        }
                        first += 2;
                    }
                    if chans <= 2 {
                        ui.label(kit::text("This device has one stereo output: choose a device with four or more outputs (an audio interface, a controller, or an Aggregate Device made in Audio MIDI Setup) to hear the CUE in headphones.", 11.0).color(DIM));
                    }
                });
            });
        }
    });
    if let Some(c) = new_ch.filter(|c| *c != app.prefs.audio_channel) {
        app.prefs.audio_channel = c;
        // A headphones pair on the same channels gives way.
        if app.prefs.hp_channel == Some(c) {
            app.prefs.hp_channel = None;
        }
        apply_audio(app);
    }
    if let Some(h) = new_hp.filter(|h| *h != app.prefs.hp_channel) {
        app.prefs.hp_channel = h;
        apply_audio(app);
    }
}

// ------------------------------------------------------------------ Analysis

fn page_analysis(app: &mut App, ui: &mut Ui, tab: usize) {
    if tab == 1 {
        card(ui, "CUE Analysis", |ui| {
            toggle(ui, &mut app.prefs.memory_cue_first_beat, "Set Memory Cue at the first beat of track when analyzing");
            note(ui, "For tracks that have no memory cue yet.");
            separator(ui);
            toggle(ui, &mut app.prefs.auto_first_downbeat_cue, "Set Hot Cue A on the first downbeat (\"1.1Bars\") when analyzing");
            note(ui, "As rekordbox does on first analysis. Only for tracks that have no cues yet; tracks from your rekordbox library keep their own cues.");
        });
        return;
    }
    let no_cloud = "recordbox has no cloud service: nothing is collected or uploaded.";
    card(ui, "Track Analysis", |ui| {
        kit::row(ui, 16.0, |ui| {
            kit::toggle_raw(ui, &mut app.prefs.refine_beatgrid_transients, "Use high precision BeatGrid analysis");
            kit::help(ui, 37.5, "Refine beat timing and tempo against repeated attacks in the audio. Works with Normal and Dynamic. Use Analyze Track to update an existing unlocked grid.");
        });
        separator(ui);
        kit::sub_help(ui, "Track Analysis Mode", Some("Normal: optimized for tracks which have a constant BPM.\nDynamic: for tracks whose tempo changes, the beat grid follows the tempo."));
        kit::labeled(ui, "Analysis Mode", |ui| {
            let modes = [("Normal", "Optimized for tracks which have a constant BPM"), ("Dynamic", "For tracks whose tempo changes: the beat grid follows the tempo")];
            let cur = (app.prefs.analysis_mode as usize).min(1);
            kit::combo("amode", 400.0, modes[cur].0, 14.0).show_ui(ui, |ui| {
                for (i, (n, tip)) in modes.iter().enumerate() {
                    if ui.selectable_label(cur == i, *n).on_hover_text(*tip).clicked() {
                        app.prefs.analysis_mode = i as u8;
                    }
                }
            });
        });
        kit::labeled(ui, "Analysis workers", |ui| {
            let before = app.prefs.analysis_threads;
            let label = if before == 0 { format!("Automatic ({})", app.pool.thread_count()) } else { app.pool.thread_count().to_string() };
            let busy = app.pool.pending.load(std::sync::atomic::Ordering::Acquire) != 0;
            ui.add_enabled_ui(!busy, |ui| {
                kit::combo("analysis_workers", 170.0, label, 14.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.prefs.analysis_threads, 0, "Automatic");
                    for n in 1..=app.pool.available_cpus.min(12) {
                        ui.selectable_value(&mut app.prefs.analysis_threads, n, n.to_string());
                    }
                });
            }).response.on_hover_text("Automatic leaves one processor available for playback when possible. Change this between analysis batches.");
            if app.prefs.analysis_threads != before { app.pool.set_threads(app.prefs.analysis_threads); }
        });
        kit::labeled(ui, "BPM Range", |ui| {
            let (lo, hi) = BPM_RANGES[app.prefs.bpm_range.min(BPM_RANGES.len() - 1)];
            kit::combo("bpmrange", 400.0, format!("{lo:.0}-{hi:.0}"), 14.0).show_ui(ui, |ui| {
                for (i, (a, b)) in BPM_RANGES.iter().enumerate() {
                    ui.selectable_value(&mut app.prefs.bpm_range, i, format!("{a:.0}-{b:.0}"));
                }
            });
        });
        separator(ui);
        sub(ui, "Track Analysis Setting");
        kit::indent(ui, 110.0);
        kit::row(ui, 16.0, |ui| {
            kit::checkbox_raw(ui, &mut app.prefs.analyze_grid, "BPM / Grid");
            kit::help(ui, 54.0, "If tracks are not analyzed, BPM/Grid analysis starts when importing, loading, or exporting the tracks whether you tick this checkbox or not.");
        });
        kit::checkbox(ui, &mut app.prefs.analyze_key, "KEY");
        kit::checkbox(ui, &mut app.prefs.phrase_analysis, "Phrase").on_hover_text("Phrases (Intro, Up, Down, Chorus, Outro) are shown under the overview and written to USB exports. rekordbox's own phrases are used for tracks it analysed.");
        kit::checkbox(ui, &mut app.prefs.vocal_analysis, "Vocal").on_hover_text("Vocal positions for loaded tracks rekordbox never analysed, found with the stem separation model once it is downloaded (about a minute per track on this Mac's GPU, in the background). rekordbox's own vocal detection is used for tracks it analysed.");
        separator(ui);
        sub(ui, "Auto Analysis");
        toggle(ui, &mut app.prefs.auto_analysis, "Enable");
        separator(ui);
        // The cloud services are not part of Recordbox: shown switched off.
        ui.add_space(-3.5);
        kit::sub_help(ui, "Cloud Analysis", Some(no_cloud));
        ui.add_space(1.5);
        let mut refused = kit::toggle_off(ui, "Participate in rekordbox data collection\nand upload analysis data", no_cloud);
        ui.add_space(5.0);
        separator(ui);
        ui.add_space(-3.5);
        kit::sub_help(ui, "Radar Analysis", Some(no_cloud));
        ui.add_space(1.5);
        refused |= kit::toggle_off(ui, "Enable", no_cloud);
        if refused {
            app.toast(no_cloud);
        }
    });
    card(ui, "Key Analysis", |ui| {
        let why = "recordbox never changes your audio files; the key is kept in its own library.";
        if kit::toggle_off(ui, "Write the value to the ID3 tag", why) {
            app.toast(why);
        }
    });
    card(ui, "Auto Gain (recordbox)", |ui| {
        kit::block(ui, |ui| {
            ui.add(egui::Slider::new(&mut app.prefs.target_lufs, -20.0..=-5.0).suffix(" LUFS").text("Reference loudness"));
        });
        toggle(ui, &mut app.prefs.auto_gain_peak_cap, "Limit gain so the loudest peak stays below full scale");
        note(ui, "Used for the preview player's gain and stored per track when analysing.");
        kit::block(ui, |ui| {
            if ui.add(btn("Re-analyze all tracks now")).clicked() {
                let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
                app.reanalyze(&ids);
            }
        });
    });
}

// ------------------------------------------------------------------ DJ System

/// A widget at an exact place inside a card that is laid out by coordinates.
fn place(ui: &mut Ui, r: Rect, enabled: bool, w: impl egui::Widget) -> egui::Response {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_enabled(enabled, w)
}

/// The grey 20 pt square with a white arrow head under the Active list.
fn order_button(ui: &mut Ui, r: Rect, up: bool, enabled: bool, tip: &str) -> bool {
    let resp = ui.interact(r, ui.id().with(("order", up, r.min.x as i32)), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    ui.painter().rect_filled(r, 2.0, if !enabled { hex(0x3A3A3A) } else if resp.hovered() { hex(0x666666) } else { hex(0x555555) });
    let (a, b) = if up { (r.min.y + 12.0, r.min.y + 8.0) } else { (r.min.y + 8.0, r.min.y + 12.0) };
    let pts = vec![pos2(r.min.x + 6.0, a), pos2(r.min.x + 10.0, b), pos2(r.min.x + 14.0, a)];
    ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.0, if enabled { WHITE } else { OFF })));
    resp.on_hover_text(tip).clicked()
}

/// rekordbox's two lists: pick a name, then move it with the arrows.
fn dual_list(app: &mut App, ui: &mut Ui, is_cat: bool) {
    use crate::export::usb::{column_name, UsbBrowse, FIXED_CATEGORIES, FIXED_SORTS};
    let all = if is_cat { UsbBrowse::all_categories() } else { UsbBrowse::all_sorts() };
    let fixed: &[u16] = if is_cat { &FIXED_CATEGORIES } else { &FIXED_SORTS };
    let mut b = app.prefs.usb_browse.normalized();
    let active: Vec<u16> = if is_cat { b.categories.clone() } else { b.sorts.clone() };
    let mut next = active.clone();
    let sel_id = egui::Id::new(("dual_list_sel", is_cat));
    let mut sel: (Option<u16>, Option<u16>) = ui.ctx().data(|d| d.get_temp(sel_id)).unwrap_or((None, None));
    let mut inactive: Vec<(u16, &str)> = all.iter().copied().filter(|(id, _)| !active.contains(id)).collect();
    inactive.sort_by_key(|x| x.1.to_uppercase());
    let row = |ui: &mut Ui, name: &str, selected: bool, greyed: bool| -> bool {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), egui::Sense::click());
        if selected {
            ui.painter().rect_filled(r, 0.0, PBLUE);
        }
        ui.painter().text(pos2(r.min.x + 6.0, r.center().y), egui::Align2::LEFT_CENTER, name.to_uppercase(), inter(13.0), if greyed { OFF } else { WHITE });
        resp.clicked()
    };
    // The card and everything in it at the original's coordinates (628 x 484 pt).
    let w = ui.available_width().min(kit::CARD_W);
    let (card, _) = ui.allocate_exact_size(vec2(w, 484.0), egui::Sense::hover());
    ui.painter().rect_filled(card, 0.0, BG2);
    let at = |x: f32, y: f32| pos2(card.min.x + x, card.min.y + y);
    let left = Rect::from_min_max(at(20.0, 54.0), at(284.0, 464.0));
    let right = Rect::from_min_max(at(344.0, 54.0), at(608.0, 441.0));
    ui.painter().text(pos2(left.center().x, card.min.y + 29.5), egui::Align2::CENTER_CENTER, if is_cat { "Inactive Categories" } else { "Inactive Sort Options" }, inter_bold(14.0), WHITE);
    ui.painter().text(pos2(right.center().x, card.min.y + 30.75), egui::Align2::CENTER_CENTER, if is_cat { "Active Categories" } else { "Active Sort Options" }, inter_bold(15.75), WHITE);
    ui.painter().rect_filled(left, 0.0, BG);
    ui.painter().rect_filled(right, 0.0, BG);
    let list = |ui: &mut Ui, r: Rect, salt: &str, add: &mut dyn FnMut(&mut Ui)| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
        egui::ScrollArea::vertical().id_salt((salt, is_cat)).max_height(r.height()).auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.add_space(1.0);
            add(ui);
        });
    };
    list(ui, left, "dl_in", &mut |ui| {
        for (id, name) in &inactive {
            if row(ui, name, sel.0 == Some(*id), false) {
                sel = (Some(*id), None);
            }
        }
    });
    list(ui, right, "dl_ac", &mut |ui| {
        for id in &active {
            let name = column_name(*id).unwrap_or("?");
            if row(ui, name, sel.1 == Some(*id), fixed.contains(id)) {
                sel = (None, Some(*id));
            }
        }
    });
    let can_add = sel.0.is_some();
    if place(ui, Rect::from_min_size(at(304.0, 221.0), vec2(20.0, 20.0)), can_add, btn(">").exact(vec2(20.0, 20.0)).font_size(12.0)).on_hover_text("Make active").clicked() {
        if let Some(id) = sel.0.take() {
            next.push(id);
            sel.1 = Some(id);
        }
    }
    let can_remove = sel.1.map(|id| !fixed.contains(&id)).unwrap_or(false);
    if place(ui, Rect::from_min_size(at(304.0, 251.0), vec2(20.0, 20.0)), can_remove, btn("<").exact(vec2(20.0, 20.0)).font_size(12.0)).on_hover_text("Make inactive").on_disabled_hover_text("Greyed items are always shown on the players.").clicked() {
        if let Some(id) = sel.1.take() {
            next.retain(|x| *x != id);
            sel.0 = Some(id);
        }
    }
    // Order: categories move freely; fixed sort options stay where the players put them.
    let pos = sel.1.and_then(|id| active.iter().position(|x| *x == id));
    let movable = |i: usize| is_cat || !fixed.contains(&active[i]);
    let can_down = pos.map(|i| i + 1 < active.len() && movable(i) && movable(i + 1)).unwrap_or(false);
    let can_up = pos.map(|i| i > 0 && movable(i) && movable(i - 1)).unwrap_or(false);
    if order_button(ui, Rect::from_min_size(at(452.0, 446.0), vec2(20.0, 20.0)), false, can_down, "Down") {
        if let Some(i) = pos {
            next.swap(i, i + 1);
        }
    }
    if order_button(ui, Rect::from_min_size(at(478.0, 446.0), vec2(20.0, 20.0)), true, can_up, "Up") {
        if let Some(i) = pos {
            next.swap(i, i - 1);
        }
    }
    ui.add_space(4.0);
    ui.ctx().data_mut(|d| d.insert_temp(sel_id, sel));
    if next != active {
        if is_cat {
            b.categories = next;
        } else {
            b.sorts = next;
        }
        app.prefs.usb_browse = b.normalized();
    }
}

fn setting_rows(app: &mut App, ui: &mut Ui, id: &str, fields: &[crate::export::settings::Field]) {
    kit::block(ui, |ui| {
        ui.spacing_mut().interact_size.y = 19.0;
        ui.spacing_mut().button_padding.y = 0.0;
        egui::Grid::new(id).num_columns(2).spacing(vec2(24.0, 7.0)).show(ui, |ui| {
            for f in fields {
                // Waveform Divisions is on the General tab, as in rekordbox.
                if f.file == SettingFile::My2 && f.offset == 4 {
                    continue;
                }
                ui.label(kit::text(f.name.to_uppercase(), 13.0).color(WHITE));
                let cur = app.prefs.usb_settings.get(f.file, f.offset);
                let shown = f.options.iter().find(|o| o.1 == cur).map(|o| o.0).unwrap_or("—");
                let file_tag = match f.file {
                    SettingFile::My => 0u8,
                    SettingFile::My2 => 1,
                    SettingFile::Dev => 2,
                    SettingFile::Djm => 3,
                };
                kit::combo(("myset", f.offset, file_tag), 240.0, shown, 12.0).show_ui(ui, |ui| {
                    for (label, v) in f.options {
                        if ui.selectable_label(cur == *v, *label).clicked() {
                            app.prefs.usb_settings.set(f.file, f.offset, *v);
                        }
                    }
                });
                ui.end_row();
            }
        });
    });
}

pub(crate) fn page_dj_system(app: &mut App, ui: &mut Ui, tab: usize) {
    match tab {
        0 => {
            // No AlphaTheta account in Recordbox: the section is shown switched off.
            card(ui, "Display username on-screen", |ui| {
                sub(ui, "Display the AlphaTheta account nickname on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let why = "recordbox has no AlphaTheta account, so there is no nickname to show.";
                if kit::toggle_off(ui, "OFF", why) {
                    app.toast(why);
                }
            });
            card(ui, "Background Color", |ui| {
                sub(ui, "Color of device displayed on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let names = ["Default Color", "Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple"];
                let cur = (app.prefs.device_bg_color as usize).min(8);
                kit::combo_row(ui, 19.0, |ui| {
                    let r = kit::combo("bgcolor", 500.0, names[cur], 12.0).show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            let text = RichText::new(format!("■ {n}")).color(if i == 0 { DIM2 } else { label_color(i as u8) });
                            if ui.selectable_label(cur == i, text).clicked() {
                                app.prefs.device_bg_color = i as u8;
                            }
                        }
                    });
                    if !BG_COLOR_WRITTEN {
                        r.response.on_hover_text("Not written to the device yet: where the players keep this colour has not been confirmed. Your choice is kept for when it is.");
                    }
                });
                ui.add_space(-3.0);
            });
            card(ui, "Waveform color", |ui| {
                sub(ui, "Waveform color displayed on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_WAVEFORM_COLOR);
                for (label, v) in [("BLUE", 1u8), ("RGB", 3), ("3Band", 4)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_WAVEFORM_COLOR, v);
                    }
                }
            });
            card(ui, "Waveform Current Position", |ui| {
                sub(ui, "Waveform Current Position displayed on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_WAVEFORM_POSITION);
                for (label, v) in [("LEFT", 2u8), ("CENTER", 1)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_WAVEFORM_POSITION, v);
                    }
                }
            });
            kit::card_help(ui, "Waveform Divisions", Some("Time Scale: bar lines under the waveform.\nPhrase: the track's phrases (Intro, Up, Down, Chorus, Outro) under the waveform, on players that show them."), |ui| {
                sub(ui, "Waveform division displayed under the CDJ/XDJ waveform");
                kit::indent(ui, 88.0);
                let cur = app.prefs.usb_settings.get(SettingFile::My2, 4);
                for (label, v) in [("Time Scale", 0x80u8), ("Phrase", 0x81)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::My2, 4, v);
                    }
                }
            });
            card(ui, "Type of the Overview Waveform", |ui| {
                sub(ui, "Type of the Overview Waveform displayed on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_OVERVIEW_TYPE);
                for (label, v) in [("Half Waveform", 1u8), ("Full Waveform", 2)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_OVERVIEW_TYPE, v);
                    }
                }
            });
        }
        1 => dual_list(app, ui, true),
        2 => dual_list(app, ui, false),
        3 => {
            card(ui, "Column", |ui| {
                sub(ui, "Select item which is shown next to track name on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let why = "Where the players store this choice has not been confirmed, so recordbox does not write it; the players show their own default.";
                kit::combo_row(ui, 19.0, |ui| {
                    ui.add_enabled_ui(false, |ui| {
                        kit::combo("rightcol", 500.0, "Not Specified", 12.0).show_ui(ui, |_| {});
                    })
                    .response
                    .on_disabled_hover_text(why);
                });
                note(ui, "Not written yet: where the players store this choice has not been confirmed, so they show their own default.");
            });
        }
        4 => {
            card(ui, "My Settings", |ui| {
                kit::line(ui, "Player settings written to every USB you export. CDJ/XDJ players apply them when the USB is loaded.");
                sub(ui, "CDJ / XDJ");
                setting_rows(app, ui, "my_settings_grid", crate::export::settings_fields::MY_SETTINGS);
                separator(ui);
                sub(ui, "DJM");
                setting_rows(app, ui, "djm_settings_grid", crate::export::settings_fields::DJM_SETTINGS);
                kit::block(ui, |ui| {
                    if ui.add(btn("Reset to rekordbox defaults")).clicked() {
                        // The General tab's choices (DEVSETTING and Waveform Divisions) stay.
                        let keep = (app.prefs.usb_settings.dev.clone(), app.prefs.usb_settings.get(SettingFile::My2, 4));
                        app.prefs.usb_settings = crate::export::settings::UsbSettings { dev: keep.0, ..Default::default() };
                        app.prefs.usb_settings.set(SettingFile::My2, 4, keep.1);
                    }
                });
            });
        }
        5 => {
            card(ui, "Delete Tracks", |ui| {
                toggle(ui, &mut app.prefs.delete_device_tracks, "Delete playlist tracks from device\nif playlist is deleted");
            });
            card(ui, "History", |ui| {
                toggle(ui, &mut app.prefs.import_device_history, "Import the play history automatically");
                toggle_if(app, ui, HISTORY_DELETE_WIRED, |p| &mut p.delete_history_after_import, "Delete from the device after importing the play history", "recordbox does not remove histories from a device yet; imported histories are left on it.");
            });
        }
        _ => {
            card(ui, "Hot Cue Auto Load", |ui| {
                toggle(ui, &mut app.prefs.hot_cue_auto_load_on_import, "Prepare Hot Cue Auto Load setting for CDJ/XDJ use (on import)");
                note(ui, "Tracks get Hot Cue Auto Load switched on as they are imported, so players set to \"rekordbox SETTING\" load their hot cues.");
            });
            card(ui, "Waveform data written to the device", |ui| {
                kit::line(ui, "Exports always include the blue, RGB and 3-band waveforms; DJ System > General chooses which one the players show.");
            });
            card(ui, "Supported devices", |ui| {
                kit::line(ui, "Drive formats: FAT32, exFAT (some players), HFS+ (USB only).");
                kit::line(ui, "Files: MP3, AAC/M4A, WAV, AIFF, FLAC.");
                note(ui, "Device library: export.pdb + exportExt.pdb with analysis files (DAT / EXT / 2EX) and the music in /Contents.");
            });
        }
    }
}

// ------------------------------------------------------------------ Keyboard

/// Shortcuts the menus and text fields keep for themselves.
const RESERVED: [&str; 9] = ["Cmd+N", "Cmd+E", "Cmd+I", "Cmd+Z", "Shift+Cmd+Z", "Cmd+A", "Cmd+F", "Cmd+Q", "Cmd+H"];

fn page_keyboard(app: &mut App, ui: &mut Ui, tab: usize) {
    let cat = ACTION_CATEGORIES[tab.min(ACTION_CATEGORIES.len() - 1)];
    card(ui, cat, |ui| {
        kit::line(ui, "Click a shortcut, then press the new key for that action, with Ctrl, Opt, Shift or ⌘ if you like (Esc cancels, Delete clears).");
        let wait_id = egui::Id::new("key_wait");
        let waiting: Option<String> = ui.ctx().data(|d| d.get_temp(wait_id)).flatten();
        if let Some(action) = &waiting {
            let got = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some(Shortcut::from_event(*key, modifiers)),
                    _ => None,
                })
            });
            if let Some(sc) = got {
                let plain = !(sc.ctrl || sc.alt || sc.shift || sc.cmd);
                if sc.key == egui::Key::Escape {
                } else if plain && matches!(sc.key, egui::Key::Delete | egui::Key::Backspace) && action != "remove" {
                    app.prefs.keymap.insert(action.clone(), String::new());
                } else if RESERVED.contains(&sc.name().as_str()) {
                    app.toast_err(format!("{} is kept by the menus.", key_text(&sc)));
                } else {
                    // A shortcut belongs to one action: take it away from any other.
                    let name = sc.name();
                    let others: Vec<String> = ACTIONS.iter().map(|a| a.0.to_string()).filter(|a| a != action && app.prefs.shortcut_for(a) == Some(sc)).collect();
                    for o in others {
                        app.prefs.keymap.insert(o, String::new());
                    }
                    app.prefs.keymap.insert(action.clone(), name);
                }
                ui.ctx().data_mut(|d| d.insert_temp::<Option<String>>(wait_id, None));
            }
        }
        kit::block(ui, |ui| egui::Grid::new(("keymap_grid", tab)).num_columns(3).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
            for (id, label, def, c) in ACTIONS {
                if c != cat {
                    continue;
                }
                ui.label(kit::text(label, 13.0).color(WHITE));
                let shown = if waiting.as_deref() == Some(id) { "Press a key…".to_string() } else { app.prefs.shortcut_for(id).map(|k| key_text(&k)).unwrap_or_else(|| "—".into()) };
                if ui.add(btn(shown).min_size(vec2(140.0, 22.0))).clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(wait_id, Some(id.to_string())));
                }
                let changed = app.prefs.keymap.contains_key(id);
                if ui.add_enabled(changed, btn("Default").small()).on_hover_text(if def.is_empty() { "Not assigned".to_string() } else { Shortcut::parse(def).map(|s| key_text(&s)).unwrap_or_default() }).clicked() {
                    app.prefs.keymap.remove(id);
                }
                ui.end_row();
            }
        }));
    });
    card(ui, "Keyboard shortcuts", |ui| {
        kit::block(ui, |ui| ui.horizontal(|ui| {
            if ui.add(btn("Reset to defaults")).clicked() {
                app.dialogs.ext.reset_ask = Some(Some(3));
            }
            if ui.add(btn("Export…")).clicked() {
                if let Some(f) = rfd::FileDialog::new().set_file_name("recordbox-shortcuts.json").save_file() {
                    let _ = std::fs::write(f, serde_json::to_vec_pretty(&app.prefs.keymap).unwrap_or_default());
                }
            }
            if ui.add(btn("Import…")).clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                    match std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<std::collections::BTreeMap<String, String>>(&b).ok()) {
                        Some(m) => app.prefs.keymap = m.into_iter().filter(|(k, v)| ACTIONS.iter().any(|a| a.0 == k) && (v.is_empty() || Shortcut::parse(v).is_some())).collect(),
                        None => app.toast_err("That file is not a recordbox shortcut profile."),
                    }
                }
            }
            if ui.add(btn("Show all…")).clicked() {
                app.dialogs.ext.shortcuts_open = true;
            }
        }));
        note(ui, "⌘ and Ctrl shortcuts of Import file, Import directory, Fullscreen mode and Preferences also appear in the menu bar.");
    });
}

// ------------------------------------------------------------------ Advanced

fn page_advanced(app: &mut App, ui: &mut Ui, tab: usize) {
    match tab {
        0 => {
            card(ui, "Database management", |ui| {
                sub(ui, "Location");
                kit::block(ui, |ui| path_row(ui, &Some(crate::db::data_dir().display().to_string()), ""));
                note(ui, "Your music files are only read. Edits (tags, cues, grids) are kept in this private library.");
                kit::block(ui, |ui| {
                    if ui.add(btn("Show in Finder")).clicked() {
                        let _ = std::process::Command::new("/usr/bin/open").arg(crate::db::data_dir()).spawn();
                    }
                });
                separator(ui);
                sub(ui, "Library");
                let busy = app.dialogs.ext.backup_job.is_some();
                kit::block(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy, btn("Backup Library").min_size(vec2(150.0, 22.0))).clicked() {
                            crate::menus::handle(app, "backup");
                        }
                        if ui.add_enabled(!busy, btn("Restore Library").min_size(vec2(150.0, 22.0))).clicked() {
                            crate::menus::handle(app, "restore");
                        }
                    });
                });
                note(ui, "A backup is one .zip file with the database, the artwork and, if you choose, the music files. Before a restore replaces the library, the present one is copied into the backups folder here.");
            });
        }
        1 => {
            card(ui, "iTunes", |ui| {
                sub(ui, "iTunes Library File");
                kit::block(ui, |ui| {
                    path_row(ui, &app.prefs.itunes_xml, "no recently selected files");
                    ui.horizontal(|ui| {
                        if ui.add(btn("Browse")).clicked() {
                            if let Some(f) = rfd::FileDialog::new().add_filter("iTunes Library", &["xml"]).pick_file() {
                                app.prefs.itunes_xml = Some(f.to_string_lossy().to_string());
                            }
                        }
                        if app.prefs.itunes_xml.is_some() && ui.add(btn("Clear")).clicked() {
                            app.prefs.itunes_xml = None;
                        }
                    });
                });
                note(ui, "The library file the iTunes source of the Tree View reads. Without one, the usual places and the Music app are tried.");
            });
            card(ui, "rekordbox xml", |ui| {
                sub(ui, "Imported Library");
                kit::block(ui, |ui| {
                    path_row(ui, &app.prefs.rekordbox_xml_path, "no recently selected files");
                    ui.horizontal(|ui| {
                        if ui.add(btn("Browse")).clicked() {
                            if let Some(f) = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file() {
                                app.prefs.rekordbox_xml_path = Some(f.to_string_lossy().to_string());
                            }
                        }
                        let path = app.prefs.rekordbox_xml_path.clone();
                        let exists = path.as_deref().map(|p| std::path::Path::new(p).is_file()).unwrap_or(false);
                        if ui.add_enabled(exists, btn("Import now")).on_disabled_hover_text("Choose a rekordbox xml file first.").clicked() {
                            if let Some(p) = path {
                                app.import_xml(std::path::Path::new(&p));
                            }
                        }
                        if app.prefs.rekordbox_xml_path.is_some() && ui.add(btn("Clear")).clicked() {
                            app.prefs.rekordbox_xml_path = None;
                        }
                    });
                });
                note(ui, "A collection exported from rekordbox (File > Export Collection in xml format).");
            });
            card(ui, "My Tag", |ui| {
                if toggle(ui, &mut app.prefs.mytag_to_comments, "Add \"My Tag\" to the \"Comments\"") && app.prefs.mytag_to_comments {
                    // Tracks that are tagged already get their tags in the comment now.
                    let ids: Vec<i64> = app.track_tags.iter().filter(|(_, s)| !s.is_empty()).map(|(id, _)| *id).collect();
                    crate::prefs_ext::sync_mytag_comments(app, &ids);
                    app.list_dirty = true;
                }
                note(ui, "A track's My Tags are added at the end of its comment as /* Tag / Tag */ whenever its tags change.");
            });
            card(ui, "Color", |ui| {
                sub(ui, "Customize Color Comments");
                note(ui, "Your own names for the eight track colours. They are shown wherever a colour is chosen.");
                kit::block(ui, |ui| {
                    egui::Grid::new("color_comments").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                        for i in 0..8usize {
                            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
                            ui.painter().rect_filled(r.shrink(2.0), 2.0, label_color(i as u8 + 1));
                            ui.add(egui::TextEdit::singleline(&mut app.prefs.color_names[i]).desired_width(300.0).hint_text(crate::model::COLOR_NAMES[i + 1]));
                            ui.end_row();
                        }
                    });
                    ui.add_space(4.0);
                    if ui.add(btn("RESET").min_size(vec2(120.0, 22.0)).font_size(12.0)).clicked() {
                        app.prefs.color_names = crate::prefs_ext::default_color_names();
                    }
                });
            });
            card(ui, "Auto Relocate Search Folders", |ui| {
                note(ui, "Track > Relocate > Auto Relocate looks in these folders (and the folders inside them) for missing files.");
                kit::block(ui, |ui| {
                    let mut remove = None;
                    for (i, f) in app.prefs.relocate_folders.iter().enumerate() {
                        ui.horizontal(|ui| {
                            path_row(ui, &Some(f.clone()), "");
                            if ui.add(btn("Remove").small()).clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        app.prefs.relocate_folders.remove(i);
                    }
                    ui.horizontal(|ui| {
                        if ui.add(btn("Add")).clicked() {
                            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                                let s = d.to_string_lossy().to_string();
                                if !app.prefs.relocate_folders.contains(&s) {
                                    app.prefs.relocate_folders.push(s);
                                }
                            }
                        }
                        let any = !app.prefs.relocate_folders.is_empty() && app.tracks.iter().any(|t| t.missing);
                        if ui.add_enabled(any, btn("Auto Relocate now")).on_disabled_hover_text("Needs a folder here and at least one missing file.").clicked() {
                            app.auto_relocate(None);
                        }
                    });
                });
            });
        }
        _ => {
            card(ui, "rekordbox Library", |ui| {
                toggle(ui, &mut app.prefs.rekordbox_sync, "Load and sync the rekordbox library when recordbox starts");
                note(ui, "recordbox copies rekordbox's library into its own folder and reads the copy. rekordbox's own files are never opened by a database or changed.");
                kit::block(ui, |ui| {
                    if ui.add_enabled(app.rb_job.is_none(), btn("Sync now")).clicked() {
                        crate::menus::handle(app, "sync_rekordbox");
                    }
                });
            });
        }
    }
}

/// Controller > Deck: rekordbox's Eject/Load Lock and the Instant Doubles switch of its Load
/// card (labels and defaults as in 7.2.19).
fn page_deck(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    card(ui, "Eject/Load Lock", |ui| {
        if radio(ui, app.prefs.deck_load_lock, "Lock") {
            app.prefs.deck_load_lock = true;
            changed = true;
        }
        if radio(ui, !app.prefs.deck_load_lock, "Unlock") {
            app.prefs.deck_load_lock = false;
            changed = true;
        }
    });
    card(ui, "Load", |ui| {
        changed |= toggle(ui, &mut app.prefs.instant_doubles, "Instant Doubles");
        note(ui, "Loading the track that is already on the other deck takes over its position, tempo and play state.");
    });
    if changed {
        app.save_prefs();
    }
}

/// Controller > Mixer, as far as the Performance mixer goes: rekordbox's EQ / ISOLATOR choice
/// and its three channel fader curves (labels and defaults as in 7.2.19).
fn page_mixer(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    card(ui, "EQ", |ui| {
        sub(ui, "EQ/ISOLATOR");
        if radio(ui, !app.prefs.eq_isolator, "EQ") {
            app.prefs.eq_isolator = false;
            changed = true;
        }
        if radio(ui, app.prefs.eq_isolator, "ISOLATOR") {
            app.prefs.eq_isolator = true;
            changed = true;
        }
        note(ui, "EQ: HIGH, MID and LOW cut to -26 dB and boost to +6 dB (the DJM-900NXS2 type). ISOLATOR: each band can be removed completely.");
    });
    card(ui, "Channel Fader Curve", |ui| {
        for (i, name) in ["Exponential Curve", "Linear Curve", "Logarithmic Curve"].iter().enumerate() {
            if radio(ui, app.prefs.ch_fader_curve == i as u8, name) {
                app.prefs.ch_fader_curve = i as u8;
                changed = true;
            }
        }
    });
    if changed {
        app.poll_decks();
        app.save_prefs();
    }
}

/// A section title over the cards (rekordbox's "SAMPLER", "SEQUENCER", "Quantize").
fn section(ui: &mut Ui, title: &str) {
    // (25 pt in, like a card's title; its middle 17 pt above the card: reference preferences/10.)
    ui.add_space(20.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(kit::CARD_W), 22.0), egui::Sense::hover());
    ui.painter().text(pos2(r.min.x + 25.0, r.center().y), egui::Align2::LEFT_CENTER, title, inter_bold(16.0), WHITE);
    ui.add_space(4.0);
}

/// Controller > Effect: the CFX switch for the sampler deck. (rekordbox shows this row with
/// hardware that has a MIC COLOR knob; recordbox has the knob on screen, S in the CFX row.)
fn page_effect(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    card(ui, "CFX", |ui| {
        changed |= toggle(ui, &mut app.prefs.sampler.cfx_on_sampler, "Apply CFX to Sampler using MIC COLOR control knob.");
        note(ui, "The S knob of the CFX row in the FX panel then acts on the sampler deck.");
    });
    if changed {
        app.save_prefs();
    }
}

/// Controller > Sampler, after rekordbox 7.2.19 (reference preferences/10): SLOT, Capture,
/// Auto Panel Mode and Sequence Load, with rekordbox's labels and defaults.
fn page_sampler(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    section(ui, "SAMPLER");
    card(ui, "SLOT", |ui| {
        if radio(ui, app.prefs.sampler.slots16, "16") {
            app.prefs.sampler.slots16 = true;
            changed = true;
        }
        if radio(ui, !app.prefs.sampler.slots16, "8") {
            app.prefs.sampler.slots16 = false;
            changed = true;
        }
        ui.add_space(6.0);
        changed |= toggle(ui, &mut app.prefs.sampler.load_lock, "Do not overwrite the loaded Slot.");
    });
    card(ui, "Capture", |ui| {
        sub(ui, "Location of Files");
        let dir = crate::sampler_ext::capture_dir(&app.prefs.sampler.capture_dir).to_string_lossy().to_string();
        // (A long path shows its end: the box is 410 pt wide, as in rekordbox.)
        let n = dir.chars().count();
        let shown = if n > 48 { format!("…{}", dir.chars().skip(n - 47).collect::<String>()) } else { dir.clone() };
        kit::combo_row(ui, 22.0, |ui| {
            kit::combo("smp_capdir", 410.0, shown, 14.0).show_ui(ui, |ui| {
                if ui.selectable_label(app.prefs.sampler.capture_dir.is_empty(), crate::sampler_ext::capture_dir("").to_string_lossy()).clicked() {
                    app.prefs.sampler.capture_dir.clear();
                    changed = true;
                }
            });
            ui.add_space(10.0);
            if ui.add(btn("Browse").min_size(vec2(100.0, 22.0))).clicked() {
                if let Some(d) = rfd::FileDialog::new().set_title("Location of Files").pick_folder() {
                    app.prefs.sampler.capture_dir = d.to_string_lossy().to_string();
                    changed = true;
                }
            }
        });
        sub(ui, "Location of a Bank for Slicer Capture");
        kit::combo_row(ui, 22.0, |ui| {
            let b = app.prefs.sampler.slicer_bank.clamp(1, 4);
            kit::combo("smp_slicerbank", 520.0, format!("Bank{b}"), 14.0).show_ui(ui, |ui| {
                for k in 1..=4u8 {
                    if ui.selectable_label(b == k, format!("Bank{k}")).clicked() {
                        app.prefs.sampler.slicer_bank = k;
                        changed = true;
                    }
                }
            });
        });
        kit::indent(ui, 52.0);
        ui.add_space(7.0);
        changed |= toggle(ui, &mut app.prefs.sampler.capture_empty, "Capture the loop in an empty slot");
    });
    card(ui, "Auto Panel Mode", |ui| {
        changed |= toggle(ui, &mut app.prefs.sampler.auto_panel_disable, "Disable");
    });
    section(ui, "SEQUENCER");
    card(ui, "Sequence Load", |ui| {
        changed |= toggle(ui, &mut app.prefs.sampler.seq.load_autoplay, "Load the sequence and automatically play.");
    });
    // (rekordbox shows this card only with a DDJ-RZX connected; the setting is kept for
    // velocity pads, which recordbox has no hardware path for yet.)
    card(ui, "Velocity Curve", |ui| {
        kit::combo_row(ui, 22.0, |ui| {
            let c = app.prefs.sampler.velocity_curve.clamp(1, 4);
            kit::combo("smp_velocity", 500.0, format!("Curve {c}"), 14.0).show_ui(ui, |ui| {
                for k in 1..=4u8 {
                    if ui.selectable_label(c == k, format!("Curve {k}")).clicked() {
                        app.prefs.sampler.velocity_curve = k;
                        changed = true;
                    }
                }
            });
        });
        note(ui, "For velocity pads of a controller. Not used yet: recordbox has no controller pads.");
    });
    if changed {
        app.save_prefs();
    }
}

/// Controller > Others, as far as it is built: the SEQUENCER row of Quantize > Type >
/// QUANTIZE (rekordbox's SequencerQuantizeStatus and SequencerQuantize; reference
/// preferences/13). It takes effect while the sampler's Q is on.
fn page_controller_others(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    section(ui, "Quantize");
    card(ui, "Type", |ui| {
        sub(ui, "QUANTIZE");
        kit::combo_row(ui, 22.0, |ui| {
            // (Label 89 pt in, switch 264, list 385: reference preferences/13.)
            ui.add_space(5.0);
            let x0 = ui.cursor().min.x;
            ui.label(kit::text("SEQUENCER", 14.0).color(WHITE));
            ui.add_space((175.0 - (ui.cursor().min.x - x0)).max(4.0));
            let x1 = ui.cursor().min.x;
            changed |= kit::toggle_raw(ui, &mut app.prefs.sampler.seq.quantize_on, "Enable");
            ui.add_space((121.0 - (ui.cursor().min.x - x1)).max(4.0));
            const NAMES: [&str; 6] = ["1/32", "1/16", "1/8", "1/4", "1/2", "1/1"];
            let q = app.prefs.sampler.seq.quantize.clamp(1, 6);
            kit::combo("seq_quantize", 200.0, NAMES[q as usize - 1], 14.0).show_ui(ui, |ui| {
                for k in 1..=6u8 {
                    if ui.selectable_label(q == k, NAMES[k as usize - 1]).clicked() {
                        app.prefs.sampler.seq.quantize = k;
                        changed = true;
                    }
                }
            });
        });
        note(ui, "Notes recorded into a sequence go to the nearest line of this grid (1/4 = one beat) while the sampler's Q is on.");
    });
    if changed {
        app.save_prefs();
    }
}

/// The STEMS page, laid out and worded as rekordbox's Extensions > STEMS (7.2.19); the model
/// download and the stem cache, which rekordbox does not have, are in a card of their own.
fn page_stems(app: &mut App, ui: &mut Ui) {
    // The switch above the cards, on the window's black: 43 pt band, the switch 20 pt in.
    let (band, _) = ui.allocate_exact_size(vec2(ui.available_width().min(kit::CARD_W), 43.0), egui::Sense::hover());
    let mut head = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(band.min.x + 20.0, band.min.y + 13.0), band.max)).layout(egui::Layout::left_to_right(egui::Align::Center)));
    kit::toggle_raw(&mut head, &mut app.prefs.show_stems, "Enable the STEMS Function");
    card(ui, "STEMS mode", |ui| {
        if radio(ui, app.prefs.stem_parts == 3, "3 Stems(VOCAL, INST, DRUMS)") {
            app.prefs.stem_parts = 3;
        }
        if radio(ui, app.prefs.stem_parts != 3, "4 Stems(VOCAL, INST, BASS, DRUMS)") {
            app.prefs.stem_parts = 4;
        }
    });
    let [quality, speed] = &crate::stems::CATALOG;
    let cur = app.stem.model.clone().or_else(|| app.prefs.stem_model.clone()).unwrap_or_default();
    let tip = format!("Prioritize sound quality: {} — {}\nPrioritize speed: {} — {}", quality.name, quality.about, speed.name, speed.about);
    kit::card_help(ui, "STEMS Analysis Process", Some(&tip), |ui| {
        ui.add_space(-2.0);
        let able = app.stem.hw.as_ref().is_none_or(|hw| hw.ram_gb >= quality.min_ram_gb && crate::stems::backend_status().0);
        let mut pick = None;
        if able || cur == quality.id {
            if radio(ui, cur == quality.id, "Prioritize sound quality") {
                pick = Some(quality);
            }
        } else {
            let ram = app.stem.hw.as_ref().map_or(0.0, |hw| hw.ram_gb);
            kit::radio_off(ui, false, "Prioritize sound quality", &format!("{} requires GPU separation and {:.0} GB of system RAM. Detected {:.0} GB of system RAM. STEMS backend: {}", quality.name, quality.min_ram_gb, ram, crate::stems::backend_status().1));
        }
        if radio(ui, cur != quality.id, "Prioritize speed") {
            pick = Some(speed);
        }
        ui.add_space(2.0);
        if let Some(m) = pick.filter(|m| m.id != cur) {
            app.prefs.stem_model = Some(m.id.to_string());
            app.stem.model = Some(m.id.to_string());
            app.stem.model_ready = crate::stems::model_ready(m);
        }
    });
    card(ui, "Layout", |ui| {
        sub(ui, "ACTIVE STEM/STEM ISO/STEM FX Layout");
        kit::indent(ui, 88.0);
        let why = "recordbox has one layout for the part buttons, in the player's STEMS strip.";
        radio(ui, true, "Auto");
        kit::radio_off(ui, false, "Customize", why);
        kit::combo_row(ui, 22.0, |ui| {
            // Greyed as in the original while "Customize" is not chosen: it opens on nothing.
            egui::ComboBox::from_id_salt("stem_layout").width(520.0).selected_text(RichText::new("CUSTOM3").font(inter_bold(14.0)).color(hex(0x878787))).icon(kit::chevron_dim).show_ui(ui, |ui| {
                ui.label(kit::text(why, 12.0).color(DIM));
            });
        });
        kit::radio_off(ui, false, "Do not display", "Switch \"Enable the STEMS Function\" off to hide the part buttons.");
    });
    kit::card_help(ui, "STEM Waveform Display", Some("When enabled, the waveform shows the parts that sound. When disabled, the original waveform is always shown."), |ui| {
        toggle_if(app, ui, STEM_WAVEFORM_WIRED, |p| &mut p.stem_waveform, "Enable", "recordbox always shows the original waveform, so far.");
    });
    card(ui, "ACTIVE STEM Setting", |ui| {
        if radio(ui, !app.prefs.stem_solo, "MUTE") {
            crate::prefs_ext::set_stem_solo(app, false);
        }
        if STEM_SOLO_WIRED {
            if radio(ui, app.prefs.stem_solo, "SOLO") {
                crate::prefs_ext::set_stem_solo(app, true);
            }
        } else {
            kit::radio_off(ui, false, "SOLO", "The part buttons mute their part, so far.");
        }
        toggle(ui, &mut app.prefs.stem_reset_on_load, "Reset the MUTE/SOLO state while loading");
    });
    card(ui, "Stem separation model (recordbox)", |ui| {
        if let Some(hw) = &app.stem.hw {
            kit::line(
                ui,
                &format!(
                    "{}{} CPU threads{} · STEMS backend: {}",
                    if hw.cpu.is_empty() { String::new() } else { format!("{}, ", hw.cpu) },
                    hw.cores,
                    if hw.ram_gb > 0.0 { format!(", {:.0} GB system RAM", hw.ram_gb) } else { String::new() },
                    crate::stems::backend_status().1
                ),
            );
            note(ui, &format!("Recommended model: {}", crate::stems::choose(hw).name));
        } else {
            note(ui, "Checking this computer…");
        }
        for m in crate::stems::CATALOG.iter() {
            let ready = crate::stems::model_ready(m);
            note(ui, &format!("{}{}{}: {}", m.name, if cur == m.id { " (in use)" } else { "" }, if ready { ", downloaded" } else { "" }, m.about));
        }
        toggle(ui, &mut app.prefs.stems_auto_download, "Download the model automatically at launch");
        if !app.stem.model_ready && app.stem.downloading.is_none() && kit::block(ui, |ui| ui.add(btn("Download now")).clicked()) {
            app.stems_download();
        }
        if let Some((d, t)) = app.stem.downloading {
            kit::line(ui, &if t > 0 { format!("Downloading… {:.0}%", d as f64 / t as f64 * 100.0) } else { "Downloading…".into() });
        }
        let automatic_available = crate::stems::radeon_hold().is_none();
        ui.add_enabled_ui(automatic_available, |ui| {
            toggle(ui, &mut app.prefs.stems_auto_separate, "Separate tracks when they are loaded");
            ui.add_enabled_ui(app.prefs.stems_auto_separate, |ui| {
                toggle(ui, &mut app.prefs.stems_auto_cpu, "Allow automatic separation on CPU");
            });
        });
        if automatic_available {
            note(ui, "CPU separation can use several cores for minutes. Cached parts load immediately.");
        } else {
            note(ui, "Automatic separation is paused. Cached parts still load. You can separate one loaded track on the CPU.");
        }
        if let Some(status) = app.stem.separation_status() {
            kit::line(ui, &status);
            if app.stem.separating.map(|(id, _)| id) == app.deck.track
                && kit::block(ui, |ui| ui.add(btn("Stop separation")).clicked()) {
                app.stop_separation();
            }
        } else if !app.prefs.show_stems {
            note(ui, "Enable the STEMS Function above to separate a track from the active deck.");
        } else if app.stem.downloading.is_none() {
            if let Some(id) = app.deck.track {
                if let Some(error) = app.stem.error_for(Some(id)) {
                    note(ui, &error.detail);
                }
                if app.stem.loading == Some(id) {
                    note(ui, "Loading saved parts for the active deck…");
                } else if app.player.state.lock().stems.is_some() {
                    note(ui, "Separated parts are ready on the active deck.");
                } else if app.stem.model.is_some() {
                    let action = if app.manual_stem_device() == crate::stems::SeparationDevice::Cpu {
                        "Separate on CPU"
                    } else {
                        "Separate"
                    };
                    let label = format!("{action} · Deck {}", app.px.active + 1);
                    if kit::block(ui, |ui| ui.add(btn(label)).clicked()) {
                        app.separate_loaded();
                    }
                }
            } else {
                note(ui, "Load a track on the active deck to separate it.");
            }
        }
        note(ui, &format!("Separated stems are kept so each track is only split once: {:.1} of at most {} GB in use (the ones used longest ago make room).", crate::stems::cache_size() as f64 / 1e9, app.prefs.stem_cache_gb));
        kit::block(ui, |ui| {
            if ui.add(btn("Clear separated stems")).clicked() {
                crate::stems::clear_cache();
            }
        });
    });
}

// ------------------------------------------------------------------ other windows

/// Keyboard Shortcuts, Backup Library and Restore Library windows.
pub fn extra_windows(app: &mut App, ctx: &Context) {
    shortcuts_window(app, ctx);
    backup_windows(app, ctx);
}

fn shortcuts_window(app: &mut App, ctx: &Context) {
    if !app.dialogs.ext.shortcuts_open {
        return;
    }
    let mut open = true;
    kit::dialog(ctx, "Keyboard Shortcuts").open(&mut open).default_size(vec2(520.0, 560.0)).show(ctx, |ui| {
        kit::apply_style(ui);
        egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
            for cat in ACTION_CATEGORIES {
                ui.add_space(4.0);
                ui.label(RichText::new(cat).font(bold(14.0)).color(WHITE));
                egui::Grid::new(("all_keys", cat)).num_columns(2).striped(true).spacing(vec2(24.0, 3.0)).min_col_width(250.0).show(ui, |ui| {
                    for (id, label, _, c) in ACTIONS {
                        if c != cat {
                            continue;
                        }
                        ui.label(RichText::new(label).size(12.5));
                        match app.prefs.shortcut_for(id) {
                            Some(k) => ui.label(RichText::new(key_text(&k)).size(12.5).color(WHITE)),
                            None => ui.label(RichText::new("—").size(12.5).color(OFF)),
                        };
                        ui.end_row();
                    }
                });
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Menus").font(bold(14.0)).color(WHITE));
            egui::Grid::new("menu_keys").num_columns(2).striped(true).spacing(vec2(24.0, 3.0)).min_col_width(250.0).show(ui, |ui| {
                for (n, k) in [("Create New Playlist", "⌘N"), ("Export to USB Device", "⌘E"), ("Property", "⌘I"), ("Search in the collection", "⌘F"), ("Select All", "⌘A"), ("Undo / Redo", "⌘Z / ⇧⌘Z")] {
                    ui.label(RichText::new(n).size(12.5));
                    ui.label(RichText::new(k).size(12.5).color(WHITE));
                    ui.end_row();
                }
            });
        });
        ui.separator();
        if ui.add(btn("Change shortcuts…")).clicked() {
            app.dialogs.prefs_open = true;
            app.dialogs.prefs_tab = 3;
        }
    });
    app.dialogs.ext.shortcuts_open = open;
}

fn backup_windows(app: &mut App, ctx: &Context) {
    let center = egui::Align2::CENTER_CENTER;
    if app.dialogs.ext.backup_ask {
        let mut choice: Option<Option<bool>> = None;
        kit::dialog(ctx, "Library Backup").resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(400.0);
            kit::message(ui, "Do you want to back up music files as well?");
            let n = app.tracks.len();
            let bytes: u64 = app.tracks.iter().filter(|t| !t.missing).map(|t| t.file_size as u64).sum();
            kit::message_note(ui, &format!("Database file, analysis and image files are always included. Music files: {n} track(s), about {:.1} GB.", bytes as f64 / 1e9));
            match kit::dialog_buttons(ui, &["Yes", "No", "Cancel"]) {
                Some(0) => choice = Some(Some(true)),
                Some(1) => choice = Some(Some(false)),
                Some(_) => choice = Some(None),
                None => {}
            }
        });
        if let Some(c) = choice {
            app.dialogs.ext.backup_ask = false;
            if let Some(music) = c {
                let name = format!("recordbox-backup-{}.zip", chrono::Local::now().format("%Y%m%d"));
                if let Some(f) = rfd::FileDialog::new().set_title("Backup file").set_file_name(name).add_filter("recordbox library backup", &["zip"]).save_file() {
                    app.save_prefs();
                    let _ = app.db.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
                    app.dialogs.ext.backup_job = Some(crate::backup::start_backup(ctx.clone(), f, music));
                }
            }
        }
    }
    if let Some(path) = app.dialogs.ext.restore_ask.clone() {
        let mut choice = None;
        kit::dialog(ctx, "Library Restoration").resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(440.0);
            kit::message(ui, "Are you sure you want to overwrite the present library with a backup?");
            kit::message_note(ui, &format!("Backup file: {}", path.display()));
            if let Ok(m) = crate::backup::read_manifest(&path) {
                kit::message_note(ui, &format!("Made {} · {} track(s) · {} artwork file(s) · {} music file(s)", m.created.get(..16).unwrap_or(&m.created).replace('T', " "), m.tracks, m.artwork_files, m.music.len()));
            }
            kit::message_note(ui, "The present library is copied into the backups folder first. Music files are only put back where the file is missing.");
            match kit::dialog_buttons(ui, &["Yes", "No"]) {
                Some(0) => choice = Some(true),
                Some(_) => choice = Some(false),
                None => {}
            }
        });
        if let Some(yes) = choice {
            app.dialogs.ext.restore_ask = None;
            if yes {
                app.save_prefs();
                app.dialogs.ext.backup_job = Some(crate::backup::start_restore(ctx.clone(), path));
            }
        }
    }
    if let Some(job) = &app.dialogs.ext.backup_job {
        let (done, total, what) = job.progress.lock().clone();
        let restore = job.restore;
        let mut cancel = false;
        kit::dialog(ctx, if restore { "Library Restoration" } else { "Library Backup" }).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(380.0);
            ui.label(RichText::new(if total == 0 { if restore { "Preparing restore..." } else { "Preparing backup..." } } else if restore { "Restore Collection" } else { "Back up Collection" }).size(14.0).color(WHITE));
            let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).show_percentage());
            ui.label(RichText::new(what).size(11.0).color(DIM));
            let cancelling = job.cancel.load(std::sync::atomic::Ordering::Relaxed);
            if ui.add_enabled(!cancelling, btn(if cancelling { "Cancelling…" } else { "Cancel" })).clicked() {
                cancel = true;
            }
        });
        if cancel {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    if let Some((title, text)) = app.dialogs.ext.backup_done.clone() {
        let mut close = false;
        kit::dialog(ctx, title).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_max_width(460.0);
            for (i, line) in text.lines().enumerate() {
                ui.label(RichText::new(line).size(if i == 0 { 14.0 } else { 12.0 }).color(if i == 0 { WHITE } else { DIM2 }));
            }
            if kit::dialog_buttons(ui, &["OK"]).is_some() {
                close = true;
            }
        });
        if close {
            app.dialogs.ext.backup_done = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_settings_and_shortcuts() {
        assert!(search_settings("").is_empty());
        let hits = search_settings("buffer");
        assert_eq!(hits, vec![(5, 0, "Buffer size".to_string())]);
        let hits = search_settings("memory cue 10");
        assert!(hits.iter().any(|h| h.0 == 3 && h.2 == "Memory Cue 10"));
        assert!(hits.iter().any(|h| h.2 == "Delete Memory Cue 10"));
        assert!(search_settings("WAVEFORM").len() >= 5, "search ignores case");
        assert!(search_settings("zzzz").is_empty());
        // Every indexed setting points at a tab that exists.
        for (page, tab, name) in SETTINGS {
            assert!(PAGES.iter().any(|p| p.0 == page), "{name}");
            assert!(tab < tabs_of(page).len(), "{name}");
        }
    }

    #[test]
    fn reset_keeps_shortcuts_columns_and_local_folders() {
        let mut p = Prefs::default();
        p.keymap.insert("play".into(), "P".into());
        p.columns = vec![Col::Title, Col::Bpm];
        p.widths.insert("title".into(), 300.0);
        p.devices_extra = vec!["/tmp/dev".into()];
        p.relocate_folders = vec!["/Volumes/Music".into()];
        p.audio_device = Some("Speakers".into());
        p.tooltips = false;
        p.wave_half = true;
        p.audio_buffer = 256;
        p.color_names[0] = "Warm up".into();
        p.usb_settings.set(SettingFile::My, 10, 0x80);
        let r = reset_prefs(&p);
        assert_eq!(r.keymap.get("play").map(|s| s.as_str()), Some("P"));
        assert_eq!(r.columns, vec![Col::Title, Col::Bpm]);
        assert_eq!(r.widths.get("title"), Some(&300.0));
        assert_eq!(r.devices_extra, p.devices_extra);
        assert_eq!(r.relocate_folders, p.relocate_folders);
        assert_eq!(r.audio_device, p.audio_device);
        assert!(r.tooltips && !r.wave_half);
        assert_eq!(r.audio_buffer, 0);
        assert_eq!(r.color_names, crate::prefs_ext::default_color_names());
        assert_eq!(r.usb_settings, Default::default());
    }
}
