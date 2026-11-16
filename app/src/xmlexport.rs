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

pub fn build(app: &App) -> String {
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n");
    x.push_str("  <PRODUCT Name=\"PrepDeck\" Version=\"1.0\" Company=\"\"/>\n");
    x.push_str(&format!("  <COLLECTION Entries=\"{}\">\n", app.tracks.len()));
    for t in &app.tracks {
        let rating = [0, 51, 102, 153, 204, 255][t.rating.min(5) as usize];
        x.push_str(&format!(
            "    <TRACK TrackID=\"{}\" Name=\"{}\" Artist=\"{}\" Composer=\"{}\" Album=\"{}\" Grouping=\"{}\" Genre=\"{}\" Kind=\"{} File\" Size=\"{}\" TotalTime=\"{}\" DiscNumber=\"{}\" TrackNumber=\"{}\" Year=\"{}\" AverageBpm=\"{:.2}\" DateAdded=\"{}\" BitRate=\"{}\" SampleRate=\"{}\" Comments=\"{}\" PlayCount=\"{}\" Rating=\"{}\" Location=\"{}\" Remixer=\"{}\" Tonality=\"{}\" Label=\"{}\" Mix=\"{}\"{}>\n",
            t.id,
            esc(&t.display_title()),
            esc(&t.artist),
            esc(&t.composer),
            esc(&t.album),
            esc(&t.grouping),
            esc(&t.genre),
            esc(&t.file_type.to_ascii_uppercase()),
            t.file_size,
            t.duration.round() as u64,
            t.disc_no,
            t.track_no,
            t.year,
            t.bpm,
            esc(&t.date_added),
            t.bitrate,
            t.sample_rate,
            esc(&t.comment),
            t.play_count,
            rating,
            location(&t.path),
            esc(&t.remixer),
            esc(&key_name(t.key, false)),
            esc(&t.label),
            esc(&t.mix_name),
            // Track colour, as rekordbox writes it (0xRRGGBB of its eight track colours).
            match t.color {
                1 => " Colour=\"0xFF007F\"",
                2 => " Colour=\"0xFF0000\"",
                3 => " Colour=\"0xFFA500\"",
                4 => " Colour=\"0xFFFF00\"",
                5 => " Colour=\"0x00FF00\"",
                6 => " Colour=\"0x25FDE9\"",
                7 => " Colour=\"0x0000FF\"",
                8 => " Colour=\"0x660099\"",
                _ => "",
            }
        ));
        if let Some(g) = app.db.grid(t.id) {
            for a in &g.anchors {
                x.push_str(&format!("      <TEMPO Inizio=\"{:.3}\" Bpm=\"{:.2}\" Metro=\"4/4\" Battito=\"{}\"/>\n", a.time, a.bpm, a.beat));
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
