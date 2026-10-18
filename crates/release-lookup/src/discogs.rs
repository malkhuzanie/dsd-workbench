//! Discogs database API: request builders and parsers.
//!
//! Searching works without an account, but a personal access token (Discogs → Settings →
//! Developers) raises the rate limit; pass it to the request builders. Discogs asks
//! clients to identify themselves with a `User-Agent`.

use serde::Deserialize;

use crate::model::{Candidate, Disc, Release, Request, Source, TrackInfo, encode};

const API: &str = "https://api.discogs.com";
/// The most search hits kept.
pub const MAX_CANDIDATES: usize = 10;
/// Bare numbers shorter than this are taken as text to search for (an album called "1984").
const MIN_BARE_ID_DIGITS: usize = 5;

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<Hit>,
}

#[derive(Debug, Deserialize)]
struct Hit {
    id: u64,
    /// "Artist - Title".
    #[serde(default)]
    title: String,
    /// Discogs sends the year as text here.
    #[serde(default)]
    year: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    label: Vec<String>,
    #[serde(default)]
    format: Vec<String>,
    #[serde(rename = "type", default)]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct Artist {
    #[serde(default)]
    name: String,
    #[serde(default)]
    join: String,
}

/// A credit with a role, such as `Composed By` or `Conductor`.
#[derive(Debug, Deserialize)]
struct ExtraArtist {
    #[serde(default)]
    name: String,
    #[serde(default)]
    role: String,
}

#[derive(Debug, Deserialize)]
struct Label {
    #[serde(default)]
    name: String,
    #[serde(default)]
    catno: String,
}

#[derive(Debug, Deserialize)]
struct Format {
    #[serde(default)]
    name: String,
    #[serde(default)]
    descriptions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Track {
    #[serde(rename = "type_", default)]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    duration: String,
    #[serde(default)]
    artists: Vec<Artist>,
    #[serde(default)]
    extraartists: Vec<ExtraArtist>,
    #[serde(default)]
    sub_tracks: Vec<Track>,
}

#[derive(Debug, Deserialize)]
struct RawRelease {
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    year: Option<u32>,
    #[serde(default)]
    released: String,
    #[serde(default)]
    artists: Vec<Artist>,
    #[serde(default)]
    extraartists: Vec<ExtraArtist>,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    formats: Vec<Format>,
    #[serde(default)]
    tracklist: Vec<Track>,
}

/// Discogs tells artists with the same name apart with a number: `Prince (2)`.
fn strip_disambiguation(name: &str) -> &str {
    let name = name.trim();
    match name.rsplit_once(" (") {
        Some((base, rest))
            if rest
                .strip_suffix(')')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) =>
        {
            base
        }
        _ => name,
    }
}

fn artist_text(artists: &[Artist]) -> String {
    let mut out = String::new();
    for (i, artist) in artists.iter().enumerate() {
        out.push_str(strip_disambiguation(&artist.name));
        if i + 1 < artists.len() {
            let join = artist.join.trim();
            if join.is_empty() || join == "," {
                out.push_str(", ");
            } else {
                out.push_str(&format!(" {join} "));
            }
        }
    }
    out
}

/// The composer and conductor among `credits`, each as names joined by `", "`.
///
/// Roles are matched loosely (`Composed By`, `Written-By`, `Music By`; `Conductor`) because
/// Discogs roles are free-form text that may carry a qualifier such as `Written-By [Lyrics]`.
fn composer_and_conductor(credits: &[ExtraArtist]) -> (String, String) {
    let named = |wanted: &[&str]| {
        let mut names: Vec<String> = Vec::new();
        for credit in credits {
            let role = credit.role.to_lowercase();
            let name = strip_disambiguation(&credit.name).to_owned();
            if !name.is_empty() && wanted.iter().any(|w| role.contains(w)) && !names.contains(&name)
            {
                names.push(name);
            }
        }
        names.join(", ")
    };
    (
        named(&["composed by", "written-by", "music by", "composer"]),
        named(&["conductor"]),
    )
}

/// `"5:35"` or `"1:02:03"` as seconds; `None` if blank or malformed.
fn seconds_of(duration: &str) -> Option<f64> {
    let mut total = 0.0;
    let parts: Vec<&str> = duration.trim().split(':').collect();
    if parts.len() > 3 || duration.trim().is_empty() {
        return None;
    }
    for part in parts {
        total = total * 60.0 + part.trim().parse::<u32>().ok().map(f64::from)?;
    }
    Some(total)
}

/// `1987-07-00` becomes `1987-07`; `1987-00-00` becomes `1987`.
fn clean_date(released: &str, year: Option<u32>) -> String {
    let parts: Vec<&str> = released
        .trim()
        .split('-')
        .take_while(|p| {
            !p.is_empty() && p.bytes().any(|b| b != b'0') && p.bytes().all(|b| b.is_ascii_digit())
        })
        .collect();
    if parts.is_empty() {
        year.map(|y| y.to_string()).unwrap_or_default()
    } else {
        parts.join("-")
    }
}

