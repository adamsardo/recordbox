//! Core data types shared by the library, analysis, player and export.

use serde::{Deserialize, Serialize};

/// Camelot-ordered key index: 0..11 = 1A..12A (minor), 12..23 = 1B..12B (major).
pub const KEY_CLASSIC: [&str; 24] = [
    "Abm", "Ebm", "Bbm", "Fm", "Cm", "Gm", "Dm", "Am", "Em", "Bm", "Gbm", "Dbm", //
    "B", "Gb", "Db", "Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E",
];

pub fn key_alnum(k: i32) -> String {
    if !(0..24).contains(&k) {
        return String::new();
    }
    let n = k % 12 + 1;
    format!("{}{}", n, if k < 12 { "A" } else { "B" })
}

pub fn key_name(k: i32, alnum: bool) -> String {
    if !(0..24).contains(&k) {
        return String::new();
    }
    if alnum {
        key_alnum(k)
    } else {
        KEY_CLASSIC[k as usize].to_string()
    }
}

/// Parse a key written either way ("Am", "A#m", "8A", "08A", "C major", "Ebm").
pub fn parse_key(s: &str) -> i32 {
    let t = s.trim();
    if t.is_empty() {
        return -1;
    }
    let up = t.to_ascii_uppercase();
    // Alphanumeric / Camelot.
    let digits: String = up.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let rest = &up[digits.len()..];
        if let Ok(n) = digits.parse::<i32>() {
            if (1..=12).contains(&n) && (rest == "A" || rest == "B" || rest == "M" || rest == "D") {
                let minor = rest == "A" || rest == "M";
                return if minor { n - 1 } else { 12 + n - 1 };
            }
        }
        return -1;
    }
    // Classic: note name to pitch class.
    let mut chars = t.chars();
    let letter = chars.next().unwrap().to_ascii_uppercase();
    let base = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return -1,
    };
    let rest: String = chars.collect();
    let mut pc = base;
    let mut r = rest.as_str();
    if r.starts_with('#') || r.starts_with('♯') {
        pc += 1;
        r = &r[r.chars().next().unwrap().len_utf8()..];
    } else if r.starts_with('b') || r.starts_with('♭') {
        pc += 11;
        r = &r[r.chars().next().unwrap().len_utf8()..];
    }
    let rl = r.trim().to_ascii_lowercase();
    let minor = rl.starts_with('m') && !rl.starts_with("maj");
    key_from_pitch(pc % 12, minor)
}

/// Pitch class (C=0) + mode to Camelot-ordered index.
pub fn key_from_pitch(pc: i32, minor: bool) -> i32 {
    // Camelot number for majors: C=8B, G=9B ... each fifth up adds 1.
    // pitch*7 mod 12 gives circle-of-fifths position from C.
    let fifths = (pc * 7).rem_euclid(12);
    if minor {
        // Am = 8A; A is pitch 9 → fifths 3. 8A index 7.
        ((fifths - 3 + 7).rem_euclid(12)) as i32
    } else {
        // C = 8B index 19.
        12 + ((fifths + 7).rem_euclid(12)) as i32
    }
}

/// Inverse of key_from_pitch.
pub fn key_to_pitch(k: i32) -> (i32, bool) {
    if k < 12 {
        let fifths = (k - 7 + 3).rem_euclid(12);
        ((fifths * 7).rem_euclid(12), true)
    } else {
        let fifths = (k - 12 - 7).rem_euclid(12);
        ((fifths * 7).rem_euclid(12), false)
    }
}

/// Track colour labels (index 1..8; 0 = none).
pub const COLOR_NAMES: [&str; 9] = ["No Color", "Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple"];
pub const COLOR_RGB: [[u8; 3]; 9] = [
    [0x50, 0x50, 0x50],
    [0xF8, 0x70, 0xF8],
    [0xF8, 0x00, 0x00],
    [0xF8, 0xA0, 0x30],
    [0xF8, 0xE3, 0x30],
    [0x00, 0xE0, 0x00],
    [0x00, 0xC0, 0xF8],
    [0x00, 0x50, 0xF8],
    [0x98, 0x08, 0xF8],
];

