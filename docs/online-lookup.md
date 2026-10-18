# Online lookup (MusicBrainz and Discogs)

DSD Workbench is offline by default. A build made with the **`online`** feature can fill in
track titles and album details from [MusicBrainz](https://musicbrainz.org) (open data, CC0)
or [Discogs](https://www.discogs.com).

```sh
cargo build --release -p dsd-workbench --features online
```

The default and release builds contain **no HTTP client at all**, so "offline, no
telemetry" stays literally true unless you build the online variant yourself.

The lookup itself lives in the reusable [`release-lookup`](../crates/release-lookup) crate,
which knows nothing about DSD or this app.

## What is sent

Only the text you searched for (by default the album's artist and title), or the release id
you give, to the chosen service, followed by a request for the chosen release. Nothing else:
no file names, no paths, no audio. Requests are paced as each service asks. If you set a
Discogs token it is sent to Discogs only.

## Searching

The box in *Album & output → Online lookup* is what is searched:

- **Empty**: the album's artist and title.
- **Text** such as `kind of blue miles davis`: searched as typed.
- **A release link or id** (a MusicBrainz UUID or `musicbrainz.org/release/…`; a number or
  `discogs.com/release/…`): that exact release is fetched.

If a search finds nothing and the title contains notes in brackets, for example
`Blood On The Tracks [1st Press US]`, it is tried again without them.

When several releases match, relevance comes first; among the best matches the one whose
track count equals yours is preferred.

## What it fills in

| From the release | Goes to |
|------------------|---------|
| Track titles (matched by track number) | each track's title |
| Track artist, when it differs from the album artist | that track's artist |
| Track composer / conductor (Discogs credits, MusicBrainz where given) | that track's fields |
| Album title, album artist, date | album fields (the artist is also the default track artist) |
| Label | Label / publisher |
| Catalogue number | custom field `CATALOGNUMBER` |

Existing values are replaced. Everything can be undone with **Undo** (Ctrl+Z).

## Telling you when it looks wrong

Each matched track's length is compared with your plan; differences over 15 seconds are
reported ("Track 3 is 4:10 on the release but 6:02 here; this may be a different edition"),
as are differing track counts.

## Discogs token

Discogs works without one but allows fewer requests per minute. Create a personal access
token under Discogs → Settings → Developers and paste it in Settings, pass `--discogs-token`,
or set `DISCOGS_TOKEN`. The settings file stores it as plain text; it is left out of copied
diagnostics.

## Use it

**In the app**: *Album & output → Online lookup*, pick a source, optionally type a query,
then **Look up**.

**On the command line:**

```sh
dsd-workbench album.cue --lookup                                  # artist and title of the album
dsd-workbench album.cue --lookup --lookup-source discogs
dsd-workbench album.cue --lookup-query "kind of blue miles davis"  # implies --lookup
dsd-workbench album.cue --lookup-query https://musicbrainz.org/release/<uuid>
dsd-workbench album.cue --lookup --write-cue fixed.cue --dry-run  # fetch, then save a CUE sheet
```

In a build without the feature, `--lookup` explains how to enable it.
