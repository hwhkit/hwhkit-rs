# Release checklist

The release flow is encoded in [`../../Makefile`](../../Makefile) —
this doc is the human-readable companion that says *when* to run
which target.

## Before tagging a release

```bash
make ci             # fmt-check + clippy -D warnings + workspace test + cargo-deny
make publish-check  # Tier 0 dry-run + Tier 1-3 cargo package --list
```

Both must be green. `make ci` is also what CI runs on every PR.

Optional but recommended:

```bash
make test-live      # all 14 #[ignore]'d integration tests (needs docker)
```

## Updating versions

All crates share `workspace.package.version` in the root `Cargo.toml`.
A version bump is a single-line edit there — no per-crate change
needed because every member uses `version.workspace = true`.

Pre-1.0 versioning: bump the second segment (`0.X.0`) for any
breaking change, the third (`0.6.0` → `0.6.1`) for additive /
bug-fix. SemVer policing happens via `cargo-semver-checks` in the
`semver.yml` workflow (advisory until 1.0).

## CHANGELOG

Every PR that touches public API moves an entry from the bottom of
`CHANGELOG.md` (`## Unreleased`) to a new top section
`## [<version>] — <date>`. The `## Unreleased` header then starts a
fresh block. Use Keep-a-Changelog categories
(Added / Changed / Removed / Fixed / Deprecated).

## Publishing to crates.io

```bash
cargo login                          # one-time
git diff --quiet HEAD --             # working tree must be clean
make publish                         # serial, dep-ordered, 30s index-wait between crates
```

If a single crate fails mid-run:

```bash
make publish-resume RESUME_AT=hwhkit-core    # continues after the last successful crate
```

Total runtime: ~7–10 minutes for the full 14-crate workspace.

## Post-publish

1. `git tag v<version> && git push --tags`.
2. Create a GitHub release pointing at the matching `CHANGELOG.md`
   section.
3. Verify docs.rs builds: visit `https://docs.rs/hwhkit/<version>`
   within ~30 minutes of publish.
4. Check the [crates.io page](https://crates.io/crates/hwhkit) shows
   the README and the correct keyword cluster.

## What this checklist deliberately doesn't track

- Per-feature test coverage targets — the `--all-features` test run is
  the gate; tracking percentages adds noise without preventing bugs.
- Documentation completeness — the docs.rs check after publish catches
  missing `[package.metadata.docs.rs]` config; nothing else needs
  pre-flight verification.
- License files — `cargo-deny` (run by `make ci`) enforces our
  allow-list policy.
