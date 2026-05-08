//! Production-ready runtime defaults wired into [`crate::run_and_serve`].
//!
//! Each submodule lives behind its own feature flag so users can opt out
//! without forking. The pieces compose into a tower middleware stack and
//! a router that auto-mounts `/health`, `/metrics`, `/version`, `/info`.
//!
//! Most consumers never need to touch these modules directly — they take
//! effect automatically when [`crate::bootstrap::run`] runs the default
//! provider chain.

#[cfg(feature = "health-endpoints")]
pub mod health;

#[cfg(feature = "metrics")]
pub mod metrics;

#[cfg(feature = "process-metrics")]
pub mod process_metrics;

#[cfg(feature = "version-endpoints")]
pub mod version;

#[cfg(feature = "rate-limit")]
pub mod rate_limit;

#[cfg(feature = "idempotency")]
pub mod idempotency;

#[cfg(feature = "circuit-breaker")]
pub mod circuit_breaker;

#[cfg(feature = "request-id")]
pub mod request_id;

#[cfg(feature = "middleware-bundle")]
pub mod middleware;

#[cfg(feature = "graceful-shutdown")]
pub mod shutdown;

#[cfg(feature = "multi-tenant")]
pub mod tenant;

pub mod server;
