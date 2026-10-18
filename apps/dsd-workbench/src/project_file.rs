//! Saving and loading a project (`.dsfproj`): the albums in the list with all your edits.
//!
//! The file is JSON with a version number. It stores each album's CUE path and the
//! edited plan, never audio, and is written atomically.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::metadata::{FieldKey, Fields};
use crate::naming::{NameStyle, Naming};
use crate::plan::{AlbumMeta, CoverPlan, SplitPlan, TrackMeta, TrackPlan};
use crate::project::{AudioFile, Entry, OutputMode, Project};

/// File extension of project files, without the dot.
pub const EXTENSION: &str = "dsfproj";
const CURRENT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct SavedProject {
    version: u32,
    albums: Vec<SavedAlbum>,
}

#[derive(Serialize, Deserialize)]
struct SavedAlbum {
    /// Absolute path of the CUE sheet when it was saved.
    cue: String,
    /// The same path relative to the project file, tried when `cue` no longer exists.
    #[serde(default)]
    cue_relative: Option<String>,
    /// Albums made from audio files have no CUE sheet; these are their source files
    /// (absolute paths), and `cue` is empty.
    #[serde(default)]
    audio: Vec<String>,
    enabled: bool,
    /// `None`: write beside the CUE sheet.
    #[serde(default)]
    output: Option<String>,
    plan: SavedPlan,
}

#[derive(Serialize, Deserialize)]
struct SavedPlan {
    album: BTreeMap<String, String>,
    tracks: Vec<SavedTrack>,
    name_file: String,
    name_folder: String,
    name_style: String,
    #[serde(default)]
    cover: Option<SavedCover>,
}

