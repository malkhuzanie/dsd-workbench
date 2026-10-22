//! The split plan: everything the engine needs, and everything the UI edits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cue::CueSheet;
use crate::metadata::{FieldKey, Fields, Standard};
use crate::naming::Naming;
use crate::snapshot::TagSnapshot;

const VARIOUS_ARTISTS: &str = "Various Artists";

/// Tag values shared by every track unless a track overrides them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumMeta {
    pub fields: Fields,
}

impl AlbumMeta {
    /// The value `track` will be tagged with: its own override, else the album's default.
    pub fn resolve<'a>(&'a self, track: &'a TrackMeta, key: &FieldKey) -> &'a str {
        track
            .overrides
            .get(key)
            .unwrap_or_else(|| self.fields.value(key))
    }

    pub fn value(&self, key: &FieldKey) -> &str {
        self.fields.value(key)
    }
}

/// One track's number and the tag values that differ from the album's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackMeta {
    pub number: u32,
    /// Absent key: inherit the album value. Empty value: deliberately blank.
    pub overrides: Fields,
}

impl TrackMeta {
    pub fn new(number: u32) -> Self {
        Self {
            number,
            overrides: Fields::default(),
        }
    }

    /// Records `value` for `key`, keeping only what differs from the album default.
    pub fn assign(&mut self, album: &AlbumMeta, key: FieldKey, value: &str) {
        if value == album.value(&key) {
            self.overrides.remove(&key);
        } else {
            self.overrides.set(key, value);
        }
    }

    /// Builder form for values known at construction time; blank values are skipped.
    #[must_use]
    pub fn with(mut self, key: FieldKey, value: &str) -> Self {
        self.overrides.set_or_clear(key, value);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackPlan {
    pub source_file: String,
    pub start_seconds: f64,
    pub meta: TrackMeta,
}

/// The value several tracks share for one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shared {
    /// No tracks were listed.
    Nothing,
    Same(String),
    /// The tracks disagree.
    Mixed,
}

/// The album's cover image and what to do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverPlan {
    /// Path of the image, relative to the CUE sheet's folder (or absolute).
    pub file: String,
    /// Embed it in every track.
    pub embed: bool,
    /// Also copy it next to the tracks as `folder.jpg` / `folder.png`.
    pub copy_to_folder: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitPlan {
    pub album: AlbumMeta,
    pub tracks: Vec<TrackPlan>,
    /// How output files and folders are named.
    pub naming: Naming,
    pub cover: Option<CoverPlan>,
}

/// Directory that relative `FILE` entries in the CUE sheet are resolved against.
pub fn base_dir(cue_path: &Path) -> PathBuf {
    cue_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The path (below the output folder) a track is written to: optional sub-folders from
/// the folder template, then the file name from the file template.
pub fn target_path(naming: &Naming, album: &AlbumMeta, track: &TrackPlan) -> PathBuf {
    let number = track.meta.number;
    let lookup = |key: &FieldKey| album.resolve(&track.meta, key);
    let mut path: PathBuf = naming.folders(number, lookup).into_iter().collect();
    path.push(naming.file_name(number, lookup));
    path
}

/// Where the audio of a pregap (`INDEX 00` up to `INDEX 01`) ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GapMode {
    /// It stays at the end of the previous track, as when a CD is ripped "gaps appended".
    #[default]
    AppendToPrevious,
    /// It opens the track it leads into, so playing the album in order sounds the same
    /// but each track starts at its `INDEX 00`.
    PrependToNext,
}

impl GapMode {
    pub const ALL: [Self; 2] = [Self::AppendToPrevious, Self::PrependToNext];

    pub fn label(self) -> &'static str {
        match self {
            Self::AppendToPrevious => "End of the previous track",
            Self::PrependToNext => "Start of the next track",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::AppendToPrevious => "append",
            Self::PrependToNext => "prepend",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }
}

/// How a CUE sheet is turned into a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CueOptions {
    pub gap: GapMode,
    /// Keep audio before the sheet's first track as its own track `00` (a hidden track).
    pub hidden_track: bool,
}

/// Everything that shapes a plan besides the CUE sheet itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanOptions {
    pub cue: CueOptions,
    pub naming: Naming,
    /// Embed a cover image found next to the CUE sheet in every track.
    pub embed_cover: bool,
    /// Also copy that image next to the tracks.
    pub copy_cover: bool,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            cue: CueOptions::default(),
            naming: Naming::default(),
            embed_cover: true,
            copy_cover: false,
        }
    }
}

/// Audio ahead of the first track shorter than this is lead-in, not a hidden track.
pub const HIDDEN_TRACK_MIN_SECONDS: f64 = 1.0;
const HIDDEN_TRACK_TITLE: &str = "Hidden Track";

