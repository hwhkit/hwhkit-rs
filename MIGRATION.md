# Migration Guide

This document covers breaking-change migrations between consecutive
HwhKit minor releases. For a full changelog see `CHANGELOG.md`.

## 0.5 → 0.6 (pre-1.0 API stabilization)

This release is intentionally **breaking** — the legacy v1 surface
(`WebServerBuilder`, `WebServer`, `Config`, `JwtAuth`) was removed,
several types had their fields privatized, and the error model was
overhauled. Most fixes are mechanical.

### Module renames

| Old | New |
|---|---|
| `hwhkit::bootstrap_v2::run` | `hwhkit::bootstrap::run` |
| `hwhkit::run_v2` | `hwhkit::run` (or `hwhkit::run_and_serve`) |
| `hwhkit::config_v2::*` | `hwhkit::config::*` (also `hwhkit_config::*` directly) |
| `hwhkit::core_v2::*` | `hwhkit::core::*` (also `hwhkit_core::*` directly) |
| `hwhkit::observability_v2::*` | `hwhkit::observability::*` |
| `hwhkit::postgres_v2`, `hwhkit::redis_v2`, … | `hwhkit::postgres`, `hwhkit::redis`, … |

### Removed

- **Legacy v1 surface gone** — `WebServerBuilder`, `WebServer`,
  `Config`, `middleware::*`, `templates`, `JwtAuth`, `Claims` are
  deleted with no shim. Move to `hwhkit::run_and_serve(MyApp,
  BootstrapConfig::default())` and an `impl Application for MyApp`.
- **`hwhkit-macros` deleted** — the empty `#[main]` / `#[handler]`
  passthroughs are gone. Use `#[tokio::main]` directly.
- **`hwhkit-transport` deleted** — gRPC / WebSocket / P2P were
  placeholder echo-clients. Drop direct deps on the crate; remove the
  `transport-grpc` / `transport-ws` / `transport-p2p` cargo features
  from your service. The `[transport]` block in your config files can
  also be removed (the field is gone from `AppConfig`).
- **`cargo hwhkit init` templates trimmed** — only `minimal-api`
  remains. The `api-grpc` and `realtime-event` templates (which
  referenced the deleted transport features) have been removed; if you
  generated a project with one of them in 0.5 or earlier, drop the
  `transport-grpc` / `transport-ws` features from its `Cargo.toml` and
  remove the placeholder `proto/` / `src/realtime/` scaffolding the
  template wrote.
- **Bulk re-exports gone** — `hwhkit::*` no longer re-exports
  `axum::*`, `tokio`, `serde::Serialize/Deserialize`, or
  `tower_http::cors::CorsLayer`. Add direct deps on those crates and
  import them yourself. Migration:
  ```rust
  // before:
  use hwhkit::{Json, Router, get, Serialize};
  // after:
  use axum::{Json, Router, routing::get};
  use serde::Serialize;
  ```
- **`IntegrationProvider::feature()` removed** — collapsed into `key()`
  (they were always equal in practice). Remove the `feature` method
  from your custom providers.
- **`JobStore::clone_box` removed** — wrap your store in
  `Arc::new(...)` once at construction and clone the `Arc` instead.
  ```rust
  // before:
  let store = MyStore::new();
  let scheduler = Scheduler::<MyJob, _>::new(store, runner);
  // after:
  let scheduler = Scheduler::<MyJob>::new(MyStore::new(), runner);
  // (or, for sharing the store handle elsewhere:)
  let store = std::sync::Arc::new(MyStore::new());
  let scheduler = Scheduler::<MyJob>::from_arc(store.clone(), runner);
  ```
- **`KNOWN_FEATURES` const slice removed** — call
  `hwhkit_core::known_features()` (returns an iterator).

### Privatized fields → use accessors

`*Handle` integration types now expose their internals only through
methods. Replacements:

| Before | After |
|---|---|
| `pg_handle.pool` | `pg_handle.pool()` |
| `pg_handle.url` | `pg_handle.url()` |
| `redis_handle.client` | `redis_handle.client()` |
| `redis_handle.manager` | `redis_handle.manager()` |
| `mongo_handle.client` | `mongo_handle.client()` |
| `mongo_handle.database` (field) | `mongo_handle.database()` |
| `nats_handle.client` | `nats_handle.client()` |
| `nats_handle.jetstream` | `nats_handle.jetstream()` |
| `qdrant_handle.client` (Arc) | `qdrant_handle.client()` (`&Qdrant`) |
| `neo4j_handle.graph` | `neo4j_handle.graph()` |
| `s3_handle.client` | `s3_handle.client()` |
| `s3_handle.bucket` | `s3_handle.bucket()` |

`BuiltApplication` is the same story:

| Before | After |
|---|---|
| `built.router` | `built.router()` (`&Router`) / `built.into_router()` |
| `built.context` | `built.context()` |
| `built.config` | `built.config()` |
| `built.bootstrap` | `built.bootstrap()` |
| `built.applied_sources` | `built.applied_sources()` |
| `built.initialized_integrations` | `built.initialized_integrations()` |
| `built.degraded_integrations` | `built.degraded_integrations()` |
| `built.shutdown` | `built.shutdown()` |
| `built.health` | `built.health()` |

