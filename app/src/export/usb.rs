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
    /// The row's original fixed header and 21 strings when it was read from a drive, so fields
    /// PrepDeck does not manage (ISRC, unknown slots and constants) are written back unchanged.
    #[serde(skip)]
    pub raw_head: Vec<u8>,
    #[serde(skip)]
    pub raw_strings: Vec<String>,
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

/// The drive carries rekordbox's newer OneLibrary catalogue (exportLibrary.db).
pub fn has_onelibrary(root: &Path) -> bool {
    root.join("PIONEER").join("rekordbox").join("exportLibrary.db").exists()
}

pub fn pdb_path(root: &Path) -> PathBuf {
    root.join("PIONEER").join("rekordbox").join("export.pdb")
}

fn sidecar_path(root: &Path) -> PathBuf {
    root.join(".PrepDeck").join("device.json")
}

fn cue_record_path(root: &Path) -> PathBuf {
    root.join(".PrepDeck").join("cues.json")
}

/// Cue fingerprints of the tracks PrepDeck last wrote to this drive, by media path.
pub fn read_cue_record(root: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(cue_record_path(root)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Update the drive's cue record (PrepDeck's own folder; players never read it).
pub fn write_cue_record(root: &Path, record: &HashMap<String, String>) -> Result<()> {
    std::fs::create_dir_all(root.join(".PrepDeck"))?;
    write_atomic(&cue_record_path(root), &serde_json::to_vec_pretty(&record.iter().collect::<BTreeMap<_, _>>())?)
}

/// A track whose cues on the drive differ from what PrepDeck last wrote there: edited on a
/// player (or by another program).
#[derive(Clone, Debug)]
pub struct PlayerCueEdit {
    pub file_path: String,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub cues: Vec<Cue>,
    /// Fingerprint PrepDeck recorded at export.
    pub exported: String,
    /// Fingerprint of the cues now on the drive.
    pub now: String,
}

/// Tracks on the drive whose cues changed since PrepDeck exported them.
pub fn player_cue_edits(root: &Path, lib: &DeviceLibrary) -> Vec<PlayerCueEdit> {
    let record = read_cue_record(root);
    // An edit made on a player rewrites the analysis file, so only files newer than the record
    // (with FAT's 2-second timestamp resolution as margin) need reading.
    let since = std::fs::metadata(cue_record_path(root)).and_then(|m| m.modified()).ok().map(|t| t + std::time::Duration::from_secs(2));
    let newer = |p: &Path| match (since, std::fs::metadata(p).and_then(|m| m.modified())) {
        (Some(s), Ok(m)) => m > s,
        _ => true,
    };
    let mut out = vec![];
    for t in lib.tracks.values() {
        let Some(exported) = record.get(&t.file_path) else { continue };
        let Some(base) = anlz_base(&t.analyze_path) else { continue };
        let b = root.join(base.trim_start_matches('/'));
        if !newer(&b.with_extension("EXT")) && !newer(&b.with_extension("DAT")) {
            continue;
        }
        let ext = anlz::read_section_list(&b.with_extension("EXT"));
        let dat = anlz::read_section_list(&b.with_extension("DAT"));
        if ext.is_empty() && dat.is_empty() {
            continue;
        }
        let cues = anlz::parse_cues(&ext, &dat);
        let now = anlz::cue_fingerprint(&cues);
        if &now != exported {
            out.push(PlayerCueEdit { file_path: t.file_path.clone(), source: t.source.clone(), title: t.title.clone(), artist: t.artist.clone(), cues, exported: exported.clone(), now });
        }
    }
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

/// Files in the drive's music, analysis and artwork folders that no track in its library uses
/// (left behind by deleted playlists, other programs or copies made by hand), with sizes.
/// Nothing is listed while an interrupted export's journal is still on the drive.
pub fn unused_files(root: &Path, lib: &DeviceLibrary) -> Vec<(PathBuf, u64)> {
    // A OneLibrary database (which PrepDeck can't read) may use files export.pdb doesn't.
    if journal_dir(root).join("journal.log").exists() || has_onelibrary(root) {
        return vec![];
    }
    let mut used: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for t in lib.tracks.values() {
        if !t.file_path.is_empty() {
            used.insert(root.join(t.file_path.trim_start_matches('/')));
        }
        if let Some(b) = anlz_base(&t.analyze_path) {
            let b = root.join(b.trim_start_matches('/'));
            for e in ["DAT", "EXT", "2EX"] {
                used.insert(b.with_extension(e));
            }
        }
        if !t.artwork_path.is_empty() {
            let a = root.join(t.artwork_path.trim_start_matches('/'));
            if let (Some(stem), Some(ext)) = (a.file_stem().map(|x| x.to_string_lossy().to_string()), a.extension().map(|x| x.to_string_lossy().to_string())) {
                used.insert(a.with_file_name(format!("{stem}_m.{ext}")));
            }
            used.insert(a);
        }
    }
    let mut out = vec![];
    let mut stack: Vec<PathBuf> = [root.join("Contents"), root.join("PIONEER").join("USBANLZ"), root.join("PIONEER").join("Artwork")].into_iter().filter(|d| d.is_dir()).collect();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && !used.contains(&p) {
                let name = e.file_name().to_string_lossy().to_string();
                // macOS bookkeeping files are not the library's concern.
                if name == ".DS_Store" || name.starts_with("._") {
                    continue;
                }
                out.push((p, e.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    out.sort();
    out
}

/// Delete the given unused files (re-checked against the drive's current library) and then
/// any folders left empty. Returns how many files were deleted.
pub fn delete_unused(root: &Path, files: &[PathBuf]) -> Result<usize> {
    let lib = read_device(root)?;
    let still: std::collections::HashSet<PathBuf> = unused_files(root, &lib).into_iter().map(|f| f.0).collect();
    let mut n = 0;
    for f in files.iter().filter(|f| still.contains(*f)) {
        if std::fs::remove_file(f).is_ok() {
            n += 1;
            let mut dir = f.parent().map(Path::to_path_buf);
            while let Some(d) = dir {
                if d == root.join("Contents") || d == root.join("PIONEER").join("USBANLZ") || d == root.join("PIONEER").join("Artwork") || !d.starts_with(root) {
                    break;
                }
                if std::fs::remove_dir(&d).is_err() {
                    break;
                }
                dir = d.parent().map(Path::to_path_buf);
            }
        }
    }
    Ok(n)
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
        let raw_head = r[..0x5c].to_vec();
        let raw_strings: Vec<String> = (0..21).map(|i| s(i)).collect();
        let t = DevTrack {
            raw_head,
            raw_strings,
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
    /// PrepDeck's vocal detection (levels 0–4 per 46.4 ms), written as PVDI when rekordbox's
    /// own is not available.
    pub vocal: Option<Vec<u8>>,
}

impl ExportTrack {
    /// rekordbox's own phrase section can be used (its grid is unchanged and it has one).
    fn has_rb_phrases(&self) -> bool {
        self.grid_same && self.rb_anlz.as_ref().map(|b| anlz::read_sections(&b.with_extension("EXT")).contains_key("PSSI")).unwrap_or(false)
    }
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
    /// Write phrase sections (PSSI) for tracks without rekordbox's own.
    pub phrases: bool,
}

/// Files this export created. Unless the export commits, they are removed again, so a
/// stopped or failed export leaves the drive as it was.
/// Export journal on the drive (`/.PrepDeck/journal/journal.log`): every file this export
/// creates ("C" lines) and, before an existing file is overwritten, a durable copy of its old
/// bytes ("R" lines). "P" is written once both new databases are fully on the drive. An
/// export that fails is undone from it at once; one cut short by a crash, power loss or the
/// drive being pulled is undone (before "P") or finished (after "P") by [`recover_device`]
/// the next time the drive is seen.
struct Created {
    root: PathBuf,
    dir: PathBuf,
    log: Option<std::fs::File>,
    files: Vec<PathBuf>,
    /// Existing files this export overwrote, with the journal copy of their previous bytes.
    replaced: Vec<(PathBuf, PathBuf)>,
    publishing: bool,
    committed: bool,
}

fn journal_dir(root: &Path) -> PathBuf {
    root.join(".PrepDeck").join("journal")
}

#[cfg(test)]
thread_local! {
    /// Fault injection: fail at this journal step (1-based); with `CRASH`, skip the in-process
    /// rollback as if the program had died there.
    pub(crate) static FAIL_AT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static STEP: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static CRASH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn fault_point() -> Result<()> {
    #[cfg(test)]
    {
        let n = STEP.with(|s| {
            s.set(s.get() + 1);
            s.get()
        });
        if FAIL_AT.with(|f| f.get()) == n {
            return Err(anyhow!("injected fault at step {n}"));
        }
    }
    Ok(())
}

impl Created {
    fn begin(root: &Path) -> Result<Created> {
        let dir = journal_dir(root);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let log = std::fs::File::create(dir.join("journal.log"))?;
        let mut c = Created { root: root.to_path_buf(), dir, log: Some(log), files: vec![], replaced: vec![], publishing: false, committed: false };
        c.line("B", true)?;
        Ok(c)
    }
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().to_string()
    }
    fn line(&mut self, l: &str, sync: bool) -> Result<()> {
        use std::io::Write;
        fault_point()?;
        if let Some(f) = self.log.as_mut() {
            writeln!(f, "{l}")?;
            if sync {
                f.sync_all()?;
            }
        }
        fault_point()
    }
    /// A new file this export is about to write.
    fn note_created(&mut self, p: &Path) -> Result<()> {
        self.files.push(p.to_path_buf());
        let l = format!("C\t{}", self.rel(p));
        // Durable before the file exists, so a crash can't leave an unlisted file.
        self.line(&l, true)
    }
    /// About to write `p`: keep a durable copy of what is there now, or note it as new.
    fn will_replace(&mut self, p: &Path) -> Result<()> {
        if !p.is_file() {
            return self.note_created(p);
        }
        let backup = self.dir.join(format!("r{}", self.replaced.len()));
        std::fs::copy(p, &backup)?;
        std::fs::File::options().write(true).open(&backup)?.sync_all()?;
        let l = format!("R\t{}\t{}", self.rel(p), backup.file_name().unwrap_or_default().to_string_lossy());
        self.line(&l, true)?;
        self.replaced.push((p.to_path_buf(), backup));
        Ok(())
    }
    /// Both new databases are on the drive: from here on the export is finished, not undone.
    fn publishing(&mut self) -> Result<()> {
        self.line("P", true)?;
        self.publishing = true;
        Ok(())
    }
    fn finish(&mut self) {
        self.committed = true;
        self.log = None;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for Created {
    fn drop(&mut self) {
        #[cfg(test)]
        if CRASH.with(|c| c.get()) {
            return;
        }
        if self.committed {
            return;
        }
        self.log = None;
        let _ = recover_device(&self.root);
    }
}

/// Finish or undo an export that did not complete, from the drive's journal. Returns what was
/// done, or None when there was nothing to recover.
pub fn recover_device(root: &Path) -> Option<String> {
    let dir = journal_dir(root);
    let text = std::fs::read_to_string(dir.join("journal.log")).ok()?;
    let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
    let rb = root.join("PIONEER").join("rekordbox");
    let msg = if lines.iter().any(|l| l[0] == "P") {
        for f in ["export.pdb", "exportExt.pdb"] {
            if tmp_for(&rb.join(f)).exists() {
                let _ = publish_tmp(&rb.join(f));
            }
        }
        "An export to this device that was interrupted has been completed."
    } else {
        for l in lines.iter().rev() {
            match l.as_slice() {
                ["R", path, backup] => {
                    if let Ok(b) = std::fs::read(dir.join(backup)) {
                        let _ = write_atomic(&root.join(path), &b);
                    }
                }
                ["C", path] => {
                    let p = root.join(path);
                    let _ = std::fs::remove_file(&p);
                    // Folders this export created and left empty go too (never the top ones).
                    let mut dir = p.parent().map(Path::to_path_buf);
                    while let Some(d) = dir {
                        if d == root || d == root.join("Contents") || d == root.join("PIONEER") || d == root.join(".PrepDeck") || !d.starts_with(root) || std::fs::remove_dir(&d).is_err() {
                            break;
                        }
                        dir = d.parent().map(Path::to_path_buf);
                    }
                }
                _ => {}
            }
            // Half-written replacements of any listed file.
            if let Some(path) = l.get(1) {
                let _ = std::fs::remove_file(tmp_for(&root.join(path)));
            }
        }
        for f in ["export.pdb", "exportExt.pdb"] {
            let _ = std::fs::remove_file(tmp_for(&rb.join(f)));
        }
        "An export to this device that was interrupted has been undone; the device is as it was before."
    };
    let _ = std::fs::remove_dir_all(&dir);
    Some(msg.to_string())
}

fn sha256_file(p: &Path) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
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
    created.note_created(&tmp)?;
    let n = std::fs::copy(src, &tmp)?;
    std::fs::File::options().write(true).open(&tmp)?.sync_all()?;
    let want = std::fs::metadata(src)?.len();
    if n != want || std::fs::metadata(&tmp)?.len() != want {
        return Err(anyhow!("copy of {} is incomplete ({n} of {want} bytes)", src.display()));
    }
    // Byte-for-byte check of what reached the drive.
    if sha256_file(src)? != sha256_file(&tmp)? {
        return Err(anyhow!("copy of {} does not match the original; the drive may be failing", src.display()));
    }
    created.note_created(dest)?;
    std::fs::rename(&tmp, dest)?;
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
    recover_device(&root);
    let mut lib = match read_device(&root) {
        Ok(l) => l,
        Err(e) => return Err(anyhow!("Device library is corrupted ({e}). Nothing was changed.")),
    };
    let mut skipped: Vec<String> = std::mem::take(&mut req.skipped);
    // Tracks whose grid is still the one rekordbox analysed can use its phrases/extended grid.
    for et in req.tracks.values_mut() {
        if let (Some(b), Some(g)) = (&et.rb_anlz, &et.grid) {
            et.grid_same = crate::rbimport::read_pqtz_grid(&b.with_extension("DAT")).map(|r| crate::app::grids_match(&r, g)).unwrap_or(false);
        }
    }

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

    // Phrases for tracks without rekordbox's own (computed on the track's grid).
    let mut phrases: HashMap<i64, crate::phrase::Structure> = HashMap::new();
    let todo: Vec<i64> = if req.phrases { req.tracks.values().filter(|t| t.grid.is_some() && !t.has_rb_phrases()).map(|t| t.track.id).collect() } else { vec![] };
    let n_todo = todo.len();
    for (i, id) in todo.into_iter().enumerate() {
        if cancel.load(Relaxed) {
            return Err(anyhow!("Export was stopped. The device library was not changed."));
        }
        let Some(et) = req.tracks.get(&id) else { continue };
        progress(Progress::Step { done: i, total: n_todo, text: format!("Analyzing phrases of {}", et.track.display_title()) });
        if let Ok(d) = crate::audio::decode_file(&et.track.path) {
            let full = d.duration();
            let beats = et.grid.as_ref().map(|g| g.beats(full)).unwrap_or_default();
            if let Some(st) = crate::phrase::analyze(&d.mono(), d.sr as f64, &beats) {
                phrases.insert(id, st);
            }
        }
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
    let mut created = Created::begin(&root)?;
    let mut cue_record = read_cue_record(&root);
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
        // A row whose file is still there wins over one whose file went missing.
        let on_drive = |d: &&DevTrack| root.join(d.file_path.trim_start_matches('/')).exists();
        let existing = lib.tracks.values().filter(|d| d.source == t.path).find(on_drive).or_else(|| lib.tracks.values().find(|d| d.source == t.path)).map(|d| (d.id, d.file_path.clone()));
        let (dev_id, usb_path) = match existing {
            Some((id, p)) if root.join(p.trim_start_matches('/')).exists() => (id, p),
            Some((id, p)) if !p.is_empty() => {
                // The file went missing from the drive: copy it back to the same place, keeping
                // the track's ID, playlists and analysis path.
                let dest = root.join(p.trim_start_matches('/'));
                std::fs::create_dir_all(dest.parent().unwrap())?;
                copy_durable(src, &dest, &mut created)?;
                (id, p)
            }
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
        let mut sections = std::collections::HashMap::new();
        if let Some(b) = &et.rb_anlz {
            for e in ["2EX", "EXT", "DAT"] {
                sections.extend(anlz::read_sections(&b.with_extension(e)));
            }
        }
        let prev = lib.tracks.get(&dev_id).cloned().unwrap_or_default();
        let mut device = std::collections::HashMap::new();
        if let Some(b) = anlz_base(&prev.analyze_path) {
            let b = root.join(b.trim_start_matches('/'));
            for e in ["DAT", "EXT", "2EX"] {
                device.insert(e, anlz::read_section_list(&b.with_extension(e)));
            }
        }
        let ov = Some(anlz::Overrides { sections, grid_same: et.grid_same, pssi: phrases.get(&t.id).map(crate::phrase::pssi), device, pvdi: et.vocal.as_deref().map(anlz::pvdi) });
        let files = anlz::build_with(&usb_path, &beats, &wave, &et.cues, t.raw_samples, t.sample_rate, ov.as_ref());
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
            created.will_replace(&path)?;
            write_atomic(&path, data)?;
        }
        cue_record.insert(usb_path.clone(), anlz::cue_fingerprint(&anlz::parse_cues(&anlz::section_list(&files.ext), &anlz::section_list(&files.dat))));
        // Artwork.
        let artwork_path = if !t.artwork.is_empty() && Path::new(&t.artwork).exists() {
            // Artwork files are numbered by artwork row, not track: reuse this track's own file
            // only when no other track shows it, else take the next free number.
            let shared = |p: &str| lib.tracks.values().any(|o| o.id != dev_id && o.artwork_path == p);
            let rel = if !prev.artwork_path.is_empty() && prev.artwork_path.starts_with("/PIONEER/Artwork/") && !shared(&prev.artwork_path) { prev.artwork_path.clone() } else { free_artwork_name(&root, &lib) };
            write_artwork(&root, &rel, &t.artwork, &mut created).unwrap_or_default()
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
            raw_head: prev.raw_head.clone(),
            raw_strings: prev.raw_strings.clone(),
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
    // Sidecar with local sources and the cue record (PrepDeck's own; never read by players).
    // Written first so a finished-after-interruption export has them too; entries for files
    // an undone export removed are simply never looked up.
    let _ = std::fs::create_dir_all(root.join(".PrepDeck"));
    // Journaled, so an undone export also puts these back.
    created.will_replace(&sidecar_path(&root))?;
    // Sorted, so the same content always gives the same bytes.
    let _ = write_atomic(&sidecar_path(&root), &serde_json::to_vec_pretty(&sources.iter().collect::<BTreeMap<_, _>>()).unwrap_or_default());
    cue_record.retain(|path, _| lib.tracks.values().any(|t| &t.file_path == path));
    created.will_replace(&cue_record_path(&root))?;
    let _ = write_cue_record(&root, &cue_record);
    // Player settings, as rekordbox writes them with every export: part of the journaled
    // export, and a failure to write them fails it.
    std::fs::create_dir_all(root.join("PIONEER"))?;
    for (name, bytes) in req.settings.files() {
        let p = root.join("PIONEER").join(name);
        created.will_replace(&p)?;
        write_atomic(&p, &bytes).map_err(|e| anyhow!("writing the player settings file {name}: {e}"))?;
    }
    backup_device_db(&root);
    write_device_journaled(&root, &lib, &mut created)?;
    created.finish();


    let mut msg = format!("Export completed: {} tracks, {} playlists on the device", lib.tracks.len(), lib.nodes.iter().filter(|n| !n.folder).count());
    if has_onelibrary(&root) {
        msg.push_str(". Note: this drive also has a OneLibrary database (written by rekordbox 7.2.11 or later), which PrepDeck does not update yet. Players that read OneLibrary will not show these changes; players that read the standard device library will.");
    }
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

/// The next artwork file name no file on the drive and no track in its library uses.
fn free_artwork_name(root: &Path, lib: &DeviceLibrary) -> String {
    let dir = root.join("PIONEER/Artwork/00001");
    let used: std::collections::HashSet<&str> = lib.tracks.values().map(|t| t.artwork_path.as_str()).collect();
    (1u32..)
        .map(|n| (n, format!("/PIONEER/Artwork/00001/a{n}.jpg")))
        .find(|(n, rel)| !used.contains(rel.as_str()) && !dir.join(format!("a{n}.jpg")).exists() && !dir.join(format!("a{n}_m.jpg")).exists())
        .map(|x| x.1)
        .unwrap()
}

/// Write a track's artwork (80 px and the 240 px "_m" variant) at `rel`.
fn write_artwork(root: &Path, rel: &str, src: &str, created: &mut Created) -> Result<String> {
    let img = image::open(src)?;
    let small_p = root.join(rel.trim_start_matches('/'));
    let stem = small_p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
    let big_p = small_p.with_file_name(format!("{stem}_m.jpg"));
    std::fs::create_dir_all(small_p.parent().unwrap())?;
    created.will_replace(&small_p)?;
    created.will_replace(&big_p)?;
    let small = img.resize_to_fill(80, 80, image::imageops::FilterType::Triangle).to_rgb8();
    small.save(&small_p)?;
    let big = img.resize_to_fill(240, 240, image::imageops::FilterType::Triangle).to_rgb8();
    big.save(&big_p)?;
    Ok(rel.to_string())
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
        // A row read from the drive keeps the bytes PrepDeck does not manage: the row header
        // bitmask (0x04..0x08; 0x02 is the positional index shift), the 0x14 value, the 0x18/0x1a values and the 0x56 constant.
        if t.raw_head.len() == 0x5c {
            for range in [0x04..0x08, 0x14..0x1c, 0x56..0x58] {
                f[range.clone()].copy_from_slice(&t.raw_head[range]);
            }
        }
        let fname = Path::new(&t.file_path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        // String slots PrepDeck does not manage keep the drive's value (ISRC, slots 2–4, 6,
        // 8, 9, 13, 18).
        let keep = |i: usize, default: &str| -> String { t.raw_strings.get(i).cloned().unwrap_or_else(|| default.to_string()) };
        let (s0, s2, s3, s4, s6, s8, s9, s13, s18) = (keep(0, ""), keep(2, "3"), keep(3, "3"), keep(4, ""), keep(6, ""), keep(8, ""), keep(9, ""), keep(13, ""), keep(18, ""));
        let strings = vec![
            dstr(&s0),
            dstr(&t.lyricist),
            dstr(&s2),
            dstr(&s3),
            dstr(&s4),
            dstr(&t.message),
            dstr(&s6),
            dstr(if t.autoload { "ON" } else { "" }),
            dstr(&s8),
            dstr(&s9),
            dstr(&t.date_added),
            dstr(&t.release_date),
            dstr(&t.mix_name),
            dstr(&s13),
            dstr(&t.analyze_path),
            dstr(&t.analyze_date),
            dstr(&t.comment),
            dstr(&t.title),
            dstr(&s18),
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
    write_device_then(root, lib, &mut || {})
}

/// As write_device; `published` runs as soon as the new export.pdb is in place (from then on
/// the drive's library refers to the files this export copied).
/// Write both databases under the export's journal: fully written and flushed, then "P",
/// then the two renames.
fn write_device_journaled(root: &Path, lib: &DeviceLibrary, created: &mut Created) -> Result<()> {
    let dir = root.join("PIONEER").join("rekordbox");
    std::fs::create_dir_all(&dir)?;
    let main = build_export_pdb(lib);
    let check = parse_pdb(main.clone())?;
    if check.tracks.len() != lib.tracks.len() {
        return Err(anyhow!("internal check failed: wrote {} tracks, read back {}", lib.tracks.len(), check.tracks.len()));
    }
    let ext = build_export_ext(lib);
    write_tmp(&dir.join("export.pdb"), &main)?;
    write_tmp(&dir.join("exportExt.pdb"), &ext)?;
    created.publishing()?;
    publish_tmp(&dir.join("export.pdb"))?;
    fault_point()?;
    publish_tmp(&dir.join("exportExt.pdb"))?;
    fault_point()
}

pub fn write_device_then(root: &Path, lib: &DeviceLibrary, published: &mut dyn FnMut()) -> Result<()> {
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
    published();
    publish_tmp(&dir.join("exportExt.pdb"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Rewriting a rekordbox-made export.pdb keeps every track's unmanaged bytes and strings.
    #[test]
    fn rewrite_keeps_unmanaged_track_fields() {
        let p = dirs::home_dir().unwrap().join("Developer/rekordbox-rea/analysis/format-oracles/Holzhaus__rekordcrate/data/complete_export/demo_tracks/PIONEER/rekordbox/export.pdb");
        let Ok(bytes) = std::fs::read(&p) else { return };
        let a = parse_pdb(bytes).unwrap();
        assert!(!a.tracks.is_empty());
        let b = parse_pdb(build_export_pdb(&a)).unwrap();
        assert_eq!(a.tracks.len(), b.tracks.len());
        for (id, ta) in &a.tracks {
            let tb = &b.tracks[id];
            assert_eq!(ta.raw_strings, tb.raw_strings, "strings of track {id}");
            for range in [0x04..0x08, 0x14..0x1c, 0x56..0x58] {
                assert_eq!(ta.raw_head[range.clone()], tb.raw_head[range.clone()], "header bytes {range:?} of track {id}");
            }
            let ids = [0x0c, 0x1c, 0x20, 0x24, 0x28, 0x2c, 0x3c, 0x40, 0x44, 0x48];
            let diff: Vec<usize> = (0..0x5c).filter(|&i| i != 2 && i != 3 && !ids.iter().any(|&o| i >= o && i < o + 4) && ta.raw_head[i] != tb.raw_head[i]).collect();
            assert!(diff.is_empty(), "track {id}: header bytes {diff:x?} changed");
            assert_eq!((ta.title.clone(), ta.artist.clone(), ta.album.clone(), ta.genre.clone(), ta.bpm100, ta.key_name.clone()), (tb.title.clone(), tb.artist.clone(), tb.album.clone(), tb.genre.clone(), tb.bpm100, tb.key_name.clone()));
        }
    }

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

    /// A cue moved on a player (the drive's analysis file rewritten) is found; an untouched one is not.
    #[test]
    fn finds_cues_changed_on_players() {
        let root = std::env::temp_dir().join(format!("prepdeck_player_edits_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = "/PIONEER/USBANLZ/P000/00000001/ANLZ0000";
        let dir = root.join(base.trim_start_matches('/')).parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let w = Waveform { frames: vec![[10; 8]; 50] };
        let cue = |start: f64| Cue { id: 0, track_id: 0, kind: CueKind::Hot, slot: 0, start, end: None, color: 0, comment: String::new(), active_loop: false };
        let write = |cues: &[Cue]| {
            let f = anlz::build("/Contents/a.mp3", &[], &w, cues, 0, 44100);
            let b = root.join(base.trim_start_matches('/'));
            std::fs::write(b.with_extension("DAT"), &f.dat).unwrap();
            std::fs::write(b.with_extension("EXT"), &f.ext).unwrap();
            anlz::cue_fingerprint(&anlz::parse_cues(&anlz::section_list(&f.ext), &anlz::section_list(&f.dat)))
        };
        let fp = write(&[cue(1.0)]);
        let mut lib = DeviceLibrary::default();
        lib.tracks.insert(1, DevTrack { id: 1, title: "A".into(), file_path: "/Contents/a.mp3".into(), analyze_path: format!("{base}.DAT"), ..Default::default() });
        write_cue_record(&root, &HashMap::from([("/Contents/a.mp3".to_string(), fp.clone())])).unwrap();
        // As if the export had happened a minute ago.
        let backdate = |p: &Path| std::fs::File::options().write(true).open(p).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60)).unwrap();
        backdate(&cue_record_path(&root));
        assert!(player_cue_edits(&root, &lib).is_empty());
        assert_eq!(fp, anlz::local_cue_fingerprint(&[cue(1.0)]));
        write(&[cue(2.0)]);
        let edits = player_cue_edits(&root, &lib);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].exported, fp);
        assert_eq!(edits[0].cues[0].start, 2.0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finds_and_deletes_only_unused_files() {
        let root = std::env::temp_dir().join(format!("prepdeck_unused_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
            p
        };
        mk("Contents/A/kept.mp3");
        let orphan = mk("Contents/B/old.mp3");
        mk("PIONEER/USBANLZ/P001/0000AAAA/ANLZ0000.DAT");
        mk("PIONEER/USBANLZ/P001/0000AAAA/ANLZ0000.EXT");
        let orphan_anlz = mk("PIONEER/USBANLZ/P002/0000BBBB/ANLZ0000.DAT");
        mk("PIONEER/Artwork/00001/a1.jpg");
        mk("PIONEER/Artwork/00001/a1_m.jpg");
        mk("Contents/A/.DS_Store");
        let mut lib = DeviceLibrary::default();
        lib.tracks.insert(1, DevTrack { id: 1, file_path: "/Contents/A/kept.mp3".into(), analyze_path: "/PIONEER/USBANLZ/P001/0000AAAA/ANLZ0000.DAT".into(), artwork_path: "/PIONEER/Artwork/00001/a1.jpg".into(), ..Default::default() });
        let found: Vec<PathBuf> = unused_files(&root, &lib).into_iter().map(|f| f.0).collect();
        assert_eq!(found, { let mut v = vec![orphan.clone(), orphan_anlz.clone()]; v.sort(); v });
        // Deletion re-reads the drive's library: with no export.pdb there, every file is
        // unused, so only the listed ones go.
        let n = delete_unused(&root, &found).unwrap();
        assert_eq!(n, 2);
        assert!(!root.join("Contents/B").exists() && !root.join("PIONEER/USBANLZ/P002").exists());
        assert!(root.join("Contents/A/kept.mp3").exists() && root.join("PIONEER/Artwork/00001/a1_m.jpg").exists());
        let _ = std::fs::remove_dir_all(&root);
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
