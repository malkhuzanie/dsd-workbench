//! Drives the real binary on synthetic DSF and DSDIFF sources and checks every
//! output bit against an independent reference, so a bug shared by the splitter
//! and the built-in verifier cannot hide.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A deliberately awkward rate: cue-sheet times land on non-byte-aligned samples.
const RATE: u32 = 2_000_001;
const FRAMES_PER_SECOND: u64 = 75;
const BLOCK: usize = 4096;
const DSF_AUDIO_OFFSET: usize = 92;

/// CUE `INDEX 01` positions in frames (1/75 s).
const CUTS: [u64; 3] = [0, 88, 191];
const TITLES: [&str; 3] = ["One", "Two", "Three"];

#[derive(Clone, Copy, Debug)]
enum Format {
    Dsf,
    Dsdiff,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Dsf => "dsf",
            Self::Dsdiff => "dff",
        }
    }
}

/// Pseudo-random bytes (an LCG) so every bit position carries distinct data.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

/// Sample `index` of a stream stored with the first sample in the least significant bit.
fn bit(stream: &[u8], index: u64) -> u8 {
    let byte = stream.get((index / 8) as usize).copied().unwrap_or(0);
    (byte >> (index % 8)) & 1
}

fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

fn le64(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

/// Source audio, one time-ordered LSB-first byte stream per channel.
struct Source {
    streams: Vec<Vec<u8>>,
    samples: u64,
}

impl Source {
    fn new(channels: usize, samples: u64) -> Self {
        let bytes = samples.div_ceil(8) as usize;
        let mut streams: Vec<Vec<u8>> =
            (0..channels).map(|c| noise(bytes, 17 + c as u32)).collect();
        // Bits past the end are padding in the file; keep the reference zero there.
        let tail = (samples % 8) as u32;
        if tail > 0 {
            for stream in &mut streams {
                if let Some(last) = stream.last_mut() {
                    *last &= (1u8 << tail) - 1;
                }
            }
        }
        Self { streams, samples }
    }

    /// Alternating music (noise) and silence (the DSD idle pattern), both channels.
    /// Each segment is `(loud, seconds)`; lengths are whole bytes so they tile exactly.
    fn songs(segments: &[(bool, f64)]) -> Self {
        const IDLE: u8 = 0x69;
        let mut stream = Vec::new();
        for (index, (loud, seconds)) in segments.iter().enumerate() {
            let bytes = (seconds * f64::from(RATE) / 8.0) as usize;
            if *loud {
                stream.extend(noise(bytes, 100 + index as u32));
            } else {
                stream.extend(vec![IDLE; bytes]);
            }
        }
        let samples = stream.len() as u64 * 8;
        Self {
            streams: vec![stream.clone(), stream],
            samples,
        }
    }

    fn write(&self, path: &Path, format: Format) -> TestResult {
        let bytes = match format {
            Format::Dsf => self.dsf(),
            Format::Dsdiff => self.dsdiff(),
        };
        fs::write(path, bytes)?;
        Ok(())
    }

    fn dsf(&self) -> Vec<u8> {
        let channels = self.streams.len();
        let blocks = (self.samples.div_ceil(8) as usize).div_ceil(BLOCK);
        let audio_len = (blocks * BLOCK * channels) as u64;

        let mut out = Vec::new();
        out.extend(b"DSD ");
        out.extend(le64(28));
        out.extend(le64(DSF_AUDIO_OFFSET as u64 + audio_len));
        out.extend(le64(0));
        out.extend(b"fmt ");
        out.extend(le64(52));
        out.extend(le32(1));
        out.extend(le32(0));
        out.extend(le32(if channels == 1 { 1 } else { 2 }));
        out.extend(le32(channels as u32));
        out.extend(le32(RATE));
        out.extend(le32(1)); // LSB first
        out.extend(le64(self.samples));
        out.extend(le32(BLOCK as u32));
        out.extend(le32(0));
        out.extend(b"data");
        out.extend(le64(12 + audio_len));
        for block in 0..blocks {
            for stream in &self.streams {
                let from = block * BLOCK;
                for i in from..from + BLOCK {
                    out.push(stream.get(i).copied().unwrap_or(0));
                }
            }
        }
        out
    }

    fn dsdiff(&self) -> Vec<u8> {
        fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = id.to_vec();
            v.extend((data.len() as u64).to_be_bytes());
            v.extend(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }

        let channels = self.streams.len();
        let mut chnl = (channels as u16).to_be_bytes().to_vec();
        for c in 0..channels {
            chnl.extend(format!("C{c:03}").bytes());
        }
        let mut cmpr = b"DSD ".to_vec();
        cmpr.push(14);
        cmpr.extend(b"not compressed");

        let mut prop = b"SND ".to_vec();
        prop.extend(chunk(b"FS  ", &RATE.to_be_bytes()));
        prop.extend(chunk(b"CHNL", &chnl));
        prop.extend(chunk(b"CMPR", &cmpr));

        // DSDIFF is byte-interleaved with the first sample in the most significant bit.
        let bytes = (self.samples / 8) as usize;
        let mut audio = Vec::with_capacity(bytes * channels);
        for i in 0..bytes {
            for stream in &self.streams {
                audio.push(stream.get(i).copied().unwrap_or(0).reverse_bits());
            }
        }

        let mut body = b"DSD ".to_vec();
        body.extend(chunk(b"FVER", &[1, 5, 0, 0]));
        body.extend(chunk(b"PROP", &prop));
        body.extend(chunk(b"DSD ", &audio));

        let mut out = b"FRM8".to_vec();
        out.extend((body.len() as u64).to_be_bytes());
        out.extend(body);
        out
    }
}

fn cue_sheet(file: &str) -> String {
    let mut text = format!("PERFORMER \"Tester\"\nTITLE \"Synthetic\"\nFILE \"{file}\" WAVE\n");
    for (i, (frames, title)) in CUTS.iter().zip(TITLES).enumerate() {
        let seconds = frames / FRAMES_PER_SECOND;
        let frame = frames % FRAMES_PER_SECOND;
        text.push_str(&format!(
            "  TRACK {:02} AUDIO\n    TITLE \"{title}\"\n    INDEX 01 {:02}:{:02}:{:02}\n",
            i + 1,
            seconds / 60,
            seconds % 60,
            frame
        ));
    }
    text
}

/// First sample of each track, rounded to the nearest sample as the CUE spec implies.
fn expected_starts() -> Vec<u64> {
    CUTS.iter().map(|frames| sample_at(*frames)).collect()
}

/// The sample nearest to a position given in CUE frames.
fn sample_at(frames: u64) -> u64 {
    (2 * frames * u64::from(RATE) + FRAMES_PER_SECOND) / (2 * FRAMES_PER_SECOND)
}

fn timecode(frames: u64) -> String {
    let seconds = frames / FRAMES_PER_SECOND;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 60,
        seconds % 60,
        frames % FRAMES_PER_SECOND
    )
}

