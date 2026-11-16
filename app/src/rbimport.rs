//! Reading the user's existing rekordbox library (master.db).
//!
//! rekordbox's own files are never opened by a database engine and never written. On every
//! launch the database and its write-ahead log are copied into Recordbox's data folder (and the
//! copy is checked against the source so a write by a running rekordbox mid-copy is detected and
//! retried). Only that copy is opened, read-only, and synced into Recordbox's own library.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// SQLCipher passphrase of rekordbox 6/7's master.db. It is the same for every installation and
/// has been public for years (pyrekordbox and other library tools use it).
const MASTER_DB_KEY: &str = "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497";

/// Where rekordbox keeps its library on this machine, if it is installed.
pub fn library_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let d = dirs::home_dir()?.join("Library/Pioneer/rekordbox");
    #[cfg(target_os = "windows")]
    let d = dirs::data_dir()?.join("Pioneer/rekordbox");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let d = dirs::data_dir()?.join("Pioneer/rekordbox");
    d.join("master.db").is_file().then_some(d)
}

fn stamp(p: &Path) -> Option<(u64, SystemTime)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// Copy master.db and its -wal into `dest_dir`. The copy is only accepted when the source's
/// size and modification time were the same before and after copying (rekordbox may be running
/// and writing). Retries a few times. Returns the path of the copied master.db.
pub fn snapshot(src_dir: &Path, dest_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dest_dir)?;
    let names = ["master.db", "master.db-wal"];
    for attempt in 0..5 {
        let before: Vec<_> = names.iter().map(|n| stamp(&src_dir.join(n))).collect();
        let mut ok = true;
        for n in names {
            let s = src_dir.join(n);
            let d = dest_dir.join(n);
            let _ = std::fs::remove_file(&d);
            if s.exists() {
                // A plain byte copy: the source is opened read-only and nothing else touches it.
                if std::fs::copy(&s, &d).is_err() {
                    ok = false;
                }
            }
        }
        // The -shm index is rebuilt from the WAL by SQLite; a stale copy must not be used.
        let _ = std::fs::remove_file(dest_dir.join("master.db-shm"));
        let after: Vec<_> = names.iter().map(|n| stamp(&src_dir.join(n))).collect();
        if ok && before == after {
            return Ok(dest_dir.join("master.db"));
        }
        std::thread::sleep(std::time::Duration::from_millis(200 * (attempt + 1)));
    }
    Err(anyhow!("rekordbox kept changing its library while it was being copied; try again in a moment"))
}

/// Open a snapshot copy made by [`snapshot`]. The connection is read-only, but SQLite still
/// needs to read the copied WAL, so the copy (never the original) must be in a writable folder.
pub fn open_snapshot(db: &Path) -> Result<rusqlite::Connection> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("opening {}", db.display()))?;
    c.pragma_update(None, "key", MASTER_DB_KEY)?;
    // Fails with "file is not a database" when the key or format does not match.
    let n: i64 = c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0)).context("the rekordbox library could not be read (unsupported version?)")?;
    if n == 0 {
        return Err(anyhow!("the rekordbox library is empty or unreadable"));
    }
    // A copy taken while rekordbox was writing could be inconsistent: check it first.
    // Full integrity check (also verifies index contents, unlike quick_check).
    let check: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap_or_default();
    if check != "ok" {
        return Err(anyhow!("the copy of the rekordbox library failed its integrity check ({check}); try again with rekordbox closed"));
    }
    Ok(c)
}


use crate::importer::{XmlNode, XmlTrack};
use crate::model::*;
use rusqlite::types::Value;
use std::collections::HashMap;

/// One rekordbox track as Recordbox needs it, plus what is used to notice later changes.
pub struct RbTrack {
    pub rb_id: String,
    /// Changes whenever rekordbox changes the track, its cues or its My Tags.
    pub fingerprint: String,
    pub xt: XmlTrack,
    /// (category, tag) My Tag assignments.
    pub tags: Vec<(String, String)>,
    /// rekordbox's own analysis files for this track, without extension (".../ANLZ0000").
    pub anlz_base: Option<std::path::PathBuf>,
}

pub struct RbLibrary {
    pub tracks: Vec<RbTrack>,
    /// Playlist tree; `track_keys` are rekordbox content IDs in playlist order.
    pub playlists: Vec<XmlNode>,
    /// My Tag categories in rekordbox's order, each with its tags.
    pub tag_categories: Vec<(String, Vec<String>)>,
    /// Tracks without a local file (streaming services or moved files).
    pub not_local: usize,
    /// Intelligent playlists that use a condition Recordbox has no equivalent for (not copied).
    pub smart_lists: usize,
    /// Play histories (rekordbox's Histories tree); `track_keys` are content IDs.
    pub histories: Vec<XmlNode>,
}

