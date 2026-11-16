//! Reading track metadata and artwork from audio files (read-only) and importing a rekordbox
//! XML collection export (read-only).

use crate::model::*;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::{Accessor, ItemKey};
use std::path::Path;

pub fn read_track(path: &Path) -> Track {
    let mut t = Track {
        path: path.to_string_lossy().to_string(),
        key: -1,
        autoload_hotcue: true,
        file_type: path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase(),
        date_added: chrono::Local::now().format("%Y-%m-%d").to_string(),
        ..Default::default()
    };
    if let Ok(md) = std::fs::metadata(path) {
        t.file_size = md.len();
        if let Ok(c) = md.created() {
            let dt: chrono::DateTime<chrono::Local> = c.into();
            t.date_created = dt.format("%Y-%m-%d").to_string();
        }
    }
    let Ok(tagged) = lofty::read_from_path(path) else {
        t.title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        return t;
    };
    let props = tagged.properties();
    t.duration = props.duration().as_secs_f64();
    t.sample_rate = props.sample_rate().unwrap_or(0);
    t.bitrate = props.audio_bitrate().unwrap_or(0);
    t.bit_depth = props.bit_depth().unwrap_or(0) as u32;
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        t.title = tag.title().map(|s| s.to_string()).unwrap_or_default();
        t.artist = tag.artist().map(|s| s.to_string()).unwrap_or_default();
        t.album = tag.album().map(|s| s.to_string()).unwrap_or_default();
        t.genre = tag.genre().map(|s| s.to_string()).unwrap_or_default();
        t.comment = tag.comment().map(|s| s.to_string()).unwrap_or_default();
        t.year = tag.year().unwrap_or(0) as i32;
        t.track_no = tag.track().unwrap_or(0) as i32;
        t.disc_no = tag.disk().unwrap_or(0) as i32;
        let g = |k: ItemKey| tag.get_string(&k).map(|s| s.to_string()).unwrap_or_default();
        t.album_artist = g(ItemKey::AlbumArtist);
        t.label = g(ItemKey::Label);
        t.remixer = g(ItemKey::Remixer);
        t.original_artist = g(ItemKey::OriginalArtist);
        t.composer = g(ItemKey::Composer);
        t.lyricist = g(ItemKey::Lyricist);
        t.grouping = g(ItemKey::ContentGroup);
        t.mix_name = g(ItemKey::TrackSubtitle);
        let rd = g(ItemKey::RecordingDate);
        if rd.len() >= 4 {
            t.release_date = rd.clone();
            if t.year == 0 {
                t.year = rd[..4].parse().unwrap_or(0);
            }
        }
        let bpm = g(ItemKey::Bpm);
        if let Ok(b) = bpm.trim().parse::<f64>() {
            if b > 0.0 && b < 1000.0 {
                t.bpm = b;
            }
        }
        let key = g(ItemKey::InitialKey);
        if !key.is_empty() {
            t.key = parse_key(&key);
        }
        if let Some(pic) = tag.pictures().first() {
            let data = pic.data();
            let ext = match pic.mime_type() {
                Some(lofty::picture::MimeType::Png) => "png",
                _ => "jpg",
            };
            let mut h: u64 = 1469598103934665603;
            for b in data.iter().step_by(7) {
                h = (h ^ *b as u64).wrapping_mul(1099511628211);
            }
            h ^= data.len() as u64;
            let out = crate::db::artwork_dir().join(format!("{h:016x}.{ext}"));
            if !out.exists() {
                let _ = std::fs::write(&out, data);
            }
            t.artwork = out.to_string_lossy().to_string();
        }
    }
    if t.title.trim().is_empty() {
        t.title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    }
    t
}

