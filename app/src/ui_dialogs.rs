//! Windows: Preferences, track Property editor, intelligent playlist editor, export to device,
//! confirmations.

use eframe::egui;
use crate::analysis::BPM_RANGES;
use crate::app::*;
use crate::model::*;
use crate::smart::*;
use crate::theme::*;
use egui::{vec2, Context, RichText, Ui};

fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::default().fill(BG2).inner_margin(12.0).corner_radius(4.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).size(14.0).strong().color(WHITE));
        ui.add_space(6.0);
        add(ui);
    });
    ui.add_space(8.0);
}

pub fn dialogs(app: &mut App, ctx: &Context) {
    prefs_window(app, ctx);
    info_window(app, ctx);
    smart_window(app, ctx);
    export_window(app, ctx);
    confirm_windows(app, ctx);
}

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

/// Blue pill switch with a white knob (off: grey).
fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(32.0, 17.0), egui::Sense::click());
        if resp.clicked() {
            *on = !*on;
            changed = true;
        }
        let p = ui.painter();
        p.rect_filled(r, 8.5, if *on { BLUE } else { DIM });
        let kx = if *on { r.max.x - 8.5 } else { r.min.x + 8.5 };
        p.circle_filled(egui::pos2(kx, r.center().y), 6.5, WHITE);
        ui.add_space(4.0);
        if ui.add(egui::Label::new(RichText::new(label).size(14.0).color(WHITE)).sense(egui::Sense::click())).clicked() {
            *on = !*on;
            changed = true;
        }
    });
    changed
}

