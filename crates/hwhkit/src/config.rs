//! 配置管理模块

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

/// 服务器架构类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ArchitectureType {
    /// 前后端分离架构（纯 API）
    Api,
    /// 前后端不分离架构（包含模板渲染）
    Full,
}

impl Default for ArchitectureType {
    fn default() -> Self {
        Self::Api
    }
}

/// 服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 服务器监听地址
    pub host: String,
    /// 服务器监听端口
    pub port: u16,
    /// 架构类型
    pub architecture: ArchitectureType,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
            architecture: ArchitectureType::default(),
        }
    }
}

/// CORS 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    /// 是否启用 CORS
    pub enabled: bool,
    /// 允许的源
    pub origins: Vec<String>,
    /// 允许的方法
    pub methods: Vec<String>,
    /// 允许的头部
    pub headers: Vec<String>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            origins: vec!["*".to_string()],
            methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "PATCH".to_string(),
                "OPTIONS".to_string(),
            ],
            headers: vec!["*".to_string()],
        }
    }
}

/// JWT 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtConfig {
    /// 是否启用 JWT
    pub enabled: bool,
    /// JWT 密钥
    pub secret: String,
    /// Token 过期时间（秒）
    pub expires_in: u64,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            secret: "your-secret-key-change-this-in-production".to_string(),
            expires_in: 3600,
        }
    }
}

/// 静态文件配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticConfig {
    /// 是否启用静态文件服务
    pub enabled: bool,
    /// 静态文件目录路径
    pub dir: String,
    /// URL 前缀
    pub prefix: String,
}

impl Default for StaticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: "static".to_string(),
            prefix: "/static".to_string(),
        }
    }
}

/// 模板配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateConfig {
    /// 是否启用模板渲染
    pub enabled: bool,
    /// 模板文件目录
    pub dir: String,
    /// 模板文件扩展名
    pub extension: String,
}

impl Default for TemplateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: "templates".to_string(),
            extension: "html".to_string(),
        }
    }
}

/// 日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// 日志级别
    pub level: String,
    /// 是否启用请求日志
    pub requests: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            requests: true,
        }
    }
}

/// 中间件配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MiddlewareConfig {
    /// CORS 配置
    pub cors: CorsConfig,
    /// JWT 配置
    pub jwt: JwtConfig,
    /// 静态文件配置
    pub static_files: StaticConfig,
    /// 模板配置
    pub templates: TemplateConfig,
    /// 日志配置
    pub logging: LogConfig,
    /// 自定义中间件参数
    pub custom: HashMap<String, serde_json::Value>,
}

/// 主配置结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// 服务器配置
    pub server: ServerConfig,
    /// 中间件配置
    pub middleware: MiddlewareConfig,
}

impl Config {
    /// 从文件加载配置
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| Error::Config(format!("无法读取配置文件 {:?}: {}", path.as_ref(), e)))?;

        let config: Config = toml::from_str(&content)
            .map_err(|e| Error::Config(format!("解析配置文件失败: {}", e)))?;

        Ok(config)
    }

    /// 保存配置到文件
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("序列化配置失败: {}", e)))?;

        std::fs::write(path.as_ref(), content)
            .map_err(|e| Error::Config(format!("写入配置文件失败: {}", e)))?;

        Ok(())
    }

    /// 获取服务器地址
    pub fn server_address(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }

    /// 验证配置的有效性
    pub fn validate(&self) -> Result<()> {
        // 验证端口范围
        if self.server.port == 0 {
            return Err(Error::Config("端口号不能为 0".to_string()));
        }

        // 如果启用了模板，检查架构类型
        if self.middleware.templates.enabled && self.server.architecture == ArchitectureType::Api {
            return Err(Error::Config("API 架构模式下不能启用模板功能".to_string()));
        }

        // 验证静态文件目录
        if self.middleware.static_files.enabled {
            let static_dir = Path::new(&self.middleware.static_files.dir);
            if !static_dir.exists() {
                return Err(Error::Config(format!(
                    "静态文件目录不存在: {}",
                    self.middleware.static_files.dir
                )));
            }
        }

        // 验证模板目录
        if self.middleware.templates.enabled {
            let template_dir = Path::new(&self.middleware.templates.dir);
            if !template_dir.exists() {
                return Err(Error::Config(format!(
                    "模板目录不存在: {}",
                    self.middleware.templates.dir
                )));
            }
        }

        Ok(())
    }
}
