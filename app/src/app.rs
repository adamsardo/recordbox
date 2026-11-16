//! Application state and operations (library, player, analysis, export).

use eframe::egui;
use crate::analysis::{AnalysisSettings, BPM_RANGES};
use crate::audio::{Decoded, Player};
use crate::db::Db;
use crate::jobs::*;
use crate::model::*;
use crate::smart::SmartRule;
use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Col {
    Num,
    Status,
    Preview,
    Artwork,
    Title,
    Artist,
    Album,
    Genre,
    Bpm,
    Rating,
    Time,
    Key,
    DateAdded,
    Color,
    Comments,
    Label,
    Remixer,
    OriginalArtist,
    Composer,
    Year,
    Bitrate,
    SampleRate,
    FileType,
    Size,
    Location,
    FileName,
    PlayCount,
    MixName,
    TrackNumber,
    MyTag,
    ReleaseDate,
    AlbumArtist,
    DiscNumber,
    Bitdepth,
    Grouping,
    Lyricist,
    Message,
    HotCue,
}

pub const ALL_COLS: [Col; 38] = [
    Col::Num,
    Col::Status,
    Col::Preview,
    Col::Artwork,
    Col::Title,
    Col::Artist,
    Col::Album,
    Col::Genre,
    Col::Bpm,
    Col::Rating,
    Col::Time,
    Col::Key,
    Col::DateAdded,
    Col::Color,
    Col::Comments,
    Col::Label,
    Col::Remixer,
    Col::OriginalArtist,
    Col::Composer,
    Col::Year,
    Col::Bitrate,
    Col::SampleRate,
    Col::FileType,
    Col::Size,
    Col::Location,
    Col::FileName,
    Col::PlayCount,
    Col::MixName,
    Col::TrackNumber,
    Col::MyTag,
    Col::ReleaseDate,
    Col::AlbumArtist,
    Col::DiscNumber,
    Col::Bitdepth,
    Col::Grouping,
    Col::Lyricist,
    Col::Message,
    Col::HotCue,
];

impl Col {
    pub fn label(self) -> &'static str {
        match self {
            Col::Num => "#",
            Col::Status => "",
            Col::Preview => "Preview",
            Col::Artwork => "Artwork",
            Col::Title => "Track Title",
            Col::Artist => "Artist",
            Col::Album => "Album",
            Col::Genre => "Genre",
            Col::Bpm => "BPM",
            Col::Rating => "Rating",
            Col::Time => "Time",
            Col::Key => "Key",
            Col::DateAdded => "Date Added",
            Col::Color => "Color",
            Col::Comments => "Comments",
            Col::Label => "Label",
            Col::Remixer => "Remixer",
            Col::OriginalArtist => "Original Artist",
            Col::Composer => "Composer",
            Col::Year => "Year",
            Col::Bitrate => "Bitrate",
            Col::SampleRate => "Sample Rate",
            Col::FileType => "File Type",
            Col::Size => "Size",
            Col::Location => "Location",
            Col::FileName => "File Name",
            Col::PlayCount => "DJ Play Count",
            Col::MixName => "Mix Name",
            Col::TrackNumber => "Track number",
            Col::MyTag => "My Tag",
            Col::ReleaseDate => "Release Date",
            Col::AlbumArtist => "Album Artist",
            Col::DiscNumber => "Disc number",
            Col::Bitdepth => "Bitdepth",
            Col::Grouping => "Grouping",
            Col::Lyricist => "Lyricist",
            Col::Message => "Message",
            Col::HotCue => "Hot Cue",
        }
    }
    pub fn key(self) -> String {
        format!("{:?}", self)
    }
    pub fn width(self) -> f32 {
        match self {
            // Default widths measured on the original's Collection view (pt).
            Col::Num => 60.0,
            Col::Status => 60.0,
            Col::Preview => 140.0,
            Col::Artwork => 79.0,
            Col::Title | Col::Artist | Col::Album | Col::Genre => 128.5,
            Col::Bpm => 79.0,
            Col::Rating => 87.0,
            Col::Time => 83.0,
            Col::Key => 128.5,
            Col::DateAdded | Col::ReleaseDate => 82.0,
            Col::Color => 46.0,
            Col::Location => 260.0,
            Col::Comments | Col::MyTag => 160.0,
            Col::HotCue => 90.0,
            _ => 100.0,
        }
    }
    pub fn right_aligned(self) -> bool {
        matches!(self, Col::Bpm | Col::Time | Col::Bitrate | Col::SampleRate | Col::Size | Col::PlayCount | Col::Year | Col::TrackNumber | Col::DiscNumber | Col::Bitdepth | Col::Num)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub key_alnum: bool,
    pub bpm_range: usize,
    pub analyze_grid: bool,
    pub analyze_key: bool,
    pub auto_analysis: bool,
    pub wave_color: u8,
    pub quantize: bool,
    pub time_remain: bool,
    pub beat_jump: f64,
    pub loop_beats: f64,
    pub loop_auto: bool,
    pub zoom: usize,
    pub columns: Vec<Col>,
    pub sort_col: Col,
    pub sort_asc: bool,
    pub delete_device_tracks: bool,
    pub target_lufs: f64,
    pub tooltips: bool,
    pub show_mytag: bool,
    pub grid_from_current: bool,
    pub devices_extra: Vec<String>,
    pub widths: HashMap<String, f32>,
    pub browser_only: bool,
    /// Copy and sync the installed rekordbox library (master.db) every time PrepDeck starts.
    pub rekordbox_sync: bool,
    /// Player settings written to every exported USB (Preferences > DJ System).
    pub usb_settings: crate::export::settings::UsbSettings,
    /// Which field the track-list search looks in (0 = all; see SEARCH_SCOPES).
    pub search_scope: u8,
    pub related: RelatedRules,
}

/// Related Tracks rules (the original offers Same Key / Related Key 1–3, BPM range and more).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelatedRules {
    /// 0 off, 1 Same Key, 2 Related Key 1, 3 Related Key 2, 4 Related Key 3.
    pub key: u8,
    pub bpm: bool,
    /// Allowed BPM difference in percent (half and double tempo also count).
    pub bpm_pct: f64,
    pub genre: bool,
    pub artist: bool,
    pub label: bool,
    pub color: bool,
    pub rating: bool,
}

impl Default for RelatedRules {
    fn default() -> Self {
        RelatedRules { key: 2, bpm: true, bpm_pct: 6.0, genre: false, artist: false, label: false, color: false, rating: false }
    }
}

/// Distance between two keys on the key wheel: steps around the wheel plus one when the
/// mode differs (0 same key, 1 neighbour or relative major/minor, 2 diagonal or two steps…).
#[cfg(test)]
mod related_tests {
    #[test]
    fn key_wheel_distance() {
        // 8A (Am, index 7) vs 8A, 9A, 8B (C), 9B, 10A, 2A
        assert_eq!(super::key_distance(7, 7), Some(0));
        assert_eq!(super::key_distance(7, 8), Some(1));
        assert_eq!(super::key_distance(7, 19), Some(1));
        assert_eq!(super::key_distance(7, 20), Some(2));
        assert_eq!(super::key_distance(7, 9), Some(2));
        assert_eq!(super::key_distance(7, 1), Some(6));
        // 12A and 1A are neighbours on the wheel
        assert_eq!(super::key_distance(11, 0), Some(1));
        assert_eq!(super::key_distance(-1, 3), None);
    }
}

pub fn key_distance(a: i32, b: i32) -> Option<i32> {
    if !(0..24).contains(&a) || !(0..24).contains(&b) {
        return None;
    }
    let d = (a % 12 - b % 12).rem_euclid(12);
    Some(d.min(12 - d) + i32::from(a / 12 != b / 12))
}

/// Search field choices, in the order of the search box's menu.
pub const SEARCH_SCOPES: [&str; 8] = ["All", "Track Title", "Artist", "Album", "Genre", "Label", "Comments", "My Tag"];

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            key_alnum: false,
            bpm_range: 0,
            analyze_grid: true,
            analyze_key: true,
            auto_analysis: true,
            wave_color: 2,
            quantize: true,
            time_remain: true,
            beat_jump: 4.0,
            loop_beats: 4.0,
            loop_auto: true,
            zoom: 2,
            columns: vec![Col::Status, Col::Preview, Col::Artwork, Col::Title, Col::Artist, Col::Album, Col::Genre, Col::Bpm, Col::Rating, Col::Time, Col::Key, Col::DateAdded],
            sort_col: Col::DateAdded,
            sort_asc: false,
            rekordbox_sync: true,
            usb_settings: Default::default(),
            search_scope: 0,
            related: Default::default(),
            delete_device_tracks: false,
            target_lufs: -8.0,
            tooltips: true,
            show_mytag: false,
            grid_from_current: false,
            devices_extra: vec![],
            widths: HashMap::new(),
            browser_only: false,
        }
    }
}

