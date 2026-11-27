//! Stem separation (vocals / instruments / bass / drums), free for everyone.
//!
//! On first use Recordbox assesses the machine, picks the most capable open model that runs
//! well on it, downloads it once (checked against its published SHA-256 prefix) and keeps it in
//! the data folder. Separated stems are cached per track so they are only computed once.
//! rekordbox's own (vendor) models are never used.

pub mod htdemucs;
mod live_app;
#[cfg(test)]
mod bench;

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
    let gpu = device().is_metal();
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

/// The device a model runs on: the Metal GPU when there is one, otherwise the CPU. Made
/// once and kept, so loaded models and compiled GPU kernels stay usable between tracks.
pub fn device() -> Device {
    static DEVICE: std::sync::OnceLock<Device> = std::sync::OnceLock::new();
    DEVICE.get_or_init(|| metal_device().unwrap_or(Device::Cpu)).clone()
}

/// Share of each model segment that overlaps the next. Upstream's default; see
/// checks/stem-speed-study.md for what lower values cost and save.
pub const OVERLAP: f64 = 0.25;
/// Overlap when the result is only analysed (vocal levels, key), never played: 17 % fewer
/// segments; against known stems the separation quality is the same within 0.1 dB.
pub const ANALYSIS_OVERLAP: f64 = 0.1;

/// Loaded models stay in memory (about 170 MB each on the GPU) so that a second track, vocal
/// detection or key analysis does not read and upload the checkpoint again.
static LOADED: std::sync::Mutex<Vec<(String, std::sync::Arc<htdemucs::Graph>)>> = std::sync::Mutex::new(Vec::new());

/// Forget the loaded models (benchmarks of the first separation).
#[cfg(test)]
fn unload_models() {
    if let Ok(mut l) = LOADED.lock() {
        l.clear();
    }
}

fn graph(file: &str) -> Result<std::sync::Arc<htdemucs::Graph>> {
    let mut loaded = LOADED.lock().map_err(|_| anyhow!("model cache is unusable"))?;
    if let Some(i) = loaded.iter().position(|(f, _)| f == file) {
        // Most recently used last.
        let entry = loaded.remove(i);
        loaded.push(entry);
    } else {
        let g = htdemucs::Graph::load(&models_dir().join(file), &device()).map_err(|e| anyhow!("loading {file}: {e}"))?;
        // Room for the four fine-tuned specialists.
        while loaded.len() >= 4 {
            loaded.remove(0);
        }
        loaded.push((file.to_string(), std::sync::Arc::new(g)));
    }
    Ok(loaded.last().unwrap().1.clone())
}

/// Planar stereo (left samples, then right) at 44.1 kHz from interleaved stereo at `sr`.
fn planar_44k(samples: &[f32], sr: u32) -> (Vec<f32>, usize) {
    let resampled;
    let inter = if sr == htdemucs::SAMPLE_RATE {
        samples
    } else {
        resampled = resample_to_44k(samples, sr);
        &resampled
    };
    let len = inter.len() / 2;
    let mut planar = vec![0f32; 2 * len];
    let (l, r) = planar.split_at_mut(len);
    for (i, f) in inter.chunks_exact(2).enumerate() {
        l[i] = f[0];
        r[i] = f[1];
    }
    (planar, len)
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

/// Both are open HTDemucs models (MIT licence) from facebookresearch/demucs; `choose` picks
/// the recommended one.
pub const CATALOG: [ModelSpec; 2] = [
    ModelSpec {
        id: "htdemucs_ft",
        name: "HTDemucs fine-tuned",
        files: &["f7e0c4bc-ba3fe64a.th", "d12395a8-e57c48e6.th", "92cfc3b6-ef3bcb9c.th", "04573f0d-f3cf25b2.th"],
        per_source: true,
        min_ram_gb: 15.0,
        needs_gpu: true,
        about: "Four models, each fine-tuned for one stem: a little better on benchmark music, 4× the time.",
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
        about: "One model for all four stems: recommended. Four times faster, and the parts add back up to the track more exactly.",
        sha256: &[None],
        licence: "MIT",
        source: "facebookresearch/demucs (Hybrid Transformer Demucs)",
    },
];

/// The recommended model: HTDemucs, on every machine. The fine-tuned set takes four times as
/// long for about +0.2 dB SDR on the MUSDB benchmark (9.0 -> 9.2, upstream's figures); on
/// the tracks tried here it was not better and its four parts add back up to the mix less
/// exactly (checks/stem-speed-study.md), so it stays a choice in Preferences rather than
/// the default.
pub fn choose(_hw: &Hardware) -> &'static ModelSpec {
    &CATALOG[1]
}

