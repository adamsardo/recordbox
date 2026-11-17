//! Read-only library sources shown in the tree: the Music app's (iTunes) library, read from an
//! iTunes Library XML file or asked from the Music app, and a rekordbox xml file. Nothing here
//! ever writes to either.

use std::collections::HashMap;

/// A track of an outside library.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SrcTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub composer: String,
    pub grouping: String,
    pub comment: String,
    pub bpm: f64,
    pub duration: f64,
    pub year: i32,
    /// 0–5 stars.
    pub rating: u8,
    /// File path ("" for tracks without a local file, e.g. cloud-only).
    pub path: String,
}

/// A playlist or folder of an outside library; `tracks` index into `SourceLib::tracks`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SrcNode {
    pub name: String,
    pub folder: bool,
    pub children: Vec<SrcNode>,
    pub tracks: Vec<usize>,
}

#[derive(Default)]
pub struct SourceLib {
    pub tracks: Vec<SrcTrack>,
    pub nodes: Vec<SrcNode>,
    /// The parsed rekordbox xml (cues and grids), for the rekordbox xml source.
    pub xml: Option<crate::importer::XmlLibrary>,
    /// Where it was read from, for the tree's tooltip.
    pub origin: String,
}

impl SourceLib {
    /// The node at `key` ("0/2/1": child indexes from the top level); None for "" (All Tracks).
    pub fn node(&self, key: &str) -> Option<&SrcNode> {
        let mut list = &self.nodes;
        let mut cur = None;
        for part in key.split('/').filter(|p| !p.is_empty()) {
            let n = list.get(part.parse::<usize>().ok()?)?;
            list = &n.children;
            cur = Some(n);
        }
        cur
    }
    /// Track indexes shown for `key`: everything for "", a playlist's tracks, or a folder's
    /// tracks (its playlists' tracks, each once).
    pub fn tracks_of(&self, key: &str) -> Vec<usize> {
        fn collect(n: &SrcNode, out: &mut Vec<usize>, seen: &mut std::collections::HashSet<usize>) {
            for t in &n.tracks {
                if seen.insert(*t) {
                    out.push(*t);
                }
            }
            for c in &n.children {
                collect(c, out, seen);
            }
        }
        match self.node(key) {
            None => (0..self.tracks.len()).collect(),
            Some(n) if n.folder => {
                let mut out = vec![];
                collect(n, &mut out, &mut Default::default());
                out
            }
            Some(n) => n.tracks.clone(),
        }
    }
}

// ------------------------------------------------------------------------------ property lists
#[derive(Clone, Debug, PartialEq)]
pub enum Pl {
    Dict(Vec<(String, Pl)>),
    Array(Vec<Pl>),
    Str(String),
    Int(i64),
    Real(f64),
    Bool(bool),
}

impl Pl {
    pub fn get(&self, key: &str) -> Option<&Pl> {
        match self {
            Pl::Dict(v) => v.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self, key: &str) -> String {
        match self.get(key) {
            Some(Pl::Str(s)) => s.clone(),
            Some(Pl::Int(i)) => i.to_string(),
            _ => String::new(),
        }
    }
    pub fn int(&self, key: &str) -> i64 {
        match self.get(key) {
            Some(Pl::Int(i)) => *i,
            Some(Pl::Real(r)) => *r as i64,
            Some(Pl::Str(s)) => s.parse().unwrap_or(0),
            _ => 0,
        }
    }
    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Pl::Bool(true)))
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")).and_then(|h| u32::from_str_radix(h, 16).ok()).or_else(|| ent.strip_prefix('#').and_then(|d| d.parse::<u32>().ok())).and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

struct PlParser<'a> {
    s: &'a str,
    pos: usize,
    depth: usize,
}