`built.providers` and `built.metrics_handle` are now hidden — they're
implementation details consumed by `production::server::run`.

`TenantId(s)` (tuple-struct construction) → `TenantId::new(s)`.
Reading the inner string: `tenant.as_str()` (unchanged).

### Hybrid error model (Option C)

`hwhkit_core::Error` is now `#[non_exhaustive]` with structured
variants:

```rust
// before:
return Err(Error::Integration {
    integration: "postgres".to_string(),
    reason: format!("connect failed: {e}"),
});

// after:
return Err(Error::integration(
    "postgres",
    IntegrationFailureKind::ConnectionRefused,
    e, // any std::error::Error + Send + Sync + 'static
));
// or, when there's no concrete source error:
return Err(Error::integration_msg(
    "postgres",
    IntegrationFailureKind::InvalidUrl,
    "url must start with postgres://",
));
```

`Error::Bootstrap(String)` and `Error::Config(String)` were replaced
by:

| Before | After |
|---|---|
| `Error::Bootstrap(s)` | `Error::Bootstrap(s)` (still present, but prefer the more specific variants) |
| `Error::Config(s)` | `Error::invalid_config(s)` / `Error::invalid_config_with_source(msg, source)` |
| `Error::FeatureMismatch(s)` | `Error::FeatureMismatch { feature: &'static str }` |

`hwhkit_core::IntegrationFailureKind` exposes a coarse retry/fail-fast
classification:
`InvalidUrl`, `AuthFailed`, `ConnectionRefused`, `Timeout`,
`Misconfigured`, `PermissionDenied`, `Other`. Use
`kind.is_transient()` to check if a retry stands a chance.

`hwhkit_config::Error::Parse(String)` and `Io(String)` are now struct
variants with `message` / `source: Option<BoxError>`. Prefer the
constructor helpers (`Error::parse_with_source`, `Error::io_with_source`)
when you have a real source error to wrap.

`hwhkit_core::jwt::JwtError::HeaderParse` / `Verify` / `Jwks` are also
struct-variant. Most call sites are upstream (inside the verifier) — if
you build them by hand, switch to:

```rust
JwtError::Verify {
    message: "...".to_string(),
    source: Some(Box::new(your_error)),
}
```

### `production::server::run` typed error

The function now returns `Result<(), ServeError>` instead of
`Result<(), String>`. `ServeError` is `#[non_exhaustive]` with
`InvalidAddr`, `Bind { addr, source }`, and `Serve(io::Error)`
variants.

### `AppContext::insert` returns prior value

`AppContext::insert<T>` now returns `Option<Arc<T>>` — the previous
value held under the same type, if any. Existing call sites that
ignored the result keep working (the compiler does not warn on
unused `Option` returns).

### `ConfigLoader::with_source` is generic

```rust
// before:
let loader = ConfigLoader::default()
    .with_source(Box::new(MySource));
// after:
let loader = ConfigLoader::default()
    .with_source(MySource);
// or, if you genuinely have a `Box<dyn ConfigSource>` already:
let loader = ConfigLoader::default()
    .with_boxed_source(boxed);
```

### Prelude

A small curated import lives at `hwhkit::prelude`:

```rust
use hwhkit::prelude::*;
// brings in: run, run_and_serve, BootstrapConfig, Application,
//            AppContext, BuiltApplication, IntegrationProvider,
//            ApiError, ApiResult, Error, Result, IntegrationFailureKind,
//            (and TenantId when the multi-tenant feature is on)
```

### MSRV

Workspace MSRV bumped to **Rust 1.76** (was 1.75 implicitly). CI runs
the matrix on 1.76 / stable / nightly.

## 0.4 → 0.5

### Dependency modernisation

The 0.5 line bumps three transitive dependencies that were previously
pinned at versions emitting `future-incompat` warnings on recent rustc
toolchains. The `Cargo.toml` of every consuming crate is unchanged
unless you explicitly track the same dep version. If you re-export
types from these libraries you may need to bump your own pins to match.

| Crate | 0.4.x pin | 0.5.x pin | Notes |
|---|---|---|---|
| `sqlx` (workspace + `hwhkit-integration-postgres` + `hwhkit-scheduler`) | `0.7` | `0.8` | `PgPool` / `PgPoolOptions` / `Migrator` API used by HwhKit is unchanged. Apps using `sqlx::query!` macros must set `DATABASE_URL` at compile time *or* enable `SQLX_OFFLINE=true` with a checked-in `.sqlx/` directory (sqlx 0.8 made the offline-mode requirement stricter). |
| `redis` (workspace `hwhkit-integration-redis`, `hwhkit-observability` `otel-redis`, `hwhkit/rate-limit`+`idempotency`) | `0.25` | `0.27` | `redis::Client`, `redis::aio::ConnectionManager`, `redis::cmd(...).query_async`, `redis::Script::new(...).invoke_async`, `AsyncCommands::{get,set_ex}` all unchanged at the call sites HwhKit uses. Apps that re-export `redis::*` from their own dependency tree must bump in lockstep. |
| `neo4rs` (`hwhkit-integration-neo4j`) | `0.7` | `0.8` | `Graph::connect`, `ConfigBuilder`, `Graph::run(query(...))` unchanged. The 0.8 Bolt parser is a clean drop-in. |

