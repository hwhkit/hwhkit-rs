//! CORS 中间件模块

use crate::{
    config::CorsConfig,
    error::{Error, Result},
};
use axum::http::{HeaderValue, Method};
use tower_http::cors::{Any, CorsLayer};

/// 创建 CORS 中间件层
pub fn create_cors_layer(config: &CorsConfig) -> Result<CorsLayer> {
    let mut cors = CorsLayer::new();

    // 设置允许的源
    if config.origins.contains(&"*".to_string()) {
        cors = cors.allow_origin(Any);
    } else {
        let origins: Result<Vec<HeaderValue>> = config
            .origins
            .iter()
            .map(|origin| {
                origin
                    .parse()
                    .map_err(|e| Error::Config(format!("无效的 CORS 源 '{}': {}", origin, e)))
            })
            .collect();
        cors = cors.allow_origin(origins?);
    }

    // 设置允许的方法
    let methods: Result<Vec<Method>> = config
        .methods
        .iter()
        .map(|method| {
            method
                .parse()
                .map_err(|e| Error::Config(format!("无效的 HTTP 方法 '{}': {}", method, e)))
        })
        .collect();
    cors = cors.allow_methods(methods?);

    // 设置允许的头部
    if config.headers.contains(&"*".to_string()) {
        cors = cors.allow_headers(Any);
    } else {
        let headers: Result<Vec<axum::http::HeaderName>> = config
            .headers
            .iter()
            .map(|header| {
                header
                    .parse()
                    .map_err(|e| Error::Config(format!("无效的 HTTP 头部 '{}': {}", header, e)))
            })
            .collect();
        cors = cors.allow_headers(headers?);
    }

    Ok(cors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cors_layer_creation() {
        let config = CorsConfig {
            enabled: true,
            origins: vec!["http://localhost:3000".to_string()],
            methods: vec!["GET".to_string(), "POST".to_string()],
            headers: vec!["Content-Type".to_string()],
        };

        let result = create_cors_layer(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cors_wildcard_origins() {
        let config = CorsConfig {
            enabled: true,
            origins: vec!["*".to_string()],
            methods: vec!["GET".to_string()],
            headers: vec!["*".to_string()],
        };

        let result = create_cors_layer(&config);
        assert!(result.is_ok());
    }
}