/// Seconds shown across the enlarged waveform; the default (index 2) matches the original's default
/// zoom, measured at about 21 beats from the playhead to the right edge at 126 BPM.
pub const ZOOMS: [f64; 6] = [5.0, 10.0, 20.0, 40.0, 80.0, 160.0];

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Collection,
    Playlist(i64),
    Device { root: String, node: Option<u32> },
    /// A play history saved on a device by the players.
    DeviceHistory { root: String, id: u32 },
    /// Tracks related to the loaded (or selected) track by the Related Tracks rules.
    Related,
    /// Collection browsed by one field: 0 Date Added, 1 Genre, 2 Artist, 3 Album.
    Category { field: u8, value: String },
    /// A folder on disk (Explorer); its audio files are listed whether or not they are in the
    /// collection.
    Explorer(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideMode {
    Collection,
    Playlists,
    Devices,
    History,
    Related,
    Explorer,
}

pub struct Deck {
    pub track: Option<i64>,
    pub audio: Option<Arc<Decoded>>,
    pub loading: Option<Receiver<(i64, Result<Arc<Decoded>, String>)>>,
    pub wave: Option<Waveform>,
    pub grid: BeatGrid,
    pub beats: Vec<Beat>,
    pub cues: Vec<Cue>,
    pub cue_point: f64,
    pub duration: f64,
    pub cue_preview: bool,
    pub hot_preview: Option<f64>,
    pub loop_in: Option<f64>,
    pub last_loop: Option<(f64, f64)>,
    pub grid_undo: Vec<BeatGrid>,
    pub grid_redo: Vec<BeatGrid>,
    pub taps: Vec<Instant>,
    pub ctrl_tab: u8,
    pub panel_tab: u8,
    pub drag_origin: Option<(f32, f64)>,
    pub editing_comment: Option<i64>,
    pub metronome: bool,
    pub bpm_text: String,
}

impl Default for Deck {
    fn default() -> Self {
        Deck {
            track: None,
            audio: None,
            loading: None,
            wave: None,
            grid: BeatGrid::default(),
            beats: vec![],
            cues: vec![],
            cue_point: 0.0,
            duration: 0.0,
            cue_preview: false,
            hot_preview: None,
            loop_in: None,
            last_loop: None,
            grid_undo: vec![],
            grid_redo: vec![],
            taps: vec![],
            ctrl_tab: 0,
            panel_tab: 0,
            drag_origin: None,
            editing_comment: None,
            metronome: false,
            bpm_text: String::new(),
        }
    }
}

pub struct Toast {
    pub text: String,
    pub until: Instant,
    pub error: bool,
}

#[derive(Default)]
pub struct Dialogs {
    pub prefs_open: bool,
    pub prefs_tab: u8,
    pub info_track: Option<Track>,
    pub smart_edit: Option<(Option<i64>, i64, String, SmartRule)>,
    pub rename: Option<(i64, String)>,
    pub new_tag: Option<(i64, String)>,
    pub rename_tag: Option<(i64, String)>,
    pub confirm_remove: Option<Vec<i64>>,
    pub export_open: bool,
    pub export_root: Option<String>,
    pub export_sel: HashSet<i64>,
    pub export_tracks: Vec<i64>,
    pub about: bool,
    pub device_remove: Option<(String, u32, String)>,
}

pub struct DeviceInfo {
    pub root: String,
    pub name: String,
    pub fs: String,
    pub lib: Option<crate::export::usb::DeviceLibrary>,
    pub error: Option<String>,
    pub total: Option<u64>,
    pub free: Option<u64>,
}

pub struct App {
    pub db: Db,
    pub prefs: Prefs,
    pub tracks: Vec<Track>,
    pub index: HashMap<i64, usize>,
    pub previews: HashMap<i64, Vec<u8>>,
    pub cue_counts: HashMap<i64, (usize, usize)>,
    pub hot_positions: HashMap<i64, Vec<(f64, i32, u8)>>,
    pub nodes: Vec<PlaylistNode>,
    pub node_counts: HashMap<i64, usize>,
    pub mytags: Vec<MyTag>,
    pub track_tags: HashMap<i64, HashSet<i64>>,
    pub view: View,
    pub side: SideMode,
    pub expanded: HashSet<i64>,
    pub list: Vec<i64>,
    pub list_dirty: bool,
    pub selected: HashSet<i64>,
    pub anchor: Option<usize>,
    /// Track the keyboard/shift-click anchor points at; `anchor` is its row in `list` and is
    /// re-derived whenever the list is rebuilt (sort, search, view change).
    pub anchor_id: Option<i64>,
    pub search: String,
    pub filter_open: bool,
    pub filter_bpm: (f64, f64),
    pub filter_key: i32,
    pub filter_rating: u8,
    pub filter_color: u8,
    pub filter_tags: HashSet<i64>,
    pub filter_tags_all: bool,
    pub player: Player,
    pub deck: Deck,
    /// The deck as it was before the load in progress, with its playback position and gain;
    /// put back if the new file can't be decoded, so a failed load never empties the player.
    pub deck_backup: Option<(Deck, f64, f32)>,
    /// Source track the Related Tracks list was last built for.
    pub related_built_for: Option<i64>,
    /// File to load into the player once the import in progress has added it.
    pub load_after_import: Option<String>,
    /// Playlist/folder being dragged in the tree.
    pub node_drag: Option<i64>,
    pub pool: AnalysisPool,
    pub analysis_total: usize,
    pub analysis_done: usize,
    pub importing: Option<Receiver<Option<Track>>>,
    pub import_count: usize,
    pub import_to_playlist: Option<i64>,
    pub export_job: Option<ExportJob>,
    /// Background snapshot + read of the rekordbox library, started at launch.
    pub rb_job: Option<crossbeam_channel::Receiver<anyhow::Result<Option<crate::rbimport::RbLibrary>>>>,
    pub devices: Vec<DeviceInfo>,
    pub devices_scanned: Option<Instant>,
    pub dialogs: Dialogs,
    pub toasts: Vec<Toast>,
    pub drag: Option<Vec<i64>>,
    pub textures: HashMap<String, Option<egui::TextureHandle>>,
    pub ctx: egui::Context,
    pub screenshot: Option<(String, u32)>,
    pub scroll_to_selected: bool,
    /// Native menu bar (kept alive for the app's lifetime).
    pub menu: Option<muda::Menu>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, screenshot: Option<String>) -> App {
        crate::theme::apply(&cc.egui_ctx);
        let db_path = crate::db::data_dir().join("library.db");
        let db = Db::open(&db_path).expect("open library database");
        let prefs: Prefs = db.setting("prefs").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let pool = AnalysisPool::new(cc.egui_ctx.clone());
        let mut app = App {
            db,
            prefs,
            tracks: vec![],
            index: HashMap::new(),
            previews: HashMap::new(),
            cue_counts: HashMap::new(),
            hot_positions: HashMap::new(),
            nodes: vec![],
            node_counts: HashMap::new(),
            mytags: vec![],
            track_tags: HashMap::new(),
            view: View::Collection,
            side: SideMode::Collection,
            expanded: HashSet::new(),
            list: vec![],
            list_dirty: true,
            selected: HashSet::new(),
            anchor: None,
            anchor_id: None,
            deck_backup: None,
            related_built_for: None,
            load_after_import: None,
            node_drag: None,
            search: String::new(),
            filter_open: false,
            filter_bpm: (0.0, 0.0),
            filter_key: -1,
            filter_rating: 0,
            filter_color: 0,
            filter_tags: HashSet::new(),
            filter_tags_all: false,
            player: Player::new(),
            deck: Deck::default(),
            pool,
            analysis_total: 0,
            analysis_done: 0,
            importing: None,
            import_count: 0,
            import_to_playlist: None,
            export_job: None,
            rb_job: None,
            devices: vec![],
            devices_scanned: None,
            dialogs: Dialogs::default(),
            toasts: vec![],
            drag: None,
            textures: HashMap::new(),
            ctx: cc.egui_ctx.clone(),
            screenshot: screenshot.map(|s| (s, 0)),
            scroll_to_selected: false,
            menu: None,
        };
        app.menu = crate::menus::build();
        app.reload_all();
        if let Some(e) = app.player.error.clone() {
            app.toast_err(format!("Audio output unavailable: {e}"));
        }
        // First start: add original demo loops (like the bundled demo tracks of the original).
        let rb_library = app.prefs.rekordbox_sync && crate::rbimport::library_dir().is_some();
        if app.tracks.is_empty() && app.db.setting("demo_added").is_none() && (!rb_library || (app.screenshot.is_some() && std::env::var("PREPDECK_SHOT_SYNC").is_err())) {
            app.db.set_setting("demo_added", "1");
            let files = crate::demo::ensure_demo_files();
            for f in files {
                let mut t = crate::importer::read_track(&f);
                t.artist = "PrepDeck".into();
                t.album = "Demo Tracks".into();
                t.genre = "Demo".into();
                let _ = app.db.insert_track(&t);
            }
            app.reload_all();
            if app.screenshot.is_some() {
                let s = app.analysis_settings();
                let todo: Vec<(i64, String)> = app.tracks.iter().map(|t| (t.id, t.path.clone())).collect();
                for (id, p) in todo {
                    if let Ok(r) = crate::analysis::analyze_file(&p, &s) {
                        app.apply_analysis(id, Ok(r));
                    }
                }
            } else {
                let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
                app.analyze(&ids, false);
            }
        }
        // Test switch: open Preferences at launch.
        if std::env::var("PREPDECK_OPEN_PREFS").is_ok() {
            app.dialogs.prefs_open = true;
        }
        if app.prefs.rekordbox_sync && (app.screenshot.is_none() || std::env::var("PREPDECK_SHOT_SYNC").is_ok()) {
            app.start_rekordbox_sync();
        }
        if app.screenshot.is_some() {
            // Optional state for checking other screens: PREPDECK_SHOT_VIEW=playlists|devices|grid|hotcue|prefs|mytag|smart|export|info
            if let Ok(v) = std::env::var("PREPDECK_SHOT_VIEW") {
                match v.as_str() {
                    "playlists" | "smart" => {
                        app.side = SideMode::Playlists;
                        if app.nodes.is_empty() {
                            let f = app.db.create_node(0, "Friday Set", NodeKind::Folder, None).unwrap_or(0);
                            let p = app.db.create_node(f, "Warm Up", NodeKind::Playlist, None).unwrap_or(0);
                            let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
                            let _ = app.db.set_playlist_tracks(p, &ids);
                            let mut rule = SmartRule::default();
                            rule.conditions[0] = crate::smart::Condition { field: crate::smart::Field::Bpm, op: crate::smart::Op::InRange, value: "120".into(), value2: "128".into(), months: false };
                            let _ = app.db.create_node(0, "120-128 BPM", NodeKind::Smart, Some(&rule));
                            app.reload_playlists();
                            app.expanded.insert(f);
                            app.view = View::Playlist(p);
                            app.set_view(View::Collection);
                            app.set_view(View::Playlist(p));
                        }
                        if v == "smart" {
                            let n = app.nodes.iter().find(|n| n.kind == NodeKind::Smart).cloned();
                            if let Some(n) = n {
                                app.dialogs.smart_edit = Some((Some(n.id), n.parent, n.name.clone(), n.smart.clone().unwrap_or_default()));
                            }
                        }
                    }
                    "devices" => {
                        app.side = SideMode::Devices;
                        app.view = View::Device { root: String::new(), node: None };
                    }
                    "history" => app.side = SideMode::History,
                    "explorer" => {
                        app.side = SideMode::Explorer;
                        let d = dirs::audio_dir().unwrap_or_default().to_string_lossy().to_string();
                        app.view = View::Explorer(d);
                    }
                    "related" => {
                        app.side = SideMode::Related;
                        app.view = View::Related;
                    }
                    "grid" => app.deck.ctrl_tab = 1,
                    "hotcue" => app.deck.panel_tab = 1,
                    "info" => app.deck.panel_tab = 2,
                    "prefs" => {
                        app.dialogs.prefs_open = true;
                        // PREPDECK_SHOT_PREFS=<tab>,<subtab> (default 1,0).
                        let (t, st) = std::env::var("PREPDECK_SHOT_PREFS").ok().and_then(|v| {
                            let mut it = v.split(',').map(|x| x.trim().parse::<usize>().ok());
                            Some((it.next()??, it.next().flatten().unwrap_or(0)))
                        }).unwrap_or((1, 0));
                        app.dialogs.prefs_tab = t as u8;
                        cc.egui_ctx.data_mut(|d| d.insert_temp(egui::Id::new(format!("prefs_subtab_{t}")), st));
                    }
                    "mytag" => app.prefs.show_mytag = true,
                    "export" => app.dialogs.export_open = true,
                    _ => {}
                }
            }
            // PREPDECK_SHOT_TRACK=<part of a title>: load that track (analysed first if needed).
            let wanted = std::env::var("PREPDECK_SHOT_TRACK").ok().map(|w| w.to_lowercase());
            let first = match &wanted {
                Some(w) => app.tracks.iter().find(|t| t.title.to_lowercase().contains(w) && !t.missing).map(|t| t.id),
                None => app.tracks.iter().find(|t| t.analyzed == 1 && !t.missing).map(|t| t.id),
            };
            if let Some(id) = first.filter(|id| app.track(*id).map(|t| t.analyzed != 1).unwrap_or(false)) {
                let s = app.analysis_settings();
                if let Some(p) = app.track(id).map(|t| t.path.clone()) {
                    if let Ok(r) = crate::analysis::analyze_file(&p, &s) {
                        app.apply_analysis(id, Ok(r));
                    }
                }
            }
            if let Some(t) = first {
                app.load_track(t);
                app.selected.insert(t);
                if let Some(rx) = app.deck.loading.take() {
                    if let Ok(msg) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
                        let (tx2, rx2) = crossbeam_channel::bounded(1);
                        let _ = tx2.send(msg);
                        app.deck.loading = Some(rx2);
                        app.poll_load();
                    }
                }
                if std::env::var("PREPDECK_SHOT_VIEW").map(|v| v == "hotcue" || v == "grid").unwrap_or(false) && app.deck.cues.is_empty() {
                    let beats = app.deck.beats.clone();
                    for (slot, bi) in [(0, 16usize), (1, 48), (3, 80)] {
                        if let Some(b) = beats.get(bi) {
                            app.seek(b.time);
                            app.hot_cue_down(slot);
                        }
                    }
                    if let Some(b) = beats.get(32) {
                        app.deck.cue_point = b.time;
                        app.memory_set();
                    }
                    let c = app.deck.cue_point;
                    app.seek(c + 2.0);
                }
            }
        }
        app
    }

    pub fn save_prefs(&self) {
        if let Ok(s) = serde_json::to_string(&self.prefs) {
            self.db.set_setting("prefs", &s);
        }
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toasts.push(Toast { text: s.into(), until: Instant::now() + std::time::Duration::from_secs(5), error: false });
    }
    pub fn toast_err(&mut self, s: impl Into<String>) {
        self.toasts.push(Toast { text: s.into(), until: Instant::now() + std::time::Duration::from_secs(8), error: true });
    }

    pub fn analysis_settings(&self) -> AnalysisSettings {
        let (lo, hi) = BPM_RANGES[self.prefs.bpm_range.min(BPM_RANGES.len() - 1)];
        AnalysisSettings { bpm_min: lo, bpm_max: hi, do_grid: self.prefs.analyze_grid, do_key: self.prefs.analyze_key, target_lufs: self.prefs.target_lufs }
    }

    // ------------------------------------------------------------------ loading
    pub fn reload_all(&mut self) {
        self.reload_tracks();
        self.previews = self.db.previews().unwrap_or_default();
        self.cue_counts = self.db.tracks_with_cues().unwrap_or_default();
        self.hot_positions = self.db.hot_positions().unwrap_or_default();
        self.reload_playlists();
        self.reload_tags();
    }
    pub fn reload_tracks(&mut self) {
        let queued: HashSet<i64> = self.tracks.iter().filter(|t| t.analyzed == 3).map(|t| t.id).collect();
        self.tracks = self.db.all_tracks().unwrap_or_default();
        for t in self.tracks.iter_mut() {
            t.missing = !std::path::Path::new(&t.path).exists();
            if queued.contains(&t.id) {
                t.analyzed = 3;
            } else if t.analyzed == 3 {
                t.analyzed = 0;
            }
        }
        self.index = self.tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        self.list_dirty = true;
    }
    /// Move a playlist/folder before or after `target`, or into `target` when it is a folder.
    /// Refuses to put a folder inside itself or one of its own subfolders.
    pub fn drop_node(&mut self, id: i64, target: i64, zone: i8) {
        if id == target {
            return;
        }
        let Some(t) = self.node(target).cloned() else { return };
        let (parent, seq) = if zone == 0 && t.kind == NodeKind::Folder {
            (t.id, self.nodes.iter().filter(|n| n.parent == t.id).count() as i64)
        } else if zone < 0 {
            (t.parent, t.seq)
        } else {
            (t.parent, t.seq + 1)
        };
        let mut p = parent;
        while p != 0 {
            if p == id {
                self.toast_err("A folder cannot be moved into itself.");
                return;
            }
            p = self.node(p).map(|n| n.parent).unwrap_or(0);
        }
        let _ = self.db.move_node(id, parent, seq);
        // Renumber the destination's children so their order stays contiguous.
        if let Ok(all) = self.db.playlists() {
            let mut kids: Vec<_> = all.into_iter().filter(|n| n.parent == parent).collect();
            kids.sort_by_key(|n| (n.seq, n.id != id));
            for (i, n) in kids.iter().enumerate() {
                let _ = self.db.conn.execute("UPDATE playlists SET seq=?2 WHERE id=?1", rusqlite::params![n.id, i as i64]);
            }
        }
        if zone == 0 && t.kind == NodeKind::Folder {
            self.expanded.insert(t.id);
        }
        self.reload_playlists();
    }

    pub fn reload_playlists(&mut self) {
        self.nodes = self.db.playlists().unwrap_or_default();
        self.node_counts = self.db.playlist_counts().unwrap_or_default();
        self.list_dirty = true;
    }
    pub fn reload_tags(&mut self) {
        self.mytags = self.db.mytags().unwrap_or_default();
        self.track_tags = self.db.track_tags().unwrap_or_default();
        self.list_dirty = true;
    }

    pub fn track(&self, id: i64) -> Option<&Track> {
        self.index.get(&id).map(|i| &self.tracks[*i])
    }
    pub fn track_mut(&mut self, id: i64) -> Option<&mut Track> {
        let i = *self.index.get(&id)?;
        Some(&mut self.tracks[i])
    }
    pub fn save_track(&mut self, id: i64) {
        if let Some(t) = self.track(id).cloned() {
            let mut t = t;
            if t.analyzed == 3 {
                t.analyzed = 0;
            }
            let _ = self.db.update_track(&t);
        }
        self.list_dirty = true;
    }

    pub fn node(&self, id: i64) -> Option<&PlaylistNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    // ------------------------------------------------------------------ list
    pub fn smart_tracks(&self, rule: &SmartRule) -> Vec<i64> {
        let today = crate::smart::today_days();
        let empty = HashSet::new();
        self.tracks
            .iter()
            .filter(|t| crate::smart::matches(rule, t, self.track_tags.get(&t.id).unwrap_or(&empty), today))
            .map(|t| t.id)
            .collect()
    }

    pub fn view_tracks(&self) -> Vec<i64> {
        match &self.view {
            View::Collection => self.tracks.iter().map(|t| t.id).collect(),
            View::Playlist(id) => match self.node(*id) {
                Some(n) if n.kind == NodeKind::Smart => n.smart.as_ref().map(|r| self.smart_tracks(r)).unwrap_or_default(),
                Some(n) if n.kind == NodeKind::Folder => {
                    let mut out = vec![];
                    let mut seen = HashSet::new();
                    self.folder_tracks(n.id, &mut out, &mut seen);
                    out
                }
                Some(_) => self.db.playlist_tracks(*id).unwrap_or_default().into_iter().filter(|t| self.index.contains_key(t)).collect(),
                None => vec![],
            },
            View::Device { .. } | View::DeviceHistory { .. } | View::Explorer(_) => vec![],
            View::Related => self.related_tracks(),
            View::Category { field, value } => self
                .tracks
                .iter()
                .filter(|t| &Self::category_value(t, *field) == value)
                .map(|t| t.id)
                .collect(),
        }
    }

    /// The value a track has for one Collection category (Date Added is grouped by day).
    pub fn category_value(t: &Track, field: u8) -> String {
        match field {
            0 => t.date_added.get(..10).unwrap_or(&t.date_added).to_string(),
            1 => t.genre.trim().to_string(),
            2 => t.artist.trim().to_string(),
            _ => t.album.trim().to_string(),
        }
    }

    /// Track the Related Tracks list is built around: the loaded track, else the selection.
    pub fn related_source(&self) -> Option<i64> {
        self.deck.track.or_else(|| self.anchor_id).or_else(|| self.selected.iter().next().copied())
    }

    /// Tracks matching the Related Tracks rules for the source track, best matches first.
    pub fn related_tracks(&self) -> Vec<i64> {
        let Some(src) = self.related_source().and_then(|id| self.track(id)).cloned() else { return vec![] };
        let r = &self.prefs.related;
        let mut out: Vec<(i64, i64)> = self
            .tracks
            .iter()
            .filter(|t| t.id != src.id && !t.missing)
            .filter_map(|t| {
                let mut score = 0i64;
                if r.key > 0 {
                    let d = key_distance(src.key, t.key)?;
                    if d > r.key as i32 - 1 {
                        return None;
                    }
                    score += (4 - d as i64) * 100;
                }
                if r.bpm {
                    if src.bpm <= 0.0 || t.bpm <= 0.0 {
                        return None;
                    }
                    let best = [t.bpm, t.bpm * 2.0, t.bpm / 2.0].iter().map(|b| ((b - src.bpm) / src.bpm * 100.0).abs()).fold(f64::MAX, f64::min);
                    if best > r.bpm_pct {
                        return None;
                    }
                    score += (100.0 - best * 10.0).max(0.0) as i64;
                }
                let same = |a: &str, b: &str| !a.trim().is_empty() && a.trim().eq_ignore_ascii_case(b.trim());
                if r.genre && !same(&src.genre, &t.genre) {
                    return None;
                }
                if r.artist && !same(&src.artist, &t.artist) {
                    return None;
                }
                if r.label && !same(&src.label, &t.label) {
                    return None;
                }
                if r.color && (src.color == 0 || src.color != t.color) {
                    return None;
                }
                if r.rating && t.rating < src.rating {
                    return None;
                }
                Some((t.id, score))
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out.into_iter().map(|x| x.0).collect()
    }

    fn folder_tracks(&self, id: i64, out: &mut Vec<i64>, seen: &mut HashSet<i64>) {
        for n in self.nodes.iter().filter(|n| n.parent == id) {
            match n.kind {
                NodeKind::Folder => self.folder_tracks(n.id, out, seen),
                NodeKind::Smart => {
                    for t in n.smart.as_ref().map(|r| self.smart_tracks(r)).unwrap_or_default() {
                        if seen.insert(t) {
                            out.push(t);
                        }
                    }
                }
                NodeKind::Playlist => {
                    for t in self.db.playlist_tracks(n.id).unwrap_or_default() {
                        if self.index.contains_key(&t) && seen.insert(t) {
                            out.push(t);
                        }
                    }
                }
            }
        }
    }

    pub fn is_manual_playlist(&self) -> Option<i64> {
        match self.view {
            View::Playlist(id) => self.node(id).filter(|n| n.kind == NodeKind::Playlist).map(|n| n.id),
            _ => None,
        }
    }

    pub fn tag_text(&self, id: i64) -> String {
        let names: HashMap<i64, &str> = self.mytags.iter().map(|t| (t.id, t.name.as_str())).collect();
        let mut v: Vec<&str> = self.track_tags.get(&id).map(|s| s.iter().filter_map(|x| names.get(x).copied()).collect()).unwrap_or_default();
        v.sort();
        v.join(", ")
    }

    pub fn rebuild_list(&mut self) {
        self.related_built_for = self.related_source();
        let base = self.view_tracks();
        let order: HashMap<i64, usize> = base.iter().enumerate().map(|(i, t)| (*t, i)).collect();
        let q = crate::smart::fold(self.search.trim());
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut v: Vec<i64> = base
            .into_iter()
            .filter(|id| {
                let Some(t) = self.track(*id) else { return false };
                if !words.is_empty() {
                    let hay = match self.prefs.search_scope {
                        1 => crate::smart::fold(&t.display_title()),
                        2 => crate::smart::fold(&format!("{} {} {}", t.artist, t.remixer, t.original_artist)),
                        3 => crate::smart::fold(&t.album),
                        4 => crate::smart::fold(&t.genre),
                        5 => crate::smart::fold(&t.label),
                        6 => crate::smart::fold(&t.comment),
                        7 => crate::smart::fold(&self.tag_text(t.id)),
                        _ => crate::smart::fold(&format!(
                        "{} {} {} {} {} {} {} {} {} {} {}",
                        t.display_title(),
                        t.artist,
                        t.album,
                        t.genre,
                        t.label,
                        t.remixer,
                        t.comment,
                        t.composer,
                        t.mix_name,
                        key_name(t.key, self.prefs.key_alnum),
                        self.tag_text(t.id)
                    )),
                    };
                    if !words.iter().all(|w| hay.contains(w)) {
                        return false;
                    }
                }
                if self.filter_open {
                    let (lo, hi) = self.filter_bpm;
                    if lo > 0.0 && t.bpm < lo {
                        return false;
                    }
                    if hi > 0.0 && t.bpm > hi {
                        return false;
                    }
                    if self.filter_key >= 0 && !compatible_key(self.filter_key, t.key) {
                        return false;
                    }
                    if self.filter_rating > 0 && t.rating < self.filter_rating {
                        return false;
                    }
                    if self.filter_color > 0 && t.color != self.filter_color {
                        return false;
                    }
                    if !self.filter_tags.is_empty() {
                        let empty = HashSet::new();
                        let tt = self.track_tags.get(&t.id).unwrap_or(&empty);
                        let ok = if self.filter_tags_all { self.filter_tags.iter().all(|x| tt.contains(x)) } else { self.filter_tags.iter().any(|x| tt.contains(x)) };
                        if !ok {
                            return false;
                        }
                    }
                }
                true
            })
            .collect();
        let col = self.prefs.sort_col;
        let asc = self.prefs.sort_asc;
        let alnum = self.prefs.key_alnum;
        let tagtexts: HashMap<i64, String> = if col == Col::MyTag { v.iter().map(|id| (*id, self.tag_text(*id))).collect() } else { HashMap::new() };
        v.sort_by(|a, b| {
            let ta = self.track(*a).unwrap();
            let tb = self.track(*b).unwrap();
            use std::cmp::Ordering;
            let tc = |x: &str, y: &str| crate::smart::fold(x).cmp(&crate::smart::fold(y));
            let fc = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
            let o = match col {
                Col::Num | Col::Status | Col::Preview | Col::Artwork => order[a].cmp(&order[b]),
                Col::Title => tc(&ta.display_title(), &tb.display_title()),
                Col::Artist => tc(&ta.artist, &tb.artist),
                Col::Album => tc(&ta.album, &tb.album),
                Col::Genre => tc(&ta.genre, &tb.genre),
                Col::Bpm => fc(ta.bpm, tb.bpm),
                Col::Rating => ta.rating.cmp(&tb.rating),
                Col::Time => fc(ta.duration, tb.duration),
                Col::Key => {
                    if alnum {
                        (ta.key % 12, ta.key / 12).cmp(&(tb.key % 12, tb.key / 12))
                    } else {
                        key_name(ta.key, false).cmp(&key_name(tb.key, false))
                    }
                }
                Col::DateAdded => ta.date_added.cmp(&tb.date_added).then(ta.id.cmp(&tb.id)),
                Col::Color => ta.color.cmp(&tb.color),
                Col::Comments => tc(&ta.comment, &tb.comment),
                Col::Label => tc(&ta.label, &tb.label),
                Col::Remixer => tc(&ta.remixer, &tb.remixer),
                Col::OriginalArtist => tc(&ta.original_artist, &tb.original_artist),
                Col::Composer => tc(&ta.composer, &tb.composer),
                Col::Year => ta.year.cmp(&tb.year),
                Col::Bitrate => ta.bitrate.cmp(&tb.bitrate),
                Col::SampleRate => ta.sample_rate.cmp(&tb.sample_rate),
                Col::FileType => ta.file_type.cmp(&tb.file_type),
                Col::Size => ta.file_size.cmp(&tb.file_size),
                Col::Location => ta.path.cmp(&tb.path),
                Col::FileName => tc(&ta.file_name(), &tb.file_name()),
                Col::PlayCount => ta.play_count.cmp(&tb.play_count),
                Col::MixName => tc(&ta.mix_name, &tb.mix_name),
                Col::TrackNumber => ta.track_no.cmp(&tb.track_no),
                Col::MyTag => tagtexts.get(a).cmp(&tagtexts.get(b)),
                Col::ReleaseDate => ta.release_date.cmp(&tb.release_date),
                Col::AlbumArtist => tc(&ta.album_artist, &tb.album_artist),
                Col::DiscNumber => ta.disc_no.cmp(&tb.disc_no),
                Col::Bitdepth => ta.bit_depth.cmp(&tb.bit_depth),
                Col::Grouping => tc(&ta.grouping, &tb.grouping),
                Col::Lyricist => tc(&ta.lyricist, &tb.lyricist),
                Col::Message => tc(&ta.message, &tb.message),
                Col::HotCue => self.cue_counts.get(a).map(|c| c.0).cmp(&self.cue_counts.get(b).map(|c| c.0)),
            };
            let o = if asc { o } else { o.reverse() };
            o.then(order[a].cmp(&order[b]))
        });
        self.list = v;
        let idx = &self.index;
        self.selected.retain(|id| idx.contains_key(id));
        self.anchor = self.anchor_id.and_then(|id| self.list.iter().position(|x| *x == id));
        self.list_dirty = false;
    }

    /// Point the selection anchor at row `i` of the current list.
    pub fn set_anchor(&mut self, i: usize) {
        self.anchor = Some(i);
        self.anchor_id = self.list.get(i).copied();
    }

    pub fn set_view(&mut self, v: View) {
        if self.view != v {
            self.view = v;
            let manual = matches!(self.view, View::Playlist(id) if self.node(id).map(|n| n.kind == NodeKind::Playlist).unwrap_or(false));
            if manual {
                self.prefs.sort_col = Col::Num;
                self.prefs.sort_asc = true;
                if !self.prefs.columns.contains(&Col::Num) {
                    self.prefs.columns.insert(0, Col::Num);
                }
            } else {
                self.prefs.columns.retain(|c| *c != Col::Num);
                if self.prefs.sort_col == Col::Num {
                    self.prefs.sort_col = Col::DateAdded;
                    self.prefs.sort_asc = false;
                }
            }
            self.list_dirty = true;
            self.anchor = None;
            self.anchor_id = None;
        }
    }

    // ------------------------------------------------------------------ import
    pub fn import_paths(&mut self, paths: Vec<std::path::PathBuf>, to_playlist: Option<i64>) {
        if paths.is_empty() {
            return;
        }
        self.import_to_playlist = to_playlist;
        self.import_count = 0;
        self.importing = Some(spawn_import(paths, self.ctx.clone()));
    }

    pub fn poll_import(&mut self) {
        let Some(rx) = self.importing.clone() else { return };
        let mut added = vec![];
        let mut finished = false;
        while let Ok(m) = rx.try_recv() {
            match m {
                Some(t) => {
                    if let Some(id) = self.db.track_by_path(&t.path) {
                        added.push(id);
                        continue;
                    }
                    if let Ok(id) = self.db.insert_track(&t) {
                        added.push(id);
                        self.import_count += 1;
                    }
                }
                None => finished = true,
            }
        }
        if !added.is_empty() {
            self.reload_tracks();
            if let Some(pl) = self.import_to_playlist {
                let mut cur = self.db.playlist_tracks(pl).unwrap_or_default();
                for a in &added {
                    if !cur.contains(a) {
                        cur.push(*a);
                    }
                }
                let _ = self.db.set_playlist_tracks(pl, &cur);
                self.reload_playlists();
            }
            if self.prefs.auto_analysis {
                let todo: Vec<i64> = added.iter().copied().filter(|id| self.track(*id).map(|t| t.analyzed == 0).unwrap_or(false)).collect();
                self.analyze(&todo, false);
            }
        }
        if finished {
            self.importing = None;
            let n = self.import_count;
            self.toast(format!("{n} track(s) added to the collection"));
            if let Some(p) = self.load_after_import.take() {
                if let Some(id) = self.db.track_by_path(&p) {
                    self.load_track(id);
                }
            }
        }
    }

    /// Copy rekordbox's library into PrepDeck's data folder and read the copy, off the UI thread.
    /// rekordbox's own files are only read (byte copy), never opened by a database or written.
    pub fn start_rekordbox_sync(&mut self) {
        if self.rb_job.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        let dest = crate::db::data_dir().join("rekordbox-copy");
        std::thread::spawn(move || {
            let _ = tx.send(crate::rbimport::load_installed(&dest));
            ctx.request_repaint();
        });
        self.rb_job = Some(rx);
    }

    pub fn poll_rekordbox_sync(&mut self) {
        let Some(rx) = &self.rb_job else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.rb_job = None;
        match r {
            Ok(Some(lib)) => self.apply_rekordbox(lib),
            Ok(None) => {}
            Err(e) => self.toast_err(format!("Could not read the rekordbox library: {e}. It was not changed.")),
        }
    }

    /// Merge a rekordbox library read from a snapshot. New tracks are added; a track that
    /// rekordbox changed since the last sync gets rekordbox's metadata, grid, cues and My Tags;
    /// one it did not change keeps any edits made in PrepDeck. The playlist tree is mirrored
    /// into the "rekordbox" folder, which is rebuilt on every sync.
    pub fn apply_rekordbox(&mut self, lib: crate::rbimport::RbLibrary) {
        // One transaction for the whole sync (much faster, and all-or-nothing).
        let _ = self.db.conn.execute_batch("BEGIN");
        let mut seen: HashMap<String, String> = self.db.setting("rb_sync_fingerprints").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let first = seen.is_empty();
        // My Tag categories and tags, matched by name.
        let mut tag_id: HashMap<(String, String), i64> = HashMap::new();
        let mut rb_tag_ids: HashSet<i64> = HashSet::new();
        let existing = self.db.mytags().unwrap_or_default();
        for (cat, tags) in &lib.tag_categories {
            let cid = match existing.iter().find(|t| t.parent == 0 && &t.name == cat) {
                Some(t) => t.id,
                None => self.db.add_mytag(0, cat).unwrap_or(0),
            };
            for tg in tags {
                let tid = match existing.iter().find(|t| t.parent == cid && &t.name == tg) {
                    Some(t) => t.id,
                    None => self.db.add_mytag(cid, tg).unwrap_or(0),
                };
                tag_id.insert((cat.clone(), tg.clone()), tid);
                rb_tag_ids.insert(tid);
            }
        }
        let (mut added, mut updated) = (0, 0);
        let mut idmap: HashMap<String, i64> = HashMap::new();
        for rt in lib.tracks {
            let changed = seen.get(&rt.rb_id) != Some(&rt.fingerprint);
            let id = match self.db.track_by_path(&rt.xt.track.path) {
                None => {
                    let mut t = rt.xt.track.clone();
                    if t.sample_rate == 0 || t.duration <= 0.0 {
                        let real = crate::importer::read_track(std::path::Path::new(&t.path));
                        t.sample_rate = real.sample_rate;
                        t.bit_depth = real.bit_depth;
                        if t.duration <= 0.0 {
                            t.duration = real.duration;
                        }
                        if t.artwork.is_empty() {
                            t.artwork = real.artwork;
                        }
                    }
                    let Ok(id) = self.db.insert_track(&t) else { continue };
                    added += 1;
                    id
                }
                Some(id) if changed => {
                    if let Some(cur) = self.track(id).cloned() {
                        let r = &rt.xt.track;
                        let t = Track {
                            id,
                            analyzed: cur.analyzed,
                            grid_locked: cur.grid_locked,
                            gain_db: cur.gain_db,
                            raw_samples: cur.raw_samples,
                            missing: cur.missing,
                            artwork: if r.artwork.is_empty() { cur.artwork.clone() } else { r.artwork.clone() },
                            sample_rate: if r.sample_rate > 0 { r.sample_rate } else { cur.sample_rate },
                            bit_depth: if r.bit_depth > 0 { r.bit_depth } else { cur.bit_depth },
                            duration: if r.duration > 0.0 { r.duration } else { cur.duration },
                            ..r.clone()
                        };
                        let _ = self.db.update_track(&t);
                        updated += 1;
                    }
                    id
                }
                Some(id) => id,
            };
            if changed {
                if let Some(g) = &rt.xt.grid {
                    let _ = self.db.save_grid(id, g);
                }
                let _ = self.db.replace_cues(id, &rt.xt.cues);
                let mine: HashSet<i64> = rt.tags.iter().filter_map(|k| tag_id.get(k).copied()).collect();
                for t in &rb_tag_ids {
                    let _ = self.db.set_track_tag(id, *t, mine.contains(t));
                }
            }
            seen.insert(rt.rb_id.clone(), rt.fingerprint.clone());
            idmap.insert(rt.rb_id, id);
        }
        self.db.set_setting("rb_sync_fingerprints", &serde_json::to_string(&seen).unwrap_or_default());
        // Playlists: rebuild the mirrored folder.
        if let Some(old) = self.db.setting("rb_folder_id").and_then(|s| s.parse::<i64>().ok()) {
            if self.db.playlists().map(|v| v.iter().any(|n| n.id == old)).unwrap_or(false) {
                let _ = self.db.delete_node(old);
            }
        }
        fn add_nodes(app: &mut App, parent: i64, nodes: &[crate::importer::XmlNode], idmap: &HashMap<String, i64>) {
            for n in nodes {
                if n.folder {
                    let id = app.db.create_node(parent, &n.name, NodeKind::Folder, None).unwrap_or(0);
                    add_nodes(app, id, &n.children, idmap);
                } else {
                    let id = app.db.create_node(parent, &n.name, NodeKind::Playlist, None).unwrap_or(0);
                    let tr: Vec<i64> = n.track_keys.iter().filter_map(|k| idmap.get(k).copied()).collect();
                    let _ = app.db.set_playlist_tracks(id, &tr);
                }
            }
        }
        if !lib.playlists.is_empty() {
            let root = self.db.create_node(0, "rekordbox", NodeKind::Folder, None).unwrap_or(0);
            self.db.set_setting("rb_folder_id", &root.to_string());
            add_nodes(self, root, &lib.playlists, &idmap);
        }
        let _ = self.db.conn.execute_batch("COMMIT");
        self.reload_all();
        let mut msg = if first {
            format!("Loaded your rekordbox library: {added} tracks, with cues, grids, My Tags and playlists. rekordbox's own files were not changed.")
        } else if added + updated > 0 {
            format!("Synced from rekordbox: {added} new, {updated} updated.")
        } else {
            String::new()
        };
        if first && lib.not_local > 0 {
            msg.push_str(&format!(" {} track(s) without a local file (streaming) were left out.", lib.not_local));
        }
        if first && lib.smart_lists > 0 {
            msg.push_str(&format!(" {} intelligent playlist(s) were not copied.", lib.smart_lists));
        }
        if !msg.is_empty() {
            self.toast(msg);
        }
        if added > 0 && self.prefs.auto_analysis {
            // Waveforms and loudness are PrepDeck's own; grids and keys from rekordbox are kept.
            let todo: Vec<i64> = self.tracks.iter().filter(|t| t.analyzed == 0 && !t.missing).map(|t| t.id).collect();
            self.analyze(&todo, false);
        }
    }

    pub fn import_xml(&mut self, path: &std::path::Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return self.toast_err(format!("Could not read the file: {e}")),
        };
        let lib = match crate::importer::parse_rekordbox_xml(&text) {
            Ok(l) => l,
            Err(e) => return self.toast_err(format!("This is not a readable collection file: {e}")),
        };
        let mut idmap: HashMap<String, i64> = HashMap::new();
        let mut added = 0;
        for xt in lib.tracks {
            let id = match self.db.track_by_path(&xt.track.path) {
                Some(id) => id,
                None => {
                    let mut t = xt.track.clone();
                    let real = crate::importer::read_track(std::path::Path::new(&t.path));
                    if real.sample_rate > 0 {
                        t.sample_rate = real.sample_rate;
                        t.bit_depth = real.bit_depth;
                        t.artwork = real.artwork;
                        if t.duration <= 0.0 {
                            t.duration = real.duration;
                        }
                    }
                    if t.date_added.is_empty() {
                        t.date_added = real.date_added;
                    }
                    match self.db.insert_track(&t) {
                        Ok(id) => {
                            added += 1;
                            id
                        }
                        Err(_) => continue,
                    }
                }
            };
            if let Some(g) = &xt.grid {
                let _ = self.db.save_grid(id, g);
            }
            if !xt.cues.is_empty() {
                let _ = self.db.replace_cues(id, &xt.cues);
            }
            idmap.insert(xt.xml_id.clone(), id);
        }
        fn add_nodes(app: &mut App, parent: i64, nodes: &[crate::importer::XmlNode], idmap: &HashMap<String, i64>) {
            for n in nodes {
                if n.folder {
                    let id = app.db.create_node(parent, &n.name, NodeKind::Folder, None).unwrap_or(0);
                    add_nodes(app, id, &n.children, idmap);
                } else {
                    let id = app.db.create_node(parent, &n.name, NodeKind::Playlist, None).unwrap_or(0);
                    let tr: Vec<i64> = n.track_keys.iter().filter_map(|k| idmap.get(k).copied()).collect();
                    let _ = app.db.set_playlist_tracks(id, &tr);
                }
            }
        }
        if !lib.playlists.is_empty() {
            let root = self.db.create_node(0, "Imported collection", NodeKind::Folder, None).unwrap_or(0);
            add_nodes(self, root, &lib.playlists, &idmap);
        }
        self.reload_all();
        self.toast(format!("Imported {added} new track(s) with their cues, grids and playlists. Original files were not changed."));
        if self.prefs.auto_analysis {
            let todo: Vec<i64> = self.tracks.iter().filter(|t| t.analyzed == 0 && !t.missing).map(|t| t.id).collect();
            self.analyze(&todo, false);
        }
    }

    // ------------------------------------------------------------------ analysis
    pub fn analyze(&mut self, ids: &[i64], force: bool) {
        let s = self.analysis_settings();
        let mut n = 0;
        for id in ids {
            let Some(t) = self.track_mut(*id) else { continue };
            // Analysis Lock: a locked track is never re-analysed (its grid, key, waveform and
            // gain stay as they are), even by "Re-analyze".
            if t.missing || (t.analyzed == 3) || (!force && t.analyzed == 1) || (t.grid_locked && t.analyzed == 1) {
                continue;
            }
            t.analyzed = 3;
            let path = t.path.clone();
            n += 1;
            self.pool.submit(AnalysisJob { track_id: *id, path, settings: s.clone() });
        }
        self.analysis_total += n;
        self.list_dirty = true;
    }

    pub fn poll_analysis(&mut self) {
        while let Ok((id, r)) = self.pool.rx.try_recv() {
            self.analysis_done += 1;
            self.apply_analysis(id, r);
        }
        if self.pool.pending.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            self.analysis_total = 0;
            self.analysis_done = 0;
        }
    }

    pub fn apply_analysis(&mut self, id: i64, r: Result<crate::analysis::AnalysisResult, String>) {
        let existing_grid = self.db.grid(id).filter(|g| !g.is_empty());
        let Some(t) = self.track_mut(id) else { return };
        match r {
            Err(e) => {
                t.analyzed = if e == "stopped" { 0 } else { 2 };
            }
            Ok(res) if t.grid_locked && t.analyzed == 1 => {
                // Locked while the job was running: keep everything that was there.
                let _ = res;
            }
            Ok(res) => {
                let first_analysis = t.analyzed != 1;
                t.analyzed = 1;
                t.duration = res.duration;
                t.sample_rate = res.sample_rate;
                t.raw_samples = res.raw_samples;
                t.gain_db = res.gain_db;
                // A locked grid, or one that came with an imported library, is kept as it is.
                let keep_grid = t.grid_locked || existing_grid.is_some();
                // A key the user set (or one from an imported library) is only replaced when the
                // track had no key, or on its first analysis when the key came from tags.
                if res.key >= 0 && (t.key < 0 || (first_analysis && !keep_grid)) {
                    t.key = res.key;
                }
                let mut save_grid = None;
                if !keep_grid {
                    if let Some(g) = res.grid {
                        t.bpm = g.first_bpm();
                        save_grid = Some(g);
                    }
                } else if let Some(g) = &existing_grid {
                    if g.first_bpm() > 0.0 {
                        t.bpm = g.first_bpm();
                    }
                }
                let preview: Vec<u8> = res.wave.columns(400).iter().flat_map(|c| c.iter().copied()).collect();
                let _ = self.db.save_wave(id, &res.wave, &preview);
                self.previews.insert(id, preview);
                if let Some(g) = save_grid {
                    let _ = self.db.save_grid(id, &g);
                }
                if self.deck.track == Some(id) {
                    self.deck.wave = Some(res.wave);
                    if self.deck.audio.is_none() {
                        self.deck.duration = res.duration;
                    }
                    if let Some(g) = self.db.grid(id) {
                        self.set_deck_grid(g, false);
                    }
                }
            }
        }
        self.save_track(id);
    }

    /// Re-analyse ignoring the stored grid (explicit "Analyze Track" from the menu).
    pub fn reanalyze(&mut self, ids: &[i64]) {
        for id in ids {
            let locked = self.track(*id).map(|t| t.grid_locked).unwrap_or(true);
            if !locked {
                let _ = self.db.conn.execute("DELETE FROM grids WHERE track_id=?1", [id]);
            }
        }
        self.analyze(ids, true);
    }

    // ------------------------------------------------------------------ player
    pub fn pos(&self) -> f64 {
        let s = self.player.state.lock();
        match &s.audio {
            Some(a) => s.pos / a.sr as f64,
            None => 0.0,
        }
    }
    pub fn playing(&self) -> bool {
        self.player.state.lock().playing
    }
    pub fn seek(&mut self, t: f64) {
        let t = t.clamp(0.0, self.deck.duration.max(0.0));
        let mut s = self.player.state.lock();
        if let Some(a) = &s.audio {
            s.pos = t * a.sr as f64;
        }
    }
    pub fn set_playing(&mut self, p: bool) {
        self.player.state.lock().playing = p && self.deck.audio.is_some();
    }

    pub fn load_track(&mut self, id: i64) {
        let Some(t) = self.track(id).cloned() else { return };
        if t.missing {
            self.toast_err(format!("The file for \"{}\" cannot be found.", t.display_title()));
            return;
        }
        let (old_pos, old_gain) = {
            let mut s = self.player.state.lock();
            let keep = (s.pos, s.gain);
            s.playing = false;
            s.audio = None;
            s.pos = 0.0;
            s.loop_range = None;
            s.metronome = None;
            s.gain = 10f32.powf(t.gain_db / 20.0).min(4.0);
            keep
        };
        let ctrl = self.deck.ctrl_tab;
        let panel = self.deck.panel_tab;
        let old = std::mem::replace(&mut self.deck, Deck { ctrl_tab: ctrl, panel_tab: panel, ..Deck::default() });
        // Keep the last successfully loaded deck (not a deck that is itself still loading).
        if old.audio.is_some() {
            self.deck_backup = Some((old, old_pos, old_gain));
        }
        self.deck.track = Some(id);
        self.deck.duration = t.duration;
        self.deck.wave = self.db.wave(id);
        self.deck.cues = self.db.cues(id).unwrap_or_default();
        let g = self.db.grid(id).unwrap_or_default();
        self.set_deck_grid(g, false);
        self.deck.bpm_text = format!("{:.2}", self.deck.grid.first_bpm());
        let first_mem = self.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).map(|c| c.start).fold(f64::NAN, f64::min);
        self.deck.cue_point = if first_mem.is_nan() { self.deck.beats.first().map(|b| b.time).unwrap_or(0.0) } else { first_mem };
        self.deck.loading = Some(spawn_load(id, t.path.clone(), self.ctx.clone()));
        if t.analyzed == 0 {
            self.analyze(&[id], false);
        }
    }

    pub fn poll_load(&mut self) {
        let Some(rx) = &self.deck.loading else { return };
        if let Ok((id, r)) = rx.try_recv() {
            self.deck.loading = None;
            if self.deck.track != Some(id) {
                return;
            }
            match r {
                Ok(a) => {
                    self.deck_backup = None;
                    self.deck.duration = a.duration();
                    let cp = self.deck.cue_point;
                    let mut s = self.player.state.lock();
                    s.pos = cp * a.sr as f64;
                    s.audio = Some(a.clone());
                    drop(s);
                    self.deck.audio = Some(a);
                    let g = self.deck.grid.clone();
                    self.set_deck_grid(g, false);
                }
                Err(e) => {
                    if let Some((old, pos, gain)) = self.deck_backup.take() {
                        let mut s = self.player.state.lock();
                        s.audio = old.audio.clone();
                        s.pos = pos;
                        s.gain = gain;
                        drop(s);
                        self.deck = old;
                    }
                    self.toast_err(format!("This file cannot be played: {e}"));
                }
            }
        }
    }

    pub fn set_deck_grid(&mut self, g: BeatGrid, save: bool) {
        let dur = if self.deck.duration > 0.0 { self.deck.duration } else { self.deck.track.and_then(|id| self.track(id)).map(|t| t.duration).unwrap_or(0.0) };
        self.deck.beats = g.beats(dur);
        self.deck.grid = g;
        self.deck.bpm_text = format!("{:.2}", self.deck.grid.first_bpm());
        if save {
            if let Some(id) = self.deck.track {
                let _ = self.db.save_grid(id, &self.deck.grid);
                let bpm = self.deck.grid.first_bpm();
                if let Some(t) = self.track_mut(id) {
                    t.bpm = bpm;
                }
                self.save_track(id);
            }
        }
        self.update_metronome();
    }

    pub fn update_metronome(&mut self) {
        let beats = if self.deck.metronome { Some(Arc::new(self.deck.beats.iter().map(|b| b.time).collect::<Vec<f64>>())) } else { None };
        self.player.state.lock().metronome = beats;
    }

    pub fn quantized(&self, t: f64) -> f64 {
        if self.prefs.quantize && !self.deck.beats.is_empty() {
            nearest_beat(&self.deck.beats, t).map(|b| b.time).unwrap_or(t)
        } else {
            t
        }
    }

    pub fn current_bpm(&self) -> f64 {
        let p = self.pos();
        let i = self.deck.beats.partition_point(|b| b.time <= p);
        if i > 0 {
            self.deck.beats[i - 1].bpm
        } else {
            self.deck.beats.first().map(|b| b.bpm).or(self.deck.track.and_then(|id| self.track(id)).map(|t| t.bpm)).unwrap_or(0.0)
        }
    }

    pub fn beat_len(&self) -> f64 {
        let b = self.current_bpm();
        if b > 0.0 {
            60.0 / b
        } else {
            0.5
        }
    }

    pub fn cue_down(&mut self) {
        if self.deck.audio.is_none() {
            return;
        }
        if self.playing() {
            self.set_playing(false);
            let c = self.deck.cue_point;
            self.seek(c);
            self.player.state.lock().loop_range = None;
        } else {
            let p = self.pos();
            if (p - self.deck.cue_point).abs() < 0.01 {
                self.deck.cue_preview = true;
                self.set_playing(true);
            } else {
                let q = self.quantized(p);
                self.deck.cue_point = q;
                self.seek(q);
            }
        }
    }
    pub fn cue_up(&mut self) {
        if self.deck.cue_preview {
            self.deck.cue_preview = false;
            self.set_playing(false);
            let c = self.deck.cue_point;
            self.seek(c);
        }
    }
    pub fn play_pause(&mut self) {
        if self.deck.audio.is_none() {
            return;
        }
        if self.deck.cue_preview {
            self.deck.cue_preview = false;
            return;
        }
        let p = !self.playing();
        self.set_playing(p);
    }

    pub fn hot_cue_down(&mut self, slot: i32) {
        let Some(tid) = self.deck.track else { return };
        if self.deck.audio.is_none() {
            return;
        }
        if let Some(c) = self.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).cloned() {
            self.seek(c.start);
            if let Some(e) = c.end.filter(|e| *e > c.start) {
                self.player.state.lock().loop_range = Some((c.start, e));
                self.deck.last_loop = Some((c.start, e));
            }
            if !self.playing() {
                self.deck.hot_preview = Some(c.start);
                self.set_playing(true);
            }
        } else {
            let lr = self.player.state.lock().loop_range;
            let (start, end, color) = match lr {
                Some((a, b)) => (a, Some(b), hotcue_default_color(slot, true)),
                None => (self.quantized(self.pos()), None, hotcue_default_color(slot, false)),
            };
            let mut c = Cue { id: 0, track_id: tid, kind: CueKind::Hot, slot, start, end, color, comment: String::new(), active_loop: false };
            if let Ok(id) = self.db.save_cue(&c) {
                c.id = id;
                self.deck.cues.push(c);
                self.refresh_cue_counts(tid);
            }
        }
    }
    pub fn hot_cue_up(&mut self) {
        if let Some(start) = self.deck.hot_preview.take() {
            self.set_playing(false);
            self.player.state.lock().loop_range = None;
            self.seek(start);
        }
    }
    pub fn delete_cue(&mut self, cue_id: i64) {
        let _ = self.db.delete_cue(cue_id);
        self.deck.cues.retain(|c| c.id != cue_id);
        if let Some(t) = self.deck.track {
            self.refresh_cue_counts(t);
        }
    }
    pub fn update_cue(&mut self, c: Cue) {
        let _ = self.db.save_cue(&c);
        if let Some(x) = self.deck.cues.iter_mut().find(|x| x.id == c.id) {
            *x = c;
        }
    }
    pub fn refresh_cue_counts(&mut self, tid: i64) {
        let hp: Vec<(f64, i32, u8)> = self.deck.cues.iter().filter(|c| c.kind == CueKind::Hot).map(|c| (c.start, c.slot, c.color)).collect();
        if hp.is_empty() {
            self.hot_positions.remove(&tid);
        } else {
            self.hot_positions.insert(tid, hp);
        }
        let h = self.deck.cues.iter().filter(|c| c.kind == CueKind::Hot).count();
        let m = self.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).count();
        if h + m == 0 {
            self.cue_counts.remove(&tid);
        } else {
            self.cue_counts.insert(tid, (h, m));
        }
    }

    pub fn memory_set(&mut self) {
        let Some(tid) = self.deck.track else { return };
        let lr = self.player.state.lock().loop_range;
        let (start, end) = match lr {
            Some((a, b)) => (a, Some(b)),
            None => (if self.playing() { self.quantized(self.pos()) } else { self.deck.cue_point }, None),
        };
        if self.deck.cues.iter().any(|c| c.kind == CueKind::Memory && (c.start - start).abs() < 0.001 && c.end == end) {
            return;
        }
        let mut c = Cue { id: 0, track_id: tid, kind: CueKind::Memory, slot: -1, start, end, color: 0, comment: String::new(), active_loop: false };
        if let Ok(id) = self.db.save_cue(&c) {
            c.id = id;
            self.deck.cues.push(c);
            self.refresh_cue_counts(tid);
        }
    }
    pub fn memory_cues(&self) -> Vec<Cue> {
        let mut v: Vec<Cue> = self.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).cloned().collect();
        v.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        v
    }
    pub fn memory_jump(&mut self, dir: i32) {
        let p = if self.playing() { self.pos() } else { self.deck.cue_point };
        let m = self.memory_cues();
        let target = if dir > 0 { m.iter().find(|c| c.start > p + 0.005) } else { m.iter().rev().find(|c| c.start < p - 0.005) };
        if let Some(c) = target.cloned() {
            self.call_memory(&c);
        }
    }
    pub fn call_memory(&mut self, c: &Cue) {
        self.deck.cue_point = c.start;
        self.seek(c.start);
        if let Some(e) = c.end.filter(|e| *e > c.start) {
            self.deck.last_loop = Some((c.start, e));
            if c.active_loop {
                self.player.state.lock().loop_range = Some((c.start, e));
            }
        }
    }
    pub fn memory_delete_current(&mut self) {
        let cp = self.deck.cue_point;
        if let Some(c) = self.deck.cues.iter().find(|c| c.kind == CueKind::Memory && (c.start - cp).abs() < 0.005).map(|c| c.id) {
            self.delete_cue(c);
        }
    }

    pub fn beat_jump(&mut self, beats: f64) {
        let p = self.pos();
        let t = p + beats * self.beat_len();
        self.seek(t);
        if !self.playing() {
            self.deck.cue_point = self.pos();
        }
    }

    pub fn auto_loop(&mut self, beats: f64) {
        if self.deck.audio.is_none() {
            return;
        }
        let start = if self.prefs.quantize && !self.deck.beats.is_empty() {
            let p = self.pos();
            let i = self.deck.beats.partition_point(|b| b.time <= p + 0.001);
            if i > 0 {
                self.deck.beats[i - 1].time
            } else {
                p
            }
        } else {
            self.pos()
        };
        let end = start + beats * self.beat_len();
        self.player.state.lock().loop_range = Some((start, end));
        self.deck.last_loop = Some((start, end));
    }
    pub fn loop_active(&self) -> Option<(f64, f64)> {
        self.player.state.lock().loop_range
    }
    pub fn loop_exit(&mut self) {
        self.player.state.lock().loop_range = None;
    }
    pub fn reloop(&mut self) {
        if let Some((a, b)) = self.deck.last_loop {
            self.player.state.lock().loop_range = Some((a, b));
            self.seek(a);
        }
    }
    pub fn loop_in(&mut self) {
        let q = self.quantized(self.pos());
        self.deck.loop_in = Some(q);
    }
    pub fn loop_out(&mut self) {
        if let Some(a) = self.deck.loop_in {
            let b = self.quantized(self.pos());
            if b > a + 0.01 {
                self.player.state.lock().loop_range = Some((a, b));
                self.deck.last_loop = Some((a, b));
                self.seek(a);
            }
        }
    }
    pub fn resize_loop(&mut self, factor: f64) {
        self.prefs.loop_beats = (self.prefs.loop_beats * factor).clamp(1.0 / 64.0, 512.0);
        if let Some((a, b)) = self.loop_active() {
            let nb = a + (b - a) * factor;
            self.player.state.lock().loop_range = Some((a, nb));
            self.deck.last_loop = Some((a, nb));
        }
    }

    // ------------------------------------------------------------------ grid editing
    fn grid_editable(&mut self) -> bool {
        let Some(id) = self.deck.track else { return false };
        if self.deck.grid.is_empty() {
            self.toast_err("This track has no beat grid yet. Analyse it first.");
            return false;
        }
        if self.track(id).map(|t| t.grid_locked).unwrap_or(false) {
            self.toast_err("The beat grid is locked (Analysis Lock). Unlock it to edit.");
            return false;
        }
        true
    }
    fn push_grid(&mut self, g: BeatGrid) {
        let old = self.deck.grid.clone();
        self.deck.grid_undo.push(old);
        self.deck.grid_redo.clear();
        self.set_deck_grid(g, true);
    }
    fn anchor_at(&self, t: f64) -> usize {
        self.deck.grid.anchors.iter().rposition(|x| x.time <= t + 1e-6).unwrap_or(0)
    }
    pub fn grid_shift(&mut self, secs: f64) {
        if !self.grid_editable() {
            return;
        }
        let mut g = self.deck.grid.clone();
        let from = if self.prefs.grid_from_current { self.anchor_at(self.pos()) } else { 0 };
        for a in g.anchors.iter_mut().skip(from) {
            a.time += secs;
        }
        self.push_grid(g);
    }
    pub fn grid_scale_bpm(&mut self, f: impl Fn(f64) -> f64) {
        if !self.grid_editable() {
            return;
        }
        let mut g = self.deck.grid.clone();
        let p = self.pos();
        let Some(b) = nearest_beat(&self.deck.beats, p) else { return };
        if self.prefs.grid_from_current {
            // Keep the beat at the playhead and change the tempo from there on.
            let i = self.anchor_at(b.time);
            let bpm = f(g.anchors[i].bpm).clamp(40.0, 499.0);
            g.anchors.truncate(i + 1);
            if (g.anchors[i].time - b.time).abs() < 1e-6 {
                g.anchors[i].bpm = bpm;
            } else {
                g.anchors.push(GridAnchor { time: b.time, bpm, beat: b.beat });
            }
        } else {
            // Whole track: one tempo, keeping the beat nearest the playhead in place.
            let bpm = f(g.anchors[self.anchor_at(b.time)].bpm).clamp(40.0, 499.0);
            g = BeatGrid { anchors: vec![GridAnchor { time: b.time, bpm, beat: b.beat }] };
        }
        self.push_grid(g);
    }
    pub fn grid_set_bpm(&mut self, bpm: f64) {
        if !(40.0..=499.0).contains(&bpm) {
            self.toast_err("BPM value should be between 40 and 499.");
            return;
        }
        self.grid_scale_bpm(|_| bpm);
    }
    pub fn grid_set_downbeat(&mut self) {
        let Some(tid) = self.deck.track else { return };
        let p = self.pos();
        if self.track(tid).map(|t| t.grid_locked).unwrap_or(false) {
            self.toast_err("The beat grid is locked (Analysis Lock). Unlock it to edit.");
            return;
        }
        if self.deck.grid.is_empty() {
            let bpm = self.track(tid).map(|t| t.bpm).filter(|b| *b > 0.0).unwrap_or(120.0);
            self.push_grid(BeatGrid::constant(p, bpm, 1));
            return;
        }
        if !self.grid_editable() {
            return;
        }
        let mut g = self.deck.grid.clone();
        let i = self.anchor_at(p);
        let bpm = g.anchors[i].bpm;
        if self.prefs.grid_from_current {
            g.anchors.truncate(i + 1);
            if (g.anchors[i].time - p).abs() < 1e-6 {
                g.anchors[i] = GridAnchor { time: p, bpm, beat: 1 };
            } else {
                g.anchors.push(GridAnchor { time: p, bpm, beat: 1 });
            }
        } else {
            // Move the whole grid so that a bar's first beat lands on the playhead.
            let shift = p - g.anchors[i].time;
            for a in g.anchors.iter_mut() {
                a.time += shift;
            }
            g.anchors[i] = GridAnchor { time: p, bpm, beat: 1 };
        }
        self.push_grid(g);
    }
    pub fn grid_tap(&mut self) {
        if self.deck.track.and_then(|id| self.track(id)).map(|t| t.grid_locked).unwrap_or(false) {
            self.toast_err("The beat grid is locked (Analysis Lock). Unlock it to edit.");
            return;
        }
        let now = Instant::now();
        if let Some(l) = self.deck.taps.last() {
            if now.duration_since(*l).as_secs_f64() > 2.0 {
                self.deck.taps.clear();
            }
        }
        self.deck.taps.push(now);
        if self.deck.taps.len() >= 4 {
            let n = self.deck.taps.len();
            let span = self.deck.taps[n - 1].duration_since(self.deck.taps[0]).as_secs_f64();
            let bpm = ((n - 1) as f64 * 60.0 / span * 100.0).round() / 100.0;
            if self.deck.grid.is_empty() {
                let p = self.pos();
                self.push_grid(BeatGrid::constant(p, bpm, 1));
            } else {
                self.grid_set_bpm(bpm);
            }
        }
    }
    pub fn grid_undo(&mut self) {
        if let Some(g) = self.deck.grid_undo.pop() {
            let cur = self.deck.grid.clone();
            self.deck.grid_redo.push(cur);
            self.set_deck_grid(g, true);
        }
    }
    pub fn grid_redo(&mut self) {
        if let Some(g) = self.deck.grid_redo.pop() {
            let cur = self.deck.grid.clone();
            self.deck.grid_undo.push(cur);
            self.set_deck_grid(g, true);
        }
    }

    // ------------------------------------------------------------------ playlists
    pub fn add_to_playlist(&mut self, pl: i64, ids: &[i64]) {
        if self.node(pl).map(|n| n.kind != NodeKind::Playlist).unwrap_or(true) {
            return;
        }
        let mut cur = self.db.playlist_tracks(pl).unwrap_or_default();
        if cur.len() + ids.len() > 10000 {
            self.toast_err("Up to 10000 tracks can be added in one playlist");
            return;
        }
        cur.extend_from_slice(ids);
        let _ = self.db.set_playlist_tracks(pl, &cur);
        self.reload_playlists();
        let name = self.node(pl).map(|n| n.name.clone()).unwrap_or_default();
        self.toast(format!("Added {} track(s) to \"{name}\"", ids.len()));
    }
    pub fn remove_from_playlist(&mut self, pl: i64, ids: &HashSet<i64>) {
        let cur: Vec<i64> = self.db.playlist_tracks(pl).unwrap_or_default().into_iter().filter(|t| !ids.contains(t)).collect();
        let _ = self.db.set_playlist_tracks(pl, &cur);
        self.reload_playlists();
    }
    pub fn move_in_playlist(&mut self, pl: i64, ids: &[i64], to: usize) {
        let cur = self.db.playlist_tracks(pl).unwrap_or_default();
        let set: HashSet<i64> = ids.iter().copied().collect();
        let before = cur.iter().take(to).filter(|t| set.contains(t)).count();
        let mut rest: Vec<i64> = cur.into_iter().filter(|t| !set.contains(t)).collect();
        let at = to.saturating_sub(before).min(rest.len());
        let moving: Vec<i64> = self.list.iter().filter(|t| set.contains(t)).copied().collect();
        for (k, t) in moving.into_iter().enumerate() {
            rest.insert(at + k, t);
        }
        let _ = self.db.set_playlist_tracks(pl, &rest);
        self.reload_playlists();
    }
    pub fn create_node(&mut self, parent: i64, kind: NodeKind) -> i64 {
        if kind == NodeKind::Smart && self.nodes.iter().filter(|n| n.kind == NodeKind::Smart).count() >= 128 {
            self.toast_err("You can't create more than 128 intelligent lists.");
            return 0;
        }
        let name = match kind {
            NodeKind::Folder => "Untitled Folder",
            NodeKind::Smart => "Untitled Intelligent List",
            NodeKind::Playlist => "Untitled Playlist",
        };
        let smart = if kind == NodeKind::Smart { Some(SmartRule::default()) } else { None };
        let id = self.db.create_node(parent, name, kind.clone(), smart.as_ref()).unwrap_or(0);
        self.reload_playlists();
        if parent > 0 {
            self.expanded.insert(parent);
        }
        if kind == NodeKind::Smart {
            self.dialogs.smart_edit = Some((Some(id), parent, name.to_string(), SmartRule::default()));
        } else {
            self.dialogs.rename = Some((id, name.to_string()));
        }
        id
    }

    pub fn remove_from_collection(&mut self, ids: &[i64]) {
        for id in ids {
            let _ = self.db.delete_track(*id);
            if self.deck.track == Some(*id) {
                let mut s = self.player.state.lock();
                s.playing = false;
                s.audio = None;
                drop(s);
                self.deck = Deck::default();
            }
        }
        self.selected.clear();
        self.reload_all();
        self.toast(format!("Removed {} track(s) from the collection. The files were not deleted.", ids.len()));
    }

    pub fn set_tag(&mut self, ids: &[i64], tag: i64, on: bool) {
        for id in ids {
            let _ = self.db.set_track_tag(*id, tag, on);
            let e = self.track_tags.entry(*id).or_default();
            if on {
                e.insert(tag);
            } else {
                e.remove(&tag);
            }
        }
        self.list_dirty = true;
    }

    pub fn reload_tags_from_files(&mut self, ids: &[i64]) {
        for id in ids {
            let Some(t) = self.track(*id).cloned() else { continue };
            let mut fresh = crate::importer::read_track(std::path::Path::new(&t.path));
            fresh.id = t.id;
            fresh.date_added = t.date_added.clone();
            fresh.rating = t.rating;
            fresh.color = t.color;
            fresh.analyzed = t.analyzed;
            fresh.grid_locked = t.grid_locked;
            fresh.autoload_hotcue = t.autoload_hotcue;
            fresh.gain_db = t.gain_db;
            fresh.play_count = t.play_count;
            fresh.missing = t.missing;
            if t.analyzed == 1 {
                fresh.bpm = t.bpm;
                fresh.key = t.key;
                fresh.duration = t.duration;
            }
            if let Some(slot) = self.track_mut(*id) {
                *slot = fresh;
            }
            self.save_track(*id);
        }
    }

    // ------------------------------------------------------------------ devices / export
    pub fn scan_devices(&mut self) {
        let mut found: Vec<(String, String, String)> = vec![];
        let mount = std::process::Command::new("/sbin/mount").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        for line in mount.lines() {
            // "/dev/disk4s1 on /Volumes/USB (msdos, local, nodev, nosuid, noowners)"
            let Some((_, rest)) = line.split_once(" on ") else { continue };
            let Some((mp, opts)) = rest.rsplit_once(" (") else { continue };
            if !mp.starts_with("/Volumes/") {
                continue;
            }
            let fs = opts.split(',').next().unwrap_or("").trim().to_string();
            if !matches!(fs.as_str(), "msdos" | "exfat" | "hfs" | "apfs") || opts.contains("read-only") {
                continue;
            }
            let name = mp.trim_start_matches("/Volumes/").to_string();
            let root = std::path::Path::new(mp);
            // Skip app installer images unless they already hold a device library.
            let installer = root.join("Applications").exists() || std::fs::read_dir(root).map(|d| d.flatten().any(|e| e.path().extension().map(|x| x == "app").unwrap_or(false))).unwrap_or(false);
            if name == "Recovery" || (installer && !root.join("PIONEER").exists()) {
                continue;
            }
            found.push((mp.to_string(), name, fs));
        }
        for extra in &self.prefs.devices_extra {
            if std::path::Path::new(extra).is_dir() {
                let name = std::path::Path::new(extra).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                found.push((extra.clone(), name, "folder".into()));
            }
        }
        let mut devices = vec![];
        for (root, name, fs) in found {
            let p = std::path::Path::new(&root);
            let (lib, error) = if crate::export::usb::pdb_path(p).exists() {
                match crate::export::usb::read_device(p) {
                    Ok(l) => (Some(l), None),
                    Err(e) => (None, Some(format!("Device library is corrupted. ({e})"))),
                }
            } else {
                (None, None)
            };
            devices.push(DeviceInfo { total: crate::export::usb::total_space(p), free: crate::export::usb::free_space(p), root, name, fs, lib, error });
        }
        self.devices = devices;
        self.devices_scanned = Some(Instant::now());
    }

    pub fn fs_label(fs: &str) -> &'static str {
        match fs {
            "msdos" => "FAT32",
            "exfat" => "exFAT",
            "hfs" => "HFS+",
            "apfs" => "APFS (not readable by DJ players)",
            _ => "Folder",
        }
    }

    pub fn start_export(&mut self, root: &str, playlist_ids: &[i64], extra_tracks: &[i64]) {
        if self.export_job.is_some() {
            self.toast_err("An export is already running.");
            return;
        }
        fn build(app: &App, id: i64) -> Option<crate::export::usb::ExportNode> {
            let n = app.node(id)?;
            match n.kind {
                NodeKind::Folder => {
                    let children: Vec<_> = app.nodes.iter().filter(|c| c.parent == id).filter_map(|c| build(app, c.id)).collect();
                    Some(crate::export::usb::ExportNode { name: n.name.clone(), folder: true, children, tracks: vec![] })
                }
                NodeKind::Smart => {
                    let tracks = n.smart.as_ref().map(|r| app.smart_tracks(r)).unwrap_or_default();
                    Some(crate::export::usb::ExportNode { name: n.name.clone(), folder: false, children: vec![], tracks })
                }
                NodeKind::Playlist => Some(crate::export::usb::ExportNode { name: n.name.clone(), folder: false, children: vec![], tracks: app.db.playlist_tracks(id).unwrap_or_default() }),
            }
        }
        // Children of a selected folder come with the folder.
        let sel: HashSet<i64> = playlist_ids.iter().copied().collect();
        let mut nodes = vec![];
        for id in playlist_ids {
            let mut p = self.node(*id).map(|n| n.parent).unwrap_or(0);
            let mut covered = false;
            while p != 0 {
                if sel.contains(&p) {
                    covered = true;
                    break;
                }
                p = self.node(p).map(|n| n.parent).unwrap_or(0);
            }
            if !covered {
                if let Some(n) = build(self, *id) {
                    nodes.push(n);
                }
            }
        }
        let mut track_ids: HashSet<i64> = extra_tracks.iter().copied().collect();
        fn collect(n: &crate::export::usb::ExportNode, out: &mut HashSet<i64>) {
            out.extend(n.tracks.iter().copied());
            for c in &n.children {
                collect(c, out);
            }
        }
        for n in &nodes {
            collect(n, &mut track_ids);
        }
        let names: HashMap<i64, (String, i64)> = self.mytags.iter().map(|t| (t.id, (t.name.clone(), t.parent))).collect();
        let mut tracks = HashMap::new();
        let mut skipped: Vec<String> = vec![];
        let settings = self.analysis_settings();
        let mut ids: Vec<i64> = track_ids.into_iter().collect();
        ids.sort();
        for id in &ids {
            let Some(t) = self.track(*id).cloned() else { continue };
            if t.missing {
                skipped.push(format!("{} (file not found)", t.display_title()));
                continue;
            }
            if !crate::export::usb::player_supported(&t.file_type) {
                skipped.push(format!("{} (file type not supported by the players)", t.display_title()));
                continue;
            }
            let tags: Vec<(String, String)> = self
                .track_tags
                .get(id)
                .map(|s| s.iter().filter_map(|x| names.get(x)).map(|(n, p)| (names.get(p).map(|c| c.0.clone()).unwrap_or_default(), n.clone())).collect())
                .unwrap_or_default();
            tracks.insert(*id, crate::export::usb::ExportTrack { cues: self.db.cues(*id).unwrap_or_default(), grid: self.db.grid(*id), wave: self.db.wave(*id), track: t, tags });
        }
        // Tracks without analysis are analysed by the export job itself (off the UI thread).
        let tag_categories: Vec<(String, Vec<String>)> = self
            .mytags
            .iter()
            .filter(|c| c.parent == 0)
            .map(|c| (c.name.clone(), self.mytags.iter().filter(|t| t.parent == c.id).map(|t| t.name.clone()).collect()))
            .collect();
        let req = crate::export::usb::ExportRequest { root: std::path::PathBuf::from(root), nodes, tracks, tag_categories, skipped, analysis: settings, settings: self.prefs.usb_settings.clone() };
        self.export_job = Some(spawn_export(req, self.ctx.clone()));
    }

    pub fn poll_export(&mut self) {
        let Some(job) = self.export_job.as_mut() else { return };
        let mut finished = None;
        let mut analyzed = vec![];
        while let Ok(p) = job.rx.try_recv() {
            match p {
                crate::export::usb::Progress::Step { done, total, text } => job.last = (done, total, text),
                crate::export::usb::Progress::Analyzed(id, r) => analyzed.push((id, *r)),
                crate::export::usb::Progress::Finished(r) => finished = Some(r),
            }
        }
        for (id, r) in analyzed {
            self.apply_analysis(id, r);
        }
        if let Some(r) = finished {
            self.export_job = None;
            match r {
                Ok(msg) => self.toast(msg),
                Err(e) => self.toast_err(e),
            }
            self.scan_devices();
        }
    }

    pub fn export_collection_xml(&mut self, path: &std::path::Path) {
        let xml = crate::xmlexport::build(self);
        match std::fs::write(path, xml) {
            Ok(_) => self.toast("Collection exported in xml format."),
            Err(e) => self.toast_err(format!("Could not write the file: {e}")),
        }
    }
}

/// Same key or harmonically related (±1 on the wheel, or relative major/minor).
pub fn compatible_key(a: i32, b: i32) -> bool {
    if a < 0 || b < 0 {
        return false;
    }
    let (na, ma) = (a % 12, a / 12);
    let (nb, mb) = (b % 12, b / 12);
    if ma == mb {
        let d = (na - nb).rem_euclid(12);
        d == 0 || d == 1 || d == 11
    } else {
        na == nb
    }
}

pub fn fmt_time(secs: f64, tenths: bool) -> String {
    let s = secs.max(0.0);
    let m = (s / 60.0).floor() as u64;
    let r = s - m as f64 * 60.0;
    if tenths {
        format!("{:02}:{:04.1}", m, (r * 10.0).floor() / 10.0)
    } else {
        format!("{:02}:{:02}", m, r.floor() as u64)
    }
}
