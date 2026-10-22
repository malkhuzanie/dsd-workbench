# Features

The showcase list: what the tool does, in user terms, for the README and for
launch posts. **Every feature added or removed must be reflected here in the
same change.** Until the first release this is simply the list of what exists;
after it, add a "since 0.x" note to new entries and move retired ones under
"Removed".

## Splitting

- **Bit-perfect, sample-accurate cuts.** Audio is never decoded; cut points land on the exact sample, even when they are not byte-aligned.
- **DSF and DSDIFF (`.dff`) input**, any channel count (mono, stereo, 5.1 and beyond), any DSD rate. Output is always standards-conforming DSF.
- **Multi-file CUE sheets** (one file per side) and batch mode: add many CUE sheets or a whole folder, review every track, choose an output folder per album.
- **Safe by default.** Source files and CUE sheets are never modified; existing files are never replaced unless you say so; every track is written to a temporary file and moved into place only when complete.
- **Cancel mid-track**: cancelling does not wait for the current track to finish.
- **Real progress**: byte-accurate progress across the whole batch, with live throughput (MB/s) and an estimated time remaining.
- **Same engine for GUI and CLI**, so scripted runs behave exactly like the app.

## Trust: proof that the output is right

- **Built-in verification.** Re-reads every output file and compares it, bit by bit, with the source. Reports identical / differs / missing, with a per-track map of where differences are.
- **Offset detection** when a file is right but shifted, and tag checks.
- **`--verify` / `--verify-only`** on the command line, with a meaningful exit code.
- **Independent test oracle**: the test suite splits synthetic DSF/DFF files with awkward cut points and checks every output bit against a separate reference implementation.
- **Fuzz targets and mutation tests** for the DSF header, ID3 tag reading, DSDIFF header and CUE parsing.
- **Crash hygiene.** Crash reports are written, worker panics become errors, the next launch tells you, and Settings has *Copy diagnostics* and *Open log folder*.
- **Disk checks.** Free space per volume, a write probe, output-path checks, Windows path-length checks, and a live "needs X · Y free" line.

## CUE sheet handling

- **Forgiving `FILE` matching**: case-insensitive, `.wav`/`.flac` → `.dsf`/`.dff` swap, Windows paths reduced to the file name. Substitutions are shown as warnings; different names are never guessed.
- **Locate…** button to relink a missing source, plus a list of nearby DSD files.
- **Encoding detection**: UTF-8, UTF-16 (BOM), Shift-JIS, GBK, Windows-1252 and more, so legacy rips keep their titles.
- **Pregaps (`INDEX 00`)**: append to the previous track or prepend to the next, in Settings or with `--gap`.
- **Hidden track 00**: keep audio before the first track as its own track (`--hidden-track`).
- **Re-read CUE** to apply new CUE settings to an album.

## Metadata editing

- **Album-level and per-track tags**: set a value once for the whole album, override it on any track, and see at a glance which is which.
- **Bulk edit**: tick any number of tracks (Shift for a range) and edit them together; mixed values are flagged.
- **Custom fields**: add your own tags (`MOOD`, `CATALOGNUMBER`, …) for the album or for particular tracks; they are written as standard ID3 `TXXX` frames and can be used in file names.
- **More of the CUE sheet is kept**: `ISRC`, `CATALOG`, `REM COMMENT` and `REM DISCNUMBER` flow into the tags.
- **14 standard fields**: title, artist, album artist, album, composer, conductor, genre, date, disc, grouping, label, copyright, ISRC and comment.
- **Current beside proposed**: the album tab shows what the first source file is tagged with today next to what will be written, with new and changed values highlighted.
- Source-file tags seed blank album fields; **Verify checks every field** of every output file against the plan. See [metadata.md](metadata.md).

## Waveform editor and splitting without a CUE

