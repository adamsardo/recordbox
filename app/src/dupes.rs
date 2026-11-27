//! Duplicated File Manager (File > Display All Duplicated Files), as rekordbox 7.2.19 has it.
//!
//! Traced from rekordbox (notes in `checks/re-7.2.19/shell-menus-traffic.md`):
//! - the search (`SearchDuplicatedFile::run` 0x100218fb0): every collection track, sorted by
//!   title and then artist; tracks next to each other with the same title (not empty) and the
//!   same artist are duplicates. The comparison is exact (case and spaces count).
//! - the flow (`DuplicatedFileDialog::show` 0x100212cfc): none found gives "No duplicate track
//!   found."; else "Duplicated tracks are found." asks whether to display them.
//! - the window (`DuplicatedFileDialog` ctor 0x100211e0c, `DuplicatedFileComponent::resized`
//!   0x1002133b4): "Duplicated File Manager", 830 x 600, resizable; the list 20 pt in with 100
//!   pt left under it, the count 70 pt above the bottom, Delete (80 x 32) and Merge Duplicate
//!   Files (200 x 32) at the left and OK (80 x 32) at the right, 42 pt above the bottom. Delete
//!   needs a selection, Merge more than one row.
//! - merging (`DuplicatedFileMergeDialog` ctor 0x101afb070, `merge` 0x101afc088): "Select
//!   Representative File" with the chosen tracks; the representative takes the others' places
//!   in playlists and the others leave the collection.
//!
//! - the list (`DuplicatedFileDialog` ctor): every column hidden, then ids 73, 74, 21, 22, 38,
//!   68, 60 shown. Through `BrowseHeaderManager::getColumnIndexByID` 0x100d7cf98 (table at
//!   0x1037B6D8C) and `browse::kColumnStrSet` these are Attribute, Cloud, Track Title, Artist,
//!   Location, Preview and Artwork; recordbox's Status column is rekordbox's Attribute + Cloud
//!   cells. A last unnamed 20 pt column (id 0x10000000, `ListViewer::paintCell` 0x100213aa0)
//!   shows `brws_list_pathsystem_pulldown.svg` (grey up / down chevrons, 10 x 16, centred) when
//!   the track is in a playlist or history; a click on it (`cellClicked` 0x100213c34) drops a
//!   list of where the track is, written "Playlists / <folder> / <playlist>" or "Histories /
//!   <history>" (`browse::updatePathSystem` 0x100b6fc28), at the click. Delete is enabled with
//!   a selection, Merge with two or more rows (`onSelectedRowsChanged` 0x100213668).
//! - merging (`DuplicatedFileMergeDialog::merge` 0x101afc088, `MergeFile::getPlaylistTree`
//!   0x101b02a80): for each other track of the group, every playlist it is in that the
//!   representative is not in gets the representative added (rekordbox's add-to-playlist, so at
//!   the end), then the others leave the collection (and so their playlists).
//!
//! Not traced: the default order of the seven columns (the header's saved layout decides; here
//! Status, Preview, Artwork, Track Title, Artist, Location), and the second icon of the last
//! column (`extension.svg` at half opacity, for a row flag that was not identified).

use crate::app::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_kit as kit;
use eframe::egui;
use egui::{pos2, vec2, Align2, Rect, Sense};
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub struct Dupes {
    /// 1: "Duplicated tracks are found." is asking; 2: "No duplicate track found." is showing.
    pub prompt: u8,
    /// Result of the last search: the duplicated tracks in list order with their group.
    pub rows: Vec<(i64, usize)>,
    pub open: bool,
    pub sel: BTreeSet<i64>,
    anchor: Option<usize>,
    confirm_delete: bool,
    /// "Select Representative File": the tracks to merge and the chosen representative.
    merge: Option<(Vec<i64>, Option<i64>)>,
    merged_msg: bool,
    /// Where each listed track is ("Playlists / ..."), read when the list is made.
    pub paths: HashMap<i64, Vec<String>>,
    /// The path list dropped from a row's last cell: the track and where it opens.
    pub path_popup: Option<(i64, egui::Pos2)>,
}

/// Groups of duplicated tracks: same title (not empty) and same artist, in the order of
/// rekordbox's search (sorted by title, then artist; tracks that compare equal keep their
/// collection order).
pub fn find_groups(tracks: &[Track]) -> Vec<Vec<i64>> {
    let mut v: Vec<&Track> = tracks.iter().collect();
    v.sort_by(|a, b| a.title.as_str().cmp(b.title.as_str()).then_with(|| a.artist.as_str().cmp(b.artist.as_str())));
    let mut out: Vec<Vec<i64>> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i + 1;
        while j < v.len() && !v[i].title.is_empty() && v[j].title == v[i].title && v[j].artist == v[i].artist {
            j += 1;
        }
        if j - i > 1 {
            out.push(v[i..j].iter().map(|t| t.id).collect());
        }
        i = j;
    }
    out
}

