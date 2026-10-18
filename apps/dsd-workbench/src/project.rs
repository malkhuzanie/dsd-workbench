//! The batch model: a list of CUE sheets, each with its own plan and output folder.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cover;
use crate::cue::{CueParser, CueSheet, CueTrack};
use crate::disk;
use crate::engine::AlbumReport;
use crate::error::{Error, Result};
use crate::locate::{self, Match};
use crate::metadata::FieldKey;
use crate::plan::{self, CoverPlan, PlanOptions, SplitPlan, target_path};
use crate::snapshot::{self, SourceInfo, SourceState};
use crate::timecode;

const MAX_SCAN_DEPTH: usize = 5;
const TAG_ALLOWANCE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyBadge {
    None,
    /// A report exists but the plan or output folder changed since.
    Stale,
    Clean,
    /// Number of tracks with differences or notes.
    Problems(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputMode {
    /// Write into the folder that contains the CUE sheet.
    BesideCue,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Queued,
    Running,
    Done,
    Previewed,
    Failed(String),
    Cancelled,
}

pub struct Entry {
    pub id: u64,
    /// Canonical path, used to detect duplicates.
    key: PathBuf,
    pub cue_path: PathBuf,
    pub base_dir: PathBuf,
    pub original: SplitPlan,
    pub plan: SplitPlan,
    pub sources: BTreeMap<String, SourceInfo>,
    pub warnings: Vec<String>,
    pub output: OutputMode,
    pub enabled: bool,
    pub state: RunState,
    pub verification: Option<AlbumReport>,
    /// Set when the album was made from audio files rather than a CUE sheet on disk:
    /// the sheet it was generated from, used to start over.
    pub generated: Option<CueSheet>,
}

impl Entry {
    fn open(id: u64, key: PathBuf, cue_path: &Path, options: &PlanOptions) -> Result<Self> {
        let sheet = CueParser::parse_file(cue_path).map_err(Error::io(format!(
            "reading CUE sheet {}",
            cue_path.display()
        )))?;
        Self::from_sheet(id, key, cue_path, sheet, options, false)
    }

    /// Builds an album from an already parsed sheet. `generated` marks sheets that do not
    /// exist as a file (see [`Project::add_audio`]).
    fn from_sheet(
        id: u64,
        key: PathBuf,
        cue_path: &Path,
        sheet: CueSheet,
        options: &PlanOptions,
        generated: bool,
    ) -> Result<Self> {
        if sheet.tracks.is_empty() {
            return Err(Error::NoTracks);
        }
        let mut plan = SplitPlan::from_cue(&sheet, options.cue);
        plan.naming = options.naming.clone();
        let base_dir = plan::base_dir(cue_path);
        if options.embed_cover || options.copy_cover {
            plan.cover = cover::find(&base_dir).map(|file| CoverPlan {
                file,
                embed: options.embed_cover,
                copy_to_folder: options.copy_cover,
            });
        }
        let kept_sheet = generated.then(|| sheet.clone());
        let mut warnings = sheet.warnings;
        resolve_sources(&base_dir, &mut plan, &mut warnings);

        let files = plan.source_files();
        let sources: BTreeMap<String, SourceInfo> = files
            .iter()
            .map(|file| (file.clone(), snapshot::probe(&base_dir.join(file))))
            .collect();
        let seed = files
            .first()
            .and_then(|file| sources.get(file))
            .and_then(|source| source.tags.tags());
        if let Some(tags) = seed {
            plan.seed_from_source(tags);
        }

        Ok(Self {
            id,
            key,
            cue_path: cue_path.to_path_buf(),
            base_dir,
            original: plan.clone(),
            plan,
            sources,
            warnings,
            output: OutputMode::BesideCue,
            enabled: true,
            state: RunState::Idle,
            verification: None,
            generated: kept_sheet,
        })
    }

    /// Re-reads the CUE sheet with `options`, discarding edits and verification results.
    /// The output choice and the enabled flag are kept. An album made from audio files
    /// starts over from its generated sheet.
    pub fn reload(&mut self, options: &PlanOptions) -> Result<()> {
        let mut fresh = match self.generated.clone() {
            Some(sheet) => Self::from_sheet(
                self.id,
                self.key.clone(),
                &self.cue_path,
                sheet,
                options,
                true,
            )?,
            None => Self::open(self.id, self.key.clone(), &self.cue_path, options)?,
        };
        fresh.output = self.output.clone();
        fresh.enabled = self.enabled;
        *self = fresh;
        Ok(())
    }

    pub fn cue_stem(&self) -> String {
        self.cue_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "album".to_owned())
    }

    pub fn display_name(&self) -> String {
        let name = self.plan.album.value(&FieldKey::ALBUM).trim();
        if name.is_empty() {
            self.cue_stem()
        } else {
            name.to_owned()
        }
    }

    pub fn output_dir(&self) -> PathBuf {
        match &self.output {
            OutputMode::BesideCue => self.base_dir.clone(),
            OutputMode::Custom(dir) => PathBuf::from(dir.trim()),
        }
    }

    pub fn is_modified(&self) -> bool {
        self.plan != self.original
    }

    pub fn reset(&mut self) {
        self.plan = self.original.clone();
    }

    /// Chooses (or removes) the cover image. Not counted as a user edit.
    pub fn set_cover(&mut self, cover: Option<CoverPlan>) {
        self.original.cover.clone_from(&cover);
        self.plan.cover = cover;
    }

    /// Points the tracks that read `old` at a file the user picked, and re-probes it.
    ///
    /// The path is stored relative to the CUE folder when it lies inside it.
    pub fn relink_source(&mut self, old: &str, picked: &Path) {
        let new = match picked.strip_prefix(&self.base_dir) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => picked.to_string_lossy().into_owned(),
        };
        // Both plans change, so relinking is not an edit the user has to undo.
        self.original.rename_source(old, &new);
        self.plan.rename_source(old, &new);
        self.sources.remove(old);
        self.warnings.retain(|w| !w.contains(&format!("'{old}'")));
        self.rescan_sources();
    }

    pub fn rescan_sources(&mut self) {
        for file in self.plan.source_files() {
            let info = snapshot::probe(&self.base_dir.join(&file));
            self.sources.insert(file, info);
        }
    }

    /// Everything that prevents this album from being split.
    pub fn problems(&self) -> Vec<String> {
        let mut out = self.plan.validate().err().unwrap_or_default();
        let out_dir = self.output_dir();
        if out_dir.as_os_str().is_empty() {
            out.push("Choose an output folder.".to_owned());
        }

        for (file, info) in &self.sources {
            match &info.state {
                SourceState::Missing => {
                    out.push(format!(
                        "Source file '{file}' was not found next to the CUE sheet."
                    ));
                }
                SourceState::Invalid(err) => {
                    out.push(format!(
                        "Source file '{file}' is not a usable DSD file: {err}"
                    ));
                }
                SourceState::Ready { duration_secs, .. } => {
                    for track in self.plan.tracks.iter().filter(|t| &t.source_file == file) {
                        if track.start_seconds >= *duration_secs {
                            out.push(format!(
                                "Track {:02} starts after the end of '{file}' ({}).",
                                track.meta.number,
                                timecode::format(*duration_secs)
                            ));
                        }
                    }
                }
            }
        }

        if out_dir.exists() && !out_dir.is_dir() {
            out.push("The output path exists but is not a folder.".to_owned());
        }
        for track in &self.plan.tracks {
            let target = out_dir.join(target_path(&self.plan.naming, &self.plan.album, track));
            if self
                .sources
                .keys()
                .any(|f| same_path(&self.base_dir.join(f), &target))
            {
                out.push(format!(
                    "Track {:02} would overwrite its own source file.",
                    track.meta.number
                ));
            }
            if disk::path_too_long(&target) {
                out.push(format!(
                    "Track {:02}: the output path is too long for Windows.",
                    track.meta.number
                ));
            }
        }
        out
    }

    /// Bytes the split will write: whole blocks per channel plus a tag allowance.
    pub fn estimated_bytes(&self) -> u64 {
        const BLOCK: u64 = 4096;
        let mut total = 0u64;
        for (i, track) in self.plan.tracks.iter().enumerate() {
            let Some(SourceState::Ready {
                duration_secs,
                sample_rate,
                channels,
            }) = self.sources.get(&track.source_file).map(|s| &s.state)
            else {
                continue;
            };
            let seconds = self
                .plan
                .duration_of(i)
                .unwrap_or(duration_secs - track.start_seconds)
                .max(0.0);
            let samples = seconds * f64::from(*sample_rate);
            let blocks = (samples / (BLOCK * 8) as f64).ceil() as u64;
            total =
                total.saturating_add(blocks * BLOCK * u64::from(*channels) + TAG_ALLOWANCE_BYTES);
        }
        total
    }

    pub fn verify_badge(&self) -> VerifyBadge {
        match &self.verification {
            None => VerifyBadge::None,
            Some(r) if r.plan != self.plan || r.output_dir != self.output_dir() => {
                VerifyBadge::Stale
            }
            Some(r) if r.is_clean() => VerifyBadge::Clean,
            Some(r) => VerifyBadge::Problems(r.problem_tracks()),
        }
    }
}

