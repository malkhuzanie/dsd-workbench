//! Bit-level helpers for DSD sample streams.
//!
//! A DSD channel is a stream of 1-bit samples packed eight to a byte. The
//! header's `bits_per_sample` says which end of a byte comes first in time.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BitOrder {
    /// The first sample in time is the least significant bit (`bits_per_sample == 1`).
    LsbFirst,
    /// The first sample in time is the most significant bit (`bits_per_sample == 8`).
    MsbFirst,
}

impl BitOrder {
    pub(super) fn from_msb(msb_first: bool) -> Self {
        if msb_first {
            Self::MsbFirst
        } else {
            Self::LsbFirst
        }
    }
}

/// Re-packs a channel stream so that output sample 0 is input sample `shift`.
///
/// `window` must hold `out.len() + 1` bytes of the channel stream.
pub(super) fn shift_into(window: &[u8], shift: u32, order: BitOrder, out: &mut [u8]) {
    let shift = shift % 8;
    for (slot, pair) in out.iter_mut().zip(window.windows(2)) {
        if let &[a, b] = pair {
            let (a, b) = (u16::from(a), u16::from(b));
            *slot = match order {
                BitOrder::LsbFirst => (((b << 8) | a) >> shift) as u8,
                BitOrder::MsbFirst => (((a << 8) | b) >> (8 - shift)) as u8,
            };
        }
    }
}

/// Zeroes every sample at or beyond `sample_count` in a block of a channel stream.
/// `first_index` is the stream byte index of `block[0]`.
pub(super) fn clear_tail(block: &mut [u8], first_index: u64, sample_count: u64, order: BitOrder) {
    let full_bytes = sample_count / 8;
    let partial = (sample_count % 8) as u32;
    let end = full_bytes + u64::from(partial > 0);

    for (offset, byte) in block.iter_mut().enumerate() {
        let index = first_index.saturating_add(offset as u64);
        if index >= end {
            *byte = 0;
        } else if index == full_bytes && partial > 0 {
            *byte &= keep_mask(partial, order);
        }
    }
}

/// Mask that keeps the first `keep` samples (in time) of a byte; `keep` is 1..=7.
fn keep_mask(keep: u32, order: BitOrder) -> u8 {
    match order {
        BitOrder::LsbFirst => ((1u16 << keep) - 1) as u8,
        BitOrder::MsbFirst => (0xFF00u16 >> keep) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_lsb_first() {
        let mut out = [0u8; 1];
        shift_into(&[0b0000_0010, 0b0000_0001], 1, BitOrder::LsbFirst, &mut out);
        assert_eq!(out, [0x81]);
    }

    #[test]
    fn shift_msb_first() {
        let mut out = [0u8; 1];
        shift_into(&[0b0100_0000, 0b1000_0000], 1, BitOrder::MsbFirst, &mut out);
        assert_eq!(out, [0x81]);
    }

    #[test]
    fn tail_is_cleared() {
        let mut lsb = [0xFFu8; 3];
        clear_tail(&mut lsb, 0, 3, BitOrder::LsbFirst);
        assert_eq!(lsb, [0x07, 0, 0]);

        let mut msb = [0xFFu8; 3];
        clear_tail(&mut msb, 0, 3, BitOrder::MsbFirst);
        assert_eq!(msb, [0xE0, 0, 0]);

        let mut whole = [0xFFu8; 3];
        clear_tail(&mut whole, 0, 8, BitOrder::LsbFirst);
        assert_eq!(whole, [0xFF, 0, 0]);
    }
}
