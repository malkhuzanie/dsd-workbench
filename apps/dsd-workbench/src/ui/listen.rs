//! Playback controls, "listen" spans for tracks, and the spectrum display.

use std::path::Path;

use eframe::egui::{self, Button, CornerRadius, Rect, RichText, Sense, Slider, Ui, pos2, vec2};

use super::theme::Palette;
use crate::audio::{Analyzer, Player, Quality, RateTarget, Request, Status};
use crate::project::Entry;
use crate::snapshot::SourceState;
use crate::timecode;

const BARS: usize = 56;
const SPECTRUM_HEIGHT: f32 = 64.0;
/// Seconds of context shown around a cut when auditioning.
const BEFORE_START: f64 = 2.0;
const AFTER_START: f64 = 3.0;
const BEFORE_END: f64 = 3.0;
const AFTER_END: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pick {
    Whole,
    Start,
    End,
}

/// Buttons shown on every track row: kind, label, hover text.
pub(super) const BUTTONS: [(Pick, &str, &str); 3] = [
    (Pick::Whole, "Track", "Play the whole track"),
    (
        Pick::Start,
        "Start",
        "Hear the cut at the start of this track",
    ),
    (Pick::End, "End", "Hear the cut at the end of this track"),
];

#[derive(Debug, Clone, Copy)]
struct Span {
    from: f64,
    to: f64,
}

/// The time ranges that can be auditioned for one track.
pub(super) struct Spans {
    whole: Span,
    start: Span,
    end: Span,
}

impl Spans {
    pub fn request(&self, pick: Pick, base_dir: &Path, source_file: &str, number: u32) -> Request {
        let (span, what) = match pick {
            Pick::Whole => (self.whole, "whole track"),
            Pick::Start => (self.start, "start cut"),
            Pick::End => (self.end, "end cut"),
        };
        Request {
            path: base_dir.join(source_file),
            start_seconds: span.from,
            end_seconds: Some(span.to),
            label: format!("Track {number:02} · {what}"),
        }
    }
}

/// `None` when the source is unreadable or the start time is invalid.
pub(super) fn spans(entry: &Entry, index: usize) -> Option<Spans> {
    let track = entry.plan.tracks.get(index)?;
    let source_len = match &entry.sources.get(&track.source_file)?.state {
        SourceState::Ready { duration_secs, .. } => *duration_secs,
        _ => return None,
    };
    let start = track.start_seconds;
    if !(start.is_finite() && start >= 0.0 && start < source_len) {
        return None;
    }
    let end = entry
        .plan
        .duration_of(index)
        .map_or(source_len, |d| start + d)
        .min(source_len);
    if end <= start {
        return None;
    }
    Some(Spans {
        whole: Span {
            from: start,
            to: end,
        },
        start: Span {
            from: (start - BEFORE_START).max(0.0),
            to: (start + AFTER_START).min(source_len),
        },
        end: Span {
            from: (end - BEFORE_END).max(0.0),
            to: (end + AFTER_END).min(source_len),
        },
    })
}

pub(super) struct ListenState {
    show_spectrum: bool,
    analyzer: Analyzer,
}

impl Default for ListenState {
    fn default() -> Self {
        Self {
            show_spectrum: false,
            analyzer: Analyzer::new(BARS),
        }
    }
}

/// The player strip: transport, seek, volume, audio settings, and the optional spectrum.
pub(super) fn bar(ui: &mut Ui, p: &Palette, player: &mut Player, state: &mut ListenState) {
    let status = player.status();
    let active = status != Status::Stopped;
    let mut seek = None;

    ui.horizontal(|ui| {
        let label = if status == Status::Playing {
            "Pause"
        } else {
            "Resume"
        };
        if ui.add_enabled(active, Button::new(label)).clicked() {
            player.toggle_pause();
        }
        if ui.add_enabled(active, Button::new("Stop")).clicked() {
            player.stop();
        }

        let duration = player.duration_seconds().max(0.001);
        let mut position = player.position_seconds().min(duration);
        ui.spacing_mut().slider_width = (ui.available_width() - 450.0).max(80.0);
        let slider = ui.add_enabled(
            active,
            Slider::new(&mut position, 0.0..=duration)
                .show_value(false)
                .trailing_fill(true),
        );
        if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
            seek = Some(position);
        }
        ui.label(
            RichText::new(format!(
                "{} / {}",
                timecode::format(position),
                timecode::format(duration)
            ))
            .monospace(),
        );

        ui.spacing_mut().slider_width = 70.0;
        let mut volume = player.volume();
        ui.label(RichText::new("Vol").color(p.muted));
        if ui
            .add(Slider::new(&mut volume, 0.0..=1.0).show_value(false))
            .changed()
        {
            player.set_volume(volume);
        }

        let mut settings = player.settings();
        ui.menu_button("Audio", |ui| {
            ui.set_min_width(250.0);
            ui.label(RichText::new("Output sample rate").strong());
            for option in RateTarget::ALL {
                ui.radio_value(&mut settings.rate, option, option.label());
            }
            ui.separator();
            ui.label(RichText::new("Filter").strong());
            for option in Quality::ALL {
                ui.radio_value(&mut settings.quality, option, option.label());
            }
            ui.separator();
            ui.label(
                RichText::new(
                    "Applies the next time you press a listen button.\n\
                     The rate is limited by the DSD rate, and your device may resample.",
                )
                .small()
                .color(p.muted),
            );
        });
        if settings != player.settings() {
            player.set_settings(settings);
        }
        ui.checkbox(&mut state.show_spectrum, "Spectrum");
    });
    if let Some(position) = seek {
        player.seek(position);
    }

    let text = match (player.label(), player.sample_rate()) {
        (Some(label), 0) => format!("{label} · opening…"),
        (Some(label), rate) => format!("{label} · {:.1} kHz", f64::from(rate) / 1000.0),
        (None, _) => "Nothing playing. Use Track / Start / End on a row to listen.".to_owned(),
    };
    ui.label(RichText::new(text).small().color(p.muted));

    if state.show_spectrum {
        if status == Status::Playing {
            let samples = player.recent_samples();
            state.analyzer.update(&samples, player.sample_rate());
        } else {
            state.analyzer.decay();
        }
        spectrum(ui, p, &state.analyzer);
    }
}

fn spectrum(ui: &mut Ui, p: &Palette, analyzer: &Analyzer) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), SPECTRUM_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(6), p.button);

    let bars = analyzer.bars();
    let slot = rect.width() / bars.len().max(1) as f32;
    for (i, level) in bars.iter().enumerate() {
        let x0 = rect.left() + i as f32 * slot;
        let top = rect.bottom() - 4.0 - level.clamp(0.0, 1.0) * (rect.height() - 8.0);
        let bar = Rect::from_min_max(
            pos2(x0 + 1.0, top),
            pos2(x0 + slot - 1.0, rect.bottom() - 4.0),
        );
        painter.rect_filled(bar, CornerRadius::same(1), p.accent);
    }
}

/// Convenience so `detail` need not import `egui` for the hint text.
pub(super) fn disabled_hint(response: egui::Response) -> egui::Response {
    response.on_disabled_hover_text("Needs a readable source file and a valid start time")
}