/// Splits Discogs's `"Artist - Title"` search title.
fn split_title(combined: &str) -> (String, String) {
    match combined.split_once(" - ") {
        Some((artist, title)) => (
            strip_disambiguation(artist).to_owned(),
            title.trim().to_owned(),
        ),
        None => (String::new(), combined.trim().to_owned()),
    }
}

/// Results of a release search. Masters and other result types are skipped.
pub fn parse_search(json: &str) -> Result<Vec<Candidate>, String> {
    let response: SearchResponse =
        serde_json::from_str(json).map_err(|e| format!("unexpected reply from Discogs: {e}"))?;
    Ok(response
        .results
        .into_iter()
        .filter(|hit| hit.kind.is_empty() || hit.kind == "release")
        .take(MAX_CANDIDATES)
        .map(|hit| {
            let (artist, title) = split_title(&hit.title);
            Candidate {
                source: Source::Discogs,
                id: hit.id.to_string(),
                title,
                artist,
                date: hit.year.unwrap_or_default(),
                country: hit.country.unwrap_or_default(),
                label: hit.label.into_iter().next().unwrap_or_default(),
                barcode: String::new(),
                // A search hit does not say how many tracks the release has.
                tracks: 0,
                formats: hit.format.join(", "),
                score: None,
            }
        })
        .collect())
}

/// Flattens a tracklist: headings are dropped and index tracks are replaced by their parts.
fn flatten(tracks: Vec<Track>, out: &mut Vec<TrackInfo>) {
    for track in tracks {
        match track.kind.as_str() {
            "heading" => {}
            "index" => flatten(track.sub_tracks, out),
            _ => {
                let (composer, conductor) = composer_and_conductor(&track.extraartists);
                out.push(TrackInfo {
                    position: out.len() as u32 + 1,
                    title: track.title.trim().to_owned(),
                    artist: artist_text(&track.artists),
                    composer,
                    conductor,
                    seconds: seconds_of(&track.duration),
                });
            }
        }
    }
}

/// A release with its track list. Discogs lists tracks as one run (sides and discs are
/// part of the position text), so the result has a single disc numbered in order.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let r: RawRelease =
        serde_json::from_str(json).map_err(|e| format!("unexpected reply from Discogs: {e}"))?;
    let mut tracks = Vec::new();
    flatten(r.tracklist, &mut tracks);
    let format = r
        .formats
        .first()
        .map(|f| {
            std::iter::once(f.name.as_str())
                .chain(f.descriptions.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let label = r.labels.first().map(|l| l.name.clone()).unwrap_or_default();
    let catalog = r
        .labels
        .iter()
        .map(|l| l.catno.trim())
        .find(|c| !c.is_empty() && !c.eq_ignore_ascii_case("none"))
        .unwrap_or_default()
        .to_owned();
    let (composer, conductor) = composer_and_conductor(&r.extraartists);
    Ok(Release {
        source: Source::Discogs,
        id: r.id.to_string(),
        title: r.title.trim().to_owned(),
        artist: artist_text(&r.artists),
        date: clean_date(&r.released, r.year),
        label,
        catalog,
        barcode: String::new(),
        composer,
        conductor,
        discs: vec![Disc {
            position: 1,
            format,
            tracks,
        }],
    })
}

fn with_token(mut request: Request, token: Option<&str>) -> Request {
    if let Some(token) = token.map(str::trim).filter(|t| !t.is_empty()) {
        request
            .headers
            .push(("Authorization".to_owned(), format!("Discogs token={token}")));
    }
    request
}

/// Search by release title and (optionally) artist. `None` if there is no title.
pub fn search_request(artist: &str, album: &str, token: Option<&str>) -> Option<Request> {
    let album = album.trim();
    if album.is_empty() {
        return None;
    }
    let mut url = format!(
        "{API}/database/search?type=release&per_page={MAX_CANDIDATES}&release_title={}",
        encode(album)
    );
    if !artist.trim().is_empty() {
        url.push_str(&format!("&artist={}", encode(artist.trim())));
    }
    Some(with_token(Request::get(url), token))
}

/// Search by free text, as typed by a person. `None` if it is blank.
pub fn text_search_request(text: &str, token: Option<&str>) -> Option<Request> {
    let text = text.trim();
    (!text.is_empty()).then(|| {
        with_token(
            Request::get(format!(
                "{API}/database/search?type=release&per_page={MAX_CANDIDATES}&q={}",
                encode(text)
            )),
            token,
        )
    })
}

/// Finds a release id in text: a Discogs link (`…/release/249504-Never-Gonna…`), the
/// short form `r249504`, or a bare number of at least five digits.
pub fn find_release_id(text: &str) -> Option<String> {
    let text = text.trim();
    for marker in ["/releases/", "/release/"] {
        if let Some((_, rest)) = text.split_once(marker) {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() {
                return Some(digits);
            }
        }
    }
    if let Some(rest) = text.strip_prefix(['r', 'R'])
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return Some(rest.to_owned());
    }
    (text.len() >= MIN_BARE_ID_DIGITS && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.to_owned())
}

/// Whether `id` is a Discogs release id (digits only).
pub fn is_release_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 12 && id.bytes().all(|b| b.is_ascii_digit())
}

