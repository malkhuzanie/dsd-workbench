use std::io::{self, Read, Write};

use crate::{Error, Result};

const DSD_CHUNK_LEN: u64 = 28;
const FMT_CHUNK_LEN: u64 = 52;
/// Size of the `data` chunk header, which `data_chunk_size` counts along with the audio.
pub const DATA_CHUNK_HEADER_LEN: u64 = 12;

/// Offset of the first audio byte in a DSF file (92).
pub const AUDIO_OFFSET: u64 = DSD_CHUNK_LEN + FMT_CHUNK_LEN + DATA_CHUNK_HEADER_LEN;

/// The fixed-size DSF header: `DSD ` chunk, `fmt ` chunk and `data` chunk header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DsfHeader {
    /// Total file size as recorded in the header.
    pub total_file_size: u64,
    /// Offset of the ID3v2 metadata chunk; `0` means no metadata.
    pub metadata_offset: u64,
    /// Channel type id (e.g. 2 = stereo).
    pub channel_type: u32,
    /// Number of channels.
    pub channel_count: u32,
    /// Sampling frequency in Hz (e.g. 2_822_400).
    pub sample_rate: u32,
    /// 1 (LSB first) or 8 (MSB first).
    pub bits_per_sample: u32,
    /// Samples per channel.
    pub sample_count: u64,
    /// Block size per channel in bytes (4096 per spec).
    pub block_size: u32,
    /// Size of the data chunk *including* its 12-byte header.
    pub data_chunk_size: u64,
}

impl DsfHeader {
    /// Parses and validates a header from the start of `r`.
    pub fn read<R: Read>(mut r: R) -> Result<Self> {
        let r = &mut r;
        magic(r, b"DSD ", "DSD")?;
        if u64_le(r)? != DSD_CHUNK_LEN {
            return Err(Error::Malformed("unexpected DSD chunk size"));
        }
        let total_file_size = u64_le(r)?;
        let metadata_offset = u64_le(r)?;

        magic(r, b"fmt ", "fmt")?;
        if u64_le(r)? != FMT_CHUNK_LEN {
            return Err(Error::Malformed("unexpected fmt chunk size"));
        }
        if u32_le(r)? != 1 {
            return Err(Error::Malformed("unsupported DSF version"));
        }
        if u32_le(r)? != 0 {
            return Err(Error::Malformed("unsupported format id (only raw DSD)"));
        }
        let channel_type = u32_le(r)?;
        let channel_count = u32_le(r)?;
        let sample_rate = u32_le(r)?;
        let bits_per_sample = u32_le(r)?;
        let sample_count = u64_le(r)?;
        let block_size = u32_le(r)?;
        let _reserved = u32_le(r)?;

        magic(r, b"data", "data")?;
        let data_chunk_size = u64_le(r)?;

        let header = Self {
            total_file_size,
            metadata_offset,
            channel_type,
            channel_count,
            sample_rate,
            bits_per_sample,
            sample_count,
            block_size,
            data_chunk_size,
        };
        header.validate()?;
        Ok(header)
    }

    /// Serializes the full 92-byte header (including the `data` chunk header).
    pub fn write<W: Write>(&self, mut w: W) -> Result<()> {
        let mut b = Vec::with_capacity(92);
        b.extend_from_slice(b"DSD ");
        b.extend_from_slice(&DSD_CHUNK_LEN.to_le_bytes());
        b.extend_from_slice(&self.total_file_size.to_le_bytes());
        b.extend_from_slice(&self.metadata_offset.to_le_bytes());
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&FMT_CHUNK_LEN.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes()); // version
        b.extend_from_slice(&0u32.to_le_bytes()); // format id: raw DSD
        b.extend_from_slice(&self.channel_type.to_le_bytes());
        b.extend_from_slice(&self.channel_count.to_le_bytes());
        b.extend_from_slice(&self.sample_rate.to_le_bytes());
        b.extend_from_slice(&self.bits_per_sample.to_le_bytes());
        b.extend_from_slice(&self.sample_count.to_le_bytes());
        b.extend_from_slice(&self.block_size.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // reserved
        b.extend_from_slice(b"data");
        b.extend_from_slice(&self.data_chunk_size.to_le_bytes());
        w.write_all(&b)?;
        Ok(())
    }

