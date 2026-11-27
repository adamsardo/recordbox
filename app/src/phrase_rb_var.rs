//! Test-only corpus for the phrase analyser port: library tracks and other audio on this machine,
//! as they are and transformed (filtered, sped up or slowed down, sections cut out or muted,
//! shortened), each run through the port and through the research harness
//! (`checks/phrase-oracle-harness/`) on identical PCM and grid, compared in every field.
//! Inputs are only read; the harness's per-case PCM is written to a temporary folder that is
//! deleted as soon as the case is compared.

use crate::model::Beat;
use crate::phrase_rb::{analyze_traced, grid_input, oracle, s16_channels, Input, RbResult};

/// rekordbox's grid from an ANLZ base path (`….DAT`'s PQTZ).
pub fn read_pqtz(base: &str) -> Option<Vec<Beat>> {
    let dat = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
    let q = dat.get("PQTZ")?;
    let n = u32::from_be_bytes(q.get(20..24)?.try_into().ok()?) as usize;
    Some(
        (0..n)
            .filter_map(|k| {
                let o = 24 + k * 8;
                let e = q.get(o..o + 8)?;
                Some(Beat { time: u32::from_be_bytes(e[4..8].try_into().unwrap()) as f64 / 1000.0, bpm: u16::from_be_bytes([e[2], e[3]]) as f64 / 100.0, beat: u16::from_be_bytes([e[0], e[1]]) as u8, index: k })
            })
            .collect(),
    )
}

/// recordbox's own grid for decoded audio (beat analysis only), as the app would make it.
pub fn own_grid(d: &crate::audio::Decoded) -> Vec<Beat> {
    let s = crate::analysis::AnalysisSettings { do_key: false, vocal: false, ..Default::default() };
    crate::analysis::analyze_decoded(d, &s).grid.map(|g| g.beats(d.duration())).unwrap_or_default()
}

fn frame_at(t: f64, sr: u32, frames: usize) -> usize {
    ((t * sr as f64).round().max(0.0) as usize).min(frames)
}

fn reindex(beats: Vec<Beat>) -> Vec<Beat> {
    beats.into_iter().enumerate().map(|(i, b)| Beat { index: i, ..b }).collect()
}