/// A playlist's tracks after a merge: when it had one of the merged tracks and not the
/// representative, the representative is added at its end; the merged tracks are gone (they
/// leave the collection).
pub fn merged_playlist(tracks: &[i64], keep: i64, others: &[i64]) -> Vec<i64> {
    let had_other = tracks.iter().any(|t| others.contains(t));
    let mut out: Vec<i64> = tracks.iter().copied().filter(|t| !others.contains(t)).collect();
    if had_other && !tracks.contains(&keep) {
        out.push(keep);
    }
    out
}

impl App {
    /// File > Display All Duplicated Files.
    pub fn dupes_search(&mut self) {
        let groups = find_groups(&self.tracks);
        self.dupes.rows = groups.iter().enumerate().flat_map(|(g, ids)| ids.iter().map(move |id| (*id, g))).collect();
        self.dupes.sel.clear();
        self.dupes.anchor = None;
        self.dupes.open = false;
        self.dupes.prompt = if groups.is_empty() { 2 } else { 1 };
        self.dupes_read_paths();
    }

    /// Read where each listed track is (rekordbox's `updatePathSystem` while searching).
    pub fn dupes_read_paths(&mut self) {
        let ids: Vec<i64> = self.dupes.rows.iter().map(|(id, _)| *id).collect();
        self.dupes.paths = ids.into_iter().map(|id| (id, self.dupes_playlists_of(id))).collect();
        self.dupes.path_popup = None;
    }

    /// Where a track is, as rekordbox's path list writes it: "Playlists / <folders> / <name>"
    /// for each playlist, "Histories / <name>" for each history.
    pub fn dupes_playlists_of(&self, id: i64) -> Vec<String> {
        let mut out = vec![];
        for n in self.nodes.iter().filter(|n| n.kind == NodeKind::Playlist) {
            if !self.db.playlist_tracks(n.id).map(|t| t.contains(&id)).unwrap_or(false) {
                continue;
            }
            let mut parts = vec![n.name.clone()];
            let mut parent = n.parent;
            while let Some(f) = self.nodes.iter().find(|x| x.id == parent && parent != 0) {
                parts.push(f.name.clone());
                parent = f.parent;
            }
            parts.push("Playlists".into());
            parts.reverse();
            out.push(parts.join(" / "));
        }
        for h in self.db.histories().unwrap_or_default() {
            if self.db.history_tracks(h.id).map(|t| t.contains(&id)).unwrap_or(false) {
                out.push(format!("Histories / {}", h.name));
            }
        }
        out
    }

    /// Merge `others` into `keep`: playlists first, then the others leave the collection.
    pub fn dupes_merge(&mut self, keep: i64, others: &[i64]) {
        let lists: Vec<i64> = self.nodes.iter().filter(|n| n.kind == NodeKind::Playlist).map(|n| n.id).collect();
        for pl in lists {
            let Ok(tracks) = self.db.playlist_tracks(pl) else { continue };
            if tracks.iter().any(|t| others.contains(t)) {
                let _ = self.db.set_playlist_tracks(pl, &merged_playlist(&tracks, keep, others));
            }
        }
        self.remove_from_collection(others);
        self.reload_playlists();
        self.list_dirty = true;
        // What is left of the search result: groups that still have two tracks.
        let gone: BTreeSet<i64> = others.iter().copied().collect();
        self.dupes_drop(&gone);
    }

    /// Take removed tracks out of the shown result (a group left with one track goes too).
    fn dupes_drop(&mut self, gone: &BTreeSet<i64>) {
        self.dupes.rows.retain(|(id, _)| !gone.contains(id));
        let mut count: HashMap<usize, usize> = HashMap::new();
        for (_, g) in &self.dupes.rows {
            *count.entry(*g).or_default() += 1;
        }
        self.dupes.rows.retain(|(_, g)| count.get(g).copied().unwrap_or(0) > 1);
        let left: BTreeSet<i64> = self.dupes.rows.iter().map(|(id, _)| *id).collect();
        self.dupes.sel.retain(|id| left.contains(id));
        self.dupes.anchor = None;
    }
}

/// The manager's columns (see the notes at the top) and the 20 pt path column after them.
const COLS: [(Col, f32); 6] = [(Col::Status, 90.0), (Col::Preview, 110.0), (Col::Artwork, -1.0), (Col::Title, 190.0), (Col::Artist, 140.0), (Col::Location, 0.0)];

/// A column's width: its own, or (-1) the collection list's width for it.
fn col_w(app: &App, c: (Col, f32)) -> f32 {
    if c.1 < 0.0 { crate::ui_browser::col_width(app, c.0) } else { c.1 }
}
const PATH_W: f32 = 20.0;

