//! Live integration tests for the Redis provider.
//!
//! Spins up a real Redis (7.x) in a Docker container via
//! `testcontainers` and exercises the full provider contract:
//! `init` → handle visible in `AppContext` → `health_check` → real
//! command through the manager → `shutdown`.
//!
//! All tests are `#[ignore]`d so the default `cargo test` stays
//! hermetic. Run with:
//!
//! ```sh
//! cargo test -p hwhkit-integration-redis -- --ignored
//! ```

use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, IntegrationProvider};
use hwhkit_integration_redis::{RedisHandle, RedisProvider};
use testcontainers_modules::{redis::Redis, testcontainers::runners::AsyncRunner};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; run with `cargo test -- --ignored`"]
async fn live_redis_full_lifecycle() {
    let container = Redis::default()
        .start()
        .await
        .expect("start redis container");
    let port = container
        .get_host_port_ipv4(6379)
        .await
        .expect("get container ipv4 port");
    let url = format!("redis://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.redis.enabled = true;
    cfg.integrations.redis.required = true;
    cfg.integrations.redis.url = url.clone();

    let provider = RedisProvider;
    let mut ctx = AppContext::default();

    // 1. init: opens client, builds ConnectionManager, PINGs the server.
    provider
        .init(&mut ctx, &cfg)
        .await
        .expect("provider init against live redis");

    // 2. Handle present + accessors return what we configured.
    let handle = ctx
        .get::<RedisHandle>()
        .expect("RedisHandle in AppContext after init");
    assert_eq!(handle.url(), url);

    // 3. Health check (uses the shared ConnectionManager — audit
    //    finding F3, fixed in TODO P0b. For now we just assert the
    //    happy path works).
    let probe = provider
        .health_check(&ctx, &cfg)
        .expect("health_check returns Some after init");
    probe
        .check()
        .await
        .expect("health probe against live redis");
    assert_eq!(probe.name(), "redis");

    // 4. Real SET / GET round-trip through the manager.
    let mut conn = handle.manager();
    let _: () = redis::cmd("SET")
        .arg("hwhkit:live-test")
        .arg("v1")
        .query_async(&mut conn)
        .await
        .expect("SET");
    let got: String = redis::cmd("GET")
        .arg("hwhkit:live-test")
        .query_async(&mut conn)
        .await
        .expect("GET");
    assert_eq!(got, "v1");

    // 5. Shutdown (no-op in current implementation; here for
    //    forward-compat — when P0d lands, this assertion will tighten).
    provider
        .shutdown(&ctx)
        .await
        .expect("provider shutdown against live redis");
}

/// Unreachable URL must surface as a typed `Integration` error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "lives alongside docker tests; run with `cargo test -- --ignored`"]
async fn live_redis_unreachable_url_surfaces_typed_error() {
    use hwhkit_core::{Error as CoreError, IntegrationFailureKind};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    let url = format!("redis://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.redis.enabled = true;
    cfg.integrations.redis.required = true;
    cfg.integrations.redis.url = url;

    let provider = RedisProvider;
    let mut ctx = AppContext::default();

    let err = provider
        .init(&mut ctx, &cfg)
        .await
        .expect_err("init must fail against an unreachable URL");

    match err {
        CoreError::Integration { name, kind, .. } => {
            assert_eq!(name, "redis");
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
