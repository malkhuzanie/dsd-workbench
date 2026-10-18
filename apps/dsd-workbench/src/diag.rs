//! Log ring buffer, crash reports and diagnostics.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const APP_DIR: &str = "dsd-workbench";
const MAX_LOG_LINES: usize = 500;
const REPORT_LOG_LINES: usize = 200;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static LAST_CRASH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn clock() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day = secs % 86_400;
    format!(
        "{:02}:{:02}:{:02}Z",
        day / 3600,
        (day % 3600) / 60,
        day % 60
    )
}

/// Appends a line to the in-memory log used by crash and diagnostic reports.
pub fn log(line: impl AsRef<str>) {
    let entry = format!("[{}] {}", clock(), line.as_ref());
    if let Ok(mut log) = LOG.lock() {
        if log.len() >= MAX_LOG_LINES {
            log.pop_front();
        }
        log.push_back(entry);
    }
}

fn recent(count: usize) -> Vec<String> {
    LOG.lock()
        .map(|log| {
            let skip = log.len().saturating_sub(count);
            log.iter().skip(skip).cloned().collect()
        })
        .unwrap_or_default()
}

/// Folder for crash reports and logs.
pub fn dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR)
}

pub fn ensure_dir() -> PathBuf {
    let dir = dir();
    let _ = fs::create_dir_all(&dir);
    dir
}

fn header() -> String {
    format!(
        "DSD Workbench {} on {}/{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn crash_text(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned());
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown location".to_owned());
    let thread = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_owned();
    let backtrace = std::backtrace::Backtrace::force_capture();
    format!(
        "{}\nPanic in thread '{thread}' at {location}\n{message}\n\nBacktrace:\n{backtrace}\n\nRecent log:\n{}\n",
        header(),
        recent(REPORT_LOG_LINES).join("\n")
    )
}

fn write_crash(text: &str) -> Option<PathBuf> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = ensure_dir().join(format!("crash-{millis}.log"));
    fs::write(&path, text).ok().map(|()| path)
}

/// Writes a crash report for every panic, then runs the previous hook.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = crash_text(info);
        if let Some(path) = write_crash(&text) {
            eprintln!("A crash report was saved to {}", path.display());
            if let Ok(mut slot) = LAST_CRASH.lock() {
                *slot = Some(path);
            }
        }
        previous(info);
    }));
}

/// The crash report written by this session, if any (returned once).
pub fn take_crash() -> Option<PathBuf> {
    LAST_CRASH.lock().ok().and_then(|mut slot| slot.take())
}

/// Crash reports from earlier sessions that were not yet acknowledged.
pub fn pending_crash_logs() -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir()) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".log"))
        })
        .collect();
    found.sort();
    found
}

/// Renames a report to `*.seen` so it is not announced again.
pub fn mark_seen(path: &Path) -> io::Result<()> {
    let mut seen = path.as_os_str().to_owned();
    seen.push(".seen");
    fs::rename(path, seen)
}

/// Opens a folder in the system file manager.
pub fn reveal(path: &Path) -> io::Result<()> {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program).arg(path).spawn().map(|_| ())
}

/// A text block for bug reports.
pub fn report(settings: &str) -> String {
    format!(
        "{}\n\nSettings:\n{settings}\nLog folder: {}\nUnacknowledged crash reports: {}\n\nRecent log:\n{}\n",
        header(),
        dir().display(),
        pending_crash_logs().len(),
        recent(REPORT_LOG_LINES).join("\n")
    )
}
