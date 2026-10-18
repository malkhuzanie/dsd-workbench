//! The tags a user can edit: a fixed set of standard fields plus any number of custom ones.
//!
//! Values live in a [`Fields`] map. An album has one map of defaults and every track has
//! another holding only what it overrides: an absent key inherits the album's value, a
//! key holding an empty string deliberately clears it for that track.

use std::collections::BTreeMap;

/// Fields with a well-known meaning. Where each one lives in the file is the `dsf`
/// crate's business ([`Standard::field`]); this type adds how the app names and groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Standard {
    Title,
    Artist,
    AlbumArtist,
    Album,
    Composer,
    Conductor,
    Genre,
    Date,
    Disc,
    Grouping,
    Publisher,
    Copyright,
    Isrc,
    Comment,
}

impl Standard {
    pub const ALL: [Self; 14] = [
        Self::Title,
        Self::Artist,
        Self::AlbumArtist,
        Self::Album,
        Self::Composer,
        Self::Conductor,
        Self::Genre,
        Self::Date,
        Self::Disc,
        Self::Grouping,
        Self::Publisher,
        Self::Copyright,
        Self::Isrc,
        Self::Comment,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::AlbumArtist => "Album artist",
            Self::Album => "Album",
            Self::Composer => "Composer",
            Self::Conductor => "Conductor",
            Self::Genre => "Genre",
            Self::Date => "Date / year",
            Self::Disc => "Disc",
            Self::Grouping => "Grouping",
            Self::Publisher => "Label / publisher",
            Self::Copyright => "Copyright",
            Self::Isrc => "ISRC",
            Self::Comment => "Comment",
        }
    }

    /// A stable machine name, used in saved project files. Never change an existing one.
    pub fn id(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::AlbumArtist => "albumartist",
            Self::Album => "album",
            Self::Composer => "composer",
            Self::Conductor => "conductor",
            Self::Genre => "genre",
            Self::Date => "date",
            Self::Disc => "disc",
            Self::Grouping => "grouping",
            Self::Publisher => "publisher",
            Self::Copyright => "copyright",
            Self::Isrc => "isrc",
            Self::Comment => "comment",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|standard| standard.id() == id)
    }

    /// Fields that only make sense per track: an album-wide title or ISRC would be wrong.
    pub fn track_only(self) -> bool {
        matches!(self, Self::Title | Self::Isrc)
    }

    /// The corresponding field of the tag library.
    pub fn field(self) -> dsf::Field {
        match self {
            Self::Title => dsf::Field::Title,
            Self::Artist => dsf::Field::Artist,
            Self::AlbumArtist => dsf::Field::AlbumArtist,
            Self::Album => dsf::Field::Album,
            Self::Composer => dsf::Field::Composer,
            Self::Conductor => dsf::Field::Conductor,
            Self::Genre => dsf::Field::Genre,
            Self::Date => dsf::Field::Date,
            Self::Disc => dsf::Field::Disc,
            Self::Grouping => dsf::Field::Grouping,
            Self::Publisher => dsf::Field::Publisher,
            Self::Copyright => dsf::Field::Copyright,
            Self::Isrc => dsf::Field::Isrc,
            Self::Comment => dsf::Field::Comment,
        }
    }
}

/// Identifies one editable tag.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldKey {
    Standard(Standard),
    /// A user-defined `TXXX` frame, identified by its description (for example `MOOD`).
    Custom(String),
}

impl FieldKey {
    pub const TITLE: Self = Self::Standard(Standard::Title);
    pub const ARTIST: Self = Self::Standard(Standard::Artist);
    pub const ALBUM_ARTIST: Self = Self::Standard(Standard::AlbumArtist);
    pub const ALBUM: Self = Self::Standard(Standard::Album);
    pub const COMPOSER: Self = Self::Standard(Standard::Composer);
    pub const GENRE: Self = Self::Standard(Standard::Genre);
    pub const DATE: Self = Self::Standard(Standard::Date);