impl<'a> PlParser<'a> {
    /// The next tag: (name, closing, self-closing), skipping text, comments and declarations.
    fn next_tag(&mut self) -> anyhow::Result<(&'a str, bool, bool)> {
        loop {
            let Some(i) = self.s[self.pos..].find('<') else { anyhow::bail!("the file ends inside an element (truncated?)") };
            let start = self.pos + i;
            let rest = &self.s[start..];
            if rest.starts_with("<!--") {
                let end = rest.find("-->").ok_or_else(|| anyhow::anyhow!("unterminated comment"))?;
                self.pos = start + end + 3;
                continue;
            }
            let end = rest.find('>').ok_or_else(|| anyhow::anyhow!("unterminated tag"))?;
            self.pos = start + end + 1;
            let inner = &rest[1..end];
            if inner.starts_with('?') || inner.starts_with('!') {
                continue;
            }
            let closing = inner.starts_with('/');
            let selfc = inner.ends_with('/');
            let name = inner.trim_start_matches('/').trim_end_matches('/').split_whitespace().next().unwrap_or("");
            return Ok((name, closing, selfc));
        }
    }
    /// Text up to the closing tag of `name` (the opening tag was just read).
    fn text(&mut self, name: &str) -> anyhow::Result<String> {
        let close = format!("</{name}>");
        let i = self.s[self.pos..].find(&close).ok_or_else(|| anyhow::anyhow!("<{name}> is not closed"))?;
        let t = &self.s[self.pos..self.pos + i];
        self.pos += i + close.len();
        Ok(unescape(t))
    }
    fn value(&mut self, name: &str, selfc: bool) -> anyhow::Result<Pl> {
        self.depth += 1;
        if self.depth > 64 {
            anyhow::bail!("nested too deeply");
        }
        let v = match name {
            "dict" => {
                let mut items = vec![];
                while !selfc {
                    let (n, closing, sc) = self.next_tag()?;
                    if closing {
                        break;
                    }
                    if n != "key" {
                        anyhow::bail!("expected <key> in a dictionary, found <{n}>");
                    }
                    let key = if sc { String::new() } else { self.text("key")? };
                    let (vn, vclosing, vsc) = self.next_tag()?;
                    if vclosing {
                        anyhow::bail!("the key \"{key}\" has no value");
                    }
                    items.push((key, self.value(vn, vsc)?));
                }
                Pl::Dict(items)
            }
            "array" => {
                let mut items = vec![];
                while !selfc {
                    let (n, closing, sc) = self.next_tag()?;
                    if closing {
                        break;
                    }
                    items.push(self.value(n, sc)?);
                }
                Pl::Array(items)
            }
            "true" => Pl::Bool(true),
            "false" => Pl::Bool(false),
            "integer" => Pl::Int(if selfc { 0 } else { self.text(name)?.trim().parse().unwrap_or(0) }),
            "real" => Pl::Real(if selfc { 0.0 } else { self.text(name)?.trim().parse().unwrap_or(0.0) }),
            "string" | "date" | "data" => Pl::Str(if selfc { String::new() } else { self.text(name)? }),
            other => anyhow::bail!("unexpected element <{other}>"),
        };
        self.depth -= 1;
        Ok(v)
    }
}

/// Parse an XML property list (the format of "iTunes Music Library.xml" / "Library.xml").
pub fn parse_plist(text: &str) -> anyhow::Result<Pl> {
    let mut p = PlParser { s: text.trim_start_matches('\u{feff}'), pos: 0, depth: 0 };
    let (n, closing, selfc) = p.next_tag()?;
    if n != "plist" || closing {
        anyhow::bail!("not a property list");
    }
    if selfc {
        anyhow::bail!("empty property list");
    }
    let (n, closing, selfc) = p.next_tag()?;
    if closing {
        anyhow::bail!("empty property list");
    }
    p.value(n, selfc)
}

/// A `file://` location as a path.
pub fn location_path(loc: &str) -> String {
    let Some(rest) = loc.strip_prefix("file://") else { return if loc.starts_with('/') { loc.to_string() } else { String::new() } };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    crate::playlist_files::percent_decode(rest)
}

