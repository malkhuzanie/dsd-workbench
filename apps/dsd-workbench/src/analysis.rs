//! Loudness over time: draws a waveform overview, finds silence, suggests where to cut,
//! and tells whether a cut point lands in loud audio.
//!
//! The audio is filtered down to 44.1 or 48 kHz first (a short, cheap filter: this is for
//! looking at the music, not listening to it), then summarised in 10 ms steps.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::audio::Decimator;
use crate::error::{Error, Result};
use crate::source::DsdSource;

/// Resolution of the envelope.
pub const HOPS_PER_SECOND: u32 = 100;
/// Filter length as a multiple of the decimation factor; short keeps analysis fast.
const TAPS_PER_DECIMATION: usize = 4;
const CHUNK_FRAMES: usize = 8192;
/// Surround channels add cost but no information for this purpose.
const MAX_CHANNELS: usize = 2;
/// The quietest level reported, so silence has a finite number.
pub const FLOOR_DB: f32 = -120.0;
/// Audio at or below this level (RMS, dBFS) counts as silence unless the user says otherwise.
pub const DEFAULT_SILENCE_DB: f32 = -60.0;
/// Silence shorter than this is not treated as the gap between tracks.
pub const DEFAULT_MIN_SILENCE_SECONDS: f64 = 1.5;
/// Tracks shorter than this are not created by automatic splitting.
pub const DEFAULT_MIN_TRACK_SECONDS: f64 = 10.0;

/// Peak and RMS level of one 10 ms step, relative to full scale (`±1.0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hop {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

#[derive(Debug, Clone)]
pub struct Envelope {
    /// Length of one hop in seconds.
    pub hop_seconds: f64,
    pub hops: Vec<Hop>,
}

/// `20 * log10(level)`, floored at [`FLOOR_DB`].
pub fn to_db(level: f32) -> f32 {
    (20.0 * level.max(0.0).log10()).max(FLOOR_DB)
}

impl Envelope {
    pub fn duration(&self) -> f64 {
        self.hops.len() as f64 * self.hop_seconds
    }

    fn range_of(&self, from: f64, to: f64) -> &[Hop] {
        let last = self.hops.len();
        let start = ((from.max(0.0) / self.hop_seconds).floor() as usize).min(last);
        let end = ((to.max(0.0) / self.hop_seconds).ceil() as usize).clamp(start, last);
        self.hops.get(start..end).unwrap_or_default()
    }

    /// Loudness between `from` and `to` seconds in dBFS (RMS), or [`FLOOR_DB`] if empty.
    pub fn level_db(&self, from: f64, to: f64) -> f32 {
        let hops = self.range_of(from, to);
        if hops.is_empty() {
            return FLOOR_DB;
        }
        let power: f32 = hops.iter().map(|h| h.rms * h.rms).sum::<f32>() / hops.len() as f32;
        to_db(power.sqrt())
    }

    /// Reduces the stretch `from..to` (seconds) to `count` columns. A column narrower than
    /// one hop shows that hop; stretches outside the file show silence.
    pub fn summarize(&self, from: f64, to: f64, count: usize) -> Vec<Hop> {
        let silent = Hop {
            min: 0.0,
            max: 0.0,
            rms: 0.0,
        };
        if count == 0
            || self.hops.is_empty()
            || to.partial_cmp(&from) != Some(std::cmp::Ordering::Greater)
        {
            return Vec::new();
        }
        let step = (to - from) / count as f64;
        (0..count)
            .map(|column| {
                let start = from + column as f64 * step;
                let first = (start / self.hop_seconds).floor();
                if first < 0.0 || first >= self.hops.len() as f64 {
                    return silent;
                }
                let first = first as usize;
                let last = (((start + step) / self.hop_seconds).ceil() as usize)
                    .clamp(first + 1, self.hops.len());
                let hops = self.hops.get(first..last).unwrap_or_default();
                let power =
                    hops.iter().map(|h| h.rms * h.rms).sum::<f32>() / hops.len().max(1) as f32;
                Hop {
                    min: hops.iter().map(|h| h.min).fold(0.0, f32::min),
                    max: hops.iter().map(|h| h.max).fold(0.0, f32::max),
                    rms: power.sqrt(),
                }
            })
            .collect()
    }
}