/// Apply one variant step (`orig`, `hp<Hz>`, `lp<Hz>`, `spd<factor>`, `cut<bar>-<bar>`,
/// `mute<bar>-<bar>`, `skip<bars>`, `head<seconds>`, `mono`) to interleaved stereo samples and
/// the grid. Bars count from the grid's first beat, four beats each.
fn step(spec: &str, mut x: Vec<f32>, sr: u32, beats: Vec<Beat>) -> Option<(Vec<f32>, Vec<Beat>)> {
    let frames = x.len() / 2;
    let num = |p: &str| spec.strip_prefix(p).and_then(|v| v.parse::<f64>().ok());
    let range = |p: &str| spec.strip_prefix(p).and_then(|v| v.split_once('-')).and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)));
    let bar_t = |k: usize| beats.get(4 * k).map(|b| b.time);
    if spec == "orig" {
        return Some((x, beats));
    }
    if spec == "mono" {
        for c in x.chunks_exact_mut(2) {
            let m = 0.5 * (c[0] + c[1]);
            c[0] = m;
            c[1] = m;
        }
        return Some((x, beats));
    }
    for (p, kind) in [("hp", "hp"), ("lp", "lp")] {
        if let Some(f) = num(p) {
            for ch in 0..2 {
                let mut a = crate::analysis::Biquad::new(kind, f, 0.707, sr as f64, 0.0);
                let mut b = crate::analysis::Biquad::new(kind, f, 0.707, sr as f64, 0.0);
                for i in 0..frames {
                    let v = &mut x[2 * i + ch];
                    *v = b.run(a.run(*v as f64)) as f32;
                }
            }
            return Some((x, beats));
        }
    }
    if let Some(s) = num("spd") {
        let out_frames = (frames as f64 / s) as usize;
        let mut y = Vec::with_capacity(out_frames * 2);
        for i in 0..out_frames {
            let p = i as f64 * s;
            let k = p as usize;
            let f = (p - k as f64) as f32;
            for ch in 0..2 {
                let a = x.get(2 * k + ch).copied().unwrap_or(0.0);
                let b = x.get(2 * (k + 1) + ch).copied().unwrap_or(a);
                y.push(a + (b - a) * f);
            }
        }
        let nb = beats.into_iter().map(|b| Beat { time: b.time / s, bpm: b.bpm * s, ..b }).collect();
        return Some((y, nb));
    }
    if let Some((a, b)) = range("cut") {
        let (t0, t1) = (bar_t(a)?, bar_t(b)?);
        let (f0, f1) = (frame_at(t0, sr, frames), frame_at(t1, sr, frames));
        x.drain(2 * f0..2 * f1);
        let d = t1 - t0;
        let nb: Vec<Beat> = beats.into_iter().filter(|bt| bt.index < 4 * a || bt.index >= 4 * b).map(|bt| if bt.index >= 4 * b { Beat { time: bt.time - d, ..bt } } else { bt }).collect();
        return Some((x, reindex(nb)));
    }
    if let Some((a, b)) = range("mute") {
        let (f0, f1) = (frame_at(bar_t(a)?, sr, frames), frame_at(bar_t(b)?, sr, frames));
        for v in &mut x[2 * f0..2 * f1] {
            *v = 0.0;
        }
        return Some((x, beats));
    }
    if let Some(k) = num("skip") {
        let k = k as usize;
        let t0 = bar_t(k)?;
        let f0 = frame_at(t0, sr, frames);
        x.drain(..2 * f0);
        let nb: Vec<Beat> = beats.into_iter().filter(|b| b.index >= 4 * k).map(|b| Beat { time: (b.time - t0).max(0.0), ..b }).collect();
        return Some((x, reindex(nb)));
    }
    if let Some(s) = num("head") {
        let f = frame_at(s, sr, frames);
        x.truncate(2 * f);
        let nb: Vec<Beat> = beats.into_iter().filter(|b| b.time < s).collect();
        return Some((x, nb));
    }
    None
}

/// A variant: steps joined with `+`, applied left to right.
pub fn apply(spec: &str, x: Vec<f32>, sr: u32, beats: Vec<Beat>) -> Option<(Vec<f32>, Vec<Beat>)> {
    let mut cur = (x, beats);
    for s in spec.split('+') {
        cur = step(s, cur.0, sr, cur.1)?;
    }
    Some(cur)
}

pub struct CaseOut {
    pub name: String,
    pub variant: String,
    pub port: RbResult,
    pub harness: Option<RbResult>,
    /// mood after the type judgement, structure type, IntroEndJudge_main's third output
    pub path: (i32, i32, i32),
    pub secs: (f64, f64),
}

pub fn full_eq(a: &RbResult, b: &RbResult) -> bool {
    a == b
}

/// Run one case: port and harness on the same PCM and grid.
pub fn run_case(name: &str, variant: &str, samples: &[f32], sr: u32, beats: &[Beat], key: i32, tag: &str) -> Option<CaseOut> {
    let (bpm100, first_ms, first_bar) = grid_input(beats, 0.0)?;
    let mut tr = crate::phrase_rb_d::Trace::default();
    let t0 = std::time::Instant::now();
    let port = analyze_traced(samples, sr, beats, 0.0, key, &mut tr).unwrap_or_default();
    let tp = t0.elapsed().as_secs_f64();
    let (l, r) = s16_channels(samples);
    let inp = Input { sr: sr as i32, total: l.len() as i32, bpm100, first_ms, first_bar, key };
    let dir = oracle::tmp_dir(tag);
    let pcm = dir.join("pcm.s16");
    let h = oracle::write_s16(&pcm, &l, &r).ok().and_then(|_| oracle::run(&pcm, &inp, &[]));
    let _ = std::fs::remove_dir_all(&dir);
    let th = h.as_ref().map(|o| o.secs).unwrap_or(0.0);
    // A failed analysis is all zeros in the harness's result block; the port's None likewise.
    let harness = h.map(|o| if o.result.rows.is_empty() { RbResult::default() } else { o.result });
    Some(CaseOut { name: name.to_string(), variant: variant.to_string(), port, harness, path: (tr.mood, tr.ty, tr.v230), secs: (tp, th) })
}

