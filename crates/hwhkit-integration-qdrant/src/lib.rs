use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use qdrant_client::Qdrant;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QdrantConfig {
    pub enabled: bool,
    pub url: String,
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
    pub fn client(&self) -> &Qdrant {
        &self.client
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }
}

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
        client.list_collections().await.map_err(|e| {
            CoreError::integration(KEY, IntegrationFailureKind::ConnectionRefused, e)
        })?;

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
