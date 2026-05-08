//! Minimal hwhkit application.
//!
//! The single goal here is to show the *shortest* possible path from
//! `cargo new` to a service that has health checks, graceful shutdown,
//! a request-id chain, the middleware bundle and a `/version` endpoint
//! enabled out of the box. We add **one** route (`GET /`) on top.

use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::config::{AppConfig, BootstrapConfig};
use hwhkit::{run_and_serve, AppContext, Application};

/// The application type. `Application::build_router` is the only method
/// you must implement — `run_and_serve` does the rest.
struct MyApp;

#[async_trait]
impl Application for MyApp {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> hwhkit::Result<Router> {
        Ok(Router::new().route("/", get(|| async { "hello from hwhkit" })))
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // `BootstrapConfig::default()` reads `config/default.toml` (if it
    // exists), then the env-specific overlay, then env vars prefixed
    // `HWHKIT__`. Drop a `config/default.toml` next to your binary to
    // override the listen port etc.
    run_and_serve(MyApp, BootstrapConfig::default()).await
}