/// The tag placeholder a translated My Tag condition carries until the sync knows Recordbox's
/// tag ids.
pub fn tag_placeholder(cat: &str, tag: &str) -> String {
    format!("\u{1}{cat}\u{1f}{tag}")
}

/// Translate a rekordbox intelligent playlist (djmdPlaylist.SmartList XML: a NODE with
/// LogicalOperator 1 = all / 2 = any and CONDITION children). None when a condition has no
/// Recordbox equivalent, since dropping it would change what the list matches.
pub fn translate_smartlist(xml: &str, tag_of: &HashMap<String, (String, String)>, keys: &HashMap<String, String>) -> Option<crate::smart::SmartRule> {
    use crate::smart::{Condition, Field, Op, SmartRule};
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut rule = SmartRule { match_all: true, conditions: vec![], auto_update: true };
    let mut saw_node = false;
    loop {
        let ev = reader.read_event().ok()?;
        let e = match &ev {
            Event::Start(e) | Event::Empty(e) => e.clone(),
            Event::Eof => break,
            _ => continue,
        };
        let mut a: HashMap<String, String> = HashMap::new();
        for at in e.attributes().flatten() {
            a.insert(String::from_utf8_lossy(at.key.as_ref()).to_string(), at.unescape_value().map(|v| v.to_string()).unwrap_or_default());
        }
        let get = |k: &str| a.get(k).cloned().unwrap_or_default();
        match e.name().as_ref() {
            b"NODE" => {
                saw_node = true;
                rule.match_all = get("LogicalOperator") != "2";
                rule.auto_update = get("AutomaticUpdate") != "0";
            }
            b"CONDITION" => {
                let field = match get("PropertyName").as_str() {
                    "artist" => Field::Artist,
                    "album" => Field::Album,
                    "albumArtist" => Field::AlbumArtist,
                    "originalArtist" => Field::OriginalArtist,
                    "bpm" => Field::Bpm,
                    // rekordbox's "grouping" property is the track colour (ColorID).
                    "grouping" => Field::Color,
                    "comments" => Field::Comments,
                    "producer" => Field::Composer,
                    "stockDate" => Field::DateAdded,
                    "counter" => Field::PlayCount,
                    "fileName" => Field::FileName,
                    "genre" => Field::Genre,
                    "key" => Field::Key,
                    "label" => Field::Label,
                    "mixName" => Field::MixName,
                    "myTag" => Field::MyTag,
                    "rating" => Field::Rating,
                    "dateReleased" => Field::ReleaseDate,
                    "remixedBy" => Field::Remixer,
                    "duration" => Field::Time,
                    "name" => Field::Title,
                    "year" => Field::Year,
                    _ => return None,
                };
                let op = match get("Operator").trim().parse::<u8>().ok()? {
                    1 => Op::Is,
                    2 => Op::IsNot,
                    3 => Op::Greater,
                    4 => Op::Less,
                    5 => Op::InRange,
                    6 => Op::InLast,
                    7 => Op::NotInLast,
                    8 => Op::Contains,
                    9 => Op::NotContains,
                    10 => Op::StartsWith,
                    11 => Op::EndsWith,
                    _ => return None,
                };
                if !Op::for_type(field.ftype()).contains(&op) {
                    return None;
                }
                let (mut value, mut value2) = (get("ValueLeft"), get("ValueRight"));
                match field {
                    Field::MyTag => {
                        // Tag IDs above 2^31 are stored sign-shifted.
                        let id = value.trim().parse::<i64>().ok()?;
                        let id = if id < 0 { id + (1i64 << 32) } else { id };
                        let (c, t) = tag_of.get(&id.to_string())?;
                        value = tag_placeholder(c, t);
                    }
                    Field::Key => {
                        if let Some(name) = keys.get(value.trim()) {
                            value = name.clone();
                        }
                        if crate::model::parse_key(&value) < 0 {
                            return None;
                        }
                    }
                    Field::Bpm => {
                        // Stored as BPM, or ×100 like djmdContent.
                        for v in [&mut value, &mut value2] {
                            if let Ok(x) = v.trim().parse::<f64>() {
                                if x > 1000.0 {
                                    *v = format!("{}", x / 100.0);
                                }
                            }
                        }
                    }
                    _ => {}
                }
                rule.conditions.push(Condition { field, op, value, value2, months: get("ValueUnit") == "month" });
            }
            _ => {}
        }
    }
    (saw_node && !rule.conditions.is_empty()).then_some(rule)
}

