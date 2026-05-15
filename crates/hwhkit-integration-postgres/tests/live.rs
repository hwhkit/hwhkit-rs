//! Live integration tests for the Postgres provider.
//!
//! These tests spin up a real Postgres in a Docker container via
//! `testcontainers` and exercise the full provider contract:
//! `init` → handle visible in `AppContext` → `health_check` → real
//! query through the pool → `shutdown`.
//!
//! All tests in this file are marked `#[ignore]` so they do **not**
//! run as part of the default `cargo test`. Run them explicitly with
//!
//! ```sh
//! cargo test -p hwhkit-integration-postgres -- --ignored
//! ```
//!
//! Requirements:
//!
//! - Docker available on the host (Docker Desktop, OrbStack, …).
//! - First run pulls the `postgres` image (~150 MB); subsequent runs
//!   reuse the cached image.
//!
//! Why `#[ignore]` instead of a feature flag: feature flags affect
//! compilation, not test selection. `#[ignore]` lets the file
//! *compile* on every `cargo build` — catching any breakage where the
//! integration's public types drift — while still keeping the default
//! `cargo test` hermetic.

use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, IntegrationProvider};
use hwhkit_integration_postgres::{PostgresHandle, PostgresProvider};
use testcontainers_modules::{postgres::Postgres, testcontainers::runners::AsyncRunner};

/// Full lifecycle smoke test: real container, real pool, real query.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; run with `cargo test -- --ignored`"]
async fn live_postgres_full_lifecycle() {
    let container = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get container ipv4 port");
    // The `testcontainers-modules` Postgres image uses the default
    // `postgres:postgres` superuser and a `postgres` database.
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let mut cfg = AppConfig::default();
    cfg.integrations.sql.postgres.enabled = true;
    cfg.integrations.sql.postgres.required = true;
    cfg.integrations.sql.postgres.url = url.clone();
    // Small pool — exercises the path with no headroom, makes future
    // saturation tests (TODO P0c) cheaper to add on top.
    cfg.integrations.sql.postgres.max_connections = 4;

    let provider = PostgresProvider;
    let mut ctx = AppContext::default();

    // 1. init: opens the pool, runs `SELECT 1` smoke test, parks the
    //    handle in the context. If the SDK / classify_sqlx_error /
    //    validate_url paths regress, this is where we see it first.
    provider
        .init(&mut ctx, &cfg)
        .await
        .expect("provider init against live postgres");

    // 2. Handle is in the context with the expected metadata.
    let handle = ctx
        .get::<PostgresHandle>()
        .expect("PostgresHandle in AppContext after init");
    assert_eq!(handle.url(), url);
    assert_eq!(handle.max_connections(), 4);

    // 3. Health check runs `SELECT 1` against the live pool.
    let probe = provider
        .health_check(&ctx, &cfg)
        .expect("health_check returns Some when the handle is initialised");
    probe
        .check()
        .await
        .expect("health probe against live postgres");
    assert_eq!(probe.name(), "postgres");
    assert!(probe.required(), "required=true was set in config");

    // 4. Real query through the handle — proves we can issue work, not
    //    just open a socket.
    let row: (i64,) = sqlx::query_as("SELECT 42::bigint")
        .fetch_one(handle.pool())
        .await
        .expect("SELECT 42 round-trip");
    assert_eq!(row.0, 42);

    // 5. Bounded shutdown. Today `PostgresProvider::shutdown` calls
    //    `PgPool::close().await` directly (audit finding F6 — can hang
    //    indefinitely). After P0d lands this assertion will tighten to
    //    `tokio::time::timeout(..., shutdown)` with a budget. For now
    //    we just verify the happy path returns Ok.
    provider
        .shutdown(&ctx)
        .await
        .expect("provider shutdown against live postgres");
}

/// Cancellation contract: an unreachable URL must surface as a typed
/// `Integration` error with `ConnectionRefused`, *not* as a panic or a
/// 30-second hang.
///
/// This test does not use Docker — it uses an ephemeral loopback port
/// that is guaranteed unbound. Kept in `live.rs` (and `#[ignore]`d)
/// alongside the Docker tests so the whole file is "the place real
/// connection behaviour is verified."
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "lives alongside docker tests; run with `cargo test -- --ignored`"]
async fn live_postgres_unreachable_url_surfaces_typed_error() {
    use hwhkit_core::{Error as CoreError, IntegrationFailureKind};

    // Bind an ephemeral port and close immediately to discover a free
    // port that is guaranteed to refuse connections for the test
    // duration (modulo a tiny race window — acceptable for this assertion).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    let url = format!("postgres://x:x@127.0.0.1:{port}/postgres");

    let mut cfg = AppConfig::default();
    cfg.integrations.sql.postgres.enabled = true;
    cfg.integrations.sql.postgres.required = true;
    cfg.integrations.sql.postgres.url = url;
    cfg.integrations.sql.postgres.max_connections = 1;

    let provider = PostgresProvider;
    let mut ctx = AppContext::default();

    let err = provider
        .init(&mut ctx, &cfg)
        .await
        .expect_err("init must fail against an unreachable URL");

    match err {
        CoreError::Integration { name, kind, .. } => {
            assert_eq!(name, "postgres");
            // Either ConnectionRefused (the typical Linux/macOS case)
            // or Timeout (if the kernel's connect retries push us past
            // the SDK default). Both are correct classifications; the
            // important thing is that we got a typed Integration error,
            // not a panic or an opaque box.
            assert!(
                matches!(
                    kind,
                    IntegrationFailureKind::ConnectionRefused | IntegrationFailureKind::Timeout
                ),
                "expected ConnectionRefused or Timeout, got {kind:?}"
            );
        }
        other => panic!("expected Integration error variant, got {other:?}"),
    }
}
