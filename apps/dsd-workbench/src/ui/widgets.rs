use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Stroke, TextEdit, Ui, vec2,
};

use super::theme::{self, Palette};

const INFO_LABEL_W: f32 = 72.0;
const FIELD_LABEL_W: f32 = 120.0;
const BADGE_W: f32 = 100.0;
const GAP: f32 = 12.0;
const ROW_H: f32 = 30.0;

/// Visual tone of a [`banner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Warn,
    Bad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diff {
    Unchanged,
    Added,
    Modified,
    Removed,
}

impl Diff {
    pub fn between(current: &str, proposed: &str) -> Self {
        let (current, proposed) = (current.trim(), proposed.trim());
        match (current.is_empty(), proposed.is_empty()) {
            (true, true) => Self::Unchanged,
            (true, false) => Self::Added,
            (false, true) => Self::Removed,
            (false, false) if current == proposed => Self::Unchanged,
            (false, false) => Self::Modified,
        }
    }

    fn tint(self, p: &Palette) -> Option<Color32> {
        match self {
            Self::Unchanged => None,
            Self::Added => Some(p.added),
            Self::Modified => Some(p.modified),
            Self::Removed => Some(p.removed),
        }
    }

    fn badge(self) -> Option<&'static str> {
        match self {
            Self::Unchanged => None,
            Self::Added => Some("new"),
            Self::Modified => Some("changed"),
            Self::Removed => Some("will be cleared"),
        }
    }
}

pub fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(26.0).strong());
}

pub fn caption(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).small().color(p.muted));
}

/// A small upper-case heading that starts a group of related content.
pub fn section(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .small()
            .strong()
            .color(p.muted),
    );
}

/// A recessed panel for read-only facts, set apart from the editable fields.
pub fn inset<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(p.button)
        .stroke(Stroke::new(1.0_f32, p.rule))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A full-width coloured strip for status messages.
pub fn banner(ui: &mut Ui, p: &Palette, tone: Tone, text: &str) {
    let (fill, ink) = match tone {
        Tone::Info => (p.added, p.ink),
        Tone::Warn => (p.modified, p.ink),
        Tone::Bad => (p.removed, p.danger),
    };
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).color(ink)).wrap());
        });
}

/// A muted label with its value beside it; long values wrap beneath themselves.
fn info(ui: &mut Ui, p: &Palette, label: &str, value: RichText) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        ui.vertical(|ui| {
            ui.set_min_width(INFO_LABEL_W);
            ui.label(RichText::new(label).color(p.muted));
        });
        ui.add(egui::Label::new(value).wrap());
    });
}

pub fn info_row(ui: &mut Ui, p: &Palette, label: &str, value: &str) {
    info(ui, p, label, RichText::new(value));
}

/// Allocates a fixed-width cell so that rows of fields line up exactly.
fn cell<R>(ui: &mut Ui, width: f32, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        vec2(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            // Reserve the whole cell, or the next one starts right after the content.
            ui.set_min_size(vec2(width, height));
            add(ui)
        },
    )
    .inner
}

/// Width of each of the two value columns.
fn field_width(ui: &Ui) -> f32 {
    ((ui.available_width() - FIELD_LABEL_W - BADGE_W - 3.0 * GAP) / 2.0).max(100.0)
}

pub fn column_headers(ui: &mut Ui, p: &Palette, left: &str, right: &str) {
    let w = field_width(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        cell(ui, FIELD_LABEL_W, 18.0, |_| {});
        cell(ui, w, 18.0, |ui| {
            ui.label(RichText::new(left).small().color(p.muted));
        });
        cell(ui, w, 18.0, |ui| {
            ui.label(RichText::new(right).small().color(p.muted));
        });
    });
}

/// One row: label, read-only current value, editable proposal, change badge.
pub fn text_field(ui: &mut Ui, p: &Palette, label: &str, current: &str, proposed: &mut String) {
    let diff = Diff::between(current, proposed);
    let w = field_width(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        cell(ui, FIELD_LABEL_W, ROW_H, |ui| {
            ui.label(RichText::new(label).strong());
        });
        cell(ui, w, ROW_H, |ui| read_only(ui, current, w));
        cell(ui, w, ROW_H, |ui| {
            let mut edit = TextEdit::singleline(proposed).desired_width(w);
            if let Some(tint) = diff.tint(p) {
                edit = edit.background_color(tint);
            }
            ui.add(edit);
        });
        badge_cell(ui, p, diff);
    });
}