/// Hot cue palette (index 1..62) as read from the original's hot cue swatch image.
pub const HOTCUE_PALETTE: [u32; 62] = [
    0x305AFF, 0x5073FF, 0x508CFF, 0x50A0FF, 0x50B4FF, 0x50B0F2, 0x50AEE8, 0x45ACDB, //
    0x00E0FF, 0x19DAF0, 0x32D2E6, 0x21B4B9, 0x20AAA0, 0x1FA392, 0x19A08C, 0x14A584, //
    0x14AA7D, 0x10B176, 0x30D26E, 0x37DE5A, 0x3CEB50, 0x28E214, 0x7DC13D, 0x8CC832, //
    0x9BD723, 0xA5E116, 0xA5DC0A, 0xAAD208, 0xB4C805, 0xB4BE04, 0xBAB404, 0xC3AF04, //
    0xE1AA00, 0xFFA000, 0xFF9600, 0xFF8C00, 0xFF7500, 0xE0641B, 0xE0461E, 0xE0301E, //
    0xE02823, 0xE62828, 0xFF376F, 0xFF2D6F, 0xFF127B, 0xF51E8C, 0xEB2DA0, 0xE637B4, //
    0xDE44CF, 0xDE448D, 0xE630B4, 0xE619DC, 0xE600FF, 0xDC00FF, 0xCC00FF, 0xB432FF, //
    0xB93CFF, 0xC542FF, 0xAA5AFF, 0xAA72FF, 0x8272FF, 0x6473FF,
];
/// The 16 colours offered in the hot cue colour picker (palette indices).
pub const HOTCUE_PICKER: [u8; 16] = [1, 5, 9, 14, 18, 22, 26, 30, 32, 38, 42, 45, 48, 56, 60, 62];
/// Default colour for a new hot cue (A observed as #FF376F = index 43; others estimated).
pub const HOTCUE_DEFAULT: u8 = 43;
pub const HOTCUE_LOOP_DEFAULT: u8 = 35; // orange, estimated

pub fn hotcue_rgb(idx: u8) -> [u8; 3] {
    if idx == 0 || idx as usize > HOTCUE_PALETTE.len() {
        return [0x28, 0xE2, 0x14];
    }
    let v = HOTCUE_PALETTE[idx as usize - 1];
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub label: String,
    pub remixer: String,
    pub original_artist: String,
    pub composer: String,
    pub lyricist: String,
    pub mix_name: String,
    pub comment: String,
    pub grouping: String,
    pub message: String,
    pub year: i32,
    pub track_no: i32,
    pub disc_no: i32,
    pub release_date: String,
    pub duration: f64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub bit_depth: u32,
    pub file_size: u64,
    pub file_type: String,
    pub bpm: f64,
    pub key: i32,
    pub rating: u8,
    pub color: u8,
    pub date_added: String,
    pub date_created: String,
    pub play_count: u32,
    /// 0 = not analysed, 1 = analysed, 2 = failed, 3 = queued/running
    pub analyzed: u8,
    pub grid_locked: bool,
    pub autoload_hotcue: bool,
    pub gain_db: f32,
    pub artwork: String,
    pub missing: bool,
    #[serde(default)]
    pub raw_samples: u64,
}

