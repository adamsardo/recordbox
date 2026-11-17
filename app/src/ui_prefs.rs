//! The Preferences window, laid out as rekordbox's: categories on the left (View, Audio,
//! Analysis, DJ System, Keyboard, Advanced), tabs across the top, cards of settings. Also the
//! Keyboard Shortcuts window and the Backup / Restore Library windows.

use crate::analysis::BPM_RANGES;
use crate::app::*;
use crate::export::settings::{SettingFile, DEV_OVERVIEW_TYPE, DEV_WAVEFORM_COLOR, DEV_WAVEFORM_POSITION};
use crate::prefs_ext::{ACTIONS, ACTION_CATEGORIES};
use crate::theme::*;
use eframe::egui;
use egui::{vec2, Context, RichText, Ui};

// Settings another part of the app has to act on. While one is false its control is shown
// switched off with the reason, so nothing in the window pretends to work.
/// Analysis reads `prefs.high_precision_grid`.
const HIGH_PRECISION_WIRED: bool = false;
/// The device writer deletes an imported history (reads `prefs.delete_history_after_import`).
const HISTORY_DELETE_WIRED: bool = false;
/// The device writer stores `prefs.device_bg_color`.
const BG_COLOR_WRITTEN: bool = false;

const PAGES: [(u8, &str); 6] = [(0, "View"), (5, "Audio"), (1, "Analysis"), (2, "DJ System"), (3, "Keyboard"), (4, "Advanced")];

fn tabs_of(page: u8) -> &'static [&'static str] {
    match page {
        0 => &["Display Type", "Layout", "Color", "Recordbox"],
        1 => &["Track Analysis", "CUE Analysis"],
        2 => &["General", "Category", "Sort", "Column", "My Settings", "Device", "Others"],
        3 => &ACTION_CATEGORIES,
        5 => &["Audio"],
        _ => &["Database", "Browse", "Others"],
    }
}

/// Every setting by name, for the search field: (page, tab, name).
const SETTINGS: [(u8, usize, &str); 71] = [
    (0, 0, "Language"),
    (0, 0, "Show Tooltips"),
    (0, 0, "Font Size"),
    (0, 0, "Bold"),
    (0, 0, "Line Space"),
    (0, 0, "Show the selected track to the center"),
    (0, 0, "Key display format"),
    (0, 1, "Tree View"),
    (0, 1, "Display the playlist palette"),
    (0, 1, "Add \"My Tag\" to the \"Comments\""),
    (0, 1, "Display Cue Markers on Preview"),
    (0, 1, "My Tag Configuration Window"),
    (0, 1, "Column browser"),
    (0, 1, "Sub-Browser Window"),
    (0, 2, "Waveform color"),
    (0, 2, "Full Waveform"),
    (0, 2, "Half Waveform"),
    (0, 2, "Color comments"),
    (0, 2, "Customize Color Comments"),
    (0, 3, "Time Mode"),
    (0, 3, "Quantize"),
    (0, 3, "Stems"),
    (0, 3, "Vocal"),
    (5, 0, "Audio"),
    (5, 0, "Sample Rate"),
    (5, 0, "Buffer size"),
    (5, 0, "Output channels"),
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
    (4, 2, "Stem separation"),
    (4, 2, "rekordbox Library"),
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

/// Card with a 17 pt bold heading on #1E1E1E, as in the original's Preferences.
fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::default().fill(BG2).inner_margin(egui::Margin { left: 25, right: 18, top: 18, bottom: 18 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).font(bold(17.0)).color(WHITE));
        ui.add_space(8.0);
        ui.indent(title, |ui| add(ui));
    });
    ui.add_space(4.0);
}

fn sub(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).font(bold(15.0)).color(WHITE));
    ui.add_space(4.0);
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(DIM));
}

/// Blue pill switch with a white knob (off: grey).
fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(32.0, 17.0), egui::Sense::click());
        if resp.clicked() {
            *on = !*on;
            changed = true;
        }
        let en = ui.is_enabled();
        let p = ui.painter();
        p.rect_filled(r, 8.5, if !en { hex(0x3A3A3A) } else if *on { BLUE } else { DIM });
        let kx = if *on { r.max.x - 8.5 } else { r.min.x + 8.5 };
        p.circle_filled(egui::pos2(kx, r.center().y), 6.5, if en { WHITE } else { OFF });
        ui.add_space(4.0);
        if ui.add(egui::Label::new(RichText::new(label).size(14.0).color(if en { WHITE } else { OFF })).sense(egui::Sense::click())).clicked() {
            *on = !*on;
            changed = true;
        }
    });
    changed
}

/// A switch that only works once `wired`; until then it is shown off, with the reason.
fn toggle_if(ui: &mut Ui, wired: bool, on: &mut bool, label: &str, why: &str) {
    if wired {
        toggle(ui, on, label);
    } else {
        let mut off = false;
        ui.add_enabled_ui(false, |ui| toggle(ui, &mut off, label)).response.on_disabled_hover_text(why);
        note(ui, why);
    }
}

fn radio(ui: &mut Ui, on: bool, label: &str) -> bool {
    ui.radio(on, RichText::new(label).size(14.0)).clicked()
}

fn path_row(ui: &mut Ui, value: &Option<String>, empty: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(520.0), 24.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, BG);
    let text = value.clone().unwrap_or_else(|| empty.to_string());
    let col = if value.is_some() { WHITE } else { DIM };
    ui.painter().with_clip_rect(r).text(egui::pos2(r.min.x + 6.0, r.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(12.5), col);
}

// ------------------------------------------------------------------ window

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
    let builder = egui::ViewportBuilder::default().with_title("Preferences").with_inner_size([800.0, 828.0]).with_min_inner_size([760.0, 520.0]);
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
    let mut open = true;
    let frame = egui::Frame::window(&ctx.style()).fill(BG).inner_margin(0.0);
    let h = (ctx.screen_rect().height() - 90.0).clamp(480.0, 800.0);
    egui::Window::new("Preferences").open(&mut open).default_size(vec2(800.0, h)).collapsible(false).resizable(false).frame(frame).show(ctx, |ui| prefs_body(app, ui, h));
    app.dialogs.prefs_open = open;
}

