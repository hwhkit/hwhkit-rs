//! 模板渲染模块

#[cfg(feature = "templates")]
use crate::{
    config::TemplateConfig,
    error::{Error, Result},
};

#[cfg(feature = "templates")]
use axum::{
    extract::{Path as AxumPath, Query, State},
    response::{Html, IntoResponse},
};

#[cfg(feature = "templates")]
use serde::Serialize;

#[cfg(feature = "templates")]
use std::{collections::HashMap, path::Path, sync::Arc};

#[cfg(feature = "templates")]
use tera::{Context, Tera};

/// 模板引擎包装器
#[cfg(feature = "templates")]
#[derive(Debug, Clone)]
pub struct TemplateEngine {
    tera: Arc<Tera>,
}

#[cfg(feature = "templates")]
impl TemplateEngine {
    /// 创建新的模板引擎
    pub fn new(config: &TemplateConfig) -> Result<Self> {
        let template_dir = Path::new(&config.dir);

        if !template_dir.exists() {
            return Err(Error::Config(format!("模板目录不存在: {}", config.dir)));
        }

        let glob_pattern = format!("{}/**/*.{}", config.dir, config.extension);
        let tera = Tera::new(&glob_pattern).map_err(Error::Template)?;

        tracing::info!("✅ 模板引擎初始化成功");
        tracing::info!("📁 模板目录: {}", config.dir);
        tracing::info!("🔗 文件扩展名: .{}", config.extension);

        Ok(Self {
            tera: Arc::new(tera),
        })
    }

    /// 渲染模板
    pub fn render<T: Serialize>(&self, template_name: &str, context: &T) -> Result<String> {
        let mut tera_context = Context::new();

        // 将传入的上下文序列化为 serde_json::Value 然后添加到 Tera Context
        let value = serde_json::to_value(context).map_err(Error::Serialization)?;
        if let serde_json::Value::Object(map) = value {
            for (key, val) in map {
                tera_context.insert(&key, &val);
            }
        }

        self.tera
            .render(template_name, &tera_context)
            .map_err(Error::Template)
    }

    /// 渲染模板（使用 HashMap 上下文）
    pub fn render_with_context(
        &self,
        template_name: &str,
        context: HashMap<String, serde_json::Value>,
    ) -> Result<String> {
        let mut tera_context = Context::new();
        for (key, value) in context {
            tera_context.insert(&key, &value);
        }

        self.tera
            .render(template_name, &tera_context)
            .map_err(Error::Template)
    }

    /// 获取所有可用的模板名称
    pub fn get_template_names(&self) -> Vec<String> {
        self.tera
            .get_template_names()
            .map(|s| s.to_string())
            .collect()
    }
}

/// 模板响应辅助函数
#[cfg(feature = "templates")]
pub async fn render_template<T: Serialize>(
    State(template_engine): State<Arc<TemplateEngine>>,
    template_name: &str,
    context: &T,
) -> Result<impl IntoResponse> {
    let html = template_engine.render(template_name, context)?;
    Ok(Html(html))
}

/// 简单的模板渲染处理器
#[cfg(feature = "templates")]
pub async fn template_handler(
    State(template_engine): State<Arc<TemplateEngine>>,
    AxumPath(template_name): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse> {
    // 将查询参数转换为 JSON 值
    let context: HashMap<String, serde_json::Value> = params
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();

    let html = template_engine.render_with_context(&template_name, context)?;
    Ok(Html(html))
}

// 当没有启用 templates 功能时的占位类型
#[cfg(not(feature = "templates"))]
#[derive(Debug, Clone)]
pub struct TemplateEngine;

#[cfg(not(feature = "templates"))]
impl TemplateEngine {
    pub fn new(_config: &crate::config::TemplateConfig) -> crate::error::Result<Self> {
        Err(crate::error::Error::Config(
            "模板功能未启用，请启用 'templates' 特性".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "templates")]
    #[test]
    fn test_template_engine_creation_without_dir() {
        use crate::config::TemplateConfig;

        let config = TemplateConfig {
            enabled: true,
            dir: "/nonexistent/directory".to_string(),
            extension: "html".to_string(),
        };

        let result = TemplateEngine::new(&config);
        assert!(result.is_err());
    }

    #[cfg(not(feature = "templates"))]
    #[test]
    fn test_template_engine_disabled() {
        use crate::config::TemplateConfig;

        let config = TemplateConfig::default();
        let result = TemplateEngine::new(&config);
        assert!(result.is_err());
    }
}
