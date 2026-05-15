# Changelog

All notable changes to this workspace are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project (still pre-1.0) uses informal SemVer: minor bumps may
contain breaking changes until `1.0`.

## Unreleased

### Fixed

- `cargo hwhkit <subcommand>` no longer fails with
  `unrecognized subcommand 'hwhkit'`. cargo prepends the subcommand
  name as the first arg when invoking a `cargo-<name>` binary; the
  CLI now strips that synthetic arg before passing to clap, so both
  `cargo hwhkit init demo` and `cargo-hwhkit init demo` work
  identically. Direct invocation (no `hwhkit` arg) is unchanged.
  Covered by four unit tests in `cargo-hwhkit/src/main.rs`.


### Added — resilience hardening (audit findings F1 / F3 / F6 / F7)

- **`hwhkit_config::ResilienceConfig`** — new shared struct embedded in
  every integration section under a `resilience` sub-key. Fields:
  `connect_timeout_ms` (5000 default), `op_timeout_ms` (5000 default),
  `probe_timeout_ms` (500 default), `shutdown_timeout_ms` (5000 default).
  All fields `#[serde(default)]`, so existing TOML files keep working
  without change.
- **`*Handle::op_timeout() -> Duration`** on every integration handle —
  user code wraps long-running futures with
  `tokio::time::timeout(handle.op_timeout(), my_call)` to enforce the
  configured bound. The integration crate also uses it where the
  underlying SDK exposes a native timeout (sqlx `acquire_timeout`,
  aws-sdk-s3 `TimeoutConfig::operation_timeout`).

### Changed — resilience (the actual bug fixes)

- **F1 / `connect_timeout`** — every provider's `init()` now bounds the
  initial connect handshake. Previously an unreachable backend (typo
  in URL, partial network outage, …) could stall bootstrap for the
  SDK default (mongodb 30s, sqlx 30s, …). Now bounded to
  `connect_timeout_ms`. Unreachable-URL live tests confirm: they return
  in ~5s where they used to hang.
- **F1 / `op_timeout`** — provider smoke-tests (`SELECT 1` / `PING` /
  `admin.ping` / `RETURN 1` / `list_collections` / `flush` /
  `head_bucket`) are now bounded by `op_timeout_ms`. Where the SDK has
  native operation timeouts (aws-sdk-s3, qdrant_client) those are also
  configured from the same value.
- **F3 / probe isolation** — every `HealthCheck::check()` now wraps its
  probe in `probe_timeout_ms` (500ms default). A saturated pool or a
  hung backend can no longer queue the readiness probe behind real
  traffic; the probe fails fast with a precise message
  ("probe exceeded probe_timeout_ms = 500") so the readiness endpoint
  stays responsive under load.
- **F6 / bounded shutdown** — every provider's `shutdown()` is now
  bounded by `shutdown_timeout_ms`. Most prominently:
  `PostgresProvider::shutdown` used to call `PgPool::close().await`
  with no upper bound — a hung transaction held the entire graceful-
  shutdown budget. Now bounded to 5s default, with a warn log if the
  budget is exceeded.
- **F7 / NATS health probe** — `NatsHealthCheck::check()` no longer
  trusts `client.connection_state()` (the local cached view). It now
  issues a real `flush()` roundtrip bounded by `probe_timeout`. A
  zombie process holding a stale socket previously reported Healthy
  until the OS killed the FD; now it correctly fails the probe.

### Added — companion infra

- `[dev-dependencies]` `tokio = { ..., features = ["time"] }` to all 7
  integration crates so `tokio::time::timeout` is available without
  pulling tokio's full feature surface into the lib crate.

### Added — observability (audit F2 + F8)

- **F2 / postgres saturation metrics** — `hwhkit-integration-postgres`
  now spawns a background sampler that emits two gauges every 10 s:
  - `postgres_pool_size{integration="postgres"}` — total connections
    owned by the pool (open + idle + in-use).
  - `postgres_pool_idle{integration="postgres"}` — connections in the
    free list. `pool_size - pool_idle` = in-use, derivable on the
    dashboard.

  The sampler shuts down cleanly when `PgPool::is_closed()` returns
  true (during graceful shutdown). Gauges are emitted via the
  `metrics` crate — calls are no-ops when no recorder is installed,
  so this is safe for binaries that don't enable the `hwhkit/metrics`
  feature.

  Redis saturation metrics (also part of F2) are deferred:
  `redis::aio::ConnectionManager` exposes no pool / inflight
  introspection in 0.27.

