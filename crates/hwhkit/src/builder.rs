//! Web 服务构建器模块

use crate::{
    config::{ArchitectureType, Config},
    error::{Error, Result},
    middleware::MiddlewareManager,
    server::WebServer,
};
use axum::Router;
use std::path::Path;

/// Web 服务构建器
///
/// 这是 HwhKit 的核心构建器，用于配置和创建 Web 服务。
///
/// # Examples
///
/// ```no_run
/// use hwhkit::WebServerBuilder;
///
/// #[tokio::main]
/// async fn main() {
///     let server = WebServerBuilder::new()
///         .config_from_file("config.toml")
///         .build()
///         .await
///         .expect("Failed to build server");
///         
///     server.serve().await;
/// }
/// ```
#[derive(Debug)]
pub struct WebServerBuilder {
    config: Config,
    router: Option<Router>,
    custom_middleware: Vec<Box<dyn MiddlewareFactory>>,
}

/// 中间件工厂特征
///
/// 实现此特征的类型可以作为自定义中间件添加到服务器中
pub trait MiddlewareFactory: Send + Sync + std::fmt::Debug {
    /// 创建中间件层
    fn create_layer(&self) -> tower::layer::util::Identity;

    /// 中间件名称
    fn name(&self) -> &str;
}

impl Default for WebServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl WebServerBuilder {
    /// 创建新的构建器实例
    pub fn new() -> Self {
        Self {
            config: Config::default(),
            router: None,
            custom_middleware: Vec::new(),
        }
    }

    /// 从文件加载配置
    ///
    /// # Arguments
    ///
    /// * `path` - 配置文件路径
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use hwhkit::WebServerBuilder;
    ///
    /// let builder = WebServerBuilder::new()
    ///     .config_from_file("config.toml");
    /// ```
    pub fn config_from_file<P: AsRef<Path>>(mut self, path: P) -> Self {
        match Config::from_file(path) {
            Ok(config) => {
                self.config = config;
            }
            Err(e) => {
                eprintln!("警告: 无法加载配置文件: {}，使用默认配置", e);
            }
        }
        self
    }

    /// 设置配置
    ///
    /// # Arguments
    ///
    /// * `config` - 配置实例
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// 设置服务器监听地址
    ///
    /// # Arguments
    ///
    /// * `host` - 监听地址
    /// * `port` - 监听端口
    pub fn listen(mut self, host: &str, port: u16) -> Self {
        self.config.server.host = host.to_string();
        self.config.server.port = port;
        self
    }

    /// 设置架构类型
    ///
    /// # Arguments
    ///
    /// * `arch_type` - 架构类型
    pub fn architecture(mut self, arch_type: ArchitectureType) -> Self {
        self.config.server.architecture = arch_type;
        self
    }

    /// 启用 CORS
    ///
    /// # Arguments
    ///
    /// * `origins` - 允许的源列表
    pub fn cors(mut self, origins: Vec<String>) -> Self {
        self.config.middleware.cors.enabled = true;
        self.config.middleware.cors.origins = origins;
        self
    }

    /// 启用静态文件服务
    ///
    /// # Arguments
    ///
    /// * `dir` - 静态文件目录
    /// * `prefix` - URL 前缀
    pub fn static_files<P: AsRef<Path>>(mut self, dir: P, prefix: &str) -> Self {
        self.config.middleware.static_files.enabled = true;
        self.config.middleware.static_files.dir = dir.as_ref().to_string_lossy().to_string();
        self.config.middleware.static_files.prefix = prefix.to_string();
        self
    }

    /// 启用模板渲染（仅在 Full 架构下有效）
    ///
    /// # Arguments
    ///
    /// * `dir` - 模板目录
    /// * `extension` - 模板文件扩展名
    pub fn templates<P: AsRef<Path>>(mut self, dir: P, extension: &str) -> Self {
        if self.config.server.architecture == ArchitectureType::Full {
            self.config.middleware.templates.enabled = true;
            self.config.middleware.templates.dir = dir.as_ref().to_string_lossy().to_string();
            self.config.middleware.templates.extension = extension.to_string();
        }
        self
    }

