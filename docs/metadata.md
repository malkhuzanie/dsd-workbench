# Editing metadata

Tags are edited at two levels, and a track always shows the result of combining them.

| Level | Where | Meaning |
|-------|-------|---------|
| **Album** | *Album & output* tab → Album metadata | The default for every track. |
| **Track** | *Tracks* tab → select tracks → editor at the bottom | A value that differs from the album default, for just those tracks. |

A track that has no value of its own follows the album: change the album's genre and
every track that has not overridden it changes with it. A track can also be blanked on
purpose, which hides the album value for that track only.

## Fields

Standard fields (each is written to its ID3 frame):

Title, Artist, Album artist, Album, Composer, Conductor, Genre, Date / year, Disc,
Grouping, Label / publisher, Copyright, ISRC, Comment.

Title and ISRC exist only per track. Everything else can be set for the album and
overridden per track.

**Custom fields** are any extra tag you invent (`MOOD`, `CATALOGNUMBER`, `RECORDING
LOCATION`…), stored as ID3 `TXXX` frames. Add one with *Add custom field* (a name and a
value). They can be set for the whole album or for particular tracks, and can be used in
file names with `{tag:NAME}` (see [naming](naming.md)). Clear a custom field's value to
remove it.

## Editing several tracks at once

1. Tick the checkbox at the start of each track row. **Shift-click** extends the range from
   the last ticked row. The checkbox in the header selects or clears every track.
2. The editor below the table shows one row per field. If the selected tracks disagree,
   the field shows *(mixed values)*; typing a value sets it on all of them.
3. Each row says where its value comes from: **track** (an override) or **album**.
   *Use album value* removes the override from the selected tracks.

Edits can be undone with **Reset edits**, which restores what the CUE sheet and source
tag gave.

## What gets written

Every track is written with all standard fields (blank ones are left out) plus every
custom field that applies to it. Track numbers are written as `n/N`; a hidden track
`00` is not counted in `N`. **Verify** re-reads each output file and compares every field
with the plan.
