//! Find an album release on MusicBrainz or Discogs and read its track list.
//!
//! The crate separates *what to ask* from *how to ask it*. The request builders and parsers
//! in [`musicbrainz`] and [`discogs`] are plain functions over text, so they work with any
//! HTTP client, blocking or async. [`find`] runs the usual search-then-fetch flow for you
//! given a closure that performs a GET; the optional `http` feature supplies one, with the
//! pacing and headers each service requires.
//!
//! ```
//! use release_lookup::{Source, Target, find, Request};
//!
//! // Any function from a request to a response body will do; here, a canned one.
//! let fetch = |request: &Request| -> Result<String, String> {
//!     if request.url.contains("database/search") {
//!         Ok(r#"{"results":[{"id":1,"type":"release","title":"Artist - Album","year":"1999"}]}"#.into())
//!     } else {
//!         Ok(r#"{"id":1,"title":"Album","artists":[{"name":"Artist"}],
//!                "tracklist":[{"type_":"track","title":"First","duration":"3:00"}]}"#.into())
//!     }
//! };
//! let target = Target::Search { artist: "Artist".into(), album: "Album".into() };
//! let found = find(Source::Discogs, None, &target, 1, &fetch)?;
//! assert_eq!(found.release.discs[0].tracks[0].title, "First");
//! # Ok::<(), String>(())
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

pub mod discogs;
#[cfg(feature = "http")]
pub mod http;
mod model;
pub mod musicbrainz;

pub use model::{Candidate, Disc, Release, Request, Source, TrackInfo};

/// What to look up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A release id, a link to a release, or the service's own short form.
    Id(String),
    /// A release title and artist, matched field by field.
    Search {
        /// May be empty.
        artist: String,
        /// The release title.
        album: String,
    },
    /// Text typed by a person, searched as a whole.
    Text(String),
}

impl Target {
    /// Interprets what a person typed into a search box.
    ///
    /// A release id or link for `source` is recognised as such; any other text is searched
    /// as typed; a blank box falls back to `album` (and `artist`) from the album being edited.
    /// Returns `None` when there is nothing to look for.
    pub fn from_input(source: Source, input: &str, artist: &str, album: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return (!album.trim().is_empty()).then(|| Self::Search {
                artist: artist.trim().to_owned(),
                album: album.trim().to_owned(),
            });
        }
        let id = match source {
            Source::MusicBrainz => musicbrainz::find_release_id(input),
            Source::Discogs => discogs::find_release_id(input),
        };
        Some(match id {
            Some(id) => Self::Id(id),
            None => Self::Text(input.to_owned()),
        })
    }

    /// What is being looked up, for a status line: `“Blue” by “Miles Davis”`.
    pub fn describe(&self) -> String {
        match self {
            Self::Id(id) => format!("release {id}"),
            Self::Text(text) => format!("“{text}”"),
            Self::Search { artist, album } if artist.trim().is_empty() => format!("“{album}”"),
            Self::Search { artist, album } => format!("“{album}” by “{artist}”"),
        }
    }
}

/// A performs-a-GET function: the crate never opens a connection itself.
pub type Fetch<'a> = &'a dyn Fn(&Request) -> Result<String, String>;

/// What a lookup found.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The release with its track list.
    pub release: Release,
    /// The search hit that was used; `None` when an id was given.
    pub candidate: Option<Candidate>,
}

/// Hits scoring within this many points of the best count as equally relevant.
const SCORE_SLACK: u32 = 5;
/// When the service gives no scores, only this many top hits count as relevant.
const UNSCORED_TOP: usize = 3;

/// Picks the candidate to use without asking.
///
/// Among the hits that are about as relevant as the best one, the first whose track count
/// equals `track_count` wins; otherwise the first hit does. Relevance always comes first, so
/// a worse match is never chosen just because its track count happens to fit.
pub fn choose(candidates: &[Candidate], track_count: usize) -> Option<&Candidate> {
    let best = candidates.iter().filter_map(|c| c.score).max();
    candidates
        .iter()
        .enumerate()
        .filter(|(index, c)| match (best, c.score) {
            (Some(best), Some(score)) => score + SCORE_SLACK >= best,
            _ => *index < UNSCORED_TOP,
        })
        .map(|(_, c)| c)
        .find(|c| c.tracks > 0 && c.tracks as usize == track_count)
        .or_else(|| candidates.first())
}

/// The text without bracketed or parenthesised notes, such as pressing details:
/// `"Blood On The Tracks [1st Press US] (1975)"` becomes `"Blood On The Tracks"`.
///
/// `None` if nothing would change or nothing would be left.
pub fn simplified(text: &str) -> Option<String> {
    let mut depth = 0u32;
    let mut kept = String::new();
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => kept.push(c),
            _ => {}
        }
    }
    let kept = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    (!kept.is_empty() && kept != text.trim()).then_some(kept)
}

