//! Background work: analysis workers, file import, player loading and device export.

use eframe::egui;
use crate::analysis::{analyze_file, AnalysisResult, AnalysisSettings};
use crate::audio::{decode_file, Decoded};
use crate::model::Track;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub struct AnalysisJob {
    pub track_id: i64,
    pub path: String,
    pub settings: AnalysisSettings,
}

pub struct AnalysisPool {
    tx: Sender<AnalysisJob>,
    pub rx: Receiver<(i64, Result<AnalysisResult, String>)>,
    pub pending: Arc<AtomicUsize>,
    pub cancel: Arc<AtomicBool>,
}

impl AnalysisPool {
    pub fn new(ctx: egui::Context) -> AnalysisPool {
        let (tx, jrx) = unbounded::<AnalysisJob>();
        let (rtx, rx) = unbounded();
        let pending = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 6).saturating_sub(1).max(1);
        for _ in 0..n {
            let jrx = jrx.clone();
            let rtx = rtx.clone();
            let pending = pending.clone();
            let cancel = cancel.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                while let Ok(job) = jrx.recv() {
                    if cancel.load(Ordering::Relaxed) {
                        pending.fetch_sub(1, Ordering::Relaxed);
                        let _ = rtx.send((job.track_id, Err("stopped".into())));
                        continue;
                    }
                    let r = analyze_file(&job.path, &job.settings).map_err(|e| e.to_string());
                    pending.fetch_sub(1, Ordering::Relaxed);
                    let _ = rtx.send((job.track_id, r));
                    ctx.request_repaint();
                }
            });
        }
        AnalysisPool { tx, rx, pending, cancel }
    }
    pub fn submit(&self, job: AnalysisJob) {
        self.cancel.store(false, Ordering::Relaxed);
        self.pending.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(job);
    }
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
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
        let r = crate::export::usb::run_export(req, &progress, &c2);
        let _ = tx.send(crate::export::usb::Progress::Finished(r.map_err(|e| e.to_string())));
        ctx.request_repaint();
    });
    ExportJob { rx, cancel, last: (0, 0, String::new()) }
}
