# Development

## Commands

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p dsd-workbench -- album.cue --dry-run
```

Requires Rust 1.88+ (edition 2024).

## Conventions

- No `unwrap`/`expect`/indexing that can panic outside tests; return typed errors.
- Pure logic (parsing, planning, arithmetic) stays separate from I/O and UI, and gets unit tests.
- Every new validation rule goes in the layer where its data lives (see [architecture](architecture.md#validation-layers)) and is surfaced identically in GUI and CLI.
- Public items in `dsf` are documented; changes to its public API need a changelog entry.

## Adding things

| Task | Touch |
|------|-------|
| New tag field | `plan` (struct, `tag_edit`, `from_cue`), `cue` if parsed, `snapshot::TagSnapshot`, UI grid/form. |
| New CUE command | `cue.rs` + test, then `cue-support.md`. |
| New run option | `engine::SplitEngine` builder, `cli`, `ui/mod.rs` action bar, `docs/cli.md` and `user-guide.md`. |

## Looking at the UI without a window

`apps/dsd-workbench/src/ui/snapshot.rs` holds `#[ignore]`d tests that run egui for a frame, rasterise it in software and write PPM pictures (they assert nothing). Use them after any layout change:

```sh
UI_SNAPSHOT_DIR=/tmp/ui cargo test -p dsd-workbench snapshot -- --ignored
magick /tmp/ui/album-accordion.ppm /tmp/ui/album-accordion.png
```

`snapshot_welcome` renders the first-run screen at several moments of its animation and two window sizes; `snapshot_album` renders the Tracks and Album & output tabs with the sections collapsible and plain; `snapshot_tag_existing` renders the Tag existing files card after a preview and after applying.

## Publishing the library crates

The packages are `dsd-workbench-dsf`, `dsd-workbench-dsdiff` and `dsd-workbench-release-lookup` (the short names are taken or ambiguous on crates.io); each keeps its short `[lib] name`.

1. Check `Cargo.toml` metadata and `repository`.
2. Add `CHANGELOG.md` and license files; test with real DSD64/128, mono/stereo/multichannel files and ID3 v2.3/v2.4 tags; check output in target players.
3. `cargo publish --dry-run`; run `cargo-semver-checks` on later releases.
4. Add a crates.io API token as the repository secret `CARGO_REGISTRY_TOKEN`. Pushing a tag `vX.Y.Z` (which must equal the three crate versions) builds the binaries and, once they succeed, the `publish-crates` job in `.github/workflows/release.yml` publishes the three crates. A published version cannot be replaced, so dry-run first.

Consider moving the slicer into the crate behind a `split` feature.

## Known limitations

- Adding many CUE files parses on the UI thread.
- DST-compressed DSDIFF is detected and refused, not decoded.
- Tags and cover art embedded in a *source* file are not carried to the tracks (album fields are seeded from the source tag; a cover must be a separate image file).
- Last-block padding is zero, per the DSF specification.
- Playback is audition quality (FIR conversion), stereo only.
- Verification's level envelope is an estimate; free-space grouping is a heuristic.
- The GUI is covered by compilation and clippy only; its logic (plan operations, selection, history, geometry, templates) is unit-tested, but layout and interaction need a manual click-through.

## Roadmap

See the local plan file for the current status. Open themes: native DST decoding, online metadata lookup, packaging and releases.

## Testing

- `cargo test --workspace` runs unit tests and the end-to-end tests in `apps/dsd-workbench/tests/`, which drive the real binary on generated DSF/DSDIFF files and check every output bit against an independent reference.
- `crates/*/fuzz` hold `cargo-fuzz` targets (nightly): `cargo +nightly fuzz run header` in `crates/dsf`, `read_info` in `crates/dsdiff`.
- On a machine with several Rust installs, make sure `cargo` and `clippy-driver` come from the same toolchain.