- **F8 / NATS JetStream init probe** — `NatsProvider::init` now issues
  a bounded `query_account` call after creating the JetStream
  `Context`. JetStream-disabled servers used to produce opaque
  runtime errors on first use; now a precise warn log fires at
  bootstrap, with a hint about the `--jetstream` flag. The probe is
  advisory — failure logs but does not abort `init`, since some
  deployments only use core NATS pub/sub.

### Not yet addressed (tracked in TODO)

The remaining observability work items are scheduled for the next
batch — they don't affect correctness:

- **F4** — full sqlx pool tuning (`min_connections` / `idle_timeout` /
  `max_lifetime`).
- **F5** — slow-call warn log per integration.
- **F2 (cont.)** — saturation metrics for mongo / nats / qdrant /
  neo4j / s3.

### Added

- `hwhkit::production::server::run_with_listener(built, listener)` — runs a
  `BuiltApplication` on a pre-bound `TcpListener`. Same OOTB wiring as
  `run` (health / version / metrics / middleware bundle / request-id /
  graceful shutdown), but lets callers pick the listener. Unblocks
  ephemeral-port end-to-end tests, systemd socket activation, and
  multi-listener deployments.
- `crates/hwhkit/tests/e2e_serve.rs` — first end-to-end smoke test for
  the bootstrap → serve → graceful-shutdown pipeline. Asserts that
  `/health`, `/health/ready`, `/version`, `/info`, `/metrics`, the
  request-id middleware, and the user-supplied route all work together
  through a real TCP socket, and that `shutdown.cancel()` drains the
  server within the configured budget.
- `doc/2026-05-14-01-integration-resilience-audit.md` — production
  resilience audit of all 7 integration crates. Identifies seven
  cross-cutting gaps (op timeout, saturation metrics, isolated health
  probe, bounded shutdown, slow-call log, pool-leak guard, tunable
  reconnect) and proposes an additive `resilience` config block.
- `crates/hwhkit-integration-{postgres,redis,nats}/tests/live.rs` —
  first live integration tests against real backends via
  `testcontainers`. Gated `#[ignore]` so default `cargo test` stays
  hermetic; run with `cargo test -p <crate> -- --ignored`. Each file
  covers full lifecycle (container → init → handle → health → roundtrip
  → shutdown) plus the unreachable-URL typed-error contract.

### Changed

- `FileDefaultSource` no longer treats `config/default.toml` as
  required. A missing file is logged at `debug` and the loader falls
  back to `AppConfig::default()` for any field the file would have
  supplied. This fixes the `cargo new` → `cargo run` DX (previously
  failed with `required config file not found: …/default.toml`) and
  matches the existing behavior of `FileEnvironmentSource`. Production
  deployments that *want* to require a config file should add an
  explicit check at startup; `validate_strict` continues to gate
  malformed/empty critical fields the same way.

## [0.6.0-alpha.1] — pre-1.0 API stabilization

### Removed (breaking)

- Deleted the legacy v1 surface entirely:
  - `hwhkit::WebServerBuilder`, `hwhkit::WebServer`, `hwhkit::Config`.
  - `hwhkit::middleware` (legacy CORS/JWT/logging/static_files manager).
  - `hwhkit::templates` (depended on legacy config).
  - `hwhkit::middleware::jwt::JwtAuth` / `Claims` (legacy HMAC-only).