- **Waveform overview with draggable cut markers**: see the whole side, zoom and pan, drag any cut to the sample you want, double-click to add one, right-click to remove one.
- **Silence detection**: finds the quiet gaps between tracks and cuts there with one click (tunable threshold, minimum gap and minimum track length); Ctrl-drag snaps a marker to the middle of a gap.
- **Cut-in-loud-audio warning**: a marker that lands in loud music is flagged ⚠ before you split.
- **Split a file that has no CUE sheet**: add a `.dsf`/`.dff` directly and cut it by silence or by hand, in the app or with `dsd-workbench file.dsf --auto` / `--at 2:30`.
- **Export CUE sheet**: save any album's cut points and tags as a CUE sheet (`--write-cue` on the CLI). See [splitting-without-a-cue.md](splitting-without-a-cue.md).

## Projects

- **Save and reopen your work**: *Project → Save project…* stores every album in the list with all your edits (tags, overrides, custom fields, cut times, naming, cover, output folder, enabled state) in a small versioned `.dsfproj` file; reopen it from the menu, by dropping it onto the window, or from the command line.
- Safe by design: written atomically, finds a moved CUE sheet by relative path, skips albums whose CUE is gone while loading the rest, and refuses files from a newer version instead of misreading them.
- **Scriptable**: `--save-project FILE` writes a project from the command line, and `dsd-workbench work.dsfproj --out DIR --verify` reproduces a saved split.

## Online metadata (optional)

- **MusicBrainz and Discogs lookup**: fill in track titles, artists, composer, conductor, date, label and catalogue number, in the app or with `--lookup`. Search by what you type, a release link or id, or the album's own artist and title; bracketed notes like `[1st Press US]` are dropped on a retry; the most relevant hit wins, with track count as the tie-break. It is a build-time option that is **off by default**: standard builds contain no network code at all. Only the search text is sent, results can be undone, mismatched editions are flagged by comparing track lengths, and an optional Discogs token never appears in copied diagnostics. See [online-lookup.md](online-lookup.md).
- **`release-lookup` library**: the lookup as a standalone, reusable crate (request builders and parsers, bring your own HTTP client or use the optional `http` feature).

## Tag-only mode

- **Retag without re-splitting**: apply the album's metadata and cover to `.dsf` files that already exist, in the app (*Album & output → Tag existing files*) or with `--tag-only [--in DIR]`. Only the tag at the end of each file is rewritten; the audio is never touched (proven by a byte-for-byte test).
- **A clear result**: *Preview changes* and *Apply tags* both leave a banner with the counts (and a hint about the next step) and a list with one row per track: a status chip (Will change, Tagged, Up to date, No file, Ambiguous, Failed), the track and file name, and which tags change (hover for each old and new value, such as `comment: nothing → 'http://…'`). The card header repeats the last result when it is collapsed, the work runs in the background with a spinner, and the confirmation box clears after a write so a second apply is deliberate.
- Files are matched by the name the tool would give them, then by leading track number (`01 - …`, `1. …`, `Track 03`). Ambiguous matches are never guessed, `--dry-run` previews, and files that are already correct are left alone.

## Cover art

- **Automatic cover detection**: `cover.jpg`, `folder.jpg`, `front.png`, `album.jpg` and similar next to the CUE sheet are found and embedded in every track as a front cover.
- Optionally **copy it into the album folder** as `folder.jpg`, choose a different image per album in the app, or use `--cover FILE` / `--no-cover` / `--copy-cover` on the CLI.
- The image type is judged from its content, not its name; oversized or non-image files are refused before anything is written, and **Verify checks that the cover really landed** in each track.

## Naming and organising output

- **Filename and folder templates**: `{track:02} - {title}`, or a folder per album such as `{albumartist}/{album} ({year})`, with a live preview in Settings and `--name` / `--folder` on the CLI. Any tag, including custom fields (`{tag:MOOD}`), can be used.
- **Safe on every system**: illegal characters, reserved Windows names, trailing dots, over-long names and `..` tricks are handled; non-Latin titles are kept as written. Duplicate targets are caught before anything is written. See [naming.md](naming.md).