fn run(args: &[&str], cwd: &Path) -> Result<Output, std::io::Error> {
    Command::new(env!("CARGO_BIN_EXE_dsd-workbench"))
        .args(args)
        .current_dir(cwd)
        .output()
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, Box<dyn std::error::Error>> {
    Ok(u32::from_le_bytes(
        bytes.get(at..at + 4).ok_or("short")?.try_into()?,
    ))
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64, Box<dyn std::error::Error>> {
    Ok(u64::from_le_bytes(
        bytes.get(at..at + 8).ok_or("short")?.try_into()?,
    ))
}

/// Checks one output file against the reference, bit by bit.
fn check_track(path: &Path, source: &Source, start: u64, count: u64) -> TestResult {
    let bytes = fs::read(path)?;
    let channels = source.streams.len();
    assert_eq!(bytes.get(..4), Some(&b"DSD "[..]), "{path:?}");
    assert_eq!(
        read_u32(&bytes, 52)? as usize,
        channels,
        "{path:?} channels"
    );
    assert_eq!(read_u32(&bytes, 56)?, RATE, "{path:?} rate");
    assert_eq!(read_u32(&bytes, 60)?, 1, "{path:?} must be LSB first");
    assert_eq!(read_u64(&bytes, 64)?, count, "{path:?} sample count");
    assert_eq!(read_u32(&bytes, 72)? as usize, BLOCK, "{path:?} block size");

    let blocks = (count.div_ceil(8) as usize).div_ceil(BLOCK);
    let audio = bytes
        .get(DSF_AUDIO_OFFSET..DSF_AUDIO_OFFSET + blocks * BLOCK * channels)
        .ok_or("audio shorter than the header claims")?;

    for (c, reference) in source.streams.iter().enumerate() {
        let mut channel = Vec::with_capacity(blocks * BLOCK);
        for b in 0..blocks {
            let from = (b * channels + c) * BLOCK;
            channel.extend_from_slice(audio.get(from..from + BLOCK).ok_or("short block")?);
        }
        for j in 0..blocks as u64 * BLOCK as u64 * 8 {
            let want = if j < count {
                bit(reference, start + j)
            } else {
                0
            };
            if bit(&channel, j) != want {
                return Err(format!("{path:?} channel {c} sample {j} differs").into());
            }
        }
    }
    Ok(())
}

fn split_and_check(format: Format, channels: usize) -> TestResult {
    let dir = tempfile::tempdir()?;
    let samples = match format {
        Format::Dsf => 6_000_003,
        Format::Dsdiff => 6_000_000,
    };
    let source = Source::new(channels, samples);
    let file = format!("side.{}", format.extension());
    source.write(&dir.path().join(&file), format)?;
    fs::write(dir.path().join("album.cue"), cue_sheet(&file))?;

    let out = run(&["album.cue", "--out", "out", "--verify"], dir.path())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{format:?} x{channels}: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("bit-identical"), "{stdout}");

    let starts = expected_starts();
    let mut ends = starts.iter().skip(1).copied().collect::<Vec<_>>();
    ends.push(samples);
    for (i, ((start, end), title)) in starts.iter().zip(&ends).zip(TITLES).enumerate() {
        let path: PathBuf = dir
            .path()
            .join("out")
            .join(format!("{:02} - {title}.dsf", i + 1));
        check_track(&path, &source, *start, end - start)?;
    }
    Ok(())
}

#[test]
fn dsf_stereo_splits_bit_exactly() -> TestResult {
    split_and_check(Format::Dsf, 2)
}

#[test]
fn dsf_mono_splits_bit_exactly() -> TestResult {
    split_and_check(Format::Dsf, 1)
}

#[test]
fn dsf_multichannel_splits_bit_exactly() -> TestResult {
    split_and_check(Format::Dsf, 6)
}

#[test]
fn dsdiff_stereo_is_converted_bit_exactly() -> TestResult {
    split_and_check(Format::Dsdiff, 2)
}

#[test]
fn dsdiff_multichannel_is_converted_bit_exactly() -> TestResult {
    split_and_check(Format::Dsdiff, 6)
}

#[test]
fn verify_only_flags_a_corrupted_output() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 6_000_000);
    source.write(&dir.path().join("side.dsf"), Format::Dsf)?;
    fs::write(dir.path().join("album.cue"), cue_sheet("side.dsf"))?;

    let split = run(&["album.cue", "--out", "out"], dir.path())?;
    assert!(split.status.success());
    let clean = run(&["album.cue", "--out", "out", "--verify-only"], dir.path())?;
    assert!(clean.status.success(), "a fresh split must verify cleanly");

    let victim = dir.path().join("out").join("02 - Two.dsf");
    let mut bytes = fs::read(&victim)?;
    let target = bytes
        .get_mut(DSF_AUDIO_OFFSET + 1000)
        .ok_or("file too short")?;
    *target ^= 0x10;
    fs::write(&victim, bytes)?;

    let broken = run(&["album.cue", "--out", "out", "--verify-only"], dir.path())?;
    assert!(
        !broken.status.success(),
        "one flipped bit must fail verification"
    );
    assert!(String::from_utf8_lossy(&broken.stdout).contains("DIFFERS"));
    Ok(())
}

