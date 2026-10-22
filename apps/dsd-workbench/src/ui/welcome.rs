//! The empty screen: an animation of what the app does (a DSD stream flows by, a cut sweeps
//! across it at the track starts, the tracks drift apart and are checked bit by bit, a result
//! badge appears, and the tracks close up again), the five steps, and the shortcuts. The
//! animation is the drop target, so its frame lights up when files hover.

use std::time::Duration;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Frame, Margin, RichText, Stroke};

use super::theme::{self, Palette};

/// The steps shown on the empty screen, in order. `(heading, explanation)`.
const STEPS: [(&str, &str); 5] = [
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

const SHORTCUTS: [(&str, &str); 4] = [
    ("Ctrl+O", "add CUE files"),
    ("Ctrl+S", "save project"),
    ("Ctrl+Z", "undo"),
    ("Ctrl+Y", "redo"),
];

/// Seconds for one pass of the animation.
const PERIOD: f32 = 18.0;
/// Tracks drawn in the animation.
const TRACKS: usize = 4;
/// Where the tracks start, as fractions of the stream (uneven, like real tracks).
const STARTS: [f32; TRACKS + 1] = [0.0, 0.22, 0.5, 0.74, 1.0];
/// The times of the cuts after the first track, as a CUE sheet would give them.
const CUT_TIMES: [&str; TRACKS - 1] = ["04:12.40", "09:31.07", "14:05.73"];
const STAGE_HEIGHT: f32 = 300.0;
const MAX_WIDTH: f32 = 1040.0;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette) {
    ui.ctx().request_repaint_after(Duration::from_millis(33));
    let time = ui.input(|i| i.time) as f32;
    let dragging = ui.input(|i| !i.raw.hovered_files.is_empty());

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width().min(MAX_WIDTH);
            let side = ((ui.available_width() - width) / 2.0).max(0.0);
            ui.horizontal_top(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(theme::space::SM);
                    ui.label(RichText::new("DSD Workbench").size(34.0).strong());
                    ui.label(
                        RichText::new(
                            "Split whole-side DSD files into tracks without touching the audio, then prove every bit survived.",
                        )
                        .size(16.0)
                        .color(p.muted),
                    );
                    ui.add_space(theme::space::LG);
                    stage(ui, p, time, dragging);
                    ui.add_space(theme::space::LG);
                    steps(ui, p, width);
                    ui.add_space(theme::space::LG);
                    shortcuts(ui, p);
                    ui.add_space(theme::space::LG);
                });
            });
        });
}

// ---- the animation ---------------------------------------------------------

/// Where each part of the animation is, 0 to 1, at a moment of the loop. Pure, so the
/// timeline can be tested.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Phase {
    /// The cut scan: 0 before it starts, 1 once it has passed the last track start.
    scan: f32,
    /// How far the tracks have drifted apart.
    spread: f32,
    /// Progress of the verification sweep over each track.
    verify: [f32; TRACKS],
    /// How much of the "verified" colour is still showing (it fades as the tracks close up).
    verified: f32,
    /// The result badge: how far it has appeared.
    badge: f32,
}

