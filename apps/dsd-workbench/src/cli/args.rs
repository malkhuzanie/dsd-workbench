//! Reading the command line into [`Args`]. Pure: no files are touched.

use std::path::{Path, PathBuf};

use crate::analysis::{CutPlacement, DetectParams};
use crate::locate;
use crate::naming::NameStyle;
use crate::plan::{GapMode, PlanOptions};
use crate::project;
use crate::project_file;
use crate::timecode;
use release_lookup::Source;

pub(super) const USAGE: &str =
    "Usage: dsd-workbench <album.cue | project.dsfproj | audio.dsf>... [options]

Input
  album.cue, project.dsfproj   albums to split (a project keeps its edits)
  audio.dsf, audio.dff         split a file that has no CUE sheet (see --auto, --at)

Where the output goes
  --out DIR          write tracks to DIR (one album) or DIR/<name> (several albums);
                     default: next to each CUE sheet or audio file
  --name TEMPLATE    file name template (default \"{track:02} - {title}\")
  --folder TEMPLATE  sub-folder template below the output folder, e.g. \"{albumartist}/{album}\"
  --native-names     only avoid characters this system forbids (default: safe everywhere)
  --overwrite        replace existing output files
  --dry-run          show what would happen without writing anything

Splitting without a CUE sheet
  --auto             find the silent gaps between tracks and cut there
  --at TIME          add a track starting at TIME (e.g. 3:20.5); repeatable; one audio file
  --silence-db DB    level below which audio counts as silence (default -60)
  --min-silence S    shortest gap, in seconds, that separates tracks (default 1.5)
  --min-track S      never create a track shorter than S seconds (default 10)
  --cut-at WHERE     middle of the gap (default) or before (just ahead of the music)
  --write-cue FILE   write the resulting CUE sheet to FILE (one album)

CUE handling
  --gap MODE         where pregap audio (INDEX 00) goes: append (end of the previous track,
                     default) or prepend (start of the next track)
  --hidden-track     keep audio before the first track as track 00

Online lookup (only in builds made with `--features online`)
  --lookup           fill titles, artist, date, label and catalogue number from a release
                     database, searching by the album's artist and title
  --lookup-query T   search for T instead; or give a release link or id to use that release
                     exactly (implies --lookup; one album)
  --lookup-source S  musicbrainz (default) or discogs
  --discogs-token T  Discogs personal access token (optional; or set DISCOGS_TOKEN)

Cover art
  --cover FILE       embed this JPEG/PNG in every track (default: cover.jpg, folder.jpg,
                     front.png and similar next to the CUE sheet)
  --no-cover         do not embed or copy any cover art
  --copy-cover       also copy the cover into the album folder as folder.jpg

Checking and tagging
  --verify           after splitting, compare every track with the source, bit by bit
  --verify-only      do not split; verify files that already exist
  --tag-only         do not split: write the album's tags (and cover) into existing .dsf
                     files, matched by name or leading track number. Honours --dry-run.
  --in DIR           with --tag-only: the folder holding the files (default: the output folder)
  --save-project F   write the loaded albums, with all options applied, to project file F first

Run without arguments to open the graphical interface.";

/// What the command line asked for.
#[derive(Debug, Default)]
pub(super) struct Args {
    pub(super) cues: Vec<PathBuf>,
    pub(super) projects: Vec<PathBuf>,
    pub(super) audio: Vec<PathBuf>,
    pub(super) out: Option<PathBuf>,
    pub(super) in_dir: Option<PathBuf>,
    pub(super) options: PlanOptions,
    pub(super) cover_file: Option<PathBuf>,
    pub(super) save_project: Option<PathBuf>,
    pub(super) write_cue: Option<PathBuf>,
    pub(super) dry_run: bool,
    pub(super) overwrite: bool,
    pub(super) verify: bool,
    pub(super) verify_only: bool,
    pub(super) tag_only: bool,
    pub(super) auto: bool,
    pub(super) at: Vec<f64>,
    pub(super) detect: DetectParams,
    pub(super) lookup: bool,
    pub(super) lookup_query: Option<String>,
    pub(super) lookup_source: Source,
    pub(super) discogs_token: Option<String>,
}

#[derive(Debug)]
pub(super) enum Parsed {
    Help,
    Run(Box<Args>),
}

pub(super) fn is_project_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(project_file::EXTENSION))
}

