# Concepts

The vocabulary used by the GUI, the CLI and these docs.

## Terms

| Term | Meaning |
|------|---------|
| **DSF file** | A DSD audio file. In this tool, usually one whole LP side or disc. |
| **CUE sheet** | A text file listing tracks and their start times within one or more DSF files. |
| **Source file** | A DSF or DSDIFF (`.dff`) file named by a `FILE` line in a CUE sheet, resolved relative to the CUE sheet's folder. Never modified. Output tracks are always DSF. |
| **Project** | The list of albums currently open in the app. It can be saved to a `.dsfproj` file (Project → Save project…) and reopened later with all edits. |
| **Album** | One project entry, created from one CUE sheet. Holds its plan, output folder, and run state. A CUE sheet can appear in a project only once. |
| **Plan** | The editable description of what will be written for an album: album metadata plus each track's number, title, artist, composer and start time. |
| **Original** | The plan as first derived from the CUE sheet (and the source file's tags). Used to highlight edits and for **Reset edits**. |
| **Track** | One output file. Its **start time** comes from the plan; its **length** runs to the next track's start in the same source file, or to the end of the source file. |
| **Output folder** | Where an album's tracks are written. Default **Beside CUE** (the CUE sheet's own folder), or a custom folder per album. |
| **Target file** | An output path built from the file-name template (default `<output folder>/NN - Title.dsf`). Unsafe characters become `_`; other punctuation and non-Latin text are kept. See [naming](naming.md). |
| **Run** | One execution of the enabled albums, in list order, in the background. Either a real run or a **dry run**. |
| **Dry run** | A run that reports what would happen and writes nothing. |
| **Overwrite** | A run option. Off by default: an album whose target files already exist fails instead of replacing them. |
| **Needs attention** | An album that fails validation. A run cannot start while any enabled album needs attention. |
| **Current / Proposed** | *Current* is what is on disk now; *Proposed* is what the plan will write. Differences are tinted. |
| **Sample-accurate** | Each track starts and ends on the exact DSD sample the CUE sheet specifies, with neighbouring tracks tiling without gap or overlap. |

## Lifecycle

1. **Add** CUE sheets. Each becomes an album; sources are checked.
2. **Review** start times, lengths, tags and output folders.
3. **Edit** anything in the plan. Edits live only in the app.
4. **Run** (preview first if you like).
5. **Verify** with the log and each album's status.

## Guarantees

- Nothing is written until you start a real run.
- CUE sheets and source files are never modified.
- Edits never change the CUE sheet.
- Existing files are replaced only with **Overwrite** on.
- A failed or cancelled track leaves no partial file: audio is written to `<name>.dsf.part` and renamed on success.
- Audio is copied bit for bit; it is never decoded or re-modulated.

## Run states

| State | Meaning |
|-------|---------|
| Idle / Ready | Not run yet and valid. |
| Skipped | Unticked, so excluded from runs. |
| Needs attention | Validation failed; hover for reasons. |
| Queued | Waiting its turn in the current run. |
| Running | Being split now. |
| Done | Split completed. |
| Previewed | Dry run completed. |
| Failed | Stopped with an error (hover for the message). Tracks already written stay. Other albums continue. |
| Cancelled | You cancelled during this album. Finished tracks stay. |

## What gets written to each track

ID3v2 tags (v2.4): the 14 standard fields (title, artist, album artist, album, composer, conductor, genre, date, disc, grouping, label, copyright, ISRC, comment), the track number as `n/total`, any custom fields, and the album's cover image when one is chosen. Blank fields are omitted. Other tags from the source file are **not** carried over. See [metadata](metadata.md).

Album fields left blank by the CUE sheet are pre-filled from the first source file's ID3 tag. Album artist defaults to the CUE album `PERFORMER`, else the common track artist, else "Various Artists".

## DSDIFF (`.dff`) sources

Read through the [`dsdiff`](../crates/dsdiff/README.md) crate. Big-endian IFF-style chunks (`FRM8` → `PROP` with `FS  `, `CHNL`, `CMPR`; then `DSD `), 64-bit sizes, chunks padded to even length. Audio is interleaved one byte per channel (L, R, L, R …) with the **most significant bit first in time**, the opposite of DSF's usual LSB-first. The splitter reverses the bits of each byte so output DSF files are always LSB-first. DST-compressed files are rejected. DSDIFF's own ID3 and marker chunks are ignored.

