//! 中间件管理模块

pub mod cors;
pub mod jwt;
pub mod logging;
pub mod static_files;

use crate::{builder::MiddlewareFactory, config::Config, error::Result};
use axum::Router;

/// 中间件管理器
///
/// 负责根据配置加载和管理各种中间件
#[derive(Debug)]
pub struct MiddlewareManager {
    config: Config,
    custom_middleware: Vec<Box<dyn MiddlewareFactory>>,
}

impl MiddlewareManager {
    /// 创建新的中间件管理器
    pub fn new(config: Config) -> Self {
        Self {
            config,
            custom_middleware: Vec::new(),
        }
    }

    /// 添加自定义中间件
    pub fn add_custom_middleware(&mut self, middleware: Box<dyn MiddlewareFactory>) {
        self.custom_middleware.push(middleware);
    }

    /// 应用所有中间件到路由器
    pub async fn apply_middleware(&self, mut app: Router) -> Result<Router> {
        // 应用日志中间件
        if self.config.middleware.logging.requests {
            app = self.apply_logging_middleware(app)?;
        }

        // 应用 CORS 中间件
        if self.config.middleware.cors.enabled {
            app = self.apply_cors_middleware(app)?;
        }

        // 应用静态文件中间件
        if self.config.middleware.static_files.enabled {
            app = self.apply_static_files_middleware(app).await?;
        }

        // 应用自定义中间件
        for middleware in &self.custom_middleware {
            tracing::info!("应用自定义中间件: {}", middleware.name());
            // 这里需要更复杂的实现来处理不同类型的中间件
            // 现在先简单处理
        }

        Ok(app)
    }

    /// 应用日志中间件
    fn apply_logging_middleware(&self, app: Router) -> Result<Router> {
        use tower_http::trace::TraceLayer;

        let trace_layer = TraceLayer::new_for_http()
            .make_span_with(|request: &axum::http::Request<_>| {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    uri = %request.uri(),
                )
            })
            .on_response(
                |response: &axum::http::Response<_>,
                 latency: std::time::Duration,
                 _span: &tracing::Span| {
                    tracing::info!(
                        status = %response.status(),
                        latency = ?latency,
                        "响应完成"
                    );
                },
            );

        Ok(app.layer(trace_layer))
    }

    /// 应用 CORS 中间件
    fn apply_cors_middleware(&self, app: Router) -> Result<Router> {
        let cors_config = &self.config.middleware.cors;
        let cors_layer = cors::create_cors_layer(cors_config)?;
        Ok(app.layer(cors_layer))
    }

    /// 应用静态文件中间件
    async fn apply_static_files_middleware(&self, app: Router) -> Result<Router> {
        let static_config = &self.config.middleware.static_files;
        static_files::apply_static_files(app, static_config).await
    }
}
