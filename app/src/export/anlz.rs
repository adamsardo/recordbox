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

/// Seek index: 400 zero entries followed by the total sample count of the file's full decode
/// (observed in the original's files for 108 local tracks).
fn pvbr(raw_samples: u64) -> Vec<u8> {
    let mut body = vec![0u8; 1600];
    body.extend_from_slice(&(raw_samples.min(u32::MAX as u64) as u32).to_be_bytes());
    section(b"PVBR", &0u32.to_be_bytes(), &body)
}

/// Linear models from this app's band levels to each waveform channel of the original's
/// files, fitted on 70 local tracks against the original's own analysis files.
/// Features: [1, peak full, low, mid, high, rms full, low, mid, high, sqrt peak full, low, mid, high].
mod model {
    pub const PWAV_HEIGHT: [f64; 13] = [3.03135, 17.12007, 0.69868, -5.55112, -5.75931, 57.90399, -25.85376, -10.60182, -3.10607, -35.62296, 16.23435, 14.38976, 7.32306];
    pub const PWAV_WHITE: [f64; 13] = [4.50570, -5.29187, 2.72777, 0.91879, -1.11088, -8.11968, 6.93162, 4.49034, 7.36645, 5.90496, -6.36616, -3.26908, 4.61879];
    pub const PWV2_HEIGHT: [f64; 13] = [2.25964, 19.70139, -22.61358, -6.68805, -1.51015, 8.95450, 1.41428, -13.41762, 0.47900, -35.83856, 36.88592, 17.87151, 2.04287];
    pub const PWV3_HEIGHT: [f64; 13] = [-2.16261, 24.90020, 12.05697, -7.79336, -32.09341, -7.40268, 9.70756, -26.52655, 45.37813, -15.67638, 1.71053, 16.95374, 17.63361];
    pub const PWV3_WHITE: [f64; 13] = [5.30439, 2.50468, 8.96202, -8.88094, -9.63281, -0.52760, -0.90728, 9.84407, 9.62981, -4.38970, -14.52101, 9.12481, 7.21387];
    pub const PWV4: [[f64; 13]; 6] = [
        [1.37612, 108.77636, 2.98012, 12.80932, -0.12571, 44.38642, -36.63809, -10.86371, 17.72992, 4.45720, 7.78352, -17.88567, 4.00188],
        [66.04648, -435.79297, 17.38686, -11.55472, 21.64486, -61.89455, 51.75436, 23.46968, 13.76904, 511.92365, -26.69181, 22.22247, -32.06844],
        [3.77519, 57.17777, 22.96768, 12.12057, 1.79882, 74.79551, -75.22735, 21.32916, -76.46239, -44.99123, 89.19619, -9.27890, -8.95646],
        [-0.59613, -21.73611, 119.44813, 21.94283, -0.91354, -22.46012, 26.58133, 0.21008, 2.54063, 32.66253, 7.76133, -38.23688, 4.44855],
        [1.98613, 13.34314, -9.12154, 66.09742, -6.18194, 139.45423, -139.31893, 26.29133, -142.39120, -27.92399, 23.61765, -1.07809, 6.25276],
        [1.96826, -49.01155, -4.50476, 17.22408, 166.87556, -58.47098, 59.95905, 29.62783, 129.29758, 86.21188, 3.98830, -13.04481, -134.28980],
    ];
    pub const PWV5: [[f64; 13]; 4] = [
        [-2.40537, 22.14186, 13.27247, -6.16213, -32.03175, -8.25699, 9.58504, -27.28274, 46.47147, -12.96487, 1.54238, 15.74140, 17.55403],
        [2.50192, 5.33513, -15.84096, 3.56750, 5.08640, -0.20227, 0.13397, -6.98770, -10.73829, -6.80494, 23.60747, -4.83515, -2.24034],
        [2.60047, 6.78224, -0.47220, -15.08857, -3.84492, 0.43141, 1.17405, 2.13511, -0.28079, -13.11151, -2.89172, 22.17049, 2.34895],
        [5.69052, 10.57925, 2.35099, -8.12937, -22.91164, -0.50346, 0.23687, 1.55529, 15.62641, -18.11316, -5.04928, 10.01469, 21.28717],
    ];
    pub const PWV6: [[f64; 13]; 3] = [
        [-0.44578, -8.51963, 4.62029, 6.83503, -1.50459, -100.18567, 152.21663, 92.37922, 57.61135, 13.34355, 7.55685, -12.06794, 3.89098],
        [-2.76985, -0.20643, -4.97277, -24.33844, -8.22292, -43.46421, 39.34981, 209.74523, 85.15138, 3.46844, 2.66735, 25.32570, 4.29872],
        [-2.07634, 12.06332, 1.11387, -5.75550, -43.41470, -24.98885, 21.18503, 40.04945, 344.46849, -17.04930, -2.05763, 6.66002, 48.77071],
    ];
    pub const PWV7: [[f64; 13]; 3] = [
        [7.36570, -5.74102, -23.95699, 35.25720, 10.71000, -4.59174, 7.14800, -90.60947, 0.72244, 35.45054, 91.57040, -28.34832, -19.11846],
        [-4.88456, 34.76648, 6.80177, -101.04472, -71.85855, 6.05436, 23.10675, -3.74485, 46.64680, -64.17889, -2.05040, 166.34076, 40.62119],
        [-4.81462, 30.39690, 8.48572, -37.01762, -128.22948, -1.55879, 7.55559, -12.14972, 274.29239, -55.50643, 5.13877, 38.86642, 100.70658],
    ];
}