- Deleted the `hwhkit-macros` crate (empty `#[main]` / `#[handler]`
  passthroughs that didn't add value).
- Deleted the `hwhkit-transport` crate (gRPC / WebSocket / P2P
  placeholders that echoed payloads). Transport-related config types
  (`TransportConfig`, `GrpcTransportConfig`, …) are also removed —
  re-introduce them per-application when a real transport implementation
  lands.
- Removed feature flags `transport-grpc`, `transport-ws`,
  `transport-p2p`, `config-v2`, `templates`, `macros`.
- Removed `cargo hwhkit init` templates `api-grpc` and `realtime-event`
  (they referenced the deleted `transport-grpc` / `transport-ws` flags
  and produced projects that did not compile). `minimal-api` is the
  only template shipped in 0.6; reach for it and add the bits you need
  by hand.
- Removed bulk `axum::*` / `tokio` / `serde::*` / `tower_http::cors::CorsLayer`
  re-exports from `hwhkit::*`. Depend on those crates directly.
- Removed `IntegrationProvider::feature()` — collapsed into `key()`.
- Removed `JobStore::clone_box` — wrap in `Arc<dyn JobStore>` once at
  construction and clone the `Arc` instead.
- Removed `KNOWN_FEATURES: &[&str]` const slice; replaced by
  `hwhkit_core::known_features()` iterator.
- Removed deprecated `*_v2` aliases from `hwhkit::*`.

### Renamed (breaking)

- `hwhkit::bootstrap_v2` → `hwhkit::bootstrap`.
- `hwhkit::config_v2` → `hwhkit::config`.
- `hwhkit::core_v2` → `hwhkit::core`.
- `hwhkit::observability_v2` → `hwhkit::observability`.
- `hwhkit::*_v2` per-integration aliases → drop the `_v2` suffix.

### Added

- `hwhkit::prelude` — small, curated re-export module. Use
  `use hwhkit::prelude::*;` for the common types.
- `hwhkit_core::error::{Error, IntegrationFailureKind, BoxError}` —
  hybrid error model. `Error::Integration` carries a semantic-category
  enum so callers can decide retry vs. fail-fast without string-matching.
  `IntegrationFailureKind::is_transient()` is the canonical retry hint.
- `hwhkit::production::server::ServeError` — typed error type replacing
  the previous `Result<(), String>`.
- `hwhkit_core::AppContext::insert` now returns the prior value (if
  any), making silent overwrites observable.
- `hwhkit_config::ConfigLoader::with_source<S>` accepts any
  `S: ConfigSource + 'static`. The pre-boxed variant is now
  `with_boxed_source`.
- All major public types now have `#[non_exhaustive]` so future field /
  variant additions are not breaking changes.
- `#[must_use]` on builder methods, `JwtVerifier::from_*`,
  `RuntimeFeatures::enable*`, `RateLimitLayer::*`, `IdempotencyLayer::*`,
  `Scheduler::with_*`, etc.
- `hwhkit::production::idempotency::fingerprint_request` is now `pub`.

### Changed (breaking)

- All integration `*Handle` structs (`PostgresHandle`, `RedisHandle`,
  `MongoDbHandle`, `NatsHandle`, `QdrantHandle`, `Neo4jHandle`,
  `S3Handle`) have private fields. Read them via the accessor methods
  (`handle.pool()`, `handle.client()`, `handle.url()`, …).
- `BuiltApplication` has private fields. Use the accessors:
  `router()`, `into_router()`, `context()`, `config()`, `bootstrap()`,
  `applied_sources()`, `initialized_integrations()`,
  `degraded_integrations()`, `shutdown()`, `health()`.
- `TenantId.0` is now private. Construct via `TenantId::new(...)`,
  read via `TenantId::as_str()`. `Deserialize` rebuilds the value
  through `new` so deserialised ids are still validated.
- All public error enums are `#[non_exhaustive]`. Constructors switched
  to `Error::invalid_config_with_source(...)`,
  `Error::integration(name, kind, source)`,
  `Error::integration_msg(name, kind, msg)`, etc.
- `JwtError::HeaderParse` / `Verify` / `Jwks` are now struct variants
  with a `message: String` and `source: Option<BoxError>`.
- `Scheduler<K, S>` is now `Scheduler<K>`; the store is held as
  `Arc<dyn JobStore>` internally. Calling code unchanged in most cases —
  pass the store value as before.

### Fixed

- Reconciled duplicate `LoggingConfig` / `OtelConfig` definitions: the
  canonical home is `hwhkit-config`. `hwhkit-observability` continues
  to ship its own `LoggingConfig`/`OtelConfig` for now (different
  shape: includes a `format`/`environment` field) but the field schemas
  will converge in a follow-up.

### Tooling

- Workspace `rust-version = "1.76"` (MSRV).
- New CI: stable / MSRV / nightly matrix (`build`, `test`,
  `clippy -D warnings`, `cargo fmt --check`, `cargo-deny check`).
- New semver-checks workflow.
- New workspace `deny.toml` with permissive-license allowlist.

## [0.5.0-alpha.1] — 2026-04

Pre-1.0 hardening; see git history. Tier-2 capabilities (rate limiter,
idempotency, circuit breaker, JWT verifier, scheduler) all reached
production quality during this cycle.

## [0.4.0-alpha.x] — 2026-03

Tier-1 production defaults: `/health`, `/metrics`, `/version`,
graceful shutdown, request-id, middleware bundle.

## [0.3.0-alpha.x] — 2026-02

Introduced `bootstrap_v2` pipeline, `IntegrationProvider` trait, and
the `hwhkit-config` layered loader.

## [0.2.0-alpha.x]

Initial multi-crate workspace split.