impl Track {
    pub fn file_name(&self) -> String {
        std::path::Path::new(&self.path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }
    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() {
            std::path::Path::new(&self.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            self.title.clone()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CueKind {
    Memory,
    Hot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cue {
    pub id: i64,
    pub track_id: i64,
    pub kind: CueKind,
    /// Hot cue slot 0..7 (A..H); -1 for memory cues.
    pub slot: i32,
    pub start: f64,
    pub end: Option<f64>,
    /// Memory cues: track colour index 0..8. Hot cues: palette index 1..62 (0 = none).
    pub color: u8,
    pub comment: String,
    pub active_loop: bool,
}

impl Cue {
    pub fn is_loop(&self) -> bool {
        self.end.map(|e| e > self.start).unwrap_or(false)
    }
}

/// A beat grid made of tempo segments. Each anchor marks a beat at `time` with tempo `bpm`;
/// beats continue at that tempo until the next anchor.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BeatGrid {
    pub anchors: Vec<GridAnchor>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GridAnchor {
    pub time: f64,
    pub bpm: f64,
    /// Beat number within the bar (1..4) of the beat at `time`.
    pub beat: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Beat {
    pub time: f64,
    pub bpm: f64,
    pub beat: u8,
    pub index: usize,
}

impl BeatGrid {
    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }
    pub fn constant(first_beat: f64, bpm: f64, beat: u8) -> BeatGrid {
        BeatGrid { anchors: vec![GridAnchor { time: first_beat, bpm, beat }] }
    }
    pub fn first_bpm(&self) -> f64 {
        self.anchors.first().map(|a| a.bpm).unwrap_or(0.0)
    }
    /// Every beat from the start of the track to `duration`.
    pub fn beats(&self, duration: f64) -> Vec<Beat> {
        let mut out = vec![];
        if self.anchors.is_empty() {
            return out;
        }
        let a0 = self.anchors[0];
        if a0.bpm <= 0.0 {
            return out;
        }
        // Extrapolate backwards from the first anchor to time 0.
        let period = 60.0 / a0.bpm;
        let back = (a0.time / period).floor() as i64;
        let mut beat_no = a0.beat as i64;
        let mut t = a0.time - back as f64 * period;
        beat_no = (beat_no - 1 - back).rem_euclid(4) + 1;
        let mut idx = 0usize;
        for (ai, a) in self.anchors.iter().enumerate() {
            let end = self.anchors.get(ai + 1).map(|n| n.time).unwrap_or(duration + 1e-9);
            if ai > 0 {
                t = a.time;
                beat_no = a.beat as i64;
            }
            let p = 60.0 / a.bpm.max(1.0);
            // A beat closer than half a period to the next anchor belongs to that anchor.
            let stop = if ai + 1 < self.anchors.len() { end - p * 0.5 } else { end - 1e-6 };
            while t < stop && t <= duration {
                if t >= -1e-6 {
                    out.push(Beat { time: t.max(0.0), bpm: a.bpm, beat: beat_no as u8, index: idx });
                    idx += 1;
                }
                t += p;
                beat_no = beat_no % 4 + 1;
            }
        }
        out
    }
    /// Snap a time to the nearest beat (for quantize).
    pub fn snap(&self, t: f64, duration: f64) -> f64 {
        let beats = self.beats(duration);
        nearest_beat(&beats, t).map(|b| b.time).unwrap_or(t)
    }
}

pub fn nearest_beat(beats: &[Beat], t: f64) -> Option<Beat> {
    if beats.is_empty() {
        return None;
    }
    let i = beats.partition_point(|b| b.time < t);
    let mut best = None;
    for j in [i.saturating_sub(1), i.min(beats.len() - 1)] {
        let b = beats[j];
        match best {
            None => best = Some(b),
            Some(bb) if (b.time - t).abs() < (bb.time - t).abs() => best = Some(b),
            _ => {}
        }
    }
    best
}

/// Waveform data per 1/150 s frame, linear levels of the full signal and three bands:
/// bytes 0..4 = peak × 255 (full, low, mid, high), bytes 4..8 = RMS × 510.
#[derive(Clone, Debug, Default)]
pub struct Waveform {
    pub frames: Vec<[u8; 8]>,
}
pub const WAVE_FPS: f64 = 150.0;

/// Display gains per band applied before the display curve (estimated for legibility).
const DISP_GAIN: [f32; 4] = [1.0, 1.0, 1.5, 2.6];

impl Waveform {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = vec![2u8];
        v.extend(self.frames.iter().flat_map(|f| f.iter().copied()));
        v
    }
    pub fn from_bytes(b: &[u8]) -> Waveform {
        if b.len() % 8 == 1 && b.first() == Some(&2) {
            return Waveform { frames: b[1..].chunks_exact(8).map(|c| [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]).collect() };
        }
        // Older format (display levels only): approximate the linear values.
        let frames = b
            .chunks_exact(4)
            .map(|c| {
                let mut f = [0u8; 8];
                for k in 0..4 {
                    let lin = ((c[k] as f32 / 255.0).powf(1.0 / 0.85) / DISP_GAIN[k]).min(1.0);
                    f[k] = (lin * 255.0) as u8;
                    f[4 + k] = (lin * 0.5 * 510.0).min(255.0) as u8;
                }
                f
            })
            .collect();
        Waveform { frames }
    }
    /// Downsample to `n` columns (max of peaks, mean of RMS per bucket).
    pub fn columns(&self, n: usize) -> Vec<[u8; 8]> {
        let len = self.frames.len();
        if len == 0 || n == 0 {
            return vec![[0; 8]; n];
        }
        (0..n)
            .map(|i| {
                let a = i * len / n;
                let b = ((i + 1) * len / n).max(a + 1).min(len);
                let mut m = [0u8; 8];
                let mut acc = [0u32; 4];
                for f in &self.frames[a..b] {
                    for k in 0..4 {
                        m[k] = m[k].max(f[k]);
                        acc[k] += f[4 + k] as u32;
                    }
                }
                for k in 0..4 {
                    m[4 + k] = (acc[k] / (b - a) as u32) as u8;
                }
                m
            })
            .collect()
    }
    /// Display levels (0..255) of a frame or column: full, low, mid, high.
    pub fn disp(v: &[u8; 8]) -> [u8; 4] {
        let mut o = [0u8; 4];
        for k in 0..4 {
            let x = (v[k] as f32 / 255.0 * DISP_GAIN[k]).min(1.0).powf(0.85);
            o[k] = (x * 255.0).round() as u8;
        }
        o
    }
    /// Display-level preview with `n` columns, 4 bytes each (stored for the track list).
    pub fn preview_bytes(&self, n: usize) -> Vec<u8> {
        self.columns(n).iter().flat_map(|c| Self::disp(c)).collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NodeKind {
    Playlist,
    Folder,
    Smart,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaylistNode {
    pub id: i64,
    pub parent: i64,
    pub name: String,
    pub kind: NodeKind,
    pub seq: i64,
    pub smart: Option<crate::smart::SmartRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MyTag {
    pub id: i64,
    /// 0 for a category row; otherwise the category id.
    pub parent: i64,
    pub name: String,
    pub seq: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys() {
        assert_eq!(parse_key("Am"), 7);
        assert_eq!(key_alnum(parse_key("Am")), "8A");
        assert_eq!(key_alnum(parse_key("C")), "8B");
        assert_eq!(key_alnum(parse_key("F#m")), "11A");
        assert_eq!(key_alnum(parse_key("Abm")), "1A");
        assert_eq!(key_alnum(parse_key("E")), "12B");
        assert_eq!(parse_key("8A"), 7);
        for k in 0..24 {
            let (pc, m) = key_to_pitch(k);
            assert_eq!(key_from_pitch(pc, m), k);
            assert_eq!(parse_key(KEY_CLASSIC[k as usize]), k);
        }
    }
    #[test]
    fn grid() {
        let g = BeatGrid::constant(0.25, 120.0, 1);
        let b = g.beats(10.0);
        assert_eq!(b[0].time, 0.25);
        assert_eq!(b[1].beat, 2);
        assert_eq!(b.len(), 20);
        let g2 = BeatGrid::constant(1.25, 120.0, 3);
        let b2 = g2.beats(4.0);
        assert!((b2[0].time - 0.25).abs() < 1e-9);
        assert_eq!(b2[0].beat, 1);
    }
}
