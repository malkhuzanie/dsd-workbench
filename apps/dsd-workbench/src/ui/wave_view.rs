//! The Waveform tab: analyse a source file, find its silent gaps, and place the cuts by eye.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Button, RichText, Ui};

use super::analysis_job::{AnalysisJob, Msg};
use super::theme::Palette;
use super::waveform::{self, Edit, Marker, Scene, Window};
use super::widgets::{self, Tone};
use crate::analysis::{CutPlacement, DetectParams, Envelope, Silence, cut_is_loud, find_silences};
use crate::project::Entry;
use crate::snapshot::SourceState;

/// Cuts closer than this to a neighbour are refused.
const MIN_CUT_GAP: f64 = 1.0;
/// A cut in audio louder than this is flagged.
const LOUD_CUT_DB: f32 = -40.0;
/// How far Ctrl-drag looks for a silence to snap to.
const SNAP_SECONDS: f64 = 3.0;
const WAVE_HEIGHT: f32 = 220.0;

enum Slot {
    Running { job: AnalysisJob, fraction: f32 },
    Ready(Arc<Envelope>),
    Failed(String),
}

/// What the tab remembers between frames.
#[derive(Default)]
pub struct WaveState {
    slots: HashMap<PathBuf, Slot>,
    /// The visible stretch; `None` until the file is analysed (then it shows everything).
    window: Option<Window>,
    /// The album and file the window belongs to; a change resets the zoom.
    showing: Option<(u64, String)>,
    params: DetectParams,
    /// Silences found with the current settings, and what they were found for.
    silences: Option<(PathBuf, [u32; 2], Vec<Silence>)>,
    message: Option<String>,
}

impl WaveState {
    /// Collects finished work from the background threads.
    fn poll(&mut self) {
        for slot in self.slots.values_mut() {
            let Slot::Running { job, fraction } = slot else {
                continue;
            };
            let (messages, _) = job.poll();
            let mut done = None;
            for message in messages {
                match message {
                    Msg::Progress(f) => *fraction = f,
                    Msg::Done(result) => done = Some(result),
                }
            }
            match done {
                Some(Ok(envelope)) => *slot = Slot::Ready(Arc::new(envelope)),
                Some(Err(err)) => *slot = Slot::Failed(err.to_string()),
                None => {}
            }
        }
    }

    /// Silences for `path` under the current detection settings, recomputed only when they change.
    fn silences_for(&mut self, path: &Path, envelope: &Envelope) -> &[Silence] {
        let key = [
            self.params.threshold_db.to_bits(),
            (self.params.min_silence as f32).to_bits(),
        ];
        let current = self
            .silences
            .as_ref()
            .is_some_and(|(p, k, _)| p == path && *k == key);
        if !current {
            let found = find_silences(envelope, self.params.threshold_db, self.params.min_silence);
            self.silences = Some((path.to_path_buf(), key, found));
        }
        self.silences.as_ref().map_or(&[], |(_, _, s)| s.as_slice())
    }
}

