use std::sync::Arc;

use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use mongodb::bson::doc;
use mongodb::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MongoDbConfig {
    pub enabled: bool,
    pub url: String,
    pub database: String,
}

/// Cheap-to-clone handle wrapping `mongodb::Client` (already `Arc`-backed
/// internally). Fields are private — use [`Self::client`],
/// [`Self::database`], [`Self::url`].
#[derive(Clone)]
#[non_exhaustive]
pub struct MongoDbHandle {
    url: String,
    database: String,
    client: Client,
}

impl std::fmt::Debug for MongoDbHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MongoDbHandle")
            .field("url", &self.url)
            .field("database", &self.database)
            .finish()
    }
}

impl MongoDbHandle {
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn database(&self) -> mongodb::Database {
        self.client.database(&self.database)
    }

    pub fn database_name(&self) -> &str {
        &self.database
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

#[derive(Debug, Default)]
pub struct MongoDbProvider;

const KEY: &str = "mongodb";

fn validate_url(url: &str) -> CoreResult<()> {
    if !url.starts_with("mongodb://") && !url.starts_with("mongodb+srv://") {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::InvalidUrl,
            "mongodb url must start with mongodb:// or mongodb+srv://",
        ));
    }
    Ok(())
}

#[async_trait]
impl IntegrationProvider for MongoDbProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.mongodb.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.mongodb.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let mongo_cfg = &cfg.integrations.mongodb;
        validate_url(&mongo_cfg.url)?;

        let client = Client::with_uri_str(&mongo_cfg.url)
            .await
            .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::InvalidUrl, e))?;

        // Ping admin db to verify reachability.
        client
            .database("admin")
            .run_command(doc! { "ping": 1 }, None)
            .await
            .map_err(|e| {
                CoreError::integration(KEY, IntegrationFailureKind::ConnectionRefused, e)
            })?;

        ctx.insert(MongoDbHandle {
            url: mongo_cfg.url.clone(),
            database: mongo_cfg.database.clone(),
            client,
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<MongoDbHandle>()?;
        Some(Arc::new(MongoDbHealthCheck {
            handle: (*handle).clone(),
            required: cfg.integrations.mongodb.required,
        }))
    }
}

#[derive(Clone)]
struct MongoDbHealthCheck {
    handle: MongoDbHandle,
    required: bool,
}

#[async_trait]
impl HealthCheck for MongoDbHealthCheck {
    fn name(&self) -> &str {
        "mongodb"
    }
    fn required(&self) -> bool {
        self.required
    }
    async fn check(&self) -> std::result::Result<(), String> {
        self.handle
            .client
            .database("admin")
            .run_command(doc! { "ping": 1 }, None)
            .await
            .map(|_| ())
            .map_err(|e| format!("ping failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_mongo_url() {
        assert!(validate_url("http://localhost:27017").is_err());
        assert!(validate_url("mongo://localhost").is_err());
        assert!(validate_url("").is_err());
    }

    #[test]
    fn accepts_mongo_url_schemes() {
        assert!(validate_url("mongodb://localhost:27017").is_ok());
        assert!(validate_url("mongodb+srv://user:pw@cluster.example.com").is_ok());
    }
}
