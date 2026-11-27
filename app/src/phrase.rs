//! Phrase analysis (song structure) for tracks rekordbox has not analysed, and the PSSI section
//! the players read. Layout and the RB6+ XOR mask follow rekordcrate (src/anlz.rs). The
//! analysis is the port of rekordbox's own analyser (`phrase_rb*.rs`, all three moods);
//! `analyze` below is recordbox's earlier algorithm, kept behind `PREPDECK_PHRASE=own`.

use crate::model::Beat;

/// High-mood phrase kinds (the mood rekordbox gives most dance music).
pub const INTRO: u16 = 1;
pub const UP: u16 = 2;
pub const DOWN: u16 = 3;
pub const CHORUS: u16 = 5;
pub const OUTRO: u16 = 6;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Phrase {
    /// 1-based beat number (index into the track's beats) where the phrase starts.
    pub beat: u16,
    pub kind: u16,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Structure {
    /// 1 High, 2 Mid, 3 Low.
    pub mood: u16,
    pub phrases: Vec<Phrase>,
    /// Beat number where the last phrase ends.
    pub end_beat: u16,
    /// rekordbox's other PSSI fields, when the structure came from rekordbox or from the port of
    /// its analyser (`phrase_rb`); written back as they are. Dropped by edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rb: Option<RbFields>,
}

/// The PSSI fields besides beat and kind: header bytes 28/29 and, per phrase, entry words 3..11
/// (k1, k2, b, beat2, beat3, beat4, k3, fill, fill beat).
#[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct RbFields {
    pub h28: u8,
    pub h29: u8,
    pub rows: Vec<[u16; 9]>,
}

/// The phrase structure for a decoded track: the port of rekordbox's own analyser
/// (`phrase_rb`); recordbox's earlier algorithm with `PREPDECK_PHRASE=own`. `None` where
/// rekordbox's analyser gives no phrases either (no grid it accepts, silence, a failed run).
/// `bpm` is the track's BPM (its tempo input, as rekordbox passes the library's BPM; 0 = the
/// first beat's). `key` is the analyser's key input as `key_input` gives it (recordbox's index,
/// −1 none); only PSSI byte 29 depends on it (see `phrase_rb::analyser_key`).
pub fn analyze_audio(d: &crate::audio::Decoded, beats: &[Beat], bpm: f64, key: i32) -> Option<Structure> {
    if std::env::var("PREPDECK_PHRASE").map(|v| v == "own").unwrap_or(false) {
        return analyze(&d.mono(), d.sr as f64, beats);
    }
    crate::phrase_rb::analyze(&d.samples, d.sr, beats, bpm, crate::phrase_rb::analyser_key(key)).map(|r| crate::phrase_rb::to_structure(&r))
}

/// The key to give the phrase analyser for a track (recordbox's index, −1 none), following
/// rekordbox: its analysis thread passes the key already in its library when the run does not
/// analyse the key, and none when it does (a fresh track gets key and phrases in one run). In
/// recordbox the phrases come after the track's analysis, so a key recordbox's analysis found
/// counts as found in that same run; a key the user chose or one imported with the track (from
/// rekordbox's library or the file's tags, `key_text`) was there before.
pub fn key_input(t: &crate::model::Track) -> i32 {
    if t.key_manual || !t.key_text.is_empty() {
        t.key
    } else {
        -1
    }
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
    Some(Structure { mood: 1, phrases, end_beat: beats.len().min(u16::MAX as usize) as u16, rb: None })
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

/// The phrase types PHRASE EDIT offers for a mood, as (kind, name): High has Intro / Up /
/// Down / Chorus / Outro; Mid and Low have Intro / Verse / Bridge / Chorus / Outro (six verses
/// in Mid; Low numbers its two verses 2 and 5, each with two variants after it).
pub fn kinds(mood: u16) -> &'static [(u16, &'static str)] {
    match mood {
        2 => &[(1, "Intro"), (2, "Verse 1"), (3, "Verse 2"), (4, "Verse 3"), (5, "Verse 4"), (6, "Verse 5"), (7, "Verse 6"), (8, "Bridge"), (9, "Chorus"), (10, "Outro")],
        3 => &[(1, "Intro"), (2, "Verse 1"), (5, "Verse 2"), (8, "Bridge"), (9, "Chorus"), (10, "Outro")],
        _ => &[(INTRO, "Intro"), (UP, "Up"), (DOWN, "Down"), (CHORUS, "Chorus"), (OUTRO, "Outro")],
    }
}

