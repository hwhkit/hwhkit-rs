//! End-to-end smoke test for the `run_and_serve` / `run_with_listener`
//! pipeline.
//!
//! What this test asserts:
//!
//! 1. `hwhkit::run` (bootstrap pipeline) produces a `BuiltApplication`
//!    that can be served.
//! 2. `production::server::run_with_listener` actually mounts every
//!    Tier-1 OOTB endpoint when default features are on:
//!    `/health`, `/health/ready`, `/version`, `/info`, `/metrics`.
//! 3. The user-supplied `Application::build_router` route is reachable
//!    through the same listener (i.e. mounting OOTB endpoints does not
//!    shadow user routes).
//! 4. Cancelling `BuiltApplication::shutdown()` causes the server task
//!    to return within the configured `max_drain_secs` budget, and
//!    returns `Ok(())`. This is the canonical graceful-shutdown
//!    contract; if it ever regresses, this test fails fast.
//!
//! What this test deliberately does **not** cover:
//!
//! - Real OS signal delivery (SIGTERM / SIGINT). Driving the kernel
//!   signal queue from inside `cargo test` is too racy to be useful in
//!   CI. `shutdown::install`'s signal path is exercised indirectly:
//!   it spawns a task that observes the same `ShutdownToken`, and that
//!   task exits cleanly when the token is cancelled via `shutdown.cancel()`.
//! - Integration providers. The test runs with an empty config dir, so
//!   no integrations are initialised and `/health/ready` is trivially
//!   green. Live integration tests live in the `hwhkit-integration-*`
//!   crates (TODO P0 #1).

use std::time::Duration;

use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::config::{AppConfig, BootstrapConfig};
use hwhkit::{AppContext, Application};
use tempfile::TempDir;

/// Minimal Application: one custom route. Everything else (health,
/// metrics, version, middleware bundle, request-id) is supplied OOTB
/// by `production::server::run_with_listener` based on default features.
struct EchoApp;

#[async_trait]
impl Application for EchoApp {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> hwhkit::Result<Router> {
        Ok(Router::new().route("/echo", get(|| async { "echo-ok" })))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn e2e_serves_ootb_endpoints_and_shuts_down_gracefully() {
    // Empty tempdir with no config files. `FileDefaultSource` and
    // `FileEnvironmentSource` both treat missing files as no-ops, so the
    // loader falls back to `AppConfig::default()` everywhere — which is
    // valid (host="0.0.0.0", port=3000, etc.). We never actually read
    // `server.port` because the test drives its own listener; this just
    // exercises the zero-config DX path.
    let dir = TempDir::new().expect("tempdir");
    let bootstrap = BootstrapConfig::default()
        .with_service_name("hwhkit-e2e")
        .with_config_dir(dir.path());

    // Bootstrap the application. After this returns we have a fully
    // initialised AppContext + user router + shutdown token, but
    // nothing is listening yet.
    let built = hwhkit::run(EchoApp, bootstrap)
        .await
        .expect("bootstrap should succeed with default config");

    let shutdown = built.shutdown();
    let drain_secs = built.config().runtime.shutdown.max_drain_secs;

    // Pre-bind on 127.0.0.1:0 so the OS picks a free port. Without
    // `run_with_listener`, e2e tests would have to either race on a
    // fixed port or guess one — both fragile in CI.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let base = format!("http://{addr}");

    let server_task = tokio::spawn(async move {
        hwhkit::production::server::run_with_listener(built, listener).await
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");

    wait_until_ready(&client, &base).await;

    // Tier-1 OOTB endpoints. Each is gated on a default feature; if any
    // of them stops being mounted by `run_with_listener` (regression in
    // the mount logic, or a feature flag flip), this loop catches it.
    for path in ["/health", "/health/ready", "/version", "/info", "/metrics"] {
        let url = format!("{base}{path}");
        let resp = client
            .get(&url)
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert!(
            resp.status().is_success(),
            "{path} expected 2xx, got {}",
            resp.status()
        );
    }

    // User route still reachable through the OOTB middleware stack.
    let echo = client
        .get(format!("{base}/echo"))
        .send()
        .await
        .expect("GET /echo");
    assert!(
        echo.status().is_success(),
        "/echo expected 2xx, got {}",
        echo.status()
    );
    let body = echo.text().await.expect("body");
    assert_eq!(body, "echo-ok");

    // /metrics should produce Prometheus text exposition (a few well-known
    // gauges / counters always present once the recorder is installed).
    let metrics_body = client
        .get(format!("{base}/metrics"))
        .send()
        .await
        .expect("GET /metrics again")
        .text()
        .await
        .expect("metrics body");
    assert!(
        metrics_body.contains("hwhkit_build_info"),
        "expected /metrics to expose hwhkit_build_info; got:\n{metrics_body}"
    );

    // Request-id middleware should echo the header we send back. This
    // exercises the request-id layer end-to-end (not just its unit test).
    let with_id = client
        .get(format!("{base}/echo"))
        .header("x-request-id", "test-rid-42")
        .send()
        .await
        .expect("GET /echo with x-request-id");
    let echoed_id = with_id
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    assert_eq!(
        echoed_id.as_deref(),
        Some("test-rid-42"),
        "expected request-id middleware to echo the inbound header"
    );

    // Trigger graceful shutdown. axum::serve must stop accepting new
    // connections immediately and the future must resolve within the
    // configured drain window (plus a small wall-clock slack to absorb
    // task scheduling jitter on slow CI runners).
    shutdown.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(drain_secs + 2), server_task)
        .await
        .expect("server did not return within drain budget");
    let serve_result = outcome.expect("server task panicked");
    serve_result.expect("server returned an error from serve loop");
}

/// Poll `/health` with a 5s deadline. The server task is racing us — we
/// can't proceed with assertions until `axum::serve` has actually started
/// accepting connections on the listener. 50ms granularity keeps the
/// happy path fast (usually one or two polls).
async fn wait_until_ready(client: &reqwest::Client, base: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let url = format!("{base}/health");
    loop {
        if let Ok(r) = client.get(&url).send().await {
            if r.status().is_success() {
                return;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("server did not become ready within 5s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
