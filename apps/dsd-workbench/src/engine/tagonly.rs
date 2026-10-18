//! Applies a plan's metadata to DSF files that already exist, without splitting anything.
//!
//! Only the tag at the end of each file is rewritten; the audio is never touched. Files
//! are matched to the plan's tracks by the name the plan would give them, then by the
//! track number at the start of the file name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::verify::tag_issues;
use crate::cover;
use crate::error::{Error, Result};
use crate::locate;
use crate::metadata::FieldKey;
use crate::plan::{SplitPlan, target_path};

/// What happened to one track's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The tags were rewritten and re-read successfully.
    Tagged,
    /// The tags already matched; the file was not touched.
    UpToDate,
    /// A dry run: the tags differ and would be rewritten.
    WouldTag,
    /// No file in the folder matches this track.
    NoFile,
    /// Several files claim this track number, so none was changed.
    Ambiguous(Vec<String>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub number: u32,
    pub title: String,
    pub file: Option<PathBuf>,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub items: Vec<Item>,
}

impl Report {
    pub fn count(&self, wanted: &Outcome) -> usize {
        self.items.iter().filter(|i| &i.outcome == wanted).count()
    }

    /// Tracks that could not be tagged (no file, ambiguous, or failed).
    pub fn problems(&self) -> usize {
        self.items
            .iter()
            .filter(|i| {
                matches!(
                    i.outcome,
                    Outcome::NoFile | Outcome::Ambiguous(_) | Outcome::Failed(_)
                )
            })
            .count()
    }

    /// One line for a status bar.
    pub fn summary(&self) -> String {
        let tagged = self.count(&Outcome::Tagged);
        let would = self.count(&Outcome::WouldTag);
        let current = self.count(&Outcome::UpToDate);
        let problems = self.problems();
        let mut parts = Vec::new();
        if tagged > 0 {
            parts.push(format!("{tagged} tagged"));
        }
        if would > 0 {
            parts.push(format!("{would} would be tagged"));
        }
        if current > 0 {
            parts.push(format!("{current} already up to date"));
        }
        if problems > 0 {
            parts.push(format!("{problems} need attention"));
        }
        if parts.is_empty() {
            "nothing to do".to_owned()
        } else {
            parts.join(", ")
        }
    }
}

impl Item {
    /// A one-line description for lists and logs.
    pub fn describe(&self) -> String {
        let file = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match &self.outcome {
            Outcome::Tagged => format!("tagged {file}"),
            Outcome::UpToDate => format!("{file} is already up to date"),
            Outcome::WouldTag => format!("would tag {file}"),
            Outcome::NoFile => "no matching file found".to_owned(),
            Outcome::Ambiguous(names) => format!("several files match: {}", names.join(", ")),
            Outcome::Failed(why) => format!("failed on {file}: {why}"),
        }
    }
}

/// The track number a file name starts with: `01 - x`, `1. x`, `Track 03`, `07-x`.
pub fn leading_number(file_name: &str) -> Option<u32> {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let stem = stem.trim_start();
    let stem = match stem.get(..5) {
        Some(prefix) if prefix.eq_ignore_ascii_case("track") => stem.get(5..)?.trim_start(),
        _ => stem,
    };
    let digits: String = stem.chars().take_while(char::is_ascii_digit).collect();
    // Years and long catalogue numbers are not track numbers.
    (!digits.is_empty() && digits.len() <= 3)
        .then(|| digits.parse().ok())
        .flatten()
}

/// How one wanted track number matched the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pairing {
    /// Index into the file list.
    File(usize),
    Missing,
    /// More than one file starts with this number.
    Ambiguous(Vec<usize>),
}

/// Matches each wanted track number with the file whose name starts with it.
pub fn pair_by_number(wanted: &[u32], files: &[String]) -> Vec<Pairing> {
    wanted
        .iter()
        .map(|number| {
            let hits: Vec<usize> = files
                .iter()
                .enumerate()
                .filter(|(_, name)| leading_number(name) == Some(*number))
                .map(|(index, _)| index)
                .collect();
            match hits.as_slice() {
                [] => Pairing::Missing,
                [one] => Pairing::File(*one),
                _ => Pairing::Ambiguous(hits),
            }
        })
        .collect()
}

fn is_dsf(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("dsf"))
}

