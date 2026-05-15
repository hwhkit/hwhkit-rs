# hwhkit-rs Testing Strategy

This document describes the testing layers used in `hwhkit-rs`, what each
layer is good for, and how to run them. New contributors should be able
to read this end-to-end and decide where any new test belongs.

## Test Categories

| Layer       | Purpose                                                                 | Where it lives                              | When to run                          |
| ----------- | ----------------------------------------------------------------------- | ------------------------------------------- | ------------------------------------ |
| Unit        | Single function / small module behaviour, deterministic                 | `crates/<crate>/src/**/tests`               | Every commit / CI                    |
| Property    | Invariants over a generated input space (`proptest`)                    | `crates/<crate>/tests/proptest_*.rs`        | Every commit / CI                    |
| Integration | Public-API contracts, multi-crate flows                                 | `crates/<crate>/tests/*.rs`                 | Every commit / CI                    |
| Bench       | Hot-path measurement (`criterion`); regression detection                | `crates/<crate>/benches/*.rs`               | Local + perf-tracking CI             |
| Fuzz        | Coverage-guided random byte exploration (`cargo-fuzz`, libFuzzer)       | `fuzz/fuzz_targets/*.rs`                    | Periodic, before releases            |
| Live       | Talks to a real backend (Postgres / Redis / etc.)                       | gated behind ignore + env-var feature flags | On-demand, in matching infra         |

## Running

### Unit + property + integration

```sh
cargo test --workspace --all-features
```

This runs every crate's unit tests, every `tests/*.rs` integration file
(including the property-based suites), and the doc-tests.

### Benchmarks

```sh
# Compile only (fast; what CI does)
cargo bench --workspace --no-run

# Run all benches (slow; local profiling)
cargo bench --workspace

# Run one specific bench
cargo bench -p hwhkit-scheduler --bench cron_eval
```

Benches live behind `harness = false` so each file has a `criterion_main!`
entry point. We do not check criterion baselines into git; record locally
and compare with `--baseline`.

### Fuzz targets

Fuzzing requires nightly Rust and a one-time install:

```sh
cargo install cargo-fuzz
rustup install nightly
```

The fuzz crate is excluded from the workspace; run fuzzers from the
`fuzz/` directory:

```sh
cd fuzz

# List targets
cargo +nightly fuzz list

# Run a target. -max_total_time bounds wall-clock; otherwise it runs forever.
cargo +nightly fuzz run cron_parse -- -max_total_time=60
cargo +nightly fuzz run config_toml -- -max_total_time=60
cargo +nightly fuzz run jwt_decode -- -max_total_time=60
cargo +nightly fuzz run path_param_extract -- -max_total_time=60
```

Each target's contract is "no panic on any input" — a successful run
produces no crashing seeds in `fuzz/artifacts/`. Crashes reproduce with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<seed>`.

### Live tests

Tests that need a real Postgres / Redis / NATS / etc. live in
`crates/hwhkit-integration-<name>/tests/live.rs` and are marked
`#[ignore]` so the default `cargo test` stays hermetic. They use
[`testcontainers`](https://docs.rs/testcontainers) to spin up the
backend in a real Docker container — no host services required, but
**Docker must be running**.

Run them explicitly:

```sh
# One integration:
cargo test -p hwhkit-integration-postgres -- --ignored
cargo test -p hwhkit-integration-redis    -- --ignored
cargo test -p hwhkit-integration-nats     -- --ignored

# All live tests across the workspace (slow — pulls every image on
# first run):
cargo test --workspace -- --ignored
```

Each `live.rs` covers the same two scenarios for consistency:

1. **Full lifecycle**: container → provider `init` → handle visible
   in `AppContext` → `health_check` passes → real roundtrip
   (`SELECT 1` / `SET+GET` / pub-sub) → `shutdown`.
2. **Unreachable URL**: bind an ephemeral port and drop it (no
   container), assert `init` surfaces a typed
   `Error::Integration { kind: ConnectionRefused | Timeout }`.

The file is intentionally `#[ignore]`d rather than feature-gated so
it still **compiles** on every `cargo build` — that catches the case
where a public type drift in the integration breaks the test long
before someone tries to run it.

## Coverage Expectations

We don't enforce a percentage. We do enforce these principles:

1. **Every public function gets a happy-path unit test.** If you change a
   public signature, the test breaks first.
2. **Anything that parses untrusted input gets a property test or fuzz
   target.** Cron expressions, JWTs, TOML config, request headers — these
   all sit on the trust boundary and benefit from random exploration.
3. **Any concurrency primitive gets a contention test.** `TenantScope`,
   the JWKS single-flight gate, the scheduler's claim race — each has at
   least one test that spawns multiple workers and asserts on the
   resulting state.
4. **Hot paths get a criterion bench.** If a piece of code runs once per
   request, we want a baseline number so future regressions are visible.

## Choosing a Test Layer

- **Pure logic, deterministic input** → unit test next to the code.
- **Pure logic, large input space** (parsers, predicates, derivations)
  → property test with `proptest`.
- **Security-sensitive untrusted parser** → fuzz target on top of the
  property test.
- **Code crossing crate boundaries** → integration test in
  `crates/<entry-crate>/tests/`.
- **Hot path, allocation- or latency-sensitive** → criterion bench.
- **Anything with a real backend** → live test gated behind `#[ignore]`
  + env-var.

## Test Isolation

Several invariants the test suite depends on:

- All temporary directories are created via `tempfile::TempDir`. Never
  use `std::env::temp_dir()` — concurrent tests would race on shared
  paths.
- The tracing subscriber is initialised with `try_init()`, so concurrent
  tests don't fail on a "global subscriber already set" error.
- Tests that mutate process-global state should be marked `#[serial]`
  (see `serial_test`), but as of this writing the test suite has none
  outside of the live-service tier.

## Adding a New Test

```text
1. Decide the layer (use the table above).
2. For property tests, set `cases` to a number that finishes in <2s in
   debug mode. 256 is the default.
3. For benches, mark the file `harness = false` in the parent
   `Cargo.toml` and use `criterion::criterion_main!`.
4. Run `cargo test --workspace --all-features`,
   `cargo clippy --workspace --all-features --all-targets -- -D warnings`,
   and `cargo bench --workspace --no-run` before opening a PR.
```