#[test]
fn hidden_track_and_prepended_pregaps_land_on_the_right_samples() -> TestResult {
    let dir = tempfile::tempdir()?;
    let samples = 6_000_003;
    let source = Source::new(2, samples);
    source.write(&dir.path().join("side.dsf"), Format::Dsf)?;
    // Track 1 starts 1.2 s in (a hidden track precedes it); track 2 has a pregap.
    let cue = format!(
        "FILE \"side.dsf\" WAVE\n\
         TRACK 01 AUDIO\n  TITLE \"One\"\n  INDEX 01 {}\n\
         TRACK 02 AUDIO\n  TITLE \"Two\"\n  INDEX 00 {}\n  INDEX 01 {}\n",
        timecode(90),
        timecode(180),
        timecode(191),
    );
    fs::write(dir.path().join("album.cue"), cue)?;

    let out = run(
        &[
            "album.cue",
            "--out",
            "out",
            "--verify",
            "--hidden-track",
            "--gap",
            "prepend",
        ],
        dir.path(),
    )?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let bounds = [0, sample_at(90), sample_at(180), samples];
    let names = ["00 - Hidden Track.dsf", "01 - One.dsf", "02 - Two.dsf"];
    for (i, name) in names.iter().enumerate() {
        let (start, end) = (bounds[i], bounds[i + 1]);
        check_track(
            &dir.path().join("out").join(name),
            &source,
            start,
            end - start,
        )?;
    }
    Ok(())
}