/// The value after an option, or a sentence saying what was missing.
fn value<'a>(
    it: &mut impl Iterator<Item = &'a String>,
    option: &str,
    what: &str,
) -> Result<&'a String, String> {
    it.next().ok_or_else(|| format!("{option} needs {what}"))
}

fn number<T: std::str::FromStr>(text: &str, option: &str, what: &str) -> Result<T, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("{option} needs {what}, not '{text}'"))
}

/// Interprets the command line. `args` excludes the program name.
pub(super) fn parse(args: &[String]) -> Result<Parsed, String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(Parsed::Help);
    }
    let mut a = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dry-run" => a.dry_run = true,
            "--overwrite" => a.overwrite = true,
            "--verify" => a.verify = true,
            "--verify-only" => a.verify_only = true,
            "--tag-only" => a.tag_only = true,
            "--auto" => a.auto = true,
            "--lookup" => a.lookup = true,
            "--lookup-query" => {
                a.lookup = true;
                a.lookup_query =
                    Some(value(&mut it, "--lookup-query", "text, a link or an id")?.clone());
            }
            "--lookup-source" => {
                let key = value(&mut it, "--lookup-source", "'musicbrainz' or 'discogs'")?;
                a.lookup_source = Source::from_key(key).ok_or_else(|| {
                    format!("--lookup-source needs 'musicbrainz' or 'discogs', not '{key}'")
                })?;
            }
            "--discogs-token" => {
                a.discogs_token = Some(value(&mut it, "--discogs-token", "a token")?.clone());
            }
            "--hidden-track" => a.options.cue.hidden_track = true,
            "--native-names" => a.options.naming.style = NameStyle::Native,
            "--copy-cover" => a.options.copy_cover = true,
            "--no-cover" => {
                a.options.embed_cover = false;
                a.options.copy_cover = false;
            }
            "--gap" => {
                let mode = value(&mut it, "--gap", "'append' or 'prepend'")?;
                a.options.cue.gap = GapMode::from_key(mode)
                    .ok_or_else(|| format!("--gap needs 'append' or 'prepend', not '{mode}'"))?;
            }
            "--name" => a.options.naming.file = value(&mut it, "--name", "a template")?.clone(),
            "--folder" => {
                a.options.naming.folder = value(&mut it, "--folder", "a template")?.clone();
            }
            "--out" => a.out = Some(value(&mut it, "--out", "a folder")?.into()),
            "--in" => a.in_dir = Some(value(&mut it, "--in", "a folder")?.into()),
            "--cover" => a.cover_file = Some(value(&mut it, "--cover", "an image file")?.into()),
            "--save-project" => {
                a.save_project = Some(value(&mut it, "--save-project", "a file name")?.into());
            }
            "--write-cue" => {
                a.write_cue = Some(value(&mut it, "--write-cue", "a file name")?.into())
            }
            "--at" => {
                let text = value(&mut it, "--at", "a time such as 3:20.5")?;
                let seconds = timecode::parse(text).filter(|s| *s > 0.0).ok_or_else(|| {
                    format!("--at needs a time after 0:00 such as 3:20.5, not '{text}'")
                })?;
                a.at.push(seconds);
            }
            "--silence-db" => {
                let db: f32 = number(
                    value(&mut it, "--silence-db", "a level in dB")?,
                    "--silence-db",
                    "a level in dB such as -60",
                )?;
                if !(db.is_finite() && db < 0.0) {
                    return Err("--silence-db must be below 0 (for example -60)".to_owned());
                }
                a.detect.threshold_db = db;
            }
            "--min-silence" => {
                a.detect.min_silence = positive(&mut it, "--min-silence")?;
            }
            "--min-track" => {
                a.detect.min_track = positive(&mut it, "--min-track")?;
            }
            "--cut-at" => {
                let place = value(&mut it, "--cut-at", "'middle' or 'before'")?;
                a.detect.placement = match place.as_str() {
                    "middle" => CutPlacement::Middle,
                    "before" => CutPlacement::BeforeMusic,
                    _ => {
                        return Err(format!(
                            "--cut-at needs 'middle' or 'before', not '{place}'"
                        ));
                    }
                };
            }
            other if other.starts_with("--") => return Err(format!("unknown option '{other}'")),
            other if project::is_cue(Path::new(other)) => a.cues.push(other.into()),
            other if is_project_file(Path::new(other)) => a.projects.push(other.into()),
            other if locate::is_dsd_path(Path::new(other)) => a.audio.push(other.into()),
            // Legacy form: any other bare argument is the output folder.
            other => a.out = Some(other.into()),
        }
    }

    if a.cues.is_empty() && a.projects.is_empty() && a.audio.is_empty() {
        return Err("no album given".to_owned());
    }
    if (a.auto || !a.at.is_empty()) && a.audio.is_empty() {
        return Err("--auto and --at need an audio file (.dsf or .dff) to cut".to_owned());
    }
    if !a.at.is_empty() && a.audio.len() != 1 {
        return Err("--at works with exactly one audio file".to_owned());
    }
    if a.lookup_query.is_some() && a.cues.len() + a.audio.len() + a.projects.len() != 1 {
        return Err("--lookup-query works with exactly one album".to_owned());
    }
    Ok(Parsed::Run(Box::new(a)))
}

