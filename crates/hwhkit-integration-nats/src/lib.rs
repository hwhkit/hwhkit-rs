//! HwhKit NATS / JetStream integration.
//!
//! Wires a connected `async_nats::Client` and a derived JetStream
//! context into the bootstrap `AppContext` and exposes a readiness
//! probe based on the underlying connection state.

#![warn(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::{self, Context as JetStreamContext};
use async_nats::Client;
use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use serde::{Deserialize, Serialize};

/// Standalone NATS section schema, mirrored from
/// `hwhkit_config::NatsIntegrationConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NatsConfig {
    /// Whether the integration should be initialised at bootstrap.
    pub enabled: bool,
    /// `nats://` or `tls://` connection URL.
    pub url: String,
}

/// Cheap-to-clone handle holding a connected NATS client and a JetStream
/// context derived from it. `async_nats::Client` is internally
/// `Arc`-backed. Fields are private — use [`Self::client`],
/// [`Self::jetstream`], [`Self::url`].
#[derive(Clone)]
#[non_exhaustive]
pub struct NatsHandle {
    url: String,
    client: Client,
    jetstream: JetStreamContext,
    op_timeout: Duration,
    shutdown_timeout: Duration,
}

impl std::fmt::Debug for NatsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsHandle")
            .field("url", &self.url)
            .finish()
    }
}

impl NatsHandle {
    /// Borrow the underlying `async_nats::Client`.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Borrow the JetStream `Context` derived from this client.
    /// JetStream features (streams, consumers, KV, …) live behind this
    /// type.
    pub fn jetstream(&self) -> &JetStreamContext {
        &self.jetstream
    }

    /// Connection URL the client was opened against.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Configured per-operation timeout (from `resilience.op_timeout_ms`).
    /// Use to wrap publish/subscribe/request futures:
    /// `tokio::time::timeout(handle.op_timeout(), client.request(...))`.
    pub fn op_timeout(&self) -> Duration {
        self.op_timeout
    }
}

/// `IntegrationProvider` impl for NATS. Register an instance of this
/// with the bootstrap pipeline to bring up an `async_nats::Client` from
/// the `[integrations.messaging.nats]` config section.
#[derive(Debug, Default)]
pub struct NatsProvider;

const KEY: &str = "nats";

fn validate_url(url: &str) -> CoreResult<()> {
    if !url.starts_with("nats://") && !url.starts_with("tls://") {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::InvalidUrl,
            "nats url must start with nats:// or tls://",
        ));
    }
    Ok(())
}

