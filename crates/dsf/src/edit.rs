use std::collections::BTreeMap;

use id3::frame::{Comment, ExtendedText, PictureType};
use id3::{Tag, TagLike, Version};

use crate::{Error, Result};

/// A validated ID3v2 text-frame identifier (`T???`, excluding `TXXX`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId([u8; 4]);

impl FrameId {
    /// Track title (`TIT2`).
    pub const TITLE: Self = Self(*b"TIT2");
    /// Content group / grouping (`TIT1`).
    pub const GROUPING: Self = Self(*b"TIT1");
    /// Lead artist (`TPE1`).
    pub const ARTIST: Self = Self(*b"TPE1");
    /// Album artist (`TPE2`).
    pub const ALBUM_ARTIST: Self = Self(*b"TPE2");
    /// Album (`TALB`).
    pub const ALBUM: Self = Self(*b"TALB");
    /// Composer (`TCOM`).
    pub const COMPOSER: Self = Self(*b"TCOM");
    /// Track number, `"n"` or `"n/total"` (`TRCK`).
    pub const TRACK: Self = Self(*b"TRCK");
    /// Disc number (`TPOS`).
    pub const DISC: Self = Self(*b"TPOS");
    /// Genre (`TCON`).
    pub const GENRE: Self = Self(*b"TCON");
    /// Recording date (`TDRC`).
    pub const RECORDING_DATE: Self = Self(*b"TDRC");
    /// Conductor (`TPE3`).
    pub const CONDUCTOR: Self = Self(*b"TPE3");
    /// Label / publisher (`TPUB`).
    pub const PUBLISHER: Self = Self(*b"TPUB");
    /// Copyright message (`TCOP`).
    pub const COPYRIGHT: Self = Self(*b"TCOP");
    /// International Standard Recording Code (`TSRC`).
    pub const ISRC: Self = Self(*b"TSRC");

    /// Validates an arbitrary frame id string.
    pub fn new(id: &str) -> Result<Self> {
        let bytes: [u8; 4] = id
            .as_bytes()
            .try_into()
            .map_err(|_| Error::InvalidFrameId(id.to_owned()))?;
        let valid = bytes[0] == b'T'
            && bytes != *b"TXXX"
            && bytes
                .iter()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if valid {
            Ok(Self(bytes))
        } else {
            Err(Error::InvalidFrameId(id.to_owned()))
        }
    }

    /// The identifier as a string (always ASCII).
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

/// What to do with a single frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Change {
    /// Leave the frame untouched.
    #[default]
    Keep,
    /// Replace the frame's text.
    Set(String),
    /// Remove the frame.
    Clear,
}

/// A front-cover image to embed (`APIC`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// MIME type, for example `image/jpeg`.
    pub mime_type: String,
    /// The encoded image.
    pub data: Vec<u8>,
}

/// What to do with the front-cover picture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum PictureChange {
    #[default]
    Keep,
    Set(Picture),
    Clear,
}

/// Language written with comments (ID3 uses ISO 639-2 codes).
const COMMENT_LANGUAGE: &str = "eng";

/// A set of per-frame changes. Frames not mentioned are preserved.
#[derive(Debug, Clone, Default)]
pub struct TagEdit {
    frames: BTreeMap<FrameId, Change>,
    user_text: BTreeMap<String, Change>,
    comment: Change,
    picture: PictureChange,
}

impl TagEdit {
    /// An empty edit (a no-op).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a text frame. A blank value is treated as [`Change::Clear`].
    #[must_use]
    pub fn set(mut self, id: FrameId, value: impl Into<String>) -> Self {
        let value = value.into();
        let change = if value.trim().is_empty() {
            Change::Clear
        } else {
            Change::Set(value)
        };
        self.frames.insert(id, change);
        self
    }

    /// Removes a text frame.
    #[must_use]
    pub fn clear(mut self, id: FrameId) -> Self {
        self.frames.insert(id, Change::Clear);
        self
    }

    /// Sets a user-defined text frame (`TXXX`) identified by `description`, for example
    /// `"MOOD"`. A blank value removes it.
    #[must_use]
    pub fn set_user_text(
        mut self,
        description: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let value = value.into();
        let change = if value.trim().is_empty() {
            Change::Clear
        } else {
            Change::Set(value)
        };
        self.user_text.insert(description.into(), change);
        self
    }

    /// Removes the user-defined text frame (`TXXX`) with this description.
    #[must_use]
    pub fn clear_user_text(mut self, description: impl Into<String>) -> Self {
        self.user_text.insert(description.into(), Change::Clear);
        self
    }

    /// Sets the general comment (`COMM` with an empty description). Blank removes it.
    #[must_use]
    pub fn set_comment(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.comment = if text.trim().is_empty() {
            Change::Clear
        } else {
            Change::Set(text)
        };
        self
    }

    /// Embeds `picture` as the front cover, replacing any existing front cover.
    #[must_use]
    pub fn set_cover(mut self, picture: Picture) -> Self {
        self.picture = PictureChange::Set(picture);
        self
    }

    /// Removes the front cover.
    #[must_use]
    pub fn clear_cover(mut self) -> Self {
        self.picture = PictureChange::Clear;
        self
    }

    /// True if applying this edit would change nothing.
    pub fn is_noop(&self) -> bool {
        self.frames.values().all(|c| *c == Change::Keep)
            && self.user_text.values().all(|c| *c == Change::Keep)
            && self.comment == Change::Keep
            && self.picture == PictureChange::Keep
    }

    pub(crate) fn apply(&self, tag: &mut Tag) {
        for (id, change) in &self.frames {
            match change {
                Change::Keep => {}
                Change::Set(text) => tag.set_text(id.as_str(), text.as_str()),
                Change::Clear => {
                    tag.remove(id.as_str());
                }
            }
        }
        for (description, change) in &self.user_text {
            if *change == Change::Keep {
                continue;
            }
            tag.remove_extended_text(Some(description.as_str()), None);
            if let Change::Set(value) = change {
                tag.add_frame(ExtendedText {
                    description: description.clone(),
                    value: value.clone(),
                });
            }
        }
        match &self.comment {
            Change::Keep => {}
            change => {
                tag.remove_comment(Some(""), None);
                if let Change::Set(text) = change {
                    tag.add_frame(Comment {
                        lang: COMMENT_LANGUAGE.to_owned(),
                        description: String::new(),
                        text: text.clone(),
                    });
                }
            }
        }
        match &self.picture {
            PictureChange::Keep => {}
            change => {
                tag.remove_picture_by_type(PictureType::CoverFront);
                if let PictureChange::Set(picture) = change {
                    tag.add_frame(id3::frame::Picture {
                        mime_type: picture.mime_type.clone(),
                        picture_type: PictureType::CoverFront,
                        description: String::new(),
                        data: picture.data.clone(),
                    });
                }
            }
        }
    }
}

/// Options for tag writes.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct WriteOptions {
    /// ID3v2 version to write. `None` preserves the existing tag's version
    /// (new tags default to v2.4).
    pub version: Option<Version>,
}

impl WriteOptions {
    /// Forces a specific ID3v2 version.
    #[must_use]
    pub fn with_version(mut self, version: Version) -> Self {
        self.version = Some(version);
        self
    }
}
