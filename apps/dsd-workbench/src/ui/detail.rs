//! The selected album: tracks grid and album/output settings.

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align, Button, Layout, RichText, TextEdit};
use egui_extras::{Column, TableBuilder};

use super::DetailTab;
use super::cells;
use super::fields;
use super::listen::{self, ListenState};
use super::theme::{self, Palette};
use super::verify_view;
use super::wave_view;
use super::widgets::{self, Tone};
use crate::audio::Player;
use crate::cue_write;
use crate::diag;
use crate::engine;
use crate::lookup;
use crate::metadata::FieldKey;
use crate::online;
use crate::plan::{CoverPlan, PlanOptions, SplitPlan, target_path};
use crate::project::{Entry, OutputMode};
use crate::selection::{Keys, TrackSelection};
use crate::settings::LookupSettings;
use crate::snapshot::{Loaded, SourceState, TagCache};
use crate::timecode;
use release_lookup::{Source, Target};

const ROW_H: f32 = 30.0;
/// Share of the pane given to the selected-track editor, within these limits.
const EDITOR_SHARE: f32 = 0.42;
const EDITOR_MIN_H: f32 = 160.0;
const EDITOR_MAX_H: f32 = 340.0;

/// Everything the detail pane needs besides the album it shows.
pub(super) struct Context<'a> {
    pub p: &'a Palette,
    pub cache: &'a mut TagCache,
    pub tab: &'a mut DetailTab,
    pub overwrite: bool,
    /// Why the album cannot be split yet.
    pub problems: &'a [String],
    pub player: &'a mut Player,
    pub listen: &'a mut ListenState,
    pub verify: &'a mut verify_view::ViewState,
    pub selection: &'a mut TrackSelection,
    pub wave: &'a mut wave_view::WaveState,
    /// Which service online lookups use, and the Discogs token.
    pub lookup: &'a mut LookupSettings,
    /// A run is in progress: the pane is read-only.
    pub busy: bool,
    /// How a re-read CUE sheet is interpreted.
    pub options: &'a PlanOptions,
}

pub(super) fn show(ui: &mut egui::Ui, cx: &mut Context<'_>, entry: &mut Entry) {
    let p = cx.p;
    cx.selection.bind(entry.id, entry.plan.tracks.len());
    ui.horizontal(|ui| {
        widgets::title(ui, &entry.display_name());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let modified = entry.is_modified();
            if ui
                .add_enabled(modified, Button::new("Reset edits"))
                .clicked()
            {
                entry.reset();
            }
            if ui
                .button("Re-read CUE")
                .on_hover_text(
                    "Start over from the CUE sheet, using the current CUE settings. Edits are lost.",
                )
                .clicked()
                && let Err(err) = entry.reload(cx.options)
            {
                diag::log(format!("could not re-read {}: {err}", entry.cue_path.display()));
            }
        });
    });
    widgets::caption(ui, p, &entry.cue_path.display().to_string());

    listen::bar(ui, p, cx.player, cx.listen);

    ui.horizontal(|ui| {
        ui.selectable_value(cx.tab, DetailTab::Tracks, "Tracks");
        ui.selectable_value(cx.tab, DetailTab::Waveform, "Waveform");
        ui.selectable_value(cx.tab, DetailTab::Album, "Album & output");
        ui.selectable_value(cx.tab, DetailTab::Verify, "Verify");
    });
    ui.separator();

    match *cx.tab {
        DetailTab::Tracks => tracks(ui, cx, entry),
        DetailTab::Waveform => wave_view::show(ui, p, &ui.ctx().clone(), entry, cx.wave),
        DetailTab::Album => album(ui, p, entry, cx.problems, cx.lookup),
        DetailTab::Verify => verify_view::show(ui, p, entry, cx.verify, cx.busy),
    }
}

