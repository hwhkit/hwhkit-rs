// Config types intentionally use manual `Default` impls so that the
// "off by default" semantics for boolean toggles (`enabled = false`,
// `required = false`) are visible at the type level rather than hidden
// behind a derive. Suppress the lint workspace-wide for this crate.
#![allow(clippy::derivable_impls)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Boxed third-party error. Mirrors `hwhkit_core::error::BoxError` but
/// is duplicated here so this crate stays free of a hard dep on the core
/// crate (downstream tooling can use `hwhkit-config` standalone).
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("config parse failed: {message}")]
    Parse {
        message: String,
        #[source]
        source: Option<BoxError>,
    },
    #[error("config io failed: {message}")]
    Io {
        message: String,
        #[source]
        source: Option<BoxError>,
    },
    #[error("config validation failed: {0}")]
    Validation(String),
}

impl Error {
    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse {
            message: message.into(),
            source: None,
        }
    }

    pub fn parse_with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Parse {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::Io {
            message: message.into(),
            source: None,
        }
    }

    pub fn io_with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Io {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Environment {
    Dev,
    Test,
    Prod,
    Custom(String),
}

impl Default for Environment {
    fn default() -> Self {
        Self::Dev
    }
}

impl Environment {
    pub fn file_stem(&self) -> &str {
        match self {
            Self::Dev => "dev",
            Self::Test => "test",
            Self::Prod => "prod",
            Self::Custom(v) => v.as_str(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BootstrapConfig {
    pub service_name: String,
    pub environment: Environment,
    pub config_dir: PathBuf,
    pub env_prefix: String,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            service_name: "hwhkit-service".to_string(),
            environment: Environment::default(),
            config_dir: PathBuf::from("config"),
            env_prefix: "HWHKIT__".to_string(),
        }
    }
}

impl BootstrapConfig {
    #[must_use]
    pub fn with_service_name(mut self, service_name: impl Into<String>) -> Self {
        self.service_name = service_name.into();
        self
    }

    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    #[must_use]
    pub fn with_config_dir(mut self, config_dir: impl AsRef<Path>) -> Self {
        self.config_dir = config_dir.as_ref().to_path_buf();
        self
    }

    #[must_use]
    pub fn with_env_prefix(mut self, env_prefix: impl Into<String>) -> Self {
        self.env_prefix = env_prefix.into();
        self
    }

    pub fn default_file(&self) -> PathBuf {
        self.config_dir.join("default.toml")
    }

    pub fn env_file(&self) -> PathBuf {
        self.config_dir
            .join(format!("{}.toml", self.environment.file_stem()))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub integrations: IntegrationsConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
}

impl AppConfig {
    pub fn validate_strict(&self) -> Result<()> {
        if self.server.host.trim().is_empty() {
            return Err(Error::Validation("server.host cannot be empty".to_string()));
        }
        if self.server.port == 0 {
            return Err(Error::Validation("server.port cannot be 0".to_string()));
        }
        if self.observability.service_name.trim().is_empty() {
            return Err(Error::Validation(
                "observability.service_name cannot be empty".to_string(),
            ));
        }

        validate_url_toggle(
            "integrations.sql.postgres",
            self.integrations.sql.postgres.enabled,
            &self.integrations.sql.postgres.url,
        )?;
        validate_url_toggle(
            "integrations.redis",
            self.integrations.redis.enabled,
            &self.integrations.redis.url,
        )?;
        validate_url_toggle(
            "integrations.mongodb",
            self.integrations.mongodb.enabled,
            &self.integrations.mongodb.url,
        )?;
        validate_url_toggle(
            "integrations.messaging.nats",
            self.integrations.messaging.nats.enabled,
            &self.integrations.messaging.nats.url,
        )?;
        validate_url_toggle(
            "integrations.vector.qdrant",
            self.integrations.vector.qdrant.enabled,
            &self.integrations.vector.qdrant.url,
        )?;
        validate_url_toggle(
            "integrations.neo4j",
            self.integrations.neo4j.enabled,
            &self.integrations.neo4j.url,
        )?;
        if self.integrations.storage.s3.enabled {
            if self.integrations.storage.s3.bucket.trim().is_empty() {
                return Err(Error::Validation(
                    "integrations.storage.s3.bucket cannot be empty when enabled".to_string(),
                ));
            }
            if self.integrations.storage.s3.region.trim().is_empty() {
                return Err(Error::Validation(
                    "integrations.storage.s3.region cannot be empty when enabled".to_string(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_url_toggle(path: &str, enabled: bool, url: &str) -> Result<()> {
    if enabled && url.trim().is_empty() {
        return Err(Error::Validation(format!(
            "{path}.url/listen cannot be empty when enabled"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ObservabilityConfig {
    pub service_name: String,
    pub environment: Environment,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub tracing: TracingConfig,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            service_name: "hwhkit-service".to_string(),
            environment: Environment::Dev,
            logging: LoggingConfig::default(),
            tracing: TracingConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "pretty".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TracingConfig {
    pub enabled: bool,
    pub sample_ratio: f64,
    #[serde(default)]
    pub otel: OtelConfig,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sample_ratio: 1.0,
            otel: OtelConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OtelConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub protocol: String, // "grpc" | "http"
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:4317".to_string(),
            protocol: "grpc".to_string(),
        }
    }
}

/// Production-runtime configuration (health/metrics/version endpoints,
/// middleware bundle, shutdown).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct RuntimeConfig {
    #[serde(default)]
    pub health: HealthConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub info: InfoConfig,
    #[serde(default)]
    pub middleware: MiddlewareConfig,
    #[serde(default)]
    pub shutdown: ShutdownConfig,
    #[serde(default)]
    pub request_id: RequestIdConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HealthConfig {
    pub enabled: bool,
    pub path_live: String,
    pub path_ready: String,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path_live: "/health".to_string(),
            path_ready: "/health/ready".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MetricsConfig {
    pub enabled: bool,
    pub path: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: "/metrics".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InfoConfig {
    pub enabled: bool,
    pub path_version: String,
    pub path_info: String,
}

impl Default for InfoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path_version: "/version".to_string(),
            path_info: "/info".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MiddlewareConfig {
    pub enabled: bool,
    pub cors: CorsConfig,
    pub compression: bool,
    pub timeout_secs: u64,
    pub body_limit_bytes: usize,
    pub catch_panic: bool,
}

impl Default for MiddlewareConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cors: CorsConfig::default(),
            compression: true,
            timeout_secs: 30,
            body_limit_bytes: 2 * 1024 * 1024,
            catch_panic: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CorsConfig {
    pub enabled: bool,
    pub allow_origins: Vec<String>,
    pub allow_credentials: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShutdownConfig {
    pub enabled: bool,
    pub max_drain_secs: u64,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_drain_secs: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RequestIdConfig {
    pub enabled: bool,
    pub header: String,
}

impl Default for RequestIdConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            header: "x-request-id".to_string(),
        }
    }
}

/// Per-integration resilience knobs.
///
/// Every integration section embeds one of these via
/// `#[serde(default)] resilience: ResilienceConfig`. All defaults are
/// safe for production — operators tune them only when the workload
/// has unusual latency expectations or backend SLOs.
///
/// **What each field guards against:**
///
/// - `connect_timeout_ms` — initial handshake. Prevents bootstrap from
///   hanging forever on a misconfigured / unreachable backend.
/// - `op_timeout_ms` — per-operation hint surfaced via
///   `*Handle::op_timeout()`. **Advisory**: the integration crate wires
///   it into the underlying SDK's native timeout where one exists
///   (e.g. `sqlx::PgPoolOptions::acquire_timeout`); user code should
///   also use it to wrap long-running operations with
///   `tokio::time::timeout(handle.op_timeout(), my_call)`.
/// - `probe_timeout_ms` — readiness health-check budget. A probe that
///   can't answer in this window is reported Down rather than blocking
///   the `/health/ready` endpoint. Critical for the
///   pool-saturation case where the probe queues behind real traffic.
/// - `shutdown_timeout_ms` — caps each provider's `shutdown()` so a
///   stuck driver (e.g. `PgPool::close` waiting on a hung transaction)
///   doesn't eat the entire `runtime.shutdown.max_drain_secs` budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResilienceConfig {
    /// Initial connect handshake budget. Default: 5_000 ms.
    pub connect_timeout_ms: u64,
    /// Per-operation budget hint. Default: 5_000 ms. Surfaced via
    /// `*Handle::op_timeout()` and used by the integration crate to
    /// configure SDK-native timeouts where available.
    pub op_timeout_ms: u64,
    /// Per-readiness-probe budget. Default: 500 ms. Probes exceeding
    /// this fail fast so `/health/ready` stays responsive under load.
    pub probe_timeout_ms: u64,
    /// Per-provider shutdown budget. Default: 5_000 ms. Bounds
    /// graceful-shutdown drain so one stuck driver can't block the
    /// whole process from exiting.
    pub shutdown_timeout_ms: u64,
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 5_000,
            op_timeout_ms: 5_000,
            probe_timeout_ms: 500,
            shutdown_timeout_ms: 5_000,
        }
    }
}

impl ResilienceConfig {
    /// `connect_timeout_ms` as a `Duration`.
    pub fn connect_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.connect_timeout_ms)
    }
    /// `op_timeout_ms` as a `Duration`.
    pub fn op_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.op_timeout_ms)
    }
    /// `probe_timeout_ms` as a `Duration`.
    pub fn probe_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.probe_timeout_ms)
    }
    /// `shutdown_timeout_ms` as a `Duration`.
    pub fn shutdown_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.shutdown_timeout_ms)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct IntegrationsConfig {
    #[serde(default)]
    pub sql: SqlConfig,
    #[serde(default)]
    pub redis: RedisConfig,
    #[serde(default)]
    pub mongodb: MongoDbConfig,
    #[serde(default)]
    pub messaging: MessagingConfig,
    #[serde(default)]
    pub vector: VectorConfig,
    #[serde(default)]
    pub neo4j: Neo4jConfig,
    #[serde(default)]
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct StorageConfig {
    #[serde(default)]
    pub s3: S3Config,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct S3Config {
    pub enabled: bool,
    pub required: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
    #[serde(default)]
    pub resilience: ResilienceConfig,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            endpoint: String::new(),
            region: "us-east-1".to_string(),
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            force_path_style: true,
            resilience: ResilienceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct SqlConfig {
    #[serde(default)]
    pub postgres: PostgresConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PostgresConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    pub max_connections: u32,
    #[serde(default)]
    pub migrations: PostgresMigrationsConfig,
    #[serde(default)]
    pub resilience: ResilienceConfig,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: true,
            url: String::new(),
            max_connections: 20,
            migrations: PostgresMigrationsConfig::default(),
            resilience: ResilienceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PostgresMigrationsConfig {
    pub run_on_start: bool,
    pub path: String,
}

impl Default for PostgresMigrationsConfig {
    fn default() -> Self {
        Self {
            run_on_start: false,
            path: "./migrations".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RedisConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    #[serde(default)]
    pub resilience: ResilienceConfig,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
            resilience: ResilienceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MongoDbConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    pub database: String,
    #[serde(default)]
    pub resilience: ResilienceConfig,
}

impl Default for MongoDbConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
            database: "app".to_string(),
            resilience: ResilienceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct MessagingConfig {
    #[serde(default)]
    pub nats: NatsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NatsConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    #[serde(default)]
    pub resilience: ResilienceConfig,
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
            resilience: ResilienceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct VectorConfig {
    #[serde(default)]
    pub qdrant: QdrantConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QdrantConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    pub api_key: String,
    #[serde(default)]
    pub resilience: ResilienceConfig,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
            api_key: String::new(),
            resilience: ResilienceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Neo4jConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub resilience: ResilienceConfig,
}

impl Default for Neo4jConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
            username: "neo4j".to_string(),
            password: String::new(),
            resilience: ResilienceConfig::default(),
        }
    }
}

/// A partial configuration overlay (a JSON object whose keys map to
/// dotted [`AppConfig`] paths).
///
/// Patches feed into the [`ConfigLoader`] merge pipeline. Marked
/// `#[non_exhaustive]` so new metadata fields can be added without
/// breaking pattern-matching callers.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ConfigPatch {
    value: Value,
}

impl ConfigPatch {
    pub fn empty() -> Self {
        Self {
            value: Value::Object(Map::new()),
        }
    }

    pub fn from_value(value: Value) -> Self {
        if value.is_object() {
            Self { value }
        } else {
            Self::empty()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.value.as_object().map(|v| v.is_empty()).unwrap_or(true)
    }

    pub fn into_value(self) -> Value {
        self.value
    }

    pub fn set_path(&mut self, path: &[&str], value: Value) {
        if path.is_empty() {
            return;
        }

        let mut current = self
            .value
            .as_object_mut()
            .expect("ConfigPatch always starts with object");

        for (idx, segment) in path.iter().enumerate() {
            let is_last = idx == path.len() - 1;

            if is_last {
                current.insert((*segment).to_string(), value);
                return;
            }

            let next = current
                .entry((*segment).to_string())
                .or_insert_with(|| Value::Object(Map::new()));

            if !next.is_object() {
                *next = Value::Object(Map::new());
            }

            current = next
                .as_object_mut()
                .expect("intermediate value should be object");
        }
    }
}

/// Pluggable configuration source. Sources are invoked in registration
/// order by [`ConfigLoader::load`] and their patches are merged onto the
/// running config (last write wins per leaf field).
///
/// **Project policy:** the trait is intentionally **open**. Future
/// methods must ship with default implementations so existing impls
/// keep compiling without churn.
#[async_trait]
pub trait ConfigSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch>;
}

/// Layer 1 of the standard load order: `config/default.toml`.
///
/// **Optional.** A missing file is not an error — the loader falls back
/// to [`AppConfig::default`] for any field this source would have
/// supplied. This matches the DX expectation that `cargo new` →
/// `cargo run` works without first authoring a config file. To enforce
/// a baseline config (e.g. in production), require the relevant
/// fields via your own [`ConfigSource`] or rely on
/// [`AppConfig::validate_strict`].
pub struct FileDefaultSource;

#[async_trait]
impl ConfigSource for FileDefaultSource {
    fn name(&self) -> &'static str {
        "file:default"
    }

    async fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
        let path = bootstrap.default_file();
        if !path.exists() {
            // A debug log instead of a warn — running without a default
            // file is a legitimate, supported mode (especially for tests
            // and one-shot binaries). `applied_sources` already records
            // that this source contributed nothing, so the absence is
            // observable to anyone who needs it.
            tracing::debug!(
                path = %path.display(),
                "default config file not present; using AppConfig::default() for unset fields"
            );
        }
        read_toml_patch(path, false)
    }
}

pub struct FileEnvironmentSource;

#[async_trait]
impl ConfigSource for FileEnvironmentSource {
    fn name(&self) -> &'static str {
        "file:environment"
    }

    async fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
        read_toml_patch(bootstrap.env_file(), false)
    }
}

pub struct EnvConfigSource;

#[async_trait]
impl ConfigSource for EnvConfigSource {
    fn name(&self) -> &'static str {
        "env"
    }

    async fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
        let mut patch = ConfigPatch::empty();

        for (key, raw_value) in env::vars() {
            if !key.starts_with(&bootstrap.env_prefix) {
                continue;
            }

            let suffix = key
                .trim_start_matches(&bootstrap.env_prefix)
                .to_ascii_lowercase();

            if suffix.is_empty() {
                continue;
            }

            let path: Vec<&str> = suffix
                .split("__")
                .filter(|s| !s.trim().is_empty())
                .collect();

            if path.is_empty() {
                continue;
            }

            patch.set_path(&path, parse_env_value(&raw_value));
        }

        Ok(patch)
    }
}

/// Async-friendly remote config provider. Implementations typically hit
/// a network — Consul, Vault, etcd, an internal control-plane HTTP API,
/// … — and return a partial JSON patch. The patch is filtered through
/// [`RemotePatchPolicy`] before reaching [`AppConfig`].
///
/// **Project policy:** the trait is intentionally **open**. Future
/// methods must ship with default implementations so existing impls
/// keep compiling without churn.
#[async_trait]
pub trait RemoteConfigProvider: Send + Sync {
    async fn fetch_patch(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch>;
}

/// Allow-list policy that gates which dotted config paths a remote
/// patch may overwrite.
///
/// The strict defaults (see [`RemotePatchPolicy::strict_defaults`]) only
/// accept observability knobs. Marked `#[non_exhaustive]` so future
/// fields (deny lists, callbacks, …) don't break pattern matching.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RemotePatchPolicy {
    allowed_paths: BTreeSet<String>,
}

impl RemotePatchPolicy {
    pub fn strict_defaults() -> Self {
        let mut allowed_paths = BTreeSet::new();
        allowed_paths.insert("observability.logging.level".to_string());
        allowed_paths.insert("observability.logging.format".to_string());
        allowed_paths.insert("observability.tracing.sample_ratio".to_string());
        allowed_paths.insert("observability.tracing.enabled".to_string());

        Self { allowed_paths }
    }

    pub fn allow_path(mut self, path: impl Into<String>) -> Self {
        self.allowed_paths.insert(path.into());
        self
    }

    fn allows(&self, path: &str) -> bool {
        self.allowed_paths.contains(path)
    }
}

pub struct RemoteConfigSource {
    provider: Arc<dyn RemoteConfigProvider>,
    policy: RemotePatchPolicy,
}

impl RemoteConfigSource {
    pub fn new(provider: Arc<dyn RemoteConfigProvider>, policy: RemotePatchPolicy) -> Self {
        Self { provider, policy }
    }
}

#[async_trait]
impl ConfigSource for RemoteConfigSource {
    fn name(&self) -> &'static str {
        "remote"
    }

    async fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
        let patch = self.provider.fetch_patch(bootstrap).await?;
        Ok(filter_patch(patch, &self.policy))
    }
}

pub struct ConfigLoader {
    sources: Vec<Box<dyn ConfigSource>>,
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self {
            sources: vec![
                Box::new(FileDefaultSource),
                Box::new(FileEnvironmentSource),
                Box::new(EnvConfigSource),
            ],
        }
    }
}

impl ConfigLoader {
    /// Append a [`ConfigSource`] to the loader. The source is boxed
    /// internally — callers pass a concrete type (or a `Box`/`Arc` of
    /// `dyn ConfigSource`) without ceremony.
    #[must_use]
    pub fn with_source<S: ConfigSource + 'static>(mut self, source: S) -> Self {
        self.sources.push(Box::new(source));
        self
    }

    /// Append a pre-boxed source. Use this when the source comes from
    /// elsewhere as a trait object (e.g. plugin loading).
    #[must_use]
    pub fn with_boxed_source(mut self, source: Box<dyn ConfigSource>) -> Self {
        self.sources.push(source);
        self
    }

    pub async fn load(&self, bootstrap: &BootstrapConfig) -> Result<LoadedConfig> {
        let mut merged = serde_json::to_value(AppConfig::default())
            .map_err(|e| Error::parse_with_source("default config serialization failed", e))?;

        let mut applied_sources = Vec::new();

        for source in &self.sources {
            let patch = source.load(bootstrap).await?;
            if patch.is_empty() {
                continue;
            }
            deep_merge(&mut merged, patch.into_value());
            applied_sources.push(source.name().to_string());
        }

        let config: AppConfig = serde_json::from_value(merged)
            .map_err(|e| Error::parse_with_source("final config deserialization failed", e))?;
        config.validate_strict()?;

        Ok(LoadedConfig {
            config,
            applied_sources,
        })
    }
}

