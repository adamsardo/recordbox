//! Export a track's vocals or instrumental as a new audio file that joins the collection
//! (recordbox's own feature: rekordbox has no stem export and has stems only in PERFORMANCE).
//!
//! - The part comes from the track's separated stems (the cache `stems::cache_path`, the
//!   deck's separation, or a separation made for the export). The instrumental is the original
//!   mix less the vocal part, as the player plays "everything but the vocals"
//!   (`player_ext::poll_stem_wave`), not the sum of three model outputs; so vocals +
//!   instrumental = the original to within rounding.
//! - The file is as long as the original and starts on the same sample, at the original's
//!   rate, so the original's beat grid, cues and phrases are copied unchanged. A lossy encoder
//!   adds its own lead (LAME 1105 samples and an Info frame, AAC 2112 priming samples) that
//!   recordbox's readers do not trim: it is measured by encoding a probe and reading it back
//!   with the app's own reader, and the same number of leading samples is dropped before
//!   encoding when they are silence (the original's own lead-in); otherwise the copied timeline
//!   is moved by it. The written file is read back and its offset measured again.
//! - Format: the original's by default (MP3 stays MP3 at its bitrate, FLAC stays FLAC at its
//!   bit depth ...), or one chosen in Preferences > Extensions > STEMS.
//! - When the export was started while a normal playlist is the browser's view, the new track
//!   also goes into that playlist, right after the source track (a setting).

use crate::app::*;
use crate::edit_render::{self as render, Encode, Mp3Rate};
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------- settings

/// Preferences > Extensions > STEMS > Stem Export.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StemExportPrefs {
    /// "Add exported stem tracks to the playlist they are exported from".
    pub to_playlist: bool,
    /// Index into `FORMAT_NAMES`: 0 Same as the original, 1 WAV, 2 FLAC, 3 AIFF, 4 M4A, 5 MP3.
    pub format: u8,
    /// Export folder; empty: `default_dir()`.
    pub dir: String,
    /// The explanation pop-up comes up at the next export.
    pub explain: bool,
}

impl Default for StemExportPrefs {
    fn default() -> Self {
        StemExportPrefs { to_playlist: true, format: 0, dir: String::new(), explain: true }
    }
}

pub const FORMAT_NAMES: [&str; 6] = ["Same as the original", "WAV", "FLAC", "AIFF", "M4A (AAC)", "MP3"];

/// recordbox's own folder for exported stems, beside the Edit renders (`ui_edit::render_dir`).
/// A test run with PREPDECK_DATA keeps it inside that folder ("Stem Exports", since the data
/// folder's "stems" is the cache of separated stems and the file system ignores case).
pub fn default_dir() -> PathBuf {
    if std::env::var("PREPDECK_DATA").is_ok() {
        return crate::db::data_dir().join("Stem Exports");
    }
    dirs::audio_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("recordbox").join("Stems")
}

/// rekordbox's folders are never written to.
fn forbidden(p: &Path) -> bool {
    let Some(home) = dirs::home_dir() else { return false };
    p.starts_with(home.join("Music").join("rekordbox")) || p.starts_with(home.join("Library").join("Pioneer")) || p.starts_with(crate::stems::stems_dir())
}

pub fn export_dir(p: &StemExportPrefs) -> PathBuf {
    let chosen = p.dir.trim();
    if chosen.is_empty() || forbidden(Path::new(chosen)) { default_dir() } else { PathBuf::from(chosen) }
}

// ---------------------------------------------------------------------------- parts

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Vocals,
    Instrumental,
}

impl Part {
    pub fn suffix(self) -> &'static str {
        match self {
            Part::Vocals => "Vocals",
            Part::Instrumental => "Instrumental",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Part::Vocals => "vocals",
            Part::Instrumental => "instrumental",
        }
    }
}

/// "<Artist> - <Title> (Vocals)", without characters a file name cannot hold.
pub fn base_name(artist: &str, title: &str, part: Part) -> String {
    let title = if title.trim().is_empty() { "Untitled" } else { title.trim() };
    let s = if artist.trim().is_empty() { format!("{title} ({})", part.suffix()) } else { format!("{} - {title} ({})", artist.trim(), part.suffix()) };
    crate::edit_project::sanitized(&s)
}

/// A file name in `dir` that is not taken: "<base>.<ext>", then "<base> 2.<ext>", "3" ...
pub fn free_path(dir: &Path, base: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{base} {n}.{ext}"));
        n += 1;
    }
    p
}

/// The exported part at the original's rate (interleaved stereo, as long as `orig`).
/// `stems` are at 44.1 kHz ([source][channel][frame], source 3 vocals); the vocal part is
/// brought to the original's rate by linear interpolation where the rates differ. The
/// instrumental is the original less that vocal part.
pub fn render_part(orig: &[f32], sr: u32, stems: &crate::stems::Stems, part: Part) -> Vec<f32> {
    let n = orig.len() / 2;
    let len = stems.len;
    let (vl, vr) = if len > 0 && stems.data.len() >= 8 * len { (&stems.data[6 * len..7 * len], &stems.data[7 * len..8 * len]) } else { (&[][..], &[][..]) };
    let k = crate::stems::htdemucs::SAMPLE_RATE as f64 / sr.max(1) as f64;
    let same = sr == crate::stems::htdemucs::SAMPLE_RATE;
    let mut out = Vec::with_capacity(n * 2);
    for j in 0..n {
        let (l, r) = if vl.is_empty() {
            (0.0, 0.0)
        } else if same {
            if j < len { (vl[j], vr[j]) } else { (0.0, 0.0) }
        } else {
            let p = j as f64 * k;
            let a = p.floor() as usize;
            if a >= len {
                (0.0, 0.0)
            } else {
                let b = (a + 1).min(len - 1);
                let t = (p - a as f64) as f32;
                (vl[a] * (1.0 - t) + vl[b] * t, vr[a] * (1.0 - t) + vr[b] * t)
            }
        };
        match part {
            Part::Vocals => {
                out.push(l);
                out.push(r);
            }
            Part::Instrumental => {
                out.push(orig[j * 2] - l);
                out.push(orig[j * 2 + 1] - r);
            }
        }
    }
    out
}

/// Interleaved stereo floats as integer samples of `ch` channels (1: the two sides
/// averaged) at `bits`, full scale 2^(bits-1) as the decoders read them; clipped, not
/// normalised.
pub fn quantize(x: &[f32], ch: u16, bits: u16) -> Vec<i32> {
    let scale = (1u32 << (bits - 1)) as f32;
    let (lo, hi) = (-(scale as i64) as f32, scale - 1.0);
    let q = |v: f32| (v * scale).round().clamp(lo, hi) as i32;
    if ch == 1 {
        x.chunks_exact(2).map(|c| q(0.5 * (c[0] + c[1]))).collect()
    } else {
        x.iter().map(|v| q(*v)).collect()
    }
}

/// Linear resampling of interleaved stereo (only when a lossy format cannot take the rate).
fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return x.to_vec();
    }
    let frames = x.len() / 2;
    let out_frames = (frames as f64 * to as f64 / from as f64).round() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let p = i as f64 * from as f64 / to as f64;
        let a = (p.floor() as usize).min(frames.saturating_sub(1));
        let b = (a + 1).min(frames.saturating_sub(1));
        let t = (p - a as f64) as f32;
        for c in 0..2 {
            out.push(x[a * 2 + c] * (1.0 - t) + x[b * 2 + c] * t);
        }
    }
    out
}

// ---------------------------------------------------------------------------- formats

/// What the original file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wav,
    Aiff,
    Flac,
    Mp3,
    Aac,
    Alac,
    OtherLossless,
    OtherLossy,
}

impl Kind {
    pub fn lossy(self) -> bool {
        matches!(self, Kind::Mp3 | Kind::Aac | Kind::OtherLossy)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub kind: Kind,
    pub rate: u32,
    pub ch: u16,
    /// Bits per sample of a lossless original (0: not known).
    pub bits: u16,
    /// Bitrate in kbps (for VBR the average; 0: not known).
    pub kbps: u32,
    pub vbr: bool,
}

/// How the export is written.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub enc: Encode,
    pub ext: &'static str,
    pub name: &'static str,
    pub rate: u32,
    pub ch: u16,
    /// Set when the original's format has no encoder here and another one is used.
    pub fallback: Option<String>,
}

impl Plan {
    pub fn lossy(&self) -> bool {
        matches!(self.enc.format, 3 | 4)
    }
    /// "MP3 320 kbps", "FLAC 24-bit" ...
    pub fn describe(&self) -> String {
        match (self.enc.format, self.enc.mp3) {
            (4, Mp3Rate::Cbr(k)) => format!("MP3 {k} kbps"),
            (4, Mp3Rate::Vbr(q)) => format!("MP3 VBR V{q}"),
            (3, _) => format!("M4A (AAC) {} kbps", self.enc.aac_bps / 1000),
            _ => format!("{} {}-bit", self.name, self.enc.bits),
        }
    }
}

/// Typical average bitrate of LAME's -V0 .. -V9.
const VBR_TYPICAL: [u32; 10] = [245, 225, 190, 175, 165, 130, 115, 100, 85, 65];

/// The VBR quality whose typical bitrate is not above `kbps` (so the file is not bigger).
pub fn vbr_quality(kbps: u32) -> u8 {
    if kbps == 0 {
        return 2;
    }
    VBR_TYPICAL.iter().position(|t| *t <= kbps).unwrap_or(9) as u8
}