/// rekordbox's `brws_list_pathsystem_pulldown.svg`: two grey chevrons, 10 x 16 pt.
fn path_icon(p: &egui::Painter, c: egui::Pos2) {
    let col = hex(0x646464);
    let o = c - vec2(5.0, 8.0);
    let up = vec![o + vec2(5.092, 0.0), o + vec2(10.0, 5.998), o + vec2(7.817, 5.998), o + vec2(5.089, 2.79), o + vec2(2.367, 5.998), o + vec2(0.0, 5.998)];
    let down = vec![o + vec2(4.908, 16.0), o + vec2(0.0, 10.001), o + vec2(2.183, 10.001), o + vec2(4.911, 13.21), o + vec2(7.633, 10.001), o + vec2(10.0, 10.001)];
    for pts in [up, down] {
        p.add(egui::Shape::Path(egui::epaint::PathShape { points: pts, closed: true, fill: col, stroke: egui::epaint::PathStroke::NONE }));
    }
}

/// A list of tracks in the manager's look. `pick_one`: a click chooses one row (the
/// representative); else rows are selected with click, Cmd-click and Shift-click.
fn track_table(app: &mut App, ui: &mut egui::Ui, table: Rect, ids: &[i64], salt: &str, pick_one: Option<&mut Option<i64>>) {
    let p = ui.painter().clone();
    p.rect_filled(table, 0.0, BG);
    let font = rb(11.5);
    let tracks: Vec<Track> = ids.iter().filter_map(|id| app.track(*id).cloned()).collect();
    let paths: Vec<Vec<String>> = tracks.iter().map(|t| app.dupes.paths.get(&t.id).cloned().unwrap_or_default()).collect();
    let loc_w = tracks.iter().map(|t| p.layout_no_wrap(t.path.clone(), font.clone(), WHITE).size().x).fold(0.0, f32::max);
    let cols: Vec<(Col, f32)> = COLS.iter().map(|c| (c.0, col_w(app, *c))).collect();
    let fixed: f32 = cols.iter().map(|c| c.1).sum::<f32>() + PATH_W;
    // (Location takes the width that is left; a long path is cut, as in rekordbox's list.)
    let _ = loc_w;
    let content_w = (fixed + 120.0).max(table.width());
    let loc = content_w - fixed;
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(table).id_salt(salt));
    tui.set_clip_rect(table);
    crate::ui_browser_ext::bar_style(&mut tui);
    tui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
    let mut pick_one = pick_one;
    let mut open_paths: Option<(i64, egui::Pos2)> = None;
    let row_h = crate::ui_browser::row_height(app);
    egui::ScrollArea::horizontal().id_salt((salt, "h")).auto_shrink([false, false]).show(&mut tui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (hr, _) = ui.allocate_exact_size(vec2(content_w, 22.0), Sense::hover());
        ui.painter().rect_filled(hr, 0.0, hex(0x1A1A1A));
        let mut x = hr.min.x;
        for (i, (c, cw)) in cols.iter().enumerate() {
            if i > 0 {
                ui.painter().rect_filled(Rect::from_min_max(pos2(x - 1.0, hr.min.y), pos2(x, hr.max.y)), 0.0, BG);
            }
            let label = if *c == Col::Status { "" } else { c.label() };
            ui.painter().with_clip_rect(Rect::from_min_size(pos2(x, hr.min.y), vec2(if *cw > 0.0 { *cw } else { loc }, hr.height()))).text(pos2(x + 6.0, hr.center().y), Align2::LEFT_CENTER, label, font.clone(), DIM);
            x += if *cw > 0.0 { *cw } else { loc };
        }
        ui.painter().rect_filled(Rect::from_min_max(pos2(x - 1.0, hr.min.y), pos2(x, hr.max.y)), 0.0, BG);
        egui::ScrollArea::vertical().id_salt((salt, "v")).auto_shrink([false, false]).show_rows(ui, row_h, tracks.len(), |ui, range| {
            for i in range {
                let t = &tracks[i];
                let (rr, resp) = ui.allocate_exact_size(vec2(content_w, row_h), Sense::click());
                let on = match &pick_one {
                    Some(one) => **one == Some(t.id),
                    None => app.dupes.sel.contains(&t.id),
                };
                if on {
                    ui.painter().rect_filled(rr, 0.0, ROW_SEL);
                } else if i % 2 == 1 {
                    ui.painter().rect_filled(rr, 0.0, hex(0x0A0A0A));
                }
                let mut x = rr.min.x;
                for (c, cw) in cols.iter() {
                    let w = if *cw > 0.0 { *cw } else { loc };
                    let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(w, rr.height())).intersect(ui.clip_rect());
                    if cr.width() > 0.0 {
                        crate::ui_browser::draw_cell(app, ui, *c, t, cr, i);
                    }
                    x += w;
                }
                // The path column: the chevrons when the track is in a playlist or history; a
                // click drops the list of where it is.
                let pr = Rect::from_min_size(pos2(x, rr.min.y), vec2(PATH_W, rr.height()));
                mark(format!("{salt}_row_{i}"), Rect::from_min_size(pos2(rr.min.x + 300.0, rr.min.y), vec2(200.0, rr.height())).intersect(table));
                mark(format!("{salt}_path_{i}"), pr.intersect(table));
                let mut path_click = false;
                if !paths[i].is_empty() {
                    path_icon(&ui.painter().with_clip_rect(pr.intersect(ui.clip_rect())), pr.center());
                    path_click = resp.clicked() && resp.interact_pointer_pos().map(|pp| pr.contains(pp)).unwrap_or(false);
                }
                if path_click && pick_one.is_none() {
                    open_paths = Some((t.id, resp.interact_pointer_pos().unwrap_or(pr.center())));
                }
                if resp.clicked() {
                    match &mut pick_one {
                        Some(one) => **one = Some(t.id),
                        None => {
                            if shift && app.dupes.anchor.is_some() {
                                let a = app.dupes.anchor.unwrap_or(i);
                                let (lo, hi) = (a.min(i), a.max(i));
                                if !cmd {
                                    app.dupes.sel.clear();
                                }
                                for k in lo..=hi {
                                    app.dupes.sel.insert(tracks[k].id);
                                }
                            } else if cmd {
                                if !app.dupes.sel.remove(&t.id) {
                                    app.dupes.sel.insert(t.id);
                                }
                                app.dupes.anchor = Some(i);
                            } else {
                                app.dupes.sel.clear();
                                app.dupes.sel.insert(t.id);
                                app.dupes.anchor = Some(i);
                            }
                        }
                    }
                }
            }
        });
    });
    if let Some(o) = open_paths {
        app.dupes.path_popup = Some(o);
    }
    // The dropped list of paths (rekordbox's `PathSystemComboBox` popup).
    if let Some((id, at)) = app.dupes.path_popup.clone() {
        let list = app.dupes.paths.get(&id).cloned().unwrap_or_default();
        let mut close = false;
        let area = egui::Area::new(egui::Id::new((salt, "paths"))).order(egui::Order::Tooltip).fixed_pos(at).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).fill(hex(0x202020)).show(ui, |ui| {
                ui.set_min_width(200.0);
                for l in &list {
                    if ui.add(egui::Label::new(egui::RichText::new(l).font(rb(11.5)).color(WHITE)).sense(Sense::click())).clicked() {
                        close = true;
                    }
                }
            });
        });
        if close || (ui.input(|i| i.pointer.any_pressed()) && !area.response.rect.contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default())) && open_paths.is_none()) {
            app.dupes.path_popup = None;
        }
    }
}

