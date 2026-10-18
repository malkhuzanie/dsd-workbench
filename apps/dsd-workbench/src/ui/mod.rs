//! Graphical front-end: a batch of CUE sheets, a grid of albums, and an editable tracks grid.

mod albums;
mod analysis_job;
mod cells;
mod detail;
mod fields;
mod fonts;
mod listen;
mod settings_window;
mod theme;
mod verify_job;
mod verify_view;
mod wave_view;
mod waveform;
mod widgets;
mod worker;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Margin, RichText};

use crate::audio::Player;
use crate::diag;
use crate::disk::{self, Need};
use crate::engine::SplitEngine;
use crate::error::Error;
use crate::history::History;
use crate::meter::RunProgress;
use crate::plan::SplitPlan;
use crate::project::{self, OutputMode, Project, RunState};
use crate::project_file;
use crate::selection::TrackSelection;
use crate::settings::AppSettings;
use crate::snapshot::TagCache;
use listen::ListenState;
use theme::Palette;
use verify_job::{VerifyItem, VerifyJob, VerifyMsg};
use worker::{BatchItem, Job, Msg};

const MAX_LOG_ENTRIES: usize = 1000;
const DISK_REFRESH: Duration = Duration::from_secs(2);

type Report = HashMap<u64, Vec<String>>;

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "DSD Workbench",
        options,
        Box::new(|cc| Ok(Box::new(DsfToolApp::new(cc)))),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    Tracks,
    Waveform,
    Album,
    Verify,
}

enum Notice {
    Info(String),
    Error(String),
}

/// The progress bookkeeping of a split run plus when it began.
struct RunClock {
    progress: RunProgress,
    started: Instant,
}

impl RunClock {
    fn new(progress: RunProgress) -> Self {
        Self {
            progress,
            started: Instant::now(),
        }
    }
}

#[derive(Default)]
struct DiskView {
    checked: Option<Instant>,
    text: String,
    blocking: bool,
}

struct DsfToolApp {
    project: Project,
    selected: Option<u64>,
    tab: DetailTab,
    dry_run: bool,
    overwrite: bool,
    settings: AppSettings,
    settings_open: bool,
    settings_dirty: bool,
    cache: TagCache,
    job: Option<Job>,
    pending_verify: bool,
    verify_job: Option<VerifyJob>,
    verify_view: verify_view::ViewState,
    // Counters for the current split run.
    finished: usize,
    succeeded: usize,
    failed: usize,
    cancelled: bool,
    progress: f32,
    /// Throughput and time left for the current split run (not for dry runs).
    run: Option<RunClock>,
    // Counters for the current verification run.
    verify_finished: usize,
    verify_clean: usize,
    verify_bad: usize,
    verify_cancelled: bool,
    verify_progress: f32,
    log: Vec<String>,
    log_open: bool,
    notice: Option<Notice>,
    disk: DiskView,
    player: Player,
    listen: ListenState,
    selection: TrackSelection,
    wave: wave_view::WaveState,
    /// Undo history per album, and the clock it is measured on.
    histories: HashMap<u64, History<SplitPlan>>,
    started: Instant,
    /// The project file last opened or saved, offered again as the default save location.
    last_project: Option<PathBuf>,
}

