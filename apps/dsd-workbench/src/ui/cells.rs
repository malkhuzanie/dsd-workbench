//! Small editable cells for the data grids.

use eframe::egui::{Id, Label, Response, RichText, TextEdit, TextStyle, Ui};

use super::theme::Palette;
use crate::timecode;

pub fn head(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).small().strong().color(p.muted));
}

/// A checkbox centred in its table cell. egui gives an empty checkbox the width of a full
/// button and draws the box at its left edge, so the width is trimmed to the box itself.
pub fn checkbox(ui: &mut Ui, value: &mut bool) -> Response {
    let width = ui.spacing().icon_width;
    let pad = ((ui.available_width() - width) / 2.0 - ui.spacing().item_spacing.x).max(0.0);
    ui.add_space(pad);
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.x = width;
        ui.checkbox(value, "")
    })
    .inner
}

pub fn muted(ui: &mut Ui, p: &Palette, text: impl Into<String>) {
    ui.add(Label::new(RichText::new(text.into()).color(p.muted)).truncate());
}

pub fn mono(ui: &mut Ui, text: impl Into<String>) {
    ui.add(Label::new(RichText::new(text.into()).monospace()).truncate());
}

/// A text cell; tinted amber when it differs from the CUE-derived original.
pub fn text(ui: &mut Ui, p: &Palette, value: &mut String, original: Option<&str>) {
    let edited = original.is_some_and(|o| o != value.as_str());
    let mut edit = TextEdit::singleline(value).desired_width(f32::INFINITY);
    if edited {
        edit = edit.background_color(p.modified);
    }
    ui.add(edit);
}

/// An editable time stamp. While focused the raw text is kept in memory so
/// half-typed input (e.g. "1:") is not rewritten under the cursor; the value
/// only changes when the text parses.
pub fn time(ui: &mut Ui, p: &Palette, id: Id, value: &mut f64, original: Option<f64>) {
    let buffer_id = id.with("buffer");
    let editing = ui.memory(|m| m.has_focus(id));
    let mut text = if editing {
        ui.data(|d| d.get_temp::<String>(buffer_id))
            .unwrap_or_else(|| timecode::format(*value))
    } else {
        timecode::format(*value)
    };

    let invalid = editing && timecode::parse(&text).is_none();
    let edited = original.is_some_and(|o| (o - *value).abs() > 0.0005);
    let mut edit = TextEdit::singleline(&mut text)
        .id(id)
        .font(TextStyle::Monospace)
        .desired_width(f32::INFINITY);
    if invalid {
        edit = edit.background_color(p.removed);
    } else if edited {
        edit = edit.background_color(p.modified);
    }

    let response = ui.add(edit);
    if response.changed() {
        if let Some(parsed) = timecode::parse(&text) {
            *value = parsed;
        }
        ui.data_mut(|d| d.insert_temp(buffer_id, text.clone()));
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(buffer_id));
    }
}
