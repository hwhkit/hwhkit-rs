use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, Error as CoreError, IntegrationProvider, Result as CoreResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsConfig {
    pub enabled: bool,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct NatsHandle {
    pub url: String,
}

#[derive(Debug, Default)]
pub struct NatsProvider;

#[async_trait]
impl IntegrationProvider for NatsProvider {
    fn key(&self) -> &'static str {
        "nats"
    }

    fn feature(&self) -> &'static str {
        "nats"
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.messaging.nats.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.messaging.nats.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let nats = &cfg.integrations.messaging.nats;
        if !nats.url.starts_with("nats://") {
            return Err(CoreError::Integration {
                integration: self.key().to_string(),
                reason: "nats url must start with nats://".to_string(),
            });
        }

        ctx.insert(NatsHandle {
            url: nats.url.clone(),
        });

        Ok(())
    }
}
