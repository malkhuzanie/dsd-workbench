# CLAUDE.md

Rust workspace: `crates/dsf`, `crates/dsdiff`, `crates/release-lookup`, `apps/dsd-workbench` (DSD Workbench: split, tag, verify and look up DSD albums; GUI + CLI). Library packages are published as `dsd-workbench-*` but keep the short lib names (`use dsf::…`).

## Start of every session
1. Read `docs/PLAN.md` (local, git-excluded): it is the source of truth for status and order of work.
2. Read `docs/architecture.md` before touching module boundaries.

## While working
- Follow the plan phases in order unless the owner says otherwise.
- After each completed item: tick it in `docs/PLAN.md`, add a line to its Log, add user-visible changes to `CHANGELOG.md`, and update the relevant `docs/*.md`.
- Showcase: every feature or tool added, changed or removed must be recorded in `docs/FEATURES.md` in the same change (user-facing wording, for the README and launch posts). Removals go under "Removed". Before the first release, `FEATURES.md` and `CHANGELOG.md` only describe what exists: no "new/changed/fixed" labels, no entries about revisions of unreleased behaviour.
- Edit existing files with the Edit tool, not python/sed rewrites (the owner reviews diffs in their IDE).
- Gate before declaring done: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Rust bar: no `unwrap`/`expect`/panicking indexing outside tests, typed errors, documented public API, pure logic separate from I/O/UI, tests for new logic.

## Git
- Never commit unless asked. No commits exist yet.
- Commit style: Linux kernel: imperative subject <= 72 cols, no trailing period, optional body explaining why. Do not mention Claude or AI in messages. Do not add trailers about Claude.
