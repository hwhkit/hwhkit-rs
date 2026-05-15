//! Core runtime abstractions for `hwhkit` services.
//!
//! This crate exports the central traits, types, and bootstrap pipeline
//! that the framework crates and integrations build on:
//!
//! - [`Application`] — the user-supplied entry point that produces a
//!   [`Router`].
//! - [`IntegrationProvider`] — pluggable connector for an external
//!   resource (Postgres, Redis, …). Trait is intentionally **open**:
//!   future methods must ship with default implementations so adding
//!   one is not a breaking change for downstream impls.
//! - [`AppContext`] — type-keyed slot map handed to handlers.
//! - [`Error`] / [`IntegrationFailureKind`] — hybrid typed-or-boxed
//!   error model (see [`error`]).
//! - [`BuiltApplication`] — outcome of bootstrap. Fields are private;
//!   use the accessor methods.

use async_trait::async_trait;
use axum::Router;
use hwhkit_config::{AppConfig, BootstrapConfig, ConfigLoader};
use std::{
    any::{Any, TypeId},
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

pub mod error;
pub mod error_response;
pub mod health;
#[cfg(feature = "jwt")]
pub mod jwt;
pub mod shutdown;
#[cfg(feature = "multi-tenant")]
pub mod tenant;

pub use error::{BoxError, Error, IntegrationFailureKind};
pub use error_response::{ApiError, ApiResult, FieldError, ProblemDetails};
pub use health::{HealthCheck, HealthCheckResult, HealthRegistry, HealthStatus};
pub use shutdown::ShutdownToken;
#[cfg(feature = "multi-tenant")]
pub use tenant::{TenantId, TenantScope};

/// Convenience alias for `Result<T, Error>`. New crates should import
/// this via `hwhkit_core::Result`; integrations use it as their public
/// result type so all errors flow through the common [`Error`] enum.
pub type Result<T> = std::result::Result<T, Error>;

/// Type-erased Prometheus exporter handle. Stored on
/// [`BuiltApplication`] so its lifetime matches the application; kept
/// as `Arc<dyn Any>` so this crate doesn't need a hard dep on
/// `metrics-exporter-prometheus`.
pub type MetricsHandle = Arc<dyn Any + Send + Sync>;

/// Type-keyed slot map handed to handlers. Insert concrete types via
/// [`Self::insert`]; insert trait-object handles via [`Self::insert_dyn`].
#[derive(Clone, Default)]
pub struct AppContext {
    /// Concrete-type slots keyed by `TypeId::of::<T>()`.
    values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// `dyn Trait`-keyed slots. The key is `TypeId::of::<dyn Trait>()`
    /// (legal because `dyn Trait + 'static` is itself `'static`). The
    /// stored value is an `Arc<dyn Any + Send + Sync>` whose underlying
    /// concrete type is `Arc<dyn Trait>`.
    dyn_values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl AppContext {
    /// Insert a concrete value keyed by its type. Returns the previously
    /// stored value of the same type if any — silent overwrites are still
    /// possible (last write wins) but now observable.
    pub fn insert<T>(&mut self, value: T) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let prior = self.values.insert(TypeId::of::<T>(), Arc::new(value));
        prior.and_then(|v| v.downcast::<T>().ok())
    }

    /// Look up a concrete value by type.
    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let value = self.values.get(&TypeId::of::<T>())?;
        Arc::clone(value).downcast::<T>().ok()
    }

    /// Insert a trait-object handle that downstream code can later look
    /// up via [`Self::get_dyn`]. Use this when you want to expose a
    /// behaviour (e.g. `dyn Cache`) without leaking the concrete type.
    ///
    /// ```ignore
    /// trait Cache: Send + Sync + 'static { /* … */ }
    /// ctx.insert_dyn::<dyn Cache>(my_redis_cache);
    /// let cache = ctx.get_dyn::<dyn Cache>().unwrap();
    /// ```
    pub fn insert_dyn<Trait: ?Sized + Send + Sync + 'static>(&mut self, value: Arc<Trait>) {
        self.dyn_values
            .insert(TypeId::of::<Trait>(), Arc::new(value));
    }

    /// Look up a trait-object handle inserted via [`Self::insert_dyn`].
    pub fn get_dyn<Trait: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<Trait>> {
        let any = self.dyn_values.get(&TypeId::of::<Trait>())?;
        any.clone()
            .downcast::<Arc<Trait>>()
            .ok()
            .map(|a| (*a).clone())
    }

    /// Insert a [`tenant::TenantScope<T>`] keyed by the inner element
    /// type. Convenience wrapper over [`Self::insert`] so downstream
    /// callers do not have to spell out the wrapping type.
    #[cfg(feature = "multi-tenant")]
    pub fn insert_tenant_scoped<T>(&mut self, scope: tenant::TenantScope<T>)
    where
        T: Send + Sync + 'static,
    {
        let _ = self.insert(scope);
    }

    /// Retrieve the [`tenant::TenantScope<T>`] previously stored via
    /// [`Self::insert_tenant_scoped`].
    #[cfg(feature = "multi-tenant")]
    pub fn get_tenant_scoped<T>(&self) -> Option<Arc<tenant::TenantScope<T>>>
    where
        T: Send + Sync + 'static,
    {
        self.get::<tenant::TenantScope<T>>()
    }
}

