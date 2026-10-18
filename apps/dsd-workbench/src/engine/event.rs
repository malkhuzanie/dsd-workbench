//! Progress events emitted while a plan is executed, and their plain-text rendering.

use std::fmt;
use std::path::PathBuf;

const SEPARATOR: &str = "---------------------------------------------------------";

/// A description of one track's work, used for progress and dry runs.
#[derive(Debug, Clone)]
pub struct TrackReport {
    pub index: usize,
    pub total: usize,
    pub number: u32,
    pub title: String,
    pub artist: String,
    pub source_file: String,
    pub start_seconds: f64,
    pub duration: Option<f64>,
    pub target: PathBuf,
}

impl TrackReport {
    fn duration_label(&self) -> String {
        match self.duration {
            Some(d) => format!("{d:.2} seconds"),
            None => "until the end of the source file".to_owned(),
        }
    }
}

/// Bytes written so far for the track being split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteProgress {
    /// Zero-based position of the track in the album.
    pub index: usize,
    /// Number of tracks in the album.
    pub total: usize,
    /// Audio bytes written so far.
    pub done: u64,
    /// Audio bytes the track will contain.
    pub len: u64,
    /// Audio bytes of the album's earlier tracks, already written.
    pub album_done: u64,
    /// Audio bytes of the whole album; zero if unknown.
    pub album_total: u64,
}

impl ByteProgress {
    /// Fraction of the album written, by bytes: `0.0..=1.0`.
    pub fn album_fraction(&self) -> Option<f32> {
        (self.album_total > 0).then(|| {
            let done = self.album_done.saturating_add(self.done);
            (done as f64 / self.album_total as f64).clamp(0.0, 1.0) as f32
        })
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Started {
        album: String,
        dry_run: bool,
    },
    Planned(TrackReport),
    Slicing(TrackReport),
    /// Throttled byte-level progress inside the current track.
    Bytes(ByteProgress),
    Finished {
        output_dir: PathBuf,
        dry_run: bool,
    },
}

impl Event {
    /// Progress through the album in `0.0..=1.0`.
    pub fn progress(&self) -> Option<f32> {
        let fraction =
            |index: usize, total: usize, within: f32| (index as f32 + within) / total.max(1) as f32;
        match self {
            Self::Started { .. } => Some(0.0),
            Self::Planned(t) | Self::Slicing(t) => Some(fraction(t.index, t.total, 0.0)),
            Self::Bytes(b) => b.album_fraction().or_else(|| {
                let within = if b.len == 0 {
                    0.0
                } else {
                    (b.done as f64 / b.len as f64) as f32
                };
                Some(fraction(b.index, b.total, within.clamp(0.0, 1.0)))
            }),
            Self::Finished { .. } => Some(1.0),
        }
    }

    /// Whether the event belongs in a scrolling log (byte progress does not).
    pub fn is_log_line(&self) -> bool {
        !matches!(self, Self::Bytes(_))
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Started {
                album,
                dry_run: true,
            } => {
                writeln!(f, "[dry run] Planned cut points:")?;
                writeln!(f, "{SEPARATOR}")?;
                writeln!(f, "Album Title  : {album}")
            }
            Self::Started { album, .. } if album.trim().is_empty() => {
                write!(f, "Splitting (untitled album)")
            }
            Self::Started { album, .. } => write!(f, "Splitting {album}"),
            Self::Planned(t) => {
                writeln!(f, "Track {:02} : {}", t.number, t.title)?;
                writeln!(f, "  ├─ Source file: {}", t.source_file)?;
                writeln!(f, "  ├─ Start Time : {:.2} seconds", t.start_seconds)?;
                writeln!(f, "  ├─ Duration   : {}", t.duration_label())?;
                writeln!(
                    f,
                    "  ├─ Metadata   : [Artist: '{}', Title: '{}', Track: '{}']",
                    t.artist, t.title, t.number
                )?;
                write!(f, "  └─ Target Path: {}", t.target.display())
            }
            Self::Slicing(t) => write!(
                f,
                " -> Track {:02} from {}: {}",
                t.number, t.source_file, t.title
            ),
            Self::Bytes(b) => write!(f, "    {} of {} bytes", b.done, b.len),
            Self::Finished { dry_run: true, .. } => {
                writeln!(f, "{SEPARATOR}")?;
                write!(f, "[dry run] Complete. No changes were made to your disk.")
            }
            Self::Finished { output_dir, .. } => {
                write!(f, "\nDone. Tracks saved to: {}", output_dir.display())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(done: u64, len: u64, album_done: u64, album_total: u64) -> Event {
        Event::Bytes(ByteProgress {
            index: 1,
            total: 4,
            done,
            len,
            album_done,
            album_total,
        })
    }

    #[test]
    fn without_album_totals_progress_interpolates_within_the_track() {
        let event = bytes(50, 100, 0, 0);
        assert_eq!(event.progress(), Some(0.375));
        assert!(!event.is_log_line());
    }

    #[test]
    fn with_album_totals_progress_follows_bytes_not_track_counts() {
        // A tiny first track and a huge second one: byte progress is not 50%.
        let event = bytes(500, 900, 100, 1000);
        assert_eq!(event.progress(), Some(0.6));
        assert_eq!(bytes(1, 1, 999, 1000).progress(), Some(1.0));
    }

    #[test]
    fn progress_is_clamped_when_totals_are_wrong() {
        assert_eq!(bytes(900, 900, 900, 1000).progress(), Some(1.0));
    }

    #[test]
    fn empty_track_does_not_divide_by_zero() {
        let event = Event::Bytes(ByteProgress {
            index: 0,
            total: 2,
            done: 0,
            len: 0,
            album_done: 0,
            album_total: 0,
        });
        assert_eq!(event.progress(), Some(0.0));
    }
}
