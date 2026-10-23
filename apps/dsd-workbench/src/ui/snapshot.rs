//! A tiny software renderer for looking at the UI without a window: runs egui for one frame,
//! tessellates, and rasterises the triangles into a PPM file. Used by `#[ignore]`d tests that
//! write pictures to the directory named by `UI_SNAPSHOT_DIR`; they do not assert anything.
//!
//! `UI_SNAPSHOT_DIR=/tmp/ui cargo test -p dsd-workbench snapshot -- --ignored`

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, Color32, epaint};

use super::theme::{self, Palette};
use super::{fonts, welcome};

/// Renders `frame` into an RGBA image of `size` points. Runs a few warm-up frames first so
/// layout that depends on the previous frame settles.
pub(super) fn render(
    size: egui::Vec2,
    dark: bool,
    time: f64,
    mut frame: impl FnMut(&egui::Context),
) -> (usize, usize, Vec<Color32>) {
    let ctx = egui::Context::default();
    fonts::install_fallbacks(&ctx);
    theme::install(&ctx);
    theme::select(&ctx, dark);

    let mut textures: HashMap<egui::TextureId, (usize, usize, Vec<Color32>)> = HashMap::new();
    let mut last = None;
    for pass in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(time + f64::from(pass) * 0.016),
            ..Default::default()
        };
        let output = ctx.run(input, &mut frame);
        for (id, delta) in &output.textures_delta.set {
            apply(&mut textures, *id, delta);
        }
        last = Some((
            ctx.tessellate(output.shapes, output.pixels_per_point),
            output.pixels_per_point,
        ));
    }
    let (primitives, ppp) = last.expect("at least one frame ran");

    let (width, height) = (
        (size.x * ppp).round() as usize,
        (size.y * ppp).round() as usize,
    );
    let background = ctx.style().visuals.panel_fill;
    let mut pixels = vec![background; width * height];
    for primitive in primitives {
        let epaint::Primitive::Mesh(mesh) = primitive.primitive else {
            continue;
        };
        let Some(texture) = textures.get(&mesh.texture_id) else {
            continue;
        };
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let vertices = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            fill(
                &mut pixels,
                (width, height),
                ppp,
                &primitive.clip_rect,
                texture,
                &vertices,
            );
        }
    }
    (width, height, pixels)
}

fn apply(
    textures: &mut HashMap<egui::TextureId, (usize, usize, Vec<Color32>)>,
    id: egui::TextureId,
    delta: &epaint::ImageDelta,
) {
    let [w, h] = delta.image.size();
    let pixels: Vec<Color32> = match &delta.image {
        epaint::ImageData::Color(image) => image.pixels.clone(),
        epaint::ImageData::Font(image) => image.srgba_pixels(None).collect(),
    };
    match delta.pos {
        None => {
            textures.insert(id, (w, h, pixels));
        }
        Some([x0, y0]) => {
            if let Some((tw, _, target)) = textures.get_mut(&id) {
                for row in 0..h {
                    for col in 0..w {
                        target[(y0 + row) * *tw + x0 + col] = pixels[row * w + col];
                    }
                }
            }
        }
    }
}

fn fill(
    pixels: &mut [Color32],
    (width, height): (usize, usize),
    ppp: f32,
    clip: &egui::Rect,
    (tw, th, texels): &(usize, usize, Vec<Color32>),
    v: &[epaint::Vertex; 3],
) {
    let p: Vec<egui::Pos2> = v.iter().map(|v| v.pos * ppp).collect();
    let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[2].x - p[0].x) * (p[1].y - p[0].y);
    if area.abs() < 1e-6 {
        return;
    }
    let (min_x, max_x) = (
        p.iter()
            .map(|q| q.x)
            .fold(f32::MAX, f32::min)
            .floor()
            .max(clip.min.x * ppp),
        p.iter()
            .map(|q| q.x)
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(clip.max.x * ppp),
    );
    let (min_y, max_y) = (
        p.iter()
            .map(|q| q.y)
            .fold(f32::MAX, f32::min)
            .floor()
            .max(clip.min.y * ppp),
        p.iter()
            .map(|q| q.y)
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(clip.max.y * ppp),
    );
    for y in (min_y.max(0.0) as usize)..(max_y.min(height as f32) as usize) {
        for x in (min_x.max(0.0) as usize)..(max_x.min(width as f32) as usize) {
            let c = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1].x - c.x) * (p[2].y - c.y) - (p[2].x - c.x) * (p[1].y - c.y)) / area;
            let w1 = ((p[2].x - c.x) * (p[0].y - c.y) - (p[0].x - c.x) * (p[2].y - c.y)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 {
                continue;
            }
            let uv = egui::pos2(
                w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x,
                w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y,
            );
            let tx = ((uv.x * *tw as f32) as usize).min(tw - 1);
            let ty = ((uv.y * *th as f32) as usize).min(th - 1);
            let texel = texels[ty * tw + tx];
            let channel = |a: [u8; 3], i: usize| {
                w0 * f32::from(a[0]) * f32::from(v[0].color.to_array()[i]) / 255.0
                    + w1 * f32::from(a[1]) * f32::from(v[1].color.to_array()[i]) / 255.0
                    + w2 * f32::from(a[2]) * f32::from(v[2].color.to_array()[i]) / 255.0
            };
            let _ = channel;
            // Vertex colours are interpolated, then multiplied by the texel (both premultiplied).
            let mut mix = [0.0_f32; 4];
            for (i, slot) in mix.iter_mut().enumerate() {
                let vertex = w0 * f32::from(v[0].color.to_array()[i])
                    + w1 * f32::from(v[1].color.to_array()[i])
                    + w2 * f32::from(v[2].color.to_array()[i]);
                *slot = vertex * f32::from(texel.to_array()[i]) / 255.0;
            }
            let dst = pixels[y * width + x].to_array();
            let inverse = 1.0 - mix[3] / 255.0;
            let out = |i: usize| (mix[i] + f32::from(dst[i]) * inverse).clamp(0.0, 255.0) as u8;
            pixels[y * width + x] = Color32::from_rgba_premultiplied(out(0), out(1), out(2), 255);
        }
    }
}