/// Smooth 0 to 1 ramp between `from` and `to`.
fn smooth(from: f32, to: f32, t: f32) -> f32 {
    let x = ((t - from) / (to - from)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn phase(seconds: f32) -> Phase {
    let t = seconds.rem_euclid(PERIOD);
    let mut verify = [0.0; TRACKS];
    for (k, slot) in verify.iter_mut().enumerate() {
        let start = 7.6 + k as f32 * 1.15;
        *slot = smooth(start, start + 1.15, t);
    }
    Phase {
        // Sweeps left to right over the stream, then is gone before the tracks drift apart.
        scan: smooth(2.0, 4.0, t),
        spread: smooth(4.6, 7.0, t) - smooth(14.8, 17.2, t),
        verify,
        verified: 1.0 - smooth(15.4, 17.0, t),
        badge: smooth(12.6, 13.4, t) - smooth(15.0, 15.8, t),
    }
}

/// A cheap, stable pseudo-random value for a cell.
fn noise(a: usize, b: usize) -> u32 {
    let mut x = (a as u32).wrapping_mul(0x9E37_79B1) ^ (b as u32).wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^ (x >> 12)
}

fn stage(ui: &mut egui::Ui, p: &Palette, time: f32, dragging: bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), STAGE_HEIGHT),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let pulse = if dragging {
        0.5 + 0.5 * (time * 6.0).sin()
    } else {
        0.0
    };

    painter.rect_filled(rect, CornerRadius::same(theme::radius::CARD), p.raised);
    drifting_dots(&painter, p, rect, time);

    // A dashed outline says "drop here"; it turns accent-coloured while files hover.
    let outline = if dragging {
        p.accent.gamma_multiply(0.55 + 0.45 * pulse)
    } else {
        p.rule
    };
    let stroke = Stroke::new(if dragging { 2.0 } else { 1.0 }, outline);
    painter.extend(egui::Shape::dashed_line(
        &[
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ],
        stroke,
        7.0,
        5.0,
    ));

    let phase = phase(time);
    paint_stream(&painter, p, rect, time, &phase);

    let hint = if dragging {
        "Release to add"
    } else {
        "Drop CUE sheets, folders or DSD files anywhere in this window"
    };
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 18.0),
        Align2::CENTER_CENTER,
        hint,
        FontId::proportional(14.0),
        if dragging { p.accent } else { p.muted },
    );
}

/// Faint specks that rise slowly through the stage, like the drifting bits behind the website.
fn drifting_dots(painter: &egui::Painter, p: &Palette, rect: egui::Rect, time: f32) {
    for i in 0..34 {
        let x = rect.left() + (noise(i, 1) % 1000) as f32 / 1000.0 * rect.width();
        let speed = 5.0 + (noise(i, 2) % 100) as f32 / 100.0 * 11.0;
        let travel = (noise(i, 3) % 1000) as f32 / 1000.0 * rect.height() - time * speed;
        let y = rect.top() + travel.rem_euclid(rect.height());
        let radius = 1.0 + (noise(i, 4) % 3) as f32 * 0.45;
        // Fade near the top and bottom edges so specks never pop in or out.
        let edge = ((y - rect.top()).min(rect.bottom() - y) / 30.0).clamp(0.0, 1.0);
        painter.circle_filled(
            egui::pos2(x, y),
            radius,
            p.accent.gamma_multiply(0.20 * edge),
        );
    }
}

fn paint_stream(painter: &egui::Painter, p: &Palette, rect: egui::Rect, time: f32, phase: &Phase) {
    const GAP: f32 = 40.0; // the gap between tracks at full spread
    let usable = rect.width() * 0.9 - (TRACKS as f32 - 1.0) * GAP;
    let columns = 62_usize;
    let pitch = usable / columns as f32;
    let total = usable + (TRACKS as f32 - 1.0) * GAP * phase.spread;
    let left = rect.center().x - total / 2.0;
    let centre_y = rect.center().y - 20.0;
    let lanes = [-22.0_f32, 22.0];
    let (cell_w, cell_h) = (pitch * 0.58, 28.0);

    let bounds: Vec<usize> = STARTS
        .iter()
        .map(|f| (f * columns as f32).round() as usize)
        .collect();
    let track_of = |column: usize| {
        (0..TRACKS)
            .find(|k| column < bounds[k + 1])
            .unwrap_or(TRACKS - 1)
    };
    let x_of =
        |column: usize| left + column as f32 * pitch + track_of(column) as f32 * GAP * phase.spread;
    // The ribbon undulates gently, and its bits flow to the left.
    let wave = |x: f32, lane: usize| (x * 0.014 + time * 1.4 + lane as f32 * 0.9).sin() * 7.0;
    let flow = (time * 7.0) as usize;

    for column in 0..columns {
        let k = track_of(column);
        let within = (column - bounds[k]) as f32 / (bounds[k + 1] - bounds[k]).max(1) as f32;
        let checked = phase.verify[k] > within && phase.verified > 0.0;
        let x = x_of(column);
        for (lane, offset) in lanes.iter().enumerate() {
            // A bit's value moves one column at a time, so the stream seems to flow.
            let on = !noise(column + flow, lane).is_multiple_of(3);
            let shimmer = 0.8 + 0.2 * (time * 2.1 + column as f32 * 0.45 + lane as f32 * 2.0).sin();
            let (base, glow) = if on {
                (lerp_colour(p.muted, p.accent, 0.55), p.accent)
            } else {
                (p.rule, p.rule)
            };
            let (colour, glow) = if checked {
                let tint = if on {
                    p.good
                } else {
                    p.good.gamma_multiply(0.30)
                };
                (
                    lerp_colour(base, tint, phase.verified),
                    lerp_colour(glow, p.good, phase.verified),
                )
            } else {
                (base, glow)
            };
            let centre = egui::pos2(x + cell_w / 2.0, centre_y + offset + wave(x, lane));
            if on {
                painter.rect_filled(
                    egui::Rect::from_center_size(centre, egui::vec2(cell_w + 5.0, cell_h + 5.0)),
                    CornerRadius::same(4),
                    glow.gamma_multiply(0.10 * shimmer),
                );
            }
            painter.rect_filled(
                egui::Rect::from_center_size(centre, egui::vec2(cell_w, cell_h)),
                CornerRadius::same(3),
                colour.gamma_multiply(if on { 0.78 + 0.22 * shimmer } else { 1.0 }),
            );
        }
    }

    cut_scan(painter, p, rect, left, pitch, centre_y, &bounds, phase);
    verify_sweeps(painter, p, &bounds, &x_of, pitch, centre_y, wave, phase);
    labels_and_badge(painter, p, rect, centre_y, &bounds, &x_of, cell_w, phase);
}

