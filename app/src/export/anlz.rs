//! Writer for the players' analysis files (ANLZ0000.DAT / .EXT / .2EX), big-endian tagged
//! sections inside a PMAI container. Section layouts are listed in MECHANICS.md.

use crate::model::{Beat, Cue, CueKind, Waveform};

fn section(tag: &[u8; 4], header_extra: &[u8], body: &[u8]) -> Vec<u8> {
    let hl = 12 + header_extra.len();
    let mut v = Vec::with_capacity(hl + body.len());
    v.extend_from_slice(tag);
    v.extend_from_slice(&(hl as u32).to_be_bytes());
    v.extend_from_slice(&((hl + body.len()) as u32).to_be_bytes());
    v.extend_from_slice(header_extra);
    v.extend_from_slice(body);
    v
}

fn file(sections: Vec<Vec<u8>>) -> Vec<u8> {
    let total: usize = 28 + sections.iter().map(|s| s.len()).sum::<usize>();
    let mut v = Vec::with_capacity(total);
    v.extend_from_slice(b"PMAI");
    v.extend_from_slice(&28u32.to_be_bytes());
    v.extend_from_slice(&(total as u32).to_be_bytes());
    v.extend_from_slice(&[0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for s in sections {
        v.extend_from_slice(&s);
    }
    v
}

fn ppth(path: &str) -> Vec<u8> {
    let mut body = vec![];
    for u in path.encode_utf16() {
        body.extend_from_slice(&u.to_be_bytes());
    }
    body.extend_from_slice(&[0, 0]);
    section(b"PPTH", &(body.len() as u32).to_be_bytes(), &body)
}

fn pvbr() -> Vec<u8> {
    // Seek index; rekordbox leaves it zero for constant-bitrate files (observed).
    section(b"PVBR", &0u32.to_be_bytes(), &vec![0u8; 1604])
}

fn pqtz(beats: &[Beat]) -> Vec<u8> {
    let mut h = vec![];
    h.extend_from_slice(&0u32.to_be_bytes());
    h.extend_from_slice(&0x0008_0000u32.to_be_bytes());
    h.extend_from_slice(&(beats.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(beats.len() * 8);
    for b in beats {
        body.extend_from_slice(&(b.beat as u16).to_be_bytes());
        body.extend_from_slice(&((b.bpm * 100.0).round() as u16).to_be_bytes());
        body.extend_from_slice(&((b.time * 1000.0).round() as u32).to_be_bytes());
    }
    section(b"PQTZ", &h, &body)
}

/// Height scale helper: 0..255 level to 0..max.
fn sc(v: u8, max: u32) -> u8 {
    ((v as u32 * max + 127) / 255) as u8
}

fn pwav(w: &Waveform) -> Vec<u8> {
    let cols = w.columns(400);
    let body: Vec<u8> = cols
        .iter()
        .map(|c| {
            let h = sc(c[0], 31).min(31);
            // "whiteness": brighter where the treble share is high (estimated mapping).
            let white = ((c[3] as u32 * 7) / (c[0] as u32 + 1).max(1)).min(7) as u8;
            h | (white << 5)
        })
        .collect();
    let mut h = vec![];
    h.extend_from_slice(&400u32.to_be_bytes());
    h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    section(b"PWAV", &h, &body)
}

fn pwv2(w: &Waveform) -> Vec<u8> {
    let body: Vec<u8> = w.columns(100).iter().map(|c| sc(c[0], 15)).collect();
    let mut h = vec![];
    h.extend_from_slice(&100u32.to_be_bytes());
    h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    section(b"PWV2", &h, &body)
}

fn pwv3(w: &Waveform) -> Vec<u8> {
    let body: Vec<u8> = w
        .frames
        .iter()
        .map(|c| {
            let h = sc(c[0], 31);
            let white = ((c[3] as u32 * 7) / (c[0] as u32 + 1).max(1)).min(7) as u8;
            h | (white << 5)
        })
        .collect();
    let mut h = vec![];
    h.extend_from_slice(&1u32.to_be_bytes());
    h.extend_from_slice(&(body.len() as u32).to_be_bytes());
    h.extend_from_slice(&0x0096_0000u32.to_be_bytes());
    section(b"PWV3", &h, &body)
}

fn pwv4(w: &Waveform) -> Vec<u8> {
    let cols = w.columns(1200);
    let mut body = Vec::with_capacity(1200 * 6);
    for c in &cols {
        let lo_half = c[1].max(c[2]);
        body.push(sc(c[0], 120));
        body.push(c[0]);
        body.push(sc(lo_half, 120));
        body.push(sc(c[1], 110));
        body.push(sc(c[2], 100));
        body.push(sc(c[3], 90));
    }
    let mut h = vec![];
    h.extend_from_slice(&6u32.to_be_bytes());
    h.extend_from_slice(&1200u32.to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    section(b"PWV4", &h, &body)
}

fn pwv5(w: &Waveform) -> Vec<u8> {
    let mut body = Vec::with_capacity(w.frames.len() * 2);
    for c in &w.frames {
        let tot = (c[1] as u32 + c[2] as u32 + c[3] as u32).max(1);
        let r = (c[1] as u32 * 7 * 2 / tot).min(7) as u16;
        let g = (c[2] as u32 * 7 * 2 / tot).min(7) as u16;
        let b = (c[3] as u32 * 7 * 2 / tot).min(7) as u16;
        let h = sc(c[0], 31) as u16;
        let v = (r << 13) | (g << 10) | (b << 7) | (h << 2);
        body.extend_from_slice(&v.to_be_bytes());
    }
    let mut h = vec![];
    h.extend_from_slice(&2u32.to_be_bytes());
    h.extend_from_slice(&(w.frames.len() as u32).to_be_bytes());
    h.extend_from_slice(&0x0096_0305u32.to_be_bytes());
    section(b"PWV5", &h, &body)
}

fn pwv6(w: &Waveform) -> Vec<u8> {
    let mut body = Vec::with_capacity(1200 * 3);
    for c in w.columns(1200) {
        body.push(sc(c[1], 43));
        body.push(sc(c[2], 43));
        body.push(sc(c[3], 43));
    }
    let mut h = vec![];
    h.extend_from_slice(&3u32.to_be_bytes());
    h.extend_from_slice(&1200u32.to_be_bytes());
    section(b"PWV6", &h, &body)
}

fn pwv7(w: &Waveform) -> Vec<u8> {
    let mut body = Vec::with_capacity(w.frames.len() * 3);
    for c in &w.frames {
        body.push(sc(c[1], 127));
        body.push(sc(c[2], 127));
        body.push(sc(c[3], 127));
    }
    let mut h = vec![];
    h.extend_from_slice(&3u32.to_be_bytes());
    h.extend_from_slice(&(w.frames.len() as u32).to_be_bytes());
    h.extend_from_slice(&0x0096_0000u32.to_be_bytes());
    section(b"PWV7", &h, &body)
}

fn pwvc() -> Vec<u8> {
    // Per-band display calibration; meaning not established, values from a real export.
    section(b"PWVC", &[0, 0], &[0x00, 0x5b, 0x00, 0x8a, 0x01, 0x8c])
}

fn ms(t: f64) -> u32 {
    (t.max(0.0) * 1000.0).round() as u32
}

/// Legacy cue list (PCOB with PCPT entries).
fn pcob(hot: bool, cues: &[&Cue]) -> Vec<u8> {
    let mut body = vec![];
    let n = cues.len();
    for (i, c) in cues.iter().enumerate() {
        let mut e = vec![];
        e.extend_from_slice(&(if hot { (c.slot + 1) as u32 } else { 0 }).to_be_bytes());
        e.extend_from_slice(&(if c.active_loop { 4u32 } else { 0 }).to_be_bytes());
        e.extend_from_slice(&0x0010_0000u32.to_be_bytes());
        e.extend_from_slice(&(if i == 0 { 0xffff } else { (i - 1) as u16 }).to_be_bytes());
        e.extend_from_slice(&(if i + 1 == n { 0xffff } else { (i + 1) as u16 }).to_be_bytes());
        let mut rest = vec![];
        rest.push(if c.is_loop() { 2 } else { 1 });
        rest.push(0);
        rest.extend_from_slice(&1000u16.to_be_bytes());
        rest.extend_from_slice(&ms(c.start).to_be_bytes());
        rest.extend_from_slice(&(if c.is_loop() { ms(c.end.unwrap()) } else { 0xffff_ffff }).to_be_bytes());
        rest.extend_from_slice(&[0u8; 16]);
        body.extend_from_slice(&section(b"PCPT", &e, &rest));
    }
    let mut h = vec![];
    h.extend_from_slice(&(hot as u32).to_be_bytes());
    h.extend_from_slice(&0u16.to_be_bytes());
    h.extend_from_slice(&(n as u16).to_be_bytes());
    h.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
    section(b"PCOB", &h, &body)
}

/// Extended cue list (PCO2 with PCP2 entries) carrying colours and comments.
fn pco2(hot: bool, cues: &[&Cue]) -> Vec<u8> {
    let mut body = vec![];
    for c in cues {
        let mut e = vec![];
        e.extend_from_slice(&(if hot { (c.slot + 1) as u32 } else { 0 }).to_be_bytes());
        let mut r = vec![];
        r.push(if c.is_loop() { 2 } else { 1 });
        r.push(0);
        r.extend_from_slice(&1000u16.to_be_bytes());
        r.extend_from_slice(&ms(c.start).to_be_bytes());
        r.extend_from_slice(&(if c.is_loop() { ms(c.end.unwrap()) } else { 0xffff_ffff }).to_be_bytes());
        // Memory cue colour (track colour ids 0..8); hot cues use the fields further on.
        r.push(if hot { 0 } else { c.color });
        r.extend_from_slice(&[0, 0, 0]);
        r.extend_from_slice(&0u32.to_be_bytes());
        // Quantized loop size (numerator / denominator) unknown here: 0/0.
        r.extend_from_slice(&0u16.to_be_bytes());
        r.extend_from_slice(&0u16.to_be_bytes());
        let mut cm: Vec<u8> = vec![];
        if !c.comment.is_empty() {
            for u in c.comment.encode_utf16() {
                cm.extend_from_slice(&u.to_be_bytes());
            }
            cm.extend_from_slice(&[0, 0]);
        }
        r.extend_from_slice(&(cm.len() as u32).to_be_bytes());
        r.extend_from_slice(&cm);
        if hot {
            let rgb = crate::model::hotcue_rgb(c.color);
            r.push(c.color);
            r.extend_from_slice(&rgb);
        } else {
            r.extend_from_slice(&[0, 0, 0, 0]);
        }
        // Remaining unknown fields (observed record: 40 bytes after the colour for an empty comment).
        r.extend_from_slice(&[0u8; 40]);
        body.extend_from_slice(&section(b"PCP2", &e, &r));
    }
    let mut h = vec![];
    h.extend_from_slice(&(hot as u32).to_be_bytes());
    h.extend_from_slice(&(cues.len() as u16).to_be_bytes());
    h.extend_from_slice(&0u16.to_be_bytes());
    section(b"PCO2", &h, &body)
}

pub struct AnlzFiles {
    pub dat: Vec<u8>,
    pub ext: Vec<u8>,
    pub ex2: Vec<u8>,
}

/// Build the three analysis files. `usb_path` is the media path on the drive
/// (e.g. "/Contents/Artist/Album/file.mp3"). Hot cues beyond H are not exported.
pub fn build(usb_path: &str, beats: &[Beat], wave: &Waveform, cues: &[Cue]) -> AnlzFiles {
    let mut hot: Vec<&Cue> = cues.iter().filter(|c| c.kind == CueKind::Hot && (0..8).contains(&c.slot)).collect();
    hot.sort_by_key(|c| c.slot);
    let mut mem: Vec<&Cue> = cues.iter().filter(|c| c.kind == CueKind::Memory).collect();
    mem.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    let dat = file(vec![ppth(usb_path), pvbr(), pqtz(beats), pwav(wave), pwv2(wave), pcob(true, &hot), pcob(false, &mem)]);
    let ext = file(vec![
        ppth(usb_path),
        pwv3(wave),
        pcob(true, &[]),
        pcob(false, &[]),
        pco2(true, &hot),
        pco2(false, &mem),
        pwv5(wave),
        pwv4(wave),
    ]);
    let ex2 = file(vec![ppth(usb_path), pwv7(wave), pwv6(wave), pwvc()]);
    AnlzFiles { dat, ext, ex2 }
}

/// Folder for a track's analysis files, from its media path on the drive (hash recovered from
/// the original; reproduces real exports, see MECHANICS.md).
pub fn anlz_folder(usb_path: &str) -> String {
    let mut h: u32 = 0;
    for c in usb_path.encode_utf16() {
        h = h.wrapping_mul(23497).wrapping_add(c as u32).wrapping_mul(37813).wrapping_add(c as u32);
    }
    let n = h % 200_003;
    let bits = [0, 2, 6, 7, 9, 13, 16];
    let mut p = 0u32;
    for (k, b) in bits.iter().enumerate() {
        p |= ((n >> b) & 1) << k;
    }
    format!("/PIONEER/USBANLZ/P{:03X}/{:08X}", p, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_hash_matches_real_exports() {
        assert_eq!(anlz_folder("/Contents/Loopmasters/UnknownAlbum/Demo Track 1.mp3"), "/PIONEER/USBANLZ/P016/0000875E");
        assert_eq!(anlz_folder("/Contents/Loopmasters/UnknownAlbum/Demo Track 2.mp3"), "/PIONEER/USBANLZ/P053/0001D21F");
    }
    #[test]
    fn sections_are_well_formed() {
        let w = Waveform { frames: vec![[100, 80, 40, 20]; 1500] };
        let g = crate::model::BeatGrid::constant(0.1, 120.0, 1).beats(10.0);
        let cue = Cue { id: 1, track_id: 1, kind: CueKind::Hot, slot: 0, start: 1.0, end: None, color: 43, comment: "Drop".into(), active_loop: false };
        let mem = Cue { id: 2, track_id: 1, kind: CueKind::Memory, slot: -1, start: 2.0, end: Some(4.0), color: 3, comment: String::new(), active_loop: false };
        let f = build("/Contents/a/b/c.mp3", &g, &w, &[cue, mem]);
        let t = std::env::temp_dir();
        let _ = std::fs::write(t.join("prepdeck_ANLZ0000.DAT"), &f.dat);
        let _ = std::fs::write(t.join("prepdeck_ANLZ0000.EXT"), &f.ext);
        let _ = std::fs::write(t.join("prepdeck_ANLZ0000.2EX"), &f.ex2);
        for d in [&f.dat, &f.ext, &f.ex2] {
            assert_eq!(&d[0..4], b"PMAI");
            let total = u32::from_be_bytes([d[8], d[9], d[10], d[11]]) as usize;
            assert_eq!(total, d.len());
            let mut o = 28;
            while o < d.len() {
                let tl = u32::from_be_bytes([d[o + 8], d[o + 9], d[o + 10], d[o + 11]]) as usize;
                assert!(tl >= 12);
                o += tl;
            }
            assert_eq!(o, d.len());
        }
    }
}
