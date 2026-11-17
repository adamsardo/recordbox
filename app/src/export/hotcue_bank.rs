//! Hot Cue Bank Lists (library side): named lists of eight slots (A–H), each holding a hot
//! cue taken from any track, as in rekordbox's tree section "Hot Cue Bank Lists".
//!
//! On a device rekordbox keeps bank lists in the OneLibrary database (exportLibrary.db,
//! tables `hotCueBankList` and `hotCueBankList_cue`), which Recordbox does not write. How
//! the standard device library (export.pdb) stores them has not been established: no real
//! export examined holds one (tables 9, 10, 14 and 15, whose layouts are unknown, are empty in
//! all of them). Exporting a bank list therefore says so instead of guessing, and whatever
//! those tables hold on a drive is written back unchanged (`DeviceLibrary::raw_tables`).

use crate::app::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_browser::{tree_row, TreeIcon};
use eframe::egui;
use egui::{Context, RichText, Ui};
use serde::{Deserialize, Serialize};

pub const SLOTS: usize = 8;
pub const DEFAULT_NAME: &str = "Untitled Hot Cue Bank List";
const SETTING: &str = "hot_cue_banks";

/// A hot cue held in a bank slot: a copy of the cue plus the track it belongs to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BankCue {
    pub track_id: i64,
    /// The track's file, so the slot survives the track being re-imported under a new ID.
    pub path: String,
    pub title: String,
    pub artist: String,
    pub start: f64,
    pub end: Option<f64>,
    pub color: u8,
    pub comment: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotCueBank {
    pub id: i64,
    pub name: String,
    pub slots: [Option<BankCue>; SLOTS],
}

/// The bank lists as a value: every edit goes through these functions and is saved at once.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Banks(pub Vec<HotCueBank>);

