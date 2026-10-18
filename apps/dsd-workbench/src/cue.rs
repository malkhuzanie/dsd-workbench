//! A forgiving line-based CUE sheet parser.

use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct CueTrack {
    pub number: u32,
    pub title: String,
    pub artist: String,
    pub composer: String,
    /// `INDEX 01`: where the track proper begins.
    pub start_seconds: f64,
    /// `INDEX 00`: where the pregap before the track begins, if the sheet has one.
    pub pregap_seconds: Option<f64>,
    /// `ISRC`, empty when the sheet has none.
    pub isrc: String,
    pub source_file: String,
}

#[derive(Debug, Clone, Default)]
pub struct CueSheet {
    /// Empty when the sheet has no album `TITLE`.
    pub album_title: String,
    pub performer: String,
    pub composer: String,
    pub genre: String,
    pub date: String,
    /// `REM COMMENT`.
    pub comment: String,
    /// `REM DISCNUMBER`, for example `1` or `1/2`.
    pub disc: String,
    /// `CATALOG`: the album's catalogue / barcode number.
    pub catalog: String,
    pub tracks: Vec<CueTrack>,
    /// Problems that did not stop parsing (skipped tracks, bad times, ...).
    pub warnings: Vec<String>,
}

struct Pending {
    number: u32,
    title: Option<String>,
    performer: Option<String>,
    composer: Option<String>,
    start: Option<f64>,
    pregap: Option<f64>,
    isrc: Option<String>,
    source_file: String,
}

pub struct CueParser;

impl CueParser {
    pub fn parse_file<P: AsRef<Path>>(path: P) -> io::Result<CueSheet> {
        let bytes = fs::read(path)?;
        Ok(Self::parse_str(&decode(&bytes)))
    }

