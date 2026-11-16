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
            Col::Num => 34.0,
            Col::Status => 40.0,
            Col::Preview => 140.0,
            Col::Artwork => 60.0,
            Col::Title => 190.0,
            Col::Artist | Col::Album => 150.0,
            Col::Genre => 110.0,
            Col::Bpm => 60.0,
            Col::Rating => 80.0,
            Col::Time => 56.0,
            Col::Key => 48.0,
            Col::DateAdded | Col::ReleaseDate => 84.0,
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
}

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

pub const ZOOMS: [f64; 6] = [4.0, 8.0, 16.0, 32.0, 64.0, 128.0];

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Collection,
    Playlist(i64),
    Device { root: String, node: Option<u32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideMode {
    Collection,
    Playlists,
    Devices,
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
    pub pool: AnalysisPool,
    pub analysis_total: usize,
    pub analysis_done: usize,
    pub importing: Option<Receiver<Option<Track>>>,
    pub import_count: usize,
    pub import_to_playlist: Option<i64>,
    pub export_job: Option<ExportJob>,
    pub devices: Vec<DeviceInfo>,
    pub devices_scanned: Option<Instant>,
    pub dialogs: Dialogs,
    pub toasts: Vec<Toast>,
    pub drag: Option<Vec<i64>>,
    pub textures: HashMap<String, Option<egui::TextureHandle>>,
    pub ctx: egui::Context,
    pub screenshot: Option<(String, u32)>,
    pub scroll_to_selected: bool,
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
            devices: vec![],
            devices_scanned: None,
            dialogs: Dialogs::default(),
            toasts: vec![],
            drag: None,
            textures: HashMap::new(),
            ctx: cc.egui_ctx.clone(),
            screenshot: screenshot.map(|s| (s, 0)),
            scroll_to_selected: false,
        };
        app.reload_all();
        if let Some(e) = app.player.error.clone() {
            app.toast_err(format!("Audio output unavailable: {e}"));
        }
        // First start: add original demo loops (like the bundled demo tracks of the original).
        if app.tracks.is_empty() && app.db.setting("demo_added").is_none() {
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
        if app.screenshot.is_some() {
            let first = app.tracks.iter().find(|t| t.analyzed == 1 && !t.missing).map(|t| t.id);
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
            View::Device { .. } => vec![],
        }
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
        let base = self.view_tracks();
        let order: HashMap<i64, usize> = base.iter().enumerate().map(|(i, t)| (*t, i)).collect();
        let q = crate::smart::fold(self.search.trim());
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut v: Vec<i64> = base
            .into_iter()
            .filter(|id| {
                let Some(t) = self.track(*id) else { return false };
                if !words.is_empty() {
                    let hay = crate::smart::fold(&format!(
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
                    ));
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
        self.list_dirty = false;
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
            if t.missing || (t.analyzed == 3) || (!force && t.analyzed == 1) {
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
            Ok(res) => {
                t.analyzed = 1;
                t.duration = res.duration;
                t.sample_rate = res.sample_rate;
                t.gain_db = res.gain_db;
                // A locked grid, or one that came with an imported library, is kept as it is.
                let keep_grid = t.grid_locked || existing_grid.is_some();
                if res.key >= 0 && (t.key < 0 || !keep_grid) {
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
        {
            let mut s = self.player.state.lock();
            s.playing = false;
            s.audio = None;
            s.pos = 0.0;
            s.loop_range = None;
            s.metronome = None;
            s.gain = 10f32.powf(t.gain_db / 20.0).min(4.0);
        }
        let ctrl = self.deck.ctrl_tab;
        let panel = self.deck.panel_tab;
        self.deck = Deck { ctrl_tab: ctrl, panel_tab: panel, ..Deck::default() };
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
                Err(e) => self.toast_err(format!("This file cannot be played: {e}")),
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
                Some((a, b)) => (a, Some(b), HOTCUE_LOOP_DEFAULT),
                None => (self.quantized(self.pos()), None, HOTCUE_DEFAULT),
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
        let mut unsupported = 0;
        let settings = self.analysis_settings();
        let mut ids: Vec<i64> = track_ids.into_iter().collect();
        ids.sort();
        for id in &ids {
            let Some(t) = self.track(*id).cloned() else { continue };
            if t.missing {
                continue;
            }
            if !crate::export::usb::player_supported(&t.file_type) {
                unsupported += 1;
                continue;
            }
            let tags: Vec<(String, String)> = self
                .track_tags
                .get(id)
                .map(|s| s.iter().filter_map(|x| names.get(x)).map(|(n, p)| (names.get(p).map(|c| c.0.clone()).unwrap_or_default(), n.clone())).collect())
                .unwrap_or_default();
            tracks.insert(*id, crate::export::usb::ExportTrack { cues: self.db.cues(*id).unwrap_or_default(), grid: self.db.grid(*id), wave: self.db.wave(*id), track: t, tags });
        }
        if unsupported > 0 {
            self.toast_err("Some of the imported files cannot be exported as they are not supported by the CDJ.");
        }
        // Tracks without analysis are analysed first (as the original does when exporting).
        let needs: Vec<(i64, String)> = tracks.values().filter(|t| t.wave.is_none() || t.grid.is_none()).map(|t| (t.track.id, t.track.path.clone())).collect();
        for (id, path) in needs {
            if let Ok(r) = crate::analysis::analyze_file(&path, &settings) {
                self.apply_analysis(id, Ok(r));
                if let Some(et) = tracks.get_mut(&id) {
                    et.grid = self.db.grid(id);
                    et.wave = self.db.wave(id);
                    if let Some(t) = self.track(id) {
                        et.track = t.clone();
                    }
                }
            }
        }
        let tag_categories: Vec<(String, Vec<String>)> = self
            .mytags
            .iter()
            .filter(|c| c.parent == 0)
            .map(|c| (c.name.clone(), self.mytags.iter().filter(|t| t.parent == c.id).map(|t| t.name.clone()).collect()))
            .collect();
        let req = crate::export::usb::ExportRequest { root: std::path::PathBuf::from(root), nodes, tracks, tag_categories };
        self.export_job = Some(spawn_export(req, self.ctx.clone()));
    }

    pub fn poll_export(&mut self) {
        let Some(job) = self.export_job.as_mut() else { return };
        let mut finished = None;
        while let Ok(p) = job.rx.try_recv() {
            match p {
                crate::export::usb::Progress::Step { done, total, text } => job.last = (done, total, text),
                crate::export::usb::Progress::Finished(r) => finished = Some(r),
            }
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
