//! USB export: keeps a device library (tracks + playlists) on the drive, merging with what is
//! already there, copies audio into /Contents, writes analysis files and rewrites
//! /PIONEER/rekordbox/export.pdb and exportExt.pdb.

use super::anlz;
use super::pdb::{self, dstr, dstr_ucs2, row_with_strings, Database, TableSpec};
use crate::model::*;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DevTrack {
    pub id: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub label: String,
    pub remixer: String,
    pub original_artist: String,
    pub composer: String,
    pub key_name: String,
    pub comment: String,
    pub mix_name: String,
    pub message: String,
    pub lyricist: String,
    pub year: u16,
    pub track_no: u32,
    pub disc_no: u16,
    pub duration: u16,
    pub bpm100: u32,
    pub rating: u8,
    pub color: u8,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub sample_depth: u16,
    pub file_size: u32,
    pub file_type: u16,
    pub file_path: String,
    pub analyze_path: String,
    pub date_added: String,
    pub release_date: String,
    pub analyze_date: String,
    pub artwork_path: String,
    pub play_count: u16,
    pub autoload: bool,
    /// Local source file for tracks this app exported (empty for foreign tracks).
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DevNode {
    pub id: u32,
    pub parent: u32,
    pub name: String,
    pub folder: bool,
    pub seq: u32,
    pub entries: Vec<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DevHistory {
    pub id: u32,
    pub name: String,
    pub entries: Vec<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceLibrary {
    pub tracks: BTreeMap<u32, DevTrack>,
    pub nodes: Vec<DevNode>,
    pub histories: Vec<DevHistory>,
    /// My Tag categories (name, tags).
    pub tag_categories: Vec<(String, Vec<String>)>,
    /// IDs the drive's exportExt.pdb already uses, keyed by `tag_key(category, tag)` (a
    /// category itself is keyed by its name). Reused on the next export so IDs stay stable.
    #[serde(default)]
    pub tag_ids: BTreeMap<String, u32>,
    /// Browse categories and sort options the players offer (Preferences > DJ System >
    /// Category / Sort). None keeps the original's defaults.
    #[serde(default)]
    pub browse: Option<UsbBrowse>,
}

/// Active browse categories and sort options, in order, by COLUMNS id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbBrowse {
    pub categories: Vec<u16>,
    pub sorts: Vec<u16>,
}

/// Categories that are always active (shown greyed in the original's Category page).
pub const FIXED_CATEGORIES: [u16; 5] = [4, 17, 19, 20, 24];
/// Sort options that are always active (DEFAULT, ALPHABET/TRACK NAME).
pub const FIXED_SORTS: [u16; 2] = [0x19, 0x1a];

impl Default for UsbBrowse {
    fn default() -> Self {
        let order = |rows: &[[u8; 8]], vis: usize, pos: usize| -> Vec<u16> {
            let mut v: Vec<(u8, u16)> = rows.iter().filter(|r| r[vis] != 1).map(|r| (r[pos], u16::from_le_bytes([r[0], r[1]]))).collect();
            v.sort();
            v.into_iter().map(|x| x.1).collect()
        };
        UsbBrowse { categories: order(&MENU, 5, 6), sorts: order(&TABLE18, 4, 5) }
    }
}

impl UsbBrowse {
    /// Every category the Category page can offer (id, name).
    pub fn all_categories() -> Vec<(u16, &'static str)> {
        MENU.iter().map(|r| u16::from_le_bytes([r[0], r[1]])).filter_map(|id| column_name(id).map(|n| (id, n))).collect()
    }
    pub fn all_sorts() -> Vec<(u16, &'static str)> {
        TABLE18.iter().map(|r| u16::from_le_bytes([r[0], r[1]])).filter_map(|id| column_name(id).map(|n| (id, n))).collect()
    }
    /// Fixed entries first, in the original's order, then the user's choices.
    pub fn normalized(&self) -> UsbBrowse {
        let mut c: Vec<u16> = self.categories.iter().copied().filter(|id| MENU.iter().any(|r| u16::from_le_bytes([r[0], r[1]]) == *id)).collect();
        for f in FIXED_CATEGORIES {
            if !c.contains(&f) {
                c.push(f);
            }
        }
        let mut s: Vec<u16> = FIXED_SORTS.to_vec();
        s.extend(self.sorts.iter().copied().filter(|id| !FIXED_SORTS.contains(id) && TABLE18.iter().any(|r| u16::from_le_bytes([r[0], r[1]]) == *id)));
        c.dedup();
        s.dedup();
        UsbBrowse { categories: c, sorts: s }
    }
}

pub fn column_name(id: u16) -> Option<&'static str> {
    COLUMNS.iter().find(|c| c.0 == id).map(|c| c.2)
}

/// Key for a tag inside its category (tags with the same name in two categories differ).
pub fn tag_key(category: &str, tag: &str) -> String {
    format!("{category}\u{1f}{tag}")
}

pub fn file_type_code(ext: &str) -> u16 {
    match ext.to_ascii_lowercase().as_str() {
        "mp3" => 1,
        "m4a" | "aac" | "mp4" | "alac" => 4,
        "flac" => 5,
        "wav" => 11,
        "aif" | "aiff" => 12,
        _ => 0,
    }
}

pub fn player_supported(ext: &str) -> bool {
    file_type_code(ext) != 0
}

// ------------------------------------------------------------------------------------------
// Reading an existing device library
// ------------------------------------------------------------------------------------------

pub fn pdb_path(root: &Path) -> PathBuf {
    root.join("PIONEER").join("rekordbox").join("export.pdb")
}

fn sidecar_path(root: &Path) -> PathBuf {
    root.join(".PrepDeck").join("device.json")
}

pub fn read_device(root: &Path) -> Result<DeviceLibrary> {
    let p = pdb_path(root);
    if !p.exists() {
        return Ok(DeviceLibrary::default());
    }
    let mut lib = parse_pdb(std::fs::read(&p)?)?;
    // Sources recorded by this app (so re-exports update instead of duplicating).
    if let Ok(s) = std::fs::read_to_string(sidecar_path(root)) {
        if let Ok(m) = serde_json::from_str::<HashMap<String, String>>(&s) {
            for t in lib.tracks.values_mut() {
                if let Some(src) = m.get(&t.file_path) {
                    t.source = src.clone();
                }
            }
        }
    }
    let ext = root.join("PIONEER").join("rekordbox").join("exportExt.pdb");
    if let Ok(b) = std::fs::read(ext) {
        let _ = read_ext_tags(b, &mut lib);
    }
    Ok(lib)
}

pub fn parse_pdb(bytes: Vec<u8>) -> Result<DeviceLibrary> {
    let db = Database::parse(bytes)?;
    let mut names: HashMap<(u32, u32), String> = HashMap::new();
    // artists (2)
    for (h, o) in db.rows(2) {
        let r = db.row_bytes(h, o);
        let sub = pdb::rd_u16(r, 0);
        let id = pdb::rd_u32(r, 4);
        let name = if sub & 4 != 0 { pdb::row_string(r, 8, 0, true) } else { pdb::row_string(r, 8, 0, false) };
        names.insert((2, id), name);
    }
    let mut album_artist: HashMap<u32, u32> = HashMap::new();
    for (h, o) in db.rows(3) {
        let r = db.row_bytes(h, o);
        let sub = pdb::rd_u16(r, 0);
        let artist = pdb::rd_u32(r, 8);
        let id = pdb::rd_u32(r, 12);
        let name = pdb::row_string(r, 20, 0, sub & 4 != 0);
        names.insert((3, id), name);
        album_artist.insert(id, artist);
    }
    for t in [1u32, 4] {
        for (h, o) in db.rows(t) {
            let r = db.row_bytes(h, o);
            names.insert((t, pdb::rd_u32(r, 0)), pdb::read_dstr(r, 4));
        }
    }
    for (h, o) in db.rows(5) {
        let r = db.row_bytes(h, o);
        names.insert((5, pdb::rd_u32(r, 0)), pdb::read_dstr(r, 8));
    }
    for (h, o) in db.rows(13) {
        let r = db.row_bytes(h, o);
        names.insert((13, pdb::rd_u32(r, 0)), pdb::read_dstr(r, 4));
    }
    let nm = |t: u32, id: u32| -> String { if id == 0 { String::new() } else { names.get(&(t, id)).cloned().unwrap_or_default() } };
    let mut lib = DeviceLibrary::default();
    for (h, o) in db.rows(0) {
        let r = db.row_bytes(h, o);
        if r.len() < 0x5e {
            continue;
        }
        let wide = pdb::rd_u16(r, 0) & 4 != 0;
        let s = |i: usize| pdb::row_string(r, 0x5c, i, wide);
        let album_id = pdb::rd_u32(r, 0x40);
        let t = DevTrack {
            id: pdb::rd_u32(r, 0x48),
            sample_rate: pdb::rd_u32(r, 0x08),
            composer: nm(2, pdb::rd_u32(r, 0x0c)),
            file_size: pdb::rd_u32(r, 0x10),
            artwork_path: nm(13, pdb::rd_u32(r, 0x1c)),
            key_name: nm(5, pdb::rd_u32(r, 0x20)),
            original_artist: nm(2, pdb::rd_u32(r, 0x24)),
            label: nm(4, pdb::rd_u32(r, 0x28)),
            remixer: nm(2, pdb::rd_u32(r, 0x2c)),
            bitrate: pdb::rd_u32(r, 0x30),
            track_no: pdb::rd_u32(r, 0x34),
            bpm100: pdb::rd_u32(r, 0x38),
            genre: nm(1, pdb::rd_u32(r, 0x3c)),
            album: nm(3, album_id),
            album_artist: album_artist.get(&album_id).map(|a| nm(2, *a)).unwrap_or_default(),
            artist: nm(2, pdb::rd_u32(r, 0x44)),
            disc_no: pdb::rd_u16(r, 0x4c),
            play_count: pdb::rd_u16(r, 0x4e),
            year: pdb::rd_u16(r, 0x50),
            sample_depth: pdb::rd_u16(r, 0x52),
            duration: pdb::rd_u16(r, 0x54),
            color: r[0x58],
            rating: r[0x59],
            file_type: pdb::rd_u16(r, 0x5a),
            lyricist: s(1),
            message: s(5),
            autoload: s(7) == "ON",
            date_added: s(10),
            release_date: s(11),
            mix_name: s(12),
            analyze_path: s(14),
            analyze_date: s(15),
            comment: s(16),
            title: s(17),
            file_path: s(20),
            ..Default::default()
        };
        lib.tracks.insert(t.id, t);
    }
    for (h, o) in db.rows(7) {
        let r = db.row_bytes(h, o);
        lib.nodes.push(DevNode {
            parent: pdb::rd_u32(r, 0),
            seq: pdb::rd_u32(r, 8),
            id: pdb::rd_u32(r, 12),
            folder: pdb::rd_u32(r, 16) != 0,
            name: pdb::read_dstr(r, 20),
            entries: vec![],
        });
    }
    let mut entries: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for (h, o) in db.rows(8) {
        let r = db.row_bytes(h, o);
        entries.entry(pdb::rd_u32(r, 8)).or_default().push((pdb::rd_u32(r, 0), pdb::rd_u32(r, 4)));
    }
    for n in lib.nodes.iter_mut() {
        if let Some(mut e) = entries.remove(&n.id) {
            e.sort();
            n.entries = e.into_iter().map(|x| x.1).filter(|t| lib.tracks.contains_key(t)).collect();
        }
    }
    for (h, o) in db.rows(11) {
        let r = db.row_bytes(h, o);
        lib.histories.push(DevHistory { id: pdb::rd_u32(r, 0), name: pdb::read_dstr(r, 4), entries: vec![] });
    }
    let mut hent: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for (h, o) in db.rows(12) {
        let r = db.row_bytes(h, o);
        hent.entry(pdb::rd_u32(r, 4)).or_default().push((pdb::rd_u32(r, 8), pdb::rd_u32(r, 0)));
    }
    for hh in lib.histories.iter_mut() {
        if let Some(mut e) = hent.remove(&hh.id) {
            e.sort();
            hh.entries = e.into_iter().map(|x| x.1).collect();
        }
    }
    Ok(lib)
}

fn read_ext_tags(bytes: Vec<u8>, lib: &mut DeviceLibrary) -> Result<()> {
    let db = Database::parse(bytes)?;
    struct T {
        parent: u32,
        pos: u32,
        id: u32,
        cat: bool,
        name: String,
    }
    let mut tags = vec![];
    for (h, o) in db.rows(3) {
        let r = db.row_bytes(h, o);
        let wide = pdb::rd_u16(r, 0) & 4 != 0;
        tags.push(T {
            parent: pdb::rd_u32(r, 12),
            pos: pdb::rd_u32(r, 16),
            id: pdb::rd_u32(r, 20),
            cat: r.get(27).copied().unwrap_or(0) == 1,
            name: pdb::row_string(r, 0x1c, 0, wide),
        });
    }
    let cat_name: HashMap<u32, String> = tags.iter().filter(|t| t.cat).map(|t| (t.id, t.name.clone())).collect();
    let by_id: HashMap<u32, String> = tags
        .iter()
        .filter(|t| !t.cat)
        .map(|t| (t.id, tag_key(cat_name.get(&t.parent).map(String::as_str).unwrap_or(""), &t.name)))
        .collect();
    for t in &tags {
        let key = if t.cat { t.name.clone() } else { by_id.get(&t.id).cloned().unwrap_or_default() };
        lib.tag_ids.insert(key, t.id);
    }
    let mut cats: Vec<&T> = tags.iter().filter(|t| t.cat).collect();
    cats.sort_by_key(|t| t.pos);
    lib.tag_categories = cats
        .iter()
        .map(|c| {
            let mut kids: Vec<&T> = tags.iter().filter(|t| !t.cat && t.parent == c.id).collect();
            kids.sort_by_key(|t| t.pos);
            (c.name.clone(), kids.iter().map(|t| t.name.clone()).collect())
        })
        .collect();
    for (h, o) in db.rows(4) {
        let r = db.row_bytes(h, o);
        let track = pdb::rd_u32(r, 4);
        let tag = pdb::rd_u32(r, 8);
        if let (Some(t), Some(n)) = (lib.tracks.get_mut(&track), by_id.get(&tag)) {
            t.tags.push(n.clone());
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Paths on the drive (rules recovered from the original, see MECHANICS.md)
// ------------------------------------------------------------------------------------------

fn sanitize(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let s: String = s.nfc().collect();
    let mut out: String = s.chars().map(|c| if "/\\:?*\"<>|~".contains(c) || c.is_control() { '_' } else { c }).collect();
    out = out.trim().to_string();
    if out.ends_with('.') {
        out.pop();
        out.push('_');
    }
    out
}

fn cap(s: &str, n: usize) -> String {
    let mut v: String = s.chars().take(n).collect();
    while v.ends_with('.') || v.ends_with(' ') {
        v.pop();
    }
    v
}

pub fn media_dir(artist: &str, album: &str) -> String {
    let mut a = cap(&sanitize(artist), 48);
    let mut b = cap(&sanitize(album), 48);
    if a.is_empty() {
        a = "UnknownArtist".into();
    }
    if b.is_empty() {
        b = "UnknownAlbum".into();
    }
    format!("/Contents/{a}/{b}")
}

fn file_candidate(stem: &str, ext: &str, suffix: Option<u32>) -> String {
    let tail = match suffix {
        Some(n) => format!("-{n}"),
        None => String::new(),
    };
    let dot_ext = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    let room = 48usize.saturating_sub(tail.chars().count() + dot_ext.chars().count()).max(1);
    let st = cap(stem, room);
    format!("{st}{tail}{dot_ext}")
}

// ------------------------------------------------------------------------------------------
// Export job
// ------------------------------------------------------------------------------------------

/// One local track ready to go to the device.
#[derive(Clone)]
pub struct ExportTrack {
    pub track: Track,
    pub cues: Vec<Cue>,
    pub grid: Option<BeatGrid>,
    pub wave: Option<Waveform>,
    pub tags: Vec<(String, String)>, // (category, tag)
    /// rekordbox's own analysis files for this audio file (base path without extension).
    pub rb_anlz: Option<PathBuf>,
    /// The track's grid is still the one rekordbox analysed (phrases/extended grid stay valid).
    pub grid_same: bool,
}

/// A playlist (or folder) to create on the device, depth-first order.
#[derive(Clone)]
pub struct ExportNode {
    pub name: String,
    pub folder: bool,
    pub children: Vec<ExportNode>,
    pub tracks: Vec<i64>,
}

pub enum Progress {
    Step { done: usize, total: usize, text: String },
    /// A track that had no analysis was analysed on the export thread; the app stores it.
    Analyzed(i64, Box<Result<crate::analysis::AnalysisResult, String>>),
    Finished(Result<String, String>),
}

pub struct ExportRequest {
    pub root: PathBuf,
    pub nodes: Vec<ExportNode>,
    pub tracks: HashMap<i64, ExportTrack>,
    pub tag_categories: Vec<(String, Vec<String>)>,
    /// Tracks the app left out before the job started, with the reason (shown in the result).
    pub skipped: Vec<String>,
    /// Settings for analysing tracks that have no grid or waveform yet.
    pub analysis: crate::analysis::AnalysisSettings,
    /// Player settings files written to /PIONEER (MYSETTING.DAT etc.).
    pub settings: super::settings::UsbSettings,
    /// Browse categories / sort options for the players' menus.
    pub browse: UsbBrowse,
}

/// Files this export created. Unless the export commits, they are removed again, so a
/// stopped or failed export leaves the drive as it was.
struct Created {
    files: Vec<PathBuf>,
    committed: bool,
}

impl Drop for Created {
    fn drop(&mut self) {
        if !self.committed {
            for f in self.files.iter().rev() {
                let _ = std::fs::remove_file(f);
            }
        }
    }
}

/// rekordbox's analysis-file numbering: list the folder's files whose name starts with
/// "ANLZ" (any case), parse the rest of the stem as hex (non-hex characters ignored, 32-bit
/// wrap), take the signed maximum (start -1) and add one, formatted with at least 4 digits.
/// See rekordbox-rea reports/usb-export-paths.md.
fn parse_anlz_suffix(stem_after_anlz: &str) -> i32 {
    let mut v: u32 = 0;
    for c in stem_after_anlz.chars() {
        if let Some(d) = c.to_digit(16) {
            v = v.wrapping_mul(16).wrapping_add(d);
        }
    }
    v as i32
}

fn next_anlz_name(dir: &Path, reserved: Option<&i32>) -> String {
    let mut max: i32 = -1;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.len() >= 4 && name[..4].eq_ignore_ascii_case("ANLZ") {
                let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let v = parse_anlz_suffix(stem.get(4..).unwrap_or(""));
                max = max.max(v);
            }
        }
    }
    if let Some(r) = reserved {
        max = max.max(*r);
    }
    format!("ANLZ{:04X}", max.wrapping_add(1) as u32)
}

/// "/PIONEER/USBANLZ/P016/0000875E/ANLZ0000.DAT" -> "/PIONEER/USBANLZ/P016/0000875E/ANLZ0000".
fn anlz_base(analyze_path: &str) -> Option<String> {
    let p = analyze_path.strip_suffix(".DAT").or_else(|| analyze_path.strip_suffix(".dat"))?;
    if p.starts_with("/PIONEER/USBANLZ/") || p.starts_with("/.PIONEER/USBANLZ/") {
        Some(p.to_string())
    } else {
        None
    }
}

/// Copy the drive's current databases to /.PrepDeck/backup before they are replaced,
/// keeping the five most recent copies.
fn backup_device_db(root: &Path) {
    let src = root.join("PIONEER").join("rekordbox");
    if !src.join("export.pdb").exists() {
        return;
    }
    let dir = root.join(".PrepDeck").join("backup");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    for f in ["export.pdb", "exportExt.pdb"] {
        if src.join(f).exists() {
            let _ = std::fs::copy(src.join(f), dir.join(format!("{stamp}-{f}")));
        }
    }
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut stamps: Vec<String> = rd.flatten().filter_map(|e| e.file_name().to_string_lossy().split('-').take(2).collect::<Vec<_>>().join("-").into()).collect();
        stamps.sort();
        stamps.dedup();
        while stamps.len() > 5 {
            let old = stamps.remove(0);
            for f in ["export.pdb", "exportExt.pdb"] {
                let _ = std::fs::remove_file(dir.join(format!("{old}-{f}")));
            }
        }
    }
}

fn sync_dir(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Copy a media file to the drive via a temporary name, flush it to the device and check
/// that the full length arrived before giving it its final name.
fn copy_durable(src: &Path, dest: &Path, created: &mut Created) -> Result<()> {
    let tmp = dest.with_file_name(format!("{}.part", dest.file_name().unwrap_or_default().to_string_lossy()));
    created.files.push(tmp.clone());
    let n = std::fs::copy(src, &tmp)?;
    std::fs::File::options().write(true).open(&tmp)?.sync_all()?;
    let want = std::fs::metadata(src)?.len();
    if n != want || std::fs::metadata(&tmp)?.len() != want {
        return Err(anyhow!("copy of {} is incomplete ({n} of {want} bytes)", src.display()));
    }
    std::fs::rename(&tmp, dest)?;
    created.files.pop();
    created.files.push(dest.to_path_buf());
    if let Some(p) = dest.parent() {
        sync_dir(p);
    }
    Ok(())
}

fn short_list(items: &[String], n: usize) -> String {
    let mut s = items.iter().take(n).cloned().collect::<Vec<_>>().join("; ");
    if items.len() > n {
        s.push_str(&format!("; and {} more", items.len() - n));
    }
    s
}

pub fn run_export(mut req: ExportRequest, progress: &dyn Fn(Progress), cancel: &std::sync::atomic::AtomicBool) -> Result<String> {
    use std::sync::atomic::Ordering::Relaxed;
    let root = req.root.clone();
    if !root.is_dir() {
        return Err(anyhow!("The device is not available"));
    }
    let mut lib = match read_device(&root) {
        Ok(l) => l,
        Err(e) => return Err(anyhow!("Device library is corrupted ({e}). Nothing was changed.")),
    };
    let mut skipped: Vec<String> = std::mem::take(&mut req.skipped);

    // Tracks without analysis are analysed first (as the original does when exporting),
    // here on the export thread so the window stays responsive.
    let needs: Vec<i64> = req.tracks.values().filter(|t| t.wave.is_none() || t.grid.is_none()).map(|t| t.track.id).collect();
    let n_needs = needs.len();
    for (i, id) in needs.into_iter().enumerate() {
        if cancel.load(Relaxed) {
            return Err(anyhow!("Export was stopped. The device library was not changed."));
        }
        let Some(et) = req.tracks.get_mut(&id) else { continue };
        progress(Progress::Step { done: i, total: n_needs, text: format!("Analyzing {}", et.track.display_title()) });
        let r = crate::analysis::analyze_file(&et.track.path, &req.analysis).map_err(|e| e.to_string());
        if let Ok(res) = &r {
            if et.grid.is_none() {
                et.grid = res.grid.clone();
            }
            if et.wave.is_none() {
                et.wave = Some(res.wave.clone());
            }
            et.track.raw_samples = res.raw_samples;
            et.track.sample_rate = res.sample_rate;
            if et.track.duration <= 0.0 {
                et.track.duration = res.duration;
            }
            if et.track.bpm <= 0.0 {
                et.track.bpm = res.bpm;
            }
        }
        progress(Progress::Analyzed(id, Box::new(r)));
    }

    // Space check.
    let need: u64 = req
        .tracks
        .values()
        .filter(|t| !lib.tracks.values().any(|d| d.source == t.track.path))
        .map(|t| std::fs::metadata(&t.track.path).map(|m| m.len()).unwrap_or(0))
        .sum();
    if let Some(free) = free_space(&root) {
        if need + 50_000_000 > free {
            return Err(anyhow!("Not enough space available on the device"));
        }
    }
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let total = req.tracks.len();
    let mut created = Created { files: vec![], committed: false };
    let mut map: HashMap<i64, u32> = HashMap::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    for t in lib.tracks.values() {
        if !t.source.is_empty() {
            sources.insert(t.file_path.clone(), t.source.clone());
        }
    }
    // Analysis numbers handed out during this export, per folder, so two new tracks that
    // share a folder get different numbers before either file exists.
    let mut reserved: HashMap<String, i32> = HashMap::new();
    let mut next_id = lib.tracks.keys().max().copied().unwrap_or(0) + 1;
    let mut ordered: Vec<&ExportTrack> = req.tracks.values().collect();
    ordered.sort_by_key(|t| t.track.id);
    for (i, et) in ordered.iter().enumerate() {
        if cancel.load(Relaxed) {
            return Err(anyhow!("Export was stopped. The device library was not changed, and files copied by this export were removed."));
        }
        let t = &et.track;
        progress(Progress::Step { done: i, total, text: format!("Transferring {}", t.display_title()) });
        let src = Path::new(&t.path);
        if !src.exists() {
            skipped.push(format!("{} (file not found)", t.display_title()));
            continue;
        }
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
        if !player_supported(&ext) {
            skipped.push(format!("{} (file type not supported by the players)", t.display_title()));
            continue;
        }
        let src_len = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
        if src_len > u32::MAX as u64 {
            skipped.push(format!("{} (larger than 4 GB, too big for a FAT32 drive)", t.display_title()));
            continue;
        }
        // Already on the device?
        let existing = lib.tracks.values().find(|d| d.source == t.path).map(|d| (d.id, d.file_path.clone()));
        let (dev_id, usb_path) = match existing {
            Some((id, p)) if root.join(p.trim_start_matches('/')).exists() => (id, p),
            _ => {
                let dir = media_dir(&t.artist, &t.album);
                let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
                let stem = sanitize(stem);
                let mut suffix = None;
                let mut n = 0;
                let usb = loop {
                    let cand = format!("{dir}/{}", file_candidate(&stem, &ext, suffix));
                    if !root.join(cand.trim_start_matches('/')).exists() {
                        break cand;
                    }
                    n += 1;
                    suffix = Some(n);
                };
                let dest = root.join(usb.trim_start_matches('/'));
                std::fs::create_dir_all(dest.parent().unwrap())?;
                copy_durable(src, &dest, &mut created)?;
                let id = next_id;
                next_id += 1;
                (id, usb)
            }
        };
        sources.insert(usb_path.clone(), t.path.clone());
        // Analysis files: a track already on the device keeps its analysis path; a new one
        // gets the next free number in its hashed folder.
        let full_len = if t.raw_samples > 0 && t.sample_rate > 0 { t.raw_samples as f64 / t.sample_rate as f64 } else { t.duration };
        let beats = et.grid.as_ref().map(|g| g.beats(full_len.max(t.duration))).unwrap_or_default();
        let wave = et.wave.clone().unwrap_or_default();
        let ov = et.rb_anlz.as_ref().map(|b| {
            let mut sections = std::collections::HashMap::new();
            for e in ["2EX", "EXT", "DAT"] {
                sections.extend(anlz::read_sections(&b.with_extension(e)));
            }
            anlz::Overrides { sections, grid_same: et.grid_same }
        });
        let files = anlz::build_with(&usb_path, &beats, &wave, &et.cues, t.raw_samples, t.sample_rate, ov.as_ref());
        let prev = lib.tracks.get(&dev_id).cloned().unwrap_or_default();
        let base = match anlz_base(&prev.analyze_path) {
            Some(b) => b,
            None => {
                let folder = anlz::anlz_folder(&usb_path);
                let dir = root.join(folder.trim_start_matches('/'));
                let name = next_anlz_name(&dir, reserved.get(&folder));
                reserved.insert(folder.clone(), parse_anlz_suffix(&name[4..]));
                format!("{folder}/{name}")
            }
        };
        let base_fs = root.join(base.trim_start_matches('/'));
        std::fs::create_dir_all(base_fs.parent().unwrap())?;
        for (ext_name, data) in [("DAT", &files.dat), ("EXT", &files.ext), ("2EX", &files.ex2)] {
            let path = base_fs.with_extension(ext_name);
            let fresh = !path.exists();
            write_atomic(&path, data)?;
            if fresh {
                created.files.push(path);
            }
        }
        // Artwork.
        let artwork_path = if !t.artwork.is_empty() && Path::new(&t.artwork).exists() {
            write_artwork(&root, dev_id, &t.artwork).unwrap_or_default()
        } else {
            String::new()
        };
        let dt = DevTrack {
            id: dev_id,
            title: t.display_title(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_artist: t.album_artist.clone(),
            genre: t.genre.clone(),
            label: t.label.clone(),
            remixer: t.remixer.clone(),
            original_artist: t.original_artist.clone(),
            composer: t.composer.clone(),
            key_name: if t.key >= 0 { key_name(t.key, false) } else { String::new() },
            comment: t.comment.clone(),
            mix_name: t.mix_name.clone(),
            message: t.message.clone(),
            lyricist: t.lyricist.clone(),
            year: t.year.clamp(0, 9999) as u16,
            track_no: t.track_no.max(0) as u32,
            disc_no: t.disc_no.max(0) as u16,
            duration: t.duration.round().clamp(0.0, 65535.0) as u16,
            bpm100: et.grid.as_ref().map(|g| g.first_bpm()).filter(|b| *b > 0.0).unwrap_or(t.bpm).mul_add(100.0, 0.0).round() as u32,
            rating: t.rating.min(5),
            color: t.color.min(8),
            bitrate: t.bitrate,
            sample_rate: t.sample_rate,
            sample_depth: if t.bit_depth > 0 { t.bit_depth as u16 } else { 16 },
            file_size: std::fs::metadata(root.join(usb_path.trim_start_matches('/'))).map(|m| m.len()).unwrap_or(src_len).min(u32::MAX as u64) as u32,
            file_type: file_type_code(&ext),
            file_path: usb_path.clone(),
            analyze_path: format!("{base}.DAT"),
            date_added: if t.date_added.len() >= 10 { t.date_added[..10].to_string() } else { today.clone() },
            release_date: t.release_date.clone(),
            analyze_date: today.clone(),
            artwork_path: if artwork_path.is_empty() { prev.artwork_path.clone() } else { artwork_path },
            play_count: prev.play_count,
            autoload: t.autoload_hotcue,
            source: t.path.clone(),
            tags: et.tags.iter().map(|(c, x)| tag_key(c, x)).collect(),
        };
        lib.tracks.insert(dev_id, dt);
        map.insert(t.id, dev_id);
    }

    // Playlists: merge by path of names (same name under same parent is replaced).
    progress(Progress::Step { done: total, total, text: "Write to the device".into() });
    for n in &req.nodes {
        merge_node(&mut lib, 0, n, &map);
    }
    if lib.nodes.len() > 1000 {
        return Err(anyhow!("The number of lists and folders cannot exceed 1000 on one USB device."));
    }
    // My Tag categories: the app's current configuration.
    if !req.tag_categories.is_empty() {
        lib.tag_categories = req.tag_categories.clone();
    }
    if cancel.load(Relaxed) {
        return Err(anyhow!("Export was stopped. The device library was not changed, and files copied by this export were removed."));
    }
    lib.browse = Some(req.browse.clone());
    backup_device_db(&root);
    write_device(&root, &lib)?;
    created.committed = true;
    // Player settings, as rekordbox writes them with every export.
    for (name, bytes) in req.settings.files() {
        let _ = write_atomic(&root.join("PIONEER").join(name), &bytes);
    }
    // Sidecar with local sources (never read by players).
    let _ = std::fs::create_dir_all(root.join(".PrepDeck"));
    let _ = write_atomic(&sidecar_path(&root), &serde_json::to_vec_pretty(&sources).unwrap_or_default());
    let mut msg = format!("Export completed: {} tracks, {} playlists on the device", lib.tracks.len(), lib.nodes.iter().filter(|n| !n.folder).count());
    if !skipped.is_empty() {
        msg.push_str(&format!(". {} track(s) were not exported: {}", skipped.len(), short_list(&skipped, 5)));
    }
    Ok(msg)
}

fn merge_node(lib: &mut DeviceLibrary, parent: u32, n: &ExportNode, map: &HashMap<i64, u32>) {
    let existing = lib.nodes.iter().position(|x| x.parent == parent && x.name == n.name && x.folder == n.folder);
    let id = match existing {
        Some(i) => lib.nodes[i].id,
        None => {
            let id = lib.nodes.iter().map(|x| x.id).max().unwrap_or(0) + 1;
            let seq = lib.nodes.iter().filter(|x| x.parent == parent).count() as u32;
            lib.nodes.push(DevNode { id, parent, name: n.name.clone(), folder: n.folder, seq, entries: vec![] });
            id
        }
    };
    if !n.folder {
        let entries: Vec<u32> = n.tracks.iter().filter_map(|t| map.get(t).copied()).collect();
        if let Some(node) = lib.nodes.iter_mut().find(|x| x.id == id) {
            node.entries = entries;
        }
    }
    for c in &n.children {
        merge_node(lib, id, c, map);
    }
}

/// Remove a playlist/folder from the device library (and optionally its tracks).
pub fn remove_node(root: &Path, node_id: u32, delete_tracks: bool) -> Result<()> {
    let mut lib = read_device(root)?;
    let mut doomed = vec![node_id];
    let mut i = 0;
    while i < doomed.len() {
        let p = doomed[i];
        doomed.extend(lib.nodes.iter().filter(|n| n.parent == p).map(|n| n.id));
        i += 1;
    }
    let removed_tracks: Vec<u32> = lib.nodes.iter().filter(|n| doomed.contains(&n.id)).flat_map(|n| n.entries.clone()).collect();
    lib.nodes.retain(|n| !doomed.contains(&n.id));
    if delete_tracks {
        for t in removed_tracks {
            let still = lib.nodes.iter().any(|n| n.entries.contains(&t));
            if !still {
                if let Some(dt) = lib.tracks.remove(&t) {
                    let _ = std::fs::remove_file(root.join(dt.file_path.trim_start_matches('/')));
                    if let Some(base) = anlz_base(&dt.analyze_path) {
                        let b = root.join(base.trim_start_matches('/'));
                        for e in ["DAT", "EXT", "2EX"] {
                            let _ = std::fs::remove_file(b.with_extension(e));
                        }
                    }
                }
            }
        }
    }
    backup_device_db(root);
    write_device(root, &lib)
}

/// Write via a temporary file that is flushed to the device before it replaces `path`.
fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    write_tmp(path, data)?;
    publish_tmp(path)
}

fn tmp_for(path: &Path) -> PathBuf {
    path.with_file_name(format!("{}.tmp_write", path.file_name().unwrap_or_default().to_string_lossy()))
}

fn write_tmp(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(tmp_for(path))?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

fn publish_tmp(path: &Path) -> Result<()> {
    std::fs::rename(tmp_for(path), path)?;
    if let Some(p) = path.parent() {
        sync_dir(p);
    }
    Ok(())
}

fn write_artwork(root: &Path, id: u32, src: &str) -> Result<String> {
    let img = image::open(src)?;
    let rel = format!("/PIONEER/Artwork/00001/a{id}.jpg");
    let dir = root.join("PIONEER/Artwork/00001");
    std::fs::create_dir_all(&dir)?;
    let small = img.resize_to_fill(80, 80, image::imageops::FilterType::Triangle).to_rgb8();
    small.save(dir.join(format!("a{id}.jpg")))?;
    let big = img.resize_to_fill(240, 240, image::imageops::FilterType::Triangle).to_rgb8();
    big.save(dir.join(format!("a{id}_m.jpg")))?;
    Ok(rel)
}

pub fn free_space(root: &Path) -> Option<u64> {
    let out = std::process::Command::new("/bin/df").arg("-k").arg(root).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    cols.get(3)?.parse::<u64>().ok().map(|k| k * 1024)
}

pub fn total_space(root: &Path) -> Option<u64> {
    let out = std::process::Command::new("/bin/df").arg("-k").arg(root).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    cols.get(1)?.parse::<u64>().ok().map(|k| k * 1024)
}

// ------------------------------------------------------------------------------------------
// Database writing
// ------------------------------------------------------------------------------------------

struct Interner {
    ids: HashMap<String, u32>,
    order: Vec<String>,
}
impl Interner {
    fn new() -> Self {
        Interner { ids: HashMap::new(), order: vec![] }
    }
    fn id(&mut self, s: &str) -> u32 {
        if s.trim().is_empty() {
            return 0;
        }
        if let Some(i) = self.ids.get(s) {
            return *i;
        }
        self.order.push(s.to_string());
        let id = self.order.len() as u32;
        self.ids.insert(s.to_string(), id);
        id
    }
}

/// Static rows every export carries (browse categories and menus), as observed in real exports.
const COLUMNS: [(u16, u16, &str); 27] = [
    (1, 0x80, "GENRE"),
    (2, 0x81, "ARTIST"),
    (3, 0x82, "ALBUM"),
    (4, 0x83, "TRACK"),
    (5, 0x85, "BPM"),
    (6, 0x86, "RATING"),
    (7, 0x87, "YEAR"),
    (8, 0x88, "REMIXER"),
    (9, 0x89, "LABEL"),
    (10, 0x8a, "ORIGINAL ARTIST"),
    (11, 0x8b, "KEY"),
    (12, 0x8d, "CUE"),
    (13, 0x8e, "COLOR"),
    (14, 0x92, "TIME"),
    (15, 0x93, "BITRATE"),
    (16, 0x94, "FILE NAME"),
    (17, 0x84, "PLAYLIST"),
    (18, 0x98, "HOT CUE BANK"),
    (19, 0x95, "HISTORY"),
    (20, 0x91, "SEARCH"),
    (21, 0x96, "COMMENTS"),
    (22, 0x8c, "DATE ADDED"),
    (23, 0x97, "DJ PLAY COUNT"),
    (24, 0x90, "FOLDER"),
    (25, 0xa1, "DEFAULT"),
    (26, 0xa2, "ALPHABET"),
    (27, 0xaa, "MATCHING"),
];
const MENU: [[u8; 8]; 22] = [
    [1, 0, 1, 0, 0x63, 1, 0, 0],
    [5, 0, 6, 0, 5, 1, 0, 0],
    [6, 0, 7, 0, 0x63, 1, 0, 0],
    [7, 0, 8, 0, 0x63, 1, 0, 0],
    [8, 0, 9, 0, 0x63, 1, 0, 0],
    [9, 0, 10, 0, 0x63, 1, 0, 0],
    [10, 0, 11, 0, 0x63, 1, 0, 0],
    [13, 0, 15, 0, 0x63, 1, 0, 0],
    [14, 0, 19, 0, 4, 1, 0, 0],
    [15, 0, 20, 0, 6, 1, 0, 0],
    [16, 0, 21, 0, 0x63, 1, 0, 0],
    [18, 0, 23, 0, 0x63, 1, 0, 0],
    [2, 0, 2, 0, 2, 0, 1, 0],
    [3, 0, 3, 0, 3, 0, 2, 0],
    [4, 0, 4, 0, 1, 0, 3, 0],
    [11, 0, 12, 0, 0x63, 0, 4, 0],
    [17, 0, 5, 0, 0x63, 0, 5, 0],
    [19, 0, 22, 0, 0x63, 0, 6, 0],
    [20, 0, 18, 0, 0x63, 0, 7, 0],
    [27, 0, 26, 0, 0x63, 2, 8, 0],
    [24, 0, 17, 0, 0x63, 0, 9, 0],
    [22, 0, 27, 0, 0x63, 0, 10, 0],
];
const TABLE18: [[u8; 8]; 17] = [
    [1, 0, 6, 0, 1, 0, 0, 0],
    [0x15, 0, 7, 0, 1, 0, 0, 0],
    [0x0e, 0, 8, 0, 1, 0, 0, 0],
    [8, 0, 9, 0, 1, 0, 0, 0],
    [9, 0, 10, 0, 1, 0, 0, 0],
    [10, 0, 11, 0, 1, 0, 0, 0],
    [0x0f, 0, 13, 0, 1, 0, 0, 0],
    [13, 0, 15, 0, 1, 0, 0, 0],
    [0x17, 0, 16, 0, 1, 0, 0, 0],
    [0x16, 0, 17, 0, 1, 0, 0, 0],
    [0x19, 0, 0, 0, 0, 1, 0, 0],
    [0x1a, 0, 1, 0, 0, 2, 0, 0],
    [2, 0, 2, 0, 0, 3, 0, 0],
    [3, 0, 3, 0, 0, 4, 0, 0],
    [5, 0, 4, 0, 0, 5, 0, 0],
    [6, 0, 5, 0, 0, 6, 0, 0],
    [11, 0, 12, 0, 0, 7, 0, 0],
];

fn simple_row(id: u32, name: &str) -> Vec<u8> {
    let mut r = id.to_le_bytes().to_vec();
    r.extend(dstr(name));
    r
}

pub fn build_export_pdb(lib: &DeviceLibrary) -> Vec<u8> {
    let mut artists = Interner::new();
    let mut genres = Interner::new();
    let mut labels = Interner::new();
    let mut keys = Interner::new();
    let mut artwork = Interner::new();
    let mut albums: HashMap<(String, u32), u32> = HashMap::new();
    let mut album_rows: Vec<(u32, u32, String)> = vec![];
    let mut track_rows = vec![];
    let db_u3: u16 = 0xfa80;
    let db_u4: u16 = 0x05e7;
    for t in lib.tracks.values() {
        let artist = artists.id(&t.artist);
        let composer = artists.id(&t.composer);
        let orig = artists.id(&t.original_artist);
        let remixer = artists.id(&t.remixer);
        let aa = if t.album_artist.trim().is_empty() { artist } else { artists.id(&t.album_artist) };
        let album = if t.album.trim().is_empty() {
            0
        } else {
            let key = (t.album.clone(), aa);
            let n = albums.len() as u32 + 1;
            *albums.entry(key).or_insert_with(|| {
                album_rows.push((n, aa, t.album.clone()));
                n
            })
        };
        let genre = genres.id(&t.genre);
        let label = labels.id(&t.label);
        let key = keys.id(&t.key_name);
        let art = artwork.id(&t.artwork_path);
        let mut f = Vec::with_capacity(0x5c);
        f.extend_from_slice(&0x24u16.to_le_bytes());
        f.extend_from_slice(&0u16.to_le_bytes());
        f.extend_from_slice(&0x000c_0700u32.to_le_bytes());
        f.extend_from_slice(&t.sample_rate.to_le_bytes());
        f.extend_from_slice(&composer.to_le_bytes());
        f.extend_from_slice(&t.file_size.to_le_bytes());
        // unknown2: observed effectively random; use a stable hash of the path.
        let mut h: u32 = 2166136261;
        for b in t.file_path.bytes() {
            h = (h ^ b as u32).wrapping_mul(16777619);
        }
        f.extend_from_slice(&h.to_le_bytes());
        f.extend_from_slice(&db_u3.to_le_bytes());
        f.extend_from_slice(&db_u4.to_le_bytes());
        for v in [art, key, orig, label, remixer, t.bitrate, t.track_no, t.bpm100, genre, album, artist, t.id] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        for v in [t.disc_no, t.play_count, t.year, t.sample_depth, t.duration, 0x29u16] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.push(t.color);
        f.push(t.rating);
        f.extend_from_slice(&t.file_type.to_le_bytes());
        debug_assert_eq!(f.len(), 0x5c);
        let fname = Path::new(&t.file_path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let strings = vec![
            dstr(""),
            dstr(&t.lyricist),
            dstr("3"),
            dstr("3"),
            dstr(""),
            dstr(&t.message),
            dstr(""),
            dstr(if t.autoload { "ON" } else { "" }),
            dstr(""),
            dstr(""),
            dstr(&t.date_added),
            dstr(&t.release_date),
            dstr(&t.mix_name),
            dstr(""),
            dstr(&t.analyze_path),
            dstr(&t.analyze_date),
            dstr(&t.comment),
            dstr(&t.title),
            dstr(""),
            dstr(&fname),
            dstr(&t.file_path),
        ];
        track_rows.push(row_with_strings(&f, &strings, true));
    }
    let artist_rows: Vec<Vec<u8>> = artists
        .order
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut f = vec![0x60, 0x00, 0, 0];
            f.extend_from_slice(&(i as u32 + 1).to_le_bytes());
            row_with_strings(&f, &[dstr(n)], false)
        })
        .collect();
    let album_rows: Vec<Vec<u8>> = album_rows
        .iter()
        .map(|(id, aa, name)| {
            let mut f = vec![0x80, 0x00, 0, 0];
            f.extend_from_slice(&0u32.to_le_bytes());
            f.extend_from_slice(&aa.to_le_bytes());
            f.extend_from_slice(&id.to_le_bytes());
            f.extend_from_slice(&0u32.to_le_bytes());
            row_with_strings(&f, &[dstr(name)], false)
        })
        .collect();
    let genre_rows = genres.order.iter().enumerate().map(|(i, n)| simple_row(i as u32 + 1, n)).collect();
    let label_rows = labels.order.iter().enumerate().map(|(i, n)| simple_row(i as u32 + 1, n)).collect();
    let key_rows = keys
        .order
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut r = (i as u32 + 1).to_le_bytes().to_vec();
            r.extend((i as u32 + 1).to_le_bytes());
            r.extend(dstr(n));
            r
        })
        .collect();
    let color_rows = COLOR_NAMES[1..]
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut r = vec![0u8; 4];
            r.push(i as u8 + 1);
            r.push(i as u8 + 1);
            r.extend([0u8, 0]);
            r.extend(dstr(n));
            r
        })
        .collect();
    let artwork_rows = artwork.order.iter().enumerate().map(|(i, n)| simple_row(i as u32 + 1, n)).collect();
    let tree_rows = lib
        .nodes
        .iter()
        .map(|n| {
            let mut r = vec![];
            r.extend(n.parent.to_le_bytes());
            r.extend(0u32.to_le_bytes());
            r.extend(n.seq.to_le_bytes());
            r.extend(n.id.to_le_bytes());
            r.extend((n.folder as u32).to_le_bytes());
            r.extend(dstr(&n.name));
            r
        })
        .collect();
    let mut entry_rows = vec![];
    for n in &lib.nodes {
        for (i, t) in n.entries.iter().enumerate() {
            let mut r = vec![];
            r.extend((i as u32 + 1).to_le_bytes());
            r.extend(t.to_le_bytes());
            r.extend(n.id.to_le_bytes());
            entry_rows.push(r);
        }
    }
    let hist_rows = lib.histories.iter().map(|h| simple_row(h.id, &h.name)).collect();
    let mut hent_rows = vec![];
    for h in &lib.histories {
        for (i, t) in h.entries.iter().enumerate() {
            let mut r = vec![];
            r.extend(t.to_le_bytes());
            r.extend(h.id.to_le_bytes());
            r.extend((i as u32 + 1).to_le_bytes());
            hent_rows.push(r);
        }
    }
    let col_rows = COLUMNS
        .iter()
        .map(|(id, u, name)| {
            let mut r = vec![];
            r.extend(id.to_le_bytes());
            r.extend(u.to_le_bytes());
            r.extend(dstr_ucs2(&format!("\u{fffa}{name}\u{fffb}")));
            r
        })
        .collect();
    // Menu (17) and sort (18) tables: the original's rows with visibility and position taken
    // from the Category/Sort preferences (visible rows keep their observed visibility value).
    let browse = lib.browse.clone().map(|b| b.normalized());
    let menu_rows = MENU
        .iter()
        .map(|m| {
            let mut r = m.to_vec();
            if let Some(b) = &browse {
                let id = u16::from_le_bytes([m[0], m[1]]);
                match b.categories.iter().position(|x| *x == id) {
                    Some(i) => {
                        r[5] = if m[5] == 1 { 0 } else { m[5] };
                        r[6..8].copy_from_slice(&(i as u16 + 1).to_le_bytes());
                    }
                    None => {
                        r[5] = 1;
                        r[6..8].copy_from_slice(&0u16.to_le_bytes());
                    }
                }
            }
            r
        })
        .collect();
    let t18_rows = TABLE18
        .iter()
        .map(|m| {
            let mut r = m.to_vec();
            if let Some(b) = &browse {
                let id = u16::from_le_bytes([m[0], m[1]]);
                match b.sorts.iter().position(|x| *x == id) {
                    Some(i) => {
                        r[4] = 0;
                        r[5] = i as u8 + 1;
                    }
                    None => {
                        r[4] = 1;
                        r[5] = 0;
                    }
                }
            }
            r
        })
        .collect();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut sync = vec![0x80, 0x02, 0, 0];
    sync.extend((lib.tracks.len() as u32).to_le_bytes());
    sync.extend(0u32.to_le_bytes());
    sync.extend(dstr(&today));
    sync.extend(0x1e19u16.to_le_bytes());
    sync.extend(dstr("1000"));
    sync.extend(dstr(""));

    let t = |page_type: u32, rows: Vec<Vec<u8>>| TableSpec { page_type, rows, index_entries: false, index_shift: false };
    let tables = vec![
        TableSpec { page_type: 0, rows: track_rows, index_entries: true, index_shift: true },
        t(1, genre_rows),
        TableSpec { page_type: 2, rows: artist_rows, index_entries: false, index_shift: true },
        TableSpec { page_type: 3, rows: album_rows, index_entries: false, index_shift: true },
        t(4, label_rows),
        t(5, key_rows),
        t(6, color_rows),
        t(7, tree_rows),
        t(8, entry_rows),
        t(9, vec![]),
        t(10, vec![]),
        t(11, hist_rows),
        t(12, hent_rows),
        t(13, artwork_rows),
        t(14, vec![]),
        t(15, vec![]),
        t(16, col_rows),
        t(17, menu_rows),
        t(18, t18_rows),
        t(19, vec![sync]),
    ];
    pdb::write_database(&tables, 5, 20)
}