fn positive<'a>(it: &mut impl Iterator<Item = &'a String>, option: &str) -> Result<f64, String> {
    let n: f64 = number(
        value(it, option, "a number of seconds")?,
        option,
        "a number of seconds",
    )?;
    if n.is_finite() && n > 0.0 {
        Ok(n)
    } else {
        Err(format!("{option} must be greater than 0"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(words: &[&str]) -> Result<Box<Args>, String> {
        let owned: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
        match parse(&owned)? {
            Parsed::Run(args) => Ok(args),
            Parsed::Help => Err("help".to_owned()),
        }
    }

    fn err(words: &[&str]) -> String {
        parse_args(words).err().unwrap_or_default()
    }

    #[test]
    fn inputs_are_classified_by_extension() {
        let a = parse_args(&["a.cue", "B.CUE", "p.dsfproj", "x.dsf", "y.DFF", "outdir"]);
        let a = a.expect("valid");
        assert_eq!(a.cues.len(), 2);
        assert_eq!(a.projects, [PathBuf::from("p.dsfproj")]);
        assert_eq!(a.audio.len(), 2);
        assert_eq!(
            a.out,
            Some(PathBuf::from("outdir")),
            "legacy bare output folder"
        );
    }

    #[test]
    fn flags_and_values_are_read() {
        let a = parse_args(&[
            "a.cue",
            "--dry-run",
            "--overwrite",
            "--verify",
            "--tag-only",
            "--hidden-track",
            "--gap",
            "prepend",
            "--name",
            "{title}",
            "--folder",
            "{album}",
            "--native-names",
            "--no-cover",
            "--out",
            "o",
            "--in",
            "i",
            "--cover",
            "c.jpg",
            "--save-project",
            "p.dsfproj",
        ])
        .expect("valid");
        assert!(a.dry_run && a.overwrite && a.verify && a.tag_only);
        assert!(a.options.cue.hidden_track);
        assert_eq!(a.options.cue.gap, GapMode::PrependToNext);
        assert_eq!(a.options.naming.file, "{title}");
        assert_eq!(a.options.naming.folder, "{album}");
        assert_eq!(a.options.naming.style, NameStyle::Native);
        assert!(!a.options.embed_cover && !a.options.copy_cover);
        assert_eq!(a.out, Some("o".into()));
        assert_eq!(a.in_dir, Some("i".into()));
        assert_eq!(a.cover_file, Some("c.jpg".into()));
        assert_eq!(a.save_project, Some("p.dsfproj".into()));
    }

    #[test]
    fn detection_settings_are_validated() {
        let a = parse_args(&[
            "x.dsf",
            "--auto",
            "--silence-db",
            "-55",
            "--min-silence",
            "2",
            "--min-track",
            "30",
            "--cut-at",
            "before",
        ])
        .expect("valid");
        assert!(a.auto);
        assert_eq!(a.detect.threshold_db, -55.0);
        assert_eq!((a.detect.min_silence, a.detect.min_track), (2.0, 30.0));
        assert_eq!(a.detect.placement, CutPlacement::BeforeMusic);

        assert!(err(&["x.dsf", "--silence-db", "5"]).contains("below 0"));
        assert!(err(&["x.dsf", "--silence-db", "loud"]).contains("not 'loud'"));
        assert!(err(&["x.dsf", "--min-silence", "0"]).contains("greater than 0"));
        assert!(err(&["x.dsf", "--min-track", "-4"]).contains("greater than 0"));
        assert!(err(&["x.dsf", "--cut-at", "never"]).contains("'middle' or 'before'"));
    }

    #[test]
    fn manual_cuts_need_one_audio_file_and_a_real_time() {
        let a = parse_args(&["x.dsf", "--at", "1:30", "--at", "3:20.5"]).expect("valid");
        assert_eq!(a.at, [90.0, 200.5]);
        assert!(err(&["x.dsf", "y.dsf", "--at", "1:00"]).contains("exactly one"));
        assert!(err(&["a.cue", "--at", "1:00"]).contains("need an audio file"));
        assert!(err(&["a.cue", "--auto"]).contains("need an audio file"));
        assert!(err(&["x.dsf", "--at", "0"]).contains("after 0:00"));
        assert!(err(&["x.dsf", "--at", "soon"]).contains("after 0:00"));
    }

    #[test]
    fn mistakes_get_a_sentence_not_a_panic() {
        assert!(err(&[]).contains("no album"));
        assert!(err(&["a.cue", "--bogus"]).contains("unknown option '--bogus'"));
        for option in [
            "--out",
            "--in",
            "--cover",
            "--name",
            "--folder",
            "--gap",
            "--save-project",
            "--write-cue",
            "--at",
        ] {
            assert!(err(&["a.cue", option]).contains("needs"), "{option}");
        }
        assert!(err(&["a.cue", "--gap", "sideways"]).contains("'append' or 'prepend'"));
    }

    #[test]
    fn lookup_options_are_validated() {
        let a = parse_args(&["a.cue", "--lookup"]).expect("valid");
        assert!(a.lookup && a.lookup_query.is_none());
        assert_eq!(a.lookup_source, Source::MusicBrainz, "the default source");

        let a = parse_args(&[
            "a.cue",
            "--lookup-query",
            "Blood on the Tracks [1st Press]",
            "--lookup-source",
            "DISCOGS",
            "--discogs-token",
            "tok",
        ])
        .expect("valid");
        assert!(a.lookup, "a query implies a lookup");
        assert_eq!(
            a.lookup_query.as_deref(),
            Some("Blood on the Tracks [1st Press]")
        );
        assert_eq!(a.lookup_source, Source::Discogs);
        assert_eq!(a.discogs_token.as_deref(), Some("tok"));

        assert!(err(&["a.cue", "b.cue", "--lookup-query", "x"]).contains("exactly one"));
        assert!(err(&["a.cue", "--lookup-query"]).contains("needs"));
        assert!(
            err(&["a.cue", "--lookup-source", "spotify"]).contains("'musicbrainz' or 'discogs'")
        );
        assert!(err(&["a.cue", "--discogs-token"]).contains("needs"));
    }

    #[test]
    fn help_wins_over_everything() {
        let owned = vec!["--bogus".to_owned(), "-h".to_owned()];
        assert!(matches!(parse(&owned), Ok(Parsed::Help)));
    }

    #[test]
    fn the_usage_line_separates_the_program_name_from_the_inputs() {
        assert!(USAGE.starts_with("Usage: dsd-workbench <album.cue"));
    }
}
