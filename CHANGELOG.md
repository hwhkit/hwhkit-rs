# Changelog

All notable changes to this workspace are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project (still pre-1.0) uses informal SemVer: minor bumps may
contain breaking changes until `1.0`.

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