    /// A custom key from user input; `None` if the name is blank or unusable.
    pub fn custom(name: &str) -> Option<Self> {
        let name = name.trim();
        let usable = !name.is_empty() && !name.chars().any(char::is_control);
        usable.then(|| Self::Custom(name.to_owned()))
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Standard(standard) => standard.label(),
            Self::Custom(name) => name,
        }
    }

    /// The key as saved in project files: `title`, `genre`, … or `tag:NAME`.
    pub fn to_id(&self) -> String {
        match self {
            Self::Standard(standard) => standard.id().to_owned(),
            Self::Custom(name) => format!("{CUSTOM_PREFIX}{name}"),
        }
    }

    /// Reads a key written by [`FieldKey::to_id`]; `None` for anything unknown.
    pub fn from_id(id: &str) -> Option<Self> {
        match id.strip_prefix(CUSTOM_PREFIX) {
            Some(name) => Self::custom(name),
            None => Standard::from_id(id).map(Self::Standard),
        }
    }
}

const CUSTOM_PREFIX: &str = "tag:";

/// Values by key. Keys are kept in a stable order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields(BTreeMap<FieldKey, String>);

impl Fields {
    pub fn get(&self, key: &FieldKey) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// The value, or an empty string when absent.
    pub fn value(&self, key: &FieldKey) -> &str {
        self.get(key).unwrap_or_default()
    }

    pub fn set(&mut self, key: FieldKey, value: impl Into<String>) {
        self.0.insert(key, value.into());
    }

    pub fn remove(&mut self, key: &FieldKey) -> Option<String> {
        self.0.remove(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &FieldKey> {
        self.0.keys()
    }

    /// Sets a value, or removes the key when the value is blank.
    pub fn set_or_clear(&mut self, key: FieldKey, value: &str) {
        if value.trim().is_empty() {
            self.0.remove(&key);
        } else {
            self.0.insert(key, value.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_and_the_library_agree_on_which_fields_exist() {
        let mut mapped: Vec<dsf::Field> = Standard::ALL.into_iter().map(Standard::field).collect();
        mapped.sort();
        mapped.dedup();
        let mut library = dsf::Field::ALL.to_vec();
        library.sort();
        assert_eq!(
            mapped, library,
            "each library field is offered exactly once"
        );
    }

    #[test]
    fn custom_names_are_trimmed_and_validated() {
        assert_eq!(
            FieldKey::custom("  MOOD "),
            Some(FieldKey::Custom("MOOD".into()))
        );
        assert_eq!(FieldKey::custom("   "), None);
        assert_eq!(FieldKey::custom("a\nb"), None);
    }

    #[test]
    fn blank_values_remove_a_key() {
        let mut fields = Fields::default();
        fields.set_or_clear(FieldKey::GENRE, "Jazz");
        assert_eq!(fields.value(&FieldKey::GENRE), "Jazz");
        fields.set_or_clear(FieldKey::GENRE, "  ");
        assert_eq!(fields.get(&FieldKey::GENRE), None);
    }

    #[test]
    fn saved_ids_round_trip_and_unknown_ids_are_rejected() {
        for standard in Standard::ALL {
            let key = FieldKey::Standard(standard);
            assert_eq!(FieldKey::from_id(&key.to_id()), Some(key));
        }
        let custom = FieldKey::Custom("RECORDING LOCATION".into());
        assert_eq!(FieldKey::from_id(&custom.to_id()), Some(custom));
        assert_eq!(FieldKey::from_id("nonsense"), None);
        assert_eq!(FieldKey::from_id("tag:"), None);
    }

    #[test]
    fn labels_use_the_custom_name() {
        assert_eq!(FieldKey::Custom("MOOD".into()).label(), "MOOD");
        assert_eq!(FieldKey::ALBUM_ARTIST.label(), "Album artist");
    }
}
