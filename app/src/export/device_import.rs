//! Reading things back from a device: a track for "Import To Collection" (audio, details,
//! cues, beat grid, analysis files, artwork, My Tags) and the players' My Settings files.

use super::anlz;
use super::settings::{crc16_xmodem, UsbSettings};
use super::usb::{self, DevTrack, DeviceLibrary};
use crate::model::*;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// A device track copied into the collection's folders, ready to be added to the library.
pub struct ImportedTrack {
    pub dev_id: u32,
    pub track: Track,
    pub cues: Vec<Cue>,
    pub grid: Option<BeatGrid>,
    /// Copy of the device's analysis files (base path without extension), kept so the
    /// players' own waveforms are shown and written back on the next export.
    pub anlz_base: Option<PathBuf>,
    /// My Tags as (category, tag).
    pub tags: Vec<(String, String)>,
    /// The audio file was already there (an earlier import): nothing was copied.
    pub reused_file: bool,
}

/// Where an imported device file goes: `<music_dir>/<artist>/<album>/<file name>`, the same
/// folder rule the export uses on the drive. A different file already at that name gets
/// " (2)", " (3)" …; the same file (same size) is reused.
pub fn import_destination(music_dir: &Path, dt: &DevTrack, size: u64) -> (PathBuf, bool) {
    let dir = music_dir.join(usb::media_dir(&dt.artist, &dt.album).trim_start_matches("/Contents/"));
    let name = Path::new(&dt.file_path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "track".into());
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.clone(), String::new()),
    };
    let mut n = 1;
    loop {
        let cand = dir.join(if n == 1 { name.clone() } else { format!("{stem} ({n}){ext}") });
        match std::fs::metadata(&cand) {
            Ok(m) if m.len() == size => return (cand, true),
            Ok(_) => n += 1,
            Err(_) => return (cand, false),
        }
    }
}