/// Whether this machine can run model `m` at all.
#[allow(dead_code)]
pub fn runs_on(m: &ModelSpec, hw: &Hardware) -> bool {
    hw.ram_gb >= m.min_ram_gb && (!m.needs_gpu || hw.gpu)
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
    let (planar, len) = planar_44k(samples, sr);
    let live = Live::new(len);
    run_live(m, &planar, len, &live, progress)?;
    live.into_stems()
}

/// Frames per readiness block of a separation in progress (93 ms at 44.1 kHz).
pub const READY_BLOCK: usize = 4096;

/// A separation the player can use while it runs (rekordbox makes the parts usable per
/// chunk as they finish, and plays the original where they are not ready yet).
///
/// The stems buffer has the whole track's length from the start. A block of it may be read
/// once `ready_at` / `ready_span` says so; until then it is being written. The separation
/// makes the segment under the position given to `want` first, then the next ones ahead of
/// it, then those around the hot cues (`set_cues`), then the rest from the playhead on and
/// finally from the start.
pub struct Live {
    stems: std::sync::Arc<Stems>,
    /// The buffer, for the one separation that fills it (taken before it was shared).
    write: WritePtr,
    claimed: std::sync::atomic::AtomicBool,
    ready: Box<[std::sync::atomic::AtomicBool]>,
    /// Final frames per block (only the separation's own worker updates it).
    filled: std::sync::Mutex<Vec<u32>>,
    ready_frames: std::sync::atomic::AtomicUsize,
    /// Playhead in frames at 44.1 kHz (usize::MAX: none).
    want: std::sync::atomic::AtomicUsize,
    cues: std::sync::Mutex<Vec<usize>>,
    complete: std::sync::atomic::AtomicBool,
}

struct WritePtr(*mut f32);
// SAFETY: the pointer is used only by the one separation that claims it (`claim`).
unsafe impl Send for WritePtr {}
unsafe impl Sync for WritePtr {}

impl Live {
    /// Empty (silent, nothing ready) stems of `len` frames at 44.1 kHz.
    pub fn new(len: usize) -> Live {
        let mut data = vec![0f32; 8 * len];
        // Taken while the buffer is still owned here, before the Arc shares it.
        let write = WritePtr(data.as_mut_ptr());
        let blocks = len.div_ceil(READY_BLOCK);
        Live {
            stems: std::sync::Arc::new(Stems { len, data }),
            write,
            claimed: Default::default(),
            ready: (0..blocks).map(|_| Default::default()).collect(),
            filled: std::sync::Mutex::new(vec![0; blocks]),
            ready_frames: Default::default(),
            want: std::sync::atomic::AtomicUsize::new(usize::MAX),
            cues: Default::default(),
            complete: Default::default(),
        }
    }

    /// Empty stems for a decoded track (interleaved stereo at `sr`).
    pub fn for_track(samples: &[f32], sr: u32) -> Live {
        Live::new(len_44k(samples.len() / 2, sr))
    }

    /// The stems (complete only once `complete` is true; before that read only ready blocks).
    pub fn stems(&self) -> std::sync::Arc<Stems> {
        self.stems.clone()
    }

    pub fn len(&self) -> usize {
        self.stems.len
    }

