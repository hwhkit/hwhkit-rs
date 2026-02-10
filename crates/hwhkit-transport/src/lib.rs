use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::broadcast;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transport config error: {0}")]
    Config(String),
    #[error("transport runtime error: {0}")]
    Runtime(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RpcBackend {
    Grpc,
    Nats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            grpc: GrpcTransportConfig::default(),
            rpc: RpcTransportConfig::default(),
            nats: NatsTransportConfig::default(),
            websocket: WebsocketTransportConfig::default(),
            p2p: P2pTransportConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrpcTransportConfig {
    pub enabled: bool,
    pub listen: String,
    pub reflection: bool,
}

impl Default for GrpcTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "0.0.0.0:50051".to_string(),
            reflection: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcTransportConfig {
    pub enabled: bool,
    pub default: RpcBackend,
    pub timeout_ms: u64,
}

impl Default for RpcTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default: RpcBackend::Grpc,
            timeout_ms: 3_000,
        }
    }
}

impl RpcTransportConfig {
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
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
    pub heartbeat_seconds: u64,
}

impl Default for WebsocketTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "/ws".to_string(),
            max_connections: 10_000,
            heartbeat_seconds: 20,
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

#[derive(Debug, Clone)]
pub struct RpcRequest {
    pub method: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RpcResponse {
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct EventMessage {
    pub topic: String,
    pub payload: Vec<u8>,
}

#[async_trait]
pub trait RpcClient: Send + Sync {
    async fn call(&self, request: RpcRequest) -> Result<RpcResponse>;
}

#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, event: EventMessage) -> Result<()>;
    async fn subscribe(&self) -> Result<EventSubscriber>;
}

pub struct EventSubscriber {
    inner: broadcast::Receiver<EventMessage>,
}

impl EventSubscriber {
    pub async fn recv(&mut self) -> Result<EventMessage> {
        self.inner
            .recv()
            .await
            .map_err(|e| Error::Runtime(e.to_string()))
    }
}

#[derive(Clone)]
pub struct MemoryEventBus {
    tx: broadcast::Sender<EventMessage>,
}

impl MemoryEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }
}

#[async_trait]
impl EventBus for MemoryEventBus {
    async fn publish(&self, event: EventMessage) -> Result<()> {
        self.tx
            .send(event)
            .map(|_| ())
            .map_err(|e| Error::Runtime(e.to_string()))
    }

    async fn subscribe(&self) -> Result<EventSubscriber> {
        Ok(EventSubscriber {
            inner: self.tx.subscribe(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct GrpcRpcClient {
    pub endpoint: String,
}

#[async_trait]
impl RpcClient for GrpcRpcClient {
    async fn call(&self, request: RpcRequest) -> Result<RpcResponse> {
        // 占位实现：后续接入 tonic 客户端。
        if self.endpoint.trim().is_empty() {
            return Err(Error::Runtime("grpc endpoint cannot be empty".to_string()));
        }
        Ok(RpcResponse {
            payload: request.payload,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NatsRpcClient {
    pub endpoint: String,
}

#[async_trait]
impl RpcClient for NatsRpcClient {
    async fn call(&self, request: RpcRequest) -> Result<RpcResponse> {
        // 占位实现：后续接入 async-nats request/reply。
        if self.endpoint.trim().is_empty() {
            return Err(Error::Runtime("nats endpoint cannot be empty".to_string()));
        }
        Ok(RpcResponse {
            payload: request.payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_event_bus_roundtrip() {
        let bus = MemoryEventBus::new(16);
        let mut subscriber = bus.subscribe().await.expect("subscribe should succeed");

        bus.publish(EventMessage {
            topic: "events.user.created".to_string(),
            payload: b"demo".to_vec(),
        })
        .await
        .expect("publish should succeed");

        let msg = subscriber.recv().await.expect("receive should succeed");
        assert_eq!(msg.topic, "events.user.created");
    }
}