/// Collect audio files from files and folders.
pub fn collect_audio(paths: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for p in paths {
        if p.is_dir() {
            for e in walkdir::WalkDir::new(p).follow_links(true).into_iter().flatten() {
                let ep = e.path();
                let hidden = ep.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with('.')).unwrap_or(false);
                if e.file_type().is_file() && !hidden && crate::audio::has_audio_extension(ep) {
                    out.push(ep.to_path_buf());
                }
            }
        } else if crate::audio::has_audio_extension(p) {
            out.push(p.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

// ------------------------------------------------------------------------------------------
// rekordbox XML (File > Export Collection in xml format) import
// ------------------------------------------------------------------------------------------

pub struct XmlTrack {
    pub xml_id: String,
    pub track: Track,
    pub grid: Option<BeatGrid>,
    pub cues: Vec<Cue>,
}

pub struct XmlNode {
    pub name: String,
    pub folder: bool,
    pub children: Vec<XmlNode>,
    pub track_keys: Vec<String>,
}

pub struct XmlLibrary {
    pub tracks: Vec<XmlTrack>,
    pub playlists: Vec<XmlNode>,
}

fn decode_location(loc: &str) -> String {
    let s = loc.strip_prefix("file://localhost").or_else(|| loc.strip_prefix("file://")).unwrap_or(loc);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn parse_rekordbox_xml(text: &str) -> anyhow::Result<XmlLibrary> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut tracks: Vec<XmlTrack> = vec![];
    let mut stack: Vec<XmlNode> = vec![];
    let mut roots: Vec<XmlNode> = vec![];
    let mut in_collection = false;
    let mut cur: Option<XmlTrack> = None;
    let mut tempos: Vec<(f64, f64, u8)> = vec![];
    loop {
        let ev = reader.read_event()?;
        let (e, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            Event::End(e) => {
                let name = e.name();
                match name.as_ref() {
                    b"COLLECTION" => in_collection = false,
                    b"TRACK" if in_collection => {
                        if let Some(mut t) = cur.take() {
                            finish_grid(&mut t, &tempos);
                            tracks.push(t);
                        }
                        tempos.clear();
                    }
                    b"NODE" => {
                        if let Some(n) = stack.pop() {
                            if let Some(p) = stack.last_mut() {
                                p.children.push(n);
                            } else {
                                roots.push(n);
                            }
                        }
                    }
                    _ => {}
                }
                (None, false)
            }
            Event::Eof => break,
            _ => (None, false),
        };
        let Some(e) = e else { continue };
        let mut attrs = std::collections::HashMap::new();
        for a in e.attributes().flatten() {
            let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
            let v = a.unescape_value().map(|v| v.to_string()).unwrap_or_default();
            attrs.insert(k, v);
        }
        let get = |k: &str| attrs.get(k).cloned().unwrap_or_default();
        let num = |k: &str| attrs.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        match e.name().as_ref() {
            b"COLLECTION" => in_collection = true,
            b"TRACK" if in_collection => {
                let loc = decode_location(&get("Location"));
                let t = Track {
                    path: loc,
                    title: get("Name"),
                    artist: get("Artist"),
                    composer: get("Composer"),
                    album: get("Album"),
                    grouping: get("Grouping"),
                    genre: get("Genre"),
                    file_type: get("Kind").split_whitespace().next().unwrap_or("").to_ascii_lowercase(),
                    file_size: num("Size") as u64,
                    duration: num("TotalTime"),
                    disc_no: num("DiscNumber") as i32,
                    track_no: num("TrackNumber") as i32,
                    year: num("Year") as i32,
                    bpm: num("AverageBpm"),
                    date_added: get("DateAdded"),
                    bitrate: num("BitRate") as u32,
                    sample_rate: num("SampleRate") as u32,
                    comment: get("Comments"),
                    play_count: num("PlayCount") as u32,
                    rating: match num("Rating") as u32 {
                        0 => 0,
                        r if r <= 51 => 1,
                        r if r <= 102 => 2,
                        r if r <= 153 => 3,
                        r if r <= 204 => 4,
                        _ => 5,
                    },
                    remixer: get("Remixer"),
                    key: parse_key(&get("Tonality")),
                    label: get("Label"),
                    mix_name: get("Mix"),
                    autoload_hotcue: true,
                    analyzed: 0,
                    ..Default::default()
                };
                let colour = get("Colour").to_ascii_uppercase();
                let color = match colour.trim_start_matches("0X") {
                    "FF007F" => 1,
                    "FF0000" => 2,
                    "FFA500" => 3,
                    "FFFF00" => 4,
                    "00FF00" => 5,
                    "25FDE9" => 6,
                    "0000FF" => 7,
                    "660099" => 8,
                    _ => 0,
                };
                let xt = XmlTrack { xml_id: get("TrackID"), track: Track { color, ..t }, grid: None, cues: vec![] };
                if empty {
                    tracks.push(xt);
                } else {
                    cur = Some(xt);
                }
            }
            b"TEMPO" => {
                tempos.push((num("Inizio"), num("Bpm"), get("Battito").parse::<u8>().unwrap_or(1)));
            }
            b"POSITION_MARK" => {
                if let Some(t) = cur.as_mut() {
                    let start = num("Start");
                    let ty = num("Type") as i32;
                    let end = if ty == 4 { attrs.get("End").and_then(|v| v.parse::<f64>().ok()) } else { None };
                    let n = attrs.get("Num").and_then(|v| v.parse::<i32>().ok()).unwrap_or(-1);
                    let (r, g, b) = (num("Red") as u8, num("Green") as u8, num("Blue") as u8);
                    let kind = if n >= 0 { CueKind::Hot } else { CueKind::Memory };
                    let color = if kind == CueKind::Hot && (r, g, b) != (0, 0, 0) { nearest_hot_color([r, g, b]) } else if kind == CueKind::Hot { HOTCUE_DEFAULT } else { 0 };
                    t.cues.push(Cue { id: 0, track_id: 0, kind, slot: n, start, end, color, comment: get("Name"), active_loop: false });
                }
            }
            b"NODE" => {
                let ty = get("Type");
                let name = get("Name");
                let node = XmlNode { name, folder: ty == "0", children: vec![], track_keys: vec![] };
                if empty {
                    if let Some(p) = stack.last_mut() {
                        p.children.push(node);
                    } else {
                        roots.push(node);
                    }
                } else {
                    stack.push(node);
                }
            }
            b"TRACK" if !in_collection => {
                if let Some(n) = stack.last_mut() {
                    n.track_keys.push(get("Key"));
                }
            }
            _ => {}
        }
    }
    // The XML wraps everything in a ROOT folder node.
    let playlists = if roots.len() == 1 && roots[0].name == "ROOT" { roots.pop().unwrap().children } else { roots };
    Ok(XmlLibrary { tracks, playlists })
}

fn finish_grid(t: &mut XmlTrack, tempos: &[(f64, f64, u8)]) {
    if tempos.is_empty() {
        return;
    }
    let mut anchors = vec![];
    for (i, (time, bpm, beat)) in tempos.iter().enumerate() {
        if i > 0 {
            let prev: &GridAnchor = anchors.last().unwrap();
            if (prev.bpm - bpm).abs() < 1e-6 {
                continue;
            }
        }
        anchors.push(GridAnchor { time: *time, bpm: *bpm, beat: (*beat).clamp(1, 4) });
    }
    t.grid = Some(BeatGrid { anchors });
}

pub fn nearest_hot_color(rgb: [u8; 3]) -> u8 {
    let mut best = (u32::MAX, HOTCUE_DEFAULT);
    for (i, v) in HOTCUE_PALETTE.iter().enumerate() {
        let c = [(v >> 16) as u8, (v >> 8) as u8, *v as u8];
        let d: u32 = (0..3).map(|k| (c[k] as i32 - rgb[k] as i32).pow(2) as u32).sum();
        if d < best.0 {
            best = (d, i as u8 + 1);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_xml() {
        let x = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0"><PRODUCT Name="x" Version="7"/>
<COLLECTION Entries="1">
<TRACK TrackID="7" Name="Song" Artist="Me" Kind="MP3 File" TotalTime="200" AverageBpm="128.00" Tonality="Am" Rating="153" Location="file://localhost/Users/a/Music/My%20Song.mp3" Colour="0xFF0000">
<TEMPO Inizio="0.025" Bpm="128.00" Metro="4/4" Battito="1"/>
<POSITION_MARK Name="Drop" Type="0" Start="32.025" Num="0" Red="230" Green="40" Blue="40"/>
<POSITION_MARK Name="" Type="4" Start="1.0" End="2.875" Num="-1"/>
</TRACK></COLLECTION>
<PLAYLISTS><NODE Type="0" Name="ROOT" Count="1"><NODE Name="Set" Type="1" KeyType="0" Entries="1"><TRACK Key="7"/></NODE></NODE></PLAYLISTS>
</DJ_PLAYLISTS>"#;
        let lib = parse_rekordbox_xml(x).unwrap();
        assert_eq!(lib.tracks.len(), 1);
        let t = &lib.tracks[0];
        assert_eq!(t.track.path, "/Users/a/Music/My Song.mp3");
        assert_eq!(t.track.rating, 3);
        assert_eq!(t.track.color, 2);
        assert_eq!(t.cues.len(), 2);
        assert_eq!(t.cues[0].kind, CueKind::Hot);
        assert!(t.cues[1].is_loop());
        assert_eq!(t.grid.as_ref().unwrap().anchors.len(), 1);
        assert_eq!(lib.playlists.len(), 1);
        assert_eq!(lib.playlists[0].track_keys, vec!["7"]);
    }
}