/// rekordbox-style 3-band detail values (low, mid, high; PWV7 scale) for one frame or column.
pub fn three_band_detail(c: &[u8; 8]) -> [u8; 3] {
    let f = feats(c);
    [apply(&model::PWV7[0], &f, 255) as u8, apply(&model::PWV7[1], &f, 255) as u8, apply(&model::PWV7[2], &f, 255) as u8]
}

/// rekordbox-style 3-band overview values (low, mid, high; PWV6 scale) for one column.
pub fn three_band_overview(c: &[u8; 8]) -> [u8; 3] {
    let f = feats(c);
    [apply(&model::PWV6[0], &f, 255) as u8, apply(&model::PWV6[1], &f, 255) as u8, apply(&model::PWV6[2], &f, 255) as u8]
}

fn feats(c: &[u8; 8]) -> [f64; 13] {
    let p = |k: usize| c[k] as f64 / 255.0;
    let r = |k: usize| c[4 + k] as f64 / 510.0;
    [1.0, p(0), p(1), p(2), p(3), r(0), r(1), r(2), r(3), p(0).sqrt(), p(1).sqrt(), p(2).sqrt(), p(3).sqrt()]
}

fn apply(w: &[f64; 13], f: &[f64; 13], max: u32) -> u32 {
    let v: f64 = w.iter().zip(f).map(|(a, b)| a * b).sum();
    v.round().clamp(0.0, max as f64) as u32
}