/// A stretch of audio quieter than the threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Silence {
    pub start: f64,
    pub end: f64,
}

impl Silence {
    pub fn length(&self) -> f64 {
        self.end - self.start
    }

    pub fn middle(&self) -> f64 {
        (self.start + self.end) / 2.0
    }
}

/// Stretches at least `min_seconds` long whose RMS stays at or below `threshold_db`.
pub fn find_silences(envelope: &Envelope, threshold_db: f32, min_seconds: f64) -> Vec<Silence> {
    let limit = 10f32.powf(threshold_db / 20.0);
    let mut found = Vec::new();
    let mut run_start: Option<usize> = None;
    let close = |found: &mut Vec<Silence>, start: usize, end: usize| {
        let silence = Silence {
            start: start as f64 * envelope.hop_seconds,
            end: end as f64 * envelope.hop_seconds,
        };
        if silence.length() >= min_seconds {
            found.push(silence);
        }
    };
    for (index, hop) in envelope.hops.iter().enumerate() {
        match (hop.rms <= limit, run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(start)) => {
                close(&mut found, start, index);
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        close(&mut found, start, envelope.hops.len());
    }
    found
}

/// Where inside a silence the next track starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CutPlacement {
    /// Halfway through the silence: the gap is shared by both tracks.
    #[default]
    Middle,
    /// Shortly before the music resumes: the silence stays with the previous track.
    BeforeMusic,
}

/// Pre-roll kept ahead of the music for [`CutPlacement::BeforeMusic`].
const PRE_ROLL_SECONDS: f64 = 0.25;
/// Silence this close to the start or end of the file is a lead-in or fade-out, not a gap.
const EDGE_SECONDS: f64 = 0.5;

/// Track start times (not counting the first track at zero) for each usable silence.
pub fn suggest_cuts(
    silences: &[Silence],
    total_seconds: f64,
    min_track_seconds: f64,
    placement: CutPlacement,
) -> Vec<f64> {
    let mut cuts: Vec<f64> = Vec::new();
    let mut previous = 0.0;
    for silence in silences {
        if silence.start < EDGE_SECONDS || silence.end > total_seconds - EDGE_SECONDS {
            continue;
        }
        let at = match placement {
            CutPlacement::Middle => silence.middle(),
            CutPlacement::BeforeMusic => (silence.end - PRE_ROLL_SECONDS).max(silence.middle()),
        };
        if at - previous < min_track_seconds || total_seconds - at < min_track_seconds {
            continue;
        }
        cuts.push(at);
        previous = at;
    }
    cuts
}

/// Everything that decides where automatic splitting cuts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectParams {
    /// Audio at or below this level (dBFS, RMS) is silence.
    pub threshold_db: f32,
    /// Silence must last at least this long to count as a gap between tracks.
    pub min_silence: f64,
    /// Never create a track shorter than this.
    pub min_track: f64,
    pub placement: CutPlacement,
}

impl Default for DetectParams {
    fn default() -> Self {
        Self {
            threshold_db: DEFAULT_SILENCE_DB,
            min_silence: DEFAULT_MIN_SILENCE_SECONDS,
            min_track: DEFAULT_MIN_TRACK_SECONDS,
            placement: CutPlacement::default(),
        }
    }
}

impl DetectParams {
    /// Track start times (after the first track) for `envelope`.
    pub fn cuts(&self, envelope: &Envelope) -> Vec<f64> {
        let silences = find_silences(envelope, self.threshold_db, self.min_silence);
        suggest_cuts(
            &silences,
            envelope.duration(),
            self.min_track,
            self.placement,
        )
    }
}

