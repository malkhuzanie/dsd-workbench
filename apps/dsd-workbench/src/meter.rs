//! Throughput and time-remaining estimates for a running job.
//!
//! The meter never reads a clock: callers pass the time elapsed since the job started,
//! which keeps it deterministic and testable.

use std::collections::HashMap;
use std::time::Duration;

use crate::disk::format_bytes;

/// Samples closer together than this are folded into the next one.
const MIN_SAMPLE_GAP: Duration = Duration::from_millis(200);
/// Weight of the newest rate sample in the running average.
const SMOOTHING: f64 = 0.25;
/// An estimate made earlier than this would swing wildly, so none is given.
const MIN_ELAPSED_FOR_ETA: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct Meter {
    total: u64,
    done: u64,
    elapsed: Duration,
    last_sample: Option<(Duration, u64)>,
    /// Smoothed bytes per second.
    rate: Option<f64>,
}

impl Meter {
    /// A meter for a job of `total` bytes.
    pub fn new(total: u64) -> Self {
        Self {
            total,
            done: 0,
            elapsed: Duration::ZERO,
            last_sample: None,
            rate: None,
        }
    }

    /// Records that `done` bytes were finished `elapsed` after the job started.
    pub fn update(&mut self, done: u64, elapsed: Duration) {
        self.done = done;
        self.elapsed = elapsed;
        let Some((then, before)) = self.last_sample else {
            self.last_sample = Some((elapsed, done));
            return;
        };
        let gap = elapsed.saturating_sub(then);
        if gap < MIN_SAMPLE_GAP {
            return;
        }
        let instant = done.saturating_sub(before) as f64 / gap.as_secs_f64();
        self.rate = Some(match self.rate {
            Some(previous) => previous + SMOOTHING * (instant - previous),
            None => instant,
        });
        self.last_sample = Some((elapsed, done));
    }

    /// Fraction finished, `0.0..=1.0`.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f64 / self.total as f64).clamp(0.0, 1.0) as f32
    }

    /// Smoothed bytes per second, once enough has been seen.
    pub fn rate(&self) -> Option<f64> {
        self.rate.filter(|rate| *rate > 0.0)
    }

    /// Estimated time left, or `None` while it would be a guess.
    pub fn remaining(&self) -> Option<Duration> {
        if self.elapsed < MIN_ELAPSED_FOR_ETA {
            return None;
        }
        let rate = self.rate()?;
        let left = self.total.saturating_sub(self.done) as f64;
        Some(Duration::from_secs_f64(left / rate))
    }

    /// A short status line such as `1.2 GiB of 3.4 GiB · 85.3 MiB/s · about 26 s left`.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} of {}",
            format_bytes(self.done),
            format_bytes(self.total.max(self.done))
        );
        if let Some(rate) = self.rate() {
            text.push_str(&format!(" · {}/s", format_bytes(rate as u64)));
        }
        if let Some(left) = self.remaining() {
            text.push_str(&format!(" · about {} left", format_duration(left)));
        }
        text
    }
}

/// Progress of a batch of albums, weighted by how much each has to write.
#[derive(Debug, Clone)]
pub struct RunProgress {
    weights: HashMap<u64, u64>,
    finished: u64,
    current: u64,
    meter: Meter,
}

impl RunProgress {
    /// One `(album id, bytes to write)` pair per album in the batch.
    pub fn new(albums: impl IntoIterator<Item = (u64, u64)>) -> Self {
        let weights: HashMap<u64, u64> = albums.into_iter().collect();
        let total = weights.values().sum();
        Self {
            weights,
            finished: 0,
            current: 0,
            meter: Meter::new(total),
        }
    }

    /// The album `id` is about to start.
    pub fn begin_album(&mut self, id: u64) {
        self.current = self.weights.get(&id).copied().unwrap_or(0);
    }

    /// The current album is `fraction` (`0.0..=1.0`) written.
    pub fn album_progress(&mut self, fraction: f32, elapsed: Duration) {
        let within = (f64::from(fraction.clamp(0.0, 1.0)) * self.current as f64) as u64;
        self.meter
            .update(self.finished.saturating_add(within), elapsed);
    }

    /// The current album is complete (or abandoned).
    pub fn finish_album(&mut self, elapsed: Duration) {
        self.finished = self.finished.saturating_add(self.current);
        self.current = 0;
        self.meter.update(self.finished, elapsed);
    }

    /// Overall fraction finished, or `None` if the batch has no known size.
    pub fn fraction(&self) -> Option<f32> {
        (self.meter.total > 0).then(|| self.meter.fraction())
    }

