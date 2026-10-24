# Changelog

All notable changes are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/).

## [0.1.0] - unreleased

First release.

### Splitting
- Split DSF and DSDIFF albums into per-track `.dsf` files from a CUE sheet, bit for bit:
  the audio is cut on block boundaries and never re-encoded.
- Split a `.dsf`/`.dff` without a CUE sheet: automatic silence detection (`--auto`), cut
  points typed or dragged on a waveform (`--at`, **Add audio file…**), and **Export CUE
  sheet** / `--write-cue`.
- CUE support: encoding detection (UTF-16, Shift-JIS, Windows-1252, …), `FILE` names matched
  case-insensitively and across `.wav`/`.flac` to `.dsf`/`.dff`, a **Locate…** button for
  missing sources, pregap (`INDEX 00`) handling and an optional hidden track `00`.
- Progress with throughput and time remaining; cancel takes effect within a block.
- Verification: every output is compared bit by bit with its source (Settings option,
  Verify tab, `--verify`).

### Tags and names
- Metadata editing: album defaults and per-track overrides for 14 standard fields, custom
  fields, bulk editing of selected tracks, track reordering and renumbering.
- Cover art found next to the CUE sheet, embedded in every track, optionally copied as
  `folder.jpg`.
- File and folder name templates (Settings, `--name`, `--folder`, `--native-names`);
  punctuation and non-Latin text are kept and only unsafe characters become `_`.
- Tag-only mode (`--tag-only`, `--in`) that writes tags into existing `.dsf` files.
- Optional MusicBrainz and Discogs lookup (`--features online`, `--lookup`,
  `--lookup-query`, `--lookup-source`, `--discogs-token`); off by default, so standard builds
  contain no network code.

### App
- GUI and command line, project files (`.dsfproj`), undo/redo, keyboard shortcuts, audition
  playback (Cargo feature `playback`, default on), first-run guide, CJK font fallback,
  actionable error messages, WCAG-checked light and dark paper themes, a settings dialog
  with page list, an animated first-run screen, collapsible album-tab sections (switch in
  Settings), a drop-target overlay, keyboard focus outlines, dismissible result
  messages, a confirmation before *Clear all*, and diagnostics that leave secrets out.

### Libraries
- `dsf`: DSF header parsing and ID3v2 tag read/write (text, custom, comment, cover) with a
  `Field` layer for the common fields.
- `dsdiff`: DSDIFF header parsing.
- `release-lookup`: MusicBrainz and Discogs search and release parsing, usable on its own.

### Quality
- End-to-end tests with an independent bit oracle, `cargo-fuzz` targets and CI on Linux,
  macOS and Windows; minimum supported Rust version 1.88.
- Release workflow producing Windows, macOS and Linux archives with checksums.
- Project website, including the guides in `docs/` as web pages built from the same
  Markdown.
