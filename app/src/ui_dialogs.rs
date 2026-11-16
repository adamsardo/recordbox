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

fn prefs_window(app: &mut App, ctx: &Context) {
    if !app.dialogs.prefs_open {
        return;
    }
    let mut open = true;
    egui::Window::new("Preferences").open(&mut open).default_size(vec2(760.0, 520.0)).collapsible(false).show(ctx, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(140.0);
                for (i, n) in ["View", "Analysis", "DJ System", "Advanced"].iter().enumerate() {
                    let sel = app.dialogs.prefs_tab == i as u8;
                    let b = ui.add_sized(vec2(136.0, 26.0), egui::Button::new(RichText::new(*n).size(13.0)).fill(if sel { BLUE } else { BG2 }));
                    if b.clicked() {
                        app.dialogs.prefs_tab = i as u8;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                egui::ScrollArea::vertical().max_height(470.0).show(ui, |ui| {
                    let before = format!("{:?}", app.prefs);
                    match app.dialogs.prefs_tab {
                        0 => {
                            section(ui, "Key display format", |ui| {
                                ui.horizontal(|ui| {
                                    if ui.radio(!app.prefs.key_alnum, "Classic").clicked() {
                                        app.prefs.key_alnum = false;
                                        app.list_dirty = true;
                                    }
                                    if ui.radio(app.prefs.key_alnum, "Alphanumeric").clicked() {
                                        app.prefs.key_alnum = true;
                                        app.list_dirty = true;
                                    }
                                });
                            });
                            section(ui, "Tooltips", |ui| {
                                ui.checkbox(&mut app.prefs.tooltips, "Show Tooltips");
                            });
                            section(ui, "Waveform color", |ui| {
                                ui.horizontal(|ui| {
                                    for (i, n) in ["BLUE", "RGB", "3Band"].iter().enumerate() {
                                        if ui.radio(app.prefs.wave_color == i as u8, *n).clicked() {
                                            app.prefs.wave_color = i as u8;
                                        }
                                    }
                                });
                            });
                            section(ui, "Player", |ui| {
                                ui.checkbox(&mut app.prefs.time_remain, "Time Mode: Remaining (off: Elapsed)");
                                ui.checkbox(&mut app.prefs.quantize, "Quantize (cues and loops snap to the beat grid)");
                            });
                        }
                        1 => {
                            section(ui, "Track Analysis Mode", |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Analysis Mode");
                                    ui.add_enabled(false, egui::Button::new("Normal")).on_disabled_hover_text("Optimized for tracks which have a constant BPM");
                                });
                                ui.horizontal(|ui| {
                                    ui.label("BPM Range");
                                    let (lo, hi) = BPM_RANGES[app.prefs.bpm_range.min(9)];
                                    egui::ComboBox::from_id_salt("bpmrange").selected_text(format!("{lo:.0}-{hi:.0}")).show_ui(ui, |ui| {
                                        for (i, (a, b)) in BPM_RANGES.iter().enumerate() {
                                            ui.selectable_value(&mut app.prefs.bpm_range, i, format!("{a:.0}-{b:.0}"));
                                        }
                                    });
                                });
                            });
                            section(ui, "Track Analysis Setting", |ui| {
                                ui.checkbox(&mut app.prefs.analyze_grid, "BPM / Grid");
                                ui.checkbox(&mut app.prefs.analyze_key, "KEY");
                                ui.label(RichText::new("If tracks are not analyzed, BPM/Grid analysis starts when importing, loading, or exporting the tracks.").size(11.0).color(DIM));
                            });
                            section(ui, "Auto Analysis", |ui| {
                                ui.checkbox(&mut app.prefs.auto_analysis, "Enable");
                            });
                            section(ui, "Auto Gain", |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Reference loudness");
                                    ui.add(egui::Slider::new(&mut app.prefs.target_lufs, -20.0..=-5.0).suffix(" LUFS"));
                                });
                                ui.label(RichText::new("Used for the player's gain and stored per track when analysing.").size(11.0).color(DIM));
                            });
                            if ui.button("Re-analyze all tracks now").clicked() {
                                let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
                                app.reanalyze(&ids);
                            }
                        }
                        2 => {
                            section(ui, "Device", |ui| {
                                ui.checkbox(&mut app.prefs.delete_device_tracks, "Delete playlist tracks from device if playlist is deleted");
                            });
                            section(ui, "USB export format", |ui| {
                                ui.label("Exports write the standard device library (export.pdb + exportExt.pdb) with analysis files (DAT / EXT / 2EX) and copies of the music in /Contents.");
                                ui.label(RichText::new("Supported drive formats: FAT32, exFAT (some players), HFS+ (USB only). Supported files: MP3, AAC/M4A, WAV, AIFF, FLAC.").size(11.0).color(DIM));
                            });
                        }
                        _ => {
                            section(ui, "Library", |ui| {
                                ui.label(format!("Library location: {}", crate::db::data_dir().display()));
                                ui.label(RichText::new("Your music files are only read. Edits (tags, cues, grids) are kept in this private library.").size(11.0).color(DIM));
                                if ui.button("Show in Finder").clicked() {
                                    let _ = std::process::Command::new("/usr/bin/open").arg(crate::db::data_dir()).spawn();
                                }
                            });
                            section(ui, "rekordbox xml", |ui| {
                                ui.label("Import a collection exported from rekordbox (File > Export Collection in xml format). The rekordbox library itself is never opened or changed.");
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
                    ui.add_space(6.0);
                    if ui.button("Reset to defaults").clicked() {
                        let extra = app.prefs.devices_extra.clone();
                        app.prefs = Prefs::default();
                        app.prefs.devices_extra = extra;
                        app.save_prefs();
                        app.list_dirty = true;
                    }
                });
            });
        });
    });
    app.dialogs.prefs_open = open;
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
                            NodeKind::Folder => "🗀",
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