/// An audio file and the times (seconds) at which new tracks begin after the first.
#[derive(Debug, Clone, Default)]
pub struct AudioFile {
    pub path: PathBuf,
    pub cuts: Vec<f64>,
}

/// A CUE sheet in memory for `files`, and the (possibly non-existent) path it stands for.
fn sheet_from_audio(files: &[AudioFile], base_dir: &Path) -> (CueSheet, PathBuf) {
    let mut sheet = CueSheet::default();
    let mut number = 1u32;
    for file in files {
        // Files beside the first are named relative to it; others by full path.
        let name = match file.path.strip_prefix(base_dir) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => file.path.to_string_lossy().into_owned(),
        };
        let mut starts = vec![0.0];
        starts.extend(file.cuts.iter().copied().filter(|cut| *cut > 0.0));
        starts.sort_by(f64::total_cmp);
        starts.dedup();
        for start in starts {
            sheet.tracks.push(CueTrack {
                number,
                title: format!("Track {number:02}"),
                artist: String::new(),
                composer: String::new(),
                start_seconds: start,
                pregap_seconds: None,
                isrc: String::new(),
                source_file: name.clone(),
            });
            number += 1;
        }
    }
    let stem = files
        .first()
        .and_then(|f| f.path.file_stem())
        .map_or_else(|| "album".to_owned(), |s| s.to_string_lossy().into_owned());
    (sheet, base_dir.join(format!("{stem}.cue")))
}