/// The highest constant MP3 bitrate not above `kbps` (1 kbps of rounding allowed).
pub fn cbr_at_most(kbps: u32) -> u32 {
    if kbps == 0 {
        return 320;
    }
    render::MP3_RATES.iter().copied().filter(|r| *r <= kbps + 1).last().unwrap_or(32)
}

/// A rate MP3 and AAC can take: the same when they can, else 48 or 44.1 kHz.
pub fn lossy_rate(rate: u32) -> u32 {
    match rate {
        8000 | 11025 | 12000 | 16000 | 22050 | 24000 | 32000 | 44100 | 48000 => rate,
        r if r > 48000 && r % 48000 == 0 => 48000,
        r if r > 48000 => 44100,
        _ => 44100,
    }
}

/// The format of an export: `setting` is `StemExportPrefs::format`.
pub fn plan(setting: u8, s: &SourceInfo) -> Plan {
    let bits = if !s.kind.lossy() && s.bits > 16 { 24 } else { 16 };
    let ch = if s.ch == 1 { 1 } else { 2 };
    let mp3 = if s.kind == Kind::Mp3 && s.vbr { Mp3Rate::Vbr(vbr_quality(s.kbps)) } else if s.kind.lossy() { Mp3Rate::Cbr(cbr_at_most(s.kbps.min(320))) } else { Mp3Rate::Cbr(320) };
    let aac_bps = if s.kind.lossy() && s.kbps > 0 { s.kbps.clamp(64, 320) * 1000 } else { 256_000 };
    let enc = |format: u8| Encode { format, bits, flac_level: 5, aac_bps, mp3 };
    let mut fallback = None;
    let format = match setting {
        1 => 1,
        2 => 0,
        3 => 2,
        4 => 3,
        5 => 4,
        _ => match s.kind {
            Kind::Wav => 1,
            Kind::Aiff => 2,
            Kind::Flac => 0,
            Kind::Mp3 => 4,
            Kind::Aac => 3,
            Kind::Alac => {
                fallback = Some("ALAC has no encoder in recordbox, so it was written as FLAC (lossless)".to_string());
                0
            }
            Kind::OtherLossless => {
                fallback = Some("the original's format has no encoder in recordbox, so it was written as FLAC (lossless)".to_string());
                0
            }
            Kind::OtherLossy => {
                fallback = Some("the original's format has no encoder in recordbox, so it was written as M4A (AAC)".to_string());
                3
            }
        },
    };
    let (ext, name) = match format {
        0 => ("flac", "FLAC"),
        2 => ("aiff", "AIFF"),
        3 => ("m4a", "M4A"),
        4 => ("mp3", "MP3"),
        _ => ("wav", "WAV"),
    };
    let mut e = enc(format);
    if matches!(format, 3 | 4) {
        e.bits = 16;
    }
    let rate = if matches!(format, 3 | 4) { lossy_rate(s.rate) } else { s.rate };
    Plan { enc: e, ext, name, rate, ch, fallback }
}

/// Read what the original is from its container and codec (no decoding).
pub fn probe_source(path: &str, kbps: u32) -> Option<SourceInfo> {
    use symphonia::core::codecs::*;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    hint.with_extension(&ext);
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let t = probed.format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?.clone();
    let c = t.codec_params.codec;
    let kind = if c == CODEC_TYPE_MP3 {
        Kind::Mp3
    } else if c == CODEC_TYPE_AAC {
        Kind::Aac
    } else if c == CODEC_TYPE_ALAC {
        Kind::Alac
    } else if c == CODEC_TYPE_FLAC {
        Kind::Flac
    } else if c == CODEC_TYPE_VORBIS || c == CODEC_TYPE_OPUS || c == CODEC_TYPE_MP1 || c == CODEC_TYPE_MP2 {
        Kind::OtherLossy
    } else if matches!(ext.as_str(), "wav" | "wave") {
        Kind::Wav
    } else if matches!(ext.as_str(), "aif" | "aiff" | "aifc") {
        Kind::Aiff
    } else {
        Kind::OtherLossless
    };
    let rate = t.codec_params.sample_rate.unwrap_or(44100);
    let ch = t.codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);
    let bits = t.codec_params.bits_per_sample.or(t.codec_params.bits_per_coded_sample).unwrap_or(0) as u16;
    let (mut kbps, mut vbr) = (kbps, false);
    if kind == Kind::Mp3 {
        if let Some((hdr_kbps, is_vbr)) = mp3_rate_info(path) {
            vbr = is_vbr;
            if !is_vbr {
                kbps = hdr_kbps;
            }
        }
    }
    Some(SourceInfo { kind, rate, ch, bits, kbps, vbr })
}

/// An MP3's constant bitrate (from its first audio frame) and whether it is VBR (a Xing or
/// VBRI header; LAME writes "Info" in a constant-bitrate file).
pub fn mp3_rate_info(path: &str) -> Option<(u32, bool)> {
    use std::io::Read;
    let mut d = Vec::new();
    std::fs::File::open(path).ok()?.take(1 << 20).read_to_end(&mut d).ok()?;
    mp3_rate_info_in(&d)
}

pub fn mp3_rate_info_in(d: &[u8]) -> Option<(u32, bool)> {
    const BR1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const BR2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let mut p = 0usize;
    if d.len() >= 10 && &d[..3] == b"ID3" {
        p = 10 + (((d[6] as usize & 0x7f) << 21) | ((d[7] as usize & 0x7f) << 14) | ((d[8] as usize & 0x7f) << 7) | (d[9] as usize & 0x7f));
    }
    let mut tagged_vbr = None;
    while p + 4 <= d.len() {
        let h = &d[p..p + 4];
        let ok = h[0] == 0xFF && h[1] & 0xE0 == 0xE0 && (h[1] >> 1) & 3 == 1 && h[2] >> 4 != 15 && h[2] >> 4 != 0 && (h[2] >> 2) & 3 != 3;
        if !ok {
            p += 1;
            continue;
        }
        let m1 = h[1] & 0x08 != 0;
        let kbps = if m1 { BR1 } else { BR2 }[(h[2] >> 4) as usize];
        let sr = if m1 { [44100, 48000, 32000] } else { [22050, 24000, 16000] }[((h[2] >> 2) & 3) as usize];
        let len = (if m1 { 144 } else { 72 }) * kbps as usize * 1000 / sr + ((h[2] >> 1) & 1) as usize;
        let frame = &d[p..(p + len.max(4)).min(d.len())];
        let has = |pat: &[u8]| frame.windows(4).any(|w| w == pat);
        if tagged_vbr.is_none() && (has(b"Xing") || has(b"VBRI") || has(b"Info")) {
            // The tag frame: its own bitrate says nothing; the next frame does.
            tagged_vbr = Some(has(b"Xing") || has(b"VBRI"));
            p += len.max(1);
            continue;
        }
        return Some((kbps, tagged_vbr.unwrap_or(false)));
    }
    None
}

// ---------------------------------------------------------------------------- timing

/// Mono of interleaved stereo.
fn mono(x: &[f32]) -> Vec<f32> {
    x.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect()
}

/// The lag (in frames, `dec` later than `fed` when positive) at which `win` (taken from
/// `fed` at frame `at`) best matches `dec`, searched within ±`max`; with its normalised
/// correlation.
fn best_lag(win: &[f32], at: usize, dec: &[f32], max: usize) -> Option<(isize, f32)> {
    let e_w: f32 = win.iter().map(|v| v * v).sum();
    if e_w <= 1e-6 {
        return None;
    }
    let mut best: Option<(isize, f32)> = None;
    for lag in -(max as isize)..=(max as isize) {
        let s = at as isize + lag;
        if s < 0 || s as usize + win.len() > dec.len() {
            continue;
        }
        let d = &dec[s as usize..s as usize + win.len()];
        let mut dot = 0.0f32;
        let mut e_d = 0.0f32;
        for (a, b) in win.iter().zip(d) {
            dot += a * b;
            e_d += b * b;
        }
        let c = dot / (e_w * e_d).sqrt().max(1e-12);
        if best.is_none_or(|b| c > b.1) {
            best = Some((lag, c));
        }
    }
    best
}

/// Where the part is loud enough to measure: up to three windows of `w` frames.
fn windows(m: &[f32], w: usize) -> Vec<usize> {
    if m.len() < w * 2 {
        return if m.len() >= w { vec![0] } else { vec![] };
    }
    let rms = |a: usize| (m[a..a + w].iter().map(|v| v * v).sum::<f32>() / w as f32).sqrt();
    let mut picks: Vec<usize> = [0.3, 0.5, 0.7].iter().map(|f| ((m.len() - w) as f64 * f) as usize).filter(|a| rms(*a) > 1e-3).collect();
    if picks.is_empty() {
        let mut cands: Vec<(f32, usize)> = (1..16).map(|i| (m.len() - w) * i / 16).map(|a| (rms(a), a)).collect();
        cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        picks = cands.into_iter().take(2).filter(|c| c.0 > 1e-5).map(|c| c.1).collect();
    }
    picks
}

/// The offset of a decoded file against the samples it was made from (frames): the lag
/// every window agrees on, or None when they disagree or nothing can be measured.
fn measure_offset(fed_mono: &[f32], dec_mono: &[f32], max: usize) -> Option<isize> {
    let w = 16384.min(fed_mono.len());
    let mut lags = vec![];
    for at in windows(fed_mono, w) {
        if let Some((lag, c)) = best_lag(&fed_mono[at..at + w], at, dec_mono, max) {
            if c > 0.5 {
                lags.push(lag);
            }
        }
    }
    let first = *lags.first()?;
    lags.iter().all(|l| *l == first).then_some(first)
}

