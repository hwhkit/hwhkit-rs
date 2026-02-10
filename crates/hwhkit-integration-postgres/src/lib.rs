use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, Error as CoreError, IntegrationProvider, Result as CoreResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    pub enabled: bool,
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone)]
pub struct PostgresHandle {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Default)]
pub struct PostgresProvider;

#[async_trait]
impl IntegrationProvider for PostgresProvider {
    fn key(&self) -> &'static str {
        "postgres"
    }

    fn feature(&self) -> &'static str {
        "postgres"
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.sql.postgres.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.sql.postgres.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let postgres = &cfg.integrations.sql.postgres;
        if !postgres.url.starts_with("postgres://") && !postgres.url.starts_with("postgresql://") {
            return Err(CoreError::Integration {
                integration: self.key().to_string(),
                reason: "postgres url must start with postgres:// or postgresql://".to_string(),
            });
        }

        ctx.insert(PostgresHandle {
            url: postgres.url.clone(),
            max_connections: postgres.max_connections,
        });

        Ok(())
    }
}
