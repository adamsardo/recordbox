//! Stem separation (vocals / instruments / bass / drums), free for everyone.
//!
//! On first use PrepDeck assesses the machine, picks the most capable open model that runs
//! well on it, downloads it once (checked against its published SHA-256 prefix) and keeps it in
//! the data folder. Separated stems are cached per track so they are only computed once.
//! rekordbox's own (vendor) models are never used.

pub mod htdemucs;

use anyhow::{anyhow, Context, Result};
use candle_core::Device;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// What this computer offers for running a separation model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hardware {
    pub cpu: String,
    pub cores: usize,
    pub ram_gb: f64,
    pub apple_silicon: bool,
    /// A Metal GPU Candle can use.
    pub gpu: bool,
}

fn sysctl(name: &str) -> Option<String> {
    let o = std::process::Command::new("/usr/sbin/sysctl").arg("-n").arg(name).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

pub fn assess() -> Hardware {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let (cpu, ram_gb) = if cfg!(target_os = "macos") {
        (sysctl("machdep.cpu.brand_string").unwrap_or_default(), sysctl("hw.memsize").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) / 1e9)
    } else {
        let mem = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| s.lines().find(|l| l.starts_with("MemTotal")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())).unwrap_or(0.0);
        (String::new(), mem / 1e6)
    };
    let apple_silicon = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let gpu = metal_device().is_some();
    Hardware { cpu, cores, ram_gb, apple_silicon, gpu }
}

fn metal_device() -> Option<Device> {
    #[cfg(target_os = "macos")]
    {
        Device::new_metal(0).ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The device a model runs on: the Metal GPU when there is one, otherwise the CPU.
pub fn device() -> Device {
    metal_device().unwrap_or(Device::Cpu)
}

/// One downloadable separation model.
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    /// Checkpoint files under ROOT_URL; the part after '-' is the first 8 hex digits of the
    /// file's SHA-256 (Demucs's own naming).
    pub files: &'static [&'static str],
    /// true: one specialised model per source (file i provides source i).
    pub per_source: bool,
    pub min_ram_gb: f64,
    pub needs_gpu: bool,
    pub about: &'static str,
    /// Full SHA-256 of each file when pinned (checked after download); otherwise the
    /// 8-digit prefix in the file name is checked.
    pub sha256: &'static [Option<&'static str>],
    pub licence: &'static str,
    pub source: &'static str,
}

pub const ROOT_URL: &str = "https://dl.fbaipublicfiles.com/demucs/hybrid_transformer/";

/// Best first. Both are open HTDemucs models (MIT licence) from facebookresearch/demucs.
pub const CATALOG: [ModelSpec; 2] = [
    ModelSpec {
        id: "htdemucs_ft",
        name: "HTDemucs fine-tuned",
        files: &["f7e0c4bc-ba3fe64a.th", "d12395a8-e57c48e6.th", "92cfc3b6-ef3bcb9c.th", "04573f0d-f3cf25b2.th"],
        per_source: true,
        min_ram_gb: 15.0,
        needs_gpu: true,
        about: "Four models, each fine-tuned for one stem: the best quality, about 4× the work.",
        // Hashes of the published files (their name prefixes match), pinned on 2026-11-17.
        sha256: &[
            Some("ba3fe64ae8ef66ac9a4857222ce48efbdc5eb3ad375cb79dd13debee5aaa4066"),
            Some("e57c48e6b0e38af4f7118d7bd08c49f0a0c0edf7d09143bdd902ea0d237303e6"),
            Some("ef3bcb9c8b40d14ae5d51b6db2587339cc12c6b77c0be151ce6d69002e087bf2"),
            Some("f3cf25b222c4eed7cd49dd8b2c9597d50c18bd154090f7b919cfa5f93cf22c49"),
        ],
        licence: "MIT",
        source: "facebookresearch/demucs (Hybrid Transformer Demucs, fine-tuned)",
    },
    ModelSpec {
        id: "htdemucs",
        name: "HTDemucs",
        files: &["955717e8-8726e21a.th"],
        per_source: false,
        min_ram_gb: 4.0,
        needs_gpu: false,
        about: "One model for all four stems: good quality, runs on any computer.",
        sha256: &[None],
        licence: "MIT",
        source: "facebookresearch/demucs (Hybrid Transformer Demucs)",
    },
];

