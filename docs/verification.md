# Verification

Verification reads each output file back and compares it with the source audio, bit by bit. It never writes anything, and it works on files made by any earlier run.

## What is checked

| Check | Meaning |
|-------|---------|
| **Audio** | Every sample of every channel in the planned range equals the file's sample. The result is *Identical* or *Differs*. |
| **Header** | Sample rate, channel count, length, recorded file size and audio chunk size. |
| **Tags** | Title, artist, composer, album, album artist, genre, date and track number match the plan. |
| **Presence** | *Missing* (no file at the target path) or *Unreadable* (not a valid DSD file). |

Header and tag findings are shown as **notes**; they do not change an *Identical* audio result.

## Running it

- **After a split** (on by default; Settings → Splitting).
- **Verify files now** on an album's Verify tab, or **Verify N album(s)** in the action bar. Use this for files you split earlier.
- CLI: `--verify` after splitting, `--verify-only` for existing files.

Verification compares against the *current plan*: if you edited start times after splitting, expect differences. A report marked stale means the plan or output folder changed since it was made.

## Reading the results

- **Album map:** one lane per source file; segments are tracks, coloured by result. Click to inspect.
- **Integrity map:** per channel, the track is divided into 400 slices; green is identical, red has differing bits (brighter means more). Hover for the time and bit count.
- **Level envelope:** the file's low-band level (a cheap estimate from bit density, not a true waveform). Differing slices are red.
- **Offset detection:** if the first samples differ, the tool searches ±65,536 samples around the planned start (about 1.5 s at DSD64, channel 0 only) for where the file's audio begins and reports the shift in samples and milliseconds. Repetitive audio such as silence can match several places; the closest is reported and marked ambiguous.

## Limits

- Bytes after the last sample in the final block (padding) are not inspected.
- Verification reads the source and output fully, so it is limited by disk speed.
- DSF and DSDIFF sources are supported; the output is always DSF.

