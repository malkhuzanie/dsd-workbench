mod analysis;
mod audio;
mod cli;
mod cover;
mod cue;
mod cue_write;
mod diag;
mod disk;
mod engine;
mod error;
mod history;
mod locate;
mod lookup;
mod metadata;
mod meter;
mod naming;
mod online;
mod plan;
mod project;
mod project_file;
mod selection;
mod settings;
mod snapshot;
mod source;
mod timecode;
mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    diag::install_panic_hook();
    diag::log(format!("started v{}", env!("CARGO_PKG_VERSION")));

    let args: Vec<String> = std::env::args().skip(1).collect();

    // Any argument selects the CLI; no arguments opens the GUI.
    if !args.is_empty() {
        return cli::run(&args);
    }

    match ui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("failed to start the interface: {err}");
            ExitCode::FAILURE
        }
    }
}
