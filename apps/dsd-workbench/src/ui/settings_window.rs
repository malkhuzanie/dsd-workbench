//! The Settings dialog: a page list on the left, the chosen page's settings on the right.

use eframe::egui::{self, RichText, Ui};

use super::theme::{self, Palette, space};
use super::widgets;
use crate::audio::{Player, Quality, RateTarget};
use crate::diag;
use crate::metadata::{FieldKey, Standard};
use crate::naming::{NameStyle, Naming};
use crate::online;
use crate::plan::GapMode;
use crate::settings::AppSettings;

/// How `naming` would name track 3 of a sample album, for the live preview.
fn naming_example(naming: &Naming) -> String {
    let lookup = |key: &FieldKey| match key {
        FieldKey::Standard(Standard::Title) => "So What",
        FieldKey::Standard(Standard::Artist | Standard::AlbumArtist) => "Miles Davis",
        FieldKey::Standard(Standard::Album) => "Kind of Blue",
        FieldKey::Standard(Standard::Date) => "1959",
        _ => "",
    };
    let mut path: std::path::PathBuf = naming.folders(3, lookup).into_iter().collect();
    path.push(naming.file_name(3, lookup));
    path.display().to_string()
}

/// Preferred size of the dialog; shrunk to fit small windows.
const WIDTH: f32 = 780.0;
const HEIGHT: f32 = 540.0;
/// Width of the page list.
const NAV_WIDTH: f32 = 168.0;
/// Space taken by the title bar and the footer.
const CHROME_HEIGHT: f32 = 96.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Appearance,
    Splitting,
    Names,
    Cover,
    Audio,
    Online,
    About,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Splitting => "Splitting",
            Self::Names => "CUE & file names",
            Self::Cover => "Cover art",
            Self::Audio => "Audition audio",
            Self::Online => "Online lookup",
            Self::About => "About & privacy",
        }
    }

    /// The pages in display order; the online page exists only in online builds.
    fn all() -> Vec<Self> {
        let mut pages = vec![
            Self::Appearance,
            Self::Splitting,
            Self::Names,
            Self::Cover,
            Self::Audio,
        ];
        if online::ENABLED {
            pages.push(Self::Online);
        }
        pages.push(Self::About);
        pages
    }
}

/// Shows the window; returns true if any setting changed.
pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    open: &mut bool,
    settings: &mut AppSettings,
    player: &mut Player,
) -> bool {
    let mut changed = false;
    let screen = ctx.screen_rect().size();
    let size = egui::vec2(
        WIDTH.min(screen.x - 2.0 * space::XL),
        HEIGHT.min(screen.y * 0.9),
    );
    let body_height = (size.y - CHROME_HEIGHT).max(160.0);

    let page_id = egui::Id::new("settings_page");
    let mut page = ctx
        .data(|d| d.get_temp::<usize>(page_id))
        .unwrap_or_default();
    let pages = Page::all();
    page = page.min(pages.len() - 1);

    egui::Window::new("Settings")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .fixed_size(size)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(space::MD, space::SM);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(NAV_WIDTH, body_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = space::XS;
                        for (index, candidate) in pages.iter().enumerate() {
                            if widgets::nav_item(ui, p, candidate.label(), index == page).clicked()
                            {
                                page = index;
                            }
                        }
                    },
                );
                ui.separator();
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), body_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt(page)
                            .max_height(body_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = space::SM;
                                match pages[page] {
                                    Page::Appearance => appearance(ui, p, settings, &mut changed),
                                    Page::Splitting => splitting(ui, p, settings, &mut changed),
                                    Page::Names => names(ui, p, settings, &mut changed),
                                    Page::Cover => cover(ui, p, settings, &mut changed),
                                    Page::Audio => audio(ui, p, settings, player, &mut changed),
                                    Page::Online => lookup(ui, p, settings, &mut changed),
                                    Page::About => about(ui, p, settings),
                                }
                            });
                    },
                );
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reset all settings").clicked() {
                    *settings = AppSettings::default();
                    player.set_settings(settings.audio);
                    theme::select(ui.ctx(), settings.dark);
                    changed = true;
                }
                widgets::caption(
                    ui,
                    p,
                    &format!("DSD Workbench {}", env!("CARGO_PKG_VERSION")),
                );
            });
        });

    ctx.data_mut(|d| d.insert_temp(page_id, page));
    changed
}