impl SplitPlan {
    pub fn from_cue(sheet: &CueSheet, options: CueOptions) -> Self {
        let mut tracks: Vec<TrackPlan> = Vec::with_capacity(sheet.tracks.len());
        let mut previous_start: Option<(&str, f64)> = None;
        for t in &sheet.tracks {
            // A pregap only counts if it lies after the previous track began.
            let pregap = t.pregap_seconds.filter(|gap| match previous_start {
                Some((file, start)) if file == t.source_file => *gap > start,
                _ => true,
            });
            let start_seconds = match (options.gap, pregap) {
                (GapMode::PrependToNext, Some(gap)) => gap,
                _ => t.start_seconds,
            };

            // Only the very first track can be preceded by "track 0".
            if options.hidden_track
                && tracks.is_empty()
                && start_seconds >= HIDDEN_TRACK_MIN_SECONDS
            {
                tracks.push(TrackPlan {
                    source_file: t.source_file.clone(),
                    start_seconds: 0.0,
                    meta: cue_meta(0, HIDDEN_TRACK_TITLE, &sheet.performer, &sheet.composer, ""),
                });
            }
            tracks.push(TrackPlan {
                source_file: t.source_file.clone(),
                start_seconds,
                meta: cue_meta(t.number, &t.title, &t.artist, &t.composer, &t.isrc),
            });
            previous_start = Some((&t.source_file, start_seconds));
        }

        let album_artist = if sheet.performer.is_empty() {
            common_artist(sheet)
        } else {
            sheet.performer.clone()
        };

        let mut fields = Fields::default();
        fields.set_or_clear(FieldKey::ALBUM, &sheet.album_title);
        fields.set_or_clear(FieldKey::ALBUM_ARTIST, &album_artist);
        fields.set_or_clear(FieldKey::GENRE, &sheet.genre);
        fields.set_or_clear(FieldKey::DATE, &sheet.date);
        fields.set_or_clear(FieldKey::Standard(Standard::Comment), &sheet.comment);
        fields.set_or_clear(FieldKey::Standard(Standard::Disc), &sheet.disc);
        fields.set_or_clear(FieldKey::Custom(CATALOG_FIELD.to_owned()), &sheet.catalog);

        Self {
            album: AlbumMeta { fields },
            tracks,
            naming: Naming::default(),
            cover: None,
        }
    }

    /// Fills blank album fields from an existing tag; never overwrites CUE data.
    pub fn seed_from_source(&mut self, source: &TagSnapshot) {
        for key in [
            FieldKey::ALBUM,
            FieldKey::ALBUM_ARTIST,
            FieldKey::GENRE,
            FieldKey::DATE,
        ] {
            if self.album.value(&key).trim().is_empty() {
                let found = source.fields.value(&key).trim().to_owned();
                self.album.fields.set_or_clear(key, &found);
            }
        }
    }

    /// Sets `key` to `value` on each listed track (rows past the end are ignored).
    pub fn assign_tracks(&mut self, rows: &[usize], key: &FieldKey, value: &str) {
        for row in rows {
            if let Some(track) = self.tracks.get_mut(*row) {
                track.meta.assign(&self.album, key.clone(), value);
            }
        }
    }

    /// Makes each listed track follow the album's value for `key` again.
    pub fn inherit_tracks(&mut self, rows: &[usize], key: &FieldKey) {
        for row in rows {
            if let Some(track) = self.tracks.get_mut(*row) {
                track.meta.overrides.remove(key);
            }
        }
    }

    /// What the listed tracks have for `key`: one shared value or a mix.
    pub fn shared_value(&self, rows: &[usize], key: &FieldKey) -> Shared {
        let mut values = rows
            .iter()
            .filter_map(|row| self.tracks.get(*row))
            .map(|track| self.album.resolve(&track.meta, key));
        let Some(first) = values.next() else {
            return Shared::Nothing;
        };
        if values.all(|value| value == first) {
            Shared::Same(first.to_owned())
        } else {
            Shared::Mixed
        }
    }

    /// Whether any listed track overrides the album's value for `key`.
    pub fn any_override(&self, rows: &[usize], key: &FieldKey) -> bool {
        rows.iter()
            .filter_map(|row| self.tracks.get(*row))
            .any(|track| track.meta.overrides.get(key).is_some())
    }

