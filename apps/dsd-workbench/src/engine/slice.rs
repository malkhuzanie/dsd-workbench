//! Pure arithmetic mapping a time range onto sample positions and output size.

use crate::error::{Error, Result};
use crate::source::StreamInfo;

/// Size of the `data` chunk header counted inside `data_chunk_size`.
pub(super) const DATA_CHUNK_HEADER_LEN: u64 = 12;
/// Block size per channel of every DSF file this tool writes.
pub(super) const OUT_BLOCK_SIZE: u32 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Slice {
    /// First sample (per channel) of the source to keep.
    pub start_sample: u64,
    /// Samples per channel in the slice.
    pub sample_count: u64,
    /// Blocks per channel in the output.
    pub out_blocks: u64,
    /// Audio bytes in the output file (all channels, padding included).
    pub audio_len: u64,
}

pub(super) fn compute(
    info: &StreamInfo,
    block_size: u32,
    start_seconds: f64,
    duration: Option<f64>,
) -> Result<Slice> {
    if !start_seconds.is_finite() || start_seconds < 0.0 {
        return Err(Error::Slice(format!("invalid start time {start_seconds}")));
    }
    if let Some(d) = duration
        && (!d.is_finite() || d <= 0.0)
    {
        return Err(Error::Slice(format!("invalid duration {d}")));
    }

    let rate = f64::from(info.sample_rate);
    let total = info.sample_count;
    let start_sample = (start_seconds * rate).round() as u64;
    if start_sample >= total {
        return Err(Error::Slice(
            "start time exceeds the source audio duration".into(),
        ));
    }

    // Both ends are rounded from absolute times, so neighbouring tracks tile exactly.
    let end_sample = match duration {
        Some(d) => ((start_seconds + d) * rate).round() as u64,
        None => total,
    }
    .min(total);
    if end_sample <= start_sample {
        return Err(Error::Slice("track is shorter than one sample".into()));
    }
    let sample_count = end_sample - start_sample;

    let samples_per_block = u64::from(block_size) * 8;
    if samples_per_block == 0 {
        return Err(Error::Slice("zero block size".into()));
    }
    let stride = u64::from(block_size) * u64::from(info.channels);
    let out_blocks = sample_count.div_ceil(samples_per_block);
    let audio_len = out_blocks
        .checked_mul(stride)
        .ok_or_else(|| Error::Slice("audio size overflows".into()))?;

    Ok(Slice {
        start_sample,
        sample_count,
        out_blocks,
        audio_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Container;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    /// 4 blocks of 32768 samples per channel; at this rate 1 s == 1 block.
    fn info() -> StreamInfo {
        StreamInfo {
            container: Container::Dsf,
            sample_rate: 32_768,
            channels: 2,
            channel_type: 2,
            sample_count: 4 * 32_768,
            msb_first: false,
        }
    }

    #[test]
    fn whole_file() -> TestResult {
        let s = compute(&info(), 4096, 0.0, None)?;
        assert_eq!(
            s,
            Slice {
                start_sample: 0,
                sample_count: 4 * 32_768,
                out_blocks: 4,
                audio_len: 4 * 8192
            }
        );
        Ok(())
    }

    #[test]
    fn cut_is_sample_exact_not_block_aligned() -> TestResult {
        let s = compute(&info(), 4096, 0.5, Some(1.0))?;
        assert_eq!(
            s,
            Slice {
                start_sample: 16_384,
                sample_count: 32_768,
                out_blocks: 1,
                audio_len: 8192
            }
        );
        Ok(())
    }

    #[test]
    fn neighbours_tile_exactly() -> TestResult {
        let first = compute(&info(), 4096, 0.3, Some(0.4))?;
        let second = compute(&info(), 4096, 0.7, None)?;
        assert_eq!(first.start_sample + first.sample_count, second.start_sample);
        Ok(())
    }

    #[test]
    fn rejects_bad_ranges() {
        let i = info();
        assert!(matches!(
            compute(&i, 4096, 10.0, None),
            Err(Error::Slice(_))
        ));
        assert!(matches!(
            compute(&i, 4096, -1.0, None),
            Err(Error::Slice(_))
        ));
        assert!(matches!(
            compute(&i, 4096, 0.0, Some(0.0)),
            Err(Error::Slice(_))
        ));
        assert!(matches!(
            compute(&i, 4096, f64::NAN, None),
            Err(Error::Slice(_))
        ));
    }
}
