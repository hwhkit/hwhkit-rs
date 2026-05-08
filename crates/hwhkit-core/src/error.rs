//! Hybrid error model for `hwhkit-core`.
//!
//! Following the kubernetes-rs / aws-sdk-rust style:
//!
//! - **strongly typed** for hwhkit's own concerns (config validation,
//!   feature/binary mismatch, …);
//! - **boxed** (`Box<dyn std::error::Error + Send + Sync>`) for opaque
//!   third-party sources (a database driver, a vector-store client, …);
//! - **semantic-category enum** ([`IntegrationFailureKind`]) so callers
//!   can make retry / fail-fast decisions without string-matching.
//!
//! All public error enums are `#[non_exhaustive]` so adding new variants
//! is not a breaking change.

use std::error::Error as StdError;

/// Boxed third-party error. We don't try to introspect these — we forward
/// them through the `source()` chain so downcasting and printing work.
pub type BoxError = Box<dyn StdError + Send + Sync + 'static>;

/// Top-level hwhkit error type.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// Config could not be loaded, parsed, or validated.
    #[error("invalid configuration: {message}")]
    InvalidConfig {
        message: String,
        #[source]
        source: Option<BoxError>,
    },

    /// The loaded config asked for a capability the running binary was
    /// not compiled with (cargo feature missing).
    #[error("feature `{feature}` enabled in config but cargo feature missing")]
    FeatureMismatch { feature: &'static str },

    /// An integration provider failed during init or runtime. The
    /// [`IntegrationFailureKind`] tells callers whether the failure is
    /// retriable, terminal-misconfiguration, etc.
    #[error("integration `{name}` failed: {kind}")]
    Integration {
        name: &'static str,
        kind: IntegrationFailureKind,
        #[source]
        source: BoxError,
    },

    /// I/O failure that doesn't fit any of the above buckets.
    #[error("io error")]
    Io(#[from] std::io::Error),

    /// Bootstrap-time failure not covered by the more specific variants.
    /// Prefer [`Error::InvalidConfig`] / [`Error::Integration`] when the
    /// failure has a clear category.
    #[error("bootstrap failed: {0}")]
    Bootstrap(String),
}

impl Error {
    /// Construct an [`Error::InvalidConfig`] with no source.
    pub fn invalid_config(message: impl Into<String>) -> Self {
        Self::InvalidConfig {
            message: message.into(),
            source: None,
        }
    }

    /// Construct an [`Error::InvalidConfig`] wrapping a third-party source.
    pub fn invalid_config_with_source(
        message: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::InvalidConfig {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Construct an [`Error::Integration`] with a static integration name,
    /// a semantic [`IntegrationFailureKind`], and a boxed third-party
    /// source.
    pub fn integration(
        name: &'static str,
        kind: IntegrationFailureKind,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Integration {
            name,
            kind,
            source: Box::new(source),
        }
    }

    /// Convenience: build an integration error from a `String`/`&str`
    /// message (used when no concrete source error is available — e.g.
    /// validation rejecting an obviously-bad URL before any I/O).
    pub fn integration_msg(
        name: &'static str,
        kind: IntegrationFailureKind,
        message: impl Into<String>,
    ) -> Self {
        Self::Integration {
            name,
            kind,
            source: Box::<dyn StdError + Send + Sync>::from(message.into()),
        }
    }
}

/// Coarse category for [`Error::Integration`] failures. Callers (e.g.
/// the bootstrap loop) consult this to decide whether to retry, fall back
/// to a degraded mode, or fail the whole startup.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[non_exhaustive]
pub enum IntegrationFailureKind {
    /// The integration's configured URL/endpoint is malformed.
    #[error("invalid URL")]
    InvalidUrl,
    /// Authentication against the upstream failed (bad credentials, expired token, …).
    #[error("authentication failed")]
    AuthFailed,
    /// Could not establish a TCP/TLS connection.
    #[error("connection refused")]
    ConnectionRefused,
    /// The upstream took too long to respond.
    #[error("timeout")]
    Timeout,
    /// The integration is misconfigured in a way that's not the URL itself
    /// (missing required field, conflicting options, …).
    #[error("misconfigured")]
    Misconfigured,
    /// The credentials authenticated but lack permission for the operation.
    #[error("permission denied")]
    PermissionDenied,
    /// Any other failure that doesn't slot into the categories above.
    #[error("other")]
    Other,
}

impl IntegrationFailureKind {
    /// Whether retrying the operation has a chance of succeeding without
    /// operator intervention. Used by the bootstrap loop to decide
    /// whether to retry `init` or skip the integration.
    pub fn is_transient(self) -> bool {
        matches!(self, Self::ConnectionRefused | Self::Timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(thiserror::Error, Debug)]
    #[error("inner")]
    struct Inner;

    #[test]
    fn invalid_config_carries_source() {
        let err = Error::invalid_config_with_source("bad", Inner);
        assert!(err.source().is_some());
    }

    #[test]
    fn integration_classifies_transient() {
        assert!(IntegrationFailureKind::Timeout.is_transient());
        assert!(IntegrationFailureKind::ConnectionRefused.is_transient());
        assert!(!IntegrationFailureKind::AuthFailed.is_transient());
        assert!(!IntegrationFailureKind::Misconfigured.is_transient());
    }
}