    /// Custom fields set on the album or on any listed track, in name order.
    pub fn custom_keys(&self, rows: &[usize]) -> Vec<FieldKey> {
        let mut keys: Vec<FieldKey> = self
            .album
            .fields
            .keys()
            .chain(
                rows.iter()
                    .filter_map(|row| self.tracks.get(*row))
                    .flat_map(|track| track.meta.overrides.keys()),
            )
            .filter(|key| matches!(key, FieldKey::Custom(_)))
            .cloned()
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// Length of track `index`: up to the next track in the same source file.
    /// `None` means "until the end of the source file".
    pub fn duration_of(&self, index: usize) -> Option<f64> {
        let track = self.tracks.get(index)?;
        let next = self.tracks.get(index + 1)?;
        (next.source_file == track.source_file).then_some(next.start_seconds - track.start_seconds)
    }

    /// Swaps the titles, artists and other per-track tags of two tracks while each keeps its
    /// time slot, number and file. This is how a track list that is in the wrong order is
    /// fixed. Returns `false` if either index does not exist.
    pub fn swap_track_details(&mut self, a: usize, b: usize) -> bool {
        if a == b {
            return a < self.tracks.len();
        }
        let (low, high) = (a.min(b), a.max(b));
        // Splitting at `high` puts the two tracks in different halves, so both can be borrowed.
        let Some((head, tail)) = self.tracks.split_at_mut_checked(high) else {
            return false;
        };
        let (Some(first), Some(second)) = (head.get_mut(low), tail.first_mut()) else {
            return false;
        };
        std::mem::swap(&mut first.meta.overrides, &mut second.meta.overrides);
        true
    }

    /// Numbers the tracks 1, 2, 3 … in plan order. A hidden track 0 in front stays 0.
    pub fn renumber(&mut self) {
        let mut next = 1;
        for (index, track) in self.tracks.iter_mut().enumerate() {
            if index == 0 && track.meta.number == 0 {
                continue;
            }
            track.meta.number = next;
            next += 1;
        }
    }

    /// Indices of the tracks cut from `file`, in plan order.
    pub fn tracks_in(&self, file: &str) -> Vec<usize> {
        self.tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.source_file == file)
            .map(|(i, _)| i)
            .collect()
    }

    /// Starts a new track at `time` in `file`, splitting the track that contains `time`.
    /// Later tracks are renumbered. Returns the new track's index, or `None` if `time`
    /// would leave either piece shorter than `min_len` seconds (or lies before every track).
    pub fn insert_cut(&mut self, file: &str, time: f64, min_len: f64) -> Option<usize> {
        if !time.is_finite() {
            return None;
        }
        let in_file = self.tracks_in(file);
        let previous = in_file
            .iter()
            .copied()
            .rfind(|i| self.tracks.get(*i).is_some_and(|t| t.start_seconds <= time))?;
        let before = self.tracks.get(previous)?;
        if time - before.start_seconds < min_len {
            return None;
        }
        let next_start = in_file
            .iter()
            .copied()
            .find(|i| *i > previous)
            .and_then(|i| self.tracks.get(i))
            .map(|t| t.start_seconds);
        if next_start.is_some_and(|next| next - time < min_len) {
            return None;
        }

        let number = before.meta.number + 1;
        let at = previous + 1;
        for track in self.tracks.iter_mut().skip(at) {
            track.meta.number += 1;
        }
        self.tracks.insert(
            at,
            TrackPlan {
                source_file: file.to_owned(),
                start_seconds: time,
                meta: TrackMeta::new(number).with(FieldKey::TITLE, &format!("Track {number:02}")),
            },
        );
        Some(at)
    }

    /// Removes track `index`, which then belongs to the end of the track before it.
    /// The first track of a file cannot be removed (its audio would be left out).
    /// Later tracks are renumbered.
    pub fn remove_cut(&mut self, index: usize) -> bool {
        let Some(track) = self.tracks.get(index) else {
            return false;
        };
        let first_of_file = self
            .tracks_in(&track.source_file)
            .first()
            .is_some_and(|first| *first == index);
        if first_of_file {
            return false;
        }
        self.tracks.remove(index);
        for later in self.tracks.iter_mut().skip(index) {
            later.meta.number = later.meta.number.saturating_sub(1);
        }
        true
    }

    /// Moves the start of track `index` to `time`, keeping every track in the same file at
    /// least `min_len` long. The first track of a file stays at 0. Returns where it ended up.
    pub fn move_cut(&mut self, index: usize, time: f64, min_len: f64) -> Option<f64> {
        let track = self.tracks.get(index)?;
        let file = track.source_file.clone();
        let in_file = self.tracks_in(&file);
        let position = in_file.iter().position(|i| *i == index)?;
        if position == 0 {
            return None;
        }
        let lower = in_file
            .get(position - 1)
            .and_then(|i| self.tracks.get(*i))
            .map_or(0.0, |t| t.start_seconds + min_len);
        let upper = in_file
            .get(position + 1)
            .and_then(|i| self.tracks.get(*i))
            .map_or(f64::INFINITY, |t| t.start_seconds - min_len);
        if upper < lower || !time.is_finite() {
            return None;
        }
        let clamped = time.clamp(lower, upper);
        self.tracks.get_mut(index)?.start_seconds = clamped;
        Some(clamped)
    }

    /// Points every track that reads `old` at `new` instead.
    pub fn rename_source(&mut self, old: &str, new: &str) {
        for track in self.tracks.iter_mut().filter(|t| t.source_file == old) {
            new.clone_into(&mut track.source_file);
        }
    }

    /// Distinct source files in first-use order.
    pub fn source_files(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for track in &self.tracks {
            if !seen.contains(&track.source_file) {
                seen.push(track.source_file.clone());
            }
        }
        seen
    }

    /// Returns every reason this plan cannot be executed.
    pub fn validate(&self) -> std::result::Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.tracks.is_empty() {
            problems.push("The plan has no tracks.".to_owned());
        }
        if let Err(err) = self.naming.validate() {
            problems.push(format!("File name template: {err}."));
        }