    pub fn describe(&self) -> String {
        self.meter.describe()
    }
}

/// `45 s`, `3 min 05 s`, `1 h 04 min`.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs_f64().round() as u64;
    match seconds {
        0..=59 => format!("{seconds} s"),
        60..=3599 => format!("{} min {:02} s", seconds / 60, seconds % 60),
        _ => format!("{} h {:02} min", seconds / 3600, (seconds % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    #[test]
    fn a_steady_rate_gives_the_right_eta() {
        let mut meter = Meter::new(1_000);
        for step in 0..=10u32 {
            meter.update(u64::from(step) * 50, secs(f64::from(step)));
        }
        assert_eq!(meter.rate().map(|r| r.round() as u64), Some(50));
        assert_eq!(meter.remaining().map(|d| d.as_secs()), Some(10));
        assert_eq!(meter.fraction(), 0.5);
    }

    #[test]
    fn no_estimate_is_given_too_early_or_without_progress() {
        let mut meter = Meter::new(1_000);
        meter.update(0, secs(0.0));
        meter.update(100, secs(1.0));
        assert!(meter.remaining().is_none(), "under two seconds");

        let mut stalled = Meter::new(1_000);
        stalled.update(0, secs(0.0));
        stalled.update(0, secs(5.0));
        assert!(stalled.rate().is_none());
        assert!(stalled.remaining().is_none());
    }

    #[test]
    fn the_rate_follows_a_change_of_speed_gradually() {
        let mut meter = Meter::new(1_000_000);
        let mut done = 0u64;
        for second in 1..=10u32 {
            done += 100;
            meter.update(done, secs(f64::from(second)));
        }
        let slow = meter.rate().unwrap_or_default();
        for second in 11..=14u32 {
            done += 1_000;
            meter.update(done, secs(f64::from(second)));
        }
        let faster = meter.rate().unwrap_or_default();
        assert!(faster > slow && faster < 1_000.0, "{slow} -> {faster}");
    }

    #[test]
    fn rapid_samples_are_folded_together() {
        let mut meter = Meter::new(100);
        meter.update(0, secs(0.0));
        meter.update(10, secs(0.05));
        assert!(meter.rate().is_none(), "too close to the first sample");
        meter.update(20, secs(0.3));
        assert!(meter.rate().is_some());
    }

    #[test]
    fn overshoot_and_empty_jobs_do_not_misbehave() {
        let mut meter = Meter::new(100);
        meter.update(150, secs(1.0));
        assert_eq!(meter.fraction(), 1.0);
        assert!(meter.describe().contains("of"));

        let empty = Meter::new(0);
        assert_eq!(empty.fraction(), 0.0);
        assert!(empty.remaining().is_none());
    }

    #[test]
    fn albums_are_weighted_by_size() {
        let mut run = RunProgress::new([(1, 100), (2, 300)]);
        run.begin_album(1);
        run.album_progress(0.5, secs(1.0));
        assert_eq!(run.fraction(), Some(0.125));
        run.finish_album(secs(2.0));
        assert_eq!(run.fraction(), Some(0.25));
        run.begin_album(2);
        run.album_progress(1.0, secs(3.0));
        assert_eq!(run.fraction(), Some(1.0));
    }

    #[test]
    fn unknown_albums_and_empty_batches_are_harmless() {
        let mut run = RunProgress::new([]);
        run.begin_album(9);
        run.album_progress(0.5, secs(1.0));
        run.finish_album(secs(1.0));
        assert_eq!(run.fraction(), None);

        let mut odd = RunProgress::new([(1, 50)]);
        odd.begin_album(7); // not in the batch
        odd.album_progress(1.0, secs(1.0));
        assert_eq!(odd.fraction(), Some(0.0));
    }

    #[test]
    fn durations_are_human_readable() {
        assert_eq!(format_duration(secs(0.4)), "0 s");
        assert_eq!(format_duration(secs(45.0)), "45 s");
        assert_eq!(format_duration(secs(185.0)), "3 min 05 s");
        assert_eq!(format_duration(secs(3_840.0)), "1 h 04 min");
    }

    #[test]
    fn the_description_includes_rate_and_eta_when_known() {
        let mut meter = Meter::new(10 * 1024 * 1024);
        for step in 0..=4u32 {
            meter.update(u64::from(step) * 1024 * 1024, secs(f64::from(step)));
        }
        let text = meter.describe();
        assert!(text.contains("/s"), "{text}");
        assert!(text.contains("left"), "{text}");
    }
}
