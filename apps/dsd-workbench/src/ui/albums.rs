//! The batch grid: one row per CUE sheet.

use std::collections::HashMap;

use eframe::egui::{Align, Color32, Label, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use super::cells;
use super::theme::Palette;
use crate::metadata::FieldKey;
use crate::project::{Entry, OutputMode, RunState, VerifyBadge};
use crate::snapshot::SourceState;

const ROW_H: f32 = 30.0;

pub(super) enum Action {
    Remove(u64),
    PickOutput(u64),
}

pub(super) fn show(
    ui: &mut Ui,
    p: &Palette,
    entries: &mut [Entry],
    selected: &mut Option<u64>,
    report: &HashMap<u64, Vec<String>>,
) -> Option<Action> {
    let mut action = None;
    let count = entries.len();

    // Roomier columns on wide windows; the last column takes whatever is left.
    let wide = ui.available_width();
    // Selectable text would swallow clicks meant for the row.
    ui.style_mut().interaction.selectable_labels = false;
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(30.0))
        .column(
            Column::initial((wide * 0.18).clamp(190.0, 380.0))
                .at_least(90.0)
                .clip(true),
        )
        .column(
            Column::initial((wide * 0.11).clamp(130.0, 240.0))
                .at_least(70.0)
                .clip(true),
        )
        .column(Column::exact(56.0))
        .column(Column::initial(130.0).at_least(70.0).clip(true))
        .column(
            Column::initial((wide * 0.24).clamp(200.0, 560.0))
                .at_least(160.0)
                .clip(true),
        )
        .column(Column::initial(120.0).at_least(80.0).clip(true))
        .column(Column::remainder().at_least(62.0))
        .header(24.0, |mut header| {
            for title in [
                "",
                "Album",
                "Artist",
                "Tracks",
                "Sources",
                "Output folder",
                "Status",
                "",
            ] {
                header.col(|ui| cells::head(ui, p, title));
            }
        })
        .body(|body| {
            body.rows(ROW_H, count, |mut row| {
                let Some(entry) = entries.get_mut(row.index()) else {
                    return;
                };
                let id = entry.id;
                row.set_selected(*selected == Some(id));

                let name = entry.display_name();
                let name = if entry.is_modified() {
                    format!("{name} •")
                } else {
                    name
                };
                let artist = entry.plan.album.value(&FieldKey::ALBUM_ARTIST).to_owned();
                let tracks = entry.plan.tracks.len().to_string();
                let files = entry.sources.len();
                let bad = entry
                    .sources
                    .values()
                    .filter(|s| !matches!(s.state, SourceState::Ready { .. }))
                    .count();
                let sources = if bad > 0 {
                    RichText::new(format!("{bad} of {files} not usable")).color(p.danger)
                } else {
                    RichText::new(format!("{files} file(s)")).color(p.muted)
                };
                let out_dir = entry.output_dir().display().to_string();
                let custom = matches!(entry.output, OutputMode::Custom(_));
                let (status, color, hover) = status_of(entry, report.get(&id), p);

                row.col(|ui| {
                    cells::checkbox(ui, &mut entry.enabled);
                });
                row.col(|ui| {
                    ui.add(Label::new(name).truncate());
                });
                row.col(|ui| cells::muted(ui, p, artist));
                row.col(|ui| {
                    ui.label(tracks);
                });
                row.col(|ui| {
                    ui.add(Label::new(sources).truncate());
                });
                row.col(|ui| {
                    let text = RichText::new(&out_dir).monospace();
                    let text = if custom { text } else { text.color(p.muted) };
                    ui.add(Label::new(text).truncate()).on_hover_text(&out_dir);
                });
                row.col(|ui| {
                    let response =
                        ui.add(Label::new(RichText::new(status).color(color)).truncate());
                    if let Some(hover) = hover {
                        response.on_hover_text(hover);
                    }
                });
                row.col(|ui| {
                    if ui
                        .small_button("…")
                        .on_hover_text("Choose output folder")
                        .clicked()
                    {
                        action = Some(Action::PickOutput(id));
                    }
                    if ui
                        .small_button("×")
                        .on_hover_text("Remove from list")
                        .clicked()
                    {
                        action = Some(Action::Remove(id));
                    }
                });

                // Render a horizontal separator line using the row response context and layer painter
                let resp = row.response();
                let rect = resp.rect;
                resp.ctx.layer_painter(resp.layer_id).hline(
                    rect.x_range(),
                    rect.bottom(),
                    eframe::egui::Stroke::new(1.0_f32, p.rule),
                );

                if row.response().clicked() {
                    *selected = Some(id);
                }
            });
        });
    action
}

fn status_of(
    entry: &Entry,
    issues: Option<&Vec<String>>,
    p: &Palette,
) -> (&'static str, Color32, Option<String>) {
    match &entry.state {
        RunState::Running => return ("Running…", p.accent, None),
        RunState::Queued => return ("Queued", p.muted, None),
        _ => {}
    }
    if let Some(issues) = issues {
        return ("Needs attention", p.danger, Some(issues.join("\n")));
    }
    if let RunState::Failed(message) = &entry.state {
        return ("Failed", p.danger, Some(message.clone()));
    }
    match entry.verify_badge() {
        VerifyBadge::Clean => {
            return (
                "Verified",
                p.good,
                Some("Every track is bit-identical to the source.".to_owned()),
            );
        }
        VerifyBadge::Problems(n) => {
            return (
                "Mismatch",
                p.bad,
                Some(format!(
                    "{n} track(s) differ or have notes; see the Verify tab."
                )),
            );
        }
        VerifyBadge::Stale | VerifyBadge::None => {}
    }
    match &entry.state {
        RunState::Done => ("Done", p.accent, None),
        RunState::Previewed => ("Previewed", p.accent, None),
        RunState::Cancelled => ("Cancelled", p.muted, None),
        _ if !entry.enabled => ("Skipped", p.muted, None),
        _ => ("Ready", p.ink, None),
    }
}
