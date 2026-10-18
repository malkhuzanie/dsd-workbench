//! Runs a batch of splits on a background thread and streams events back to the UI.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui;

use crate::engine::{Event, SplitEngine};
use crate::error::{Error, Result};

pub struct BatchItem {
    pub id: u64,
    pub engine: SplitEngine,
}

pub enum Msg {
    Started(u64),
    Event(Event),
    Done(u64, Result<()>),
    Finished,
    WorkerStopped,
}

pub struct Job {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    pub dry_run: bool,
    pub total: usize,
}

impl Job {
    pub fn spawn(ctx: &egui::Context, items: Vec<BatchItem>, dry_run: bool) -> Self {
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
                let id = item.id;
                let _ = tx.send(Msg::Started(id)); // receiver gone means the app closed
                ctx.request_repaint();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    item.engine.execute(&flag, &mut |event| {
                        let _ = tx.send(Msg::Event(event));
                        ctx.request_repaint();
                    })
                }))
                .unwrap_or_else(|_| {
                    Err(Error::Internal(
                        "the splitter crashed; see the crash report".to_owned(),
                    ))
                });

                let _ = tx.send(Msg::Done(id, result));
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::Finished);
            ctx.request_repaint();
        });

        Self {
            rx,
            cancel,
            dry_run,
            total,
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Delivers pending messages to `sink`. Returns `true` while the job is still running.
    pub fn drain(&self, mut sink: impl FnMut(Msg)) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    let done = matches!(msg, Msg::Finished);
                    sink(msg);
                    if done {
                        return false;
                    }
                }
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => {
                    sink(Msg::WorkerStopped);
                    return false;
                }
            }
        }
    }
}