/// Set of cargo-feature flags compiled into the running binary. The
/// validator uses this to verify that the loaded config does not enable
/// integrations the binary cannot serve.
///
/// The internal storage is a `BTreeSet<&'static str>` — adding a new
/// integration is a one-liner in `bootstrap::runtime_features` and
/// requires no churn in [`RuntimeFeatures`] itself.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RuntimeFeatures {
    enabled: BTreeSet<&'static str>,
}

/// Iterator over the canonical feature strings the validator understands.
/// Returning an iterator (instead of exposing a `pub const` slice) lets
/// us evolve the storage without a breaking change. (N9.)
pub fn known_features() -> impl Iterator<Item = &'static str> {
    [
        "postgres", "redis", "mongodb", "nats", "qdrant", "neo4j", "s3", "oss",
    ]
    .into_iter()
}

impl RuntimeFeatures {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn enable(mut self, feature: &'static str) -> Self {
        self.enabled.insert(feature);
        self
    }

    #[must_use]
    pub fn enable_if(mut self, feature: &'static str, on: bool) -> Self {
        if on {
            self.enabled.insert(feature);
        }
        self
    }

    pub fn contains(&self, feature: &str) -> bool {
        self.enabled.contains(feature)
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.enabled.iter().copied()
    }
}

/// Result of a successful bootstrap.
///
/// All fields are private; access them via the accessor methods. This
/// struct is `#[non_exhaustive]` — adding fields is not a breaking
/// change. `BuiltApplication` is `Clone` and the clone is allocation-light
/// (every interior field is either an `Arc` or a small descriptor `Vec`).
#[derive(Clone)]
#[non_exhaustive]
#[must_use = "BuiltApplication must be served (e.g. `production::server::run`) or driven manually; \
              dropping it on the floor wastes the integrations that were just initialized"]
pub struct BuiltApplication {
    router: Router,
    context: AppContext,
    bootstrap: BootstrapConfig,
    config: AppConfig,
    applied_sources: Vec<String>,
    initialized_integrations: Vec<String>,
    degraded_integrations: Vec<String>,
    shutdown: ShutdownToken,
    health: HealthRegistry,
    /// Providers retained in the order they successfully initialised, so
    /// the runtime can call `shutdown` on each in *reverse* order during
    /// drain. `Arc<dyn IntegrationProvider>` is cheap to clone.
    providers: Vec<Arc<dyn IntegrationProvider>>,
    /// Optional Prometheus handle, kept here so its lifetime matches the
    /// lifetime of the surrounding application instead of the spawn
    /// scope of `production::server::run`. Always `None` when the
    /// `metrics` feature is disabled or recorder install failed.
    metrics_handle: Option<MetricsHandle>,
}

impl BuiltApplication {
    /// Borrow the built [`Router`].
    pub fn router(&self) -> &Router {
        &self.router
    }

    /// Consume the built application and return the inner [`Router`].
    /// Use this when you want to drive the HTTP runtime yourself.
    pub fn into_router(self) -> Router {
        self.router
    }

    /// Borrow the [`AppContext`] handed to handlers.
    pub fn context(&self) -> &AppContext {
        &self.context
    }

