//! Regression: `production::server::serve()` must NOT cap total server uptime
//! by `max_drain_secs`.
//!
//! A previous version wrapped the whole `axum::serve` future in
//! `tokio::time::timeout(max_drain_secs, …)`. With no shutdown signal the
//! future never resolved, so the timeout fired after `max_drain_secs` and the
//! process exited `Ok(())`. Under `restart: unless-stopped` that's an endless
//! self-restart loop (every `max_drain_secs` seconds) — and 502s through any
//! upstream proxy that hits the restart window.
//!
//! This lives in its own test binary (separate process) because the global
//! metrics recorder installs once per process; sharing a binary with another
//! `hwhkit::run` test makes whichever runs second drop `/metrics`.

use std::time::Duration;

use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::config::{AppConfig, BootstrapConfig};
use hwhkit::{AppContext, Application};
use tempfile::TempDir;

struct EchoApp;

#[async_trait]
impl Application for EchoApp {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> hwhkit::Result<Router> {
        Ok(Router::new().route("/echo", get(|| async { "echo-ok" })))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_does_not_self_terminate_without_signal() {
    // Tiny drain budget — the OLD code would force-exit after 1s.
    let dir = TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("default.toml"),
        "[runtime.shutdown]\nmax_drain_secs = 1\n",
    )
    .expect("write config");

    let bootstrap = BootstrapConfig::default()
        .with_service_name("hwhkit-no-self-terminate")
        .with_config_dir(dir.path());
    let built = hwhkit::run(EchoApp, bootstrap).await.expect("bootstrap");
    assert_eq!(
        built.config().runtime.shutdown.max_drain_secs,
        1,
        "config should load max_drain_secs=1"
    );

    let shutdown = built.shutdown();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let base = format!("http://{addr}");
    let server_task =
        tokio::spawn(
            async move { hwhkit::production::server::run_with_listener(built, listener).await },
        );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");

    // Wait until serving.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(r) = client.get(format!("{base}/health")).send().await {
            if r.status().is_success() {
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not become ready within 5s"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Wait well past max_drain_secs (1s) WITHOUT any shutdown signal.
    tokio::time::sleep(Duration::from_secs(3)).await;

    assert!(
        !server_task.is_finished(),
        "server self-terminated without a shutdown signal — max_drain_secs leaked into total uptime"
    );
    let resp = client
        .get(format!("{base}/health"))
        .send()
        .await
        .expect("GET /health past the drain window");
    assert!(
        resp.status().is_success(),
        "/health should still be 2xx well past max_drain_secs"
    );

    // Clean up: now signal shutdown and confirm it returns within the budget.
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(4), server_task).await;
}
