use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("config parse failed: {0}")]
    Parse(String),
    #[error("config io failed: {0}")]
    Io(String),
    #[error("config validation failed: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
    pub fn with_service_name(mut self, service_name: impl Into<String>) -> Self {
        self.service_name = service_name.into();
        self
    }

    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    pub fn with_config_dir(mut self, config_dir: impl AsRef<Path>) -> Self {
        self.config_dir = config_dir.as_ref().to_path_buf();
        self
    }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub integrations: IntegrationsConfig,
    #[serde(default)]
    pub transport: TransportConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            observability: ObservabilityConfig::default(),
            integrations: IntegrationsConfig::default(),
            transport: TransportConfig::default(),
        }
    }
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
        validate_url_toggle(
            "transport.grpc",
            self.transport.grpc.enabled,
            &self.transport.grpc.listen,
        )?;

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
pub struct TracingConfig {
    pub enabled: bool,
    pub sample_ratio: f64,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sample_ratio: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SqlConfig {
    #[serde(default)]
    pub postgres: PostgresConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    pub max_connections: u32,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: true,
            url: String::new(),
            max_connections: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoDbConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    pub database: String,
}

impl Default for MongoDbConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
            database: "app".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessagingConfig {
    #[serde(default)]
    pub nats: NatsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VectorConfig {
    #[serde(default)]
    pub qdrant: QdrantConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    pub api_key: String,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
            api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jConfig {
    pub enabled: bool,
    pub required: bool,
    pub url: String,
    pub username: String,
    pub password: String,
}

impl Default for Neo4jConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            required: false,
            url: String::new(),
            username: "neo4j".to_string(),
            password: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransportConfig {
    #[serde(default)]
    pub grpc: GrpcTransportConfig,
    #[serde(default)]
    pub rpc: RpcTransportConfig,
    #[serde(default)]
    pub nats: NatsTransportConfig,
    #[serde(default)]
    pub websocket: WebsocketTransportConfig,
    #[serde(default)]
    pub p2p: P2pTransportConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrpcTransportConfig {
    pub enabled: bool,
    pub listen: String,
}

impl Default for GrpcTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "0.0.0.0:50051".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcTransportConfig {
    pub enabled: bool,
    pub default: String,
    pub timeout_ms: u64,
}

impl Default for RpcTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default: "grpc".to_string(),
            timeout_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsTransportConfig {
    pub enabled: bool,
    pub url: String,
    pub jetstream: bool,
}

impl Default for NatsTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: "nats://127.0.0.1:4222".to_string(),
            jetstream: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebsocketTransportConfig {
    pub enabled: bool,
    pub path: String,
    pub max_connections: usize,
}

impl Default for WebsocketTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "/ws".to_string(),
            max_connections: 10_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2pTransportConfig {
    pub enabled: bool,
    pub listen: String,
}

impl Default for P2pTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "/ip4/0.0.0.0/tcp/7001".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
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

pub trait ConfigSource: Send + Sync {
    fn name(&self) -> &'static str;
    fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch>;
}

pub struct FileDefaultSource;

impl ConfigSource for FileDefaultSource {
    fn name(&self) -> &'static str {
        "file:default"
    }

    fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
        read_toml_patch(bootstrap.default_file(), true)
    }
}

pub struct FileEnvironmentSource;

impl ConfigSource for FileEnvironmentSource {
    fn name(&self) -> &'static str {
        "file:environment"
    }

    fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
        read_toml_patch(bootstrap.env_file(), false)
    }
}

pub struct EnvConfigSource;

impl ConfigSource for EnvConfigSource {
    fn name(&self) -> &'static str {
        "env"
    }

    fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
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

pub trait RemoteConfigProvider: Send + Sync {
    fn fetch_patch(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch>;
}

#[derive(Clone, Debug)]
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

impl ConfigSource for RemoteConfigSource {
    fn name(&self) -> &'static str {
        "remote"
    }

    fn load(&self, bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
        let patch = self.provider.fetch_patch(bootstrap)?;
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
    pub fn with_source(mut self, source: Box<dyn ConfigSource>) -> Self {
        self.sources.push(source);
        self
    }

    pub fn load(&self, bootstrap: &BootstrapConfig) -> Result<LoadedConfig> {
        let mut merged = serde_json::to_value(AppConfig::default())
            .map_err(|e| Error::Parse(format!("default config serialization failed: {e}")))?;

        let mut applied_sources = Vec::new();

        for source in &self.sources {
            let patch = source.load(bootstrap)?;
            if patch.is_empty() {
                continue;
            }
            deep_merge(&mut merged, patch.into_value());
            applied_sources.push(source.name().to_string());
        }

        let config: AppConfig = serde_json::from_value(merged)
            .map_err(|e| Error::Parse(format!("final config deserialization failed: {e}")))?;
        config.validate_strict()?;

        Ok(LoadedConfig {
            config,
            applied_sources,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: AppConfig,
    pub applied_sources: Vec<String>,
}

fn read_toml_patch(path: PathBuf, required: bool) -> Result<ConfigPatch> {
    if !path.exists() {
        if required {
            return Err(Error::Io(format!(
                "required config file not found: {}",
                path.display()
            )));
        }
        return Ok(ConfigPatch::empty());
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| Error::Io(format!("cannot read {}: {e}", path.display())))?;

    if content.trim().is_empty() {
        return Ok(ConfigPatch::empty());
    }

    let parsed: toml::Value = toml::from_str(&content)
        .map_err(|e| Error::Parse(format!("toml parse failed {}: {e}", path.display())))?;
    let value = serde_json::to_value(parsed)
        .map_err(|e| Error::Parse(format!("toml conversion failed {}: {e}", path.display())))?;

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
    use std::time::{SystemTime, UNIX_EPOCH};

    struct DemoRemote;

    impl RemoteConfigProvider for DemoRemote {
        fn fetch_patch(&self, _bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
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

    fn temp_config_dir() -> PathBuf {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("duration should be valid")
            .as_millis();
        let dir = env::temp_dir().join(format!("hwhkit-config-test-{millis}"));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    #[test]
    fn bootstrap_config_defaults_to_layered_files() {
        let cfg = BootstrapConfig::default();
        assert_eq!(cfg.default_file(), PathBuf::from("config/default.toml"));
        assert_eq!(cfg.env_file(), PathBuf::from("config/dev.toml"));
    }

    #[test]
    fn layered_loader_merges_default_and_env() {
        let dir = temp_config_dir();
        fs::write(
            dir.join("default.toml"),
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
            dir.join("test.toml"),
            r#"
[server]
port = 8081
"#,
        )
        .expect("write env");

        let bootstrap = BootstrapConfig::default()
            .with_environment(Environment::Test)
            .with_config_dir(&dir);

        let loaded = ConfigLoader::default()
            .load(&bootstrap)
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

    #[test]
    fn remote_patch_is_filtered_by_policy() {
        let policy = RemotePatchPolicy::strict_defaults();
        let source = RemoteConfigSource::new(Arc::new(DemoRemote), policy);
        let patch = source
            .load(&BootstrapConfig::default())
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