/// Copy one device track into `music_dir` and read everything the device knows about it.
/// Nothing on the device is changed. `data_dir` receives the copy of its analysis files and
/// `artwork_dir` its artwork when the audio file has none embedded.
pub fn import_track(root: &Path, lib: &DeviceLibrary, id: u32, music_dir: &Path, data_dir: &Path, artwork_dir: &Path) -> Result<ImportedTrack> {
    let dt = lib.tracks.get(&id).ok_or_else(|| anyhow!("the track is no longer on the device"))?;
    let src = root.join(dt.file_path.trim_start_matches('/'));
    let size = std::fs::metadata(&src).map(|m| m.len()).map_err(|_| anyhow!("file not found on the device"))?;
    let (dest, reused_file) = import_destination(music_dir, dt, size);
    if !reused_file {
        std::fs::create_dir_all(dest.parent().unwrap())?;
        let part = dest.with_file_name(format!("{}.part", dest.file_name().unwrap_or_default().to_string_lossy()));
        let copied = std::fs::copy(&src, &part).map_err(|e| anyhow!("could not copy the file: {e}"))?;
        if copied != size {
            let _ = std::fs::remove_file(&part);
            return Err(anyhow!("the copy is incomplete ({copied} of {size} bytes)"));
        }
        std::fs::rename(&part, &dest)?;
    }
    // The file's own tags first, then what the device library says (which is what the DJ saw
    // and edited in rekordbox), field by field where the device has a value.
    let mut t = crate::importer::read_track(&dest);
    let set = |to: &mut String, from: &str| {
        if !from.trim().is_empty() {
            *to = from.to_string();
        }
    };
    set(&mut t.title, &dt.title);
    set(&mut t.artist, &dt.artist);
    set(&mut t.album, &dt.album);
    set(&mut t.album_artist, &dt.album_artist);
    set(&mut t.genre, &dt.genre);
    set(&mut t.label, &dt.label);
    set(&mut t.remixer, &dt.remixer);
    set(&mut t.original_artist, &dt.original_artist);
    set(&mut t.composer, &dt.composer);
    set(&mut t.lyricist, &dt.lyricist);
    set(&mut t.comment, &dt.comment);
    set(&mut t.message, &dt.message);
    set(&mut t.release_date, &dt.release_date);
    // The device's title is the library's display title: a mix name is already part of it.
    if !dt.mix_name.is_empty() && !t.title.contains(&dt.mix_name) {
        t.mix_name = dt.mix_name.clone();
    } else {
        t.mix_name.clear();
    }
    if dt.year > 0 {
        t.year = dt.year as i32;
    }
    if dt.track_no > 0 {
        t.track_no = dt.track_no as i32;
    }
    if dt.disc_no > 0 {
        t.disc_no = dt.disc_no as i32;
    }
    if dt.bpm100 > 0 {
        t.bpm = dt.bpm100 as f64 / 100.0;
    }
    if !dt.key_name.is_empty() {
        t.key = parse_key(&dt.key_name);
        t.key_text = dt.key_name.clone();
    }
    t.rating = dt.rating.min(5);
    t.color = dt.color.min(8);
    t.play_count = dt.play_count as u32;
    t.autoload_hotcue = dt.autoload;
    if dt.date_added.len() == 10 {
        t.date_added = dt.date_added.clone();
    }
    // Analysis files: cues and grid are read, and the files are copied next to the library.
    let mut cues = vec![];
    let mut grid = None;
    let mut anlz_base = None;
    if let Some(base) = usb::anlz_base(&dt.analyze_path) {
        let b = root.join(base.trim_start_matches('/'));
        let (ext, dat) = (anlz::read_valid_section_list(&b.with_extension("EXT")), anlz::read_valid_section_list(&b.with_extension("DAT")));
        cues = anlz::parse_cues(&ext, &dat);
        grid = crate::rbimport::read_pqtz_grid(&b.with_extension("DAT")).filter(|g| !g.is_empty());
        if !dat.is_empty() {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in dest.to_string_lossy().bytes() {
                h = (h ^ byte as u64).wrapping_mul(0x0100_0000_01b3);
            }
            let dir = data_dir.join("device-anlz").join(format!("{h:016x}"));
            std::fs::create_dir_all(&dir)?;
            for e in usb::ANLZ_EXTS {
                let from = b.with_extension(e);
                if from.is_file() {
                    std::fs::copy(&from, dir.join(format!("ANLZ0000.{e}")))?;
                }
            }
            anlz_base = Some(dir.join("ANLZ0000"));
        }
    }
    // Artwork: the file's embedded picture when it has one, else the device's (the larger
    // "_m" version when it exists).
    if t.artwork.is_empty() && !dt.artwork_path.is_empty() {
        let small = root.join(dt.artwork_path.trim_start_matches('/'));
        let big = small.with_file_name(format!("{}_m.jpg", small.file_stem().unwrap_or_default().to_string_lossy()));
        if let Some(from) = [big, small].into_iter().find(|p| p.is_file()) {
            if let Ok(hash) = usb::sha256_file(&from) {
                let _ = std::fs::create_dir_all(artwork_dir);
                let to = artwork_dir.join(format!("device-{}.jpg", &usb::hex(hash)[..16]));
                if to.is_file() || std::fs::copy(&from, &to).is_ok() {
                    t.artwork = to.to_string_lossy().to_string();
                }
            }
        }
    }
    let tags = dt
        .tags
        .iter()
        .map(|k| match k.split_once('\u{1f}') {
            Some((c, n)) => (c.to_string(), n.to_string()),
            None => (String::new(), k.clone()),
        })
        .collect();
    Ok(ImportedTrack { dev_id: id, track: t, cues, grid, anlz_base, tags, reused_file })
}

/// The My Settings files on a drive: (file name, payload length, checksum over whole file).
const SETTING_FILES: [(&str, usize, bool); 4] = [("MYSETTING.DAT", 40, false), ("MYSETTING2.DAT", 40, false), ("DEVSETTING.DAT", 32, false), ("DJMMYSETTING.DAT", 52, true)];
/// The files "Delete My Settings" removes (the players' and mixers' settings; DEVSETTING.DAT
/// holds the device's display settings and stays).
pub const MY_SETTING_FILES: [&str; 3] = ["MYSETTING.DAT", "MYSETTING2.DAT", "DJMMYSETTING.DAT"];

