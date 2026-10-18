use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::copy;
use super::event::{ByteProgress, Event, TrackReport};
use super::slice::{self, DATA_CHUNK_HEADER_LEN, OUT_BLOCK_SIZE};
use crate::cover;
use crate::error::{Error, Result};
use crate::metadata::FieldKey;
use crate::plan::{SplitPlan, TrackPlan, target_path};

/// Report byte progress after this many output blocks (about 2 MB for stereo).
const PROGRESS_EVERY_BLOCKS: u64 = 256;

/// What `write_track` needs to report progress and honour cancellation.
struct TrackRun<'a> {
    index: usize,
    total: usize,
    cancel: &'a AtomicBool,
    on_event: &'a mut dyn FnMut(Event),
    /// The image to embed in this track, if any.
    cover: Option<&'a dsf::Picture>,
    /// Audio bytes of the album's earlier tracks.
    album_done: u64,
    /// Audio bytes of the whole album, zero if unknown.
    album_total: u64,
}

pub struct SplitEngine {
    base_dir: PathBuf,
    plan: SplitPlan,
    output_dir: PathBuf,
    dry_run: bool,
    overwrite: bool,
}

impl SplitEngine {
    pub fn new(
        base_dir: impl Into<PathBuf>,
        plan: SplitPlan,
        output_dir: impl Into<PathBuf>,
        dry_run: bool,
    ) -> Self {
        Self {
            base_dir: base_dir.into(),
            plan,
            output_dir: output_dir.into(),
            dry_run,
            overwrite: false,
        }
    }

    /// Allow replacing target files that already exist (default: refuse).
    #[must_use]
    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Runs the plan. `cancel` is checked between tracks and between blocks;
    /// `on_event` receives progress.
    pub fn execute(&self, cancel: &AtomicBool, on_event: &mut dyn FnMut(Event)) -> Result<()> {
        self.plan.validate().map_err(Error::InvalidPlan)?;
        if !self.dry_run && !self.overwrite {
            self.ensure_no_conflicts()?;
        }

        on_event(Event::Started {
            album: self.plan.album.value(&FieldKey::ALBUM).to_owned(),
            dry_run: self.dry_run,
        });
        if !self.dry_run {
            fs::create_dir_all(&self.output_dir).map_err(Error::io(format!(
                "creating output directory {}",
                self.output_dir.display()
            )))?;
        }

        // Fail before writing anything if the cover cannot be used.
        let picture = if self.dry_run {
            None
        } else {
            self.load_cover()?
        };

        let total = self.plan.tracks.len();
        let lengths = if self.dry_run {
            Vec::new()
        } else {
            self.track_audio_lengths()
        };
        let album_total: u64 = lengths.iter().sum();
        let mut album_done = 0u64;
        for (index, track) in self.plan.tracks.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }

            let duration = self.plan.duration_of(index);
            let target = self.output_dir.join(self.target_of(track));
            let resolve = |key: &FieldKey| self.plan.album.resolve(&track.meta, key).to_owned();
            let report = TrackReport {
                index,
                total,
                number: track.meta.number,
                title: resolve(&FieldKey::TITLE),
                artist: resolve(&FieldKey::ARTIST),
                source_file: track.source_file.clone(),
                start_seconds: track.start_seconds,
                duration,
                target: target.clone(),
            };