/// How many DSD files to name when a `FILE` entry cannot be found.
const SUGGESTION_LIMIT: usize = 5;

/// Replaces each `FILE` name by the file that really exists, warning about guesses.
fn resolve_sources(base_dir: &Path, plan: &mut SplitPlan, warnings: &mut Vec<String>) {
    let listing = locate::list_files(base_dir);
    for file in plan.source_files() {
        match locate::resolve(base_dir, &file) {
            Some(found) => {
                if found.how != Match::Exact {
                    warnings.push(format!(
                        "FILE '{file}' was not found; using '{}' instead.",
                        found.name
                    ));
                }
                if found.name != file {
                    plan.rename_source(&file, &found.name);
                }
            }
            None => {
                let mut message = format!("FILE '{file}' was not found next to the CUE sheet.");
                let nearby = locate::dsd_files(&listing);
                if !nearby.is_empty() {
                    let shown: Vec<&str> = nearby
                        .iter()
                        .take(SUGGESTION_LIMIT)
                        .map(String::as_str)
                        .collect();
                    message.push_str(&format!(" DSD files in that folder: {}.", shown.join(", ")));
                }
                warnings.push(message);
            }
        }
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

#[derive(Default)]
pub struct Project {
    pub entries: Vec<Entry>,
    /// How CUE sheets added from now on are interpreted.
    pub options: PlanOptions,
    next_id: u64,
}

impl Project {
    pub fn add(&mut self, cue_path: &Path) -> Result<u64> {
        let key = fs::canonicalize(cue_path)
            .map_err(Error::io(format!("opening {}", cue_path.display())))?;
        if self.entries.iter().any(|e| e.key == key) {
            return Err(Error::AlreadyAdded);
        }
        let id = self.next_id;
        let entry = Entry::open(id, key, cue_path, &self.options)?;
        self.next_id += 1;
        self.entries.push(entry);
        Ok(id)
    }

    /// Starts an album from audio files, with no CUE sheet. Each file contributes a first
    /// track at 0:00 plus one track per cut time (seconds); numbering runs on across files.
    ///
    /// The album is named after its first file and writes beside it.
    pub fn add_audio(&mut self, files: &[AudioFile]) -> Result<u64> {
        let first = files.first().ok_or(Error::NoTracks)?;
        let key = fs::canonicalize(&first.path)
            .map_err(Error::io(format!("opening {}", first.path.display())))?;
        if self.entries.iter().any(|e| e.key == key) {
            return Err(Error::AlreadyAdded);
        }
        let base_dir = plan::base_dir(&first.path);
        let (sheet, cue_path) = sheet_from_audio(files, &base_dir);
        let id = self.next_id;
        let entry = Entry::from_sheet(id, key, &cue_path, sheet, &self.options, true)?;
        self.next_id += 1;
        self.entries.push(entry);
        Ok(id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn remove(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Every album goes to `root/<cue name>`.
    pub fn set_output_under(&mut self, root: &Path) {
        for entry in &mut self.entries {
            let dir = root.join(entry.cue_stem());
            entry.output = OutputMode::Custom(dir.to_string_lossy().into_owned());
        }
    }

    pub fn reset_outputs(&mut self) {
        for entry in &mut self.entries {
            entry.output = OutputMode::BesideCue;
        }
    }

    /// Problems per enabled album (albums without problems are absent),
    /// including output files that two albums would both write.
    pub fn validate(&self) -> HashMap<u64, Vec<String>> {
        let mut report: HashMap<u64, Vec<String>> = HashMap::new();
        let mut owners: BTreeMap<String, (PathBuf, Vec<u64>)> = BTreeMap::new();

        for entry in self.entries.iter().filter(|e| e.enabled) {
            let problems = entry.problems();
            if !problems.is_empty() {
                report.insert(entry.id, problems);
            }
            let dir = entry.output_dir();
            for track in &entry.plan.tracks {
                let path = dir.join(target_path(&entry.plan.naming, &entry.plan.album, track));
                let slot = owners
                    .entry(path.to_string_lossy().to_lowercase())
                    .or_insert_with(|| (path, Vec::new()));
                if !slot.1.contains(&entry.id) {
                    slot.1.push(entry.id);
                }
            }
        }

        for (path, ids) in owners.values().filter(|(_, ids)| ids.len() > 1) {
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for id in ids {
                let others: Vec<String> = ids
                    .iter()
                    .filter(|other| *other != id)
                    .filter_map(|other| self.entries.iter().find(|e| e.id == *other))
                    .map(Entry::display_name)
                    .collect();
                report.entry(*id).or_default().push(format!(
                    "'{file}' in the same folder is also produced by {}.",
                    others.join(", ")
                ));
            }
        }
        report
    }
}

pub fn is_cue(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
}

/// Recursively finds `.cue` files below `dir`.
pub fn find_cues(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(dir, 0, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else { return };
    for item in read.flatten() {
        let path = item.path();
        if path.is_dir() {
            walk(&path, depth + 1, found);
        } else if is_cue(&path) {
            found.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    const CUE: &str =
        "TITLE \"A\"\nFILE \"Side A.wav\" WAVE\n  TRACK 01 AUDIO\n    INDEX 01 00:00:00\n";

    fn project_with(
        files: &[&str],
    ) -> std::result::Result<(tempfile::TempDir, Project), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        for file in files {
            fs::write(dir.path().join(file), b"not audio")?;
        }
        fs::write(dir.path().join("album.cue"), CUE)?;
        let mut project = Project::default();
        project.add(&dir.path().join("album.cue"))?;
        Ok((dir, project))
    }

    #[test]
    fn a_wav_entry_resolves_to_the_dsf_beside_the_cue() -> TestResult {
        let (_dir, project) = project_with(&["side a.dsf"])?;
        let entry = project.entries.first().ok_or("no entry")?;
        assert_eq!(entry.plan.source_files(), ["side a.dsf"]);
        assert_eq!(entry.original.source_files(), ["side a.dsf"]);
        assert!(
            entry
                .warnings
                .iter()
                .any(|w| w.contains("using 'side a.dsf'"))
        );
        assert!(!entry.is_modified(), "resolving is not a user edit");
        Ok(())
    }

    #[test]
    fn an_unresolvable_entry_names_nearby_dsd_files() -> TestResult {
        let (_dir, project) = project_with(&["other.dsf", "notes.txt"])?;
        let entry = project.entries.first().ok_or("no entry")?;
        assert_eq!(entry.plan.source_files(), ["Side A.wav"]);
        let warning = entry.warnings.last().ok_or("no warning")?;
        assert!(
            warning.contains("other.dsf") && !warning.contains("notes.txt"),
            "{warning}"
        );
        Ok(())
    }

    fn audio(path: &Path, cuts: &[f64]) -> AudioFile {
        AudioFile {
            path: path.to_path_buf(),
            cuts: cuts.to_vec(),
        }
    }

    #[test]
    fn an_album_can_start_from_audio_files_with_cuts() -> TestResult {
        let dir = tempfile::tempdir()?;
        let (a, b) = (dir.path().join("a.dsf"), dir.path().join("b.dsf"));
        fs::write(&a, b"x")?;
        fs::write(&b, b"x")?;

        let mut project = Project::default();
        let id = project.add_audio(&[audio(&a, &[30.0, 10.0, 0.0, 10.0, -3.0]), audio(&b, &[])])?;
        let entry = project.get_mut(id).ok_or("no entry")?;

        let tracks: Vec<(u32, &str, f64)> = entry
            .plan
            .tracks
            .iter()
            .map(|t| (t.meta.number, t.source_file.as_str(), t.start_seconds))
            .collect();
        assert_eq!(
            tracks,
            [
                (1, "a.dsf", 0.0),
                (2, "a.dsf", 10.0),
                (3, "a.dsf", 30.0),
                (4, "b.dsf", 0.0)
            ],
            "cuts are sorted, deduplicated and positive; numbering continues across files"
        );
        assert_eq!(entry.cue_path, dir.path().join("a.cue"));
        assert!(entry.generated.is_some());
        assert!(!entry.cue_path.exists(), "no file is written");
        Ok(())
    }

    #[test]
    fn the_same_audio_cannot_be_added_twice_and_empty_input_is_refused() -> TestResult {
        let dir = tempfile::tempdir()?;
        let a = dir.path().join("a.dsf");
        fs::write(&a, b"x")?;
        let mut project = Project::default();
        project.add_audio(&[audio(&a, &[])])?;
        assert!(matches!(
            project.add_audio(&[audio(&a, &[])]),
            Err(Error::AlreadyAdded)
        ));
        assert!(project.add_audio(&[]).is_err());
        Ok(())
    }

    #[test]
    fn re_reading_an_audio_album_starts_from_its_generated_sheet() -> TestResult {
        let dir = tempfile::tempdir()?;
        let a = dir.path().join("a.dsf");
        fs::write(&a, b"x")?;
        let mut project = Project::default();
        let id = project.add_audio(&[audio(&a, &[10.0])])?;
        let options = project.options.clone();
        let entry = project.get_mut(id).ok_or("no entry")?;
        entry.plan.tracks.pop();
        assert!(entry.is_modified());
        entry.reload(&options)?;
        assert_eq!(entry.plan.tracks.len(), 2);
        assert!(!entry.is_modified());
        assert!(entry.generated.is_some());
        Ok(())
    }

    #[test]
    fn relinking_updates_both_plans_and_keeps_the_album_unmodified() -> TestResult {
        let (dir, mut project) = project_with(&["other.dsf"])?;
        let entry = project.entries.first_mut().ok_or("no entry")?;
        entry.relink_source("Side A.wav", &dir.path().join("other.dsf"));

        assert_eq!(entry.plan.source_files(), ["other.dsf"]);
        assert!(!entry.is_modified());
        assert!(entry.sources.contains_key("other.dsf"));
        assert!(!entry.sources.contains_key("Side A.wav"));
        assert!(entry.warnings.iter().all(|w| !w.contains("'Side A.wav'")));
        Ok(())
    }
}