fn prefs_window(app: &mut App, ctx: &Context) {
    if !app.dialogs.prefs_open {
        return;
    }
    // Preferences opens in its own window, like the original. Screenshot mode (and backends
    // without multiple windows) draw it inside the main window instead.
    if app.screenshot.is_some() {
        prefs_embedded(app, ctx);
        return;
    }
    let vid = egui::ViewportId::from_hash_of("prepdeck_preferences");
    let builder = egui::ViewportBuilder::default().with_title("Preferences").with_inner_size([800.0, 828.0]).with_min_inner_size([640.0, 480.0]);
    ctx.show_viewport_immediate(vid, builder, |vctx, class| {
        if class == egui::ViewportClass::Embedded {
            prefs_embedded(app, vctx);
            return;
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(vctx, |ui| prefs_body(app, ui));
        if vctx.input(|i| i.viewport().close_requested()) {
            app.dialogs.prefs_open = false;
        }
    });
}

fn prefs_embedded(app: &mut App, ctx: &Context) {
    let mut open = true;
    let frame = egui::Frame::window(&ctx.style()).fill(BG).inner_margin(0.0);
    egui::Window::new("Preferences").open(&mut open).default_size(vec2(800.0, 640.0)).collapsible(false).frame(frame).show(ctx, |ui| prefs_body(app, ui));
    app.dialogs.prefs_open = open;
}

fn prefs_body(app: &mut App, ui: &mut Ui) {
        ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
        ui.horizontal_top(|ui| {
            // Sidebar 160 pt, #1E1E1E: search field, then categories with icons.
            egui::Frame::default().fill(BG2).inner_margin(8.0).show(ui, |ui| ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 3.0);
                ui.set_width(144.0);
                ui.set_min_height(600.0);
                let (sr, _) = ui.allocate_exact_size(vec2(144.0, 24.0), egui::Sense::hover());
                ui.painter().rect_filled(sr, 0.0, BG);
                crate::icons::magnifier(ui.painter(), egui::Rect::from_center_size(egui::pos2(sr.min.x + 12.0, sr.center().y), vec2(16.0, 16.0)), DIM2, 0);
                ui.add_space(14.0);
                type Ic = fn(&egui::Painter, egui::Rect, egui::Color32);
                let items: [(&str, Ic); 4] = [("View", crate::icons::info), ("Analysis", crate::icons::view_columns), ("DJ System", crate::icons::usb), ("Advanced", crate::icons::hamburger)];
                for (i, (n, ic)) in items.iter().enumerate() {
                    let sel = app.dialogs.prefs_tab == i as u8;
                    let (r, resp) = ui.allocate_exact_size(vec2(144.0, 30.0), egui::Sense::click());
                    if sel {
                        ui.painter().rect_filled(r, 0.0, BLUE);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, hex(0x2A2A2A));
                    }
                    ic(ui.painter(), egui::Rect::from_center_size(egui::pos2(r.min.x + 15.0, r.center().y), vec2(18.0, 18.0)), if sel { WHITE } else { DIM });
                    ui.painter().text(egui::pos2(r.min.x + 40.0, r.center().y), egui::Align2::LEFT_CENTER, *n, egui::FontId::proportional(14.0), if sel { WHITE } else { DIM2 });
                    if resp.clicked() {
                        app.dialogs.prefs_tab = i as u8;
                    }
                }
            }));
            ui.vertical(|ui| {
                ui.set_width(620.0);
                // Tab strip (black): active tab blue, inactive #1E1E1E with #AAAAAA text.
                let tabs: &[&str] = match app.dialogs.prefs_tab {
                    0 => &["Display Type", "Color", "Player"],
                    1 => &["Track Analysis", "CUE Analysis"],
                    2 => &["General", "Category", "Sort", "Column", "My Settings", "Device"],
                    _ => &["Library"],
                };
                let key = format!("prefs_subtab_{}", app.dialogs.prefs_tab);
                let mut sub_tab: usize = ui.ctx().data_mut(|d| *d.get_temp_mut_or(egui::Id::new(&key), 0usize));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add_space(4.0);
                    for (i, t) in tabs.iter().enumerate() {
                        let disabled = app.dialogs.prefs_tab == 1 && i == 1;
                        let sel = sub_tab == i;
                        let b = egui::Button::new(RichText::new(*t).size(14.0).color(if sel { WHITE } else if disabled { OFF } else { DIM2 })).fill(if sel { BLUE } else { BG2 }).corner_radius(0.0).min_size(vec2(0.0, 30.0));
                        let r = ui.add(b);
                        let r = if disabled { r.on_hover_text("Automatic cue analysis is not part of this app.") } else { r };
                        if r.clicked() && !disabled {
                            sub_tab = i;
                        }
                    }
                });
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(&key), sub_tab));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    let before = format!("{:?}", app.prefs);
                    match (app.dialogs.prefs_tab, sub_tab) {
                        (0, 0) => {
                            card(ui, "Key display format", |ui| {
                                if ui.radio(!app.prefs.key_alnum, RichText::new("Classic").size(14.0)).clicked() {
                                    app.prefs.key_alnum = false;
                                    app.list_dirty = true;
                                }
                                if ui.radio(app.prefs.key_alnum, RichText::new("Alphanumeric").size(14.0)).clicked() {
                                    app.prefs.key_alnum = true;
                                    app.list_dirty = true;
                                }
                            });
                            card(ui, "Tooltips", |ui| {
                                toggle(ui, &mut app.prefs.tooltips, "Show Tooltips");
                            });
                        }
                        (0, 1) => {
                            card(ui, "Waveform color", |ui| {
                                sub(ui, "Waveform color displayed in this app");
                                for (i, n) in ["BLUE", "RGB", "3Band"].iter().enumerate() {
                                    if ui.radio(app.prefs.wave_color == i as u8, RichText::new(*n).size(14.0)).clicked() {
                                        app.prefs.wave_color = i as u8;
                                    }
                                }
                            });
                        }
                        (0, _) => {
                            card(ui, "Time Mode", |ui| {
                                if ui.radio(!app.prefs.time_remain, RichText::new("Elapsed").size(14.0)).clicked() {
                                    app.prefs.time_remain = false;
                                }
                                if ui.radio(app.prefs.time_remain, RichText::new("Remaining").size(14.0)).clicked() {
                                    app.prefs.time_remain = true;
                                }
                            });
                            card(ui, "Quantize", |ui| {
                                toggle(ui, &mut app.prefs.quantize, "Cues and loops snap to the beat grid");
                            });
                        }
                        (1, _) => {
                            card(ui, "Track Analysis Mode", |ui| {
                                egui::Grid::new("tam").num_columns(2).spacing(vec2(40.0, 10.0)).show(ui, |ui| {
                                    ui.label(RichText::new("Analysis Mode").size(14.0));
                                    egui::ComboBox::from_id_salt("amode").width(330.0).selected_text(RichText::new("Normal").font(bold(14.0))).show_ui(ui, |ui| {
                                        let _ = ui.selectable_label(true, "Normal").on_hover_text("Optimized for tracks which have a constant BPM");
                                    });
                                    ui.end_row();
                                    ui.label(RichText::new("BPM Range").size(14.0));
                                    let (lo, hi) = BPM_RANGES[app.prefs.bpm_range.min(9)];
                                    egui::ComboBox::from_id_salt("bpmrange").width(330.0).selected_text(RichText::new(format!("{lo:.0}-{hi:.0}")).font(bold(14.0))).show_ui(ui, |ui| {
                                        for (i, (a, b)) in BPM_RANGES.iter().enumerate() {
                                            ui.selectable_value(&mut app.prefs.bpm_range, i, format!("{a:.0}-{b:.0}"));
                                        }
                                    });
                                    ui.end_row();
                                });
                            });
                            card(ui, "Track Analysis Setting", |ui| {
                                ui.checkbox(&mut app.prefs.analyze_grid, RichText::new("BPM / Grid").size(14.0));
                                ui.checkbox(&mut app.prefs.analyze_key, RichText::new("KEY").size(14.0));
                                ui.label(RichText::new("If tracks are not analyzed, BPM/Grid analysis starts when importing, loading, or exporting the tracks whether you tick this checkbox or not.").size(11.0).color(DIM));
                            });
                            card(ui, "Auto Analysis", |ui| {
                                toggle(ui, &mut app.prefs.auto_analysis, "Enable");
                            });
                            card(ui, "Auto Gain", |ui| {
                                ui.add(egui::Slider::new(&mut app.prefs.target_lufs, -20.0..=-5.0).suffix(" LUFS").text("Reference loudness"));
                                ui.label(RichText::new("Used for the preview player's gain and stored per track when analysing.").size(11.0).color(DIM));
                            });
                            if ui.button("Re-analyze all tracks now").clicked() {
                                let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
                                app.reanalyze(&ids);
                            }
                        }
                        (2, 0) => {
                            card(ui, "Waveform color", |ui| {
                                sub(ui, "Waveform data written to the device");
                                ui.label(RichText::new("Exports always include the blue, RGB and 3-band waveforms; the player's own setting chooses which one it shows.").size(12.0).color(DIM2));
                            });
                            card(ui, "Supported devices", |ui| {
                                ui.label(RichText::new("Drive formats: FAT32, exFAT (some players), HFS+ (USB only).").size(13.0));
                                ui.label(RichText::new("Files: MP3, AAC/M4A, WAV, AIFF, FLAC.").size(13.0));
                            });
                        }
                        (2, 0) => {
                            use crate::export::settings::{SettingFile, DEV_WAVEFORM_COLOR, DEV_WAVEFORM_POSITION};
                            card(ui, "Waveform color", |ui| {
                                sub(ui, "Waveform color displayed on CDJ/XDJ");
                                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_WAVEFORM_COLOR);
                                for (label, v) in [("BLUE", 1u8), ("RGB", 3), ("3Band", 4)] {
                                    if ui.radio(cur == v, RichText::new(label).size(14.0)).clicked() {
                                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_WAVEFORM_COLOR, v);
                                    }
                                }
                            });
                            card(ui, "Waveform Current Position", |ui| {
                                sub(ui, "Waveform Current Position displayed on CDJ/XDJ");
                                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_WAVEFORM_POSITION);
                                for (label, v) in [("LEFT", 2u8), ("CENTER", 1)] {
                                    if ui.radio(cur == v, RichText::new(label).size(14.0)).clicked() {
                                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_WAVEFORM_POSITION, v);
                                    }
                                }
                            });
                            card(ui, "Waveform data written to the device", |ui| {
                                ui.label(RichText::new("Exports always include the blue, RGB and 3-band waveforms; the setting above chooses which one the players show.").size(12.0).color(DIM2));
                            });
                        }
                        (2, 1) | (2, 2) => {
                            let is_cat = sub_tab == 1;
                            let all = if is_cat { crate::export::usb::UsbBrowse::all_categories() } else { crate::export::usb::UsbBrowse::all_sorts() };
                            let fixed: &[u16] = if is_cat { &crate::export::usb::FIXED_CATEGORIES } else { &crate::export::usb::FIXED_SORTS };
                            let mut b = app.prefs.usb_browse.normalized();
                            let active: Vec<u16> = if is_cat { b.categories.clone() } else { b.sorts.clone() };
                            let mut new_active = active.clone();
                            card(ui, if is_cat { "Category" } else { "Sort" }, |ui| {
                                ui.label(RichText::new(if is_cat { "Browse categories shown on CDJ/XDJ for USBs you export. Greyed items are always shown." } else { "Sort options offered on CDJ/XDJ for USBs you export." }).size(12.0).color(DIM2));
                                ui.horizontal_top(|ui| {
                                    ui.vertical(|ui| {
                                        ui.set_width(220.0);
                                        ui.label(RichText::new(if is_cat { "Inactive Categories" } else { "Inactive Sort Options" }).size(14.0).strong());
                                        let mut inactive: Vec<(u16, &str)> = all.iter().copied().filter(|(id, _)| !active.contains(id)).collect();
                                        inactive.sort_by_key(|x| x.1);
                                        for (id, name) in inactive {
                                            ui.horizontal(|ui| {
                                                if ui.small_button("›").on_hover_text("Make active").clicked() {
                                                    new_active.push(id);
                                                }
                                                ui.label(RichText::new(name).size(13.0));
                                            });
                                        }
                                    });
                                    ui.add_space(20.0);
                                    ui.vertical(|ui| {
                                        ui.set_width(260.0);
                                        ui.label(RichText::new(if is_cat { "Active Categories" } else { "Active Sort Options" }).size(14.0).strong());
                                        for (i, id) in active.iter().enumerate() {
                                            let name = crate::export::usb::column_name(*id).unwrap_or("?");
                                            let is_fixed = fixed.contains(id);
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(name).size(13.0).color(if is_fixed { OFF } else { WHITE }));
                                                if !is_fixed {
                                                    if ui.small_button("‹").on_hover_text("Make inactive").clicked() {
                                                        new_active.retain(|x| x != id);
                                                    }
                                                }
                                                if (is_cat || !is_fixed) && i > 0 && !(!is_cat && fixed.contains(&active[i - 1])) && ui.small_button("▲").clicked() {
                                                    new_active.swap(i, i - 1);
                                                }
                                                if (is_cat || !is_fixed) && i + 1 < active.len() && ui.small_button("▼").clicked() {
                                                    new_active.swap(i, i + 1);
                                                }
                                            });
                                        }
                                    });
                                });
                            });
                            if new_active != active {
                                if is_cat {
                                    b.categories = new_active;
                                } else {
                                    b.sorts = new_active;
                                }
                                app.prefs.usb_browse = b.normalized();
                            }
                        }
                        (2, 3) => {
                            card(ui, "Column", |ui| {
                                sub(ui, "Select item which is shown next to track name on CDJ/XDJ");
                                ui.add_enabled(false, egui::Button::new("KEY"));
                                ui.label(RichText::new("Not written yet: where the players store this choice has not been confirmed, so they show their default (KEY).").size(11.0).color(DIM));
                            });
                        }
                        (2, 4) => {
                            card(ui, "My Settings", |ui| {
                                ui.label(RichText::new("Player settings written to every USB you export (MYSETTING.DAT). CDJ/XDJ players apply them when the USB is loaded.").size(12.0).color(DIM2));
                                egui::Grid::new("my_settings_grid").num_columns(2).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                                    for f in crate::export::settings_fields::MY_SETTINGS {
                                        ui.label(RichText::new(f.name).size(13.0));
                                        let cur = app.prefs.usb_settings.get(f.file, f.offset);
                                        let shown = f.options.iter().find(|o| o.1 == cur).map(|o| o.0).unwrap_or("—");
                                        egui::ComboBox::from_id_salt(("myset", f.offset, f.file == crate::export::settings::SettingFile::My2)).selected_text(shown).width(180.0).show_ui(ui, |ui| {
                                            for (label, v) in f.options {
                                                if ui.selectable_label(cur == *v, *label).clicked() {
                                                    app.prefs.usb_settings.set(f.file, f.offset, *v);
                                                }
                                            }
                                        });
                                        ui.end_row();
                                    }
                                });
                                if ui.button("Reset to rekordbox defaults").clicked() {
                                    let dev = app.prefs.usb_settings.dev.clone();
                                    app.prefs.usb_settings = crate::export::settings::UsbSettings { dev, ..Default::default() };
                                }
                            });
                        }
                        (2, _) => {
                            card(ui, "Delete Tracks", |ui| {
                                toggle(ui, &mut app.prefs.delete_device_tracks, "Delete playlist tracks from device if playlist is deleted");
                            });
                            card(ui, "Library", |ui| {
                                sub(ui, "Device library format");
                                ui.label(RichText::new("Standard device library (export.pdb + exportExt.pdb) with analysis files (DAT / EXT / 2EX) and the music in /Contents.").size(12.0).color(DIM2));
                            });
                        }
                        _ => {
                            card(ui, "Library", |ui| {
                                ui.label(RichText::new(format!("Library location: {}", crate::db::data_dir().display())).size(13.0));
                                ui.label(RichText::new("Your music files are only read. Edits (tags, cues, grids) are kept in this private library.").size(11.0).color(DIM));
                                if ui.button("Show in Finder").clicked() {
                                    let _ = std::process::Command::new("/usr/bin/open").arg(crate::db::data_dir()).spawn();
                                }
                            });
                            card(ui, "Stem separation", |ui| {
                                if let Some(hw) = &app.stem.hw {
                                    ui.label(RichText::new(format!(
                                        "This computer: {}{} cores, {:.0} GB memory, {}.",
                                        if hw.cpu.is_empty() { String::new() } else { format!("{}, ", hw.cpu) },
                                        hw.cores,
                                        hw.ram_gb,
                                        if hw.gpu { "GPU (Metal) available" } else { "no usable GPU, separation runs on the CPU" }
                                    )).size(12.0));
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
                                    ui.label(RichText::new(m.about).size(11.0).color(DIM));
                                }
                                toggle(ui, &mut app.prefs.stems_auto_download, "Download the model automatically at launch");
                                if !app.stem.model_ready && app.stem.downloading.is_none() && ui.button("Download now").clicked() {
                                    app.stems_download();
                                }
                                if let Some((d, t)) = app.stem.downloading {
                                    ui.label(RichText::new(if t > 0 { format!("Downloading… {:.0}%", d as f64 / t as f64 * 100.0) } else { "Downloading…".into() }).size(12.0));
                                }
                                ui.label(RichText::new(format!("Separated stems are kept so each track is only split once: {:.1} GB in use.", crate::stems::cache_size() as f64 / 1e9)).size(11.0).color(DIM));
                                if ui.button("Clear separated stems").clicked() {
                                    crate::stems::clear_cache();
                                }
                            });
                            card(ui, "rekordbox Library", |ui| {
                                toggle(ui, &mut app.prefs.rekordbox_sync, "Load and sync the rekordbox library when PrepDeck starts");
                                ui.label(RichText::new("PrepDeck copies rekordbox's library into its own folder and reads the copy. rekordbox's own files are never opened by a database or changed.").size(11.0).color(DIM));
                                if ui.button("Sync now").clicked() {
                                    app.start_rekordbox_sync();
                                }
                            });
                            card(ui, "rekordbox xml", |ui| {
                                ui.label(RichText::new("Import a collection exported from rekordbox (File > Export Collection in xml format).").size(13.0));
                                if ui.button("Import xml…").clicked() {
                                    if let Some(f) = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file() {
                                        app.import_xml(&f);
                                    }
                                }
                            });
                        }
                    }
                    if format!("{:?}", app.prefs) != before {
                        app.save_prefs();
                    }
                });
                ui.add_space(6.0);
                if ui.add(egui::Button::new(RichText::new("Reset to defaults").size(14.0)).fill(hex(0x1E1E20)).min_size(vec2(250.0, 30.0))).clicked() {
                    let extra = app.prefs.devices_extra.clone();
                    app.prefs = Prefs::default();
                    app.prefs.devices_extra = extra;
                    app.save_prefs();
                    app.list_dirty = true;
                }
            });
        });
}

