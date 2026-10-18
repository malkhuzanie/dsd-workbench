//! The tag fields people expect on an album track, mapped to ID3v2 frames.
//!
//! This is the format-level knowledge of "which frame holds the conductor?" and "where is
//! the date in an ID3v2.3 tag?", so applications do not each have to rediscover it.

use std::fmt;

use id3::{Tag, TagLike};

use crate::edit::{FrameId, TagEdit};

/// A well-known field of a track's tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Field {
    /// Track title (`TIT2`).
    Title,
    /// Lead artist (`TPE1`).
    Artist,
    /// Album artist (`TPE2`).
    AlbumArtist,
    /// Album title (`TALB`).
    Album,
    /// Composer (`TCOM`).
    Composer,
    /// Conductor (`TPE3`).
    Conductor,
    /// Genre (`TCON`).
    Genre,
    /// Recording date (`TDRC`; `TYER` in ID3v2.3 tags).
    Date,
    /// Disc number, `"n"` or `"n/total"` (`TPOS`).
    Disc,
    /// Content group, such as a work or movement (`TIT1`; `GRP1` in some tags).
    Grouping,
    /// Label or publisher (`TPUB`).
    Publisher,
    /// Copyright message (`TCOP`).
    Copyright,
    /// International Standard Recording Code (`TSRC`).
    Isrc,
    /// General comment (`COMM` with an empty description).
    Comment,
}

/// The kind of frame a [`Field`] is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Target {
    /// A text frame such as `TIT2`.
    Text(FrameId),
    /// The general comment frame.
    Comment,
}

impl Field {
    /// Every field, in a stable order.
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

    /// Where this field is written.
    pub fn target(self) -> Target {
        match self {
            Self::Title => Target::Text(FrameId::TITLE),
            Self::Artist => Target::Text(FrameId::ARTIST),
            Self::AlbumArtist => Target::Text(FrameId::ALBUM_ARTIST),
            Self::Album => Target::Text(FrameId::ALBUM),
            Self::Composer => Target::Text(FrameId::COMPOSER),
            Self::Conductor => Target::Text(FrameId::CONDUCTOR),
            Self::Genre => Target::Text(FrameId::GENRE),
            Self::Date => Target::Text(FrameId::RECORDING_DATE),
            Self::Disc => Target::Text(FrameId::DISC),
            Self::Grouping => Target::Text(FrameId::GROUPING),
            Self::Publisher => Target::Text(FrameId::PUBLISHER),
            Self::Copyright => Target::Text(FrameId::COPYRIGHT),
            Self::Isrc => Target::Text(FrameId::ISRC),
            Self::Comment => Target::Comment,
        }
    }

    /// Older frames that carry the same information in tags written by other software,
    /// tried in order when the main frame is absent.
    fn fallbacks(self) -> &'static [&'static str] {
        match self {
            Self::Date => &["TYER"],
            Self::Grouping => &["GRP1"],
            _ => &[],
        }
    }

    /// The field's value in `tag`, or `None` if it is absent or blank.
    ///
    /// Several values stored in one frame are joined with `"; "`.
    pub fn read(self, tag: &Tag) -> Option<String> {
        let value = match self.target() {
            Target::Text(id) => std::iter::once(id.as_str())
                .chain(self.fallbacks().iter().copied())
                .find_map(|frame| text(tag, frame)),
            Target::Comment => tag
                .comments()
                .find(|c| c.description.is_empty())
                .map(|c| c.text.clone()),
        };
        value.filter(|v| !v.trim().is_empty())
    }
}

/// The text of frame `id`, with multiple values joined by `"; "`. `None` if absent.
pub fn text(tag: &Tag, id: &str) -> Option<String> {
    tag.get(id)
        .and_then(|frame| frame.content().text())
        .map(|t| t.trim_end_matches('\0').replace('\0', "; "))
}

/// Every text frame in the tag (except user-defined `TXXX` frames), in tag order.
pub fn text_frames(tag: &Tag) -> Vec<(FrameId, String)> {
    tag.frames()
        .filter_map(|frame| {
            let id = FrameId::new(frame.id()).ok()?;
            let value = text(tag, frame.id())?;
            Some((id, value))
        })
        .collect()
}

/// The user-defined text frames (`TXXX`) as `(description, value)` pairs.
pub fn custom_fields(tag: &Tag) -> Vec<(String, String)> {
    tag.extended_texts()
        .map(|t| (t.description.clone(), t.value.clone()))
        .collect()
}