/// The lead a lossy encoder (with the app's reader) puts in front of the audio, in frames:
/// a noise burst is encoded with the same settings, read back with `audio::decode_file` and
/// found again.
pub fn encoder_lead(enc: &Encode, ch: u16, rate: u32, dir: &Path) -> Result<usize, String> {
    if !matches!(enc.format, 3 | 4) {
        return Ok(0);
    }
    let n = rate as usize * 2;
    let mut seed = 0x1234_5678u32;
    let mut x = vec![0.0f32; n * 2];
    for i in rate as usize * 3 / 10..rate as usize * 13 / 10 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let v = ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.6;
        x[i * 2] = v;
        x[i * 2 + 1] = v;
    }
    let ext = if enc.format == 4 { "mp3" } else { "m4a" };
    let tmp = dir.join(format!(".recordbox-probe-{}-{}.{ext}", std::process::id(), seq()));
    assert!(is_temp_in(dir, &tmp));
    let r = (|| {
        render::write_n(&tmp, &quantize(&x, ch, 16), ch, rate, enc)?;
        let d = crate::audio::decode_file(&tmp.to_string_lossy()).map_err(|e| format!("The encoder's output could not be read back: {e}"))?;
        let m = mono(&x);
        let a = rate as usize * 3 / 10;
        let w = rate as usize / 2;
        let (lag, c) = best_lag(&m[a..a + w], a, &mono(&d.samples), 6000).ok_or("The encoder's lead could not be measured.")?;
        if c < 0.5 || lag < 0 {
            return Err(format!("The encoder's lead could not be measured (best match {c:.2} at {lag})."));
        }
        Ok(lag as usize)
    })();
    remove_temp(dir, &tmp);
    r
}

fn seq() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed) + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

// ---------------------------------------------------------------------------- the file

/// What was written.
#[derive(Clone, Debug)]
pub struct Written {
    pub path: PathBuf,
    pub plan: Plan,
    /// Frames of the file as the app reads it, its peak, and its offset against the original's
    /// timeline (frames at `plan.rate`; 0 when it lines up).
    pub frames: usize,
    pub peak: f32,
    pub offset: Option<isize>,
    /// Leading frames dropped to make up for the encoder's lead.
    pub dropped: usize,
    /// Seconds the copied grid, cues and phrases are moved by (0 when the audio lines up).
    pub shift: f64,
    pub src_frames: usize,
    pub src_rate: u32,
}

/// Progress of a background export (0..1 per stage).
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    Decoding,
    Separating(f32),
    Writing,
    Checking,
}

/// Everything the background export needs.
pub struct Work {
    pub path: String,
    pub kbps: u32,
    pub base: String,
    pub dir: PathBuf,
    pub part: Part,
    pub setting: u8,
    /// The original, decoded (else it is decoded here).
    pub audio: Option<Arc<crate::audio::Decoded>>,
    /// Complete stems (else from the cache, or separated here).
    pub stems: Option<Arc<crate::stems::Stems>>,
    pub model: Option<&'static crate::stems::ModelSpec>,
    pub cache_limit: u64,
}

const MSG_STOPPED: &str = "stopped";

/// The whole export on the calling thread. Nothing is left behind when it fails or is
/// cancelled: the file is written under a temporary name and renamed when it is complete.
pub fn run(w: Work, progress: &mut dyn FnMut(Stage), cancel: &AtomicBool) -> Result<Written, String> {
    let stop = || cancel.load(Ordering::Relaxed);
    if !w.dir.is_dir() {
        std::fs::create_dir_all(&w.dir).map_err(|e| format!("The export folder {} could not be made: {e}", w.dir.display()))?;
    }
    clean_partials(&w.dir);
    progress(Stage::Decoding);
    let audio = match w.audio {
        Some(a) => a,
        None => Arc::new(crate::audio::decode_file(&w.path).map_err(|e| format!("The track could not be read: {e}"))?),
    };
    if stop() {
        return Err(MSG_STOPPED.into());
    }
    let src = probe_source(&w.path, w.kbps).unwrap_or(SourceInfo { kind: Kind::OtherLossless, rate: audio.sr, ch: 2, bits: 16, kbps: w.kbps, vbr: false });
    let stems = match w.stems {
        Some(s) => s,
        None => {
            let m = w.model.ok_or("No separation model is chosen yet.")?;
            let kept = crate::stems::cache_path(&w.path, m.id);
            match crate::stems::load(&kept) {
                Ok(s) => Arc::new(s),
                Err(_) => {
                    progress(Stage::Separating(0.0));
                    let s = crate::stems::separate(m, &audio.samples, audio.sr, &mut |p| {
                        progress(Stage::Separating(p));
                        !stop()
                    })
                    .map_err(|e| if stop() { MSG_STOPPED.to_string() } else { format!("Stem separation failed: {e}") })?;
                    let _ = crate::stems::save(&kept, &s);
                    crate::stems::trim_cache(w.cache_limit, &kept);
                    Arc::new(s)
                }
            }
        }
    };
    if stop() {
        return Err(MSG_STOPPED.into());
    }
    progress(Stage::Writing);
    let mut plan = plan(w.setting, &SourceInfo { rate: audio.sr, ..src });
    let src_frames = audio.frames();
    let part = render_part(&audio.samples, audio.sr, &stems, w.part);
    drop(stems);
    let part = if plan.rate != audio.sr { resample(&part, audio.sr, plan.rate) } else { part };
    let src_rate = audio.sr;
    // The encoder's lead, and whether the same number of leading samples can go: they can when
    // the original is silent there (its own lead-in: an MP3's Info frame and encoder delay).
    // The part may hold a little of the model's noise there, which goes with them.
    let lead = encoder_lead(&plan.enc, plan.ch, plan.rate, &w.dir)?;
    let head = ((lead as f64 * src_rate as f64 / plan.rate as f64).ceil() as usize).min(audio.frames());
    let silent_head = audio.samples[..head * 2].iter().all(|v| v.abs() < 1e-3);
    drop(audio);
    let dropped = if lead > 0 && silent_head { lead.min(part.len() / 2) } else { 0 };
    let fed_mono = mono(&part);
    let pcm = quantize(&part[dropped * 2..], plan.ch, plan.enc.bits);
    drop(part);
    if stop() {
        return Err(MSG_STOPPED.into());
    }
    let tmp = w.dir.join(format!(".recordbox-stem-{}-{}.partial.{}", std::process::id(), seq(), plan.ext));
    assert!(is_temp_in(&w.dir, &tmp) && Path::new(&w.path) != tmp);
    let r = render::write_n(&tmp, &pcm, plan.ch, plan.rate, &plan.enc).map_err(|e| e.replace("Rendering will stop.", "The export stopped."));
    drop(pcm);
    if let Err(e) = r {
        remove_temp(&w.dir, &tmp);
        return Err(e);
    }
    if stop() {
        remove_temp(&w.dir, &tmp);
        return Err(MSG_STOPPED.into());
    }
    // Read back with the app's own reader: length, peak and where it sits on the timeline.
    progress(Stage::Checking);
    let back = match crate::audio::decode_file(&tmp.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => {
            remove_temp(&w.dir, &tmp);
            return Err(format!("The written file could not be read back: {e}"));
        }
    };
    let peak = back.samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let frames = back.frames();
    let dec_mono = mono(&back.samples);
    drop(back);
    let offset = measure_offset(&fed_mono, &dec_mono, if plan.lossy() { 6000 } else { 64 });
    let expected = lead as isize - dropped as isize;
    let shift = offset.unwrap_or(expected) as f64 / plan.rate as f64;
    if stop() {
        remove_temp(&w.dir, &tmp);
        return Err(MSG_STOPPED.into());
    }
    // The final name is free (never written over), in the export folder, not the source.
    let path = free_path(&w.dir, &w.base, plan.ext);
    assert!(path.parent() == Some(w.dir.as_path()) && !path.exists() && Path::new(&w.path) != path && is_temp_in(&w.dir, &tmp));
    if let Err(e) = std::fs::rename(&tmp, &path) {
        remove_temp(&w.dir, &tmp);
        return Err(format!("The file could not be saved: {e}"));
    }
    if plan.rate != src_rate {
        let note = format!("resampled to {:.1} kHz for {}", plan.rate as f64 / 1000.0, plan.name);
        plan.fallback = Some(match plan.fallback.take() { Some(f) => format!("{f}; {note}"), None => note });
    }
    Ok(Written { path, plan, frames, peak, offset, dropped, shift, src_frames, src_rate })
}

/// Whether `p` is one of the export's own temporary files in `dir` (the only files the
/// export ever renames or removes, besides a finished file it made itself).
pub fn is_temp_in(dir: &Path, p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    p.parent() == Some(dir) && ((n.starts_with(".recordbox-stem-") && n.contains(".partial.")) || n.starts_with(".recordbox-probe-"))
}

/// Remove one of the export's temporary files; anything else is refused (and reported).
fn remove_temp(dir: &Path, p: &Path) {
    if is_temp_in(dir, p) {
        let _ = std::fs::remove_file(p);
    } else {
        debug_assert!(false, "refused to remove {}", p.display());
        eprintln!("stem export: refused to remove {} (not a temporary file of the export folder)", p.display());
    }
}