    /// Offset one past the last audio byte; where a metadata chunk belongs.
    pub fn audio_end(&self) -> u64 {
        (DSD_CHUNK_LEN + FMT_CHUNK_LEN).saturating_add(self.data_chunk_size)
    }

    fn validate(&self) -> Result<()> {
        if self.channel_count == 0 {
            return Err(Error::Malformed("zero channels"));
        }
        if self.block_size == 0 {
            return Err(Error::Malformed("zero block size"));
        }
        if !matches!(self.bits_per_sample, 1 | 8) {
            return Err(Error::Malformed("bits per sample must be 1 or 8"));
        }
        if self.data_chunk_size < DATA_CHUNK_HEADER_LEN
            || (DSD_CHUNK_LEN + FMT_CHUNK_LEN)
                .checked_add(self.data_chunk_size)
                .is_none()
        {
            return Err(Error::Malformed("invalid data chunk size"));
        }
        Ok(())
    }
}

fn fill<R: Read, const N: usize>(r: &mut R) -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    r.read_exact(&mut buf).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => Error::Malformed("truncated header"),
        _ => Error::Io(e),
    })?;
    Ok(buf)
}

fn u32_le<R: Read>(r: &mut R) -> Result<u32> {
    Ok(u32::from_le_bytes(fill(r)?))
}

fn u64_le<R: Read>(r: &mut R) -> Result<u64> {
    Ok(u64::from_le_bytes(fill(r)?))
}

fn magic<R: Read>(r: &mut R, expected: &[u8; 4], name: &'static str) -> Result<()> {
    if &fill::<R, 4>(r)? == expected {
        Ok(())
    } else {
        Err(Error::BadSignature(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DsfHeader {
        DsfHeader {
            total_file_size: AUDIO_OFFSET + 8192,
            metadata_offset: 0,
            channel_type: 2,
            channel_count: 2,
            sample_rate: 2_822_400,
            bits_per_sample: 1,
            sample_count: 32_768,
            block_size: 4096,
            data_chunk_size: DATA_CHUNK_HEADER_LEN + 8192,
        }
    }

    fn encoded() -> Vec<u8> {
        let mut bytes = Vec::new();
        assert!(sample().write(&mut bytes).is_ok());
        bytes
    }

    #[test]
    fn round_trips() {
        assert_eq!(DsfHeader::read(encoded().as_slice()).ok(), Some(sample()));
    }

    #[test]
    fn every_truncation_is_an_error_not_a_panic() {
        let bytes = encoded();
        for len in 0..bytes.len() {
            let prefix = bytes.get(..len).unwrap_or_default();
            assert!(DsfHeader::read(prefix).is_err(), "prefix of {len} bytes");
        }
    }

    #[test]
    fn every_single_byte_corruption_is_handled() {
        let bytes = encoded();
        for index in 0..bytes.len() {
            for flip in [0x01u8, 0x80, 0xFF] {
                let mut copy = bytes.clone();
                if let Some(byte) = copy.get_mut(index) {
                    *byte ^= flip;
                }
                // Either rejected or accepted; what matters is that it returns.
                let _ = DsfHeader::read(copy.as_slice());
            }
        }
    }

    #[test]
    fn rejects_zero_channels_and_block_size() {
        for mutate in [
            |h: &mut DsfHeader| h.channel_count = 0,
            |h: &mut DsfHeader| h.block_size = 0,
            |h: &mut DsfHeader| h.bits_per_sample = 4,
            |h: &mut DsfHeader| h.data_chunk_size = 3,
        ] {
            let mut header = sample();
            mutate(&mut header);
            let mut bytes = Vec::new();
            assert!(header.write(&mut bytes).is_ok());
            assert!(DsfHeader::read(bytes.as_slice()).is_err());
        }
    }
}
