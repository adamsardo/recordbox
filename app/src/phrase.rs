//! Phrase analysis (song structure) for tracks rekordbox has not analysed, and the PSSI section
//! the players read. Layout and the RB6+ XOR mask follow rekordcrate (src/anlz.rs); the
//! segmentation and labels are PrepDeck's own, tuned against rekordbox's phrases for the
//! user's library (rekordbox's models are not used).

use crate::model::Beat;

/// High-mood phrase kinds (the mood rekordbox gives most dance music).
pub const INTRO: u16 = 1;
pub const UP: u16 = 2;
pub const DOWN: u16 = 3;
pub const CHORUS: u16 = 5;
pub const OUTRO: u16 = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct Phrase {
    /// 1-based beat number (index into the track's beats) where the phrase starts.
    pub beat: u16,
    pub kind: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Structure {
    /// 1 High, 2 Mid, 3 Low.
    pub mood: u16,
    pub phrases: Vec<Phrase>,
    /// Beat number where the last phrase ends.
    pub end_beat: u16,
}

/// Per-bar log energies in three bands (low < 200 Hz, mid, high > 2.5 kHz).
fn bar_features(mono: &[f32], sr: f64, bar_times: &[f64], end_time: f64) -> Vec<[f64; 3]> {
    let mut lp = crate::analysis::Biquad::new("lp", 200.0, 0.707, sr, 0.0);
    let mut hp = crate::analysis::Biquad::new("hp", 2500.0, 0.707, sr, 0.0);
    let n = mono.len();
    let mut idx = 0usize;
    let mut out = Vec::with_capacity(bar_times.len());
    for (k, t) in bar_times.iter().enumerate() {
        let start = ((*t * sr) as usize).min(n);
        let end = ((bar_times.get(k + 1).copied().unwrap_or(end_time)) * sr) as usize;
        let end = end.min(n);
        while idx < start {
            lp.run(mono[idx] as f64);
            hp.run(mono[idx] as f64);
            idx += 1;
        }
        let mut e = [0f64; 3];
        let mut c = 0f64;
        while idx < end {
            let x = mono[idx] as f64;
            let l = lp.run(x);
            let h = hp.run(x);
            e[0] += l * l;
            e[1] += (x - l - h).powi(2);
            e[2] += h * h;
            c += 1.0;
            idx += 1;
        }
        let c = c.max(1.0);
        out.push([10.0 * (e[0] / c + 1e-10).log10(), 10.0 * (e[1] / c + 1e-10).log10(), 10.0 * (e[2] / c + 1e-10).log10()]);
    }
    out
}

/// Analyse the phrase structure of a track from its audio and beat grid.
pub fn analyze(mono: &[f32], sr: f64, beats: &[Beat]) -> Option<Structure> {
    if beats.len() < 64 {
        return None;
    }
    let end_time = mono.len() as f64 / sr;
    // Bars start at beats numbered 1. Beats before the first bar start belong to phrase 1.
    let bar_idx: Vec<usize> = beats.iter().enumerate().filter(|(_, b)| b.beat == 1).map(|(i, _)| i).collect();
    if bar_idx.len() < 8 {
        return None;
    }
    let bar_times: Vec<f64> = bar_idx.iter().map(|i| beats[*i].time).collect();
    let f = bar_features(mono, sr, &bar_times, end_time);
    let nb = f.len();
    // Novelty between consecutive 4-bar blocks at every bar.
    let mean = |a: usize, b: usize, band: usize| -> f64 { f[a..b].iter().map(|x| x[band]).sum::<f64>() / (b - a).max(1) as f64 };
    let nov = |i: usize| -> f64 {
        if i < 4 || i + 4 > nb {
            return 0.0;
        }
        (0..3).map(|b| (mean(i, i + 4, b) - mean(i - 4, i, b)).powi(2)).sum::<f64>().sqrt()
    };
    // Boundaries: every 8 bars from the first bar; a 4-bar boundary in between when the music
    // changes clearly there; a final short phrase is merged into the previous one.
    let threshold = params().0;
    let mut cuts: Vec<usize> = vec![0];
    let mut last = 0usize;
    let mut i = 4;
    while i < nb {
        let since = i - last;
        if since >= max_bars() || (since >= 4 && nov(i) > threshold) {
            cuts.push(i);
            last = i;
        }
        i += 4;
    }
    if nb - last < 2 && cuts.len() > 1 {
        cuts.pop();
    }
    // Label each phrase from its low-band (kick/bass) and overall energy.
    let seg: Vec<(usize, usize)> = cuts.iter().enumerate().map(|(k, a)| (*a, cuts.get(k + 1).copied().unwrap_or(nb))).collect();
    let low: Vec<f64> = seg.iter().map(|(a, b)| mean(*a, *b, 0)).collect();
    let tot: Vec<f64> = seg.iter().map(|(a, b)| (mean(*a, *b, 0) + mean(*a, *b, 1) + mean(*a, *b, 2)) / 3.0).collect();
    let low_max = low.iter().cloned().fold(f64::MIN, f64::max);
    let tot_max = tot.iter().cloned().fold(f64::MIN, f64::max);
    let (_, down_db, up_db) = params();
    let mut kinds: Vec<u16> = seg
        .iter()
        .enumerate()
        .map(|(k, _)| {
            if low[k] < low_max - down_db {
                DOWN
            } else if tot[k] < tot_max - up_db {
                UP
            } else {
                CHORUS
            }
        })
        .collect();
    // A quieter phrase right before a chorus builds up to it.
    for k in 0..kinds.len().saturating_sub(1) {
        if kinds[k] == DOWN && kinds[k + 1] == CHORUS && low[k] >= low_max - down_db * 1.5 {
            kinds[k] = UP;
        }
    }
    let n = kinds.len();
    kinds[0] = INTRO;
    if n > 2 {
        kinds[n - 1] = OUTRO;
    }
    let phrases = seg
        .iter()
        .zip(&kinds)
        .enumerate()
        .map(|(k, ((a, _), kind))| Phrase { beat: if k == 0 { 1 } else { (bar_idx[*a] + 1) as u16 }, kind: *kind })
        .collect();
    Some(Structure { mood: 1, phrases, end_beat: beats.len().min(u16::MAX as usize) as u16 })
}

fn max_bars() -> usize {
    std::env::var("PREPDECK_PHRASE_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(8)
}

fn params() -> (f64, f64, f64) {
    let g = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    // Tuned against rekordbox's phrases for 99 tracks: 74 % per-beat label agreement (all
    // "Chorus" scores 57 %), boundary F1 69.5 %.
    (g("PREPDECK_PHRASE_NOV", 8.0), g("PREPDECK_PHRASE_DOWN", 18.0), g("PREPDECK_PHRASE_UP", 2.5))
}

const KEY: [u8; 19] = [0xCB, 0xE1, 0xEE, 0xFA, 0xE5, 0xEE, 0xAD, 0xEE, 0xE9, 0xD2, 0xE9, 0xEB, 0xE1, 0xE9, 0xF3, 0xE8, 0xE9, 0xF4, 0xE1];

/// XOR the data part of a PSSI section (everything after the entry count) with rekordbox 6's
/// mask. Applying it twice restores the original.
pub fn mask_section(section: &mut [u8]) {
    if section.len() < 18 || &section[..4] != b"PSSI" {
        return;
    }
    let n = u16::from_be_bytes([section[16], section[17]]);
    for (i, b) in section[18..].iter_mut().enumerate() {
        *b ^= KEY[i % KEY.len()].wrapping_add(n as u8);
    }
}

/// The mood field of an unmasked section is 1..=3; a masked one decodes to that range.
pub fn is_masked(section: &[u8]) -> bool {
    if section.len() < 20 {
        return false;
    }
    let mood = u16::from_be_bytes([section[18], section[19]]);
    !(1..=3).contains(&mood)
}

/// The PSSI section (unmasked) for a structure.
pub fn pssi(s: &Structure) -> Vec<u8> {
    let n = s.phrases.len() as u16;
    let mut body = vec![];
    body.extend_from_slice(&s.mood.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&s.end_beat.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.push(0); // bank: default
    body.push(0);
    for (i, p) in s.phrases.iter().enumerate() {
        let mut e = [0u8; 24];
        e[0..2].copy_from_slice(&(i as u16 + 1).to_be_bytes());
        e[2..4].copy_from_slice(&p.beat.to_be_bytes());
        e[4..6].copy_from_slice(&p.kind.to_be_bytes());
        // k1 = 1 for intro, chorus and outro, as rekordbox writes them most often.
        e[7] = matches!(p.kind, INTRO | CHORUS | OUTRO) as u8;
        body.extend_from_slice(&e);
    }
    let mut v = vec![];
    v.extend_from_slice(b"PSSI");
    v.extend_from_slice(&32u32.to_be_bytes());
    v.extend_from_slice(&((18 + body.len()) as u32).to_be_bytes());
    v.extend_from_slice(&24u32.to_be_bytes());
    v.extend_from_slice(&n.to_be_bytes());
    v.extend_from_slice(&body);
    v
}

/// Parse an (unmasked or masked) PSSI section.
pub fn parse(section: &[u8]) -> Option<Structure> {
    let mut s = section.to_vec();
    if is_masked(&s) {
        mask_section(&mut s);
    }
    if s.len() < 32 || &s[..4] != b"PSSI" {
        return None;
    }
    let n = u16::from_be_bytes([s[16], s[17]]) as usize;
    let mood = u16::from_be_bytes([s[18], s[19]]);
    let end_beat = u16::from_be_bytes([s[26], s[27]]);
    let mut phrases = vec![];
    for k in 0..n {
        let e = s.get(32 + k * 24..32 + (k + 1) * 24)?;
        phrases.push(Phrase { beat: u16::from_be_bytes([e[2], e[3]]), kind: u16::from_be_bytes([e[4], e[5]]) });
    }
    Some(Structure { mood, phrases, end_beat })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pssi_roundtrip_and_mask() {
        let s = Structure { mood: 1, phrases: vec![Phrase { beat: 1, kind: INTRO }, Phrase { beat: 33, kind: UP }, Phrase { beat: 65, kind: CHORUS }, Phrase { beat: 129, kind: OUTRO }], end_beat: 160 };
        let mut b = pssi(&s);
        assert_eq!(b.len(), 32 + 4 * 24);
        assert_eq!(parse(&b), Some(s.clone()));
        mask_section(&mut b);
        assert!(is_masked(&b));
        assert_eq!(parse(&b), Some(s));
    }

    /// Compare PrepDeck's phrases with rekordbox's for the user's library, on rekordbox's own
    /// grids. PREPDECK_ANLZ_PAIRS = tsv "audio path\trekordbox ANLZ base". Run with --ignored.
    #[test]
    #[ignore]
    fn phrase_benchmark() {
        let Ok(f) = std::env::var("PREPDECK_ANLZ_PAIRS") else { return };
        let rows: Vec<(String, String)> = std::fs::read_to_string(f).unwrap().lines().filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string()))).collect();
        let res = std::sync::Mutex::new(vec![]);
        std::thread::scope(|sc| {
            for ch in rows.chunks((rows.len() / 8).max(1)) {
                let res = &res;
                sc.spawn(move || {
                    for (audio, base) in ch {
                        let ext = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.EXT")));
                        let Some(rb) = ext.get("PSSI").and_then(|s| parse(s)) else { continue };
                        if rb.mood != 1 {
                            continue;
                        }
                        let dat = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
                        let Some(q) = dat.get("PQTZ") else { continue };
                        let n = u32::from_be_bytes(q[20..24].try_into().unwrap()) as usize;
                        let beats: Vec<Beat> = (0..n).map(|k| {
                            let o = 24 + k * 8;
                            Beat { time: u32::from_be_bytes(q[o + 4..o + 8].try_into().unwrap()) as f64 / 1000.0, bpm: u16::from_be_bytes([q[o + 2], q[o + 3]]) as f64 / 100.0, beat: u16::from_be_bytes([q[o], q[o + 1]]) as u8, index: k }
                        }).collect();
                        let Ok(d) = crate::audio::decode_file(audio) else { continue };
                        let Some(mine) = analyze(&d.mono(), d.sr as f64, &beats) else { continue };
                        // Per-beat label agreement and boundary F1 (exact beat).
                        let label_at = |s: &Structure, b: u16| s.phrases.iter().rev().find(|p| p.beat <= b).map(|p| p.kind).unwrap_or(0);
                        let total = rb.end_beat.max(1);
                        let same = (1..=total).filter(|b| label_at(&rb, *b) == label_at(&mine, *b)).count();
                        let rb_b: std::collections::HashSet<u16> = rb.phrases.iter().map(|p| p.beat).collect();
                        let my_b: std::collections::HashSet<u16> = mine.phrases.iter().map(|p| p.beat).collect();
                        let hit = rb_b.intersection(&my_b).count();
                        res.lock().unwrap().push((same as f64 / total as f64, hit, rb_b.len(), my_b.len()));
                    }
                });
            }
        });
        let r = res.into_inner().unwrap();
        let n = r.len() as f64;
        let label = r.iter().map(|x| x.0).sum::<f64>() / n;
        let (hit, rbn, myn): (usize, usize, usize) = r.iter().fold((0, 0, 0), |a, x| (a.0 + x.1, a.1 + x.2, a.2 + x.3));
        let p = hit as f64 / myn.max(1) as f64;
        let rc = hit as f64 / rbn.max(1) as f64;
        eprintln!("tracks {n}  label agreement {:.1}%  boundary precision {:.1}% recall {:.1}% F1 {:.1}%", label * 100.0, p * 100.0, rc * 100.0, 2.0 * p * rc / (p + rc).max(1e-9) * 100.0);
    }
}
