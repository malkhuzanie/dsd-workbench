//! The "Verify" tab: results of comparing split files with their source.

use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, FontId, Label, Layout, Rect, RichText,
    Sense, Stroke, Ui, pos2, vec2,
};
use egui_extras::{Column, TableBuilder};

use super::cells;
use super::theme::Palette;
use super::widgets::{self, Tone};
use crate::disk::format_bytes;
use crate::engine::{AlbumReport, BUCKETS, TrackCheck, Verdict};
use crate::metadata::FieldKey;
use crate::project::Entry;
use crate::snapshot::SourceState;
use crate::timecode;

const LANE_H: f32 = 26.0;
const STRIP_H: f32 = 14.0;
const ENVELOPE_H: f32 = 84.0;
const MAX_STRIPS: usize = 8;

#[derive(Default)]
pub(super) struct ViewState {
    pub selected: Option<usize>,
    /// Set when the user asked to verify this album.
    pub request: bool,
}

pub(super) fn show(ui: &mut Ui, p: &Palette, entry: &Entry, state: &mut ViewState, busy: bool) {
    egui::ScrollArea::vertical()
        .id_salt(("verify_tab", entry.id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, Button::new("Verify files now")).clicked() {
                state.request = true;
            }
            widgets::caption(
                ui,
                p,
                "Reads each output file back and compares it with the source, bit by bit. Nothing is written.",
            );
        });

        let Some(report) = &entry.verification else {
            widgets::banner(
                ui,
                p,
                Tone::Warn,
                "Not verified yet. Verification runs after a split (see Settings), or press “Verify files now” to check files that already exist.",
            );
            return;
        };

        if report.plan != entry.plan || report.output_dir != entry.output_dir() {
            widgets::banner(
                ui,
                p,
                Tone::Warn,
                "The plan or output folder changed since this report. Verify again to refresh it.",
            );
        }
        summary(ui, p, report);

        let invalid = state.selected.is_none_or(|i| i >= report.tracks.len());
        if invalid {
            state.selected = report
                .tracks
                .iter()
                .position(|t| t.verdict != Verdict::Identical)
                .or(Some(0));
        }

        album_map(ui, p, entry, report, state);
        table(ui, p, report, state);
        if let Some(track) = state.selected.and_then(|i| report.tracks.get(i)) {
            comparison(ui, p, track);
        }
    });
}

fn summary(ui: &mut Ui, p: &Palette, report: &AlbumReport) {
    let n = report.tracks.len();
    if report.is_clean() {
        widgets::banner(
            ui,
            p,
            Tone::Info,
            &format!(
                "All {n} tracks are bit-identical to the source ({} compared in {:.1} s).",
                format_bytes(report.compared_bytes()),
                report.seconds
            ),
        );
    } else if report.identical() == n {
        widgets::banner(
            ui,
            p,
            Tone::Warn,
            &format!(
                "The audio of all {n} tracks is identical, but {} note(s) need a look (tags or headers).",
                report.issue_count()
            ),
        );
    } else {
        widgets::banner(
            ui,
            p,
            Tone::Bad,
            &format!(
                "{} identical · {} differ · {} missing · {} unreadable · {} note(s).",
                report.identical(),
                report.differs(),
                report.missing(),
                report.unreadable(),
                report.issue_count()
            ),
        );
    }
}

fn verdict_color(p: &Palette, verdict: &Verdict, has_notes: bool) -> Color32 {
    match verdict {
        Verdict::Identical if has_notes => p.accent,
        Verdict::Identical => p.good,
        Verdict::Differs => p.bad,
        Verdict::Missing => p.muted,
        Verdict::Unreadable(_) => p.danger,
    }
}

fn verdict_text(check: &TrackCheck) -> &'static str {
    match (&check.verdict, check.issues.is_empty()) {
        (Verdict::Identical, true) => "Identical",
        (Verdict::Identical, false) => "Identical, with notes",
        (Verdict::Differs, _) => "Differs",
        (Verdict::Missing, _) => "Missing",
        (Verdict::Unreadable(_), _) => "Unreadable",
    }
}

fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One lane per source file; each track is a segment coloured by its result.
fn album_map(ui: &mut Ui, p: &Palette, entry: &Entry, report: &AlbumReport, state: &mut ViewState) {
    widgets::section(ui, p, "Album map");
    widgets::caption(
        ui,
        p,
        "Each lane is a source file; segments are the planned tracks, coloured by result. Click one to inspect it.",
    );
    let plan = &report.plan;

    for file in plan.source_files() {
        let in_file = |t: &crate::plan::TrackPlan| t.source_file == file;
        let duration = match entry.sources.get(&file).map(|s| &s.state) {
            Some(SourceState::Ready { duration_secs, .. }) => *duration_secs,
            _ => plan
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| in_file(t))
                .map(|(i, t)| t.start_seconds + plan.duration_of(i).unwrap_or(1.0))
                .fold(1.0, f64::max),
        }
        .max(0.001);

        ui.label(RichText::new(&file).monospace().small().color(p.muted));
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), LANE_H), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(4), p.button);
        let x = |t: f64| rect.left() + (t / duration) as f32 * rect.width();

        let first_start = plan
            .tracks
            .iter()
            .filter(|t| in_file(t))
            .map(|t| t.start_seconds)
            .fold(f64::MAX, f64::min);
        if first_start < f64::MAX && x(first_start) - rect.left() > 60.0 {
            painter.text(
                pos2(rect.left() + 6.0, rect.center().y),
                Align2::LEFT_CENTER,
                "not used",
                FontId::proportional(11.0),
                p.muted,
            );
        }

        let pointer = response.hover_pos();
        let mut hover = None;
        for (i, t) in plan.tracks.iter().enumerate().filter(|(_, t)| in_file(t)) {
            let end = (t.start_seconds + plan.duration_of(i).unwrap_or(duration - t.start_seconds))
                .min(duration);
            let left = x(t.start_seconds) + 0.5;
            let seg = Rect::from_min_max(
                pos2(left, rect.top() + 2.0),
                pos2((x(end) - 0.5).max(left + 1.0), rect.bottom() - 2.0),
            );
            let color = report.tracks.get(i).map_or(p.muted, |c| {
                verdict_color(p, &c.verdict, !c.issues.is_empty())
            });
            if state.selected == Some(i) {
                painter.rect_filled(seg.expand(2.0), CornerRadius::same(4), p.accent);
            }
            painter.rect_filled(seg, CornerRadius::same(3), color);
            if seg.width() > 22.0 {
                painter.text(
                    seg.center(),
                    Align2::CENTER_CENTER,
                    t.meta.number.to_string(),
                    FontId::proportional(11.0),
                    Color32::WHITE,
                );
            }
            if pointer.is_some_and(|pos| seg.contains(pos)) {
                hover = Some((i, end));
            }
        }

        if let Some((i, end)) = hover {
            if let (Some(t), Some(c)) = (plan.tracks.get(i), report.tracks.get(i)) {
                let text = format!(
                    "{:02} {}\n{} → {}\n{}",
                    t.meta.number,
                    plan.album.resolve(&t.meta, &FieldKey::TITLE),
                    timecode::format(t.start_seconds),
                    timecode::format(end),
                    verdict_text(c)
                );
                response.clone().on_hover_text_at_pointer(text);
            }
            if response.clicked() {
                state.selected = Some(i);
            }
        }
    }
}

fn table(ui: &mut Ui, p: &Palette, report: &AlbumReport, state: &mut ViewState) {
    widgets::section(ui, p, "Tracks");
    let rows = report.tracks.len();
    let mut clicked = None;

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .vscroll(false)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(40.0))
        .column(Column::initial(220.0).at_least(80.0).clip(true))
        .column(Column::exact(150.0))
        .column(Column::exact(150.0))
        .column(Column::exact(110.0))
        .column(Column::remainder().at_least(160.0).clip(true))
        .header(24.0, |mut header| {
            for title in ["#", "Title", "Result", "Samples", "Differing bits", "Note"] {
                header.col(|ui| cells::head(ui, p, title));
            }
        })
        .body(|body| {
            body.rows(26.0, rows, |mut row| {
                let i = row.index();
                let Some(t) = report.tracks.get(i) else {
                    return;
                };
                row.set_selected(state.selected == Some(i));

                let color = verdict_color(p, &t.verdict, !t.issues.is_empty());
                let samples = if t.actual_samples == t.expected_samples {
                    count(t.expected_samples)
                } else {
                    format!(
                        "{} ≠ {}",
                        count(t.actual_samples),
                        count(t.expected_samples)
                    )
                };
                let bits = t.mismatched_bits();

                row.col(|ui| cells::mono(ui, format!("{:02}", t.number)));
                row.col(|ui| {
                    ui.add(Label::new(&t.title).truncate());
                });
                row.col(|ui| {
                    ui.label(RichText::new(verdict_text(t)).color(color));
                });
                row.col(|ui| cells::mono(ui, samples));
                row.col(|ui| {
                    if bits > 0 {
                        ui.label(RichText::new(count(bits)).monospace().color(p.bad));
                    } else {
                        cells::muted(ui, p, "0");
                    }
                });
                row.col(|ui| cells::muted(ui, p, t.note()));

                if row.response().clicked() {
                    clicked = Some(i);
                }
            });
        });
    if let Some(i) = clicked {
        state.selected = Some(i);
    }
}

