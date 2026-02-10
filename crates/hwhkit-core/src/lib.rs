use async_trait::async_trait;
use axum::Router;
use hwhkit_config::{AppConfig, BootstrapConfig, ConfigLoader};
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bootstrap error: {0}")]
    Bootstrap(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("feature mismatch: {0}")]
    FeatureMismatch(String),
    #[error("integration error [{integration}]: {reason}")]
    Integration { integration: String, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Default)]
pub struct AppContext {
    values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl AppContext {
    pub fn insert<T>(&mut self, value: T)
    where
        T: Send + Sync + 'static,
    {
        self.values.insert(TypeId::of::<T>(), Arc::new(value));
    }

    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let value = self.values.get(&TypeId::of::<T>())?;
        Arc::clone(value).downcast::<T>().ok()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeFeatures {
    pub postgres: bool,
    pub redis: bool,
    pub mongodb: bool,
    pub nats: bool,
    pub qdrant: bool,
    pub neo4j: bool,
    pub transport_grpc: bool,
    pub transport_ws: bool,
    pub transport_p2p: bool,
}

#[derive(Clone)]
pub struct BuiltApplication {
    pub router: Router,
    pub context: AppContext,
    pub bootstrap: BootstrapConfig,
    pub config: AppConfig,
    pub applied_sources: Vec<String>,
    pub initialized_integrations: Vec<String>,
    pub degraded_integrations: Vec<String>,
}

#[async_trait]
pub trait Application: Send + Sync + 'static {
    async fn build_router(&self, ctx: AppContext, cfg: &AppConfig) -> Result<Router>;
}

#[async_trait]
pub trait IntegrationProvider: Send + Sync {
    fn key(&self) -> &'static str;
    fn feature(&self) -> &'static str;
    fn enabled(&self, cfg: &AppConfig) -> bool;
    fn required(&self, _cfg: &AppConfig) -> bool {
        true
    }
    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> Result<()>;
}

pub async fn bootstrap<A: Application>(
    app: A,
    bootstrap: BootstrapConfig,
) -> Result<BuiltApplication> {
    bootstrap_with(
        app,
        bootstrap,
        ConfigLoader::default(),
        RuntimeFeatures::default(),
        Vec::new(),
    )
    .await
}

pub async fn bootstrap_with<A: Application>(
    app: A,
    bootstrap: BootstrapConfig,
    loader: ConfigLoader,
    runtime_features: RuntimeFeatures,
    providers: Vec<Arc<dyn IntegrationProvider>>,
) -> Result<BuiltApplication> {
    let loaded = loader
        .load(&bootstrap)
        .map_err(|e| Error::Config(e.to_string()))?;

    validate_feature_mapping(&loaded.config, &runtime_features)?;

    let mut ctx = AppContext::default();
    let mut initialized_integrations = Vec::new();
    let mut degraded_integrations = Vec::new();

    for provider in providers {
        if !provider.enabled(&loaded.config) {
            continue;
        }

        let key = provider.key().to_string();
        match provider.init(&mut ctx, &loaded.config).await {
            Ok(_) => initialized_integrations.push(key),
            Err(err) => {
                if provider.required(&loaded.config) {
                    return Err(Error::Integration {
                        integration: key,
                        reason: err.to_string(),
                    });
                }
                degraded_integrations.push(key);
            }
        }
    }

    let router = app.build_router(ctx.clone(), &loaded.config).await?;

    Ok(BuiltApplication {
        router,
        context: ctx,
        bootstrap,
        config: loaded.config,
        applied_sources: loaded.applied_sources,
        initialized_integrations,
        degraded_integrations,
    })
}

pub fn validate_feature_mapping(
    config: &AppConfig,
    runtime_features: &RuntimeFeatures,
) -> Result<()> {
    ensure_feature(
        "integrations.sql.postgres.enabled",
        config.integrations.sql.postgres.enabled,
        runtime_features.postgres,
        "postgres",
    )?;
    ensure_feature(
        "integrations.redis.enabled",
        config.integrations.redis.enabled,
        runtime_features.redis,
        "redis",
    )?;
    ensure_feature(
        "integrations.mongodb.enabled",
        config.integrations.mongodb.enabled,
        runtime_features.mongodb,
        "mongodb",
    )?;
    ensure_feature(
        "integrations.messaging.nats.enabled",
        config.integrations.messaging.nats.enabled,
        runtime_features.nats,
        "nats",
    )?;
    ensure_feature(
        "integrations.vector.qdrant.enabled",
        config.integrations.vector.qdrant.enabled,
        runtime_features.qdrant,
        "qdrant",
    )?;
    ensure_feature(
        "integrations.neo4j.enabled",
        config.integrations.neo4j.enabled,
        runtime_features.neo4j,
        "neo4j",
    )?;
    ensure_feature(
        "transport.grpc.enabled",
        config.transport.grpc.enabled,
        runtime_features.transport_grpc,
        "transport-grpc",
    )?;
    ensure_feature(
        "transport.websocket.enabled",
        config.transport.websocket.enabled,
        runtime_features.transport_ws,
        "transport-ws",
    )?;
    ensure_feature(
        "transport.p2p.enabled",
        config.transport.p2p.enabled,
        runtime_features.transport_p2p,
        "transport-p2p",
    )?;

    if config.transport.rpc.enabled
        && config.transport.rpc.default == "grpc"
        && !runtime_features.transport_grpc
    {
        return Err(Error::FeatureMismatch(
            "transport.rpc.default=grpc requires feature `transport-grpc`".to_string(),
        ));
    }
    if config.transport.rpc.enabled
        && config.transport.rpc.default == "nats"
        && !runtime_features.nats
    {
        return Err(Error::FeatureMismatch(
            "transport.rpc.default=nats requires feature `nats`".to_string(),
        ));
    }

    Ok(())
}

fn ensure_feature(
    path: &str,
    enabled: bool,
    feature_enabled: bool,
    feature_name: &str,
) -> Result<()> {
    if enabled && !feature_enabled {
        return Err(Error::FeatureMismatch(format!(
            "{path}=true requires cargo feature `{feature_name}`"
        )));
    }
    Ok(())
}
