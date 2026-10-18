//! Log-spaced spectrum bars from recent mono samples.

use std::f32::consts::PI;

use super::fft::fft;

pub const FFT_SIZE: usize = 4096;
const MIN_HZ: f64 = 30.0;
const MAX_HZ: f64 = 20_000.0;
const FLOOR_DB: f32 = -90.0;
/// Per-update fall-off of a bar (peaks rise instantly, then decay).
const FALL: f32 = 0.9;

pub struct Analyzer {
    bars: Vec<f32>,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
}

fn top_hz(rate: u32) -> f64 {
    MAX_HZ.min(f64::from(rate) / 2.0 * 0.98)
}

impl Analyzer {
    pub fn new(bars: usize) -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        Self {
            bars: vec![0.0; bars.max(1)],
            window,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
        }
    }

    /// Bar levels in `0.0..=1.0`.
    pub fn bars(&self) -> &[f32] {
        &self.bars
    }

    /// Lets the bars fall while nothing is playing.
    pub fn decay(&mut self) {
        self.bars.iter_mut().for_each(|b| *b *= FALL);
    }

    pub fn range_hz(&self, bar: usize, rate: u32) -> (f64, f64) {
        let top = top_hz(rate);
        let n = self.bars.len() as f64;
        let edge = |i: f64| MIN_HZ * (top / MIN_HZ).powf(i / n);
        (edge(bar as f64), edge(bar as f64 + 1.0))
    }

    /// Analyses the last [`FFT_SIZE`] samples of `samples`.
    pub fn update(&mut self, samples: &[f32], rate: u32) {
        if rate == 0 {
            return;
        }
        self.re.fill(0.0);
        self.im.fill(0.0);
        let take = samples.len().min(FFT_SIZE);
        let offset = FFT_SIZE - take;
        if let (Some(src), Some(dst), Some(win)) = (
            samples.get(samples.len() - take..),
            self.re.get_mut(offset..),
            self.window.get(offset..),
        ) {
            for ((d, s), w) in dst.iter_mut().zip(src).zip(win) {
                *d = s * w;
            }
        }
        fft(&mut self.re, &mut self.im);

        let half = FFT_SIZE / 2;
        let norm = FFT_SIZE as f32 / 4.0; // a full-scale sine reads as 1.0 with a Hann window
        let levels: Vec<f32> = self
            .re
            .iter()
            .zip(&self.im)
            .take(half)
            .map(|(r, i)| {
                let db = 20.0 * (r.hypot(*i) / norm).max(1e-9).log10();
                ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
            })
            .collect();

        let bin_hz = f64::from(rate) / FFT_SIZE as f64;
        for b in 0..self.bars.len() {
            let (lo, hi) = self.range_hz(b, rate);
            let first = ((lo / bin_hz).floor() as usize).clamp(1, half - 1);
            let last = ((hi / bin_hz).ceil() as usize).clamp(first + 1, half);
            let peak = levels
                .get(first..last)
                .map_or(0.0, |s| s.iter().copied().fold(0.0, f32::max));
            if let Some(bar) = self.bars.get_mut(b) {
                *bar = peak.max(*bar * FALL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_lights_the_right_bar() {
        let rate = 88_200;
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 * (2.0 * PI * 1000.0 * i as f32 / rate as f32).sin())
            .collect();
        let mut analyzer = Analyzer::new(48);
        analyzer.update(&samples, rate);

        let loudest = analyzer
            .bars()
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i);
        let (lo, hi) = analyzer.range_hz(loudest, rate);
        assert!(
            lo * 0.9 <= 1000.0 && 1000.0 <= hi * 1.1,
            "bar {loudest}: {lo}..{hi}"
        );
    }
}
