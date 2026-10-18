//! Runs the loudness analysis of one source file on a background thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui;

use crate::analysis::{self, Envelope};
use crate::error::{Error, Result};
use crate::source;

pub enum Msg {
    /// Fraction of the file read so far.
    Progress(f32),
    Done(Result<Envelope>),
}

pub struct AnalysisJob {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
}

impl AnalysisJob {
    pub fn spawn(ctx: &egui::Context, path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let ctx = ctx.clone();

        thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut src = source::open(&path)?;
                let mut last = 0u32;
                analysis::analyze(src.as_mut(), &flag, &mut |fraction| {
                    // Roughly 1% steps keep the channel and the repaint rate modest.
                    let percent = (fraction * 100.0) as u32;
                    if percent != last {
                        last = percent;
                        let _ = tx.send(Msg::Progress(fraction));
                        ctx.request_repaint();
                    }
                })
            }))
            .unwrap_or_else(|_| {
                Err(Error::Internal(
                    "the analysis crashed; see the crash report".to_owned(),
                ))
            });
            let _ = tx.send(Msg::Done(outcome)); // the receiver is gone if the app closed
            ctx.request_repaint();
        });

        Self { rx, cancel }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Pending messages, and whether the job may still send more.
    pub fn poll(&self) -> (Vec<Msg>, bool) {
        let mut messages = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    let finished = matches!(msg, Msg::Done(_));
                    messages.push(msg);
                    if finished {
                        return (messages, false);
                    }
                }
                Err(TryRecvError::Empty) => return (messages, true),
                Err(TryRecvError::Disconnected) => return (messages, false),
            }
        }
    }
}
