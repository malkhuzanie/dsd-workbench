//! Output file and folder names built from templates such as `{track:02} - {title}`.
//!
//! A template is literal text plus placeholders in braces:
//!
//! | Placeholder | Value |
//! |-------------|-------|
//! | `{track}` | track number; `{track:02}` pads it to two digits |
//! | `{title}`, `{artist}`, `{albumartist}`, `{album}`, `{composer}`, `{conductor}`, `{genre}`, `{publisher}` | the tag values |
//! | `{date}`, `{year}` | the date as written, or its first four digits |
//! | `{disc}` | disc number (the part before any `/`) |
//! | `{tag:NAME}` | a custom field |
//!
//! `{{` and `}}` produce literal braces. Values are made safe for file names; the
//! literal text of the template is kept as written. In a folder template `/` starts a
//! new sub-folder.

use std::fmt;

use crate::metadata::{FieldKey, Standard};

/// File name template used unless the user picks another.
pub const DEFAULT_FILE_TEMPLATE: &str = "{track:02} - {title}";
/// Extension of every file the tool writes.
pub const EXTENSION: &str = "dsf";

const FALLBACK_STEM: &str = "Untitled";
/// Longest file stem (and folder name), in bytes, leaving room for `.dsf.part`.
const MAX_COMPONENT_BYTES: usize = 200;
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// How strictly names are made safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NameStyle {
    /// Works on every file system (Windows, macOS, Linux, exFAT, NAS shares).
    #[default]
    Portable,
    /// Only what the current operating system forbids.
    Native,
}

impl NameStyle {
    pub const ALL: [Self; 2] = [Self::Portable, Self::Native];

    pub fn label(self) -> &'static str {
        match self {
            Self::Portable => "Portable (safe on every system)",
            Self::Native => "This system only",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::Native => "native",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.key() == key)
    }

    /// Whether Windows' rules apply.
    fn windows_rules(self) -> bool {
        self == Self::Portable || cfg!(windows)
    }
}

/// Where tracks are written, relative to the album's output folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    /// Template for the file name, without extension.
    pub file: String,
    /// Template for sub-folders below the output folder; empty for none.
    pub folder: String,
    pub style: NameStyle,
}

impl Default for Naming {
    fn default() -> Self {
        Self {
            file: DEFAULT_FILE_TEMPLATE.to_owned(),
            folder: String::new(),
            style: NameStyle::default(),
        }
    }
}

impl Naming {
    /// Why the templates cannot be used, if they cannot.
    pub fn validate(&self) -> Result<(), TemplateError> {
        let file = Template::parse(&self.file)?;
        Template::parse(&self.folder)?;
        if file.is_blank() {
            return Err(TemplateError::Empty);
        }
        Ok(())
    }
}

/// Why a template could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// The file name template has nothing in it.
    Empty,
    /// A `{` was never closed.
    Unclosed,
    /// A `}` appeared without a matching `{`.
    Stray,
    /// The placeholder name is not known.
    Unknown(String),
    /// The width after `:` is not a number.
    BadWidth(String),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the file name template is empty"),
            Self::Unclosed => write!(f, "a '{{' is missing its '}}'"),
            Self::Stray => write!(
                f,
                "a '}}' has no matching '{{' (use '}}}}' for a literal brace)"
            ),
            Self::Unknown(name) => write!(f, "unknown placeholder '{{{name}}}'"),
            Self::BadWidth(width) => write!(f, "'{width}' is not a number of digits"),
        }
    }
}

