//! Finding and loading the album's cover image.

use std::fs;
use std::path::Path;

use crate::error::{Error, Result};
use crate::locate;

/// File names (without extension) that are taken to be the album cover, best first.
const COVER_NAMES: [&str; 5] = ["cover", "folder", "front", "album", "albumart"];
const EXTENSIONS: [&str; 3] = ["jpg", "jpeg", "png"];
/// Larger images are refused: they would be copied into every track.
pub const MAX_COVER_BYTES: u64 = 16 * 1024 * 1024;

const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The best cover image among `listing` (file names of one folder), if any.
pub fn find_in(listing: &[String]) -> Option<String> {
    COVER_NAMES.iter().find_map(|wanted| {
        let mut hits: Vec<&String> = listing
            .iter()
            .filter(|name| {
                let lower = name.to_lowercase();
                lower
                    .rsplit_once('.')
                    .is_some_and(|(stem, ext)| stem == *wanted && EXTENSIONS.contains(&ext))
            })
            .collect();
        hits.sort();
        hits.first().map(|name| (*name).clone())
    })
}

/// The best cover image in `dir`, as a file name.
pub fn find(dir: &Path) -> Option<String> {
    find_in(&locate::list_files(dir))
}

/// The MIME type of a JPEG or PNG image, judged by its first bytes.
pub fn mime_of(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&JPEG_MAGIC) {
        Some("image/jpeg")
    } else if data.starts_with(&PNG_MAGIC) {
        Some("image/png")
    } else {
        None
    }
}

/// File extension for a MIME type returned by [`mime_of`].
pub fn extension_of(mime: &str) -> &'static str {
    if mime == "image/png" { "png" } else { "jpg" }
}

/// Reads and checks an image file.
pub fn load(path: &Path) -> Result<dsf::Picture> {
    let shown = path.display();
    let size = fs::metadata(path)
        .map_err(Error::io(format!("reading cover image {shown}")))?
        .len();
    if size > MAX_COVER_BYTES {
        return Err(Error::Cover(format!(
            "{shown} is {} MB; covers over {} MB are not embedded",
            size / (1024 * 1024),
            MAX_COVER_BYTES / (1024 * 1024)
        )));
    }
    let data = fs::read(path).map_err(Error::io(format!("reading cover image {shown}")))?;
    let mime = mime_of(&data)
        .ok_or_else(|| Error::Cover(format!("{shown} is not a JPEG or PNG image")))?;
    Ok(dsf::Picture {
        mime_type: mime.to_owned(),
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn well_known_names_are_found_in_priority_order() {
        let files = listing(&["Front.PNG", "notes.txt", "Cover.JPG", "folder.jpg"]);
        assert_eq!(find_in(&files), Some("Cover.JPG".to_owned()));
        assert_eq!(
            find_in(&listing(&["album.jpeg", "front.png"])),
            Some("front.png".to_owned())
        );
    }

    #[test]
    fn unrelated_images_are_ignored() {
        assert_eq!(
            find_in(&listing(&["booklet.jpg", "cover.gif", "cover"])),
            None
        );
        assert_eq!(find_in(&[]), None);
    }

    #[test]
    fn mime_is_judged_by_content_not_name() {
        assert_eq!(mime_of(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(mime_of(&PNG_MAGIC), Some("image/png"));
        assert_eq!(mime_of(b"GIF89a"), None);
        assert_eq!(mime_of(&[]), None);
        assert_eq!(extension_of("image/png"), "png");
        assert_eq!(extension_of("image/jpeg"), "jpg");
    }

    #[test]
    fn load_rejects_non_images_and_accepts_jpeg()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let bad = dir.path().join("cover.jpg");
        fs::write(&bad, b"not an image")?;
        assert!(matches!(load(&bad), Err(Error::Cover(_))));

        let good = dir.path().join("front.jpg");
        fs::write(&good, [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3])?;
        let picture = load(&good)?;
        assert_eq!(picture.mime_type, "image/jpeg");
        assert_eq!(picture.data.len(), 7);

        assert!(load(&dir.path().join("missing.jpg")).is_err());
        Ok(())
    }
}
