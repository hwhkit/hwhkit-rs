//! Convenience facade over [`hwhkit_core::bootstrap_with`].
//!
//! Builds the standard [`RuntimeFeatures`] and [`IntegrationProvider`]
//! chain from the cargo features the binary was compiled with, then
//! delegates to `hwhkit_core` for the actual pipeline.

use std::sync::Arc;

use hwhkit_config::{BootstrapConfig, ConfigLoader};
use hwhkit_core::{
    bootstrap_with, Application, BuiltApplication, IntegrationProvider, Result, RuntimeFeatures,
};

pub fn runtime_features() -> RuntimeFeatures {
    RuntimeFeatures::new()
        .enable_if("postgres", cfg!(feature = "postgres"))
        .enable_if("redis", cfg!(feature = "redis"))
        .enable_if("mongodb", cfg!(feature = "mongodb"))
        .enable_if("nats", cfg!(feature = "nats"))
        .enable_if("qdrant", cfg!(feature = "qdrant"))
        .enable_if("neo4j", cfg!(feature = "neo4j"))
        .enable_if("s3", cfg!(feature = "s3"))
        .enable_if("oss", cfg!(feature = "oss"))
}

#[allow(clippy::vec_init_then_push)]
pub fn default_providers() -> Vec<Arc<dyn IntegrationProvider>> {
    #[allow(unused_mut)]
    let mut providers: Vec<Arc<dyn IntegrationProvider>> = Vec::new();

    #[cfg(feature = "postgres")]
    providers.push(Arc::new(hwhkit_integration_postgres::PostgresProvider));
    #[cfg(feature = "redis")]
    providers.push(Arc::new(hwhkit_integration_redis::RedisProvider));
    #[cfg(feature = "mongodb")]
    providers.push(Arc::new(hwhkit_integration_mongodb::MongoDbProvider));
    #[cfg(feature = "nats")]
    providers.push(Arc::new(hwhkit_integration_nats::NatsProvider));
    #[cfg(feature = "qdrant")]
    providers.push(Arc::new(hwhkit_integration_qdrant::QdrantProvider));
    #[cfg(feature = "neo4j")]
    providers.push(Arc::new(hwhkit_integration_neo4j::Neo4jProvider));
    #[cfg(feature = "s3")]
    providers.push(Arc::new(hwhkit_integration_s3::S3Provider));
    #[cfg(feature = "oss")]
    providers.push(Arc::new(hwhkit_integration_oss::OssProvider));

    providers
}

pub async fn run<A: Application>(app: A, bootstrap: BootstrapConfig) -> Result<BuiltApplication> {
    bootstrap_with(
        app,
        bootstrap,
        ConfigLoader::default(),
        runtime_features(),
        default_providers(),
    )
    .await
}

/// One-call OOTB entry point: boots the application, mounts the standard
/// production endpoints + middleware bundle, installs SIGINT/SIGTERM
/// handlers, and serves until shutdown completes.
///
/// Typical usage:
///
/// ```ignore
/// use hwhkit::prelude::*;
/// run_and_serve(MyApp, BootstrapConfig::default()).await?;
/// ```
pub async fn run_and_serve<A: Application>(
    app: A,
    bootstrap: BootstrapConfig,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let built = run(app, bootstrap).await?;
    crate::production::server::run(built)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
}
