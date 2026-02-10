use std::sync::Arc;

use hwhkit_config::{BootstrapConfig, ConfigLoader};
use hwhkit_core::{
    bootstrap_with, Application, BuiltApplication, IntegrationProvider, Result, RuntimeFeatures,
};

pub fn runtime_features() -> RuntimeFeatures {
    RuntimeFeatures {
        postgres: cfg!(feature = "postgres"),
        redis: cfg!(feature = "redis"),
        mongodb: cfg!(feature = "mongodb"),
        nats: cfg!(feature = "nats"),
        qdrant: cfg!(feature = "qdrant"),
        neo4j: cfg!(feature = "neo4j"),
        transport_grpc: cfg!(feature = "transport-grpc"),
        transport_ws: cfg!(feature = "transport-ws"),
        transport_p2p: cfg!(feature = "transport-p2p"),
    }
}

pub fn default_providers() -> Vec<Arc<dyn IntegrationProvider>> {
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
