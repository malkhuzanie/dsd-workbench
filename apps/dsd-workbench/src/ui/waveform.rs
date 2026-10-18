//! The waveform editor: an overview of a source file with draggable cut markers.
//!
//! Interactions: drag a marker to move a cut (hold Ctrl to snap to a silent gap),
//! double-click empty space to add a cut, right-click a marker to remove it, scroll to
//! zoom around the pointer, drag empty space to pan.

use eframe::egui::{self, Align2, Color32, FontId, Id, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::theme::Palette;
use crate::analysis::{Envelope, Silence, snap_to_silence};
use crate::timecode;

/// How close (in pixels) the pointer must be to a marker to grab it.
const GRAB_PX: f32 = 7.0;
/// The most the view can be zoomed in.
const MIN_VISIBLE_SECONDS: f64 = 0.5;
/// Zoom per scroll point.
const ZOOM_SENSITIVITY: f64 = 0.004;
/// Room kept at the bottom of the widget for the time ruler.
const RULER_H: f32 = 18.0;
/// Peaks smaller than this are not magnified, so quiet noise does not fill the view.
const MIN_FULL_SCALE: f32 = 0.05;
/// Candidate spacing for ruler labels, in seconds.
const TICKS: [f64; 14] = [
    0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0, 7200.0,
];
const MIN_TICK_PX: f32 = 90.0;

/// The part of the file on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub start: f64,
    pub end: f64,
}

impl Window {
    pub fn whole(total: f64) -> Self {
        Self {
            start: 0.0,
            end: total.max(MIN_VISIBLE_SECONDS),
        }
    }

    pub fn span(&self) -> f64 {
        self.end - self.start
    }

    pub fn time_to_x(&self, time: f64, left: f32, width: f32) -> f32 {
        left + ((time - self.start) / self.span().max(f64::EPSILON)) as f32 * width
    }

    pub fn x_to_time(&self, x: f32, left: f32, width: f32) -> f64 {
        self.start + f64::from((x - left) / width.max(1.0)) * self.span()
    }

    /// Keeps the window inside `0..total` without changing its length when possible.
    pub fn clamp_to(&mut self, total: f64) {
        let total = total.max(MIN_VISIBLE_SECONDS);
        let span = self.span().clamp(MIN_VISIBLE_SECONDS, total);
        self.start = self.start.clamp(0.0, total - span);
        self.end = self.start + span;
    }

    /// Zooms by `factor` (below 1 zooms in) keeping `anchor` at the same place on screen.
    pub fn zoom(&mut self, factor: f64, anchor: f64, total: f64) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let span = self.span().max(f64::EPSILON);
        let relative = ((anchor - self.start) / span).clamp(0.0, 1.0);
        let new_span = (span * factor).clamp(MIN_VISIBLE_SECONDS, total.max(MIN_VISIBLE_SECONDS));
        self.start = anchor - relative * new_span;
        self.end = self.start + new_span;
        self.clamp_to(total);
    }

    pub fn pan(&mut self, delta: f64, total: f64) {
        if !delta.is_finite() {
            return;
        }
        self.start += delta;
        self.end += delta;
        self.clamp_to(total);
    }
}

/// A cut point drawn on the waveform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    /// Index of the track in the plan.
    pub track: usize,
    pub number: u32,
    pub time: f64,
    /// The cut lands in loud audio.
    pub loud: bool,
    /// The first track of a file always starts at zero.
    pub movable: bool,
}

/// What the user did to the cuts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit {
    Move { track: usize, time: f64 },
    Insert { time: f64 },
    Remove { track: usize },
}

