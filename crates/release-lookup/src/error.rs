use std::fmt;

use crate::Source;

/// Why a lookup failed. `Display` gives a sentence a person can read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// There was nothing to search for: the artist, album and text were all blank.
    NothingToSearch,
    /// The request could not be made. The text is the [`Fetch`](crate::Fetch) function's own
    /// message (the `http` feature's client writes a sentence for the user).
    Fetch(String),
    /// The service answered with something that is not the expected JSON.
    UnexpectedReply {
        /// Which service answered.
        source: Source,
        /// What was wrong with the reply.
        detail: String,
    },
    /// The search found no release.
    NoMatch {
        /// The service that was searched.
        source: Source,
        /// What was searched for.
        wanted: String,
        /// The simplified search that was also tried, if there was one.
        also_tried: Option<String>,
    },
    /// The text is not a release id for the service.
    BadId {
        /// The service the id was meant for.
        source: Source,
        /// The id as given.
        id: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToSearch => f.write_str("there is nothing to search for"),
            Self::Fetch(message) => f.write_str(message),
            Self::UnexpectedReply { source, detail } => {
                write!(f, "unexpected reply from {source}: {detail}")
            }
            Self::NoMatch {
                source,
                wanted,
                also_tried: None,
            } => write!(f, "{source} has no release matching {wanted}"),
            Self::NoMatch {
                source,
                wanted,
                also_tried: Some(other),
            } => write!(
                f,
                "{source} has no release matching {wanted} (also tried {other})"
            ),
            Self::BadId { source, id } => write!(f, "'{id}' is not a {source} release id"),
        }
    }
}

impl std::error::Error for Error {}