/// Remove a finished export that could not be added to the collection: only a file in the
/// export folder that no track of the collection uses.
fn remove_finished(app: &App, p: &Path) {
    let dir = export_dir(&app.prefs.stemx);
    let in_dir = p.parent() == Some(dir.as_path());
    let used = app.db.track_by_path(&p.to_string_lossy()).is_some() || app.tracks.iter().any(|t| Path::new(&t.path) == p);
    if in_dir && !used {
        let _ = std::fs::remove_file(p);
    } else {
        eprintln!("stem export: left {} in place (not an unused file of the export folder)", p.display());
    }
}

/// Temporary files of exports that did not finish (a crash): never shown, removed.
fn clean_partials(dir: &Path) {
    let mine = format!(".recordbox-stem-{}-", std::process::id());
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if is_temp_in(dir, &e.path()) && !n.starts_with(&mine) {
                remove_temp(dir, &e.path());
            }
        }
    }
}

// ---------------------------------------------------------------------------- playlists

/// The playlist an export started from goes into: the browser's view when it is a normal
/// playlist (not Collection, a folder, a smart playlist, a history, the Tag List, a device or
/// any other list). A playlist filtered by the search field is still that playlist.
pub fn view_playlist(view: &View, nodes: &[PlaylistNode]) -> Option<i64> {
    match view {
        View::Playlist(id) => nodes.iter().find(|n| n.id == *id && n.kind == NodeKind::Playlist).map(|n| n.id),
        _ => None,
    }
}

/// Where the new track goes in the playlist: right after the source's row (and after the
/// stem tracks of the same source already right behind it), else at the end.
pub fn insert_at(list: &[i64], src: i64, of_src: impl Fn(i64) -> bool) -> usize {
    match list.iter().position(|t| *t == src) {
        Some(mut i) => {
            i += 1;
            while i < list.len() && of_src(list[i]) {
                i += 1;
            }
            i
        }
        None => list.len(),
    }
}

// ---------------------------------------------------------------------------- the first export

/// The explanation pop-up waiting for an answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub reqs: Vec<Request>,
    /// The playlist the export was started in (id, name).
    pub playlist: Option<(i64, String)>,
    /// "Also add exported stem tracks to the playlist they are exported from".
    pub add: bool,
    /// "Don't show this again".
    pub dont_show: bool,
}

/// Whether an export started now asks first.
pub fn asks(p: &StemExportPrefs) -> bool {
    p.explain
}

/// The pop-up's answer: OK saves the choices and returns true (the export goes ahead);
/// Cancel changes nothing and starts nothing.
pub fn answer(p: &mut StemExportPrefs, a: &Ask, ok: bool) -> bool {
    if !ok {
        return false;
    }
    if a.playlist.is_some() {
        p.to_playlist = a.add;
    }
    if a.dont_show {
        p.explain = false;
    }
    true
}

// ---------------------------------------------------------------------------- the queue

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub track: i64,
    pub part: Part,
    /// The normal playlist that was the browser's view when the export was started.
    pub view_playlist: Option<i64>,
    /// Whether it goes into that playlist (the setting when the export was queued).
    pub to_playlist: bool,
    /// Started from the deck's stem strip.
    pub from_strip: bool,
}

pub enum Msg {
    Stage(Stage),
    Done(Result<Written, String>),
}

pub struct Active {
    pub req: Request,
    pub name: String,
    pub cancel: Arc<AtomicBool>,
    /// None while waiting for the deck's separation (or the model download).
    pub rx: Option<crossbeam_channel::Receiver<Msg>>,
    pub stage: Stage,
    /// The deck's separation was started for this export (stopping the export stops it).
    pub started_separation: bool,
}

/// Results kept for the scripted check.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub req: Request,
    pub result: Result<(i64, Written), String>,
}

#[derive(Default)]
pub struct StemExport {
    pub queue: VecDeque<Request>,
    pub active: Option<Active>,
    pub ask: Option<Ask>,
    /// Exports in the current batch and how many have finished.
    pub total: usize,
    pub finished: usize,
    /// Tracks made by a stem export (their `stem_of_<id>` link), read once.
    pub made: Option<HashSet<i64>>,
    pub outcomes: Vec<Outcome>,
    /// Places of the pop-up's controls this frame (for the scripted run).
    pub rects: std::collections::HashMap<String, eframe::egui::Rect>,
}

/// Key of the setting that links an exported stem track to its source ("<id>:<part>").
pub fn link_key(id: i64) -> String {
    format!("stem_of_{id}")
}

/// The tracks made by stem exports.
pub fn made_set(app: &mut App) -> &HashSet<i64> {
    if app.stemx.made.is_none() {
        let mut set = HashSet::new();
        if let Ok(mut st) = app.db.conn.prepare("SELECT key FROM settings WHERE substr(key, 1, 8) = 'stem_of_'") {
            if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                for k in rows.flatten() {
                    if let Some(id) = k.strip_prefix("stem_of_").and_then(|v| v.parse::<i64>().ok()) {
                        set.insert(id);
                    }
                }
            }
        }
        app.stemx.made = Some(set);
    }
    app.stemx.made.as_ref().unwrap()
}

pub fn is_stem_track(app: &mut App, id: i64) -> bool {
    made_set(app).contains(&id)
}

/// Start exporting `part` of `tracks` (the selected tracks, or the deck's track). The first
/// time, the explanation comes up first.
pub fn request(app: &mut App, tracks: &[i64], part: Part, from_strip: bool) {
    let tracks: Vec<i64> = tracks.iter().copied().filter(|t| app.track(*t).is_some_and(|t| !t.missing)).collect();
    if tracks.is_empty() {
        app.toast_err("There is no track to export stems from.");
        return;
    }
    let pl = view_playlist(&app.view, &app.nodes);
    let reqs: Vec<Request> = tracks.iter().map(|t| Request { track: *t, part, view_playlist: pl, to_playlist: false, from_strip }).collect();
    if asks(&app.prefs.stemx) {
        let playlist = pl.and_then(|id| app.node(id).map(|n| (id, n.name.clone())));
        app.stemx.ask = Some(Ask { reqs, playlist, add: app.prefs.stemx.to_playlist, dont_show: true });
        return;
    }
    enqueue(app, reqs);
}

/// The pop-up was answered.
pub fn answered(app: &mut App, a: Ask, ok: bool) {
    if answer(&mut app.prefs.stemx, &a, ok) {
        app.save_prefs();
        enqueue(app, a.reqs);
    }
}

fn enqueue(app: &mut App, reqs: Vec<Request>) {
    if app.stemx.active.is_none() && app.stemx.queue.is_empty() {
        app.stemx.total = 0;
        app.stemx.finished = 0;
    }
    let to = app.prefs.stemx.to_playlist;
    for mut r in reqs {
        r.to_playlist = to && r.view_playlist.is_some();
        // The same export already waiting is not queued twice.
        if app.stemx.queue.iter().any(|q| q.track == r.track && q.part == r.part) || app.stemx.active.as_ref().is_some_and(|a| a.req.track == r.track && a.req.part == r.part) {
            continue;
        }
        app.stemx.queue.push_back(r);
        app.stemx.total += 1;
    }
    poll(app);
}

/// Stop the export in progress and those waiting.
pub fn stop(app: &mut App) {
    app.stemx.queue.clear();
    if let Some(a) = app.stemx.active.take() {
        a.cancel.store(true, Ordering::Relaxed);
        if a.started_separation && app.stem.separating.map(|s| s.0) == Some(a.req.track) {
            app.stop_separation();
        }
        app.toast("Stem export stopped. Nothing was added.");
    }
    app.stemx.total = 0;
    app.stemx.finished = 0;
}

pub fn busy(app: &App) -> bool {
    app.stemx.active.is_some() || !app.stemx.queue.is_empty()
}

/// Whether an export of this track is in progress or waiting.
pub fn busy_for(app: &App, track: Option<i64>) -> bool {
    let Some(t) = track else { return false };
    app.stemx.active.as_ref().is_some_and(|a| a.req.track == t) || app.stemx.queue.iter().any(|q| q.track == t)
}

/// The deck has complete stems for its track.
fn deck_stems(app: &App, id: i64) -> Option<Arc<crate::stems::Stems>> {
    if app.deck.track != Some(id) || app.stem.loading == Some(id) {
        return None;
    }
    let s = app.player.state.lock();
    let done = s.stems_ready.as_ref().is_none_or(|l| l.complete());
    if done { s.stems.clone() } else { None }
}

fn model(app: &App) -> Option<&'static crate::stems::ModelSpec> {
    app.stem.model.as_deref().or(app.prefs.stem_model.as_deref()).and_then(crate::stems::spec)
}

/// What the export in progress is doing, for the strip's status line and the footer.
pub fn status(app: &App) -> Option<String> {
    let a = app.stemx.active.as_ref()?;
    let what = a.req.part.suffix();
    let s = match (&a.rx, &a.stage) {
        (None, _) => match app.stem.separating {
            Some((id, p)) if id == a.req.track => format!("separating {:.0}%", p * 100.0),
            _ if app.stem.downloading.is_some() => "downloading the separation model".to_string(),
            _ => "waiting for the separation".to_string(),
        },
        (_, Stage::Decoding) => "reading the track".to_string(),
        (_, Stage::Separating(p)) => format!("separating {:.0}%", p * 100.0),
        (_, Stage::Writing) => "writing the file".to_string(),
        (_, Stage::Checking) => "checking the file".to_string(),
    };
    Some(format!("Exporting {what}: {s}"))
}

