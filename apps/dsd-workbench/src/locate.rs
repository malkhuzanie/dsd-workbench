//! Finds the DSD file a CUE `FILE` line really means.
//!
//! Rips are usually tagged by a different tool than the one that wrote the
//! audio, so the sheet often says `Side A.wav` while the folder holds
//! `side a.dsf`. Matching is forgiving, but never guesses between candidates of
//! different names.

use std::fs;
use std::path::Path;

/// Extensions the tool can read, in order of preference.
const DSD_EXTENSIONS: [&str; 2] = ["dsf", "dff"];

/// How a requested name was matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// The file exists under exactly this name.
    Exact,
    /// Same name, different letter case.
    Case,
    /// Same name with a DSD extension instead (for example `.wav` -> `.dsf`).
    Extension,
    /// Both of the above.
    CaseAndExtension,
}

/// A file found for a requested name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// Path relative to the CUE sheet's folder.
    pub name: String,
    pub how: Match,
}

/// Last path component, accepting both `/` and `\` as separators.
fn basename(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, ext),
        _ => (name, ""),
    }
}

fn is_dsd_extension(ext: &str) -> bool {
    DSD_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether `path` names a DSD file (`.dsf` or `.dff`, in any case).
pub fn is_dsd_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(is_dsd_extension)
}

/// Matches `requested` against the file names in one folder.
pub fn match_listing(listing: &[String], requested: &str) -> Option<Located> {
    let wanted = basename(requested);
    let found = |name: &str, how| {
        Some(Located {
            name: name.to_owned(),
            how,
        })
    };

    if let Some(name) = listing.iter().find(|n| n.as_str() == wanted) {
        return found(name, Match::Exact);
    }
    // Sorted once, so that ties resolve the same way on every platform.
    let mut names: Vec<&String> = listing.iter().collect();
    names.sort();
    let lower = wanted.to_lowercase();
    if let Some(name) = names.iter().find(|n| n.to_lowercase() == lower) {
        return found(name, Match::Case);
    }

    let (stem, _) = split_extension(wanted);
    let stem_lower = stem.to_lowercase();
    // Prefer `.dsf` over `.dff`, exact-case stems over case-insensitive ones.
    for exact_case in [true, false] {
        for wanted_ext in DSD_EXTENSIONS {
            let hit = names.iter().find(|n| {
                let (s, e) = split_extension(n);
                e.eq_ignore_ascii_case(wanted_ext)
                    && if exact_case {
                        s == stem
                    } else {
                        s.to_lowercase() == stem_lower
                    }
            });
            if let Some(name) = hit {
                let how = if exact_case {
                    Match::Extension
                } else {
                    Match::CaseAndExtension
                };
                return found(name, how);
            }
        }
    }
    None
}

/// DSD files in a listing, for suggestions.
pub fn dsd_files(listing: &[String]) -> Vec<String> {
    let mut files: Vec<String> = listing
        .iter()
        .filter(|n| is_dsd_extension(split_extension(n).1))
        .cloned()
        .collect();
    files.sort();
    files
}

/// Names of the regular files directly inside `dir` (empty if it cannot be read).
pub fn list_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Resolves a `FILE` name relative to `base`.
///
/// A relative path that exists is used as written (with `\` read as `/`).
/// Otherwise the name's last component is matched against the files in `base`.
pub fn resolve(base: &Path, requested: &str) -> Option<Located> {
    let normalised = requested.replace('\\', "/");
    if base.join(&normalised).is_file() {
        return Some(Located {
            name: normalised,
            how: Match::Exact,
        });
    }
    match_listing(&list_files(base), requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    fn hit(names: &[&str], requested: &str) -> Option<(String, Match)> {
        match_listing(&listing(names), requested).map(|l| (l.name, l.how))
    }

    #[test]
    fn exact_name_wins() {
        let got = hit(&["Side A.dsf", "side a.dsf"], "Side A.dsf");
        assert_eq!(got, Some(("Side A.dsf".into(), Match::Exact)));
    }

    #[test]
    fn case_differences_are_forgiven() {
        let got = hit(&["SIDE A.DSF"], "side a.dsf");
        assert_eq!(got, Some(("SIDE A.DSF".into(), Match::Case)));
    }

    #[test]
    fn audio_extensions_are_swapped_for_dsd() {
        for requested in ["Side A.wav", "Side A.flac", "Side A.wv", "Side A"] {
            let got = hit(&["Side A.dsf", "cover.jpg"], requested);
            assert_eq!(
                got,
                Some(("Side A.dsf".into(), Match::Extension)),
                "{requested}"
            );
        }
    }

    #[test]
    fn case_and_extension_combine() {
        let got = hit(&["side a.DSF"], "Side A.wav");
        assert_eq!(got, Some(("side a.DSF".into(), Match::CaseAndExtension)));
    }

    #[test]
    fn dsf_is_preferred_over_dff() {
        let got = hit(&["x.dff", "x.dsf"], "x.wav");
        assert_eq!(got, Some(("x.dsf".into(), Match::Extension)));
    }

    #[test]
    fn windows_paths_reduce_to_the_file_name() {
        let got = hit(&["Side A.dsf"], r"C:\Rips\Album\Side A.wav");
        assert_eq!(got, Some(("Side A.dsf".into(), Match::Extension)));
    }

    #[test]
    fn different_names_are_never_guessed() {
        assert_eq!(hit(&["other.dsf"], "Side A.wav"), None);
        assert_eq!(hit(&[], "Side A.wav"), None);
    }

    #[test]
    fn dotfiles_and_extensionless_names_do_not_panic() {
        assert_eq!(hit(&[".dsf"], ".dsf"), Some((".dsf".into(), Match::Exact)));
        assert_eq!(hit(&["a.dsf"], ""), None);
    }

    #[test]
    fn dsd_files_are_listed_sorted() {
        let files = dsd_files(&listing(&["b.DFF", "a.dsf", "c.txt"]));
        assert_eq!(files, ["a.dsf", "b.DFF"]);
    }

    #[test]
    fn resolve_uses_the_real_folder() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("Side A.dsf"), b"x")?;
        fs::create_dir(dir.path().join("sub"))?;
        fs::write(dir.path().join("sub").join("b.dsf"), b"x")?;

        let swapped = resolve(dir.path(), "side a.wav");
        assert_eq!(swapped.map(|l| l.name), Some("Side A.dsf".to_owned()));
        let nested = resolve(dir.path(), r"sub\b.dsf");
        assert_eq!(nested.map(|l| l.name), Some("sub/b.dsf".to_owned()));
        assert_eq!(resolve(dir.path(), "missing.wav"), None);
        Ok(())
    }
}
