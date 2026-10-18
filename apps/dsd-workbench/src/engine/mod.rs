mod bits;
mod copy;
mod event;
mod slice;
mod splitter;
mod tagonly;
mod verify;

pub use event::Event;
pub use splitter::SplitEngine;
pub use tagonly::apply as tag_existing;
pub use verify::{AlbumReport, BUCKETS, TrackCheck, Verdict, VerifyProgress, verify_album};
