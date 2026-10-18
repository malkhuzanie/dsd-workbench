//! Read-only views of the files currently on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dsf::Tag;

use crate::metadata::{FieldKey, Fields, Standard};

/// The editable fields found in a tag, plus the track number. A missing value is absent
/// from the map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagSnapshot {
    pub fields: Fields,
    /// The raw `TRCK` text, for example `3/12`.
    pub track: String,
    /// Whether the tag carries a picture.
    pub has_cover: bool,
}

impl TagSnapshot {
    pub fn from_tag(tag: &Tag) -> Self {
        let mut fields = Fields::default();
        for standard in Standard::ALL {
            let value = standard.field().read(tag).unwrap_or_default();
            fields.set_or_clear(FieldKey::Standard(standard), &value);
        }
        for (description, value) in dsf::custom_fields(tag) {
            if let Some(key) = FieldKey::custom(&description) {
                fields.set_or_clear(key, &value);
            }
        }
        Self {
            fields,
            track: dsf::read_track(tag)
                .map(|t| t.to_string())
                .unwrap_or_default(),
            has_cover: tag.pictures().next().is_some(),
        }
    }

    /// The value for `key`, or an empty string.
    pub fn value(&self, key: &FieldKey) -> &str {
        self.fields.value(key)
    }
}

/// Outcome of looking at a path on disk.
#[derive(Debug, Clone)]
pub enum Loaded {
    Missing,
    Untagged,
    Tags(TagSnapshot),
    Failed(String),
}

impl Loaded {
    pub fn tags(&self) -> Option<&TagSnapshot> {
        match self {
            Self::Tags(tags) => Some(tags),
            _ => None,
        }
    }
}

pub fn load(path: &Path) -> Loaded {
    if !path.is_file() {
        return Loaded::Missing;
    }
    match dsf::read_path(path) {
        Ok(Some(tag)) => Loaded::Tags(TagSnapshot::from_tag(&tag)),
        Ok(None) => Loaded::Untagged,
        Err(err) => Loaded::Failed(err.to_string()),
    }
}

/// Health and audio facts of a source DSF file referenced by a CUE sheet.
#[derive(Debug, Clone)]
pub enum SourceState {
    Ready {
        duration_secs: f64,
        sample_rate: u32,
        channels: u32,
    },
    Missing,
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub state: SourceState,
    pub tags: Loaded,
}

pub fn probe(path: &Path) -> SourceInfo {
    if !path.is_file() {
        return SourceInfo {
            state: SourceState::Missing,
            tags: Loaded::Missing,
        };
    }
    match crate::source::open(path) {
        Ok(source) => {
            let info = source.info();
            // Only DSF carries ID3 tags this tool reads.
            let tags = if info.container == crate::source::Container::Dsf {
                load(path)
            } else {
                Loaded::Untagged
            };
            SourceInfo {
                state: SourceState::Ready {
                    duration_secs: info.duration_secs(),
                    sample_rate: info.sample_rate,
                    channels: info.channels,
                },
                tags,
            }
        }
        Err(err) => {
            let message = err.with_advice();
            SourceInfo {
                tags: Loaded::Failed(message.clone()),
                state: SourceState::Invalid(message),
            }
        }
    }
}

/// Memoises disk reads so the UI can ask every frame.
#[derive(Debug, Default)]
pub struct TagCache {
    entries: HashMap<PathBuf, Loaded>,
}

impl TagCache {
    pub fn get(&mut self, path: &Path) -> &Loaded {
        self.entries
            .entry(path.to_path_buf())
            .or_insert_with(|| load(path))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