/// Random inputs for the type-2 helpers (`SouonChangeDet`, `VocalInDetection`,
/// `CandidatesAdjustment`) through the port and through rekordbox's own routines
/// (`scratch/harness/rbfn`, in PREPDECK_RBPHRASE). PREPDECK_FUZZ = cases per routine (default
/// 3000). Inputs mimic the real ones: per-beat levels with steps and silent stretches, vocal
/// presence in runs, sorted boundary beats and bars.
#[test]
#[ignore]
fn phrase_fuzz_genre23() {
    let (Ok(dir), Ok(slice)) = (std::env::var("PREPDECK_RBPHRASE"), std::env::var("PREPDECK_RBSLICE")) else { return };
    let n: usize = std::env::var("PREPDECK_FUZZ").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
    let mut seed = 0x2545F4914F6CDD1Du64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut lines = vec![];
    let mut mine: Vec<Vec<i32>> = vec![];
    let fl = |v: &[f64]| format!("F {} {}", v.len(), v.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>().join(" "));
    let il = |v: &[i32]| format!("I {} {}", v.len(), v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" "));
    for k in 0..3 * n {
        let which = k % 3;
        let len = match (rnd() * 10.0) as u32 {
            0 => (rnd() * 24.0) as usize,
            1 => (rnd() * 64.0) as usize,
            _ => (rnd() * 1600.0) as usize,
        };
        match which {
            0 | 1 => {
                // runs of a level (or silence), each 1..40 beats; levels sometimes jump by ×2..×6
                let mut v = Vec::with_capacity(len);
                let mut lvl = rnd();
                while v.len() < len {
                    let run = 1 + (rnd() * 40.0) as usize;
                    let r = rnd();
                    lvl = if r < 0.25 { 0.0 } else if r < 0.45 { (lvl.max(0.05)) * (2.0 + rnd() * 4.0) } else if r < 0.6 { lvl / (2.0 + rnd() * 4.0) } else { rnd() };
                    let noise = rnd() * 0.2;
                    for _ in 0..run.min(len - v.len()) {
                        let x = if lvl == 0.0 { 0.0 } else { (lvl * (1.0 + noise * (rnd() - 0.5))).max(0.0) };
                        v.push(if rnd() < 0.03 { 0.0 } else { x });
                    }
                }
                if rnd() < 0.02 {
                    v.iter_mut().for_each(|x| *x = 0.0);
                }
                if which == 1 && rnd() < 0.5 {
                    // sung lines: 3..12 voiced beats, gaps of 3..7 beats, long rests in between
                    v.clear();
                    while v.len() < len {
                        let (on, off) = (3 + (rnd() * 10.0) as usize, if rnd() < 0.3 { 8 + (rnd() * 40.0) as usize } else { 3 + (rnd() * 5.0) as usize });
                        let lvl = 0.2 + rnd();
                        for j in 0..on {
                            v.push(lvl * (if j == 0 { 1.0 + rnd() } else { 0.7 + 0.3 * rnd() }));
                        }
                        v.extend(std::iter::repeat(0.0).take(off));
                    }
                    v.truncate(len);
                }
                if which == 0 {
                    lines.push(format!("SouonChangeDet {} O", fl(&v)));
                    mine.push(crate::phrase_rb_d15::souon_change_det(&v));
                } else {
                    lines.push(format!("VocalInDetection {} i 0 O", fl(&v)));
                    mine.push(crate::phrase_rb_d15::vocal_in_detection(&v));
                }
            }
            _ => {
                let bar = 1 + (rnd() * 4.0) as i32;
                let nbars = (rnd() * 200.0) as i32;
                let mut beats: Vec<i32> = vec![];
                let mut b = bar;
                for _ in 0..(rnd() * 30.0) as usize {
                    b += 4 * (1 + (rnd() * 16.0) as i32);
                    beats.push(if rnd() < 0.05 { b + 1 } else { b });
                }
                if rnd() < 0.7 {
                    beats.insert(0, bar);
                }
                let mut bars: Vec<i32> = (0..(rnd() * 25.0) as usize).map(|_| 1 + (rnd() * (nbars.max(1) as f64 + 8.0)) as i32).collect();
                bars.sort();
                bars.dedup();
                lines.push(format!("CandidatesAdjustment {} {} i {bar} i {nbars} O", il(&beats), il(&bars)));
                mine.push(crate::phrase_rb_d15::candidates_adjustment(&beats, &bars, bar, nbars));
            }
        }
    }
    let input = lines.join("\n") + "\n";
    let mut child = std::process::Command::new(format!("{dir}/rbfn")).args([slice.as_str(), &format!("{dir}/fixups.txt")]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    {
        use std::io::Write;
        let mut si = child.stdin.take().unwrap();
        std::thread::spawn(move || {
            let _ = si.write_all(input.as_bytes());
        });
    }
    let o = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    let theirs: Vec<Vec<i32>> = text.lines().map(|l| l.split_whitespace().skip(3).map(|x| x.parse().unwrap()).collect()).collect();
    let rets: Vec<i32> = text.lines().map(|l| l.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(-9)).collect();
    assert_eq!(theirs.len(), mine.len(), "harness answered every call");
    let names = ["SouonChangeDet", "VocalInDetection", "CandidatesAdjustment"];
    let mut eq = [0usize; 3];
    let mut nonempty = [0usize; 3];
    let mut failed = [0usize; 3];
    let mut second = 0usize;
    for (k, (a, b)) in mine.iter().zip(&theirs).enumerate() {
        let w = k % 3;
        if rets[k] != 1 {
            failed[w] += 1;
        }
        // SouonChangeDet keeps its zero padding; the others are compacted.
        if a == b {
            eq[w] += 1;
        } else if eq[w] + 5 > k / 3 {
            eprintln!("DIFF {} case {k}: port {:?}\n  reference {:?}\n  input {}", names[w], a, b, &lines[k][..lines[k].len().min(400)]);
        }
        nonempty[w] += a.iter().any(|x| *x != 0) as usize;
        if w == 1 && a == b {
            let v: Vec<f64> = lines[k].split_whitespace().skip(3).take_while(|t| *t != "i").map(|t| t.parse().unwrap()).collect();
            second += (crate::phrase_rb_d15::vocal_in_detection_with(&v, false) != *a) as usize;
        }
    }
    eprintln!("VocalInDetection: {second} equal cases where the short-gap detector changed the result");
    for w in 0..3 {
        eprintln!("{}: {}/{} equal ({} with a non-empty result, {} calls returned false)", names[w], eq[w], n, nonempty[w], failed[w]);
    }
}