/// A bright line sweeps left to right; as it passes each track start it leaves a cut mark.
#[allow(clippy::too_many_arguments)]
fn cut_scan(
    painter: &egui::Painter,
    p: &Palette,
    rect: egui::Rect,
    left: f32,
    pitch: f32,
    centre_y: f32,
    bounds: &[usize],
    phase: &Phase,
) {
    let visible = (1.0 - phase.spread * 3.0).clamp(0.0, 1.0);
    if visible <= 0.0 || phase.scan <= 0.0 {
        return;
    }
    let (top, bottom) = (centre_y - 62.0, centre_y + 62.0);
    let start = left;
    let end = left + bounds[TRACKS] as f32 * pitch;
    let head = start + (end - start) * phase.scan;

    for (index, cut) in bounds[1..TRACKS].iter().enumerate() {
        let x = left + *cut as f32 * pitch - pitch * 0.2;
        if head >= x {
            // The time the cut lands on, as a CUE sheet would give it.
            painter.text(
                egui::pos2(x, top - 10.0),
                Align2::CENTER_CENTER,
                CUT_TIMES[index],
                FontId::monospace(11.5),
                p.accent.gamma_multiply(visible),
            );
            // A mark with a spark where the scan crossed it.
            let age = ((head - x) / (end - start) * 10.0).min(1.0);
            painter.line_segment(
                [egui::pos2(x, top), egui::pos2(x, bottom)],
                Stroke::new(2.0, p.accent.gamma_multiply(visible)),
            );
            let spark = (1.0 - age).max(0.0);
            painter.circle_filled(
                egui::pos2(x, centre_y),
                4.0 + 10.0 * spark,
                p.accent.gamma_multiply(0.25 * spark * visible),
            );
        }
    }
    if phase.scan < 1.0 {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(head - 14.0, top),
                egui::pos2(head + 14.0, bottom),
            ),
            CornerRadius::same(10),
            p.accent.gamma_multiply(0.16 * visible),
        );
        painter.line_segment(
            [egui::pos2(head, top - 6.0), egui::pos2(head, bottom + 6.0)],
            Stroke::new(2.5, p.accent),
        );
    }
    let _ = rect;
}

