//! Editors for tag fields: the album's defaults and the selected tracks' overrides.

use eframe::egui::{self, Button, Id, RichText, TextEdit, Ui};

use super::theme::Palette;
use super::widgets;
use crate::metadata::{FieldKey, Standard};
use crate::plan::{Shared, SplitPlan};
use crate::snapshot::TagSnapshot;

const LABEL_W: f32 = 130.0;
const VALUE_W: f32 = 320.0;
const NAME_W: f32 = 150.0;
const NEW_VALUE_W: f32 = 220.0;

/// Album-wide defaults: every field that can apply to the whole album, then custom fields.
///
/// `current` is what the first source file is tagged with today.
pub(super) fn album_editor(ui: &mut Ui, p: &Palette, plan: &mut SplitPlan, current: &TagSnapshot) {
    widgets::column_headers(ui, p, "Current (first source file)", "Proposed");
    for standard in Standard::ALL.into_iter().filter(|s| !s.track_only()) {
        let key = FieldKey::Standard(standard);
        let mut text = plan.album.value(&key).to_owned();
        widgets::text_field(ui, p, standard.label(), current.value(&key), &mut text);
        plan.album.fields.set_or_clear(key, &text);
    }

    widgets::section(ui, p, "Custom fields");
    widgets::caption(
        ui,
        p,
        "Any extra tag (for example MOOD or CATALOGNUMBER), written to every track unless a track has its own value. Clear a value to remove the field.",
    );
    for key in plan.custom_keys(&[]) {
        let mut text = plan.album.value(&key).to_owned();
        widgets::text_field(ui, p, key.label(), current.value(&key), &mut text);
        plan.album.fields.set_or_clear(key, &text);
    }
    if let Some((key, value)) = new_field_row(ui, Id::new("album_new_field")) {
        plan.album.fields.set(key, value);
    }
}

/// Edits one or several tracks at once. Values that differ from the album default become
/// per-track overrides; "Use album value" removes the override again.
///
/// Returns the new row of the track whose details were moved, so the selection can follow.
pub(super) fn track_editor(
    ui: &mut Ui,
    p: &Palette,
    plan: &mut SplitPlan,
    rows: &[usize],
) -> Option<usize> {
    let count = rows.len();
    let moved = reorder_buttons(ui, plan, rows);
    let heading = if count == 1 {
        let title = rows
            .first()
            .and_then(|row| plan.tracks.get(*row))
            .map(|track| plan.album.resolve(&track.meta, &FieldKey::TITLE))
            .unwrap_or_default();
        format!("Selected track: {title}")
    } else {
        format!("{count} selected tracks")
    };
    ui.label(RichText::new(heading).strong());
    widgets::caption(
        ui,
        p,
        "Changes apply to every selected track. “track” marks a value that differs from the album default.",
    );

    for standard in Standard::ALL {
        // A title or ISRC shared by several tracks is never what anyone wants.
        if standard.track_only() && count > 1 {
            continue;
        }
        field_row(ui, p, plan, rows, &FieldKey::Standard(standard));
    }
    for key in plan.custom_keys(rows) {
        field_row(ui, p, plan, rows, &key);
    }
    if let Some((key, value)) = new_field_row(ui, Id::new("track_new_field")) {
        plan.assign_tracks(rows, &key, &value);
    }
    moved
}

/// Buttons to fix a track list that is in the wrong order. Each track keeps its time slot;
/// the title, artist and other per-track tags move between slots.
fn reorder_buttons(ui: &mut Ui, plan: &mut SplitPlan, rows: &[usize]) -> Option<usize> {
    let mut moved = None;
    ui.horizontal(|ui| {
        if let [row] = rows {
            let (up, down) = (
                ui.add_enabled(*row > 0, Button::new("Move title up"))
                    .on_hover_text("Swap this track's title and tags with the track above; the time slots stay")
                    .clicked(),
                ui.add_enabled(row + 1 < plan.tracks.len(), Button::new("Move title down"))
                    .on_hover_text("Swap this track's title and tags with the track below; the time slots stay")
                    .clicked(),
            );
            let target = if up {
                row.checked_sub(1)
            } else if down {
                Some(row + 1)
            } else {
                None
            };
            if let Some(target) = target
                && plan.swap_track_details(*row, target)
            {
                moved = Some(target);
            }
        }
        if ui
            .button("Renumber 1…N")
            .on_hover_text("Number the tracks 1, 2, 3 … in the order shown")
            .clicked()
        {
            plan.renumber();
        }
    });
    moved
}

/// One editable value for the selected tracks, with its origin and a way back to the album value.
fn field_row(ui: &mut Ui, p: &Palette, plan: &mut SplitPlan, rows: &[usize], key: &FieldKey) {
    let (mut text, hint) = match plan.shared_value(rows, key) {
        Shared::Same(value) => (value, ""),
        Shared::Mixed => (String::new(), "(mixed values)"),
        Shared::Nothing => return,
    };
    let overridden = plan.any_override(rows, key);

    let mut edited = false;
    let mut inherit = false;
    ui.horizontal(|ui| {
        ui.add_sized(
            [LABEL_W, 22.0],
            egui::Label::new(RichText::new(key.label()).strong()),
        );
        edited = ui
            .add(
                TextEdit::singleline(&mut text)
                    .hint_text(hint)
                    .desired_width(VALUE_W),
            )
            .changed();
        if overridden {
            ui.label(RichText::new("track").small().color(p.accent));
        } else {
            ui.label(RichText::new("album").small().color(p.muted));
        }
        inherit = ui
            .add_enabled(overridden, Button::new("Use album value").small())
            .clicked();
    });

    if edited {
        plan.assign_tracks(rows, key, &text);
    } else if inherit {
        plan.inherit_tracks(rows, key);
    }
}

/// A name and value box with an Add button; returns the new field when it is added.
fn new_field_row(ui: &mut Ui, id: Id) -> Option<(FieldKey, String)> {
    let (mut name, mut value) = ui
        .data(|data| data.get_temp::<(String, String)>(id))
        .unwrap_or_default();
    let mut added = None;
    ui.horizontal(|ui| {
        ui.add_sized(
            [LABEL_W, 22.0],
            egui::Label::new(RichText::new("Add custom field").strong()),
        );
        ui.add(
            TextEdit::singleline(&mut name)
                .hint_text("Name, e.g. MOOD")
                .desired_width(NAME_W),
        );
        ui.add(
            TextEdit::singleline(&mut value)
                .hint_text("Value")
                .desired_width(NEW_VALUE_W),
        );
        let key = FieldKey::custom(&name);
        let ready = key.is_some() && !value.trim().is_empty();
        if ui.add_enabled(ready, Button::new("Add")).clicked() {
            added = key.map(|key| (key, value.trim().to_owned()));
        }
    });
    if added.is_some() {
        name.clear();
        value.clear();
    }
    ui.data_mut(|data| data.insert_temp(id, (name, value)));
    added
}
