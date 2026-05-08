use std::sync::Arc;

use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RedisConfig {
    pub enabled: bool,
    pub url: String,
}

/// Cheap-to-clone handle that exposes both a `redis::Client` and a managed
/// async connection (`ConnectionManager`). The connection manager
/// auto-reconnects. Fields are private — use [`Self::client`],
/// [`Self::manager`], [`Self::url`].
#[derive(Clone)]
#[non_exhaustive]
pub struct RedisHandle {
    url: String,
    client: redis::Client,
    manager: ConnectionManager,
}

impl std::fmt::Debug for RedisHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisHandle")
            .field("url", &self.url)
            .finish()
    }
}

impl RedisHandle {
    pub fn client(&self) -> &redis::Client {
        &self.client
    }

    pub fn manager(&self) -> ConnectionManager {
        self.manager.clone()
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

#[derive(Debug, Default)]
pub struct RedisProvider;

const KEY: &str = "redis";

fn validate_url(url: &str) -> CoreResult<()> {
    if !url.starts_with("redis://") && !url.starts_with("rediss://") {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::InvalidUrl,
            "redis url must start with redis:// or rediss://",
        ));
    }
    Ok(())
}

#[async_trait]
impl IntegrationProvider for RedisProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.redis.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.redis.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let redis_cfg = &cfg.integrations.redis;
        validate_url(&redis_cfg.url)?;

        let client = redis::Client::open(redis_cfg.url.as_str())
            .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::InvalidUrl, e))?;

        let mut manager = ConnectionManager::new(client.clone()).await.map_err(|e| {
            CoreError::integration(KEY, IntegrationFailureKind::ConnectionRefused, e)
        })?;

        // Ping to verify reachability.
        let pong: String = redis::cmd("PING")
            .query_async(&mut manager)
            .await
            .map_err(|e| {
                CoreError::integration(KEY, IntegrationFailureKind::ConnectionRefused, e)
            })?;

        if pong.to_uppercase() != "PONG" {
            return Err(CoreError::integration_msg(
                KEY,
                IntegrationFailureKind::Other,
                format!("unexpected PING response: {pong}"),
            ));
        }

        ctx.insert(RedisHandle {
            url: redis_cfg.url.clone(),
            client,
            manager,
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<RedisHandle>()?;
        Some(Arc::new(RedisHealthCheck {
            handle: (*handle).clone(),
            required: cfg.integrations.redis.required,
        }))
    }

    async fn shutdown(&self, _ctx: &AppContext) -> CoreResult<()> {
        // The redis crate's `ConnectionManager` does not expose an
        // explicit close. Dropping the references inside `AppContext`
        // happens automatically when the application is dropped; we
        // simply log a confirmation here so operators can correlate.
        tracing::info!("redis: shutdown hook invoked (manager will drop with context)");
        Ok(())
    }
}

/// Health check for the Redis integration. The probe issues a single
/// `PING` against the *shared* multiplexed [`ConnectionManager`] held
/// in [`RedisHandle`]; it does not open a new connection. That keeps
/// the readiness probe cheap, but means a single broken connection in
/// the manager will fail the probe even when the service itself is
/// reachable from other connections.
#[derive(Clone)]
struct RedisHealthCheck {
    handle: RedisHandle,
    required: bool,
}

#[async_trait]
impl HealthCheck for RedisHealthCheck {
    fn name(&self) -> &str {
        "redis"
    }
    fn required(&self) -> bool {
        self.required
    }
    async fn check(&self) -> std::result::Result<(), String> {
        let mut conn = self.handle.manager.clone();
        let pong: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|e| format!("PING failed: {e}"))?;
        if pong.to_uppercase() != "PONG" {
            return Err(format!("unexpected PING response: {pong}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_redis_url() {
        assert!(validate_url("http://localhost:6379").is_err());
        assert!(validate_url("memcached://localhost:11211").is_err());
        assert!(validate_url("").is_err());
    }

    #[test]
    fn accepts_redis_url_schemes() {
        assert!(validate_url("redis://localhost:6379").is_ok());
        assert!(validate_url("rediss://user:pw@localhost:6380/0").is_ok());
    }
}
