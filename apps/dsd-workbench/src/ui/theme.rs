//! Paper-style light and dark palettes.

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, Shadow, Stroke};

/// Spacing scale (a 4 px grid) shared by every screen.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Corner radii: small for controls, large for cards.
pub mod radius {
    pub const CONTROL: u8 = 6;
    pub const CARD: u8 = 10;
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Background behind the page.
    pub desk: Color32,
    /// The "sheet of paper" itself.
    pub paper: Color32,
    /// Surface of cards sitting on the paper: one step above it.
    pub raised: Color32,
    /// Text input background.
    pub field: Color32,
    pub button: Color32,
    pub button_hover: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub rule: Color32,
    pub accent: Color32,
    pub added: Color32,
    pub modified: Color32,
    pub removed: Color32,
    pub danger: Color32,
    pub good: Color32,
    pub bad: Color32,
}

impl Palette {
    pub const LIGHT: Self = Self {
        desk: Color32::from_rgb(0xE9, 0xE1, 0xCB),
        paper: Color32::from_rgb(0xFA, 0xF6, 0xEA),
        raised: Color32::from_rgb(0xF5, 0xEF, 0xDD),
        field: Color32::from_rgb(0xFF, 0xFD, 0xF6),
        button: Color32::from_rgb(0xEF, 0xE7, 0xD2),
        button_hover: Color32::from_rgb(0xE6, 0xDC, 0xC2),
        ink: Color32::from_rgb(0x2F, 0x2A, 0x22),
        muted: Color32::from_rgb(0x66, 0x5D, 0x47),
        rule: Color32::from_rgb(0xD8, 0xCD, 0xB2),
        accent: Color32::from_rgb(0x8A, 0x5A, 0x2B),
        added: Color32::from_rgb(0xDD, 0xEB, 0xC8),
        modified: Color32::from_rgb(0xF6, 0xE3, 0xB0),
        removed: Color32::from_rgb(0xF2, 0xCF, 0xC3),
        danger: Color32::from_rgb(0xA2, 0x3B, 0x2A),
        good: Color32::from_rgb(0x5B, 0x8C, 0x3E),
        bad: Color32::from_rgb(0xB5, 0x3A, 0x2B),
    };

    pub const DARK: Self = Self {
        desk: Color32::from_rgb(0x15, 0x14, 0x12),
        paper: Color32::from_rgb(0x20, 0x1E, 0x1A),
        raised: Color32::from_rgb(0x28, 0x25, 0x20),
        field: Color32::from_rgb(0x2A, 0x27, 0x22),
        button: Color32::from_rgb(0x2E, 0x2A, 0x24),
        button_hover: Color32::from_rgb(0x3A, 0x35, 0x2D),
        ink: Color32::from_rgb(0xE3, 0xDA, 0xC8),
        muted: Color32::from_rgb(0x9E, 0x95, 0x80),
        rule: Color32::from_rgb(0x39, 0x35, 0x2D),
        accent: Color32::from_rgb(0xD9, 0xA7, 0x60),
        added: Color32::from_rgb(0x2C, 0x3B, 0x24),
        modified: Color32::from_rgb(0x4A, 0x3D, 0x1C),
        removed: Color32::from_rgb(0x4A, 0x2A, 0x24),
        danger: Color32::from_rgb(0xE5, 0x8A, 0x78),
        good: Color32::from_rgb(0x7F, 0xB0, 0x5E),
        bad: Color32::from_rgb(0xE0, 0x6C, 0x58),
    };

    pub const fn of(dark: bool) -> Self {
        if dark { Self::DARK } else { Self::LIGHT }
    }
}

/// Registers both themes once at startup.
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Light, visuals(Palette::LIGHT, false));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(Palette::DARK, true));

    // Roomier type scale and spacing than egui's compact defaults
    // (Body 12.5 / Small 9 are easy to outgrow in a form-heavy window).
    ctx.all_styles_mut(|style| {
        use egui::{FontId, TextStyle};
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(24.0));
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.5));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(13.5));
        // style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 28.0;
    });
}

/// Switches the active theme.
pub fn select(ctx: &egui::Context, dark: bool) {
    ctx.set_theme(if dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
}

fn visuals(p: Palette, dark: bool) -> egui::Visuals {
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.desk;
    v.window_fill = p.paper;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.paper;
    v.hyperlink_color = p.accent;
    v.window_stroke = Stroke::new(1.0_f32, p.rule);
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.paper;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.rule);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.ink);
    for (state, fill) in [
        (&mut w.inactive, p.button),
        (&mut w.hovered, p.button_hover),
        (&mut w.active, p.button_hover),
        (&mut w.open, p.button),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke.color = p.rule;
        state.fg_stroke.color = p.ink;
    }
    // Text boxes take their outline from the idle state: without it a box on a card
    // (nearly the same colour) is invisible until it is clicked.
    w.inactive.bg_stroke.width = 1.0;
    w.hovered.bg_stroke.color = p.accent;
    v
}

/// A raised card: paper fill, hairline border, rounded corners, soft shadow.
pub fn card(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.paper)
        .stroke(Stroke::new(1.0_f32, p.rule))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(16))
        .shadow(Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(26),
        })
}

/// Transparent panel frame; the margin becomes the gutter between cards.
pub fn gutter(margin: Margin) -> Frame {
    Frame::new().inner_margin(margin)
}

/// WCAG 2.x relative luminance of an sRGB colour.
#[cfg(test)]
fn luminance(c: Color32) -> f64 {
    let channel = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.039_28 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// WCAG contrast ratio between two colours, `1.0..=21.0`.
#[cfg(test)]
fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_math_matches_known_values() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-9);
    }

    /// Body text must reach 4.5:1 (WCAG AA); secondary text and accents 3:1 at least.
    #[test]
    fn both_themes_meet_wcag_contrast() {
        for (name, p) in [("light", Palette::LIGHT), ("dark", Palette::DARK)] {
            for (what, fg, bg, minimum) in [
                ("ink on paper", p.ink, p.paper, 4.5),
                ("primary button text", p.paper, p.accent, 4.5),
                ("ink on raised cards", p.ink, p.raised, 4.5),
                ("muted text on raised cards", p.muted, p.raised, 4.5),
                ("accent on raised cards", p.accent, p.raised, 3.0),
                ("ink on fields", p.ink, p.field, 4.5),
                ("ink on buttons", p.ink, p.button, 4.5),
                ("ink on modified rows", p.ink, p.modified, 4.5),
                ("ink on added rows", p.ink, p.added, 4.5),
                ("ink on removed rows", p.ink, p.removed, 4.5),
                ("muted text on paper", p.muted, p.paper, 4.5),
                ("muted text on fields", p.muted, p.field, 4.5),
                ("muted text on buttons", p.muted, p.button, 4.5),
                ("muted text on the desk", p.muted, p.desk, 4.5),
                ("accent on paper", p.accent, p.paper, 3.0),
                ("danger on paper", p.danger, p.paper, 3.0),
                ("good on paper", p.good, p.paper, 3.0),
            ] {
                let ratio = contrast(fg, bg);
                assert!(
                    ratio >= minimum,
                    "{name}: {what} is {ratio:.2}:1, needs {minimum}:1"
                );
            }
        }
    }
}
