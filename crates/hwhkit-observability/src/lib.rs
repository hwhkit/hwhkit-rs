//! Logging + tracing setup for hwhkit services.
//!
//! - [`init_logging`] accepts `format = "auto" | "json" | "pretty"`.
//!   `auto` picks JSON when stdout is not a TTY (production / containers)
//!   and pretty when running interactively.
//! - With the `otel` feature enabled,
//!   [`otel_layer::init_with_otel`] wires an OTLP gRPC exporter into the
//!   `tracing` subscriber so every span is shipped to the configured
//!   collector.
//! - All public init functions return [`ObservabilityError`]; the crate
//!   has no remaining `Result<_, String>` surface (project-wide policy as
//!   of 0.6).

#![warn(missing_docs)]

use std::error::Error as StdError;

use serde::{Deserialize, Serialize};
use tracing_subscriber::{fmt, prelude::*, EnvFilter, Registry};

/// Error returned by the public initialisation entry points
/// ([`init_logging`], [`otel_layer::init_with_otel`]).
///
/// Marked `#[non_exhaustive]` so future variants (e.g. registry-already-
/// installed, exporter-handshake-failed) can be added without breaking
/// existing pattern matching.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ObservabilityError {
    /// The log filter directive (passed via [`LoggingConfig::level`])
    /// could not be parsed by `tracing-subscriber`'s `EnvFilter`.
    #[error("invalid log filter `{filter}`")]
    BadFilter {
        /// The directive string that failed to parse.
        filter: String,
        /// Underlying parse error from `tracing-subscriber`.
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },

    /// The OTLP / OpenTelemetry exporter pipeline failed to install.
    /// Common causes: TLS setup mismatch, the global tracer provider was
    /// already installed, or the collector endpoint was unreachable on
    /// the synchronous handshake.
    #[error("OTel exporter init failed")]
    OtelInit {
        /// Optional human-friendly context (e.g. "install_batch failed").
        #[allow(dead_code)]
        context: Option<String>,
        /// Underlying error from the OTel pipeline builder.
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },

    /// The crate was built without the `otel` feature but the caller
    /// asked for the OTel-aware initialiser. Rebuild with
    /// `--features otel` (or, when consuming via the umbrella `hwhkit`
    /// crate, `--features otel`) and try again.
    #[error("hwhkit-observability built without `otel` feature")]
    OtelDisabled,
}

/// Standalone logging configuration for callers that drive
/// [`init_logging`] directly without going through the
/// `hwhkit_config::AppConfig` pipeline.
///
/// **The canonical type lives in `hwhkit_config::LoggingConfig`** and is
/// what the bootstrap pipeline consumes. This type is intentionally kept
/// in sync (same `level`/`format` fields) so the two can be converted
/// trivially. Prefer the config-crate one when wiring through the
/// framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoggingConfig {
    /// `tracing-subscriber` env-filter directive, e.g. `"info"` or
    /// `"hyper=warn,my_app=debug"`.
    pub level: String,
    /// One of `"auto"`, `"pretty"`, or `"json"`.
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "auto".to_string(),
        }
    }
}

impl LoggingConfig {
    /// Convenience constructor: pretty-printed logs at the given level.
    pub fn pretty(level: impl Into<String>) -> Self {
        Self {
            level: level.into(),
            format: "pretty".to_string(),
        }
    }
    /// Convenience constructor: JSON-formatted logs at the given level.
    pub fn json(level: impl Into<String>) -> Self {
        Self {
            level: level.into(),
            format: "json".to_string(),
        }
    }
}

/// Standalone OTLP/OTel configuration for callers that drive
/// [`otel_layer::init_with_otel`] directly. Mirrors
/// `hwhkit_config::OtelConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OtelConfig {
    /// Master switch — when `false` the helper falls back to plain
    /// logging without ever opening a connection to the collector.
    pub enabled: bool,
    /// OTLP/gRPC endpoint URL.
    pub endpoint: String,
    /// Value emitted as the `service.name` resource attribute.
    pub service_name: String,
    /// Value emitted as the `service.version` resource attribute.
    pub service_version: String,
    /// Value emitted as the `deployment.environment` resource attribute.
    pub environment: String,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:4317".to_string(),
            service_name: "hwhkit-service".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            environment: "dev".to_string(),
        }
    }
}

