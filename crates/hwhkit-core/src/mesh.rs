use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationMode {
    Local,
    Remote,
    Auto,
}

#[derive(Debug, Clone)]
pub struct ServiceRequest {
    pub operation: String,
    pub payload: Vec<u8>,
    pub metadata: HashMap<String, String>,
}

impl ServiceRequest {
    pub fn new(operation: impl Into<String>, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            operation: operation.into(),
            payload: payload.into(),
            metadata: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ServiceResponse {
    pub payload: Vec<u8>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ServiceTarget {
    pub service: String,
    pub mode: InvocationMode,
}

impl ServiceTarget {
    pub fn local(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            mode: InvocationMode::Local,
        }
    }

    pub fn remote(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            mode: InvocationMode::Remote,
        }
    }

    pub fn auto(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            mode: InvocationMode::Auto,
        }
    }
}

#[async_trait]
pub trait LocalService: Send + Sync {
    async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse>;
}

#[async_trait]
pub trait ServiceClient: Send + Sync {
    async fn call(&self, target: ServiceTarget, request: ServiceRequest)
        -> Result<ServiceResponse>;
}

#[derive(Clone, Default)]
pub struct ServiceRegistry {
    local_services: Arc<RwLock<HashMap<String, Arc<dyn LocalService>>>>,
    client: Arc<RwLock<Option<Arc<dyn ServiceClient>>>>,
}

impl ServiceRegistry {
    pub fn register_local(
        &self,
        name: impl Into<String>,
        service: Arc<dyn LocalService>,
    ) -> Option<Arc<dyn LocalService>> {
        self.local_services
            .write()
            .expect("service registry lock poisoned")
            .insert(name.into(), service)
    }

    pub fn set_client(&self, client: Arc<dyn ServiceClient>) {
        *self.client.write().expect("service registry lock poisoned") = Some(client);
    }

    pub fn has_local(&self, name: &str) -> bool {
        self.local_services
            .read()
            .expect("service registry lock poisoned")
            .contains_key(name)
    }

    pub async fn call(
        &self,
        target: ServiceTarget,
        request: ServiceRequest,
    ) -> Result<ServiceResponse> {
        match target.mode {
            InvocationMode::Local => self.call_local(&target.service, request).await,
            InvocationMode::Remote => self.call_remote(target, request).await,
            InvocationMode::Auto => {
                if self.has_local(&target.service) {
                    self.call_local(&target.service, request).await
                } else {
                    self.call_remote(target, request).await
                }
            }
        }
    }

    async fn call_local(&self, name: &str, request: ServiceRequest) -> Result<ServiceResponse> {
        let service = self
            .local_services
            .read()
            .expect("service registry lock poisoned")
            .get(name)
            .cloned()
            .ok_or_else(|| Error::Service(format!("local service not found: {name}")))?;
        service.call(request).await
    }

    async fn call_remote(
        &self,
        target: ServiceTarget,
        request: ServiceRequest,
    ) -> Result<ServiceResponse> {
        let client = self
            .client
            .read()
            .expect("service registry lock poisoned")
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                Error::Service(format!(
                    "remote service client not configured for {}",
                    target.service
                ))
            })?;

        client.call(target, request).await
    }
}
