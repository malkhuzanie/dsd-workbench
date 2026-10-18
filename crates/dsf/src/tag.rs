use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use id3::{ErrorKind, Tag, Version};

use crate::edit::{TagEdit, WriteOptions};
use crate::header::DsfHeader;
use crate::storage::Storage;
use crate::{Error, Result};

/// Parses the header and, if present, the ID3v2 tag. Validates that the
/// declared audio fits in the file and the metadata pointer is sane.
fn load<R: Read + Seek>(r: &mut R) -> Result<(DsfHeader, Option<Tag>)> {
    r.seek(SeekFrom::Start(0))?;
    let header = DsfHeader::read(&mut *r)?;
    let file_len = r.seek(SeekFrom::End(0))?;

    if header.audio_end() > file_len {
        return Err(Error::Malformed(
            "file is shorter than its declared audio data",
        ));
    }
    let offset = header.metadata_offset;
    if offset == 0 || offset == file_len {
        return Ok((header, None));
    }
    if offset < header.audio_end() || offset > file_len {
        return Err(Error::Malformed("metadata pointer outside the file tail"));
    }

    r.seek(SeekFrom::Start(offset))?;
    match Tag::read_from2(&mut *r) {
        Ok(tag) => Ok((header, Some(tag))),
        Err(e) if matches!(e.kind, ErrorKind::NoTag) => Ok((header, None)),
        // A corrupt tag is an error, never silently replaced.
        Err(e) => Err(e.into()),
    }
}

/// Writes `tag_bytes` (or removes the tag when `None`) and fixes the header.
fn commit<S: Storage>(s: &mut S, header: &DsfHeader, tag_bytes: Option<&[u8]>) -> Result<()> {
    let audio_end = header.audio_end();
    let (offset, new_len) = match tag_bytes {
        Some(bytes) => {
            let len = u64::try_from(bytes.len()).map_err(|_| Error::Malformed("tag too large"))?;
            let end = audio_end
                .checked_add(len)
                .ok_or(Error::Malformed("tag too large"))?;
            s.seek(SeekFrom::Start(audio_end))?;
            s.write_all(bytes)?;
            (audio_end, end)
        }
        None => (0, audio_end),
    };
    s.set_len(new_len)?;

    // total_file_size (bytes 12..20) and metadata pointer (20..28) in one write.
    let mut fields = [0u8; 16];
    fields[..8].copy_from_slice(&new_len.to_le_bytes());
    fields[8..].copy_from_slice(&offset.to_le_bytes());
    s.seek(SeekFrom::Start(12))?;
    s.write_all(&fields)?;
    s.sync()?;
    Ok(())
}

fn encode(tag: &Tag, version: Version) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    tag.write_to(&mut buf, version)?;
    Ok(buf)
}

/// Reads the ID3v2 tag, or `None` if the file has none.
pub fn read_tag<R: Read + Seek>(r: &mut R) -> Result<Option<Tag>> {
    Ok(load(r)?.1)
}

/// Replaces the whole tag.
pub fn write_tag<S: Storage>(s: &mut S, tag: &Tag, version: Version) -> Result<()> {
    let (header, _) = load(s)?;
    commit(s, &header, Some(&encode(tag, version)?))
}

/// Applies `edit` to the existing tag (creating one if absent), preserving all other frames.
pub fn update_tag<S: Storage>(s: &mut S, edit: &TagEdit, opts: &WriteOptions) -> Result<()> {
    let (header, existing) = load(s)?;
    let mut tag = existing.unwrap_or_else(Tag::new);
    edit.apply(&mut tag);
    let version = opts.version.unwrap_or_else(|| tag.version());
    commit(s, &header, Some(&encode(&tag, version)?))
}

/// Removes the tag and zeroes the metadata pointer.
pub fn remove_tag<S: Storage>(s: &mut S) -> Result<()> {
    let (header, _) = load(s)?;
    commit(s, &header, None)
}

/// Convenience: read the tag from a path.
pub fn read_path(path: impl AsRef<Path>) -> Result<Option<Tag>> {
    read_tag(&mut BufReader::new(File::open(path)?))
}

