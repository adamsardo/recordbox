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
//! Not traced: which columns the two lists show (rekordbox switches seven on by number) and
//! the look of its header pull-down that names the playlists a track is in. Here the lists
//! show Track Title, Artist, Album, BPM, Time, Date Added and Location, and the playlists of
//! the chosen row are named in a line above the list.

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

/// A playlist's tracks after a merge: the representative stands where the first of the merged
/// tracks stood (unless the playlist has it already), and the other merged tracks are gone.
pub fn merged_playlist(tracks: &[i64], keep: i64, others: &[i64]) -> Vec<i64> {
    let had = tracks.contains(&keep);
    let mut placed = had;
    let mut out = Vec::with_capacity(tracks.len());
    for t in tracks {
        if others.contains(t) {
            if !placed {
                out.push(keep);
                placed = true;
            }
        } else {
            out.push(*t);
        }
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
    }

    /// Names of the playlists a track is in.
    fn dupes_playlists_of(&self, id: i64) -> Vec<String> {
        self.nodes.iter().filter(|n| n.kind == NodeKind::Playlist).filter(|n| self.db.playlist_tracks(n.id).map(|t| t.contains(&id)).unwrap_or(false)).map(|n| n.name.clone()).collect()
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

const COLS: [(&str, f32); 7] = [("Track Title", 190.0), ("Artist", 130.0), ("Album", 120.0), ("BPM", 54.0), ("Time", 48.0), ("Date Added", 84.0), ("Location", 0.0)];

fn cells(t: &Track) -> [String; 7] {
    [t.display_title(), t.artist.clone(), t.album.clone(), if t.bpm > 0.0 { format!("{:.2}", t.bpm) } else { String::new() }, fmt_time(t.duration, false), t.date_added.chars().take(10).collect(), t.path.clone()]
}

/// A list of tracks in the manager's look. `pick_one`: a click chooses one row (the
/// representative); else rows are selected with click, Cmd-click and Shift-click.
fn track_table(app: &mut App, ui: &mut egui::Ui, table: Rect, ids: &[i64], salt: &str, pick_one: Option<&mut Option<i64>>) {
    let p = ui.painter().clone();
    p.rect_filled(table, 0.0, BG);
    let font = rb(11.5);
    let tracks: Vec<Track> = ids.iter().filter_map(|id| app.track(*id).cloned()).collect();
    let loc_w = tracks.iter().map(|t| p.layout_no_wrap(t.path.clone(), font.clone(), WHITE).size().x).fold(0.0, f32::max);
    let fixed: f32 = COLS.iter().map(|c| c.1).sum();
    let content_w = (fixed + loc_w + 16.0).max(table.width());
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(table).id_salt(salt));
    tui.set_clip_rect(table);
    crate::ui_browser_ext::bar_style(&mut tui);
    tui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
    let mut pick_one = pick_one;
    egui::ScrollArea::horizontal().id_salt((salt, "h")).auto_shrink([false, false]).show(&mut tui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (hr, _) = ui.allocate_exact_size(vec2(content_w, 22.0), Sense::hover());
        ui.painter().rect_filled(hr, 0.0, hex(0x1A1A1A));
        let mut x = hr.min.x;
        for (i, (label, cw)) in COLS.iter().enumerate() {
            if i > 0 {
                ui.painter().rect_filled(Rect::from_min_max(pos2(x - 1.0, hr.min.y), pos2(x, hr.max.y)), 0.0, BG);
            }
            ui.painter().text(pos2(x + 6.0, hr.center().y), Align2::LEFT_CENTER, *label, font.clone(), DIM);
            x += if *cw > 0.0 { *cw } else { content_w - fixed };
        }
        egui::ScrollArea::vertical().id_salt((salt, "v")).auto_shrink([false, false]).show_rows(ui, 20.0, tracks.len(), |ui, range| {
            for i in range {
                let t = &tracks[i];
                let (rr, resp) = ui.allocate_exact_size(vec2(content_w, 20.0), Sense::click());
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
                for (v, (_, cw)) in cells(t).into_iter().zip(COLS.iter()) {
                    let w = if *cw > 0.0 { *cw } else { content_w - fixed };
                    let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(w - 2.0, rr.height()));
                    ui.painter().with_clip_rect(cr.intersect(ui.clip_rect())).text(pos2(cr.min.x + 5.0, cr.center().y), Align2::LEFT_CENTER, v, font.clone(), if t.missing { CAUTION_TX } else { WHITE });
                    x += w;
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
}

/// A dialog button of the manager (rekordbox's sizes; the look of the Missing File Manager's).
fn button(ui: &mut egui::Ui, r: Rect, id: &str, label: &str, enabled: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(id), if enabled { Sense::click() } else { Sense::hover() });
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
                answer = kit::dialog_buttons(ui, &["OK", "Cancel"]);
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
        // The playlists of the chosen row, in the list's header line.
        let list = Rect::from_min_size(pos2(r.min.x + 20.0, r.min.y + 20.0), vec2(w - 40.0, h - 100.0));
        let head = Rect::from_min_size(list.min, vec2(list.width(), 22.0));
        p.rect_filled(head, 0.0, hex(0x1E1E1E));
        let where_text = if app.dupes.sel.len() == 1 {
            let names = app.dupes_playlists_of(*app.dupes.sel.iter().next().unwrap_or(&0));
            match names.len() {
                0 => "All Tracks".to_string(),
                1 => names[0].clone(),
                _ => format!("Multiple playlists: {}", names.join(", ")),
            }
        } else {
            String::new()
        };
        p.with_clip_rect(head).text(pos2(head.min.x + 8.0, head.center().y), Align2::LEFT_CENTER, where_text, rb(11.5), WHITE);
        if !modal {
            track_table(app, ui, Rect::from_min_max(pos2(list.min.x, head.max.y + 1.0), list.max), &ids, "dupes_table", None);
        } else {
            p.rect_filled(Rect::from_min_max(pos2(list.min.x, head.max.y + 1.0), list.max), 0.0, BG);
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
            answer = kit::dialog_buttons(ui, &["OK", "Cancel"]);
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
            close = kit::dialog_buttons(ui, &["OK"]).is_some();
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
    fn merge_puts_the_representative_in_the_others_places() {
        // The representative takes the first merged track's place.
        assert_eq!(merged_playlist(&[10, 2, 11, 3], 1, &[2, 3]), vec![10, 1, 11]);
        // Already in the playlist: it stays where it is and the others just go.
        assert_eq!(merged_playlist(&[10, 2, 1, 3], 1, &[2, 3]), vec![10, 1]);
        // A playlist without any of them is unchanged.
        assert_eq!(merged_playlist(&[10, 11], 1, &[2, 3]), vec![10, 11]);
    }
}
