//! Export the collection in the common DJ collection XML format (tracks, grids, cues,
//! playlists) so prepared data can move to other software.

use crate::app::App;
use crate::model::*;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn location(path: &str) -> String {
    let mut out = String::from("file://localhost");
    for b in path.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "/-_.~".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Marks an attribute the imported XML did not have (written back as absent).
pub const ABSENT: &str = "\u{0}absent";

/// TRACK attributes as PrepDeck writes them, unescaped (TrackID aside). Colour is empty when
/// the track has none (then not written).
pub fn track_attrs(t: &Track) -> Vec<(&'static str, String)> {
    let rating = [0, 51, 102, 153, 204, 255][t.rating.min(5) as usize];
    let colour = match t.color {
        1 => "0xFF007F",
        2 => "0xFF0000",
        3 => "0xFFA500",
        4 => "0xFFFF00",
        5 => "0x00FF00",
        6 => "0x25FDE9",
        7 => "0x0000FF",
        8 => "0x660099",
        _ => "",
    };
    vec![
        ("Name", t.display_title()),
        ("Artist", t.artist.clone()),
        ("Composer", t.composer.clone()),
        ("Album", t.album.clone()),
        ("Grouping", t.grouping.clone()),
        ("Genre", t.genre.clone()),
        ("Kind", format!("{} File", t.file_type.to_ascii_uppercase())),
        ("Size", t.file_size.to_string()),
        ("TotalTime", (t.duration.round() as u64).to_string()),
        ("DiscNumber", t.disc_no.to_string()),
        ("TrackNumber", t.track_no.to_string()),
        ("Year", t.year.to_string()),
        ("AverageBpm", format!("{:.2}", t.bpm)),
        ("DateAdded", t.date_added.clone()),
        ("BitRate", t.bitrate.to_string()),
        ("SampleRate", t.sample_rate.to_string()),
        ("Comments", t.comment.clone()),
        ("PlayCount", t.play_count.to_string()),
        ("Rating", rating.to_string()),
        ("Location", location(&t.path)),
        ("Remixer", t.remixer.clone()),
        ("Tonality", key_name(t.key, false)),
        ("Label", t.label.clone()),
        ("Mix", t.mix_name.clone()),
        // Track colour, as rekordbox writes it (0xRRGGBB of its eight track colours).
        ("Colour", colour.to_string()),
    ]
}

/// The opening TRACK element. A field whose value is unchanged since an XML import is written
/// with the imported text (precision, key spelling, URL encoding, absent attributes); edited
/// fields are written from PrepDeck's value; unknown attributes follow.
pub fn track_line(t: &Track, extra: &crate::importer::XmlExtra) -> String {
    let mut line = format!("    <TRACK TrackID=\"{}\"", t.id);
    for (k, v) in track_attrs(t) {
        let v = extra.orig.iter().find(|o| o.0 == k && o.2 == v).map(|o| o.1.clone()).unwrap_or(v);
        if v == ABSENT || (k == "Colour" && v.is_empty()) {
            continue;
        }
        line.push_str(&format!(" {k}=\"{}\"", esc(&v)));
    }
    line.push_str(&extra_xml(extra, "").0);
    line.push_str(">\n");
    line
}

pub fn build(app: &App) -> String {
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n");
    x.push_str("  <PRODUCT Name=\"PrepDeck\" Version=\"1.0\" Company=\"\"/>\n");
    x.push_str(&format!("  <COLLECTION Entries=\"{}\">\n", app.tracks.len()));
    for t in &app.tracks {
        // Fields PrepDeck doesn't model, and original text of unchanged fields, as imported.
        let extra = app.db.xml_extra(t.id);
        x.push_str(&track_line(t, &extra));
        let (_, extra_nodes) = extra_xml(&extra, "      ");
        if let Some(g) = app.db.grid(t.id) {
            // The per-beat records PrepDeck writes to PQTZ, filtered as rekordbox does.
            let full = if t.raw_samples > 0 && t.sample_rate > 0 { t.raw_samples as f64 / t.sample_rate as f64 } else { t.duration };
            let recs: Vec<(u16, u16, u32)> = g.beats(full.max(t.duration)).iter().map(|b| (b.beat as u16, (b.bpm * 100.0).round() as u16, (b.time * 1000.0).round() as u32)).collect();
            for i in tempo_keep(&recs) {
                let (beat, bpm, ms) = recs[i];
                x.push_str(&format!("      <TEMPO Inizio=\"{:.3}\" Bpm=\"{:.2}\" Metro=\"4/4\" Battito=\"{}\"/>\n", ms as f64 * 0.001, bpm as f64 * 0.01, beat));
            }
        }
        for c in app.db.cues(t.id).unwrap_or_default() {
            // rekordbox's XML writer emits numbered cues for slots A–H only (rekordbox-rea XML-003).
            if c.kind == CueKind::Hot && !(0..8).contains(&c.slot) {
                continue;
            }
            let num = if c.kind == CueKind::Hot { c.slot } else { -1 };
            let ty = if c.is_loop() { 4 } else { 0 };
            let end = if c.is_loop() { format!(" End=\"{:.3}\"", c.end.unwrap()) } else { String::new() };
            let rgb = if c.kind == CueKind::Hot {
                let [r, g, b] = hotcue_rgb(c.color);
                format!(" Red=\"{r}\" Green=\"{g}\" Blue=\"{b}\"")
            } else {
                String::new()
            };
            x.push_str(&format!("      <POSITION_MARK Name=\"{}\" Type=\"{}\" Start=\"{:.3}\"{} Num=\"{}\"{}/>\n", esc(&c.comment), ty, c.start, end, num, rgb));
        }
        x.push_str(&extra_nodes);
        x.push_str("    </TRACK>\n");
    }
    x.push_str("  </COLLECTION>\n  <PLAYLISTS>\n");
    fn node(app: &App, parent: i64, depth: usize, x: &mut String) {
        for n in app.nodes.iter().filter(|n| n.parent == parent) {
            let pad = "  ".repeat(depth + 3);
            match n.kind {
                NodeKind::Folder => {
                    let count = app.nodes.iter().filter(|c| c.parent == n.id).count();
                    x.push_str(&format!("{pad}<NODE Type=\"0\" Name=\"{}\" Count=\"{}\">\n", esc(&n.name), count));
                    node(app, n.id, depth + 1, x);
                    x.push_str(&format!("{pad}</NODE>\n"));
                }
                _ => {
                    let tracks = if n.kind == NodeKind::Smart { n.smart.as_ref().map(|r| app.smart_tracks(r)).unwrap_or_default() } else { app.db.playlist_tracks(n.id).unwrap_or_default() };
                    x.push_str(&format!("{pad}<NODE Name=\"{}\" Type=\"1\" KeyType=\"0\" Entries=\"{}\">\n", esc(&n.name), tracks.len()));
                    for t in tracks {
                        x.push_str(&format!("{pad}  <TRACK Key=\"{t}\"/>\n"));
                    }
                    x.push_str(&format!("{pad}</NODE>\n"));
                }
            }
        }
    }
    let top = app.nodes.iter().filter(|n| n.parent == 0).count();
    x.push_str(&format!("    <NODE Type=\"0\" Name=\"ROOT\" Count=\"{top}\">\n"));
    node(app, 0, 0, &mut x);
    x.push_str("    </NODE>\n  </PLAYLISTS>\n</DJ_PLAYLISTS>\n");
    x
}

/// Which beat records become TEMPO nodes, as rekordbox's XML export chooses them with its
/// default `bridgeExportBeatGrid` = 1 (traced in rekordbox-rea reports/native-xml-trace.md):
/// a beat is kept when its BPM changes or its time (ms) matches neither the running
/// prediction nor the prediction from the previous beat, each truncated with and without
/// +0.5. Records are (beat in bar, BPM × 100, time in ms), as in PQTZ.
pub fn tempo_keep(records: &[(u16, u16, u32)]) -> Vec<usize> {
    let trunc = |x: f64| x as u32;
    let (mut prev_bpm, mut expected, mut from_prev) = (0u16, 0.0f64, 0.0f64);
    let mut out = vec![];
    for (i, &(_, bpm, ms)) in records.iter().enumerate() {
        let hit_expected = ms == trunc(expected) || ms == trunc(expected + 0.5);
        let hit_prev = ms == trunc(from_prev) || ms == trunc(from_prev + 0.5);
        if bpm != prev_bpm || !(hit_expected || hit_prev) {
            out.push(i);
            if !hit_expected {
                expected = ms as f64;
            }
            prev_bpm = bpm;
        }
        let interval = if bpm > 0 { 6_000_000.0 / bpm as f64 } else { 0.0 };
        expected += interval;
        from_prev = ms as f64 + interval;
    }
    out
}

#[cfg(test)]
mod tempo_tests {
    use super::tempo_keep;

    #[test]
    fn constant_grid_keeps_first_beat_only() {
        // 128 BPM from 0.120 s, times rounded to ms as in PQTZ.
        let recs: Vec<(u16, u16, u32)> = (0..64).map(|k| ((k % 4 + 1) as u16, 12800, (120.0 + k as f64 * 468.75).round() as u32)).collect();
        assert_eq!(tempo_keep(&recs), vec![0]);
        // A BPM change and a timing jump each add a node.
        let mut v = recs.clone();
        for r in v.iter_mut().skip(32) {
            r.2 += 7;
        }
        assert_eq!(tempo_keep(&v), vec![0, 32]);
    }

    /// rekordbox's own XML export against its own PQTZ for the same tracks. Run with
    /// PREPDECK_XML and PREPDECK_ANLZ_PAIRS set, --ignored.
    #[test]
    #[ignore]
    fn matches_rekordbox_xml_tempo_nodes() {
        let (Ok(xml), Ok(pairs)) = (std::env::var("PREPDECK_XML"), std::env::var("PREPDECK_ANLZ_PAIRS")) else { return };
        let text = std::fs::read_to_string(xml).unwrap();
        // Location -> TEMPO nodes as written.
        use quick_xml::events::Event;
        let mut r = quick_xml::Reader::from_str(&text);
        let mut by_loc: std::collections::HashMap<String, Vec<(String, String, String)>> = Default::default();
        let mut cur: Option<String> = None;
        loop {
            match r.read_event().unwrap() {
                Event::Start(e) | Event::Empty(e) => {
                    let a: std::collections::HashMap<String, String> = e.attributes().flatten().map(|a| (String::from_utf8_lossy(a.key.as_ref()).to_string(), a.unescape_value().unwrap().to_string())).collect();
                    match e.name().as_ref() {
                        b"TRACK" if a.contains_key("Location") => {
                            let loc = crate::importer::decode_location(&a["Location"]);
                            by_loc.entry(loc.clone()).or_default();
                            cur = Some(loc);
                        }
                        b"TEMPO" => {
                            if let Some(c) = &cur {
                                by_loc.get_mut(c).unwrap().push((a["Inizio"].clone(), a["Bpm"].clone(), a["Battito"].clone()));
                            }
                        }
                        _ => {}
                    }
                }
                Event::End(e) if e.name().as_ref() == b"TRACK" => cur = None,
                Event::Eof => break,
                _ => {}
            }
        }
        let (mut same, mut total, mut nodes) = (0, 0, 0);
        for line in std::fs::read_to_string(pairs).unwrap().lines() {
            let Some((path, base)) = line.split_once('\t') else { continue };
            let Some(want) = by_loc.get(path) else { continue };
            let b = std::fs::read(format!("{base}.DAT")).unwrap();
            let be32 = |o: usize| u32::from_be_bytes(b[o..o + 4].try_into().unwrap());
            let mut i = be32(4) as usize;
            let mut recs = vec![];
            while i + 12 <= b.len() {
                if &b[i..i + 4] == b"PQTZ" {
                    for k in 0..be32(i + 20) as usize {
                        let o = i + 24 + k * 8;
                        recs.push((u16::from_be_bytes([b[o], b[o + 1]]), u16::from_be_bytes([b[o + 2], b[o + 3]]), be32(o + 4)));
                    }
                }
                i += be32(i + 8) as usize;
            }
            let got: Vec<(String, String, String)> = tempo_keep(&recs).into_iter().map(|k| (format!("{:.3}", recs[k].2 as f64 * 0.001), format!("{:.2}", recs[k].1 as f64 * 0.01), recs[k].0.to_string())).collect();
            total += 1;
            nodes += want.len();
            if &got == want {
                same += 1;
            } else {
                eprintln!("differs: {path}\n  rekordbox {want:?}\n  ours      {got:?}");
            }
        }
        eprintln!("TEMPO nodes identical on {same} / {total} tracks ({nodes} nodes in rekordbox's XML)");
        assert_eq!(same, total);
    }
}

/// Unknown TRACK attributes (" Key=\"value\"…") and child elements (one line each, with
/// `pad`), written exactly as imported.
pub fn extra_xml(e: &crate::importer::XmlExtra, pad: &str) -> (String, String) {
    let attrs: String = e.attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect();
    let nodes: String = e
        .nodes
        .iter()
        .map(|(tag, a)| format!("{pad}<{tag}{}/>\n", a.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect::<String>()))
        .collect();
    (attrs, nodes)
}