    /// Borrow the resolved application config.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Borrow the bootstrap config used to load this application.
    pub fn bootstrap(&self) -> &BootstrapConfig {
        &self.bootstrap
    }

    /// Names of the config sources that contributed to the final config,
    /// in apply order.
    pub fn applied_sources(&self) -> &[String] {
        &self.applied_sources
    }

    /// Integration keys that initialised successfully, in order.
    pub fn initialized_integrations(&self) -> &[String] {
        &self.initialized_integrations
    }

    /// Integration keys whose `init()` failed but were marked
    /// non-required and so were skipped.
    pub fn degraded_integrations(&self) -> &[String] {
        &self.degraded_integrations
    }

    /// Cheap clone of the shutdown token so callers can install custom
    /// drain logic.
    pub fn shutdown(&self) -> ShutdownToken {
        self.shutdown.clone()
    }

    /// Cheap clone of the health-check registry.
    pub fn health(&self) -> HealthRegistry {
        self.health.clone()
    }

    // The following accessors are deliberately `pub(crate)` /
    // `#[doc(hidden)]` — they're consumed by the `hwhkit` facade's
    // `production::server::run` and are not part of the user-facing
    // surface. Keeping them out of the docs prevents downstream code
    // from forming dependencies on internals that may change.

    #[doc(hidden)]
    pub fn providers(&self) -> &[Arc<dyn IntegrationProvider>] {
        &self.providers
    }

    #[doc(hidden)]
    pub fn metrics_handle(&self) -> Option<&MetricsHandle> {
        self.metrics_handle.as_ref()
    }

    #[doc(hidden)]
    pub fn set_metrics_handle(&mut self, handle: MetricsHandle) {
        self.metrics_handle = Some(handle);
    }
}

/// User-supplied entrypoint. Implementations are typically zero-sized
/// structs whose `build_router` materialises the application's routes
/// from values the providers stashed in [`AppContext`].
///
/// **Project policy:** every method added to this trait in the future
/// must ship with a default implementation so existing impls keep
/// compiling without churn.
#[async_trait]
pub trait Application: Send + Sync + 'static {
    async fn build_router(&self, ctx: AppContext, cfg: &AppConfig) -> Result<Router>;
}

/// Pluggable connector for an external resource. Implementors register
/// values into the [`AppContext`] during [`Self::init`] so handlers can
/// reach them through type-keyed lookup.
///
/// **Project policy:** the trait is intentionally **open**. Future
/// methods must ship with default implementations so existing impls
/// keep compiling without churn.
#[async_trait]
pub trait IntegrationProvider: Send + Sync + 'static {
    /// Stable identifier (used in logs, metrics, error messages).
    fn key(&self) -> &'static str;

    /// Whether the loaded config asks for this integration.
    fn enabled(&self, cfg: &AppConfig) -> bool;

    /// If `true`, an `init` failure aborts bootstrap; otherwise the
    /// integration is recorded under [`BuiltApplication::degraded_integrations`]
    /// and skipped. Default: `true`.
    fn required(&self, _cfg: &AppConfig) -> bool {
        true
    }

    /// Initialise the integration: open connections, run schema, …,
    /// and stash any resulting handles into `ctx` for handlers to pick
    /// up via `ctx.get::<Handle>()`.
    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> Result<()>;

    /// Optional health check for the readiness endpoint. Default
    /// implementation returns `None` (no readiness probe). Providers
    /// that establish a connection pool should return a probe that
    /// exercises the live connection (e.g. `SELECT 1`, `PING`,
    /// `head_bucket`).
    fn health_check(&self, _ctx: &AppContext, _cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        None
    }

    /// Drain hook invoked in *reverse-init order* once the HTTP server
    /// has stopped accepting new connections. The default is a no-op so
    /// existing providers do not need any change to compile;
    /// integrations that own connection pools should override this to
    /// call their `close()` / `flush()` equivalents.
    async fn shutdown(&self, _ctx: &AppContext) -> Result<()> {
        Ok(())
    }
}

/// Run the bootstrap pipeline with the default [`ConfigLoader`] and no
/// integration providers.
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

