use std::io;

/// Errors produced by this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Underlying I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A chunk signature did not match the DSF specification.
    #[error("not a DSF file: bad `{0}` chunk signature")]
    BadSignature(&'static str),
    /// The file violates the DSF layout.
    #[error("malformed DSF: {0}")]
    Malformed(&'static str),
    /// The embedded ID3v2 tag could not be parsed or written.
    #[error("ID3 error: {0}")]
    Id3(#[from] id3::Error),
    /// A frame id was not a valid four-character ID3v2 text frame.
    #[error("invalid text frame id {0:?} (expected e.g. \"TIT2\"; TXXX is not supported)")]
    InvalidFrameId(String),
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, Error>;
