//! Copying a looked-up release (MusicBrainz or Discogs) into a plan.
//!
//! Finding and parsing releases is the `release-lookup` crate's job; this module only knows
//! about plans: which of the release's values go to which tags, and what to report back.

use release_lookup::{Fetch, Release, Source, Target};

use crate::metadata::{FieldKey, Standard};
use crate::plan::{CATALOG_FIELD, SplitPlan};

/// Matching track lengths further apart than this (seconds) are reported.
const LENGTH_TOLERANCE_SECONDS: f64 = 15.0;

/// What to copy from a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOptions {
    /// Album title, album artist, date, label and catalogue number.
    pub album: bool,
    /// Track titles, artists, composers and conductors.
    pub tracks: bool,
    /// Replace values that are already set; otherwise only blanks are filled.
    pub overwrite: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            album: true,
            tracks: true,
            overwrite: true,
        }
    }
}

/// What applying a release did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// Album-level fields set.
    pub fields_set: usize,
    /// Track titles set.
    pub titles_set: usize,
    /// Things worth a second look: unmatched tracks, lengths that disagree.
    pub notes: Vec<String>,
}

fn set_album_field(plan: &mut SplitPlan, key: FieldKey, value: &str, overwrite: bool) -> bool {
    let value = value.trim();
    if value.is_empty() || (!overwrite && !plan.album.value(&key).trim().is_empty()) {
        return false;
    }
    if plan.album.value(&key) == value {
        return false;
    }
    plan.album.fields.set(key, value);
    true
}

/// Copies the track list (disc `disc`, counted from 0) and album details of `release`
/// into `plan`, matching tracks by number.
pub fn apply(
    plan: &mut SplitPlan,
    release: &Release,
    disc: usize,
    options: ApplyOptions,
) -> Applied {
    let mut done = Applied::default();
    if options.album {
        let fields = [
            (FieldKey::ALBUM, release.title.as_str()),
            (FieldKey::ALBUM_ARTIST, release.artist.as_str()),
            // The default track artist; tracks that differ keep their own below.
            (FieldKey::ARTIST, release.artist.as_str()),
            (FieldKey::DATE, release.date.as_str()),
            (
                FieldKey::Standard(Standard::Publisher),
                release.label.as_str(),
            ),
            (
                FieldKey::Standard(Standard::Composer),
                release.composer.as_str(),
            ),
            (
                FieldKey::Standard(Standard::Conductor),
                release.conductor.as_str(),
            ),
            (
                FieldKey::Custom(CATALOG_FIELD.to_owned()),
                release.catalog.as_str(),
            ),
        ];
        for (key, value) in fields {
            if set_album_field(plan, key, value, options.overwrite) {
                done.fields_set += 1;
            }
        }
    }
    if !options.tracks {
        return done;
    }
    let Some(chosen) = release.discs.get(disc) else {
        done.notes.push("The release has no such disc.".to_owned());
        return done;
    };
    let numbered = plan.tracks.iter().filter(|t| t.meta.number > 0).count();
    if chosen.tracks.len() != numbered {
        done.notes.push(format!(
            "The release has {} tracks on this disc but the plan has {numbered}; only tracks with the same number were filled in.",
            chosen.tracks.len(),
        ));
    }

    let durations: Vec<Option<f64>> = (0..plan.tracks.len())
        .map(|index| plan.duration_of(index))
        .collect();
    // The album and the tracks are separate fields, so one can be read while the other changes.
    let album = &plan.album;
    for (track, duration) in plan.tracks.iter_mut().zip(durations) {
        let number = track.meta.number;
        let Some(info) = chosen.tracks.iter().find(|t| t.position == number) else {
            if number > 0 {
                done.notes
                    .push(format!("Track {number} is not on the release."));
            }
            continue;
        };
        let wanted = [
            (FieldKey::TITLE, info.title.as_str()),
            (FieldKey::ARTIST, info.artist.as_str()),
            (
                FieldKey::Standard(Standard::Composer),
                info.composer.as_str(),
            ),
            (
                FieldKey::Standard(Standard::Conductor),
                info.conductor.as_str(),
            ),
        ];
        for (key, value) in wanted {
            let value = value.trim();
            let blank = album.resolve(&track.meta, &key).trim().is_empty();
            if value.is_empty() || !(options.overwrite || blank) {
                continue;
            }
            // `assign` stores a value on the track only when it differs from the album's.
            track.meta.assign(album, key.clone(), value);
            if key == FieldKey::TITLE {
                done.titles_set += 1;
            }
        }
        if let (Some(wanted), Some(actual)) = (info.seconds, duration)
            && (wanted - actual).abs() > LENGTH_TOLERANCE_SECONDS
        {
            done.notes.push(format!(
                "Track {number} is {} on the release but {} here; this may be a different edition.",
                crate::timecode::format(wanted),
                crate::timecode::format(actual)
            ));
        }
    }
    done
}

