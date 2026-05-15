# Integration Resilience Audit (2026-05-14)

Audit of the seven `hwhkit-integration-*` crates for production
connection resilience. Scope: pool sizing, connect / op timeouts, idle
& max-lifetime, reconnect, pool-leak guards, slow-call logging,
saturation metrics, health-check isolation, bounded shutdown.

This document is the deliverable for **TODO P0 #5**. It does **not**
land any code changes — those are broken out into follow-up TODOs at
the end (each scoped tight enough to land in one PR).

## TL;DR

Every integration crate is a **thin happy-path wrapper** around its
SDK: open client → smoke-test (`SELECT 1` / `PING` / `head_bucket` /
…) → register health check → done. None of them expose timeouts,
none emit saturation metrics, none log slow calls, none isolate the
health probe from the hot path, and only Postgres lets you size the
pool from config.

This is fine for development. It is **not** fine for production
traffic — a stuck DB will park HTTP handlers indefinitely, a saturated
pool will fail the readiness probe before it fails real requests, and
operators have no visibility into pool utilization until something
breaks.

## Per-integration matrix

Legend: ✅ exposed via typed config · ⚠️ uses SDK default (often too
long or unbounded) · ❌ not configurable at all · — N/A.

| Capability                  | postgres | redis | mongodb | nats | qdrant | neo4j | s3  |
| --------------------------- | -------- | ----- | ------- | ---- | ------ | ----- | --- |
| Pool size                   | ✅       | —¹   | ⚠️ URI  | —²  | —²    | ❌    | ⚠️  |
| Min idle conns              | ❌       | —    | ⚠️ URI  | —   | —     | ❌    | —   |
| Connect timeout             | ❌       | ❌   | ⚠️ URI  | ❌  | ❌    | ❌    | ❌  |
| Op / call timeout           | ❌       | ❌   | ❌      | ❌  | ❌    | ❌    | ❌  |
| Idle timeout                | ❌       | —    | ⚠️ URI  | —   | —     | ❌    | —   |
| Max conn lifetime           | ❌       | —    | ⚠️ URI  | —   | —     | ❌    | —   |
| Acquire timeout (pool leak) | ❌       | —    | —       | —   | —     | ❌    | —   |
| Auto-reconnect              | ✅ sqlx | ✅ CM | ✅ SDK  | ⚠️  | —     | ✅    | ⚠️  |
| Reconnect strategy tunable  | ❌       | ❌   | ⚠️ URI  | ❌  | —     | ❌    | ❌  |
| Slow-call warn log          | ❌       | ❌   | ❌      | ❌  | ❌    | ❌    | ❌  |
| Saturation metrics          | ❌       | ❌   | ❌      | ❌  | ❌    | ❌    | ❌  |
| Health probe isolated       | ❌       | ❌   | ❌      | ⚠️³ | ❌    | ❌    | ❌  |
| Per-probe timeout           | ❌       | ❌   | ❌      | ❌  | ❌    | ❌    | ❌  |
| Bounded shutdown            | ❌⁴     | —    | ❌      | ⚠️⁵ | —    | ❌    | —   |

¹ `redis::aio::ConnectionManager` is multiplexed — no traditional pool.
² `async_nats::Client` / Qdrant gRPC are multiplexed per-connection.
³ Health check uses `client.connection_state()` — local cached state,
  not a fresh PING. Can lag reality by seconds.
⁴ `PgPool::close().await` waits *forever* for inflight queries.
⁵ NATS flush is bounded by the SDK's own write timeout, but not by us.

## Cross-cutting findings

### F1 — No per-call timeout layer (severity: high)

Every integration relies on its SDK's default op timeout. For sqlx
there is **none** (a stuck `SELECT` will wait until the TCP socket
times out — which under most kernel settings means tens of minutes).
For `async_nats` and `mongodb::Client`, defaults are similarly
permissive.

The user-visible failure: a backend hiccup makes HTTP handlers park
indefinitely; the request-level `tower-http::timeout` middleware (set
to 30s by default in `MiddlewareConfig`) eventually fires, but the
underlying DB call keeps running and holds a pool slot. Successive
slow requests starve the pool.

