//! HwhKit NATS / JetStream integration.
//!
//! Wires a connected `async_nats::Client` and a derived JetStream
//! context into the bootstrap `AppContext` and exposes a readiness
//! probe based on the underlying connection state.

#![warn(missing_docs)]

use std::sync::Arc;

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

        let client = async_nats::connect(&nats_cfg.url)
            .await
            .map_err(|e| CoreError::integration(KEY, classify_nats_connect_error(&e), e))?;

        // Verify the connection is alive by flushing any pending messages.
        client
            .flush()
            .await
            .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::Other, e))?;

        let jetstream = jetstream::new(client.clone());

        ctx.insert(NatsHandle {
            url: nats_cfg.url.clone(),
            client,
            jetstream,
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<NatsHandle>()?;
        Some(Arc::new(NatsHealthCheck {
            handle: (*handle).clone(),
            required: cfg.integrations.messaging.nats.required,
        }))
    }

    async fn shutdown(&self, ctx: &AppContext) -> CoreResult<()> {
        if let Some(handle) = ctx.get::<NatsHandle>() {
            // Push any buffered publishes to the server before tearing
            // down so we do not lose at-most-once messages on shutdown.
            // `async_nats` 0.35 does not expose an explicit `drain` on the
            // client; flushing is the strongest portable shutdown signal
            // — the runtime then drops its `Client` reference, which
            // closes the underlying TCP/TLS connection.
            if let Err(err) = handle.client.flush().await {
                tracing::warn!(error = %err, "nats: flush during shutdown failed");
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct NatsHealthCheck {
    handle: NatsHandle,
    required: bool,
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
        match self.handle.client.connection_state() {
            async_nats::connection::State::Connected => Ok(()),
            other => Err(format!("nats connection not ready: {other:?}")),
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