/// The library an iTunes Library XML describes: tracks with a local file, and the user's
/// playlists nested in their folders. The master list and the built-in category lists
/// (Music, Films, Downloaded…) are left out, as rekordbox does.
pub fn parse_itunes_xml(text: &str) -> anyhow::Result<SourceLib> {
    let root = parse_plist(text)?;
    let Some(Pl::Dict(tracks)) = root.get("Tracks") else { anyhow::bail!("no Tracks in this file") };
    let mut lib = SourceLib::default();
    let mut by_id: HashMap<i64, usize> = HashMap::new();
    for (_, t) in tracks {
        let path = location_path(&t.str("Location"));
        if path.is_empty() || t.str("Track Type") == "URL" {
            continue;
        }
        let kind = t.str("Kind").to_ascii_lowercase();
        if t.flag("Has Video") || t.flag("Podcast") || kind.contains("video") || kind.contains("book") {
            continue;
        }
        by_id.insert(t.int("Track ID"), lib.tracks.len());
        lib.tracks.push(SrcTrack {
            title: t.str("Name"),
            artist: t.str("Artist"),
            album: t.str("Album"),
            genre: t.str("Genre"),
            composer: t.str("Composer"),
            grouping: t.str("Grouping"),
            comment: t.str("Comments"),
            bpm: t.int("BPM") as f64,
            duration: t.int("Total Time") as f64 / 1000.0,
            year: t.int("Year") as i32,
            rating: (t.int("Rating") / 20).clamp(0, 5) as u8,
            path,
        });
    }
    // Playlists: flat in the file, each naming its parent folder by persistent ID.
    struct Raw {
        id: String,
        parent: String,
        node: SrcNode,
    }
    let mut raws: Vec<Raw> = vec![];
    if let Some(Pl::Array(pls)) = root.get("Playlists") {
        for p in pls {
            if p.flag("Master") || p.get("Distinguished Kind").is_some() {
                continue;
            }
            // "Visible" only ever appears as false (the hidden master list).
            if matches!(p.get("Visible"), Some(Pl::Bool(false))) {
                continue;
            }
            let tracks: Vec<usize> = match p.get("Playlist Items") {
                Some(Pl::Array(items)) => items.iter().filter_map(|i| by_id.get(&i.int("Track ID")).copied()).collect(),
                _ => vec![],
            };
            let folder = p.flag("Folder");
            raws.push(Raw { id: p.str("Playlist Persistent ID"), parent: p.str("Parent Persistent ID"), node: SrcNode { name: p.str("Name"), folder, children: vec![], tracks: if folder { vec![] } else { tracks } } });
        }
    }
    fn build(parent: &str, raws: &[Raw], depth: usize) -> Vec<SrcNode> {
        if depth > 32 {
            return vec![];
        }
        raws.iter()
            .filter(|r| r.parent == parent)
            .map(|r| {
                let mut n = r.node.clone();
                if n.folder && !r.id.is_empty() {
                    n.children = build(&r.id, raws, depth + 1);
                }
                n
            })
            .collect()
    }
    let known: std::collections::HashSet<&str> = raws.iter().map(|r| r.id.as_str()).collect();
    // A playlist whose parent is not in the file sits at the top level.
    let mut top = build("", &raws, 0);
    for r in raws.iter().filter(|r| !r.parent.is_empty() && !known.contains(r.parent.as_str())) {
        let mut n = r.node.clone();
        if n.folder {
            n.children = build(&r.id, &raws, 1);
        }
        top.push(n);
    }
    lib.nodes = top;
    Ok(lib)
}

/// JavaScript for Automation that lists the Music app's file tracks and user playlists as
/// JSON. It only reads properties.
pub const MUSIC_JXA: &str = r#"
function run() {
  const m = Application('Music');
  const lib = m.libraryPlaylists[0];
  const ts = lib.fileTracks;
  const ids = ts.persistentID();
  const names = ts.name(), artists = ts.artist(), albums = ts.album(), genres = ts.genre();
  const durs = ts.duration(), bpms = ts.bpm(), years = ts.year(), ratings = ts.rating();
  const comps = ts.composer(), groups = ts.grouping(), comments = ts.comment();
  const locs = ts.location();
  const tracks = [];
  for (let i = 0; i < ids.length; i++) {
    tracks.push({id: ids[i], name: names[i], artist: artists[i], album: albums[i], genre: genres[i], duration: durs[i], bpm: bpms[i], year: years[i], rating: ratings[i], composer: comps[i], grouping: groups[i], comment: comments[i], location: locs[i] ? locs[i].toString() : ''});
  }
  const playlists = [];
  const ups = m.userPlaylists;
  const pn = ups.name(), pid = ups.persistentID();
  for (let i = 0; i < pn.length; i++) {
    let kind = '';
    try { kind = String(ups[i].specialKind()); } catch (e) {}
    if (kind && kind !== 'none') continue;
    let parent = '';
    try { parent = ups[i].parent().persistentID(); } catch (e) {}
    let t = [];
    try { t = ups[i].fileTracks.persistentID(); } catch (e) {}
    playlists.push({name: pn[i], id: pid[i], parent: parent, tracks: t});
  }
  const fps = m.folderPlaylists;
  const fn = fps.name(), fid = fps.persistentID();
  for (let i = 0; i < fn.length; i++) {
    let parent = '';
    try { parent = fps[i].parent().persistentID(); } catch (e) {}
    playlists.push({name: fn[i], id: fid[i], parent: parent, folder: true, tracks: []});
  }
  return JSON.stringify({tracks: tracks, playlists: playlists});
}
"#;

