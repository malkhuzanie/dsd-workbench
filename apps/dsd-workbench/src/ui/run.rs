//! Starting split and verification runs and folding their progress messages into the app.

use eframe::egui;

use super::verify_job::{VerifyItem, VerifyJob, VerifyMsg};
use super::worker::{BatchItem, Job, Msg};
use super::{DetailTab, DsfToolApp, Notice, RunClock, Tally};
use crate::disk;
use crate::engine::{SplitEngine, Verdict};
use crate::error::Error;
use crate::meter::RunProgress;
use crate::project::RunState;

impl DsfToolApp {
    pub(super) fn start(&mut self, ctx: &egui::Context) {
        if !self.dry_run {
            let report = disk::preflight(&self.needs(), true);
            if !report.errors.is_empty() {
                for error in &report.errors {
                    self.push_log(format!("ERROR: {error}"));
                }
                self.notice = Some(Notice::Error(report.errors.join(" ")));
                return;
            }
        }

        let items: Vec<BatchItem> = self
            .project
            .entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| BatchItem {
                id: e.id,
                engine: SplitEngine::new(
                    e.base_dir.clone(),
                    e.plan.clone(),
                    e.output_dir(),
                    self.dry_run,
                )
                .with_overwrite(self.overwrite),
            })
            .collect();
        if items.is_empty() {
            return;
        }
        self.player.stop();
        for entry in self.project.entries.iter_mut().filter(|e| e.enabled) {
            entry.state = RunState::Queued;
        }