/// Tags the files in `dir` according to `plan`. With `dry_run`, only reports what would change.
pub fn apply(
    plan: &SplitPlan,
    base_dir: &Path,
    dir: &Path,
    dry_run: bool,
    cancel: &AtomicBool,
) -> Result<Report> {
    plan.validate().map_err(Error::InvalidPlan)?;
    if !dir.is_dir() {
        return Err(Error::NotAFolder(dir.display().to_string()));
    }
    let picture = match &plan.cover {
        Some(c) if c.embed && !dry_run => Some(cover::load(&base_dir.join(&c.file))?),
        _ => None,
    };

    // First the names the plan itself would produce, then the track number in the name.
    let direct: Vec<Option<PathBuf>> = plan
        .tracks
        .iter()
        .map(|track| {
            let path = dir.join(target_path(&plan.naming, &plan.album, track));
            path.is_file().then_some(path)
        })
        .collect();
    let listing: Vec<String> = locate::list_files(dir)
        .into_iter()
        .filter(|name| is_dsf(name))
        .filter(|name| !direct.iter().flatten().any(|p| *p == dir.join(name)))
        .collect();
    let open: Vec<usize> = (0..plan.tracks.len())
        .filter(|i| direct.get(*i).is_some_and(Option::is_none))
        .collect();
    let numbers: Vec<u32> = open
        .iter()
        .filter_map(|i| plan.tracks.get(*i))
        .map(|t| t.meta.number)
        .collect();
    let mut by_number: HashMap<usize, Pairing> = open
        .iter()
        .copied()
        .zip(pair_by_number(&numbers, &listing))
        .collect();

    let mut report = Report::default();
    for (index, track) in plan.tracks.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let title = plan.album.resolve(&track.meta, &FieldKey::TITLE).to_owned();
        let mut item = Item {
            number: track.meta.number,
            title,
            file: None,
            outcome: Outcome::NoFile,
        };

        let path = match direct.get(index).cloned().flatten() {
            Some(path) => Some(path),
            None => match by_number.remove(&index) {
                Some(Pairing::File(f)) => listing.get(f).map(|name| dir.join(name)),
                Some(Pairing::Ambiguous(many)) => {
                    item.outcome = Outcome::Ambiguous(
                        many.iter()
                            .filter_map(|f| listing.get(*f))
                            .cloned()
                            .collect(),
                    );
                    None
                }
                _ => None,
            },
        };

        if let Some(path) = path {
            item.outcome = tag_one(plan, track, &path, picture.as_ref(), dry_run);
            item.file = Some(path);
        }
        report.items.push(item);
    }
    Ok(report)
}