impl DsfToolApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = AppSettings::load();
        theme::install(&cc.egui_ctx);
        theme::select(&cc.egui_ctx, settings.dark);
        fonts::install_fallbacks(&cc.egui_ctx);

        let mut player = Player::default();
        player.set_settings(settings.audio);

        let crashes = diag::pending_crash_logs();
        let notice = (!crashes.is_empty()).then(|| {
            Notice::Error(format!(
                "The previous session ended unexpectedly. Crash report(s) are in {}",
                diag::dir().display()
            ))
        });
        for path in &crashes {
            let _ = diag::mark_seen(path);
        }

        Self {
            project: Project::default(),
            selected: None,
            tab: DetailTab::Tracks,
            dry_run: false,
            overwrite: false,
            settings,
            settings_open: false,
            settings_dirty: false,
            cache: TagCache::default(),
            job: None,
            pending_verify: false,
            verify_job: None,
            verify_view: verify_view::ViewState::default(),
            finished: 0,
            succeeded: 0,
            failed: 0,
            cancelled: false,
            progress: 0.0,
            run: None,
            verify_finished: 0,
            verify_clean: 0,
            verify_bad: 0,
            verify_cancelled: false,
            verify_progress: 0.0,
            log: Vec::new(),
            log_open: false,
            notice,
            disk: DiskView::default(),
            player,
            listen: ListenState::default(),
            selection: TrackSelection::default(),
            wave: wave_view::WaveState::default(),
            histories: HashMap::new(),
            started: Instant::now(),
            last_project: None,
        }
    }

    // ---- actions ---------------------------------------------------------

    /// Adds the albums of a saved project, with their edits.
    fn open_project(&mut self, path: &Path) {
        let first_new = self.project.entries.len();
        self.notice = Some(match project_file::load(&mut self.project, path) {
            Ok(report) => {
                if let Some(entry) = self.project.entries.get(first_new) {
                    self.selected = Some(entry.id);
                }
                self.last_project = Some(path.to_path_buf());
                let mut text = format!(
                    "Opened {} album(s) from {}.",
                    report.added,
                    file_label(path)
                );
                if !report.problems.is_empty() {
                    let shown = report.problems.iter().take(2).cloned().collect::<Vec<_>>();
                    text.push_str(&format!(" {}", shown.join(" · ")));
                    if report.problems.len() > 2 {
                        text.push_str(&format!(" (+{} more)", report.problems.len() - 2));
                    }
                    Notice::Error(text)
                } else {
                    Notice::Info(text)
                }
            }
            Err(err) => Notice::Error(format!("{}: {err}", file_label(path))),
        });
    }

    /// Asks where to save, then writes the project.
    fn save_project(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("DSD Workbench project", &[project_file::EXTENSION])
            .set_file_name(format!("albums.{}", project_file::EXTENSION));
        if let Some(previous) = self.last_project.as_deref() {
            if let Some(dir) = previous.parent() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(name) = previous.file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
        }
        let Some(mut path) = dialog.save_file() else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension(project_file::EXTENSION);
        }
        self.notice = Some(match project_file::save(&self.project, &path) {
            Ok(()) => {
                self.last_project = Some(path.clone());
                Notice::Info(format!("Saved {}.", file_label(&path)))
            }
            Err(err) => Notice::Error(format!("Could not save the project: {err}")),
        });
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        let mut cues = Vec::new();
        let mut audio = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            let has_extension = |wanted: &str| {
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(wanted))
            };
            if has_extension(project_file::EXTENSION) {
                self.open_project(&path);
                return;
            }
            if path.is_dir() {
                cues.extend(project::find_cues(&path));
            } else if project::is_cue(&path) {
                cues.push(path);
            } else if has_extension("dsf") || has_extension("dff") {
                audio.push(path);
            } else {
                errors.push(format!(
                    "{}: not a CUE sheet or DSD file",
                    file_label(&path)
                ));
            }
        }

        let mut added = 0usize;
        for path in audio {
            let file = project::AudioFile {
                path: path.clone(),
                cuts: Vec::new(),
            };
            match self.project.add_audio(std::slice::from_ref(&file)) {
                Ok(id) => {
                    if added == 0 {
                        self.selected = Some(id);
                        // A file without a CUE sheet is cut on the waveform.
                        self.tab = DetailTab::Waveform;
                    }
                    added += 1;
                }
                Err(Error::AlreadyAdded) => {}
                Err(err) => errors.push(format!("{}: {}", file_label(&path), err.with_advice())),
            }
        }
        for cue in cues {
            match self.project.add(&cue) {
                Ok(id) => {
                    if added == 0 {
                        self.selected = Some(id);
                    }
                    added += 1;
                }
                Err(Error::AlreadyAdded) => {}
                Err(err) => errors.push(format!("{}: {}", file_label(&cue), err.with_advice())),
            }
        }

        self.notice = if errors.is_empty() {
            Some(Notice::Info(format!("Added {added} album(s).")))
        } else {
            let shown = errors[..errors.len().min(2)].join(" · ");
            for e in &errors {
                self.push_log(e.clone());
            }
            Some(Notice::Error(format!(
                "Added {added}, {} failed: {shown}",
                errors.len()
            )))
        };
        self.disk.checked = None;
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        if self.job.is_some() || self.verify_job.is_some() {
            return;
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_paths(dropped);
        }
    }

    fn needs(&self) -> Vec<Need> {
        self.project
            .entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| Need {
                name: e.display_name(),
                dir: e.output_dir(),
                bytes: e.estimated_bytes(),
            })
            .collect()
    }

    fn refresh_disk(&mut self) {
        if self.project.entries.is_empty() {
            self.disk = DiskView::default();
            return;
        }
        if self
            .disk
            .checked
            .is_some_and(|t| t.elapsed() < DISK_REFRESH)
        {
            return;
        }
        let report = disk::preflight(&self.needs(), false);
        self.disk.checked = Some(Instant::now());
        self.disk.blocking = !report.errors.is_empty();
        self.disk.text = match report.errors.first() {
            Some(error) => error.clone(),
            None => match report.lowest_free() {
                Some(free) => format!(
                    "Needs about {} · {} free",
                    disk::format_bytes(report.required_total()),
                    disk::format_bytes(free)
                ),
                None => String::new(),
            },
        };
    }

    fn start(&mut self, ctx: &egui::Context) {
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
        self.finished = 0;
        self.succeeded = 0;
        self.failed = 0;
        self.cancelled = false;
        self.progress = 0.0;
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

    fn start_verify(&mut self, ctx: &egui::Context, ids: Vec<u64>) {
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
        self.verify_finished = 0;
        self.verify_clean = 0;
        self.verify_bad = 0;
        self.verify_cancelled = false;
        self.verify_progress = 0.0;
        self.notice = Some(Notice::Info("Verifying…".to_owned()));
        self.verify_job = Some(VerifyJob::spawn(ctx, items));
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
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
        let mut summary = format!("{} album(s) {verb}", self.succeeded);
        if self.failed > 0 {
            summary.push_str(&format!(", {} failed", self.failed));
        }
        if self.cancelled {
            summary.push_str(", cancelled");
        }
        summary.push('.');
        if !matches!(self.notice, Some(Notice::Error(_))) || self.failed > 0 {
            self.notice = Some(if self.failed > 0 {
                Notice::Error(summary)
            } else {
                Notice::Info(summary)
            });
        }

        let verify_ids: Vec<u64> = if self.pending_verify && self.succeeded > 0 && !self.cancelled {
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
                    self.progress =
                        weighted.unwrap_or((self.finished as f32 + fraction) / total as f32);
                }
                if event.is_log_line() {
                    self.push_log(event.to_string());
                }
            }
            Msg::Done(id, result) => {
                self.finished += 1;
                self.progress = match &mut self.run {
                    Some(run) => {
                        run.progress.finish_album(run.started.elapsed());
                        run.progress.fraction()
                    }
                    None => None,
                }
                .unwrap_or(self.finished as f32 / total as f32);
                let state = match result {
                    Ok(()) => {
                        self.succeeded += 1;
                        if dry_run {
                            RunState::Previewed
                        } else {
                            RunState::Done
                        }
                    }
                    Err(Error::Cancelled) => {
                        self.cancelled = true;
                        RunState::Cancelled
                    }
                    Err(err) => {
                        self.failed += 1;
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

    fn poll_verify(&mut self) {
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
                    self.verify_progress = (self.verify_finished as f32 + fraction) / total as f32;
                }
                VerifyMsg::Done(id, result) => {
                    self.verify_finished += 1;
                    self.verify_progress = self.verify_finished as f32 / total as f32;
                    match result {
                        Ok(report) => {
                            if report.is_clean() {
                                self.verify_clean += 1;
                                self.push_log(format!(
                                    "all {} tracks are bit-identical to the source",
                                    report.tracks.len()
                                ));
                            } else {
                                self.verify_bad += 1;
                                for t in report.tracks.iter().filter(|t| {
                                    t.verdict != crate::engine::Verdict::Identical
                                        || !t.issues.is_empty()
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
                        Err(Error::Cancelled) => self.verify_cancelled = true,
                        Err(err) => {
                            self.verify_bad += 1;
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
        let mut summary = format!("Verification: {} album(s) identical", self.verify_clean);
        if self.verify_bad > 0 {
            summary.push_str(&format!(", {} with differences or notes", self.verify_bad));
        }
        if self.verify_cancelled {
            summary.push_str(", cancelled");
        }
        summary.push('.');
        self.notice = Some(if self.verify_bad > 0 {
            Notice::Error(summary)
        } else {
            Notice::Info(summary)
        });
    }

    fn push_log(&mut self, line: String) {
        diag::log(&line);
        if self.log.len() >= MAX_LOG_ENTRIES {
            self.log.remove(0);
        }
        self.log.push(line);
    }

    fn ensure_selection(&mut self) {
        let valid = self
            .selected
            .is_some_and(|id| self.project.entries.iter().any(|e| e.id == id));
        if !valid {
            self.selected = self.project.entries.first().map(|e| e.id);
        }
    }

    /// Records every album's edits so they can be undone.
    fn track_history(&mut self) {
        let now = self.started.elapsed();
        for entry in &self.project.entries {
            self.histories
                .entry(entry.id)
                .or_insert_with(|| History::new(entry.plan.clone()))
                .observe(&entry.plan, now);
        }
        let project = &self.project;
        self.histories
            .retain(|id, _| project.entries.iter().any(|e| e.id == *id));
    }

    fn can_undo(&self) -> bool {
        self.selected
            .and_then(|id| self.histories.get(&id))
            .is_some_and(History::can_undo)
    }

    fn can_redo(&self) -> bool {
        self.selected
            .and_then(|id| self.histories.get(&id))
            .is_some_and(History::can_redo)
    }

    /// Steps the selected album's edits back (`forward == false`) or forward.
    fn step_history(&mut self, forward: bool) {
        let Some(id) = self.selected else { return };
        let (Some(entry), Some(history)) = (self.project.get_mut(id), self.histories.get_mut(&id))
        else {
            return;
        };
        let restored = if forward {
            history.redo(&entry.plan)
        } else {
            history.undo(&entry.plan)
        };
        if let Some(plan) = restored {
            entry.plan = plan;
        }
    }

    /// Keyboard shortcuts: Ctrl+Z / Ctrl+Y (or Ctrl+Shift+Z) undo and redo, Ctrl+O adds
    /// CUE files, Ctrl+S saves the project.
    fn handle_shortcuts(&mut self, ctx: &egui::Context, busy: bool) {
        use egui::{Key, Modifiers};
        if busy {
            return;
        }
        // A focused text box has its own undo, so leave Z and Y to it.
        let typing = ctx.wants_keyboard_input();
        let (redo, undo, open, save) = ctx.input_mut(|i| {
            let redo = !typing
                && (i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                    || i.consume_key(Modifiers::COMMAND, Key::Y));
            let undo = !typing && i.consume_key(Modifiers::COMMAND, Key::Z);
            let open = i.consume_key(Modifiers::COMMAND, Key::O);
            let save = i.consume_key(Modifiers::COMMAND, Key::S);
            (redo, undo, open, save)
        });
        if undo {
            self.step_history(false);
        }
        if redo {
            self.step_history(true);
        }
        if open {
            self.pick_cue_files();
        }
        if save && !self.project.entries.is_empty() {
            self.save_project();
        }
    }

    fn pick_cue_files(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("CUE sheet", &["cue"])
            .pick_files()
        {
            self.add_paths(paths);
        }
    }

    fn sync_settings(&mut self) {
        let wanted = self.settings.plan_options();
        if self.project.options != wanted {
            self.project.options = wanted;
        }
        // The player bar's Audio menu edits the player directly.
        if self.player.settings() != self.settings.audio {
            self.settings.audio = self.player.settings();
            self.settings_dirty = true;
        }
        if self.settings_dirty {
            self.settings_dirty = false;
            if let Err(err) = self.settings.save() {
                diag::log(format!("could not save settings: {err}"));
            }
        }
    }

    // ---- layout ----------------------------------------------------------

    fn toolbar(&mut self, ctx: &egui::Context, p: Palette, busy: bool) {
        let has_entries = !self.project.entries.is_empty();
        let (mut add_files, mut add_folder, mut clear) = (false, false, false);
        let (mut beside_all, mut under_folder) = (false, false);
        let (mut open_project, mut save_project) = (false, false);
        let mut add_audio = false;
        let (mut undo, mut redo) = (false, false);
        let (can_undo, can_redo) = (self.can_undo(), self.can_redo());

        egui::TopBottomPanel::top("toolbar")
            .show_separator_line(false)
            .frame(theme::gutter(Margin {
                left: 12,
                right: 12,
                top: 12,
                bottom: 6,
            }))
            .show(ctx, |ui| {
                theme::card(&p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("DSD Workbench").size(20.0).strong());
                        ui.add_space(12.0);
                        ui.add_enabled_ui(!busy, |ui| {
                            if ui
                                .button("Add CUE files…")
                                .on_hover_text("Ctrl+O")
                                .clicked()
                            {
                                add_files = true;
                            }
                            if ui.button("Add folder…").clicked() {
                                add_folder = true;
                            }
                            if ui
                                .button("Add audio file…")
                                .on_hover_text(
                                    "Split a .dsf or .dff that has no CUE sheet, by silence or by hand",
                                )
                                .clicked()
                            {
                                add_audio = true;
                            }
                            ui.separator();
                            if ui
                                .add_enabled(can_undo, egui::Button::new("Undo"))
                                .on_hover_text("Ctrl+Z")
                                .clicked()
                            {
                                undo = true;
                            }
                            if ui
                                .add_enabled(can_redo, egui::Button::new("Redo"))
                                .on_hover_text("Ctrl+Y or Ctrl+Shift+Z")
                                .clicked()
                            {
                                redo = true;
                            }
                            ui.separator();
                            ui.menu_button("Project", |ui| {
                                if ui.button("Open project…").clicked() {
                                    open_project = true;
                                    ui.close_menu();
                                }
                                if ui
                                    .add_enabled(has_entries, egui::Button::new("Save project…"))
                                    .clicked()
                                {
                                    save_project = true;
                                    ui.close_menu();
                                }
                            });
                            ui.add_enabled_ui(has_entries, |ui| {
                                ui.menu_button("Output for all", |ui| {
                                    if ui.button("Beside each CUE sheet").clicked() {
                                        beside_all = true;
                                        ui.close_menu();
                                    }
                                    if ui
                                        .button("Choose a folder (one subfolder per album)…")
                                        .clicked()
                                    {
                                        under_folder = true;
                                        ui.close_menu();
                                    }
                                });
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Settings").clicked() {
                                self.settings_open = true;
                            }
                            let label = if self.settings.dark { "Light" } else { "Dark" };
                            if ui.button(label).clicked() {
                                self.settings.dark = !self.settings.dark;
                                theme::select(ui.ctx(), self.settings.dark);
                                self.settings_dirty = true;
                            }
                            ui.separator();
                            if ui
                                .add_enabled(has_entries && !busy, egui::Button::new("Clear all"))
                                .clicked()
                            {
                                clear = true;
                            }
                        });
                    });
                });
            });

        if add_files {
            self.pick_cue_files();
        }
        if undo {
            self.step_history(false);
        }
        if redo {
            self.step_history(true);
        }
        if add_folder && let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.add_paths(vec![dir]);
        }
        if add_audio
            && let Some(paths) = rfd::FileDialog::new()
                .add_filter("DSD audio", &["dsf", "dff"])
                .pick_files()
        {
            self.add_paths(paths);
        }
        if open_project
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("DSD Workbench project", &[project_file::EXTENSION])
                .pick_file()
        {
            self.open_project(&path);
        }
        if save_project {
            self.save_project();
        }
        if beside_all {
            self.project.reset_outputs();
            self.disk.checked = None;
        }
        if under_folder && let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.project.set_output_under(&dir);
            self.disk.checked = None;
        }
        if clear {
            self.player.stop();
            self.project.clear();
            self.selected = None;
            self.cache.clear();
            self.notice = None;
            self.disk = DiskView::default();
        }
    }

    fn action_bar(&mut self, ctx: &egui::Context, p: Palette, report: &Report, busy: bool) {
        let total = self.project.entries.len();
        let enabled: Vec<u64> = self
            .project
            .entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.id)
            .collect();
        let selected = enabled.len();
        let tracks: usize = self
            .project
            .entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.plan.tracks.len())
            .sum();
        let blocked = report.len();
        let can_run = !busy && selected > 0 && blocked == 0 && !self.disk.blocking;
        let can_verify = !busy && selected > 0 && blocked == 0;
        let (mut start, mut verify) = (false, false);

        let log_open = self.log_open;
        let mut panel =
            egui::TopBottomPanel::bottom(if log_open { "actions_log" } else { "actions" })
                .show_separator_line(false)
                .frame(theme::gutter(Margin {
                    left: 12,
                    right: 12,
                    top: 8,
                    bottom: 12,
                }));
        if log_open {
            panel = panel
                .resizable(true)
                .default_height(240.0)
                .min_height(140.0)
                .max_height(ctx.screen_rect().height() * 0.6);
        }

        panel.show(ctx, |ui| {
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if log_open {
                    ui.set_min_size(ui.available_size());
                }
                ui.horizontal_wrapped(|ui| {
                    let label = if self.dry_run {
                        format!("Preview {selected} album(s)")
                    } else {
                        format!("Split {selected} album(s)")
                    };
                    if widgets::primary_button(ui, &p, can_run, &label).clicked() {
                        start = true;
                    }
                    if ui
                        .add_enabled(
                            can_verify,
                            egui::Button::new(format!("Verify {selected} album(s)")),
                        )
                        .on_hover_text("Compare existing output files with the source, bit by bit")
                        .clicked()
                    {
                        verify = true;
                    }
                    if busy && ui.button("Cancel").clicked() {
                        if let Some(job) = &self.job {
                            job.cancel();
                        }
                        if let Some(job) = &self.verify_job {
                            job.cancel();
                        }
                    }
                    ui.separator();
                    ui.add_enabled(!busy, egui::Checkbox::new(&mut self.dry_run, "Dry run"));
                    ui.add_enabled(
                        !busy,
                        egui::Checkbox::new(&mut self.overwrite, "Overwrite existing files"),
                    );
                    ui.checkbox(&mut self.log_open, "Show log");
                    ui.separator();
                    if busy {
                        let value = if self.verify_job.is_some() {
                            self.verify_progress
                        } else {
                            self.progress
                        };
                        ui.add(egui::ProgressBar::new(value).desired_width(180.0));
                        if self.verify_job.is_none()
                            && let Some(run) = &self.run
                        {
                            ui.label(RichText::new(run.progress.describe()).color(p.muted));
                        }
                    }

                    ui.label(
                        RichText::new(format!("{selected} of {total} selected · {tracks} tracks"))
                            .color(p.muted),
                    );
                    if blocked > 0 {
                        ui.label(
                            RichText::new(format!("{blocked} need attention")).color(p.danger),
                        );
                    }
                    if !self.disk.text.is_empty() {
                        let color = if self.disk.blocking {
                            p.danger
                        } else {
                            p.muted
                        };
                        ui.label(RichText::new(&self.disk.text).color(color));
                    }
                    match &self.notice {
                        Some(Notice::Info(msg)) => {
                            ui.label(RichText::new(msg).color(p.muted));
                        }
                        Some(Notice::Error(msg)) => {
                            ui.label(RichText::new(msg).color(p.danger));
                        }
                        None => {}
                    }
                });
                if log_open {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for entry in &self.log {
                                ui.monospace(entry);
                            }
                        });
                }
            });
        });

        if start {
            self.start(ctx);
        }
        if verify {
            self.start_verify(ctx, enabled);
        }
    }

    fn albums_panel(&mut self, ctx: &egui::Context, p: Palette, report: &Report, busy: bool) {
        if self.project.entries.is_empty() {
            return;
        }
        let mut action = None;

        egui::TopBottomPanel::top("albums")
            .resizable(true)
            .default_height(230.0)
            .min_height(110.0)
            .max_height(ctx.screen_rect().height() * 0.55)
            .show_separator_line(false)
            .frame(theme::gutter(Margin {
                left: 12,
                right: 12,
                top: 6,
                bottom: 6,
            }))
            .show(ctx, |ui| {
                theme::card(&p).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.add_enabled_ui(!busy, |ui| {
                        action = albums::show(
                            ui,
                            &p,
                            &mut self.project.entries,
                            &mut self.selected,
                            report,
                        );
                    });
                });
            });

        match action {
            Some(albums::Action::Remove(id)) => {
                self.player.stop();
                self.project.remove(id);
                self.cache.clear();
                self.disk.checked = None;
            }
            Some(albums::Action::PickOutput(id)) => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    if let Some(entry) = self.project.get_mut(id) {
                        entry.output = OutputMode::Custom(dir.to_string_lossy().into_owned());
                    }
                    self.disk.checked = None;
                }
            }
            None => {}
        }
    }

    fn central(&mut self, ctx: &egui::Context, p: Palette, report: &Report, busy: bool) {
        egui::CentralPanel::default()
            .frame(theme::gutter(Margin {
                left: 12,
                right: 12,
                top: 6,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                theme::card(&p)
                    .inner_margin(Margin::same(20))
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);

                        let selected = self.selected;
                        let plan_options = self.settings.plan_options();
                        match selected.and_then(|id| self.project.get_mut(id)) {
                            Some(entry) => {
                                let problems =
                                    report.get(&entry.id).map(Vec::as_slice).unwrap_or(&[]);
                                ui.add_enabled_ui(!busy, |ui| {
                                    let mut cx = detail::Context {
                                        p: &p,
                                        cache: &mut self.cache,
                                        tab: &mut self.tab,
                                        overwrite: self.overwrite,
                                        problems,
                                        player: &mut self.player,
                                        listen: &mut self.listen,
                                        verify: &mut self.verify_view,
                                        selection: &mut self.selection,
                                        wave: &mut self.wave,
                                        lookup: &mut self.settings.lookup,
                                        busy,
                                        options: &plan_options,
                                    };
                                    detail::show(ui, &mut cx, entry);
                                });
                            }
                            None => welcome(ui, &p),
                        }
                    });
            });

        if self.verify_view.request {
            self.verify_view.request = false;
            if let Some(id) = self.selected {
                self.start_verify(ctx, vec![id]);
            }
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context, p: Palette) {
        if !self.settings_open {
            return;
        }
        let changed = settings_window::show(
            ctx,
            &p,
            &mut self.settings_open,
            &mut self.settings,
            &mut self.player,
        );
        if changed {
            self.settings_dirty = true;
        }
    }
}