/// Run the bootstrap pipeline with caller-supplied loader, runtime
/// feature set, and provider chain.
pub async fn bootstrap_with<A: Application>(
    app: A,
    bootstrap: BootstrapConfig,
    loader: ConfigLoader,
    runtime_features: RuntimeFeatures,
    providers: Vec<Arc<dyn IntegrationProvider>>,
) -> Result<BuiltApplication> {
    let loaded = loader
        .load(&bootstrap)
        .await
        .map_err(|e| Error::invalid_config_with_source("config load failed", e))?;

    validate_feature_mapping(&loaded.config, &runtime_features)?;

    let shutdown = ShutdownToken::new();
    let health_registry = HealthRegistry::new();

    let mut ctx = AppContext::default();
    ctx.insert(shutdown.clone());
    ctx.insert(health_registry.clone());

    let mut initialized_integrations = Vec::new();
    let mut degraded_integrations = Vec::new();
    let mut retained_providers: Vec<Arc<dyn IntegrationProvider>> = Vec::new();

    for provider in providers {
        if !provider.enabled(&loaded.config) {
            continue;
        }

        let key = provider.key();
        if !runtime_features.contains(key) {
            return Err(Error::FeatureMismatch { feature: key });
        }

        let key_owned = key.to_string();
        match provider.init(&mut ctx, &loaded.config).await {
            Ok(_) => {
                if let Some(check) = provider.health_check(&ctx, &loaded.config) {
                    health_registry.register_arc(check);
                }
                initialized_integrations.push(key_owned);
                retained_providers.push(provider);
            }
            Err(err) => {
                if provider.required(&loaded.config) {
                    return Err(err);
                }
                degraded_integrations.push(key_owned);
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
        shutdown,
        health: health_registry,
        providers: retained_providers,
        metrics_handle: None,
    })
}

pub fn validate_feature_mapping(
    config: &AppConfig,
    runtime_features: &RuntimeFeatures,
) -> Result<()> {
    ensure_feature(
        config.integrations.sql.postgres.enabled,
        runtime_features,
        "postgres",
    )?;
    ensure_feature(config.integrations.redis.enabled, runtime_features, "redis")?;
    ensure_feature(
        config.integrations.mongodb.enabled,
        runtime_features,
        "mongodb",
    )?;
    ensure_feature(
        config.integrations.messaging.nats.enabled,
        runtime_features,
        "nats",
    )?;
    ensure_feature(
        config.integrations.vector.qdrant.enabled,
        runtime_features,
        "qdrant",
    )?;
    ensure_feature(config.integrations.neo4j.enabled, runtime_features, "neo4j")?;
    ensure_feature(
        config.integrations.storage.s3.enabled,
        runtime_features,
        "s3",
    )?;
    Ok(())
}

fn ensure_feature(
    enabled: bool,
    runtime_features: &RuntimeFeatures,
    feature_name: &'static str,
) -> Result<()> {
    if enabled && !runtime_features.contains(feature_name) {
        return Err(Error::FeatureMismatch {
            feature: feature_name,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    trait DummyService: Send + Sync {
        fn label(&self) -> &str;
    }

    struct PingService;
    impl DummyService for PingService {
        fn label(&self) -> &str {
            "ping"
        }
    }

    #[test]
    fn insert_and_get_dyn_trait() {
        let mut ctx = AppContext::default();
        ctx.insert_dyn::<dyn DummyService>(Arc::new(PingService));
        let svc = ctx.get_dyn::<dyn DummyService>().expect("dyn slot present");
        assert_eq!(svc.label(), "ping");
    }

    #[test]
    fn insert_returns_prior_value() {
        let mut ctx = AppContext::default();
        let prior = ctx.insert(7u32);
        assert!(prior.is_none());
        let prior = ctx.insert(8u32);
        assert_eq!(prior.as_deref().copied(), Some(7));
    }

    #[test]
    fn runtime_features_round_trip() {
        let f = RuntimeFeatures::new()
            .enable("postgres")
            .enable_if("redis", false)
            .enable_if("nats", true);
        assert!(f.contains("postgres"));
        assert!(!f.contains("redis"));
        assert!(f.contains("nats"));
    }

    #[test]
    fn known_features_contains_canonical_set() {
        let names: Vec<&str> = known_features().collect();
        for f in ["postgres", "redis", "nats"] {
            assert!(names.contains(&f), "missing feature: {f}");
        }
    }
}