pub fn build_export_ext(lib: &DeviceLibrary) -> Vec<u8> {
    let mut tag_rows = vec![];
    let tag_row = |parent: u32, pos: u32, id: u32, cat: bool, name: &str| -> Vec<u8> {
        let mut f = vec![0x80, 0x06, 0, 0];
        f.extend(0u32.to_le_bytes());
        f.extend(0u32.to_le_bytes());
        f.extend(parent.to_le_bytes());
        f.extend(pos.to_le_bytes());
        f.extend(id.to_le_bytes());
        f.extend(if cat { [0u8, 0, 0, 1] } else { [0u8; 4] });
        row_with_strings(&f, &[dstr(name), dstr("")], false)
    };
    // Keep the IDs the drive already uses; new categories/tags get the next free ID.
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut next = lib.tag_ids.values().max().copied().unwrap_or(0) + 1;
    let mut id_for = |key: String| -> u32 {
        if let Some(i) = ids.get(&key) {
            return *i;
        }
        let i = match lib.tag_ids.get(&key) {
            Some(i) => *i,
            None => {
                let i = next;
                next += 1;
                i
            }
        };
        ids.insert(key, i);
        i
    };
    let mut tag_ids: HashMap<String, u32> = HashMap::new();
    for (ci, (cat, tags)) in lib.tag_categories.iter().enumerate() {
        let cid = id_for(cat.clone());
        tag_rows.push(tag_row(0, ci as u32, cid, true, cat));
        for (ti, tg) in tags.iter().enumerate() {
            let key = tag_key(cat, tg);
            let id = id_for(key.clone());
            tag_ids.insert(key, id);
            tag_rows.push(tag_row(cid, ti as u32, id, false, tg));
        }
    }
    let mut tt_rows = vec![];
    for t in lib.tracks.values() {
        for tg in &t.tags {
            // Older sidecars stored bare tag names; match those by name in any category.
            let id = tag_ids.get(tg).or_else(|| tag_ids.iter().find(|(k, _)| k.rsplit('\u{1f}').next() == Some(tg.as_str())).map(|(_, v)| v));
            if let Some(id) = id {
                let mut r = vec![];
                r.extend(0u32.to_le_bytes());
                r.extend(t.id.to_le_bytes());
                r.extend(id.to_le_bytes());
                r.extend(3u32.to_le_bytes());
                tt_rows.push(r);
            }
        }
    }
    let t7 = {
        let mut r = vec![0u8, 7, 0, 0];
        r.extend([0u8; 20]);
        r.extend([0x81, 0xfa, 0xe7, 0x05, 0x03, 0x22, 0x23, 0x24, 0x25, 0x26, 3, 3, 3, 3, 3]);
        r.resize(60, 0);
        r
    };
    let t = |page_type: u32, rows: Vec<Vec<u8>>| TableSpec { page_type, rows, index_entries: false, index_shift: false };
    let tables = vec![
        t(0, vec![]),
        t(1, vec![]),
        t(2, vec![]),
        TableSpec { page_type: 3, rows: tag_rows, index_entries: false, index_shift: true },
        t(4, tt_rows),
        t(5, vec![]),
        t(6, vec![]),
        t(7, vec![t7]),
        t(8, vec![]),
    ];
    pdb::write_database(&tables, 5, 6)
}

