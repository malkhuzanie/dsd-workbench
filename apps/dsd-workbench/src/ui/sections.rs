//! The cards of the Album & output tab that do something beyond editing fields: online lookup,
//! CUE export, tagging existing files, and the cover image.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Button, RichText};

use super::theme::{self, Palette};
use super::widgets::{self, Tone};
use crate::cue_write;
use crate::engine;
use crate::lookup;
use crate::metadata::FieldKey;
use crate::online;
use crate::plan::{CoverPlan, SplitPlan};
use crate::project::Entry;
use crate::settings::LookupSettings;
use release_lookup::{Source, Target};

/// The outcome of a background lookup, shared with the thread that does it.
type LookupResult = Result<(SplitPlan, lookup::Looked), String>;

#[derive(Clone, Default)]
struct LookupState {
    pending: Option<Arc<Mutex<Option<LookupResult>>>>,
    /// What the person typed: search text, or a release link or id.
    query: String,
    /// What the running lookup is searching for, shown with its result.
    searching: String,
    message: String,
}

/// Fills titles and album details from MusicBrainz or Discogs (builds with the `online` feature).
pub(super) fn lookup_section(
    ui: &mut egui::Ui,
    p: &Palette,
    collapsible: bool,
    entry: &mut Entry,
    settings: &mut LookupSettings,
) {
    if !online::ENABLED {
        widgets::card_section(
            ui,
            p,
            collapsible,
            "lookup",
            "Online lookup",
            "not in this build",
            false,
            |ui| {
                widgets::caption(
                    ui,
                    p,
                    "This build is strictly offline. To look up track titles on MusicBrainz or Discogs, build with `--features online`.",
                );
            },
        );
        return;
    }
    let summary = format!("Fill in titles and details from {}", settings.source.name());
    widgets::card_section(
        ui,
        p,
        collapsible,
        "lookup",
        "Online lookup",
        &summary,
        false,
        |ui| lookup_body(ui, p, entry, settings),
    );
}

fn lookup_body(ui: &mut egui::Ui, p: &Palette, entry: &mut Entry, settings: &mut LookupSettings) {
    ui.add_space(theme::space::SM);
    let sources: Vec<_> = Source::ALL.iter().map(|s| (*s, s.name())).collect();
    widgets::segmented(ui, &mut settings.source, &sources);
    widgets::caption(
        ui,
        p,
        &format!(
            "Fills in titles, artists, date, label and catalogue number. Only the text you search for (or this album's artist and title) is sent to {}. You can undo the result.",
            settings.source.name()
        ),
    );

    let id = egui::Id::new(("lookup", entry.id));
    let mut state: LookupState = ui.data(|d| d.get_temp(id)).unwrap_or_default();

    // Collect a finished lookup.
    let finished = state
        .pending
        .as_ref()
        .and_then(|slot| slot.lock().ok().and_then(|mut result| result.take()));
    if let Some(result) = finished {
        state.pending = None;
        state.message = match result {
            Ok((plan, looked)) => {
                entry.plan = plan;
                let mut text = format!(
                    "Searched {}. Matched {} — {} ({}): {} title(s) and {} album field(s) filled in.",
                    state.searching,
                    looked.found.release.artist,
                    looked.found.release.title,
                    looked.found.release.date,
                    looked.applied.titles_set,
                    looked.applied.fields_set
                );
                if let Some(candidate) = &looked.found.candidate {
                    text.push_str(&format!("\nUsed: {}", candidate.describe()));
                }
                for note in &looked.applied.notes {
                    text.push_str(&format!("\n{note}"));
                }
                text
            }
            Err(why) => format!("Searched {}. {why}", state.searching),
        };
    }

    let running = state.pending.is_some();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .hint_text("What to search for, or a release link/id. Empty: this album.")
                .desired_width(380.0),
        );
        if ui
            .add_enabled(!running, egui::Button::new("Look up"))
            .clicked()
        {
            let source = settings.source;
            let artist = entry.plan.album.value(&FieldKey::ALBUM_ARTIST).to_owned();
            let album = entry.plan.album.value(&FieldKey::ALBUM).to_owned();
            match Target::from_input(source, &state.query, &artist, &album) {
                None => {
                    state.message =
                        "Type what to search for, or give this album a title first.".to_owned();
                }
                Some(target) => {
                    state.searching = format!("{} for {}", source.name(), target.describe());
                    state.message = format!("Searching {}…", state.searching);
                    let slot: Arc<Mutex<Option<LookupResult>>> = Arc::new(Mutex::new(None));
                    state.pending = Some(Arc::clone(&slot));
                    let mut plan = entry.plan.clone();
                    let token = settings.token();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let result = lookup::lookup(
                            &mut plan,
                            source,
                            token.as_deref(),
                            &target,
                            &online::get,
                            lookup::ApplyOptions::default(),
                        )
                        .map(|looked| (plan, looked))
                        .map_err(|e| e.to_string());
                        if let Ok(mut guard) = slot.lock() {
                            *guard = Some(result);
                        }
                        ctx.request_repaint();
                    });
                }
            }
        }
    });
    if running {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
    for line in state.message.lines() {
        widgets::caption(ui, p, line);
    }
    ui.data_mut(|d| d.insert_temp(id, state));
    ui.add_space(theme::space::SM);
}