impl std::error::Error for TemplateError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Literal(String),
    Field {
        source: Source,
        /// Zero-pad numbers to this width.
        width: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Track,
    Year,
    Disc,
    Key(FieldKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Template(Vec<Part>);

impl Template {
    fn parse(text: &str) -> Result<Self, TemplateError> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '}' => return Err(TemplateError::Stray),
                '{' => {
                    let mut name = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) => name.push(c),
                            None => return Err(TemplateError::Unclosed),
                        }
                    }
                    if !literal.is_empty() {
                        parts.push(Part::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(parse_placeholder(&name)?);
                }
                c => literal.push(c),
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }
        Ok(Self(parts))
    }

    /// True if the template has no content at all.
    fn is_blank(&self) -> bool {
        self.0.is_empty()
    }

    /// Fills in the placeholders; `lookup` gives the raw value for a key.
    fn render<'a>(
        &self,
        style: NameStyle,
        number: u32,
        lookup: impl Fn(&FieldKey) -> &'a str,
    ) -> String {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                Part::Literal(text) => out.push_str(text),
                Part::Field { source, width } => {
                    let value = match source {
                        Source::Track => {
                            let width = width.unwrap_or(0);
                            format!("{number:0width$}")
                        }
                        Source::Year => year_of(lookup(&FieldKey::DATE)).to_owned(),
                        Source::Disc => {
                            disc_of(lookup(&FieldKey::Standard(Standard::Disc))).to_owned()
                        }
                        Source::Key(key) => lookup(key).to_owned(),
                    };
                    out.push_str(&clean_value(&value, style));
                }
            }
        }
        out
    }
}

fn parse_placeholder(name: &str) -> Result<Part, TemplateError> {
    let (name, width) = match name.split_once(':') {
        Some(("tag", custom)) => {
            let key =
                FieldKey::custom(custom).ok_or_else(|| TemplateError::Unknown(name.to_owned()))?;
            return Ok(Part::Field {
                source: Source::Key(key),
                width: None,
            });
        }
        Some((base, width)) => {
            let width = width
                .trim()
                .parse::<usize>()
                .map_err(|_| TemplateError::BadWidth(width.to_owned()))?;
            (base, Some(width.min(9)))
        }
        None => (name, None),
    };
    let standard = |s: Standard| Source::Key(FieldKey::Standard(s));
    let source = match name.trim().to_ascii_lowercase().as_str() {
        "track" => Source::Track,
        "year" => Source::Year,
        "disc" => Source::Disc,
        "title" => standard(Standard::Title),
        "artist" => standard(Standard::Artist),
        "albumartist" => standard(Standard::AlbumArtist),
        "album" => standard(Standard::Album),
        "composer" => standard(Standard::Composer),
        "conductor" => standard(Standard::Conductor),
        "genre" => standard(Standard::Genre),
        "date" => standard(Standard::Date),
        "publisher" => standard(Standard::Publisher),
        _ => return Err(TemplateError::Unknown(name.to_owned())),
    };
    Ok(Part::Field { source, width })
}

/// The first four consecutive digits of a date, or the empty string.
fn year_of(date: &str) -> &str {
    let bytes = date.as_bytes();
    (0..bytes.len().saturating_sub(3))
        .find(|&i| {
            bytes
                .get(i..i + 4)
                .is_some_and(|w| w.iter().all(u8::is_ascii_digit))
        })
        .and_then(|i| date.get(i..i + 4))
        .unwrap_or_default()
}

/// `"1/2"` -> `"1"`.
fn disc_of(disc: &str) -> &str {
    disc.split('/').next().unwrap_or_default().trim()
}

/// Characters no path component may contain on Windows (a superset of other systems').
const WINDOWS_FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

fn is_forbidden(c: char, style: NameStyle) -> bool {
    if c.is_control() || c == '/' || c == '\0' {
        return true;
    }
    style.windows_rules() && WINDOWS_FORBIDDEN.contains(&c)
}

/// A tag value made safe to embed in a name: forbidden characters become `_`.
fn clean_value(value: &str, style: NameStyle) -> String {
    value
        .chars()
        .map(|c| if is_forbidden(c, style) { '_' } else { c })
        .collect()
}

/// Final checks on one path component (a file stem or folder name).
fn finish_component(component: &str, style: NameStyle) -> String {
    let mut text = if style.windows_rules() {
        // Windows silently drops trailing dots and spaces, which then collide.
        component
            .trim_end_matches(['.', ' '])
            .trim_start()
            .to_owned()
    } else {
        component.trim().to_owned()
    };
    text = truncate_bytes(&text, MAX_COMPONENT_BYTES);
    if style.windows_rules() {
        text = text.trim_end_matches(['.', ' ']).to_owned();
        let stem = text.split('.').next().unwrap_or_default();
        if WINDOWS_RESERVED
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(stem))
        {
            text.insert(0, '_');
        }
    }
    if text.is_empty() || text == "." || text == ".." {
        FALLBACK_STEM.to_owned()
    } else {
        text
    }
}