fn page_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(22.0).strong());
}

fn appearance(ui: &mut Ui, p: &Palette, settings: &mut AppSettings, changed: &mut bool) {
    page_title(ui, "Appearance");
    widgets::group(ui, p, |ui| {
        widgets::setting_row(
            ui,
            p,
            true,
            "Theme",
            "Paper is easy on the eyes in daylight.",
            |ui| {
                if widgets::segmented(
                    ui,
                    &mut settings.dark,
                    &[(false, "Light (paper)"), (true, "Dark")],
                ) {
                    theme::select(ui.ctx(), settings.dark);
                    *changed = true;
                }
            },
        );
    });
}

fn splitting(ui: &mut Ui, p: &Palette, settings: &mut AppSettings, changed: &mut bool) {
    page_title(ui, "Splitting");
    widgets::group(ui, p, |ui| {
        widgets::setting_row(
            ui,
            p,
            true,
            "Verify files after splitting",
            "Reads every new file back and compares it with the source. Adds some time.",
            |ui| *changed |= widgets::toggle(ui, p, &mut settings.verify_after_split),
        );
    });
}

fn names(ui: &mut Ui, p: &Palette, settings: &mut AppSettings, changed: &mut bool) {
    page_title(ui, "CUE sheets and file names");
    widgets::caption(
        ui,
        p,
        "These apply to CUE sheets you add afterwards, or to an album when you re-read its CUE sheet.",
    );

    widgets::group_title(ui, p, "CUE sheets");
    widgets::group(ui, p, |ui| {
        widgets::setting_stack(
            ui,
            p,
            true,
            "Pregap audio (INDEX 00) goes to the",
            "",
            |ui| {
                let options: Vec<_> = GapMode::ALL.iter().map(|m| (*m, m.label())).collect();
                *changed |= widgets::segmented(ui, &mut settings.cue.gap, &options);
            },
        );
        widgets::setting_row(
            ui,
            p,
            false,
            "Hidden track 00",
            "Keep audio before the first track as its own track.",
            |ui| *changed |= widgets::toggle(ui, p, &mut settings.cue.hidden_track),
        );
    });

    widgets::group_title(ui, p, "File names");
    widgets::group(ui, p, |ui| {
        widgets::setting_stack(
            ui,
            p,
            true,
            "File name",
            "Placeholders: {track} {track:02} {title} {artist} {albumartist} {album} {composer} {genre} {year} {date} {disc} {tag:NAME}",
            |ui| {
                *changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut settings.naming.file)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
            },
        );
        widgets::setting_stack(
            ui,
            p,
            false,
            "Sub-folder (optional)",
            "A \"/\" starts a new folder.",
            |ui| {
                *changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut settings.naming.folder)
                            .hint_text("{albumartist}/{album} ({year})")
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
            },
        );
        widgets::setting_stack(ui, p, false, "Characters", "", |ui| {
            let options: Vec<_> = NameStyle::ALL.iter().map(|s| (*s, s.label())).collect();
            *changed |= widgets::segmented(ui, &mut settings.naming.style, &options);
        });
    });
    ui.add_space(space::SM);
    match settings.naming.validate() {
        Ok(()) => widgets::caption(
            ui,
            p,
            &format!("Example: {}", naming_example(&settings.naming)),
        ),
        Err(err) => widgets::banner(ui, p, widgets::Tone::Bad, &err.to_string()),
    }
}