/// Phrases on recordbox's own grids: for every PREPDECK_ANLZ_PAIRS track with a stored PSSI,
/// the port on recordbox's own beat analysis (standard, or `PREPDECK_HP_MODE=2` for the
/// high-precision port) against rekordbox's stored phrases, with the grid difference that goes
/// with each disagreement. With PREPDECK_RBPHRASE / PREPDECK_RBSLICE set the harness also runs
/// on the own grid with the analyser's own first-beat and bar corrections switched on (what
/// rekordbox does when it analyses the grid in the same run). PREPDECK_OWNGRID_OUT = TSV.
#[test]
#[ignore]
fn phrase_own_grid() {
    let rows = oracle::tracks();
    if rows.is_empty() {
        return;
    }
    let lim: usize = std::env::var("PREPDECK_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let threads: usize = std::env::var("PREPDECK_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
    let with_h = std::env::var("PREPDECK_RBPHRASE").is_ok();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let res = std::sync::Mutex::new(vec![]);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= rows.len().min(lim) {
                    break;
                }
                let t = &rows[i];
                let Ok(d) = crate::audio::decode_file(&t.audio) else { continue };
                let own = own_grid(&d);
                let port = crate::phrase_rb::analyze(&d.samples, d.sr, &own, 0.0, 0).unwrap_or_default();
                let (mut hc, mut ho) = (None, None);
                if with_h {
                    if let Some((bpm100, first_ms, first_bar)) = grid_input(&own, 0.0) {
                        let (l, r) = s16_channels(&d.samples);
                        let inp = Input { sr: d.sr as i32, total: l.len() as i32, bpm100, first_ms, first_bar, key: 0 };
                        let dir = oracle::tmp_dir(&format!("g{i}"));
                        let pcm = dir.join("pcm.s16");
                        if oracle::write_s16(&pcm, &l, &r).is_ok() {
                            hc = oracle::run(&pcm, &inp, &[("RB_NOCORR", "0".to_string())]).map(|o| o.result);
                            ho = oracle::run(&pcm, &inp, &[]).map(|o| if o.result.rows.is_empty() { RbResult::default() } else { o.result });
                        }
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                }
                res.lock().unwrap().push((i, own, port, hc, ho));
            });
        }
    });
    let mut r = res.into_inner().unwrap();
    r.sort_by_key(|x| x.0);
    let out = std::env::var("PREPDECK_OWNGRID_OUT").ok().map(|p| std::fs::File::create(p).unwrap());
    let mut out = out;
    let (mut n, mut eq_rows, mut eq_all, mut eq_hc, mut nh) = (0, 0, 0, 0, 0);
    let (mut lab_sum, mut hit, mut nb_st, mut nb_own) = (0.0, 0usize, 0usize, 0usize);
    let mut causes: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    let (mut eq_ho, mut nho) = (0, 0);
    for (i, own, port, hc, ho) in &r {
        let t = &rows[*i];
        let name = t.audio.rsplit('/').next().unwrap_or("");
        n += 1;
        let st = &t.stored;
        let rows_eq = port.rows.iter().map(|e| (e[0], e[1])).eq(st.rows.iter().map(|e| (e[0], e[1]))) && port.mood == st.mood;
        eq_rows += rows_eq as usize;
        eq_all += (port.rows == st.rows && port.mood == st.mood && port.end_beat == st.end_beat && port.h4 == st.h4) as usize;
        // Label agreement and boundaries in time (the grids may number beats differently).
        let time_of = |g: &[Beat], b: i32| g.get((b - 1).max(0) as usize).map(|x| x.time);
        let label_at = |r: &RbResult, g: &[Beat], tm: f64| r.rows.iter().rev().find(|e| time_of(g, e[0]).map(|x| x <= tm + 1e-3).unwrap_or(false)).map(|e| e[1]).unwrap_or(0);
        let end = st.end_beat.max(1) as usize;
        let same = (0..end.min(t.beats.len())).filter(|k| label_at(st, &t.beats, t.beats[*k].time) == label_at(port, own, t.beats[*k].time) && port.mood == st.mood).count();
        lab_sum += same as f64 / end.min(t.beats.len()).max(1) as f64;
        let beat_len = 60.0 / t.beats.first().map(|b| b.bpm).unwrap_or(120.0).max(1.0);
        let sb: Vec<f64> = st.rows.iter().filter_map(|e| time_of(&t.beats, e[0])).collect();
        let ob: Vec<f64> = port.rows.iter().filter_map(|e| time_of(own, e[0])).collect();
        hit += sb.iter().filter(|x| ob.iter().any(|y| (*x - y).abs() <= 0.5 * beat_len)).count();
        nb_st += sb.len();
        nb_own += ob.len();
        // The grid difference: tempo, first beat, bar phase (downbeat position mod 4 beats).
        let gs = grid_input(&t.beats, t.bpm);
        let go = grid_input(own, 0.0);
        let cause = match (gs, go) {
            (Some(a), Some(b)) if a == b => "same grid input".to_string(),
            (Some(a), Some(b)) if a.0 != b.0 => "tempo differs".to_string(),
            (Some(a), Some(b)) => {
                // Same tempo: where each grid puts its first downbeat, in ms, and the offset
                // between them reduced to within half a beat (the rest is whole beats).
                let per = 6_000_000.0 / a.0 as f64;
                let down = |g: (i32, i32, i32)| g.1 as f64 + (g.2 - 1) as f64 * per;
                let d = down(b) - down(a);
                let beats = (d / per).round() as i64;
                let ms = (d - beats as f64 * per).round().abs() as i64;
                if beats.rem_euclid(4) != 0 {
                    "bar phase differs".to_string()
                } else if ms <= 1 {
                    "first beat 1 ms apart".to_string()
                } else if ms <= 5 {
                    "first beat 2-5 ms apart".to_string()
                } else {
                    "first beat over 5 ms apart".to_string()
                }
            }
            (None, _) => "no stored grid input".to_string(),
            (_, None) => "own grid not accepted".to_string(),
        };
        let c = causes.entry(cause.clone()).or_default();
        c.0 += 1;
        c.1 += rows_eq as usize;
        if let Some(h) = ho {
            nho += 1;
            if h == port {
                eq_ho += 1;
            } else {
                eprintln!("PORT-HARNESS {name}: {}", oracle::row_diff(port, h));
            }
        }
        if let Some(h) = hc {
            nh += 1;
            let e = h.rows.iter().map(|e| (e[0], e[1])).eq(st.rows.iter().map(|e| (e[0], e[1]))) && h.mood == st.mood;
            eq_hc += e as usize;
        }
        if let Some(f) = out.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{name}\t{cause}\t{rows_eq}\t{:?}\t{:?}\t{}", gs, go, hc.as_ref().map(|h| (h.rows.iter().map(|e| (e[0], e[1])).eq(st.rows.iter().map(|e| (e[0], e[1]))) && h.mood == st.mood).to_string()).unwrap_or_default());
        }
        if !rows_eq {
            eprintln!("DIFF {name}: {cause} stored {:?} own {:?}\n  {}", gs, go, oracle::row_diff(port, st));
        }
    }
    for (k, (a, b)) in &causes {
        eprintln!("grid {k}: {b}/{a} with the stored phrases");
    }
    let p = hit as f64 / nb_own.max(1) as f64;
    let rc = hit as f64 / nb_st.max(1) as f64;
    eprintln!("own grid: {eq_rows}/{n} tracks with the stored phrases and mood, {eq_all}/{n} also in every per-phrase field; label agreement {:.1}%, boundaries (within half a beat) precision {:.1}% recall {:.1}% F1 {:.1}%", 100.0 * lab_sum / n.max(1) as f64, 100.0 * p, 100.0 * rc, 200.0 * p * rc / (p + rc).max(1e-9));
    if nho > 0 {
        eprintln!("port vs harness (corrections off) on the own grid: {eq_ho}/{nho} equal in every field");
    }
    if nh > 0 {
        eprintln!("harness with corrections on, own grid: {eq_hc}/{nh} with the stored phrases and mood");
    }
}