/// Saves the album's cut points and tags as a CUE sheet.
pub(super) fn export_cue_section(ui: &mut egui::Ui, p: &Palette, collapsible: bool, entry: &Entry) {
    let id = egui::Id::new(("export_cue", entry.id));
    let mut message: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let summary = "Save the cut points and tags as a CUE sheet";
    widgets::card_section(
        ui,
        p,
        collapsible,
        "cue",
        "CUE sheet",
        summary,
        false,
        |ui| {
            widgets::setting_row(
                ui,
                p,
                true,
                "Export as a CUE sheet",
                "Save these cut points and tags next to the audio.",
                |ui| {
                    if ui.button("Export CUE sheet…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_directory(&entry.base_dir)
                            .set_file_name(format!("{}.cue", entry.cue_stem()))
                            .add_filter("CUE sheet", &["cue"])
                            .save_file()
                    {
                        message = match std::fs::write(&path, cue_write::to_cue(&entry.plan)) {
                            Ok(()) => format!(
                                "Saved {}. Cut times are rounded to 1/75 s; save a project to keep them exactly.",
                                path.display()
                            ),
                            Err(err) => format!("Could not save {}: {err}", path.display()),
                        };
                    }
                },
            );
            if !message.is_empty() {
                widgets::caption(ui, p, &message);
                ui.add_space(theme::space::SM);
            }
        },
    );
    ui.data_mut(|d| d.insert_temp(id, message));
}

/// A finished tag-only run, kept on screen until the next one.
#[derive(Clone)]
struct TagResult {
    /// Whether nothing was written (a preview).
    dry_run: bool,
    folder: std::path::PathBuf,
    outcome: Result<engine::Report, String>,
}

/// Where a background tag-only run leaves its result when it finishes.
type TagJob = Arc<Mutex<Option<TagResult>>>;

#[derive(Clone, Default)]
struct TagState {
    /// The "modify the files" box, which Apply needs.
    confirmed: bool,
    result: Option<TagResult>,
    /// The run in progress, and whether it is a preview.
    pending: Option<(TagJob, bool)>,
}

/// Writes the album's tags into files that already exist, without splitting.
pub(super) fn tag_existing_section(
    ui: &mut egui::Ui,
    p: &Palette,
    collapsible: bool,
    entry: &Entry,
) {
    let id = egui::Id::new(("tag_existing", entry.id));
    let mut state: TagState = ui.data(|data| data.get_temp(id)).unwrap_or_default();

    // A finished job replaces the result; a running one keeps the buttons off. After files
    // were written the box is cleared, so a second Apply is a deliberate act.
    if let Some((job, _)) = &state.pending {
        let done = job.lock().ok().and_then(|mut slot| slot.take());
        match done {
            Some(result) => {
                if !result.dry_run && result.outcome.is_ok() {
                    state.confirmed = false;
                }
                state.result = Some(result);
                state.pending = None;
            }
            None => ui.ctx().request_repaint_after(Duration::from_millis(100)),
        }
    }
    let working = state.pending.as_ref().map(|(_, dry_run)| *dry_run);

    let summary = match (&working, &state.result) {
        (Some(_), _) => "Working…".to_owned(),
        (None, Some(result)) => format!("Last run: {}", result_headline(result)),
        (None, None) => "Write tags into files you already have".to_owned(),
    };
    let mut run: Option<bool> = None; // Some(dry_run)
    widgets::card_section(
        ui,
        p,
        collapsible,
        "tag-existing",
        "Tag existing files",
        &summary,
        false,
        |ui| {
            widgets::caption(
                ui,
                p,
                "Writes this album's tags and cover into .dsf files already in the output folder, without splitting. Files are matched by name or leading track number. Only the tag is rewritten; the audio is never touched.",
            );
            ui.add_space(theme::space::SM);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(working.is_none(), Button::new("Preview changes"))
                    .on_hover_text("Shows what would change. Nothing is written.")
                    .clicked()
                {
                    run = Some(true);
                }
                ui.add_space(theme::space::SM);
                ui.checkbox(&mut state.confirmed, "Modify the files in this folder");
                let can_apply = state.confirmed && working.is_none();
                if widgets::primary_button(ui, p, can_apply, "Apply tags").clicked() {
                    run = Some(false);
                }
                if let Some(preview) = working {
                    ui.spinner();
                    let what = if preview {
                        "Checking the files…"
                    } else {
                        "Writing tags…"
                    };
                    ui.label(RichText::new(what).color(p.muted));
                }
            });
            if let Some(result) = &state.result {
                ui.add_space(theme::space::SM);
                tag_result_view(ui, p, entry.id, result);
            }
        },
    );
    if let Some(dry_run) = run {
        // Rewriting tags reads and writes every file, so it runs off the UI thread.
        let slot: TagJob = Arc::default();
        let (plan, base_dir, dir) = (
            entry.plan.clone(),
            entry.base_dir.clone(),
            entry.output_dir(),
        );
        let (job, ctx) = (Arc::clone(&slot), ui.ctx().clone());
        std::thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            let outcome = engine::tag_existing(&plan, &base_dir, &dir, dry_run, &cancel)
                .map_err(|err| err.with_advice());
            if let Ok(mut slot) = job.lock() {
                *slot = Some(TagResult {
                    dry_run,
                    folder: dir,
                    outcome,
                });
            }
            ctx.request_repaint();
        });
        state.pending = Some((slot, dry_run));
        ui.ctx().request_repaint();
    }
    ui.data_mut(|data| data.insert_temp(id, state));
}

