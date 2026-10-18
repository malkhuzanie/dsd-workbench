use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Dsf(#[from] dsf::Error),
    #[error("the CUE sheet contains no tracks")]
    NoTracks,
    #[error("invalid split plan: {}", .0.join("; "))]
    InvalidPlan(Vec<String>),
    #[error("cannot slice audio: {0}")]
    Slice(String),
    #[error(
        "{count} target file(s) already exist (first: {first}); enable overwrite to replace them"
    )]
    TargetsExist { count: usize, first: String },
    #[error("split cancelled")]
    Cancelled,
    #[error("the worker thread stopped unexpectedly")]
    WorkerStopped,
    #[error("already in the list")]
    AlreadyAdded,
    #[error(transparent)]
    Dsdiff(#[from] dsdiff::Error),
    #[error("{0} is not a folder")]
    NotAFolder(String),
    #[error("cover art: {0}")]
    Cover(String),
    #[error("unsupported source file: {0}")]
    UnsupportedSource(String),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// What the user can do about this error, if there is a useful suggestion.
    pub fn advice(&self) -> Option<&'static str> {
        match self {
            Self::Dsdiff(dsdiff::Error::Compressed) => Some(
                "DST-compressed files are not supported yet. Convert the file to uncompressed DSD (DSF or DFF) with your ripper or player, then add it again.",
            ),
            Self::Dsdiff(_) | Self::Dsf(_) | Self::UnsupportedSource(_) => {
                Some("The file may be damaged or not a DSD file. Try another copy, or re-rip it.")
            }
            Self::TargetsExist { .. } => {
                Some("Turn on \"Overwrite existing files\", or choose another output folder.")
            }
            Self::InvalidPlan(_) | Self::NoTracks => {
                Some("Open the \"Album & output\" tab to see what needs fixing.")
            }
            Self::NotAFolder(_) => Some("Pick an existing folder."),
            Self::Cover(_) => {
                Some("Choose a JPEG or PNG under 16 MB, or turn cover art off in Settings.")
            }
            Self::Io { source, .. } => match source.kind() {
                io::ErrorKind::PermissionDenied => {
                    Some("Check that you are allowed to write to that folder.")
                }
                io::ErrorKind::NotFound => {
                    Some("The file or folder was moved or deleted. Check the path and try again.")
                }
                io::ErrorKind::StorageFull => {
                    Some("The drive is full. Free some space or choose another output folder.")
                }
                _ => None,
            },
            Self::Slice(_)
            | Self::Cancelled
            | Self::WorkerStopped
            | Self::AlreadyAdded
            | Self::Internal(_) => None,
        }
    }

    /// The message followed by the advice, as one sentence for the user.
    pub fn with_advice(&self) -> String {
        match self.advice() {
            Some(advice) => format!("{self}. {advice}"),
            None => self.to_string(),
        }
    }

    /// Adapter for `map_err` that attaches human-readable context to an I/O error.
    pub fn io(context: impl Into<String>) -> impl FnOnce(io::Error) -> Self {
        let context = context.into();
        move |source| Self::Io { context, source }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io_error(kind: io::ErrorKind) -> Error {
        Error::io("writing the file")(io::Error::from(kind))
    }

    #[test]
    fn dst_files_get_a_clear_instruction() {
        let err = Error::Dsdiff(dsdiff::Error::Compressed);
        let text = err.with_advice();
        assert!(text.contains("DST"), "{text}");
        assert!(
            text.contains("Convert the file to uncompressed DSD"),
            "{text}"
        );
    }

    #[test]
    fn common_disk_problems_say_what_to_do() {
        assert!(
            io_error(io::ErrorKind::PermissionDenied)
                .with_advice()
                .contains("allowed to write")
        );
        assert!(
            io_error(io::ErrorKind::NotFound)
                .with_advice()
                .contains("moved or deleted")
        );
        assert!(
            io_error(io::ErrorKind::StorageFull)
                .with_advice()
                .contains("drive is full")
        );
        assert_eq!(io_error(io::ErrorKind::Other).advice(), None);
    }

    #[test]
    fn existing_files_point_at_the_overwrite_switch() {
        let err = Error::TargetsExist {
            count: 2,
            first: "01.dsf".into(),
        };
        assert!(err.with_advice().contains("Overwrite existing files"));
    }

    #[test]
    fn errors_without_advice_are_unchanged() {
        assert_eq!(Error::Cancelled.with_advice(), "split cancelled");
        assert_eq!(Error::AlreadyAdded.advice(), None);
    }

    #[test]
    fn every_advised_message_is_a_sentence() {
        let samples = [
            Error::Dsdiff(dsdiff::Error::Compressed),
            Error::UnsupportedSource("x".into()),
            Error::InvalidPlan(vec!["bad".into()]),
            Error::NoTracks,
            Error::NotAFolder("x".into()),
            Error::Cover("x".into()),
            Error::TargetsExist {
                count: 1,
                first: "x".into(),
            },
        ];
        for error in samples {
            let advice = error.advice().unwrap_or_default();
            assert!(advice.ends_with('.') && advice.len() > 20, "{error}");
        }
    }
}
