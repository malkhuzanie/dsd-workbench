//! The selected album: tracks grid and album/output settings.

use eframe::egui::{self, Align, Button, Layout, RichText, TextEdit};
use egui_extras::{Column, TableBuilder};

use super::DetailTab;
use super::cells;
use super::fields;
use super::listen::{self, ListenState};
use super::sections::{cover_section, export_cue_section, lookup_section, tag_existing_section};
use super::theme::{self, Palette};
use super::verify_view;
use super::wave_view;
use super::widgets::{self, Tone};
use crate::audio::Player;
use crate::diag;
use crate::metadata::FieldKey;
use crate::plan::{PlanOptions, target_path};
use crate::project::{Entry, OutputMode};
use crate::selection::{Keys, TrackSelection};
use crate::settings::LookupSettings;
use crate::snapshot::{Loaded, SourceState, TagCache};
use crate::timecode;

use theme::ROW_H;
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
    /// Show the album tab's cards as collapsible sections (the "Collapsible sections" setting).
    pub collapsible: bool,
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
        DetailTab::Album => album(ui, p, cx.collapsible, entry, cx.problems, cx.lookup),
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
            Column::initial((wide * 0.15).clamp(170.0, 420.0))
                .at_least(100.0)
                .clip(true),
        )
        .column(
            Column::initial((wide * 0.09).clamp(110.0, 220.0))
                .at_least(80.0)
                .clip(true),
        )
        .column(Column::initial(110.0).at_least(80.0).clip(true))
        .column(Column::exact(104.0))
        .column(Column::exact(88.0));
    if multi {
        table = table.column(Column::initial(110.0).at_least(60.0).clip(true));
    }
    table
        .column(
            Column::initial((wide * 0.14).clamp(150.0, 400.0))
                .at_least(120.0)
                .clip(true),
        )
        .column(Column::exact(64.0))
        // The listen buttons (Track, Start, End and the cut sides) need about this much.
        .column(Column::remainder().at_least(200.0))
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

fn album(
    ui: &mut egui::Ui,
    p: &Palette,
    collapsible: bool,
    entry: &mut Entry,
    problems: &[String],
    lookup_settings: &mut LookupSettings,
) {
    // Salted by album: scroll offsets are stored by id, and one left over from another tab or
    // album would push the first heading under the tab bar.
    egui::ScrollArea::vertical()
        .id_salt(("album_tab", entry.id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(theme::space::XS);
            for problem in problems {
                widgets::banner(ui, p, Tone::Bad, problem);
            }
            for warning in &entry.warnings {
                widgets::banner(ui, p, Tone::Warn, warning);
            }

            let mut text = entry.output_dir().to_string_lossy().into_owned();
            let folder_summary = text.clone();
            widgets::card_section(
                ui,
                p,
                collapsible,
                "output",
                "Output folder",
                &folder_summary,
                true,
                |ui| {
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
                },
            );

            let mut relink: Option<(String, std::path::PathBuf)> = None;
            let not_ready = entry
                .sources
                .values()
                .filter(|s| !matches!(s.state, SourceState::Ready { .. }))
                .count();
            let sources_summary = match (entry.sources.len(), not_ready) {
                (n, 0) => format!("{n} file(s), all found"),
                (n, bad) => format!("{n} file(s), {bad} need attention"),
            };
            // A problem opens the card the first time it is seen.
            let sources_open = not_ready > 0;
            widgets::card_section(
                ui,
                p,
                collapsible,
                "sources",
                "Source files",
                &sources_summary,
                sources_open,
                |ui| {
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
                },
            );
            if let Some((old, picked)) = relink {
                entry.relink_source(&old, &picked);
            }

            let first = entry.plan.source_files().into_iter().next();
            let current = first
                .and_then(|file| entry.sources.get(&file))
                .and_then(|source| source.tags.tags())
                .cloned()
                .unwrap_or_default();
            let metadata_summary = {
                let (artist, album) = (
                    entry.plan.album.value(&FieldKey::ALBUM_ARTIST),
                    entry.plan.album.value(&FieldKey::ALBUM),
                );
                match (artist.is_empty(), album.is_empty()) {
                    (false, false) => format!("{artist} — {album}"),
                    (true, false) => album.to_owned(),
                    (false, true) => artist.to_owned(),
                    (true, true) => "no album tags yet".to_owned(),
                }
            };
            widgets::card_section(
                ui,
                p,
                collapsible,
                "metadata",
                "Album metadata",
                &metadata_summary,
                true,
                |ui| fields::album_editor(ui, p, &mut entry.plan, &current),
            );

            cover_section(ui, p, collapsible, entry);
            lookup_section(ui, p, collapsible, entry, lookup_settings);
            export_cue_section(ui, p, collapsible, entry);
            tag_existing_section(ui, p, collapsible, entry);
        });
}