#[test]
fn a_wav_cue_finds_its_dsf_with_a_different_case() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 6_000_003);
    source.write(&dir.path().join("SIDE.DSF"), Format::Dsf)?;
    fs::write(dir.path().join("album.cue"), cue_sheet("side.wav"))?;

    let out = run(&["album.cue", "--out", "out", "--verify"], dir.path())?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("bit-identical"));
    Ok(())
}

#[test]
fn name_and_folder_templates_place_files_and_still_verify() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 6_000_003);
    source.write(&dir.path().join("side.dsf"), Format::Dsf)?;
    fs::write(dir.path().join("album.cue"), cue_sheet("side.dsf"))?;

    let out = run(
        &[
            "album.cue",
            "--out",
            "out",
            "--verify",
            "--folder",
            "{albumartist}/{album}",
            "--name",
            "{track:02}. {title}",
        ],
        dir.path(),
    )?;
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let album = dir.path().join("out").join("Tester").join("Synthetic");
    let starts = expected_starts();
    let mut ends: Vec<u64> = starts.iter().skip(1).copied().collect();
    ends.push(6_000_003);
    for (i, ((start, end), title)) in starts.iter().zip(&ends).zip(TITLES).enumerate() {
        let path = album.join(format!("{:02}. {title}.dsf", i + 1));
        check_track(&path, &source, *start, end - start)?;
    }
    Ok(())
}

#[test]
fn an_invalid_template_is_refused_before_anything_is_written() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 6_000_003);
    source.write(&dir.path().join("side.dsf"), Format::Dsf)?;
    fs::write(dir.path().join("album.cue"), cue_sheet("side.dsf"))?;

    let out = run(
        &["album.cue", "--out", "out", "--name", "{nope}"],
        dir.path(),
    )?;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown placeholder"));
    assert!(!dir.path().join("out").exists());
    Ok(())
}