**Fix shape:** every `*Handle` should expose `with_timeout(Duration)`
that wraps the operation in `tokio::time::timeout` and emits a
`Timeout`-classified error. Config field: `op_timeout_ms`. Default:
5_000 ms.

### F2 — No saturation metrics (severity: high for prod, ok for alpha)

`hwhkit-observability` already wires Prometheus. None of the
integrations emit metrics. The interesting gauges per integration:

- `postgres`: `pool_size`, `pool_idle`, `pool_acquire_wait_seconds`
  (histogram)
- `redis`: `inflight_commands`, `connection_state{state="connected|disconnected|reconnecting"}`
- `mongodb`: `pool_in_use`, `pool_available`, `server_selection_failures_total`
- `nats`: `published_total`, `subscribers`, `pending_bytes`, `reconnects_total`
- `qdrant`/`neo4j`/`s3`: per-op latency histograms (the client SDKs
  give us hooks)

**Fix shape:** add a `metrics: MetricsConfig { enabled: bool }` field
to each integration's section; when enabled, spawn a background task
during `init` that samples the SDK's exposed stats every N seconds and
emits gauges with the `integration` label.

### F3 — Health probe shares the hot-path pool (severity: medium)

`PostgresHealthCheck` runs `SELECT 1` against the same `PgPool` used
by handlers. Under saturation:

1. All `max_connections` pool slots are checked out by slow queries.
2. `/health/ready` arrives, tries to acquire a slot, blocks on the
   pool's wait queue.
