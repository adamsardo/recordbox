//! Background work: analysis workers, file import, player loading and device export.

use eframe::egui;
use crate::analysis::{analyze_file_on_worker, AnalysisResult, AnalysisSettings};
use crate::audio::{decode_file, Decoded};
use crate::model::Track;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::VecDeque;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct AnalysisJob {
    pub track_id: i64,
    pub path: String,
    pub settings: AnalysisSettings,
}

#[derive(Default)]
struct AnalysisQueue {
    jobs: VecDeque<AnalysisJob>,
    active: usize,
}

type AnalysisOutcome = (i64, Result<AnalysisResult, String>);

pub struct AnalysisPool {
    workers: rayon::ThreadPool,
    queue: Arc<Mutex<AnalysisQueue>>,
    results: Sender<AnalysisOutcome>,
    pub rx: Receiver<AnalysisOutcome>,
    pub pending: Arc<AtomicUsize>,
    ctx: egui::Context,
    pub available_cpus: usize,
}

fn analysis_threads(requested: usize, available: usize) -> usize {
    let limit = available.clamp(1, 12);
    if requested == 0 { available.saturating_sub(1).clamp(1, limit) } else { requested.min(limit) }
}

fn build_workers(count: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(count).thread_name(|i| format!("rb-analysis-{i}"))
        .build().expect("start analysis workers")
}

impl AnalysisPool {
    pub fn new(ctx: egui::Context, requested: usize) -> AnalysisPool {
        let available = std::thread::available_parallelism().map(usize::from).unwrap_or(1);
        let (results, rx) = unbounded();
        AnalysisPool {
            workers: build_workers(analysis_threads(requested, available)), available_cpus: available,
            queue: Default::default(), results, rx, pending: Default::default(), ctx,
        }
    }

    pub fn thread_count(&self) -> usize {
        self.workers.current_num_threads()
    }

    pub fn set_threads(&mut self, requested: usize) {
        let count = analysis_threads(requested, self.available_cpus);
        if count != self.thread_count() && self.pending.load(Ordering::Acquire) == 0 {
            // Results already delivered to the UI channel survive a change of pool.
            self.workers = build_workers(count);
            self.queue = Default::default();
        }
    }

    pub fn submit(&self, job: AnalysisJob) {
        let mut queue = self.queue.lock();
        queue.jobs.push_back(job);
        self.pending.fetch_add(1, Ordering::Relaxed);
        // Bound live decoded tracks as well as CPU threads. Work stealing inside
        // a DSP pass must never start an unbounded number of waiting tracks.
        if queue.active >= self.thread_count() { return; }
        queue.active += 1;
        drop(queue);
        let queue = self.queue.clone();
        let results = self.results.clone();
        let pending = self.pending.clone();
        let ctx = self.ctx.clone();
        self.workers.spawn_fifo(move || loop {
            let job = {
                let mut q = queue.lock();
                match q.jobs.pop_front() {
                    Some(job) => job,
                    None => { q.active -= 1; return; }
                }
            };
            let result = analyze_file_on_worker(&job.path, &job.settings).map_err(|e| e.to_string());
            let _ = results.send((job.track_id, result));
            pending.fetch_sub(1, Ordering::Release);
            ctx.request_repaint();
        });
    }

    pub fn stop(&self) {
        let waiting = std::mem::take(&mut self.queue.lock().jobs);
        let cancelled = waiting.len();
        for job in waiting {
            let _ = self.results.send((job.track_id, Err("stopped".into())));
        }
        self.pending.fetch_sub(cancelled, Ordering::Release);
        self.ctx.request_repaint();
    }
}

pub fn spawn_import(paths: Vec<std::path::PathBuf>, ctx: egui::Context) -> Receiver<Option<Track>> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let files = crate::importer::collect_audio(&paths);
        for f in files {
            let t = crate::importer::read_track(&f);
            if tx.send(Some(t)).is_err() {
                return;
            }
            ctx.request_repaint();
        }
        let _ = tx.send(None);
        ctx.request_repaint();
    });
    rx
}

pub fn spawn_load(track_id: i64, path: String, ctx: egui::Context) -> Receiver<(i64, Result<Arc<Decoded>, String>)> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let r = decode_file(&path).map(Arc::new).map_err(|e| e.to_string());
        let _ = tx.send((track_id, r));
        ctx.request_repaint();
    });
    rx
}

pub struct ExportJob {
    pub rx: Receiver<crate::export::usb::Progress>,
    pub cancel: Arc<AtomicBool>,
    pub last: (usize, usize, String),
}