fn text(v: Value) -> String {
    match v {
        Value::Text(s) => s,
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        _ => String::new(),
    }
}
fn int(v: &Value) -> i64 {
    match v {
        Value::Integer(i) => *i,
        Value::Real(f) => *f as i64,
        Value::Text(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Run `sql` and return each row as a name -> value map.
fn rows(c: &rusqlite::Connection, sql: &str) -> Result<Vec<HashMap<String, Value>>> {
    let mut st = c.prepare(sql)?;
    let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
    let out = st
        .query_map([], |r| {
            let mut m = HashMap::new();
            for (i, n) in names.iter().enumerate() {
                m.insert(n.clone(), r.get::<_, Value>(i)?);
            }
            Ok(m)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

fn names(c: &rusqlite::Connection, table: &str) -> Result<HashMap<String, String>> {
    Ok(rows(c, &format!("SELECT ID, Name FROM {table} WHERE rb_local_deleted = 0"))?
        .into_iter()
        .map(|mut r| (text(r.remove("ID").unwrap_or(Value::Null)), text(r.remove("Name").unwrap_or(Value::Null))))
        .collect())
}

/// Beat grid from rekordbox's own analysis file (the PQTZ section of ANLZxxxx.DAT): one
/// anchor at the first beat and one wherever the tempo changes.
pub fn read_pqtz_grid(dat: &Path) -> Option<BeatGrid> {
    let b = std::fs::read(dat).ok()?;
    if b.get(0..4)? != b"PMAI" {
        return None;
    }
    let be32 = |o: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?)) };
    let be16 = |o: usize| -> Option<u16> { Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?)) };
    let mut i = be32(4)? as usize;
    while i + 12 <= b.len() {
        let total = be32(i + 8)? as usize;
        if total < 12 {
            return None;
        }
        if &b[i..i + 4] == b"PQTZ" {
            let n = be32(i + 20)? as usize;
            let mut anchors: Vec<GridAnchor> = vec![];
            let mut last_tempo = u16::MAX;
            for k in 0..n {
                let o = i + 24 + k * 8;
                let beat = be16(o)? as u8;
                let tempo = be16(o + 2)?;
                let ms = be32(o + 4)?;
                if tempo != last_tempo {
                    anchors.push(GridAnchor { time: ms as f64 / 1000.0, bpm: tempo as f64 / 100.0, beat: beat.clamp(1, 4) });
                    last_tempo = tempo;
                }
            }
            return (!anchors.is_empty()).then_some(BeatGrid { anchors });
        }
        i += total;
    }
    None
}

/// Cue kind in master.db -> (hot cue slot 0-based, or None for a memory cue). Kinds 1–3 are hot
/// cues A–C, kind 5 is D and 6–17 are E–P (rekordbox-rea RF-07); 0 and 4 are memory cues.
fn cue_slot(kind: i64) -> Option<i32> {
    match kind {
        1..=3 => Some(kind as i32 - 1),
        5 => Some(3),
        6..=17 => Some(kind as i32 - 2),
        _ => None,
    }
}

/// Read everything Recordbox imports from a snapshot opened with [`open_snapshot`].
/// `share_dir` is rekordbox's "share" folder, where its analysis files and artwork live.
pub fn read_library(c: &rusqlite::Connection, share_dir: &Path) -> Result<RbLibrary> {
    let artists = names(c, "djmdArtist")?;
    let albums = names(c, "djmdAlbum")?;
    let genres = names(c, "djmdGenre")?;
    let labels = names(c, "djmdLabel")?;
    let keys: HashMap<String, String> = rows(c, "SELECT ID, ScaleName FROM djmdKey WHERE rb_local_deleted = 0")?
        .into_iter()
        .map(|mut r| (text(r.remove("ID").unwrap_or(Value::Null)), text(r.remove("ScaleName").unwrap_or(Value::Null))))
        .collect();
    let lookup = |m: &HashMap<String, String>, v: Option<&Value>| v.map(|v| text(v.clone())).and_then(|id| m.get(&id).cloned()).unwrap_or_default();

    // Cues per track, with their newest change time for the fingerprint.
    let mut cues: HashMap<String, Vec<Cue>> = HashMap::new();
    let mut cue_stamp: HashMap<String, String> = HashMap::new();
    for r in rows(c, "SELECT ContentID, InMsec, OutMsec, Kind, Color, ActiveLoop, Comment, updated_at FROM djmdCue WHERE rb_local_deleted = 0 ORDER BY InMsec")? {
        let cid = text(r["ContentID"].clone());
        let kind = int(&r["Kind"]);
        let start = int(&r["InMsec"]).max(0) as f64 / 1000.0;
        let out = int(&r["OutMsec"]);
        let end = (out > 0 && out as f64 / 1000.0 > start).then_some(out as f64 / 1000.0);
        let color = int(&r["Color"]);
        let (ck, slot) = match cue_slot(kind) {
            Some(s) => (CueKind::Hot, s),
            None => (CueKind::Memory, -1),
        };
        let color = match ck {
            CueKind::Hot if (1..=64).contains(&color) => color as u8,
            CueKind::Hot => hotcue_default_color(slot, end.is_some()),
            _ if (1..=8).contains(&color) => color as u8,
            _ => 0,
        };
        cues.entry(cid.clone()).or_default().push(Cue { id: 0, track_id: 0, kind: ck, slot, start, end, color, comment: text(r["Comment"].clone()), active_loop: int(&r["ActiveLoop"]) != 0 });
        let u = text(r["updated_at"].clone());
        let e = cue_stamp.entry(cid).or_default();
        if u > *e {
            *e = u;
        }
    }

    // My Tag: categories are rows whose parent is "root".
    let tag_rows = rows(c, "SELECT ID, Name, ParentID, Seq, Attribute FROM djmdMyTag WHERE rb_local_deleted = 0 ORDER BY Seq")?;
    let mut tag_categories: Vec<(String, String, Vec<String>)> = vec![];
    let mut tag_of: HashMap<String, (String, String)> = HashMap::new();
    for r in tag_rows.iter().filter(|r| text(r["ParentID"].clone()) == "root") {
        tag_categories.push((text(r["ID"].clone()), text(r["Name"].clone()), vec![]));
    }
    for r in tag_rows.iter().filter(|r| text(r["ParentID"].clone()) != "root") {
        let parent = text(r["ParentID"].clone());
        if let Some(cat) = tag_categories.iter_mut().find(|c| c.0 == parent) {
            let name = text(r["Name"].clone());
            cat.2.push(name.clone());
            tag_of.insert(text(r["ID"].clone()), (cat.1.clone(), name));
        }
    }
    let mut song_tags: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for r in rows(c, "SELECT ContentID, MyTagID FROM djmdSongMyTag WHERE rb_local_deleted = 0")? {
        if let Some(t) = tag_of.get(&text(r["MyTagID"].clone())) {
            song_tags.entry(text(r["ContentID"].clone())).or_default().push(t.clone());
        }
    }

    let mut tracks = vec![];
    let mut not_local = 0;
    for r in rows(c, "SELECT * FROM djmdContent WHERE rb_local_deleted = 0")? {
        let rb_id = text(r["ID"].clone());
        let path = text(r["FolderPath"].clone());
        if !Path::new(&path).is_file() {
            not_local += 1;
            continue;
        }
        let g = |k: &str| r.get(k).cloned().map(text).unwrap_or_default();
        let n = |k: &str| r.get(k).map(int).unwrap_or(0);
        let ext = Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let image = g("ImagePath");
        let artwork = if image.is_empty() { String::new() } else { share_dir.join(image.trim_start_matches('/')).to_string_lossy().to_string() };
        let t = Track {
            path: path.clone(),
            title: g("Title"),
            artist: lookup(&artists, r.get("ArtistID")),
            album: lookup(&albums, r.get("AlbumID")),
            genre: lookup(&genres, r.get("GenreID")),
            label: lookup(&labels, r.get("LabelID")),
            remixer: lookup(&artists, r.get("RemixerID")),
            original_artist: lookup(&artists, r.get("OrgArtistID")),
            composer: lookup(&artists, r.get("ComposerID")),
            lyricist: g("Lyricist"),
            comment: g("Commnt"),
            message: String::new(),
            year: n("ReleaseYear") as i32,
            track_no: n("TrackNo") as i32,
            disc_no: n("DiscNo") as i32,
            release_date: g("ReleaseDate"),
            duration: n("Length") as f64,
            sample_rate: n("SampleRate") as u32,
            bitrate: n("BitRate") as u32,
            bit_depth: n("BitDepth") as u32,
            file_size: n("FileSize") as u64,
            file_type: ext,
            bpm: n("BPM") as f64 / 100.0,
            key: parse_key(&lookup(&keys, r.get("KeyID"))),
            key_text: lookup(&keys, r.get("KeyID")).trim().to_string(),
            rating: n("Rating").clamp(0, 5) as u8,
            color: n("ColorID").clamp(0, 8) as u8,
            date_added: g("StockDate"),
            date_created: g("DateCreated"),
            play_count: n("DJPlayCount").max(0) as u32,
            autoload_hotcue: g("HotCueAutoLoad") != "off",
            artwork: if Path::new(&artwork).is_file() { artwork } else { String::new() },
            analyzed: 0,
            ..Default::default()
        };
        let anlz = g("AnalysisDataPath");
        let grid = if anlz.is_empty() { None } else { read_pqtz_grid(&share_dir.join(anlz.trim_start_matches('/'))) };
        let anlz_base = (!anlz.is_empty()).then(|| share_dir.join(anlz.trim_start_matches('/')).with_extension("")).filter(|p| p.with_extension("DAT").is_file());
        let tags = song_tags.remove(&rb_id).unwrap_or_default();
        let mut tag_names: Vec<String> = tags.iter().map(|(c, t)| format!("{c}/{t}")).collect();
        tag_names.sort();
        let fingerprint = format!("{}|{}|{}|{}", g("updated_at"), cue_stamp.get(&rb_id).cloned().unwrap_or_default(), g("AnalysisUpdated"), tag_names.join(","));
        let xt = XmlTrack { xml_id: rb_id.clone(), track: t, grid, cues: cues.remove(&rb_id).unwrap_or_default(), extra: Default::default() };
        tracks.push(RbTrack { rb_id, fingerprint, xt, tags, anlz_base });
    }

    // Playlist tree.
    let pl = rows(c, "SELECT ID, Seq, Name, Attribute, ParentID, SmartList FROM djmdPlaylist WHERE rb_local_deleted = 0 ORDER BY Seq")?;
    let mut members: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for r in rows(c, "SELECT PlaylistID, ContentID, TrackNo FROM djmdSongPlaylist WHERE rb_local_deleted = 0")? {
        members.entry(text(r["PlaylistID"].clone())).or_default().push((int(&r["TrackNo"]), text(r["ContentID"].clone())));
    }
    let mut smart_lists = 0;
    struct Ctx<'a> {
        tag_of: &'a HashMap<String, (String, String)>,
        keys: &'a HashMap<String, String>,
        smart: usize,
    }
    fn build(parent: &str, pl: &[HashMap<String, Value>], members: &mut HashMap<String, Vec<(i64, String)>>, cx: &mut Ctx) -> Vec<XmlNode> {
        let mut out = vec![];
        for r in pl.iter().filter(|r| r.get("ParentID").map(|v| text(v.clone())).unwrap_or_default() == parent) {
            let id = text(r["ID"].clone());
            let folder = int(&r["Attribute"]) == 1;
            let smart_xml = r.get("SmartList").map(|v| text(v.clone())).unwrap_or_default();
            let smart = if smart_xml.is_empty() {
                None
            } else {
                match translate_smartlist(&smart_xml, cx.tag_of, cx.keys) {
                    Some(rule) => Some(rule),
                    None => {
                        cx.smart += 1;
                        continue;
                    }
                }
            };
            let mut keys = members.remove(&id).unwrap_or_default();
            keys.sort_by_key(|k| k.0);
            out.push(XmlNode { name: text(r["Name"].clone()), folder, children: if folder { build(&id, pl, members, cx) } else { vec![] }, track_keys: keys.into_iter().map(|k| k.1).collect(), by_location: false, smart });
        }
        out
    }
    let mut cx = Ctx { tag_of: &tag_of, keys: &keys, smart: 0 };
    let playlists = build("root", &pl, &mut members, &mut cx);
    // Histories: the same tree shape (year and month folders, one list per day).
    let hist = rows(c, "SELECT ID, Seq, Name, Attribute, ParentID FROM djmdHistory WHERE rb_local_deleted = 0 ORDER BY Seq").unwrap_or_default();
    let mut hmembers: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for r in rows(c, "SELECT HistoryID, ContentID, TrackNo FROM djmdSongHistory WHERE rb_local_deleted = 0").unwrap_or_default() {
        hmembers.entry(text(r["HistoryID"].clone())).or_default().push((int(&r["TrackNo"]), text(r["ContentID"].clone())));
    }
    let histories = build("root", &hist, &mut hmembers, &mut cx);
    smart_lists += cx.smart;
    Ok(RbLibrary {
        tracks,
        playlists,
        tag_categories: tag_categories.into_iter().map(|(_, n, t)| (n, t)).collect(),
        not_local,
        smart_lists,
        histories,
    })
}

