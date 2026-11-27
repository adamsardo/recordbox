//! Speed and sameness checks for stem separation on real tracks. All are ignored tests, run
//! by hand (`cargo test --release <name> -- --ignored --nocapture`); none writes to the stems
//! cache or the user's data.
//!
//! Environment (paths stay out of the code; the test tracks are the user's own files):
//! - `PREPDECK_STEM_TEST_AUDIO`: the whole-track test file.
//! - `PREPDECK_STEM_EXCERPTS`: `path@start_seconds` entries separated by `|`; 45 s of each.
//! - `PREPDECK_STEM_REF`: folder for the reference outputs (raw little-endian f32, exactly
//!   what the separation returns).
//! - `PREPDECK_STEM_REF_MAKE=1`: write the reference instead of checking against it.
//! - `PREPDECK_STEM_CASES`: only run cases whose name contains one of these `,`-separated
//!   words.

use super::*;
use std::time::Instant;

const EXCERPT_SECS: usize = 45;

/// Decoded test audio: (name, interleaved stereo, rate).
fn inputs() -> Vec<(String, Vec<f32>, u32)> {
    let mut out = vec![];
    if let Ok(p) = std::env::var("PREPDECK_STEM_TEST_AUDIO") {
        if !p.is_empty() {
            let d = crate::audio::decode_file(&p).expect("decoding the whole-track test file");
            out.push(("whole".to_string(), d.samples, d.sr));
        }
    }
    if let Ok(list) = std::env::var("PREPDECK_STEM_EXCERPTS") {
        for (i, e) in list.split('|').filter(|e| !e.is_empty()).enumerate() {
            let (path, start) = e.rsplit_once('@').expect("excerpt as path@seconds");
            let d = crate::audio::decode_file(path).expect("decoding an excerpt");
            let start: usize = start.trim().parse::<usize>().unwrap() * d.sr as usize;
            let len = EXCERPT_SECS * d.sr as usize;
            let a = (2 * start).min(d.samples.len());
            let b = (2 * (start + len)).min(d.samples.len());
            out.push((format!("ex{}", i + 1), d.samples[a..b].to_vec(), d.sr));
        }
    }
    out
}

fn wanted(case: &str) -> bool {
    match std::env::var("PREPDECK_STEM_CASES") {
        Ok(f) if !f.is_empty() => f.split(',').any(|w| case.contains(w.trim())),
        _ => true,
    }
}

/// Signal-to-error ratio of `new` against `reference`, in dB (infinite when identical).
pub(super) fn snr_db(reference: &[f32], new: &[f32]) -> f64 {
    let (mut s, mut e) = (0f64, 0f64);
    for (a, b) in reference.iter().zip(new) {
        s += (*a as f64).powi(2);
        e += ((*a - *b) as f64).powi(2);
    }
    if e == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (s / e).log10()
    }
}

fn write_f32(path: &Path, v: &[f32]) {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

fn read_f32(path: &Path) -> Vec<f32> {
    std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display())).chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// One way the app runs a model, and what it returns.
