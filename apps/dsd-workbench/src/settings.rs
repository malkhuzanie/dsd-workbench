//! Persistent app settings, stored as a tiny `key=value` file.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::audio::{Quality, RateTarget, Settings as AudioSettings};
use crate::diag;
use crate::naming::{NameStyle, Naming};
use crate::plan::{CueOptions, GapMode, PlanOptions};
use release_lookup::Source;

/// The settings file is line based, so a template must not contain a line break.
fn single_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Where online lookups go. Only used by builds with the `online` feature.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LookupSettings {
    pub source: Source,
    /// A Discogs personal access token (optional; raises the rate limit). Stored in the
    /// settings file as plain text, like any other setting.
    pub discogs_token: String,
}

impl LookupSettings {
    /// The Discogs token to use: the one saved here, else the `DISCOGS_TOKEN` environment
    /// variable. `None` if there is neither.
    pub fn token(&self) -> Option<String> {
        let saved = self.discogs_token.trim();
        if !saved.is_empty() {
            return Some(saved.to_owned());
        }
        std::env::var("DISCOGS_TOKEN")
            .ok()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppSettings {
    pub dark: bool,
    pub verify_after_split: bool,
    pub audio: AudioSettings,
    pub cue: CueOptions,
    pub naming: Naming,
    pub embed_cover: bool,
    pub copy_cover: bool,
    pub lookup: LookupSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        let plan = PlanOptions::default();
        Self {
            dark: false,
            verify_after_split: true,
            audio: AudioSettings::default(),
            cue: plan.cue,
            naming: plan.naming,
            embed_cover: plan.embed_cover,
            copy_cover: plan.copy_cover,
            lookup: LookupSettings::default(),
        }
    }
}

impl AppSettings {
    /// The options applied to CUE sheets added from now on.
    pub fn plan_options(&self) -> PlanOptions {
        PlanOptions {
            cue: self.cue,
            naming: self.naming.clone(),
            embed_cover: self.embed_cover,
            copy_cover: self.copy_cover,
        }
    }

    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(diag::APP_DIR).join("settings.conf"))
    }

    /// Unknown keys and invalid values are ignored.
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "theme" => settings.dark = value == "dark",
                "verify_after_split" => {
                    if let Ok(on) = value.parse() {
                        settings.verify_after_split = on;
                    }
                }
                "audio_rate" => {
                    if let Some(rate) = RateTarget::from_key(value) {
                        settings.audio.rate = rate;
                    }
                }
                "audio_quality" => {
                    if let Some(quality) = Quality::from_key(value) {
                        settings.audio.quality = quality;
                    }
                }
                "cue_gap" => {
                    if let Some(gap) = GapMode::from_key(value) {
                        settings.cue.gap = gap;
                    }
                }
                "cue_hidden_track" => {
                    if let Ok(on) = value.parse() {
                        settings.cue.hidden_track = on;
                    }
                }
                "lookup_source" => {
                    if let Some(source) = Source::from_key(value) {
                        settings.lookup.source = source;
                    }
                }
                "discogs_token" => settings.lookup.discogs_token = value.to_owned(),
                "embed_cover" => {
                    if let Ok(on) = value.parse() {
                        settings.embed_cover = on;
                    }
                }
                "copy_cover" => {
                    if let Ok(on) = value.parse() {
                        settings.copy_cover = on;
                    }
                }
                "name_file" if !value.is_empty() => settings.naming.file = value.to_owned(),
                "name_folder" => settings.naming.folder = value.to_owned(),
                "name_style" => {
                    if let Some(style) = NameStyle::from_key(value) {
                        settings.naming.style = style;
                    }
                }
                _ => {}
            }
        }
        settings
    }

    pub fn serialize(&self) -> String {
        format!(
            "theme={}\nverify_after_split={}\naudio_rate={}\naudio_quality={}\ncue_gap={}\ncue_hidden_track={}\nname_file={}\nname_folder={}\nname_style={}\nembed_cover={}\ncopy_cover={}\nlookup_source={}\ndiscogs_token={}\n",
            if self.dark { "dark" } else { "light" },
            self.verify_after_split,
            self.audio.rate.key(),
            self.audio.quality.key(),
            self.cue.gap.key(),
            self.cue.hidden_track,
            single_line(&self.naming.file),
            single_line(&self.naming.folder),
            self.naming.style.key(),
            self.embed_cover,
            self.copy_cover,
            self.lookup.source.key(),
            single_line(self.lookup.discogs_token.trim()),
        )
    }

    /// The settings as text for a bug report: like [`AppSettings::serialize`] but without
    /// the Discogs token, which is a credential.
    pub fn serialize_for_report(&self) -> String {
        let mut shown = self.clone();
        let had_token = !shown.lookup.discogs_token.trim().is_empty();
        shown.lookup.discogs_token = if had_token { "(set)" } else { "(none)" }.to_owned();
        shown.serialize()
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("conf.tmp");
        fs::write(&temp, self.serialize())?;
        fs::rename(temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let settings = AppSettings {
            dark: true,
            verify_after_split: false,
            audio: AudioSettings {
                rate: RateTarget::Maximum,
                quality: Quality::Precise,
            },
            cue: CueOptions {
                gap: GapMode::PrependToNext,
                hidden_track: true,
            },
            naming: Naming {
                file: "{artist} - {title} = {track}".to_owned(),
                folder: "{albumartist}/{album} ({year})".to_owned(),
                style: NameStyle::Native,
            },
            embed_cover: false,
            copy_cover: true,
            lookup: LookupSettings {
                source: Source::Discogs,
                discogs_token: "abc=123".to_owned(),
            },
        };
        assert_eq!(AppSettings::parse(&settings.serialize()), settings);
    }

    #[test]
    fn bug_reports_never_contain_the_discogs_token() {
        let mut settings = AppSettings::default();
        settings.lookup.discogs_token = "SECRET-TOKEN".to_owned();
        let report = settings.serialize_for_report();
        assert!(!report.contains("SECRET-TOKEN"), "{report}");
        assert!(report.contains("discogs_token=(set)"));
        assert!(
            AppSettings::default()
                .serialize_for_report()
                .contains("discogs_token=(none)")
        );
        assert!(
            settings.serialize().contains("SECRET-TOKEN"),
            "the settings file keeps it"
        );
    }

    #[test]
    fn the_saved_token_wins_over_the_environment_and_blank_means_none() {
        let lookup = LookupSettings {
            discogs_token: "  saved  ".to_owned(),
            ..LookupSettings::default()
        };
        assert_eq!(lookup.token().as_deref(), Some("saved"));
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        let parsed = AppSettings::parse("nonsense\nverify_after_split=maybe\naudio_rate=huge\n=\n");
        assert_eq!(parsed, AppSettings::default());
    }
}