fn prefs_body(app: &mut App, ui: &mut Ui, height: f32) {
    ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
    ui.horizontal_top(|ui| {
        // Sidebar 160 pt, #1E1E1E: search field, then categories with icons.
        egui::Frame::default().fill(BG2).inner_margin(8.0).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 3.0);
                ui.set_width(144.0);
                ui.set_min_height(height - 16.0);
                let (sr, _) = ui.allocate_exact_size(vec2(144.0, 24.0), egui::Sense::hover());
                ui.painter().rect_filled(sr, 0.0, BG);
                crate::icons::magnifier(ui.painter(), egui::Rect::from_center_size(egui::pos2(sr.min.x + 12.0, sr.center().y), vec2(16.0, 16.0)), DIM2, 0);
                let field = egui::Rect::from_min_max(egui::pos2(sr.min.x + 24.0, sr.min.y + 2.0), sr.max);
                ui.put(field, egui::TextEdit::singleline(&mut app.dialogs.ext.prefs_search).frame(false).font(egui::FontId::proportional(13.0)).text_color(WHITE).hint_text("Search"));
                ui.add_space(14.0);
                type Ic = fn(&egui::Painter, egui::Rect, egui::Color32);
                let icons: [Ic; 6] = [crate::icons::info, crate::icons::speech, crate::icons::view_columns, crate::icons::usb, crate::icons::list, crate::icons::hamburger];
                for ((i, n), ic) in PAGES.iter().zip(icons.iter()) {
                    let sel = app.dialogs.prefs_tab == *i && app.dialogs.ext.prefs_search.trim().is_empty();
                    let (r, resp) = ui.allocate_exact_size(vec2(144.0, 30.0), egui::Sense::click());
                    if sel {
                        ui.painter().rect_filled(r, 0.0, BLUE);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, hex(0x2A2A2A));
                    }
                    ic(ui.painter(), egui::Rect::from_center_size(egui::pos2(r.min.x + 15.0, r.center().y), vec2(18.0, 18.0)), if sel { WHITE } else { DIM });
                    ui.painter().text(egui::pos2(r.min.x + 40.0, r.center().y), egui::Align2::LEFT_CENTER, *n, egui::FontId::proportional(14.0), if sel { WHITE } else { DIM2 });
                    if resp.clicked() {
                        app.dialogs.prefs_tab = *i;
                        app.dialogs.ext.prefs_search.clear();
                    }
                }
            })
        });
        ui.vertical(|ui| {
            ui.set_width(620.0);
            let page = app.dialogs.prefs_tab;
            let tabs = tabs_of(page);
            let key = format!("prefs_subtab_{page}");
            let mut sub_tab: usize = ui.ctx().data_mut(|d| *d.get_temp_mut_or(egui::Id::new(&key), 0usize)).min(tabs.len() - 1);
            let searching = !app.dialogs.ext.prefs_search.trim().is_empty();
            ui.add_space(4.0);
            // Tab strip (black): active tab blue, inactive #1E1E1E with #AAAAAA text.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(4.0);
                if searching {
                    let b = egui::Button::new(RichText::new("Search results").size(14.0).color(WHITE)).fill(BLUE).corner_radius(0.0).min_size(vec2(0.0, 30.0));
                    let _ = ui.add(b);
                } else {
                    for (i, t) in tabs.iter().enumerate() {
                        let sel = sub_tab == i;
                        let b = egui::Button::new(RichText::new(*t).size(14.0).color(if sel { WHITE } else { DIM2 })).fill(if sel { BLUE } else { BG2 }).corner_radius(0.0).min_size(vec2(0.0, 30.0));
                        if ui.add(b).clicked() {
                            sub_tab = i;
                        }
                    }
                }
            });
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(&key), sub_tab));
            ui.add_space(4.0);
            let before = format!("{:?}", app.prefs);
            egui::ScrollArea::vertical().id_salt(("prefs_scroll", page, sub_tab, searching)).max_height(height - 92.0).min_scrolled_height(height - 92.0).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if searching {
                    search_results(app, ui);
                } else {
                    match page {
                        0 => page_view(app, ui, sub_tab),
                        1 => page_analysis(app, ui, sub_tab),
                        2 => page_dj_system(app, ui, sub_tab),
                        3 => page_keyboard(app, ui, sub_tab),
                        5 => page_audio(app, ui),
                        _ => page_advanced(app, ui, sub_tab),
                    }
                }
            });
            if format!("{:?}", app.prefs) != before {
                app.save_prefs();
            }
            ui.add_space(6.0);
            reset_row(app, ui);
        });
    });
}

fn search_results(app: &mut App, ui: &mut Ui) {
    let hits = search_settings(&app.dialogs.ext.prefs_search);
    card(ui, "Search results", |ui| {
        if hits.is_empty() {
            ui.label(RichText::new("No setting has that in its name.").size(13.0).color(DIM2));
        }
        for (page, tab, name) in hits {
            let place = format!("{} > {}", PAGES.iter().find(|p| p.0 == page).map(|p| p.1).unwrap_or(""), tabs_of(page).get(tab).copied().unwrap_or(""));
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new(&name).size(14.0).color(WHITE)).fill(BG).min_size(vec2(300.0, 24.0))).clicked() {
                    app.dialogs.prefs_tab = page;
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(format!("prefs_subtab_{page}")), tab));
                    app.dialogs.ext.prefs_search.clear();
                }
                ui.label(RichText::new(place).size(12.0).color(DIM));
            });
        }
    });
}

