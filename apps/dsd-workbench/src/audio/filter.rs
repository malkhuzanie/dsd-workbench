//! DSD to PCM: a windowed-sinc low-pass with decimation. Each lookup table covers
//! eight 1-bit taps, so one output sample costs one table read per input byte of the window.

use std::f64::consts::PI;

use super::settings::Settings;
#[cfg(test)]
use crate::source::PAD_BYTE;

const PASSBAND_HZ: f64 = 20_000.0;

pub struct Decimator {
    decim: usize,
    out_rate: u32,
    tables: Vec<[f32; 256]>,
}

/// Returns `(decimation, output rate)` for the nearest achievable rate at or below `target`.
/// The decimation is a multiple of 8.
fn plan(rate: u32, target: u32) -> (usize, u32) {
    let decim = ((rate / target.max(1)) as usize / 8 * 8).max(8);
    (decim, rate / decim as u32)
}

fn design(taps: usize, cutoff: f64) -> Vec<f64> {
    let center = (taps as f64 - 1.0) / 2.0;
    let last = (taps - 1) as f64;
    let mut h: Vec<f64> = (0..taps)
        .map(|n| {
            let m = n as f64 - center;
            let sinc = if m.abs() < 1e-12 {
                2.0 * cutoff
            } else {
                (2.0 * PI * cutoff * m).sin() / (PI * m)
            };
            let x = n as f64 / last;
            let blackman = 0.42 - 0.5 * (2.0 * PI * x).cos() + 0.08 * (4.0 * PI * x).cos();
            sinc * blackman
        })
        .collect();
    let sum: f64 = h.iter().sum();
    if sum.abs() > f64::EPSILON {
        h.iter_mut().for_each(|v| *v /= sum);
    }
    h
}

impl Decimator {
    pub fn new(rate: u32, msb_first: bool, settings: &Settings) -> Option<Self> {
        Self::custom(
            rate,
            msb_first,
            settings.rate.target_hz(rate),
            settings.quality.taps_per_decimation(),
        )
    }

    /// A filter for `target_hz` output with `taps_per_decimation` taps per input sample
    /// removed (longer is steeper and slower). Returns `None` for unusable rates.
    pub fn custom(
        rate: u32,
        msb_first: bool,
        target_hz: u32,
        taps_per_decimation: usize,
    ) -> Option<Self> {
        if rate < 64 || taps_per_decimation == 0 {
            return None;
        }
        let (decim, out_rate) = plan(rate, target_hz);
        let taps = decim * taps_per_decimation;
        let cutoff_hz = PASSBAND_HZ.min(0.45 * f64::from(out_rate));
        let coefficients = design(taps, cutoff_hz / f64::from(rate));

        let tables = coefficients
            .as_chunks::<8>()
            .0
            .iter()
            .map(|group| {
                let mut table = [0f32; 256];
                for (value, slot) in table.iter_mut().enumerate() {
                    let sum: f64 = group
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let bit = if msb_first {
                                (value >> (7 - i)) & 1
                            } else {
                                (value >> i) & 1
                            };
                            if bit == 1 { *c } else { -*c }
                        })
                        .sum();
                    *slot = sum as f32;
                }
                table
            })
            .collect();

        Some(Self {
            decim,
            out_rate,
            tables,
        })
    }

    /// Input samples consumed per output sample.
    pub fn decim(&self) -> usize {
        self.decim
    }

    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    pub fn taps(&self) -> usize {
        self.tables.len() * 8
    }

    /// Bytes of the input stream covered by one filter window.
    pub fn window_bytes(&self) -> usize {
        self.tables.len()
    }

    /// Produces `count` output samples. Output `j` uses the window starting at byte
    /// `j * decim / 8`; a window that does not fit in `stream` yields silence.
    pub fn process(&self, stream: &[u8], count: usize) -> Vec<f32> {
        let step = self.decim / 8;
        let width = self.tables.len();
        (0..count)
            .map(|j| {
                let from = j * step;
                stream.get(from..from + width).map_or(0.0, |window| {
                    window
                        .iter()
                        .zip(&self.tables)
                        .map(|(byte, table)| table[usize::from(*byte)])
                        .sum::<f32>()
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{Quality, RateTarget};

    #[test]
    fn plans_rates() {
        assert_eq!(plan(2_822_400, 88_200), (32, 88_200));
        assert_eq!(plan(5_644_800, 88_200), (64, 88_200));
        assert_eq!(plan(5_644_800, 352_800), (16, 352_800));
        assert_eq!(
            plan(2_822_400, 705_600),
            (8, 352_800),
            "clamped to rate / 8"
        );
        assert_eq!(plan(3_072_000, 96_000), (32, 96_000));
    }

    #[test]
    fn dc_levels_and_idle_pattern() -> Result<(), String> {
        for msb in [false, true] {
            for rate in RateTarget::ALL {
                let settings = Settings {
                    rate,
                    quality: Quality::Fast,
                };
                let dec = Decimator::new(2_822_400, msb, &settings).ok_or("filter")?;
                let len = dec.window_bytes() + 4;
                let ones = dec.process(&vec![0xFF; len], 1);
                let zeros = dec.process(&vec![0x00; len], 1);
                let idle = dec.process(&vec![PAD_BYTE; len], 1);
                assert!((ones[0] - 1.0).abs() < 1e-3);
                assert!((zeros[0] + 1.0).abs() < 1e-3);
                assert!(idle[0].abs() < 0.01, "idle pattern must decode near zero");
            }
        }
        Ok(())
    }
}