/// The middle of the silence nearest to `time`, if one lies within `radius` seconds.
pub fn snap_to_silence(time: f64, silences: &[Silence], radius: f64) -> Option<f64> {
    silences
        .iter()
        .map(Silence::middle)
        .filter(|middle| (middle - time).abs() <= radius)
        .min_by(|a, b| (a - time).abs().total_cmp(&(b - time).abs()))
}

/// Half-width of the stretch checked around a cut.
const CUT_CHECK_RADIUS: f64 = 0.05;

/// Whether a cut at `time` lands in loud audio (RMS above `threshold_db` nearby).
pub fn cut_is_loud(envelope: &Envelope, time: f64, threshold_db: f32) -> bool {
    envelope.level_db(time - CUT_CHECK_RADIUS, time + CUT_CHECK_RADIUS) > threshold_db
}

/// Reads the whole source and measures its loudness.
///
/// `progress` receives the fraction done; `cancel` stops the work with [`Error::Cancelled`].
pub fn analyze(
    src: &mut dyn DsdSource,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Envelope> {
    let info = src.info().clone();
    let base = if info.sample_rate.is_multiple_of(44_100) {
        44_100
    } else {
        48_000
    };
    let filter = Decimator::custom(info.sample_rate, info.msb_first, base, TAPS_PER_DECIMATION)
        .ok_or_else(|| Error::Slice("unsupported sample rate".into()))?;

    let out_rate = f64::from(filter.out_rate());
    let hop_frames = ((out_rate / f64::from(HOPS_PER_SECOND)) as usize).max(1);
    let total_frames = (info.sample_count / filter.decim() as u64) as usize;
    let channels = (info.channels as usize).clamp(1, MAX_CHANNELS);
    let step = filter.decim() / 8;
    // Centre the first filter window on sample zero (a negative start reads as idle pattern).
    let base_byte = -((filter.taps() / 2 / 8) as i64);

    let mut hops: Vec<Hop> = Vec::with_capacity(total_frames / hop_frames + 1);
    let mut accumulators = vec![Accumulator::default(); channels];
    let mut in_hop = 0usize;
    let mut done = 0usize;

    while done < total_frames {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let frames = CHUNK_FRAMES.min(total_frames - done);
        let first = base_byte + (done * step) as i64;
        let len = (frames - 1) * step + filter.window_bytes();
        let streams = src
            .read(first, len)
            .map_err(Error::io("reading audio for analysis"))?;

        let pcm: Vec<Vec<f32>> = streams
            .iter()
            .take(channels)
            .map(|bytes| filter.process(bytes, frames))
            .collect();
        for frame in 0..frames {
            for (accumulator, samples) in accumulators.iter_mut().zip(&pcm) {
                if let Some(sample) = samples.get(frame) {
                    accumulator.add(*sample);
                }
            }
            in_hop += 1;
            if in_hop == hop_frames {
                hops.push(close_hop(&mut accumulators));
                in_hop = 0;
            }
        }
        done += frames;
        progress(done as f32 / total_frames.max(1) as f32);
    }
    if in_hop > 0 {
        hops.push(close_hop(&mut accumulators));
    }

    Ok(Envelope {
        hop_seconds: hop_frames as f64 / out_rate,
        hops,
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct Accumulator {
    min: f32,
    max: f32,
    power: f32,
    count: u32,
}

impl Accumulator {
    fn add(&mut self, sample: f32) {
        self.min = self.min.min(sample);
        self.max = self.max.max(sample);
        self.power += sample * sample;
        self.count += 1;
    }
}

/// Combines the channels' accumulators into one hop and resets them.
fn close_hop(accumulators: &mut [Accumulator]) -> Hop {
    let mut hop = Hop {
        min: 0.0,
        max: 0.0,
        rms: 0.0,
    };
    let mut mean_power = 0.0;
    for accumulator in accumulators.iter_mut() {
        hop.min = hop.min.min(accumulator.min);
        hop.max = hop.max.max(accumulator.max);
        mean_power += accumulator.power / accumulator.count.max(1) as f32;
        *accumulator = Accumulator::default();
    }
    hop.rms = (mean_power / accumulators.len().max(1) as f32).sqrt();
    hop
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::PAD_BYTE;
    use crate::source::testing::{MemSource, noise};

    const RATE: u32 = 2_822_400;
    const BYTES_PER_SECOND: usize = RATE as usize / 8;

    fn envelope(levels: &[(f32, usize)]) -> Envelope {
        let hops = levels
            .iter()
            .flat_map(|(rms, count)| {
                std::iter::repeat_n(
                    Hop {
                        min: -*rms,
                        max: *rms,
                        rms: *rms,
                    },
                    *count,
                )
            })
            .collect();
        Envelope {
            hop_seconds: 0.01,
            hops,
        }
    }

    #[test]
    fn decibels_are_floored() {
        assert_eq!(to_db(1.0), 0.0);
        assert!((to_db(0.1) + 20.0).abs() < 1e-4);
        assert_eq!(to_db(0.0), FLOOR_DB);
    }

    #[test]
    fn silences_are_runs_below_the_threshold_that_are_long_enough() {
        // 2 s loud, 1.5 s quiet, 1 s loud, 0.4 s quiet (too short), 1 s loud, 2 s quiet at the end.
        let env = envelope(&[
            (0.3, 200),
            (0.0001, 150),
            (0.3, 100),
            (0.0001, 40),
            (0.3, 100),
            (0.0001, 200),
        ]);
        let silences = find_silences(&env, -60.0, 1.0);
        assert_eq!(silences.len(), 2);
        assert!((silences[0].start - 2.0).abs() < 1e-9 && (silences[0].end - 3.5).abs() < 1e-9);
        assert!(
            (silences[1].end - env.duration()).abs() < 1e-9,
            "a trailing run is closed"
        );
    }

    #[test]
    fn a_threshold_is_inclusive_and_empty_input_is_fine() {
        let env = envelope(&[(0.001, 300)]);
        assert_eq!(
            find_silences(&env, -60.0, 1.0).len(),
            1,
            "exactly -60 dB counts as quiet"
        );
        assert!(
            find_silences(
                &Envelope {
                    hop_seconds: 0.01,
                    hops: vec![]
                },
                -60.0,
                1.0
            )
            .is_empty()
        );
    }

    #[test]
    fn cuts_go_in_the_middle_or_before_the_music_and_skip_edges() {
        let silences = [
            Silence {
                start: 0.0,
                end: 1.0,
            },
            Silence {
                start: 10.0,
                end: 12.0,
            },
            Silence {
                start: 20.0,
                end: 20.2,
            },
            Silence {
                start: 29.0,
                end: 30.0,
            },
        ];
        let middle = suggest_cuts(&silences, 30.0, 5.0, CutPlacement::Middle);
        assert_eq!(
            middle,
            [11.0, 20.1],
            "leading and trailing silence are not gaps"
        );
        let before = suggest_cuts(&silences, 30.0, 5.0, CutPlacement::BeforeMusic);
        assert!((before[0] - 11.75).abs() < 1e-9, "{before:?}");
    }

    #[test]
    fn short_tracks_are_not_created() {
        let silences = [
            Silence {
                start: 10.0,
                end: 11.0,
            },
            Silence {
                start: 12.0,
                end: 13.0,
            },
        ];
        let cuts = suggest_cuts(&silences, 40.0, 5.0, CutPlacement::Middle);
        assert_eq!(cuts, [10.5], "the second gap would make a 2 s track");
    }

    #[test]
    fn snapping_picks_the_nearest_silence_within_the_radius() {
        let silences = [
            Silence {
                start: 9.0,
                end: 11.0,
            },
            Silence {
                start: 19.0,
                end: 21.0,
            },
        ];
        assert_eq!(snap_to_silence(12.0, &silences, 3.0), Some(10.0));
        assert_eq!(snap_to_silence(17.5, &silences, 3.0), Some(20.0));
        assert_eq!(snap_to_silence(15.0, &silences, 3.0), None);
        assert_eq!(snap_to_silence(1.0, &[], 3.0), None);
    }

    #[test]
    fn loud_cuts_are_flagged() {
        let env = envelope(&[(0.3, 200), (0.0001, 200), (0.3, 200)]);
        assert!(cut_is_loud(&env, 1.0, -40.0));
        assert!(!cut_is_loud(&env, 3.0, -40.0));
        assert!(cut_is_loud(&env, 5.0, -40.0));
        assert!(
            !cut_is_loud(&env, 99.0, -40.0),
            "beyond the end there is nothing to hear"
        );
    }

    #[test]
    fn columns_keep_peaks_and_cover_everything() {
        let env = envelope(&[(0.1, 50), (0.9, 1), (0.1, 49)]);
        let whole = env.duration();
        let columns = env.summarize(0.0, whole, 10);
        assert_eq!(columns.len(), 10);
        assert!(
            columns.iter().any(|c| (c.max - 0.9).abs() < 1e-6),
            "a single loud hop survives"
        );
        assert!(env.summarize(0.0, whole, 0).is_empty());
        assert_eq!(
            env.summarize(0.0, whole, 1000).len(),
            1000,
            "more columns than hops still works"
        );
    }

    #[test]
    fn a_zoomed_summary_follows_the_window_and_shows_silence_outside_the_file() {
        let env = envelope(&[(0.1, 100), (0.9, 100)]); // 1 s quiet-ish, 1 s loud
        let early = env.summarize(0.0, 1.0, 10);
        let late = env.summarize(1.0, 2.0, 10);
        assert!(early.iter().all(|h| h.rms < 0.2));
        assert!(late.iter().all(|h| h.rms > 0.8));

        let outside = env.summarize(5.0, 6.0, 4);
        assert_eq!(outside.len(), 4);
        assert!(outside.iter().all(|h| h.rms == 0.0 && h.max == 0.0));
        let before = env.summarize(-2.0, -1.0, 4);
        assert!(before.iter().all(|h| h.rms == 0.0));

        assert!(env.summarize(1.0, 1.0, 10).is_empty(), "an empty window");
        assert!(env.summarize(2.0, 1.0, 10).is_empty(), "a backwards window");
        assert_eq!(
            env.summarize(0.0, 0.05, 20).len(),
            20,
            "narrower than a hop still works"
        );
    }

    /// `seconds` of noise (loud) or idle pattern (silent), 1 channel.
    fn segment(loud: bool, seconds: usize, seed: u32) -> Vec<u8> {
        if loud {
            noise(BYTES_PER_SECOND * seconds, seed)
        } else {
            vec![PAD_BYTE; BYTES_PER_SECOND * seconds]
        }
    }

    fn song_gap_song() -> MemSource {
        let mut stream = Vec::new();
        stream.extend(segment(false, 1, 0)); // lead-in
        stream.extend(segment(true, 4, 1));
        stream.extend(segment(false, 3, 0)); // the gap
        stream.extend(segment(true, 4, 2));
        stream.extend(segment(false, 1, 0)); // fade-out
        MemSource::new(RATE, vec![stream.clone(), stream])
    }

    #[test]
    fn analysis_finds_the_gap_between_two_songs()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut source = song_gap_song();
        let mut last = 0.0;
        let env = analyze(&mut source, &AtomicBool::new(false), &mut |f| last = f)?;
        assert!((env.duration() - 13.0).abs() < 0.05, "{}", env.duration());
        assert!(last > 0.99);

        let silences = find_silences(&env, -60.0, 1.5);
        let from_params = DetectParams {
            min_track: 2.0,
            ..DetectParams::default()
        }
        .cuts(&env);
        assert_eq!(from_params.len(), 1);
        let gap = silences
            .iter()
            .find(|s| s.length() > 2.5)
            .ok_or("the 3 s gap was not found")?;
        assert!(
            (gap.start - 5.0).abs() < 0.1 && (gap.end - 8.0).abs() < 0.1,
            "{gap:?}"
        );

        let cuts = suggest_cuts(&silences, env.duration(), 2.0, CutPlacement::Middle);
        assert_eq!(cuts.len(), 1, "{cuts:?}");
        assert!((cuts[0] - 6.5).abs() < 0.1);

        assert!(cut_is_loud(&env, 3.0, -40.0), "inside a song");
        assert!(!cut_is_loud(&env, 6.5, -40.0), "inside the gap");
        Ok(())
    }

    /// A second-order sigma-delta modulator, as in a real DSD encoder: `input` in `-1..1`
    /// per sample becomes a 1-bit stream, packed LSB first.
    fn modulate(input: impl Iterator<Item = f64>) -> Vec<u8> {
        let (mut v1, mut v2) = (0.0f64, 0.0f64);
        let mut bytes = Vec::new();
        let (mut byte, mut bit) = (0u8, 0);
        for x in input {
            let y = if v2 >= 0.0 { 1.0 } else { -1.0 };
            v1 += x - y;
            v2 += v1 - y;
            if y > 0.0 {
                byte |= 1 << bit;
            }
            bit += 1;
            if bit == 8 {
                bytes.push(byte);
                (byte, bit) = (0, 0);
            }
        }
        bytes
    }

    #[test]
    fn a_modulated_tone_is_loud_and_modulated_silence_is_far_below_the_default_threshold() {
        let seconds = 6;
        let samples = RATE as usize * seconds;
        let tone_level = 0.3; // -10.5 dBFS peak
        let signal = (0..samples).map(|n| {
            let t = n as f64 / f64::from(RATE);
            if t < 3.0 {
                tone_level * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()
            } else {
                0.0
            }
        });
        let mut source = MemSource::new(RATE, vec![modulate(signal)]);
        let env = analyze(&mut source, &AtomicBool::new(false), &mut |_| {}).expect("analysis");

        let tone = env.level_db(0.5, 2.5);
        let quiet = env.level_db(3.5, 5.5);
        let expected = 20.0 * (tone_level / 2f64.sqrt()).log10() as f32;
        assert!(
            (tone - expected).abs() < 1.5,
            "tone {tone} dB, expected about {expected} dB"
        );
        assert!(
            quiet < DEFAULT_SILENCE_DB - 10.0,
            "modulator idle noise is {quiet} dB"
        );

        let silences = find_silences(&env, DEFAULT_SILENCE_DB, 1.5);
        assert_eq!(silences.len(), 1, "{silences:?}");
        assert!((silences[0].start - 3.0).abs() < 0.2);
    }

    #[test]
    fn analysis_can_be_cancelled_and_rejects_nonsense_rates() {
        let mut source = song_gap_song();
        let stop = AtomicBool::new(true);
        assert!(matches!(
            analyze(&mut source, &stop, &mut |_| {}),
            Err(Error::Cancelled)
        ));

        let mut tiny = MemSource::new(32, vec![vec![0; 16]]);
        assert!(analyze(&mut tiny, &AtomicBool::new(false), &mut |_| {}).is_err());
    }

    #[test]
    fn a_short_source_still_produces_an_envelope()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut source = MemSource::new(RATE, vec![noise(1000, 3)]);
        let env = analyze(&mut source, &AtomicBool::new(false), &mut |_| {})?;
        assert!(env.hops.len() <= 2);
        Ok(())
    }
}