/// "Reset to defaults": asks first; keyboard shortcuts, columns and this computer's devices
/// and folders are kept.
fn reset_row(app: &mut App, ui: &mut Ui) {
    match app.dialogs.ext.reset_ask {
        None => {
            if ui.add(egui::Button::new(RichText::new("Reset to defaults").size(14.0)).fill(hex(0x1E1E20)).min_size(vec2(250.0, 30.0))).clicked() {
                app.dialogs.ext.reset_ask = Some(None);
            }
        }
        Some(scope) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let text = if scope == Some(3) { "Reset every keyboard shortcut to its default?" } else { "Reset all settings to their defaults? Keyboard shortcuts and columns are kept." };
                ui.label(RichText::new(text).size(13.0).color(WHITE));
                if ui.add(egui::Button::new(RichText::new("Reset").size(14.0)).fill(BLUE).min_size(vec2(80.0, 30.0))).clicked() {
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
                if ui.add(egui::Button::new(RichText::new("Cancel").size(14.0)).fill(hex(0x1E1E20)).min_size(vec2(80.0, 30.0))).clicked() {
                    app.dialogs.ext.reset_ask = None;
                }
            });
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
                egui::ComboBox::from_id_salt("lang").width(330.0).selected_text(RichText::new("English").font(bold(14.0))).show_ui(ui, |ui| {
                    let _ = ui.selectable_label(true, "English");
                });
            });
            card(ui, "Tooltips", |ui| {
                toggle(ui, &mut app.prefs.tooltips, "Show Tooltips");
            });
            card(ui, "Browse", |ui| {
                sub(ui, "Font Size");
                let mut f = app.prefs.browse_font as u32;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Small").size(12.0).color(DIM2));
                    if ui.add(egui::Slider::new(&mut f, 0..=4).show_value(false)).changed() {
                        app.prefs.browse_font = f as u8;
                    }
                    ui.label(RichText::new("Large").size(12.0).color(DIM2));
                });
                toggle(ui, &mut app.prefs.browse_bold, "Bold");
                ui.separator();
                sub(ui, "Line Space");
                let mut l = app.prefs.browse_line as u32;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Small").size(12.0).color(DIM2));
                    if ui.add(egui::Slider::new(&mut l, 0..=4).show_value(false)).changed() {
                        app.prefs.browse_line = l as u8;
                    }
                    ui.label(RichText::new("Large").size(12.0).color(DIM2));
                });
                ui.separator();
                toggle(ui, &mut app.prefs.center_selected, "Show the selected track to the center");
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
        }
        1 => {
            card(ui, "Tree View", |ui| {
                for name in crate::prefs_ext::TREE_ITEMS {
                    let avail = crate::prefs_ext::tree_item_available(name);
                    let fixed = name == "Collection";
                    let mut on = avail && app.prefs.tree_item(name);
                    let r = ui.add_enabled(avail && !fixed, egui::Checkbox::new(&mut on, RichText::new(name).size(14.0)));
                    if r.changed() {
                        app.prefs.tree_items.insert(name.to_string(), on);
                    }
                    r.on_disabled_hover_text(if fixed { "Collection is always shown.".to_string() } else { format!("Recordbox has no {name} source yet.") });
                }
            });
            card(ui, "Playlist", |ui| {
                toggle(ui, &mut app.prefs.palette_open, "Display the playlist palette");
            });
            card(ui, "My Tag", |ui| {
                if toggle(ui, &mut app.prefs.mytag_to_comments, "Add \"My Tag\" to the \"Comments\"") && app.prefs.mytag_to_comments {
                    // Tracks that are tagged already get their tags in the comment now.
                    let ids: Vec<i64> = app.track_tags.iter().filter(|(_, s)| !s.is_empty()).map(|(id, _)| *id).collect();
                    crate::prefs_ext::sync_mytag_comments(app, &ids);
                    app.list_dirty = true;
                }
                note(ui, "A track's My Tags are added at the end of its comment as /* Tag / Tag */ whenever its tags change.");
                toggle(ui, &mut app.prefs.show_mytag, "My Tag Configuration Window");
            });
            card(ui, "Preview", |ui| {
                toggle(ui, &mut app.prefs.cue_markers_on_preview, "Display Cue Markers on Preview");
            });
            card(ui, "Browser", |ui| {
                if toggle(ui, &mut app.prefs.column_browser, "Column browser (Genre / Artist / Album)") {
                    app.list_dirty = true;
                }
                toggle(ui, &mut app.prefs.sub_browser, "Sub-Browser Window");
            });
        }
        2 => {
            card(ui, "Waveform color", |ui| {
                sub(ui, "Waveform color displayed in this app");
                for (i, n) in ["BLUE", "RGB", "3Band"].iter().enumerate() {
                    if radio(ui, app.prefs.wave_color == i as u8, n) {
                        app.prefs.wave_color = i as u8;
                    }
                }
            });
            card(ui, "Full/Preview Waveform", |ui| {
                if radio(ui, !app.prefs.wave_half, "Full Waveform") {
                    app.prefs.wave_half = false;
                }
                if radio(ui, app.prefs.wave_half, "Half Waveform") {
                    app.prefs.wave_half = true;
                }
            });
            card(ui, "Color comments", |ui| {
                sub(ui, "Customize Color Comments");
                note(ui, "Your own names for the eight track colours. They are shown wherever a colour is chosen.");
                egui::Grid::new("color_comments").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    for i in 0..8usize {
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
                        ui.painter().rect_filled(r.shrink(2.0), 2.0, label_color(i as u8 + 1));
                        ui.add(egui::TextEdit::singleline(&mut app.prefs.color_names[i]).desired_width(300.0).hint_text(crate::model::COLOR_NAMES[i + 1]));
                        ui.end_row();
                    }
                });
                if ui.button("Reset").clicked() {
                    app.prefs.color_names = crate::prefs_ext::default_color_names();
                }
            });
        }
        _ => {
            ui.label(RichText::new("  Settings of Recordbox that rekordbox keeps elsewhere or does not have.").size(12.0).color(DIM2));
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
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Resolution").size(13.0));
                    for (d, n) in [(1u8, "1 beat"), (2, "1/2"), (4, "1/4"), (8, "1/8")] {
                        if ui.radio(app.prefs.quantize_div.max(1) == d, n).clicked() {
                            app.prefs.quantize_div = d;
                        }
                    }
                });
            });
            card(ui, "Stems", |ui| {
                if radio(ui, app.prefs.stem_parts != 3, "4 parts: Vocal, Inst, Bass, Drums") {
                    app.prefs.stem_parts = 4;
                }
                if radio(ui, app.prefs.stem_parts == 3, "3 parts: Vocal, Inst (with bass), Drums") {
                    app.prefs.stem_parts = 3;
                }
            });
            card(ui, "Vocal", |ui| {
                toggle(ui, &mut app.prefs.show_vocal, "Show vocal positions above the overview waveform");
                note(ui, "From rekordbox's vocal detection, for tracks it analysed.");
            });
        }
    }
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

