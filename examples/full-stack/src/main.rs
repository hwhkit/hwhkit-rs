//! Full-stack hwhkit example.
//!
//! Pulls in the full feature menu (Postgres, Redis, NATS, S3, JWT,
//! rate-limit, idempotency, circuit-breaker, scheduler, OTel) to
//! demonstrate that every capability is one feature flag away. The
//! umbrella crate's `default_providers()` auto-registers the
//! integration providers based on which features the binary is built
//! with; each provider is then a no-op when its
//! `[integrations.*].enabled` config field is `false`.

use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::config::{AppConfig, BootstrapConfig};
use hwhkit::{run_and_serve, AppContext, Application};

struct MyApp;

#[async_trait]
impl Application for MyApp {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> hwhkit::Result<Router> {
        // The router itself is intentionally tiny — the point of this
        // example is the bootstrap surface and the feature flags, not
        // the route shape. Add JWT extractors / rate-limit layers /
        // idempotency layers from `hwhkit::production::*` here in your
        // real service.
        Ok(Router::new()
            .route("/", get(|| async { "hwhkit full-stack example" }))
            .route("/whoami", get(|| async { "anonymous" })))
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_and_serve(MyApp, BootstrapConfig::default()).await
}
