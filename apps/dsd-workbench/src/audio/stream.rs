//! Turns a range of a DSD file into interleaved `f32` PCM, chunk by chunk.

use super::filter::Decimator;
use super::settings::Settings;
use crate::source::DsdSource;

const CHUNK_FRAMES: usize = 8192;
const MAX_OUT_CHANNELS: usize = 2;
const FADE_SECONDS: f64 = 0.005;

pub struct PcmStream {
    src: Box<dyn DsdSource>,
    filter: Decimator,
    out_channels: usize,
    /// Channel-stream byte index where the first filter window starts (may be negative).
    base_byte: i64,
    next_out: usize,
    total_out: usize,
    fade: usize,
}

impl PcmStream {
    /// Plays samples `start_sample..end_sample`. The start is aligned down to 8 samples.
    pub fn new(
        src: Box<dyn DsdSource>,
        start_sample: u64,
        end_sample: u64,
        settings: &Settings,
    ) -> Result<Self, String> {
        let info = src.info().clone();
        let filter = Decimator::new(info.sample_rate, info.msb_first, settings)
            .ok_or_else(|| "unsupported sample rate".to_owned())?;

        let aligned = start_sample & !7;
        let end = end_sample.min(info.sample_count);
        if end <= aligned {
            return Err("nothing to play".to_owned());
        }
        let total_out = (end - aligned).div_ceil(filter.decim() as u64) as usize;
        // Both terms are multiples of 8, so the division is exact.
        let base_byte = (aligned as i64 - (filter.taps() / 2) as i64) / 8;
        let fade = ((f64::from(filter.out_rate()) * FADE_SECONDS) as usize).max(1);

        Ok(Self {
            src,
            out_channels: (info.channels as usize).min(MAX_OUT_CHANNELS),
            filter,
            base_byte,
            next_out: 0,
            total_out,
            fade,
        })
    }

    pub fn out_rate(&self) -> u32 {
        self.filter.out_rate()
    }

    pub fn channels(&self) -> usize {
        self.out_channels
    }

    /// Next block of interleaved samples, or `None` at the end of the range.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<f32>>, String> {
        if self.next_out >= self.total_out {
            return Ok(None);
        }
        let n = CHUNK_FRAMES.min(self.total_out - self.next_out);
        let step = self.filter.decim() / 8;
        let first = self.base_byte + (self.next_out * step) as i64;
        let len = (n - 1) * step + self.filter.window_bytes();
        let channels = self.src.read(first, len).map_err(|e| e.to_string())?;

        let oc = self.out_channels;
        let mut out = vec![0.0f32; n * oc];
        for (channel, bytes) in channels.iter().take(oc).enumerate() {
            let samples = self.filter.process(bytes, n);
            for (i, (frame, sample)) in out.chunks_exact_mut(oc).zip(samples).enumerate() {
                let gain = self.gain(self.next_out + i);
                if let Some(slot) = frame.get_mut(channel) {
                    *slot = (sample * gain).clamp(-1.0, 1.0);
                }
            }
        }
        self.next_out += n;
        Ok(Some(out))
    }

    /// Short fades at both ends of the range avoid clicks.
    fn gain(&self, index: usize) -> f32 {
        let from_end = self.total_out.saturating_sub(index).saturating_sub(1);
        (index.min(from_end) as f32 / self.fade as f32).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;
    use std::io::Cursor;

    use dsf::DsfHeader;

    use super::*;
    use crate::audio::{Quality, RateTarget};
    use crate::source::DsfSource;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const RATE: u32 = 2_822_400;
    const SAMPLES: u64 = 282_240; // 0.1 s
    const TONE_HZ: f64 = 1_000.0;
    const AMPLITUDE: f64 = 0.5;
    const EDGE: usize = 600;

    /// A first-order sigma-delta modulation of a sine, packed as a mono DSF.
    fn modulate(msb_first: bool) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let block = 4096usize;
        let blocks = (SAMPLES as usize).div_ceil(block * 8);
        let mut stream = vec![0u8; blocks * block];
        let (mut integrator, mut previous) = (0.0f64, -1.0f64);
        for n in 0..SAMPLES as usize {
            let x = AMPLITUDE * (2.0 * PI * TONE_HZ * n as f64 / f64::from(RATE)).sin();
            integrator += x - previous;
            previous = if integrator >= 0.0 { 1.0 } else { -1.0 };
            if previous > 0.0 {
                let bit = n % 8;
                let shift = if msb_first { 7 - bit } else { bit };
                if let Some(byte) = stream.get_mut(n / 8) {
                    *byte |= 1 << shift;
                }
            }
        }
        let header = DsfHeader {
            total_file_size: 0,
            metadata_offset: 0,
            channel_type: 1,
            channel_count: 1,
            sample_rate: RATE,
            bits_per_sample: if msb_first { 8 } else { 1 },
            sample_count: SAMPLES,
            block_size: block as u32,
            data_chunk_size: 12 + stream.len() as u64,
        };
        let mut file = Vec::new();
        header.write(&mut file)?;
        file.extend(stream);
        Ok(file)
    }

    fn check(msb_first: bool, settings: Settings) -> TestResult {
        let file = modulate(msb_first)?;
        let source = Box::new(DsfSource::new(Cursor::new(file))?);
        let mut pcm = PcmStream::new(source, 0, SAMPLES, &settings)?;
        let rate = f64::from(pcm.out_rate());
        let mut samples = Vec::new();
        while let Some(chunk) = pcm.next_chunk()? {
            samples.extend(chunk);
        }

        let trimmed = samples
            .get(EDGE..samples.len().saturating_sub(EDGE))
            .ok_or("too few samples")?;
        let (mut i, mut q) = (0.0f64, 0.0f64);
        for (k, s) in trimmed.iter().enumerate() {
            let phase = 2.0 * PI * TONE_HZ * (k + EDGE) as f64 / rate;
            i += f64::from(*s) * phase.sin();
            q += f64::from(*s) * phase.cos();
        }
        let amplitude = 2.0 * (i * i + q * q).sqrt() / trimmed.len() as f64;
        assert!(
            (amplitude - AMPLITUDE).abs() < 0.05,
            "amplitude {amplitude}"
        );
        Ok(())
    }

    #[test]
    fn decodes_a_tone_lsb_first() -> TestResult {
        check(false, Settings::default())
    }

    #[test]
    fn decodes_a_tone_msb_first() -> TestResult {
        check(true, Settings::default())
    }

    #[test]
    fn decodes_a_tone_at_high_rate_and_quality() -> TestResult {
        check(
            false,
            Settings {
                rate: RateTarget::Maximum,
                quality: Quality::Precise,
            },
        )
    }
}