fn page_audio(app: &mut App, ui: &mut Ui) {
    card(ui, "Audio", |ui| {
        // Listing devices can block, so it runs in the background (once per opening, or on
        // Refresh); None while waiting, Some(None) if it gave up.
        type Slot = std::sync::Arc<parking_lot::Mutex<Option<Option<Vec<String>>>>>;
        type CapSlot = std::sync::Arc<parking_lot::Mutex<Option<Option<crate::prefs_ext::AudioCaps>>>>;
        let list_id = egui::Id::new("audio_devices");
        let slot: Slot = match ui.ctx().data(|d| d.get_temp::<Slot>(list_id)) {
            Some(s) => s,
            None => {
                let s: Slot = Default::default();
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
        // What the device in use offers (asked once per device).
        let caps_id = egui::Id::new(("audio_caps", current.clone()));
        let cap_slot: CapSlot = match ui.ctx().data(|d| d.get_temp::<CapSlot>(caps_id)) {
            Some(s) => s,
            None => {
                let s: CapSlot = Default::default();
                let s2 = s.clone();
                let ctx = ui.ctx().clone();
                let dev = current.clone();
                std::thread::spawn(move || {
                    *s2.lock() = Some(crate::prefs_ext::audio_caps_within(dev, std::time::Duration::from_secs(5)));
                    ctx.request_repaint();
                });
                ui.ctx().data_mut(|m| m.insert_temp(caps_id, s.clone()));
                s
            }
        };
        let caps = cap_slot.lock().clone().flatten();

        egui::Grid::new("audio_grid").num_columns(2).spacing(vec2(40.0, 12.0)).show(ui, |ui| {
            ui.label(RichText::new("Audio").size(14.0));
            let shown = current.clone().unwrap_or_else(|| "System default".to_string());
            let mut pick: Option<Option<String>> = None;
            egui::ComboBox::from_id_salt("audio_dev").width(330.0).selected_text(RichText::new(shown).font(bold(14.0))).show_ui(ui, |ui| {
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
            ui.end_row();

            ui.label(RichText::new("Sample Rate").size(14.0));
            let rate_text = |r: u32| format!("{} Hz", r);
            let sel = if app.prefs.audio_rate == 0 { format!("{} (device setting)", rate_text(app.player.device_rate)) } else { rate_text(app.prefs.audio_rate) };
            let mut new_rate = None;
            ui.add_enabled_ui(caps.is_some(), |ui| {
                egui::ComboBox::from_id_salt("audio_rate").width(330.0).selected_text(RichText::new(sel).font(bold(14.0))).show_ui(ui, |ui| {
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
            if let Some(r) = new_rate.filter(|r| *r != app.prefs.audio_rate) {
                app.prefs.audio_rate = r;
                apply_audio(app);
            }
            ui.end_row();

            ui.label(RichText::new("Buffer size").size(14.0));
            let sel = if app.prefs.audio_buffer == 0 { "Device setting".to_string() } else { format!("{} samples", app.prefs.audio_buffer) };
            let range = caps.as_ref().and_then(|c| c.buffer);
            let mut new_buf = None;
            ui.add_enabled_ui(caps.is_some(), |ui| {
                egui::ComboBox::from_id_salt("audio_buf").width(330.0).selected_text(RichText::new(sel).font(bold(14.0))).show_ui(ui, |ui| {
                    if ui.selectable_label(app.prefs.audio_buffer == 0, "Device setting").clicked() {
                        new_buf = Some(0);
                    }
                    for b in crate::prefs_ext::BUFFER_SIZES {
                        if range.map(|(lo, hi)| b < lo || b > hi).unwrap_or(false) {
                            continue;
                        }
                        let ms = b as f64 / app.player.device_rate.max(1) as f64 * 1000.0;
                        if ui.selectable_label(app.prefs.audio_buffer == b, format!("{b} samples ({ms:.1} ms)")).clicked() {
                            new_buf = Some(b);
                        }
                    }
                });
            });
            if let Some(b) = new_buf.filter(|b| *b != app.prefs.audio_buffer) {
                app.prefs.audio_buffer = b;
                apply_audio(app);
            }
            ui.end_row();

            ui.label(RichText::new("Output channels").size(14.0));
            let chans = caps.as_ref().map(|c| c.channels).unwrap_or(2);
            let pair = |first: u16| format!("{} / {}", first + 1, first + 2);
            let cur = if app.prefs.audio_channel + 1 < chans { app.prefs.audio_channel } else { 0 };
            let mut new_ch = None;
            ui.add_enabled_ui(chans > 2, |ui| {
                egui::ComboBox::from_id_salt("audio_ch").width(330.0).selected_text(RichText::new(if chans >= 2 { pair(cur) } else { "1".into() }).font(bold(14.0))).show_ui(ui, |ui| {
                    let mut first = 0;
                    while first + 1 < chans {
                        if ui.selectable_label(cur == first, pair(first)).clicked() {
                            new_ch = Some(first);
                        }
                        first += 2;
                    }
                });
            })
            .response
            .on_disabled_hover_text("This device has one stereo output.");
            if let Some(c) = new_ch.filter(|c| *c != app.prefs.audio_channel) {
                app.prefs.audio_channel = c;
                apply_audio(app);
            }
            ui.end_row();
        });

        match &state {
            None => note(ui, "Looking for audio devices…"),
            Some(None) => {
                ui.label(RichText::new("macOS did not answer with the list of audio devices. The system's default output is used; try Refresh.").size(11.0).color(CAUTION_TX));
            }
            _ => {}
        }
        if let (Some(d), Some(Some(_))) = (&current, &state) {
            if !devices.contains(d) {
                ui.label(RichText::new(format!("\"{d}\" is not connected; the system's default output is used.")).size(11.0).color(CAUTION_TX));
            }
        }
        ui.horizontal(|ui| {
            let dev = caps.as_ref().map(|c| c.name.clone()).unwrap_or_default();
            note(ui, &format!("Running at {} Hz{}.", app.player.device_rate, if dev.is_empty() { String::new() } else { format!(" on {dev}") }));
            if ui.small_button("Refresh").clicked() {
                ui.ctx().data_mut(|m| {
                    m.remove::<Slot>(list_id);
                    m.remove::<CapSlot>(caps_id);
                });
            }
        });
    });
}

// ------------------------------------------------------------------ Analysis

fn page_analysis(app: &mut App, ui: &mut Ui, tab: usize) {
    if tab == 1 {
        card(ui, "CUE Analysis", |ui| {
            toggle(ui, &mut app.prefs.memory_cue_first_beat, "Set Memory Cue at the first beat of track when analyzing");
            note(ui, "For tracks that have no memory cue yet.");
            ui.separator();
            toggle(ui, &mut app.prefs.auto_first_downbeat_cue, "Set Hot Cue A on the first downbeat (\"1.1Bars\") when analyzing");
            note(ui, "As rekordbox does on first analysis. Only for tracks that have no cues yet; tracks from your rekordbox library keep their own cues.");
        });
        return;
    }
    card(ui, "Track Analysis", |ui| {
        toggle_if(ui, HIGH_PRECISION_WIRED, &mut app.prefs.high_precision_grid, "Use high precision BeatGrid analysis", "Recordbox has one beat grid analysis so far; this switch has nothing to choose between yet.");
        ui.separator();
        sub(ui, "Track Analysis Mode");
        egui::Grid::new("tam").num_columns(2).spacing(vec2(40.0, 10.0)).show(ui, |ui| {
            ui.label(RichText::new("Analysis Mode").size(14.0));
            let modes = [("Normal", "Optimized for tracks which have a constant BPM"), ("Dynamic", "For tracks whose tempo changes: the beat grid follows the tempo")];
            let cur = (app.prefs.analysis_mode as usize).min(1);
            egui::ComboBox::from_id_salt("amode").width(330.0).selected_text(RichText::new(modes[cur].0).font(bold(14.0))).show_ui(ui, |ui| {
                for (i, (n, tip)) in modes.iter().enumerate() {
                    if ui.selectable_label(cur == i, *n).on_hover_text(*tip).clicked() {
                        app.prefs.analysis_mode = i as u8;
                    }
                }
            });
            ui.end_row();
            ui.label(RichText::new("BPM Range").size(14.0));
            let (lo, hi) = BPM_RANGES[app.prefs.bpm_range.min(BPM_RANGES.len() - 1)];
            egui::ComboBox::from_id_salt("bpmrange").width(330.0).selected_text(RichText::new(format!("{lo:.0}-{hi:.0}")).font(bold(14.0))).show_ui(ui, |ui| {
                for (i, (a, b)) in BPM_RANGES.iter().enumerate() {
                    ui.selectable_value(&mut app.prefs.bpm_range, i, format!("{a:.0}-{b:.0}"));
                }
            });
            ui.end_row();
        });
        ui.separator();
        sub(ui, "Track Analysis Setting");
        ui.indent("tas", |ui| {
            ui.checkbox(&mut app.prefs.analyze_grid, RichText::new("BPM / Grid").size(14.0));
            ui.checkbox(&mut app.prefs.analyze_key, RichText::new("KEY").size(14.0));
            ui.checkbox(&mut app.prefs.phrase_analysis, RichText::new("Phrase").size(14.0)).on_hover_text("Phrases (Intro, Up, Down, Chorus, Outro) are shown under the overview and written to USB exports. rekordbox's own phrases are used for tracks it analysed.");
            ui.checkbox(&mut app.prefs.vocal_analysis, RichText::new("Vocal").size(14.0)).on_hover_text("Vocal positions for loaded tracks rekordbox never analysed, found with the stem separation model once it is downloaded (about a minute per track on this Mac's GPU, in the background). rekordbox's own vocal detection is used for tracks it analysed.");
        });
        note(ui, "If tracks are not analyzed, BPM/Grid analysis starts when importing, loading, or exporting the tracks whether you tick this checkbox or not.");
        ui.separator();
        sub(ui, "Auto Analysis");
        ui.indent("aa", |ui| {
            toggle(ui, &mut app.prefs.auto_analysis, "Enable");
        });
    });
    card(ui, "Auto Gain (Recordbox)", |ui| {
        ui.add(egui::Slider::new(&mut app.prefs.target_lufs, -20.0..=-5.0).suffix(" LUFS").text("Reference loudness"));
        toggle(ui, &mut app.prefs.auto_gain_peak_cap, "Limit gain so the loudest peak stays below full scale");
        note(ui, "Used for the preview player's gain and stored per track when analysing.");
        if ui.button("Re-analyze all tracks now").clicked() {
            let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
            app.reanalyze(&ids);
        }
    });
}

// ------------------------------------------------------------------ DJ System

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
            ui.painter().rect_filled(r, 0.0, BLUE);
        }
        ui.painter().text(egui::pos2(r.min.x + 6.0, r.center().y), egui::Align2::LEFT_CENTER, name.to_uppercase(), egui::FontId::proportional(13.0), if greyed { OFF } else { WHITE });
        resp.clicked()
    };
    let list_h = 380.0;
    egui::Frame::default().fill(BG2).inner_margin(egui::Margin { left: 20, right: 20, top: 18, bottom: 18 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.vertical(|ui| {
                ui.set_width(250.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(if is_cat { "Inactive Categories" } else { "Inactive Sort Options" }).font(bold(14.0)).color(WHITE)));
                ui.add_space(10.0);
                egui::Frame::default().fill(BG).show(ui, |ui| {
                    ui.set_width(250.0);
                    ui.set_min_height(list_h);
                    egui::ScrollArea::vertical().id_salt(("dl_in", is_cat)).max_height(list_h).show(ui, |ui| {
                        for (id, name) in &inactive {
                            if row(ui, name, sel.0 == Some(*id), false) {
                                sel = (Some(*id), None);
                            }
                        }
                    });
                });
            });
            ui.vertical(|ui| {
                ui.set_width(56.0);
                ui.add_space(list_h / 2.0 - 6.0);
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    let can_add = sel.0.is_some();
                    if ui.add_enabled(can_add, egui::Button::new(">").min_size(vec2(22.0, 22.0))).on_hover_text("Make active").clicked() {
                        if let Some(id) = sel.0.take() {
                            next.push(id);
                            sel.1 = Some(id);
                        }
                    }
                    let can_remove = sel.1.map(|id| !fixed.contains(&id)).unwrap_or(false);
                    if ui.add_enabled(can_remove, egui::Button::new("<").min_size(vec2(22.0, 22.0))).on_hover_text("Make inactive").on_disabled_hover_text("Greyed items are always shown on the players.").clicked() {
                        if let Some(id) = sel.1.take() {
                            next.retain(|x| *x != id);
                            sel.0 = Some(id);
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                ui.set_width(250.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(if is_cat { "Active Categories" } else { "Active Sort Options" }).font(bold(14.0)).color(WHITE)));
                ui.add_space(10.0);
                egui::Frame::default().fill(BG).show(ui, |ui| {
                    ui.set_width(250.0);
                    ui.set_min_height(list_h - 34.0);
                    egui::ScrollArea::vertical().id_salt(("dl_ac", is_cat)).max_height(list_h - 34.0).show(ui, |ui| {
                        for id in &active {
                            let name = column_name(*id).unwrap_or("?");
                            if row(ui, name, sel.1 == Some(*id), fixed.contains(id)) {
                                sel = (None, Some(*id));
                            }
                        }
                    });
                });
                ui.add_space(8.0);
                // Order: categories move freely; fixed sort options stay where the players put them.
                let pos = sel.1.and_then(|id| active.iter().position(|x| *x == id));
                let movable = |i: usize| is_cat || !fixed.contains(&active[i]);
                let can_down = pos.map(|i| i + 1 < active.len() && movable(i) && movable(i + 1)).unwrap_or(false);
                let can_up = pos.map(|i| i > 0 && movable(i) && movable(i - 1)).unwrap_or(false);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_space(250.0 / 2.0 - 25.0);
                    let down = ui.add_enabled(can_down, egui::Button::new(" ").min_size(vec2(22.0, 22.0))).on_hover_text("Down");
                    crate::icons::chevron_down(ui.painter(), down.rect.shrink(5.0), if can_down { WHITE } else { OFF });
                    if down.clicked() {
                        if let Some(i) = pos {
                            next.swap(i, i + 1);
                        }
                    }
                    let up = ui.add_enabled(can_up, egui::Button::new(" ").min_size(vec2(22.0, 22.0))).on_hover_text("Up");
                    let r = up.rect.shrink(5.0);
                    let c = if can_up { WHITE } else { OFF };
                    ui.painter().line_segment([egui::pos2(r.min.x, r.max.y - 2.0), egui::pos2(r.center().x, r.min.y + 2.0)], egui::Stroke::new(1.5, c));
                    ui.painter().line_segment([egui::pos2(r.center().x, r.min.y + 2.0), egui::pos2(r.max.x, r.max.y - 2.0)], egui::Stroke::new(1.5, c));
                    if up.clicked() {
                        if let Some(i) = pos {
                            next.swap(i, i - 1);
                        }
                    }
                });
            });
        });
    });
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
    egui::Grid::new(id).num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
        for f in fields {
            // Waveform Divisions is on the General tab, as in rekordbox.
            if f.file == SettingFile::My2 && f.offset == 4 {
                continue;
            }
            ui.label(RichText::new(f.name.to_uppercase()).size(13.0));
            let cur = app.prefs.usb_settings.get(f.file, f.offset);
            let shown = f.options.iter().find(|o| o.1 == cur).map(|o| o.0).unwrap_or("—");
            let file_tag = match f.file {
                SettingFile::My => 0u8,
                SettingFile::My2 => 1,
                SettingFile::Dev => 2,
                SettingFile::Djm => 3,
            };
            egui::ComboBox::from_id_salt(("myset", f.offset, file_tag)).selected_text(RichText::new(shown).font(bold(13.0))).width(220.0).show_ui(ui, |ui| {
                for (label, v) in f.options {
                    if ui.selectable_label(cur == *v, *label).clicked() {
                        app.prefs.usb_settings.set(f.file, f.offset, *v);
                    }
                }
            });
            ui.end_row();
        }
    });
}