/// Footer text while exports run: "Exporting stems… 1 of 3 · <name>: separating 40%".
pub fn footer_text(app: &App) -> Option<String> {
    let a = app.stemx.active.as_ref()?;
    let st = status(app).unwrap_or_default();
    let st = st.split_once(": ").map(|x| x.1.to_string()).unwrap_or_default();
    Some(format!("Exporting stems… {} of {} · {}: {st}", app.stemx.finished + 1, app.stemx.total.max(1), a.name))
}

/// Each frame: start the next export, follow the one in progress, add finished files.
pub fn poll(app: &mut App) {
    if app.stemx.active.is_none() {
        let Some(req) = app.stemx.queue.pop_front() else { return };
        let Some(t) = app.track(req.track).cloned() else {
            app.stemx.finished += 1;
            return;
        };
        let name = base_name(&t.artist, &t.title, req.part);
        app.stemx.active = Some(Active { req, name, cancel: Default::default(), rx: None, stage: Stage::Decoding, started_separation: false });
    }
    let Some(a) = app.stemx.active.as_ref() else { return };
    let id = a.req.track;
    let rx = a.rx.clone();
    let Some(rx) = rx else {
        // The track on the deck uses the deck's separation (its progress shows in the strip
        // and the parts become playable); any other track is separated by the export itself.
        if app.deck.track == Some(id) {
            if deck_stems(app, id).is_some() {
                start_work(app);
            } else if app.stem.separating.map(|s| s.0) == Some(id) || app.stem.loading == Some(id) || (app.stem.downloading.is_some() && app.stem.pending == Some(id)) {
                app.ctx.request_repaint_after(std::time::Duration::from_millis(100));
            } else if app.stem.model.is_none() {
                // (The computer is still being checked for a separation model.)
                app.ctx.request_repaint_after(std::time::Duration::from_millis(200));
            } else if !app.stemx.active.as_ref().unwrap().started_separation {
                app.stemx.active.as_mut().unwrap().started_separation = true;
                app.separate_loaded();
                if app.stem.separating.is_none() && app.stem.pending.is_none() && app.stem.loading.is_none() {
                    fail(app, app.stem.error.clone().unwrap_or_else(|| "Stem separation could not start.".into()));
                }
            } else {
                // The separation ended without stems: stopped (STOP) or failed.
                match app.stem.error.clone() {
                    Some(e) => fail(app, e),
                    None => {
                        app.stemx.active = None;
                        app.stemx.queue.clear();
                        app.toast("Stem export stopped. Nothing was added.");
                    }
                }
            }
        } else {
            match model(app) {
                Some(m) if crate::stems::model_ready(m) => start_work(app),
                Some(_) if app.stem.downloading.is_some() => app.ctx.request_repaint_after(std::time::Duration::from_millis(200)),
                Some(_) if !app.stemx.active.as_ref().unwrap().started_separation => {
                    app.stemx.active.as_mut().unwrap().started_separation = true;
                    app.stems_download();
                }
                Some(_) => fail(app, app.stem.error.clone().unwrap_or_else(|| "The separation model could not be downloaded.".into())),
                None => app.ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            }
        }
        return;
    };
    while let Ok(m) = rx.try_recv() {
        match m {
            Msg::Stage(s) => {
                if let Some(a) = app.stemx.active.as_mut() {
                    a.stage = s;
                }
            }
            Msg::Done(r) => {
                let a = app.stemx.active.take().unwrap();
                app.stemx.finished += 1;
                match r {
                    Ok(w) => match import(app, &a.req, &w) {
                        Ok((nid, msg)) => {
                            app.toast(msg);
                            app.stemx.outcomes.push(Outcome { req: a.req, result: Ok((nid, w)) });
                        }
                        Err(e) => {
                            remove_finished(app, &w.path);
                            app.toast_err(format!("The exported file could not be added to the collection: {e}"));
                            app.stemx.outcomes.push(Outcome { req: a.req, result: Err(e) });
                        }
                    },
                    Err(e) if e == MSG_STOPPED => {}
                    Err(e) => {
                        app.toast_err(format!("Stem export of \"{}\" failed: {e}", a.name));
                        app.stemx.outcomes.push(Outcome { req: a.req, result: Err(e) });
                    }
                }
                return poll(app);
            }
        }
    }
}

fn fail(app: &mut App, e: String) {
    let Some(a) = app.stemx.active.take() else { return };
    app.stemx.finished += 1;
    app.toast_err(format!("Stem export of \"{}\" failed: {e}", a.name));
    app.stemx.outcomes.push(Outcome { req: a.req, result: Err(e) });
}