fn info_window(app: &mut App, ctx: &Context) {
    let Some(mut t) = app.dialogs.info_track.clone() else { return };
    let mut open = true;
    let mut save = false;
    egui::Window::new("Property").open(&mut open).default_width(460.0).collapsible(false).show(ctx, |ui| {
        egui::Grid::new("prop").num_columns(2).spacing(vec2(10.0, 5.0)).show(ui, |ui| {
            let mut field = |ui: &mut Ui, label: &str, v: &mut String| {
                ui.label(RichText::new(label).color(DIM2));
                ui.add(egui::TextEdit::singleline(v).desired_width(320.0));
                ui.end_row();
            };
            field(ui, "Track Title", &mut t.title);
            field(ui, "Artist", &mut t.artist);
            field(ui, "Album", &mut t.album);
            field(ui, "Album Artist", &mut t.album_artist);
            field(ui, "Genre", &mut t.genre);
            field(ui, "Label", &mut t.label);
            field(ui, "Remixer", &mut t.remixer);
            field(ui, "Original Artist", &mut t.original_artist);
            field(ui, "Composer", &mut t.composer);
            field(ui, "Lyricist", &mut t.lyricist);
            field(ui, "Mix Name", &mut t.mix_name);
            field(ui, "Grouping", &mut t.grouping);
            field(ui, "Comments", &mut t.comment);
            field(ui, "Message", &mut t.message);
            field(ui, "Release Date", &mut t.release_date);
            ui.label(RichText::new("Year / Track / Disc").color(DIM2));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut t.year).range(0..=9999));
                ui.add(egui::DragValue::new(&mut t.track_no).range(0..=999));
                ui.add(egui::DragValue::new(&mut t.disc_no).range(0..=99));
            });
            ui.end_row();
            ui.label(RichText::new("Key").color(DIM2));
            let alnum = app.prefs.key_alnum;
            egui::ComboBox::from_id_salt("propkey").selected_text(if t.key < 0 { "—".to_string() } else { key_name(t.key, alnum) }).show_ui(ui, |ui| {
                ui.selectable_value(&mut t.key, -1, "—");
                for k in 0..24 {
                    ui.selectable_value(&mut t.key, k, format!("{} ({})", key_name(k, alnum), key_name(k, !alnum)));
                }
            });
            ui.end_row();
            ui.label(RichText::new("Rating").color(DIM2));
            ui.add(egui::Slider::new(&mut t.rating, 0..=5));
            ui.end_row();
            ui.label(RichText::new("Color").color(DIM2));
            egui::ComboBox::from_id_salt("propcolor").selected_text(COLOR_NAMES[t.color as usize]).show_ui(ui, |ui| {
                for i in 0..9u8 {
                    ui.selectable_value(&mut t.color, i, COLOR_NAMES[i as usize]);
                }
            });
            ui.end_row();
            ui.label(RichText::new("Location").color(DIM2));
            ui.label(RichText::new(&t.path).size(10.5).color(DIM));
            ui.end_row();
            ui.label(RichText::new("File").color(DIM2));
            ui.label(RichText::new(format!("{} · {} kbps · {} Hz · {:.1} MB · {}", t.file_type.to_uppercase(), t.bitrate, t.sample_rate, t.file_size as f64 / 1e6, fmt_time(t.duration, false))).size(10.5).color(DIM));
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Changes are saved in your library only; the audio file is not modified.").size(10.5).color(DIM));
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                save = true;
            }
            if ui.button("Cancel").clicked() {
                app.dialogs.info_track = None;
            }
        });
    });
    if save {
        let id = t.id;
        if let Some(slot) = app.track_mut(id) {
            *slot = t;
        }
        app.save_track(id);
        app.dialogs.info_track = None;
    } else if !open {
        app.dialogs.info_track = None;
    } else if app.dialogs.info_track.is_some() {
        app.dialogs.info_track = Some(t);
    }
}