pub fn write_device(root: &Path, lib: &DeviceLibrary) -> Result<()> {
    let dir = root.join("PIONEER").join("rekordbox");
    std::fs::create_dir_all(&dir)?;
    let main = build_export_pdb(lib);
    // Verify by reading back before replacing the drive's database.
    let check = parse_pdb(main.clone())?;
    if check.tracks.len() != lib.tracks.len() {
        return Err(anyhow!("internal check failed: wrote {} tracks, read back {}", lib.tracks.len(), check.tracks.len()));
    }
    // Both databases are fully written and flushed before either replaces the old one, so
    // the window in which the pair is inconsistent is two renames long.
    let ext = build_export_ext(lib);
    write_tmp(&dir.join("export.pdb"), &main)?;
    write_tmp(&dir.join("exportExt.pdb"), &ext)?;
    publish_tmp(&dir.join("export.pdb"))?;
    publish_tmp(&dir.join("exportExt.pdb"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_browse_matches_original_tables() {
        let d = UsbBrowse::default();
        // Category page defaults: ARTIST, ALBUM, TRACK, KEY, PLAYLIST, HISTORY, SEARCH,
        // MATCHING, FOLDER, DATE ADDED.
        assert_eq!(d.categories, vec![2, 3, 4, 11, 17, 19, 20, 27, 24, 22]);
        // Sort page defaults: DEFAULT, ALPHABET, ARTIST, ALBUM, BPM, RATING, KEY.
        assert_eq!(d.sorts, vec![0x19, 0x1a, 2, 3, 5, 6, 11]);
        // Writing the defaults reproduces the original rows byte for byte.
        let mut lib = DeviceLibrary::default();
        let plain = build_export_pdb(&lib);
        lib.browse = Some(d);
        assert_eq!(build_export_pdb(&lib), plain);
    }

    #[test]
    fn anlz_numbering_matches_original() {
        let base = std::env::temp_dir().join(format!("prepdeck_anlz_num_{}", std::process::id()));
        let case = |files: &[&str]| -> String {
            let d = base.join(files.join("_").replace('.', "-"));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            for f in files {
                std::fs::write(d.join(f), b"x").unwrap();
            }
            next_anlz_name(&d, None)
        };
        assert_eq!(case(&[]), "ANLZ0000");
        assert_eq!(case(&["ANLZ0000.DAT", "ANLZ0000.EXT"]), "ANLZ0001");
        assert_eq!(case(&["ANLZFFFFFFFF.DAT"]), "ANLZ0000");
        assert_eq!(case(&["ANLZ7FFFFFFF.DAT"]), "ANLZ80000000");
        assert_eq!(case(&["anlz0003.anything"]), "ANLZ0004");
        assert_eq!(case(&["ANLZGGGG.DAT"]), "ANLZ0001");
        // A number reserved earlier in the same export counts even before its file exists.
        let d = base.join("reserved");
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(next_anlz_name(&d, Some(&0)), "ANLZ0001");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(anlz_base("/PIONEER/USBANLZ/P016/0000875E/ANLZ0003.DAT").as_deref(), Some("/PIONEER/USBANLZ/P016/0000875E/ANLZ0003"));
        assert_eq!(anlz_base("/Contents/x.DAT"), None);
    }

    use super::*;
    #[test]
    fn media_paths_follow_rules() {
        assert_eq!(media_dir("", ""), "/Contents/UnknownArtist/UnknownAlbum");
        assert_eq!(media_dir("AC/DC", "Back: In ~ Black?"), "/Contents/AC_DC/Back_ In _ Black_");
        let f = file_candidate(&"x".repeat(80), "mp3", None);
        assert_eq!(f.chars().count(), 48);
        assert!(file_candidate("song", "mp3", Some(2)).ends_with("song-2.mp3"));
    }

    #[test]
    fn device_roundtrip() {
        let mut lib = DeviceLibrary::default();
        for i in 1..=60u32 {
            lib.tracks.insert(
                i,
                DevTrack {
                    id: i,
                    title: format!("Track {i} – ünïcode"),
                    artist: format!("Artist {}", i % 7),
                    album: format!("Album {}", i % 5),
                    genre: "House".into(),
                    key_name: "Am".into(),
                    bpm100: 12800,
                    file_path: format!("/Contents/A/B/t{i}.mp3"),
                    analyze_path: "/PIONEER/USBANLZ/P000/00000000/ANLZ0000.DAT".into(),
                    file_type: 1,
                    rating: 3,
                    tags: vec![tag_key("Genre", "Techno")],
                    ..Default::default()
                },
            );
        }
        lib.nodes.push(DevNode { id: 1, parent: 0, name: "Folder".into(), folder: true, seq: 0, entries: vec![] });
        lib.nodes.push(DevNode { id: 2, parent: 1, name: "Set".into(), folder: false, seq: 0, entries: vec![3, 1, 2] });
        lib.tag_categories = vec![("Genre".into(), vec!["Techno".into()])];
        let bytes = build_export_pdb(&lib);
        let _ = std::fs::write(std::env::temp_dir().join("prepdeck_sample_export.pdb"), &bytes);
        let _ = std::fs::write(std::env::temp_dir().join("prepdeck_sample_exportExt.pdb"), build_export_ext(&lib));
        let back = parse_pdb(bytes).unwrap();
        assert_eq!(back.tracks.len(), 60);
        assert_eq!(back.tracks[&5].title, "Track 5 – ünïcode");
        assert_eq!(back.tracks[&5].artist, "Artist 5");
        assert_eq!(back.tracks[&5].album, "Album 0");
        assert_eq!(back.tracks[&5].key_name, "Am");
        assert_eq!(back.nodes.len(), 2);
        assert_eq!(back.nodes.iter().find(|n| n.id == 2).unwrap().entries, vec![3, 1, 2]);
        let ext = build_export_ext(&lib);
        let mut l2 = back.clone();
        read_ext_tags(ext, &mut l2).unwrap();
        assert_eq!(l2.tag_categories[0].1, vec!["Techno".to_string()]);
        assert_eq!(l2.tracks[&1].tags, vec![tag_key("Genre", "Techno")]);
    }

    #[test]
    fn reads_real_rekordbox_export_if_available() {
        let p = std::env::var("PREPDECK_FIXTURE_PDB").unwrap_or_default();
        if p.is_empty() {
            return;
        }
        let lib = parse_pdb(std::fs::read(p).unwrap()).unwrap();
        assert!(!lib.tracks.is_empty());
        for t in lib.tracks.values() {
            assert!(t.file_path.starts_with("/Contents"));
        }
    }
}