fn page_dj_system(app: &mut App, ui: &mut Ui, tab: usize) {
    match tab {
        0 => {
            card(ui, "Background Color", |ui| {
                sub(ui, "Color of device displayed on CDJ/XDJ");
                let names = ["Default Color", "Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple"];
                let cur = (app.prefs.device_bg_color as usize).min(8);
                egui::ComboBox::from_id_salt("bgcolor").width(330.0).selected_text(RichText::new(names[cur]).font(bold(14.0))).show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        let text = RichText::new(format!("■ {n}")).color(if i == 0 { DIM2 } else { label_color(i as u8) });
                        if ui.selectable_label(cur == i, text).clicked() {
                            app.prefs.device_bg_color = i as u8;
                        }
                    }
                });
                if !BG_COLOR_WRITTEN {
                    note(ui, "Not written to the device yet: where the players keep this colour has not been confirmed. Your choice is kept for when it is.");
                }
            });
            card(ui, "Waveform color", |ui| {
                sub(ui, "Waveform color displayed on CDJ/XDJ");
                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_WAVEFORM_COLOR);
                for (label, v) in [("BLUE", 1u8), ("RGB", 3), ("3Band", 4)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_WAVEFORM_COLOR, v);
                    }
                }
            });
            card(ui, "Waveform Current Position", |ui| {
                sub(ui, "Waveform Current Position displayed on CDJ/XDJ");
                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_WAVEFORM_POSITION);
                for (label, v) in [("LEFT", 2u8), ("CENTER", 1)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_WAVEFORM_POSITION, v);
                    }
                }
            });
            card(ui, "Waveform Divisions", |ui| {
                sub(ui, "Waveform division displayed under the CDJ/XDJ waveform");
                let cur = app.prefs.usb_settings.get(SettingFile::My2, 4);
                for (label, v) in [("Time Scale", 0x80u8), ("Phrase", 0x81)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::My2, 4, v);
                    }
                }
            });
            card(ui, "Type of the Overview Waveform", |ui| {
                sub(ui, "Type of the Overview Waveform displayed on CDJ/XDJ");
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
                ui.add_enabled_ui(false, |ui| {
                    egui::ComboBox::from_id_salt("rightcol").width(330.0).selected_text(RichText::new("Not Specified").font(bold(14.0))).show_ui(ui, |_| {});
                })
                .response
                .on_disabled_hover_text("Where the players store this choice has not been confirmed, so Recordbox does not write it; the players show their own default.");
                note(ui, "Not written yet: where the players store this choice has not been confirmed, so they show their own default.");
            });
        }
        4 => {
            card(ui, "My Settings", |ui| {
                ui.label(RichText::new("Player settings written to every USB you export. CDJ/XDJ players apply them when the USB is loaded.").size(12.0).color(DIM2));
                sub(ui, "CDJ / XDJ");
                setting_rows(app, ui, "my_settings_grid", crate::export::settings_fields::MY_SETTINGS);
                ui.separator();
                sub(ui, "DJM");
                setting_rows(app, ui, "djm_settings_grid", crate::export::settings_fields::DJM_SETTINGS);
                ui.add_space(6.0);
                if ui.button("Reset to rekordbox defaults").clicked() {
                    // The General tab's choices (DEVSETTING and Waveform Divisions) stay.
                    let keep = (app.prefs.usb_settings.dev.clone(), app.prefs.usb_settings.get(SettingFile::My2, 4));
                    app.prefs.usb_settings = crate::export::settings::UsbSettings { dev: keep.0, ..Default::default() };
                    app.prefs.usb_settings.set(SettingFile::My2, 4, keep.1);
                }
            });
        }
        5 => {
            card(ui, "Delete Tracks", |ui| {
                toggle(ui, &mut app.prefs.delete_device_tracks, "Delete playlist tracks from device if playlist is deleted");
            });
            card(ui, "History", |ui| {
                toggle(ui, &mut app.prefs.import_device_history, "Import the play history automatically");
                toggle_if(ui, HISTORY_DELETE_WIRED, &mut app.prefs.delete_history_after_import, "Delete from the device after importing the play history", "Recordbox does not remove histories from a device yet; imported histories are left on it.");
            });
        }
        _ => {
            card(ui, "Hot Cue Auto Load", |ui| {
                toggle(ui, &mut app.prefs.hot_cue_auto_load_on_import, "Prepare Hot Cue Auto Load setting for CDJ/XDJ use (on import)");
                note(ui, "Tracks get Hot Cue Auto Load switched on as they are imported, so players set to \"rekordbox SETTING\" load their hot cues.");
            });
            card(ui, "Waveform data written to the device", |ui| {
                ui.label(RichText::new("Exports always include the blue, RGB and 3-band waveforms; DJ System > General chooses which one the players show.").size(12.0).color(DIM2));
            });
            card(ui, "Supported devices", |ui| {
                ui.label(RichText::new("Drive formats: FAT32, exFAT (some players), HFS+ (USB only).").size(13.0));
                ui.label(RichText::new("Files: MP3, AAC/M4A, WAV, AIFF, FLAC.").size(13.0));
                ui.label(RichText::new("Device library: export.pdb + exportExt.pdb with analysis files (DAT / EXT / 2EX) and the music in /Contents.").size(12.0).color(DIM2));
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
        ui.label(RichText::new("Click a shortcut, then press the new key for that action, with ⌃ ⌥ ⇧ ⌘ if you like (Esc cancels, Delete clears).").size(12.0).color(DIM2));
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
                    app.toast_err(format!("{} is kept by the menus.", sc.label()));
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
        egui::Grid::new(("keymap_grid", tab)).num_columns(3).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
            for (id, label, def, c) in ACTIONS {
                if c != cat {
                    continue;
                }
                ui.label(RichText::new(label).size(13.0));
                let shown = if waiting.as_deref() == Some(id) { "Press a key…".to_string() } else { app.prefs.shortcut_for(id).map(|k| k.label()).unwrap_or_else(|| "—".into()) };
                if ui.add(egui::Button::new(RichText::new(shown).size(13.0)).min_size(vec2(140.0, 22.0))).clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(wait_id, Some(id.to_string())));
                }
                let changed = app.prefs.keymap.contains_key(id);
                if ui.add_enabled(changed, egui::Button::new(RichText::new("Default").size(11.0))).on_hover_text(if def.is_empty() { "Not assigned".to_string() } else { Shortcut::parse(def).map(|s| s.label()).unwrap_or_default() }).clicked() {
                    app.prefs.keymap.remove(id);
                }
                ui.end_row();
            }
        });
    });
    card(ui, "Keyboard shortcuts", |ui| {
        ui.horizontal(|ui| {
            if ui.button("Reset to defaults").clicked() {
                app.dialogs.ext.reset_ask = Some(Some(3));
            }
            if ui.button("Export…").clicked() {
                if let Some(f) = rfd::FileDialog::new().set_file_name("recordbox-shortcuts.json").save_file() {
                    let _ = std::fs::write(f, serde_json::to_vec_pretty(&app.prefs.keymap).unwrap_or_default());
                }
            }
            if ui.button("Import…").clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                    match std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<std::collections::BTreeMap<String, String>>(&b).ok()) {
                        Some(m) => app.prefs.keymap = m.into_iter().filter(|(k, v)| ACTIONS.iter().any(|a| a.0 == k) && (v.is_empty() || Shortcut::parse(v).is_some())).collect(),
                        None => app.toast_err("That file is not a Recordbox shortcut profile."),
                    }
                }
            }
            if ui.button("Show all…").clicked() {
                app.dialogs.ext.shortcuts_open = true;
            }
        });
        note(ui, "⌘ and ⌃ shortcuts of Import file, Import directory, Fullscreen mode and Preferences also appear in the menu bar.");
    });
}

