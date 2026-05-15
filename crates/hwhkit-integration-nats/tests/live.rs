//! Live integration tests for the NATS provider.
//!
//! Spins up a real `nats:latest` server in a Docker container via
//! `testcontainers` (with JetStream enabled for the JetStream
//! roundtrip assertion). Exercises the provider lifecycle plus a
//! pub/sub roundtrip end-to-end.
//!
//! All tests are `#[ignore]`d so the default `cargo test` stays
//! hermetic. Run with:
//!
//! ```sh
//! cargo test -p hwhkit-integration-nats -- --ignored
//! ```

use futures::StreamExt;
use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, IntegrationProvider};
use hwhkit_integration_nats::{NatsHandle, NatsProvider};
use testcontainers_modules::{
    nats::Nats,
    testcontainers::{runners::AsyncRunner, ImageExt},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; run with `cargo test -- --ignored`"]
async fn live_nats_full_lifecycle() {
    // `--jetstream` arg enables JetStream so the JetStream context in
    // NatsHandle has a real backend to talk to. Without it,
    // `NatsHandle::jetstream()` would still be present but every
    // JetStream call would fail.
    let container = Nats::default()
        .with_cmd(["--jetstream"])
        .start()
        .await
        .expect("start nats container");
    let port = container
        .get_host_port_ipv4(4222)
        .await
        .expect("get container ipv4 port");
    let url = format!("nats://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.messaging.nats.enabled = true;
    cfg.integrations.messaging.nats.required = true;
    cfg.integrations.messaging.nats.url = url.clone();

    let provider = NatsProvider;
    let mut ctx = AppContext::default();

    // 1. init: connects + flushes + creates JetStream context.
    provider
        .init(&mut ctx, &cfg)
        .await
        .expect("provider init against live nats");

    // 2. Handle is in context.
    let handle = ctx
        .get::<NatsHandle>()
        .expect("NatsHandle in AppContext after init");
    assert_eq!(handle.url(), url);

    // 3. Health check. Current implementation reads
    //    `client.connection_state()` (audit finding F7 — local cached
    //    state, not a fresh PING). For now we just assert the happy
    //    path returns Ok. When P0b lands this will switch to a real
    //    roundtrip check.
    let probe = provider
        .health_check(&ctx, &cfg)
        .expect("health_check returns Some after init");
    probe.check().await.expect("health probe against live nats");
    assert_eq!(probe.name(), "nats");

    // 4. Real publish/subscribe roundtrip. This proves the connection
    //    actually carries traffic, not just opens a socket.
    let mut sub = handle
        .client()
        .subscribe("hwhkit.live.test")
        .await
        .expect("subscribe");
    handle
        .client()
        .publish("hwhkit.live.test", "ping".into())
        .await
        .expect("publish");
    handle.client().flush().await.expect("flush");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), sub.next())
        .await
        .expect("receive within 5s")
        .expect("subscription yielded a message");
    assert_eq!(&msg.payload[..], b"ping");

    // 5. Shutdown.
    provider
        .shutdown(&ctx)
        .await
        .expect("provider shutdown against live nats");
}

/// Unreachable URL → typed `Integration` error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "lives alongside docker tests; run with `cargo test -- --ignored`"]
async fn live_nats_unreachable_url_surfaces_typed_error() {
    use hwhkit_core::{Error as CoreError, IntegrationFailureKind};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    let url = format!("nats://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.messaging.nats.enabled = true;
    cfg.integrations.messaging.nats.required = true;
    cfg.integrations.messaging.nats.url = url;

    let provider = NatsProvider;
    let mut ctx = AppContext::default();

    let err = provider
        .init(&mut ctx, &cfg)
        .await
        .expect_err("init must fail against an unreachable URL");

    match err {
        CoreError::Integration { name, kind, .. } => {
            assert_eq!(name, "nats");
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
