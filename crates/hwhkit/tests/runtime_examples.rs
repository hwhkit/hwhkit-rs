use std::{
    fs,
    net::{TcpListener as StdTcpListener, TcpStream as StdTcpStream},
    sync::Arc,
};

use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use hwhkit::{
    config_v2::{AppConfig, BootstrapConfig, Environment},
    core_v2::{AppContext, LocalService, MessageBusResource, Module, ResourceKind, Result, ServiceRequest, ServiceResponse, ServiceTarget},
    RuntimeBuilder,
};
use hwhkit_integration_nats::NatsHandle;
use hwhkit_transport::{CommunicationPattern, LoopbackAdapter, ProtocolAdapter, ProtocolCall, ProtocolKind};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tokio::{net::TcpListener, task::JoinHandle};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserProfile {
    id: u64,
    name: String,
}

struct ProfilesModule;
struct EchoRouteModule;
struct EventModule;
struct ProfilesService;

#[async_trait]
impl LocalService for ProfilesService {
    async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse> {
        let id = String::from_utf8(request.payload)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1);

        Ok(ServiceResponse {
            payload: serde_json::to_vec(&UserProfile {
                id,
                name: format!("user-{id}"),
            })
            .expect("profile should serialize"),
            metadata: request.metadata,
        })
    }
}

#[async_trait]
impl Module for ProfilesModule {
    fn name(&self) -> &'static str {
        "profiles"
    }

    async fn register_services(&self, ctx: AppContext, _cfg: &AppConfig) -> Result<()> {
        ctx.register_local_service("profiles", Arc::new(ProfilesService));
        Ok(())
    }

    async fn router(&self, ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route("/api/v1/profiles/:id", get(get_profile))
            .with_state(ctx))
    }
}

#[async_trait]
impl Module for EchoRouteModule {
    fn name(&self) -> &'static str {
        "echo-route"
    }

    async fn router(&self, ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new()
            .route("/api/v1/relay/:value", get(relay_value))
            .with_state(ctx))
    }
}

#[async_trait]
impl Module for EventModule {
    fn name(&self) -> &'static str {
        "events"
    }

    async fn router(&self, ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new()
            .route("/api/v1/events/publish/:topic/:payload", post(publish_event))
            .route("/api/v1/events/stream/:topic", get(stream_event))
            .with_state(ctx))
    }
}

async fn get_profile(
    State(ctx): State<AppContext>,
    Path(id): Path<u64>,
) -> std::result::Result<impl IntoResponse, StatusCode> {
    let response = ctx
        .call_service(
            ServiceTarget::auto("profiles"),
            ServiceRequest::new("GetProfile", id.to_string().into_bytes()),
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let profile: UserProfile =
        serde_json::from_slice(&response.payload).map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(Json(profile))
}

async fn relay_value(
    State(ctx): State<AppContext>,
    Path(value): Path<String>,
) -> std::result::Result<impl IntoResponse, StatusCode> {
    let response = ctx
        .call_service(
            ServiceTarget::auto("echo"),
            ServiceRequest::new("Echo", value.clone().into_bytes()),
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(Json(serde_json::json!({
        "payload": String::from_utf8_lossy(&response.payload),
        "service": response.metadata.get("service").cloned(),
        "serializer": response.metadata.get("serializer").cloned(),
        "transport": response.metadata.get("transport").cloned(),
    })))
}

async fn publish_event(
    State(ctx): State<AppContext>,
    Path((topic, payload)): Path<(String, String)>,
) -> std::result::Result<impl IntoResponse, StatusCode> {
    let bus = ctx
        .resource::<NatsHandle>(ResourceKind::MessageBus, "default")
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    bus.publish(&topic, payload.clone().into_bytes())
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(Json(serde_json::json!({ "topic": topic, "payload": payload })))
}

async fn stream_event(
    State(ctx): State<AppContext>,
    Path(topic): Path<String>,
) -> std::result::Result<impl IntoResponse, StatusCode> {
    let handle = ctx
        .native::<NatsHandle>()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut subscriber = handle
        .subscribe(&topic)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    use futures_util::StreamExt;
    let message = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        subscriber.next().await
    })
    .await
    .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
    .ok_or(StatusCode::GATEWAY_TIMEOUT)?;

    Ok(String::from_utf8_lossy(&message.payload).to_string())
}

