//! Graphical front-end: a batch of CUE sheets, a grid of albums, and an editable tracks grid.

mod albums;
mod analysis_job;
mod cells;
mod chrome;
mod detail;
mod fields;
mod fonts;
mod listen;
mod run;
mod sections;
mod settings_window;
#[cfg(test)]
mod snapshot;
mod theme;
mod verify_job;
mod verify_view;
mod wave_view;
mod waveform;
mod welcome;
mod widgets;
mod worker;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Margin};

use crate::audio::Player;
use crate::diag;
use crate::disk::{self, Need};
use crate::error::Error;
use crate::history::History;
use crate::locate;
use crate::meter::RunProgress;
use crate::plan::SplitPlan;
use crate::project::{self, Project};
use crate::project_file;
use crate::selection::TrackSelection;
use crate::settings::AppSettings;
use crate::snapshot::TagCache;
use listen::ListenState;
use theme::Palette;
use verify_job::VerifyJob;
use worker::Job;

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

impl Notice {
    /// An error notice when `error`, else an informational one.
    fn new(error: bool, text: String) -> Self {
        if error {
            Self::Error(text)
        } else {
            Self::Info(text)
        }
    }
}

/// How far a batch of albums (a split run or a verification) has got.
#[derive(Default)]
struct Tally {
    /// Albums done, however they ended.
    finished: usize,
    /// Albums that went well (split, or found identical).
    ok: usize,
    /// Albums that failed (split), or differ or could not be checked (verification).
    bad: usize,
    cancelled: bool,
    /// 0 to 1 across the whole batch.
    progress: f32,
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
    /// The current split run, and the current verification run.
    split: Tally,
    check: Tally,
    /// Throughput and time left for the current split run (not for dry runs).
    run: Option<RunClock>,
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
    /// The last validation and when it ran; see [`Self::validated`].
    report: Report,
    validated: Option<Instant>,
    /// Whether the "remove all albums?" question is showing.
    confirm_clear: bool,
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
            split: Tally::default(),
            check: Tally::default(),
            run: None,
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
            report: Report::new(),
            validated: None,
            confirm_clear: false,
        }
    }

    /// Validation checks every track of every album and touches the file system, so it is not
    /// repeated every frame: it runs after any click, key press or drop, and otherwise at most
    /// four times a second (which still notices files appearing or going missing).
    fn validated(&mut self, ctx: &egui::Context) -> Report {
        const STALE: Duration = Duration::from_millis(250);
        let touched = ctx.input(|i| {
            i.events.iter().any(|e| {
                !matches!(
                    e,
                    egui::Event::PointerMoved(_)
                        | egui::Event::MouseMoved(_)
                        | egui::Event::PointerGone
                )
            }) || !i.raw.dropped_files.is_empty()
        });
        if touched || self.validated.is_none_or(|at| at.elapsed() >= STALE) {
            self.report = self.project.validate();
            self.validated = Some(Instant::now());
        }
        self.report.clone()
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
                }
                Notice::new(!report.problems.is_empty(), text)
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
            } else if locate::is_dsd_path(&path) {
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
        let busy = self.job.is_some() || self.verify_job.is_some();
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            // Files are being dragged over the window: say what will happen if they are dropped.
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(150));
            let text = if busy {
                "A run is in progress: wait for it to finish before adding albums"
            } else {
                "Drop CUE sheets, DSD files or folders to add them"
            };
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(22.0),
                egui::Color32::WHITE,
            );
        }
        if busy {
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
                                        collapsible: self.settings.collapsible_sections,
                                        options: &plan_options,
                                    };
                                    detail::show(ui, &mut cx, entry);
                                });
                            }
                            None => welcome::show(ui, &p),
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
        let report = self.validated(ctx);
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
        self.confirm_clear_window(ctx);
        self.sync_settings();
        self.track_history();
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