        let mut targets: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for (index, track) in self.tracks.iter().enumerate() {
            let number = track.meta.number;
            // Number 0 is reserved for a hidden track that precedes track 1.
            if number == 0 && index != 0 {
                let title = self.album.resolve(&track.meta, &FieldKey::TITLE);
                problems.push(format!("Track '{title}' has number 0."));
            }
            if !(track.start_seconds.is_finite() && track.start_seconds >= 0.0) {
                problems.push(format!("Track {number:02} has an invalid start time."));
            }
            if let Some(d) = self.duration_of(index)
                && !(d.is_finite() && d > 0.0)
            {
                problems.push(format!(
                    "Track {number:02} has no length: the next track does not start after it."
                ));
            }
            targets
                .entry(self.target(track).to_string_lossy().into_owned())
                .or_default()
                .push(number);
        }
        for (name, numbers) in targets {
            if numbers.len() > 1 {
                let list = numbers
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                problems.push(format!("Tracks {list} would all be written to '{name}'."));
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// The `N` of every `n/N` track tag: the highest track number, so a hidden track 0 is not counted.
    pub fn track_total(&self) -> u32 {
        self.tracks.iter().map(|t| t.meta.number).max().unwrap_or(0)
    }

    /// Where `track` is written, relative to the output folder.
    pub fn target(&self, track: &TrackPlan) -> PathBuf {
        target_path(&self.naming, &self.album, track)
    }

    /// Where `track` is written when the album goes to `output_dir`.
    pub fn output_path(&self, output_dir: &Path, track: &TrackPlan) -> PathBuf {
        output_dir.join(self.target(track))
    }

    /// The `TRCK` value written for `track`. Writing and verifying both use it, so they agree.
    pub fn track_tag(&self, track: &TrackPlan) -> dsf::Track {
        let total = self.track_total();
        dsf::Track {
            number: track.meta.number,
            total: (total > 0).then_some(total),
        }
    }

    /// The tag changes written to a freshly split track.
    pub fn tag_edit(&self, track: &TrackPlan, cover: Option<&dsf::Picture>) -> dsf::TagEdit {
        let meta = &track.meta;
        let mut edit = dsf::TagEdit::new().set_track(self.track_tag(track));
        for key in self.tag_keys(track) {
            let value = self.album.resolve(meta, &key);
            edit = match &key {
                FieldKey::Standard(standard) => edit.set_field(standard.field(), value),
                FieldKey::Custom(name) => edit.set_user_text(name.as_str(), value),
            };
        }
        match cover {
            Some(picture) => edit.set_cover(picture.clone()),
            None => edit,
        }
    }

    /// Every key written for `track`: all standard fields (blank ones clear the frame)
    /// plus any custom field the album or the track defines.
    pub fn tag_keys(&self, track: &TrackPlan) -> Vec<FieldKey> {
        let mut keys: Vec<FieldKey> = Standard::ALL.into_iter().map(FieldKey::Standard).collect();
        for key in self.album.fields.keys().chain(track.meta.overrides.keys()) {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        keys
    }
}

/// Track metadata as read from a CUE sheet; blank values are not stored.
fn cue_meta(number: u32, title: &str, artist: &str, composer: &str, isrc: &str) -> TrackMeta {
    TrackMeta::new(number)
        .with(FieldKey::TITLE, title)
        .with(FieldKey::ARTIST, artist)
        .with(FieldKey::COMPOSER, composer)
        .with(FieldKey::Standard(Standard::Isrc), isrc)
}

/// Name of the custom field that carries a CUE sheet's `CATALOG` number.
pub const CATALOG_FIELD: &str = "CATALOGNUMBER";

fn common_artist(sheet: &CueSheet) -> String {
    let Some(first) = sheet.tracks.first() else {
        return String::new();
    };
    if sheet.tracks.iter().all(|t| t.artist == first.artist) {
        first.artist.clone()
    } else {
        VARIOUS_ARTISTS.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::CueTrack;

    fn track(number: u32, title: &str, artist: &str, start: f64, file: &str) -> CueTrack {
        CueTrack {
            number,
            title: title.to_owned(),
            artist: artist.to_owned(),
            composer: String::new(),
            start_seconds: start,
            pregap_seconds: None,
            isrc: String::new(),
            source_file: file.to_owned(),
        }
    }

    #[test]
    fn cue_extras_become_fields() {
        let mut sheet = sheet();
        sheet.comment = "Remastered".to_owned();
        sheet.disc = "1/2".to_owned();
        sheet.catalog = "0602498".to_owned();
        sheet.tracks[1].isrc = "USABC1234567".to_owned();
        let plan = SplitPlan::from_cue(&sheet, CueOptions::default());

        let album = &plan.album;
        assert_eq!(
            album.value(&FieldKey::Standard(Standard::Comment)),
            "Remastered"
        );
        assert_eq!(album.value(&FieldKey::Standard(Standard::Disc)), "1/2");
        let catalog = FieldKey::Custom(CATALOG_FIELD.to_owned());
        assert_eq!(album.value(&catalog), "0602498");
        let isrc = FieldKey::Standard(Standard::Isrc);
        assert_eq!(album.resolve(&plan.tracks[1].meta, &isrc), "USABC1234567");
        assert_eq!(album.resolve(&plan.tracks[0].meta, &isrc), "");
        assert!(plan.tag_keys(&plan.tracks[0]).contains(&catalog));
    }

    /// Side A: tracks 1-2 with a pregap before track 2 and a 5 s lead-in; side B: track 3.
    fn gapped_sheet() -> CueSheet {
        let mut sheet = sheet();
        sheet.tracks[0].start_seconds = 5.0;
        sheet.tracks[1].start_seconds = 62.0;
        sheet.tracks[1].pregap_seconds = Some(60.0);
        sheet
    }

    fn starts(plan: &SplitPlan) -> Vec<(u32, f64)> {
        plan.tracks
            .iter()
            .map(|t| (t.meta.number, t.start_seconds))
            .collect()
    }

    #[test]
    fn gaps_are_appended_to_the_previous_track_by_default() {
        let plan = SplitPlan::from_cue(&gapped_sheet(), CueOptions::default());
        assert_eq!(starts(&plan), [(1, 5.0), (2, 62.0), (3, 0.0)]);
    }

    #[test]
    fn prepending_starts_the_track_at_its_pregap() {
        let options = CueOptions {
            gap: GapMode::PrependToNext,
            hidden_track: false,
        };
        let plan = SplitPlan::from_cue(&gapped_sheet(), options);
        assert_eq!(starts(&plan), [(1, 5.0), (2, 60.0), (3, 0.0)]);
        assert_eq!(plan.duration_of(0), Some(55.0));
    }

    #[test]
    fn a_pregap_before_the_previous_track_is_ignored() {
        let mut sheet = gapped_sheet();
        sheet.tracks[1].pregap_seconds = Some(3.0);
        let options = CueOptions {
            gap: GapMode::PrependToNext,
            hidden_track: false,
        };
        let plan = SplitPlan::from_cue(&sheet, options);
        assert_eq!(starts(&plan)[1], (2, 62.0));
    }

    #[test]
    fn a_lead_in_becomes_hidden_track_zero_for_the_first_track_only() {
        let mut sheet = gapped_sheet();
        sheet.tracks[2].start_seconds = 2.0; // side B has a lead-in too
        let options = CueOptions {
            gap: GapMode::AppendToPrevious,
            hidden_track: true,
        };
        let plan = SplitPlan::from_cue(&sheet, options);
        assert_eq!(starts(&plan), [(0, 0.0), (1, 5.0), (2, 62.0), (3, 2.0)]);
        assert_eq!(plan.duration_of(0), Some(5.0));
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn short_lead_ins_are_not_hidden_tracks() {
        let mut sheet = gapped_sheet();
        sheet.tracks[0].start_seconds = 0.4;
        let options = CueOptions {
            gap: GapMode::AppendToPrevious,
            hidden_track: true,
        };
        let plan = SplitPlan::from_cue(&sheet, options);
        assert_eq!(plan.tracks.first().map(|t| t.meta.number), Some(1));
    }

    #[test]
    fn prepending_a_pregap_at_zero_needs_no_hidden_track() {
        let mut sheet = gapped_sheet();
        sheet.tracks[0].pregap_seconds = Some(0.0);
        let options = CueOptions {
            gap: GapMode::PrependToNext,
            hidden_track: true,
        };
        let plan = SplitPlan::from_cue(&sheet, options);
        assert_eq!(starts(&plan)[0], (1, 0.0));
    }

    #[test]
    fn track_number_zero_is_only_valid_in_front() {
        let options = CueOptions {
            gap: GapMode::AppendToPrevious,
            hidden_track: true,
        };
        let mut plan = SplitPlan::from_cue(&gapped_sheet(), options);
        assert!(plan.validate().is_ok());
        if let Some(track) = plan.tracks.get_mut(2) {
            track.meta.number = 0;
        }
        assert!(plan.validate().is_err());
    }

    #[test]
    fn track_total_ignores_the_hidden_track() {
        let options = CueOptions {
            gap: GapMode::AppendToPrevious,
            hidden_track: true,
        };
        let plan = SplitPlan::from_cue(&gapped_sheet(), options);
        assert_eq!(plan.tracks.len(), 4);
        assert_eq!(plan.track_total(), 3);
    }

    #[test]
    fn the_track_tag_has_no_total_when_every_track_is_numbered_zero() {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        assert_eq!(plan.track_tag(&plan.tracks[0]).to_string(), "1/3");
        for track in &mut plan.tracks {
            track.meta.number = 0;
        }
        assert_eq!(plan.track_tag(&plan.tracks[0]).to_string(), "0");
    }

    fn sheet() -> CueSheet {
        CueSheet {
            album_title: "Album".to_owned(),
            tracks: vec![
                track(1, "One", "A", 0.0, "side_a.dsf"),
                track(2, "Two", "A", 60.0, "side_a.dsf"),
                track(3, "Three", "B", 0.0, "side_b.dsf"),
            ],
            ..CueSheet::default()
        }
    }

    #[test]
    fn durations_are_derived_per_source_file() {
        let plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        assert_eq!(plan.duration_of(0), Some(60.0));
        assert_eq!(
            plan.duration_of(1),
            None,
            "last track of a side runs to the end"
        );
        assert_eq!(plan.duration_of(2), None);
    }

    #[test]
    fn renaming_a_source_touches_only_its_tracks() {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        let before = plan.source_files();
        assert!(before.len() > 1, "fixture should use several files");
        let first = before[0].clone();
        let other = before[1].clone();

        plan.rename_source(&first, "relinked.dsf");
        let after = plan.source_files();
        assert_eq!(after[0], "relinked.dsf");
        assert_eq!(after[1], other);
    }

    #[test]
    fn editing_a_start_changes_the_previous_length() {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        plan.tracks[1].start_seconds = 45.0;
        assert_eq!(plan.duration_of(0), Some(45.0));
        plan.tracks[1].start_seconds = 0.0;
        assert!(
            plan.validate().is_err(),
            "zero-length track must be rejected"
        );
    }

    #[test]
    fn mixed_artists_become_various_artists() {
        let plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        assert_eq!(plan.album.value(&FieldKey::ALBUM_ARTIST), VARIOUS_ARTISTS);
    }

    #[test]
    fn target_paths_follow_the_naming_templates() {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        plan.album.fields.set(FieldKey::DATE, "1959");
        let first = &plan.tracks[0];
        assert_eq!(
            target_path(&plan.naming, &plan.album, first),
            PathBuf::from("01 - One.dsf")
        );

        plan.naming.folder = "{albumartist}/{album} ({year})".to_owned();
        plan.naming.file = "{track:02}. {title}".to_owned();
        let first = &plan.tracks[0];
        assert_eq!(
            target_path(&plan.naming, &plan.album, first),
            ["Various Artists", "Album (1959)", "01. One.dsf"]
                .iter()
                .collect::<PathBuf>()
        );
    }

    #[test]
    fn a_broken_template_is_a_validation_problem() {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        plan.naming.file = "{nope}".to_owned();
        let problems = plan.validate().err().unwrap_or_default();
        assert!(problems.iter().any(|p| p.contains("File name template")));
    }

    #[test]
    fn a_template_without_the_track_number_reports_collisions() {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        plan.naming.file = "{album}".to_owned();
        let problems = plan.validate().err().unwrap_or_default();
        assert!(problems.iter().any(|p| p.contains("would all be written")));
    }

    #[test]
    fn duplicate_targets_are_reported() {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        plan.tracks[1].meta.number = 1;
        plan.tracks[1]
            .meta
            .assign(&plan.album, FieldKey::TITLE, "One");
        let problems = plan.validate().err().unwrap_or_default();
        assert!(problems.iter().any(|p| p.contains("would all be written")));
    }

    #[test]
    fn seeding_never_overwrites_cue_values() {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        plan.album.fields.set(FieldKey::ALBUM_ARTIST, "Kept");
        let mut source = TagSnapshot::default();
        source.fields.set(FieldKey::ALBUM_ARTIST, "Ignored");
        source.fields.set(FieldKey::GENRE, "Jazz");
        plan.seed_from_source(&source);
        assert_eq!(plan.album.value(&FieldKey::ALBUM_ARTIST), "Kept");
        assert_eq!(plan.album.value(&FieldKey::GENRE), "Jazz");
    }

    fn two_track_plan() -> SplitPlan {
        let mut plan = SplitPlan::from_cue(&sheet(), CueOptions::default());
        plan.album.fields.set(FieldKey::GENRE, "Jazz");
        plan
    }

    #[test]
    fn tracks_inherit_album_values_until_overridden() {
        let mut plan = two_track_plan();
        let album = plan.album.clone();
        let meta = &mut plan.tracks[0].meta;
        assert_eq!(album.resolve(meta, &FieldKey::GENRE), "Jazz");

        meta.assign(&album, FieldKey::GENRE, "Blues");
        assert_eq!(album.resolve(meta, &FieldKey::GENRE), "Blues");
        assert_eq!(
            album.resolve(&plan.tracks[1].meta, &FieldKey::GENRE),
            "Jazz",
            "other tracks keep the album value"
        );
    }

    #[test]
    fn assigning_the_inherited_value_removes_the_override() {
        let mut plan = two_track_plan();
        let album = plan.album.clone();
        let meta = &mut plan.tracks[0].meta;
        meta.assign(&album, FieldKey::GENRE, "Blues");
        meta.assign(&album, FieldKey::GENRE, "Jazz");
        assert_eq!(meta.overrides.get(&FieldKey::GENRE), None);
    }

    #[test]
    fn a_track_can_blank_an_inherited_value() {
        let mut plan = two_track_plan();
        let album = plan.album.clone();
        let meta = &mut plan.tracks[0].meta;
        meta.assign(&album, FieldKey::GENRE, "");
        assert_eq!(album.resolve(meta, &FieldKey::GENRE), "");
        let tag = plan.tag_edit(&plan.tracks[0], None);
        assert!(!tag.is_noop());
    }

    /// Side A has tracks at 0 and 60 s (numbers 1, 2); side B one track (number 3).
    fn editable_plan() -> SplitPlan {
        SplitPlan::from_cue(&sheet(), CueOptions::default())
    }

    fn layout(plan: &SplitPlan) -> Vec<(u32, &str, f64)> {
        plan.tracks
            .iter()
            .map(|t| (t.meta.number, t.source_file.as_str(), t.start_seconds))
            .collect()
    }

    #[test]
    fn inserting_a_cut_splits_a_track_and_renumbers_the_rest() {
        let mut plan = editable_plan();
        let index = plan.insert_cut("side_a.dsf", 20.0, 1.0);
        assert_eq!(index, Some(1));
        assert_eq!(
            layout(&plan),
            [
                (1, "side_a.dsf", 0.0),
                (2, "side_a.dsf", 20.0),
                (3, "side_a.dsf", 60.0),
                (4, "side_b.dsf", 0.0)
            ]
        );
        assert_eq!(
            plan.album.resolve(&plan.tracks[1].meta, &FieldKey::TITLE),
            "Track 02"
        );
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn a_cut_too_close_to_a_neighbour_or_off_the_map_is_refused() {
        let mut plan = editable_plan();
        assert_eq!(
            plan.insert_cut("side_a.dsf", 0.5, 1.0),
            None,
            "too close to the start"
        );
        assert_eq!(
            plan.insert_cut("side_a.dsf", 59.5, 1.0),
            None,
            "too close to the next track"
        );
        assert_eq!(plan.insert_cut("side_a.dsf", f64::NAN, 1.0), None);
        assert_eq!(plan.insert_cut("missing.dsf", 10.0, 1.0), None);
        assert_eq!(plan.tracks.len(), 3, "nothing changed");
    }

    #[test]
    fn the_last_track_of_a_file_can_be_split_too() {
        let mut plan = editable_plan();
        assert_eq!(plan.insert_cut("side_a.dsf", 500.0, 1.0), Some(2));
        assert_eq!(layout(&plan)[2], (3, "side_a.dsf", 500.0));
        assert_eq!(layout(&plan)[3], (4, "side_b.dsf", 0.0));
    }

    #[test]
    fn removing_a_cut_merges_with_the_previous_track() {
        let mut plan = editable_plan();
        plan.insert_cut("side_a.dsf", 20.0, 1.0);
        assert!(plan.remove_cut(2));
        assert_eq!(
            layout(&plan),
            [
                (1, "side_a.dsf", 0.0),
                (2, "side_a.dsf", 20.0),
                (3, "side_b.dsf", 0.0)
            ]
        );
        assert!(!plan.remove_cut(0), "the first track of a file stays");
        assert!(!plan.remove_cut(2), "side B's only track stays");
        assert!(!plan.remove_cut(99));
    }

    #[test]
    fn moving_a_cut_is_clamped_between_its_neighbours() {
        let mut plan = editable_plan();
        plan.insert_cut("side_a.dsf", 20.0, 1.0); // tracks at 0, 20, 60
        assert_eq!(plan.move_cut(1, 30.0, 2.0), Some(30.0));
        assert_eq!(
            plan.move_cut(1, 1.0, 2.0),
            Some(2.0),
            "not closer than 2 s to the start"
        );
        assert_eq!(
            plan.move_cut(1, 999.0, 2.0),
            Some(58.0),
            "nor to the next track"
        );
        assert_eq!(
            plan.move_cut(0, 5.0, 2.0),
            None,
            "the first track stays at 0"
        );
        assert_eq!(plan.move_cut(1, f64::NAN, 2.0), None);
        assert_eq!(plan.move_cut(42, 5.0, 2.0), None);
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn swapping_details_moves_titles_but_not_time_slots() {
        let mut plan = editable_plan();
        let title = |plan: &SplitPlan, i: usize| {
            plan.album
                .resolve(&plan.tracks[i].meta, &FieldKey::TITLE)
                .to_owned()
        };
        let before: Vec<_> = plan
            .tracks
            .iter()
            .map(|t| (t.start_seconds, t.meta.number))
            .collect();
        assert_eq!(
            (title(&plan, 0), title(&plan, 1)),
            ("One".into(), "Two".into())
        );

        assert!(plan.swap_track_details(0, 1));
        assert_eq!(
            (title(&plan, 0), title(&plan, 1)),
            ("Two".into(), "One".into())
        );
        let after: Vec<_> = plan
            .tracks
            .iter()
            .map(|t| (t.start_seconds, t.meta.number))
            .collect();
        assert_eq!(before, after, "slots and numbers stay");

        assert!(plan.swap_track_details(1, 0));
        assert_eq!(title(&plan, 0), "One", "swapping twice restores it");
        assert!(
            plan.swap_track_details(2, 2),
            "swapping with itself is a no-op"
        );
        assert!(!plan.swap_track_details(0, 99));
        assert!(!plan.swap_track_details(99, 99));
    }

    #[test]
    fn renumbering_is_sequential_and_keeps_a_hidden_track() {
        let mut plan = editable_plan();
        plan.tracks[0].meta.number = 7;
        plan.tracks[1].meta.number = 2;
        plan.tracks[2].meta.number = 40;
        plan.renumber();
        let numbers: Vec<u32> = plan.tracks.iter().map(|t| t.meta.number).collect();
        assert_eq!(numbers, [1, 2, 3]);

        let hidden = CueOptions {
            gap: GapMode::AppendToPrevious,
            hidden_track: true,
        };
        let mut with_hidden = SplitPlan::from_cue(&gapped_sheet(), hidden);
        with_hidden.tracks[2].meta.number = 9;
        with_hidden.renumber();
        let numbers: Vec<u32> = with_hidden.tracks.iter().map(|t| t.meta.number).collect();
        assert_eq!(numbers, [0, 1, 2, 3]);
    }

    #[test]
    fn a_files_tracks_are_listed_in_order() {
        let mut plan = editable_plan();
        plan.insert_cut("side_a.dsf", 20.0, 1.0);
        assert_eq!(plan.tracks_in("side_a.dsf"), [0, 1, 2]);
        assert_eq!(plan.tracks_in("side_b.dsf"), [3]);
        assert!(plan.tracks_in("nope.dsf").is_empty());
    }

    #[test]
    fn bulk_assign_sets_every_listed_track_and_nothing_else() {
        let mut plan = two_track_plan();
        plan.assign_tracks(&[0, 1, 99], &FieldKey::GENRE, "Blues");
        for track in &plan.tracks[..2] {
            assert_eq!(plan.album.resolve(&track.meta, &FieldKey::GENRE), "Blues");
        }
        assert_eq!(
            plan.album.value(&FieldKey::GENRE),
            "Jazz",
            "album default is untouched"
        );
        assert_eq!(plan.tracks.len(), 3, "an out-of-range row adds nothing");
        assert_eq!(
            plan.album.resolve(&plan.tracks[2].meta, &FieldKey::GENRE),
            "Jazz"
        );
    }

    #[test]
    fn inherit_returns_tracks_to_the_album_value() {
        let mut plan = two_track_plan();
        plan.assign_tracks(&[0, 1], &FieldKey::GENRE, "Blues");
        assert!(plan.any_override(&[0], &FieldKey::GENRE));
        plan.inherit_tracks(&[0, 1], &FieldKey::GENRE);
        assert!(!plan.any_override(&[0, 1], &FieldKey::GENRE));
        assert_eq!(
            plan.shared_value(&[0, 1], &FieldKey::GENRE),
            Shared::Same("Jazz".into())
        );
    }

    #[test]
    fn shared_value_reports_same_mixed_and_nothing() {
        let mut plan = two_track_plan();
        assert_eq!(plan.shared_value(&[], &FieldKey::GENRE), Shared::Nothing);
        assert_eq!(
            plan.shared_value(&[0, 1], &FieldKey::GENRE),
            Shared::Same("Jazz".into())
        );
        plan.assign_tracks(&[1], &FieldKey::GENRE, "Blues");
        assert_eq!(plan.shared_value(&[0, 1], &FieldKey::GENRE), Shared::Mixed);
        assert_eq!(
            plan.shared_value(&[1], &FieldKey::GENRE),
            Shared::Same("Blues".into())
        );
    }

    #[test]
    fn custom_keys_are_collected_from_the_album_and_selected_tracks() {
        let mut plan = two_track_plan();
        let mood = FieldKey::Custom("MOOD".to_owned());
        let label = FieldKey::Custom("LABEL".to_owned());
        plan.album.fields.set(mood.clone(), "calm");
        plan.assign_tracks(&[1], &label, "Indie");

        assert_eq!(plan.custom_keys(&[0]), std::slice::from_ref(&mood));
        assert_eq!(plan.custom_keys(&[0, 1]), [label.clone(), mood.clone()]);

        // A track-only field disappears when the track follows the album again,
        // and an album field when its value is cleared.
        plan.inherit_tracks(&[1], &label);
        plan.album.fields.set_or_clear(mood, "");
        assert!(plan.custom_keys(&[0, 1]).is_empty());
    }

    #[test]
    fn custom_fields_are_tagged_from_album_and_track() {
        // The tags actually written are checked end to end in `engine::splitter`.
        let mut plan = two_track_plan();
        let album = plan.album.clone();
        let mood = FieldKey::Custom("MOOD".to_owned());
        let label = FieldKey::Custom("LABEL".to_owned());
        plan.album.fields.set(mood.clone(), "calm");
        plan.tracks[1].meta.assign(&album, label.clone(), "Indie");

        assert_eq!(plan.album.resolve(&plan.tracks[0].meta, &mood), "calm");
        assert_eq!(plan.album.resolve(&plan.tracks[1].meta, &label), "Indie");
        assert!(plan.tag_keys(&plan.tracks[0]).contains(&mood));
        assert!(
            !plan.tag_keys(&plan.tracks[0]).contains(&label),
            "a track-only custom field is not written to other tracks"
        );
    }
}