/// The request for a release's full track list; `None` if `id` is not a release id.
pub fn release_request(id: &str, token: Option<&str>) -> Option<Request> {
    is_release_id(id).then(|| with_token(Request::get(format!("{API}/releases/{id}")), token))
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// A trimmed search reply (shape from the live API).
    pub const SEARCH: &str = r#"{"pagination":{"page":1,"pages":3,"per_page":3,"items":9},"results":[
      {"country":"France","year":"2010","format":["Vinyl","LP","Album","Reissue"],
       "label":["Columbia","Vinyles De Jazz"],"type":"release","genre":["Jazz"],"id":2772432,
       "master_id":5460,"catno":"CS 8163","title":"Miles Davis - Kind Of Blue",
       "resource_url":"https://api.discogs.com/releases/2772432"},
      {"year":"1959","type":"master","id":5460,"title":"Miles Davis - Kind Of Blue"},
      {"country":"US","year":"1997","format":["CD","Album"],"label":["Columbia"],"type":"release",
       "id":1234567,"title":"Prince (2) - Kind Of Blue"}]}"#;

    /// A trimmed release reply with sides, a heading and an index track.
    pub const RELEASE: &str = r#"{"id":249504,"title":"Never Gonna Give You Up","year":1987,
      "released":"1987-07-00","country":"UK",
      "artists":[{"name":"Rick Astley","anv":"","join":"","role":""}],
      "labels":[{"name":"RCA","catno":"PB 41447"},{"name":"Stock Aitken Waterman","catno":"none"}],
      "formats":[{"name":"Vinyl","qty":"1","descriptions":["7\"","45 RPM","Single"]}],
      "tracklist":[
        {"position":"","type_":"heading","title":"Side One","duration":""},
        {"position":"A","type_":"track","title":"Never Gonna Give You Up","duration":"3:32"},
        {"position":"B1","type_":"index","title":"Medley","duration":"","sub_tracks":[
          {"position":"B1.1","type_":"track","title":"Part One","duration":"1:10"},
          {"position":"B1.2","type_":"track","title":"Part Two","duration":""}]},
        {"position":"B2","type_":"track","title":"Duet","duration":"1:02:03",
         "artists":[{"name":"Rick Astley","join":"&"},{"name":"Prince (2)","join":""}],
         "extraartists":[{"name":"Mike Stock","role":"Written-By [Lyrics]"},
                         {"name":"Matt Aitken","role":"Written-By"},
                         {"name":"Pete Waterman","role":"Producer"}]}],
      "extraartists":[{"name":"Simon Rattle","role":"Conductor"}]}"#;
}

#[cfg(test)]
mod tests {
    use super::fixtures::{RELEASE, SEARCH};
    use super::*;

    #[test]
    fn search_hits_become_candidates_and_masters_are_skipped() {
        let found = parse_search(SEARCH).expect("valid reply");
        assert_eq!(found.len(), 2, "the master is skipped");
        let first = &found[0];
        assert_eq!(
            (first.artist.as_str(), first.title.as_str()),
            ("Miles Davis", "Kind Of Blue")
        );
        assert_eq!(
            (
                first.id.as_str(),
                first.date.as_str(),
                first.country.as_str()
            ),
            ("2772432", "2010", "France")
        );
        assert_eq!(
            (first.label.as_str(), first.formats.as_str()),
            ("Columbia", "Vinyl, LP, Album, Reissue")
        );
        assert_eq!((first.source, first.tracks), (Source::Discogs, 0));
        assert_eq!(
            found[1].artist, "Prince",
            "the disambiguation number is dropped"
        );
    }