/// The resolved [`AppConfig`] together with the list of sources that
/// contributed to it.
///
/// Returned by [`ConfigLoader::load`]. Marked `#[non_exhaustive]` so
/// future fields (e.g. effective env, override origin tracking) can be
/// added without breaking match patterns; access via the
/// [`LoadedConfig::config`] / [`LoadedConfig::applied_sources`] methods.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LoadedConfig {
    /// The fully merged + validated configuration.
    pub config: AppConfig,
    /// Human-readable identifiers of the sources whose patches were
    /// successfully merged (file paths, `env`, remote-source names…).
    pub applied_sources: Vec<String>,
}

impl LoadedConfig {
    /// Borrow the resolved [`AppConfig`].
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Sources that contributed to the final configuration, in the order
    /// they were applied.
    pub fn applied_sources(&self) -> &[String] {
        &self.applied_sources
    }
}

fn read_toml_patch(path: PathBuf, required: bool) -> Result<ConfigPatch> {
    if !path.exists() {
        if required {
            return Err(Error::io(format!(
                "required config file not found: {}",
                path.display()
            )));
        }
        return Ok(ConfigPatch::empty());
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| Error::io_with_source(format!("cannot read {}", path.display()), e))?;

    if content.trim().is_empty() {
        return Ok(ConfigPatch::empty());
    }

    let parsed: toml::Value = toml::from_str(&content).map_err(|e| {
        Error::parse_with_source(format!("toml parse failed {}", path.display()), e)
    })?;
    let value = serde_json::to_value(parsed).map_err(|e| {
        Error::parse_with_source(format!("toml conversion failed {}", path.display()), e)
    })?;

    Ok(ConfigPatch::from_value(value))
}