3. The readiness endpoint times out (or hangs — currently no per-probe
   timeout at the endpoint level either; see TODO #6).
4. K8s declares the pod unready and stops sending traffic.
5. Inflight requests have nowhere to drain; situation worsens.

Same shape for Redis (shared `ConnectionManager`), MongoDB (shared
client), Qdrant (shared client), Neo4j (shared graph pool).

**Fix shape (two layers):**
1. **Endpoint-level**: per-probe `tokio::time::timeout` at the readiness
   handler (TODO #6 — separate work item).
2. **Integration-level**: probe through an isolated path where
   possible. For sqlx, use `pool.acquire_timeout(probe_timeout)` so the
   probe fails fast instead of queueing. For mongodb / nats / redis,
   `tokio::time::timeout` around the probe call.

### F4 — Pool-leak guard missing on sqlx (severity: medium)

`PgPoolOptions` is not given an `acquire_timeout`. sqlx's default is
30s, which is too long for a request-driven service and too short for
a heavy migration job — needs to be config-driven.

A leaked connection (handler held an `Acquire` past its scope) under
heavy load will manifest as 30s tail-latency spikes that the
`tower-http::timeout` middleware kills mid-flight, leaving the slot
permanently checked out until lifetime expiry. With explicit
`acquire_timeout` we at least get a typed error and an early
`pool_acquire_failed` metric tick.

### F5 — No slow-call warn log (severity: low; high once we have traffic)

Every integration call should log `warn!` when wall time exceeds a
configurable threshold. Default thresholds:

| Integration | Default slow threshold |
| ----------- | ---------------------- |
| postgres    | 500 ms                 |
| mongodb     | 500 ms                 |
| redis       | 100 ms                 |
| neo4j       | 500 ms                 |
| qdrant      | 1_000 ms               |
| nats publish | 100 ms                 |
| s3          | 2_000 ms               |

**Fix shape:** wrap each `*Handle`'s outbound call site, OR provide a
`MeasuredHandle` adaptor that downstream code wraps when it cares.

### F6 — Bounded shutdown (severity: medium)

Only `PostgresProvider` does anything in `shutdown` — and it calls
`PgPool::close().await` which can block forever. The rest are no-ops.
On graceful shutdown, the outer drain timeout (`max_drain_secs`, 30s)
will eventually fire, but in the meantime nothing is releasing
sockets.

**Fix shape:** wrap shutdown in `tokio::time::timeout(drain_budget /
N_providers, …)` per provider, where the budget is propagated from
the caller. The integration's shutdown returns immediately on
timeout, logging a warning.

### F7 — Health-check inconsistency: NATS uses cached state

`NatsHealthCheck` returns Ok based on `client.connection_state()`.
That's the client's local view, not a server-side liveness check. A
zombie process holding a stale connection will report Healthy until
the OS kills the socket.

**Fix shape:** issue a real `flush()` or `request("$SYS.PING", ...)`
during the probe. Costs a roundtrip but is the actual contract a
readiness probe should make.

## Proposed config schema (additive, non-breaking)

Each `*IntegrationConfig` gains an optional `resilience` block. All
fields have defaults so existing TOML files keep working.

```toml
[integrations.sql.postgres]
enabled = true
url = "postgres://..."

[integrations.sql.postgres.resilience]
max_connections = 20         # already exists, just moved here
min_connections = 2          # NEW
connect_timeout_ms = 5_000   # NEW
acquire_timeout_ms = 3_000   # NEW (sqlx default 30s is too long)
op_timeout_ms = 5_000        # NEW
idle_timeout_ms = 600_000    # NEW (10 min)
max_lifetime_ms = 1_800_000  # NEW (30 min — survives connection rotation)
slow_threshold_ms = 500      # NEW
shutdown_timeout_ms = 5_000  # NEW (bound PgPool::close)

[integrations.sql.postgres.health]
probe_timeout_ms = 500       # NEW (probe fails fast even if pool saturated)
```

Identical shape — minus pool-specific knobs — for every other
integration. The key invariant: **no field is required**; resilience
defaults are baked in. The schema only adds operator-controllable
knobs for the cases the defaults get wrong.

## Implementation priority

The audit produces eight follow-up TODOs, ordered by ROI when
mota-agent-firm starts running real traffic:

| Pri | Item | Why first |
| --- | --- | --- |
| **P0a** | Per-call timeout layer + `op_timeout_ms` config (all 7) | F1 prevents the worst failure mode (handlers parked indefinitely). One pattern across all integrations. |
| **P0b** | Per-probe timeout + isolated probe path (F3, F7) | Pairs with TODO #6 (readiness chaos test) — they share fixtures. |
| **P0c** | Saturation metrics for postgres + redis (F2 partial) | The two highest-traffic integrations; pattern then ports to others. |
| **P0d** | Bounded shutdown per provider (F6) | One-liner per integration; prevents graceful-shutdown timeouts from masking real issues. |
| **P1a** | Slow-call warn log (F5) | Tooling exists once F2 lands; reuses the same wrapper. |
| **P1b** | Pool-leak guard on sqlx (`acquire_timeout`) (F4) | Sub-task of P0a but worth calling out — fixes a specific 30s tail. |
| **P1c** | Saturation metrics for remaining 5 integrations | Apply the pattern from P0c. |
| **P2** | Tunable reconnect strategy per integration | Each SDK has a different idiom; do this once we have one production deploy that needs it. |

## What's NOT in scope here

- Adding `tokio-retry` / `backoff` for application-level retries. That
  belongs in handlers, not in the integration layer.
- Per-integration circuit breakers. The existing
  `hwhkit::production::circuit_breaker` is HTTP-only (outbound
  `reqwest`); per-integration breakers are a separate design (would
  need typed error → breaker-state mapping). Park until F1 + F2 land
  and we have real failure data.
- Connection-pool warming / pre-fork. Not needed pre-1.0.
- Read replicas / topology-aware routing. Out of scope; should live in
  the application, not the framework.

## Verification plan (once fixes land)

The audit's fixes are testable using the live-integration harness
TODO #1 will introduce (`testcontainers`-based). For each integration:

1. **Saturation test**: spin up the backend, configure
   `max_connections=2`, fire 10 concurrent slow queries → assert the
   8 queued requests fail with `Timeout` after `acquire_timeout_ms`,
   not after 30s.
2. **Hung-backend test**: pause the container (`docker pause`),
   fire a query → assert it fails with `Timeout` after `op_timeout_ms`,
   not after the SDK default.
3. **Readiness isolation test**: saturate the pool → assert
   `/health/ready` still returns within `probe_timeout_ms`.
4. **Bounded-shutdown test**: start a long query, send SIGTERM →
   assert the process exits within `max_drain_secs +
   shutdown_timeout_ms`, not after the query completes.

These four tests should ship alongside the fixes, not after.
