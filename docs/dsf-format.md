# DSF format, tagging and slicing

Little-endian throughout.

## Layout

| Offset | Size | Field |
|-------:|-----:|-------|
| 0 | 4 | `"DSD "` |
| 4 | 8 | DSD chunk size (28) |
| 12 | 8 | **Total file size** |
| 20 | 8 | **Metadata pointer** (offset of the ID3v2 tag; 0 = none) |
| 28 | 4 | `"fmt "` |
| 32 | 8 | fmt chunk size (52) |
| 40 | 4 | Version (1) |
| 44 | 4 | Format id (0 = raw DSD) |
| 48 | 4 | Channel type |
| 52 | 4 | Channel count |
| 56 | 4 | Sampling frequency (Hz) |
| 60 | 4 | Bits per sample: **1 = LSB first, 8 = MSB first** |
| 64 | 8 | **Sample count** per channel |
| 72 | 4 | Block size per channel (4096) |
| 76 | 4 | Reserved |
| 80 | 4 | `"data"` |
| 84 | 8 | Data chunk size (includes this 12-byte header) |
| 92 | … | Audio blocks |
| *pointer* | … | ID3v2 tag (normally the end of the file) |

`dsf::AUDIO_OFFSET` is 92.

## Audio blocks

Audio is stored as blocks of `block_size` bytes per channel, interleaved: block 0 of ch 0, block 0 of ch 1, … then block 1 of ch 0, and so on. Each byte holds eight 1-bit samples. Bit order within a byte follows *bits per sample*. The final block is padded; players stop at *sample count*.

## Tagging

`dsf::update_tag` never rewrites audio:

1. Read the header and the existing tag (a corrupt tag is an error, never silently replaced).
2. Apply the edit to the tag (unlisted frames, such as cover art, are kept).
3. Serialise the tag in memory, write it at the end of the audio, truncate the file after it.
4. Update *total file size* and *metadata pointer* in one 16-byte write.

Not transactional: a crash between steps 3 and 4 can leave header and tail disagreeing. The splitter avoids the risk by tagging the `.part` file before renaming it.

## Sample-accurate slicing

For a track starting at `t` seconds with an end at `e` seconds (or the end of the source):

1. `start = round(t × rate)`, `end = round(e × rate)`, `count = end − start`. Both ends come from absolute times, so neighbouring tracks tile exactly.
2. In each channel's byte stream, the cut begins at byte `start / 8`, bit `start % 8`.
3. Output blocks are built by shifting the stream by that bit offset (using the correct bit order) and re-packing into fresh blocks; `ceil(count / (block_size × 8))` blocks per channel.
4. Samples beyond `count` in the last block are zeroed.
5. The new header gets `sample count = count` and sizes derived from the block count.

Audio samples are copied unchanged: only their packing moves. Memory use is two source blocks plus one output block, regardless of track length.

## DSDIFF (`.dff`) sources

Read through the [`dsdiff`](../crates/dsdiff/README.md) crate. Big-endian IFF-style chunks (`FRM8` → `PROP` with `FS  `, `CHNL`, `CMPR`; then `DSD `), 64-bit sizes, chunks padded to even length. Audio is interleaved one byte per channel (L, R, L, R …) with the **most significant bit first in time**, the opposite of DSF's usual LSB-first. The splitter reverses the bits of each byte so output DSF files are always LSB-first. DST-compressed files are rejected. DSDIFF's own ID3 and marker chunks are ignored.

