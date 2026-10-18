# User guide

Run `cargo run -p dsd-workbench --release` (or the built `dsd-workbench` binary) with no arguments to open the GUI. See [Concepts](concepts.md) for terms.

## The window

| Area | Purpose |
|------|---------|
| **Toolbar** (top) | Add CUE files / folders / audio files, undo and redo, open and save projects, set output for all albums, clear the list, Settings, light/dark. Related actions are grouped; the Split button in the bottom bar is the one filled button. |
| **Albums grid** | One row per album. Drag the divider below it to resize. |
| **Detail card** (centre) | Tracks and settings for the selected album. |
| **Action bar** (bottom) | Run options, Split/Preview, Cancel, progress, log. |

## 1. Add albums

- **Add CUE files…** to pick one or more `.cue` files.
- **Add folder…** to scan a folder recursively (5 levels deep) for `.cue` files.
- **Add audio file…** (or drop a `.dsf` / `.dff`) for a file with no CUE sheet; see [splitting without a CUE](splitting-without-a-cue.md).
- **Drag and drop** files or folders onto the window. A dropped `.dsfproj` opens that project.

If a CUE sheet names `Side A.wav` but the folder holds `side a.dsf`, the tool finds it and tells you it did. If it cannot, **Locate…** next to the missing file lets you pick it.

Duplicates are ignored. A sheet that cannot be read, or has no usable tracks, is reported in the status line and the log; the rest are still added. Source files are checked as soon as an album is added; use **Rescan source files** (Album & output tab) after fixing a missing file.

## 2. Read the albums grid

| Column | Meaning |
|--------|---------|
| ☑ | Include in the run. |
| Album, Artist, Tracks | From the plan. A `•` marks an album you edited. |
| Sources | Number of source files, or how many are unusable. |
| Output folder | Muted = beside CUE; normal = custom. |
| Status | See [run states](concepts.md#run-states). Hover for details. |
| `…` / `×` | Choose output folder / remove from the project. |

Click a row to select it. The grid is locked while a run is active.

## 3. Review and edit tracks

The **Tracks** tab is an editable grid.

| Column | Notes |
|--------|-------|
| ☐ | Tick tracks (Shift extends a range) to edit them together in the panel below the grid. |
| # | Track number; also affects the file name and the `n/total` tag. |
| Title, Artist, Composer | Free text. Amber = differs from the CUE sheet. |
| Start | `m:ss.mmm`, `h:mm:ss.mmm`, or plain seconds. Red while the text is invalid. |
| Length | Computed: next track's start, or end of source. Not editable; change the next track's start instead. |
| Source | Shown only when the album uses several source files. |
| Output file | The target file name. |
| On disk | `new`, `replace` (exists, Overwrite on), `exists` (exists, Overwrite off: the album will fail), or `unreadable`. Hover to see the tags currently on disk. |

**Reset edits** (top right of the card) restores the album to its original plan.

## 4. Album & output tab

- **Output folder:** type a path, **Browse…**, or **Same as CUE**.
- **Source files:** duration, sample rate and channels, or the problem found.
- **Album metadata:** every album-level field and any custom fields, with *Current* (first source file) beside *Proposed*. Tinted: green new, amber changed, red will be cleared. See [metadata](metadata.md).
- **Cover art:** the image found next to the CUE sheet, or one you choose; embed it, copy it as `folder.jpg`, or neither.
- **CUE sheet:** *Export CUE sheet…* saves the current cuts and tags.
- **Tag existing files:** write these tags into `.dsf` files that already exist, without splitting.

The **Waveform** tab shows the audio of a source file with the cut points on it; drag a marker to move a cut, or let the tool find the silent gaps.

To set outputs for every album at once, use **Output for all** in the toolbar:
- **Beside each CUE sheet** resets all albums to the default.
- **Choose a folder…** gives each album `<folder>/<cue name>`, so albums never collide.

## 5. Run

1. Tick the albums to include.
2. Optionally tick **Dry run** (preview only) and/or **Overwrite existing files**.
3. Click **Split N album(s)** (or **Preview**). It is disabled while any selected album **needs attention**.
4. Watch progress, with throughput and time remaining. **Cancel** stops within a moment, even in the middle of a track; the half-written track is removed and finished tracks stay.
5. Open **Show log** for details. The log keeps the last 1000 lines.

Albums run one after another in list order. If one fails, the others continue.

## What "Needs attention" means

An enabled album is blocked when any of these hold:

- no tracks, a track numbered 0, or an invalid start time;
- a track of zero or negative length (the next track does not start after it);
- two tracks in the album, or two albums in the same folder, would write the same file;
- a source file is missing or not a valid DSF;
- a track starts after the end of its source file;
- the output folder is empty, or a target would overwrite a source file.

Untick an album to skip it without fixing it.

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| "Source file … was not found" | The CUE `FILE` name must match a `.dsf` next to the CUE sheet. Fix the file name, then **Rescan source files**. |
| Album fails with "target file(s) already exist" | Tick **Overwrite existing files**, or choose another output folder. |
| Track has no length | The next track's start is not after this one's. Correct the start times. |
| Added a folder and the window paused | Large folders are parsed on the UI thread; wait for it to finish. |
| Edits vanished | Edits are kept only while the app is open. Use **Project → Save project…** (Ctrl+S) to keep them; **Undo** (Ctrl+Z) steps back through edits. |
| Titles show as boxes | The app borrows a system font for Chinese, Japanese and Korean text. If none is installed, install one (for example Noto Sans CJK). |
| "DST-compressed files are not supported" | Convert the file to uncompressed DSD (DSF or DFF) with your ripper or player, then add it again. |


## Settings, safety checks and diagnostics

**Settings** (toolbar) is a dialog with a page list on the left: Appearance, Splitting, CUE & file names, Cover art, Audition audio, Online lookup (online builds) and About & privacy (diagnostics, log folder). Switches and choices apply immediately. They are saved in a small settings file. *Overwrite existing files* is deliberately not saved.

Before a real run the tool estimates the space needed, compares it with the free space per drive, and checks every output folder is writable. A failing check blocks the run; the status line shows “needs X · Y free”. Estimates are approximate (they allow a small margin), and two folders with identical free space are treated as one drive.

If something crashes, a report is written to the log folder (Settings → Open log folder). The next start tells you. **Copy diagnostics** puts a bug-report block on the clipboard. Nothing is sent anywhere.

See [Verification](verification.md) for the Verify tab.