fn smart_window(app: &mut App, ctx: &Context) {
    let Some((id, parent, mut name, mut rule)) = app.dialogs.smart_edit.clone() else { return };
    let mut open = true;
    let mut done: Option<bool> = None;
    let title = if id.is_some() { "Edit the Intelligent Playlist" } else { "Create New Intelligent Playlist" };
    egui::Window::new(title).open(&mut open).default_width(640.0).collapsible(false).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("List Name");
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(300.0));
        });
        ui.horizontal(|ui| {
            ui.label("Match");
            egui::ComboBox::from_id_salt("matchall").selected_text(if rule.match_all { "all of the" } else { "any of the" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut rule.match_all, true, "all of the");
                ui.selectable_value(&mut rule.match_all, false, "any of the");
            });
            ui.label("following conditions:");
        });
        ui.separator();
        let mut remove = None;
        let alnum = app.prefs.key_alnum;
        let tags: Vec<MyTag> = app.mytags.clone();
        for (i, c) in rule.conditions.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("field", i)).width(130.0).selected_text(c.field.label()).show_ui(ui, |ui| {
                    for f in FIELDS {
                        if ui.selectable_label(c.field == f, f.label()).clicked() {
                            c.field = f;
                            let ops = Op::for_type(f.ftype());
                            if !ops.contains(&c.op) {
                                c.op = ops[0];
                            }
                            c.value.clear();
                            c.value2.clear();
                        }
                    }
                });
                let ops = Op::for_type(c.field.ftype());
                egui::ComboBox::from_id_salt(("op", i)).width(120.0).selected_text(c.op.label()).show_ui(ui, |ui| {
                    for o in ops {
                        ui.selectable_value(&mut c.op, *o, o.label());
                    }
                });
                match c.field.ftype() {
                    FieldType::Key => {
                        let k = parse_key(&c.value);
                        egui::ComboBox::from_id_salt(("kv", i)).selected_text(if k < 0 { "—".to_string() } else { key_name(k, alnum) }).show_ui(ui, |ui| {
                            for k in 0..24 {
                                if ui.selectable_label(false, key_name(k, alnum)).clicked() {
                                    c.value = key_name(k, false);
                                }
                            }
                        });
                    }
                    FieldType::Color => {
                        let v = c.value.parse::<usize>().unwrap_or(0).min(8);
                        egui::ComboBox::from_id_salt(("cv", i)).selected_text(COLOR_NAMES[v]).show_ui(ui, |ui| {
                            for k in 0..9 {
                                if ui.selectable_label(v == k, COLOR_NAMES[k]).clicked() {
                                    c.value = k.to_string();
                                }
                            }
                        });
                    }
                    FieldType::Tag => {
                        let cur = c.value.parse::<i64>().unwrap_or(-1);
                        let label = tags.iter().find(|t| t.id == cur).map(|t| t.name.clone()).unwrap_or("—".into());
                        egui::ComboBox::from_id_salt(("tv", i)).selected_text(label).show_ui(ui, |ui| {
                            for t in tags.iter().filter(|t| t.parent != 0) {
                                if ui.selectable_label(cur == t.id, &t.name).clicked() {
                                    c.value = t.id.to_string();
                                }
                            }
                        });
                    }
                    FieldType::Rating => {
                        let mut v = c.value.parse::<i32>().unwrap_or(0);
                        if ui.add(egui::DragValue::new(&mut v).range(0..=5).suffix(" ★")).changed() {
                            c.value = v.to_string();
                        }
                        if c.op == Op::InRange {
                            ui.label("to");
                            let mut v2 = c.value2.parse::<i32>().unwrap_or(5);
                            if ui.add(egui::DragValue::new(&mut v2).range(0..=5).suffix(" ★")).changed() {
                                c.value2 = v2.to_string();
                            }
                        }
                    }
                    _ => {
                        let hint = match c.field.ftype() {
                            FieldType::Date => "YYYY-MM-DD",
                            FieldType::Number if c.field == Field::Time => "mm:ss",
                            _ => "",
                        };
                        if matches!(c.op, Op::InLast | Op::NotInLast) {
                            ui.add(egui::TextEdit::singleline(&mut c.value).desired_width(50.0));
                            egui::ComboBox::from_id_salt(("unit", i)).width(80.0).selected_text(if c.months { "month(s)" } else { "day(s)" }).show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.months, false, "day(s)");
                                ui.selectable_value(&mut c.months, true, "month(s)");
                            });
                        } else {
                            ui.add(egui::TextEdit::singleline(&mut c.value).desired_width(140.0).hint_text(hint));
                            if c.op == Op::InRange {
                                ui.label("to");
                                ui.add(egui::TextEdit::singleline(&mut c.value2).desired_width(120.0).hint_text(hint));
                            }
                        }
                    }
                }
                if ui.small_button("−").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            rule.conditions.remove(i);
        }
        if ui.small_button("+ Add condition").clicked() {
            rule.conditions.push(Condition::default());
        }
        ui.separator();
        ui.checkbox(&mut rule.auto_update, "Update automatically");
        let n = app.smart_tracks(&rule).len();
        ui.label(RichText::new(format!("{n} track(s) match now")).color(DIM));
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() {
                done = Some(false);
            }
        });
    });
    match done {
        Some(true) => {
            let nm = if name.trim().is_empty() { "Untitled Intelligent List".to_string() } else { name.trim().to_string() };
            match id {
                Some(id) => {
                    let _ = app.db.rename_node(id, &nm);
                    let _ = app.db.set_smart(id, &rule);
                }
                None => {
                    let _ = app.db.create_node(parent, &nm, NodeKind::Smart, Some(&rule));
                }
            }
            app.reload_playlists();
            app.dialogs.smart_edit = None;
        }
        Some(false) => app.dialogs.smart_edit = None,
        None => {
            if open {
                app.dialogs.smart_edit = Some((id, parent, name, rule));
            } else {
                app.dialogs.smart_edit = None;
            }
        }
    }
}

