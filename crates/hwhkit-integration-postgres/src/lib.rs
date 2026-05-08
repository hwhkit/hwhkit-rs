use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PostgresConfig {
    pub enabled: bool,
    pub url: String,
    pub max_connections: u32,
}

/// Cheap-to-clone handle around a real `sqlx::PgPool`.
///
/// `PgPool` is itself an `Arc`-backed pool, so cloning the handle is cheap
/// and safe to share across tasks. Fields are private — use [`Self::pool`],
/// [`Self::url`], and [`Self::max_connections`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PostgresHandle {
    url: String,
    max_connections: u32,
    pool: PgPool,
}

impl PostgresHandle {
    /// Returns a reference to the underlying `sqlx::PgPool`.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn max_connections(&self) -> u32 {
        self.max_connections
    }
}

#[derive(Debug, Default)]
pub struct PostgresProvider;

const KEY: &str = "postgres";

fn validate_url(url: &str) -> CoreResult<()> {
    if !url.starts_with("postgres://") && !url.starts_with("postgresql://") {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::InvalidUrl,
            "postgres url must start with postgres:// or postgresql://",
        ));
    }
    Ok(())
}

#[async_trait]
impl IntegrationProvider for PostgresProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.sql.postgres.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.sql.postgres.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let postgres = &cfg.integrations.sql.postgres;
        validate_url(&postgres.url)?;

        let pool = PgPoolOptions::new()
            .max_connections(postgres.max_connections.max(1))
            .connect(&postgres.url)
            .await
            .map_err(|e| {
                CoreError::integration(KEY, IntegrationFailureKind::ConnectionRefused, e)
            })?;

        // Smoke test the connection.
        sqlx::query("SELECT 1").execute(&pool).await.map_err(|e| {
            CoreError::integration(KEY, IntegrationFailureKind::ConnectionRefused, e)
        })?;

        if postgres.migrations.run_on_start {
            run_migrations(&pool, &postgres.migrations.path).await?;
        }

        ctx.insert(PostgresHandle {
            url: postgres.url.clone(),
            max_connections: postgres.max_connections,
            pool,
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<PostgresHandle>()?;
        Some(Arc::new(PostgresHealthCheck {
            handle: PostgresHandle::clone(&handle),
            required: cfg.integrations.sql.postgres.required,
        }))
    }

    async fn shutdown(&self, ctx: &AppContext) -> CoreResult<()> {
        if let Some(handle) = ctx.get::<PostgresHandle>() {
            // `PgPool::close` waits for inflight queries to settle and
            // proactively releases sockets, which is friendlier to a
            // load balancer than dropping references.
            handle.pool.close().await;
        }
        Ok(())
    }
}

async fn run_migrations(pool: &PgPool, path: &str) -> CoreResult<()> {
    let migrations_path = PathBuf::from(path);
    if !migrations_path.exists() {
        tracing::warn!(
            path = %migrations_path.display(),
            "migrations.run_on_start enabled but migrations path does not exist; skipping"
        );
        return Ok(());
    }

    let migrator = sqlx::migrate::Migrator::new(migrations_path.clone())
        .await
        .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::Misconfigured, e))?;

    migrator
        .run(pool)
        .await
        .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::Other, e))?;

    Ok(())
}

#[derive(Clone)]
struct PostgresHealthCheck {
    handle: PostgresHandle,
    required: bool,
}

#[async_trait]
impl HealthCheck for PostgresHealthCheck {
    fn name(&self) -> &str {
        "postgres"
    }

    fn required(&self) -> bool {
        self.required
    }

    async fn check(&self) -> std::result::Result<(), String> {
        sqlx::query("SELECT 1")
            .execute(&self.handle.pool)
            .await
            .map_err(|e| format!("SELECT 1 failed: {e}"))
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_postgres_url() {
        assert!(validate_url("mysql://localhost").is_err());
        assert!(validate_url("http://localhost:5432").is_err());
        assert!(validate_url("").is_err());
    }

    #[test]
    fn accepts_postgres_url_schemes() {
        assert!(validate_url("postgres://user:pw@localhost:5432/db").is_ok());
        assert!(validate_url("postgresql://user:pw@localhost:5432/db").is_ok());
    }

    #[test]
    fn invalid_url_is_classified() {
        let err = validate_url("nope://x").unwrap_err();
        match err {
            CoreError::Integration { name, kind, .. } => {
                assert_eq!(name, "postgres");
                assert!(matches!(kind, IntegrationFailureKind::InvalidUrl));
            }
            _ => panic!("expected Integration variant"),
        }
    }
}