fn start_work(app: &mut App) {
    let a = app.stemx.active.as_ref().unwrap();
    let id = a.req.track;
    let Some(t) = app.track(id).cloned() else { return fail(app, "The track is no longer in the collection.".into()) };
    let on_deck = app.deck.track == Some(id);
    let work = Work {
        path: t.path.clone(),
        kbps: t.bitrate,
        base: a.name.clone(),
        dir: export_dir(&app.prefs.stemx),
        part: a.req.part,
        setting: app.prefs.stemx.format,
        audio: if on_deck { app.deck.audio.clone() } else { None },
        stems: if on_deck { deck_stems(app, id) } else { None },
        model: model(app),
        cache_limit: app.prefs.stem_cache_gb as u64 * 1_000_000_000,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = a.cancel.clone();
    let ctx = app.ctx.clone();
    std::thread::spawn(move || {
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        let r = run(work, &mut |s| {
            // (Separation progress is sent at most five times a second.)
            if !matches!(s, Stage::Separating(_)) || last.elapsed().as_millis() >= 200 {
                last = std::time::Instant::now();
                let _ = tx2.send(Msg::Stage(s));
                ctx2.request_repaint();
            }
        }, &cancel);
        let _ = tx.send(Msg::Done(r));
        ctx.request_repaint();
    });
    if let Some(a) = app.stemx.active.as_mut() {
        a.rx = Some(rx);
    }
}

/// The finished file joins the collection with the source's details, artwork, grid, cues,
/// phrases and My Tags, a link to its source, and (by the playlist rule) the playlist.
/// Returns the new track and the message for the status line.
pub fn import(app: &mut App, req: &Request, w: &Written) -> Result<(i64, String), String> {
    let src = app.track(req.track).cloned().ok_or("the source track is no longer in the collection")?;
    let p = w.path.to_string_lossy().to_string();
    let id = match app.db.track_by_path(&p) {
        Some(id) => id,
        None => app.db.insert_track(&crate::importer::read_track(&w.path)).map_err(|e| e.to_string())?,
    };
    let shift = w.shift;
    if let Some(mut g) = app.db.grid(src.id).filter(|g| !g.is_empty()) {
        for a in g.anchors.iter_mut() {
            a.time += shift;
        }
        let _ = app.db.save_grid(id, &g);
    }
    let cues: Vec<Cue> = app.db.cues(src.id).unwrap_or_default().into_iter().map(|c| Cue { id: 0, track_id: id, start: c.start + shift, end: c.end.map(|e| e + shift), ..c }).collect();
    let _ = app.db.replace_cues(id, &cues);
    // Phrases are counted in beats of the grid, which moved with everything else.
    let phrases = app.stored_phrases(src.id).or_else(|| if app.deck.track == Some(src.id) { app.deck.phrases.clone() } else { None }).or_else(|| rb_phrases(app, &src));
    if let Some(st) = phrases {
        app.store_phrases(id, Some(st));
    }
    app.db.set_setting(&link_key(id), &format!("{}:{}", src.id, req.part.key()));
    made_set(app);
    if let Some(m) = app.stemx.made.as_mut() {
        m.insert(id);
    }
    crate::ui_edit::edited_set(app);
    app.edit.edited.get_or_insert_with(Default::default).insert(id);
    if let Some(mut t) = app.db.track(id) {
        t.title = format!("{} ({})", if src.title.trim().is_empty() { "Untitled" } else { src.title.trim() }, req.part.suffix());
        t.artist = src.artist.clone();
        t.bpm = src.bpm;
        t.album = src.album.clone();
        t.album_artist = src.album_artist.clone();
        t.genre = src.genre.clone();
        t.label = src.label.clone();
        t.remixer = src.remixer.clone();
        t.original_artist = src.original_artist.clone();
        t.composer = src.composer.clone();
        t.lyricist = src.lyricist.clone();
        t.mix_name = src.mix_name.clone();
        t.comment = src.comment.clone();
        t.grouping = src.grouping.clone();
        t.message = src.message.clone();
        t.year = src.year;
        t.track_no = src.track_no;
        t.disc_no = src.disc_no;
        t.release_date = src.release_date.clone();
        t.key = src.key;
        t.key_text = src.key_text.clone();
        t.key_manual = src.key_manual;
        t.rating = src.rating;
        t.color = src.color;
        t.artwork = src.artwork.clone();
        let _ = app.db.update_track(&t);
    }
    for tag in app.track_tags.get(&src.id).cloned().unwrap_or_default() {
        let _ = app.db.set_track_tag(id, tag, true);
    }
    app.reload_tracks();
    app.reload_tags();
    app.refresh_cue_counts(id);
    let mut linked = app.matching_of(src.id);
    linked.push(src.id);
    for other in linked {
        if other != id {
            app.matching_set(other, id, true);
        }
    }
    // Waveform, gain and vocal levels from the new audio (the copied grid and key are kept).
    app.analyze(&[id], false);
    let file = w.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut msg = format!("Exported \"{file}\" ({}) and added it to the collection", w.plan.describe());
    if let Some(pl) = req.view_playlist.filter(|_| req.to_playlist) {
        if app.prefs.rb.library_protection {
            msg.push_str("; not added to the playlist because Library Protection is on");
        } else if let Some(name) = app.node(pl).filter(|n| n.kind == NodeKind::Playlist).map(|n| n.name.clone()) {
            let mut cur = app.db.playlist_tracks(pl).unwrap_or_default();
            let made = made_set(app).clone();
            let at = insert_at(&cur, src.id, |t| made.contains(&t) && t != id && app.db.setting(&link_key(t)).is_some_and(|v| v.starts_with(&format!("{}:", src.id))));
            cur.insert(at, id);
            let _ = app.db.set_playlist_tracks(pl, &cur);
            app.reload_playlists();
            msg.push_str(&format!(" and to \"{name}\""));
        }
    }
    if let Some(f) = &w.plan.fallback {
        msg.push_str(&format!(" ({f})"));
    }
    msg.push('.');
    Ok((id, msg))
}

/// rekordbox's own phrases of the source when its grid is the one rekordbox analysed.
fn rb_phrases(app: &App, src: &Track) -> Option<crate::phrase::Structure> {
    let map: std::collections::HashMap<String, String> = app.db.setting("rb_anlz").and_then(|s| serde_json::from_str(&s).ok())?;
    let base = PathBuf::from(map.get(&src.path)?);
    crate::export::anlz::read_sections(&base.with_extension("EXT")).get("PSSI").and_then(|s| crate::phrase::parse(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stems_with_vocal(len: usize, vocal: impl Fn(usize) -> (f32, f32)) -> crate::stems::Stems {
        let mut data = vec![0.0f32; 8 * len];
        for i in 0..len {
            let (l, r) = vocal(i);
            data[6 * len + i] = l;
            data[7 * len + i] = r;
            // (The other parts are filled to show they are not used.)
            data[i] = 0.3;
            data[2 * len + i] = -0.2;
        }
        crate::stems::Stems { len, data }
    }

    fn signal(frames: usize, sr: u32) -> Vec<f32> {
        let mut seed = 7u32;
        (0..frames)
            .flat_map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let t = i as f32 / sr as f32;
                let nz = ((seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5) * 0.05;
                [0.4 * (t * 220.0 * 6.283).sin() + nz, 0.35 * (t * 331.0 * 6.283).sin() - nz]
            })
            .collect()
    }

    #[test]
    fn names_and_numbering() {
        assert_eq!(base_name("Artist", "Song", Part::Vocals), "Artist - Song (Vocals)");
        assert_eq!(base_name("", "Song", Part::Instrumental), "Song (Instrumental)");
        assert_eq!(base_name("A/B", "x:y", Part::Vocals), "A_B - x_y (Vocals)");
        assert_eq!(base_name(" ", " ", Part::Vocals), "Untitled (Vocals)");
        let dir = std::env::temp_dir().join(format!("recordbox_stemx_names_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = free_path(&dir, "A - S (Vocals)", "wav");
        assert_eq!(a.file_name().unwrap(), "A - S (Vocals).wav");
        std::fs::write(&a, b"x").unwrap();
        let b = free_path(&dir, "A - S (Vocals)", "wav");
        assert_eq!(b.file_name().unwrap(), "A - S (Vocals) 2.wav");
        std::fs::write(&b, b"x").unwrap();
        assert_eq!(free_path(&dir, "A - S (Vocals)", "wav").file_name().unwrap(), "A - S (Vocals) 3.wav");
        // Another format does not count as taken.
        assert_eq!(free_path(&dir, "A - S (Vocals)", "flac").file_name().unwrap(), "A - S (Vocals).flac");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instrumental_is_the_original_less_the_vocal_and_the_two_add_up() {
        for sr in [44100u32, 48000] {
            let frames = sr as usize * 2 + 123;
            let orig = signal(frames, sr);
            let len = (frames as f64 * 44100.0 / sr as f64).round() as usize;
            let st = stems_with_vocal(len, |i| (0.2 * (i as f32 * 0.01).sin(), 0.1 * (i as f32 * 0.013).cos()));
            let v = render_part(&orig, sr, &st, Part::Vocals);
            let ins = render_part(&orig, sr, &st, Part::Instrumental);
            // Sample-aligned and as long as the original.
            assert_eq!(v.len(), orig.len());
            assert_eq!(ins.len(), orig.len());
            let worst = v.iter().zip(&ins).zip(&orig).map(|((a, b), o)| (a + b - o).abs()).fold(0.0f32, f32::max);
            assert!(worst < 1e-6, "{sr}: vocal + instrumental differs from the original by {worst}");
            // At 44.1 kHz the vocal is the model's vocal part sample for sample.
            if sr == 44100 {
                assert_eq!(v[2 * 100], st.data[6 * len + 100]);
                assert_eq!(v[2 * 100 + 1], st.data[7 * len + 100]);
            }
            // Through 16-bit files: within a step of rounding.
            let (qv, qi, qo) = (quantize(&v, 2, 16), quantize(&ins, 2, 16), quantize(&orig, 2, 16));
            let worst = qv.iter().zip(&qi).zip(&qo).map(|((a, b), o)| (a + b - o).abs()).max().unwrap();
            assert!(worst <= 1, "{sr}: {worst} steps");
        }
        // Silent stems: the instrumental is the original exactly, also as 24-bit integers.
        let orig = signal(5000, 44100);
        let st = stems_with_vocal(5000, |_| (0.0, 0.0));
        assert_eq!(quantize(&render_part(&orig, 44100, &st, Part::Instrumental), 2, 24), quantize(&orig, 2, 24));
        // Mono output averages the sides.
        assert_eq!(quantize(&[0.5, 0.25], 1, 16), vec![(0.375f32 * 32768.0).round() as i32]);
    }

    fn src(kind: Kind, bits: u16, kbps: u32, vbr: bool) -> SourceInfo {
        SourceInfo { kind, rate: 44100, ch: 2, bits, kbps, vbr }
    }

    #[test]
    fn the_format_table() {
        let d = |s: &SourceInfo| {
            let p = plan(0, s);
            (p.name, p.describe(), p.fallback.is_some(), p.rate)
        };
        assert_eq!(d(&src(Kind::Wav, 16, 0, false)), ("WAV", "WAV 16-bit".into(), false, 44100));
        assert_eq!(d(&src(Kind::Wav, 24, 0, false)), ("WAV", "WAV 24-bit".into(), false, 44100));
        assert_eq!(d(&src(Kind::Wav, 32, 0, false)), ("WAV", "WAV 24-bit".into(), false, 44100));
        assert_eq!(d(&src(Kind::Aiff, 16, 0, false)), ("AIFF", "AIFF 16-bit".into(), false, 44100));
        assert_eq!(d(&src(Kind::Flac, 24, 0, false)), ("FLAC", "FLAC 24-bit".into(), false, 44100));
        assert_eq!(d(&src(Kind::Mp3, 0, 320, false)), ("MP3", "MP3 320 kbps".into(), false, 44100));
        assert_eq!(d(&src(Kind::Mp3, 0, 192, false)), ("MP3", "MP3 192 kbps".into(), false, 44100));
        assert_eq!(d(&src(Kind::Mp3, 0, 250, true)), ("MP3", "MP3 VBR V0".into(), false, 44100));
        assert_eq!(d(&src(Kind::Mp3, 0, 200, true)), ("MP3", "MP3 VBR V2".into(), false, 44100));
        assert_eq!(d(&src(Kind::Aac, 0, 256, false)), ("M4A", "M4A (AAC) 256 kbps".into(), false, 44100));
        assert_eq!(d(&src(Kind::Alac, 16, 0, false)), ("FLAC", "FLAC 16-bit".into(), true, 44100));
        assert_eq!(d(&src(Kind::OtherLossy, 0, 160, false)), ("M4A", "M4A (AAC) 160 kbps".into(), true, 44100));
        assert_eq!(d(&src(Kind::OtherLossless, 24, 0, false)), ("FLAC", "FLAC 24-bit".into(), true, 44100));
        // Never above the original's bitrate.
        assert_eq!(cbr_at_most(319), 320);
        assert_eq!(cbr_at_most(300), 256);
        assert_eq!(cbr_at_most(128), 128);
        assert!(VBR_TYPICAL[vbr_quality(180) as usize] <= 180);
        // Manual settings: lossless keeps the original's depth, lossy outputs are 16-bit.
        let flac24 = src(Kind::Flac, 24, 0, false);
        let mp3 = src(Kind::Mp3, 0, 320, false);
        for (setting, s, want) in [
            (1u8, &flac24, "WAV 24-bit"),
            (1, &mp3, "WAV 16-bit"),
            (2, &mp3, "FLAC 16-bit"),
            (2, &flac24, "FLAC 24-bit"),
            (3, &flac24, "AIFF 24-bit"),
            (4, &flac24, "M4A (AAC) 256 kbps"),
            (4, &mp3, "M4A (AAC) 320 kbps"),
            (5, &flac24, "MP3 320 kbps"),
            (5, &mp3, "MP3 320 kbps"),
            (5, &src(Kind::Aac, 0, 128, false), "MP3 128 kbps"),
        ] {
            let p = plan(setting, s);
            assert_eq!(p.describe(), want, "setting {setting} for {:?}", s.kind);
            assert!(p.fallback.is_none());
        }
        // Rates lossy formats cannot take; channel count kept.
        let mut hi = src(Kind::Flac, 24, 0, false);
        hi.rate = 96000;
        hi.ch = 1;
        assert_eq!((plan(5, &hi).rate, plan(5, &hi).ch, plan(0, &hi).rate), (48000, 1, 96000));
        hi.rate = 88200;
        assert_eq!(plan(4, &hi).rate, 44100);
    }

    #[test]
    fn mp3_headers_tell_constant_from_variable() {
        // 320 kbps 44.1 kHz frames (1044 bytes), the first an Info / Xing tag frame.
        let fr = |tag: Option<&[u8; 4]>, b2: u8| {
            let len = 144 * if b2 == 0xE0 { 320 } else { 128 } * 1000 / 44100;
            let mut f = vec![0u8; len];
            f[..4].copy_from_slice(&[0xFF, 0xFB, b2, 0x44]);
            if let Some(t) = tag {
                f[36..40].copy_from_slice(t);
            }
            f
        };
        let mut cbr = fr(Some(b"Info"), 0x90);
        cbr.extend(fr(None, 0xE0));
        assert_eq!(mp3_rate_info_in(&cbr), Some((320, false)));
        let mut vbr = fr(Some(b"Xing"), 0x90);
        vbr.extend(fr(None, 0xE0));
        assert_eq!(mp3_rate_info_in(&vbr), Some((320, true)));
        assert_eq!(mp3_rate_info_in(&fr(None, 0x90)), Some((128, false)));
    }

    #[test]
    fn the_playlist_rule_for_each_kind_of_view() {
        let node = |id, kind| PlaylistNode { id, parent: 0, name: format!("n{id}"), kind, seq: 0, smart: None };
        let nodes = vec![node(1, NodeKind::Playlist), node(2, NodeKind::Folder), node(3, NodeKind::Smart)];
        assert_eq!(view_playlist(&View::Playlist(1), &nodes), Some(1));
        assert_eq!(view_playlist(&View::Playlist(2), &nodes), None);
        assert_eq!(view_playlist(&View::Playlist(3), &nodes), None);
        assert_eq!(view_playlist(&View::Playlist(99), &nodes), None);
        for v in [
            View::Collection,
            View::History(4),
            View::TagList,
            View::Related,
            View::Matching,
            View::Suggest(0),
            View::Explorer("/x".into()),
            View::Category { field: 1, value: "House".into() },
            View::Device { root: "/Volumes/X".into(), node: None },
            View::DeviceHistory { root: "/Volumes/X".into(), id: 1 },
            View::Source { kind: 0, node: String::new() },
            View::Sampler(-1),
        ] {
            assert_eq!(view_playlist(&v, &nodes), None, "{v:?}");
        }
        // Right after the source, after its stem tracks already there, else at the end.
        assert_eq!(insert_at(&[10, 20, 30], 20, |_| false), 2);
        assert_eq!(insert_at(&[10, 20, 21, 30], 20, |t| t == 21), 3);
        assert_eq!(insert_at(&[10, 30], 20, |_| false), 2);
        assert_eq!(insert_at(&[], 20, |_| false), 0);
    }

    #[test]
    fn the_first_export_asks_once_and_cancel_starts_nothing() {
        let mut p = StemExportPrefs::default();
        assert!(asks(&p) && p.to_playlist && p.format == 0);
        let r = Request { track: 1, part: Part::Vocals, view_playlist: Some(5), to_playlist: false, from_strip: true };
        let mut a = Ask { reqs: vec![r], playlist: Some((5, "Set".into())), add: false, dont_show: true };
        // Cancel: nothing saved, nothing started, the next export asks again.
        assert!(!answer(&mut p, &a, false));
        assert_eq!(p, StemExportPrefs::default());
        // OK with the box unticked and "Don't show this again": saved, not asked again.
        assert!(answer(&mut p, &a, true));
        assert!(!p.to_playlist && !asks(&p));
        // Outside a playlist the playlist setting is not touched; without "Don't show this
        // again" the explanation comes back next time.
        let mut p = StemExportPrefs::default();
        a.playlist = None;
        a.dont_show = false;
        assert!(answer(&mut p, &a, true));
        assert!(p.to_playlist && asks(&p));
    }

    #[test]
    fn settings_round_trip() {
        let p = StemExportPrefs { to_playlist: false, format: 5, dir: "/tmp/x".into(), explain: false };
        let back: StemExportPrefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        // Older preferences without these fields get the defaults.
        let back: StemExportPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(back, StemExportPrefs::default());
        let prefs: crate::app::Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.stemx, StemExportPrefs::default());
        // A folder inside rekordbox's is never used.
        if let Some(h) = dirs::home_dir() {
            let p = StemExportPrefs { dir: h.join("Music/rekordbox/x").to_string_lossy().to_string(), ..Default::default() };
            assert_eq!(export_dir(&p), default_dir());
        }
    }

    /// A run with given audio and stems into `dir` (lossless or lossy, by `setting`).
    fn run_with(dir: &Path, orig: Vec<f32>, sr: u32, stems: crate::stems::Stems, part: Part, setting: u8, cancel: &AtomicBool) -> Result<Written, String> {
        let audio = crate::audio::Decoded { sr, samples: orig, raw_frames: 0, damaged: 0, truncated: false };
        let w = Work { path: dir.join("missing-source.wav").to_string_lossy().to_string(), kbps: 0, base: "A - T (Instrumental)".into(), dir: dir.to_path_buf(), part, setting, audio: Some(Arc::new(audio)), stems: Some(Arc::new(stems)), model: None, cache_limit: 0 };
        run(w, &mut |_| {}, cancel)
    }

    #[test]
    fn a_lossless_export_is_sample_aligned_and_cancel_leaves_nothing() {
        let dir = std::env::temp_dir().join(format!("recordbox_stemx_run_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let frames = 44100 * 3 + 77;
        let orig = signal(frames, 44100);
        let st = stems_with_vocal(frames, |i| (0.1 * (i as f32 * 0.02).sin(), 0.0));
        let w = run_with(&dir, orig.clone(), 44100, st, Part::Instrumental, 1, &AtomicBool::new(false)).unwrap();
        assert_eq!(w.path.file_name().unwrap(), "A - T (Instrumental).wav");
        assert_eq!((w.frames, w.src_frames, w.offset, w.dropped, w.shift), (frames, frames, Some(0), 0, 0.0));
        let back = crate::audio::decode_file(&w.path.to_string_lossy()).unwrap();
        let want = quantize(&render_part(&orig, 44100, &stems_with_vocal(frames, |i| (0.1 * (i as f32 * 0.02).sin(), 0.0)), Part::Instrumental), 2, 16);
        let got: Vec<i32> = back.samples.iter().map(|v| (v * 32768.0).round() as i32).collect();
        assert_eq!(got, want);
        // A second export is numbered, the first is untouched.
        let st = stems_with_vocal(frames, |_| (0.0, 0.0));
        let w2 = run_with(&dir, orig.clone(), 44100, st, Part::Instrumental, 1, &AtomicBool::new(false)).unwrap();
        assert_eq!(w2.path.file_name().unwrap(), "A - T (Instrumental) 2.wav");
        // Cancelled: no file, no temporary file.
        let st = stems_with_vocal(frames, |_| (0.0, 0.0));
        let r = run_with(&dir, orig, 44100, st, Part::Instrumental, 1, &AtomicBool::new(true));
        assert_eq!(r.unwrap_err(), MSG_STOPPED);
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The source file is only read: a cancelled, a failed and a finished export leave its
    /// bytes and modification time as they were and put nothing beside it (unless its folder
    /// is the one chosen for exports); temporary files go.
    #[test]
    fn the_source_file_is_never_touched() {
        let root = std::env::temp_dir().join(format!("recordbox_stemx_src_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let music = root.join("music");
        let out = root.join("exports");
        std::fs::create_dir_all(&music).unwrap();
        let frames = 44100 * 2;
        let src = music.join("Artist - Song.wav");
        std::fs::write(&src, render::wav_bytes_n(&quantize(&signal(frames, 44100), 2, 16), 2, 16, 44100)).unwrap();
        let bytes = std::fs::read(&src).unwrap();
        let mtime = std::fs::metadata(&src).unwrap().modified().unwrap();
        let listing = |d: &Path| -> Vec<String> { let mut v: Vec<String> = std::fs::read_dir(d).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default(); v.sort(); v };
        let work = |dir: &Path, setting: u8| Work { path: src.to_string_lossy().to_string(), kbps: 0, base: "Artist - Song (Instrumental)".into(), dir: dir.to_path_buf(), part: Part::Instrumental, setting, audio: None, stems: Some(Arc::new(stems_with_vocal(frames, |i| (0.05 * (i as f32 * 0.03).sin(), 0.0)))), model: None, cache_limit: 0 };
        let same = || {
            assert_eq!(std::fs::read(&src).unwrap(), bytes, "source bytes changed");
            assert_eq!(std::fs::metadata(&src).unwrap().modified().unwrap(), mtime, "source modification time changed");
            assert_eq!(listing(&music), vec!["Artist - Song.wav".to_string()]);
        };
        // Cancelled.
        assert_eq!(run(work(&out, 0), &mut |_| {}, &AtomicBool::new(true)).unwrap_err(), MSG_STOPPED);
        same();
        assert!(listing(&out).is_empty(), "{:?}", listing(&out));
        // Failed: the export folder cannot be made (a file is in its way).
        let blocked = root.join("blocked");
        std::fs::write(&blocked, b"x").unwrap();
        assert!(run(work(&blocked.join("sub"), 0), &mut |_| {}, &AtomicBool::new(false)).is_err());
        same();
        // Cancelled while writing a lossy file (the probe and the temporary file go).
        let flag = AtomicBool::new(false);
        let r = run(work(&out, 5), &mut |s| if s == Stage::Writing { flag.store(true, Ordering::Relaxed) }, &flag);
        assert_eq!(r.unwrap_err(), MSG_STOPPED);
        same();
        assert!(listing(&out).is_empty(), "{:?}", listing(&out));
        // Finished: the new file is in the export folder only.
        let w = run(work(&out, 0), &mut |_| {}, &AtomicBool::new(false)).unwrap();
        same();
        assert_eq!(listing(&out), vec!["Artist - Song (Instrumental).wav".to_string()]);
        assert_eq!(w.path.parent(), Some(out.as_path()));
        // The source's own folder chosen for exports: a new file beside it, the source untouched.
        let w = run(work(&music, 0), &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), bytes);
        assert_eq!(std::fs::metadata(&src).unwrap().modified().unwrap(), mtime);
        assert_eq!(listing(&music), vec!["Artist - Song (Instrumental).wav".to_string(), "Artist - Song.wav".to_string()]);
        assert_ne!(w.path, src);
        // Only temporary files of the export are ever removed.
        assert!(!is_temp_in(&music, &src));
        assert!(!is_temp_in(&out, &out.join(".recordbox-stem-1-2.partial.wav").with_file_name("x.wav")));
        assert!(is_temp_in(&out, &out.join(".recordbox-stem-1-2.partial.wav")));
        assert!(!is_temp_in(&music, &out.join(".recordbox-stem-1-2.partial.wav")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A click at a known sample of the original is at the same place on the exported file's
    /// timeline (as the app reads it) after the lossy encoder's lead is made up for: by
    /// dropping the silent lead-in, or, when the start is not silent, by moving the copied
    /// grid (`shift`).
    #[test]
    fn lossy_exports_keep_the_timeline() {
        let dir = std::env::temp_dir().join(format!("recordbox_stemx_lossy_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sr = 44100u32;
        let frames = sr as usize * 4;
        let click = 70_000usize;
        for (setting, name) in [(5u8, "MP3"), (4, "M4A")] {
            if setting == 4 && !Path::new("/usr/bin/afconvert").exists() {
                continue;
            }
            for loud_start in [false, true] {
                // Quiet music with a click, and (second case) a tone from the first sample.
                let mut orig = vec![0.0f32; frames * 2];
                let mut seed = 99u32;
                for i in 0..frames {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let nz = ((seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5) * 0.1;
                    let v = nz * if loud_start || i > 20_000 { 1.0 } else { 0.0 };
                    orig[i * 2] = v;
                    orig[i * 2 + 1] = v;
                }
                for k in 0..4 {
                    orig[(click + k) * 2] = 0.9;
                    orig[(click + k) * 2 + 1] = 0.9;
                }
                let st = stems_with_vocal(frames, |_| (0.0, 0.0));
                let w = run_with(&dir, orig.clone(), sr, st, Part::Instrumental, setting, &AtomicBool::new(false)).unwrap();
                let back = crate::audio::decode_file(&w.path.to_string_lossy()).unwrap();
                let m = mono(&back.samples);
                let at = m.iter().enumerate().skip(click - 3000).take(6000).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0;
                // Where the click is on the copied timeline: the file's position less the shift.
                let on_timeline = at as f64 - w.shift * sr as f64;
                eprintln!("{name} loud start {loud_start}: dropped {} shift {:.5} s offset {:?}, click at {at} (timeline {on_timeline}), source {click}", w.dropped, w.shift, w.offset);
                assert!((on_timeline - click as f64).abs() <= 4.0, "{name}: click at {on_timeline}, not {click}");
                assert_eq!(w.offset.unwrap_or(99) as f64, (w.shift * sr as f64).round(), "{name}: measured offset and shift agree");
                if loud_start {
                    assert_eq!(w.dropped, 0);
                    assert!(w.shift > 0.0);
                } else {
                    assert!(w.dropped > 1000, "{name}: the silent lead-in is dropped");
                    assert_eq!(w.offset, Some(0));
                    assert_eq!(w.shift, 0.0);
                }
                // A beat grid copied with the shift has its first beat on the same transient.
                let g = BeatGrid::constant(click as f64 / sr as f64, 120.0, 1);
                let moved = g.anchors[0].time + w.shift;
                assert!(((moved * sr as f64) - at as f64).abs() <= 4.0);
                let _ = std::fs::remove_file(&w.path);
            }
        }
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert!(left.is_empty(), "probe and temporary files are removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mp3_files_have_the_info_tag_and_decode_to_the_length() {
        let dir = std::env::temp_dir().join(format!("recordbox_stemx_mp3_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let x = signal(44100 * 2, 44100);
        for r in [Mp3Rate::Cbr(320), Mp3Rate::Vbr(2)] {
            let bytes = render::mp3_bytes(&quantize(&x, 2, 16).iter().map(|v| *v as i16).collect::<Vec<_>>(), 2, 44100, r).unwrap();
            let tagged = bytes[..2000].windows(4).any(|w| w == b"Info" || w == b"Xing");
            assert!(tagged, "{r:?}: no Info/Xing frame");
            let (kbps, vbr) = mp3_rate_info_in(&bytes).unwrap();
            assert_eq!(vbr, matches!(r, Mp3Rate::Vbr(_)), "{r:?}");
            if r == Mp3Rate::Cbr(320) {
                assert_eq!(kbps, 320);
            }
            let f = dir.join("a.mp3");
            std::fs::write(&f, &bytes).unwrap();
            let d = crate::audio::decode_file(&f.to_string_lossy()).unwrap();
            assert_eq!(d.sr, 44100);
            // Nothing is cut at the end: the lead (2257 frames as the app reads it) plus the
            // whole input, rounded up to a frame.
            eprintln!("{r:?}: {} frames read back for 88200 written", d.frames());
            assert!(d.frames() >= 88200 + 2257 && d.frames() < 88200 + 2257 + 1152, "{r:?}: {} frames", d.frames());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lossless_encoders_keep_24_bit_and_mono() {
        let dir = std::env::temp_dir().join(format!("recordbox_stemx_24_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let x = signal(30_000, 48000);
        for ch in [1u16, 2] {
            let pcm = quantize(&x, ch, 24);
            for (fmt, ext) in [(0u8, "flac"), (1, "wav"), (2, "aiff")] {
                let f = dir.join(format!("a{ch}.{ext}"));
                render::write_n(&f, &pcm, ch, 48000, &Encode { format: fmt, bits: 24, flac_level: 5, aac_bps: 0, mp3: Mp3Rate::Cbr(320) }).unwrap();
                let info = probe_source(&f.to_string_lossy(), 0).unwrap();
                assert_eq!((info.ch, info.rate), (ch, 48000), "{ext} {ch}");
                assert_eq!(info.bits, 24, "{ext} {ch}");
                let d = crate::audio::decode_file(&f.to_string_lossy()).unwrap();
                let got: Vec<i32> = d.samples.chunks_exact(2).flat_map(|c| if ch == 1 { vec![(c[0] * 8_388_608.0).round() as i32] } else { vec![(c[0] * 8_388_608.0).round() as i32, (c[1] * 8_388_608.0).round() as i32] }).collect();
                assert_eq!(got.len(), pcm.len(), "{ext} {ch}");
                assert!(got == pcm, "{ext} {ch}-channel 24-bit is not lossless");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod probe {
    /// `PREPDECK_PROBE_FILE=<audio> cargo test --release stemx_head_probe -- --ignored --nocapture`:
    /// the peak of the first frames of a file as the app reads it (read only).
    #[test]
    #[ignore]
    fn stemx_head_probe() {
        let Ok(p) = std::env::var("PREPDECK_PROBE_FILE") else { return };
        let d = crate::audio::decode_file(&p).unwrap();
        let pk = |a: usize, b: usize| d.samples[a * 2..b * 2].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        eprintln!("class {:?} rb lead {} | peak 0..1152 {:.5}, 1152..1681 {:.5}, 1681..2257 {:.5}, 2257..4410 {:.5}", crate::audio::mp3_timing_class(&p), crate::analysis::mp3_timebase::lead_frames(&p), pk(0, 1152), pk(1152, 1681), pk(1681, 2257), pk(2257, 4410));
    }

    /// `PREPDECK_PROBE_SUM=<original>|<vocals file>|<instrumental file>`: how far vocals +
    /// instrumental, as the app reads them, are from the original (read only).
    #[test]
    #[ignore]
    fn stemx_sum_probe() {
        let Ok(v) = std::env::var("PREPDECK_PROBE_SUM") else { return };
        let f: Vec<&str> = v.split('|').collect();
        let d: Vec<crate::audio::Decoded> = f.iter().map(|p| crate::audio::decode_file(p).unwrap()).collect();
        let n = d.iter().map(|x| x.samples.len()).min().unwrap();
        let (mut e_o, mut e_r, mut e_v, mut e_i) = (0f64, 0f64, 0f64, 0f64);
        for k in 0..n {
            let (o, a, b) = (d[0].samples[k] as f64, d[1].samples[k] as f64, d[2].samples[k] as f64);
            e_o += o * o;
            e_v += a * a;
            e_i += b * b;
            e_r += (a + b - o) * (a + b - o);
        }
        let db = |e: f64| 10.0 * (e / n as f64 + 1e-20).log10();
        eprintln!("frames {} {} {} | original {:.2} dB, vocals {:.2} dB, instrumental {:.2} dB, vocals + instrumental - original {:.2} dB ({:.2} dB below the original)", d[0].frames(), d[1].frames(), d[2].frames(), db(e_o), db(e_v), db(e_i), db(e_r), db(e_o) - db(e_r));
    }
}
