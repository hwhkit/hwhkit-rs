//! HwhKit MongoDB integration.
//!
//! Wires a `mongodb::Client` into the bootstrap `AppContext` and
//! registers a readiness probe that runs `db.adminCommand({ping: 1})`.

#![warn(missing_docs)]

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

/// Standalone MongoDB section schema, mirrored from
/// `hwhkit_config::MongoDbIntegrationConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MongoDbConfig {
    /// Whether the integration should be initialised at bootstrap.
    pub enabled: bool,
    /// `mongodb://` or `mongodb+srv://` connection URL.
    pub url: String,
    /// Name of the default database the handle's `database()` accessor
    /// returns.
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
    /// Borrow the underlying `mongodb::Client`.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Returns a fresh `mongodb::Database` handle bound to the
    /// configured default database. The returned value is internally
    /// `Arc`-backed; this is *not* a long-lived getter.
    pub fn database(&self) -> mongodb::Database {
        self.client.database(&self.database)
    }

    /// Name of the configured default database (the one `database()`
    /// returns). Useful for logging / connection-string display.
    pub fn database_name(&self) -> &str {
        &self.database
    }

    /// Connection URL the client was opened against.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// `IntegrationProvider` impl for MongoDB. Register an instance of this
/// with the bootstrap pipeline to bring up a `mongodb::Client` from the
/// `[integrations.mongodb]` config section.
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
            .map_err(|e| CoreError::integration(KEY, classify_mongo_error(&e), e))?;

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

/// Map a `mongodb::error::Error` to the corresponding
/// [`IntegrationFailureKind`].
///
/// `Io` errors carrying `io::ErrorKind::TimedOut` are surfaced as
/// [`IntegrationFailureKind::Timeout`]; everything else falls through to
/// [`IntegrationFailureKind::ConnectionRefused`] (preserving prior
/// behaviour for non-timeout failures).
fn classify_mongo_error(err: &mongodb::error::Error) -> IntegrationFailureKind {
    use mongodb::error::ErrorKind;
    if let ErrorKind::Io(io_err) = &*err.kind {
        if io_err.kind() == std::io::ErrorKind::TimedOut {
            return IntegrationFailureKind::Timeout;
        }
    }
    IntegrationFailureKind::ConnectionRefused
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

    #[test]
    fn classify_mongo_io_timeout_to_timeout() {
        let io = std::io::Error::from(std::io::ErrorKind::TimedOut);
        // mongodb::error::Error has a From<std::io::Error> impl that
        // wraps it as `ErrorKind::Io(...)` — exactly the path our
        // classifier targets.
        let mongo_err: mongodb::error::Error = io.into();
        assert!(matches!(
            classify_mongo_error(&mongo_err),
            IntegrationFailureKind::Timeout
        ));
    }
}