/// Payload of a settings file when its lengths and CRC-16/XMODEM checksum are right.
fn valid_payload(bytes: &[u8], len: usize, whole: bool) -> Option<Vec<u8>> {
    if bytes.len() != 108 + len || u32::from_le_bytes(bytes.get(100..104)?.try_into().ok()?) as usize != len {
        return None;
    }
    let crc = u32::from_le_bytes(bytes[104 + len..108 + len].try_into().ok()?) as u16;
    let covered = if whole { &bytes[..104 + len] } else { &bytes[104..104 + len] };
    (crc == crc16_xmodem(covered)).then(|| bytes[104..104 + len].to_vec())
}

/// "Import My Settings": the drive's settings files laid over `current`. Only files that are
/// present and valid are taken; returns the result and the names of the files read (empty
/// when the drive has none: "My Settings file is not available.").
pub fn read_my_settings(root: &Path, current: &UsbSettings) -> (UsbSettings, Vec<&'static str>) {
    let mut out = current.clone().normalized();
    let mut found = vec![];
    for dir in ["PIONEER", ".PIONEER"] {
        for (name, len, whole) in SETTING_FILES {
            if found.contains(&name) {
                continue;
            }
            let Ok(bytes) = std::fs::read(root.join(dir).join(name)) else { continue };
            let Some(p) = valid_payload(&bytes, len, whole) else { continue };
            match name {
                "MYSETTING.DAT" => out.my = p,
                "MYSETTING2.DAT" => out.my2 = p,
                "DEVSETTING.DAT" => out.dev = p,
                _ => out.djm = p,
            }
            found.push(name);
        }
    }
    (out, found)
}

/// Whether the drive has any of the My Settings files.
pub fn has_my_settings(root: &Path) -> bool {
    ["PIONEER", ".PIONEER"].iter().any(|d| MY_SETTING_FILES.iter().any(|f| root.join(d).join(f).is_file()))
}

/// "Delete My Settings": remove the players' and mixers' settings files from the drive.
/// Returns how many files were deleted.
pub fn delete_my_settings(root: &Path) -> usize {
    let mut n = 0;
    for dir in ["PIONEER", ".PIONEER"] {
        for f in MY_SETTING_FILES {
            if std::fs::remove_file(root.join(dir).join(f)).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// Recordbox's own per-device properties (`/.Recordbox/props.json`).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DeviceProps {
    /// Background Color: 0 Default Color, 1–8 Pink … Purple. The standard device library
    /// (export.pdb, DEVSETTING.DAT) has no field for it; rekordbox keeps it in OneLibrary's
    /// `property.backGroundColorType`, which Recordbox does not write. Kept here so the
    /// choice travels with the drive and colours its row in Recordbox.
    pub bg_color: u8,
}

pub fn read_props(root: &Path) -> DeviceProps {
    std::fs::read_to_string(root.join(".Recordbox").join("props.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write_props(root: &Path, p: &DeviceProps) -> Result<()> {
    std::fs::create_dir_all(root.join(".Recordbox"))?;
    usb::write_atomic(&root.join(".Recordbox").join("props.json"), &serde_json::to_vec_pretty(p)?)
}

/// Which library playlists a device is kept in sync with (`/.Recordbox/sync.json`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// "All playlists" (true) or "Selected playlists".
    pub all: bool,
    /// Library IDs of the ticked playlists and folders.
    pub playlists: Vec<i64>,
    /// "Automatic synchronization": offered each time the device is connected.
    pub auto: bool,
    /// "including cue points": tracks already on the device get the library's cues again.
    pub cues: bool,
    /// The library (computer) this device is synchronised with.
    pub library: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig { all: false, playlists: vec![], auto: false, cues: true, library: String::new() }
    }
}

pub fn read_sync(root: &Path) -> Option<SyncConfig> {
    serde_json::from_str(&std::fs::read_to_string(root.join(".Recordbox").join("sync.json")).ok()?).ok()
}

pub fn write_sync(root: &Path, c: &SyncConfig) -> Result<()> {
    std::fs::create_dir_all(root.join(".Recordbox"))?;
    usb::write_atomic(&root.join(".Recordbox").join("sync.json"), &serde_json::to_vec_pretty(c)?)
}