/// A JPEG-looking blob; only the magic bytes matter to the tool.
const FAKE_JPEG: [u8; 10] = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn cover_fixture() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 6_000_003);
    source.write(&dir.path().join("side.dsf"), Format::Dsf)?;
    fs::write(dir.path().join("album.cue"), cue_sheet("side.dsf"))?;
    fs::write(dir.path().join("Cover.JPG"), FAKE_JPEG)?;
    Ok(dir)
}

#[test]
fn a_cover_next_to_the_cue_is_embedded_and_verified() -> TestResult {
    let dir = cover_fixture()?;
    let out = run(&["album.cue", "--out", "out", "--verify"], dir.path())?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    for title in TITLES.iter().enumerate() {
        let path = dir
            .path()
            .join("out")
            .join(format!("{:02} - {}.dsf", title.0 + 1, title.1));
        let bytes = fs::read(path)?;
        assert!(
            contains(&bytes, b"APIC"),
            "{} has no picture frame",
            title.1
        );
        assert!(
            contains(&bytes, &FAKE_JPEG),
            "{} lost the image bytes",
            title.1
        );
    }
    assert!(!dir.path().join("out").join("folder.jpg").exists());
    Ok(())
}

#[test]
fn copy_cover_and_no_cover_are_honoured() -> TestResult {
    let dir = cover_fixture()?;
    let out = run(
        &["album.cue", "--out", "copied", "--copy-cover"],
        dir.path(),
    )?;
    assert!(out.status.success());
    assert_eq!(
        fs::read(dir.path().join("copied").join("folder.jpg"))?,
        FAKE_JPEG
    );

    let out = run(&["album.cue", "--out", "bare", "--no-cover"], dir.path())?;
    assert!(out.status.success());
    let bytes = fs::read(dir.path().join("bare").join("01 - One.dsf"))?;
    assert!(!contains(&bytes, b"APIC"));
    assert!(!dir.path().join("bare").join("folder.jpg").exists());
    Ok(())
}

#[test]
fn a_cover_that_is_not_an_image_stops_the_run_before_writing() -> TestResult {
    let dir = cover_fixture()?;
    fs::write(dir.path().join("bad.jpg"), b"definitely not an image")?;
    let out = run(
        &["album.cue", "--out", "out", "--cover", "bad.jpg"],
        dir.path(),
    )?;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a JPEG or PNG"));
    assert!(!dir.path().join("out").join("01 - One.dsf").exists());
    Ok(())
}

#[test]
fn tag_only_retitles_existing_files_without_touching_audio() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 6_000_003);
    source.write(&dir.path().join("side.dsf"), Format::Dsf)?;
    fs::write(dir.path().join("album.cue"), cue_sheet("side.dsf"))?;
    let split = run(&["album.cue", "--out", "out", "--no-cover"], dir.path())?;
    assert!(split.status.success());

    let file = dir.path().join("out").join("01 - One.dsf");
    let before = fs::read(&file)?;
    let audio_len = {
        let blocks = (source_len(&before)?).div_ceil(8).div_ceil(BLOCK as u64);
        (blocks as usize) * BLOCK * 2
    };

    // The user fixes a title in the CUE sheet; the files keep their old names.
    let cue = fs::read_to_string(dir.path().join("album.cue"))?.replace("\"One\"", "\"Uno\"");
    fs::write(dir.path().join("album.cue"), cue)?;

    let preview = run(
        &["album.cue", "--out", "out", "--tag-only", "--dry-run"],
        dir.path(),
    )?;
    assert!(preview.status.success());
    assert!(String::from_utf8_lossy(&preview.stdout).contains("would tag"));
    assert_eq!(fs::read(&file)?, before, "a dry run must not write");

    let real = run(&["album.cue", "--out", "out", "--tag-only"], dir.path())?;
    assert!(
        real.status.success(),
        "{}",
        String::from_utf8_lossy(&real.stdout)
    );
    let after = fs::read(&file)?;
    assert!(contains(&after, b"Uno"));
    assert_ne!(after, before);
    let audio = DSF_AUDIO_OFFSET..DSF_AUDIO_OFFSET + audio_len;
    assert_eq!(
        after.get(audio.clone()),
        before.get(audio),
        "audio must be identical"
    );

    let again = run(&["album.cue", "--out", "out", "--tag-only"], dir.path())?;
    assert!(String::from_utf8_lossy(&again.stdout).contains("already up to date"));
    Ok(())
}

