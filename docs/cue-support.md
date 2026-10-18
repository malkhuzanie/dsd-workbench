# CUE support

The parser is line-based and forgiving. Text encoding is detected: a byte-order mark (UTF-8, UTF-16) wins, then valid UTF-8, then a statistical guess (Shift-JIS, GBK, Windows-1252 and so on), so titles from legacy rips survive.

## Understood commands

| Command | Used for |
|---------|----------|
| `FILE "name.dsf" WAVE` (quotes optional) | Starts a source file; following tracks belong to it. |
| `TRACK nn AUDIO` | Starts a track. Non-`AUDIO` tracks are skipped with a warning. |
| `TITLE` | Album title (before the first track) or track title. Missing track title becomes `Track NN`. |
| `PERFORMER` | Album artist or track artist. Tracks inherit the album performer. |
| `SONGWRITER`, `REM COMPOSER` | Composer (album-level is inherited by tracks). |
| `INDEX 01 mm:ss:ff` | Track start. 75 frames per second; `ss` < 60 and `ff` < 75. |
| `INDEX 00 mm:ss:ff` | Start of the pregap before a track (see below). |
| `REM DATE`, `REM GENRE` | Album date and genre. |
| `REM COMMENT`, `REM DISCNUMBER` | Album comment and disc number (`1` or `1/2`). |
| `CATALOG` | Written to a custom `CATALOGNUMBER` field on every track. |
| `ISRC` | The track's ISRC code. |

Everything else (`PREGAP`, `FLAGS`, other `REM`) is ignored.

## Pregaps and hidden tracks

Audio between a track's `INDEX 00` and `INDEX 01` is its pregap. Choose where it goes (Settings → CUE sheets, or `--gap` on the CLI):

- **End of the previous track** (default): each track starts at its `INDEX 01`; the pregap stays at the end of the track before it.
- **Start of the next track**: each track starts at its `INDEX 00` when it has one. A pregap that starts before the previous track began is ignored.

**Hidden track** (Settings, or `--hidden-track`): audio before the first track's start becomes its own track number `00`, named `Hidden Track`, if it lasts at least one second. Only the sheet's first track can have one; lead-ins before later files are not kept. The track-number tag total (`n/N`) counts only the numbered tracks.

Settings apply to sheets added afterwards; **Re-read CUE** on an album applies them to that album and discards its edits.

## Warnings

A track without a usable `INDEX 01`, or with an invalid time, is **skipped** and a warning is shown on the album. A sheet with no usable tracks is rejected.

## Source files

- Each `FILE` name is resolved relative to the CUE sheet's folder. Windows-style paths (`C:\Rips\side a.wav`) reduce to the file name.
- If the exact name is missing, the tool looks in that folder for the same name with different letter case, and for a `.dsf` (then `.dff`) with the same name, so `Side A.wav` finds `side a.dsf`. Such substitutions are shown as a warning on the album. Different names are never guessed.
- If nothing matches, the album lists nearby DSD files and the **Locate…** button next to the missing file lets you pick it.
- Sources may be `.dsf` or `.dff` (DSDIFF), detected by content, not extension. DST-compressed DSDIFF and non-DSD files are reported as unusable sources.
- Output is always `.dsf`, whatever the source.
- A sheet may use several `FILE` entries (sides); tracks are cut from the file they follow.
- A track's length is the next track's start *in the same file*; the last track of each file runs to the end of that file.

## Precision

CUE times are in 1/75 s. At DSD64 (2,822,400 Hz) one frame is exactly 37,632 samples, so frame-based cuts fall on whole samples. In the GUI you can enter finer start times (milliseconds); they are rounded to the nearest sample.

