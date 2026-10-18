# dsf

Read and write DSF (DSD Stream File) headers and ID3v2 metadata, without touching the audio.

Published as `dsd-workbench-dsf` (the plain name `dsf` is taken on crates.io); the library is still imported as `dsf`:

```toml
dsf = { package = "dsd-workbench-dsf", version = "0.1" }
```

## Features

- Parse and validate the 92-byte DSF header; write headers for new files.
- Read, edit, replace or remove the ID3v2 tag. Editing keeps unrelated frames (such as cover art) and fixes the header's file size and metadata pointer.
- Works on `File` or in-memory `Cursor<Vec<u8>>`.
- Typed errors, no panics, `#![forbid(unsafe_code)]`.

## Example

```rust
use dsf::{FrameId, TagEdit, WriteOptions};

fn main() -> dsf::Result<()> {
    let edit = TagEdit::new()
        .set(FrameId::TITLE, "Allemande")
        .set(FrameId::COMPOSER, "J.S. Bach")
        .clear(FrameId::GENRE);
    dsf::update_path("track.dsf", &edit, &WriteOptions::default())?;

    if let Some(tag) = dsf::read_path("track.dsf")? {
        println!("{} tags", tag.frames().count());
    }
    Ok(())
}
```

## Common fields

`Field` names the fields an album track usually has (title, artist, album artist, album,
composer, conductor, genre, date, disc, grouping, label, copyright, ISRC, comment) and knows
which ID3 frame holds each, including the older frames other software writes: a date in
`TYER` (ID3v2.3) and a grouping in `GRP1` are found too.

```rust
use dsf::{Field, Track};

let tag = dsf::read_path("track.dsf")?.unwrap_or_default();
let date = Field::Date.read(&tag);              // TDRC, else TYER
let track = dsf::read_track(&tag);              // Some(Track { number: 3, total: Some(12) })

let edit = dsf::TagEdit::new()
    .set_field(Field::Composer, "J.S. Bach")
    .set_track(Track { number: 3, total: Some(12) });
```

`dsf::text_frames` lists every text frame and `dsf::custom_fields` the user-defined (`TXXX`) ones.

## API at a glance

| Item | Purpose |
|------|---------|
| `DsfHeader` | `read`, `write`, `audio_end`; `AUDIO_OFFSET` = 92. |
| `read_tag`, `read_path` | Returns `Option<Tag>` (`None` = no tag). |
| `update_tag`, `update_path` | Apply a `TagEdit`, creating the tag if needed. |
| `write_tag`, `remove_tag` | Replace the whole tag / delete it. |
| `TagEdit`, `FrameId` | Per-frame set/clear for text frames, plus `set_user_text` (`TXXX`), `set_comment`, `set_cover`. Blank values clear. |
| `Field`, `Target`, `Track` | Common track fields, the frame each lives in, and `n/total` track positions. |
| `text`, `text_frames`, `custom_fields`, `read_track` | Read helpers for any `Tag`. |
| `WriteOptions` | Choose the ID3v2 version (default: keep, or v2.4 for new tags). |
| `Storage` | Sealed trait for seekable, resizable stores. |
| `Error` | `Io`, `BadSignature`, `Malformed`, `Id3`, `InvalidFrameId`. |

## Guarantees and limits

- Audio bytes are never rewritten when editing tags.
- A corrupt existing tag returns an error rather than being replaced.
- Tag writes are not crash-atomic; work on a copy if that matters.
- Only DSF is supported (not DSDIFF `.dff`).
- Layout details: [DSF format](../../docs/dsf-format.md).

## License

MIT.