#[tokio::test]
async fn runtime_builder_serves_healthz_and_local_service_route() {
    let config_dir = write_config(
        &format!(
            r#"
[server]
host = "127.0.0.1"
port = {}

[observability]
service_name = "runtime-local"
environment = "test"
"#,
            find_free_port()
        ),
    );

    let built = RuntimeBuilder::new()
        .disable_default_providers()
        .bootstrap(
            BootstrapConfig::default()
                .with_environment(Environment::Test)
                .with_config_dir(config_dir.path()),
        )
        .module(ProfilesModule)
        .build()
        .await
        .expect("runtime builder should succeed");

    let (base_url, _server) = spawn_server(built.router).await;
    let client = reqwest::Client::new();

    let health = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await
        .expect("healthz request should succeed");
    assert_eq!(health.status(), reqwest::StatusCode::OK);

    let profile = client
        .get(format!("{base_url}/api/v1/profiles/7"))
        .send()
        .await
        .expect("profile request should succeed");
    assert_eq!(profile.status(), reqwest::StatusCode::OK);

    let profile: UserProfile = profile.json().await.expect("profile should decode");
    assert_eq!(profile.id, 7);
    assert_eq!(profile.name, "user-7");
}

#[tokio::test]
async fn runtime_builder_serves_remote_mesh_loopback_route() {
    let config_dir = write_config(
        &format!(
            r#"
[server]
host = "127.0.0.1"
port = {}

[observability]
service_name = "runtime-remote"
environment = "test"

[mesh.services.echo]
mode = "remote"
protocol = "rpc"
endpoint = "svc.echo"
subject = ""
serializer = "json"
timeout_ms = 1500
"#,
            find_free_port()
        ),
    );

    let built = RuntimeBuilder::new()
        .disable_default_providers()
        .bootstrap(
            BootstrapConfig::default()
                .with_environment(Environment::Test)
                .with_config_dir(config_dir.path()),
        )
        .adapter(Arc::new(LoopbackAdapter))
        .module(EchoRouteModule)
        .build()
        .await
        .expect("runtime builder should succeed");

    let (base_url, _server) = spawn_server(built.router).await;
    let response = reqwest::get(format!("{base_url}/api/v1/relay/demo"))
        .await
        .expect("relay request should succeed");
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: serde_json::Value = response.json().await.expect("json body should decode");
    assert_eq!(body["payload"], "demo");
    assert_eq!(body["service"], "echo");
    assert_eq!(body["serializer"], "json");
}

#[tokio::test]
async fn runtime_builder_switches_protocols_without_route_changes() {
    for (protocol, adapter, expected_transport) in [
        (
            "rpc",
            Arc::new(LoopbackAdapter) as Arc<dyn ProtocolAdapter>,
            "loopback-rpc",
        ),
        (
            "http",
            Arc::new(TestProtocolAdapter::new(ProtocolKind::Http, "loopback-http"))
                as Arc<dyn ProtocolAdapter>,
            "loopback-http",
        ),
        (
            "nats",
            Arc::new(TestProtocolAdapter::new(ProtocolKind::Nats, "loopback-nats"))
                as Arc<dyn ProtocolAdapter>,
            "loopback-nats",
        ),
    ] {
        let config_dir = write_config(
            &format!(
                r#"
[server]
host = "127.0.0.1"
port = {}

[observability]
service_name = "runtime-protocol-{protocol}"
environment = "test"

[mesh.services.echo]
mode = "remote"
protocol = "{protocol}"
endpoint = "svc.echo"
subject = ""
serializer = "json"
timeout_ms = 1500
"#,
                find_free_port()
            ),
        );

        let built = RuntimeBuilder::new()
            .disable_default_providers()
            .bootstrap(
                BootstrapConfig::default()
                    .with_environment(Environment::Test)
                    .with_config_dir(config_dir.path()),
            )
            .adapter(adapter)
            .module(EchoRouteModule)
            .build()
            .await
            .expect("runtime builder should succeed");

        let (base_url, _server) = spawn_server(built.router).await;
        let response = reqwest::get(format!("{base_url}/api/v1/relay/demo"))
            .await
            .expect("relay request should succeed");
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let body: serde_json::Value = response.json().await.expect("json body should decode");
        assert_eq!(body["payload"], "demo");
        assert_eq!(body["service"], "echo");
        assert_eq!(body["serializer"], "json");
        assert_eq!(body["transport"], expected_transport);
    }
}