/// Where the manager's rows and buttons were drawn last (the scripted run clicks them).
static RECTS: std::sync::Mutex<Vec<(String, Rect)>> = std::sync::Mutex::new(Vec::new());

fn mark(key: String, r: Rect) {
    if let Ok(mut v) = RECTS.lock() {
        v.retain(|(k, _)| *k != key);
        v.push((key, r));
    }
}

fn rect_of(key: &str) -> Option<Rect> {
    RECTS.lock().ok().and_then(|v| v.iter().find(|(k, _)| k == key).map(|x| x.1))
}

/// `kit::dialog_buttons`, also noting where each button is.
fn dlg_buttons(ui: &mut egui::Ui, key: &str, labels: &[&str]) -> Option<usize> {
    let mut hit = None;
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let w: f32 = labels.len() as f32 * 96.0 + (labels.len().saturating_sub(1)) as f32 * 8.0;
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        for (i, l) in labels.iter().enumerate() {
            let r = ui.add(kit::btn(*l).min_size(vec2(96.0, 24.0)));
            mark(format!("{key}_{l}"), r.rect);
            if r.clicked() {
                hit = Some(i);
            }
        }
    });
    hit
}

/// A dialog button of the manager (rekordbox's sizes; the look of the Missing File Manager's).
fn button(ui: &mut egui::Ui, r: Rect, id: &str, label: &str, enabled: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(id), if enabled { Sense::click() } else { Sense::hover() });
    mark(id.to_string(), r);
    let p = ui.painter();
    p.rect_filled(r, 2.0, hex(0x141414));
    p.rect_filled(r.shrink(1.0), 1.5, if enabled && resp.is_pointer_button_down_on() { hex(0x2E2E2E) } else if enabled && resp.hovered() { hex(0x4A4A4A) } else { hex(0x3C3C3C) });
    p.text(r.center(), Align2::CENTER_CENTER, label, rb(13.0), if enabled { hex(0xC8C8C8) } else { hex(0x787878) });
    resp.clicked()
}