/// The same target with notes stripped from its title, if that makes a difference.
fn simpler(target: &Target) -> Option<Target> {
    match target {
        Target::Id(_) => None,
        Target::Text(text) => simplified(text).map(Target::Text),
        Target::Search { artist, album } => simplified(album).map(|album| Target::Search {
            artist: artist.clone(),
            album,
        }),
    }
}

/// Searches, and if that finds nothing tries again without bracketed notes in the title.
/// Returns the hits and the target that produced them.
fn search_hits(
    source: Source,
    token: Option<&str>,
    target: &Target,
    fetch: Fetch<'_>,
) -> Result<(Vec<Candidate>, Target), String> {
    let request = search_request(source, token, target)
        .ok_or_else(|| "there is nothing to search for".to_owned())?;
    let hits = parse_search(source, &fetch(&request)?)?;
    if hits.is_empty()
        && let Some(retry) = simpler(target)
        && let Some(request) = search_request(source, token, &retry)
    {
        let hits = parse_search(source, &fetch(&request)?)?;
        return Ok((hits, retry));
    }
    Ok((hits, target.clone()))
}

fn search_request(source: Source, token: Option<&str>, target: &Target) -> Option<Request> {
    match (source, target) {
        (_, Target::Id(_)) => None,
        (Source::MusicBrainz, Target::Search { artist, album }) => {
            musicbrainz::search_request(artist, album)
        }
        (Source::MusicBrainz, Target::Text(text)) => musicbrainz::text_search_request(text),
        (Source::Discogs, Target::Search { artist, album }) => {
            discogs::search_request(artist, album, token)
        }
        (Source::Discogs, Target::Text(text)) => discogs::text_search_request(text, token),
    }
}

fn release_request(source: Source, token: Option<&str>, id: &str) -> Option<Request> {
    match source {
        Source::MusicBrainz => musicbrainz::release_request(id),
        Source::Discogs => discogs::release_request(id, token),
    }
}

fn parse_search(source: Source, json: &str) -> Result<Vec<Candidate>, String> {
    match source {
        Source::MusicBrainz => musicbrainz::parse_search(json),
        Source::Discogs => discogs::parse_search(json),
    }
}

fn parse_release(source: Source, json: &str) -> Result<Release, String> {
    match source {
        Source::MusicBrainz => musicbrainz::parse_release(json),
        Source::Discogs => discogs::parse_release(json),
    }
}

