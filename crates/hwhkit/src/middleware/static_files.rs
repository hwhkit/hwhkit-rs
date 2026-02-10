//! 静态文件中间件模块

use crate::{
    config::StaticConfig,
    error::{Error, Result},
};
use axum::{routing::get_service, Router};
use std::path::Path;
use tower_http::services::ServeDir;

/// 应用静态文件服务
pub async fn apply_static_files(app: Router, config: &StaticConfig) -> Result<Router> {
    let static_dir = Path::new(&config.dir);

    // 检查静态文件目录是否存在
    if !static_dir.exists() {
        return Err(Error::Config(format!("静态文件目录不存在: {}", config.dir)));
    }

    // 创建静态文件服务
    let serve_dir = ServeDir::new(&config.dir);

    // 添加到路由
    let static_route = format!("{}/*file", config.prefix.trim_end_matches('/'));

    tracing::info!("启用静态文件服务: {} -> {}", static_route, config.dir);

    Ok(app.route(&static_route, get_service(serve_dir)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_static_files_with_existing_dir() {
        let temp_dir = TempDir::new().unwrap();
        let config = StaticConfig {
            enabled: true,
            dir: temp_dir.path().to_string_lossy().to_string(),
            prefix: "/static".to_string(),
        };

        let app = Router::new();
        let result = apply_static_files(app, &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_static_files_with_nonexistent_dir() {
        let config = StaticConfig {
            enabled: true,
            dir: "/nonexistent/directory".to_string(),
            prefix: "/static".to_string(),
        };

        let app = Router::new();
        let result = apply_static_files(app, &config).await;
        assert!(result.is_err());
    }
}