fn tag_one(
    plan: &SplitPlan,
    track: &crate::plan::TrackPlan,
    path: &Path,
    picture: Option<&dsf::Picture>,
    dry_run: bool,
) -> Outcome {
    if tag_issues(plan, track, path).is_empty() {
        return Outcome::UpToDate;
    }
    if dry_run {
        return Outcome::WouldTag;
    }
    let edit = plan.tag_edit(track, picture);
    if let Err(err) = dsf::update_path(path, &edit, &dsf::WriteOptions::default()) {
        return Outcome::Failed(err.to_string());
    }
    let left = tag_issues(plan, track, path);
    if left.is_empty() {
        Outcome::Tagged
    } else {
        Outcome::Failed(format!(
            "tags still differ after writing: {}",
            left.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use dsf::id3::TagLike;

    use super::*;
    use crate::cue::{CueSheet, CueTrack};
    use crate::plan::CueOptions;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn write_dsf(path: &Path) -> TestResult {
        let audio = 4096 * 2;
        let header = dsf::DsfHeader {
            total_file_size: dsf::AUDIO_OFFSET + audio,
            metadata_offset: 0,
            channel_type: 2,
            channel_count: 2,
            sample_rate: 2_822_400,
            bits_per_sample: 1,
            sample_count: 32_768,
            block_size: 4096,
            data_chunk_size: 12 + audio,
        };
        let mut bytes = Vec::new();
        header.write(&mut bytes)?;
        bytes.extend(vec![0x5A; audio as usize]);
        fs::write(path, bytes)?;
        Ok(())
    }

    fn plan() -> SplitPlan {
        let track = |number, title: &str, start| CueTrack {
            number,
            title: title.to_owned(),
            artist: "Artist".to_owned(),
            composer: String::new(),
            start_seconds: start,
            pregap_seconds: None,
            isrc: String::new(),
            source_file: "side.dsf".to_owned(),
        };
        let sheet = CueSheet {
            album_title: "Album".to_owned(),
            performer: "Artist".to_owned(),
            tracks: vec![
                track(1, "One", 0.0),
                track(2, "Two", 10.0),
                track(3, "Three", 20.0),
            ],
            ..CueSheet::default()
        };
        SplitPlan::from_cue(&sheet, CueOptions::default())
    }

    fn run(plan: &SplitPlan, dir: &Path, dry: bool) -> Result<Report> {
        apply(plan, dir, dir, dry, &AtomicBool::new(false))
    }

    #[test]
    fn leading_numbers_are_read_from_common_name_styles() {
        for (name, want) in [
            ("01 - Intro.dsf", Some(1)),
            ("1. Intro.dsf", Some(1)),
            ("Track 07.dsf", Some(7)),
            ("track12 - x.dsf", Some(12)),
            ("007-x.dsf", Some(7)),
            ("  03 x.dsf", Some(3)),
            ("1959 Kind of Blue.dsf", None),
            ("Intro.dsf", None),
            ("", None),
            ("Tr", None),
        ] {
            assert_eq!(leading_number(name), want, "{name}");
        }
    }

    #[test]
    fn pairing_flags_missing_and_ambiguous_numbers() {
        let files = ["01 a.dsf", "02 b.dsf", "02 c.dsf"].map(String::from);
        assert_eq!(
            pair_by_number(&[1, 2, 3], &files),
            [
                Pairing::File(0),
                Pairing::Ambiguous(vec![1, 2]),
                Pairing::Missing
            ]
        );
    }

    #[test]
    fn files_are_matched_by_number_and_tagged_in_place() -> TestResult {
        let dir = tempfile::tempdir()?;
        write_dsf(&dir.path().join("01 - Whatever.dsf"))?;
        write_dsf(&dir.path().join("Track 02.dsf"))?;
        let audio_before = fs::read(dir.path().join("01 - Whatever.dsf"))?;

        let report = run(&plan(), dir.path(), false)?;
        assert_eq!(report.count(&Outcome::Tagged), 2);
        assert_eq!(report.count(&Outcome::NoFile), 1, "track 3 has no file");
        assert_eq!(report.problems(), 1);

        let tag = dsf::read_path(dir.path().join("Track 02.dsf"))?.ok_or("no tag")?;
        assert_eq!(tag.title(), Some("Two"));
        assert_eq!(tag.album(), Some("Album"));

        let after = fs::read(dir.path().join("01 - Whatever.dsf"))?;
        let audio = 92..92 + 8192;
        assert_eq!(
            audio_before.get(audio.clone()),
            after.get(audio),
            "audio untouched"
        );
        Ok(())
    }

    #[test]
    fn a_second_run_changes_nothing() -> TestResult {
        let dir = tempfile::tempdir()?;
        write_dsf(&dir.path().join("01.dsf"))?;
        run(&plan(), dir.path(), false)?;
        let before = fs::read(dir.path().join("01.dsf"))?;
        let report = run(&plan(), dir.path(), false)?;
        assert_eq!(report.count(&Outcome::UpToDate), 1);
        assert_eq!(fs::read(dir.path().join("01.dsf"))?, before);
        Ok(())
    }

    #[test]
    fn a_dry_run_reports_without_writing() -> TestResult {
        let dir = tempfile::tempdir()?;
        write_dsf(&dir.path().join("01.dsf"))?;
        let before = fs::read(dir.path().join("01.dsf"))?;
        let report = run(&plan(), dir.path(), true)?;
        assert_eq!(report.count(&Outcome::WouldTag), 1);
        assert_eq!(fs::read(dir.path().join("01.dsf"))?, before);
        assert!(report.summary().contains("would be tagged"));
        Ok(())
    }

    #[test]
    fn ambiguous_numbers_are_left_alone() -> TestResult {
        let dir = tempfile::tempdir()?;
        write_dsf(&dir.path().join("01 a.dsf"))?;
        write_dsf(&dir.path().join("01 b.dsf"))?;
        let report = run(&plan(), dir.path(), false)?;
        assert!(matches!(
            report.items.first().map(|i| &i.outcome),
            Some(Outcome::Ambiguous(names)) if names.len() == 2
        ));
        assert!(dsf::read_path(dir.path().join("01 a.dsf"))?.is_none());
        Ok(())
    }

    #[test]
    fn the_plans_own_names_win_over_number_guessing() -> TestResult {
        let dir = tempfile::tempdir()?;
        write_dsf(&dir.path().join("01 - One.dsf"))?;
        write_dsf(&dir.path().join("01 - decoy.dsf"))?;
        let report = run(&plan(), dir.path(), false)?;
        let first = report.items.first().ok_or("no items")?;
        assert_eq!(first.outcome, Outcome::Tagged);
        assert_eq!(
            first.file.as_deref(),
            Some(dir.path().join("01 - One.dsf").as_path())
        );
        Ok(())
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let result = run(&plan(), Path::new("/definitely/not/here"), false);
        assert!(result.is_err());
    }

    #[test]
    fn corrupt_files_fail_without_stopping_the_run() -> TestResult {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("01.dsf"), b"not a dsf file")?;
        write_dsf(&dir.path().join("02.dsf"))?;
        let report = run(&plan(), dir.path(), false)?;
        assert!(matches!(
            report.items.first().map(|i| &i.outcome),
            Some(Outcome::Failed(_))
        ));
        assert_eq!(report.count(&Outcome::Tagged), 1);
        Ok(())
    }
}
