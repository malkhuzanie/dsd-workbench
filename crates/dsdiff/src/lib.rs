//! Parse the header of DSDIFF (`.dff`) files: sample rate, channel count and where the audio is.
//!
//! Supported audio is uncompressed DSD, stored interleaved one byte per channel with the
//! most significant bit first in time. DST-compressed files give [`Error::Compressed`].
#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use std::io::{self, Read, Seek, SeekFrom};

const CHUNK_HEADER_LEN: u64 = 12;
const MAX_CHANNELS: u32 = 64;

/// Errors produced by this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Underlying I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// The file does not start with a DSDIFF `FRM8` header.
    #[error("not a DSDIFF file")]
    NotDsdiff,
    /// The file violates the DSDIFF layout.
    #[error("malformed DSDIFF: {0}")]
    Malformed(&'static str),
    /// The audio is DST-compressed, which is not supported.
    #[error("DST-compressed DSDIFF is not supported")]
    Compressed,
    /// A required chunk is missing.
    #[error("DSDIFF file has no {0}")]
    Missing(&'static str),
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Facts about the audio in a DSDIFF file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Sampling frequency in Hz (for example 2_822_400).
    pub sample_rate: u32,
    /// Number of channels.
    pub channel_count: u32,
    /// Samples per channel.
    pub sample_count: u64,
    /// File offset of the first audio byte.
    pub audio_offset: u64,
    /// Audio size in bytes (all channels).
    pub audio_len: u64,
}

struct Chunk {
    id: [u8; 4],
    start: u64,
    len: u64,
}

impl Chunk {
    fn end(&self) -> u64 {
        self.start + self.len
    }
    /// Chunks are padded to an even length.
    fn next(&self) -> u64 {
        self.end() + (self.len & 1)
    }
}

fn fill<R: Read, const N: usize>(r: &mut R) -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    r.read_exact(&mut buf).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => Error::Malformed("truncated file"),
        _ => Error::Io(e),
    })?;
    Ok(buf)
}

fn u16_be<R: Read>(r: &mut R) -> Result<u16> {
    Ok(u16::from_be_bytes(fill(r)?))
}
fn u32_be<R: Read>(r: &mut R) -> Result<u32> {
    Ok(u32::from_be_bytes(fill(r)?))
}
fn u64_be<R: Read>(r: &mut R) -> Result<u64> {
    Ok(u64::from_be_bytes(fill(r)?))
}

/// Reads the chunk header at the current position, or `None` if no room is left before `limit`.
fn chunk_header<R: Read + Seek>(r: &mut R, limit: u64) -> Result<Option<Chunk>> {
    let pos = r.stream_position()?;
    if limit.saturating_sub(pos) < CHUNK_HEADER_LEN {
        return Ok(None);
    }
    let id = fill::<_, 4>(r)?;
    let len = u64_be(r)?;
    let start = pos + CHUNK_HEADER_LEN;
    let end = start
        .checked_add(len)
        .ok_or(Error::Malformed("chunk size overflows"))?;
    if end > limit {
        return Err(Error::Malformed(
            "chunk extends past the end of its container",
        ));
    }
    Ok(Some(Chunk { id, start, len }))
}

fn need(chunk: &Chunk, bytes: u64) -> Result<()> {
    if chunk.len < bytes {
        Err(Error::Malformed("chunk is too short"))
    } else {
        Ok(())
    }
}

fn parse_prop<R: Read + Seek>(
    r: &mut R,
    prop: &Chunk,
    rate: &mut Option<u32>,
    channels: &mut Option<u32>,
) -> Result<()> {
    r.seek(SeekFrom::Start(prop.start))?;
    need(prop, 4)?;
    if fill::<_, 4>(r)? != *b"SND " {
        return Ok(());
    }
    while let Some(chunk) = chunk_header(r, prop.end())? {
        match &chunk.id {
            b"FS  " => {
                need(&chunk, 4)?;
                *rate = Some(u32_be(r)?);
            }
            b"CHNL" => {
                need(&chunk, 2)?;
                *channels = Some(u32::from(u16_be(r)?));
            }
            b"CMPR" => {
                need(&chunk, 4)?;
                if fill::<_, 4>(r)? != *b"DSD " {
                    return Err(Error::Compressed);
                }
            }
            _ => {}
        }
        r.seek(SeekFrom::Start(chunk.next()))?;
    }
    Ok(())
}

