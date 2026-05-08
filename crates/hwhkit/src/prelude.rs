//! Curated set of re-exports for the most common `hwhkit` APIs.
//!
//! ```ignore
//! use hwhkit::prelude::*;
//! ```
//!
//! **Philosophy:** the prelude is small on purpose. Only the handful of
//! types you reach for in 90% of services is here — the bootstrap entry
//! points, the application trait, the typed result/error pair, and the
//! HTTP problem-details types. Anything more specialised (integration
//! handles, middleware layers, …) lives at `hwhkit::*` and should be
//! imported explicitly.

pub use crate::bootstrap::{run, run_and_serve};
pub use hwhkit_config::BootstrapConfig;
pub use hwhkit_core::{
    error::{Error, IntegrationFailureKind},
    ApiError, ApiResult, AppContext, Application, BuiltApplication, IntegrationProvider, Result,
};

#[cfg(feature = "multi-tenant")]
pub use hwhkit_core::TenantId;