    /// 启用 JWT 认证
    ///
    /// # Arguments
    ///
    /// * `secret` - JWT 密钥
    /// * `expires_in` - 过期时间（秒）
    pub fn jwt_auth(mut self, secret: &str, expires_in: u64) -> Self {
        self.config.middleware.jwt.enabled = true;
        self.config.middleware.jwt.secret = secret.to_string();
        self.config.middleware.jwt.expires_in = expires_in;
        self
    }

    /// 设置日志级别
    ///
    /// # Arguments
    ///
    /// * `level` - 日志级别 (trace, debug, info, warn, error)
    pub fn log_level(mut self, level: &str) -> Self {
        self.config.middleware.logging.level = level.to_string();
        self
    }

    /// 添加自定义路由
    ///
    /// # Arguments
    ///
    /// * `router` - Axum 路由器
    pub fn routes(mut self, router: Router) -> Self {
        self.router = Some(router);
        self
    }

    /// 添加自定义中间件
    ///
    /// # Arguments
    ///
    /// * `middleware` - 实现了 MiddlewareFactory 的中间件
    pub fn middleware<M: MiddlewareFactory + 'static>(mut self, middleware: M) -> Self {
        self.custom_middleware.push(Box::new(middleware));
        self
    }

    /// 添加自定义配置参数
    ///
    /// # Arguments
    ///
    /// * `key` - 参数键
    /// * `value` - 参数值
    pub fn custom_config<T: serde::Serialize>(mut self, key: &str, value: T) -> Self {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.config
                .middleware
                .custom
                .insert(key.to_string(), json_value);
        }
        self
    }

    /// 构建 Web 服务器
    ///
    /// 此方法会验证配置，初始化中间件，并创建 WebServer 实例。
    ///
    /// # Returns
    ///
    /// 返回配置好的 WebServer 实例或错误
    pub async fn build(self) -> Result<WebServer> {
        // 验证配置
        self.config.validate()?;

        // 初始化日志
        self.init_logging()?;

        // 创建中间件管理器
        let mut middleware_manager = MiddlewareManager::new(self.config.clone());

        // 添加自定义中间件
        for middleware in self.custom_middleware {
            middleware_manager.add_custom_middleware(middleware);
        }

        // 构建路由器
        let base_router = self.router.unwrap_or_default();
        let app = middleware_manager.apply_middleware(base_router).await?;

        // 创建服务器
        Ok(WebServer::new(app, self.config))
    }

    /// 初始化日志系统
    fn init_logging(&self) -> Result<()> {
        use tracing_subscriber::{fmt, prelude::*, EnvFilter};

        let filter = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new(&self.config.middleware.logging.level))
            .map_err(|e| Error::Config(format!("无效的日志级别: {}", e)))?;

        let fmt_layer = fmt::layer()
            .with_target(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false);

        // 尝试设置全局默认订阅者，如果失败则忽略（可能已经设置过）
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .try_init();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ArchitectureType;

    #[test]
    fn test_builder_creation() {
        let builder = WebServerBuilder::new();
        assert_eq!(builder.config.server.port, 3000);
        assert_eq!(builder.config.server.architecture, ArchitectureType::Api);
    }

    #[test]
    fn test_builder_configuration() {
        let builder = WebServerBuilder::new()
            .listen("127.0.0.1", 8080)
            .architecture(ArchitectureType::Full)
            .cors(vec!["http://localhost:3000".to_string()])
            .log_level("debug");

        assert_eq!(builder.config.server.host, "127.0.0.1");
        assert_eq!(builder.config.server.port, 8080);
        assert_eq!(builder.config.server.architecture, ArchitectureType::Full);
        assert!(builder.config.middleware.cors.enabled);
        assert_eq!(builder.config.middleware.logging.level, "debug");
    }
}