fn export_window(app: &mut App, ctx: &Context) {
    if !app.dialogs.export_open {
        return;
    }
    let mut open = true;
    egui::Window::new("Export to device").open(&mut open).default_width(560.0).collapsible(false).show(ctx, |ui| {
        if let Some(job) = &app.export_job {
            let (d, t, txt) = job.last.clone();
            ui.label(RichText::new("Write to the device").strong());
            ui.add(egui::ProgressBar::new(if t > 0 { d as f32 / t as f32 } else { 0.0 }).text(format!("{d} / {t}")));
            ui.label(RichText::new(txt).size(11.0).color(DIM));
            if ui.button("Stop").clicked() {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            ui.ctx().request_repaint();
            return;
        }
        ui.horizontal(|ui| {
            ui.label("Device");
            let cur = app.dialogs.export_root.clone().unwrap_or_default();
            let label = app.devices.iter().find(|d| d.root == cur).map(|d| format!("{} ({})", d.name, App::fs_label(&d.fs))).unwrap_or("Select a device".into());
            egui::ComboBox::from_id_salt("exdev").width(300.0).selected_text(label).show_ui(ui, |ui| {
                for d in &app.devices {
                    if ui.selectable_label(cur == d.root, format!("{} ({})", d.name, App::fs_label(&d.fs))).clicked() {
                        app.dialogs.export_root = Some(d.root.clone());
                    }
                }
            });
            if ui.button("Refresh").clicked() {
                app.scan_devices();
            }
        });
        if app.devices.is_empty() {
            ui.label(RichText::new("No USB drive or SD card found. Connect one, or add a folder as a device in the Devices view.").color(CAUTION_TX));
        }
        if let Some(d) = app.devices.iter().find(|d| Some(&d.root) == app.dialogs.export_root.as_ref()) {
            if let (Some(f), Some(t)) = (d.free, d.total) {
                ui.label(RichText::new(format!("Available Space {:.1} GB of {:.1} GB", f as f64 / 1e9, t as f64 / 1e9)).size(11.0).color(DIM));
            }
            if d.fs == "apfs" {
                ui.label(RichText::new("This drive is formatted as APFS, which DJ players cannot read. Use FAT32 or exFAT.").color(CAUTION_TX));
            }
            if let Some(l) = &d.lib {
                ui.label(RichText::new(format!("Already on the device: {} tracks, {} playlists. New items are added; existing ones are updated.", l.tracks.len(), l.nodes.iter().filter(|n| !n.folder).count())).size(11.0).color(DIM));
            }
            if let Some(e) = &d.error {
                ui.label(RichText::new(e).color(CAUTION_TX));
            }
        }
        ui.separator();
        ui.label(RichText::new("Playlists to export").strong());
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            fn rows(app: &mut App, ui: &mut Ui, parent: i64, depth: usize) {
                let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == parent).cloned().collect();
                for n in kids {
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 16.0);
                        let mut on = app.dialogs.export_sel.contains(&n.id);
                        let count = match n.kind {
                            NodeKind::Folder => String::new(),
                            NodeKind::Smart => format!(" ({})", n.smart.as_ref().map(|r| app.smart_tracks(r).len()).unwrap_or(0)),
                            NodeKind::Playlist => format!(" ({})", app.node_counts.get(&n.id).copied().unwrap_or(0)),
                        };
                        let icon = match n.kind {
                            NodeKind::Folder => "▶",
                            NodeKind::Smart => "⚙",
                            NodeKind::Playlist => "☰",
                        };
                        if ui.checkbox(&mut on, format!("{icon} {}{count}", n.name)).changed() {
                            if on {
                                app.dialogs.export_sel.insert(n.id);
                            } else {
                                app.dialogs.export_sel.remove(&n.id);
                            }
                        }
                    });
                    if n.kind == NodeKind::Folder {
                        rows(app, ui, n.id, depth + 1);
                    }
                }
            }
            rows(app, ui, 0, 0);
            if app.nodes.is_empty() {
                ui.label(RichText::new("No playlists yet. Create playlists, or export selected tracks.").color(DIM));
            }
        });
        if !app.dialogs.export_tracks.is_empty() {
            ui.label(format!("Plus {} selected track(s) (added to the device's track list)", app.dialogs.export_tracks.len()));
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Music files are copied to the device; your originals are only read. Tracks that are not analysed yet are analysed first.").size(10.5).color(DIM));
        ui.horizontal(|ui| {
            let can = app.dialogs.export_root.is_some() && (!app.dialogs.export_sel.is_empty() || !app.dialogs.export_tracks.is_empty());
            if ui.add_enabled(can, egui::Button::new(RichText::new("Export").strong()).fill(BLUE)).clicked() {
                let root = app.dialogs.export_root.clone().unwrap();
                let sel: Vec<i64> = app.dialogs.export_sel.iter().copied().collect();
                let tracks = app.dialogs.export_tracks.clone();
                app.start_export(&root, &sel, &tracks);
            }
            if ui.button("Close").clicked() {
                app.dialogs.export_open = false;
            }
        });
    });
    if !open {
        app.dialogs.export_open = false;
    }
}

