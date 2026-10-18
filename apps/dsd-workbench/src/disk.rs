//! Pre-flight disk checks: free space, write permission, path limits.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

const SAFETY_MARGIN_PERCENT: u64 = 3;
/// Windows rejects longer paths unless long paths are enabled.
const WINDOWS_PATH_LIMIT: usize = 259;

#[derive(Debug, Clone)]
pub struct Need {
    pub name: String,
    pub dir: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct Volume {
    pub free: u64,
    pub required: u64,
    pub albums: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DiskReport {
    pub volumes: Vec<Volume>,
    /// Problems that block a run.
    pub errors: Vec<String>,
    /// Things that could not be checked.
    pub notes: Vec<String>,
}

impl DiskReport {
    pub fn required_total(&self) -> u64 {
        self.volumes.iter().map(|v| v.required).sum()
    }

    pub fn lowest_free(&self) -> Option<u64> {
        self.volumes.iter().map(|v| v.free).min()
    }
}

/// The deepest existing folder at or above `path`.
pub fn nearest_existing(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|p| !p.as_os_str().is_empty() && p.is_dir())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

pub fn free_space(path: &Path) -> io::Result<u64> {
    fs2::available_space(nearest_existing(path))
}

pub fn path_too_long(path: &Path) -> bool {
    cfg!(windows) && path.as_os_str().len() > WINDOWS_PATH_LIMIT
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS.get(unit).copied().unwrap_or("B"))
    }
}

/// Groups needs by volume and compares them with the free space.
///
/// Folders that report the same free space are treated as one volume, which is
/// a heuristic that avoids platform-specific device queries.
pub fn assess(needs: &[Need], free: &dyn Fn(&Path) -> io::Result<u64>) -> DiskReport {
    let mut report = DiskReport::default();
    let mut by_free: BTreeMap<u64, Volume> = BTreeMap::new();

    for need in needs {
        match free(&need.dir) {
            Ok(available) => {
                let volume = by_free.entry(available).or_insert_with(|| Volume {
                    free: available,
                    required: 0,
                    albums: Vec::new(),
                });
                volume.required = volume.required.saturating_add(need.bytes);
                volume.albums.push(need.name.clone());
            }
            Err(err) => report.notes.push(format!(
                "Could not read the free space for {}: {err}",
                need.dir.display()
            )),
        }
    }

    for volume in by_free.into_values() {
        let margin = volume.required / 100 * SAFETY_MARGIN_PERCENT;
        if volume.required.saturating_add(margin) > volume.free {
            let who = if volume.albums.len() <= 3 {
                volume.albums.join(", ")
            } else {
                format!("{} albums", volume.albums.len())
            };
            report.errors.push(format!(
                "Not enough free space for {who}: about {} needed, {} free.",
                format_bytes(volume.required),
                format_bytes(volume.free)
            ));
        }
        report.volumes.push(volume);
    }
    report
}

/// Creates and removes a small file to prove the folder is writable.
pub fn check_writable(dir: &Path) -> io::Result<()> {
    let probe = dir.join(format!(".dsd_workbench_write_test_{}", std::process::id()));
    let _ = fs::remove_file(&probe);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    fs::remove_file(&probe)
}

/// Space check plus (optionally) a write probe in every output location.
pub fn preflight(needs: &[Need], probe_writes: bool) -> DiskReport {
    let mut report = assess(needs, &free_space);
    if probe_writes {
        let mut seen: Vec<PathBuf> = Vec::new();
        for need in needs {
            let target = nearest_existing(&need.dir);
            if seen.contains(&target) {
                continue;
            }
            if let Err(err) = check_writable(&target) {
                report
                    .errors
                    .push(format!("Cannot write to {}: {err}", target.display()));
            }
            seen.push(target);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn need(name: &str, dir: &str, bytes: u64) -> Need {
        Need {
            name: name.to_owned(),
            dir: PathBuf::from(dir),
            bytes,
        }
    }

    #[test]
    fn same_free_space_means_same_volume() {
        let free = |_: &Path| -> io::Result<u64> { Ok(1000) };
        let report = assess(&[need("a", "/x/a", 600), need("b", "/x/b", 500)], &free);
        assert_eq!(report.volumes.len(), 1);
        assert_eq!(report.required_total(), 1100);
        assert_eq!(report.errors.len(), 1, "600 + 500 does not fit in 1000");
    }

    #[test]
    fn different_volumes_are_checked_separately() {
        let free = |p: &Path| -> io::Result<u64> {
            Ok(if p.starts_with("/small") { 100 } else { 10_000 })
        };
        let report = assess(
            &[need("a", "/small/a", 500), need("b", "/big/b", 500)],
            &free,
        );
        assert_eq!(report.volumes.len(), 2);
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains('a'));
    }

    #[test]
    fn unreadable_free_space_is_a_note_not_an_error() {
        let free = |_: &Path| -> io::Result<u64> { Err(io::Error::other("nope")) };
        let report = assess(&[need("a", "/x", 1)], &free);
        assert!(report.errors.is_empty());
        assert_eq!(report.notes.len(), 1);
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }
}