## Tag library (`dsf` crate)

- Read and write ID3v2 tags inside DSF files without touching the audio; ID3v2.3 and v2.4.
- Patch-style edits: set or clear text frames, **custom `TXXX` fields**, **comments** and a **front-cover image**; every other frame in the file is preserved.
- **Common track fields built in**: title, artist, album artist, album, composer, conductor, genre, date, disc, grouping, label, copyright, ISRC and comment, with the older frames other software writes (ID3v2.3 dates, `GRP1`) understood, plus `n/total` track positions.
- A corrupt tag is an error, never silently replaced.

## Listening

- **Audition playback** of any cut before splitting, with a spectrum display and a higher-quality audio option.

## Interface

- **Cards and clear grouping**: the album tab is a stack of titled cards (output folder, source files, metadata, cover art, online lookup, CUE sheet, tag existing files) on a raised surface; the Split button is the one filled accent button; toolbar and action bar actions are grouped by purpose.
- **Settings dialog**: a fixed-size centered dialog with a page list on the left (Appearance, Splitting, CUE & file names, Cover art, Audition audio, Online lookup, About & privacy), switches for on/off options and segmented controls for choices.
- **Tables that use the window**: columns start wider on big windows, the last column takes the remaining space, and clicking anywhere on an album row selects it.
- **Undo and redo**: every edit to an album can be undone and redone (Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z, or the toolbar); a burst of typing counts as one step.
- **Shortcuts**: Ctrl+O add CUE files, Ctrl+S save project, Ctrl+Z / Ctrl+Y undo and redo.
- **Fix a wrong track order**: move a track's title and tags up or down between time slots, and renumber 1…N.
- **Accessible colours**: both themes are checked automatically against WCAG AA contrast (4.5:1 for text).
- **Guided first run**: the empty screen shows an animation of what the app does (a DSD stream cut at the track starts, the tracks drifting apart and being checked bit by bit), then walks through add → review → listen → split → verify as five cards, with the shortcuts underneath.
- **Drop target**: dragging files over the window dims it and says what a drop will do; on the empty screen the animation's frame lights up too.
- **Collapsible sections**: the album tab shows its cards (output folder, source files, album metadata, cover art, online lookup, CUE sheet, tag existing files) as an accordion. Each header has a one-line summary, such as the album and artist or the cover file, so you can see the state of a card without opening it. Each card remembers whether it is open. *Settings → Appearance → Collapsible sections* turns it off for plain headings over cards that are always open.
- **Keyboard use**: Tab reaches the switches and the settings list, shows a focus outline, and Space or Enter activates them; they also announce themselves to screen readers.
- **Messages that stay out of the way**: the result of the last action has its own line below the buttons, can be dismissed, and never wraps into the controls.
- **Asks before discarding work**: *Clear all* confirms first, since it drops every album's edits and undo history.
- **Chinese, Japanese and Korean titles display correctly**: the app borrows a CJK font the system already has (Noto Sans CJK, Microsoft YaHei, Yu Gothic, PingFang, …) instead of showing boxes.
- **Errors say what to do next**: DST-compressed files, full drives, missing permissions, existing files, bad cover images and more each come with a plain-language next step, in the app and on the command line.
- Light (paper) and dark themes, persistent settings.
- GUI and a full CLI (see [cli.md](cli.md)).

## Builds

- **Headless build**: audio output is a Cargo feature (`playback`, on by default); build without it for servers and minimal Linux systems. Everything except audition playback works.
- **Release binaries**: a tagged release builds archives (with checksums) for Windows, macOS (Apple silicon and Intel) and Linux.

## Quality bar

- No `unsafe` in the format crates; no panicking `unwrap`/indexing in shipped code; typed errors.
- CI on Linux, Windows and macOS (format, clippy, tests), including a minimum-supported-Rust-version job.
- Offline. No telemetry. Never modifies your sources.

## Removed

- Nothing yet.