fn confirm_windows(app: &mut App, ctx: &Context) {
    if let Some(ids) = app.dialogs.confirm_remove.clone() {
        egui::Window::new("Remove from Collection").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("Remove {} track(s) from the collection?", ids.len()));
            ui.label(RichText::new("Their cues, grids and tags in this library are removed. The music files themselves are not deleted.").size(11.0).color(DIM));
            ui.horizontal(|ui| {
                if ui.button("Remove").clicked() {
                    app.remove_from_collection(&ids);
                    app.dialogs.confirm_remove = None;
                }
                if ui.button("Cancel").clicked() {
                    app.dialogs.confirm_remove = None;
                }
            });
        });
    }
    if let Some((root, id, name)) = app.dialogs.device_remove.clone() {
        egui::Window::new("Delete from device").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("Delete \"{name}\" from the device?"));
            let del = app.prefs.delete_device_tracks;
            ui.label(RichText::new(if del { "Its tracks that are in no other playlist are also deleted from the device." } else { "Tracks stay on the device (change this in Preferences > DJ System)." }).size(11.0).color(DIM));
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    match crate::export::usb::remove_node(std::path::Path::new(&root), id, del) {
                        Ok(_) => app.toast(format!("Deleted \"{name}\" from the device")),
                        Err(e) => app.toast_err(format!("Could not change the device: {e}")),
                    }
                    app.dialogs.device_remove = None;
                    app.scan_devices();
                    app.set_view(View::Device { root: root.clone(), node: None });
                }
                if ui.button("Cancel").clicked() {
                    app.dialogs.device_remove = None;
                }
            });
        });
    }
    if app.dialogs.about {
        let mut open = true;
        egui::Window::new("About PrepDeck").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(RichText::new("PrepDeck").size(18.0).strong());
            ui.label("Library management, track analysis, set preparation and USB export for CDJ/XDJ players.");
            ui.label(RichText::new("Everything stays on this Mac. Your music files and any existing rekordbox library are never modified.").size(11.0).color(DIM));
        });
        app.dialogs.about = open;
    }
}
