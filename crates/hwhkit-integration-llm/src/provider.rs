//! `IntegrationProvider` implementation that exposes `LlmHandle` to the
//! bootstrap pipeline.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationProvider, Result as CoreResult,
};
use tracing::info;

use crate::backend::{AnthropicBackend, Backend, OpenAiCompatBackend};
use crate::handle::LlmHandle;

/// Bootstrap-time provider. Reads `[integrations.llm]`, builds one
/// `Backend` per configured provider, and exposes the merged
/// `LlmHandle` via `AppContext`.
#[derive(Debug, Default)]
pub struct LlmProvider;

impl LlmProvider {
    /// Construct a new provider. Stateless — config is read at `init`.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl IntegrationProvider for LlmProvider {
    fn key(&self) -> &'static str {
        "llm"
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.llm.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.llm.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let llm_cfg = &cfg.integrations.llm;
        let op_timeout: Duration = llm_cfg.resilience.op_timeout();

        let mut backends: Vec<Arc<dyn Backend>> = Vec::new();

        if llm_cfg.providers.anthropic.is_configured() {
            backends.push(Arc::new(AnthropicBackend::new(
                llm_cfg.providers.anthropic.api_key.clone(),
                if llm_cfg.providers.anthropic.base_url.is_empty() {
                    None
                } else {
                    Some(llm_cfg.providers.anthropic.base_url.clone())
                },
                op_timeout,
            )));
        }
        for (key, cred) in [
            ("openai", &llm_cfg.providers.openai),
            ("deepseek", &llm_cfg.providers.deepseek),
            ("moonshot", &llm_cfg.providers.moonshot),
            ("ollama", &llm_cfg.providers.ollama),
        ] {
            if cred.is_configured() {
                backends.push(Arc::new(OpenAiCompatBackend::new(
                    key,
                    cred.api_key.clone(),
                    if cred.base_url.is_empty() {
                        None
                    } else {
                        Some(cred.base_url.clone())
                    },
                    op_timeout,
                )));
            }
        }

        if backends.is_empty() {
            return Err(CoreError::integration_msg(
                "llm",
                hwhkit_core::IntegrationFailureKind::Misconfigured,
                "no backends configured under [integrations.llm.providers.*]",
            ));
        }

        let handle = LlmHandle::new(llm_cfg, backends);
        info!(backends = ?handle.backend_keys(), "llm integration ready");
        ctx.insert(handle);
        Ok(())
    }

    fn health_check(&self, _ctx: &AppContext, _cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        // LLM backends charge per ping — skip readiness probing.
        // Adopters who want a smoke test can issue a low-token chat()
        // from a /diag endpoint instead.
        None
    }

    async fn shutdown(&self, _ctx: &AppContext) -> CoreResult<()> {
        // Nothing to flush — reqwest clients are managed by the
        // handle's Arc and dropped with it.
        Ok(())
    }
}

