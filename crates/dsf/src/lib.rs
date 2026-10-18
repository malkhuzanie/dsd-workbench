//! Read and write DSF files: header parsing and ID3v2 metadata.
//!
//! Audio payload bytes are never rewritten when editing tags.
#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

mod edit;
mod error;
mod field;
mod header;
mod storage;
mod tag;

pub use edit::{Change, FrameId, Picture, TagEdit, WriteOptions};
pub use error::{Error, Result};
pub use field::{Field, Target, Track, custom_fields, read_track, text, text_frames};
pub use header::{AUDIO_OFFSET, DsfHeader};
pub use storage::Storage;
pub use tag::{read_path, read_tag, remove_tag, update_path, update_tag, write_tag};

/// Re-exported so callers use exactly the `id3` version this crate was built with.
pub use id3::{self, Tag, Version};