pub fn spawn_export(req: crate::export::usb::ExportRequest, ctx: egui::Context) -> ExportJob {
    let (tx, rx) = unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        let progress = move |p: crate::export::usb::Progress| {
            let _ = tx2.send(p);
            ctx2.request_repaint();
        };
        // A panic anywhere in the export is reported as a failed export, not a hung window.
        let r = crate::export::usb::run_guarded(|| crate::export::usb::run_export(req, &progress, &c2));
        let _ = tx.send(crate::export::usb::Progress::Finished(r));
        ctx.request_repaint();
    });
    ExportJob { rx, cancel, last: (0, 0, String::new()) }
}

#[cfg(test)]
mod throughput_probe {
    /// Opt-in: PREPDECK_PATHS=<file with one audio path per line> PREPDECK_N=<workers>.
    /// Prints how long analysing those files takes with that many workers.
    #[test]
    #[ignore]
    fn analysis_workers() {
        let paths: Vec<String> = std::fs::read_to_string(std::env::var("PREPDECK_PATHS").unwrap()).unwrap().lines().map(|s| s.to_string()).collect();
        let n = std::env::var("PREPDECK_N").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let pool = super::AnalysisPool::new(eframe::egui::Context::default(), n);
        let start = std::time::Instant::now();
        for (i, path) in paths.iter().enumerate() {
            pool.submit(super::AnalysisJob { track_id: i as i64, path: path.clone(), settings: Default::default() });
        }
        for _ in &paths { pool.rx.recv().unwrap().1.unwrap(); }
        eprintln!("{} files, {} workers: {:.3} s", paths.len(), pool.thread_count(), start.elapsed().as_secs_f64());
    }
}

#[cfg(test)]
mod analysis_pool_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn worker_budget_reserves_a_processor_and_caps_live_decodes() {
        for (cpus, automatic) in [(1, 1), (2, 1), (4, 3), (8, 7), (13, 12), (64, 12)] {
            assert_eq!(analysis_threads(0, cpus), automatic);
            assert_eq!(analysis_threads(1, cpus), 1);
            assert_eq!(analysis_threads(99, cpus), cpus.min(12));
        }
    }

    #[test]
    fn stopping_queued_work_does_not_cancel_the_next_submission() {
        let ctx = egui::Context::default();
        let pool = AnalysisPool::new(ctx, 1);
        let (entered, wait_entered) = crossbeam_channel::bounded(0);
        let (release, wait_release) = crossbeam_channel::bounded(0);
        pool.workers.spawn(move || { entered.send(()).unwrap(); wait_release.recv().unwrap(); });
        wait_entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let job = |track_id| AnalysisJob { track_id, path: "/missing/synthetic.wav".into(), settings: Default::default() };
        for id in 1..=3 { pool.submit(job(id)); }
        pool.stop();
        pool.submit(job(4));
        release.send(()).unwrap();
        for id in 1..=4 {
            let (track, result) = pool.rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(track, id);
            assert_eq!(result.err().unwrap() == "stopped", id < 4);
        }
    }

    #[test]
    fn parallel_analysis_matches_serial_for_normal_dynamic_and_disabled_passes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pulse.wav");
        let sr = 22050;
        let samples: Vec<f32> = (0..sr * 10).flat_map(|i| {
            let beat = if i % (sr / 2) < 200 { 0.55 } else { 0.03 };
            let v = beat * (std::f32::consts::TAU * 220.0 * i as f32 / sr as f32).sin();
            [v, v * 0.8]
        }).collect();
        crate::demo::write_wav(&path, sr, &samples).unwrap();
        let path = path.to_str().unwrap();
        let workers = build_workers(3);
        for (dynamic, do_grid, do_key) in [(false,true,true), (true,true,true), (false,false,false)] {
            let settings = AnalysisSettings { dynamic, do_grid, do_key, ..Default::default() };
            let a = crate::analysis::analyze_file(path, &settings).unwrap();
            let b = workers.install(|| analyze_file_on_worker(path, &settings)).unwrap();
            assert_eq!((a.duration.to_bits(), a.raw_samples, a.sample_rate, a.bpm.to_bits(), a.key),
                       (b.duration.to_bits(), b.raw_samples, b.sample_rate, b.bpm.to_bits(), b.key));
            assert_eq!(serde_json::to_value(a.grid).unwrap(), serde_json::to_value(b.grid).unwrap());
            assert_eq!(a.wave.frames, b.wave.frames);
            assert_eq!((a.gain_db.to_bits(), a.peak.to_bits()), (b.gain_db.to_bits(), b.peak.to_bits()));
        }
    }
}