/// The most capable model this machine runs well.
pub fn choose(hw: &Hardware) -> &'static ModelSpec {
    CATALOG.iter().find(|m| hw.ram_gb >= m.min_ram_gb && (!m.needs_gpu || hw.gpu)).unwrap_or(&CATALOG[CATALOG.len() - 1])
}

pub fn spec(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn models_dir() -> PathBuf {
    let d = crate::db::data_dir().join("models");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn stems_dir() -> PathBuf {
    let d = crate::db::data_dir().join("stems");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn model_ready(m: &ModelSpec) -> bool {
    m.files.iter().all(|f| models_dir().join(f).metadata().map(|x| x.len() > 1_000_000).unwrap_or(false))
}

fn sha_prefix_ok(path: &Path, file_name: &str) -> Result<bool> {
    let want = file_name.split('-').nth(1).and_then(|s| s.split('.').next()).unwrap_or("");
    let got = sha_hex(path)?;
    Ok(!want.is_empty() && got.starts_with(want))
}

fn sha_hex(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// Download any missing checkpoint of `m` into the models folder. Each file is written to a
/// temporary name, checked against its SHA-256 prefix and only then renamed into place.
pub fn download(m: &ModelSpec, progress: &mut dyn FnMut(u64, u64, &str), cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    for (k, f) in m.files.iter().enumerate() {
        let dest = models_dir().join(f);
        if dest.exists() {
            continue;
        }
        let url = format!("{ROOT_URL}{f}");
        let resp = ureq::get(&url).call().with_context(|| format!("downloading {url}"))?;
        let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let tmp = dest.with_extension("part");
        let mut out = std::fs::File::create(&tmp)?;
        let mut reader = resp.into_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                drop(out);
                let _ = std::fs::remove_file(&tmp);
                return Err(anyhow!("stopped"));
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, total, f);
        }
        out.sync_all()?;
        drop(out);
        let ok = match m.sha256.get(k).copied().flatten() {
            Some(full) => sha_hex(&tmp)? == full,
            None => sha_prefix_ok(&tmp, f)?,
        };
        if !ok {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow!("{f} did not match its published checksum; it was not used"));
        }
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(())
}

/// Separated stems of one track: planar f32, [source][channel][frame], 44.1 kHz.
pub struct Stems {
    pub len: usize,
    pub data: Vec<f32>,
}

impl Stems {
    /// Interleaved stereo samples of one source (0 drums, 1 bass, 2 other, 3 vocals).
    pub fn interleaved(&self, source: usize) -> Vec<f32> {
        let (l, r) = (&self.data[(source * 2) * self.len..(source * 2 + 1) * self.len], &self.data[(source * 2 + 1) * self.len..(source * 2 + 2) * self.len]);
        l.iter().zip(r).flat_map(|(a, b)| [*a, *b]).collect()
    }
}

/// Linear resampling of interleaved stereo (used only when a file is not 44.1 kHz).
fn resample_to_44k(samples: &[f32], sr: u32) -> Vec<f32> {
    if sr == htdemucs::SAMPLE_RATE {
        return samples.to_vec();
    }
    let frames = samples.len() / 2;
    let out_frames = (frames as f64 * htdemucs::SAMPLE_RATE as f64 / sr as f64).round() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let p = i as f64 * sr as f64 / htdemucs::SAMPLE_RATE as f64;
        let a = (p.floor() as usize).min(frames.saturating_sub(1));
        let b = (a + 1).min(frames.saturating_sub(1));
        let t = (p - a as f64) as f32;
        for ch in 0..2 {
            out.push(samples[a * 2 + ch] * (1.0 - t) + samples[b * 2 + ch] * t);
        }
    }
    out
}

/// Separate a decoded track with model `m` (which must be downloaded).
pub fn separate(m: &ModelSpec, samples: &[f32], sr: u32, progress: &mut dyn FnMut(f32) -> bool) -> Result<Stems> {
    let device = device();
    let inter = resample_to_44k(samples, sr);
    let len = inter.len() / 2;
    let mut planar = vec![0f32; 2 * len];
    for i in 0..len {
        planar[i] = inter[2 * i];
        planar[len + i] = inter[2 * i + 1];
    }
    let n_models = m.files.len();
    let mut out = vec![0f32; 8 * len];
    for (k, f) in m.files.iter().enumerate() {
        let graph = htdemucs::Graph::load(&models_dir().join(f), &device).map_err(|e| anyhow!("loading {f}: {e}"))?;
        let mut cb = |done: usize, total: usize| progress((k as f32 + done as f32 / total as f32) / n_models as f32);
        let y = htdemucs::separate(&graph, &device, &planar, len, &mut cb).map_err(|e| anyhow!("{e}"))?;
        if m.per_source {
            // Model k is the specialist for source k (weights matrix is the identity).
            out[k * 2 * len..(k + 1) * 2 * len].copy_from_slice(&y[k * 2 * len..(k + 1) * 2 * len]);
        } else {
            out.copy_from_slice(&y);
        }
    }
    Ok(Stems { len, data: out })
}

/// The vocal stem alone (mono, 44.1 kHz): the vocals specialist of HTDemucs fine-tuned when
/// it is downloaded, else HTDemucs. Used for vocal detection.
pub fn separate_vocals(samples: &[f32], sr: u32, progress: &mut dyn FnMut(f32) -> bool) -> Result<Vec<f32>> {
    separate_one(3, samples, sr, progress)
}

/// One source (0 drums, 1 bass, 2 other, 3 vocals) as mono 44.1 kHz, running a single model:
/// that source's specialist of HTDemucs fine-tuned when downloaded, else HTDemucs.
pub fn separate_one(source: usize, samples: &[f32], sr: u32, progress: &mut dyn FnMut(f32) -> bool) -> Result<Vec<f32>> {
    let ft = &CATALOG[0];
    let file = if model_ready(ft) { ft.files[source] } else { CATALOG[1].files[0] };
    let device = device();
    let inter = resample_to_44k(samples, sr);
    let len = inter.len() / 2;
    let mut planar = vec![0f32; 2 * len];
    for i in 0..len {
        planar[i] = inter[2 * i];
        planar[len + i] = inter[2 * i + 1];
    }
    let graph = htdemucs::Graph::load(&models_dir().join(file), &device).map_err(|e| anyhow!("loading {file}: {e}"))?;
    let mut cb = |done: usize, total: usize| progress(done as f32 / total as f32);
    let y = htdemucs::separate(&graph, &device, &planar, len, &mut cb).map_err(|e| anyhow!("{e}"))?;
    let (l, r) = (&y[source * 2 * len..(source * 2 + 1) * len], &y[(source * 2 + 1) * len..(source * 2 + 2) * len]);
    Ok(l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect())
}

/// Samples per vocal level (rekordbox's PVDI uses 1024 at 22,050 Hz: the same 46.4 ms).
pub const VOCAL_HOP: usize = 2048;
/// Feature: per 46.4 ms frame, the mean of the vocal stem's level (dB) and its level relative
/// to the mix (smoothed over 21 frames), then a centred moving mean over VOCAL_SMOOTH frames;
/// levels from four thresholds. Fitted against rekordbox's own vocal detection (PVDI) on
/// local tracks with leave-one-track-out checks; see PARITY-GAPS.md.
pub const VOCAL_SMOOTH: usize = 15;
pub const VOCAL_THRESHOLDS: [f32; 4] = [-25.41, -21.05, -17.95, -15.47];

fn frame_db(x: &[f32]) -> Vec<f32> {
    x.chunks(VOCAL_HOP).map(|c| 10.0 * (c.iter().map(|v| v * v).sum::<f32>() / c.len().max(1) as f32 + 1e-10).log10()).collect()
}

/// Centred moving mean with the edges held (as numpy's edge padding).
fn moving_mean(x: &[f32], w: usize) -> Vec<f32> {
    let n = x.len();
    if w <= 1 || n == 0 {
        return x.to_vec();
    }
    let pad = w / 2;
    let at = |i: isize| x[i.clamp(0, n as isize - 1) as usize];
    (0..n as isize).map(|i| (0..w as isize).map(|k| at(i - pad as isize + k)).sum::<f32>() / w as f32).collect()
}

/// Mono 44.1 kHz from interleaved stereo at any rate.
pub fn mono_44k(samples: &[f32], sr: u32) -> Vec<f32> {
    resample_to_44k(samples, sr).chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect()
}

/// Vocal levels 0–4 per 2048 samples from mono 44.1 kHz vocal stem and mix.
pub fn vocal_levels(vocal: &[f32], mix: &[f32]) -> Vec<u8> {
    let v = frame_db(vocal);
    let m = moving_mean(&frame_db(mix), 21);
    let f: Vec<f32> = v.iter().enumerate().map(|(i, x)| 0.5 * x + 0.5 * (x - m.get(i).copied().unwrap_or(*x))).collect();
    moving_mean(&f, VOCAL_SMOOTH).iter().map(|x| VOCAL_THRESHOLDS.iter().filter(|t| x >= *t).count() as u8).collect()
}

#[cfg(test)]
mod catalog_tests {
    #[test]
    fn pinned_hashes_match_file_names() {
        for m in super::CATALOG {
            assert_eq!(m.sha256.len(), m.files.len(), "{}", m.id);
            for (f, h) in m.files.iter().zip(m.sha256) {
                if let Some(h) = h {
                    let prefix = f.split('-').nth(1).and_then(|s| s.split('.').next()).unwrap();
                    assert!(h.starts_with(prefix) && h.len() == 64, "{f}");
                }
            }
        }
    }
}

#[cfg(test)]
mod key_bench {
    /// Key from the drum-free mix (mix minus the drums stem) against rekordbox's keys.
    /// PREPDECK_KEYGT (path<TAB>key), PREPDECK_KEY_N; run with --ignored.
    #[test]
    #[ignore]
    fn key_without_drums() {
        let rows: Vec<(String, i32)> = std::fs::read_to_string(std::env::var("PREPDECK_KEYGT").unwrap()).unwrap().lines().filter_map(|l| l.split_once('\t').map(|(p, k)| (p.to_string(), crate::model::parse_key(k)))).collect();
        let n: usize = std::env::var("PREPDECK_KEY_N").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
        let step = (rows.len() / n).max(1);
        let (mut full_ok, mut harm_ok, mut total) = (0, 0, 0);
        for (p, want) in rows.iter().step_by(step).take(n) {
            let Ok(d) = crate::audio::decode_file(p) else { continue };
            let mix44 = super::resample_to_44k(&d.samples, d.sr);
            let mono: Vec<f32> = mix44.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect();
            let drums = super::separate_one(0, &d.samples, d.sr, &mut |_| true).unwrap();
            let harm: Vec<f32> = mono.iter().zip(&drums).map(|(m, dr)| m - dr).collect();
            let k_full = crate::key::detect_key(&mono, 44100.0);
            let k_harm = crate::key::detect_key(&harm, 44100.0);
            total += 1;
            full_ok += (k_full == *want) as usize;
            harm_ok += (k_harm == *want) as usize;
            eprintln!("{} want {} full {} harm {}", p.rsplit('/').next().unwrap_or(""), crate::model::key_name(*want, false), crate::model::key_name(k_full, false), crate::model::key_name(k_harm, false));
        }
        eprintln!("key: full mix {full_ok}/{total}, without drums {harm_ok}/{total}");
    }
}

#[cfg(test)]
mod vocal_bench {
    /// Dump per-frame vocal-stem features next to rekordbox's PVDI levels, for fitting.
    /// PREPDECK_ANLZ_PAIRS, PREPDECK_VOCAL_N, PREPDECK_VOCAL_OUT; run with --ignored.
    #[test]
    #[ignore]
    fn dump_vocal_features() {
        use std::io::Write;
        let pairs = std::fs::read_to_string(std::env::var("PREPDECK_ANLZ_PAIRS").unwrap()).unwrap();
        let n: usize = std::env::var("PREPDECK_VOCAL_N").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
        let mut out = std::fs::File::create(std::env::var("PREPDECK_VOCAL_OUT").unwrap()).unwrap();
        writeln!(out, "track\tframe\tvocal_db\tmix_db\tlevel\tvocal_hp_db").unwrap();
        // Spread over the list (every k-th pair).
        let lines: Vec<&str> = pairs.lines().collect();
        let step = (lines.len() / n).max(1);
        for (ti, line) in lines.iter().step_by(step).take(n).enumerate() {
            let Some((path, base)) = line.split_once('\t') else { continue };
            let sec = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.2EX")));
            let Some(pv) = sec.get("PVDI").and_then(|s| crate::app::parse_pvdi(s)) else { continue };
            let Ok(dec) = crate::audio::decode_file(path) else { continue };
            let t0 = std::time::Instant::now();
            let voc = super::separate_vocals(&dec.samples, dec.sr, &mut |_| true).unwrap();
            let mix = super::resample_to_44k(&dec.samples, dec.sr);
            // Vocal stem high-passed at ~200 Hz (one-pole, twice) to drop bass/kick bleed.
            let a = (1.0 / (1.0 + 2.0 * std::f32::consts::PI * 200.0 / 44100.0)) as f32;
            let hp = |x: &[f32]| {
                let mut y = vec![0f32; x.len()];
                let (mut px, mut py) = (0f32, 0f32);
                for (i, v) in x.iter().enumerate() {
                    py = a * (py + v - px);
                    px = *v;
                    y[i] = py;
                }
                y
            };
            let voc_hp = hp(&hp(&voc));
            let hop = (pv.0 * 44100.0).round() as usize;
            for (f, lvl) in pv.1.iter().enumerate() {
                let a = f * hop;
                if a + hop > voc.len() {
                    break;
                }
                let e_v: f32 = voc[a..a + hop].iter().map(|x| x * x).sum::<f32>() / hop as f32;
                let e_m: f32 = (a..a + hop).map(|i| 0.25 * (mix[2 * i] + mix[2 * i + 1]).powi(2)).sum::<f32>() / hop as f32;
                let e_h: f32 = voc_hp[a..a + hop].iter().map(|x| x * x).sum::<f32>() / hop as f32;
                writeln!(out, "{ti}\t{f}\t{:.2}\t{:.2}\t{lvl}\t{:.2}", 10.0 * (e_v + 1e-10).log10(), 10.0 * (e_m + 1e-10).log10(), 10.0 * (e_h + 1e-10).log10()).unwrap();
            }
            eprintln!("{ti}: {} in {:.1}s", path.rsplit('/').next().unwrap_or(""), t0.elapsed().as_secs_f32());
        }
    }
}

// ------------------------------------------------------------------ cache

/// Cache file for a track's stems: keyed by the audio file's path, size and modification time
/// and the model, so a replaced file or another model is separated again.
pub fn cache_path(track_path: &str, model: &str) -> PathBuf {
    let meta = std::fs::metadata(track_path).ok();
    let mut h = Sha256::new();
    h.update(track_path.as_bytes());
    h.update(meta.as_ref().map(|m| m.len()).unwrap_or(0).to_le_bytes());
    if let Some(t) = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
        h.update(t.as_secs().to_le_bytes());
    }
    let key = format!("{:x}", h.finalize());
    stems_dir().join(format!("{}-{model}.stems", &key[..24]))
}

