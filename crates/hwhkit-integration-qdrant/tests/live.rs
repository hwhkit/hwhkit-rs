//! Live integration tests for the Qdrant provider.
//!
//! `testcontainers-modules` does not ship a Qdrant module, so we drive
//! the `qdrant/qdrant` image directly via `GenericImage`. The waitFor
//! strategy watches stderr for the "Actix runtime found" line that the
//! Qdrant server logs once both the HTTP (6333) and gRPC (6334) ports
//! are live.
//!
//! The hwhkit Qdrant provider's `validate_url` accepts only
//! `http://` / `https://` URLs and the underlying client speaks gRPC
//! over HTTP/2 on port 6334.
//!
//! Run with:
//!
//! ```sh
//! cargo test -p hwhkit-integration-qdrant -- --ignored
//! ```

use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, IntegrationProvider};
use hwhkit_integration_qdrant::{QdrantHandle, QdrantProvider};
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    GenericImage,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; run with `cargo test -- --ignored`"]
async fn live_qdrant_full_lifecycle() {
    // We pin the image tag so a future `qdrant/qdrant:latest` log-line
    // rewording doesn't break the wait strategy without a code change.
    //
    // Qdrant emits the readiness signal via `tracing` to **stdout** (not
    // stderr — verified on v1.12.4). "Qdrant gRPC listening on" is the
    // last initialisation line the binary emits, so it's the safest
    // anchor.
    let container = GenericImage::new("qdrant/qdrant", "v1.12.4")
        .with_exposed_port(6333.tcp())
        .with_exposed_port(6334.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Qdrant gRPC listening on"))
        .start()
        .await
        .expect("start qdrant container");

    let grpc_port = container
        .get_host_port_ipv4(6334)
        .await
        .expect("get container grpc port");
    let url = format!("http://127.0.0.1:{grpc_port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.vector.qdrant.enabled = true;
    cfg.integrations.vector.qdrant.required = true;
    cfg.integrations.vector.qdrant.url = url.clone();
    // No api_key — anonymous mode is the qdrant/qdrant default.

    let provider = QdrantProvider;
    let mut ctx = AppContext::default();

    // 1. init: builds Qdrant client, lists collections (proves the
    //    gRPC channel works), parks the handle.
    provider
        .init(&mut ctx, &cfg)
        .await
        .expect("provider init against live qdrant");

    // 2. Handle present.
    let handle = ctx
        .get::<QdrantHandle>()
        .expect("QdrantHandle in AppContext after init");
    assert_eq!(handle.url(), url);
    assert!(!handle.has_api_key());

    // 3. Health check.
    let probe = provider
        .health_check(&ctx, &cfg)
        .expect("health_check returns Some after init");
    probe
        .check()
        .await
        .expect("health probe against live qdrant");
    assert_eq!(probe.name(), "qdrant");

    // 4. Real roundtrip: list_collections is the same call init does,
    //    so it's already covered. We additionally call `health_check`
    //    on the client itself to round-trip a different RPC and prove
    //    multiple operations work — anything beyond that requires
    //    creating a collection, which is heavier than needed for a
    //    smoke test.
    let _info = handle
        .client()
        .health_check()
        .await
        .expect("Qdrant::health_check RPC");

    // 5. Shutdown.
    provider
        .shutdown(&ctx)
        .await
        .expect("provider shutdown against live qdrant");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "lives alongside docker tests; run with `cargo test -- --ignored`"]
async fn live_qdrant_unreachable_url_surfaces_typed_error() {
    use hwhkit_core::{Error as CoreError, IntegrationFailureKind};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    let url = format!("http://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.vector.qdrant.enabled = true;
    cfg.integrations.vector.qdrant.required = true;
    cfg.integrations.vector.qdrant.url = url;

    let provider = QdrantProvider;
    let mut ctx = AppContext::default();

    let err = provider
        .init(&mut ctx, &cfg)
        .await
        .expect_err("init must fail against an unreachable URL");

    match err {
        CoreError::Integration { name, kind, .. } => {
            assert_eq!(name, "qdrant");
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
