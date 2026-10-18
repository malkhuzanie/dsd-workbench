use std::io::{self, Read, Seek, SeekFrom};

use dsf::{AUDIO_OFFSET, DsfHeader};

use super::{Container, DsdSource, PAD_BYTE, StreamInfo};
use crate::error::{Error, Result};

const MAX_BLOCK_SIZE: u32 = 1 << 20;
const MAX_CHANNELS: u32 = 64;
const DATA_CHUNK_HEADER_LEN: u64 = 12;

/// Reads per-channel byte ranges out of the interleaved blocks of a DSF file.
pub struct DsfSource<R> {
    src: R,
    info: StreamInfo,
    channels: usize,
    block_size: usize,
    /// Real audio bytes per channel; everything beyond reads as idle pattern.
    valid: u64,
    scratch: Vec<u8>,
}

impl<R: Read + Seek> DsfSource<R> {
    pub fn new(mut src: R) -> Result<Self> {
        src.seek(SeekFrom::Start(0))
            .map_err(Error::io("seeking in source"))?;
        let header = DsfHeader::read(&mut src)?;
        if header.sample_rate == 0
            || header.channel_count > MAX_CHANNELS
            || header.block_size > MAX_BLOCK_SIZE
        {
            return Err(Error::UnsupportedSource(
                "unsupported sample rate, channel count or block size".to_owned(),
            ));
        }
        let channels = header.channel_count as usize;
        let block_size = header.block_size as usize;
        let stride = (channels * block_size) as u64;
        let audio_len = header.data_chunk_size.saturating_sub(DATA_CHUNK_HEADER_LEN);
        let stored = (audio_len / stride) * block_size as u64;
        let valid = header.sample_count.div_ceil(8).min(stored);

        Ok(Self {
            src,
            info: StreamInfo {
                container: Container::Dsf,
                sample_rate: header.sample_rate,
                channels: header.channel_count,
                channel_type: header.channel_type,
                sample_count: header.sample_count,
                msb_first: header.bits_per_sample == 8,
            },
            channels,
            block_size,
            valid,
            scratch: Vec::new(),
        })
    }
}

impl<R: Read + Seek + Send> DsdSource for DsfSource<R> {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, first: i64, len: usize) -> io::Result<Vec<Vec<u8>>> {
        let mut out = vec![vec![PAD_BYTE; len]; self.channels];
        let end = first.saturating_add(len as i64);
        let lo = first.max(0) as u64;
        let hi = (end.max(0) as u64).min(self.valid);

        let bs = self.block_size as u64;
        let stride = self.channels * self.block_size;
        let mut pos = lo;
        while pos < hi {
            let block = pos / bs;
            let in_block = (pos % bs) as usize;
            let take = (bs - in_block as u64).min(hi - pos) as usize;

            self.src
                .seek(SeekFrom::Start(AUDIO_OFFSET + block * stride as u64))?;
            self.scratch.resize(stride, 0);
            self.src.read_exact(&mut self.scratch)?;

            let dst = (pos as i64 - first) as usize;
            for (channel, target) in out.iter_mut().enumerate() {
                let from = channel * self.block_size + in_block;
                if let (Some(s), Some(d)) = (
                    self.scratch.get(from..from + take),
                    target.get_mut(dst..dst + take),
                ) {
                    d.copy_from_slice(s);
                }
            }
            pos += take as u64;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn deinterleaves_and_pads() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let header = DsfHeader {
            total_file_size: 0,
            metadata_offset: 0,
            channel_type: 2,
            channel_count: 2,
            sample_rate: 2_822_400,
            bits_per_sample: 1,
            sample_count: 64,
            block_size: 4,
            data_chunk_size: DATA_CHUNK_HEADER_LEN + 16,
        };
        let mut file = Vec::new();
        header.write(&mut file)?;
        file.extend(1u8..=16);

        let mut source = DsfSource::new(Cursor::new(file))?;
        let channels = source.read(-2, 12)?;
        let p = PAD_BYTE;
        assert_eq!(channels[0], [p, p, 1, 2, 3, 4, 9, 10, 11, 12, p, p]);
        assert_eq!(channels[1], [p, p, 5, 6, 7, 8, 13, 14, 15, 16, p, p]);
        Ok(())
    }
}