/// The one-line result of a run, for the card header and the banner.
fn result_headline(result: &TagResult) -> String {
    match &result.outcome {
        Err(why) => why.clone(),
        Ok(report) if result.dry_run => format!("preview, {}", report.summary()),
        Ok(report) => report.summary(),
    }
}

/// The banner and the per-file list of a finished run.
fn tag_result_view(ui: &mut egui::Ui, p: &Palette, album: u64, result: &TagResult) {
    let report = match &result.outcome {
        Ok(report) => report,
        Err(why) => {
            widgets::banner(ui, p, Tone::Bad, why);
            return;
        }
    };
    let folder = result.folder.display();
    let (tone, headline) = if result.dry_run {
        (
            if report.problems() > 0 {
                Tone::Warn
            } else {
                Tone::Info
            },
            format!(
                "Preview of {folder}: {}. Nothing was written.",
                report.summary()
            ),
        )
    } else if report.problems() > 0 {
        (
            Tone::Warn,
            format!("Finished in {folder}: {}.", report.summary()),
        )
    } else {
        (
            Tone::Info,
            format!("Done in {folder}: {}.", report.summary()),
        )
    };
    widgets::banner(ui, p, tone, &headline);
    let hint = if result.dry_run && report.count(&engine::Outcome::WouldTag) > 0 {
        Some("Tick “Modify the files in this folder” and choose Apply tags to write these changes.")
    } else if !result.dry_run && report.count(&engine::Outcome::Tagged) > 0 {
        Some("The files were rewritten. Use the Verify tab to check them against the plan.")
    } else {
        None
    };
    if let Some(hint) = hint {
        ui.add_space(theme::space::XS);
        widgets::caption(ui, p, hint);
    }
    if report.items.is_empty() {
        return;
    }
    ui.add_space(theme::space::SM);
    let name_width = (ui.available_width() * 0.42).clamp(200.0, 420.0);
    ui.push_id(("tag_rows", album), |ui| {
        egui_extras::TableBuilder::new(ui)
            .striped(true)
            .max_scroll_height(300.0)
            .auto_shrink([false, true])
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(egui_extras::Column::exact(100.0))
            .column(egui_extras::Column::exact(name_width))
            .column(egui_extras::Column::remainder().at_least(120.0).clip(true))
            .header(22.0, |mut header| {
                for title in ["Result", "Track and file", "What changes"] {
                    header.col(|ui| {
                        ui.label(RichText::new(title).small().color(p.muted));
                    });
                }
            })
            .body(|body| {
                body.rows(46.0, report.items.len(), |mut row| {
                    if let Some(item) = report.items.get(row.index()) {
                        tag_row(&mut row, p, item);
                    }
                });
            });
    });
}