/// Convenience: apply `edit` to the file at `path`.
pub fn update_path(path: impl AsRef<Path>, edit: &TagEdit, opts: &WriteOptions) -> Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    update_tag(&mut file, edit, opts)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use id3::TagLike;
    use id3::frame::Comment;

    use super::*;
    use crate::edit::FrameId;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    const AUDIO: usize = 1024;
    const AUDIO_END: usize = 92 + AUDIO;

    /// Minimal valid DSF with no metadata and an 0xA5-filled payload.
    fn synth() -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
        let header = DsfHeader {
            total_file_size: AUDIO_END as u64,
            metadata_offset: 0,
            channel_type: 2,
            channel_count: 2,
            sample_rate: 2_822_400,
            bits_per_sample: 1,
            sample_count: 4096,
            block_size: 4096,
            data_chunk_size: 12 + AUDIO as u64,
        };
        let mut v = Vec::new();
        header.write(&mut v)?;
        v.extend(std::iter::repeat_n(0xA5u8, AUDIO));
        Ok(v)
    }

    #[test]
    fn creates_tag_and_keeps_header_consistent() -> TestResult {
        let mut f = Cursor::new(synth()?);
        let edit = TagEdit::new()
            .set(FrameId::TITLE, "Allemande")
            .set(FrameId::COMPOSER, "J.S. Bach");
        update_tag(&mut f, &edit, &WriteOptions::default())?;

        let tag = read_tag(&mut f)?.ok_or("tag missing")?;
        assert_eq!(tag.title(), Some("Allemande"));

        let bytes = f.into_inner();
        let header = DsfHeader::read(&bytes[..])?;
        assert_eq!(header.metadata_offset, AUDIO_END as u64);
        assert_eq!(header.total_file_size, bytes.len() as u64);
        assert!(
            bytes[92..AUDIO_END].iter().all(|b| *b == 0xA5),
            "audio must be untouched"
        );
        Ok(())
    }

    #[test]
    fn shrinking_a_tag_truncates_the_file() -> TestResult {
        let mut f = Cursor::new(synth()?);
        let long = "x".repeat(2000);
        update_tag(
            &mut f,
            &TagEdit::new().set(FrameId::TITLE, long),
            &WriteOptions::default(),
        )?;
        let big = f.get_ref().len();

        update_tag(
            &mut f,
            &TagEdit::new().set(FrameId::TITLE, "short"),
            &WriteOptions::default(),
        )?;
        let bytes = f.into_inner();
        assert!(bytes.len() < big);
        let header = DsfHeader::read(&bytes[..])?;
        assert_eq!(header.total_file_size, bytes.len() as u64);
        Ok(())
    }

    #[test]
    fn unrelated_frames_survive_an_edit() -> TestResult {
        let mut f = Cursor::new(synth()?);
        let mut tag = Tag::new();
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: String::new(),
            text: "keep me".into(),
        });
        write_tag(&mut f, &tag, Version::Id3v24)?;

        update_tag(
            &mut f,
            &TagEdit::new().set(FrameId::TITLE, "T"),
            &WriteOptions::default(),
        )?;
        let tag = read_tag(&mut f)?.ok_or("tag missing")?;
        assert_eq!(tag.comments().count(), 1);
        assert_eq!(tag.title(), Some("T"));
        Ok(())
    }

    #[test]
    fn user_text_comment_and_cover_round_trip() -> TestResult {
        let mut f = Cursor::new(synth()?);
        let cover = crate::edit::Picture {
            mime_type: "image/jpeg".to_owned(),
            data: vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3],
        };
        let edit = TagEdit::new()
            .set_user_text("MOOD", "calm")
            .set_user_text("LABEL", "Test Records")
            .set_comment("remastered")
            .set_cover(cover.clone());
        update_tag(&mut f, &edit, &WriteOptions::default())?;

        let tag = read_tag(&mut f)?.ok_or("tag missing")?;
        let mut texts: Vec<(&str, &str)> = tag
            .extended_texts()
            .map(|t| (t.description.as_str(), t.value.as_str()))
            .collect();
        texts.sort_unstable();
        assert_eq!(texts, [("LABEL", "Test Records"), ("MOOD", "calm")]);
        assert_eq!(
            tag.comments().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            ["remastered"]
        );
        let picture = tag.pictures().next().ok_or("cover missing")?;
        assert_eq!(picture.data, cover.data);
        assert_eq!(picture.mime_type, cover.mime_type);
        assert_eq!(picture.picture_type, id3::frame::PictureType::CoverFront);
        Ok(())
    }

    #[test]
    fn replacing_and_clearing_extras_leaves_no_duplicates() -> TestResult {
        let mut f = Cursor::new(synth()?);
        let first = TagEdit::new()
            .set_user_text("MOOD", "calm")
            .set_comment("one")
            .set_cover(crate::edit::Picture {
                mime_type: "image/png".to_owned(),
                data: vec![1],
            });
        update_tag(&mut f, &first, &WriteOptions::default())?;
        let second = TagEdit::new()
            .set_user_text("MOOD", "tense")
            .set_comment("two")
            .set_cover(crate::edit::Picture {
                mime_type: "image/png".to_owned(),
                data: vec![2],
            });
        update_tag(&mut f, &second, &WriteOptions::default())?;

        let tag = read_tag(&mut f)?.ok_or("tag missing")?;
        assert_eq!(tag.extended_texts().count(), 1);
        assert_eq!(tag.comments().count(), 1);
        assert_eq!(
            tag.pictures().map(|p| p.data.clone()).collect::<Vec<_>>(),
            [vec![2]]
        );

        let clear = TagEdit::new()
            .clear_user_text("MOOD")
            .set_comment("  ")
            .clear_cover();
        update_tag(&mut f, &clear, &WriteOptions::default())?;
        let tag = read_tag(&mut f)?.ok_or("tag missing")?;
        assert_eq!(tag.extended_texts().count(), 0);
        assert_eq!(tag.comments().count(), 0);
        assert_eq!(tag.pictures().count(), 0);
        Ok(())
    }

    #[test]
    fn other_user_text_and_comments_survive() -> TestResult {
        let mut f = Cursor::new(synth()?);
        let mut tag = Tag::new();
        tag.add_frame(id3::frame::ExtendedText {
            description: "KEEP".into(),
            value: "me".into(),
        });
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: "other".into(),
            text: "described comment".into(),
        });
        write_tag(&mut f, &tag, Version::Id3v24)?;

        let edit = TagEdit::new()
            .set_user_text("MOOD", "x")
            .set_comment("plain");
        update_tag(&mut f, &edit, &WriteOptions::default())?;
        let tag = read_tag(&mut f)?.ok_or("tag missing")?;
        assert_eq!(tag.extended_texts().count(), 2);
        assert_eq!(tag.comments().count(), 2);
        Ok(())
    }

    #[test]
    fn extras_survive_id3v23() -> TestResult {
        let mut f = Cursor::new(synth()?);
        let edit = TagEdit::new()
            .set_user_text("MOOD", "calm")
            .set_comment("hello")
            .set_cover(crate::edit::Picture {
                mime_type: "image/jpeg".to_owned(),
                data: vec![9, 9],
            });
        let opts = WriteOptions::default().with_version(Version::Id3v23);
        update_tag(&mut f, &edit, &opts)?;
        let tag = read_tag(&mut f)?.ok_or("tag missing")?;
        assert_eq!(tag.version(), Version::Id3v23);
        assert_eq!(tag.extended_texts().count(), 1);
        assert_eq!(tag.comments().count(), 1);
        assert_eq!(tag.pictures().count(), 1);
        Ok(())
    }

    #[test]
    fn remove_tag_restores_a_tagless_file() -> TestResult {
        let original = synth()?;
        let mut f = Cursor::new(original.clone());
        update_tag(
            &mut f,
            &TagEdit::new().set(FrameId::TITLE, "T"),
            &WriteOptions::default(),
        )?;
        remove_tag(&mut f)?;
        assert_eq!(f.into_inner(), original);
        Ok(())
    }

    #[test]
    fn rejects_foreign_and_truncated_input() {
        let riff = read_tag(&mut Cursor::new(b"RIFF....".to_vec()));
        assert!(matches!(riff, Err(Error::BadSignature(_))));
        let short = read_tag(&mut Cursor::new(b"DSD ".to_vec()));
        assert!(matches!(short, Err(Error::Malformed(_))));
    }

    #[test]
    fn frame_id_validation() {
        assert!(FrameId::new("TIT2").is_ok());
        assert!(FrameId::new("tit2").is_err());
        assert!(FrameId::new("TXXX").is_err());
        assert!(FrameId::new("APIC").is_err());
        assert!(FrameId::new("TI").is_err());
    }
}