/// Cuts `text` to at most `max` bytes without splitting a character.
fn truncate_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default().to_owned()
}

impl Naming {
    /// The folder components below the output folder (possibly none).
    pub fn folders<'a>(
        &self,
        number: u32,
        lookup: impl Fn(&FieldKey) -> &'a str + Copy,
    ) -> Vec<String> {
        let Ok(template) = Template::parse(&self.folder) else {
            return Vec::new();
        };
        template
            .render(self.style, number, lookup)
            .split('/')
            .filter(|part| !part.trim().is_empty())
            .map(|part| finish_component(part, self.style))
            .collect()
    }

    /// The file name, including the extension. An unusable template falls back to the default.
    pub fn file_name<'a>(&self, number: u32, lookup: impl Fn(&FieldKey) -> &'a str) -> String {
        let template = Template::parse(&self.file)
            .ok()
            .filter(|t| !t.is_blank())
            .or_else(|| Template::parse(DEFAULT_FILE_TEMPLATE).ok());
        let stem = template.map_or_else(String::new, |t| t.render(self.style, number, lookup));
        // A '/' written in the template itself must not create a path.
        let stem = stem.replace('/', "_");
        format!("{}.{EXTENSION}", finish_component(&stem, self.style))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(pairs: &'a [(FieldKey, &'a str)]) -> impl Fn(&FieldKey) -> &'a str + Copy {
        move |key| pairs.iter().find(|(k, _)| k == key).map_or("", |(_, v)| *v)
    }

    fn name(file: &str, number: u32, pairs: &[(FieldKey, &str)]) -> String {
        let naming = Naming {
            file: file.to_owned(),
            ..Naming::default()
        };
        naming.file_name(number, lookup(pairs))
    }

    #[test]
    fn default_template_pads_the_track_number() {
        let pairs = [(FieldKey::TITLE, "So What")];
        assert_eq!(
            Naming::default().file_name(3, lookup(&pairs)),
            "03 - So What.dsf"
        );
    }

    #[test]
    fn placeholders_and_widths() {
        let pairs = [
            (FieldKey::TITLE, "Blue"),
            (FieldKey::ARTIST, "Miles"),
            (FieldKey::DATE, "1959-08-17"),
        ];
        assert_eq!(
            name("{artist} - {title} ({year}) #{track:3}", 7, &pairs),
            "Miles - Blue (1959) #007.dsf"
        );
        assert_eq!(
            name("{TITLE}", 1, &pairs),
            "Blue.dsf",
            "names are case-insensitive"
        );
    }

    #[test]
    fn unsafe_characters_in_values_become_underscores() {
        let pairs = [(FieldKey::TITLE, "AC/DC: Hells*Bells?")];
        assert_eq!(name("{title}", 1, &pairs), "AC_DC_ Hells_Bells_.dsf");
    }

    #[test]
    fn literal_text_is_kept_and_braces_can_be_escaped() {
        let pairs = [(FieldKey::TITLE, "T")];
        assert_eq!(name("[{{{title}}}]", 1, &pairs), "[{T}].dsf");
    }

    #[test]
    fn custom_fields_are_available() {
        let pairs = [(FieldKey::Custom("MOOD".into()), "calm")];
        assert_eq!(name("{tag:MOOD}-{track}", 2, &pairs), "calm-2.dsf");
    }

    #[test]
    fn missing_values_leave_the_rest_intact() {
        assert_eq!(name("{artist} - {title}", 1, &[]), "-.dsf");
        assert_eq!(name("{title}", 1, &[]), "Untitled.dsf");
    }

    #[test]
    fn invalid_templates_are_reported() {
        let check = |text: &str| {
            Naming {
                file: text.to_owned(),
                ..Naming::default()
            }
            .validate()
        };
        assert_eq!(check("{title"), Err(TemplateError::Unclosed));
        assert_eq!(check("title}"), Err(TemplateError::Stray));
        assert_eq!(check("{nope}"), Err(TemplateError::Unknown("nope".into())));
        assert_eq!(
            check("{track:xx}"),
            Err(TemplateError::BadWidth("xx".into()))
        );
        assert_eq!(check(""), Err(TemplateError::Empty));
        assert!(check("{track:02} {title}").is_ok());
    }

    #[test]
    fn an_invalid_file_template_falls_back_to_the_default() {
        let pairs = [(FieldKey::TITLE, "T")];
        assert_eq!(name("{broken", 4, &pairs), "04 - T.dsf");
    }

    #[test]
    fn windows_reserved_names_trailing_dots_and_spaces_are_handled() {
        let reserved = |title: &str| name("{title}", 1, &[(FieldKey::TITLE, title)]);
        assert_eq!(reserved("CON"), "_CON.dsf");
        assert_eq!(reserved("nul"), "_nul.dsf");
        assert_eq!(reserved("Intro. "), "Intro.dsf");
        assert_eq!(reserved("..."), "Untitled.dsf");
        assert_eq!(reserved("   "), "Untitled.dsf");
        assert_eq!(reserved("Console"), "Console.dsf");
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary() {
        let long = "é".repeat(300);
        let result = name("{title}", 1, &[(FieldKey::TITLE, &long)]);
        let stem = result.trim_end_matches(".dsf");
        assert!(stem.len() <= MAX_COMPONENT_BYTES);
        assert!(stem.chars().all(|c| c == 'é'));
    }

    #[test]
    fn non_latin_text_is_preserved() {
        let pairs = [(FieldKey::TITLE, "初恋 – First Love")];
        assert_eq!(name("{title}", 1, &pairs), "初恋 – First Love.dsf");
    }

    #[test]
    fn folders_split_on_slashes_and_skip_empty_parts() {
        let pairs = [
            (FieldKey::ALBUM_ARTIST, "Miles Davis"),
            (FieldKey::ALBUM, "Kind of Blue"),
            (FieldKey::DATE, "1959"),
        ];
        let naming = Naming {
            folder: "{albumartist}/{album} ({year})".to_owned(),
            ..Naming::default()
        };
        assert_eq!(
            naming.folders(1, lookup(&pairs)),
            ["Miles Davis", "Kind of Blue (1959)"]
        );

        let missing = Naming {
            folder: "{albumartist}/{album}".to_owned(),
            ..Naming::default()
        };
        assert!(missing.folders(1, lookup(&[])).is_empty());
        assert!(Naming::default().folders(1, lookup(&pairs)).is_empty());
    }

    #[test]
    fn slashes_in_values_cannot_create_folders() {
        let pairs = [(FieldKey::ALBUM, "AC/DC Live")];
        let naming = Naming {
            folder: "{album}".to_owned(),
            ..Naming::default()
        };
        assert_eq!(naming.folders(1, lookup(&pairs)), ["AC_DC Live"]);
    }

    #[test]
    fn dot_dot_cannot_escape_the_output_folder() {
        let pairs = [(FieldKey::ALBUM, "..")];
        let naming = Naming {
            folder: "{album}".to_owned(),
            ..Naming::default()
        };
        assert_eq!(naming.folders(1, lookup(&pairs)), ["Untitled"]);
        let literal = Naming {
            folder: "../x".to_owned(),
            ..Naming::default()
        };
        let parts = literal.folders(1, lookup(&[]));
        assert!(parts.iter().all(|p| p != ".."), "{parts:?}");
    }

    #[test]
    fn year_and_disc_helpers() {
        assert_eq!(year_of("Recorded 1959-08"), "1959");
        assert_eq!(year_of("59"), "");
        assert_eq!(year_of(""), "");
        assert_eq!(disc_of("2/3"), "2");
        assert_eq!(disc_of(""), "");
    }

    #[test]
    fn style_keys_round_trip() {
        for style in NameStyle::ALL {
            assert_eq!(NameStyle::from_key(style.key()), Some(style));
        }
        assert_eq!(NameStyle::from_key("other"), None);
    }
}
