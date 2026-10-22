//! Uniform access to DSD audio, whatever the container.

mod dff_file;
mod dsf_file;

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub use dff_file::DffSource;
pub use dsf_file::DsfSource;

use crate::error::{Error, Result};

/// Idle DSD pattern: equal numbers of ones and zeros, so it decodes to silence.
pub const PAD_BYTE: u8 = 0x69;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Dsf,
    Dsdiff,
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub container: Container,
    pub sample_rate: u32,
    pub channels: u32,
    /// DSF channel-type id for the output header.
    pub channel_type: u32,
    /// Samples per channel.
    pub sample_count: u64,
    /// True if the first sample in time is the most significant bit of a byte.
    pub msb_first: bool,
}

impl StreamInfo {
    pub fn duration_secs(&self) -> f64 {
        self.sample_count as f64 / f64::from(self.sample_rate.max(1))
    }
}

/// A DSD stream addressed as one byte stream per channel (8 samples per byte).
pub trait DsdSource: Send {
    fn info(&self) -> &StreamInfo;

    /// `len` bytes of every channel starting at byte `first` of the channel stream.
    /// Positions before the start or after the end read as [`PAD_BYTE`].
    fn read(&mut self, first: i64, len: usize) -> io::Result<Vec<Vec<u8>>>;
}

/// Opens a DSF or DSDIFF file, chosen by its magic bytes rather than its extension.
pub fn open(path: &Path) -> Result<Box<dyn DsdSource>> {
    let mut file = File::open(path).map_err(Error::io(format!("opening {}", path.display())))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(Error::io(format!("reading {}", path.display())))?;
    file.seek(SeekFrom::Start(0))
        .map_err(Error::io(format!("reading {}", path.display())))?;
    let reader = BufReader::new(file);

    match &magic {
        b"DSD " => Ok(Box::new(DsfSource::new(reader)?)),
        b"FRM8" => Ok(Box::new(DffSource::new(reader)?)),
        _ => Err(Error::UnsupportedSource(format!(
            "{} is neither a DSF nor a DSDIFF file",
            path.display()
        ))),
    }
}

/// An in-memory source for tests.
#[cfg(test)]
pub mod testing {
    use std::io;

    use super::{Container, DsdSource, PAD_BYTE, StreamInfo};

    pub struct MemSource {
        info: StreamInfo,
        streams: Vec<Vec<u8>>,
    }

    impl MemSource {
        /// A source with the given stream description, for tests that need a particular rate,
        /// bit order or length. `streams` must all be the same length.
        pub fn with_info(info: StreamInfo, streams: Vec<Vec<u8>>) -> Self {
            Self { info, streams }
        }

        /// LSB-first channel streams; `streams` must all be the same length.
        pub fn new(rate: u32, streams: Vec<Vec<u8>>) -> Self {
            let bytes = streams.first().map_or(0, Vec::len);
            Self {
                info: StreamInfo {
                    container: Container::Dsf,
                    sample_rate: rate,
                    channels: streams.len() as u32,
                    channel_type: 2,
                    sample_count: bytes as u64 * 8,
                    msb_first: false,
                },
                streams,
            }
        }
    }

    impl DsdSource for MemSource {
        fn info(&self) -> &StreamInfo {
            &self.info
        }

        fn read(&mut self, first: i64, len: usize) -> io::Result<Vec<Vec<u8>>> {
            Ok(self
                .streams
                .iter()
                .map(|stream| {
                    (0..len)
                        .map(|k| {
                            usize::try_from(first + k as i64)
                                .ok()
                                .and_then(|i| stream.get(i).copied())
                                .unwrap_or(PAD_BYTE)
                        })
                        .collect()
                })
                .collect())
        }
    }

    /// The bytes of a stereo DSF file with 4096-byte blocks: a header for `sample_count`
    /// samples per channel at `rate`, then `audio` as the (already interleaved) data.
    pub fn stereo_dsf(rate: u32, sample_count: u64, audio: &[u8]) -> Vec<u8> {
        let len = audio.len() as u64;
        let header = dsf::DsfHeader {
            total_file_size: dsf::AUDIO_OFFSET + len,
            metadata_offset: 0,
            channel_type: 2,
            channel_count: 2,
            sample_rate: rate,
            bits_per_sample: 1,
            sample_count,
            block_size: 4096,
            data_chunk_size: dsf::DATA_CHUNK_HEADER_LEN + len,
        };
        let mut bytes = Vec::new();
        header
            .write(&mut bytes)
            .expect("writing to a Vec cannot fail");
        bytes.extend_from_slice(audio);
        bytes
    }

    /// Pseudo-random bytes (an LCG).
    pub fn noise(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect()
    }
}