/// One file of the list: a status chip, the track, the file and what changes in it.
fn tag_row(row: &mut egui_extras::TableRow<'_, '_>, p: &Palette, item: &engine::Item) {
    use engine::Outcome;

    let (label, colour) = match &item.outcome {
        Outcome::Tagged => ("Tagged", p.good),
        Outcome::UpToDate => ("Up to date", p.muted),
        Outcome::WouldTag => ("Will change", p.accent),
        Outcome::NoFile => ("No file", p.danger),
        Outcome::Ambiguous(_) => ("Ambiguous", p.danger),
        Outcome::Failed(_) => ("Failed", p.danger),
    };
    row.col(|ui| status_chip(ui, label, colour));

    let file = item
        .file
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    row.col(|ui| {
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{:02}  {}", item.number, item.title)).color(p.ink),
                )
                .truncate(),
            );
            if !file.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(&file).monospace().size(12.0).color(p.muted))
                        .truncate(),
                )
                .on_hover_text(&file);
            }
        });
    });

    let (text, hover) = match &item.outcome {
        Outcome::Ambiguous(names) => ("several files match".to_owned(), names.join("\n")),
        Outcome::Failed(why) => ("see details".to_owned(), why.clone()),
        Outcome::NoFile => ("no file matches this track".to_owned(), String::new()),
        Outcome::UpToDate => ("nothing to change".to_owned(), String::new()),
        Outcome::Tagged | Outcome::WouldTag => {
            let fields: Vec<&str> = item.changes.iter().map(|c| c.field.as_str()).collect();
            let shown = fields
                .iter()
                .take(4)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            let text = match fields.len() {
                0 => String::new(),
                n if n > 4 => format!("{shown} and {} more", n - 4),
                _ => shown,
            };
            let hover = item
                .changes
                .iter()
                .map(engine::TagChange::describe)
                .collect::<Vec<_>>()
                .join("\n");
            (text, hover)
        }
    };
    row.col(|ui| {
        let response = ui.add(egui::Label::new(RichText::new(text).color(p.ink)).truncate());
        if !hover.is_empty() {
            response.on_hover_text(hover);
        }
    });
}

/// A small rounded status label with a tinted background.
fn status_chip(ui: &mut egui::Ui, text: &str, colour: egui::Color32) {
    egui::Frame::new()
        .fill(colour.gamma_multiply(0.14))
        .stroke(egui::Stroke::new(1.0_f32, colour.gamma_multiply(0.55)))
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(egui::Margin::symmetric(8, 1))
        .show(ui, |ui| {
            ui.set_min_width(72.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.label(RichText::new(text).size(12.0).color(colour));
            });
        });
}

/// Choose the album's cover image and whether to embed and/or copy it.
pub(super) fn cover_section(ui: &mut egui::Ui, p: &Palette, collapsible: bool, entry: &mut Entry) {
    let mut plan = entry.plan.cover.clone();
    let summary = match &plan {
        Some(cover) if cover.embed => format!("{}, embedded in every track", cover.file),
        Some(cover) => format!("{}, not embedded", cover.file),
        None => "none found".to_owned(),
    };

    widgets::card_section(
        ui,
        p,
        collapsible,
        "cover",
        "Cover art",
        &summary,
        false,
        |ui| {
            cover_body(ui, p, entry, &mut plan);
        },
    );

    // Choosing a cover is a deliberate edit of the album, so it counts as one.
    entry.plan.cover = plan;
}