/// Samples per channel from a DSF header.
fn source_len(bytes: &[u8]) -> Result<u64, Box<dyn std::error::Error>> {
    read_u64(bytes, 64)
}

#[test]
fn tag_only_reports_files_it_cannot_find() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 6_000_003);
    source.write(&dir.path().join("side.dsf"), Format::Dsf)?;
    fs::write(dir.path().join("album.cue"), cue_sheet("side.dsf"))?;
    fs::create_dir(dir.path().join("empty"))?;
    let out = run(&["album.cue", "--tag-only", "--in", "empty"], dir.path())?;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("no matching file"));
    Ok(())
}

#[test]
fn a_saved_project_reproduces_the_same_split() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 6_000_003);
    source.write(&dir.path().join("side.dsf"), Format::Dsf)?;
    fs::write(dir.path().join("album.cue"), cue_sheet("side.dsf"))?;

    // Save with unusual naming, then run from the project file alone.
    let saved = run(
        &[
            "album.cue",
            "--dry-run",
            "--name",
            "{title} [{track}]",
            "--save-project",
            "work.dsfproj",
        ],
        dir.path(),
    )?;
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    assert!(dir.path().join("work.dsfproj").is_file());

    let out = run(&["work.dsfproj", "--out", "out", "--verify"], dir.path())?;
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let starts = expected_starts();
    let mut ends: Vec<u64> = starts.iter().skip(1).copied().collect();
    ends.push(6_000_003);
    for (i, ((start, end), title)) in starts.iter().zip(&ends).zip(TITLES).enumerate() {
        let path = dir
            .path()
            .join("out")
            .join(format!("{title} [{}].dsf", i + 1));
        check_track(&path, &source, *start, end - start)?;
    }
    Ok(())
}

/// 26 s: lead-in, song, 3 s gap, song, 3 s gap, song, tail.
fn three_songs() -> Source {
    Source::songs(&[
        (false, 1.0),
        (true, 6.0),
        (false, 3.0),
        (true, 7.0),
        (false, 3.0),
        (true, 5.0),
        (false, 1.0),
    ])
}

fn samples_in(path: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    read_u64(&fs::read(path)?, 64)
}

#[test]
fn auto_split_cuts_a_file_with_no_cue_at_the_silent_gaps() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = three_songs();
    source.write(&dir.path().join("live.dsf"), Format::Dsf)?;

    let out = run(
        &[
            "live.dsf",
            "--auto",
            "--min-track",
            "3",
            "--out",
            "out",
            "--verify",
            "--write-cue",
            "live.cue",
        ],
        dir.path(),
    )?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("bit-identical"), "{stdout}");

    // Gaps are 7-10 s and 17-20 s, so the cuts are at their middles, 8.5 s and 18.5 s.
    let rate = f64::from(RATE);
    let lengths: Vec<u64> = [
        "01 - Track 01.dsf",
        "02 - Track 02.dsf",
        "03 - Track 03.dsf",
    ]
    .iter()
    .map(|name| samples_in(&dir.path().join("out").join(name)))
    .collect::<Result<_, _>>()?;
    let seconds: Vec<f64> = lengths.iter().map(|n| *n as f64 / rate).collect();
    assert!((seconds[0] - 8.5).abs() < 0.1, "{seconds:?}");
    assert!((seconds[1] - 10.0).abs() < 0.1, "{seconds:?}");
    assert!((seconds[2] - 7.5).abs() < 0.1, "{seconds:?}");
    assert_eq!(
        lengths.iter().sum::<u64>(),
        source.samples,
        "tracks tile the file exactly"
    );

    let cue = fs::read_to_string(dir.path().join("live.cue"))?;
    assert_eq!(cue.matches("TRACK").count(), 3, "{cue}");
    assert!(cue.contains("FILE \"live.dsf\" WAVE"));
    assert!(cue.contains("INDEX 01 00:08:"), "{cue}");
    Ok(())
}

