use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, Error as CoreError, IntegrationProvider, Result as CoreResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoDbConfig {
    pub enabled: bool,
    pub url: String,
    pub database: String,
}

#[derive(Debug, Clone)]
pub struct MongoDbHandle {
    pub url: String,
    pub database: String,
}

#[derive(Debug, Default)]
pub struct MongoDbProvider;

#[async_trait]
impl IntegrationProvider for MongoDbProvider {
    fn key(&self) -> &'static str {
        "mongodb"
    }

    fn feature(&self) -> &'static str {
        "mongodb"
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.mongodb.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.mongodb.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let mongodb = &cfg.integrations.mongodb;
        if !mongodb.url.starts_with("mongodb://") && !mongodb.url.starts_with("mongodb+srv://") {
            return Err(CoreError::Integration {
                integration: self.key().to_string(),
                reason: "mongodb url must start with mongodb:// or mongodb+srv://".to_string(),
            });
        }

        ctx.insert(MongoDbHandle {
            url: mongodb.url.clone(),
            database: mongodb.database.clone(),
        });

        Ok(())
    }
}
