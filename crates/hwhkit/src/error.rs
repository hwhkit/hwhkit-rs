//! 错误处理模块

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// HwhKit 的主要错误类型
#[derive(Error, Debug)]
pub enum Error {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("序列化错误: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("TOML 解析错误: {0}")]
    TomlParsing(#[from] toml::de::Error),

    #[error("服务器启动失败: {0}")]
    ServerStart(String),

    #[error("中间件错误: {0}")]
    Middleware(String),

    #[error("模板错误: {0}")]
    #[cfg(feature = "templates")]
    Template(#[from] tera::Error),

    #[error("JWT 错误: {0}")]
    #[cfg(feature = "jwt")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("内部错误: {0}")]
    Internal(String),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            Error::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            Error::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            Error::Serialization(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Error::TomlParsing(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            Error::ServerStart(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            Error::Middleware(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            #[cfg(feature = "templates")]
            Error::Template(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            #[cfg(feature = "jwt")]
            Error::Jwt(_) => (StatusCode::UNAUTHORIZED, "认证失败".to_string()),
            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = Json(json!({
            "error": error_message,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}

/// HwhKit 的结果类型
pub type Result<T> = std::result::Result<T, Error>;