#[test]
fn auto_split_reports_when_there_is_nothing_to_cut() -> TestResult {
    let dir = tempfile::tempdir()?;
    Source::songs(&[(true, 6.0)]).write(&dir.path().join("one.dsf"), Format::Dsf)?;
    let out = run(&["one.dsf", "--auto", "--out", "out"], dir.path())?;
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no gaps found"));
    assert!(dir.path().join("out").join("01 - Track 01.dsf").is_file());
    assert!(!dir.path().join("out").join("02 - Track 02.dsf").exists());
    Ok(())
}

#[test]
fn manual_cut_times_land_on_exact_samples() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = Source::new(2, 8 * u64::from(RATE));
    source.write(&dir.path().join("take.dsf"), Format::Dsf)?;

    let out = run(
        &[
            "take.dsf", "--at", "0:02.5", "--at", "5", "--out", "out", "--verify",
        ],
        dir.path(),
    )?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let rate = f64::from(RATE);
    let bounds = [
        0,
        (2.5 * rate).round() as u64,
        (5.0 * rate).round() as u64,
        source.samples,
    ];
    for (i, window) in bounds.windows(2).enumerate() {
        let path = dir
            .path()
            .join("out")
            .join(format!("{:02} - Track {:02}.dsf", i + 1, i + 1));
        check_track(&path, &source, window[0], window[1] - window[0])?;
    }
    Ok(())
}

#[test]
fn write_cue_output_can_be_split_again_identically() -> TestResult {
    let dir = tempfile::tempdir()?;
    three_songs().write(&dir.path().join("live.dsf"), Format::Dsf)?;
    let first = run(
        &[
            "live.dsf",
            "--auto",
            "--min-track",
            "3",
            "--dry-run",
            "--write-cue",
            "live.cue",
        ],
        dir.path(),
    )?;
    assert!(first.status.success());

    let again = run(&["live.cue", "--out", "out", "--verify"], dir.path())?;
    assert!(
        again.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&again.stdout),
        String::from_utf8_lossy(&again.stderr)
    );
    Ok(())
}

#[test]
fn bad_detection_settings_are_refused_with_a_message() -> TestResult {
    let dir = tempfile::tempdir()?;
    three_songs().write(&dir.path().join("live.dsf"), Format::Dsf)?;
    let out = run(&["live.dsf", "--auto", "--silence-db", "12"], dir.path())?;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("below 0"));
    Ok(())
}

#[test]
fn a_dst_compressed_file_is_refused_with_instructions() -> TestResult {
    let dir = tempfile::tempdir()?;
    let mut bytes = Source::new(2, 8_000).dsdiff();
    let at = bytes
        .windows(4)
        .position(|w| w == b"CMPR")
        .ok_or("no CMPR chunk")?
        + 12;
    bytes
        .get_mut(at..at + 4)
        .ok_or("short file")?
        .copy_from_slice(b"DST ");
    fs::write(dir.path().join("sacd.dff"), bytes)?;

    let out = run(&["sacd.dff", "--out", "out"], dir.path())?;
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("DST"), "{stderr}");
    assert!(
        stderr.contains("Convert the file to uncompressed DSD"),
        "{stderr}"
    );
    assert!(!dir.path().join("out").exists(), "nothing is written");
    Ok(())
}