/// The green bar that checks each track in turn.
#[allow(clippy::too_many_arguments)]
fn verify_sweeps(
    painter: &egui::Painter,
    p: &Palette,
    bounds: &[usize],
    x_of: &dyn Fn(usize) -> f32,
    pitch: f32,
    centre_y: f32,
    wave: impl Fn(f32, usize) -> f32,
    phase: &Phase,
) {
    for k in 0..TRACKS {
        let v = phase.verify[k];
        if v <= 0.0 || v >= 1.0 || phase.verified <= 0.0 {
            continue;
        }
        let first = bounds[k];
        let span = (bounds[k + 1] - bounds[k]) as f32;
        let x = x_of(first) + v * span * pitch;
        let (top, bottom) = (centre_y - 50.0, centre_y + 50.0);
        let drift = wave(x, 0) * 0.5;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x - 16.0, top + drift),
                egui::pos2(x + 16.0, bottom + drift),
            ),
            CornerRadius::same(12),
            p.good.gamma_multiply(0.16),
        );
        painter.line_segment(
            [egui::pos2(x, top + drift), egui::pos2(x, bottom + drift)],
            Stroke::new(2.5, p.good),
        );
    }
}

/// Track numbers while the tracks are apart (with a tick once checked) and the result badge.
#[allow(clippy::too_many_arguments)]
fn labels_and_badge(
    painter: &egui::Painter,
    p: &Palette,
    rect: egui::Rect,
    centre_y: f32,
    bounds: &[usize],
    x_of: &dyn Fn(usize) -> f32,
    cell_w: f32,
    phase: &Phase,
) {
    if phase.spread > 0.05 {
        for k in 0..TRACKS {
            let (first, last) = (bounds[k], bounds[k + 1].saturating_sub(1));
            let middle = (x_of(first) + x_of(last) + cell_w) / 2.0;
            let y = centre_y - 66.0;
            let alpha = phase.spread;
            let chip = egui::Rect::from_center_size(egui::pos2(middle, y), egui::vec2(66.0, 22.0));
            painter.rect_filled(chip, CornerRadius::same(11), p.paper.gamma_multiply(alpha));
            painter.rect_stroke(
                chip,
                CornerRadius::same(11),
                Stroke::new(1.0, p.rule.gamma_multiply(alpha)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                egui::pos2(middle - 9.0, y),
                Align2::CENTER_CENTER,
                format!("Track {}", k + 1),
                FontId::proportional(11.5),
                p.ink.gamma_multiply(alpha),
            );
            if phase.verify[k] >= 1.0 && phase.verified > 0.0 {
                let tick = egui::pos2(chip.right() - 11.0, y);
                let colour = p.good.gamma_multiply(phase.verified * alpha);
                painter.line_segment(
                    [tick + egui::vec2(-3.5, 0.0), tick + egui::vec2(-1.0, 3.0)],
                    Stroke::new(2.0, colour),
                );
                painter.line_segment(
                    [tick + egui::vec2(-1.0, 3.0), tick + egui::vec2(4.0, -3.5)],
                    Stroke::new(2.0, colour),
                );
            }
        }
    }

    if phase.badge > 0.01 {
        let centre = egui::pos2(rect.center().x, centre_y + 76.0);
        let badge = egui::Rect::from_center_size(centre, egui::vec2(260.0, 30.0));
        let alpha = phase.badge;
        painter.rect_filled(
            badge,
            CornerRadius::same(15),
            p.good.gamma_multiply(0.16 * alpha),
        );
        painter.rect_stroke(
            badge,
            CornerRadius::same(15),
            Stroke::new(1.0, p.good.gamma_multiply(alpha)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            centre,
            Align2::CENTER_CENTER,
            "4 of 4 tracks bit-identical",
            FontId::proportional(14.0),
            p.good.gamma_multiply(alpha),
        );
    }
}

fn lerp_colour(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
        mix(from.a(), to.a()),
    )
}

// ---- the steps -------------------------------------------------------------

fn steps(ui: &mut egui::Ui, p: &Palette, width: f32) {
    let gap = theme::space::MD;
    let per_row = if width >= 900.0 {
        5
    } else if width >= 560.0 {
        3
    } else {
        1
    };
    let card_width = (width - gap * (per_row as f32 - 1.0)) / per_row as f32;
    egui::Grid::new("welcome_steps")
        .spacing([gap, gap])
        .show(ui, |ui| {
            for (index, (heading, text)) in STEPS.iter().enumerate() {
                step_card(ui, p, card_width, index + 1, heading, text);
                if (index + 1) % per_row == 0 {
                    ui.end_row();
                }
            }
        });
}

fn step_card(ui: &mut egui::Ui, p: &Palette, width: f32, number: usize, heading: &str, text: &str) {
    // The frame's outline adds a point on each side.
    let inner = width - 2.0 * theme::space::MD - 2.0;
    Frame::new()
        .fill(p.raised)
        .stroke(Stroke::new(1.0, p.rule))
        .corner_radius(CornerRadius::same(theme::radius::CARD))
        .inner_margin(Margin::same(theme::space::MD as i8))
        .show(ui, |ui| {
            // A grid cell lays out left to right with unlimited width, so the card pins its own
            // width and stacks its content.
            ui.set_width(inner);
            ui.set_min_height(168.0);
            ui.vertical(|ui| {
                ui.set_max_width(inner);
                let (badge, _) =
                    ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                ui.painter().circle_filled(badge.center(), 14.0, p.accent);
                ui.painter().text(
                    badge.center(),
                    Align2::CENTER_CENTER,
                    number.to_string(),
                    FontId::proportional(14.0),
                    p.paper,
                );
                ui.add_space(theme::space::SM);
                ui.add(egui::Label::new(RichText::new(heading).size(15.5).color(p.ink)).wrap());
                ui.add_space(theme::space::XS);
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.muted)).wrap());
            });
        });
}