/// Parses the header of a DSDIFF file.
pub fn read_info<R: Read + Seek>(r: &mut R) -> Result<Info> {
    let file_len = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    if fill::<_, 4>(r).map_err(|_| Error::NotDsdiff)? != *b"FRM8" {
        return Err(Error::NotDsdiff);
    }
    let _form_len = u64_be(r)?; // often unreliable, so the real file length is used instead
    if fill::<_, 4>(r)? != *b"DSD " {
        return Err(Error::NotDsdiff);
    }

    let (mut rate, mut channels, mut audio) = (None, None, None);
    while let Some(chunk) = chunk_header(r, file_len)? {
        match &chunk.id {
            b"PROP" => parse_prop(r, &chunk, &mut rate, &mut channels)?,
            b"DSD " => {
                audio = Some((chunk.start, chunk.len));
                break;
            }
            b"DST " => return Err(Error::Compressed),
            _ => {}
        }
        r.seek(SeekFrom::Start(chunk.next()))?;
    }

    let sample_rate = rate.ok_or(Error::Missing("sampling frequency (FS chunk)"))?;
    let channel_count = channels.ok_or(Error::Missing("channel layout (CHNL chunk)"))?;
    let (audio_offset, audio_len) = audio.ok_or(Error::Missing("DSD sound data chunk"))?;
    if sample_rate == 0 || channel_count == 0 || channel_count > MAX_CHANNELS {
        return Err(Error::Malformed("invalid sample rate or channel count"));
    }
    let per_channel = audio_len / u64::from(channel_count);
    if per_channel * u64::from(channel_count) != audio_len {
        return Err(Error::Malformed(
            "audio size is not a multiple of the channel count",
        ));
    }
    Ok(Info {
        sample_rate,
        channel_count,
        sample_count: per_channel.saturating_mul(8),
        audio_offset,
        audio_len,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend((data.len() as u64).to_be_bytes());
        v.extend(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn file(cmpr: &[u8; 4], audio: &[u8]) -> Vec<u8> {
        let mut chnl = 2u16.to_be_bytes().to_vec();
        chnl.extend(b"SLFTSRGT");
        let mut cmpr_data = cmpr.to_vec();
        cmpr_data.push(14);
        cmpr_data.extend(b"not compressed");

        let mut prop = b"SND ".to_vec();
        prop.extend(chunk(b"FS  ", &2_822_400u32.to_be_bytes()));
        prop.extend(chunk(b"CHNL", &chnl));
        prop.extend(chunk(b"CMPR", &cmpr_data));

        let mut body = b"DSD ".to_vec();
        body.extend(chunk(b"FVER", &[1, 5, 0, 0]));
        body.extend(chunk(b"PROP", &prop));
        body.extend(chunk(b"DSD ", audio));

        let mut out = b"FRM8".to_vec();
        out.extend((body.len() as u64).to_be_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn reads_format_and_audio_location() -> TestResult {
        let audio = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let bytes = file(b"DSD ", &audio);
        let info = read_info(&mut Cursor::new(&bytes))?;
        assert_eq!(info.sample_rate, 2_822_400);
        assert_eq!(info.channel_count, 2);
        assert_eq!(info.sample_count, 32);
        let from = usize::try_from(info.audio_offset)?;
        assert_eq!(bytes.get(from..from + 8), Some(&audio[..]));
        Ok(())
    }

    #[test]
    fn rejects_compressed_foreign_and_truncated() {
        let dst = file(b"DST ", &[0; 8]);
        assert!(matches!(
            read_info(&mut Cursor::new(dst)),
            Err(Error::Compressed)
        ));
        assert!(matches!(
            read_info(&mut Cursor::new(b"RIFF....".to_vec())),
            Err(Error::NotDsdiff)
        ));
        let mut cut = file(b"DSD ", &[0; 8]);
        cut.truncate(cut.len() - 3);
        assert!(matches!(
            read_info(&mut Cursor::new(cut)),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn every_prefix_and_byte_flip_returns_without_panicking() {
        let bytes = file(b"DSD ", &[0x55; 16]);
        for len in 0..bytes.len() {
            let prefix = bytes.get(..len).unwrap_or_default().to_vec();
            let _ = read_info(&mut Cursor::new(prefix));
        }
        for index in 0..bytes.len() {
            for flip in [0x01u8, 0x80, 0xFF] {
                let mut copy = bytes.clone();
                if let Some(byte) = copy.get_mut(index) {
                    *byte ^= flip;
                }
                if let Ok(info) = read_info(&mut Cursor::new(copy.clone())) {
                    assert!(info.channel_count > 0);
                    assert!(info.audio_offset + info.audio_len <= copy.len() as u64);
                }
            }
        }
    }
}