enum Run {
    /// `separate` with this model: all four stems, stereo.
    Full(&'static str),
    /// `separate_one(source)`: one source, mono (the fine-tuned specialist when downloaded).
    One(usize),
    /// HTDemucs (single model) asked for one source at the analysis overlap, as
    /// `separate_one` runs it when the fine-tuned set is not downloaded.
    HtOne(usize),
}

fn run(r: &Run, samples: &[f32], sr: u32) -> (Vec<f32>, usize) {
    match r {
        Run::Full(id) => (separate(spec(id).unwrap(), samples, sr, &mut |_| true).unwrap().data, 4),
        Run::One(s) => (separate_one(*s, samples, sr, &mut |_| true).unwrap(), 1),
        Run::HtOne(s) => {
            let (planar, len) = planar_44k(samples, sr);
            let g = graph(CATALOG[1].files[0]).unwrap();
            let y = htdemucs::separate(&g, &device(), &planar, len, &htdemucs::Options { overlap: ANALYSIS_OVERLAP, source: Some(*s) }, &mut |_, _| true).unwrap();
            htdemucs::release_scratch();
            (y[..len].iter().zip(&y[len..]).map(|(a, b)| 0.5 * (a + b)).collect(), 1)
        }
    }
}

/// Every stem of every case against the reference made with the code as it was before the
/// 2026-11-27 speed work. A change passes when every stem agrees to at least 100 dB.
#[test]
#[ignore]
fn stems_match_reference() {
    let Ok(dir) = std::env::var("PREPDECK_STEM_REF") else {
        println!("set PREPDECK_STEM_REF");
        return;
    };
    let dir = PathBuf::from(dir);
    let make = std::env::var("PREPDECK_STEM_REF_MAKE").is_ok_and(|v| v == "1");
    std::fs::create_dir_all(&dir).unwrap();
    let ins = inputs();
    let runs: Vec<(String, Run)> = vec![
        ("htdemucs".into(), Run::Full("htdemucs")),
        ("htdemucs_ft".into(), Run::Full("htdemucs_ft")),
        ("vocal".into(), Run::One(3)),
        ("drums".into(), Run::One(0)),
        ("htvocal".into(), Run::HtOne(3)),
    ];
    let mut worst = f64::INFINITY;
    let mut stems = 0;
    for (rname, r) in &runs {
        for (iname, samples, sr) in &ins {
            let case = format!("{rname}-{iname}");
            // The drums and single-model vocal paths are the same code as `vocal`; the
            // excerpts are enough for them.
            if !wanted(&case) || (matches!(r, Run::One(0) | Run::HtOne(_)) && iname == "whole") {
                continue;
            }
            let t = Instant::now();
            let (y, n_src) = run(r, samples, *sr);
            let secs = t.elapsed().as_secs_f64();
            let path = dir.join(format!("{case}.f32"));
            if make {
                write_f32(&path, &y);
                println!("{case}: reference written ({} values, {secs:.2} s)", y.len());
                continue;
            }
            let reference = read_f32(&path);
            assert_eq!(reference.len(), y.len(), "{case}: length differs from the reference");
            let per = y.len() / n_src;
            let names: Vec<&str> = if n_src == 4 { htdemucs::SOURCES.to_vec() } else { vec![if matches!(r, Run::One(0)) { "drums" } else { "vocals" }] };
            let mut line = format!("{case:<22} {secs:6.2} s ");
            for (k, name) in names.iter().enumerate() {
                let snr = snr_db(&reference[k * per..(k + 1) * per], &y[k * per..(k + 1) * per]);
                worst = worst.min(snr);
                stems += 1;
                line += &if snr.is_infinite() { format!(" {name} identical") } else { format!(" {name} {snr:.1} dB") };
            }
            println!("{line}");
        }
    }
    if !make {
        println!("REFERENCE CHECK: {stems} stems, lowest SNR {}", if worst.is_infinite() { "inf (all identical)".to_string() } else { format!("{worst:.1} dB") });
        assert!(stems > 0 && worst >= 100.0, "a stem differs from the reference by more than rounding");
    }
}

/// Peak physical memory footprint of this process so far (bytes), from the kernel's own
/// ledger (`proc_pid_rusage`, as Activity Monitor and `time -l` report it).
#[cfg(target_os = "macos")]
pub(super) fn peak_footprint() -> u64 {
    extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
    }
    // rusage_info_v4 as 64-bit words: [0..2) uuid, ... [30] ri_lifetime_max_phys_footprint.
    let mut info = [0u64; 64];
    // SAFETY: the buffer is larger than rusage_info_v4 (flavor 4) and outlives the call.
    let ok = unsafe { proc_pid_rusage(std::process::id() as i32, 4, info.as_mut_ptr()) } == 0;
    if ok {
        info[30]
    } else {
        0
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn peak_footprint() -> u64 {
    0
}

fn print_profile(title: &str, rows: &[(String, f64, usize)], segments: usize, mark_cost: f64) {
    let total: f64 = rows.iter().map(|r| r.1 - r.2 as f64 * mark_cost).sum();
    println!("\n{title}: {:.1} ms per segment over {segments} segments (mark cost {:.3} ms subtracted)", total / segments as f64 * 1e3, mark_cost * 1e3);
    let mut sorted: Vec<&(String, f64, usize)> = rows.iter().collect();
    if std::env::var("PREPDECK_PROFILE_ORDER").is_err() {
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    }
    for (label, secs, n) in sorted {
        let t = secs - *n as f64 * mark_cost;
        println!("  {:>8.2} ms/seg {:>5.1} %  {:>5} calls  {label}", t / segments as f64 * 1e3, 100.0 * t / total, n);
    }
}

/// Where the GPU time of the Metal path goes: per stage of the network over the whole track,
/// then per operation over the first excerpt. HTDemucs (or PREPDECK_STEM_MODEL).
#[test]
#[ignore]
fn profile_network() {
    let ins = inputs();
    let Some(whole) = ins.iter().find(|i| i.0 == "whole") else { return };
    let m = std::env::var("PREPDECK_STEM_MODEL").ok().and_then(|id| spec(&id)).unwrap_or(&CATALOG[1]);
    let g = graph(m.files[0]).unwrap();
    let dev = device();
    let opts = htdemucs::Options { overlap: OVERLAP, source: None };
    let (planar, len) = planar_44k(&whole.1, whole.2);
    // Warm up (pipelines, pool).
    htdemucs::separate(&g, &dev, &planar[..], len, &opts, &mut |_, _| true).unwrap();
    let cost = htdemucs::profile_mark_cost(&dev);
    let segments = |len: usize| (0..len).step_by(((1.0 - OVERLAP) * htdemucs::TRAINING_FRAMES as f64) as usize).count();
    htdemucs::profile_begin(false);
    let t = Instant::now();
    htdemucs::separate(&g, &dev, &planar, len, &opts, &mut |_, _| true).unwrap();
    let wall = t.elapsed().as_secs_f64();
    let rows = htdemucs::profile_take();
    println!("whole track with a mark after every stage: {wall:.2} s");
    print_profile("stages", &rows, segments(len), cost);
    if let Some(ex) = ins.iter().find(|i| i.0 == "ex1") {
        let (planar, len) = planar_44k(&ex.1, ex.2);
        htdemucs::profile_begin(true);
        htdemucs::separate(&g, &dev, &planar, len, &opts, &mut |_, _| true).unwrap();
        let rows = htdemucs::profile_take();
        print_profile("operations", &rows, segments(len), cost);
    }
    htdemucs::release_scratch();
}

/// Time until the parts under the playhead can be played, from the start of the separation
/// (audio already decoded), with the playhead at the start and in the middle of the track,
/// first with the model to load and then with it loaded; and the model load alone.
/// PREPDECK_STEM_MODEL picks the model (HTDemucs by default).
#[test]
#[ignore]
fn bench_first_chunk() {
    let ins = inputs();
    let Some(whole) = ins.iter().find(|i| i.0 == "whole") else { return };
    let m = std::env::var("PREPDECK_STEM_MODEL").ok().and_then(|id| spec(&id)).unwrap_or(&CATALOG[1]);
    if !model_ready(m) {
        println!("{} is not downloaded", m.id);
        return;
    }
    // Warm the GPU kernels once so the model load is what the first row measures.
    separate_one(3, &whole.1[..2 * 44100 * 10], whole.2, &mut |_| true).unwrap();
    unload_models();
    let t = Instant::now();
    for f in m.files {
        graph(f).unwrap();
    }
    println!("model load ({} file(s))     {:6.3} s", m.files.len(), t.elapsed().as_secs_f64());
    let len = len_44k(whole.1.len() / 2, whole.2);
    for (label, at, unload) in [("start, model not loaded", 0usize, true), ("start, model loaded", 0, false), ("middle, model loaded", len / 2, false)] {
        if unload {
            unload_models();
        }
        let live = std::sync::Arc::new(Live::for_track(&whole.1, whole.2));
        live.want(Some(at));
        let t = Instant::now();
        let first = std::thread::scope(|s| {
            let job = s.spawn(|| separate_live(m, &whole.1, whole.2, &live, &mut |_| true));
            let mut first = None;
            while !job.is_finished() {
                if first.is_none() && live.ready_span(at, at + READY_BLOCK) {
                    first = Some(t.elapsed().as_secs_f64());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            job.join().unwrap().unwrap();
            first
        });
        let total = t.elapsed().as_secs_f64();
        println!("first usable chunk, {label:<24} {:6.3} s (whole track {total:.2} s)", first.unwrap_or(total));
    }
}