/// A completed lookup and what it changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Looked {
    /// The release that was found.
    pub found: release_lookup::Found,
    /// What was copied into the plan.
    pub applied: Applied,
}

/// Finds the release and applies it to `plan`.
///
/// `fetch` performs a GET (see [`release_lookup::Fetch`]), which keeps the network out of
/// this module and makes it testable.
pub fn lookup(
    plan: &mut SplitPlan,
    source: Source,
    token: Option<&str>,
    target: &Target,
    fetch: Fetch<'_>,
    options: ApplyOptions,
) -> Result<Looked, release_lookup::Error> {
    let tracks = plan.tracks.iter().filter(|t| t.meta.number > 0).count();
    let found = release_lookup::find(source, token, target, tracks, fetch)?;
    let applied = apply(plan, &found.release, 0, options);
    Ok(Looked { found, applied })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use release_lookup::{Disc, Request, TrackInfo};

    use super::*;
    use crate::cue::{CueSheet, CueTrack};
    use crate::plan::CueOptions;

    fn plan(tracks: &[(u32, &str, f64)]) -> SplitPlan {
        let tracks = tracks
            .iter()
            .map(|(number, title, start)| CueTrack {
                number: *number,
                title: (*title).to_owned(),
                artist: String::new(),
                composer: String::new(),
                start_seconds: *start,
                pregap_seconds: None,
                isrc: String::new(),
                source_file: "a.dsf".to_owned(),
            })
            .collect();
        SplitPlan::from_cue(
            &CueSheet {
                tracks,
                ..CueSheet::default()
            },
            CueOptions::default(),
        )
    }

    fn track(position: u32, title: &str, artist: &str, seconds: Option<f64>) -> TrackInfo {
        TrackInfo {
            position,
            title: title.to_owned(),
            artist: artist.to_owned(),
            composer: String::new(),
            conductor: String::new(),
            seconds,
        }
    }

    fn release() -> Release {
        Release {
            source: Source::MusicBrainz,
            id: "id".to_owned(),
            title: "Kind of Blue".to_owned(),
            artist: "Miles Davis".to_owned(),
            date: "2004".to_owned(),
            label: "Columbia".to_owned(),
            catalog: "CN 90887".to_owned(),
            barcode: String::new(),
            composer: String::new(),
            conductor: String::new(),
            discs: vec![Disc {
                position: 1,
                format: "CD".to_owned(),
                tracks: vec![
                    track(1, "So What", "Miles Davis", Some(565.0)),
                    track(2, "Freddie Freeloader", "Miles Davis", Some(589.0)),
                    track(3, "Blue in Green", "Miles Davis & Bill Evans", Some(337.0)),
                ],
            }],
        }
    }

    fn title(plan: &SplitPlan, index: usize) -> String {
        plan.album
            .resolve(&plan.tracks[index].meta, &FieldKey::TITLE)
            .to_owned()
    }

    #[test]
    fn applying_fills_titles_artists_and_album_details() {
        let mut plan = plan(&[
            (1, "Track 01", 0.0),
            (2, "Track 02", 565.0),
            (3, "Track 03", 1154.0),
        ]);
        let done = apply(&mut plan, &release(), 0, ApplyOptions::default());

        assert_eq!(
            [title(&plan, 0), title(&plan, 1), title(&plan, 2)],
            ["So What", "Freddie Freeloader", "Blue in Green"]
        );
        assert_eq!(done.titles_set, 3);
        assert_eq!(plan.album.value(&FieldKey::ALBUM), "Kind of Blue");
        assert_eq!(plan.album.value(&FieldKey::ALBUM_ARTIST), "Miles Davis");
        assert_eq!(plan.album.value(&FieldKey::DATE), "2004");
        assert_eq!(
            plan.album.value(&FieldKey::Standard(Standard::Publisher)),
            "Columbia"
        );
        assert_eq!(
            plan.album
                .value(&FieldKey::Custom(CATALOG_FIELD.to_owned())),
            "CN 90887"
        );
        assert_eq!(done.fields_set, 6);

        // Only the track whose artist differs from the album artist keeps its own.
        let artist = |i: usize| {
            plan.album
                .resolve(&plan.tracks[i].meta, &FieldKey::ARTIST)
                .to_owned()
        };
        assert_eq!(artist(0), "Miles Davis");
        assert_eq!(artist(2), "Miles Davis & Bill Evans");
        assert!(
            plan.tracks[0]
                .meta
                .overrides
                .get(&FieldKey::ARTIST)
                .is_none()
        );
        assert!(done.notes.is_empty(), "{:?}", done.notes);
    }

    #[test]
    fn composers_and_conductors_go_to_the_album_or_the_track() {
        let mut release = release();
        release.conductor = "Simon Rattle".to_owned();
        release.discs[0].tracks[1].composer = "Miles Davis".to_owned();
        let mut plan = plan(&[(1, "a", 0.0), (2, "b", 565.0), (3, "c", 1154.0)]);
        apply(&mut plan, &release, 0, ApplyOptions::default());

        let composer = FieldKey::Standard(Standard::Composer);
        let conductor = FieldKey::Standard(Standard::Conductor);
        assert_eq!(plan.album.value(&conductor), "Simon Rattle");
        assert_eq!(
            plan.album.resolve(&plan.tracks[1].meta, &composer),
            "Miles Davis"
        );
        assert_eq!(plan.album.resolve(&plan.tracks[0].meta, &composer), "");
        assert_eq!(
            plan.album.resolve(&plan.tracks[2].meta, &conductor),
            "Simon Rattle"
        );
    }

    #[test]
    fn without_overwrite_existing_values_are_kept() {
        let mut plan = plan(&[
            (1, "My title", 0.0),
            (2, "", 565.0),
            (3, "Track 03", 1154.0),
        ]);
        plan.album.fields.set(FieldKey::ALBUM, "My album");
        let options = ApplyOptions {
            overwrite: false,
            ..ApplyOptions::default()
        };
        apply(&mut plan, &release(), 0, options);
        assert_eq!(title(&plan, 0), "My title", "kept");
        assert_eq!(title(&plan, 1), "Freddie Freeloader", "blank, so filled");
        assert_eq!(plan.album.value(&FieldKey::ALBUM), "My album");
        assert_eq!(
            plan.album.value(&FieldKey::DATE),
            "2004",
            "blank album fields are filled"
        );
    }

    #[test]
    fn mismatches_are_reported_not_hidden() {
        // The second track is far shorter than on the release; track 9 is not on it.
        let mut plan = plan(&[(1, "a", 0.0), (2, "b", 565.0), (9, "c", 700.0)]);
        let done = apply(&mut plan, &release(), 0, ApplyOptions::default());
        let notes = done.notes.join("\n");
        assert!(notes.contains("Track 9 is not on the release"), "{notes}");
        assert!(notes.contains("Track 2 is"), "length mismatch: {notes}");
        assert_eq!(title(&plan, 2), "c", "an unmatched track is left alone");

        let mut one = plan_with_one();
        let missing = apply(&mut one, &release(), 5, ApplyOptions::default());
        assert!(missing.notes.iter().any(|n| n.contains("no such disc")));
    }

    fn plan_with_one() -> SplitPlan {
        plan(&[(1, "a", 0.0)])
    }

    #[test]
    fn album_only_and_tracks_only_are_independent() {
        let mut only_tracks = plan_with_one();
        apply(
            &mut only_tracks,
            &release(),
            0,
            ApplyOptions {
                album: false,
                ..ApplyOptions::default()
            },
        );
        assert_eq!(only_tracks.album.value(&FieldKey::ALBUM), "");
        assert_eq!(title(&only_tracks, 0), "So What");

        let mut only_album = plan_with_one();
        apply(
            &mut only_album,
            &release(),
            0,
            ApplyOptions {
                tracks: false,
                ..ApplyOptions::default()
            },
        );
        assert_eq!(only_album.album.value(&FieldKey::ALBUM), "Kind of Blue");
        assert_eq!(title(&only_album, 0), "a");
    }

    /// A fetcher that serves the same two replies whichever service is asked.
    fn fake(requests: &RefCell<Vec<Request>>) -> impl Fn(&Request) -> Result<String, String> + '_ {
        move |request: &Request| {
            requests.borrow_mut().push(request.clone());
            if request.url.contains("?query=") {
                Ok(r#"{"releases":[{"id":"e8324f18-31cd-4a2f-877b-08affdccfff1","title":"Kind of Blue",
                    "artist-credit":[{"name":"Miles Davis"}],"media":[{"track-count":3}]}]}"#
                    .to_owned())
            } else {
                Ok(r#"{"id":"e8324f18-31cd-4a2f-877b-08affdccfff1","title":"Kind of Blue","date":"2004",
                    "artist-credit":[{"name":"Miles Davis"}],
                    "media":[{"tracks":[{"position":1,"title":"So What"},{"position":2,"title":"Freddie"},
                                        {"position":3,"title":"Blue in Green"}]}]}"#
                    .to_owned())
            }
        }
    }

    #[test]
    fn a_lookup_finds_the_release_and_applies_it() {
        let mut plan = plan(&[(1, "a", 0.0), (2, "b", 565.0), (3, "c", 1154.0)]);
        let target = Target::Text("kind of blue [1st press]".to_owned());
        let requests = RefCell::new(Vec::new());
        let looked = lookup(
            &mut plan,
            Source::MusicBrainz,
            None,
            &target,
            &fake(&requests),
            ApplyOptions::default(),
        )
        .expect("lookup works");
        assert_eq!(requests.borrow().len(), 2);
        assert!(
            requests.borrow()[0].url.contains("%5C%5B1st%20press%5C%5D"),
            "typed text is escaped and used, not the album's own title: {}",
            requests.borrow()[0].url
        );
        assert_eq!(looked.applied.titles_set, 3);
        assert_eq!(title(&plan, 0), "So What");
    }
}
