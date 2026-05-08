use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use neo4rs::{ConfigBuilder, Graph};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Neo4jConfig {
    pub enabled: bool,
    pub url: String,
    pub username: String,
    pub password: String,
}

/// Cheap-to-clone handle wrapping a `neo4rs::Graph` connection pool in an
/// `Arc`. Fields are private — use [`Self::graph`], [`Self::url`],
/// [`Self::username`].
#[derive(Clone)]
#[non_exhaustive]
pub struct Neo4jHandle {
    url: String,
    username: String,
    graph: Arc<Graph>,
}

impl std::fmt::Debug for Neo4jHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Neo4jHandle")
            .field("url", &self.url)
            .field("username", &self.username)
            .finish()
    }
}

impl Neo4jHandle {
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn username(&self) -> &str {
        &self.username
    }
}

#[derive(Debug, Default)]
pub struct Neo4jProvider;

const KEY: &str = "neo4j";

fn validate(url: &str, username: &str) -> CoreResult<()> {
    if !url.starts_with("bolt://")
        && !url.starts_with("bolt+s://")
        && !url.starts_with("neo4j://")
        && !url.starts_with("neo4j+s://")
    {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::InvalidUrl,
            "neo4j url must start with bolt://, bolt+s://, neo4j://, or neo4j+s://",
        ));
    }
    if username.trim().is_empty() {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::Misconfigured,
            "neo4j username cannot be empty",
        ));
    }
    Ok(())
}

#[async_trait]
impl IntegrationProvider for Neo4jProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.neo4j.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.neo4j.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let neo4j = &cfg.integrations.neo4j;
        validate(&neo4j.url, &neo4j.username)?;

        let config = ConfigBuilder::default()
            .uri(neo4j.url.as_str())
            .user(neo4j.username.as_str())
            .password(neo4j.password.as_str())
            .build()
            .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::Misconfigured, e))?;

        let graph = Graph::connect(config).await.map_err(|e| {
            CoreError::integration(KEY, IntegrationFailureKind::ConnectionRefused, e)
        })?;

        // Live `RETURN 1` ping. `Graph::run` consumes the query and
        // resolves once Neo4j has acknowledged the statement; if the
        // server is unreachable or the credentials are wrong we surface
        // a precise error rather than waiting for the first real query.
        graph
            .run(neo4rs::query("RETURN 1"))
            .await
            .map_err(|e| CoreError::integration(KEY, IntegrationFailureKind::AuthFailed, e))?;

        ctx.insert(Neo4jHandle {
            url: neo4j.url.clone(),
            username: neo4j.username.clone(),
            graph: Arc::new(graph),
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<Neo4jHandle>()?;
        Some(Arc::new(Neo4jHealthCheck {
            graph: handle.graph.clone(),
            required: cfg.integrations.neo4j.required,
        }))
    }
}

struct Neo4jHealthCheck {
    graph: Arc<Graph>,
    required: bool,
}

#[async_trait]
impl HealthCheck for Neo4jHealthCheck {
    fn name(&self) -> &str {
        "neo4j"
    }
    fn required(&self) -> bool {
        self.required
    }
    async fn check(&self) -> std::result::Result<(), String> {
        // neo4rs exposes Graph::run for fire-and-forget queries.
        self.graph
            .run(neo4rs::query("RETURN 1"))
            .await
            .map(|_| ())
            .map_err(|e| format!("RETURN 1 failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_neo4j_url() {
        assert!(validate("http://localhost:7687", "neo4j").is_err());
        assert!(validate("graph://localhost", "neo4j").is_err());
        assert!(validate("", "neo4j").is_err());
    }

    #[test]
    fn rejects_empty_username() {
        assert!(validate("bolt://localhost:7687", "").is_err());
        assert!(validate("bolt://localhost:7687", "  ").is_err());
    }

    #[test]
    fn accepts_bolt_and_neo4j_schemes() {
        assert!(validate("bolt://localhost:7687", "neo4j").is_ok());
        assert!(validate("bolt+s://localhost:7687", "neo4j").is_ok());
        assert!(validate("neo4j://localhost:7687", "neo4j").is_ok());
        assert!(validate("neo4j+s://localhost:7687", "neo4j").is_ok());
    }
}