        self.log.clear();
        self.split = Tally::default();
        self.run = (!self.dry_run).then(|| {
            RunClock::new(RunProgress::new(
                self.project
                    .entries
                    .iter()
                    .filter(|e| e.enabled)
                    .map(|e| (e.id, e.estimated_bytes())),
            ))
        });
        self.pending_verify = !self.dry_run && self.settings.verify_after_split;
        self.notice = Some(Notice::Info("Working…".to_owned()));
        self.job = Some(Job::spawn(ctx, items, self.dry_run));
    }

    pub(super) fn start_verify(&mut self, ctx: &egui::Context, ids: Vec<u64>) {
        let items: Vec<VerifyItem> = self
            .project
            .entries
            .iter()
            .filter(|e| ids.contains(&e.id))
            .map(|e| VerifyItem {
                id: e.id,
                name: e.display_name(),
                base_dir: e.base_dir.clone(),
                plan: e.plan.clone(),
                output_dir: e.output_dir(),
            })
            .collect();
        if items.is_empty() {
            return;
        }
        self.player.stop();
        self.cache.clear(); // tags on disk may have changed
        self.check = Tally::default();
        self.notice = Some(Notice::Info("Verifying…".to_owned()));
        self.verify_job = Some(VerifyJob::spawn(ctx, items));
    }

    pub(super) fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = self.job.as_ref() else { return };
        let (dry_run, total) = (job.dry_run, job.total.max(1));
        let mut messages = Vec::new();
        let running = job.drain(|msg| messages.push(msg));

        for msg in messages {
            self.apply(msg, dry_run, total);
        }
        if running {
            return;
        }

        self.job = None;
        self.run = None;
        for entry in &mut self.project.entries {
            if entry.state == RunState::Queued {
                entry.state = RunState::Idle; // never started (cancelled)
            }
        }
        if !dry_run {
            self.cache.clear(); // files on disk may have changed
        }

        let verb = if dry_run { "previewed" } else { "split" };
        let mut summary = format!("{} album(s) {verb}", self.split.ok);
        if self.split.bad > 0 {
            summary.push_str(&format!(", {} failed", self.split.bad));
        }
        if self.split.cancelled {
            summary.push_str(", cancelled");
        }
        summary.push('.');
        if !matches!(self.notice, Some(Notice::Error(_))) || self.split.bad > 0 {
            self.notice = Some(Notice::new(self.split.bad > 0, summary));
        }

        let verify_ids: Vec<u64> =
            if self.pending_verify && self.split.ok > 0 && !self.split.cancelled {
                self.project
                    .entries
                    .iter()
                    .filter(|e| e.enabled && e.state == RunState::Done)
                    .map(|e| e.id)
                    .collect()
            } else {
                Vec::new()
            };
        self.pending_verify = false;
        if !verify_ids.is_empty() {
            self.start_verify(ctx, verify_ids);
        }
    }

    fn apply(&mut self, msg: Msg, dry_run: bool, total: usize) {
        match msg {
            Msg::Started(id) => {
                let name = self
                    .project
                    .get_mut(id)
                    .map(|e| e.display_name())
                    .unwrap_or_default();
                if let Some(entry) = self.project.get_mut(id) {
                    entry.state = RunState::Running;
                }
                if let Some(run) = &mut self.run {
                    run.progress.begin_album(id);
                }
                self.push_log(format!("== {name} =="));
            }
            Msg::Event(event) => {
                if let Some(fraction) = event.progress() {
                    let weighted = self.run.as_mut().and_then(|run| {
                        run.progress.album_progress(fraction, run.started.elapsed());
                        run.progress.fraction()
                    });
                    self.split.progress =
                        weighted.unwrap_or((self.split.finished as f32 + fraction) / total as f32);
                }
                if event.is_log_line() {
                    self.push_log(event.to_string());
                }
            }
            Msg::Done(id, result) => {
                self.split.finished += 1;
                self.split.progress = match &mut self.run {
                    Some(run) => {
                        run.progress.finish_album(run.started.elapsed());
                        run.progress.fraction()
                    }
                    None => None,
                }
                .unwrap_or(self.split.finished as f32 / total as f32);
                let state = match result {
                    Ok(()) => {
                        self.split.ok += 1;
                        if dry_run {
                            RunState::Previewed
                        } else {
                            RunState::Done
                        }
                    }
                    Err(Error::Cancelled) => {
                        self.split.cancelled = true;
                        RunState::Cancelled
                    }
                    Err(err) => {
                        self.split.bad += 1;
                        self.push_log(format!("ERROR: {err}"));
                        RunState::Failed(err.with_advice())
                    }
                };
                if let Some(entry) = self.project.get_mut(id) {
                    entry.state = state;
                }
            }
            Msg::Finished => {}
            Msg::WorkerStopped => {
                self.notice = Some(Notice::Error(Error::WorkerStopped.to_string()));
            }
        }
    }

    pub(super) fn poll_verify(&mut self) {
        let Some(job) = self.verify_job.as_ref() else {
            return;
        };
        let total = job.total.max(1);
        let mut messages = Vec::new();
        let running = job.drain(|msg| messages.push(msg));

        for msg in messages {
            match msg {
                VerifyMsg::Started(name) => self.push_log(format!("== verifying {name} ==")),
                VerifyMsg::Progress(fraction) => {
                    self.check.progress = (self.check.finished as f32 + fraction) / total as f32;
                }
                VerifyMsg::Done(id, result) => {
                    self.check.finished += 1;
                    self.check.progress = self.check.finished as f32 / total as f32;
                    match result {
                        Ok(report) => {
                            if report.is_clean() {
                                self.check.ok += 1;
                                self.push_log(format!(
                                    "all {} tracks are bit-identical to the source",
                                    report.tracks.len()
                                ));
                            } else {
                                self.check.bad += 1;
                                for t in report.tracks.iter().filter(|t| {
                                    t.verdict != Verdict::Identical || !t.issues.is_empty()
                                }) {
                                    self.push_log(format!(
                                        "track {:02} {}: {}",
                                        t.number,
                                        t.title,
                                        t.note()
                                    ));
                                }
                            }
                            if let Some(entry) = self.project.get_mut(id) {
                                entry.verification = Some(report);
                            }
                        }
                        Err(Error::Cancelled) => self.check.cancelled = true,
                        Err(err) => {
                            self.check.bad += 1;
                            self.push_log(format!("ERROR: verification failed: {err}"));
                        }
                    }
                }
                VerifyMsg::Finished => {}
                VerifyMsg::WorkerStopped => {
                    self.notice = Some(Notice::Error(Error::WorkerStopped.to_string()));
                }
            }
        }
        if running {
            return;
        }

        self.verify_job = None;
        self.tab = DetailTab::Verify;
        let mut summary = format!("Verification: {} album(s) identical", self.check.ok);
        if self.check.bad > 0 {
            summary.push_str(&format!(", {} with differences or notes", self.check.bad));
        }
        if self.check.cancelled {
            summary.push_str(", cancelled");
        }
        summary.push('.');
        self.notice = Some(Notice::new(self.check.bad > 0, summary));
    }
}