/// Index into `markers` of the movable marker nearest `pointer_x`, within `radius` pixels.
fn nearest_marker(
    markers: &[Marker],
    x_of: impl Fn(f64) -> f32,
    pointer_x: f32,
    radius: f32,
) -> Option<usize> {
    markers
        .iter()
        .enumerate()
        .filter(|(_, m)| m.movable)
        .map(|(i, m)| (i, (x_of(m.time) - pointer_x).abs()))
        .filter(|(_, distance)| *distance <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The first label spacing (from [`TICKS`]) that leaves at least [`MIN_TICK_PX`] between labels.
fn tick_spacing(seconds_per_pixel: f64) -> f64 {
    TICKS
        .iter()
        .copied()
        .find(|tick| tick / seconds_per_pixel.max(f64::EPSILON) >= f64::from(MIN_TICK_PX))
        .unwrap_or(TICKS[TICKS.len() - 1])
}

/// Everything the widget draws.
pub struct Scene<'a> {
    pub envelope: &'a Envelope,
    pub silences: &'a [Silence],
    pub markers: &'a [Marker],
    /// How far Ctrl-drag may snap to a silence, in seconds.
    pub snap_radius: f64,
}

/// Draws the waveform and reports what the user changed.
pub fn show(
    ui: &mut Ui,
    p: &Palette,
    id: Id,
    window: &mut Window,
    scene: &Scene<'_>,
    height: f32,
) -> Option<Edit> {
    let total = scene.envelope.duration();
    let width = ui.available_width().max(120.0);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, height.max(RULER_H * 3.0)),
        Sense::click_and_drag(),
    );
    if total <= 0.0 {
        ui.painter().rect_filled(rect, 4.0, p.field);
        return None;
    }
    window.clamp_to(total);

    let wave = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - RULER_H));
    let x_of = |time: f64| window.time_to_x(time, rect.left(), rect.width());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, p.field);

    draw_silences(&painter, p, scene.silences, &x_of, wave);
    draw_wave(&painter, p, scene.envelope, window, wave);
    draw_ruler(&painter, p, window, rect);
    draw_markers(&painter, p, scene.markers, &x_of, wave);

    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    interact(ui, id, &response, rect, window, scene, pointer, total)
}

fn draw_silences(
    painter: &egui::Painter,
    p: &Palette,
    silences: &[Silence],
    x_of: &impl Fn(f64) -> f32,
    wave: Rect,
) {
    let tint = p.accent.gamma_multiply(0.18);
    for silence in silences {
        let (a, b) = (x_of(silence.start), x_of(silence.end));
        if b < wave.left() || a > wave.right() {
            continue;
        }
        let band = Rect::from_min_max(
            pos2(a.max(wave.left()), wave.top()),
            pos2(b.min(wave.right()), wave.bottom()),
        );
        painter.rect_filled(band, 0.0, tint);
    }
}

fn draw_wave(
    painter: &egui::Painter,
    p: &Palette,
    envelope: &Envelope,
    window: &Window,
    wave: Rect,
) {
    let columns = wave.width().max(1.0) as usize;
    let hops = envelope.summarize(window.start, window.end, columns);
    let peak = hops
        .iter()
        .map(|h| h.max.max(-h.min))
        .fold(MIN_FULL_SCALE, f32::max);
    let (middle, half) = (wave.center().y, wave.height() / 2.0 - 2.0);
    let outer = p.muted;
    let inner = p.accent;

    for (i, hop) in hops.iter().enumerate() {
        let x = wave.left() + i as f32 + 0.5;
        let top = middle - (hop.max / peak).clamp(0.0, 1.0) * half;
        let bottom = middle - (hop.min / peak).clamp(-1.0, 0.0) * half;
        painter.line_segment(
            [pos2(x, top), pos2(x, bottom.max(top + 1.0))],
            Stroke::new(1.0_f32, outer),
        );
        let rms = (hop.rms / peak).clamp(0.0, 1.0) * half;
        if rms >= 0.5 {
            painter.line_segment(
                [pos2(x, middle - rms), pos2(x, middle + rms)],
                Stroke::new(1.0_f32, inner),
            );
        }
    }
    painter.line_segment(
        [pos2(wave.left(), middle), pos2(wave.right(), middle)],
        Stroke::new(0.5_f32, p.rule),
    );
}

fn draw_ruler(painter: &egui::Painter, p: &Palette, window: &Window, rect: Rect) {
    let tick = tick_spacing(window.span() / f64::from(rect.width().max(1.0)));
    let y = rect.bottom() - RULER_H;
    painter.line_segment(
        [pos2(rect.left(), y), pos2(rect.right(), y)],
        Stroke::new(1.0_f32, p.rule),
    );
    let mut time = (window.start / tick).ceil() * tick;
    while time <= window.end {
        let x = window.time_to_x(time, rect.left(), rect.width());
        painter.line_segment([pos2(x, y), pos2(x, y + 4.0)], Stroke::new(1.0_f32, p.rule));
        painter.text(
            pos2(x + 3.0, y + 5.0),
            Align2::LEFT_TOP,
            timecode::format(time),
            FontId::proportional(10.0),
            p.muted,
        );
        time += tick;
    }
}

