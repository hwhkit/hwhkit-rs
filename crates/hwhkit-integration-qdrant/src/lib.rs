use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, Error as CoreError, IntegrationProvider, Result as CoreResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantConfig {
    pub enabled: bool,
    pub url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QdrantHandle {
    pub url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Default)]
pub struct QdrantProvider;

#[async_trait]
impl IntegrationProvider for QdrantProvider {
    fn key(&self) -> &'static str {
        "qdrant"
    }

    fn feature(&self) -> &'static str {
        "qdrant"
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.vector.qdrant.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.vector.qdrant.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let qdrant = &cfg.integrations.vector.qdrant;
        if !qdrant.url.starts_with("http://") && !qdrant.url.starts_with("https://") {
            return Err(CoreError::Integration {
                integration: self.key().to_string(),
                reason: "qdrant url must start with http:// or https://".to_string(),
            });
        }

        let api_key = if qdrant.api_key.trim().is_empty() {
            None
        } else {
            Some(qdrant.api_key.clone())
        };

        ctx.insert(QdrantHandle {
            url: qdrant.url.clone(),
            api_key,
        });

        Ok(())
    }
}
