//! Playlist artwork, as rekordbox 7.2.19 has it (traced with IDA, notes in the track's
//! report): a playlist's right-click menu (`BrowsePopupMenuManager::showTreeViewPopupMenu`
//! 0x100118eec) has, between the Create items and Delete Playlist,
//! `addItemArtworkMenu` (0x1001140b4): "Add Artwork" when the playlist has none; else the
//! artwork itself as the menu's first item (`browse::ArtworkComponent`: the picture fitted
//! into 100 x 100 pt, centred, 10 pt from the item's top, item 120 pt tall), then "Change
//! Artwork" and "Delete Artwork". Add / Change open a file panel titled "Select Artwork" on the
//! pictures folder, filtered `*.jpg;*.jpeg;*.png` (`TreeViewer::treeMessageAddArtwork`
//! 0x101565de0), and the picture is stored with the list (`DatabaseIF::modifyListArtwork`).
//! That menu is the only place the artwork is seen in the local library: the other playlist
//! artwork views (`PlaylistDataPanel`, `PlaylistPage`) belong to the streaming services' pages.
//!
//! Here the picture is copied into `<data folder>/playlist_artwork/<id>-<time>.<ext>` and the
//! setting `playlist_art_<id>` names the copy.

use crate::app::*;
use eframe::egui;
use std::path::PathBuf;

pub fn setting_key(id: i64) -> String {
    format!("playlist_art_{id}")
}

fn art_dir() -> PathBuf {
    crate::db::data_dir().join("playlist_artwork")
}

impl App {
    /// The playlist's artwork file, if it has one.
    pub fn playlist_art(&self, id: i64) -> Option<String> {
        self.db.setting(&setting_key(id)).filter(|p| !p.is_empty() && std::path::Path::new(p).is_file())
    }

    /// Store a picture as the playlist's artwork (a copy in the data folder).
    pub fn playlist_art_set(&mut self, id: i64, src: &std::path::Path) -> Result<(), String> {
        let ext = src.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !matches!(ext.as_str(), "jpg" | "jpeg" | "png") {
            return Err("The file is not a JPEG or PNG picture.".into());
        }
        let dir = art_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        if let Some(old) = self.playlist_art(id) {
            let _ = std::fs::remove_file(old);
        }
        // (A new name each time, so that the menu does not show a cached older picture.)
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let dst = dir.join(format!("{id}-{stamp}.{ext}"));
        std::fs::copy(src, &dst).map_err(|e| e.to_string())?;
        self.db.set_setting(&setting_key(id), &dst.to_string_lossy());
        Ok(())
    }

    pub fn playlist_art_delete(&mut self, id: i64) {
        if let Some(old) = self.playlist_art(id) {
            let _ = std::fs::remove_file(old);
        }
        self.db.set_setting(&setting_key(id), "");
    }

    /// "Add Artwork" / "Change Artwork": the file panel, then the picture is stored.
    pub fn playlist_art_choose(&mut self, id: i64) {
        let mut d = rfd::FileDialog::new().set_title("Select Artwork").add_filter("*.jpg;*.jpeg;*.png", &["jpg", "jpeg", "png"]);
        if let Some(pics) = dirs_pictures() {
            d = d.set_directory(pics);
        }
        if let Some(f) = d.pick_file() {
            if let Err(e) = self.playlist_art_set(id, &f) {
                self.toast_err(e);
            }
        }
    }
}

fn dirs_pictures() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Pictures")).filter(|p| p.is_dir())
}

/// The playlist menu's artwork items (called from `ui_library_ext::node_menu`).
pub fn menu(app: &mut App, ui: &mut egui::Ui, id: i64) {
    match app.playlist_art(id) {
        None => {
            if ui.button("Add Artwork").clicked() {
                ui.close_menu();
                app.playlist_art_choose(id);
            }
        }
        Some(path) => {
            // The artwork as the first item: fitted into 100 x 100, centred, 10 pt down.
            let w = ui.available_width().max(120.0);
            let (r, _) = ui.allocate_exact_size(egui::vec2(w, 120.0), egui::Sense::hover());
            if let Some(tex) = crate::ui_browser::artwork_texture(app, &path) {
                let [tw, th] = tex.size();
                let k = (100.0 / tw.max(1) as f32).min(100.0 / th.max(1) as f32);
                let size = egui::vec2(tw as f32 * k, th as f32 * k);
                let area = egui::Rect::from_min_size(egui::pos2(r.center().x - 50.0, r.min.y + 10.0), egui::vec2(100.0, 100.0));
                let img = egui::Rect::from_center_size(area.center(), size);
                ui.painter().image(tex.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            if ui.button("Change Artwork").clicked() {
                ui.close_menu();
                app.playlist_art_choose(id);
            }
            if ui.button("Delete Artwork").clicked() {
                ui.close_menu();
                app.playlist_art_delete(id);
            }
        }
    }
    ui.separator();
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_pictures_are_taken() {
        let ok = |p: &str| matches!(std::path::Path::new(p).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref(), Some("jpg" | "jpeg" | "png"));
        assert!(ok("a.JPG") && ok("b.png") && ok("c.jpeg") && !ok("d.gif") && !ok("e"));
        assert_eq!(super::setting_key(12), "playlist_art_12");
    }
}