/// A track's place on its disc: number and, if known, how many tracks there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Track {
    /// Position, from 1 (0 is used for a hidden track in front of track 1).
    pub number: u32,
    /// Number of tracks, if known.
    pub total: Option<u32>,
}

impl Track {
    /// Parses `"3"`, `"3/12"` or `" 3 / 12 "`. A total of zero counts as unknown.
    pub fn parse(text: &str) -> Option<Self> {
        let (number, total) = match text.split_once('/') {
            Some((number, total)) => (number, Some(total)),
            None => (text, None),
        };
        Some(Self {
            number: number.trim().parse().ok()?,
            total: total
                .and_then(|t| t.trim().parse::<u32>().ok())
                .filter(|t| *t > 0),
        })
    }
}

impl fmt::Display for Track {
    /// `n` or `n/total`, as stored in a `TRCK` frame.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.total {
            Some(total) => write!(f, "{}/{total}", self.number),
            None => write!(f, "{}", self.number),
        }
    }
}

/// The track position from the `TRCK` frame, if present and well formed.
pub fn read_track(tag: &Tag) -> Option<Track> {
    text(tag, FrameId::TRACK.as_str()).and_then(|t| Track::parse(&t))
}

impl TagEdit {
    /// Sets a [`Field`]. A blank value removes it.
    #[must_use]
    pub fn set_field(self, field: Field, value: impl Into<String>) -> Self {
        match field.target() {
            Target::Text(id) => self.set(id, value),
            Target::Comment => self.set_comment(value),
        }
    }

