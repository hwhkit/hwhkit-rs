use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit_config::{AppConfig, BootstrapConfig, Environment};
use hwhkit_core::{bootstrap, AppContext, Application, Result};
use std::{
    env, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct DemoApp;

#[async_trait]
impl Application for DemoApp {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        async fn health() -> &'static str {
            "ok"
        }

        Ok(Router::new().route("/healthz", get(health)))
    }
}

#[tokio::test]
async fn application_contract_bootstraps_successfully() {
    let config_dir = make_config_dir();
    fs::write(
        config_dir.join("default.toml"),
        r#"
[server]
host = "127.0.0.1"
port = 8080

[observability]
service_name = "demo-service"
environment = "test"
"#,
    )
    .expect("default config should be written");
    fs::write(config_dir.join("test.toml"), "").expect("test config should be written");

    let cfg = BootstrapConfig::default()
        .with_service_name("demo-service")
        .with_environment(Environment::Test)
        .with_config_dir(&config_dir);

    let built = bootstrap(DemoApp, cfg)
        .await
        .expect("bootstrap should succeed");
    assert_eq!(built.bootstrap.service_name, "demo-service");
    assert_eq!(built.config.server.port, 8080);
}

fn make_config_dir() -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("duration should be valid")
        .as_millis();
    let dir = env::temp_dir().join(format!("hwhkit-core-contract-{millis}"));
    fs::create_dir_all(&dir).expect("temp config dir should be created");
    dir
}
