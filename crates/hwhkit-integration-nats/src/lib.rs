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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NatsConfig {
    pub enabled: bool,
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
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn jetstream(&self) -> &JetStreamContext {
        &self.jetstream
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

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

        let client = async_nats::connect(&nats_cfg.url).await.map_err(|e| {
            CoreError::integration(KEY, IntegrationFailureKind::ConnectionRefused, e)
        })?;

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
