//! Streams a slice of audio from any DSD source into DSF blocks (LSB first), re-blocking at bit level.

use std::io::Write;

use super::bits::{self, BitOrder};
use super::slice::Slice;
use crate::error::{Error, Result};
use crate::source::DsdSource;

/// Writes the audio payload for `slice` (no header) to `out`.
///
/// `after_block` receives the number of audio bytes written so far after every
/// block; returning an error stops the copy and is passed on.
pub(super) fn copy_slice<W: Write>(
    src: &mut dyn DsdSource,
    block_size: u32,
    slice: &Slice,
    out: &mut W,
    after_block: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<()> {
    let info = src.info().clone();
    let bs = block_size as usize;
    let channels = info.channels as usize;
    if bs == 0 || channels == 0 {
        return Err(Error::Slice("zero block size or channel count".into()));
    }
    let order = BitOrder::from_msb(info.msb_first);

    // Position of the first kept sample inside each channel's byte stream.
    let start_byte = slice.start_sample / 8;
    let shift = (slice.start_sample % 8) as u32;
    let mut block = vec![0u8; bs * channels];

    for index in 0..slice.out_blocks {
        let first = start_byte
            .checked_add(index.saturating_mul(bs as u64))
            .and_then(|v| i64::try_from(v).ok())
            .ok_or_else(|| Error::Slice("audio offsets overflow".into()))?;
        let streams = src
            .read(first, bs + 1)
            .map_err(Error::io("reading source audio"))?;
        let is_last = index + 1 == slice.out_blocks;

        for (channel, target) in block.chunks_exact_mut(bs).enumerate() {
            let window = streams
                .get(channel)
                .ok_or_else(|| Error::Slice("source returned too few channels".into()))?;
            bits::shift_to_lsb_first(window, shift, order, target);
            if is_last {
                let first_index = index.saturating_mul(bs as u64);
                bits::clear_tail(target, first_index, slice.sample_count, BitOrder::LsbFirst);
            }
        }
        out.write_all(&block).map_err(Error::io("writing audio"))?;
        after_block((index + 1).saturating_mul(block.len() as u64))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::testing::{MemSource, noise};
    use crate::source::{Container, StreamInfo};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    const BS: usize = 16;
    const CH: usize = 2;
    const BLOCKS: usize = 5;
    const SPB: u64 = (BS * 8) as u64;

    fn bit(stream: &[u8], index: u64, order: BitOrder) -> u8 {
        let byte = stream.get((index / 8) as usize).copied().unwrap_or(0);
        let pos = (index % 8) as u32;
        let shift = match order {
            BitOrder::LsbFirst => pos,
            BitOrder::MsbFirst => 7 - pos,
        };
        (byte >> shift) & 1
    }

    /// Output is always LSB first, whatever the source order.
    fn check(msb_first: bool) -> TestResult {
        let total = BLOCKS as u64 * SPB;
        let order = BitOrder::from_msb(msb_first);
        let streams: Vec<Vec<u8>> = (0..CH).map(|c| noise(BLOCKS * BS, 7 + c as u32)).collect();
        let mut source = MemSource::with_info(
            StreamInfo {
                container: Container::Dsf,
                sample_rate: 2_822_400,
                channels: CH as u32,
                channel_type: 2,
                sample_count: total,
                msb_first,
            },
            streams.clone(),
        );

        for start in [0u64, 1, 7, 8, 100, 127, 128, 130, 300] {
            for count in [1u64, 5, 8, 100, 128, 200, 340] {
                if start + count > total {
                    continue;
                }
                let out_blocks = count.div_ceil(SPB);
                let slice = Slice {
                    start_sample: start,
                    sample_count: count,
                    out_blocks,
                    audio_len: out_blocks * (BS * CH) as u64,
                };
                let mut out = Vec::new();
                copy_slice(&mut source, BS as u32, &slice, &mut out, &mut |_| Ok(()))?;
                assert_eq!(out.len() as u64, slice.audio_len);

                for (c, reference) in streams.iter().enumerate() {
                    let mut channel = Vec::new();
                    for b in 0..out_blocks as usize {
                        let from = (b * CH + c) * BS;
                        channel.extend_from_slice(out.get(from..from + BS).ok_or("short output")?);
                    }
                    for j in 0..out_blocks * SPB {
                        let want = if j < count {
                            bit(reference, start + j, order)
                        } else {
                            0
                        };
                        assert_eq!(
                            bit(&channel, j, BitOrder::LsbFirst),
                            want,
                            "start {start} count {count} channel {c} sample {j}"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn callback_can_stop_the_copy() {
        let mut source = MemSource::with_info(
            StreamInfo {
                container: Container::Dsf,
                sample_rate: 2_822_400,
                channels: CH as u32,
                channel_type: 2,
                sample_count: BLOCKS as u64 * SPB,
                msb_first: false,
            },
            vec![vec![0; BLOCKS * BS]; CH],
        );
        let slice = Slice {
            start_sample: 0,
            sample_count: BLOCKS as u64 * SPB,
            out_blocks: BLOCKS as u64,
            audio_len: (BLOCKS * BS * CH) as u64,
        };
        let mut out = Vec::new();
        let mut seen = Vec::new();
        let result = copy_slice(&mut source, BS as u32, &slice, &mut out, &mut |done| {
            seen.push(done);
            if seen.len() == 2 {
                Err(Error::Cancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(seen, [(BS * CH) as u64, (2 * BS * CH) as u64]);
        assert_eq!(out.len(), 2 * BS * CH);
    }

    #[test]
    fn lsb_first_source_is_sample_exact() -> TestResult {
        check(false)
    }

    #[test]
    fn msb_first_source_is_converted_to_lsb_first() -> TestResult {
        check(true)
    }
}