    #[test]
    fn a_release_reply_has_a_flat_numbered_track_list() {
        let release = parse_release(RELEASE).expect("valid reply");
        assert_eq!(release.title, "Never Gonna Give You Up");
        assert_eq!(release.artist, "Rick Astley");
        assert_eq!(release.date, "1987-07", "unknown day is dropped");
        assert_eq!(
            (release.label.as_str(), release.catalog.as_str()),
            ("RCA", "PB 41447")
        );
        assert_eq!(release.discs.len(), 1);
        assert_eq!(release.discs[0].format, "Vinyl, 7\", 45 RPM, Single");

        let tracks: Vec<(u32, &str, Option<f64>)> = release.discs[0]
            .tracks
            .iter()
            .map(|t| (t.position, t.title.as_str(), t.seconds))
            .collect();
        assert_eq!(
            tracks,
            [
                (1, "Never Gonna Give You Up", Some(212.0)),
                (2, "Part One", Some(70.0)),
                (3, "Part Two", None),
                (4, "Duet", Some(3723.0)),
            ],
            "the heading is skipped and the index track is replaced by its parts"
        );
        assert_eq!(release.discs[0].tracks[3].artist, "Rick Astley & Prince");
    }

    #[test]
    fn role_credits_become_composer_and_conductor() {
        let release = parse_release(RELEASE).expect("valid reply");
        assert_eq!(release.conductor, "Simon Rattle", "release-level credit");
        assert_eq!(release.composer, "");
        let duet = &release.discs[0].tracks[3];
        assert_eq!(
            duet.composer, "Mike Stock, Matt Aitken",
            "qualified roles count; producers do not"
        );
        assert_eq!(duet.conductor, "");
        assert_eq!(release.discs[0].tracks[0].composer, "");
    }

    #[test]
    fn odd_replies_are_errors_or_empty_not_panics() {
        assert!(parse_search("nope").is_err());
        assert_eq!(parse_search("{}").expect("empty"), Vec::new());
        assert!(parse_release("{}").is_err(), "a release needs an id");
        let sparse = parse_release(r#"{"id":1}"#).expect("only an id");
        assert!(sparse.discs[0].tracks.is_empty());
    }

    #[test]
    fn dates_durations_and_names_are_cleaned() {
        assert_eq!(clean_date("1987-07-15", None), "1987-07-15");
        assert_eq!(clean_date("1987-00-00", Some(1990)), "1987");
        assert_eq!(clean_date("", Some(1990)), "1990");
        assert_eq!(clean_date("0000-00-00", None), "");
        assert_eq!(seconds_of("3:32"), Some(212.0));
        assert_eq!(seconds_of("1:02:03"), Some(3723.0));
        assert_eq!(seconds_of(""), None);
        assert_eq!(seconds_of("3:xx"), None);
        assert_eq!(seconds_of("1:2:3:4"), None);
        assert_eq!(strip_disambiguation("Prince (2)"), "Prince");
        assert_eq!(strip_disambiguation("Yes (live)"), "Yes (live)");
        assert_eq!(strip_disambiguation("Blur"), "Blur");
    }

    #[test]
    fn ids_are_recognised_in_links_short_forms_and_long_numbers_only() {
        assert_eq!(
            find_release_id("https://www.discogs.com/release/249504-Rick-Astley-Never").as_deref(),
            Some("249504")
        );
        assert_eq!(
            find_release_id("https://api.discogs.com/releases/249504").as_deref(),
            Some("249504")
        );
        assert_eq!(find_release_id("r249504").as_deref(), Some("249504"));
        assert_eq!(find_release_id("1111111111").as_deref(), Some("1111111111"));
        assert_eq!(
            find_release_id("1984"),
            None,
            "an album called 1984, not an id"
        );
        assert_eq!(find_release_id("Blood on the Tracks"), None);
        assert_eq!(find_release_id("  "), None);
        assert_eq!(find_release_id("r"), None);
    }

    #[test]
    fn requests_are_encoded_validated_and_carry_the_token() {
        let search = search_request("Miles Davis", "Kind of Blue", None).expect("request");
        assert_eq!(
            search.url,
            "https://api.discogs.com/database/search?type=release&per_page=10&release_title=Kind%20of%20Blue&artist=Miles%20Davis"
        );
        assert!(search.headers.is_empty());
        assert!(search_request("x", " ", None).is_none());
        assert!(
            !search_request("", "Album", None)
                .expect("request")
                .url
                .contains("artist=")
        );

        let free = text_search_request("bob dylan blood", Some(" abc123 ")).expect("request");
        assert!(free.url.ends_with("&q=bob%20dylan%20blood"));
        assert_eq!(
            free.headers,
            [(
                "Authorization".to_owned(),
                "Discogs token=abc123".to_owned()
            )]
        );
        assert!(text_search_request("  ", None).is_none());
        assert!(
            text_search_request("x", Some("  "))
                .expect("request")
                .headers
                .is_empty()
        );

        assert_eq!(
            release_request("249504", None).expect("request").url,
            "https://api.discogs.com/releases/249504"
        );
        assert!(release_request("../etc", None).is_none());
        assert!(release_request("", None).is_none());
    }
}