fn tracks(ui: &mut egui::Ui, cx: &mut Context<'_>, entry: &mut Entry) {
    let p = cx.p;
    let problems = cx.problems;
    let overwrite = cx.overwrite;
    for problem in problems.iter().take(2) {
        widgets::banner(ui, p, Tone::Bad, problem);
    }
    if problems.len() > 2 {
        widgets::caption(
            ui,
            p,
            &format!(
                "+{} more issues: see the “Album & output” tab.",
                problems.len() - 2
            ),
        );
    }
    for warning in entry.warnings.iter().take(2) {
        widgets::banner(ui, p, Tone::Warn, warning);
    }

    let out_dir = entry.output_dir();
    let multi = entry.plan.source_files().len() > 1;
    let entry_id = entry.id;

    // Lengths: up to the next track, or to the end of the source for the last track of a file.
    let lengths: Vec<Option<f64>> = entry
        .plan
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            entry.plan.duration_of(i).or_else(|| {
                match entry.sources.get(&t.source_file).map(|s| &s.state) {
                    Some(SourceState::Ready { duration_secs, .. }) => {
                        Some(duration_secs - t.start_seconds)
                    }
                    _ => None,
                }
            })
        })
        .collect();
    let spans: Vec<Option<listen::Spans>> = (0..entry.plan.tracks.len())
        .map(|i| listen::spans(entry, i))
        .collect();

    let Entry {
        plan,
        original,
        base_dir,
        ..
    } = &mut *entry;
    let count = plan.tracks.len();

    // With tracks selected, the lower part of the pane edits them.
    let editor_height = if cx.selection.rows.is_empty() {
        0.0
    } else {
        (ui.available_height() * EDITOR_SHARE).clamp(EDITOR_MIN_H, EDITOR_MAX_H)
    };
    let table_height = (ui.available_height() - editor_height).max(ROW_H * 4.0);

    // Roomier columns on wide windows; the last column takes whatever is left.
    let wide = ui.available_width();
    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .max_scroll_height(table_height)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(28.0))
        .column(Column::exact(48.0))
        .column(
            Column::initial((wide * 0.16).clamp(200.0, 420.0))
                .at_least(100.0)
                .clip(true),
        )
        .column(
            Column::initial((wide * 0.09).clamp(130.0, 220.0))
                .at_least(80.0)
                .clip(true),
        )
        .column(Column::initial(130.0).at_least(80.0).clip(true))
        .column(Column::exact(104.0))
        .column(Column::exact(96.0));
    if multi {
        table = table.column(Column::initial(110.0).at_least(60.0).clip(true));
    }
    table
        .column(
            Column::initial((wide * 0.16).clamp(170.0, 400.0))
                .at_least(120.0)
                .clip(true),
        )
        .column(Column::exact(72.0))
        .column(Column::remainder().at_least(160.0))
        .header(26.0, |mut header| {
            header.col(|ui| {
                let mut all = count > 0 && cx.selection.rows.len() == count;
                if cells::checkbox(ui, &mut all)
                    .on_hover_text("Select every track to edit them together")
                    .changed()
                {
                    if all {
                        cx.selection.rows.select_all(count);
                    } else {
                        cx.selection.rows.clear();
                    }
                }
            });
            for title in ["#", "Title", "Artist", "Composer", "Start", "Length"] {
                header.col(|ui| cells::head(ui, p, title));
            }
            if multi {
                header.col(|ui| cells::head(ui, p, "Source"));
            }
            header.col(|ui| cells::head(ui, p, "Output file"));
            header.col(|ui| cells::head(ui, p, "On disk"));
            header.col(|ui| cells::head(ui, p, "Listen"));
        })
        .body(|body| {
            body.rows(ROW_H, count, |mut row| {
                let i = row.index();
                let Some(track) = plan.tracks.get_mut(i) else {
                    return;
                };
                let orig = original.tracks.get(i);

                let target = out_dir.join(target_path(&plan.naming, &plan.album, track));
                let file = target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let length = lengths.get(i).copied().flatten().filter(|l| *l > 0.0);

                let selected = cx.selection.rows.contains(i);
                row.set_selected(selected);
                row.col(|ui| {
                    let mut checked = selected;
                    if cells::checkbox(ui, &mut checked).clicked() {
                        // Shift extends a range; a plain click toggles just this row.
                        let shift = ui.input(|input| input.modifiers.shift);
                        cx.selection.rows.click(
                            i,
                            Keys {
                                toggle: !shift,
                                extend: shift,
                            },
                        );
                    }
                });
                row.col(|ui| {
                    // Fill the cell so the number sits in the middle of the column.
                    ui.add_sized(
                        [ui.available_width(), ui.spacing().interact_size.y],
                        egui::DragValue::new(&mut track.meta.number).range(1..=999),
                    );
                });
                for key in [FieldKey::TITLE, FieldKey::ARTIST, FieldKey::COMPOSER] {
                    row.col(|ui| {
                        let mut text = plan.album.resolve(&track.meta, &key).to_owned();
                        let was = orig.map(|o| original.album.resolve(&o.meta, &key));
                        cells::text(ui, p, &mut text, was);
                        track.meta.assign(&plan.album, key.clone(), &text);
                    });
                }
                row.col(|ui| {
                    let id = egui::Id::new(("track_start", entry_id, i));
                    cells::time(
                        ui,
                        p,
                        id,
                        &mut track.start_seconds,
                        orig.map(|o| o.start_seconds),
                    );
                });
                row.col(|ui| match length {
                    Some(l) => cells::mono(ui, timecode::format(l)),
                    None => cells::muted(ui, p, "—"),
                });
                if multi {
                    row.col(|ui| cells::muted(ui, p, track.source_file.clone()));
                }
                row.col(|ui| cells::mono(ui, file));
                row.col(|ui| disk_cell(ui, p, cx.cache.get(&target), overwrite));
                row.col(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let span = spans.get(i).and_then(Option::as_ref);
                    for (pick, label, hint) in listen::BUTTONS {
                        let response = ui
                            .add_enabled(span.is_some(), Button::new(label).small())
                            .on_hover_text(hint);
                        let response = listen::disabled_hint(response);
                        if response.clicked()
                            && let Some(span) = span
                        {
                            cx.player.play(span.request(
                                pick,
                                base_dir.as_path(),
                                &track.source_file,
                                track.meta.number,
                            ));
                        }
                    }
                });
            });
        });

    if editor_height > 0.0 {
        ui.separator();
        let rows: Vec<usize> = cx.selection.rows.indices().collect();
        let moved = egui::ScrollArea::vertical()
            .id_salt(("track_editor", entry_id))
            .auto_shrink([false, false])
            .show(ui, |ui| fields::track_editor(ui, p, &mut entry.plan, &rows))
            .inner;
        if let Some(row) = moved {
            cx.selection.rows.click(row, Keys::default());
        }
    }
}

