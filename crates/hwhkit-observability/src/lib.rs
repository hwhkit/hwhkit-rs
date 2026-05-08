//! Logging + tracing setup for hwhkit services.
//!
//! - `init_logging` accepts `format = "auto" | "json" | "pretty"`.
//!   `auto` picks JSON when stdout is not a TTY (production / containers)
//!   and pretty when running interactively.
//! - With the `otel` feature enabled, `init_with_otel` wires an OTLP gRPC
//!   exporter into the `tracing` subscriber so every span is shipped to
//!   the configured collector.

use serde::{Deserialize, Serialize};
use tracing_subscriber::{fmt, prelude::*, EnvFilter, Registry};

/// Standalone logging configuration for callers that drive
/// [`init_logging`] directly without going through the
/// [`hwhkit_config::AppConfig`] pipeline.
///
/// **The canonical type lives in `hwhkit_config::LoggingConfig`** and is
/// what the bootstrap pipeline consumes. This type is intentionally kept
/// in sync (same `level`/`format` fields) so the two can be converted
/// trivially. Prefer the config-crate one when wiring through the
/// framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoggingConfig {
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
    pub fn pretty(level: impl Into<String>) -> Self {
        Self {
            level: level.into(),
            format: "pretty".to_string(),
        }
    }
    pub fn json(level: impl Into<String>) -> Self {
        Self {
            level: level.into(),
            format: "json".to_string(),
        }
    }
}

/// Standalone OTLP/OTel configuration for callers that drive
/// [`init_with_otel`] directly. Mirrors `hwhkit_config::OtelConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OtelConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub service_name: String,
    pub service_version: String,
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

fn make_filter(level: &str) -> Result<EnvFilter, String> {
    EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .map_err(|e| e.to_string())
}

/// Initialize tracing logging without OTel. Safe to call once per process.
pub fn init_logging(config: &LoggingConfig) -> Result<(), String> {
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
    ) -> Result<OtelGuard, String> {
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
            .map_err(|e| format!("failed to install OTLP pipeline: {e}"))?;

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
    use super::*;
    pub struct OtelGuard;
    /// Stub when the `otel` feature is disabled. Returns an error so callers
    /// can fall back to plain logging.
    pub fn init_with_otel(_: &LoggingConfig, _: &OtelConfig) -> Result<OtelGuard, String> {
        Err("hwhkit-observability built without `otel` feature".to_string())
    }
}

#[cfg(feature = "otel-sqlx")]
pub mod sqlx_instrument;

#[cfg(feature = "otel-redis")]
pub mod redis_instrument;

#[cfg(feature = "otel-reqwest")]
pub mod reqwest_instrument;