/// Writes `pixels` as a binary PPM; ImageMagick (`convert`) turns it into a PNG.
pub(super) fn write_ppm(path: &std::path::Path, width: usize, height: usize, pixels: &[Color32]) {
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in pixels {
        bytes.extend_from_slice(&pixel.to_array()[..3]);
    }
    std::fs::write(path, bytes).expect("write the snapshot");
}

pub(super) fn output_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("UI_SNAPSHOT_DIR")?);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Renders the welcome screen at several moments of its animation, at two window sizes.
#[test]
#[ignore = "writes pictures; run on purpose with UI_SNAPSHOT_DIR set"]
fn snapshot_welcome() {
    let Some(dir) = output_dir() else { return };
    for (name, size) in [
        ("wide", egui::vec2(1280.0, 800.0)),
        ("narrow", egui::vec2(640.0, 800.0)),
    ] {
        for time in [1.0_f64, 3.0, 6.0, 9.0, 13.2, 16.0] {
            let p = Palette::of(false);
            let (w, h, pixels) = render(size, false, time, |ctx| {
                egui::CentralPanel::default()
                    .frame(theme::gutter(egui::Margin::same(12)))
                    .show(ctx, |ui| {
                        theme::card(&p).show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            welcome::show(ui, &p);
                        });
                    });
            });
            write_ppm(
                &dir.join(format!("welcome-{name}-{time}.ppm")),
                w,
                h,
                &pixels,
            );
        }
    }
}

/// Renders the album pane (Tracks and Album & output) for a generated album, with the
/// accordion on and off, so the layout can be checked by eye.
#[test]
#[ignore = "writes pictures; run on purpose with UI_SNAPSHOT_DIR set"]
fn snapshot_album() {
    use super::detail::{self, Context};
    use super::{DetailTab, listen::ListenState, verify_view, wave_view};
    use crate::audio::Player;
    use crate::metadata::FieldKey;
    use crate::plan::PlanOptions;
    use crate::project::Project;
    use crate::selection::TrackSelection;
    use crate::settings::LookupSettings;
    use crate::snapshot::TagCache;
    use crate::source::testing::stereo_dsf;

    let Some(out) = output_dir() else { return };
    let dir = tempfile::tempdir().expect("temp dir");
    let audio = vec![0x5A; 4096 * 2 * 4];
    std::fs::write(
        dir.path().join("side.dsf"),
        stereo_dsf(32_768, 4 * 32_768, &audio),
    )
    .expect("write the source");
    std::fs::write(
        dir.path().join("album.cue"),
        "PERFORMER \"Bob Dylan\"\nTITLE \"Blood On The Tracks\"\nFILE \"side.dsf\" WAVE\n\
         TRACK 01 AUDIO\n TITLE \"Tangled Up In Blue\"\n INDEX 01 00:00:00\n\
         TRACK 02 AUDIO\n TITLE \"Simple Twist Of Fate\"\n INDEX 01 00:01:00\n\
         TRACK 03 AUDIO\n TITLE \"You're A Big Girl Now\"\n INDEX 01 00:02:00\n",
    )
    .expect("write the cue");
    let mut project = Project::default();
    project.add(&dir.path().join("album.cue")).expect("add");
    let entry = project.entries.first_mut().expect("an entry");
    entry.plan.album.fields.set(FieldKey::GENRE, "Folk Rock");
    entry.plan.album.fields.set(FieldKey::DATE, "1975");

    let p = Palette::of(false);
    for (name, tab, collapsible) in [
        ("album-accordion", DetailTab::Album, true),
        ("album-plain", DetailTab::Album, false),
        ("tracks", DetailTab::Tracks, true),
    ] {
        let mut cache = TagCache::default();
        let mut tab = tab;
        let mut player = Player::default();
        let mut listen = ListenState::default();
        let mut verify = verify_view::ViewState::default();
        let mut selection = TrackSelection::default();
        let mut wave = wave_view::WaveState::default();
        let mut lookup = LookupSettings::default();
        let options = PlanOptions::default();
        let (w, h, pixels) = render(egui::vec2(1180.0, 820.0), false, 2.0, |ctx| {
            egui::CentralPanel::default()
                .frame(theme::gutter(egui::Margin::same(12)))
                .show(ctx, |ui| {
                    theme::card(&p).show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        let mut cx = Context {
                            p: &p,
                            cache: &mut cache,
                            tab: &mut tab,
                            overwrite: false,
                            problems: &[],
                            player: &mut player,
                            listen: &mut listen,
                            verify: &mut verify,
                            selection: &mut selection,
                            wave: &mut wave,
                            lookup: &mut lookup,
                            busy: false,
                            collapsible,
                            options: &options,
                        };
                        detail::show(ui, &mut cx, &mut project.entries[0]);
                    });
                });
        });
        write_ppm(&out.join(format!("{name}.ppm")), w, h, &pixels);
    }
}
