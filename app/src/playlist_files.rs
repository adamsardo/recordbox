//! Playlist files: m3u / m3u8, pls and rekordbox's tab-separated playlist text
//! ("Export a playlist to a file (*.txt)", UTF-16 with a header row), written and read.

use std::path::{Path, PathBuf};

/// One playlist entry as the file formats carry it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub bpm: f64,
    pub rating: u8,
    /// Seconds.
    pub duration: f64,
    pub key: String,
    pub date_added: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    M3u8,
    Pls,
    Txt,
}

impl Format {
    /// From a file name's extension (m3u and m3u8 are the same format; m3u8 is UTF-8 by name).
    pub fn of(path: &Path) -> Option<Format> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "m3u" | "m3u8" => Some(Format::M3u8),
            "pls" => Some(Format::Pls),
            "txt" => Some(Format::Txt),
            _ => None,
        }
    }
}

fn artist_title(e: &Entry) -> String {
    if e.artist.trim().is_empty() {
        e.title.clone()
    } else {
        format!("{} - {}", e.artist, e.title)
    }
}

pub fn write_m3u8(entries: &[Entry]) -> String {
    let mut s = String::from("#EXTM3U\n");
    for e in entries {
        s.push_str(&format!("#EXTINF:{},{}\n{}\n", e.duration.round() as i64, artist_title(e).replace(['\n', '\r'], " "), e.path));
    }
    s
}

pub fn write_pls(entries: &[Entry]) -> String {
    let mut s = String::from("[playlist]\n");
    for (i, e) in entries.iter().enumerate() {
        let n = i + 1;
        s.push_str(&format!("File{n}={}\nTitle{n}={}\nLength{n}={}\n", e.path, artist_title(e).replace(['\n', '\r'], " "), e.duration.round() as i64));
    }
    s.push_str(&format!("NumberOfEntries={}\nVersion=2\n", entries.len()));
    s
}

/// Columns of the playlist text file, in order (rekordbox writes the track list's columns under
/// their header names; Location is what lets the file be imported again).
pub const TXT_COLUMNS: [&str; 11] = ["#", "Track Title", "Artist", "Album", "Genre", "BPM", "Rating", "Time", "Key", "Date Added", "Location"];

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn time_text(secs: f64) -> String {
    let t = secs.round().max(0.0) as i64;
    format!("{:02}:{:02}", t / 60, t % 60)
}

fn parse_time(s: &str) -> f64 {
    let parts: Vec<f64> = s.trim().split(':').map(|p| p.parse::<f64>().unwrap_or(0.0)).collect();
    parts.iter().fold(0.0, |acc, p| acc * 60.0 + p)
}

/// The text of a rekordbox playlist file (tab-separated, CRLF lines).
pub fn write_txt(entries: &[Entry]) -> String {
    let mut s = TXT_COLUMNS.join("\t");
    s.push_str("\r\n");
    for (i, e) in entries.iter().enumerate() {
        let cols = [
            (i + 1).to_string(),
            clean(&e.title),
            clean(&e.artist),
            clean(&e.album),
            clean(&e.genre),
            if e.bpm > 0.0 { format!("{:.2}", e.bpm) } else { String::new() },
            "*".repeat(e.rating.min(5) as usize) + &" ".repeat(5 - e.rating.min(5) as usize),
            time_text(e.duration),
            clean(&e.key),
            clean(&e.date_added),
            clean(&e.path),
        ];
        s.push_str(&cols.join("\t"));
        s.push_str("\r\n");
    }
    s
}

/// rekordbox saves its playlist text as UTF-16 (little endian, with a byte order mark).
pub fn utf16le_bom(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

/// File bytes for `format`.
pub fn encode(format: Format, entries: &[Entry]) -> Vec<u8> {
    match format {
        Format::M3u8 => write_m3u8(entries).into_bytes(),
        Format::Pls => write_pls(entries).into_bytes(),
        Format::Txt => utf16le_bom(&write_txt(entries)),
    }
}

/// Text of a playlist file: UTF-16 (either byte order, by its mark), UTF-8 (mark dropped), or
/// Latin-1 for old .m3u files that are not valid UTF-8.
pub fn decode_text(bytes: &[u8]) -> String {
    let u16s = |le: bool| -> String {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&units)
    };
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return u16s(true);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return u16s(false);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(body) {
        Ok(s) => s.to_string(),
        Err(_) => body.iter().map(|b| *b as char).collect(),
    }
}