/// Name of a phrase kind in its mood.
pub fn label(mood: u16, kind: u16) -> &'static str {
    match (mood, kind) {
        (2 | 3, 1) => "Intro",
        (2, 2..=7) => ["Verse 1", "Verse 2", "Verse 3", "Verse 4", "Verse 5", "Verse 6"][kind as usize - 2],
        (3, 2..=4) => "Verse 1",
        (3, 5..=7) => "Verse 2",
        (2 | 3, 8) => "Bridge",
        (2 | 3, 9) => "Chorus",
        (2 | 3, 10) => "Outro",
        (2 | 3, _) => "Phrase",
        _ => kind_name(kind),
    }
}

/// Colour of a phrase block in the phrase bar (0xRRGGBB).
pub fn color(mood: u16, kind: u16) -> u32 {
    match label(mood, kind) {
        "Intro" => 0x3E7BD6,
        "Up" | "Bridge" => 0xD6A23E,
        "Down" => 0x5AA05A,
        "Chorus" => 0xC8464B,
        "Outro" => 0x8A62C8,
        l if l.starts_with("Verse") => 0x3CAAB4,
        _ => 0x606060,
    }
}

/// Index of the phrase that contains 1-based beat number `beat`.
pub fn index_at(s: &Structure, beat: u16) -> Option<usize> {
    if s.phrases.is_empty() || beat > s.end_beat.max(s.phrases.last().map(|p| p.beat).unwrap_or(0)) {
        return None;
    }
    let i = s.phrases.partition_point(|p| p.beat <= beat);
    Some(i.saturating_sub(1))
}

/// PHRASE EDIT > CUT: split the phrase containing `beat` so that a new phrase of the same
/// type starts there. False (nothing changed) when a phrase already starts on that beat or
/// the beat is outside the phrases.
pub fn cut(s: &mut Structure, beat: u16) -> bool {
    if beat < 1 || beat >= s.end_beat || s.phrases.iter().any(|p| p.beat == beat) {
        return false;
    }
    let Some(i) = index_at(s, beat) else { return false };
    if s.phrases[i].beat > beat {
        return false;
    }
    let kind = s.phrases[i].kind;
    s.phrases.insert(i + 1, Phrase { beat, kind });
    s.rb = None;
    true
}

/// PHRASE EDIT's type list: give the phrase containing `beat` another type. False when it
/// already has it or there is no phrase there.
pub fn set_kind(s: &mut Structure, beat: u16, kind: u16) -> bool {
    match index_at(s, beat) {
        Some(i) if s.phrases[i].kind != kind => {
            s.phrases[i].kind = kind;
            s.rb = None;
            true
        }
        _ => false,
    }
}

/// 1-based number of the beat at or nearest before `t` (1 before the first beat).
pub fn beat_number(beats: &[Beat], t: f64) -> u16 {
    beats.partition_point(|b| b.time <= t + 1e-4).clamp(1, u16::MAX as usize) as u16
}

const KEY: [u8; 19] = [0xCB, 0xE1, 0xEE, 0xFA, 0xE5, 0xEE, 0xAD, 0xEE, 0xE9, 0xD2, 0xE9, 0xEB, 0xE1, 0xE9, 0xF3, 0xE8, 0xE9, 0xF4, 0xE1];

/// XOR the data part of a PSSI section (everything after the entry count) with rekordbox 6's
/// mask. Applying it twice restores the original.
/// Name of a phrase kind (high-mood labels).
pub fn kind_name(kind: u16) -> &'static str {
    match kind {
        INTRO => "Intro",
        UP => "Up",
        DOWN => "Down",
        CHORUS => "Chorus",
        OUTRO => "Outro",
        _ => "Phrase",
    }
}