    pub fn parse_str(text: &str) -> CueSheet {
        let mut sheet = CueSheet::default();
        let mut file = String::new();
        let mut pending: Option<Pending> = None;
        let mut in_track = false;

        for raw in text.lines() {
            let line = raw.trim_start_matches('\u{feff}').trim();
            let (command, rest) = split_command(line);

            match command.to_ascii_uppercase().as_str() {
                "FILE" => {
                    finish(&mut pending, &mut sheet);
                    file = parse_file_name(rest);
                }
                "TRACK" => {
                    finish(&mut pending, &mut sheet);
                    in_track = true;
                    let mut words = rest.split_whitespace();
                    let number = words.next().and_then(|n| n.parse::<u32>().ok());
                    let kind = words.next().unwrap_or("");
                    pending = match number {
                        Some(number) if kind.eq_ignore_ascii_case("AUDIO") => Some(Pending {
                            number,
                            title: None,
                            performer: None,
                            composer: None,
                            start: None,
                            pregap: None,
                            isrc: None,
                            source_file: file.clone(),
                        }),
                        _ => {
                            sheet
                                .warnings
                                .push(format!("Ignored non-audio or malformed track: {line}"));
                            None
                        }
                    };
                }
                "TITLE" => match pending.as_mut() {
                    Some(track) => track.title = Some(unquote(rest)),
                    None if !in_track => sheet.album_title = unquote(rest),
                    None => {}
                },
                "PERFORMER" => match pending.as_mut() {
                    Some(track) => track.performer = Some(unquote(rest)),
                    None if !in_track => sheet.performer = unquote(rest),
                    None => {}
                },
                "SONGWRITER" => set_composer(&mut pending, &mut sheet, in_track, unquote(rest)),
                "CATALOG" if !in_track => sheet.catalog = unquote(rest),
                "ISRC" => {
                    if let Some(track) = pending.as_mut() {
                        track.isrc = Some(unquote(rest));
                    }
                }
                "REM" => {
                    let (key, value) = split_command(rest);
                    match key.to_ascii_uppercase().as_str() {
                        "DATE" if !in_track => sheet.date = unquote(value),
                        "GENRE" if !in_track => sheet.genre = unquote(value),
                        "COMMENT" if !in_track => sheet.comment = unquote(value),
                        "DISCNUMBER" if !in_track => sheet.disc = unquote(value),
                        "COMPOSER" => {
                            set_composer(&mut pending, &mut sheet, in_track, unquote(value))
                        }
                        _ => {}
                    }
                }
                "INDEX" => {
                    if let Some(track) = pending.as_mut() {
                        let mut words = rest.split_whitespace();
                        if let (Some(index), Some(time)) = (words.next(), words.next())
                            && let Ok(index @ (0 | 1)) = index.parse::<u32>()
                        {
                            match parse_index_time(time) {
                                Some(seconds) if index == 1 => track.start = Some(seconds),
                                Some(seconds) => track.pregap = Some(seconds),
                                None => sheet.warnings.push(format!(
                                    "Track {:02}: invalid INDEX {index:02} time '{time}'.",
                                    track.number
                                )),
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        finish(&mut pending, &mut sheet);

        let (performer, composer) = (sheet.performer.clone(), sheet.composer.clone());
        for track in &mut sheet.tracks {
            if track.artist.is_empty() {
                track.artist.clone_from(&performer);
            }
            if track.composer.is_empty() {
                track.composer.clone_from(&composer);
            }
        }
        sheet
    }
}

fn set_composer(
    pending: &mut Option<Pending>,
    sheet: &mut CueSheet,
    in_track: bool,
    value: String,
) {
    match pending.as_mut() {
        Some(track) => track.composer = Some(value),
        None if !in_track => sheet.composer = value,
        None => {}
    }
}

fn finish(pending: &mut Option<Pending>, sheet: &mut CueSheet) {
    let Some(track) = pending.take() else { return };
    match track.start {
        Some(start_seconds) => sheet.tracks.push(CueTrack {
            number: track.number,
            title: track
                .title
                .unwrap_or_else(|| format!("Track {:02}", track.number)),
            artist: track.performer.unwrap_or_default(),
            composer: track.composer.unwrap_or_default(),
            start_seconds,
            // A pregap that starts after the track proper is nonsense; ignore it.
            pregap_seconds: track.pregap.filter(|gap| *gap <= start_seconds),
            isrc: track.isrc.unwrap_or_default(),
            source_file: track.source_file,
        }),
        None => sheet.warnings.push(format!(
            "Track {:02} has no usable INDEX 01 and was skipped.",
            track.number
        )),
    }
}

/// Decodes a CUE file: a byte-order mark wins, then valid UTF-8, then a statistical guess
/// (Shift-JIS, GBK, Windows-1252 and so on) so legacy rips keep their titles.
fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _) = encoding.decode_without_bom_handling(bytes.get(bom_len..).unwrap_or(&[]));
        return text.into_owned();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
    detector.feed(bytes, true);
    let (text, _) = detector
        .guess(None, chardetng::Utf8Detection::Deny)
        .decode_without_bom_handling(bytes);
    text.into_owned()
}

fn split_command(line: &str) -> (&str, &str) {
    match line.split_once(char::is_whitespace) {
        Some((command, rest)) => (command, rest.trim()),
        None => (line, ""),
    }
}

fn unquote(text: &str) -> String {
    let text = text.trim();
    match text.strip_prefix('"') {
        Some(inner) => inner.split('"').next().unwrap_or("").to_owned(),
        None => text.to_owned(),
    }
}

/// `FILE "name.dsf" WAVE` or `FILE name.dsf WAVE`.
fn parse_file_name(rest: &str) -> String {
    if rest.starts_with('"') {
        return unquote(rest);
    }
    rest.rsplit_once(char::is_whitespace)
        .map_or(rest, |(name, _kind)| name)
        .trim()
        .to_owned()
}

/// `mm:ss:ff` with 75 frames per second.
fn parse_index_time(text: &str) -> Option<f64> {
    let mut parts = text.split(':');
    let (m, s, f) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let (m, s, f): (u32, u32, u32) = (m.parse().ok()?, s.parse().ok()?, f.parse().ok()?);
    if s >= 60 || f >= 75 {
        return None;
    }
    Some(f64::from(m) * 60.0 + f64::from(s) + f64::from(f) / 75.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = "\u{feff}REM GENRE Jazz\r\nREM DATE 1959\r\nPERFORMER \"Miles\"\r\nTITLE \"Kind of Blue\"\r\n\
FILE \"side a.dsf\" WAVE\r\n  TRACK 01 AUDIO\r\n    TITLE \"So What\"\r\n    INDEX 01 00:00:00\r\n\
  TRACK 02 AUDIO\r\n    TITLE \"Blue\"\r\n    PERFORMER \"Coltrane\"\r\n    INDEX 00 09:00:00\r\n    INDEX 01 09:30:15\r\n\
FILE side_b.dsf WAVE\r\n  TRACK 03 AUDIO\r\n    INDEX 01 00:00:00\r\n  TRACK 04 AUDIO\r\n    TITLE \"No index\"\r\n";

    #[test]
    fn parses_album_and_tracks() {
        let sheet = CueParser::parse_str(SHEET);
        assert_eq!(sheet.album_title, "Kind of Blue");
        assert_eq!(
            (sheet.genre.as_str(), sheet.date.as_str()),
            ("Jazz", "1959")
        );
        assert_eq!(sheet.tracks.len(), 3);
        assert_eq!(
            sheet.tracks[0].artist, "Miles",
            "inherits the album performer"
        );
        assert_eq!(sheet.tracks[1].artist, "Coltrane");
        assert!((sheet.tracks[1].start_seconds - (9.0 * 60.0 + 30.0 + 15.0 / 75.0)).abs() < 1e-9);
        assert_eq!(sheet.tracks[0].pregap_seconds, None);
        assert_eq!(sheet.tracks[1].pregap_seconds, Some(9.0 * 60.0));
        assert_eq!(sheet.tracks[2].title, "Track 03");
        assert_eq!(sheet.tracks[2].source_file, "side_b.dsf");
        assert_eq!(sheet.warnings.len(), 1, "track 4 has no INDEX 01");
    }

    const TITLE_SHEET: &str = "PERFORMER \"{artist}\"\r\nTITLE \"{album}\"\r\nFILE \"a.wav\" WAVE\r\n  TRACK 01 AUDIO\r\n    TITLE \"{track}\"\r\n    INDEX 01 00:00:00\r\n";

    fn sheet_with(artist: &str, album: &str, track: &str) -> String {
        TITLE_SHEET
            .replace("{artist}", artist)
            .replace("{album}", album)
            .replace("{track}", track)
    }

    #[test]
    fn utf16_with_bom_is_decoded() {
        let text = sheet_with("坂本龍一", "async", "andata");
        for (bom, encode) in [
            ([0xFFu8, 0xFE], u16::to_le_bytes as fn(u16) -> [u8; 2]),
            ([0xFE, 0xFF], u16::to_be_bytes),
        ] {
            let mut bytes = bom.to_vec();
            text.encode_utf16().for_each(|u| bytes.extend(encode(u)));
            let sheet = CueParser::parse_str(&decode(&bytes));
            assert_eq!(sheet.performer, "坂本龍一");
        }
    }

    #[test]
    fn utf8_with_bom_is_decoded() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(sheet_with("Björk", "Homogenic", "Jóga").bytes());
        assert_eq!(CueParser::parse_str(&decode(&bytes)).performer, "Björk");
    }

    #[test]
    fn shift_jis_is_detected() {
        let text = sheet_with("宇多田ヒカル", "初恋", "First Love");
        let (bytes, _, lossy) = encoding_rs::SHIFT_JIS.encode(&text);
        assert!(!lossy);
        let sheet = CueParser::parse_str(&decode(&bytes));
        assert_eq!(sheet.performer, "宇多田ヒカル");
        assert_eq!(sheet.album_title, "初恋");
    }

    #[test]
    fn windows_1252_is_detected() {
        let text = sheet_with("Mötley Crüe", "Dr. Feelgood", "Kickstart My Heart");
        let (bytes, _, lossy) = encoding_rs::WINDOWS_1252.encode(&text);
        assert!(!lossy);
        assert_eq!(
            CueParser::parse_str(&decode(&bytes)).performer,
            "Mötley Crüe"
        );
    }

    #[test]
    fn undecodable_input_still_yields_text() {
        let _ = decode(&[0xFF, 0xFE, 0x00, 0xD8, 0x00]);
        let _ = decode(&[]);
    }

    #[test]
    fn extra_metadata_commands_are_read() {
        let sheet = CueParser::parse_str(
            "CATALOG 0602498\nREM COMMENT \"Remastered 2020\"\nREM DISCNUMBER 1/2\n\
             FILE \"a.dsf\" WAVE\n TRACK 01 AUDIO\n  ISRC USABC1234567\n  INDEX 01 00:00:00\n\
             TRACK 02 AUDIO\n  REM COMMENT \"not the album's\"\n  INDEX 01 03:00:00\n",
        );
        assert_eq!(sheet.catalog, "0602498");
        assert_eq!(sheet.comment, "Remastered 2020");
        assert_eq!(sheet.disc, "1/2");
        assert_eq!(sheet.tracks[0].isrc, "USABC1234567");
        assert_eq!(sheet.tracks[1].isrc, "");
        assert_eq!(
            sheet.comment, "Remastered 2020",
            "a track's REM COMMENT is not the album's"
        );
    }

    #[test]
    fn mangled_sheets_never_panic() {
        let bytes = SHEET.as_bytes();
        // Every prefix, and a deterministic spread of single-byte replacements.
        for len in 0..=bytes.len() {
            let text = String::from_utf8_lossy(bytes.get(..len).unwrap_or_default());
            let _ = CueParser::parse_str(&text);
        }
        let mut x = 12_345u32;
        for _ in 0..2_000 {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let mut copy = bytes.to_vec();
            let at = (x >> 8) as usize % copy.len();
            if let Some(byte) = copy.get_mut(at) {
                *byte = (x >> 24) as u8;
            }
            let _ = CueParser::parse_str(&String::from_utf8_lossy(&copy));
        }
    }

    #[test]
    fn rejects_out_of_range_times() {
        assert_eq!(parse_index_time("00:60:00"), None);
        assert_eq!(parse_index_time("00:00:75"), None);
        assert_eq!(parse_index_time("1:2"), None);
    }
}