    /// Sets the track number (and total) in the `TRCK` frame.
    #[must_use]
    pub fn set_track(self, track: Track) -> Self {
        self.set(FrameId::TRACK, track.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use id3::Version;
    use id3::frame::Comment;

    use super::*;
    use crate::{WriteOptions, read_tag, update_tag, write_tag};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    /// A minimal DSF file with no tag.
    fn blank() -> std::result::Result<Cursor<Vec<u8>>, Box<dyn std::error::Error>> {
        let audio = 4096 * 2;
        let header = crate::DsfHeader {
            total_file_size: crate::AUDIO_OFFSET + audio,
            metadata_offset: 0,
            channel_type: 2,
            channel_count: 2,
            sample_rate: 2_822_400,
            bits_per_sample: 1,
            sample_count: 32_768,
            block_size: 4096,
            data_chunk_size: 12 + audio,
        };
        let mut bytes = Vec::new();
        header.write(&mut bytes)?;
        bytes.extend(vec![0xA5; audio as usize]);
        Ok(Cursor::new(bytes))
    }

    #[test]
    fn every_field_has_its_own_frame() {
        let mut seen: Vec<Target> = Vec::new();
        for field in Field::ALL {
            let target = field.target();
            assert!(!seen.contains(&target), "{field:?} shares a frame");
            seen.push(target);
            if let Target::Text(id) = target {
                assert_eq!(FrameId::new(id.as_str()).ok(), Some(id), "{field:?}");
            }
        }
        assert_eq!(seen.len(), Field::ALL.len());
    }

    #[test]
    fn every_field_round_trips_through_a_file() -> TestResult {
        let mut file = blank()?;
        let mut edit = TagEdit::new();
        for (i, field) in Field::ALL.into_iter().enumerate() {
            edit = edit.set_field(field, format!("value {i}"));
        }
        update_tag(&mut file, &edit, &WriteOptions::default())?;
        let tag = read_tag(&mut file)?.ok_or("tag missing")?;
        for (i, field) in Field::ALL.into_iter().enumerate() {
            assert_eq!(field.read(&tag), Some(format!("value {i}")), "{field:?}");
        }
        Ok(())
    }

    #[test]
    fn blank_and_absent_fields_read_as_none_and_a_blank_set_removes() -> TestResult {
        let mut file = blank()?;
        let edit = TagEdit::new()
            .set_field(Field::Genre, "Jazz")
            .set_field(Field::Title, "T");
        update_tag(&mut file, &edit, &WriteOptions::default())?;
        let tag = read_tag(&mut file)?.ok_or("tag missing")?;
        assert_eq!(Field::Composer.read(&tag), None);

        update_tag(
            &mut file,
            &TagEdit::new().set_field(Field::Genre, "  "),
            &WriteOptions::default(),
        )?;
        let tag = read_tag(&mut file)?.ok_or("tag missing")?;
        assert_eq!(Field::Genre.read(&tag), None);
        assert_eq!(
            Field::Title.read(&tag).as_deref(),
            Some("T"),
            "other fields stay"
        );
        Ok(())
    }

    #[test]
    fn id3v23_dates_are_found_under_tyer() -> TestResult {
        let mut tag = Tag::new();
        tag.add_frame(id3::Frame::text("TYER", "1959"));
        tag.add_frame(id3::Frame::text("GRP1", "Symphony No. 9"));
        assert_eq!(Field::Date.read(&tag).as_deref(), Some("1959"));
        assert_eq!(
            Field::Grouping.read(&tag).as_deref(),
            Some("Symphony No. 9")
        );

        tag.add_frame(id3::Frame::text("TDRC", "1960-01-02"));
        tag.add_frame(id3::Frame::text("TIT1", "Real group"));
        assert_eq!(
            Field::Date.read(&tag).as_deref(),
            Some("1960-01-02"),
            "the modern frame wins"
        );
        assert_eq!(Field::Grouping.read(&tag).as_deref(), Some("Real group"));
        Ok(())
    }

    #[test]
    fn a_date_written_into_an_id3v23_file_reads_back() -> TestResult {
        let mut file = blank()?;
        write_tag(&mut file, &Tag::new(), Version::Id3v23)?;
        let edit = TagEdit::new()
            .set_field(Field::Date, "1959")
            .set_field(Field::Title, "T");
        update_tag(&mut file, &edit, &WriteOptions::default())?;
        let tag = read_tag(&mut file)?.ok_or("tag missing")?;
        assert_eq!(tag.version(), Version::Id3v23);
        assert_eq!(
            Field::Date.read(&tag).as_deref(),
            Some("1959"),
            "the date survives v2.3"
        );
        Ok(())
    }

    #[test]
    fn multiple_values_are_joined_and_only_the_plain_comment_counts() {
        let mut tag = Tag::new();
        tag.set_text("TPE1", "Miles Davis\0John Coltrane");
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: "other".into(),
            text: "not this one".into(),
        });
        assert_eq!(
            Field::Artist.read(&tag).as_deref(),
            Some("Miles Davis; John Coltrane")
        );
        assert_eq!(
            Field::Comment.read(&tag),
            None,
            "described comments are not the general comment"
        );
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: String::new(),
            text: "plain".into(),
        });
        assert_eq!(Field::Comment.read(&tag).as_deref(), Some("plain"));
    }

    #[test]
    fn text_frames_lists_every_text_frame_but_not_user_text() {
        let mut tag = Tag::new();
        tag.set_text("TIT2", "Title");
        tag.set_text("TKEY", "Am");
        tag.add_frame(id3::frame::ExtendedText {
            description: "MOOD".into(),
            value: "calm".into(),
        });
        let frames = text_frames(&tag);
        let ids: Vec<&str> = frames.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"TIT2") && ids.contains(&"TKEY"), "{ids:?}");
        assert!(!ids.contains(&"TXXX"), "{ids:?}");
        assert_eq!(
            custom_fields(&tag),
            [("MOOD".to_owned(), "calm".to_owned())]
        );
    }

    #[test]
    fn track_positions_parse_format_and_round_trip() -> TestResult {
        assert_eq!(
            Track::parse("3"),
            Some(Track {
                number: 3,
                total: None
            })
        );
        assert_eq!(
            Track::parse(" 3 / 12 "),
            Some(Track {
                number: 3,
                total: Some(12)
            })
        );
        assert_eq!(
            Track::parse("3/0"),
            Some(Track {
                number: 3,
                total: None
            })
        );
        assert_eq!(
            Track::parse("0/9"),
            Some(Track {
                number: 0,
                total: Some(9)
            })
        );
        assert_eq!(Track::parse("three"), None);
        assert_eq!(Track::parse(""), None);
        assert_eq!(Track::parse("-1"), None);
        assert_eq!(
            Track {
                number: 3,
                total: Some(12)
            }
            .to_string(),
            "3/12"
        );
        assert_eq!(
            Track {
                number: 3,
                total: None
            }
            .to_string(),
            "3"
        );

        let mut file = blank()?;
        let edit = TagEdit::new().set_track(Track {
            number: 7,
            total: Some(10),
        });
        update_tag(&mut file, &edit, &WriteOptions::default())?;
        let tag = read_tag(&mut file)?.ok_or("tag missing")?;
        assert_eq!(
            read_track(&tag),
            Some(Track {
                number: 7,
                total: Some(10)
            })
        );
        assert_eq!(text(&tag, "TRCK").as_deref(), Some("7/10"));
        Ok(())
    }
}
