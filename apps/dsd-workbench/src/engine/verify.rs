//! Bit-exact comparison of split tracks on disk against their source audio.

use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use super::bits::{self, BitOrder};
use super::slice::{self, DATA_CHUNK_HEADER_LEN, OUT_BLOCK_SIZE, Slice};
use crate::error::{Error, Result};
use crate::metadata::FieldKey;
use crate::plan::{SplitPlan, TrackPlan, target_path};
use crate::snapshot::{self, Loaded};
use crate::source::{self, Container, DsdSource, StreamInfo};

/// Columns in the per-track comparison maps.
pub const BUCKETS: usize = 400;
const CHUNK_BYTES: usize = 64 * 1024;
const LEVEL_WINDOW_BYTES: usize = 128;
const PROBE_BITS: usize = 2048;
const MIN_PROBE_BITS: usize = 128;
const ALIGN_RADIUS: i64 = 65_536;
const MAX_ALIGN_HITS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Identical,
    Differs,
    Missing,
    Unreadable(String),
}

#[derive(Debug, Clone)]
pub struct ChannelResult {
    pub mismatched_bits: u64,
    /// Sample index (within the track) of the first differing bit.
    pub first_mismatch: Option<u64>,
    /// Differing bits per slice of the track.
    pub heat: Vec<u32>,
}

impl ChannelResult {
    fn empty() -> Self {
        Self {
            mismatched_bits: 0,
            first_mismatch: None,
            heat: vec![0; BUCKETS],
        }
    }
}

/// Where the file's audio actually begins, relative to the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alignment {
    /// The file's first sample equals source sample `planned start + shift_samples`.
    pub shift_samples: i64,
    /// False if several positions matched (repetitive audio such as silence).
    pub unique: bool,
}

impl Alignment {
    fn millis(&self, rate: u32) -> f64 {
        self.shift_samples.unsigned_abs() as f64 / f64::from(rate.max(1)) * 1000.0
    }

    pub fn short(&self, rate: u32) -> String {
        let when = if self.shift_samples < 0 {
            "early"
        } else {
            "late"
        };
        format!("starts {:.2} ms {when}", self.millis(rate))
    }

    pub fn describe(&self, rate: u32) -> String {
        let when = if self.shift_samples < 0 {
            "earlier"
        } else {
            "later"
        };
        let ambiguous = if self.unique {
            ""
        } else {
            " Several positions matched, so this is the closest one."
        };
        format!(
            "The file's audio begins {} samples ({:.2} ms) {when} in the source than the plan says. \
             This usually means a different cut position, or a file made by an older block-aligned split.{ambiguous}",
            self.shift_samples.unsigned_abs(),
            self.millis(rate),
        )
    }
}

#[derive(Debug, Clone)]
pub struct TrackCheck {
    pub number: u32,
    pub title: String,
    pub source_file: String,
    pub target: PathBuf,
    pub verdict: Verdict,
    pub rate: u32,
    /// Planned first sample of the track in the source.
    pub start_sample: u64,
    pub expected_samples: u64,
    pub actual_samples: u64,
    pub compared_samples: u64,
    pub channels: Vec<ChannelResult>,
    /// Low-band level per slice, `0.0..=1.0`.
    pub level: Vec<f32>,
    /// Header and tag findings.
    pub issues: Vec<String>,
    pub alignment: Option<Alignment>,
}

impl TrackCheck {
    fn new(plan: &SplitPlan, track: &TrackPlan, target: &Path) -> Self {
        Self {
            number: track.meta.number,
            title: plan.album.resolve(&track.meta, &FieldKey::TITLE).to_owned(),
            source_file: track.source_file.clone(),
            target: target.to_path_buf(),
            verdict: Verdict::Unreadable("not checked".to_owned()),
            rate: 0,
            start_sample: 0,
            expected_samples: 0,
            actual_samples: 0,
            compared_samples: 0,
            channels: Vec::new(),
            level: Vec::new(),
            issues: Vec::new(),
            alignment: None,
        }
    }

    pub fn mismatched_bits(&self) -> u64 {
        self.channels.iter().map(|c| c.mismatched_bits).sum()
    }