#[derive(Serialize, Deserialize)]
struct SavedTrack {
    source_file: String,
    start_seconds: f64,
    number: u32,
    #[serde(default)]
    overrides: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct SavedCover {
    file: String,
    embed: bool,
    copy_to_folder: bool,
}

/// What loading a project produced.
#[derive(Debug, Default)]
pub struct LoadReport {
    pub added: usize,
    /// Albums or fields that could not be restored, in plain words.
    pub problems: Vec<String>,
}

fn encode_fields(fields: &Fields) -> BTreeMap<String, String> {
    fields
        .keys()
        .map(|key| (key.to_id(), fields.value(key).to_owned()))
        .collect()
}

fn decode_fields(saved: &BTreeMap<String, String>, problems: &mut Vec<String>) -> Fields {
    let mut fields = Fields::default();
    for (id, value) in saved {
        match FieldKey::from_id(id) {
            Some(key) => fields.set(key, value.as_str()),
            None => problems.push(format!("ignored the unknown field '{id}'")),
        }
    }
    fields
}

fn encode_plan(plan: &SplitPlan) -> SavedPlan {
    SavedPlan {
        album: encode_fields(&plan.album.fields),
        tracks: plan
            .tracks
            .iter()
            .map(|track| SavedTrack {
                source_file: track.source_file.clone(),
                start_seconds: track.start_seconds,
                number: track.meta.number,
                overrides: encode_fields(&track.meta.overrides),
            })
            .collect(),
        name_file: plan.naming.file.clone(),
        name_folder: plan.naming.folder.clone(),
        name_style: plan.naming.style.key().to_owned(),
        cover: plan.cover.as_ref().map(|c| SavedCover {
            file: c.file.clone(),
            embed: c.embed,
            copy_to_folder: c.copy_to_folder,
        }),
    }
}

fn decode_plan(saved: &SavedPlan, problems: &mut Vec<String>) -> SplitPlan {
    SplitPlan {
        album: AlbumMeta {
            fields: decode_fields(&saved.album, problems),
        },
        tracks: saved
            .tracks
            .iter()
            .map(|track| TrackPlan {
                source_file: track.source_file.clone(),
                start_seconds: track.start_seconds,
                meta: TrackMeta {
                    number: track.number,
                    overrides: decode_fields(&track.overrides, problems),
                },
            })
            .collect(),
        naming: Naming {
            file: saved.name_file.clone(),
            folder: saved.name_folder.clone(),
            style: NameStyle::from_key(&saved.name_style).unwrap_or_default(),
        },
        cover: saved.cover.as_ref().map(|c| CoverPlan {
            file: c.file.clone(),
            embed: c.embed,
            copy_to_folder: c.copy_to_folder,
        }),
    }
}

fn relative_to(path: &Path, base: &Path) -> Option<String> {
    path.strip_prefix(base)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

/// The project as JSON text. `project_dir` is where the file will live, for relative paths.
pub fn to_json(project: &Project, project_dir: &Path) -> Result<String> {
    let albums = project
        .entries
        .iter()
        .map(|entry| {
            let from_audio = entry.generated.is_some();
            SavedAlbum {
                cue: if from_audio {
                    String::new()
                } else {
                    entry.cue_path.to_string_lossy().into_owned()
                },
                cue_relative: relative_to(&entry.cue_path, project_dir).filter(|_| !from_audio),
                audio: if from_audio {
                    entry
                        .plan
                        .source_files()
                        .iter()
                        .map(|file| entry.base_dir.join(file).to_string_lossy().into_owned())
                        .collect()
                } else {
                    Vec::new()
                },
                enabled: entry.enabled,
                output: match &entry.output {
                    OutputMode::BesideCue => None,
                    OutputMode::Custom(dir) => Some(dir.clone()),
                },
                plan: encode_plan(&entry.plan),
            }
        })
        .collect();
    serde_json::to_string_pretty(&SavedProject {
        version: CURRENT_VERSION,
        albums,
    })
    .map_err(|err| Error::Internal(format!("could not encode the project: {err}")))
}

/// Writes the project to `path`, replacing any existing file only when complete.
pub fn save(project: &Project, path: &Path) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let text = to_json(project, dir)?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    fs::write(&temp, text).map_err(Error::io(format!("writing {}", temp.display())))?;
    fs::rename(&temp, path).map_err(Error::io(format!("saving {}", path.display())))
}

/// Adds the albums stored in `path` to `project`, restoring their edits.
///
/// Albums whose CUE sheet is gone are skipped and reported; the rest still load.
pub fn load(project: &mut Project, path: &Path) -> Result<LoadReport> {
    let text =
        fs::read_to_string(path).map_err(Error::io(format!("reading {}", path.display())))?;
    from_json(
        project,
        &text,
        path.parent().unwrap_or_else(|| Path::new(".")),
    )
}

/// [`load`] from text already in memory.
pub fn from_json(project: &mut Project, text: &str, project_dir: &Path) -> Result<LoadReport> {
    let saved: SavedProject = serde_json::from_str(text)
        .map_err(|err| Error::Internal(format!("this is not a valid project file: {err}")))?;
    if saved.version > CURRENT_VERSION {
        return Err(Error::Internal(format!(
            "this project was saved by a newer version (format {}); update the app to open it",
            saved.version
        )));
    }

    let mut report = LoadReport::default();
    for album in saved.albums {
        match add_album(project, &album, project_dir) {
            Ok(id) => {
                if let Some(entry) = project.get_mut(id) {
                    restore(entry, &album, &mut report.problems);
                }
                report.added += 1;
            }
            Err(problem) => report.problems.push(problem),
        }
    }
    Ok(report)
}

/// Opens the CUE sheet (or audio files) a saved album came from; the error is a sentence
/// for the user.
fn add_album(
    project: &mut Project,
    album: &SavedAlbum,
    project_dir: &Path,
) -> std::result::Result<u64, String> {
    if album.cue.is_empty() {
        // Made from audio files: the plan carries the cuts, the files are enough here.
        let files: Vec<AudioFile> = album
            .audio
            .iter()
            .map(|path| AudioFile {
                path: PathBuf::from(path),
                cuts: Vec::new(),
            })
            .collect();
        if let Some(missing) = files.iter().find(|f| !f.path.is_file()) {
            return Err(format!(
                "{} was not found and was skipped",
                missing.path.display()
            ));
        }
        let shown = album.audio.first().cloned().unwrap_or_default();
        return project.add_audio(&files).map_err(|err| match err {
            Error::AlreadyAdded => format!("{shown} is already in the list"),
            other => format!("{shown} could not be opened: {other}"),
        });
    }

    let absolute = PathBuf::from(&album.cue);
    let cue = if absolute.is_file() {
        absolute
    } else {
        match album.cue_relative.as_deref().map(|r| project_dir.join(r)) {
            Some(relative) if relative.is_file() => relative,
            _ => return Err(format!("{} was not found and was skipped", album.cue)),
        }
    };
    project.add(&cue).map_err(|err| match err {
        Error::AlreadyAdded => format!("{} is already in the list", cue.display()),
        other => format!("{} could not be opened: {other}", cue.display()),
    })
}

/// Lays the saved edits over a freshly opened album. `original` stays as the CUE sheet
/// gave it, so the edits show up as changes.
fn restore(entry: &mut Entry, saved: &SavedAlbum, problems: &mut Vec<String>) {
    let name = entry.display_name();
    let mut notes = Vec::new();
    let plan = decode_plan(&saved.plan, &mut notes);
    problems.extend(notes.into_iter().map(|note| format!("{name}: {note}")));

    entry.plan = plan;
    entry.enabled = saved.enabled;
    entry.output = match &saved.output {
        Some(dir) => OutputMode::Custom(dir.clone()),
        None => OutputMode::BesideCue,
    };
    // Sources may have been relinked or renamed since the CUE sheet was read.
    entry.rescan_sources();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Standard;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    const CUE: &str = "PERFORMER \"Tester\"\nTITLE \"Album\"\nFILE \"side.dsf\" WAVE\n\
        TRACK 01 AUDIO\n TITLE \"One\"\n INDEX 01 00:00:00\n\
        TRACK 02 AUDIO\n TITLE \"Two\"\n INDEX 01 03:00:00\n";

    fn fixture() -> std::result::Result<(tempfile::TempDir, Project), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("album.cue"), CUE)?;
        let mut project = Project::default();
        project.add(&dir.path().join("album.cue"))?;
        Ok((dir, project))
    }

    #[test]
    fn edits_survive_a_save_and_load() -> TestResult {
        let (dir, mut project) = fixture()?;
        {
            let entry = project.entries.first_mut().ok_or("no entry")?;
            entry.plan.album.fields.set(FieldKey::GENRE, "Jazz");
            entry
                .plan
                .album
                .fields
                .set(FieldKey::Custom("MOOD".into()), "calm");
            let album = entry.plan.album.clone();
            entry.plan.tracks[1].meta.assign(
                &album,
                FieldKey::Standard(Standard::Conductor),
                "Karajan",
            );
            entry.plan.tracks[1].start_seconds = 181.5;
            entry.plan.naming.file = "{title}".to_owned();
            entry.plan.naming.style = NameStyle::Native;
            entry.plan.cover = Some(CoverPlan {
                file: "art.png".to_owned(),
                embed: false,
                copy_to_folder: true,
            });
            entry.enabled = false;
            entry.output = OutputMode::Custom("/music/out".to_owned());
        }
        let saved_plan = project.entries[0].plan.clone();

        let file = dir.path().join("work.dsfproj");
        save(&project, &file)?;
        assert!(!dir.path().join("work.dsfproj.tmp").exists());

        let mut restored = Project::default();
        let report = load(&mut restored, &file)?;
        assert_eq!(
            (report.added, report.problems.len()),
            (1, 0),
            "{:?}",
            report.problems
        );
        let entry = restored.entries.first().ok_or("no entry")?;
        assert_eq!(entry.plan, saved_plan);
        assert!(!entry.enabled);
        assert_eq!(entry.output, OutputMode::Custom("/music/out".to_owned()));
        assert!(entry.is_modified(), "restored edits still count as edits");
        Ok(())
    }

    #[test]
    fn an_album_made_from_audio_files_round_trips_with_its_cuts() -> TestResult {
        let dir = tempfile::tempdir()?;
        let audio = dir.path().join("live.dsf");
        fs::write(&audio, b"x")?;
        let mut project = Project::default();
        project.add_audio(&[AudioFile {
            path: audio,
            cuts: vec![61.25, 130.5],
        }])?;
        let before = project.entries[0].plan.clone();

        let file = dir.path().join("work.dsfproj");
        save(&project, &file)?;
        assert!(fs::read_to_string(&file)?.contains("live.dsf"));

        let mut restored = Project::default();
        let report = load(&mut restored, &file)?;
        assert_eq!(
            (report.added, report.problems.len()),
            (1, 0),
            "{:?}",
            report.problems
        );
        let entry = restored.entries.first().ok_or("no entry")?;
        assert_eq!(entry.plan, before, "the cuts come back exactly");
        assert!(entry.generated.is_some());
        Ok(())
    }

    #[test]
    fn a_missing_audio_file_is_skipped_with_a_message() -> TestResult {
        let dir = tempfile::tempdir()?;
        let audio = dir.path().join("live.dsf");
        fs::write(&audio, b"x")?;
        let mut project = Project::default();
        project.add_audio(&[AudioFile {
            path: audio.clone(),
            cuts: vec![],
        }])?;
        let file = dir.path().join("work.dsfproj");
        save(&project, &file)?;
        fs::remove_file(&audio)?;

        let mut restored = Project::default();
        let report = load(&mut restored, &file)?;
        assert_eq!(report.added, 0);
        assert!(report.problems.iter().any(|p| p.contains("live.dsf")));
        Ok(())
    }

    #[test]
    fn a_moved_project_finds_its_cue_by_relative_path() -> TestResult {
        let (dir, project) = fixture()?;
        let file = dir.path().join("work.dsfproj");
        save(&project, &file)?;

        // Same folder tree, different absolute location.
        let moved = tempfile::tempdir()?;
        fs::copy(dir.path().join("album.cue"), moved.path().join("album.cue"))?;
        let text = fs::read_to_string(&file)?.replace(
            &dir.path().to_string_lossy().replace('\\', "\\\\"),
            "/nowhere",
        );
        let mut restored = Project::default();
        let report = from_json(&mut restored, &text, moved.path())?;
        assert_eq!(report.added, 1, "{:?}", report.problems);
        Ok(())
    }

    #[test]
    fn missing_cues_are_skipped_but_the_rest_load() -> TestResult {
        let (dir, project) = fixture()?;
        let file = dir.path().join("work.dsfproj");
        save(&project, &file)?;
        fs::remove_file(dir.path().join("album.cue"))?;

        let mut restored = Project::default();
        let report = load(&mut restored, &file)?;
        assert_eq!(report.added, 0);
        assert!(report.problems.iter().any(|p| p.contains("not found")));
        Ok(())
    }

    #[test]
    fn unknown_fields_are_reported_not_fatal() -> TestResult {
        let (dir, project) = fixture()?;
        let file = dir.path().join("work.dsfproj");
        save(&project, &file)?;
        let text = fs::read_to_string(&file)?
            .replace("\"album\":", "\"flux capacitor\": \"x\",\n \"album\":");
        let mut restored = Project::default();
        let report = from_json(&mut restored, &text, dir.path())?;
        assert_eq!(report.added, 1);
        assert!(report.problems.iter().any(|p| p.contains("flux capacitor")));
        Ok(())
    }

    #[test]
    fn garbage_and_future_versions_are_refused() {
        let mut project = Project::default();
        assert!(from_json(&mut project, "not json", Path::new(".")).is_err());
        assert!(from_json(&mut project, "{}", Path::new(".")).is_err());
        let future = format!("{{\"version\": {}, \"albums\": []}}", CURRENT_VERSION + 1);
        let err = from_json(&mut project, &future, Path::new(".")).err();
        assert!(err.is_some_and(|e| e.to_string().contains("newer version")));
    }

    #[test]
    fn loading_the_same_project_twice_does_not_duplicate_albums() -> TestResult {
        let (dir, project) = fixture()?;
        let file = dir.path().join("work.dsfproj");
        save(&project, &file)?;
        let mut restored = Project::default();
        load(&mut restored, &file)?;
        let again = load(&mut restored, &file)?;
        assert_eq!(restored.entries.len(), 1);
        assert_eq!(again.added, 0);
        assert!(
            again
                .problems
                .iter()
                .any(|p| p.contains("already in the list"))
        );
        Ok(())
    }
}