There are no signature changes on the `*Handle::pool()` / `manager()` /
`graph()` accessors. Existing application code that obtained a `PgPool`,
`ConnectionManager`, or `Arc<Graph>` from a `*Handle` continues to work
unchanged.

### `future-incompat` warnings cleared

`cargo build --workspace --all-features` is silent again on stable Rust
1.85+. If your CI was previously gating on
`cargo report future-incompatibilities`, the 0.4 → 0.5 jump removes the
last reported entries (redis 0.25 / sqlx-postgres 0.7).

### Path-dependency version pins

Every internal `version = "0.4.0-alpha.1"` pin was bumped to
`0.5.0-alpha.1`. Downstream consumers using a tip-of-tree `path = "..."`
override do not need to take any action; consumers tracking the
published versions must update both `hwhkit` and any directly-pulled
integration crates (e.g. `hwhkit-integration-postgres`).

## 0.3 → 0.4

### `IntegrationProvider::shutdown`

The trait now has a default-implemented `shutdown(&self, &AppContext) ->
Result<()>` hook. Existing providers compile unchanged; if you wrote a
custom provider that owns connection pools, override `shutdown` to drain
gracefully. The Tier-1 server (`production::server::run`) now calls each
provider in *reverse* of the init order during shutdown.

```rust
#[async_trait]
impl IntegrationProvider for MyProvider {
    // … key/feature/enabled/init unchanged …
    async fn shutdown(&self, ctx: &AppContext) -> Result<()> {
        if let Some(handle) = ctx.get::<MyHandle>() {
            handle.close().await;
        }
        Ok(())
    }
}
```

### `RuntimeFeatures` shape

The struct of bools became a `BTreeSet<&'static str>` wrapper. Replace:

```rust
// 0.3
RuntimeFeatures { postgres: true, redis: true, ..Default::default() }
```

with:

```rust
// 0.4
RuntimeFeatures::new().enable("postgres").enable("redis")
```

Use `RuntimeFeatures::contains(name)` to query. The canonical feature
name list is exported as `hwhkit_core::KNOWN_FEATURES`.

### `RemoteConfigProvider::fetch_patch` is now async

`ConfigSource::load`, `RemoteConfigProvider::fetch_patch`, and
`ConfigLoader::load` are all `async fn` (via `#[async_trait]`). Update
your custom remote providers:

```rust
// 0.3
impl RemoteConfigProvider for ConsulProvider {
    fn fetch_patch(&self, b: &BootstrapConfig) -> Result<ConfigPatch> { … }
}

// 0.4
#[async_trait]
impl RemoteConfigProvider for ConsulProvider {
    async fn fetch_patch(&self, b: &BootstrapConfig) -> Result<ConfigPatch> { … }
}
```

`ConfigLoader::load` callers must `.await` the result.

### `BuiltApplication` has new fields

`providers: Vec<Arc<dyn IntegrationProvider>>` and
`metrics_handle: Option<MetricsHandle>` are now part of the struct. Code
that constructs a `BuiltApplication` by hand (rare) must populate them.
Code that destructures the struct should use `..` to remain
forward-compatible.

`#[must_use]` was added to the type. Bare `let _built = bootstrap(...).await?;`
now warns; either serve the result via `production::server::run` or
suppress the lint with a `_` binding plus `let _: () = ()` after, or use
`drop(built)` to make discard explicit.

### Multi-tenant primitives (new feature, default-on)

`hwhkit_core::TenantId` / `TenantScope<T>` and
`hwhkit::production::tenant::TenantExtractorLayer` ship under the
`multi-tenant` feature, default-on. Disable with
`default-features = false` if you don't need them. The header used by
the extractor is **untrusted by default** — pair it with JWT/mTLS auth.

### Graceful-shutdown semantics fix

Previously the server called `shutdown.cancel()` then *kept accepting
new requests* for `max_drain_secs` seconds. The 0.4 implementation
resolves the `with_graceful_shutdown` future immediately on cancellation
and bounds the inflight drain with `tokio::time::timeout(drain, …)`. If
your service depended on the (incorrect) old behaviour, raise
`max_drain_secs` to compensate.

### Prometheus `path` label cardinality

The HTTP RED middleware now reads `axum::extract::MatchedPath` instead
of `req.uri().path()`. This caps the `path` label cardinality to your
*route table*, not the request URL space. Existing dashboards may need
to re-query against the matched-path values (e.g. `/users/:id` rather
than `/users/42`).

## 0.2 → 0.3

The 0.2 → 0.3 jump introduced the v2 bootstrap pipeline
(`run_v2` / `run_and_serve`) alongside the legacy `WebServerBuilder`.
Both APIs ship in 0.3 and 0.4; the v2 pipeline is the recommended path.
No code change is required to upgrade — just add the v2 entry point
when you're ready.