/// A path written in a playlist: absolute as it is, `file://` URLs decoded, anything else
/// relative to the playlist file's folder.
pub fn resolve(base: &Path, raw: &str) -> String {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        return percent_decode(rest);
    }
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        raw.to_string()
    } else {
        base.join(p).to_string_lossy().to_string()
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn split_artist_title(s: &str) -> (String, String) {
    match s.split_once(" - ") {
        Some((a, t)) => (a.trim().to_string(), t.trim().to_string()),
        None => (String::new(), s.trim().to_string()),
    }
}

pub fn parse_m3u(text: &str, base: &Path) -> Vec<Entry> {
    let mut out = vec![];
    let mut pending: Option<(f64, String)> = None;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(info) = l.strip_prefix("#EXTINF:") {
            let (len, name) = info.split_once(',').unwrap_or((info, ""));
            pending = Some((len.trim().parse::<f64>().unwrap_or(0.0).max(0.0), name.to_string()));
            continue;
        }
        if l.starts_with('#') {
            continue;
        }
        let mut e = Entry { path: resolve(base, l), ..Default::default() };
        if let Some((len, name)) = pending.take() {
            e.duration = len;
            (e.artist, e.title) = split_artist_title(&name);
        }
        out.push(e);
    }
    out
}

pub fn parse_pls(text: &str, base: &Path) -> Vec<Entry> {
    let mut map: std::collections::BTreeMap<usize, Entry> = Default::default();
    for line in text.lines() {
        let Some((k, v)) = line.trim().split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        let num = |prefix: &str| k.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok());
        if let Some(n) = num("file") {
            map.entry(n).or_default().path = resolve(base, v);
        } else if let Some(n) = num("title") {
            let e = map.entry(n).or_default();
            (e.artist, e.title) = split_artist_title(v);
        } else if let Some(n) = num("length") {
            map.entry(n).or_default().duration = v.parse::<f64>().unwrap_or(0.0).max(0.0);
        }
    }
    map.into_values().filter(|e| !e.path.is_empty()).collect()
}

/// rekordbox playlist text: the header row names the columns (any order, any subset). Entries
/// without a Location keep their title and artist, for matching against the collection.
pub fn parse_txt(text: &str) -> Vec<Entry> {
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    let Some(header) = lines.next() else { return vec![] };
    let cols: Vec<String> = header.split('\t').map(|c| c.trim().to_ascii_lowercase()).collect();
    let find = |names: &[&str]| cols.iter().position(|c| names.contains(&c.as_str()));
    let (title, artist, album, genre, bpm, rating, time, key, date, loc) = (
        find(&["track title", "title"]),
        find(&["artist"]),
        find(&["album"]),
        find(&["genre"]),
        find(&["bpm"]),
        find(&["rating"]),
        find(&["time"]),
        find(&["key"]),
        find(&["date added"]),
        find(&["location"]),
    );
    if title.is_none() && loc.is_none() {
        return vec![];
    }
    let mut out = vec![];
    for l in lines {
        if l.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = l.split('\t').collect();
        let get = |i: Option<usize>| i.and_then(|i| f.get(i)).map(|s| s.trim().to_string()).unwrap_or_default();
        out.push(Entry {
            path: get(loc),
            title: get(title),
            artist: get(artist),
            album: get(album),
            genre: get(genre),
            bpm: get(bpm).parse().unwrap_or(0.0),
            rating: get(rating).chars().filter(|c| *c == '*' || *c == '★').count().min(5) as u8,
            duration: parse_time(&get(time)),
            key: get(key),
            date_added: get(date),
        });
    }
    out
}

/// Read any supported playlist file.
pub fn parse(format: Format, bytes: &[u8], base: &Path) -> Vec<Entry> {
    let text = decode_text(bytes);
    match format {
        Format::M3u8 => parse_m3u(&text, base),
        Format::Pls => parse_pls(&text, base),
        Format::Txt => parse_txt(&text),
    }
}