fn comparison(ui: &mut Ui, p: &Palette, t: &TrackCheck) {
    widgets::section(ui, p, &format!("Comparison · track {:02}", t.number));
    ui.label(RichText::new(&t.title).strong());

    match &t.verdict {
        Verdict::Missing => {
            widgets::banner(
                ui,
                p,
                Tone::Bad,
                &format!("No file at {}", t.target.display()),
            );
        }
        Verdict::Unreadable(why) => widgets::banner(ui, p, Tone::Bad, why),
        _ => {}
    }
    if let Some(alignment) = &t.alignment {
        widgets::banner(ui, p, Tone::Warn, &alignment.describe(t.rate));
    }
    for issue in &t.issues {
        widgets::banner(ui, p, Tone::Warn, issue);
    }
    if t.compared_samples == 0 {
        return;
    }

    let start = t.start_sample as f64 / f64::from(t.rate.max(1));
    widgets::inset(ui, p, |ui| {
        widgets::info_row(
            ui,
            p,
            "Source",
            &format!("{} from {}", t.source_file, timecode::format(start)),
        );
        widgets::info_row(
            ui,
            p,
            "Planned",
            &format!("{} samples", count(t.expected_samples)),
        );
        widgets::info_row(
            ui,
            p,
            "In file",
            &format!("{} samples", count(t.actual_samples)),
        );
        widgets::info_row(
            ui,
            p,
            "Compared",
            &format!(
                "{} samples × {} channel(s)",
                count(t.compared_samples),
                t.channels.len()
            ),
        );
        match t.first_difference_seconds() {
            Some(s) => widgets::info_row(
                ui,
                p,
                "First difference",
                &format!("{} into the track", timecode::format(s)),
            ),
            None => widgets::info_row(ui, p, "Differences", "none"),
        }
    });

    widgets::caption(
        ui,
        p,
        "Integrity map: each column is a slice of the track. Green = identical, red = differing bits. Below, the file's low-band level, with differing slices in red.",
    );
    integrity_map(ui, p, t);
}

fn integrity_map(ui: &mut Ui, p: &Palette, t: &TrackCheck) {
    let strips = t.channels.len().min(MAX_STRIPS);
    let strips_h = strips as f32 * (STRIP_H + 2.0);
    let height = strips_h + 6.0 + ENVELOPE_H + 18.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let cw = rect.width() / BUCKETS as f32;

    let max_heat = t
        .channels
        .iter()
        .flat_map(|c| c.heat.iter().copied())
        .max()
        .unwrap_or(0);
    for (c, channel) in t.channels.iter().take(strips).enumerate() {
        let top = rect.top() + c as f32 * (STRIP_H + 2.0);
        for (b, heat) in channel.heat.iter().enumerate() {
            let x0 = rect.left() + b as f32 * cw;
            let cell = Rect::from_min_max(pos2(x0, top), pos2(x0 + cw + 0.6, top + STRIP_H));
            let color = if *heat == 0 {
                p.good.gamma_multiply(0.85)
            } else {
                let t = ((f64::from(*heat) + 1.0).ln() / (f64::from(max_heat) + 1.0).ln()) as f32;
                p.bad.gamma_multiply(0.6 + 0.4 * t)
            };
            painter.rect_filled(cell, CornerRadius::ZERO, color);
        }
    }

    let env_top = rect.top() + strips_h + 6.0;
    let mid = env_top + ENVELOPE_H / 2.0;
    let peak = t.level.iter().copied().fold(0.02f32, f32::max);
    for (b, level) in t.level.iter().enumerate() {
        let x0 = rect.left() + b as f32 * cw;
        let half = (level / peak * ENVELOPE_H / 2.0).max(1.0);
        let differs = t
            .channels
            .iter()
            .any(|c| c.heat.get(b).is_some_and(|h| *h > 0));
        let color = if differs {
            p.bad
        } else {
            p.accent.gamma_multiply(0.8)
        };
        let bar = Rect::from_min_max(pos2(x0, mid - half), pos2(x0 + cw + 0.6, mid + half));
        painter.rect_filled(bar, CornerRadius::ZERO, color);
    }
    painter.line_segment(
        [pos2(rect.left(), mid), pos2(rect.right(), mid)],
        Stroke::new(1.0_f32, p.rule),
    );

    let duration = t.compared_samples as f64 / f64::from(t.rate.max(1));
    for k in 0..=4u32 {
        let f = f64::from(k) / 4.0;
        let anchor = match k {
            0 => Align2::LEFT_TOP,
            4 => Align2::RIGHT_TOP,
            _ => Align2::CENTER_TOP,
        };
        painter.text(
            pos2(rect.left() + f as f32 * rect.width(), rect.bottom() - 14.0),
            anchor,
            timecode::format(duration * f),
            FontId::proportional(11.0),
            p.muted,
        );
    }

    if let Some(pos) = response.hover_pos() {
        let b = (((pos.x - rect.left()) / cw) as usize).min(BUCKETS - 1);
        let time = duration * (b as f64 + 0.5) / BUCKETS as f64;
        let mut text = format!("{}  (slice {} of {BUCKETS})", timecode::format(time), b + 1);
        for (c, channel) in t.channels.iter().enumerate() {
            let heat = channel.heat.get(b).copied().unwrap_or(0);
            text.push_str(&format!("\nChannel {}: {heat} differing bits", c + 1));
        }
        response.on_hover_text_at_pointer(text);
    }
}
