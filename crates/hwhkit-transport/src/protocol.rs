use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use hwhkit_core::{Error as CoreError, Result as CoreResult, ServiceRequest, ServiceResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolKind {
    Http,
    WebSocket,
    Rpc,
    Nats,
    Socket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommunicationPattern {
    RequestResponse,
    Stream,
    PublishSubscribe,
    FireAndForget,
}

#[derive(Debug, Clone)]
pub struct ProtocolCall {
    pub target: String,
    pub request: ServiceRequest,
}

impl ProtocolCall {
    pub fn new(target: impl Into<String>, request: ServiceRequest) -> Self {
        Self {
            target: target.into(),
            request,
        }
    }
}

#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    fn kind(&self) -> ProtocolKind;
    fn patterns(&self) -> &'static [CommunicationPattern];
    async fn call(&self, call: ProtocolCall) -> CoreResult<ServiceResponse>;
}

#[derive(Clone, Default)]
pub struct AdapterRegistry {
    adapters: Arc<RwLock<HashMap<ProtocolKind, Arc<dyn ProtocolAdapter>>>>,
}

impl AdapterRegistry {
    pub fn register(&self, adapter: Arc<dyn ProtocolAdapter>) -> Option<Arc<dyn ProtocolAdapter>> {
        self.adapters
            .write()
            .expect("adapter registry lock poisoned")
            .insert(adapter.kind(), adapter)
    }

    pub fn get(&self, kind: ProtocolKind) -> Option<Arc<dyn ProtocolAdapter>> {
        self.adapters
            .read()
            .expect("adapter registry lock poisoned")
            .get(&kind)
            .cloned()
    }

    pub async fn call(
        &self,
        kind: ProtocolKind,
        call: ProtocolCall,
    ) -> CoreResult<ServiceResponse> {
        let adapter = self
            .get(kind)
            .ok_or_else(|| CoreError::Service(format!("protocol adapter not found: {:?}", kind)))?;
        adapter.call(call).await
    }
}

pub struct LoopbackAdapter;

#[async_trait]
impl ProtocolAdapter for LoopbackAdapter {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::Rpc
    }

    fn patterns(&self) -> &'static [CommunicationPattern] {
        &[CommunicationPattern::RequestResponse]
    }

    async fn call(&self, call: ProtocolCall) -> CoreResult<ServiceResponse> {
        let mut metadata = call.request.metadata;
        metadata.insert("transport".to_string(), "loopback-rpc".to_string());

        Ok(ServiceResponse {
            payload: call.request.payload,
            metadata,
        })
    }
}
