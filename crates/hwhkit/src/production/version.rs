//! `/version` and `/info` endpoints. JSON responses with compile-time
//! build info (git sha, build time, rustc version, crate version).

use axum::{routing::get, Json, Router};
use hwhkit_buildinfo::{BuildInfo, BUILD_TIME_UNIX, GIT_SHA, RUST_VERSION};
use hwhkit_config::InfoConfig;
use serde::Serialize;

#[derive(Serialize, Clone)]
#[non_exhaustive]
pub struct VersionResponse {
    pub version: &'static str,
    pub git_sha: &'static str,
    pub build_time_unix: &'static str,
    pub rust_version: &'static str,
}

#[derive(Serialize, Clone)]
#[non_exhaustive]
pub struct InfoResponse {
    pub service_name: String,
    pub environment: String,
    pub build: BuildInfo,
    pub initialized_integrations: Vec<String>,
    pub degraded_integrations: Vec<String>,
}

pub fn router(cfg: &InfoConfig, info: InfoResponse, version: VersionResponse) -> Router {
    let info = std::sync::Arc::new(info);
    let version = std::sync::Arc::new(version);

    Router::new()
        .route(
            &cfg.path_version,
            get({
                let v = version.clone();
                move || {
                    let v = v.clone();
                    async move { Json((*v).clone()) }
                }
            }),
        )
        .route(
            &cfg.path_info,
            get({
                let i = info.clone();
                move || {
                    let i = i.clone();
                    async move { Json((*i).clone()) }
                }
            }),
        )
}

/// Build a default version payload using the workspace build info.
pub fn default_version() -> VersionResponse {
    VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
        git_sha: GIT_SHA,
        build_time_unix: BUILD_TIME_UNIX,
        rust_version: RUST_VERSION,
    }
}
