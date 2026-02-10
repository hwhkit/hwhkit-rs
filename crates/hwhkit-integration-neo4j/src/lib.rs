use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, Error as CoreError, IntegrationProvider, Result as CoreResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jConfig {
    pub enabled: bool,
    pub url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct Neo4jHandle {
    pub url: String,
    pub username: String,
}

#[derive(Debug, Default)]
pub struct Neo4jProvider;

#[async_trait]
impl IntegrationProvider for Neo4jProvider {
    fn key(&self) -> &'static str {
        "neo4j"
    }

    fn feature(&self) -> &'static str {
        "neo4j"
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.neo4j.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.neo4j.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let neo4j = &cfg.integrations.neo4j;
        if !neo4j.url.starts_with("bolt://") && !neo4j.url.starts_with("neo4j://") {
            return Err(CoreError::Integration {
                integration: self.key().to_string(),
                reason: "neo4j url must start with bolt:// or neo4j://".to_string(),
            });
        }
        if neo4j.username.trim().is_empty() {
            return Err(CoreError::Integration {
                integration: self.key().to_string(),
                reason: "neo4j username cannot be empty".to_string(),
            });
        }

        ctx.insert(Neo4jHandle {
            url: neo4j.url.clone(),
            username: neo4j.username.clone(),
        });

        Ok(())
    }
}