#[tokio::test]
async fn runtime_builder_bridges_http_publish_to_nats_stream() {
    let Some(nats) = TestNatsServer::spawn().await else {
        eprintln!("skipping runtime nats e2e: `nats-server` not available in PATH");
        return;
    };

    let config_dir = write_config(
        &format!(
            r#"
[server]
host = "127.0.0.1"
port = {}

[observability]
service_name = "runtime-nats"
environment = "test"

[integrations.messaging.nats]
enabled = true
required = true
url = "{}"
"#,
            find_free_port(),
            nats.url()
        ),
    );

    let built = RuntimeBuilder::new()
        .bootstrap(
            BootstrapConfig::default()
                .with_environment(Environment::Test)
                .with_config_dir(config_dir.path()),
        )
        .module(EventModule)
        .build()
        .await
        .expect("runtime builder should succeed");

    let (base_url, _server) = spawn_server(built.router).await;
    let stream_url = format!("{base_url}/api/v1/events/stream/events.demo")
        .replace("http://", "ws://");

    let stream_task = tokio::spawn(async move {
        let (mut socket, _) = tokio_tungstenite::connect_async(stream_url)
            .await
            .expect("websocket stream should connect");
        use futures_util::StreamExt;
        let message = socket
            .next()
            .await
            .expect("websocket should yield a message")
            .expect("websocket frame should be ok");
        message.into_text().expect("websocket payload should be text")
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let publish = reqwest::Client::new()
        .post(format!("{base_url}/api/v1/events/publish/events.demo/hello"))
        .send()
        .await
        .expect("publish request should succeed");
    assert_eq!(publish.status(), reqwest::StatusCode::OK);

    let payload = stream_task.await.expect("stream task should complete");
    assert_eq!(payload, "hello");
}

fn write_config(default_toml: &str) -> TempDir {
    let dir = tempfile::tempdir().expect("temp config dir should be created");
    fs::write(dir.path().join("default.toml"), default_toml).expect("default config should write");
    fs::write(dir.path().join("test.toml"), "").expect("test config should write");
    dir
}

fn find_free_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .expect("ephemeral port should bind")
        .local_addr()
        .expect("local addr should exist")
        .port()
}

async fn spawn_server(router: Router) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener.local_addr().expect("local addr should exist");
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("server should run");
    });

    (format!("http://{}", address), server)
}

struct TestNatsServer {
    child: tokio::process::Child,
    port: u16,
}

impl TestNatsServer {
    async fn spawn() -> Option<Self> {
        let port = find_free_port();
        let child = match tokio::process::Command::new("nats-server")
            .args(["-p", &port.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => panic!("failed to spawn nats-server: {err}"),
        };

        let server = Self { child, port };
        server.wait_until_ready().await;
        Some(server)
    }

    fn url(&self) -> String {
        format!("nats://127.0.0.1:{}", self.port)
    }

    async fn wait_until_ready(&self) {
        for _ in 0..20 {
            if StdTcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("nats-server did not become ready on port {}", self.port);
    }
}

impl Drop for TestNatsServer {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

struct TestProtocolAdapter {
    kind: ProtocolKind,
    transport: &'static str,
}

impl TestProtocolAdapter {
    fn new(kind: ProtocolKind, transport: &'static str) -> Self {
        Self { kind, transport }
    }
}

#[async_trait]
impl ProtocolAdapter for TestProtocolAdapter {
    fn kind(&self) -> ProtocolKind {
        self.kind
    }

    fn patterns(&self) -> &'static [CommunicationPattern] {
        &[CommunicationPattern::RequestResponse]
    }

    async fn call(&self, call: ProtocolCall) -> Result<ServiceResponse> {
        let mut metadata = call.request.metadata;
        metadata.insert("transport".to_string(), self.transport.to_string());
        Ok(ServiceResponse {
            payload: call.request.payload,
            metadata,
        })
    }
}