/// The library described by the JSON `MUSIC_JXA` prints.
pub fn parse_music_json(text: &str) -> anyhow::Result<SourceLib> {
    let v: serde_json::Value = serde_json::from_str(text.trim())?;
    let s = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let f = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let mut lib = SourceLib::default();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for t in v.get("tracks").and_then(|t| t.as_array()).ok_or_else(|| anyhow::anyhow!("no tracks in the answer"))? {
        let path = location_path(&s(t, "location"));
        if path.is_empty() {
            continue;
        }
        by_id.insert(s(t, "id"), lib.tracks.len());
        lib.tracks.push(SrcTrack { title: s(t, "name"), artist: s(t, "artist"), album: s(t, "album"), genre: s(t, "genre"), composer: s(t, "composer"), grouping: s(t, "grouping"), comment: s(t, "comment"), bpm: f(t, "bpm"), duration: f(t, "duration"), year: f(t, "year") as i32, rating: (f(t, "rating") / 20.0).clamp(0.0, 5.0) as u8, path });
    }
    let pls: Vec<&serde_json::Value> = v.get("playlists").and_then(|p| p.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
    let ids: std::collections::HashSet<String> = pls.iter().map(|p| s(p, "id")).collect();
    fn build(parent: &str, top: bool, pls: &[&serde_json::Value], ids: &std::collections::HashSet<String>, by_id: &HashMap<String, usize>, depth: usize) -> Vec<SrcNode> {
        if depth > 32 {
            return vec![];
        }
        let s = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        pls.iter()
            .filter(|p| {
                let par = s(p, "parent");
                if top { par.is_empty() || !ids.contains(&par) } else { par == parent }
            })
            .map(|p| {
                let folder = p.get("folder").and_then(|x| x.as_bool()).unwrap_or(false);
                let tracks = p.get("tracks").and_then(|t| t.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().and_then(|x| by_id.get(x).copied())).collect()).unwrap_or_default();
                SrcNode { name: s(p, "name"), folder, children: if folder { build(&s(p, "id"), false, pls, ids, by_id, depth + 1) } else { vec![] }, tracks: if folder { vec![] } else { tracks } }
            })
            .collect()
    }
    lib.nodes = build("", true, &pls, &ids, &by_id, 0);
    Ok(lib)
}

/// Ask the Music app for its library (read-only), giving up after `timeout`.
pub fn ask_music_app(timeout: std::time::Duration) -> anyhow::Result<SourceLib> {
    use std::io::Read;
    let mut child = std::process::Command::new("/usr/bin/osascript").args(["-l", "JavaScript", "-e", MUSIC_JXA]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()?;
    let mut out = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no output"))?;
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        let _ = tx.send(s);
    });
    match rx.recv_timeout(timeout) {
        Ok(text) => {
            let status = child.wait()?;
            if !status.success() {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                anyhow::bail!("the Music app did not answer ({})", err.trim());
            }
            let mut lib = parse_music_json(&text)?;
            lib.origin = "Music app".into();
            Ok(lib)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("the Music app did not answer in time")
        }
    }
}

/// Where an iTunes Library XML may be: the chosen file first, then the usual places.
pub fn itunes_xml_candidates(chosen: Option<&str>) -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = chosen.filter(|c| !c.trim().is_empty()).map(std::path::PathBuf::from).into_iter().collect();
    if let Some(music) = dirs::audio_dir() {
        v.push(music.join("Music").join("Library.xml"));
        v.push(music.join("iTunes").join("iTunes Music Library.xml"));
        v.push(music.join("iTunes").join("iTunes Library.xml"));
    }
    v
}

