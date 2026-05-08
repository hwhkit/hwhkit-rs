//! Postgres-backed REST example.
//!
//! Demonstrates:
//!  * Auto-wiring of the Postgres integration via the `postgres` cargo
//!    feature — the umbrella `hwhkit` crate registers
//!    [`hwhkit::postgres::PostgresProvider`] for you when the feature
//!    is on.
//!  * Pulling the [`PostgresHandle`] out of [`AppContext`] and reusing
//!    its `pool()` accessor in a handler.
//!  * Mapping `sqlx` errors into `hwhkit::ApiError` so the handler
//!    returns RFC-7807 problem-details on failure.

use async_trait::async_trait;
use axum::{extract::State, routing::get, Json, Router};
use hwhkit::config::{AppConfig, BootstrapConfig};
use hwhkit::postgres::PostgresHandle;
use hwhkit::{run_and_serve, ApiError, AppContext, Application};
use serde::Serialize;

#[derive(Clone)]
struct AppState {
    pg: PostgresHandle,
}

struct MyApp;

#[async_trait]
impl Application for MyApp {
    async fn build_router(&self, ctx: AppContext, _cfg: &AppConfig) -> hwhkit::Result<Router> {
        // The umbrella crate's `default_providers()` already registered
        // PostgresProvider when the `postgres` feature is enabled;
        // by the time we reach `build_router` the pool is live and
        // sitting in the context.
        let pg = (*ctx
            .get::<PostgresHandle>()
            .expect("postgres provider must be enabled for this example"))
        .clone();
        let state = AppState { pg };

        Ok(Router::new()
            .route("/db/now", get(db_now))
            .with_state(state))
    }
}

#[derive(Serialize)]
struct NowResponse {
    /// Server-side `now()` reported by Postgres as a string.
    now: String,
}

/// `GET /db/now` — runs `SELECT now()` against the configured pool and
/// returns the result. Any `sqlx` error becomes a 500 `ApiError`
/// carrying the database message.
async fn db_now(State(state): State<AppState>) -> std::result::Result<Json<NowResponse>, ApiError> {
    let row: (String,) = sqlx::query_as("SELECT now()::text")
        .fetch_one(state.pg.pool())
        .await
        .map_err(|e| ApiError::internal(format!("postgres query failed: {e}")))?;
    Ok(Json(NowResponse { now: row.0 }))
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_and_serve(MyApp, BootstrapConfig::default()).await
}