fn cover(ui: &mut Ui, p: &Palette, settings: &mut AppSettings, changed: &mut bool) {
    page_title(ui, "Cover art");
    widgets::caption(
        ui,
        p,
        "Applies to CUE sheets you add afterwards. You can pick a different image per album.",
    );
    widgets::group(ui, p, |ui| {
        widgets::setting_row(
            ui,
            p,
            true,
            "Embed cover art in every track",
            "Uses cover.jpg, folder.jpg or front.png found next to the CUE sheet.",
            |ui| *changed |= widgets::toggle(ui, p, &mut settings.embed_cover),
        );
        widgets::setting_row(
            ui,
            p,
            false,
            "Also copy it into the album folder",
            "Saved as folder.jpg.",
            |ui| *changed |= widgets::toggle(ui, p, &mut settings.copy_cover),
        );
    });
}

fn audio(
    ui: &mut Ui,
    p: &Palette,
    settings: &mut AppSettings,
    player: &mut Player,
    changed: &mut bool,
) {
    page_title(ui, "Audition audio");
    widgets::caption(ui, p, "Applies the next time you press a listen button.");
    let before = settings.audio;
    widgets::group(ui, p, |ui| {
        widgets::setting_stack(ui, p, true, "Output sample rate", "", |ui| {
            let options: Vec<_> = RateTarget::ALL.iter().map(|r| (*r, r.label())).collect();
            widgets::segmented(ui, &mut settings.audio.rate, &options);
        });
        widgets::setting_stack(ui, p, false, "Filter", "", |ui| {
            let options: Vec<_> = Quality::ALL.iter().map(|q| (*q, q.label())).collect();
            widgets::segmented(ui, &mut settings.audio.quality, &options);
        });
    });
    if settings.audio != before {
        player.set_settings(settings.audio);
        *changed = true;
    }
}

fn lookup(ui: &mut Ui, p: &Palette, settings: &mut AppSettings, changed: &mut bool) {
    page_title(ui, "Online lookup");
    widgets::caption(
        ui,
        p,
        "Fills in titles and album details from a release database. Only what you search for is sent.",
    );
    widgets::group(ui, p, |ui| {
        widgets::setting_row(ui, p, true, "Default source", "", |ui| {
            let options: Vec<_> = release_lookup::Source::ALL
                .iter()
                .map(|s| (*s, s.name()))
                .collect();
            *changed |= widgets::segmented(ui, &mut settings.lookup.source, &options);
        });
        widgets::setting_stack(
            ui,
            p,
            false,
            "Discogs personal access token (optional)",
            "Discogs works without one but allows fewer requests per minute. Kept in the settings file as plain text and left out of copied diagnostics. The DISCOGS_TOKEN environment variable is used when this is empty.",
            |ui| {
                *changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut settings.lookup.discogs_token)
                            .password(true)
                            .hint_text("Discogs → Settings → Developers → generate a token")
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
            },
        );
    });
}

fn about(ui: &mut Ui, p: &Palette, settings: &AppSettings) {
    page_title(ui, "About and privacy");
    widgets::group_title(ui, p, "Privacy");
    widgets::group(ui, p, |ui| {
        ui.add_space(space::SM);
        ui.add(
            egui::Label::new(if online::ENABLED {
                "No telemetry. Your source files are never modified. This is an online build: the only network use is the lookup you start yourself, which sends what you searched for."
            } else {
                "Fully offline: no telemetry, no network access. Your source files are never modified."
            })
            .wrap(),
        );
        ui.add_space(space::SM);
    });

    widgets::group_title(ui, p, "Diagnostics");
    widgets::group(ui, p, |ui| {
        widgets::setting_row(
            ui,
            p,
            true,
            "Copy diagnostics",
            "System details and settings for a bug report. Secrets are left out.",
            |ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx()
                        .copy_text(diag::report(&settings.serialize_for_report()));
                }
            },
        );
        widgets::setting_row(
            ui,
            p,
            false,
            "Logs and crash reports",
            &diag::dir().display().to_string(),
            |ui| {
                if ui.button("Open folder").clicked()
                    && let Err(err) = diag::reveal(&diag::ensure_dir())
                {
                    diag::log(format!("could not open the log folder: {err}"));
                }
            },
        );
        if let Some(path) = AppSettings::path() {
            widgets::setting_row(
                ui,
                p,
                false,
                "Settings file",
                &path.display().to_string(),
                |_| {},
            );
        }
    });
}