pub(super) fn show(
    ui: &mut Ui,
    p: &Palette,
    ctx: &egui::Context,
    entry: &mut Entry,
    state: &mut WaveState,
) {
    state.poll();
    let files = entry.plan.source_files();
    let Some(first) = files.first().cloned() else {
        return;
    };

    // Which file: the one last shown for this album, else the first.
    let mut file = state
        .showing
        .as_ref()
        .filter(|(album, name)| *album == entry.id && files.contains(name))
        .map_or(first, |(_, name)| name.clone());
    if files.len() > 1 {
        egui::ComboBox::from_label("Source file")
            .selected_text(file.clone())
            .show_ui(ui, |ui| {
                for name in &files {
                    ui.selectable_value(&mut file, name.clone(), name);
                }
            });
    }
    if state.showing.as_ref() != Some(&(entry.id, file.clone())) {
        state.showing = Some((entry.id, file.clone()));
        state.window = None;
        state.message = None;
    }

    let path = entry.base_dir.join(&file);
    let usable = matches!(
        entry.sources.get(&file).map(|s| &s.state),
        Some(SourceState::Ready { .. })
    );
    if !usable {
        widgets::banner(
            ui,
            p,
            Tone::Warn,
            "This source file is missing or unreadable.",
        );
        return;
    }

    match state.slots.get(&path) {
        None => {
            widgets::caption(
                ui,
                p,
                "Analyse the audio once to see its waveform and find the silent gaps between tracks. It reads the whole file, which takes a little while for a long side.",
            );
            if ui.button("Analyse audio").clicked() {
                state.slots.insert(
                    path.clone(),
                    Slot::Running {
                        job: AnalysisJob::spawn(ctx, path),
                        fraction: 0.0,
                    },
                );
            }
        }
        Some(Slot::Running { job, fraction }) => {
            ui.horizontal(|ui| {
                ui.add(egui::ProgressBar::new(*fraction).desired_width(260.0));
                if ui.button("Cancel").clicked() {
                    job.cancel();
                }
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        Some(Slot::Failed(why)) => {
            widgets::banner(ui, p, Tone::Bad, &format!("Analysis failed: {why}"));
            if ui.button("Try again").clicked() {
                state.slots.remove(&path);
            }
        }
        Some(Slot::Ready(envelope)) => {
            let envelope = Arc::clone(envelope);
            editor(ui, p, entry, state, &file, &path, &envelope);
        }
    }
}

fn editor(
    ui: &mut Ui,
    p: &Palette,
    entry: &mut Entry,
    state: &mut WaveState,
    file: &str,
    path: &Path,
    envelope: &Envelope,
) {
    let mut window = state
        .window
        .unwrap_or_else(|| Window::whole(envelope.duration()));
    detection_controls(ui, p, state);

    let silences = state.silences_for(path, envelope).to_vec();
    let existing: Vec<f64> = entry
        .plan
        .tracks_in(file)
        .iter()
        .filter_map(|i| entry.plan.tracks.get(*i))
        .map(|t| t.start_seconds)
        .collect();
    let suggested = state.params.cuts(envelope);
    let fresh: Vec<f64> = suggested
        .iter()
        .copied()
        .filter(|cut| {
            existing
                .iter()
                .all(|e| (e - cut).abs() >= state.params.min_track)
        })
        .collect();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} silent gap(s) · {} new cut(s) suggested",
                silences.len(),
                fresh.len()
            ))
            .color(p.muted),
        );
        if ui
            .add_enabled(!fresh.is_empty(), Button::new("Add cuts at the gaps"))
            .clicked()
        {
            let added = fresh
                .iter()
                .filter(|cut| entry.plan.insert_cut(file, **cut, MIN_CUT_GAP).is_some())
                .count();
            state.message = Some(format!("Added {added} cut(s)."));
        }
        if ui.button("Show whole file").clicked() {
            window = Window::whole(envelope.duration());
        }
    });

    let markers: Vec<Marker> = entry
        .plan
        .tracks_in(file)
        .iter()
        .enumerate()
        .filter_map(|(position, index)| {
            let track = entry.plan.tracks.get(*index)?;
            Some(Marker {
                track: *index,
                number: track.meta.number,
                time: track.start_seconds,
                loud: position > 0 && cut_is_loud(envelope, track.start_seconds, LOUD_CUT_DB),
                movable: position > 0,
            })
        })
        .collect();

    let scene = Scene {
        envelope,
        silences: &silences,
        markers: &markers,
        snap_radius: SNAP_SECONDS,
    };
    let id = egui::Id::new(("waveform", entry.id, file));
    let edit = waveform::show(ui, p, id, &mut window, &scene, WAVE_HEIGHT);
    state.window = Some(window);
    apply(entry, file, edit, state);

    if markers.iter().any(|m| m.loud) {
        widgets::banner(
            ui,
            p,
            Tone::Warn,
            "A cut marked ⚠ lands in loud audio and may be audible. Drag it into a quiet spot (hold Ctrl to snap to a gap).",
        );
    }
    if let Some(message) = &state.message {
        widgets::caption(ui, p, message);
    }
    widgets::caption(
        ui,
        p,
        "Drag a marker to move a cut · Ctrl-drag snaps to a silent gap · double-click to add a cut · right-click a marker to remove it · scroll to zoom · drag to pan.",
    );
}

fn detection_controls(ui: &mut Ui, p: &Palette, state: &mut WaveState) {
    let params = &mut state.params;
    ui.horizontal_wrapped(|ui| {
        ui.label("Silence below");
        ui.add(
            egui::DragValue::new(&mut params.threshold_db)
                .range(-100.0..=-20.0)
                .suffix(" dB"),
        );
        ui.label("for at least");
        ui.add(
            egui::DragValue::new(&mut params.min_silence)
                .range(0.2..=30.0)
                .speed(0.1)
                .suffix(" s"),
        );
        ui.label("Shortest track");
        ui.add(
            egui::DragValue::new(&mut params.min_track)
                .range(1.0..=600.0)
                .suffix(" s"),
        );
        ui.radio_value(
            &mut params.placement,
            CutPlacement::Middle,
            "Cut in the middle of the gap",
        );
        ui.radio_value(
            &mut params.placement,
            CutPlacement::BeforeMusic,
            "Cut just before the music",
        );
    });
    widgets::caption(
        ui,
        p,
        "Quiet passages that last long enough are treated as the gaps between tracks.",
    );
}

/// Applies one waveform edit to the plan and notes the outcome.
fn apply(entry: &mut Entry, file: &str, edit: Option<Edit>, state: &mut WaveState) {
    match edit {
        Some(Edit::Move { track, time }) => {
            entry.plan.move_cut(track, time, MIN_CUT_GAP);
        }
        Some(Edit::Insert { time }) => {
            state.message = Some(match entry.plan.insert_cut(file, time, MIN_CUT_GAP) {
                Some(_) => "Added a cut.".to_owned(),
                None => format!("A cut needs at least {MIN_CUT_GAP:.0} s on both sides."),
            });
        }
        Some(Edit::Remove { track }) => {
            state.message = Some(if entry.plan.remove_cut(track) {
                "Removed the cut; the track now continues into the next one.".to_owned()
            } else {
                "The first track of a file cannot be removed.".to_owned()
            });
        }
        None => {}
    }
}
