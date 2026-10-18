//! Runs album verification on a background thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::engine::{self, AlbumReport, VerifyProgress};
use crate::error::{Error, Result};
use crate::plan::SplitPlan;

const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

pub struct VerifyItem {
    pub id: u64,
    pub name: String,
    pub base_dir: PathBuf,
    pub plan: SplitPlan,
    pub output_dir: PathBuf,
}

pub enum VerifyMsg {
    Started(String),
    /// Progress through the current album, `0.0..=1.0`.
    Progress(f32),
    Done(u64, Result<AlbumReport>),
    Finished,
    WorkerStopped,
}

pub struct VerifyJob {
    rx: Receiver<VerifyMsg>,
    cancel: Arc<AtomicBool>,
    pub total: usize,
}

impl VerifyJob {
    pub fn spawn(ctx: &egui::Context, items: Vec<VerifyItem>) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let ctx = ctx.clone();
        let total = items.len();

        thread::spawn(move || {
            for item in items {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let _ = tx.send(VerifyMsg::Started(item.name.clone()));
                ctx.request_repaint();

                let mut last: Option<Instant> = None;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    engine::verify_album(
                        &item.base_dir,
                        &item.plan,
                        &item.output_dir,
                        &flag,
                        &mut |p: VerifyProgress| {
                            if last.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
                                last = Some(Instant::now());
                                let _ = tx.send(VerifyMsg::Progress(p.overall()));
                                ctx.request_repaint();
                            }
                        },
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(Error::Internal(
                        "verification crashed; see the crash report".to_owned(),
                    ))
                });
                let _ = tx.send(VerifyMsg::Done(item.id, result));
                ctx.request_repaint();
            }
            let _ = tx.send(VerifyMsg::Finished);
            ctx.request_repaint();
        });

        Self { rx, cancel, total }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Delivers pending messages to `sink`. Returns `true` while the job is still running.
    pub fn drain(&self, mut sink: impl FnMut(VerifyMsg)) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    let done = matches!(msg, VerifyMsg::Finished);
                    sink(msg);
                    if done {
                        return false;
                    }
                }
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => {
                    sink(VerifyMsg::WorkerStopped);
                    return false;
                }
            }
        }
    }
}
