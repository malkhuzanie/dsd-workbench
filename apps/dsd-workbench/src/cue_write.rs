//! Writes a plan back out as a CUE sheet.
//!
//! CUE times have a resolution of 1/75 s (about 13 ms), so a cut point that was placed
//! more precisely is rounded to the nearest frame. Save a project file to keep exact cuts.

use crate::metadata::{FieldKey, Standard};
use crate::plan::{CATALOG_FIELD, SplitPlan};

const FRAMES_PER_SECOND: f64 = 75.0;

/// `mm:ss:ff`, rounded to the nearest frame.
pub fn format_index(seconds: f64) -> String {
    let frames = (seconds.max(0.0) * FRAMES_PER_SECOND).round() as u64;
    let (minutes, rest) = (frames / (60 * 75), frames % (60 * 75));
    format!("{minutes:02}:{:02}:{:02}", rest / 75, rest % 75)
}

/// Quotes a value, replacing the one character CUE cannot hold inside quotes.
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "'").replace(['\r', '\n'], " "))
}

/// The plan as CUE text (UTF-8, `\n` line endings).
pub fn to_cue(plan: &SplitPlan) -> String {
    let album = &plan.album;
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };

    let rem = |name: &str, key: &FieldKey| {
        let value = album.value(key);
        (!value.trim().is_empty()).then(|| format!("REM {name} {}", quote(value)))
    };
    let catalog = FieldKey::Custom(CATALOG_FIELD.to_owned());
    let catalog_value = album.value(&catalog);
    if !catalog_value.trim().is_empty() {
        line(format!("CATALOG {}", catalog_value.trim()));
    }
    for entry in [
        rem("GENRE", &FieldKey::GENRE),
        rem("DATE", &FieldKey::DATE),
        rem("DISCNUMBER", &FieldKey::Standard(Standard::Disc)),
        rem("COMMENT", &FieldKey::Standard(Standard::Comment)),
    ]
    .into_iter()
    .flatten()
    {
        line(entry);
    }
    if !album.value(&FieldKey::ALBUM_ARTIST).trim().is_empty() {
        line(format!(
            "PERFORMER {}",
            quote(album.value(&FieldKey::ALBUM_ARTIST))
        ));
    }
    if !album.value(&FieldKey::ALBUM).trim().is_empty() {
        line(format!("TITLE {}", quote(album.value(&FieldKey::ALBUM))));
    }

    let mut file = "";
    for track in &plan.tracks {
        if track.source_file != file {
            file = &track.source_file;
            line(format!("FILE {} WAVE", quote(file)));
        }
        let resolve = |key: &FieldKey| album.resolve(&track.meta, key);
        line(format!("  TRACK {:02} AUDIO", track.meta.number));
        let title = resolve(&FieldKey::TITLE);
        if !title.trim().is_empty() {
            line(format!("    TITLE {}", quote(title)));
        }
        let artist = resolve(&FieldKey::ARTIST);
        if !artist.trim().is_empty() {
            line(format!("    PERFORMER {}", quote(artist)));
        }
        let composer = resolve(&FieldKey::COMPOSER);
        if !composer.trim().is_empty() {
            line(format!("    SONGWRITER {}", quote(composer)));
        }
        let isrc = resolve(&FieldKey::Standard(Standard::Isrc));
        if !isrc.trim().is_empty() {
            line(format!("    ISRC {}", isrc.trim()));
        }
        line(format!(
            "    INDEX 01 {}",
            format_index(track.start_seconds)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::CueParser;
    use crate::plan::CueOptions;

    #[test]
    fn index_times_round_to_frames() {
        assert_eq!(format_index(0.0), "00:00:00");
        assert_eq!(format_index(1.0), "00:01:00");
        assert_eq!(format_index(61.5), "01:01:38", "37.5 frames rounds up");
        assert_eq!(format_index(61.0 + 37.0 / 75.0), "01:01:37");
        assert_eq!(format_index(3.0 + 74.4 / 75.0), "00:03:74");
        assert_eq!(format_index(-5.0), "00:00:00");
    }

    #[test]
    fn quotes_cannot_break_out_of_a_value() {
        assert_eq!(quote("Say \"hi\"\nthere"), "\"Say 'hi' there\"");
    }

    const SOURCE: &str = "CATALOG 0602498\nREM GENRE \"Jazz\"\nREM DATE \"1959\"\n\
        REM COMMENT \"Remastered\"\nPERFORMER \"Miles Davis\"\nTITLE \"Kind of Blue\"\n\
        FILE \"a.dsf\" WAVE\n TRACK 01 AUDIO\n  TITLE \"So What\"\n  ISRC USABC1234567\n  INDEX 01 00:00:00\n\
         TRACK 02 AUDIO\n  TITLE \"Blue in Green\"\n  PERFORMER \"Bill Evans\"\n  INDEX 01 09:22:30\n\
        FILE \"b.dsf\" WAVE\n TRACK 03 AUDIO\n  TITLE \"Flamenco Sketches\"\n  SONGWRITER \"Davis\"\n  INDEX 01 00:00:00\n";

    #[test]
    fn a_written_cue_parses_back_to_the_same_plan() {
        let plan = SplitPlan::from_cue(&CueParser::parse_str(SOURCE), CueOptions::default());
        let text = to_cue(&plan);
        let again = SplitPlan::from_cue(&CueParser::parse_str(&text), CueOptions::default());
        assert_eq!(again, plan, "{text}");
    }

    #[test]
    fn an_empty_plan_still_produces_valid_text() {
        let text = to_cue(&SplitPlan::from_cue(
            &CueParser::parse_str("FILE \"x.dsf\" WAVE\n TRACK 01 AUDIO\n  INDEX 01 00:00:00\n"),
            CueOptions::default(),
        ));
        assert!(text.contains("FILE \"x.dsf\" WAVE"));
        assert!(text.contains("TRACK 01 AUDIO"));
        assert!(!text.contains("PERFORMER"));
    }
}
