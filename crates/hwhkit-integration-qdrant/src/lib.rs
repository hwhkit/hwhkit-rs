//! HwhKit Qdrant vector-database integration.
//!
//! Wires a `qdrant_client::Qdrant` client into the bootstrap
//! `AppContext` and exposes a `list_collections`-based readiness probe.

#![warn(missing_docs)]

use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use qdrant_client::Qdrant;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Standalone Qdrant section schema, mirrored from
/// `hwhkit_config::QdrantIntegrationConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QdrantConfig {
    /// Whether the integration should be initialised at bootstrap.
    pub enabled: bool,
    /// `http://` / `https://` URL of the Qdrant REST/gRPC endpoint.
    pub url: String,
    /// Optional API key — sent as the `api-key` header.
    pub api_key: Option<String>,
}

/// Cheap-to-clone handle wrapping a `qdrant_client::Qdrant` client in an
/// `Arc`. Fields are private — use [`Self::client`], [`Self::url`].
#[derive(Clone)]
#[non_exhaustive]
pub struct QdrantHandle {
    url: String,
    api_key: Option<String>,
    client: Arc<Qdrant>,
}

impl std::fmt::Debug for QdrantHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QdrantHandle")
            .field("url", &self.url)
            .field("has_api_key", &self.api_key.is_some())
            .finish()
    }
}

impl QdrantHandle {
    /// Borrow the underlying `qdrant_client::Qdrant` client.
    pub fn client(&self) -> &Qdrant {
        &self.client
    }

    /// URL the client was opened against.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Whether an API key was configured. The key itself is intentionally
    /// not exposed via an accessor.
    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }
}

/// `IntegrationProvider` impl for Qdrant. Register an instance of this
/// with the bootstrap pipeline to bring up a `qdrant_client::Qdrant`
/// from the `[integrations.vector.qdrant]` config section.
#[derive(Debug, Default)]
pub struct QdrantProvider;

const KEY: &str = "qdrant";

fn validate_url(url: &str) -> CoreResult<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::InvalidUrl,
            "qdrant url must start with http:// or https://",
        ));
    }
    Ok(())
}

#[async_trait]
impl IntegrationProvider for QdrantProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.vector.qdrant.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.vector.qdrant.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let qdrant_cfg = &cfg.integrations.vector.qdrant;
        validate_url(&qdrant_cfg.url)?;

        let api_key = if qdrant_cfg.api_key.trim().is_empty() {
            None
        } else {
            Some(qdrant_cfg.api_key.clone())
        };

        let mut builder = Qdrant::from_url(&qdrant_cfg.url);
        if let Some(key) = api_key.as_ref() {
            builder = builder.api_key(key.clone());
        }
        let client = builder
            .build()
            .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::Misconfigured, e))?;

        // Verify reachability by listing collections.
        client
            .list_collections()
            .await
            .map_err(|e| CoreError::integration(KEY, classify_qdrant_error(&e), e))?;

        ctx.insert(QdrantHandle {
            url: qdrant_cfg.url.clone(),
            api_key,
            client: Arc::new(client),
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<QdrantHandle>()?;
        Some(Arc::new(QdrantHealthCheck {
            client: handle.client.clone(),
            required: cfg.integrations.vector.qdrant.required,
        }))
    }
}

struct QdrantHealthCheck {
    client: Arc<Qdrant>,
    required: bool,
}

#[async_trait]
impl HealthCheck for QdrantHealthCheck {
    fn name(&self) -> &str {
        "qdrant"
    }
    fn required(&self) -> bool {
        self.required
    }
    async fn check(&self) -> std::result::Result<(), String> {
        self.client
            .list_collections()
            .await
            .map(|_| ())
            .map_err(|e| format!("list_collections failed: {e}"))
    }
}

/// Map a `qdrant_client::QdrantError` to the corresponding
/// [`IntegrationFailureKind`].
///
/// qdrant-client surfaces gRPC failures via `tonic::Status` internally,
/// but does not re-export `tonic` for direct downcasting. We instead
/// walk the [`std::error::Error::source`] chain looking for either an
/// `io::Error` of kind `TimedOut` or a status string indicating
/// `DeadlineExceeded` — both signal a true timeout. Falls back to
/// [`IntegrationFailureKind::ConnectionRefused`] otherwise.
fn classify_qdrant_error(err: &qdrant_client::QdrantError) -> IntegrationFailureKind {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::TimedOut {
                return IntegrationFailureKind::Timeout;
            }
        }
        // `tonic::Status` Display includes its `code` (e.g.
        // "status: DeadlineExceeded ..."); a substring match keeps us
        // free of a direct tonic dependency.
        let display = e.to_string();
        if display.contains("DeadlineExceeded") || display.contains("deadline exceeded") {
            return IntegrationFailureKind::Timeout;
        }
        current = e.source();
    }
    IntegrationFailureKind::ConnectionRefused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_url() {
        assert!(validate_url("grpc://localhost:6334").is_err());
        assert!(validate_url("qdrant://localhost").is_err());
        assert!(validate_url("").is_err());
    }

    #[test]
    fn accepts_http_url_schemes() {
        assert!(validate_url("http://localhost:6333").is_ok());
        assert!(validate_url("https://qdrant.example.com:6333").is_ok());
    }
}