impl eframe::App for DsfToolApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        // Panels are transparent, so this is the "desk" visible in the gutters.
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_job(ctx);
        self.poll_verify();
        if let Some(err) = self.player.take_error() {
            self.notice = Some(Notice::Error(err));
        }
        if let Some(path) = diag::take_crash() {
            self.notice = Some(Notice::Error(format!(
                "A background task crashed. Report saved to {}",
                path.display()
            )));
        }
        if self.player.is_active() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        self.handle_drops(ctx);
        self.ensure_selection();
        self.refresh_disk();

        let palette = Palette::of(self.settings.dark);
        let report = self.project.validate();
        let busy = self.job.is_some() || self.verify_job.is_some();

        self.handle_shortcuts(ctx, busy);
        self.toolbar(ctx, palette, busy);
        self.action_bar(ctx, palette, &report, busy);
        self.albums_panel(ctx, palette, &report, busy);
        let lookup_before = self.settings.lookup.clone();
        self.central(ctx, palette, &report, busy);
        if self.settings.lookup != lookup_before {
            self.settings_dirty = true;
        }
        self.settings_window(ctx, palette);
        self.sync_settings();
        self.track_history();
    }
}

/// The steps shown on the empty screen, in order. `(heading, explanation)`.
const WELCOME_STEPS: [(&str, &str); 5] = [
    (
        "Add your albums",
        "Drop .cue files or folders anywhere in this window, or use “Add CUE files…” above. A single .dsf or .dff with no CUE sheet works too: you cut it on a waveform.",
    ),
    (
        "Review the tracks",
        "Check the titles, artists and start times. Tick tracks to edit them together, and use “Album & output” for album tags, cover art and the folder.",
    ),
    (
        "Listen to a cut",
        "The buttons on each track play the start, the end, or either side of a cut, so you can hear the join before anything is written.",
    ),
    (
        "Split",
        "Press the Split button in the bar at the bottom. Your source files are never changed, and nothing is overwritten unless you allow it.",
    ),
    (
        "Check the result",
        "Verification reads every new file back and compares it with the source, bit by bit. Keep it on until you trust your setup.",
    ),
];

fn welcome(ui: &mut egui::Ui, p: &Palette) {
    widgets::title(ui, "Welcome");
    widgets::caption(
        ui,
        p,
        "Split whole-side DSD files into tracks without touching the audio. Nothing leaves your computer.",
    );
    ui.add_space(8.0);
    for (number, (heading, text)) in WELCOME_STEPS.iter().enumerate() {
        ui.label(RichText::new(format!("{}. {heading}", number + 1)).strong());
        widgets::caption(ui, p, text);
        ui.add_space(4.0);
    }
    widgets::caption(
        ui,
        p,
        "Shortcuts: Ctrl+O add files · Ctrl+S save project · Ctrl+Z undo · Ctrl+Y redo.",
    );
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