            if self.dry_run {
                on_event(Event::Planned(report));
                continue;
            }
            on_event(Event::Slicing(report));

            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(Error::io(format!("creating folder {}", parent.display())))?;
            }

            // Write beside the target, then rename: a failure never clobbers an existing file.
            let part = part_path(&target);
            let source = self.base_dir.join(&track.source_file);
            let mut run = TrackRun {
                index,
                total,
                cancel,
                on_event: &mut *on_event,
                cover: picture.as_ref().filter(|_| self.embeds_cover()),
                album_done,
                album_total,
            };
            let outcome = self
                .write_track(track, duration, &source, &part, &mut run)
                .and_then(|()| {
                    fs::rename(&part, &target)
                        .map_err(Error::io(format!("moving {} into place", target.display())))
                });
            if let Err(err) = outcome {
                let _ = fs::remove_file(&part); // best effort; the original error matters more
                return Err(err);
            }
            album_done = album_done.saturating_add(lengths.get(index).copied().unwrap_or(0));
        }

        if let Some(picture) = &picture {
            self.copy_cover(picture)?;
        }

        on_event(Event::Finished {
            output_dir: self.output_dir.clone(),
            dry_run: self.dry_run,
        });
        Ok(())
    }

    /// Audio bytes each track will contain, from the source headers; zero where a track's
    /// source cannot be read (that track then fails when it is reached).
    fn track_audio_lengths(&self) -> Vec<u64> {
        let mut infos: HashMap<&str, Option<crate::source::StreamInfo>> = HashMap::new();
        self.plan
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| {
                let info = infos.entry(track.source_file.as_str()).or_insert_with(|| {
                    crate::source::open(&self.base_dir.join(&track.source_file))
                        .ok()
                        .map(|source| source.info().clone())
                });
                info.as_ref()
                    .and_then(|info| {
                        slice::compute(
                            info,
                            OUT_BLOCK_SIZE,
                            track.start_seconds,
                            self.plan.duration_of(index),
                        )
                        .ok()
                    })
                    .map_or(0, |slice| slice.audio_len)
            })
            .collect()
    }

    fn embeds_cover(&self) -> bool {
        self.plan.cover.as_ref().is_some_and(|c| c.embed)
    }

    /// Reads the cover image if the plan embeds or copies it.
    fn load_cover(&self) -> Result<Option<dsf::Picture>> {
        match &self.plan.cover {
            Some(cover) if cover.embed || cover.copy_to_folder => {
                cover::load(&self.base_dir.join(&cover.file)).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// Copies the cover into the album folder as `folder.jpg` / `folder.png`.
    /// An existing file is only replaced when overwriting is allowed.
    fn copy_cover(&self, picture: &dsf::Picture) -> Result<()> {
        let wanted = self.plan.cover.as_ref().is_some_and(|c| c.copy_to_folder);
        let Some(first) = self.plan.tracks.first().filter(|_| wanted) else {
            return Ok(());
        };
        let album_dir = self.output_dir.join(self.target_of(first));
        let Some(album_dir) = album_dir.parent() else {
            return Ok(());
        };
        let dest = album_dir.join(format!(
            "folder.{}",
            cover::extension_of(&picture.mime_type)
        ));
        if dest.exists() && !self.overwrite {
            return Ok(());
        }
        fs::write(&dest, &picture.data).map_err(Error::io(format!(
            "copying the cover to {}",
            dest.display()
        )))
    }

    /// Where `track` goes, relative to the output folder.
    fn target_of(&self, track: &TrackPlan) -> PathBuf {
        target_path(&self.plan.naming, &self.plan.album, track)
    }

    fn ensure_no_conflicts(&self) -> Result<()> {
        let existing: Vec<PathBuf> = self
            .plan
            .tracks
            .iter()
            .map(|t| self.output_dir.join(self.target_of(t)))
            .filter(|path| path.exists())
            .collect();
        match existing.first() {
            Some(first) => Err(Error::TargetsExist {
                count: existing.len(),
                first: first.display().to_string(),
            }),
            None => Ok(()),
        }
    }

    fn write_track(
        &self,
        track: &TrackPlan,
        duration: Option<f64>,
        source: &Path,
        dest: &Path,
        run: &mut TrackRun<'_>,
    ) -> Result<()> {
        let mut src = crate::source::open(source)?;
        let info = src.info().clone();
        let slice = slice::compute(&info, OUT_BLOCK_SIZE, track.start_seconds, duration)?;

        let write_ctx = format!("writing {}", dest.display());
        let file = File::create(dest).map_err(Error::io(format!("creating {}", dest.display())))?;
        let mut out = BufWriter::new(file);

        let out_header = dsf::DsfHeader {
            total_file_size: dsf::AUDIO_OFFSET + slice.audio_len,
            metadata_offset: 0,
            channel_type: info.channel_type,
            channel_count: info.channels,
            sample_rate: info.sample_rate,
            bits_per_sample: 1, // LSB first
            sample_count: slice.sample_count,
            block_size: OUT_BLOCK_SIZE,
            data_chunk_size: DATA_CHUNK_HEADER_LEN + slice.audio_len,
        };
        out_header.write(&mut out)?;
        let mut blocks_seen = 0u64;
        copy::copy_slice(
            src.as_mut(),
            OUT_BLOCK_SIZE,
            &slice,
            &mut out,
            &mut |done| {
                if run.cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                blocks_seen += 1;
                if blocks_seen.is_multiple_of(PROGRESS_EVERY_BLOCKS) || done == slice.audio_len {
                    (run.on_event)(Event::Bytes(ByteProgress {
                        index: run.index,
                        total: run.total,
                        done,
                        len: slice.audio_len,
                        album_done: run.album_done,
                        album_total: run.album_total,
                    }));
                }
                Ok(())
            },
        )?;
        out.flush().map_err(Error::io(write_ctx))?;
        drop(out);

        dsf::update_path(
            dest,
            &self.plan.tag_edit(track, run.cover),
            &dsf::WriteOptions::default(),
        )?;
        Ok(())
    }
}

fn part_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Fields;
    use crate::naming::Naming;
    use crate::plan::{AlbumMeta, TrackMeta};
    use dsf::id3::TagLike;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    const BLOCKS: u64 = 4;
    /// One block is 32768 samples per channel, so at this rate 1 s == 1 block.
    const RATE: u32 = 32_768;

    fn write_source(path: &Path) -> TestResult {
        let audio = BLOCKS * 4096 * 2;
        let header = dsf::DsfHeader {
            total_file_size: dsf::AUDIO_OFFSET + audio,
            metadata_offset: 0,
            channel_type: 2,
            channel_count: 2,
            sample_rate: RATE,
            bits_per_sample: 1,
            sample_count: BLOCKS * 32_768,
            block_size: 4096,
            data_chunk_size: DATA_CHUNK_HEADER_LEN + audio,
        };
        let mut file = File::create(path)?;
        header.write(&mut file)?;
        file.write_all(&vec![0x5A; audio as usize])?;
        Ok(())
    }

    fn track(number: u32, title: &str, start: f64) -> TrackPlan {
        TrackPlan {
            source_file: "side.dsf".to_owned(),
            start_seconds: start,
            meta: TrackMeta::new(number)
                .with(FieldKey::TITLE, title)
                .with(FieldKey::ARTIST, "Artist"),
        }
    }

    #[test]
    fn splits_tags_and_refuses_to_overwrite() -> TestResult {
        let dir = std::env::temp_dir().join(format!("dsd_workbench_split_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        write_source(&dir.join("side.dsf"))?;

        let mut fields = Fields::default();
        fields.set(FieldKey::ALBUM, "Album");
        fields.set(FieldKey::ALBUM_ARTIST, "Artist");
        fields.set(FieldKey::DATE, "2020");
        fields.set(FieldKey::Custom("MOOD".to_owned()), "calm");
        let plan = SplitPlan {
            album: AlbumMeta { fields },
            naming: Naming::default(),
            cover: None,
            tracks: vec![track(1, "One", 0.0), track(2, "Two", 2.0)],
        };
        let out = dir.join("out");
        let engine = SplitEngine::new(&dir, plan, &out, false);

        engine.execute(&AtomicBool::new(false), &mut |_| {})?;

        let first = out.join("01 - One.dsf");
        let header = dsf::DsfHeader::read(File::open(&first)?)?;
        assert_eq!(header.sample_count, 2 * 32_768);
        assert_eq!(header.data_chunk_size, DATA_CHUNK_HEADER_LEN + 2 * 2 * 4096);
        assert!(header.metadata_offset > 0);
        let tag = dsf::read_path(&first)?.ok_or("missing tag")?;
        assert_eq!(tag.title(), Some("One"));
        assert_eq!(tag.album(), Some("Album"));
        let moods: Vec<_> = tag.extended_texts().map(|t| t.value.as_str()).collect();
        assert_eq!(moods, ["calm"], "custom album fields reach every track");

        let second = engine.execute(&AtomicBool::new(false), &mut |_| {});
        assert!(matches!(second, Err(Error::TargetsExist { .. })));

        fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