/// The manager's windows and message boxes. Called every frame.
pub fn windows(app: &mut App, ctx: &egui::Context) {
    let center = Align2::CENTER_CENTER;
    match app.dupes.prompt {
        1 => {
            let mut answer = None;
            kit::dialog(ctx, "Duplicated tracks are found.").resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
                kit::apply_style(ui);
                ui.set_width(400.0);
                kit::message(ui, "Check the database and display duplicate tracks?\nThis operation can take few minutes.");
                answer = dlg_buttons(ui, "prompt", &["OK", "Cancel"]);
            });
            if let Some(a) = answer {
                app.dupes.prompt = 0;
                app.dupes.open = a == 0;
                if a != 0 {
                    app.dupes.rows.clear();
                }
            }
        }
        2 => {
            let mut close = false;
            kit::dialog(ctx, "Duplicated Files").resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
                kit::apply_style(ui);
                ui.set_width(320.0);
                kit::message(ui, "No duplicate track found.");
                close = kit::dialog_buttons(ui, &["Yes"]).is_some();
            });
            if close {
                app.dupes.prompt = 0;
            }
        }
        _ => {}
    }
    if !app.dupes.open {
        return;
    }
    let mut open = true;
    let ids: Vec<i64> = app.dupes.rows.iter().map(|(id, _)| *id).collect();
    let modal = app.dupes.confirm_delete || app.dupes.merge.is_some() || app.dupes.merged_msg;
    kit::dialog(ctx, "Duplicated File Manager").id(egui::Id::new("dupes_manager")).open(&mut open).resizable(true).default_pos(ctx.screen_rect().center() - vec2(415.0, 315.0)).default_size([830.0, 600.0]).min_size([520.0, 320.0]).frame(egui::Frame::window(&ctx.style()).fill(hex(0x191919)).inner_margin(egui::Margin::ZERO)).show(ctx, |ui| {
        let size = ui.available_size().max(vec2(520.0, 320.0));
        let (r, _) = ui.allocate_exact_size(size, Sense::hover());
        let (w, h) = (r.width(), r.height());
        let p = ui.painter().clone();
        p.rect_filled(r, 0.0, hex(0x191919));
        let list = Rect::from_min_size(pos2(r.min.x + 20.0, r.min.y + 20.0), vec2(w - 40.0, h - 100.0));
        if !modal {
            track_table(app, ui, list, &ids, "dupes_table", None);
        } else {
            p.rect_filled(list, 0.0, BG);
        }
        // "<n> Track", 16 pt.
        p.text(pos2(r.min.x + 20.0, r.max.y - 70.0 + 9.0), Align2::LEFT_CENTER, format!("{} Track", ids.len()), rb(16.0), WHITE);
        let y = r.max.y - 42.0;
        let n = app.dupes.sel.len();
        if button(ui, Rect::from_min_size(pos2(r.min.x + 20.0, y), vec2(80.0, 32.0)), "dupes_delete", "Delete", n > 0 && !modal) {
            app.dupes.confirm_delete = true;
        }
        if button(ui, Rect::from_min_size(pos2(r.min.x + 120.0, y), vec2(200.0, 32.0)), "dupes_merge", "Merge Duplicate Files", n > 1 && !modal) {
            let chosen: Vec<i64> = ids.iter().copied().filter(|id| app.dupes.sel.contains(id)).collect();
            app.dupes.merge = Some((chosen, None));
        }
        if button(ui, Rect::from_min_size(pos2(r.max.x - 100.0, y), vec2(80.0, 32.0)), "dupes_ok", "OK", !modal) {
            app.dupes.open = false;
        }
        // Delete key: the same as the Delete button.
        if !modal && n > 0 && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) && ui.memory(|m| m.focused().is_none()) {
            app.dupes.confirm_delete = true;
        }
    });
    if !open {
        app.dupes.open = false;
    }
    if app.dupes.confirm_delete {
        let mut answer = None;
        kit::dialog(ctx, "Remove").id(egui::Id::new("dupes_remove")).resizable(false).anchor(center, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(400.0);
            kit::message(ui, "Are you sure you want to remove the selected tracks?");
            kit::message_note(ui, "The music files are not deleted. For 30 days the tracks can be brought back with Track > Restore Removed Tracks.");
            answer = dlg_buttons(ui, "remove", &["OK", "Cancel"]);
        });
        if let Some(a) = answer {
            app.dupes.confirm_delete = false;
            if a == 0 {
                let gone = app.dupes.sel.clone();
                let ids: Vec<i64> = gone.iter().copied().collect();
                app.remove_from_collection(&ids);
                app.dupes_drop(&gone);
            }
        }
    }
    if let Some((chosen, mut keep)) = app.dupes.merge.clone() {
        let mut answer = None;
        kit::dialog(ctx, "Select Representative File").id(egui::Id::new("dupes_merge_window")).resizable(true).default_pos(ctx.screen_rect().center() - vec2(415.0, 315.0)).default_size([830.0, 600.0]).min_size([520.0, 320.0]).order(egui::Order::Foreground).frame(egui::Frame::window(&ctx.style()).fill(hex(0x191919)).inner_margin(egui::Margin::ZERO)).show(ctx, |ui| {
            let size = ui.available_size().max(vec2(520.0, 320.0));
            let (r, _) = ui.allocate_exact_size(size, Sense::hover());
            let (w, h) = (r.width(), r.height());
            let p = ui.painter().clone();
            p.rect_filled(r, 0.0, hex(0x191919));
            let tr = Rect::from_min_size(pos2(r.min.x + 20.0, r.min.y + 20.0), vec2(w - 40.0, 40.0));
            // Two centred lines (rekordbox: a centred 16 pt label 40 pt tall).
            p.text(pos2(tr.center().x, tr.min.y + 10.0), Align2::CENTER_CENTER, "Selected files will be merged.", rb(16.0), WHITE);
            p.text(pos2(tr.center().x, tr.min.y + 30.0), Align2::CENTER_CENTER, "Please select a representative file as the merge destination.", rb(16.0), WHITE);
            let table = Rect::from_min_size(pos2(r.min.x + 20.0, r.min.y + 80.0), vec2(w - 40.0, h - 160.0));
            track_table(app, ui, table, &chosen, "dupes_merge_table", Some(&mut keep));
            let cx = r.min.x + w / 2.0;
            if button(ui, Rect::from_min_size(pos2(cx - 90.0, r.max.y - 70.0), vec2(80.0, 32.0)), "dupes_merge_cancel", "Cancel", true) {
                answer = Some(false);
            }
            if button(ui, Rect::from_min_size(pos2(cx + 10.0, r.max.y - 70.0), vec2(80.0, 32.0)), "dupes_merge_go", "Merge", keep.is_some()) {
                answer = Some(true);
            }
        });
        match answer {
            Some(true) => {
                app.dupes.merge = None;
                if let Some(k) = keep {
                    let others: Vec<i64> = chosen.iter().copied().filter(|id| *id != k).collect();
                    app.dupes_merge(k, &others);
                    app.dupes.merged_msg = true;
                }
            }
            Some(false) => app.dupes.merge = None,
            None => app.dupes.merge = Some((chosen, keep)),
        }
    }
    if app.dupes.merged_msg {
        let mut close = false;
        kit::dialog(ctx, " ").id(egui::Id::new("dupes_merged")).resizable(false).anchor(center, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(320.0);
            kit::message(ui, "Selected files have been merged.");
            close = dlg_buttons(ui, "merged", &["OK"]).is_some();
        });
        if close {
            app.dupes.merged_msg = false;
        }
    }
}