fn disk_cell(ui: &mut egui::Ui, p: &Palette, loaded: &Loaded, overwrite: bool) {
    let (text, color) = match loaded {
        Loaded::Missing => ("new", p.muted),
        Loaded::Failed(_) => ("unreadable", p.danger),
        _ if overwrite => ("replace", p.accent),
        _ => ("exists", p.danger),
    };
    let response = ui.label(RichText::new(text).color(color));
    if let Some(tags) = loaded.tags() {
        let dash = |s: &str| {
            if s.is_empty() {
                "—".to_owned()
            } else {
                s.to_owned()
            }
        };
        response.on_hover_text(format!(
            "Currently on disk:\nTitle: {}\nArtist: {}\nComposer: {}\nTrack: {}",
            dash(tags.value(&FieldKey::TITLE)),
            dash(tags.value(&FieldKey::ARTIST)),
            dash(tags.value(&FieldKey::COMPOSER)),
            dash(&tags.track),
        ));
    }
}

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
fn lookup_section(
    ui: &mut egui::Ui,
    p: &Palette,
    entry: &mut Entry,
    settings: &mut LookupSettings,
) {
    widgets::group_title(ui, p, "Online lookup");
    if !online::ENABLED {
        widgets::caption(
            ui,
            p,
            "This build is strictly offline. To look up track titles on MusicBrainz or Discogs, build with `--features online`.",
        );
        return;
    }
    widgets::group(ui, p, |ui| lookup_body(ui, p, entry, settings));
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
                        .map(|looked| (plan, looked));
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
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    for line in state.message.lines() {
        widgets::caption(ui, p, line);
    }
    ui.data_mut(|d| d.insert_temp(id, state));
    ui.add_space(theme::space::SM);
}

/// Saves the album's cut points and tags as a CUE sheet.
fn export_cue_section(ui: &mut egui::Ui, p: &Palette, entry: &Entry) {
    widgets::group_title(ui, p, "CUE sheet");
    let id = egui::Id::new(("export_cue", entry.id));
    let mut message: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    widgets::group(ui, p, |ui| {
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
    });
    ui.data_mut(|d| d.insert_temp(id, message));
}

/// Writes the album's tags into files that already exist, without splitting.
fn tag_existing_section(ui: &mut egui::Ui, p: &Palette, entry: &Entry) {
    widgets::group_title(ui, p, "Tag existing files");
    let id = egui::Id::new(("tag_existing", entry.id));
    let (mut confirmed, mut lines) = ui
        .data(|data| data.get_temp::<(bool, Vec<String>)>(id))
        .unwrap_or_default();

    let mut run: Option<bool> = None; // Some(dry_run)
    widgets::group(ui, p, |ui| {
        ui.add_space(theme::space::SM);
        widgets::caption(
            ui,
            p,
            "Writes this album's tags and cover into .dsf files already in the output folder, without splitting. Files are matched by name or leading track number. Only the tag is rewritten; the audio is never touched.",
        );
        ui.add_space(theme::space::XS);
        ui.horizontal(|ui| {
            if ui.button("Preview").clicked() {
                run = Some(true);
            }
            ui.checkbox(&mut confirmed, "Modify the files in this folder");
            if ui
                .add_enabled(confirmed, Button::new("Apply tags"))
                .clicked()
            {
                run = Some(false);
            }
        });
        ui.add_space(theme::space::SM);
    });
    if let Some(dry_run) = run {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let dir = entry.output_dir();
        lines =
            match engine::tag_existing(&entry.plan, &entry.base_dir, &dir, dry_run, &cancel) {
                Ok(report) => {
                    let mut lines = vec![format!("{}: {}", dir.display(), report.summary())];
                    lines.extend(report.items.iter().map(|item| {
                        format!("{:02} {}: {}", item.number, item.title, item.describe())
                    }));
                    lines
                }
                Err(err) => vec![err.to_string()],
            };
    }
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            ui.label(RichText::new(line).strong());
        } else {
            widgets::caption(ui, p, line);
        }
    }
    ui.data_mut(|data| data.insert_temp(id, (confirmed, lines)));
}