fn detect_tty() -> bool {
    // Avoid pulling in extra crates: probe via libc on unix, fall back to
    // false elsewhere.
    #[cfg(unix)]
    {
        // SAFETY: isatty(3) is signal-safe and only reads kernel state.
        unsafe { libc_inline::isatty(1) != 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(unix)]
mod libc_inline {
    extern "C" {
        pub fn isatty(fd: i32) -> i32;
    }
}

fn make_filter(level: &str) -> Result<EnvFilter, ObservabilityError> {
    EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .map_err(|e| ObservabilityError::BadFilter {
            filter: level.to_string(),
            source: Box::new(e),
        })
}

/// Initialize tracing logging without OTel. Safe to call once per process.
pub fn init_logging(config: &LoggingConfig) -> Result<(), ObservabilityError> {
    let filter = make_filter(&config.level)?;
    let format = resolve_format(&config.format);
    let registry = Registry::default().with(filter);

    let _ = match format {
        ResolvedFormat::Json => registry.with(fmt::layer().json()).try_init(),
        ResolvedFormat::Pretty => registry
            .with(
                fmt::layer()
                    .with_target(false)
                    .with_file(false)
                    .with_line_number(false)
                    .with_thread_ids(false),
            )
            .try_init(),
    };

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum ResolvedFormat {
    Json,
    Pretty,
}

fn resolve_format(raw: &str) -> ResolvedFormat {
    match raw {
        "json" => ResolvedFormat::Json,
        "pretty" => ResolvedFormat::Pretty,
        // "auto" or anything else → tty-aware
        _ => {
            if detect_tty() {
                ResolvedFormat::Pretty
            } else {
                ResolvedFormat::Json
            }
        }
    }
}

#[cfg(feature = "otel")]
pub mod otel_layer {
    //! OpenTelemetry-aware initialiser, gated on the `otel` feature.

    use super::*;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry::KeyValue;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::{
        propagation::TraceContextPropagator,
        trace::{self as sdktrace, RandomIdGenerator, Sampler},
        Resource,
    };
    use tracing_opentelemetry::OpenTelemetryLayer;

    /// Initialize tracing-subscriber with OTLP gRPC exporter. Returns a
    /// guard that should be dropped at shutdown to flush spans.
    pub fn init_with_otel(
        log_cfg: &LoggingConfig,
        otel_cfg: &OtelConfig,
    ) -> Result<OtelGuard, ObservabilityError> {
        let filter = make_filter(&log_cfg.level)?;
        let format = resolve_format(&log_cfg.format);

        let resource = Resource::new(vec![
            KeyValue::new("service.name", otel_cfg.service_name.clone()),
            KeyValue::new("service.version", otel_cfg.service_version.clone()),
            KeyValue::new("deployment.environment", otel_cfg.environment.clone()),
        ]);

        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(otel_cfg.endpoint.clone());

        let provider = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(exporter)
            .with_trace_config(
                sdktrace::Config::default()
                    .with_sampler(Sampler::AlwaysOn)
                    .with_id_generator(RandomIdGenerator::default())
                    .with_resource(resource),
            )
            .install_batch(opentelemetry_sdk::runtime::Tokio)
            .map_err(|e| ObservabilityError::OtelInit {
                context: Some("install_batch failed".to_string()),
                source: Box::new(e),
            })?;

        let tracer = provider.tracer("hwhkit");
        let otel_layer = OpenTelemetryLayer::new(tracer);

        let registry = Registry::default().with(filter).with(otel_layer);
        let _ = match format {
            ResolvedFormat::Json => registry.with(fmt::layer().json()).try_init(),
            ResolvedFormat::Pretty => registry
                .with(
                    fmt::layer()
                        .with_target(false)
                        .with_file(false)
                        .with_line_number(false),
                )
                .try_init(),
        };

        Ok(OtelGuard {
            _provider: provider,
        })
    }

    /// Holder that flushes the OTLP pipeline on drop.
    pub struct OtelGuard {
        _provider: opentelemetry_sdk::trace::TracerProvider,
    }

    impl Drop for OtelGuard {
        fn drop(&mut self) {
            opentelemetry::global::shutdown_tracer_provider();
        }
    }
}

#[cfg(not(feature = "otel"))]
pub mod otel_layer {
    //! Stub module when the `otel` feature is disabled.

    use super::*;
    /// Empty placeholder — same shape as the real guard so callers don't
    /// need to feature-gate at the use-site.
    pub struct OtelGuard;
    /// Stub when the `otel` feature is disabled. Returns
    /// [`ObservabilityError::OtelDisabled`] so callers can fall back to
    /// plain logging.
    pub fn init_with_otel(
        _: &LoggingConfig,
        _: &OtelConfig,
    ) -> Result<OtelGuard, ObservabilityError> {
        Err(ObservabilityError::OtelDisabled)
    }
}

#[cfg(feature = "otel-sqlx")]
pub mod sqlx_instrument;

#[cfg(feature = "otel-redis")]
pub mod redis_instrument;

#[cfg(feature = "otel-reqwest")]
pub mod reqwest_instrument;