/// Load the iTunes library: the first XML file that exists, else (when allowed) the Music app.
pub fn load_itunes(chosen: Option<&str>, ask_app: bool) -> Result<SourceLib, String> {
    for p in itunes_xml_candidates(chosen) {
        if p.is_file() {
            let text = std::fs::read_to_string(&p).map_err(|e| format!("Could not read {}: {e}", p.display()))?;
            let mut lib = parse_itunes_xml(&text).map_err(|_| "The iTunes xml file specified is unable to be read properly.".to_string())?;
            lib.origin = p.to_string_lossy().to_string();
            return Ok(lib);
        }
    }
    if !ask_app {
        return Err("iTunes library not found. Please ensure you have correctly installed iTunes on your computer.".into());
    }
    ask_music_app(std::time::Duration::from_secs(90)).map_err(|e| format!("iTunes library not found. ({e})"))
}

/// A rekordbox xml file as a browsable source (All Tracks and its playlists).
pub fn load_rekordbox_xml(path: &std::path::Path) -> Result<SourceLib, String> {
    if std::fs::metadata(path).map(|m| m.len() > 1 << 30).unwrap_or(false) {
        return Err("This file is larger than 1 GB, too large to be a collection file.".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read the file: {e}"))?;
    let xml = crate::importer::parse_rekordbox_xml(&text).map_err(|e| format!("This is not a readable collection file: {e}"))?;
    Ok(from_rekordbox_xml(xml, &path.to_string_lossy()))
}

pub fn from_rekordbox_xml(xml: crate::importer::XmlLibrary, origin: &str) -> SourceLib {
    let mut lib = SourceLib { origin: origin.to_string(), ..Default::default() };
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut by_path: HashMap<String, usize> = HashMap::new();
    for (i, xt) in xml.tracks.iter().enumerate() {
        let t = &xt.track;
        by_id.insert(xt.xml_id.clone(), i);
        by_path.insert(t.path.clone(), i);
        lib.tracks.push(SrcTrack { title: t.display_title(), artist: t.artist.clone(), album: t.album.clone(), genre: t.genre.clone(), composer: t.composer.clone(), grouping: t.grouping.clone(), comment: t.comment.clone(), bpm: t.bpm, duration: t.duration, year: t.year, rating: t.rating, path: t.path.clone() });
    }
    fn conv(n: &crate::importer::XmlNode, by_id: &HashMap<String, usize>, by_path: &HashMap<String, usize>) -> SrcNode {
        SrcNode {
            name: n.name.clone(),
            folder: n.folder,
            children: n.children.iter().map(|c| conv(c, by_id, by_path)).collect(),
            tracks: n.track_keys.iter().filter_map(|k| if n.by_location { by_path.get(k) } else { by_id.get(k) }.copied()).collect(),
        }
    }
    lib.nodes = xml.playlists.iter().map(|n| conv(n, &by_id, &by_path)).collect();
    lib.xml = Some(xml);
    lib
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple Computer//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Major Version</key><integer>1</integer>
	<key>Show Content Ratings</key><true/>
	<key>Tracks</key>
	<dict>
		<key>101</key>
		<dict>
			<key>Track ID</key><integer>101</integer>
			<key>Name</key><string>Tom &amp; Jerry &#8211; Dub</string>
			<key>Artist</key><string>Ann</string>
			<key>Album</key><string>First</string>
			<key>Genre</key><string>House</string>
			<key>Total Time</key><integer>321500</integer>
			<key>BPM</key><integer>126</integer>
			<key>Year</key><integer>2021</integer>
			<key>Rating</key><integer>80</integer>
			<key>Track Type</key><string>File</string>
			<key>Location</key><string>file:///Users/x/Music/Tom%20%26%20Jerry.mp3</string>
		</dict>
		<key>102</key>
		<dict>
			<key>Track ID</key><integer>102</integer>
			<key>Name</key><string>Second</string>
			<key>Total Time</key><integer>60000</integer>
			<key>Location</key><string>file://localhost/Volumes/USB/two.m4a</string>
		</dict>
		<key>103</key>
		<dict>
			<key>Track ID</key><integer>103</integer>
			<key>Name</key><string>Cloud only</string>
			<key>Track Type</key><string>Remote</string>
		</dict>
		<key>104</key>
		<dict>
			<key>Track ID</key><integer>104</integer>
			<key>Name</key><string>A film</string>
			<key>Has Video</key><true/>
			<key>Location</key><string>file:///Users/x/Movies/f.m4v</string>
		</dict>
	</dict>
	<key>Playlists</key>
	<array>
		<dict>
			<key>Master</key><true/>
			<key>Name</key><string>Library</string>
			<key>Playlist Persistent ID</key><string>AAAA</string>
			<key>Visible</key><false/>
			<key>Playlist Items</key>
			<array><dict><key>Track ID</key><integer>101</integer></dict></array>
		</dict>
		<dict>
			<key>Name</key><string>Music</string>
			<key>Distinguished Kind</key><integer>4</integer>
			<key>Playlist Persistent ID</key><string>BBBB</string>
		</dict>
		<dict>
			<key>Name</key><string>Sets</string>
			<key>Folder</key><true/>
			<key>Playlist Persistent ID</key><string>F001</string>
		</dict>
		<dict>
			<key>Name</key><string>Friday</string>
			<key>Playlist Persistent ID</key><string>P001</string>
			<key>Parent Persistent ID</key><string>F001</string>
			<key>Playlist Items</key>
			<array>
				<dict><key>Track ID</key><integer>102</integer></dict>
				<dict><key>Track ID</key><integer>103</integer></dict>
				<dict><key>Track ID</key><integer>101</integer></dict>
			</array>
		</dict>
		<dict>
			<key>Name</key><string>Empty</string>
			<key>Playlist Persistent ID</key><string>P002</string>
			<key>Playlist Items</key><array/>
		</dict>
	</array>
</dict>
</plist>"#;

    #[test]
    fn reads_itunes_library_xml() {
        let lib = parse_itunes_xml(XML).unwrap();
        // The cloud-only track and the film are left out.
        assert_eq!(lib.tracks.len(), 2);
        let t = &lib.tracks[0];
        assert_eq!((t.title.as_str(), t.artist.as_str(), t.path.as_str()), ("Tom & Jerry – Dub", "Ann", "/Users/x/Music/Tom & Jerry.mp3"));
        assert_eq!((t.bpm, t.duration, t.year, t.rating), (126.0, 321.5, 2021, 4));
        assert_eq!(lib.tracks[1].path, "/Volumes/USB/two.m4a");
        // The master and the built-in "Music" list are left out; Friday sits in its folder.
        assert_eq!(lib.nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["Sets", "Empty"]);
        assert!(lib.nodes[0].folder);
        assert_eq!(lib.nodes[0].children[0].name, "Friday");
        assert_eq!(lib.nodes[0].children[0].tracks, [1, 0]);
        assert_eq!(lib.node("0/0").unwrap().name, "Friday");
        assert_eq!(lib.tracks_of("0"), [1, 0]);
        assert_eq!(lib.tracks_of(""), [0, 1]);
        assert!(lib.tracks_of("1").is_empty());
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse_itunes_xml("<html></html>").is_err());
        assert!(parse_itunes_xml("<plist><dict><key>Tracks</key><dict>").is_err());
        assert!(parse_itunes_xml("<plist version=\"1.0\"><dict><key>X</key><integer>1</integer></dict></plist>").is_err());
    }

    #[test]
    fn reads_music_app_answer() {
        let json = r#"{"tracks":[{"id":"T1","name":"One","artist":"Ann","album":"","genre":"House","duration":200.5,"bpm":124,"year":2020,"rating":60,"composer":"","grouping":"","comment":"","location":"/Users/x/Music/one.mp3"},{"id":"T2","name":"No file","location":""}],
            "playlists":[{"name":"Inner","id":"P1","parent":"F1","tracks":["T1","T2"]},{"name":"Folder","id":"F1","parent":"","folder":true,"tracks":[]},{"name":"Orphan","id":"P2","parent":"GONE","tracks":[]}]}"#;
        let lib = parse_music_json(json).unwrap();
        assert_eq!(lib.tracks.len(), 1);
        assert_eq!((lib.tracks[0].title.as_str(), lib.tracks[0].rating, lib.tracks[0].duration), ("One", 3, 200.5));
        assert_eq!(lib.nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["Folder", "Orphan"]);
        assert_eq!(lib.nodes[0].children[0].tracks, [0]);
        assert!(parse_music_json("not json").is_err());
    }
}
