# CLI reference

```
dsd-workbench <album.cue>... [--out DIR] [--dry-run] [--overwrite]
```

Any argument selects CLI mode; with no arguments the GUI opens.

| Option | Effect |
|--------|--------|
| `<album.cue>...` | One or more CUE sheets. |
| `--out DIR` | One album: write into `DIR`. Several albums: write into `DIR/<cue name>`. Default: beside each CUE sheet. |
| `--dry-run` | Print what would happen; write nothing. |
| `--overwrite` | Replace existing output files. Without it, an album with existing targets fails. |
| `--verify` | After splitting, compare every track with the source, bit by bit. |
| `--verify-only` | Do not split; verify files that already exist. |
| `--gap append\|prepend` | Where pregap audio (`INDEX 00`) goes: the end of the previous track (default) or the start of the next. |
| `--hidden-track` | Keep audio before the first track as track `00`. |
| `--name TEMPLATE` | File name template, default `{track:02} - {title}`. See [naming](naming.md). |
| `--folder TEMPLATE` | Sub-folder template below the output folder, for example `{albumartist}/{album} ({year})`. |
| `<file.dsf>`, `<file.dff>` | An audio file with no CUE sheet. See [splitting without a CUE](splitting-without-a-cue.md) for `--auto`, `--at`, `--silence-db`, `--min-silence`, `--min-track`, `--cut-at` and `--write-cue`. |
| `<project.dsfproj>` | Load a saved project (its albums keep their edits). Can be mixed with CUE sheets. |
| `--save-project FILE` | Write the loaded albums, with all options applied, to a project file before running. Combine with `--dry-run` to only create the file. |
| `--lookup` | Fill titles and album details from MusicBrainz or Discogs (builds with `--features online` only). See [online-lookup](online-lookup.md). |
| `--lookup-query TEXT` | Search for `TEXT` instead of the album's artist and title, or give a release link or id for that exact release. Implies `--lookup`; one album. |
| `--lookup-source S` | `musicbrainz` (default) or `discogs`. |
| `--discogs-token T` | Discogs personal access token (optional; `DISCOGS_TOKEN` also works). |
| `--tag-only` | Do not split: write the CUE sheet's tags (and cover) into existing `.dsf` files, matched by planned name or leading track number. Prints one line per track; exit code 1 if any track has no file, several candidates, or a failed write. Honours `--dry-run`. |
| `--in DIR` | With `--tag-only`: the folder that holds the files (default: the output folder). One CUE sheet only. |
| `--cover FILE` | Embed this JPEG/PNG in every track. Default: `cover`, `folder`, `front`, `album` or `albumart` (`.jpg`/`.jpeg`/`.png`) next to the CUE sheet. |
| `--no-cover` | Do not embed or copy any cover art. |
| `--copy-cover` | Also copy the cover into the album folder as `folder.jpg`/`folder.png` (an existing file is kept unless `--overwrite`). |
| `--native-names` | Only avoid characters this system forbids (default: names are safe everywhere). |
| `-h`, `--help` | Usage. |

A bare non-`.cue` argument is treated as the output folder (legacy form). Unknown `--options` are an error.

## Behaviour

1. Every CUE sheet is loaded; a load failure prints a message and exits with code 1 before anything is written.
2. All albums are validated (the same checks as the GUI's "Needs attention"). Problems are printed per album and the exit code is 1.
3. Albums run in order. A failed album prints `Execution failure (<album>): ...` and the run continues.
4. Exit code is `0` only if every album succeeded.

## Examples

```sh
dsd-workbench side_a.cue                      # split beside the CUE sheet
dsd-workbench a.cue b.cue --out ~/Music/DSD   # ~/Music/DSD/a and ~/Music/DSD/b
dsd-workbench album.cue --dry-run             # preview
dsd-workbench album.cue --out out --overwrite
```

Track edits are GUI-only; the CLI uses the CUE sheet's values (plus tags seeded from the source file).

