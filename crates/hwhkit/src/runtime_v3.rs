use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use hwhkit_config::{AppConfig, BootstrapConfig, ConfigLoader};
use hwhkit_core::{
    bootstrap_with, AppContext, Application, BuiltApplication, IntegrationProvider, Module, Result,
    RuntimeFeatures, ServiceClient,
};
use hwhkit_transport::{AdapterRegistry, MeshClient, ProtocolAdapter};

use crate::bootstrap_v2::{default_providers, runtime_features};

pub struct RuntimeBuilder {
    bootstrap: BootstrapConfig,
    loader: ConfigLoader,
    runtime_features: RuntimeFeatures,
    providers: Vec<Arc<dyn IntegrationProvider>>,
    modules: Vec<Arc<dyn Module>>,
    adapters: AdapterRegistry,
    service_client: Option<Arc<dyn ServiceClient>>,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            bootstrap: BootstrapConfig::default(),
            loader: ConfigLoader::default(),
            runtime_features: runtime_features(),
            providers: default_providers(),
            modules: Vec::new(),
            adapters: AdapterRegistry::default(),
            service_client: None,
        }
    }

    pub fn bootstrap(mut self, bootstrap: BootstrapConfig) -> Self {
        self.bootstrap = bootstrap;
        self
    }

    pub fn config_loader(mut self, loader: ConfigLoader) -> Self {
        self.loader = loader;
        self
    }

    pub fn runtime_features(mut self, runtime_features: RuntimeFeatures) -> Self {
        self.runtime_features = runtime_features;
        self
    }

    pub fn disable_default_providers(mut self) -> Self {
        self.providers.clear();
        self
    }

    pub fn provider<P>(mut self, provider: P) -> Self
    where
        P: IntegrationProvider + 'static,
    {
        self.providers.push(Arc::new(provider));
        self
    }

    pub fn module<M>(mut self, module: M) -> Self
    where
        M: Module + 'static,
    {
        self.modules.push(Arc::new(module));
        self
    }

    pub fn adapter(self, adapter: Arc<dyn ProtocolAdapter>) -> Self {
        self.adapters.register(adapter);
        self
    }

    pub fn service_client(mut self, client: Arc<dyn ServiceClient>) -> Self {
        self.service_client = Some(client);
        self
    }

    pub async fn build(self) -> Result<BuiltApplication> {
        let app = ModuleApplication {
            modules: self.modules,
            adapters: self.adapters,
            service_client: self.service_client,
        };

        bootstrap_with(
            app,
            self.bootstrap,
            self.loader,
            self.runtime_features,
            self.providers,
        )
        .await
    }
}

struct ModuleApplication {
    modules: Vec<Arc<dyn Module>>,
    adapters: AdapterRegistry,
    service_client: Option<Arc<dyn ServiceClient>>,
}

#[async_trait]
impl Application for ModuleApplication {
    async fn build_router(&self, ctx: AppContext, cfg: &AppConfig) -> Result<Router> {
        if let Some(client) = &self.service_client {
            ctx.set_service_client(Arc::clone(client));
        } else if !cfg.mesh.services.is_empty() {
            let client = Arc::new(MeshClient::from_config(&cfg.mesh, self.adapters.clone())?);
            ctx.set_service_client(client);
        }

        let mut router = Router::new();
        for module in &self.modules {
            module.register_services(ctx.clone(), cfg).await?;
            router = router.merge(module.router(ctx.clone(), cfg).await?);
        }

        Ok(router)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwhkit_core::{LocalService, ServiceRequest, ServiceResponse, ServiceTarget};
    use hwhkit_transport::LoopbackAdapter;
    use std::{
        fs,
        sync::Arc,
    };
    use tempfile::TempDir;

    struct EchoModule;
    struct EchoService;

    #[async_trait]
    impl LocalService for EchoService {
        async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse> {
            Ok(ServiceResponse {
                payload: request.payload,
                metadata: request.metadata,
            })
        }
    }

    #[async_trait]
    impl Module for EchoModule {
        fn name(&self) -> &'static str {
            "echo"
        }

        async fn register_services(&self, ctx: AppContext, _cfg: &AppConfig) -> Result<()> {
            ctx.register_local_service("echo", Arc::new(EchoService));
            Ok(())
        }

        async fn router(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
            async fn health() -> &'static str {
                "ok"
            }

            Ok(Router::new().route("/healthz", axum::routing::get(health)))
        }
    }

    #[tokio::test]
    async fn runtime_builder_registers_module_services() {
        let config_dir = make_config_dir();
        fs::write(
            config_dir.path().join("default.toml"),
            r#"
[server]
host = "127.0.0.1"
port = 3000

[observability]
service_name = "module-app"
environment = "test"
"#,
        )
        .expect("default config should be written");
        fs::write(config_dir.path().join("test.toml"), "").expect("test config should be written");

        let built = RuntimeBuilder::new()
            .disable_default_providers()
            .bootstrap(
                BootstrapConfig::default()
                    .with_environment(hwhkit_config::Environment::Test)
                    .with_config_dir(config_dir.path()),
            )
            .module(EchoModule)
            .build()
            .await
            .expect("runtime builder should succeed");

        let response = built
            .context
            .call_service(
                ServiceTarget::local("echo"),
                ServiceRequest::new("Echo", b"hello".to_vec()),
            )
            .await
            .expect("local service should be registered");

        assert_eq!(response.payload, b"hello".to_vec());
    }

    #[tokio::test]
    async fn runtime_builder_builds_mesh_client_from_config() {
        let config_dir = make_config_dir();
        fs::write(
            config_dir.path().join("default.toml"),
            r#"
[server]
host = "127.0.0.1"
port = 3001

[observability]
service_name = "mesh-app"
environment = "test"

[mesh.services.echo]
mode = "remote"
protocol = "rpc"
endpoint = "svc.echo"
subject = ""
serializer = "json"
timeout_ms = 1500
"#,
        )
        .expect("default config should be written");
        fs::write(config_dir.path().join("test.toml"), "").expect("test config should be written");

        let built = RuntimeBuilder::new()
            .disable_default_providers()
            .bootstrap(
                BootstrapConfig::default()
                    .with_environment(hwhkit_config::Environment::Test)
                    .with_config_dir(config_dir.path()),
            )
            .adapter(Arc::new(LoopbackAdapter))
            .build()
            .await
            .expect("runtime builder should succeed");

        assert!(built.config.mesh.services.contains_key("echo"));

        let response = built
            .context
            .call_service(
                ServiceTarget::auto("echo"),
                ServiceRequest::new("Echo", b"hello".to_vec()),
            )
            .await
            .expect("mesh-backed remote service should be registered");

        assert_eq!(response.payload, b"hello".to_vec());
        assert_eq!(
            response.metadata.get("service").map(String::as_str),
            Some("echo")
        );
        assert_eq!(
            response.metadata.get("serializer").map(String::as_str),
            Some("json")
        );
        assert_eq!(
            response.metadata.get("timeout_ms").map(String::as_str),
            Some("1500")
        );
    }

    fn make_config_dir() -> TempDir {
        tempfile::tempdir().expect("temp config dir should be created")
    }
}