/// Frames padded (with silence) to the count the original uses: ceil(raw samples / hop).
fn padded_frames(w: &Waveform, raw_samples: u64, sample_rate: u32) -> Vec<[u8; 8]> {
    let mut v = w.frames.clone();
    if raw_samples > 0 && sample_rate > 0 {
        let hop = sample_rate as f64 / crate::model::WAVE_FPS;
        let n = (raw_samples as f64 / hop).ceil() as usize;
        v.resize(n.max(v.len().min(n)), [0u8; 8]);
        v.truncate(n);
    }
    v
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

fn pwav(w: &Waveform) -> Vec<u8> {
    let cols = w.columns(400);
    let body: Vec<u8> = cols
        .iter()
        .map(|c| {
            let f = feats(c);
            let h = apply(&model::PWAV_HEIGHT, &f, 31);
            let white = apply(&model::PWAV_WHITE, &f, 7);
            (h | (white << 5)) as u8
        })
        .collect();
    let mut h = vec![];
    h.extend_from_slice(&400u32.to_be_bytes());
    h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    section(b"PWAV", &h, &body)
}

fn pwv2(w: &Waveform) -> Vec<u8> {
    let body: Vec<u8> = w.columns(100).iter().map(|c| apply(&model::PWV2_HEIGHT, &feats(c), 15) as u8).collect();
    let mut h = vec![];
    h.extend_from_slice(&100u32.to_be_bytes());
    h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    section(b"PWV2", &h, &body)
}

fn pwv3(frames: &[[u8; 8]]) -> Vec<u8> {
    let body: Vec<u8> = frames
        .iter()
        .map(|c| {
            let f = feats(c);
            let h = apply(&model::PWV3_HEIGHT, &f, 31);
            let white = apply(&model::PWV3_WHITE, &f, 7);
            (h | (white << 5)) as u8
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
        let f = feats(c);
        for m in &model::PWV4 {
            body.push(apply(m, &f, 255) as u8);
        }
    }
    let mut h = vec![];
    h.extend_from_slice(&6u32.to_be_bytes());
    h.extend_from_slice(&1200u32.to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    section(b"PWV4", &h, &body)
}

fn pwv5(frames: &[[u8; 8]]) -> Vec<u8> {
    let mut body = Vec::with_capacity(frames.len() * 2);
    for c in frames {
        let f = feats(c);
        let h = apply(&model::PWV5[0], &f, 31);
        let r = apply(&model::PWV5[1], &f, 7);
        let g = apply(&model::PWV5[2], &f, 7);
        let b = apply(&model::PWV5[3], &f, 7);
        let v = ((r << 13) | (g << 10) | (b << 7) | (h << 2)) as u16;
        body.extend_from_slice(&v.to_be_bytes());
    }
    let mut h = vec![];
    h.extend_from_slice(&2u32.to_be_bytes());
    h.extend_from_slice(&(frames.len() as u32).to_be_bytes());
    h.extend_from_slice(&0x0096_0305u32.to_be_bytes());
    section(b"PWV5", &h, &body)
}

fn pwv6(w: &Waveform) -> Vec<u8> {
    let mut body = Vec::with_capacity(1200 * 3);
    for c in w.columns(1200) {
        let f = feats(&c);
        for m in &model::PWV6 {
            body.push(apply(m, &f, 255) as u8);
        }
    }
    let mut h = vec![];
    h.extend_from_slice(&3u32.to_be_bytes());
    h.extend_from_slice(&1200u32.to_be_bytes());
    section(b"PWV6", &h, &body)
}

fn pwv7(frames: &[[u8; 8]]) -> Vec<u8> {
    let mut body = Vec::with_capacity(frames.len() * 3);
    for c in frames {
        let f = feats(c);
        for m in &model::PWV7 {
            body.push(apply(m, &f, 255) as u8);
        }
    }
    let mut h = vec![];
    h.extend_from_slice(&3u32.to_be_bytes());
    h.extend_from_slice(&(frames.len() as u32).to_be_bytes());
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
        // Offsets 20/22 are 1/0 (RF-07). Offsets 24/26 link memory records to their
        // neighbours in destination order; hot cues keep 0xffff in both.
        e.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        let (prev, next) = if hot { (0xffff, 0xffff) } else { (if i == 0 { 0xffff } else { (i - 1) as u16 }, if i + 1 == n { 0xffff } else { (i + 1) as u16 }) };
        e.extend_from_slice(&prev.to_be_bytes());
        e.extend_from_slice(&next.to_be_bytes());
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
        // Byte 29 is always 1 in the original's converter.
        r.push(if hot { 0 } else { c.color });
        r.extend_from_slice(&[1, 0, 0]);
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
            // Explicit palette indices 1–64 keep their index; RGB comes from the device palette.
            let idx = if (1..=64).contains(&c.color) { c.color } else { crate::model::hotcue_default_color(c.slot, c.is_loop()) };
            let rgb = crate::model::device_rgb(idx);
            r.push(idx);
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
/// `raw_samples`/`sample_rate`: the file's full decoded length, which the original uses for the
/// seek index total and the number of detail waveform frames (0 = unknown, use the waveform).
pub fn build(usb_path: &str, beats: &[Beat], wave: &Waveform, cues: &[Cue], raw_samples: u64, sample_rate: u32) -> AnlzFiles {
    build_with(usb_path, beats, wave, cues, raw_samples, sample_rate, None)
}

/// Sections taken from rekordbox's own analysis of the same audio file (for tracks synced from
/// the rekordbox library). Waveforms, the VBR seek index and vocal detection always apply; the
/// beat grid, extended grid (PQT2) and phrases (PSSI) only when `grid_same` (the track's grid
/// is still the one rekordbox analysed, since phrases are counted in its beats).
pub struct Overrides {
    pub sections: std::collections::HashMap<String, Vec<u8>>,
    pub grid_same: bool,
    /// PrepDeck's own phrase section (unmasked), used when rekordbox's can't be.
    pub pssi: Option<Vec<u8>>,
    /// The track's analysis files already on the drive, by extension ("DAT", "EXT", "2EX"), in
    /// file order. Audio-derived sections fall back to these, and sections PrepDeck does not
    /// know are carried over unchanged.
    pub device: std::collections::HashMap<&'static str, Vec<(String, Vec<u8>)>>,
    /// PrepDeck's own vocal section, used when rekordbox's is not available.
    pub pvdi: Option<Vec<u8>>,
}

/// Vocal detection section (2EX): discriminator 0, hop 1024 at 22,050 Hz, stride 1, then one
/// level 0–4 per hop, as rekordbox writes it unscrambled.
pub fn pvdi(levels: &[u8]) -> Vec<u8> {
    let mut h = vec![];
    for w in [0u16, 1024, 22050, 1] {
        h.extend_from_slice(&w.to_be_bytes());
    }
    h.extend_from_slice(&(levels.len() as u32).to_be_bytes());
    section(b"PVDI", &h, levels)
}

/// Section tags PrepDeck writes itself (or deliberately drops when the grid changed).
const KNOWN_TAGS: [&str; 16] = ["PPTH", "PVBR", "PQTZ", "PWAV", "PWV2", "PCOB", "PWV3", "PCO2", "PQT2", "PWV5", "PWV4", "PSSI", "PWV7", "PWV6", "PWVC", "PVDI"];
/// Sections that depend only on the audio, so a copy already on the drive stays valid.
const AUDIO_TAGS: [&str; 10] = ["PVBR", "PWAV", "PWV2", "PWV3", "PWV4", "PWV5", "PWV6", "PWV7", "PWVC", "PVDI"];

/// Every top-level section of an ANLZ file in order, including repeated tags.
pub fn read_section_list(path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    std::fs::read(path).map(|b| section_list(&b)).unwrap_or_default()
}

/// Sections of a file on a drive, only when its framing is valid (a damaged file's pieces are
/// not carried into a new one).
pub fn read_valid_section_list(path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    match std::fs::read(path) {
        Ok(b) if validate(&b).is_ok() => section_list(&b),
        _ => vec![],
    }
}

/// Check an ANLZ file's framing: the PMAI header (28 bytes, declared total = file length) and
/// every section's header and total lengths lying inside the file and tiling it exactly.
pub fn validate(b: &[u8]) -> Result<(), String> {
    let be32 = |o: usize| -> Option<usize> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
    if b.len() < 28 || &b[..4] != b"PMAI" {
        return Err("not an analysis file".into());
    }
    let (hl, total) = (be32(4).unwrap_or(0), be32(8).unwrap_or(0));
    if hl != 28 {
        return Err(format!("header length {hl}, expected 28"));
    }
    if total != b.len() {
        return Err(format!("declares {total} bytes, file has {}", b.len()));
    }
    let mut i = hl;
    while i < b.len() {
        let (Some(shl), Some(st)) = (be32(i + 4), be32(i + 8)) else { return Err(format!("section at {i} is cut short")) };
        if st < 12 || shl < 12 || shl > st || i + st > b.len() {
            return Err(format!("section {} at {i} has header {shl} / total {st} outside the file", String::from_utf8_lossy(&b[i..(i + 4).min(b.len())])));
        }
        i += st;
    }
    Ok(())
}

/// Every top-level section of ANLZ file bytes, in order.
pub fn section_list(b: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    if b.len() < 12 || &b[..4] != b"PMAI" {
        return out;
    }
    let be32 = |o: usize| -> Option<usize> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
    let mut i = be32(4).unwrap_or(b.len());
    while i + 12 <= b.len() {
        let Some(total) = be32(i + 8) else { break };
        if total < 12 || i + total > b.len() {
            break;
        }
        out.push((String::from_utf8_lossy(&b[i..i + 4]).to_string(), b[i..i + total].to_vec()));
        i += total;
    }
    out
}

/// Every top-level section of an ANLZ file, by tag (first occurrence), as raw bytes.
pub fn read_sections(path: &std::path::Path) -> std::collections::HashMap<String, Vec<u8>> {
    let mut out = std::collections::HashMap::new();
    let Ok(b) = std::fs::read(path) else { return out };
    if b.len() < 12 || &b[..4] != b"PMAI" {
        return out;
    }
    let be32 = |o: usize| -> Option<usize> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
    let mut i = be32(4).unwrap_or(b.len());
    while i + 12 <= b.len() {
        let Some(total) = be32(i + 8) else { break };
        if total < 12 || i + total > b.len() {
            break;
        }
        let tag = String::from_utf8_lossy(&b[i..i + 4]).to_string();
        out.entry(tag).or_insert_with(|| b[i..i + total].to_vec());
        i += total;
    }
    out
}

/// Cues stored in a track's analysis files: the EXT file's PCO2 lists (names, colours) when
/// present, else the DAT file's PCOB lists.
pub fn parse_cues(ext: &[(String, Vec<u8>)], dat: &[(String, Vec<u8>)]) -> Vec<Cue> {
    let be32 = |b: &[u8], o: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?)) };
    let mut out = vec![];
    let mk = |hot: u32, kind: u8, start: u32, end: u32, color: u8, comment: String| Cue {
        id: 0,
        track_id: 0,
        kind: if hot > 0 { CueKind::Hot } else { CueKind::Memory },
        slot: if hot > 0 { hot as i32 - 1 } else { -1 },
        start: start as f64 / 1000.0,
        end: (kind == 2 && end != u32::MAX).then(|| end as f64 / 1000.0),
        color,
        comment,
        active_loop: false,
    };
    let entries = |sec: &[u8]| -> Vec<Vec<u8>> {
        let mut v = vec![];
        let mut i = be32(sec, 4).unwrap_or(u32::MAX) as usize;
        while i + 12 <= sec.len() {
            let Some(total) = be32(sec, i + 8).map(|x| x as usize) else { break };
            if total < 12 || i + total > sec.len() {
                break;
            }
            v.push(sec[i..i + total].to_vec());
            i += total;
        }
        v
    };
    let pco2: Vec<&Vec<u8>> = ext.iter().filter(|(t, _)| t == "PCO2").map(|(_, b)| b).collect();
    if !pco2.is_empty() {
        for sec in pco2 {
            for e in entries(sec) {
                // Only point cues (1) and loops (2) are known entry types.
                if &e[..4] != b"PCP2" || e.len() < 44 || !matches!(e[16], 1 | 2) {
                    continue;
                }
                let hot = be32(&e, 12).unwrap_or(0);
                let n = be32(&e, 40).unwrap_or(0) as usize;
                let raw = e.get(44..44 + n).unwrap_or(&[]);
                let comment = String::from_utf16_lossy(&raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect::<Vec<_>>());
                let color = if hot > 0 { e.get(44 + n).copied().unwrap_or(0) } else { e[28] };
                out.push(mk(hot, e[16], be32(&e, 20).unwrap_or(0), be32(&e, 24).unwrap_or(u32::MAX), color, comment));
            }
        }
        return out;
    }
    for (_, sec) in dat.iter().filter(|(t, _)| t == "PCOB") {
        for e in entries(sec) {
            if &e[..4] != b"PCPT" || e.len() < 40 || !matches!(e[28], 1 | 2) {
                continue;
            }
            let hot = be32(&e, 12).unwrap_or(0);
            let mut c = mk(hot, e[28], be32(&e, 32).unwrap_or(0), be32(&e, 36).unwrap_or(u32::MAX), 0, String::new());
            c.active_loop = be32(&e, 16) == Some(4);
            out.push(c);
        }
    }
    out
}

/// A stable digest of a cue list as the drive stores it (millisecond positions, colour, name),
/// so edits made on a player can be told apart from what PrepDeck exported.
pub fn cue_fingerprint(cues: &[Cue]) -> String {
    let mut rows: Vec<String> = cues
        .iter()
        .map(|c| format!("{}{}:{}:{}:{}:{}", if c.kind == CueKind::Hot { 'H' } else { 'M' }, c.slot, ms(c.start), c.end.map(ms).map(|x| x as i64).unwrap_or(-1), c.color, c.comment))
        .collect();
    rows.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rows.join("\u{1f}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The fingerprint PrepDeck's own cues get once written to a drive (same normalisation as
/// [`build_with`]: hot cues A–H, default colours filled in).
pub fn local_cue_fingerprint(cues: &[Cue]) -> String {
    let files = build("/x", &[], &Waveform::default(), cues, 0, 44100);
    cue_fingerprint(&parse_cues(&section_list(&files.ext), &section_list(&files.dat)))
}

pub fn build_with(usb_path: &str, beats: &[Beat], wave: &Waveform, cues: &[Cue], raw_samples: u64, sample_rate: u32, ov: Option<&Overrides>) -> AnlzFiles {
    let mut hot: Vec<&Cue> = cues.iter().filter(|c| c.kind == CueKind::Hot && (0..8).contains(&c.slot)).collect();
    hot.sort_by_key(|c| c.slot);
    let mut mem: Vec<&Cue> = cues.iter().filter(|c| c.kind == CueKind::Memory).collect();
    mem.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    let frames = padded_frames(wave, raw_samples, sample_rate);
    let from_device = |tag: &str| -> Option<Vec<u8>> {
        if !AUDIO_TAGS.contains(&tag) {
            return None;
        }
        ov?.device.values().flatten().find(|(t, _)| t == tag).map(|(_, b)| b.clone())
    };
    let take = |tag: &str| ov.and_then(|o| o.sections.get(tag).cloned()).or_else(|| from_device(tag));
    let take_grid = |tag: &str| ov.filter(|o| o.grid_same).and_then(|o| o.sections.get(tag).cloned());
    let or = |tag: &str, f: &dyn Fn() -> Vec<u8>| take(tag).unwrap_or_else(f);
    let dat = vec![
        ppth(usb_path),
        // The trailing sample count is written for MP3 files only (rekordbox writes 0 for WAV:
        // 13 of 13 reference files; 117 MP3s carry the count).
        or("PVBR", &|| pvbr(if usb_path.to_ascii_lowercase().ends_with(".mp3") { raw_samples } else { 0 })),
        take_grid("PQTZ").unwrap_or_else(|| pqtz(beats)),
        or("PWAV", &|| pwav(wave)),
        or("PWV2", &|| pwv2(wave)),
        pcob(true, &hot),
        pcob(false, &mem),
    ];
    // Section order as rekordbox writes it: PPTH, PWV3, PCOB×2, PCO2×2, PQT2, PWV5, PWV4, PSSI.
    let mut ext = vec![ppth(usb_path), or("PWV3", &|| pwv3(&frames)), pcob(true, &[]), pcob(false, &[]), pco2(true, &hot), pco2(false, &mem)];
    if let Some(s) = take_grid("PQT2") {
        ext.push(s);
    }
    ext.push(or("PWV5", &|| pwv5(&frames)));
    ext.push(or("PWV4", &|| pwv4(wave)));
    // Phrases: rekordbox's own when its grid still applies, else PrepDeck's. Written with the
    // XOR mask rekordbox 6+ applies to exported files.
    if let Some(mut s) = take_grid("PSSI").or_else(|| ov.and_then(|o| o.pssi.clone())) {
        if !crate::phrase::is_masked(&s) {
            crate::phrase::mask_section(&mut s);
        }
        ext.push(s);
    }
    let mut ex2 = vec![ppth(usb_path), or("PWV7", &|| pwv7(&frames)), or("PWV6", &|| pwv6(wave)), or("PWVC", &pwvc)];
    if let Some(s) = take("PVDI").or_else(|| ov.and_then(|o| o.pvdi.clone())) {
        ex2.push(s);
    }
    let mut dat = dat;
    // Sections a newer rekordbox (or another tool) wrote that PrepDeck doesn't know stay in
    // their file, after PrepDeck's own.
    if let Some(o) = ov {
        for (ext_name, list) in [("DAT", &mut dat), ("EXT", &mut ext), ("2EX", &mut ex2)] {
            for (tag, bytes) in o.device.get(ext_name).into_iter().flatten() {
                if !KNOWN_TAGS.contains(&tag.as_str()) {
                    list.push(bytes.clone());
                }
            }
        }
    }
    AnlzFiles { dat: file(dat), ext: file(ext), ex2: file(ex2) }
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
    /// Unknown sections already on the drive survive a rewrite; audio sections are reused.
    #[test]
    fn rewrite_keeps_device_sections() {
        let w = Waveform { frames: vec![[100, 80, 40, 20, 60, 50, 20, 10]; 1500] };
        let g = crate::model::BeatGrid::constant(0.1, 120.0, 1).beats(10.0);
        let first = build("/Contents/a.mp3", &g, &w, &[], 441_000, 44100);
        let dir = std::env::temp_dir().join(format!("prepdeck_anlz_keep_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A device file with an extra unknown section appended.
        let mut ext_sections: Vec<Vec<u8>> = vec![];
        {
            let p = dir.join("A.EXT");
            std::fs::write(&p, &first.ext).unwrap();
            for (_, b) in read_section_list(&p) {
                ext_sections.push(b);
            }
        }
        let mut odd = b"PZZZ".to_vec();
        odd.extend_from_slice(&12u32.to_be_bytes());
        odd.extend_from_slice(&16u32.to_be_bytes());
        odd.extend_from_slice(&[1, 2, 3, 4]);
        ext_sections.push(odd.clone());
        std::fs::write(dir.join("A.EXT"), file(ext_sections)).unwrap();
        let mut marker = pwv4(&w);
        let n = marker.len();
        marker[n - 1] ^= 0x5a; // distinguishable from a freshly built PWV4
        let mut device = std::collections::HashMap::new();
        let mut list = read_section_list(&dir.join("A.EXT"));
        for (t, b) in list.iter_mut() {
            if t == "PWV4" {
                *b = marker.clone();
            }
        }
        device.insert("EXT", list);
        let ov = Overrides { sections: Default::default(), grid_same: false, pssi: None, device, pvdi: None };
        let again = build_with("/Contents/a.mp3", &g, &w, &[], 441_000, 44100, Some(&ov));
        std::fs::write(dir.join("B.EXT"), &again.ext).unwrap();
        let out = read_section_list(&dir.join("B.EXT"));
        assert_eq!(out.last().map(|(t, b)| (t.as_str(), b.clone())), Some(("PZZZ", odd)));
        assert_eq!(out.iter().find(|(t, _)| t == "PWV4").map(|(_, b)| b.clone()), Some(marker));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cues read back from built sections match what was written.
    #[test]
    fn cues_round_trip() {
        let w = Waveform { frames: vec![[100, 80, 40, 20, 60, 50, 20, 10]; 100] };
        let cues = vec![
            Cue { id: 1, track_id: 1, kind: CueKind::Hot, slot: 2, start: 1.234, end: Some(5.0), color: 9, comment: "Drop é".into(), active_loop: false },
            Cue { id: 2, track_id: 1, kind: CueKind::Memory, slot: -1, start: 2.5, end: None, color: 3, comment: String::new(), active_loop: false },
        ];
        let f = build("/a.mp3", &[], &w, &cues, 0, 44100);
        let back = parse_cues(&section_list(&f.ext), &section_list(&f.dat));
        assert_eq!(back.len(), 2);
        let h = back.iter().find(|c| c.kind == CueKind::Hot).unwrap();
        assert_eq!((h.slot, h.start, h.end, h.color, h.comment.as_str()), (2, 1.234, Some(5.0), 9, "Drop é"));
        let m = back.iter().find(|c| c.kind == CueKind::Memory).unwrap();
        assert_eq!((m.start, m.end, m.color), (2.5, None, 3));
        assert_eq!(cue_fingerprint(&back), local_cue_fingerprint(&cues));
        // DAT-only fallback keeps positions.
        let dat_only = parse_cues(&[], &section_list(&f.dat));
        assert_eq!(dat_only.len(), 2);
        let mut moved = cues.clone();
        moved[1].start = 2.6;
        assert_ne!(local_cue_fingerprint(&moved), local_cue_fingerprint(&cues));
    }

    #[test]
    fn validates_framing() {
        let w = Waveform { frames: vec![[10; 8]; 50] };
        let f = build("/a.mp3", &[], &w, &[], 0, 44100);
        assert!(validate(&f.dat).is_ok() && validate(&f.ext).is_ok() && validate(&f.ex2).is_ok());
        assert!(validate(&f.dat[..f.dat.len() - 3]).is_err());
        let mut bad = f.ext.clone();
        bad[28 + 8..28 + 12].copy_from_slice(&9_999_999u32.to_be_bytes());
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn folder_hash_matches_real_exports() {
        assert_eq!(anlz_folder("/Contents/Loopmasters/UnknownAlbum/Demo Track 1.mp3"), "/PIONEER/USBANLZ/P016/0000875E");
        assert_eq!(anlz_folder("/Contents/Loopmasters/UnknownAlbum/Demo Track 2.mp3"), "/PIONEER/USBANLZ/P053/0001D21F");
    }
    #[test]
    fn sections_are_well_formed() {
        let w = Waveform { frames: vec![[100, 80, 40, 20, 60, 50, 20, 10]; 1500] };
        let g = crate::model::BeatGrid::constant(0.1, 120.0, 1).beats(10.0);
        let cue = Cue { id: 1, track_id: 1, kind: CueKind::Hot, slot: 0, start: 1.0, end: None, color: 43, comment: "Drop".into(), active_loop: false };
        let mem = Cue { id: 2, track_id: 1, kind: CueKind::Memory, slot: -1, start: 2.0, end: Some(4.0), color: 3, comment: String::new(), active_loop: false };
        let f = build("/Contents/a/b/c.mp3", &g, &w, &[cue, mem], 441_000, 44100);
        // Seek index ends with the total sample count; detail waveform spans the full length.
        assert!(f.dat.windows(4).any(|x| x == 441_000u32.to_be_bytes()));
        let pvbr_tail = |dat: &[u8]| section_list(dat).into_iter().find(|(t, _)| t == "PVBR").map(|(_, b)| b[b.len() - 4..].to_vec()).unwrap();
        assert_eq!(pvbr_tail(&f.dat), 441_000u32.to_be_bytes());
        let wav = build("/Contents/a/b/c.wav", &g, &w, &[], 441_000, 44100);
        assert_eq!(pvbr_tail(&wav.dat), [0, 0, 0, 0]);
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
