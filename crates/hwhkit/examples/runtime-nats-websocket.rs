use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use hwhkit::{
    config_v2::{AppConfig, BootstrapConfig},
    core_v2::{AppContext, MessageBusResource, Module, ResourceKind, Result},
    RuntimeBuilder,
};
use hwhkit_integration_nats::NatsHandle;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

struct EventModule;

#[derive(Debug, Deserialize)]
struct PublishEventRequest {
    topic: String,
    payload: String,
}

#[derive(Debug, Serialize)]
struct PublishEventResponse {
    delivered_to: String,
    bytes: usize,
}

#[async_trait]
impl Module for EventModule {
    fn name(&self) -> &'static str {
        "events"
    }

    async fn router(&self, ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new()
            .route("/healthz", get(healthz))
            .route("/api/v1/events/publish", post(publish_event))
            .route("/ws/events/:topic", get(stream_events))
            .with_state(ctx))
    }
}

async fn healthz() -> &'static str {
    "ok"
}

async fn publish_event(
    State(ctx): State<AppContext>,
    Json(request): Json<PublishEventRequest>,
) -> std::result::Result<Json<PublishEventResponse>, StatusCode> {
    let bus = ctx
        .resource::<NatsHandle>(ResourceKind::MessageBus, "default")
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    bus.publish(&request.topic, request.payload.clone().into_bytes())
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(Json(PublishEventResponse {
        delivered_to: request.topic,
        bytes: request.payload.len(),
    }))
}

async fn stream_events(
    ws: WebSocketUpgrade,
    State(ctx): State<AppContext>,
    Path(topic): Path<String>,
) -> std::result::Result<impl IntoResponse, StatusCode> {
    let handle = ctx
        .native::<NatsHandle>()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    Ok(ws.on_upgrade(move |socket| websocket_bridge(socket, handle, topic)))
}

async fn websocket_bridge(mut socket: WebSocket, handle: Arc<NatsHandle>, topic: String) {
    let mut subscriber = match handle.subscribe(&topic).await {
        Ok(subscriber) => subscriber,
        Err(err) => {
            let _ = socket
                .send(Message::Text(format!("nats subscribe failed: {err}")))
                .await;
            return;
        }
    };

    while let Some(message) = subscriber.next().await {
        let payload = String::from_utf8_lossy(&message.payload).to_string();
        if socket.send(Message::Text(payload)).await.is_err() {
            break;
        }
    }
}

fn example_config_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("runtime-nats-config")
}

#[tokio::main]
async fn main() -> Result<()> {
    let built = RuntimeBuilder::new()
        .bootstrap(
            BootstrapConfig::default()
                .with_service_name("runtime-nats-websocket")
                .with_config_dir(example_config_dir()),
        )
        .module(EventModule)
        .build()
        .await?;

    let address = format!("{}:{}", built.config.server.host, built.config.server.port);
    let listener = TcpListener::bind(&address)
        .await
        .map_err(|err| hwhkit::core_v2::Error::Bootstrap(err.to_string()))?;

    println!("runtime nats websocket demo listening on http://{address}");
    println!("publish: POST http://{address}/api/v1/events/publish");
    println!("stream: ws://{address}/ws/events/events.demo");
    println!("requires local nats-server on {}", built.config.integrations.messaging.nats.url);

    axum::serve(listener, built.router)
        .await
        .map_err(|err| hwhkit::core_v2::Error::Bootstrap(err.to_string()))
}