/// Snapshot the installed rekordbox library into `dest_dir` and read it.
pub fn load_installed(dest_dir: &Path) -> Result<Option<RbLibrary>> {
    let Some(src) = library_dir() else { return Ok(None) };
    let db = snapshot(&src, dest_dir)?;
    let c = open_snapshot(&db)?;
    Ok(Some(read_library(&c, &src.join("share"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real library on this machine, if present (copy only). Run with --ignored.
    #[test]
    #[ignore]
    fn reads_local_library_snapshot() {
        let Some(src) = library_dir() else { return };
        let dest = std::env::temp_dir().join(format!("recordbox_rb_snapshot_{}", std::process::id()));
        let before = stamp(&src.join("master.db"));
        let db = snapshot(&src, &dest).unwrap();
        let c = open_snapshot(&db).unwrap();
        for t in ["djmdContent", "djmdPlaylist", "djmdSongPlaylist", "djmdCue", "djmdMyTag"] {
            let n: i64 = c.query_row(&format!("SELECT count(*) FROM {t} WHERE rb_local_deleted = 0"), [], |r| r.get(0)).unwrap();
            println!("{t}: {n}");
        }
        let lib = read_library(&c, &src.join("share")).unwrap();
        for t in lib.tracks.iter().take(6) {
            println!("key {:?} -> {} :: {}", t.xt.track.key_text, t.xt.track.key, t.xt.track.title);
        }
        let with_grid = lib.tracks.iter().filter(|t| t.xt.grid.is_some()).count();
        let hot = lib.tracks.iter().map(|t| t.xt.cues.iter().filter(|c| c.kind == CueKind::Hot).count()).sum::<usize>();
        println!("local tracks {}, not local {}, with grid {}, hot cues {}, playlists {}, smart {}, tag categories {}", lib.tracks.len(), lib.not_local, with_grid, hot, lib.playlists.len(), lib.smart_lists, lib.tag_categories.len());
        fn walk(n: &[XmlNode], d: usize) {
            for x in n {
                println!("{}history node {:?} folder={} tracks={}", "  ".repeat(d), x.name, x.folder, x.track_keys.len());
                walk(&x.children, d + 1);
            }
        }
        walk(&lib.histories, 0);
        assert_eq!(before, stamp(&src.join("master.db")), "the original must not change");
        let mut st = c.prepare("SELECT Name, SmartList FROM djmdPlaylist WHERE rb_local_deleted=0 AND SmartList IS NOT NULL AND SmartList != ''").unwrap();
        for r in st.query_map([], |r| Ok(format!("{} => {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap().flatten() {
            println!("smartlist: {r}");
        }
        for t in ["djmdHistory", "djmdSongHistory"] {
            if let Ok(mut st) = c.prepare(&format!("PRAGMA table_info({t})")) {
                let cols: Vec<String> = st.query_map([], |r| r.get::<_, String>(1)).unwrap().flatten().collect();
                let n: i64 = c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)).unwrap_or(-1);
                println!("{t} ({n} rows): {}", cols.join(","));
            }
        }
        if let Ok(mut st) = c.prepare("SELECT ID, Seq, Name, Attribute, ParentID, DateCreated FROM djmdHistory WHERE rb_local_deleted=0 LIMIT 10") {
            for r in st.query_map([], |r| Ok((0..6).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
                println!("history: {r}");
            }
        }
        let _ = std::fs::remove_dir_all(&dest);
    }
}

#[cfg(test)]
mod smartlist_tests {
    use super::*;
    use crate::smart::{Field, Op};

    #[test]
    fn translates_rekordbox_smartlists() {
        let tags = HashMap::from([("3937219461".to_string(), ("Mood".to_string(), "Dark".to_string()))]);
        let keys = HashMap::from([("5".to_string(), "Am".to_string())]);
        let xml = r#"<NODE Id="-12345" LogicalOperator="2" AutomaticUpdate="1"><CONDITION PropertyName="genre" Operator="8" ValueUnit="" ValueLeft="Techno &amp; House" ValueRight=""/><CONDITION PropertyName="bpm" Operator="5" ValueUnit="" ValueLeft="12000" ValueRight="128"/><CONDITION PropertyName="myTag" Operator="8" ValueUnit="" ValueLeft="-357747835" ValueRight=""/><CONDITION PropertyName="stockDate" Operator="6" ValueUnit="month" ValueLeft="3" ValueRight=""/><CONDITION PropertyName="key" Operator="1" ValueUnit="" ValueLeft="5" ValueRight=""/><CONDITION PropertyName="grouping" Operator="1" ValueUnit="" ValueLeft="2" ValueRight=""/></NODE>"#;
        let r = translate_smartlist(xml, &tags, &keys).unwrap();
        assert!(!r.match_all);
        assert_eq!(r.conditions.len(), 6);
        assert_eq!((r.conditions[0].field, r.conditions[0].op, r.conditions[0].value.as_str()), (Field::Genre, Op::Contains, "Techno & House"));
        assert_eq!((r.conditions[1].value.as_str(), r.conditions[1].value2.as_str()), ("120", "128"));
        assert_eq!(r.conditions[2].value, tag_placeholder("Mood", "Dark"));
        assert!(r.conditions[3].months && r.conditions[3].op == Op::InLast);
        assert_eq!(r.conditions[4].value, "Am");
        assert_eq!(r.conditions[5].field, Field::Color);
        // A condition with no equivalent keeps the whole list out.
        let odd = r#"<NODE Id="1" LogicalOperator="1" AutomaticUpdate="1"><CONDITION PropertyName="dateCreated" Operator="6" ValueUnit="day" ValueLeft="7" ValueRight=""/></NODE>"#;
        assert!(translate_smartlist(odd, &tags, &keys).is_none());
    }
}

#[cfg(test)]
mod schema_dump {
    #[test]
    #[ignore]
    fn dump_columns() {
        let Some(src) = super::library_dir() else { return };
        let dest = std::env::temp_dir().join(format!("recordbox_rb_schema_{}", std::process::id()));
        let c = super::open_snapshot(&super::snapshot(&src, &dest).unwrap()).unwrap();
        for t in ["djmdContent", "djmdCue", "djmdPlaylist", "djmdSongPlaylist", "djmdKey", "djmdArtist", "djmdAlbum", "djmdGenre", "djmdLabel", "djmdColor", "djmdMyTag", "djmdSongMyTag"] {
            let mut st = c.prepare(&format!("PRAGMA table_info({t})")).unwrap();
            let cols: Vec<String> = st.query_map([], |r| r.get::<_, String>(1)).unwrap().flatten().collect();
            println!("{t}: {}", cols.join(","));
        }
        let mut st = c.prepare("SELECT Kind, count(*), min(InMsec), max(OutMsec), min(Color), max(Color), min(ColorTableIndex), max(ColorTableIndex), sum(ActiveLoop) FROM djmdCue WHERE rb_local_deleted=0 GROUP BY Kind").unwrap();
        for r in st.query_map([], |r| Ok((0..9).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" "))).unwrap().flatten() {
            println!("cue kind: {r}");
        }
        let mut st = c.prepare("SELECT ScaleName FROM djmdKey LIMIT 30").unwrap();
        println!("keys: {:?}", st.query_map([], |r| r.get::<_, String>(0)).unwrap().flatten().collect::<Vec<_>>());
        let mut st = c.prepare("SELECT AnalysisDataPath, FolderPath, FileType, Rating, ColorID, DJPlayCount, BPM FROM djmdContent LIMIT 2").unwrap();
        for r in st.query_map([], |r| Ok((0..7).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
            println!("content sample: {r}");
        }
        let mut st = c.prepare("SELECT ID, Seq, Name, Attribute, ParentID, SmartList IS NOT NULL AND SmartList != '' FROM djmdPlaylist WHERE rb_local_deleted=0").unwrap();
        for r in st.query_map([], |r| Ok((0..6).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
            println!("playlist: {r}");
        }
        let mut st = c.prepare("SELECT Name, SmartList FROM djmdPlaylist WHERE rb_local_deleted=0 AND SmartList IS NOT NULL AND SmartList != ''").unwrap();
        for r in st.query_map([], |r| Ok(format!("{} => {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap().flatten() {
            println!("smartlist: {r}");
        }
        for t in ["djmdHistory", "djmdSongHistory"] {
            if let Ok(mut st) = c.prepare(&format!("PRAGMA table_info({t})")) {
                let cols: Vec<String> = st.query_map([], |r| r.get::<_, String>(1)).unwrap().flatten().collect();
                let n: i64 = c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)).unwrap_or(-1);
                println!("{t} ({n} rows): {}", cols.join(","));
            }
        }
        if let Ok(mut st) = c.prepare("SELECT ID, Seq, Name, Attribute, ParentID, DateCreated FROM djmdHistory WHERE rb_local_deleted=0 LIMIT 10") {
            for r in st.query_map([], |r| Ok((0..6).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
                println!("history: {r}");
            }
        }
        let _ = std::fs::remove_dir_all(&dest);
    }
}

#[cfg(test)]
mod onelibrary_probe {
    /// Does the master.db key open a OneLibrary catalog? Run with --ignored.
    #[test]
    #[ignore]
    fn probe_key() {
        let p = std::env::var("PREPDECK_ONELIB").unwrap();
        let c = rusqlite::Connection::open_with_flags(&p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let key = std::env::var("PREPDECK_ONELIB_KEY_FILE").ok().map(|f| std::fs::read_to_string(f).unwrap()).unwrap_or_else(|| super::MASTER_DB_KEY.to_string());
        c.pragma_update(None, "key", key.trim()).unwrap();
        match c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)) {
            Ok(n) => {
                println!("opened: {n} objects");
                let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type='table'").unwrap();
                println!("{:?}", st.query_map([], |r| r.get::<_, String>(0)).unwrap().flatten().collect::<Vec<_>>());
                if let Ok(out) = std::env::var("PREPDECK_ONELIB_DDL") { let out = out.as_str();
                    let mut st = c.prepare("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY rowid").unwrap();
                    let ddl: Vec<String> = st.query_map([], |r| Ok(format!("-- {} {}\n{};", r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).unwrap().flatten().collect();
                    std::fs::write(out, ddl.join("\n\n")).unwrap();
                    let mut rows = String::new();
                    for t in ["menuItem", "category", "sort", "property", "color", "key", "myTag", "image"] {
                        let mut st = c.prepare(&format!("SELECT * FROM {t}")).unwrap();
                        let n = st.column_count();
                        for r in st.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap())).collect::<Vec<_>>().join(", "))).unwrap().flatten() {
                            rows.push_str(&format!("{t}: {r}\n"));
                        }
                    }
                    std::fs::write(format!("{out}.rows"), rows).unwrap();
                    for pr in ["journal_mode", "page_size", "cipher_version", "kdf_iter", "cipher_page_size", "cipher_hmac_algorithm", "cipher_kdf_algorithm", "user_version", "encoding"] {
                        println!("{pr}: {:?}", c.query_row(&format!("PRAGMA {pr}"), [], |r| r.get::<_, rusqlite::types::Value>(0)));
                    }
                }
                println!("property: {:?}", c.query_row("SELECT deviceName, dbVersion, numberOfContents, createdDate FROM property", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))));
            }
            Err(e) => println!("not opened: {e}"),
        }
    }
}

#[cfg(test)]
mod cue_origin_probe {
    /// When were the user's cues created relative to the track's analysis? Run with --ignored.
    #[test]
    #[ignore]
    fn cue_creation_times() {
        let Some(src) = super::library_dir() else { return };
        let dest = std::env::temp_dir().join(format!("recordbox_cue_origin_{}", std::process::id()));
        let c = super::open_snapshot(&super::snapshot(&src, &dest).unwrap()).unwrap();
        let mut st = c.prepare("SELECT k.Kind, k.Comment, k.ColorTableIndex, k.created_at, c.created_at, c.updated_at, c.AnalysisUpdated FROM djmdCue k JOIN djmdContent c ON c.ID = k.ContentID WHERE k.rb_local_deleted = 0 LIMIT 12").unwrap();
        for r in st.query_map([], |r| Ok((0..7).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
            println!("cue: {r}");
        }
        for q in ["SELECT Analysed, count(*) FROM djmdContent WHERE rb_local_deleted = 0 GROUP BY 1", "SELECT group_concat(name, ' ') FROM pragma_table_info('djmdContent')", "SELECT Analysed, AnalysisDataPath IS NOT NULL, count(*) FROM djmdContent WHERE rb_local_deleted = 0 GROUP BY 1,2", "SELECT DBID FROM djmdProperty", "SELECT ID, MasterDBID, MasterSongID, ContentLink, ModifiedByRBM, SearchStr, DateCreated, StockDate, CueUpdated, AnalysisUpdated, TrackInfoUpdated, DeliveryControl, HotCueAutoLoad, Analysed FROM djmdContent WHERE rb_local_deleted = 0 LIMIT 6"] {
            let mut st = c.prepare(q).unwrap();
            let n = st.column_count();
            for r in st.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap())).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
                println!("{q}: {r}");
            }
        }
        let n: i64 = c.query_row("SELECT count(DISTINCT substr(created_at,1,16)) FROM djmdCue WHERE rb_local_deleted = 0", [], |r| r.get(0)).unwrap();
        println!("distinct creation minutes: {n}");
        let _ = std::fs::remove_dir_all(&dest);
    }
}