    /// Whether frame `f` (44.1 kHz) of every part is final.
    #[inline]
    pub fn ready_at(&self, f: usize) -> bool {
        self.ready.get(f / READY_BLOCK).is_some_and(|r| r.load(std::sync::atomic::Ordering::Acquire))
    }

    /// Whether frames `a ..= b` are all final (clamped to the track).
    #[inline]
    pub fn ready_span(&self, a: usize, b: usize) -> bool {
        if self.complete() {
            return true;
        }
        let last = self.ready.len().saturating_sub(1);
        let (a, b) = ((a / READY_BLOCK).min(last), (b / READY_BLOCK).min(last));
        !self.ready.is_empty() && (a.min(b)..=a.max(b)).all(|i| self.ready[i].load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn complete(&self) -> bool {
        self.complete.load(std::sync::atomic::Ordering::Acquire)
    }

    /// For the audio callback: whether the stretches within `reach` source frames of each
    /// position (frames of a file at `sr`) are ready. No allocation, no locks.
    #[inline]
    pub fn ready_around(&self, positions: [f64; 2], reach: f64, sr: u32) -> bool {
        let k = htdemucs::SAMPLE_RATE as f64 / sr.max(1) as f64;
        positions.iter().all(|&p| self.ready_span(((p - reach).max(0.0) * k) as usize, ((p + reach).max(0.0) * k) as usize))
    }

    /// Share of the track that is ready (0 to 1).
    pub fn fraction(&self) -> f32 {
        self.ready_frames.load(std::sync::atomic::Ordering::Relaxed) as f32 / self.len().max(1) as f32
    }

    /// The ready stretches as fractions of the track (for the progress line under the
    /// overview, which rekordbox draws per separated range).
    pub fn ranges(&self) -> Vec<(f32, f32)> {
        let n = self.ready.len();
        let mut out: Vec<(f32, f32)> = vec![];
        let mut start = None;
        for i in 0..=n {
            let r = i < n && self.ready[i].load(std::sync::atomic::Ordering::Relaxed);
            match (r, start) {
                (true, None) => start = Some(i),
                (false, Some(a)) => {
                    out.push(((a * READY_BLOCK) as f32 / self.len().max(1) as f32, ((i * READY_BLOCK).min(self.len())) as f32 / self.len().max(1) as f32));
                    start = None;
                }
                _ => {}
            }
        }
        out
    }

    /// Where the listener is (frames at 44.1 kHz), or None.
    pub fn want(&self, frame: Option<usize>) {
        self.want.store(frame.unwrap_or(usize::MAX), std::sync::atomic::Ordering::Relaxed);
    }

    /// Hot cue positions (frames at 44.1 kHz), made after the playhead's stretch.
    pub fn set_cues(&self, frames: Vec<usize>) {
        if let Ok(mut c) = self.cues.lock() {
            *c = frames;
        }
    }

    fn wanted(&self) -> (Option<usize>, Vec<usize>) {
        let w = self.want.load(std::sync::atomic::Ordering::Relaxed);
        ((w != usize::MAX).then_some(w), self.cues.lock().map(|c| c.clone()).unwrap_or_default())
    }

    /// Frames [a, b) of every part are final.
    fn mark(&self, a: usize, b: usize) {
        let Ok(mut filled) = self.filled.lock() else { return };
        let len = self.len();
        for blk in a / READY_BLOCK..b.div_ceil(READY_BLOCK).min(filled.len()) {
            let (s, e) = (blk * READY_BLOCK, ((blk + 1) * READY_BLOCK).min(len));
            filled[blk] += (e.min(b) - s.max(a)) as u32;
            if filled[blk] as usize == e - s {
                // Release: the block's samples were written before this.
                self.ready[blk].store(true, std::sync::atomic::Ordering::Release);
            }
        }
        self.ready_frames.fetch_add(b - a, std::sync::atomic::Ordering::Relaxed);
    }

    /// The buffer to write, for the one separation allowed to fill it.
    fn claim(&self) -> Result<*mut f32> {
        if self.claimed.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(anyhow!("these stems are already being separated"));
        }
        Ok(self.write.0)
    }

    /// The finished stems, when nothing else holds them.
    fn into_stems(self) -> Result<Stems> {
        std::sync::Arc::try_unwrap(self.stems).map_err(|_| anyhow!("stems still shared"))
    }
}

/// Frames at 44.1 kHz of `frames` frames at `sr` (as `resample_to_44k` makes them).
fn len_44k(frames: usize, sr: u32) -> usize {
    if sr == htdemucs::SAMPLE_RATE {
        frames
    } else {
        (frames as f64 * htdemucs::SAMPLE_RATE as f64 / sr as f64).round() as usize
    }
}

/// Seconds of audio ahead of the playhead that are made before the hot cues.
const LOOKAHEAD_FRAMES: usize = 15 * 44100;

/// Which segment to make next. Segments are (start, end) frames; `taken` marks the ones
/// already made or under way. Order: the segments under the playhead, then those starting
/// within LOOKAHEAD_FRAMES after it, then the ones under each hot cue, then the rest from the
/// playhead on, then from the start. With no playhead: the hot cues, then in order.
pub(crate) fn next_segment(spans: &[(usize, usize)], taken: &[bool], want: Option<usize>, cues: &[usize]) -> Option<usize> {
    let free = |k: &usize| !taken[*k];
    let covering = |f: usize| (0..spans.len()).filter(move |&k| spans[k].0 <= f && f < spans[k].1);
    if let Some(p) = want {
        if let Some(k) = covering(p).find(free) {
            return Some(k);
        }
        if let Some(k) = (0..spans.len()).filter(|&k| spans[k].0 > p && spans[k].0 <= p + LOOKAHEAD_FRAMES).find(free) {
            return Some(k);
        }
    }
    for &c in cues {
        if let Some(k) = covering(c).find(free) {
            return Some(k);
        }
    }
    let p = want.unwrap_or(0);
    (0..spans.len()).filter(|&k| spans[k].1 > p).find(free).or_else(|| (0..spans.len()).find(free))
}

/// Separate a decoded track into `live` (made with `Live::for_track` for it), so the player
/// can use each stretch as soon as it is ready. The model must be downloaded.
pub fn separate_live(m: &ModelSpec, samples: &[f32], sr: u32, live: &Live, progress: &mut dyn FnMut(f32) -> bool) -> Result<()> {
    let (planar, len) = planar_44k(samples, sr);
    if len != live.len() {
        return Err(anyhow!("stems were made for another length"));
    }
    run_live(m, &planar, len, live, progress)
}

fn run_live(m: &ModelSpec, planar: &[f32], len: usize, live: &Live, progress: &mut dyn FnMut(f32) -> bool) -> Result<()> {
    let device = device();
    let out = live.claim()?;
    // One model for all four sources, or the fine-tuned specialists: model k is the one for
    // source k (upstream's weights matrix is the identity), so only that source is taken
    // from it. Each segment goes through all of them before the next, so a stretch is
    // complete after one segment.
    let graphs = m.files.iter().map(|f| graph(f)).collect::<Result<Vec<_>>>()?;
    let parts: Vec<htdemucs::Part> = graphs.iter().enumerate().map(|(k, g)| htdemucs::Part { graph: g, source: m.per_source.then_some(k) }).collect();
    let spans = htdemucs::Plan::new(len, OVERLAP).spans;
    let next = |taken: &[bool]| {
        let (want, cues) = live.wanted();
        next_segment(&spans, taken, want, &cues)
    };
    let ready = |a: usize, b: usize| live.mark(a, b);
    let sink = htdemucs::Sink { out, ready: &ready, next: Some(&next) };
    let mut cb = |done: usize, total: usize| progress(done as f32 / total as f32);
    let r = htdemucs::separate_with(&parts, &device, planar, len, OVERLAP, &sink, &mut cb).map_err(|e| anyhow!("{e}"));
    htdemucs::release_scratch();
    r?;
    live.complete.store(true, std::sync::atomic::Ordering::Release);
    Ok(())
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
    let (planar, len) = planar_44k(samples, sr);
    let g = graph(file)?;
    let mut cb = |done: usize, total: usize| progress(done as f32 / total as f32);
    let y = htdemucs::separate(&g, &device, &planar, len, &htdemucs::Options { overlap: ANALYSIS_OVERLAP, source: Some(source) }, &mut cb).map_err(|e| anyhow!("{e}"));
    htdemucs::release_scratch();
    let y = y?;
    Ok(y[..len].iter().zip(&y[len..]).map(|(a, b)| 0.5 * (a + b)).collect())
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
    // MP3s decoded before the timebase fix start 1105-1152 samples earlier: don't reuse those stems.
    if track_path.to_ascii_lowercase().ends_with(".mp3") {
        h.update(b"timebase-2");
    }
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
    // Converted a block at a time: one write per sample made this the slowest step after the
    // model itself.
    let mut block = Vec::with_capacity(2 << 16);
    for chunk in s.data.chunks(1 << 16) {
        block.clear();
        for v in chunk {
            block.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
        f.write_all(&block)?;
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

/// Keep the cache within `limit` bytes by removing the stems used longest ago (never `keep`,
/// the file just written). Separating on every load would otherwise fill the disk: a track's
/// stems take about 250 MB.
pub fn trim_cache(limit: u64, keep: &Path) {
    let Ok(rd) = std::fs::read_dir(stems_dir()) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "stems").unwrap_or(false))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.accessed().or_else(|_| m.modified()).ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= limit {
            break;
        }
        if path != keep && std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
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
        // The single model is recommended everywhere; the fine-tuned set is an option where
        // it can run.
        assert_eq!(choose(&big).id, "htdemucs");
        assert_eq!(choose(&small).id, "htdemucs");
        assert_eq!(choose(&no_gpu).id, "htdemucs");
        let ft = spec("htdemucs_ft").unwrap();
        assert!(runs_on(ft, &big) && !runs_on(ft, &small) && !runs_on(ft, &no_gpu));
    }

    /// Segment spans of a track of `n` segments at the played overlap.
    fn spans(len: usize) -> Vec<(usize, usize)> {
        htdemucs::Plan::new(len, OVERLAP).spans
    }

    #[test]
    fn makes_the_playhead_first_then_ahead_then_cues_then_the_rest() {
        let len = 60 * 44100 * 5; // 5 minutes
        let sp = spans(len);
        let n = sp.len();
        let mut taken = vec![false; n];
        // Playhead in the middle, in the overlap of two segments: both of those first.
        let k = n / 2;
        let p = sp[k].0 + 1000;
        assert!(sp[k - 1].1 > p, "test position should be in an overlap");
        let cue = sp[3].0 + 10;
        let mut order = vec![];
        while let Some(i) = next_segment(&sp, &taken, Some(p), &[cue]) {
            assert!(!taken[i], "segment {i} handed out twice");
            taken[i] = true;
            order.push(i);
        }
        assert_eq!(order.len(), n, "every segment is made once");
        assert_eq!(&order[..2], &[k - 1, k]);
        // Then the ones starting within the look-ahead, in order.
        let ahead: Vec<usize> = (0..n).filter(|&i| sp[i].0 > p && sp[i].0 <= p + LOOKAHEAD_FRAMES).collect();
        assert!(!ahead.is_empty());
        assert_eq!(&order[2..2 + ahead.len()], &ahead[..]);
        // Then the two segments under the hot cue.
        let after = 2 + ahead.len();
        assert_eq!(&order[after..after + 2], &[2, 3]);
        // The rest: forward from the playhead to the end, then from the start, each in order.
        let rest = &order[after + 2..];
        let split = rest.iter().position(|&i| sp[i].1 <= p).unwrap();
        assert!(rest[..split].iter().all(|&i| sp[i].1 > p) && rest[split..].iter().all(|&i| sp[i].1 <= p || i < k));
        assert!(rest[..split].windows(2).all(|w| w[0] < w[1]) && rest[split..].windows(2).all(|w| w[0] < w[1]));
        // Without a playhead: in order.
        let mut taken = vec![false; n];
        let plain: Vec<usize> = std::iter::from_fn(|| {
            let i = next_segment(&sp, &taken, None, &[])?;
            taken[i] = true;
            Some(i)
        })
        .collect();
        assert_eq!(plain, (0..n).collect::<Vec<_>>());
    }

    #[test]
    fn ready_blocks_follow_the_finished_frames() {
        let live = Live::new(10 * READY_BLOCK + 100);
        assert!(!live.ready_at(0) && !live.ready_span(0, 10));
        // A piece that ends inside a block leaves that block not ready.
        live.mark(0, 2 * READY_BLOCK + 10);
        assert!(live.ready_at(0) && live.ready_at(2 * READY_BLOCK - 1) && !live.ready_at(2 * READY_BLOCK));
        live.mark(2 * READY_BLOCK + 10, 3 * READY_BLOCK);
        assert!(live.ready_span(0, 3 * READY_BLOCK - 1) && !live.ready_span(0, 3 * READY_BLOCK));
        // The short last block.
        live.mark(10 * READY_BLOCK, 10 * READY_BLOCK + 100);
        assert!(live.ready_at(10 * READY_BLOCK + 99));
        let r = live.ranges();
        assert_eq!(r.len(), 2);
        assert!(r[0].0 == 0.0 && (r[0].1 - 3.0 * READY_BLOCK as f32 / live.len() as f32).abs() < 1e-6 && r[1].1 == 1.0);
        assert!((live.fraction() - (3 * READY_BLOCK + 100) as f32 / live.len() as f32).abs() < 1e-6);
        // Only one separation may fill the buffer.
        assert!(live.claim().is_ok() && live.claim().is_err());
    }

    /// The player uses the parts where they are ready and the original where they are not,
    /// while a part is muted (rekordbox's per-chunk rule).
    #[test]
    fn player_uses_only_ready_stretches() {
        use crate::audio::{render_deck, Decoded, PlayState};
        let n = 16 * READY_BLOCK;
        let mix: Vec<f32> = (0..n).map(|i| (i as f32 * 220.0 * std::f32::consts::TAU / 44100.0).sin() * 0.5).collect();
        let live = std::sync::Arc::new(Live::new(n));
        let out = live.claim().unwrap();
        // Drums and vocals are half the mix each; bass and other are silent. The first half
        // of the track is ready.
        for (src, k) in [(0usize, 0.5f32), (3, 0.5)] {
            for ch in 0..2 {
                // SAFETY: the claimed buffer, written before any reader is given it.
                let plane = unsafe { std::slice::from_raw_parts_mut(out.add((src * 2 + ch) * n), n) };
                for (p, m) in plane.iter_mut().zip(&mix) {
                    *p = m * k;
                }
            }
        }
        live.mark(0, n / 2);
        let audio = Decoded { sr: 44100, samples: mix.iter().flat_map(|x| [*x, *x]).collect(), raw_frames: n, damaged: 0, truncated: false };
        let mut s = PlayState::default();
        (s.audio, s.stems, s.stems_ready) = (Some(std::sync::Arc::new(audio)), Some(live.stems()), Some(live.clone()));
        (s.playing, s.volume, s.gain) = (true, 1.0, 1.0);
        s.stem_on[3] = false;
        let mut render = |s: &mut PlayState, frames: usize| {
            let mut o = vec![0f32; frames * 2];
            render_deck(s, &mut o, 2, 44100);
            o.chunks(2).map(|c| c[0]).collect::<Vec<f32>>()
        };
        // Ready stretch: vocals muted, so half the mix (after the 3.3 ms fade).
        s.pos = 1000.0;
        let a = render(&mut s, 2048);
        assert!(a[1000..].iter().zip(&mix[2000..3048]).all(|(o, m)| (o - 0.5 * m).abs() < 1e-4), "ready stretch should play the parts");
        // Not ready: the original, whatever is muted.
        s.pos = (n / 2 + 4 * READY_BLOCK) as f64;
        let start = s.pos as usize;
        let b = render(&mut s, 2048);
        assert!(b[1000..].iter().zip(&mix[start + 1000..start + 2048]).all(|(o, m)| (o - m).abs() < 1e-4), "unready stretch should play the original");
    }

    #[test]
    fn cache_file_round_trip() {
        let s = Stems { len: 70_000, data: (0..8 * 70_000).map(|i| ((i % 2001) as f32 / 1000.0 - 1.0) * 1.2).collect() };
        let path = std::env::temp_dir().join(format!("recordbox-stems-test-{}.stems", std::process::id()));
        save(&path, &s).unwrap();
        let back = load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.len, s.len);
        // 16-bit, clipped to full scale.
        assert!(s.data.iter().zip(&back.data).all(|(a, b)| (a.clamp(-1.0, 1.0) - b).abs() < 1.0 / 32000.0));
    }