/// Screenshot states: "dupes" (the manager with the library's duplicates, or with made-up
/// ones when it has none), "dupesmerge" (the representative window).
pub fn shot_setup(app: &mut App, name: &str) {
    app.dupes_search();
    if app.dupes.rows.is_empty() {
        let ids: Vec<i64> = app.tracks.iter().take(6).map(|t| t.id).collect();
        app.dupes.rows = ids.iter().enumerate().map(|(i, id)| (*id, i / 2)).collect();
    }
    app.dupes.prompt = 0;
    app.dupes.open = true;
    if let Some((id, _)) = app.dupes.rows.first().copied() {
        app.dupes.sel.insert(id);
    }
    if name == "dupesmerge" {
        let chosen: Vec<i64> = app.dupes.rows.iter().take(2).map(|(id, _)| *id).collect();
        app.dupes.merge = Some((chosen, None));
    }
}

/// `PREPDECK_SHOT_PLAYER=dupestest` (with --screenshot and `PREPDECK_BENCH=600`, on a
/// throwaway library copy): File > Display All Duplicated Files, then with real pointer
/// events: OK in the question, a row, its path list, Delete and OK, two rows of one group,
/// Merge Duplicate Files, the representative, Merge, OK. Each step's result goes to stderr as
/// "dupestest:" lines.
pub fn shot_events(app: &mut App, frame: u32, raw: &mut egui::RawInput) {
    if !std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "dupestest")).unwrap_or(false) {
        return;
    }
    const START: u32 = 150;
    const SPAN: u32 = 30;
    if frame < START {
        return;
    }
    let step = ((frame - START) / SPAN) as usize;
    let f = (frame - START) % SPAN;
    let state = |app: &App| {
        let groups: std::collections::BTreeSet<usize> = app.dupes.rows.iter().map(|(_, g)| *g).collect();
        format!("prompt {} open {} rows {} groups {} selected {:?} merge window {} merged message {} collection {} tracks", app.dupes.prompt, app.dupes.open, app.dupes.rows.len(), groups.len(), app.dupes.sel, app.dupes.merge.is_some(), app.dupes.merged_msg, app.tracks.len())
    };
    // The group to merge: the one with a made-up copy (it is in a playlist), else the first;
    // the group to delete from: another one.
    let merge_group = |app: &App| -> Option<usize> {
        app.dupes.rows.iter().find(|(id, _)| app.track(*id).map(|t| t.path.ends_with("(dupestest copy)")).unwrap_or(false)).or(app.dupes.rows.first()).map(|r| r.1)
    };
    let pair = |app: &App| -> Option<(usize, usize)> {
        let g = merge_group(app)?;
        let idx: Vec<usize> = app.dupes.rows.iter().enumerate().filter(|(_, r)| r.1 == g).map(|(i, _)| i).collect();
        Some((*idx.first()?, *idx.get(1)?))
    };
    let delete_row = |app: &App| -> Option<usize> {
        let g = merge_group(app);
        app.dupes.rows.iter().position(|r| Some(r.1) != g)
    };
    let first_with_paths = |app: &App| app.dupes.rows.iter().position(|(id, _)| app.dupes.paths.get(id).map(|p| !p.is_empty()).unwrap_or(false));
    let click = |raw: &mut egui::RawInput, r: Option<Rect>, mods: egui::Modifiers| {
        let Some(r) = r else { return };
        let at = r.center();
        match f {
            1 => raw.events.push(egui::Event::PointerMoved(at)),
            3 => {
                raw.modifiers = mods;
                raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: mods });
            }
            5 => {
                raw.modifiers = mods;
                raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: mods });
            }
            _ => {}
        }
    };
    let none = egui::Modifiers::default();
    let cmdm = egui::Modifiers { command: true, mac_cmd: true, ..Default::default() };
    static BEFORE: std::sync::Mutex<Option<(i64, Vec<i64>, Vec<(i64, Vec<i64>)>)>> = std::sync::Mutex::new(None);
    let name = match step {
        0 => {
            if f == 0 {
                // The throwaway library gets made-up duplicates first when it has fewer than
                // three groups: copies (another path) of tracks that are in a playlist, each
                // copy put into another playlist, so that the merge has playlists to rewrite.
                if find_groups(&app.tracks).len() < 3 {
                    let lists: Vec<i64> = app.nodes.iter().filter(|n| n.kind == NodeKind::Playlist && n.smart.is_none()).map(|n| n.id).collect();
                    let mut made = 0;
                    let picks: Vec<(usize, i64)> = lists.iter().enumerate().flat_map(|(k, pl)| app.db.playlist_tracks(*pl).unwrap_or_default().into_iter().take(2).map(move |t| (k, t))).take(2).collect();
                    for (k, tid) in picks {
                        let pl = &lists[k];
                        let Some(orig) = app.track(tid).cloned() else { continue };
                        let mut copy = orig.clone();
                        copy.id = 0;
                        copy.path = format!("{} (dupestest copy)", orig.path);
                        if let Ok(nid) = app.db.insert_track(&copy) {
                            if let Some(other) = lists.get(k + 1).or(lists.first()).filter(|o| **o != *pl) {
                                let mut cur = app.db.playlist_tracks(*other).unwrap_or_default();
                                cur.push(nid);
                                let _ = app.db.set_playlist_tracks(*other, &cur);
                            }
                            made += 1;
                        }
                    }
                    app.reload_tracks();
                    app.reload_playlists();
                    eprintln!("dupestest: made {made} duplicate(s) of playlist tracks in the throwaway library");
                }
                app.dupes_search();
            }
            "File > Display All Duplicated Files"
        }
        1 => {
            click(raw, rect_of("prompt_OK"), none);
            "OK in \"Duplicated tracks are found.\""
        }
        2 => {
            if let Some(i) = delete_row(app) {
                click(raw, rect_of(&format!("dupes_table_row_{i}")), none);
            }
            "click the first row of a group that is not merged later"
        }
        3 => {
            let i = first_with_paths(app);
            if let Some(i) = i {
                click(raw, rect_of(&format!("dupes_table_path_{i}")), none);
            }
            "click the path cell of the first row that is in a playlist or history"
        }
        4 => {
            if f == 1 {
                app.dupes.path_popup = None;
            }
            if let Some(i) = delete_row(app) {
                click(raw, rect_of(&format!("dupes_table_row_{i}")), none);
            }
            "click that row again (closes the path list)"
        }
        5 => {
            click(raw, rect_of("dupes_delete"), none);
            "Delete"
        }
        6 => {
            click(raw, rect_of("remove_OK"), none);
            "OK in \"Are you sure you want to remove the selected tracks?\""
        }
        7 => {
            if let Some((a, _)) = pair(app) {
                click(raw, rect_of(&format!("dupes_table_row_{a}")), none);
            }
            "click the first row of a group"
        }
        8 => {
            if let Some((_, b)) = pair(app) {
                click(raw, rect_of(&format!("dupes_table_row_{b}")), cmdm);
            }
            "Cmd-click the second row of that group"
        }
        9 => {
            if f == 0 {
                // What the merge should do: the representative (the first row) and the
                // playlists of both.
                if let Some((a, b)) = pair(app) {
                    let (keep, other) = (app.dupes.rows[a].0, app.dupes.rows[b].0);
                    let lists: Vec<(i64, Vec<i64>)> = app.nodes.iter().filter(|n| n.kind == NodeKind::Playlist).map(|n| (n.id, app.db.playlist_tracks(n.id).unwrap_or_default())).filter(|(_, t)| t.contains(&keep) || t.contains(&other)).collect();
                    *BEFORE.lock().unwrap() = Some((keep, vec![other], lists));
                }
            }
            click(raw, rect_of("dupes_merge"), none);
            "Merge Duplicate Files"
        }
        10 => {
            click(raw, rect_of("dupes_merge_table_row_0"), none);
            "click the representative (first row)"
        }
        11 => {
            click(raw, rect_of("dupes_merge_go"), none);
            "Merge"
        }
        12 => {
            click(raw, rect_of("merged_OK"), none);
            "OK in \"Selected files have been merged.\""
        }
        13 => {
            if f == 20 {
                let b = BEFORE.lock().unwrap().clone();
                match b {
                    Some((keep, others, lists)) => {
                        let mut ok = true;
                        for (pl, before) in &lists {
                            let after = app.db.playlist_tracks(*pl).unwrap_or_default();
                            let want = merged_playlist(before, keep, &others);
                            ok &= after == want;
                            eprintln!("dupestest: CHECK playlist {pl}: before {before:?} after {after:?} expected {want:?}");
                        }
                        let gone = others.iter().all(|o| app.track(*o).is_none());
                        let kept = app.track(keep).is_some();
                        eprintln!("dupestest: CHECK merged track left the collection {gone}, representative kept {kept}");
                        eprintln!("dupestest: CHECK {}", if ok && gone && kept { "PASS" } else { "FAIL" });
                    }
                    None => eprintln!("dupestest: CHECK no group of two was left to merge"),
                }
            }
            "check"
        }
        _ => return,
    };
    if f == 25 {
        let extra = if step == 3 { format!(" | path list {:?} = {:?}", app.dupes.path_popup.map(|p| p.0), app.dupes.path_popup.and_then(|p| app.dupes.paths.get(&p.0).cloned())) } else { String::new() };
        eprintln!("dupestest: {step}: {name}: {}{extra}", state(app));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: i64, title: &str, artist: &str) -> Track {
        Track { id, title: title.into(), artist: artist.into(), ..Default::default() }
    }

    #[test]
    fn duplicates_are_same_title_and_artist() {
        let tracks = vec![t(1, "Pacha", "ZARO"), t(2, "Brickell", "ZARO"), t(3, "Pacha", "ZARO"), t(4, "Pacha", "Other"), t(5, "", ""), t(6, "", ""), t(7, "pacha", "ZARO"), t(8, "Pacha", "ZARO")];
        // Exact comparison: another artist, another case and empty titles are not duplicates.
        assert_eq!(find_groups(&tracks), vec![vec![1, 3, 8]]);
        assert!(find_groups(&[t(1, "A", "x"), t(2, "B", "x")]).is_empty());
        // Two groups come out in title order.
        let two = vec![t(1, "B", "x"), t(2, "A", "y"), t(3, "B", "x"), t(4, "A", "y")];
        assert_eq!(find_groups(&two), vec![vec![2, 4], vec![1, 3]]);
    }

    #[test]
    fn merge_adds_the_representative_to_the_others_playlists() {
        // As rekordbox's add-to-playlist: at the end; the merged tracks go.
        assert_eq!(merged_playlist(&[10, 2, 11, 3], 1, &[2, 3]), vec![10, 11, 1]);
        // Already in the playlist: it stays where it is and the others just go.
        assert_eq!(merged_playlist(&[10, 2, 1, 3], 1, &[2, 3]), vec![10, 1]);
        // A playlist without any of them is unchanged.
        assert_eq!(merged_playlist(&[10, 11], 1, &[2, 3]), vec![10, 11]);
    }
}