    /// Seconds into the track of the first differing bit.
    pub fn first_difference_seconds(&self) -> Option<f64> {
        let first = self
            .channels
            .iter()
            .filter_map(|c| c.first_mismatch)
            .min()?;
        Some(first as f64 / f64::from(self.rate.max(1)))
    }

    /// One short line for tables.
    pub fn note(&self) -> String {
        if let Some(alignment) = &self.alignment {
            return alignment.short(self.rate);
        }
        self.issues.first().cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct AlbumReport {
    /// The plan that was verified (kept to detect later edits).
    pub plan: SplitPlan,
    pub output_dir: PathBuf,
    pub tracks: Vec<TrackCheck>,
    pub seconds: f64,
}

impl AlbumReport {
    fn count(&self, f: impl Fn(&Verdict) -> bool) -> usize {
        self.tracks.iter().filter(|t| f(&t.verdict)).count()
    }

    pub fn identical(&self) -> usize {
        self.count(|v| *v == Verdict::Identical)
    }
    pub fn differs(&self) -> usize {
        self.count(|v| *v == Verdict::Differs)
    }
    pub fn missing(&self) -> usize {
        self.count(|v| *v == Verdict::Missing)
    }
    pub fn unreadable(&self) -> usize {
        self.count(|v| matches!(v, Verdict::Unreadable(_)))
    }
    pub fn issue_count(&self) -> usize {
        self.tracks.iter().map(|t| t.issues.len()).sum()
    }

    /// Tracks that are not identical or have notes.
    pub fn problem_tracks(&self) -> usize {
        self.tracks
            .iter()
            .filter(|t| t.verdict != Verdict::Identical || !t.issues.is_empty())
            .count()
    }

    pub fn is_clean(&self) -> bool {
        !self.tracks.is_empty() && self.problem_tracks() == 0
    }

    /// Audio bytes that were compared.
    pub fn compared_bytes(&self) -> u64 {
        self.tracks
            .iter()
            .map(|t| t.compared_samples / 8 * t.channels.len() as u64)
            .sum()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VerifyProgress {
    pub index: usize,
    pub total: usize,
    /// Progress within the current track.
    pub fraction: f32,
}

impl VerifyProgress {
    /// Progress through the whole album.
    pub fn overall(&self) -> f32 {
        (self.index as f32 + self.fraction) / self.total.max(1) as f32
    }
}

struct Ctx<'a> {
    base_dir: &'a Path,
    plan: &'a SplitPlan,
    cancel: &'a AtomicBool,
}

/// Compares every track of `plan` in `output_dir` with its source. Problems with
/// individual files become verdicts; only cancellation is an error.
pub fn verify_album(
    base_dir: &Path,
    plan: &SplitPlan,
    output_dir: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(VerifyProgress),
) -> Result<AlbumReport> {
    let started = Instant::now();
    let total = plan.tracks.len();
    let ctx = Ctx {
        base_dir,
        plan,
        cancel,
    };
    let mut tracks = Vec::with_capacity(total);

    for (index, track) in plan.tracks.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let mut step = |fraction: f32| {
            on_progress(VerifyProgress {
                index,
                total,
                fraction,
            })
        };
        tracks.push(check_track(&ctx, index, track, output_dir, &mut step)?);
    }
    on_progress(VerifyProgress {
        index: total,
        total,
        fraction: 0.0,
    });

    Ok(AlbumReport {
        plan: plan.clone(),
        output_dir: output_dir.to_path_buf(),
        tracks,
        seconds: started.elapsed().as_secs_f64(),
    })
}

fn check_track(
    ctx: &Ctx<'_>,
    index: usize,
    track: &TrackPlan,
    output_dir: &Path,
    progress: &mut dyn FnMut(f32),
) -> Result<TrackCheck> {
    let target = output_dir.join(target_path(&ctx.plan.naming, &ctx.plan.album, track));
    let mut check = TrackCheck::new(ctx.plan, track, &target);
    if !target.is_file() {
        check.verdict = Verdict::Missing;
        return Ok(check);
    }
    match fill(ctx, index, track, &target, &mut check, progress) {
        Ok(()) => Ok(check),
        Err(Error::Cancelled) => Err(Error::Cancelled),
        Err(err) => {
            check.verdict = Verdict::Unreadable(err.to_string());
            Ok(check)
        }
    }
}

fn fill(
    ctx: &Ctx<'_>,
    index: usize,
    track: &TrackPlan,
    target: &Path,
    check: &mut TrackCheck,
    progress: &mut dyn FnMut(f32),
) -> Result<()> {
    let mut src = source::open(&ctx.base_dir.join(&track.source_file))?;
    let src_info = src.info().clone();
    let slice = slice::compute(
        &src_info,
        OUT_BLOCK_SIZE,
        track.start_seconds,
        ctx.plan.duration_of(index),
    )?;
    let mut out = source::open(target)?;
    let out_info = out.info().clone();

    check.rate = src_info.sample_rate;
    check.start_sample = slice.start_sample;
    check.expected_samples = slice.sample_count;
    check.actual_samples = out_info.sample_count;

    header_checks(check, target, &src_info, &out_info, &slice)?;
    check.issues.extend(tag_issues(ctx.plan, track, target));

    let channels = src_info.channels.min(out_info.channels) as usize;
    let count = slice.sample_count.min(out_info.sample_count);
    let compared = compare(
        src.as_mut(),
        out.as_mut(),
        slice.start_sample,
        count,
        channels,
        ctx.cancel,
        progress,
    )?;
    check.compared_samples = count;
    check.channels = compared.channels;
    check.level = compared.level;

    let mismatched = check.channels.iter().any(|c| c.mismatched_bits > 0);
    let same_shape = slice.sample_count == out_info.sample_count
        && src_info.channels == out_info.channels
        && src_info.sample_rate == out_info.sample_rate;
    check.verdict = if mismatched || !same_shape {
        Verdict::Differs
    } else {
        Verdict::Identical
    };

    if mismatched {
        let starts_wrong = check
            .channels
            .first()
            .and_then(|c| c.first_mismatch)
            .is_some_and(|s| s < PROBE_BITS as u64);
        if starts_wrong {
            check.alignment = find_alignment(
                src.as_mut(),
                out.as_mut(),
                slice.start_sample,
                out_info.sample_count,
            )?;
        }
    }
    Ok(())
}

fn header_checks(
    check: &mut TrackCheck,
    target: &Path,
    src: &StreamInfo,
    out: &StreamInfo,
    slice: &Slice,
) -> Result<()> {
    if out.container != Container::Dsf {
        check.issues.push("The file is not a DSF file.".to_owned());
        return Ok(());
    }
    if out.sample_rate != src.sample_rate {
        check.issues.push(format!(
            "Sample rate: the file is {} Hz, the source is {} Hz.",
            out.sample_rate, src.sample_rate
        ));
    }
    if out.channels != src.channels {
        check.issues.push(format!(
            "Channels: the file has {}, the source has {}.",
            out.channels, src.channels
        ));
    }
    if out.sample_count != slice.sample_count {
        let diff = i128::from(out.sample_count) - i128::from(slice.sample_count);
        let ms = diff.unsigned_abs() as f64 / f64::from(src.sample_rate.max(1)) * 1000.0;
        check.issues.push(format!(
            "Length: the file has {} samples, the plan expects {} ({}{} samples, {ms:.2} ms).",
            out.sample_count,
            slice.sample_count,
            if diff > 0 { "+" } else { "-" },
            diff.unsigned_abs()
        ));
    }

    let ctx = format!("reading {}", target.display());
    let len = fs::metadata(target).map_err(Error::io(ctx.clone()))?.len();
    let file = File::open(target).map_err(Error::io(ctx))?;
    let header = dsf::DsfHeader::read(BufReader::new(file))?;
    if header.total_file_size != len {
        check.issues.push(format!(
            "Header: records {} bytes but the file is {len} bytes.",
            header.total_file_size
        ));
    }
    if header.sample_count == slice.sample_count
        && header.data_chunk_size != DATA_CHUNK_HEADER_LEN + slice.audio_len
    {
        check
            .issues
            .push("Header: the audio chunk size does not match the track length.".to_owned());
    }
    Ok(())
}

/// How the tags of the file at `target` differ from what `plan` says `track` should have.
/// An empty list means the file's tags match.
pub(super) fn tag_issues(plan: &SplitPlan, track: &TrackPlan, target: &Path) -> Vec<String> {
    let mut issues = Vec::new();
    let quote = |s: &str| {
        if s.is_empty() {
            "nothing".to_owned()
        } else {
            format!("'{s}'")
        }
    };
    match snapshot::load(target) {
        Loaded::Tags(found) => {
            let mut rows: Vec<(String, String, String)> = plan
                .tag_keys(track)
                .into_iter()
                .map(|key| {
                    let wanted = plan.album.resolve(&track.meta, &key);
                    let wanted = if wanted.trim().is_empty() { "" } else { wanted };
                    let name = key.label().to_lowercase();
                    (name, found.value(&key).to_owned(), wanted.to_owned())
                })
                .collect();
            rows.push((
                "track".to_owned(),
                found.track.clone(),
                format!("{}/{}", track.meta.number, plan.track_total()),
            ));
            for (name, have, wanted) in rows {
                if have != wanted {
                    issues.push(format!(
                        "Tag {name}: the file has {}, the plan says {}.",
                        quote(&have),
                        quote(&wanted)
                    ));
                }
            }
            let embeds = plan.cover.as_ref().is_some_and(|c| c.embed);
            if embeds && !found.has_cover {
                issues.push("The plan embeds cover art, but the file has none.".to_owned());
            }
        }
        Loaded::Untagged => issues.push("The file has no ID3 tag.".to_owned()),
        Loaded::Failed(err) => issues.push(format!("Tags could not be read: {err}")),
        Loaded::Missing => {}
    }
    issues
}

struct Compared {
    channels: Vec<ChannelResult>,
    level: Vec<f32>,
}

fn bucket_of(byte_index: u64, count: u64) -> usize {
    let scaled = byte_index.saturating_mul(8).saturating_mul(BUCKETS as u64) / count.max(1);
    scaled.min(BUCKETS as u64 - 1) as usize
}

/// Converts a channel stream to the canonical LSB-first bit order.
fn canonical(bytes: &mut [u8], msb_first: bool) {
    if msb_first {
        bytes.iter_mut().for_each(|b| *b = b.reverse_bits());
    }
}

fn compare(
    src: &mut dyn DsdSource,
    out: &mut dyn DsdSource,
    src_start: u64,
    count: u64,
    channels: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Compared> {
    let src_msb = src.info().msb_first;
    let src_order = BitOrder::from_msb(src_msb);
    let out_msb = out.info().msb_first;

    let mut results: Vec<ChannelResult> = (0..channels).map(|_| ChannelResult::empty()).collect();
    let mut level_sq = vec![0f64; BUCKETS];
    let mut level_n = vec![0u32; BUCKETS];

    let total_bytes = count.div_ceil(8);
    let (start_byte, shift) = (src_start / 8, (src_start % 8) as u32);
    let mut done = 0u64;

    while done < total_bytes {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let n = (CHUNK_BYTES as u64).min(total_bytes - done) as usize;
        let first = i64::try_from(start_byte + done)
            .map_err(|_| Error::Slice("source offset overflows".into()))?;
        let s = src
            .read(first, n + 1)
            .map_err(Error::io("reading source audio"))?;
        let o = out
            .read(done as i64, n)
            .map_err(Error::io("reading output audio"))?;
        let last = done + n as u64 == total_bytes;

        for (ch, result) in results.iter_mut().enumerate() {
            let (Some(window), Some(written)) = (s.get(ch), o.get(ch)) else {
                continue;
            };
            let mut want = vec![0u8; n];
            bits::shift_into(window, shift, src_order, &mut want);
            canonical(&mut want, src_msb);
            let mut got = written.clone();
            canonical(&mut got, out_msb);
            if last {
                bits::clear_tail(&mut want, done, count, BitOrder::LsbFirst);
                bits::clear_tail(&mut got, done, count, BitOrder::LsbFirst);
            }

            for (i, (a, b)) in want.iter().zip(&got).enumerate() {
                let diff = a ^ b;
                if diff == 0 {
                    continue;
                }
                let byte = done + i as u64;
                result.mismatched_bits += u64::from(diff.count_ones());
                if result.first_mismatch.is_none() {
                    result.first_mismatch = Some(byte * 8 + u64::from(diff.trailing_zeros()));
                }
                if let Some(cell) = result.heat.get_mut(bucket_of(byte, count)) {
                    *cell = cell.saturating_add(diff.count_ones());
                }
            }

            if ch == 0 {
                for (w, block) in got.chunks(LEVEL_WINDOW_BYTES).enumerate() {
                    let ones: u32 = block.iter().map(|b| b.count_ones()).sum();
                    let density = f64::from(ones) / (block.len() * 8) as f64;
                    let x = 2.0 * density - 1.0;
                    let bucket = bucket_of(done + (w * LEVEL_WINDOW_BYTES) as u64, count);
                    if let (Some(sq), Some(cnt)) =
                        (level_sq.get_mut(bucket), level_n.get_mut(bucket))
                    {
                        *sq += x * x;
                        *cnt += 1;
                    }
                }
            }
        }
        done += n as u64;
        progress(done as f32 / total_bytes as f32);
    }

    let level = level_sq
        .iter()
        .zip(&level_n)
        .map(|(sq, n)| {
            if *n > 0 {
                (sq / f64::from(*n)).sqrt() as f32
            } else {
                0.0
            }
        })
        .collect();
    Ok(Compared {
        channels: results,
        level,
    })
}

/// 64 bits of an LSB-first bit string starting at bit `pos`.
fn word(bytes: &[u8], pos: usize) -> u64 {
    let byte = pos / 8;
    let mut v: u128 = 0;
    for k in 0..9 {
        v |= u128::from(bytes.get(byte + k).copied().unwrap_or(0)) << (8 * k);
    }
    (v >> (pos % 8)) as u64
}

/// Looks for where the file's first samples occur in the source near the planned start.
fn find_alignment(
    src: &mut dyn DsdSource,
    out: &mut dyn DsdSource,
    expected_start: u64,
    actual_samples: u64,
) -> Result<Option<Alignment>> {
    let probe_bits = (actual_samples.min(PROBE_BITS as u64) as usize / 64) * 64;
    if probe_bits < MIN_PROBE_BITS {
        return Ok(None);
    }
    let no_data = || Error::Slice("no channel data for the alignment search".into());

    let out_msb = out.info().msb_first;
    let mut probe = out
        .read(0, probe_bits / 8)
        .map_err(Error::io("reading output audio"))?
        .into_iter()
        .next()
        .ok_or_else(no_data)?;
    canonical(&mut probe, out_msb);

    let first_byte = (expected_start as i64 - ALIGN_RADIUS).div_euclid(8);
    let len = ((2 * ALIGN_RADIUS) as usize + probe_bits) / 8 + 2;
    let src_msb = src.info().msb_first;
    let total = src.info().sample_count;
    let mut window = src
        .read(first_byte, len)
        .map_err(Error::io("reading source audio"))?
        .into_iter()
        .next()
        .ok_or_else(no_data)?;
    canonical(&mut window, src_msb);
    let base = first_byte * 8;

    let words = probe_bits / 64;
    let mut hits: Vec<i64> = Vec::new();
    for step in 0..=(2 * ALIGN_RADIUS) {
        // Candidates in order of distance: 0, -1, +1, -2, +2, ...
        let d = if step % 2 == 0 {
            step / 2
        } else {
            -((step + 1) / 2)
        };
        if d == 0 {
            continue;
        }
        let pos = expected_start as i64 + d;
        if pos < 0 || pos as u64 + probe_bits as u64 > total {
            continue;
        }
        let idx = (pos - base) as usize;
        if (0..words).all(|w| word(&window, idx + 64 * w) == word(&probe, 64 * w)) {
            hits.push(d);
            if hits.len() >= MAX_ALIGN_HITS {
                break;
            }
        }
    }
    Ok(hits.first().map(|d| Alignment {
        shift_samples: *d,
        unique: hits.len() == 1,
    }))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::super::splitter::SplitEngine;
    use super::*;
    use crate::metadata::Fields;
    use crate::naming::Naming;
    use crate::plan::{AlbumMeta, TrackMeta};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    /// One block is 32768 samples per channel, so at this rate 1 s == 1 block.
    const RATE: u32 = 32_768;
    const BLOCKS: u64 = 4;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsd_workbench_verify_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn write_noise_source(path: &Path) -> TestResult {
        let audio = (BLOCKS * 4096 * 2) as usize;
        let mut x = 12_345u32;
        let data: Vec<u8> = (0..audio)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect();
        let header = dsf::DsfHeader {
            total_file_size: dsf::AUDIO_OFFSET + audio as u64,
            metadata_offset: 0,
            channel_type: 2,
            channel_count: 2,
            sample_rate: RATE,
            bits_per_sample: 1,
            sample_count: BLOCKS * 32_768,
            block_size: 4096,
            data_chunk_size: DATA_CHUNK_HEADER_LEN + audio as u64,
        };
        let mut file = File::create(path)?;
        header.write(&mut file)?;
        file.write_all(&data)?;
        Ok(())
    }

    fn plan(starts: &[f64]) -> SplitPlan {
        let names = ["One", "Two", "Three"];
        let mut fields = Fields::default();
        fields.set(FieldKey::ALBUM, "Album");
        fields.set(FieldKey::ALBUM_ARTIST, "Artist");
        SplitPlan {
            album: AlbumMeta { fields },
            naming: Naming::default(),
            cover: None,
            tracks: starts
                .iter()
                .enumerate()
                .map(|(i, start)| TrackPlan {
                    source_file: "side.dsf".to_owned(),
                    start_seconds: *start,
                    meta: TrackMeta::new(i as u32 + 1)
                        .with(FieldKey::TITLE, names.get(i).copied().unwrap_or("Extra"))
                        .with(FieldKey::ARTIST, "Artist"),
                })
                .collect(),
        }
    }

    fn split(dir: &Path, plan: &SplitPlan) -> TestResult {
        write_noise_source(&dir.join("side.dsf"))?;
        SplitEngine::new(dir, plan.clone(), dir.join("out"), false)
            .execute(&AtomicBool::new(false), &mut |_| {})?;
        Ok(())
    }

    fn verify(dir: &Path, plan: &SplitPlan) -> Result<AlbumReport> {
        verify_album(
            dir,
            plan,
            &dir.join("out"),
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }

    #[test]
    fn a_fresh_split_verifies_clean() -> TestResult {
        let dir = temp("clean");
        fs::create_dir_all(&dir)?;
        let plan = plan(&[0.0, 1.5]);
        split(&dir, &plan)?;

        let report = verify(&dir, &plan)?;
        assert!(
            report.is_clean(),
            "{:?}",
            report.tracks.iter().map(|t| &t.issues).collect::<Vec<_>>()
        );
        assert_eq!(report.identical(), 2);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_flipped_byte_is_located() -> TestResult {
        let dir = temp("flip");
        fs::create_dir_all(&dir)?;
        let plan = plan(&[0.0, 1.5]);
        split(&dir, &plan)?;

        let path = dir.join("out").join("01 - One.dsf");
        let mut bytes = fs::read(&path)?;
        let at = dsf::AUDIO_OFFSET as usize + 100; // channel 0, byte 100 of block 0
        *bytes.get_mut(at).ok_or("offset")? ^= 0xFF;
        fs::write(&path, bytes)?;

        let report = verify(&dir, &plan)?;
        let first = report.tracks.first().ok_or("track")?;
        assert_eq!(first.verdict, Verdict::Differs);
        assert_eq!(first.mismatched_bits(), 8);
        assert_eq!(
            first.channels.first().and_then(|c| c.first_mismatch),
            Some(800)
        );
        assert_eq!(
            report.tracks.get(1).map(|t| &t.verdict),
            Some(&Verdict::Identical)
        );
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_shifted_start_is_measured() -> TestResult {
        let dir = temp("shift");
        fs::create_dir_all(&dir)?;
        split(&dir, &plan(&[1.0]))?;

        // Verify against a plan that starts half a second later.
        let report = verify(&dir, &plan(&[1.5]))?;
        let track = report.tracks.first().ok_or("track")?;
        assert_eq!(track.verdict, Verdict::Differs);
        assert_eq!(
            track.alignment,
            Some(Alignment {
                shift_samples: -16_384,
                unique: true
            })
        );
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn missing_files_are_reported() -> TestResult {
        let dir = temp("missing");
        fs::create_dir_all(&dir)?;
        write_noise_source(&dir.join("side.dsf"))?;
        let report = verify(&dir, &plan(&[0.0]))?;
        assert_eq!(report.missing(), 1);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
