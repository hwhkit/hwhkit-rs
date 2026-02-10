//! # HwhKit
//!
//! 一个用于快速构建 Web 服务的 Rust 工具库，支持前后端分离和不分离两种架构。
//!
//! ## 特性
//!
//! - 🚀 一键构建 Web 服务
//! - 🔧 灵活的中间件系统
//! - 📝 支持模板渲染（前后端不分离）
//! - 🌐 支持 API 服务（前后端分离）
//! - ⚙️ 基于配置的中间件装载
//!
//! ## 快速开始
//!
//! ```no_run
//! use hwhkit::WebServerBuilder;
//!
//! #[tokio::main]
//! async fn main() {
//!     let app = WebServerBuilder::new()
//!         .config_from_file("config.toml")
//!         .build()
//!         .await
//!         .expect("Failed to build server");
//!
//!     app.serve().await;
//! }
//! ```

#[cfg(feature = "config-v2")]
pub mod bootstrap_v2;
pub mod builder;
pub mod config;
pub mod error;
pub mod middleware;
pub mod server;

#[cfg(feature = "templates")]
pub mod templates;

#[cfg(feature = "config-v2")]
pub use bootstrap_v2::run as run_v2;
pub use builder::WebServerBuilder;
pub use config::Config;
pub use error::{Error, Result};
pub use server::WebServer;

// 重新导出常用的类型
pub use axum::{
    extract::{Json, Path, Query, State},
    http::{Method, StatusCode},
    response::{Html, IntoResponse},
    routing::{delete, get, patch, post, put, Router},
};

pub use serde::{Deserialize, Serialize};
pub use tokio;
pub use tower_http::cors::CorsLayer;

// v2 模块的分阶段导出，保证现有 API 兼容。
#[cfg(feature = "config-v2")]
pub use hwhkit_config as config_v2;
#[cfg(feature = "config-v2")]
pub use hwhkit_core as core_v2;
#[cfg(feature = "mongodb")]
pub use hwhkit_integration_mongodb as mongodb_v2;
#[cfg(feature = "nats")]
pub use hwhkit_integration_nats as nats_v2;
#[cfg(feature = "neo4j")]
pub use hwhkit_integration_neo4j as neo4j_v2;
#[cfg(feature = "postgres")]
pub use hwhkit_integration_postgres as postgres_v2;
#[cfg(feature = "qdrant")]
pub use hwhkit_integration_qdrant as qdrant_v2;
#[cfg(feature = "redis")]
pub use hwhkit_integration_redis as redis_v2;
#[cfg(feature = "macros")]
pub use hwhkit_macros::{handler, main};
#[cfg(feature = "config-v2")]
pub use hwhkit_observability as observability_v2;
#[cfg(any(
    feature = "transport-grpc",
    feature = "transport-ws",
    feature = "transport-p2p"
))]
pub use hwhkit_transport as transport_v2;