fn draw_markers(
    painter: &egui::Painter,
    p: &Palette,
    markers: &[Marker],
    x_of: &impl Fn(f64) -> f32,
    wave: Rect,
) {
    for marker in markers {
        let x = x_of(marker.time);
        if x < wave.left() - 1.0 || x > wave.right() + 1.0 {
            continue;
        }
        let colour: Color32 = if marker.loud { p.danger } else { p.good };
        let width: f32 = if marker.movable { 2.0 } else { 1.0 };
        painter.line_segment(
            [pos2(x, wave.top()), pos2(x, wave.bottom())],
            Stroke::new(width, colour),
        );
        let label = if marker.movable && marker.loud {
            format!("{} ⚠", marker.number)
        } else {
            marker.number.to_string()
        };
        painter.text(
            pos2(x + 3.0, wave.top() + 2.0),
            Align2::LEFT_TOP,
            label,
            FontId::proportional(12.0),
            colour,
        );
    }
}

/// Reads the pointer and keyboard; returns the edit to apply, if any.
#[allow(clippy::too_many_arguments)]
fn interact(
    ui: &Ui,
    id: Id,
    response: &egui::Response,
    rect: Rect,
    window: &mut Window,
    scene: &Scene<'_>,
    pointer: Option<egui::Pos2>,
    total: f64,
) -> Option<Edit> {
    // Positions are judged against the view as it was at the start of the frame.
    let view = *window;
    let x_of = move |time: f64| view.time_to_x(time, rect.left(), rect.width());
    let grab_id = id.with("grabbed");
    let menu_id = id.with("menu_target");

    if let Some(pos) = pointer {
        if nearest_marker(scene.markers, x_of, pos.x, GRAB_PX).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            if scroll.y != 0.0 {
                let anchor = window.x_to_time(pos.x, rect.left(), rect.width());
                window.zoom(
                    (-f64::from(scroll.y) * ZOOM_SENSITIVITY).exp(),
                    anchor,
                    total,
                );
            }
            if scroll.x != 0.0 {
                let per_pixel = window.span() / f64::from(rect.width().max(1.0));
                window.pan(-f64::from(scroll.x) * per_pixel, total);
            }
        }
    }

    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let grabbed = nearest_marker(scene.markers, x_of, pos.x, GRAB_PX)
            .and_then(|i| scene.markers.get(i))
            .map(|m| m.track);
        ui.data_mut(|d| d.insert_temp(grab_id, grabbed));
    }
    let mut edit = None;
    if response.dragged() {
        let grabbed: Option<usize> = ui.data(|d| d.get_temp::<Option<usize>>(grab_id)).flatten();
        match (grabbed, response.interact_pointer_pos()) {
            (Some(track), Some(pos)) => {
                let mut time = window.x_to_time(pos.x, rect.left(), rect.width());
                if ui.input(|i| i.modifiers.command || i.modifiers.ctrl) {
                    time = snap_to_silence(time, scene.silences, scene.snap_radius).unwrap_or(time);
                }
                edit = Some(Edit::Move { track, time });
            }
            (None, _) => {
                let per_pixel = window.span() / f64::from(rect.width().max(1.0));
                window.pan(-f64::from(response.drag_delta().x) * per_pixel, total);
            }
            _ => {}
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<Option<usize>>(grab_id));
    }

    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && nearest_marker(scene.markers, x_of, pos.x, GRAB_PX).is_none()
    {
        let time = window.x_to_time(pos.x, rect.left(), rect.width());
        edit = Some(Edit::Insert { time });
    }

    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let target = nearest_marker(scene.markers, x_of, pos.x, GRAB_PX)
            .and_then(|i| scene.markers.get(i))
            .map(|m| m.track);
        ui.data_mut(|d| d.insert_temp(menu_id, target));
    }
    let target: Option<usize> = ui.data(|d| d.get_temp::<Option<usize>>(menu_id)).flatten();
    response.context_menu(|ui| match target {
        Some(track) => {
            if ui.button("Remove this cut").clicked() {
                edit = Some(Edit::Remove { track });
                ui.close_menu();
            }
        }
        None => {
            ui.label("Double-click to add a cut");
        }
    });
    edit
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1000.0;

    fn marker(track: usize, time: f64, movable: bool) -> Marker {
        Marker {
            track,
            number: track as u32 + 1,
            time,
            loud: false,
            movable,
        }
    }

    #[test]
    fn time_and_x_are_inverse() {
        let window = Window {
            start: 10.0,
            end: 70.0,
        };
        for time in [10.0, 25.5, 70.0] {
            let x = window.time_to_x(time, 50.0, W);
            assert!((window.x_to_time(x, 50.0, W) - time).abs() < 1e-3);
        }
        assert_eq!(window.time_to_x(10.0, 50.0, W), 50.0);
        assert_eq!(window.time_to_x(70.0, 50.0, W), 1050.0);
    }

    #[test]
    fn zooming_keeps_the_anchor_under_the_pointer() {
        let mut window = Window::whole(600.0);
        let anchor = 150.0;
        let before = window.time_to_x(anchor, 0.0, W);
        window.zoom(0.5, anchor, 600.0);
        assert!((window.span() - 300.0).abs() < 1e-9);
        assert!((window.time_to_x(anchor, 0.0, W) - before).abs() < 0.5);
    }

    #[test]
    fn zoom_is_limited_and_ignores_nonsense() {
        let mut window = Window::whole(600.0);
        window.zoom(1e-9, 300.0, 600.0);
        assert!(
            (window.span() - MIN_VISIBLE_SECONDS).abs() < 1e-9,
            "cannot zoom past the limit"
        );
        window.zoom(1e9, 300.0, 600.0);
        assert_eq!(
            window,
            Window::whole(600.0),
            "cannot zoom out past the whole file"
        );
        let copy = window;
        window.zoom(f64::NAN, 1.0, 600.0);
        window.zoom(-1.0, 1.0, 600.0);
        window.pan(f64::INFINITY, 600.0);
        assert_eq!(window, copy);
    }

    #[test]
    fn panning_stops_at_the_ends() {
        let mut window = Window {
            start: 100.0,
            end: 200.0,
        };
        window.pan(-500.0, 600.0);
        assert_eq!((window.start, window.end), (0.0, 100.0));
        window.pan(5000.0, 600.0);
        assert_eq!((window.start, window.end), (500.0, 600.0));
    }

    #[test]
    fn a_window_survives_a_shorter_file() {
        let mut window = Window {
            start: 400.0,
            end: 500.0,
        };
        window.clamp_to(120.0);
        assert!(window.start >= 0.0 && window.end <= 120.0 + 1e-9);
        let mut tiny = Window::whole(0.0);
        tiny.clamp_to(0.0);
        assert!(tiny.span() >= MIN_VISIBLE_SECONDS - 1e-9);
    }

    #[test]
    fn markers_are_grabbed_by_proximity_and_only_when_movable() {
        let markers = [
            marker(0, 0.0, false),
            marker(1, 100.0, true),
            marker(2, 104.0, true),
        ];
        let window = Window {
            start: 0.0,
            end: 1000.0,
        };
        let x_of = |t: f64| window.time_to_x(t, 0.0, W);
        assert_eq!(
            nearest_marker(&markers, x_of, 0.0, GRAB_PX),
            None,
            "the first track is fixed"
        );
        // 100 s is at x = 100, 104 s at x = 104.
        assert_eq!(nearest_marker(&markers, x_of, 101.0, GRAB_PX), Some(1));
        assert_eq!(
            nearest_marker(&markers, x_of, 103.5, GRAB_PX),
            Some(2),
            "the nearer of two"
        );
        assert_eq!(nearest_marker(&markers, x_of, 300.0, GRAB_PX), None);
        assert_eq!(nearest_marker(&[], x_of, 1.0, GRAB_PX), None);
    }

    #[test]
    fn ruler_labels_stay_readable_at_every_zoom() {
        for seconds_per_pixel in [0.001, 0.01, 0.1, 1.0, 10.0, 100.0] {
            let tick = tick_spacing(seconds_per_pixel);
            assert!(tick / seconds_per_pixel >= f64::from(MIN_TICK_PX) - 1e-6 || tick == 7200.0);
        }
        assert_eq!(
            tick_spacing(0.0),
            TICKS[0],
            "fully zoomed in uses the finest tick"
        );
        assert_eq!(
            tick_spacing(f64::INFINITY),
            7200.0,
            "fully zoomed out uses the coarsest"
        );
        assert_eq!(tick_spacing(0.5), 60.0);
    }
}
