//! MusicBrainz web service v2: request builders and parsers.
//!
//! MusicBrainz data is released under CC0. Clients must identify themselves with a
//! `User-Agent` and make at most one request per second.

use serde::Deserialize;

use crate::Error;
use crate::model::{Candidate, Disc, Release, Request, Source, TrackInfo, encode};

const API: &str = "https://musicbrainz.org/ws/2/release";
/// The most search hits kept.
pub const MAX_CANDIDATES: usize = 10;

#[derive(Debug, Deserialize)]
struct Credit {
    #[serde(default)]
    name: String,
    #[serde(default)]
    joinphrase: String,
}

#[derive(Debug, Deserialize)]
struct LabelName {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct LabelInfo {
    #[serde(rename = "catalog-number")]
    catalog_number: Option<String>,
    label: Option<LabelName>,
}

#[derive(Debug, Deserialize)]
struct RawTrack {
    position: Option<u32>,
    #[serde(default)]
    title: String,
    /// Milliseconds.
    length: Option<u64>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<Credit>,
}

#[derive(Debug, Deserialize)]
struct RawMedium {
    format: Option<String>,
    position: Option<u32>,
    #[serde(rename = "track-count")]
    track_count: Option<u32>,
    #[serde(default)]
    tracks: Vec<RawTrack>,
}

#[derive(Debug, Deserialize)]
struct RawRelease {
    id: String,
    /// Search relevance, 0–100 (only in search results).
    score: Option<u32>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: String,
    country: Option<String>,
    barcode: Option<String>,
    #[serde(rename = "track-count")]
    track_count: Option<u32>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<Credit>,
    #[serde(rename = "label-info", default)]
    label_info: Vec<LabelInfo>,
    #[serde(default)]
    media: Vec<RawMedium>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    releases: Vec<RawRelease>,
}

/// "A", "A & B" … from an artist credit.
fn credit_text(credits: &[Credit]) -> String {
    credits
        .iter()
        .map(|c| format!("{}{}", c.name, c.joinphrase))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn first_label(info: &[LabelInfo]) -> (String, String) {
    let label = info
        .iter()
        .find_map(|i| i.label.as_ref().map(|l| l.name.clone()))
        .unwrap_or_default();
    let catalog = info
        .iter()
        .find_map(|i| i.catalog_number.clone())
        .unwrap_or_default();
    (label, catalog)
}

/// Results of a release search.
pub fn parse_search(json: &str) -> Result<Vec<Candidate>, Error> {
    let response: SearchResponse =
        serde_json::from_str(json).map_err(|e| Error::UnexpectedReply {
            source: Source::MusicBrainz,
            detail: e.to_string(),
        })?;
    Ok(response
        .releases
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|r| {
            let media_tracks: u32 = r.media.iter().filter_map(|m| m.track_count).sum();
            let mut formats: Vec<String> =
                r.media.iter().filter_map(|m| m.format.clone()).collect();
            formats.dedup();
            let (label, _) = first_label(&r.label_info);
            Candidate {
                source: Source::MusicBrainz,
                tracks: if media_tracks > 0 {
                    media_tracks
                } else {
                    r.track_count.unwrap_or(0)
                },
                artist: credit_text(&r.artist_credit),
                id: r.id,
                title: r.title,
                date: r.date,
                country: r.country.unwrap_or_default(),
                label,
                barcode: r.barcode.unwrap_or_default(),
                formats: formats.join(" + "),
                score: r.score,
            }
        })
        .collect())
}

/// A release with its full track list.
pub fn parse_release(json: &str) -> Result<Release, Error> {
    let r: RawRelease = serde_json::from_str(json).map_err(|e| Error::UnexpectedReply {
        source: Source::MusicBrainz,
        detail: e.to_string(),
    })?;
    let (label, catalog) = first_label(&r.label_info);
    let discs = r
        .media
        .into_iter()
        .enumerate()
        .map(|(i, medium)| Disc {
            position: medium.position.unwrap_or(i as u32 + 1),
            format: medium.format.unwrap_or_default(),
            tracks: medium
                .tracks
                .into_iter()
                .enumerate()
                .map(|(t, track)| TrackInfo {
                    position: track.position.unwrap_or(t as u32 + 1),
                    title: track.title,
                    artist: credit_text(&track.artist_credit),
                    // Credits need MusicBrainz's work relationships, which are not requested.
                    composer: String::new(),
                    conductor: String::new(),
                    seconds: track.length.map(|ms| ms as f64 / 1000.0),
                })
                .collect(),
        })
        .collect();
    Ok(Release {
        source: Source::MusicBrainz,
        id: r.id,
        title: r.title,
        artist: credit_text(&r.artist_credit),
        date: r.date,
        label,
        catalog,
        barcode: r.barcode.unwrap_or_default(),
        composer: String::new(),
        conductor: String::new(),
        discs,
    })
}

/// A Lucene phrase with quotes and backslashes made harmless.
fn phrase(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !matches!(c, '"' | '\\') && !c.is_control())
        .collect();
    format!("\"{}\"", cleaned.trim())
}

