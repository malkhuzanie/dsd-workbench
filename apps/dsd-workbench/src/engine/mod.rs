mod bits;
mod copy;
mod event;
mod slice;
mod splitter;
mod tagcheck;
mod tagonly;
mod verify;

pub(crate) use slice::{OUT_BLOCK_SIZE, audio_bytes};

pub use event::Event;
pub use splitter::SplitEngine;
pub use tagcheck::TagChange;
pub use tagonly::{Item, Outcome, Report, apply as tag_existing};
pub use verify::{AlbumReport, BUCKETS, TrackCheck, Verdict, VerifyProgress, verify_album};
