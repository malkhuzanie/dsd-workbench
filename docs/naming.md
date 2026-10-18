# File and folder names

Tracks are written as `<output folder>/<sub-folders>/<file name>.dsf`. Both parts come
from templates, set in **Settings → File names** or with `--name` / `--folder` on the CLI.

| Template | Default | Example result |
|----------|---------|----------------|
| File name | `{track:02} - {title}` | `03 - So What.dsf` |
| Sub-folder | *(none)* | `{albumartist}/{album} ({year})` → `Miles Davis/Kind of Blue (1959)/` |

## Placeholders

| Placeholder | Value |
|-------------|-------|
| `{track}` | Track number. `{track:02}` pads to two digits (up to 9). |
| `{title}` `{artist}` `{albumartist}` `{album}` `{composer}` `{conductor}` `{genre}` `{publisher}` | The tag values, after per-track overrides. |
| `{date}` | The date exactly as tagged. |
| `{year}` | The first four consecutive digits of the date. |
| `{disc}` | The disc number (`1/2` gives `1`). |
| `{tag:NAME}` | A custom field, for example `{tag:MOOD}`. |
| `{{` `}}` | A literal `{` or `}`. |

Names are case-insensitive. A `/` in the **sub-folder** template starts a new folder; a
`/` inside a tag value never does. Parts that come out empty are skipped, so
`{albumartist}/{album}` on an untagged album simply writes into the output folder.

## Keeping names safe

Values (not the literal text of your template) are cleaned:

- **Portable** (default): safe on Windows, macOS, Linux, exFAT and network shares.
  `\ / : * ? " < > |` and control characters become `_`; trailing dots and spaces are
  removed; reserved Windows names (`CON`, `NUL`, `COM1`, …) get a leading `_`.
- **This system only**: only what the current system forbids.

In both modes names are cut to 200 bytes on a character boundary, `.` and `..` can never
escape the output folder, and an empty name becomes `Untitled`.

Two tracks that would get the same path are reported as a problem before anything is
written, so a template without `{track}` and `{title}` cannot silently overwrite files.

An invalid template (unknown placeholder, unmatched brace) is shown as a problem on the
album and refused by the CLI.

Templates apply to CUE sheets added afterwards, or to an album when you press **Re-read CUE**.