fn badge_cell(ui: &mut Ui, p: &Palette, diff: Diff) {
    cell(ui, BADGE_W, ROW_H, |ui| {
        if let Some(badge) = diff.badge() {
            ui.add(egui::Label::new(RichText::new(badge).small().color(p.accent)).wrap());
        }
    });
}

fn read_only(ui: &mut Ui, value: &str, width: f32) {
    let mut shown = value.to_owned();
    ui.add(
        TextEdit::singleline(&mut shown)
            .desired_width(width)
            .interactive(false)
            .hint_text("—"),
    );
}

/// A group of related settings on a raised surface. Rows inside are separated by hairlines
/// (see [`setting_row`]).
pub fn group<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(p.raised)
        .stroke(Stroke::new(1.0_f32, p.rule))
        .corner_radius(CornerRadius::same(theme::radius::CARD))
        .inner_margin(Margin::symmetric(
            theme::space::LG as i8,
            theme::space::SM as i8,
        ))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A small heading above a [`group`].
pub fn group_title(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(theme::space::MD);
    ui.label(RichText::new(text).strong().size(16.0).color(p.ink));
    ui.add_space(theme::space::XS);
}

fn hairline(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.rule);
}

/// A label with an optional explanation on the left and a control on the right.
/// Pass `first = true` for the first row of a group so no line is drawn above it.
pub fn setting_row(
    ui: &mut Ui,
    p: &Palette,
    first: bool,
    label: &str,
    hint: &str,
    control: impl FnOnce(&mut Ui),
) {
    if !first {
        hairline(ui, p);
    }
    ui.add_space(theme::space::SM);
    let total = ui.available_width();
    let left = (total * 0.58).max(160.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = theme::space::MD;
        ui.allocate_ui_with_layout(vec2(left, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(left);
            ui.add(egui::Label::new(RichText::new(label).strong()).wrap());
            if !hint.is_empty() {
                ui.add(egui::Label::new(RichText::new(hint).small().color(p.muted)).wrap());
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), control);
    });
    ui.add_space(theme::space::SM);
}

/// Like [`setting_row`] but the control gets the full width beneath the label, for text
/// fields and long option lists.
pub fn setting_stack(
    ui: &mut Ui,
    p: &Palette,
    first: bool,
    label: &str,
    hint: &str,
    control: impl FnOnce(&mut Ui),
) {
    if !first {
        hairline(ui, p);
    }
    ui.add_space(theme::space::SM);
    ui.label(RichText::new(label).strong());
    if !hint.is_empty() {
        ui.add(egui::Label::new(RichText::new(hint).small().color(p.muted)).wrap());
    }
    ui.add_space(theme::space::XS);
    control(ui);
    ui.add_space(theme::space::SM);
}

/// A switch. Returns true when it was flipped this frame.
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> bool {
    let size = vec2(38.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    let flipped = response.clicked();
    if flipped {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let track = if *on { p.accent } else { p.button_hover };
    let radius = rect.height() / 2.0;
    ui.painter().rect_filled(rect, radius, track);
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0_f32, if *on { p.accent } else { p.muted }),
        egui::StrokeKind::Inside,
    );
    let knob_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    let knob = if *on { p.raised } else { p.muted };
    ui.painter()
        .circle_filled(egui::pos2(knob_x, rect.center().y), radius - 4.0, knob);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    flipped
}

/// A row of mutually exclusive options drawn as one segmented control.
/// Returns true when the choice changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (option, label) in options {
            if ui.selectable_label(*value == *option, *label).clicked() && *value != *option {
                *value = *option;
                changed = true;
            }
        }
    });
    changed
}

/// The one main action of a screen: filled with the accent colour.
pub fn primary_button(ui: &mut Ui, p: &Palette, enabled: bool, label: &str) -> egui::Response {
    let fill = if enabled {
        p.accent
    } else {
        p.accent.gamma_multiply(0.35)
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).strong().color(p.paper))
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(theme::radius::CONTROL)),
    )
}

/// An entry in a vertical navigation list: highlighted when selected, with an accent bar.
pub fn nav_item(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), egui::Sense::click());
    let fill = if selected {
        p.accent.gamma_multiply(0.22)
    } else if response.hovered() {
        p.button_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::radius::CONTROL), fill);
    if selected {
        let bar = egui::Rect::from_min_size(
            rect.left_top() + vec2(0.0, 7.0),
            vec2(3.0, rect.height() - 14.0),
        );
        ui.painter().rect_filled(bar, 2.0, p.accent);
    }
    let text = RichText::new(label).color(p.ink);
    let text = if selected { text.strong() } else { text };
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        rect.width() - 24.0,
        egui::TextStyle::Body,
    );
    let pos = egui::pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, p.ink);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}