    /// End-to-end timing on a real track, per stage, as the app runs it. Set
    /// PREPDECK_STEM_TEST_AUDIO to an audio file and optionally PREPDECK_STEM_MODEL
    /// (htdemucs / htdemucs_ft; the model must be in the models folder). Nothing is written
    /// to the stems cache. Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn bench_full_track() {
        let audio = std::env::var("PREPDECK_STEM_TEST_AUDIO").unwrap_or_default();
        if audio.is_empty() {
            return;
        }
        let m = std::env::var("PREPDECK_STEM_MODEL").ok().and_then(|id| spec(&id)).unwrap_or_else(|| choose(&assess()));
        if !model_ready(m) {
            println!("{} is not downloaded", m.id);
            return;
        }
        // Resident passes after the first; the best is reported (other work shares the GPU).
        let passes: usize = std::env::var("PREPDECK_STEM_PASSES").ok().and_then(|v| v.parse().ok()).unwrap_or(2).max(1);
        let t = std::time::Instant::now();
        let dec = crate::audio::decode_file(&audio).unwrap();
        let secs = dec.samples.len() as f64 / 2.0 / dec.sr as f64;
        println!("{}: {secs:.1} s of audio, device {:?}", m.id, device());
        println!("decode            {:6.2} s", t.elapsed().as_secs_f64());
        let mut best = f64::INFINITY;
        for pass in 0..=passes {
            let t = std::time::Instant::now();
            let st = separate(m, &dec.samples, dec.sr, &mut |_| true).unwrap();
            let d = t.elapsed().as_secs_f64();
            let label = if pass == 0 { "first (loads the model)".to_string() } else { format!("resident pass {pass}") };
            println!("separate, {label:<24} {d:6.2} s = {:.1}x real time", secs / d);
            if pass > 0 {
                best = best.min(d);
            }
            if pass == 1 {
                let path = std::env::temp_dir().join(format!("recordbox-stems-bench-{}.stems", std::process::id()));
                let t = std::time::Instant::now();
                save(&path, &st).unwrap();
                println!("cache write       {:6.2} s", t.elapsed().as_secs_f64());
                let t = std::time::Instant::now();
                let back = load(&path).unwrap();
                println!("cache read        {:6.2} s", t.elapsed().as_secs_f64());
                let _ = std::fs::remove_file(&path);
                assert_eq!(back.len, st.len);
            }
        }
        println!("separate, best resident    {best:6.2} s = {:.1}x real time", secs / best);
        let mut best_v = f64::INFINITY;
        for _ in 0..passes {
            let t = std::time::Instant::now();
            let v = separate_vocals(&dec.samples, dec.sr, &mut |_| true).unwrap();
            best_v = best_v.min(t.elapsed().as_secs_f64());
            assert!(!v.is_empty());
        }
        println!("vocal stem only, best {best_v:6.2} s");
        println!("peak memory footprint {:.2} GB", bench::peak_footprint() as f64 / 1e9);
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
        let opts = htdemucs::Options::default();
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
        let y = htdemucs::separate(&graph, &device, &planar, len, &opts, &mut |d, t| {
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