fn parse_env_value(raw: &str) -> Value {
    match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            if let Ok(v) = raw.parse::<i64>() {
                return Value::Number(v.into());
            }
            if let Ok(v) = raw.parse::<f64>() {
                if let Some(v) = serde_json::Number::from_f64(v) {
                    return Value::Number(v);
                }
            }
            if (raw.starts_with('[') && raw.ends_with(']'))
                || (raw.starts_with('{') && raw.ends_with('}'))
            {
                if let Ok(v) = serde_json::from_str::<Value>(raw) {
                    return v;
                }
            }
            Value::String(raw.to_string())
        }
    }
}

fn deep_merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                deep_merge(base_map.entry(k).or_insert(Value::Null), v);
            }
        }
        (base_value, patch_value) => {
            *base_value = patch_value;
        }
    }
}

fn filter_patch(patch: ConfigPatch, policy: &RemotePatchPolicy) -> ConfigPatch {
    let mut filtered = ConfigPatch::empty();
    collect_allowed_leaf_paths("", &patch.value, &mut filtered, policy);
    filtered
}

fn collect_allowed_leaf_paths(
    prefix: &str,
    value: &Value,
    filtered: &mut ConfigPatch,
    policy: &RemotePatchPolicy,
) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let path = if prefix.is_empty() {
                    k.to_string()
                } else {
                    format!("{prefix}.{k}")
                };
                collect_allowed_leaf_paths(&path, v, filtered, policy);
            }
        }
        _ => {
            if policy.allows(prefix) {
                let segments: Vec<&str> = prefix.split('.').collect();
                filtered.set_path(&segments, value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct DemoRemote;

    #[async_trait]
    impl RemoteConfigProvider for DemoRemote {
        async fn fetch_patch(&self, _bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
            let patch = serde_json::json!({
                "observability": {
                    "logging": {
                        "level": "debug"
                    }
                },
                "server": {
                    "port": 9000
                }
            });
            Ok(ConfigPatch::from_value(patch))
        }
    }

    #[test]
    fn bootstrap_config_defaults_to_layered_files() {
        let cfg = BootstrapConfig::default();
        assert_eq!(cfg.default_file(), PathBuf::from("config/default.toml"));
        assert_eq!(cfg.env_file(), PathBuf::from("config/dev.toml"));
    }

    #[tokio::test]
    async fn layered_loader_merges_default_and_env() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("default.toml"),
            r#"
[server]
host = "127.0.0.1"
port = 3000

[observability]
service_name = "test-svc"
environment = "dev"
"#,
        )
        .expect("write default");
        fs::write(
            dir.path().join("test.toml"),
            r#"
[server]
port = 8081
"#,
        )
        .expect("write env");

        let bootstrap = BootstrapConfig::default()
            .with_environment(Environment::Test)
            .with_config_dir(dir.path());

        let loaded = ConfigLoader::default()
            .load(&bootstrap)
            .await
            .expect("config should load");
        assert_eq!(loaded.config.server.port, 8081);
        assert!(loaded
            .applied_sources
            .iter()
            .any(|source| source == "file:default"));
        assert!(loaded
            .applied_sources
            .iter()
            .any(|source| source == "file:environment"));
    }

    #[tokio::test]
    async fn loader_succeeds_when_default_file_is_missing() {
        // DX contract: `cargo new` → `cargo run` must work without
        // authoring a config file. The loader returns the validated
        // `AppConfig::default()` and records that no file-based source
        // contributed.
        let dir = TempDir::new().expect("tempdir");
        let bootstrap = BootstrapConfig::default().with_config_dir(dir.path());

        let loaded = ConfigLoader::default()
            .load(&bootstrap)
            .await
            .expect("missing default.toml must not be a hard error");

        // Pure defaults made it through validation.
        assert_eq!(loaded.config.server.port, 3000);
        assert_eq!(loaded.config.server.host, "0.0.0.0");

        // Neither file source contributed (both files absent), so
        // `applied_sources` does not list them. `env` may or may not
        // appear depending on the test environment, but the file
        // entries definitely should not.
        assert!(
            !loaded
                .applied_sources
                .iter()
                .any(|s| s == "file:default" || s == "file:environment"),
            "no file source should be listed when neither file exists: {:?}",
            loaded.applied_sources
        );
    }

    #[tokio::test]
    async fn remote_patch_is_filtered_by_policy() {
        let policy = RemotePatchPolicy::strict_defaults();
        let source = RemoteConfigSource::new(Arc::new(DemoRemote), policy);
        let patch = source
            .load(&BootstrapConfig::default())
            .await
            .expect("remote patch should load");
        let value = patch.into_value();

        assert_eq!(
            value
                .get("observability")
                .and_then(|v| v.get("logging"))
                .and_then(|v| v.get("level"))
                .and_then(Value::as_str),
            Some("debug")
        );
        assert!(value.get("server").and_then(|v| v.get("port")).is_none());
    }
}