impl Banks {
    pub fn from_json(s: &str) -> Banks {
        serde_json::from_str(s).unwrap_or_default()
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    /// "Create New Hot Cue Bank List": named like rekordbox's, numbered when the name is taken.
    pub fn create(&mut self) -> i64 {
        let id = self.0.iter().map(|b| b.id).max().unwrap_or(0) + 1;
        let mut name = DEFAULT_NAME.to_string();
        let mut n = 2;
        while self.0.iter().any(|b| b.name == name) {
            name = format!("{DEFAULT_NAME} {n}");
            n += 1;
        }
        self.0.push(HotCueBank { id, name, slots: Default::default() });
        id
    }
    pub fn delete(&mut self, id: i64) -> bool {
        let n = self.0.len();
        self.0.retain(|b| b.id != id);
        self.0.len() != n
    }
    pub fn rename(&mut self, id: i64, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.0.iter().any(|b| b.id != id && b.name == name) {
            return false;
        }
        match self.0.iter_mut().find(|b| b.id == id) {
            Some(b) => {
                b.name = name.to_string();
                true
            }
            None => false,
        }
    }
    /// Put a track's hot cue into a slot (replacing what is there). Memory cues are refused.
    pub fn set_slot(&mut self, id: i64, slot: usize, track: &Track, cue: &Cue) -> bool {
        if slot >= SLOTS || cue.kind != CueKind::Hot {
            return false;
        }
        let Some(b) = self.0.iter_mut().find(|b| b.id == id) else { return false };
        b.slots[slot] = Some(BankCue { track_id: track.id, path: track.path.clone(), title: track.display_title(), artist: track.artist.clone(), start: cue.start, end: cue.end, color: cue.color, comment: cue.comment.clone() });
        true
    }
    pub fn clear_slot(&mut self, id: i64, slot: usize) -> bool {
        match self.0.iter_mut().find(|b| b.id == id).and_then(|b| b.slots.get_mut(slot)) {
            Some(s) => s.take().is_some(),
            None => false,
        }
    }
    /// Fill the free slots, in order, with a track's hot cues A–H (the quick way to bank a
    /// whole track). Returns how many were placed.
    pub fn fill_from_track(&mut self, id: i64, track: &Track, cues: &[Cue]) -> usize {
        let mut hot: Vec<&Cue> = cues.iter().filter(|c| c.kind == CueKind::Hot).collect();
        hot.sort_by_key(|c| c.slot);
        let free: Vec<usize> = match self.0.iter().find(|b| b.id == id) {
            Some(b) => (0..SLOTS).filter(|i| b.slots[*i].is_none()).collect(),
            None => return 0,
        };
        let mut n = 0;
        for (slot, cue) in free.into_iter().zip(hot) {
            if self.set_slot(id, slot, track, cue) {
                n += 1;
            }
        }
        n
    }
}

/// Why a bank list cannot be written to a device yet (shown by "Export Hot Cue Bank List").
pub const EXPORT_NOT_KNOWN: &str = "Hot Cue Bank Lists can't be exported to a device yet. rekordbox keeps them in the device's OneLibrary database, which Recordbox does not write, and how the standard device library stores them is not known: none of the rekordbox-made drives examined contains one. Bank lists already on a device are left exactly as they are.";

/// Window state (kept in `DeviceUi`).
#[derive(Default)]
pub struct BankUi {
    pub open: Option<i64>,
    pub rename: Option<(i64, String)>,
    pub confirm_delete: Option<i64>,
}

fn load(app: &App) -> Banks {
    Banks::from_json(&app.db.setting(SETTING).unwrap_or_default())
}
fn save(app: &App, b: &Banks) {
    app.db.set_setting(SETTING, &b.to_json());
}

/// Tree section "Hot Cue Bank Lists" (Preferences > View > Layout > Tree View shows or hides it).
pub fn hot_cue_bank_tree(app: &mut App, ui: &mut Ui) {
    if !app.prefs.tree_item("Hot Cue Bank List") {
        return;
    }
    let banks = load(app);
    let head = tree_row(ui, 0, "Hot Cue Bank Lists", None, false, if banks.0.is_empty() { TreeIcon::Closed } else { TreeIcon::Open }, 15.0);
    head.context_menu(|ui| {
        if ui.button("Create New Hot Cue Bank List").clicked() {
            let mut b = load(app);
            let id = b.create();
            save(app, &b);
            app.device_ui.banks.open = Some(id);
            ui.close_menu();
        }
    });
    for b in &banks.0 {
        let used = b.slots.iter().filter(|s| s.is_some()).count();
        let row = tree_row(ui, 1, &b.name, Some(used), app.device_ui.banks.open == Some(b.id), TreeIcon::Playlist, 15.0);
        if row.clicked() {
            app.device_ui.banks.open = Some(b.id);
        }
        let (id, name) = (b.id, b.name.clone());
        row.context_menu(|ui| {
            if ui.button("Export Hot Cue Bank List").clicked() {
                app.toast_err(EXPORT_NOT_KNOWN);
                ui.close_menu();
            }
            if ui.button("Rename").clicked() {
                app.device_ui.banks.rename = Some((id, name.clone()));
                app.device_ui.banks.open = Some(id);
                ui.close_menu();
            }
            if ui.button("Delete Hot Cue Bank List").clicked() {
                app.device_ui.banks.confirm_delete = Some(id);
                ui.close_menu();
            }
        });
    }
}

fn slot_letter(i: usize) -> char {
    (b'A' + i as u8) as char
}

/// The bank list window and its confirmation; called once per frame.
pub fn windows(app: &mut App, ctx: &Context) {
    if let Some(id) = app.device_ui.banks.confirm_delete {
        egui::Window::new("Delete Hot Cue Bank List").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("Are you sure you want to delete this hot cue bank list?");
            ui.horizontal(|ui| {
                if ui.button("Yes").clicked() {
                    let mut b = load(app);
                    b.delete(id);
                    save(app, &b);
                    if app.device_ui.banks.open == Some(id) {
                        app.device_ui.banks.open = None;
                    }
                    app.device_ui.banks.confirm_delete = None;
                }
                if ui.button("No").clicked() {
                    app.device_ui.banks.confirm_delete = None;
                }
            });
        });
    }
    let Some(id) = app.device_ui.banks.open else { return };
    let mut banks = load(app);
    let Some(bank) = banks.0.iter().find(|b| b.id == id).cloned() else {
        app.device_ui.banks.open = None;
        return;
    };
    // The track whose hot cues can be banked: the selected one, else the loaded one.
    let source = app.selected.iter().next().copied().filter(|_| app.selected.len() == 1).or(app.deck.track).and_then(|t| app.track(t).cloned());
    let source_cues: Vec<Cue> = source.as_ref().map(|t| app.db.cues(t.id).unwrap_or_default().into_iter().filter(|c| c.kind == CueKind::Hot).collect()).unwrap_or_default();
    let mut open = true;
    let mut changed = false;
    egui::Window::new("Hot Cue Bank List").id(egui::Id::new("hot_cue_bank")).open(&mut open).collapsible(false).default_width(520.0).show(ctx, |ui| {
        match app.device_ui.banks.rename.clone() {
            Some((rid, mut text)) if rid == id => {
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(300.0));
                    let done = ui.button("OK").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if done {
                        if banks.rename(id, &text) {
                            changed = true;
                            app.device_ui.banks.rename = None;
                        } else {
                            app.toast_err("Enter a name no other hot cue bank list uses.");
                        }
                    } else {
                        app.device_ui.banks.rename = Some((rid, text));
                    }
                });
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&bank.name).strong().size(14.0));
                    if ui.small_button("Rename").clicked() {
                        app.device_ui.banks.rename = Some((id, bank.name.clone()));
                    }
                });
            }
        }
        match &source {
            Some(t) if !source_cues.is_empty() => ui.label(RichText::new(format!("Hot cues of \"{}\" can be put into the slots.", t.display_title())).size(11.0).color(DIM)),
            Some(t) => ui.label(RichText::new(format!("\"{}\" has no hot cues to put into the slots.", t.display_title())).size(11.0).color(DIM)),
            None => ui.label(RichText::new("Select or load a track to put its hot cues into the slots.").size(11.0).color(DIM)),
        };
        ui.separator();
        egui::Grid::new("bank_slots").num_columns(4).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
            for i in 0..SLOTS {
                ui.label(RichText::new(slot_letter(i).to_string()).strong());
                match &bank.slots[i] {
                    Some(c) => {
                        let kind = if c.end.is_some() { "Loop" } else { "Cue" };
                        let name = if c.comment.is_empty() { String::new() } else { format!("  “{}”", c.comment) };
                        ui.label(format!("{} — {}", c.title, c.artist)).on_hover_text(&c.path);
                        ui.label(RichText::new(format!("{kind} at {}{name}", fmt_time(c.start, true))).size(11.0).color(DIM));
                    }
                    None => {
                        ui.label(RichText::new("Empty").color(DIM));
                        ui.label("");
                    }
                }
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!source_cues.is_empty(), |ui| {
                        ui.menu_button("Set", |ui| {
                            for c in &source_cues {
                                let label = format!("Hot Cue {}  {}{}", slot_letter(c.slot.max(0) as usize), fmt_time(c.start, true), if c.comment.is_empty() { String::new() } else { format!("  {}", c.comment) });
                                if ui.button(label).clicked() {
                                    if let Some(t) = &source {
                                        changed |= banks.set_slot(id, i, t, c);
                                    }
                                    ui.close_menu();
                                }
                            }
                        })
                        .response
                        .on_disabled_hover_text("Select or load a track that has hot cues.");
                    });
                    if ui.add_enabled(bank.slots[i].is_some(), egui::Button::new("Clear")).clicked() {
                        changed |= banks.clear_slot(id, i);
                    }
                });
                ui.end_row();
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.add_enabled(!source_cues.is_empty(), egui::Button::new("Fill empty slots from this track")).on_disabled_hover_text("Select or load a track that has hot cues.").clicked() {
                if let Some(t) = &source {
                    changed |= banks.fill_from_track(id, t, &source_cues) > 0;
                }
            }
            if ui.button("Export Hot Cue Bank List").clicked() {
                app.toast_err(EXPORT_NOT_KNOWN);
            }
            if ui.button("Delete Hot Cue Bank List").clicked() {
                app.device_ui.banks.confirm_delete = Some(id);
            }
        });
    });
    if changed {
        save(app, &banks);
    }
    if !open {
        app.device_ui.banks.open = None;
        app.device_ui.banks.rename = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(slot: i32, start: f64, kind: CueKind) -> Cue {
        Cue { id: 0, track_id: 7, kind, slot, start, end: None, color: 0, comment: format!("c{slot}"), active_loop: false }
    }

    #[test]
    fn bank_lists_are_created_filled_renamed_persisted_and_deleted() {
        let mut b = Banks::default();
        let one = b.create();
        let two = b.create();
        assert_eq!(b.0.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), [DEFAULT_NAME, "Untitled Hot Cue Bank List 2"]);
        let t = Track { id: 7, path: "/music/a.mp3".into(), title: "Song".into(), artist: "Artist".into(), ..Default::default() };
        let cues = vec![cue(2, 30.0, CueKind::Hot), cue(0, 10.0, CueKind::Hot), cue(-1, 5.0, CueKind::Memory), cue(1, 20.0, CueKind::Hot)];
        // A single slot; memory cues and slots beyond H are refused.
        assert!(b.set_slot(one, 1, &t, &cues[0]));
        assert!(!b.set_slot(one, 0, &t, &cues[2]));
        assert!(!b.set_slot(one, SLOTS, &t, &cues[0]));
        assert!(!b.set_slot(99, 0, &t, &cues[0]));
        // Filling: the track's hot cues in A, B, C order go into the free slots (B is taken).
        assert_eq!(b.fill_from_track(one, &t, &cues), 3);
        let starts: Vec<Option<f64>> = b.0[0].slots.iter().map(|s| s.as_ref().map(|c| c.start)).collect();
        assert_eq!(starts[..4], [Some(10.0), Some(30.0), Some(20.0), Some(30.0)]);
        assert_eq!(b.0[0].slots[0].as_ref().unwrap().path, "/music/a.mp3");
        assert!(b.clear_slot(one, 3) && !b.clear_slot(one, 3));
        // Names stay unique.
        assert!(b.rename(two, "Drops") && !b.rename(one, "Drops") && !b.rename(one, "  "));
        // What is saved is what is loaded.
        assert_eq!(Banks::from_json(&b.to_json()), b);
        assert_eq!(Banks::from_json("not json"), Banks::default());
        assert!(b.delete(one) && !b.delete(one));
        assert_eq!(b.0.len(), 1);
        assert_eq!(b.create(), 3, "IDs are not reused while a higher one exists");
    }

    #[test]
    fn no_real_export_holds_a_bank_list_so_the_export_says_why() {
        assert!(EXPORT_NOT_KNOWN.contains("not known"));
        let root = dirs::home_dir().unwrap().join("Developer/rekordbox-rea/analysis/format-oracles");
        for p in ["Holzhaus__rekordcrate/data/complete_export/demo_tracks/PIONEER/rekordbox/export.pdb", "Holzhaus__rekordcrate/data/pdb/num_rows/export.pdb", "fragmede__rekordbox-pdb/tests/data/bigger-export.pdb"] {
            let Ok(bytes) = std::fs::read(root.join(p)) else { continue };
            let db = crate::export::pdb::Database::parse(bytes).unwrap();
            for t in crate::export::usb::KEPT_TABLES {
                assert!(db.data_pages(t).is_empty(), "{p}: table {t} has rows; its layout can now be studied");
            }
        }
    }
}