const MAGIC: &[u8; 8] = b"PDSTEMS1";

/// Stored as 16-bit PCM (planar) to keep the cache small.
pub fn save(path: &Path, s: &Stems) -> Result<()> {
    let tmp = path.with_extension("part");
    let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    f.write_all(MAGIC)?;
    f.write_all(&(s.len as u64).to_le_bytes())?;
    for v in &s.data {
        f.write_all(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())?;
    }
    // On disk before it gets its final name, so a crash can't leave a short cache file that
    // looks complete.
    let file = f.into_inner().map_err(|e| anyhow!("{e}"))?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Stems> {
    let b = std::fs::read(path)?;
    if b.len() < 16 || &b[..8] != MAGIC {
        return Err(anyhow!("not a stems cache file"));
    }
    let len = u64::from_le_bytes(b[8..16].try_into().unwrap()) as usize;
    let body = &b[16..];
    if body.len() != 8 * len * 2 {
        return Err(anyhow!("stems cache file is incomplete"));
    }
    let data = body.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32767.0).collect();
    Ok(Stems { len, data })
}

/// Total size of the stems cache in bytes.
pub fn cache_size() -> u64 {
    std::fs::read_dir(stems_dir()).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
}

pub fn clear_cache() {
    if let Ok(rd) = std::fs::read_dir(stems_dir()) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_best_model_for_hardware() {
        let big = Hardware { cpu: String::new(), cores: 10, ram_gb: 32.0, apple_silicon: true, gpu: true };
        let small = Hardware { cpu: String::new(), cores: 4, ram_gb: 8.0, apple_silicon: false, gpu: false };
        let no_gpu = Hardware { gpu: false, ..big.clone() };
        assert_eq!(choose(&big).id, "htdemucs_ft");
        assert_eq!(choose(&small).id, "htdemucs");
        assert_eq!(choose(&no_gpu).id, "htdemucs");
    }

    /// Separates 20 s of a real track with the local HTDemucs checkpoint (from the research
    /// folder) and checks that the four stems add back up to the mix. Run with --ignored.
    #[test]
    #[ignore]
    fn separates_and_stems_sum_to_mix() {
        let th = dirs::home_dir().unwrap().join("Developer/rekordbox-rea/analysis/open-model-harness/open-reference/955717e8-8726e21a.th");
        let audio = std::env::var("PREPDECK_STEM_TEST_AUDIO").unwrap_or_default();
        if !th.exists() || audio.is_empty() {
            return;
        }
        let device = device();
        println!("device: {:?}", device);
        let t0 = std::time::Instant::now();
        let graph = htdemucs::Graph::load(&th, &device).unwrap();
        println!("loaded .th in {:.1}s", t0.elapsed().as_secs_f64());
        let dec = crate::audio::decode_file(&audio).unwrap();
        assert_eq!(dec.sr, 44100);
        let start = 60 * 44100;
        let len = 20 * 44100;
        let seg = &dec.samples[start * 2..(start + len) * 2];
        let mut planar = vec![0f32; 2 * len];
        for i in 0..len {
            planar[i] = seg[2 * i];
            planar[len + i] = seg[2 * i + 1];
        }
        let t1 = std::time::Instant::now();
        let y = htdemucs::separate(&graph, &device, &planar, len, &mut |d, t| {
            println!("chunk {d}/{t} at {:.1}s", t1.elapsed().as_secs_f64());
            true
        })
        .unwrap();
        println!("separated 20 s in {:.1}s", t1.elapsed().as_secs_f64());
        let mut err = 0f64;
        let mut energy = 0f64;
        let mut src_energy = [0f64; 4];
        for ch in 0..2 {
            for i in 0..len {
                let sum: f32 = (0..4).map(|s| y[(s * 2 + ch) * len + i]).sum();
                let x = planar[ch * len + i];
                err += ((sum - x) as f64).powi(2);
                energy += (x as f64).powi(2);
                for s in 0..4 {
                    src_energy[s] += (y[(s * 2 + ch) * len + i] as f64).powi(2);
                }
            }
        }
        let snr = 10.0 * (energy / err.max(1e-12)).log10();
        println!("mix reconstruction SNR {snr:.1} dB; stem energy share {:?}", src_energy.map(|e| (e / energy * 100.0).round()));
        assert!(snr > 20.0, "stems should add back up to the mix");
    }
}
