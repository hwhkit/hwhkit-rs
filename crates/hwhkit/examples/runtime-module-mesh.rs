use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use hwhkit::{
    config_v2::{AppConfig, BootstrapConfig},
    core_v2::{AppContext, LocalService, Module, Result, ServiceRequest, ServiceResponse, ServiceTarget},
    RuntimeBuilder,
};
use hwhkit_transport::LoopbackAdapter;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserProfile {
    id: u64,
    name: String,
    tier: String,
}

struct ProfilesModule;
struct ProfilesService;

#[async_trait]
impl LocalService for ProfilesService {
    async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse> {
        let id = String::from_utf8(request.payload)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1);

        let profile = UserProfile {
            id,
            name: format!("user-{id}"),
            tier: if id.is_multiple_of(2) { "pro" } else { "free" }.to_string(),
        };

        Ok(ServiceResponse {
            payload: serde_json::to_vec(&profile)
                .map_err(|err| hwhkit::core_v2::Error::Service(err.to_string()))?,
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
            .route("/healthz", get(healthz))
            .route("/api/v1/profiles/:id", get(get_profile))
            .with_state(ctx))
    }
}

async fn healthz() -> &'static str {
    "ok"
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

    let profile: UserProfile = serde_json::from_slice(&response.payload)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(profile))
}

fn example_config_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("runtime-module-config")
}

#[tokio::main]
async fn main() -> Result<()> {
    let built = RuntimeBuilder::new()
        .disable_default_providers()
        .bootstrap(
            BootstrapConfig::default()
                .with_service_name("runtime-module-mesh")
                .with_config_dir(example_config_dir()),
        )
        .adapter(Arc::new(LoopbackAdapter))
        .module(ProfilesModule)
        .build()
        .await?;

    let address = format!("{}:{}", built.config.server.host, built.config.server.port);
    let listener = TcpListener::bind(&address)
        .await
        .map_err(|err| hwhkit::core_v2::Error::Bootstrap(err.to_string()))?;

    println!("runtime builder demo listening on http://{address}");
    println!("healthz: http://{address}/healthz");
    println!("profile: http://{address}/api/v1/profiles/7");

    axum::serve(listener, built.router)
        .await
        .map_err(|err| hwhkit::core_v2::Error::Bootstrap(err.to_string()))
}
