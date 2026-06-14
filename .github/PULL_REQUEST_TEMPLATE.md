## Summary

<1-3 bullets of what & why>

## Why

<problem this solves; link issue / discussion if any>

## Wire shape

<for API changes: a short code snippet showing the new public surface>

## Test plan

- [ ] `cargo test --workspace` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo fmt --all -- --check` clean
- [ ] Doc comments on new public items; doctest if non-trivial
- [ ] CHANGELOG entry under `## [Unreleased]`

## Risk

- [ ] Additive only (no existing callers affected)
- [ ] Behaviour change (call it out; downstream impact?)
- [ ] Breaking change (pre-1.0 SemVer minor bump?)

## Out of scope

<things deliberately left for follow-ups, with issue links if open>