// ------------------------------------------------------------------ Advanced

fn page_advanced(app: &mut App, ui: &mut Ui, tab: usize) {
    match tab {
        0 => {
            card(ui, "Database management", |ui| {
                sub(ui, "Location");
                path_row(ui, &Some(crate::db::data_dir().display().to_string()), "");
                note(ui, "Your music files are only read. Edits (tags, cues, grids) are kept in this private library.");
                if ui.button("Show in Finder").clicked() {
                    let _ = std::process::Command::new("/usr/bin/open").arg(crate::db::data_dir()).spawn();
                }
                ui.separator();
                sub(ui, "Library");
                let busy = app.dialogs.ext.backup_job.is_some();
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new(RichText::new("Backup Library").size(14.0)).min_size(vec2(150.0, 26.0))).clicked() {
                        crate::menus::handle(app, "backup");
                    }
                    if ui.add_enabled(!busy, egui::Button::new(RichText::new("Restore Library").size(14.0)).min_size(vec2(150.0, 26.0))).clicked() {
                        crate::menus::handle(app, "restore");
                    }
                });
                note(ui, "A backup is one .zip file with the database, the artwork and, if you choose, the music files. Before a restore replaces the library, the present one is copied into the backups folder here.");
            });
        }
        1 => {
            card(ui, "iTunes", |ui| {
                sub(ui, "iTunes Library File");
                path_row(ui, &app.prefs.itunes_xml, "no recently selected files");
                ui.horizontal(|ui| {
                    if ui.button("Browse").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("iTunes Library", &["xml"]).pick_file() {
                            app.prefs.itunes_xml = Some(f.to_string_lossy().to_string());
                        }
                    }
                    if app.prefs.itunes_xml.is_some() && ui.button("Clear").clicked() {
                        app.prefs.itunes_xml = None;
                    }
                });
                note(ui, "The library file the iTunes source of the Tree View reads. Without one, the usual places and the Music app are tried.");
            });
            card(ui, "rekordbox xml", |ui| {
                sub(ui, "Imported Library");
                path_row(ui, &app.prefs.rekordbox_xml_path, "no recently selected files");
                ui.horizontal(|ui| {
                    if ui.button("Browse").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file() {
                            app.prefs.rekordbox_xml_path = Some(f.to_string_lossy().to_string());
                        }
                    }
                    let path = app.prefs.rekordbox_xml_path.clone();
                    let exists = path.as_deref().map(|p| std::path::Path::new(p).is_file()).unwrap_or(false);
                    if ui.add_enabled(exists, egui::Button::new("Import now")).on_disabled_hover_text("Choose a rekordbox xml file first.").clicked() {
                        if let Some(p) = path {
                            app.import_xml(std::path::Path::new(&p));
                        }
                    }
                    if app.prefs.rekordbox_xml_path.is_some() && ui.button("Clear").clicked() {
                        app.prefs.rekordbox_xml_path = None;
                    }
                });
                note(ui, "A collection exported from rekordbox (File > Export Collection in xml format).");
            });
            card(ui, "Auto Relocate Search Folders", |ui| {
                note(ui, "Track > Relocate > Auto Relocate looks in these folders (and the folders inside them) for missing files.");
                let mut remove = None;
                for (i, f) in app.prefs.relocate_folders.iter().enumerate() {
                    ui.horizontal(|ui| {
                        path_row(ui, &Some(f.clone()), "");
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    app.prefs.relocate_folders.remove(i);
                }
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            let s = d.to_string_lossy().to_string();
                            if !app.prefs.relocate_folders.contains(&s) {
                                app.prefs.relocate_folders.push(s);
                            }
                        }
                    }
                    let any = !app.prefs.relocate_folders.is_empty() && app.tracks.iter().any(|t| t.missing);
                    if ui.add_enabled(any, egui::Button::new("Auto Relocate now")).on_disabled_hover_text("Needs a folder here and at least one missing file.").clicked() {
                        app.auto_relocate(None);
                    }
                });
            });
        }
        _ => {
            card(ui, "Stem separation", |ui| {
                if let Some(hw) = &app.stem.hw {
                    ui.label(
                        RichText::new(format!(
                            "This computer: {}{} cores, {:.0} GB memory, {}.",
                            if hw.cpu.is_empty() { String::new() } else { format!("{}, ", hw.cpu) },
                            hw.cores,
                            hw.ram_gb,
                            if hw.gpu { "GPU (Metal) available" } else { "no usable GPU, separation runs on the CPU" }
                        ))
                        .size(12.0),
                    );
                    ui.label(RichText::new(format!("Recommended model: {}", crate::stems::choose(hw).name)).size(12.0).color(DIM2));
                } else {
                    ui.label(RichText::new("Checking this computer…").size(12.0).color(DIM));
                }
                let cur = app.stem.model.clone().or_else(|| app.prefs.stem_model.clone()).unwrap_or_default();
                for m in crate::stems::CATALOG.iter() {
                    let ready = crate::stems::model_ready(m);
                    let label = format!("{}{}", m.name, if ready { "  (downloaded)" } else { "" });
                    if ui.radio(cur == m.id, RichText::new(label).size(13.0)).on_hover_text(m.about).clicked() && cur != m.id {
                        app.prefs.stem_model = Some(m.id.to_string());
                        app.stem.model = Some(m.id.to_string());
                        app.stem.model_ready = ready;
                    }
                    note(ui, m.about);
                }
                toggle(ui, &mut app.prefs.stems_auto_download, "Download the model automatically at launch");
                if !app.stem.model_ready && app.stem.downloading.is_none() && ui.button("Download now").clicked() {
                    app.stems_download();
                }
                if let Some((d, t)) = app.stem.downloading {
                    ui.label(RichText::new(if t > 0 { format!("Downloading… {:.0}%", d as f64 / t as f64 * 100.0) } else { "Downloading…".into() }).size(12.0));
                }
                note(ui, &format!("Separated stems are kept so each track is only split once: {:.1} GB in use.", crate::stems::cache_size() as f64 / 1e9));
                if ui.button("Clear separated stems").clicked() {
                    crate::stems::clear_cache();
                }
            });
            card(ui, "rekordbox Library", |ui| {
                toggle(ui, &mut app.prefs.rekordbox_sync, "Load and sync the rekordbox library when Recordbox starts");
                note(ui, "Recordbox copies rekordbox's library into its own folder and reads the copy. rekordbox's own files are never opened by a database or changed.");
                if ui.add_enabled(app.rb_job.is_none(), egui::Button::new("Sync now")).clicked() {
                    crate::menus::handle(app, "sync_rekordbox");
                }
            });
        }
    }
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
    egui::Window::new("Keyboard Shortcuts").open(&mut open).default_size(vec2(520.0, 560.0)).collapsible(false).show(ctx, |ui| {
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
                            Some(k) => ui.label(RichText::new(k.label()).size(12.5).color(WHITE)),
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
        if ui.button("Change shortcuts…").clicked() {
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
        egui::Window::new("Library Backup").collapsible(false).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.label(RichText::new("Do you want to back up music files as well?").size(14.0).color(WHITE));
            let n = app.tracks.len();
            let bytes: u64 = app.tracks.iter().filter(|t| !t.missing).map(|t| t.file_size as u64).sum();
            note(ui, &format!("Database file, analysis and image files are always included. Music files: {n} track(s), about {:.1} GB.", bytes as f64 / 1e9));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Yes").clicked() {
                    choice = Some(Some(true));
                }
                if ui.button("No").clicked() {
                    choice = Some(Some(false));
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(None);
                }
            });
        });
        if let Some(c) = choice {
            app.dialogs.ext.backup_ask = false;
            if let Some(music) = c {
                let name = format!("Recordbox-backup-{}.zip", chrono::Local::now().format("%Y%m%d"));
                if let Some(f) = rfd::FileDialog::new().set_title("Backup file").set_file_name(name).add_filter("Recordbox library backup", &["zip"]).save_file() {
                    app.save_prefs();
                    let _ = app.db.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
                    app.dialogs.ext.backup_job = Some(crate::backup::start_backup(ctx.clone(), f, music));
                }
            }
        }
    }
    if let Some(path) = app.dialogs.ext.restore_ask.clone() {
        let mut choice = None;
        egui::Window::new("Library Restoration").collapsible(false).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.label(RichText::new("Are you sure you want to overwrite the present library with a backup?").size(14.0).color(WHITE));
            note(ui, &format!("Backup file: {}", path.display()));
            if let Ok(m) = crate::backup::read_manifest(&path) {
                note(ui, &format!("Made {} · {} track(s) · {} artwork file(s) · {} music file(s)", m.created.get(..16).unwrap_or(&m.created).replace('T', " "), m.tracks, m.artwork_files, m.music.len()));
            }
            note(ui, "The present library is copied into the backups folder first. Music files are only put back where the file is missing.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Yes").clicked() {
                    choice = Some(true);
                }
                if ui.button("No").clicked() {
                    choice = Some(false);
                }
            });
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
        egui::Window::new(if restore { "Library Restoration" } else { "Library Backup" }).collapsible(false).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(if total == 0 { if restore { "Preparing restore..." } else { "Preparing backup..." } } else if restore { "Restore Collection" } else { "Back up Collection" }).size(14.0).color(WHITE));
            let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).show_percentage());
            ui.label(RichText::new(what).size(11.0).color(DIM));
            let cancelling = job.cancel.load(std::sync::atomic::Ordering::Relaxed);
            if ui.add_enabled(!cancelling, egui::Button::new(if cancelling { "Cancelling…" } else { "Cancel" })).clicked() {
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
        egui::Window::new(title).collapsible(false).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.set_max_width(460.0);
            for (i, line) in text.lines().enumerate() {
                ui.label(RichText::new(line).size(if i == 0 { 14.0 } else { 12.0 }).color(if i == 0 { WHITE } else { DIM2 }));
            }
            ui.add_space(6.0);
            if ui.button("OK").clicked() {
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
