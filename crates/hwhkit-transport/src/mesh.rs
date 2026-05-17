use async_trait::async_trait;
use hwhkit_config::{MeshConfig, ServiceRouteConfig};
use hwhkit_core::{
    Error as CoreError, Result as CoreResult, ServiceClient, ServiceRequest, ServiceResponse,
    ServiceTarget,
};
use std::collections::HashMap;

use crate::protocol::{AdapterRegistry, ProtocolCall, ProtocolKind};

#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    pub service: String,
    pub protocol: ProtocolKind,
    pub target: String,
    pub serializer: String,
    pub timeout_ms: u64,
}

#[derive(Clone, Default)]
pub struct MeshClient {
    routes: HashMap<String, ResolvedRoute>,
    adapters: AdapterRegistry,
}

impl MeshClient {
    pub fn new(routes: HashMap<String, ResolvedRoute>, adapters: AdapterRegistry) -> Self {
        Self { routes, adapters }
    }

    pub fn from_config(config: &MeshConfig, adapters: AdapterRegistry) -> CoreResult<Self> {
        let mut routes = HashMap::new();

        for (service, route) in &config.services {
            routes.insert(service.clone(), resolve_route(service, route)?);
        }

        Ok(Self { routes, adapters })
    }

    pub fn route(&self, service: &str) -> Option<&ResolvedRoute> {
        self.routes.get(service)
    }
}

#[async_trait]
impl ServiceClient for MeshClient {
    async fn call(
        &self,
        target: ServiceTarget,
        mut request: ServiceRequest,
    ) -> CoreResult<ServiceResponse> {
        let route = self.routes.get(&target.service).ok_or_else(|| {
            CoreError::Service(format!("mesh route not configured for {}", target.service))
        })?;

        request
            .metadata
            .insert("serializer".to_string(), route.serializer.clone());
        request
            .metadata
            .insert("timeout_ms".to_string(), route.timeout_ms.to_string());
        request
            .metadata
            .insert("service".to_string(), route.service.clone());

        self.adapters
            .call(
                route.protocol,
                ProtocolCall::new(route.target.clone(), request),
            )
            .await
    }
}

fn resolve_route(service: &str, route: &ServiceRouteConfig) -> CoreResult<ResolvedRoute> {
    let protocol = parse_protocol(&route.protocol)?;
    let target = if !route.subject.trim().is_empty() {
        route.subject.clone()
    } else {
        route.endpoint.clone()
    };

    if target.trim().is_empty() {
        return Err(CoreError::Service(format!(
            "mesh route target cannot be empty for {service}"
        )));
    }

    Ok(ResolvedRoute {
        service: service.to_string(),
        protocol,
        target,
        serializer: route.serializer.clone(),
        timeout_ms: route.timeout_ms,
    })
}

fn parse_protocol(value: &str) -> CoreResult<ProtocolKind> {
    match value.to_ascii_lowercase().as_str() {
        "http" => Ok(ProtocolKind::Http),
        "ws" | "websocket" => Ok(ProtocolKind::WebSocket),
        "rpc" | "grpc" => Ok(ProtocolKind::Rpc),
        "nats" => Ok(ProtocolKind::Nats),
        "socket" | "tcp" => Ok(ProtocolKind::Socket),
        _ => Err(CoreError::Service(format!(
            "unsupported mesh protocol: {value}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::{AdapterRegistry, LoopbackAdapter};

    #[tokio::test]
    async fn mesh_client_dispatches_using_configured_route() {
        let mut config = MeshConfig::default();
        config.services.insert(
            "user".to_string(),
            ServiceRouteConfig {
                mode: "remote".to_string(),
                protocol: "rpc".to_string(),
                endpoint: "svc.user".to_string(),
                subject: String::new(),
                serializer: "json".to_string(),
                timeout_ms: 1200,
            },
        );

        let adapters = AdapterRegistry::default();
        adapters.register(Arc::new(LoopbackAdapter));

        let client = MeshClient::from_config(&config, adapters).expect("mesh client should build");
        let response = client
            .call(
                ServiceTarget::remote("user"),
                ServiceRequest::new("Ping", b"pong".to_vec()),
            )
            .await
            .expect("mesh call should succeed");

        assert_eq!(response.payload, b"pong".to_vec());
    }
}
