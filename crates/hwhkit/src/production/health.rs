//! `/health` (liveness) and `/health/ready` (readiness) handlers.
//!
//! Liveness always returns 200 if the process is up. Readiness runs every
//! [`HealthCheck`] registered on the [`HealthRegistry`] (each integration
//! provider registers one during init). Required failures yield 503;
//! optional failures degrade to 200 with a `degraded: true` flag.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use hwhkit_config::HealthConfig;
use hwhkit_core::{HealthCheckResult, HealthRegistry, HealthStatus};
use serde::Serialize;

#[derive(Clone)]
pub struct HealthState {
    pub registry: HealthRegistry,
}

#[derive(Serialize)]
struct LiveResponse<'a> {
    status: &'a str,
}

#[derive(Serialize)]
struct ReadyResponse {
    status: &'static str,
    degraded: bool,
    checks: Vec<HealthCheckResult>,
}

async fn liveness() -> impl IntoResponse {
    Json(LiveResponse { status: "ok" })
}

async fn readiness(State(state): State<Arc<HealthState>>) -> impl IntoResponse {
    let results = state.registry.run_all().await;

    let mut required_down = false;
    let mut optional_down = false;
    for r in &results {
        if r.status != HealthStatus::Up {
            if r.required {
                required_down = true;
            } else {
                optional_down = true;
            }
        }
    }

    let status = if required_down {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    let body = ReadyResponse {
        status: if required_down { "down" } else { "up" },
        degraded: optional_down,
        checks: results,
    };
    (status, Json(body))
}

/// Build a router that serves the configured live + ready paths.
pub fn router(cfg: &HealthConfig, registry: HealthRegistry) -> Router {
    let state = Arc::new(HealthState { registry });
    Router::new()
        .route(&cfg.path_live, get(liveness))
        .route(&cfg.path_ready, get(readiness))
        .with_state(state)
}