/// Byte 29 study: for every rekordbox track with a stored PSSI, the stored header bytes, the key
/// in rekordbox's library now and its analysis bookkeeping columns (from a snapshot of
/// master.db in PREPDECK_SCRATCH; the original is only read). PREPDECK_KEYTAB_OUT = TSV.
#[test]
#[ignore]
fn phrase_key_table() {
    let (Ok(out), Ok(tmp)) = (std::env::var("PREPDECK_KEYTAB_OUT"), std::env::var("PREPDECK_SCRATCH")) else { return };
    let Some(src) = crate::rbimport::library_dir() else { return };
    let dest = std::path::Path::new(&tmp).join(format!("keytab-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dest);
    let db = crate::rbimport::snapshot(&src, &dest).unwrap();
    let c = crate::rbimport::open_snapshot(&db).unwrap();
    fn vs(v: rusqlite::types::Value) -> String {
        match v {
            rusqlite::types::Value::Text(s) => s,
            rusqlite::types::Value::Integer(n) => n.to_string(),
            rusqlite::types::Value::Real(x) => x.to_string(),
            _ => String::new(),
        }
    }
    let keys: std::collections::HashMap<String, String> = {
        let mut st = c.prepare("SELECT ID, ScaleName FROM djmdKey").unwrap();
        st.query_map([], |r| Ok((vs(r.get(0)?), vs(r.get(1)?)))).unwrap().filter_map(|x| x.ok()).collect()
    };
    let cols: Vec<String> = {
        let mut st = c.prepare("SELECT name FROM pragma_table_info('djmdContent')").unwrap();
        st.query_map([], |r| r.get::<_, String>(0)).unwrap().filter_map(|x| x.ok()).collect()
    };
    let want = ["FolderPath", "KeyID", "Analysed", "AnalysisDataPath", "created_at", "updated_at", "AnalysisUpdated", "DateCreated", "StockDate", "ReleaseDate", "rb_LocalFolderPath", "FileSize", "Length", "SampleRate", "BPM"];
    let sel: Vec<&str> = want.iter().copied().filter(|w| cols.iter().any(|c| c == w)).collect();
    let mut f = std::fs::File::create(&out).unwrap();
    use std::io::Write;
    let _ = writeln!(f, "h28\th29\tkey\t{}", sel.join("\t"));
    let sql = format!("SELECT {} FROM djmdContent WHERE rb_local_deleted = 0", sel.join(", "));
    let mut st = c.prepare(&sql).unwrap();
    let share = src.join("share");
    let mut rows = st.query([]).unwrap();
    while let Ok(Some(r)) = rows.next() {
        let v: Vec<String> = (0..sel.len()).map(|i| r.get(i).map(vs).unwrap_or_default()).collect();
        let ai = sel.iter().position(|s| *s == "AnalysisDataPath").unwrap();
        let ki = sel.iter().position(|s| *s == "KeyID").unwrap();
        let ap = &v[ai];
        if ap.is_empty() {
            continue;
        }
        let ext = share.join(ap.trim_start_matches('/')).with_extension("EXT");
        let Some(p) = crate::export::anlz::read_sections(&ext).get("PSSI").and_then(|s| crate::phrase_rb::parse_pssi(s)) else { continue };
        let key = keys.get(&v[ki]).cloned().unwrap_or_default();
        let _ = writeln!(f, "{}\t{}\t{}\t{}", p.h4, p.h5, key, v.join("\t"));
    }
    drop(rows);
    drop(st);
    drop(c);
    let _ = std::fs::remove_dir_all(&dest);
}

/// PREPDECK_CORPUS = TSV of `audio<TAB>grid<TAB>variants`, where grid is an ANLZ base path
/// (rekordbox's grid) or `own` (recordbox's beat analysis) and variants a space-separated list.
/// PREPDECK_CORPUS_OUT = result TSV (one line per case). PREPDECK_THREADS (default 6),
/// PREPDECK_KEY (analyser key input, default 0). Needs PREPDECK_RBPHRASE / PREPDECK_RBSLICE.
#[test]
#[ignore]
fn phrase_corpus() {
    let Ok(f) = std::env::var("PREPDECK_CORPUS") else { return };
    let key: i32 = std::env::var("PREPDECK_KEY").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let rows: Vec<(String, String, Vec<String>)> = std::fs::read_to_string(f)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.split('\t');
            Some((it.next()?.to_string(), it.next()?.to_string(), it.next().unwrap_or("orig").split_whitespace().map(|s| s.to_string()).collect()))
        })
        .collect();
    let threads: usize = std::env::var("PREPDECK_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let res = std::sync::Mutex::new(vec![]);
    let out_path = std::env::var("PREPDECK_CORPUS_OUT").ok();
    let out_file = std::sync::Mutex::new(out_path.as_ref().map(|p| std::fs::File::create(p).unwrap()));
    let fmt = |r: &RbResult| r.rows.iter().map(|e| e.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join(" ");
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((audio, grid, vars)) = rows.get(i) else { break };
                let Ok(d) = crate::audio::decode_file(audio) else {
                    eprintln!("UNREADABLE {audio}");
                    continue;
                };
                let beats = if grid == "own" { own_grid(&d) } else { read_pqtz(grid).unwrap_or_default() };
                let name = audio.rsplit('/').next().unwrap_or("").to_string();
                for (j, v) in vars.iter().enumerate() {
                    let Some((x, b)) = apply(v, d.samples.clone(), d.sr, beats.clone()) else {
                        eprintln!("NOVARIANT {name} {v}");
                        continue;
                    };
                    let Some(c) = run_case(&name, v, &x, d.sr, &b, key, &format!("c{i}-{j}")) else {
                        eprintln!("NOGRID {name} {v}");
                        continue;
                    };
                    if let Some(f) = out_file.lock().unwrap().as_mut() {
                        use std::io::Write;
                        let h = c.harness.clone().unwrap_or_default();
                        let _ = writeln!(f, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", c.name, c.variant, grid == "own", c.path.0, c.path.1, c.path.2, c.port.mood, c.port.rows.len(), c.harness.as_ref().map(|h| full_eq(&c.port, h)).unwrap_or(false), format!("{} {} {} {} {} {}", c.port.first_ms, c.port.first_bar, c.port.h4, c.port.h5, c.port.end_beat, fmt(&c.port)), format!("{} {} {} {} {} {}", h.first_ms, h.first_bar, h.h4, h.h5, h.end_beat, fmt(&h)), format!("{:.2} {:.2}", c.secs.0, c.secs.1));
                    }
                    res.lock().unwrap().push(c);
                }
            });
        }
    });
    let r = res.into_inner().unwrap();
    let mut by: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    let (mut n, mut eq, mut noh) = (0, 0, 0);
    for c in &r {
        let Some(h) = &c.harness else {
            noh += 1;
            eprintln!("NOHARNESS {} {}", c.name, c.variant);
            continue;
        };
        n += 1;
        let e = full_eq(&c.port, h);
        eq += e as usize;
        let cls = if c.port.rows.is_empty() && h.rows.is_empty() {
            "no result".to_string()
        } else {
            let m = ["?", "High", "Mid", "Low"].get(h.mood.clamp(0, 3) as usize).copied().unwrap_or("?");
            format!("{m} type {}{}", c.path.1, if h.rows.len() == 1 { " single phrase" } else { "" })
        };
        let x = by.entry(cls).or_default();
        x.0 += 1;
        x.1 += e as usize;
        if !e {
            eprintln!("DIFF {} [{}] path {:?}\n  {}", c.name, c.variant, c.path, oracle::row_diff(&c.port, h));
        }
    }
    for (k, (a, b)) in &by {
        eprintln!("class {k}: {b}/{a} equal");
    }
    eprintln!("corpus: {eq}/{n} cases equal in every field ({noh} without a harness run)");
}