fn cover_body(ui: &mut egui::Ui, p: &Palette, entry: &Entry, plan: &mut Option<CoverPlan>) {
    match plan {
        Some(cover) => {
            let path = entry.base_dir.join(&cover.file);
            let status = match std::fs::metadata(&path) {
                Ok(meta) if meta.len() > crate::cover::MAX_COVER_BYTES => {
                    "too large to embed".to_owned()
                }
                Ok(meta) => crate::disk::format_bytes(meta.len()),
                Err(_) => "not found".to_owned(),
            };
            widgets::setting_row(ui, p, true, &cover.file, &status, |_| {});
            widgets::setting_row(ui, p, false, "Embed in every track", "", |ui| {
                widgets::toggle(ui, p, &mut cover.embed);
            });
            widgets::setting_row(
                ui,
                p,
                false,
                "Copy into the album folder",
                "Saved as folder.jpg.",
                |ui| {
                    widgets::toggle(ui, p, &mut cover.copy_to_folder);
                },
            );
        }
        None => {
            ui.add_space(theme::space::SM);
            widgets::caption(
                ui,
                p,
                "No cover.jpg, folder.jpg or front.png was found next to the CUE sheet.",
            );
            ui.add_space(theme::space::XS);
        }
    }
    ui.add_space(theme::space::XS);
    ui.horizontal(|ui| {
        if ui.button("Choose image…").clicked()
            && let Some(file) = rfd::FileDialog::new()
                .set_directory(&entry.base_dir)
                .add_filter("Image", &["jpg", "jpeg", "png"])
                .pick_file()
        {
            let relative = file
                .strip_prefix(&entry.base_dir)
                .map_or_else(|_| file.clone(), std::path::Path::to_path_buf);
            let (embed, copy) = plan
                .as_ref()
                .map_or((true, false), |c| (c.embed, c.copy_to_folder));
            *plan = Some(CoverPlan {
                file: relative.to_string_lossy().replace('\\', "/"),
                embed,
                copy_to_folder: copy,
            });
        }
        if plan.is_some() && ui.button("Remove").clicked() {
            *plan = None;
        }
    });
    ui.add_space(theme::space::SM);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;
    use crate::source::testing::stereo_dsf;
    use crate::ui::snapshot::{output_dir, render, write_ppm};

    /// Renders the Tag existing files card after a preview and after applying, for looking at.
    #[test]
    #[ignore = "writes pictures; run on purpose with UI_SNAPSHOT_DIR set"]
    fn snapshot_tag_existing() {
        let Some(out) = output_dir() else { return };
        let dir = tempfile::tempdir().expect("temp dir");
        let audio = vec![0x5A; 4096 * 2];
        for name in [
            "01 - Tangled Up In Blue.dsf",
            "02 - Simple Twist Of Fate.dsf",
        ] {
            std::fs::write(dir.path().join(name), stereo_dsf(32_768, 32_768, &audio))
                .expect("write a file");
        }
        std::fs::write(
            dir.path().join("side.dsf"),
            stereo_dsf(32_768, 4 * 32_768, &audio),
        )
        .expect("write the source");
        std::fs::write(
            dir.path().join("album.cue"),
            "PERFORMER \"Bob Dylan\"\nTITLE \"Blood On The Tracks\"\nREM COMMENT \"http://www.discogs.com/x\"\nFILE \"side.dsf\" WAVE\n\
             TRACK 01 AUDIO\n TITLE \"Tangled Up In Blue\"\n INDEX 01 00:00:00\n\
             TRACK 02 AUDIO\n TITLE \"Simple Twist Of Fate\"\n INDEX 01 00:01:00\n\
             TRACK 03 AUDIO\n TITLE \"You're A Big Girl Now\"\n INDEX 01 00:02:00\n",
        )
        .expect("write the cue");
        let mut project = Project::default();
        project.add(&dir.path().join("album.cue")).expect("add");
        let entry = project.entries.first().expect("an entry");
        let never = AtomicBool::new(false);
        let preview = engine::tag_existing(&entry.plan, &entry.base_dir, dir.path(), true, &never)
            .expect("preview");
        let applied = engine::tag_existing(&entry.plan, &entry.base_dir, dir.path(), false, &never)
            .expect("apply");

        let p = Palette::of(false);
        for (name, dry_run, report) in [
            ("tag-preview", true, preview),
            ("tag-applied", false, applied),
        ] {
            let state = TagState {
                confirmed: !dry_run,
                result: Some(TagResult {
                    dry_run,
                    folder: dir.path().to_path_buf(),
                    outcome: Ok(report),
                }),
                pending: None,
            };
            let (w, h, pixels) = render(egui::vec2(1100.0, 560.0), false, 1.0, |ctx| {
                egui::CentralPanel::default()
                    .frame(theme::gutter(egui::Margin::same(12)))
                    .show(ctx, |ui| {
                        ui.data_mut(|d| {
                            d.insert_temp(egui::Id::new(("tag_existing", entry.id)), state.clone());
                        });
                        tag_existing_section(ui, &p, false, entry);
                    });
            });
            write_ppm(&out.join(format!("{name}.ppm")), w, h, &pixels);
        }
    }
}
