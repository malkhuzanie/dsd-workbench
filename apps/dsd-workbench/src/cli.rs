//! The command line: the batch runner. Reading the arguments is in [`args`].

mod args;

use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use crate::analysis::{self, DetectParams};
use crate::cue_write;
use crate::disk::{self, Need};
use crate::engine::{self, AlbumReport, Event, SplitEngine, Verdict};
use crate::lookup;
use crate::metadata::FieldKey;
use crate::online;
use crate::plan::CoverPlan;
use crate::project::{AudioFile, OutputMode, Project};
use crate::project_file;
use crate::settings::LookupSettings;
use crate::source;
use crate::timecode;
use args::{Args, Parsed, USAGE, parse};
use release_lookup::Target;

/// `args` excludes the program name.
pub fn run(args: &[String]) -> ExitCode {
    let args = match parse(args) {
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Parsed::Run(args)) => args,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match execute(*args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn execute(args: Args) -> Result<ExitCode, String> {
    let mut project = load_albums(&args)?;
    apply_overrides(&mut project, &args)?;
    if args.lookup {
        lookup_albums(&mut project, &args)?;
    }

    if let Some(file) = &args.save_project {
        project_file::save(&project, file)
            .map_err(|e| format!("Could not save the project: {e}"))?;
        println!("Saved project to {}", file.display());
    }
    if let Some(file) = &args.write_cue {
        write_cue(&project, file)?;
    }
    if args.tag_only {
        return Ok(run_tag_only(&project, args.in_dir.as_deref(), args.dry_run));
    }
    run_split(&project, &args)
}

/// Loads every album named on the command line.
fn load_albums(args: &Args) -> Result<Project, String> {
    let mut project = Project::default();
    project.options = args.options.clone();
    let mut failed = false;

    for file in &args.projects {
        match project_file::load(&mut project, file) {
            Ok(report) => report
                .problems
                .iter()
                .for_each(|p| eprintln!("warning: {}: {p}", file.display())),
            Err(err) => {
                eprintln!("Failed to load {}: {}", file.display(), err.with_advice());
                failed = true;
            }
        }
    }
    for cue in &args.cues {
        if let Err(err) = project.add(cue) {
            eprintln!("Failed to load {}: {}", cue.display(), err.with_advice());
            failed = true;
        }
    }
    if !args.audio.is_empty() {
        // Without --auto/--at each file is one track, as in "retag these files".
        for file in audio_files(args)? {
            if let Err(err) = project.add_audio(std::slice::from_ref(&file)) {
                eprintln!(
                    "Failed to load {}: {}",
                    file.path.display(),
                    err.with_advice()
                );
                failed = true;
            }
        }
    }
    if failed || project.entries.is_empty() {
        return Err("Nothing could be loaded.".to_owned());
    }
    Ok(project)
}

/// The audio files with their cut points: from `--at`, from silence detection, or none.
fn audio_files(args: &Args) -> Result<Vec<AudioFile>, String> {
    args.audio
        .iter()
        .map(|path| {
            let cuts = if args.auto {
                detect_cuts(path, &args.detect)?
            } else {
                args.at.clone()
            };
            Ok(AudioFile {
                path: path.clone(),
                cuts,
            })
        })
        .collect()
}

/// Listens to `path` and returns where the tracks begin.
fn detect_cuts(path: &Path, params: &DetectParams) -> Result<Vec<f64>, String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    eprintln!("Analyzing {name} …");
    let mut src = source::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut shown = 0u32;
    let envelope = analysis::analyze(src.as_mut(), &AtomicBool::new(false), &mut |fraction| {
        let percent = (fraction * 100.0) as u32;
        if percent >= shown + 10 {
            shown = percent - percent % 10;
            eprintln!("  {shown}%");
        }
    })
    .map_err(|e| format!("{name}: {e}"))?;

    let cuts = params.cuts(&envelope);
    if cuts.is_empty() {
        eprintln!("  no gaps found: {name} stays one track (try --silence-db or --min-silence)");
    } else {
        let times: Vec<String> = cuts.iter().map(|c| timecode::format(*c)).collect();
        eprintln!(
            "  {} track(s): cuts at {}",
            cuts.len() + 1,
            times.join(", ")
        );
    }
    Ok(cuts)
}

/// `--cover`, `--out`: settings that apply to the albums once they are loaded.
fn apply_overrides(project: &mut Project, args: &Args) -> Result<(), String> {
    if let Some(file) = &args.cover_file {
        // An explicit image replaces whatever was found next to each CUE sheet.
        let file = std::path::absolute(file)
            .map_err(|_| format!("cannot resolve cover image {}", file.display()))?;
        let (embed, copy) = (project.options.embed_cover, project.options.copy_cover);
        for entry in &mut project.entries {
            entry.set_cover(Some(CoverPlan {
                file: file.to_string_lossy().into_owned(),
                embed,
                copy_to_folder: copy,
            }));
        }
    }
    if let Some(out) = &args.out {
        match project.entries.as_mut_slice() {
            [only] => only.output = OutputMode::Custom(out.to_string_lossy().into_owned()),
            _ => project.set_output_under(out),
        }
    }
    Ok(())
}

/// Fills titles and album details from MusicBrainz or Discogs (needs a build with
/// `--features online`).
fn lookup_albums(project: &mut Project, args: &Args) -> Result<(), String> {
    let source = args.lookup_source;
    let token = args
        .discogs_token
        .clone()
        .or_else(|| LookupSettings::default().token());
    for entry in &mut project.entries {
        let name = entry.display_name();
        let artist = entry.plan.album.value(&FieldKey::ALBUM_ARTIST).to_owned();
        let album = entry.plan.album.value(&FieldKey::ALBUM).to_owned();
        let query = args.lookup_query.as_deref().unwrap_or_default();
        let target = Target::from_input(source, query, &artist, &album).ok_or_else(|| {
            format!("{name}: the album has no title to search for; use --lookup-query")
        })?;
        eprintln!("Searching {source} for {} …", target.describe());
        let looked = lookup::lookup(
            &mut entry.plan,
            source,
            token.as_deref(),
            &target,
            &online::get,
            lookup::ApplyOptions::default(),
        )
        .map_err(|e| format!("{name}: {e}"))?;
        if let Some(candidate) = &looked.found.candidate {
            println!("  using {}", candidate.describe());
        }
        let release = &looked.found.release;
        println!(
            "  matched {} — {} ({}); {} title(s) and {} album field(s) filled in",
            release.artist,
            release.title,
            release.date,
            looked.applied.titles_set,
            looked.applied.fields_set
        );
        for note in &looked.applied.notes {
            eprintln!("  note: {note}");
        }
    }
    Ok(())
}

fn write_cue(project: &Project, file: &Path) -> Result<(), String> {
    let [entry] = project.entries.as_slice() else {
        return Err("--write-cue works with exactly one album".to_owned());
    };
    std::fs::write(file, cue_write::to_cue(&entry.plan))
        .map_err(|e| format!("Could not write {}: {e}", file.display()))?;
    println!("Wrote {}", file.display());
    let same_folder = file
        .parent()
        .map(|p| {
            if p.as_os_str().is_empty() {
                Path::new(".")
            } else {
                p
            }
        })
        .is_some_and(|p| {
            std::fs::canonicalize(p).ok() == std::fs::canonicalize(&entry.base_dir).ok()
        });
    if !same_folder {
        eprintln!(
            "note: the CUE sheet names its audio relative to {}; keep it in that folder",
            entry.base_dir.display()
        );
    }
    Ok(())
}

/// Splits (and optionally verifies) every album.
fn run_split(project: &Project, args: &Args) -> Result<ExitCode, String> {
    let report = project.validate();
    if !report.is_empty() {
        for entry in &project.entries {
            if let Some(problems) = report.get(&entry.id) {
                eprintln!("{}:", entry.display_name());
                for problem in problems {
                    eprintln!("  - {problem}");
                }
            }
        }
        return Ok(ExitCode::FAILURE);
    }

    if !args.dry_run && !args.verify_only {
        let needs: Vec<Need> = project
            .entries
            .iter()
            .map(|e| Need {
                name: e.display_name(),
                dir: e.output_dir(),
                bytes: e.estimated_bytes(),
            })
            .collect();
        let disk_report = disk::preflight(&needs, true);
        disk_report
            .notes
            .iter()
            .for_each(|n| eprintln!("note: {n}"));
        if !disk_report.errors.is_empty() {
            disk_report
                .errors
                .iter()
                .for_each(|e| eprintln!("error: {e}"));
            return Ok(ExitCode::FAILURE);
        }
    }

    let cancel = AtomicBool::new(false);
    let mut failures = 0usize;
    for entry in &project.entries {
        println!("== {} ==", entry.display_name());
        entry
            .warnings
            .iter()
            .for_each(|w| eprintln!("warning: {w}"));

        let mut split_ok = true;
        if !args.verify_only {
            let engine = SplitEngine::new(
                entry.base_dir.clone(),
                entry.plan.clone(),
                entry.output_dir(),
                args.dry_run,
            )
            .with_overwrite(args.overwrite);
            let result = engine.execute(&cancel, &mut |event| {
                if event.is_log_line() {
                    println!("{event}");
                    if matches!(event, Event::Planned(_)) {
                        println!();
                    }
                }
            });
            if let Err(err) = result {
                eprintln!(
                    "Execution failure ({}): {}",
                    entry.display_name(),
                    err.with_advice()
                );
                failures += 1;
                split_ok = false;
            }
        }

        if split_ok && (args.verify_only || (args.verify && !args.dry_run)) {
            let result = engine::verify_album(
                &entry.base_dir,
                &entry.plan,
                &entry.output_dir(),
                &cancel,
                &mut |_| {},
            );
            match result {
                Ok(report) if print_verification(&report) => {}
                Ok(_) => failures += 1,
                Err(err) => {
                    eprintln!("Verification failed ({}): {err}", entry.display_name());
                    failures += 1;
                }
            }
        }
    }
    Ok(exit_code(failures))
}

fn exit_code(failures: usize) -> ExitCode {
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Applies each album's tags to files that already exist; nothing is split or read for audio.
fn run_tag_only(project: &Project, in_dir: Option<&Path>, dry_run: bool) -> ExitCode {
    if in_dir.is_some() && project.entries.len() > 1 {
        eprintln!("--in works with one CUE sheet; use --out to name each album's folder");
        return ExitCode::FAILURE;
    }
    let cancel = AtomicBool::new(false);
    let mut failures = 0usize;
    for entry in &project.entries {
        println!("== {} ==", entry.display_name());
        entry
            .warnings
            .iter()
            .for_each(|w| eprintln!("warning: {w}"));
        let dir = in_dir.map_or_else(|| entry.output_dir(), Path::to_path_buf);
        match engine::tag_existing(&entry.plan, &entry.base_dir, &dir, dry_run, &cancel) {
            Ok(report) => {
                for item in &report.items {
                    println!("  {:02} {} - {}", item.number, item.title, item.describe());
                }
                println!("{}: {}", dir.display(), report.summary());
                if report.problems() > 0 {
                    failures += 1;
                }
            }
            Err(err) => {
                eprintln!(
                    "Tagging failed ({}): {}",
                    entry.display_name(),
                    err.with_advice()
                );
                failures += 1;
            }
        }
    }
    exit_code(failures)
}

/// Prints a report; returns true if everything is clean.
fn print_verification(report: &AlbumReport) -> bool {
    println!("Verification ({:.1} s):", report.seconds);
    for track in &report.tracks {
        let status = match &track.verdict {
            Verdict::Identical => "identical".to_owned(),
            Verdict::Differs => format!("DIFFERS ({} differing bits)", track.mismatched_bits()),
            Verdict::Missing => "MISSING".to_owned(),
            Verdict::Unreadable(why) => format!("UNREADABLE: {why}"),
        };
        println!("  {:02} {} - {status}", track.number, track.title);
        if let Some(alignment) = &track.alignment {
            println!("      {}", alignment.describe(track.rate));
        }
        for issue in &track.issues {
            println!("      note: {issue}");
        }
    }
    if report.is_clean() {
        println!(
            "All {} tracks are bit-identical to the source.",
            report.tracks.len()
        );
        true
    } else {
        println!(
            "{} identical, {} differ, {} missing, {} unreadable, {} note(s).",
            report.identical(),
            report.differs(),
            report.missing(),
            report.unreadable(),
            report.issue_count()
        );
        false
    }
}