/// Choose the album's cover image and whether to embed and/or copy it.
fn cover_section(ui: &mut egui::Ui, p: &Palette, entry: &mut Entry) {
    widgets::group_title(ui, p, "Cover art");
    let mut plan = entry.plan.cover.clone();

    widgets::group(ui, p, |ui| {
        cover_body(ui, p, entry, &mut plan);
    });

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

fn album(
    ui: &mut egui::Ui,
    p: &Palette,
    entry: &mut Entry,
    problems: &[String],
    lookup_settings: &mut LookupSettings,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for problem in problems {
                widgets::banner(ui, p, Tone::Bad, problem);
            }
            for warning in &entry.warnings {
                widgets::banner(ui, p, Tone::Warn, warning);
            }

            widgets::group_title(ui, p, "Output folder");
            let mut text = entry.output_dir().to_string_lossy().into_owned();
            widgets::group(ui, p, |ui| {
                ui.add_space(theme::space::SM);
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 240.0).max(120.0);
                    if ui
                        .add(TextEdit::singleline(&mut text).desired_width(width))
                        .changed()
                    {
                        entry.output = OutputMode::Custom(text.clone());
                    }
                    if ui.button("Browse…").clicked()
                        && let Some(dir) = rfd::FileDialog::new()
                            .set_directory(&entry.base_dir)
                            .pick_folder()
                    {
                        entry.output = OutputMode::Custom(dir.to_string_lossy().into_owned());
                    }
                    let custom = matches!(entry.output, OutputMode::Custom(_));
                    if ui.add_enabled(custom, Button::new("Same as CUE")).clicked() {
                        entry.output = OutputMode::BesideCue;
                    }
                });
                ui.add_space(theme::space::SM);
            });

            widgets::group_title(ui, p, "Source files");
            let mut relink: Option<(String, std::path::PathBuf)> = None;
            widgets::group(ui, p, |ui| {
                ui.add_space(theme::space::SM);
                for (file, info) in &entry.sources {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(file).monospace());
                        match &info.state {
                            SourceState::Ready {
                                duration_secs,
                                sample_rate,
                                channels,
                            } => {
                                ui.label(
                                    RichText::new(format!(
                                        "{} · {:.4} MHz · {channels} ch",
                                        timecode::format(*duration_secs),
                                        f64::from(*sample_rate) / 1e6
                                    ))
                                    .color(p.muted),
                                );
                            }
                            SourceState::Missing => {
                                ui.label(RichText::new("not found").color(p.danger));
                                if ui.button("Locate…").clicked()
                                    && let Some(picked) = rfd::FileDialog::new()
                                        .set_directory(&entry.base_dir)
                                        .add_filter("DSD audio", &["dsf", "dff"])
                                        .pick_file()
                                {
                                    relink = Some((file.clone(), picked));
                                }
                            }
                            SourceState::Invalid(err) => {
                                ui.label(RichText::new(err).color(p.danger));
                            }
                        }
                    });
                }
                ui.add_space(theme::space::XS);
                if ui.button("Rescan source files").clicked() {
                    entry.rescan_sources();
                }
                ui.add_space(theme::space::SM);
            });
            if let Some((old, picked)) = relink {
                entry.relink_source(&old, &picked);
            }

            widgets::group_title(ui, p, "Album metadata");
            let first = entry.plan.source_files().into_iter().next();
            let current = first
                .and_then(|file| entry.sources.get(&file))
                .and_then(|source| source.tags.tags())
                .cloned()
                .unwrap_or_default();
            widgets::group(ui, p, |ui| {
                fields::album_editor(ui, p, &mut entry.plan, &current);
            });

            cover_section(ui, p, entry);
            lookup_section(ui, p, entry, lookup_settings);
            export_cue_section(ui, p, entry);
            tag_existing_section(ui, p, entry);
        });
}
