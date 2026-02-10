use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, Error as CoreError, IntegrationProvider, Result as CoreResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    pub enabled: bool,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct RedisHandle {
    pub url: String,
}

#[derive(Debug, Default)]
pub struct RedisProvider;

#[async_trait]
impl IntegrationProvider for RedisProvider {
    fn key(&self) -> &'static str {
        "redis"
    }

    fn feature(&self) -> &'static str {
        "redis"
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.redis.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.redis.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let redis = &cfg.integrations.redis;
        if !redis.url.starts_with("redis://") {
            return Err(CoreError::Integration {
                integration: self.key().to_string(),
                reason: "redis url must start with redis://".to_string(),
            });
        }

        ctx.insert(RedisHandle {
            url: redis.url.clone(),
        });

        Ok(())
    }
}