// ---- the shortcuts ---------------------------------------------------------

fn shortcuts(ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = theme::space::MD;
        for (keys, action) in SHORTCUTS {
            Frame::new()
                .fill(p.button)
                .stroke(Stroke::new(1.0, p.rule))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.label(RichText::new(keys).monospace().size(12.0).color(p.ink));
                });
            ui.label(RichText::new(action).size(12.5).color(p.muted));
            ui.add_space(theme::space::SM);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_starts_and_ends_at_rest() {
        // Nothing is showing at the start, and nothing is left showing at the end, so the
        // loop restarts without a jump.
        for t in [0.0, PERIOD - 0.01, PERIOD] {
            let p = phase(t);
            assert!(p.spread.abs() < 0.01, "{t}: {p:?}");
            assert!(p.badge.abs() < 0.01, "{t}: {p:?}");
        }
        let start = phase(0.0);
        assert!(start.scan == 0.0 && start.verify.iter().all(|v| *v == 0.0));
        assert!(phase(PERIOD - 0.01).verified < 0.01);
    }

    #[test]
    fn the_tracks_are_apart_while_they_are_checked() {
        let p = phase(9.0);
        assert!(p.spread > 0.99, "{p:?}");
        assert!(p.verified > 0.99);
        // Tracks are checked one after another.
        assert!(p.verify[0] >= p.verify[1] && p.verify[1] >= p.verify[2]);
    }

    #[test]
    fn the_badge_appears_after_every_track_is_checked() {
        let all_checked = (0..180)
            .map(|i| i as f32 * 0.1)
            .find(|t| phase(*t).verify.iter().all(|v| *v >= 1.0))
            .expect("every track is checked at some point");
        let badge_at = (0..180)
            .map(|i| i as f32 * 0.1)
            .find(|t| phase(*t).badge > 0.5)
            .expect("the badge appears");
        assert!(badge_at >= all_checked - 0.5, "{badge_at} vs {all_checked}");
    }

    #[test]
    fn every_value_stays_in_range_over_the_whole_loop() {
        let mut t = 0.0;
        while t < PERIOD * 2.0 {
            let p = phase(t);
            for v in p
                .verify
                .iter()
                .chain([&p.scan, &p.spread, &p.verified, &p.badge])
            {
                assert!((-0.001..=1.001).contains(v), "{t}: {p:?}");
            }
            t += 0.05;
        }
    }

    #[test]
    fn the_track_starts_are_ordered_and_span_the_stream() {
        assert_eq!(STARTS[0], 0.0);
        assert_eq!(STARTS[TRACKS], 1.0);
        assert!(STARTS.windows(2).all(|w| w[0] < w[1]));
    }
}