/// Suggested memory cues: the start of each phrase after the first, named after the phrase
/// it starts in the structure's mood (provenance: phrase analysis). Times come from the grid's
/// beats.
pub fn cue_proposals(s: &Structure, beats: &[Beat]) -> Vec<(f64, &'static str)> {
    s.phrases
        .iter()
        .filter(|p| p.beat > 1)
        .filter_map(|p| beats.get(p.beat as usize - 1).map(|b| (b.time, label(s.mood, p.kind))))
        .collect()
}

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
    let rb = s.rb.as_ref().filter(|r| r.rows.len() == s.phrases.len());
    body.push(rb.map(|r| r.h28).unwrap_or(0));
    body.push(rb.map(|r| r.h29).unwrap_or(0));
    body.push(0); // bank: default
    body.push(0);
    for (i, p) in s.phrases.iter().enumerate() {
        let mut e = [0u8; 24];
        e[0..2].copy_from_slice(&(i as u16 + 1).to_be_bytes());
        e[2..4].copy_from_slice(&p.beat.to_be_bytes());
        e[4..6].copy_from_slice(&p.kind.to_be_bytes());
        match rb {
            Some(r) => {
                for (j, w) in r.rows[i].iter().enumerate() {
                    e[6 + 2 * j..8 + 2 * j].copy_from_slice(&w.to_be_bytes());
                }
            }
            // k1 = 1 for intro, chorus and outro, as rekordbox writes them most often.
            None => e[7] = matches!(p.kind, INTRO | CHORUS | OUTRO) as u8,
        }
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
    let mut rows = vec![];
    for k in 0..n {
        let e = s.get(32 + k * 24..32 + (k + 1) * 24)?;
        phrases.push(Phrase { beat: u16::from_be_bytes([e[2], e[3]]), kind: u16::from_be_bytes([e[4], e[5]]) });
        let mut r = [0u16; 9];
        for (j, w) in r.iter_mut().enumerate() {
            *w = u16::from_be_bytes([e[6 + 2 * j], e[7 + 2 * j]]);
        }
        rows.push(r);
    }
    // Fields equal to what `pssi` writes without them are left out, so the round trip holds.
    let plain = s[28] == 0 && s[29] == 0 && rows.iter().zip(&phrases).all(|(r, p)| *r == [matches!(p.kind, INTRO | CHORUS | OUTRO) as u16, 0, 0, 0, 0, 0, 0, 0, 0]);
    let rb = if plain { None } else { Some(RbFields { h28: s[28], h29: s[29], rows }) };
    Some(Structure { mood, phrases, end_beat, rb })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_edits() {
        let mut s = Structure { mood: 1, phrases: vec![Phrase { beat: 1, kind: INTRO }, Phrase { beat: 33, kind: UP }, Phrase { beat: 65, kind: CHORUS }], end_beat: 129, rb: None };
        // CUT inside the Up phrase: a second Up phrase starts at the cut.
        assert!(cut(&mut s, 49));
        assert_eq!(s.phrases.iter().map(|p| (p.beat, p.kind)).collect::<Vec<_>>(), vec![(1, INTRO), (33, UP), (49, UP), (65, CHORUS)]);
        // On an existing boundary, before the start or at the end: nothing happens.
        assert!(!cut(&mut s, 49));
        assert!(!cut(&mut s, 33));
        assert!(!cut(&mut s, 0));
        assert!(!cut(&mut s, 129));
        assert!(!cut(&mut s, 400));
        assert_eq!(s.phrases.len(), 4);
        // The type list changes the phrase under the playhead only.
        assert!(set_kind(&mut s, 50, DOWN));
        assert!(!set_kind(&mut s, 64, DOWN));
        assert_eq!(s.phrases[2], Phrase { beat: 49, kind: DOWN });
        assert_eq!(s.phrases[1].kind, UP);
        assert_eq!(index_at(&s, 1), Some(0));
        assert_eq!(index_at(&s, 128), Some(3));
        // The edited structure is what the PSSI section carries.
        assert_eq!(parse(&pssi(&s)), Some(s.clone()));
        // It survives being stored as JSON (how edits are kept per track).
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Structure>(&j).unwrap(), s);
        // Type lists per mood.
        assert_eq!(kinds(1).iter().map(|k| k.1).collect::<Vec<_>>(), vec!["Intro", "Up", "Down", "Chorus", "Outro"]);
        assert_eq!(kinds(2).len(), 10);
        assert_eq!(kinds(3).iter().map(|k| k.1).collect::<Vec<_>>(), vec!["Intro", "Verse 1", "Verse 2", "Bridge", "Chorus", "Outro"]);
        assert_eq!(label(1, 5), "Chorus");
        assert_eq!(label(2, 5), "Verse 4");
        assert_eq!(label(3, 6), "Verse 2");
        assert_eq!(label(2, 8), "Bridge");
        // Beat numbers from times.
        let beats: Vec<Beat> = (0..16).map(|i| Beat { time: 1.0 + i as f64 * 0.5, bpm: 120.0, beat: (i % 4) as u8 + 1, index: i }).collect();
        assert_eq!(beat_number(&beats, 0.0), 1);
        assert_eq!(beat_number(&beats, 1.0), 1);
        assert_eq!(beat_number(&beats, 1.6), 2);
        assert_eq!(beat_number(&beats, 99.0), 16);
    }

    #[test]
    fn key_input_follows_where_the_key_came_from() {
        let mut t = crate::model::Track { key: 5, ..Default::default() };
        // Found by recordbox's own analysis: rekordbox analyses such a key with the phrases.
        assert_eq!(key_input(&t), -1);
        t.key_text = "6A".into();
        assert_eq!(key_input(&t), 5, "imported with the track");
        t.key_text.clear();
        t.key_manual = true;
        assert_eq!(key_input(&t), 5, "chosen by the user");
        // Minor keys clear PSSI byte 29, major and unknown ones set it.
        assert!(crate::phrase_rb::analyser_key(5) >= 12 && crate::phrase_rb::analyser_key(17) < 12 && crate::phrase_rb::analyser_key(-1) < 12);
    }

    #[test]
    fn pssi_roundtrip_and_mask() {
        let s = Structure { mood: 1, phrases: vec![Phrase { beat: 1, kind: INTRO }, Phrase { beat: 33, kind: UP }, Phrase { beat: 65, kind: CHORUS }, Phrase { beat: 129, kind: OUTRO }], end_beat: 160, rb: None };
        let mut b = pssi(&s);
        assert_eq!(b.len(), 32 + 4 * 24);
        let p = parse(&b).unwrap();
        assert_eq!((p.mood, &p.phrases, p.end_beat), (s.mood, &s.phrases, s.end_beat));
        // The other fields come back from the section and are written as they were.
        assert_eq!(pssi(&p), b);
        mask_section(&mut b);
        assert!(is_masked(&b));
        assert_eq!(parse(&b), Some(p));
        // rekordbox's own flags, fill beats and header bytes survive a round trip.
        let rb = RbFields { h28: 0, h29: 1, rows: vec![[1, 0, 0, 0, 0, 0, 0, 0, 0], [0, 1, 1, 37, 45, 49, 0, 0, 0], [1, 0, 0, 0, 0, 0, 0, 1, 127], [0, 0, 0, 0, 0, 0, 1, 0, 0]] };
        let s2 = Structure { rb: Some(rb), ..s.clone() };
        let b2 = pssi(&s2);
        assert_eq!((b2[28], b2[29]), (0, 1));
        assert_eq!(parse(&b2), Some(s2.clone()));
        // An edit drops them (the phrases no longer match what rekordbox wrote).
        let mut s3 = s2;
        assert!(set_kind(&mut s3, 33, DOWN));
        assert_eq!(s3.rb, None);
    }

    /// Compare Recordbox's phrases with rekordbox's for the user's library, on rekordbox's own
    /// grids. PREPDECK_ANLZ_PAIRS = tsv "audio path\trekordbox ANLZ base". Run with --ignored.
    #[test]
    #[ignore]
    fn cue_proposal_benchmark() {
        // How well phrase-start proposals match the cues the user set in rekordbox (hot and
        // memory, read from rekordbox's own files): recall = user cues within one beat of a
        // proposal, precision = proposals within one beat of a user cue. rekordbox's own phrase
        // starts are scored the same way for reference.
        // The user's cues come from a snapshot of the rekordbox library (the original is not
        // touched); rekordbox keeps cues in master.db, not in its analysis files.
        let dest = std::env::temp_dir().join(format!("recordbox_cuebench_{}", std::process::id()));
        let Ok(Some(lib)) = crate::rbimport::load_installed(&dest) else { return };
        let rows: Vec<(String, String, Vec<f64>, f64)> = lib
            .tracks
            .iter()
            .filter_map(|t| Some((t.xt.track.path.clone(), t.anlz_base.as_ref()?.to_string_lossy().to_string(), t.xt.cues.iter().map(|c| c.start).collect::<Vec<f64>>(), t.xt.track.bpm)))
            .filter(|r| !r.2.is_empty())
            .collect();
        let res = std::sync::Mutex::new(vec![]);
        std::thread::scope(|sc| {
            for ch in rows.chunks((rows.len() / 8).max(1)) {
                let res = &res;
                sc.spawn(move || {
                    for (audio, base, user, track_bpm) in ch {
                        let dat = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
                        let Some(q) = dat.get("PQTZ") else { continue };
                        let n = u32::from_be_bytes(q[20..24].try_into().unwrap()) as usize;
                        let beats: Vec<Beat> = (0..n).map(|k| {
                            let o = 24 + k * 8;
                            Beat { time: u32::from_be_bytes(q[o + 4..o + 8].try_into().unwrap()) as f64 / 1000.0, bpm: u16::from_be_bytes([q[o + 2], q[o + 3]]) as f64 / 100.0, beat: u16::from_be_bytes([q[o], q[o + 1]]) as u8, index: k }
                        }).collect();
                        let beat_len = 60.0 / beats.first().map(|b| b.bpm).unwrap_or(120.0).max(1.0);
                        if std::env::var("PREPDECK_CUE_DUMP2").is_ok() {
                            let down = beats.iter().find(|b| b.beat == 1).map(|b| b.time).unwrap_or(-1.0);
                            for u in user.iter() {
                                eprintln!("cue2 at_first_downbeat={} at_first_beat={}", (u - down).abs() < 0.002, (u - beats[0].time).abs() < 0.002);
                            }
                        }
                        if std::env::var("PREPDECK_CUE_DUMP").is_ok() {
                            let first = beats.first().map(|b| b.time).unwrap_or(0.0);
                            eprintln!("cues {:?} first beat {:.3}", user.iter().map(|u| format!("{:.2}s (beat {:.1})", u, (u - first) / beat_len)).collect::<Vec<_>>(), first);
                        }
                        let Ok(d) = crate::audio::decode_file(audio) else { continue };
                        // PREPDECK_PHRASE=own measures the earlier algorithm, the default the port.
                        let Some(mine) = analyze_audio(&d, &beats, *track_bpm, -1) else { continue };
                        let ours: Vec<f64> = cue_proposals(&mine, &beats).iter().map(|p| p.0).collect();
                        let rb_ext = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.EXT")));
                        let theirs: Vec<f64> = rb_ext.get("PSSI").and_then(|s| parse(s)).map(|s| cue_proposals(&s, &beats).iter().map(|p| p.0).collect()).unwrap_or_default();
                        let near = |a: &[f64], t: f64| a.iter().any(|x| (x - t).abs() <= beat_len * 1.01);
                        let score = |props: &[f64]| (user.iter().filter(|u| near(props, **u)).count(), props.iter().filter(|p| near(&user, **p)).count(), props.len());
                        res.lock().unwrap().push((user.len(), score(&ours), score(&theirs)));
                    }
                });
            }
        });
        let r = res.into_inner().unwrap();
        let users: usize = r.iter().map(|x| x.0).sum();
        let sum = |f: &dyn Fn(&(usize, (usize, usize, usize), (usize, usize, usize))) -> (usize, usize, usize)| r.iter().map(f).fold((0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
        let o = sum(&|x| x.1);
        let t = sum(&|x| x.2);
        eprintln!("{} tracks, {users} user cues", r.len());
        eprintln!("recordbox phrase starts: recall {:.1}%, precision {:.1}% ({} proposals)", 100.0 * o.0 as f64 / users as f64, 100.0 * o.1 as f64 / o.2.max(1) as f64, o.2);
        eprintln!("rekordbox phrase starts: recall {:.1}%, precision {:.1}% ({} proposals)", 100.0 * t.0 as f64 / users as f64, 100.0 * t.1 as f64 / t.2.max(1) as f64, t.2);
    }

    #[test]
    #[ignore]
    fn phrase_benchmark() {
        let Ok(f) = std::env::var("PREPDECK_ANLZ_PAIRS") else { return };
        let rows: Vec<(String, String)> = std::fs::read_to_string(f).unwrap().lines().filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string()))).collect();
        // The track's BPM from rekordbox's library when PREPDECK_KEYTAB is given (variable-tempo
        // grids), else the first beat's.
        let keytab = crate::phrase_rb::oracle::read_keytab();
        let bpm_of = |base: &str| keytab.iter().find(|(k, _)| !k.is_empty() && base.ends_with(k.as_str())).map(|x| x.1 .2).unwrap_or(0.0);
        let res = std::sync::Mutex::new(vec![]);
        std::thread::scope(|sc| {
            for ch in rows.chunks((rows.len() / 8).max(1)) {
                let res = &res;
                sc.spawn(move || {
                    for (audio, base) in ch {
                        let ext = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.EXT")));
                        let Some(rb) = ext.get("PSSI").and_then(|s| parse(s)) else { continue };
                        let dat = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
                        let Some(q) = dat.get("PQTZ") else { continue };
                        let n = u32::from_be_bytes(q[20..24].try_into().unwrap()) as usize;
                        let beats: Vec<Beat> = (0..n).map(|k| {
                            let o = 24 + k * 8;
                            Beat { time: u32::from_be_bytes(q[o + 4..o + 8].try_into().unwrap()) as f64 / 1000.0, bpm: u16::from_be_bytes([q[o + 2], q[o + 3]]) as f64 / 100.0, beat: u16::from_be_bytes([q[o], q[o + 1]]) as u8, index: k }
                        }).collect();
                        let Ok(d) = crate::audio::decode_file(audio) else { continue };
                        // PREPDECK_PHRASE=own measures the earlier algorithm, the default the port.
                        let Some(mine) = analyze_audio(&d, &beats, bpm_of(base), -1) else { continue };
                        // Per-beat label agreement and boundary F1 (exact beat).
                        let label_at = |s: &Structure, b: u16| s.phrases.iter().rev().find(|p| p.beat <= b).map(|p| p.kind).unwrap_or(0);
                        let total = rb.end_beat.max(1);
                        let same = (1..=total).filter(|b| label_at(&rb, *b) == label_at(&mine, *b) && rb.mood == mine.mood).count();
                        let rb_b: std::collections::HashSet<u16> = rb.phrases.iter().map(|p| p.beat).collect();
                        let my_b: std::collections::HashSet<u16> = mine.phrases.iter().map(|p| p.beat).collect();
                        let hit = rb_b.intersection(&my_b).count();
                        res.lock().unwrap().push((same as f64 / total as f64, hit, rb_b.len(), my_b.len(), rb.mood));
                    }
                });
            }
        });
        let r = res.into_inner().unwrap();
        // High-mood tracks only (the set the earlier benchmark used), then every mood (a label
        // counts as agreeing only when the mood agrees too).
        for (what, high_only) in [("High mood", true), ("all moods", false)] {
            let r: Vec<_> = r.iter().filter(|x| !high_only || x.4 == 1).collect();
            let n = r.len() as f64;
            let label = r.iter().map(|x| x.0).sum::<f64>() / n.max(1.0);
            let (hit, rbn, myn): (usize, usize, usize) = r.iter().fold((0, 0, 0), |a, x| (a.0 + x.1, a.1 + x.2, a.2 + x.3));
            let p = hit as f64 / myn.max(1) as f64;
            let rc = hit as f64 / rbn.max(1) as f64;
            eprintln!("{what}: tracks {n}  label agreement {:.1}%  boundary precision {:.1}% recall {:.1}% F1 {:.1}%", label * 100.0, p * 100.0, rc * 100.0, 2.0 * p * rc / (p + rc).max(1e-9) * 100.0);
        }
    }
}