/// A file name safe on every file system for a playlist called `name`.
pub fn safe_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '?' | '*' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() {
        "Untitled Playlist".into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Entry> {
        vec![
            Entry { path: "/Music/A/One.mp3".into(), title: "One".into(), artist: "Ann".into(), album: "First".into(), genre: "House".into(), bpm: 126.0, rating: 3, duration: 321.0, key: "8A".into(), date_added: "2026-01-02".into() },
            Entry { path: "/Music/B/Zwei – ü.flac".into(), title: "Zwei – ü".into(), artist: String::new(), album: String::new(), genre: String::new(), bpm: 0.0, rating: 0, duration: 59.6, key: String::new(), date_added: String::new() },
        ]
    }

    #[test]
    fn m3u8_round_trip() {
        let text = write_m3u8(&sample());
        assert!(text.starts_with("#EXTM3U\n#EXTINF:321,Ann - One\n/Music/A/One.mp3\n"));
        let back = parse(Format::M3u8, text.as_bytes(), Path::new("/x"));
        assert_eq!(back.len(), 2);
        assert_eq!((back[0].path.as_str(), back[0].artist.as_str(), back[0].title.as_str(), back[0].duration), ("/Music/A/One.mp3", "Ann", "One", 321.0));
        assert_eq!((back[1].path.as_str(), back[1].title.as_str(), back[1].duration), ("/Music/B/Zwei – ü.flac", "Zwei – ü", 60.0));
    }

    #[test]
    fn m3u_relative_urls_and_latin1() {
        let text = "one.mp3\r\nsub/two.mp3\r\nfile:///Vol/My%20Music/three.mp3\r\n# note\r\n";
        let e = parse_m3u(text, Path::new("/lists"));
        assert_eq!(e.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["/lists/one.mp3", "/lists/sub/two.mp3", "/Vol/My Music/three.mp3"]);
        // 0xE9 alone is not UTF-8: read as Latin-1.
        assert_eq!(decode_text(&[b'/', b'a', 0xE9, b'.', b'm', b'p', b'3']), "/aé.mp3");
    }

    #[test]
    fn pls_round_trip() {
        let text = write_pls(&sample());
        assert!(text.contains("NumberOfEntries=2") && text.contains("File2=/Music/B/Zwei – ü.flac"));
        let back = parse(Format::Pls, text.as_bytes(), Path::new("/x"));
        assert_eq!(back.len(), 2);
        assert_eq!((back[0].path.as_str(), back[0].artist.as_str(), back[0].title.as_str(), back[0].duration), ("/Music/A/One.mp3", "Ann", "One", 321.0));
    }

    #[test]
    fn txt_round_trip_utf16() {
        let bytes = encode(Format::Txt, &sample());
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let text = decode_text(&bytes);
        assert!(text.starts_with("#\tTrack Title\tArtist\tAlbum\tGenre\tBPM\tRating\tTime\tKey\tDate Added\tLocation\r\n1\tOne\tAnn\tFirst\tHouse\t126.00\t***  \t05:21\t8A\t2026-01-02\t/Music/A/One.mp3\r\n"));
        let back = parse(Format::Txt, &bytes, Path::new("/x"));
        let mut want = sample();
        want[1].duration = 60.0;
        assert_eq!(back, want);
    }

    #[test]
    fn txt_from_rekordbox_without_location() {
        // rekordbox's own file: its visible columns only, in its order.
        let text = "#\tArtwork\tTrack Title\tArtist\tBPM\tTime\r\n1\t\tSong\tSomeone\t128.00\t06:10\r\n";
        let e = parse_txt(text);
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].title.as_str(), e[0].artist.as_str(), e[0].bpm, e[0].duration, e[0].path.as_str()), ("Song", "Someone", 128.0, 370.0, ""));
        assert!(parse_txt("just some text\nmore").is_empty());
    }

    #[test]
    fn formats_and_names() {
        assert_eq!(Format::of(Path::new("a.M3U")), Some(Format::M3u8));
        assert_eq!(Format::of(Path::new("a.pls")), Some(Format::Pls));
        assert_eq!(Format::of(Path::new("a.txt")), Some(Format::Txt));
        assert_eq!(Format::of(Path::new("a.xml")), None);
        assert_eq!(safe_file_name("A/B: C?."), "A_B_ C_");
        assert_eq!(safe_file_name(" . "), "Untitled Playlist");
        assert_eq!(percent_decode("a%20b%zz%2"), "a b%zz%2");
    }
}