/// Searches (unless `target` is an id) and fetches the chosen release.
///
/// `track_count` helps choose between search hits. `token` is a Discogs personal access
/// token, ignored for MusicBrainz.
pub fn find(
    source: Source,
    token: Option<&str>,
    target: &Target,
    track_count: usize,
    fetch: Fetch<'_>,
) -> Result<Found, String> {
    let mut chosen = None;
    let id = match target {
        Target::Id(id) => id.clone(),
        _ => {
            let (candidates, used) = search_hits(source, token, target, fetch)?;
            let best = choose(&candidates, track_count)
                .ok_or_else(|| {
                    if used == *target {
                        format!("{source} has no release matching {}", target.describe())
                    } else {
                        format!(
                            "{source} has no release matching {} (also tried {})",
                            target.describe(),
                            used.describe()
                        )
                    }
                })?
                .clone();
            let id = best.id.clone();
            chosen = Some(best);
            id
        }
    };
    let request = release_request(source, token, &id)
        .ok_or_else(|| format!("'{id}' is not a {source} release id"))?;
    let release = parse_release(source, &fetch(&request)?)?;
    Ok(Found {
        release,
        candidate: chosen,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A fetcher serving fixtures for both services and recording every request.
    fn fake(requests: &RefCell<Vec<Request>>) -> impl Fn(&Request) -> Result<String, String> + '_ {
        move |request: &Request| {
            requests.borrow_mut().push(request.clone());
            let url = request.url.as_str();
            if url.contains("musicbrainz.org") && url.contains("?query=") {
                Ok(musicbrainz::fixtures::SEARCH.to_owned())
            } else if url.contains("musicbrainz.org") {
                Ok(musicbrainz::fixtures::RELEASE.to_owned())
            } else if url.contains("database/search") {
                Ok(discogs::fixtures::SEARCH.to_owned())
            } else if url.contains("api.discogs.com/releases/") {
                Ok(discogs::fixtures::RELEASE.to_owned())
            } else {
                Err(format!("unexpected request {url}"))
            }
        }
    }

    fn album() -> Target {
        Target::Search {
            artist: "Miles Davis".into(),
            album: "Kind of Blue".into(),
        }
    }

    #[test]
    fn musicbrainz_searches_then_fetches_the_hit_with_the_right_track_count() {
        let requests = RefCell::new(Vec::new());
        let found = find(Source::MusicBrainz, None, &album(), 11, &fake(&requests)).expect("found");
        assert_eq!(requests.borrow().len(), 2);
        assert!(
            requests.borrow()[1]
                .url
                .contains("e8324f18-31cd-4a2f-877b-08affdccfff1")
        );
        assert_eq!(found.candidate.map(|c| c.date), Some("2004".to_owned()));
        assert_eq!(found.release.discs[0].tracks.len(), 3);
    }

    #[test]
    fn discogs_searches_then_fetches_the_first_hit_and_sends_the_token() {
        let requests = RefCell::new(Vec::new());
        let found =
            find(Source::Discogs, Some("tok"), &album(), 4, &fake(&requests)).expect("found");
        let requests = requests.borrow();
        assert!(
            requests[0]
                .url
                .contains("release_title=Kind%20of%20Blue&artist=Miles%20Davis")
        );
        assert!(requests[1].url.ends_with("/releases/2772432"));
        for request in requests.iter() {
            assert_eq!(request.headers.len(), 1, "{}", request.url);
        }
        assert_eq!(found.release.source, Source::Discogs);
        assert_eq!(found.release.discs[0].tracks.len(), 4);
    }

    #[test]
    fn an_id_skips_the_search() {
        let requests = RefCell::new(Vec::new());
        let found = find(
            Source::Discogs,
            None,
            &Target::Id("249504".into()),
            0,
            &fake(&requests),
        )
        .expect("found");
        assert_eq!(requests.borrow().len(), 1);
        assert!(found.candidate.is_none());
    }

    #[test]
    fn failures_say_what_was_wrong() {
        let requests = RefCell::new(Vec::new());
        let blank = Target::Text("  ".into());
        assert_eq!(
            find(Source::MusicBrainz, None, &blank, 0, &fake(&requests)).expect_err("blank"),
            "there is nothing to search for"
        );
        assert!(requests.borrow().is_empty(), "nothing was requested");

        let bad = find(
            Source::MusicBrainz,
            None,
            &Target::Id("nope".into()),
            0,
            &fake(&requests),
        )
        .expect_err("bad id");
        assert_eq!(bad, "'nope' is not a MusicBrainz release id");

        let offline = |_: &Request| Err::<String, String>("network unreachable".into());
        assert_eq!(
            find(Source::Discogs, None, &album(), 0, &offline).expect_err("offline"),
            "network unreachable"
        );

        let empty = |_: &Request| Ok::<String, String>(r#"{"results":[],"releases":[]}"#.into());
        let none = find(Source::Discogs, None, &album(), 0, &empty).expect_err("no hits");
        assert!(
            none.contains("Discogs has no release matching “Kind of Blue” by “Miles Davis”"),
            "{none}"
        );
    }

    #[test]
    fn typed_input_is_an_id_when_it_looks_like_one_and_a_search_otherwise() {
        let id = "e8324f18-31cd-4a2f-877b-08affdccfff1";
        let from = |source, input: &str| {
            Target::from_input(source, input, "Bob Dylan", "Blood On The Tracks")
        };

        assert_eq!(from(Source::MusicBrainz, id), Some(Target::Id(id.into())));
        assert_eq!(
            from(
                Source::MusicBrainz,
                &format!("https://musicbrainz.org/release/{id}")
            ),
            Some(Target::Id(id.into()))
        );
        // The bug that prompted this: arbitrary text must be searched, not ignored.
        assert_eq!(
            from(Source::MusicBrainz, "1111111111"),
            Some(Target::Text("1111111111".into()))
        );
        assert_eq!(
            from(Source::Discogs, "https://www.discogs.com/release/249504-x"),
            Some(Target::Id("249504".into()))
        );
        assert_eq!(
            from(Source::Discogs, "1984"),
            Some(Target::Text("1984".into()))
        );
        assert_eq!(
            from(Source::Discogs, "  some words  "),
            Some(Target::Text("some words".into()))
        );
        assert_eq!(
            from(Source::MusicBrainz, "   "),
            Some(Target::Search {
                artist: "Bob Dylan".into(),
                album: "Blood On The Tracks".into()
            }),
            "a blank box searches for the album being edited"
        );
        assert_eq!(Target::from_input(Source::Discogs, "", "x", " "), None);
    }

    #[test]
    fn targets_describe_themselves() {
        assert_eq!(album().describe(), "“Kind of Blue” by “Miles Davis”");
        assert_eq!(Target::Text("x y".into()).describe(), "“x y”");
        assert_eq!(Target::Id("7".into()).describe(), "release 7");
        let no_artist = Target::Search {
            artist: " ".into(),
            album: "A".into(),
        };
        assert_eq!(no_artist.describe(), "“A”");
    }

    #[test]
    fn choose_prefers_a_matching_track_count_and_ignores_unknown_counts() {
        let make = |id: &str, tracks| Candidate {
            source: Source::Discogs,
            id: id.into(),
            title: String::new(),
            artist: String::new(),
            date: String::new(),
            country: String::new(),
            label: String::new(),
            barcode: String::new(),
            tracks,
            formats: String::new(),
            score: None,
        };
        let list = [make("a", 0), make("b", 5), make("c", 9)];
        assert_eq!(choose(&list, 9).map(|c| c.id.as_str()), Some("c"));
        assert_eq!(
            choose(&list, 0).map(|c| c.id.as_str()),
            Some("a"),
            "unknown counts never match"
        );
        assert_eq!(choose(&list, 7).map(|c| c.id.as_str()), Some("a"));
        assert!(choose(&[], 1).is_none());
    }

    #[test]
    fn relevance_beats_a_matching_track_count() {
        let make = |id: &str, tracks, score| Candidate {
            source: Source::MusicBrainz,
            id: id.into(),
            title: String::new(),
            artist: String::new(),
            date: String::new(),
            country: String::new(),
            label: String::new(),
            barcode: String::new(),
            tracks,
            formats: String::new(),
            score,
        };
        let scored = [
            make("real", 5, Some(100)),
            make("close", 10, Some(97)),
            make("poor", 10, Some(60)),
        ];
        assert_eq!(
            choose(&scored, 10).map(|c| c.id.as_str()),
            Some("close"),
            "within the slack"
        );
        let only_poor = [make("real", 5, Some(100)), make("poor", 10, Some(60))];
        assert_eq!(choose(&only_poor, 10).map(|c| c.id.as_str()), Some("real"));

        let unscored: Vec<Candidate> = (0..6)
            .map(|i| make(&format!("h{i}"), if i == 5 { 10 } else { 3 }, None))
            .collect();
        assert_eq!(choose(&unscored, 10).map(|c| c.id.as_str()), Some("h0"));
    }

    #[test]
    fn notes_in_brackets_are_stripped_for_the_retry() {
        assert_eq!(
            simplified("Blood On The Tracks [1st Press US]").as_deref(),
            Some("Blood On The Tracks")
        );
        assert_eq!(
            simplified("Kind of Blue (1959) [Mono]").as_deref(),
            Some("Kind of Blue")
        );
        assert_eq!(simplified("Plain Title"), None);
        assert_eq!(simplified("[Only a note]"), None);
        assert_eq!(
            simplified("Unbalanced [note").as_deref(),
            Some("Unbalanced")
        );
        assert_eq!(simplified(""), None);
    }

    #[test]
    fn a_search_that_finds_nothing_is_retried_without_the_notes() {
        let requests = RefCell::new(Vec::new());
        let fetch = |request: &Request| -> Result<String, String> {
            requests.borrow_mut().push(request.clone());
            if request.url.contains("database/search") {
                if request.url.contains("1st%20Press") {
                    Ok(r#"{"results":[]}"#.to_owned())
                } else {
                    Ok(discogs::fixtures::SEARCH.to_owned())
                }
            } else {
                Ok(discogs::fixtures::RELEASE.to_owned())
            }
        };
        let target = Target::Search {
            artist: "Bob Dylan".into(),
            album: "Blood On The Tracks [1st Press US]".into(),
        };
        let found = find(Source::Discogs, None, &target, 4, &fetch).expect("found on retry");
        assert_eq!(requests.borrow().len(), 3, "search, retry, release");
        assert!(!requests.borrow()[1].url.contains("1st"));
        assert!(found.candidate.is_some());

        let empty = |_: &Request| Ok::<String, String>(r#"{"results":[]}"#.into());
        let err = find(Source::Discogs, None, &target, 4, &empty).expect_err("nothing");
        assert!(err.contains("also tried"), "{err}");

        let calls = RefCell::new(0);
        let counting = |_: &Request| {
            *calls.borrow_mut() += 1;
            Ok::<String, String>(r#"{"results":[]}"#.into())
        };
        let plain = Target::Search {
            artist: String::new(),
            album: "Plain".into(),
        };
        let _ = find(Source::Discogs, None, &plain, 4, &counting);
        assert_eq!(*calls.borrow(), 1);
    }
}
