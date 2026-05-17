use std::fs;

use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::{
    config_v2::{AppConfig, BootstrapConfig, Environment},
    core_v2::{AppContext, Application, Result},
    run_v2, RuntimeBuilder, WebServerBuilder,
};

struct DemoApp;

#[async_trait]
impl Application for DemoApp {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new().route("/healthz", get(|| async { "ok" })))
    }
}

#[tokio::test]
async fn facade_reexports_run_v2_stack() {
    let config_dir = tempfile::tempdir().expect("temp config dir should exist");
    fs::write(
        config_dir.path().join("default.toml"),
        r#"
[server]
host = "127.0.0.1"
port = 3100

[observability]
service_name = "facade-run-v2"
environment = "test"
"#,
    )
    .expect("default config should be written");
    fs::write(config_dir.path().join("test.toml"), "").expect("test config should be written");

    let built = run_v2(
        DemoApp,
        BootstrapConfig::default()
            .with_environment(Environment::Test)
            .with_config_dir(config_dir.path()),
    )
    .await
    .expect("run_v2 should bootstrap through facade reexports");

    assert_eq!(built.config.server.port, 3100);
}

#[test]
fn facade_exposes_v1_v2_and_transport_side_by_side() {
    let _v1 = WebServerBuilder::new();
    let _v2 = RuntimeBuilder::new();
    let _protocol = hwhkit::transport_v2::ProtocolKind::Rpc;
}
