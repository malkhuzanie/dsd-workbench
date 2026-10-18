//! The source-neutral description of a release, whichever service it came from.

use std::fmt;

/// A service releases can be looked up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Source {
    /// <https://musicbrainz.org>, the open music encyclopedia.
    #[default]
    MusicBrainz,
    /// <https://www.discogs.com>, a marketplace-backed release database.
    Discogs,
}

impl Source {
    /// Every supported source.
    pub const ALL: [Self; 2] = [Self::MusicBrainz, Self::Discogs];

    /// The service's name for display.
    pub fn name(self) -> &'static str {
        match self {
            Self::MusicBrainz => "MusicBrainz",
            Self::Discogs => "Discogs",
        }
    }

    /// A stable lower-case key, for settings files and command lines.
    pub fn key(self) -> &'static str {
        match self {
            Self::MusicBrainz => "musicbrainz",
            Self::Discogs => "discogs",
        }
    }

    /// The source with this key (case-insensitive).
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|s| s.key().eq_ignore_ascii_case(key.trim()))
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One search hit: enough to choose between releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The service the hit came from.
    pub source: Source,
    /// The service's id for the release; pass it to [`crate::Target::Id`].
    pub id: String,
    /// The release title.
    pub title: String,
    /// The release artist.
    pub artist: String,
    /// Release date as the service gives it (a year, or `YYYY-MM-DD`); may be empty.
    pub date: String,
    /// Country of release; may be empty.
    pub country: String,
    /// The first label; may be empty.
    pub label: String,
    /// The barcode, where the service gives one; may be empty.
    pub barcode: String,
    /// Tracks on all discs; zero if the service did not say.
    pub tracks: u32,
    /// For example `CD`, `SACD + CD`, `Vinyl, LP`.
    pub formats: String,
    /// How well the service thinks the hit matches the search (MusicBrainz: 0–100);
    /// `None` when the service gives no score.
    pub score: Option<u32>,
}

impl Candidate {
    /// A one-line description for a list.
    pub fn describe(&self) -> String {
        let mut parts = vec![format!("{} — {}", self.artist, self.title)];
        for part in [&self.date, &self.country, &self.label, &self.formats] {
            if !part.is_empty() {
                parts.push(part.clone());
            }
        }
        if self.tracks > 0 {
            parts.push(format!("{} tracks", self.tracks));
        }
        parts.join(" · ")
    }
}

/// One track of a release.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    /// Position counted from 1 across the disc, whatever the service calls it (`A2`, `1-3`).
    pub position: u32,
    /// The track title.
    pub title: String,
    /// The track's artist; empty if the service gives only the release artist.
    pub artist: String,
    /// Composer credited on this track; empty if unknown.
    pub composer: String,
    /// Conductor credited on this track; empty if unknown.
    pub conductor: String,
    /// Length in seconds, if the service knows it.
    pub seconds: Option<f64>,
}

/// One disc (or, for services that list a single run of tracks, the whole release).
#[derive(Debug, Clone, PartialEq)]
pub struct Disc {
    /// Disc number, from 1.
    pub position: u32,
    /// The medium, for example `CD` or `Vinyl, LP`; may be empty.
    pub format: String,
    /// The disc's tracks in order.
    pub tracks: Vec<TrackInfo>,
}

/// A release with its full track list.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// The service the release came from.
    pub source: Source,
    /// The service's id for the release.
    pub id: String,
    /// The release title.
    pub title: String,
    /// The release artist, with joint credits written out (`A & B`).
    pub artist: String,
    /// Release date as the service gives it; may be empty.
    pub date: String,
    /// The first label; may be empty.
    pub label: String,
    /// The first catalogue number; may be empty.
    pub catalog: String,
    /// The barcode, where the service gives one; may be empty.
    pub barcode: String,
    /// Composer credited on the release as a whole; empty if unknown.
    pub composer: String,
    /// Conductor credited on the release as a whole; empty if unknown.
    pub conductor: String,
    /// The discs, in order. Discogs lists one run of tracks, so it has a single disc.
    pub discs: Vec<Disc>,
}

/// A request to make, described without performing it, so any HTTP client can run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The full `https://` URL to GET.
    pub url: String,
    /// Extra headers (for example `Authorization`). A `User-Agent` is the client's job.
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub(crate) fn get(url: String) -> Self {
        Self {
            url,
            headers: Vec::new(),
        }
    }
}

/// Percent-encodes a query-string value.
pub(crate) fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_have_stable_keys() {
        for source in Source::ALL {
            assert_eq!(Source::from_key(source.key()), Some(source));
            assert_eq!(Source::from_key(&source.key().to_uppercase()), Some(source));
        }
        assert_eq!(Source::from_key(" discogs "), Some(Source::Discogs));
        assert_eq!(Source::from_key("spotify"), None);
        assert_eq!(Source::Discogs.to_string(), "Discogs");
    }

    #[test]
    fn candidates_describe_themselves_without_empty_parts() {
        let candidate = Candidate {
            source: Source::MusicBrainz,
            id: "x".into(),
            title: "Kind of Blue".into(),
            artist: "Miles Davis".into(),
            date: "1959".into(),
            country: String::new(),
            label: "Columbia".into(),
            barcode: String::new(),
            tracks: 0,
            formats: String::new(),
            score: None,
        };
        assert_eq!(
            candidate.describe(),
            "Miles Davis — Kind of Blue · 1959 · Columbia"
        );
    }

    #[test]
    fn encoding_is_percent_encoding() {
        assert_eq!(encode("a b&c/é"), "a%20b%26c%2F%C3%A9");
        assert_eq!(encode("safe-_.~09Az"), "safe-_.~09Az");
        assert_eq!(encode(""), "");
    }
}
