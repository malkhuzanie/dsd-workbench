//! A fallback font for Chinese, Japanese and Korean text.
//!
//! egui's built-in fonts have no CJK glyphs, so such titles would show as boxes. Rather
//! than bundling a 15 MB font, the app borrows one the system already has.

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

use crate::diag;

/// Fonts known to cover CJK, best first, as fixed paths on Windows and macOS.
#[cfg(target_os = "windows")]
const KNOWN_FILES: [&str; 6] = [
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\YuGothM.ttc",
    r"C:\Windows\Fonts\meiryo.ttc",
    r"C:\Windows\Fonts\malgun.ttf",
    r"C:\Windows\Fonts\simsun.ttc",
    r"C:\Windows\Fonts\mingliu.ttc",
];
#[cfg(target_os = "macos")]
const KNOWN_FILES: [&str; 5] = [
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    "/Library/Fonts/Arial Unicode.ttf",
    "/System/Library/Fonts/STHeiti Light.ttc",
];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const KNOWN_FILES: [&str; 0] = [];

/// Folder names to search on systems that keep fonts in many places (Linux, BSD).
const SEARCH_DIRS: [&str; 4] = [
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "/usr/X11R6/lib/X11/fonts",
    "/run/current-system/sw/share/X11/fonts",
];
/// File-name fragments of fonts with CJK coverage, best first (lower case).
const NAME_HINTS: [&str; 9] = [
    "notosanscjk",
    "notosansjp",
    "notosanssc",
    "sourcehansans",
    "wqy-microhei",
    "wqy-zenhei",
    "droidsansfallback",
    "ipagothic",
    "unifont",
];
const FONT_EXTENSIONS: [&str; 4] = ["ttc", "otc", "ttf", "otf"];
const MAX_DEPTH: usize = 6;
const MAX_FONT_BYTES: u64 = 80 * 1024 * 1024;

/// All font files below `dir`, to a limited depth, in a stable order.
fn font_files(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            font_files(&path, depth + 1, found);
        } else if path
            .extension()
            .is_some_and(|e| FONT_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        {
            found.push(path);
        }
    }
}

/// The preferred CJK-capable font among `files`, judged by its file name.
pub fn pick_cjk(files: &[PathBuf]) -> Option<&PathBuf> {
    NAME_HINTS.iter().find_map(|hint| {
        files.iter().find(|path| {
            path.file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase().contains(hint))
        })
    })
}

/// Looks for a CJK font in `dirs` (searched recursively).
pub fn find_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    let mut files = Vec::new();
    for dir in dirs {
        font_files(dir, 0, &mut files);
    }
    pick_cjk(&files).cloned()
}

/// The system's CJK font, if it has one.
fn find_system_font() -> Option<PathBuf> {
    if let Some(known) = KNOWN_FILES
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
    {
        return Some(known);
    }
    let mut dirs: Vec<PathBuf> = SEARCH_DIRS.iter().map(PathBuf::from).collect();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    find_in(&dirs)
}

/// Reads a font file unless it is implausibly large.
fn read_font(path: &Path) -> Option<Vec<u8>> {
    let size = fs::metadata(path).ok()?.len();
    (size <= MAX_FONT_BYTES)
        .then(|| fs::read(path).ok())
        .flatten()
}

/// Makes CJK text readable by adding a system font after egui's own. Does nothing when
/// the system has no suitable font.
pub fn install_fallbacks(ctx: &egui::Context) {
    let Some(path) = find_system_font() else {
        diag::log("no CJK font found; Chinese, Japanese and Korean text may show as boxes");
        return;
    };
    let Some(bytes) = read_font(&path) else {
        diag::log(format!("could not read the CJK font {}", path.display()));
        return;
    };

    let mut fonts = FontDefinitions::default();
    let name = "system-cjk".to_owned();
    fonts.font_data.insert(
        name.clone(),
        std::sync::Arc::new(FontData::from_owned(bytes)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push(name.clone());
    }
    ctx.set_fonts(fonts);
    diag::log(format!("CJK fallback font: {}", path.display()));
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn touch(root: &Path, relative: &str) -> TestResult {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, b"font")?;
        Ok(())
    }

    #[test]
    fn the_best_named_font_wins() {
        let files: Vec<PathBuf> = [
            "/f/DroidSansFallback.ttf",
            "/f/NotoSansCJK-Regular.ttc",
            "/f/DejaVuSans.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(
            pick_cjk(&files),
            Some(&PathBuf::from("/f/NotoSansCJK-Regular.ttc"))
        );
        assert_eq!(pick_cjk(&[PathBuf::from("/f/DejaVuSans.ttf")]), None);
        assert_eq!(pick_cjk(&[]), None);
    }

    #[test]
    fn fonts_are_found_in_nested_folders_and_other_files_ignored() -> TestResult {
        let dir = tempfile::tempdir()?;
        touch(dir.path(), "truetype/dejavu/DejaVuSans.ttf")?;
        touch(dir.path(), "opentype/noto/NotoSansCJK-Regular.ttc")?;
        touch(dir.path(), "opentype/noto/NotoSansCJK-Regular.txt")?;
        let found = find_in(&[dir.path().to_path_buf()]);
        assert_eq!(
            found.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
            Some("NotoSansCJK-Regular.ttc".to_owned())
        );
        Ok(())
    }

    #[test]
    fn missing_folders_and_no_match_are_fine() -> TestResult {
        let dir = tempfile::tempdir()?;
        touch(dir.path(), "plain/Arial.ttf")?;
        assert_eq!(find_in(&[dir.path().to_path_buf()]), None);
        assert_eq!(find_in(&[PathBuf::from("/definitely/not/here")]), None);
        assert_eq!(find_in(&[]), None);
        Ok(())
    }

    #[test]
    fn oversized_fonts_are_not_read() -> TestResult {
        let dir = tempfile::tempdir()?;
        touch(dir.path(), "tiny.ttf")?;
        assert!(read_font(&dir.path().join("tiny.ttf")).is_some());
        assert!(read_font(&dir.path().join("missing.ttf")).is_none());
        Ok(())
    }
}