#[async_trait]
impl IntegrationProvider for NatsProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.messaging.nats.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.messaging.nats.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let nats_cfg = &cfg.integrations.messaging.nats;
        validate_url(&nats_cfg.url)?;

        // Bound the initial connect.
        let client = tokio::time::timeout(
            nats_cfg.resilience.connect_timeout(),
            async_nats::connect(&nats_cfg.url),
        )
        .await
        .map_err(|_| {
            CoreError::integration_msg(
                KEY,
                IntegrationFailureKind::Timeout,
                format!(
                    "nats connect exceeded connect_timeout_ms = {}",
                    nats_cfg.resilience.connect_timeout_ms
                ),
            )
        })?
        .map_err(|e| CoreError::integration(KEY, classify_nats_connect_error(&e), e))?;

        // Verify reachability with a bounded flush.
        tokio::time::timeout(nats_cfg.resilience.op_timeout(), client.flush())
            .await
            .map_err(|_| {
                CoreError::integration_msg(
                    KEY,
                    IntegrationFailureKind::Timeout,
                    "nats smoke-test flush exceeded op_timeout_ms",
                )
            })?
            .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::Other, e))?;

        let jetstream = jetstream::new(client.clone());

        // F8: probe JetStream availability at init time. We always build
        // a JetStream context (so `NatsHandle::jetstream()` is always
        // callable), but if the server was started without `--jetstream`
        // every JS call will fail at runtime with an opaque error. A
        // bounded `query_account` probe lets us surface that
        // misconfiguration *now*, at bootstrap, instead of at first use.
        //
        // The probe is advisory — failure logs a warning but does not
        // fail `init`. Reasons:
        //   - Some deployments only use core NATS pub/sub and intentionally
        //     don't enable JetStream.
        //   - JetStream support can be added later without restarting our
        //     process (the context reconnects automatically).
        //
        // If you require JetStream, gate startup on this check at the
        // application layer using `handle.jetstream().query_account()`.
        let probe = jetstream.query_account();
        match tokio::time::timeout(nats_cfg.resilience.op_timeout(), probe).await {
            Ok(Ok(_)) => {
                tracing::debug!(integration = KEY, "JetStream available on server");
            }
            Ok(Err(err)) => {
                tracing::warn!(
                    integration = KEY,
                    error = %err,
                    "JetStream probe failed at init; calls via `handle.jetstream()` \
                     will error at runtime. If you only use core NATS pub/sub this \
                     is benign; otherwise restart the server with `--jetstream`."
                );
            }
            Err(_) => {
                tracing::warn!(
                    integration = KEY,
                    "JetStream probe exceeded op_timeout_ms — likely unreachable or disabled"
                );
            }
        }

        ctx.insert(NatsHandle {
            url: nats_cfg.url.clone(),
            client,
            jetstream,
            op_timeout: nats_cfg.resilience.op_timeout(),
            shutdown_timeout: nats_cfg.resilience.shutdown_timeout(),
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<NatsHandle>()?;
        Some(Arc::new(NatsHealthCheck {
            handle: (*handle).clone(),
            required: cfg.integrations.messaging.nats.required,
            probe_timeout: cfg.integrations.messaging.nats.resilience.probe_timeout(),
        }))
    }

    async fn shutdown(&self, ctx: &AppContext) -> CoreResult<()> {
        if let Some(handle) = ctx.get::<NatsHandle>() {
            // Push any buffered publishes to the server before tearing
            // down so we do not lose at-most-once messages on shutdown.
            // Bounded by shutdown_timeout so a hung server can't trap
            // the drain loop.
            let budget = handle.shutdown_timeout;
            match tokio::time::timeout(budget, handle.client.flush()).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tracing::warn!(error = %err, "nats: flush during shutdown failed"),
                Err(_) => tracing::warn!(
                    integration = KEY,
                    budget_ms = budget.as_millis() as u64,
                    "nats flush during shutdown exceeded shutdown_timeout_ms; forcing drop"
                ),
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct NatsHealthCheck {
    handle: NatsHandle,
    required: bool,
    probe_timeout: Duration,
}

#[async_trait]
impl HealthCheck for NatsHealthCheck {
    fn name(&self) -> &str {
        "nats"
    }
    fn required(&self) -> bool {
        self.required
    }
    async fn check(&self) -> std::result::Result<(), String> {
        // Audit F7 fix: previously this returned Ok based on
        // `client.connection_state()` — the client's *local cached*
        // view of the connection. A zombie process holding a stale
        // socket would report Healthy until the OS killed the FD.
        //
        // Now we do a real `flush()` roundtrip bounded by
        // probe_timeout. A reachable server acknowledges the flush
        // immediately; a wedged or partitioned server times out.
        match tokio::time::timeout(self.probe_timeout, self.handle.client.flush()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format!("flush failed: {e}")),
            Err(_) => Err(format!(
                "probe exceeded probe_timeout_ms = {}",
                self.probe_timeout.as_millis()
            )),
        }
    }
}

/// Map a `async_nats::ConnectError` to the corresponding
/// [`IntegrationFailureKind`].
///
/// `async_nats` 0.35 doesn't expose its internal error variants
/// publicly, so we walk the [`std::error::Error::source`] chain looking
/// for an `io::Error` with `ErrorKind::TimedOut`. This is robust to the
/// crate moving variants around in patch releases.
fn classify_nats_connect_error(err: &async_nats::ConnectError) -> IntegrationFailureKind {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::TimedOut {
                return IntegrationFailureKind::Timeout;
            }
        }
        current = e.source();
    }
    IntegrationFailureKind::ConnectionRefused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_nats_url() {
        assert!(validate_url("http://localhost:4222").is_err());
        assert!(validate_url("redis://localhost:6379").is_err());
        assert!(validate_url("").is_err());
    }

    #[test]
    fn accepts_nats_url_schemes() {
        assert!(validate_url("nats://127.0.0.1:4222").is_ok());
        assert!(validate_url("tls://nats.example.com:4222").is_ok());
    }
}
