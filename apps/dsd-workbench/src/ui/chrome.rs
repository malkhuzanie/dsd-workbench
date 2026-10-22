//! The frame around the album view: the toolbar on top, the albums list, and the action bar at
//! the bottom with its notice line and log.

use eframe::egui::{self, Margin, RichText};

use super::theme::{self, Palette};
use super::{DiskView, DsfToolApp, Notice, Report, albums, widgets};
use crate::project::OutputMode;
use crate::project_file;

impl DsfToolApp {
    pub(super) fn toolbar(&mut self, ctx: &egui::Context, p: Palette, busy: bool) {
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
            self.confirm_clear = true;
        }
    }

    /// Removing every album also discards their edits and undo history, so it asks first.
    pub(super) fn confirm_clear_window(&mut self, ctx: &egui::Context) {
        if !self.confirm_clear {
            return;
        }
        let (mut yes, mut no) = (false, false);
        egui::Window::new("Remove all albums?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Your edits and undo history for every album are discarded. Nothing on disk changes.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    yes = ui.button("Remove all").clicked();
                    no = ui.button("Cancel").clicked();
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            no = true;
        }
        if yes {
            self.player.stop();
            self.project.clear();
            self.selected = None;
            self.cache.clear();
            self.notice = None;
            self.disk = DiskView::default();
        }
        if yes || no {
            self.confirm_clear = false;
        }
    }

    pub(super) fn action_bar(
        &mut self,
        ctx: &egui::Context,
        p: Palette,
        report: &Report,
        busy: bool,
    ) {
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
                            self.check.progress
                        } else {
                            self.split.progress
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
                });
                // The result of the last action gets its own line, so it never wraps into the
                // controls above.
                let (msg, color) = match &self.notice {
                    Some(Notice::Info(msg)) => (Some(msg), p.muted),
                    Some(Notice::Error(msg)) => (Some(msg), p.danger),
                    None => (None, p.muted),
                };
                if let Some(msg) = msg {
                    let mut dismiss = false;
                    ui.horizontal(|ui| {
                        dismiss = ui.small_button("×").on_hover_text("Dismiss").clicked();
                        ui.add(egui::Label::new(RichText::new(msg).color(color)).truncate())
                            .on_hover_text(msg);
                    });
                    if dismiss {
                        self.notice = None;
                    }
                }
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

    pub(super) fn albums_panel(
        &mut self,
        ctx: &egui::Context,
        p: Palette,
        report: &Report,
        busy: bool,
    ) {
        if self.project.entries.is_empty() {
            return;
        }
        let mut action = None;

        // Open just tall enough for the rows (header, rows, card padding), up to about eight.
        // The count is part of the id so a changed list is sized afresh rather than keeping
        // the height left over from another one.
        let rows = self.project.entries.len().min(8);
        egui::TopBottomPanel::top(egui::Id::new(("albums", rows)))
            .resizable(true)
            .default_height(80.0 + rows as f32 * 31.0)
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
}