fn search(query: &str) -> Request {
    Request::get(format!(
        "{API}/?query={}&fmt=json&limit={MAX_CANDIDATES}",
        encode(query)
    ))
}

/// Search by release title and (optionally) artist. `None` if there is no title.
pub fn search_request(artist: &str, album: &str) -> Option<Request> {
    let album = album.trim();
    if album.is_empty() {
        return None;
    }
    let mut query = format!("release:{}", phrase(album));
    if !artist.trim().is_empty() {
        query.push_str(&format!(" AND artist:{}", phrase(artist)));
    }
    Some(search(&query))
}

/// Escapes the characters that mean something in Lucene query syntax, so text typed by a
/// person ("Blood on the Tracks [1st Press]", "AC/DC", "Live: 1975") is searched literally
/// instead of being rejected as a malformed query.
pub fn lucene_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().filter(|c| !c.is_control()) {
        if matches!(
            c,
            '+' | '-'
                | '!'
                | '('
                | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | '^'
                | '"'
                | '~'
                | '*'
                | '?'
                | ':'
                | '\\'
                | '/'
                | '&'
                | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Search by free text, as typed by a person. `None` if it is blank.
pub fn text_search_request(text: &str) -> Option<Request> {
    let text = text.trim();
    (!text.is_empty()).then(|| search(&lucene_escape(text)))
}

/// Whether `id` looks like a MusicBrainz release id (a UUID).
pub fn is_release_id(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Finds a release id in pasted text, such as `https://musicbrainz.org/release/<id>`.
pub fn find_release_id(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(35))
        .filter_map(|start| text.get(start..start + 36))
        .find(|candidate| is_release_id(candidate))
        .map(str::to_owned)
}

/// The request for a release's full track list; `None` if `id` is not a release id.
pub fn release_request(id: &str) -> Option<Request> {
    is_release_id(id).then(|| {
        Request::get(format!(
            "{API}/{id}?inc=recordings+artist-credits+labels&fmt=json"
        ))
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// A trimmed reply in the shape MusicBrainz returns for a release search.
    pub const SEARCH: &str = r#"{"created":"2026-10-18T07:29:29.721Z","count":138,"offset":0,"releases":[
      {"id":"e32a3f0b-1c19-3170-bb1c-650893774744","score":100,"title":"Kind of Blue","status":"Official",
       "artist-credit":[{"name":"Miles Davis","artist":{"id":"561d854a","name":"Miles Davis"}}],
       "date":"1987","country":"US","barcode":"074644057928",
       "label-info":[{"catalog-number":"CK 40579","label":{"id":"x","name":"Columbia"}}],
       "track-count":5,"media":[{"format":"CD","disc-count":5,"track-count":5}]},
      {"id":"e8324f18-31cd-4a2f-877b-08affdccfff1","score":100,"title":"Kind of Blue",
       "artist-credit":[{"name":"Miles Davis","joinphrase":""}],"date":"2004",
       "media":[{"format":"DualDisc","track-count":6},{"format":"SACD","track-count":5}]}]}"#;

    /// A trimmed reply for a release lookup.
    pub const RELEASE: &str = r#"{"id":"e8324f18-31cd-4a2f-877b-08affdccfff1","title":"Kind of Blue","date":"2004",
      "artist-credit":[{"name":"Miles Davis","joinphrase":""}],"barcode":"827969088722",
      "label-info":[{"catalog-number":"CN 90887","label":{"name":"Columbia"}}],
      "media":[{"position":1,"format":"DualDisc","track-count":3,"tracks":[
        {"position":1,"number":"1","title":"So What","length":565000,
         "artist-credit":[{"name":"Miles Davis","joinphrase":""}]},
        {"position":2,"number":"2","title":"Freddie Freeloader","length":589000,
         "artist-credit":[{"name":"Miles Davis","joinphrase":""}]},
        {"position":3,"number":"3","title":"Blue in Green","length":337000,
         "artist-credit":[{"name":"Miles Davis & Bill Evans","joinphrase":""}]}]}]}"#;
}

#[cfg(test)]
mod tests {
    use super::fixtures::{RELEASE, SEARCH};
    use super::*;

    #[test]
    fn search_replies_become_candidates() {
        let found = parse_search(SEARCH).expect("valid reply");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].artist, "Miles Davis");
        assert_eq!(
            (found[0].date.as_str(), found[0].country.as_str()),
            ("1987", "US")
        );
        assert_eq!((found[0].label.as_str(), found[0].tracks), ("Columbia", 5));
        assert_eq!(found[0].source, Source::MusicBrainz);
        assert_eq!(found[1].tracks, 11, "tracks on every disc are counted");
        assert_eq!(found[1].formats, "DualDisc + SACD");
    }

    #[test]
    fn odd_replies_are_errors_or_empty_not_panics() {
        assert!(parse_search("not json").is_err());
        assert_eq!(parse_search("{}").expect("empty is fine"), Vec::new());
        assert!(parse_release("{}").is_err(), "a release needs an id");
        assert!(parse_release("[]").is_err());
        let sparse = parse_release(r#"{"id":"x"}"#).expect("only an id");
        assert!(sparse.discs.is_empty() && sparse.title.is_empty());
    }

    #[test]
    fn a_release_reply_has_its_track_list() {
        let release = parse_release(RELEASE).expect("valid reply");
        assert_eq!(release.title, "Kind of Blue");
        assert_eq!(
            (release.label.as_str(), release.catalog.as_str()),
            ("Columbia", "CN 90887")
        );
        let disc = &release.discs[0];
        assert_eq!(disc.tracks.len(), 3);
        assert_eq!(disc.tracks[0].title, "So What");
        assert_eq!(disc.tracks[0].seconds, Some(565.0));
        assert_eq!(disc.tracks[2].artist, "Miles Davis & Bill Evans");
    }

    #[test]
    fn requests_are_encoded_and_validated() {
        let url = search_request("Miles Davis", "Kind of \"Blue\"")
            .expect("request")
            .url;
        assert!(
            url.starts_with("https://musicbrainz.org/ws/2/release/?query=release%3A%22Kind%20of%20Blue%22%20AND%20artist%3A%22Miles%20Davis%22"),
            "{url}"
        );
        assert!(url.ends_with("&fmt=json&limit=10"));
        assert!(search_request("Anyone", "  ").is_none());
        assert!(search_request("", "Album").is_some());

        let free = text_search_request(" bob dylan \"blood\" on the tracks ")
            .expect("request")
            .url;
        assert!(
            free.contains("query=bob%20dylan%20%5C%22blood%5C%22%20on%20the%20tracks&"),
            "{free}"
        );
        assert!(text_search_request("  ").is_none());

        assert!(release_request("e8324f18-31cd-4a2f-877b-08affdccfff1").is_some());
        assert!(release_request("../../etc/passwd").is_none());
        assert!(release_request("e8324f18-31cd-4a2f-877b-08affdccfffZ").is_none());
    }

    #[test]
    fn typed_text_with_query_syntax_is_searched_literally() {
        assert_eq!(
            lucene_escape("Blood On The Tracks [1st Press US]"),
            r"Blood On The Tracks \[1st Press US\]"
        );
        assert_eq!(
            lucene_escape("AC/DC: Live (1978) && more!"),
            r"AC\/DC\: Live \(1978\) \&\& more\!"
        );
        assert_eq!(lucene_escape("plain words"), "plain words");
        assert_eq!(
            lucene_escape("a\nb"),
            "ab",
            "control characters are dropped"
        );
        let url = text_search_request("Blood On The Tracks [1st Press US]")
            .expect("request")
            .url;
        assert!(url.contains("%5C%5B1st%20Press%20US%5C%5D"), "{url}");
    }

    #[test]
    fn a_release_id_is_found_in_pasted_text() {
        let id = "e8324f18-31cd-4a2f-877b-08affdccfff1";
        assert_eq!(find_release_id(id).as_deref(), Some(id));
        assert_eq!(
            find_release_id(&format!("https://musicbrainz.org/release/{id}?x=1")).as_deref(),
            Some(id)
        );
        assert_eq!(find_release_id("  ").as_deref(), None);
        assert_eq!(find_release_id("1111111111").as_deref(), None);
        assert_eq!(find_release_id("e8324f18-31cd-4a2f-877b").as_deref(), None);
        assert_eq!(
            find_release_id("日本語のテキスト日本語のテキスト日本語のテキスト日本語").as_deref(),
            None
        );
    }
}
