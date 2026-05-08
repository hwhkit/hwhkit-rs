//! # HwhKit
//!
//! A batteries-included toolkit for building production Rust web
//! services. Built on top of [`axum`], [`tokio`], and `tower-http`.
//!
//! ## Features
//!
//! - One-call server bootstrap ([`run_and_serve`])
//! - Pluggable middleware bundle (CORS, compression, panic-catcher, …)
//! - First-class production defaults: `/health`, `/health/ready`,
//!   `/metrics`, `/version`, `/info`, request-id, graceful shutdown
//! - Optional capabilities: rate-limit, idempotency, circuit-breaker,
//!   JWT verifier, scheduler
//! - Per-tenant scoping primitives ([`hwhkit_core::TenantId`])
//!
//! ## Quick start
//!
//! ```no_run
//! use hwhkit::prelude::*;
//! use axum::{routing::get, Router};
//! use async_trait::async_trait;
//!
//! struct MyApp;
//!
//! #[async_trait]
//! impl Application for MyApp {
//!     async fn build_router(
//!         &self,
//!         _ctx: AppContext,
//!         _cfg: &hwhkit_config::AppConfig,
//!     ) -> Result<Router> {
//!         Ok(Router::new().route("/", get(|| async { "ok" })))
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     run_and_serve(MyApp, BootstrapConfig::default()).await
//! }
//! ```
//!
//! ## Re-exports
//!
//! Per-crate exports under `hwhkit::*` are kept narrow and explicit. To
//! pull in `axum::Router` or `serde::Serialize`, depend on those crates
//! directly — `hwhkit` does not re-export them.

pub mod bootstrap;
pub mod prelude;
pub mod production;

// Convenience re-exports of the most-used `hwhkit_core` items so
// downstream code can `use hwhkit::*` for the headline types without
// reaching into the workspace crates. The full surface still lives at
// `hwhkit::core` for advanced use.
pub use bootstrap::{run, run_and_serve};
pub use hwhkit_core::error::{Error, IntegrationFailureKind};
pub use hwhkit_core::{
    ApiError, ApiResult, AppContext, Application, BuiltApplication, FieldError, HealthCheck,
    HealthRegistry, IntegrationProvider, ProblemDetails, Result, RuntimeFeatures, ShutdownToken,
};

#[cfg(feature = "multi-tenant")]
pub use hwhkit_core::{TenantId, TenantScope};

// Workspace-crate aliases. These are the `0.6` canonical names. The
// historical `*_v2` aliases were dropped in 0.6 — depend on the
// workspace crates by their package names instead.
pub use hwhkit_config as config;
pub use hwhkit_core as core;
pub use hwhkit_observability as observability;

#[cfg(feature = "mongodb")]
pub use hwhkit_integration_mongodb as mongodb;
#[cfg(feature = "nats")]
pub use hwhkit_integration_nats as nats;
#[cfg(feature = "neo4j")]
pub use hwhkit_integration_neo4j as neo4j;
#[cfg(feature = "postgres")]
pub use hwhkit_integration_postgres as postgres;
#[cfg(feature = "qdrant")]
pub use hwhkit_integration_qdrant as qdrant;
#[cfg(feature = "redis")]
pub use hwhkit_integration_redis as redis;
#[cfg(feature = "s3")]
pub use hwhkit_integration_s3 as s3;
#[cfg(feature = "scheduler")]
pub use hwhkit_scheduler as scheduler;

/// JWT verifier facade. Re-exports the modern verifier from
/// [`hwhkit_core::jwt`].
#[cfg(feature = "jwt")]
pub mod jwt {
    pub use hwhkit_core::jwt::{Claims, CtxClaims, JwtError, JwtVerifier, JwtVerifierConfig};
}
