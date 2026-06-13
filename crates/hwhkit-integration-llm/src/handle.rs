//! Cheap-clone, dispatching handle exposed via `AppContext`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::backend::{split_model, Backend};
use crate::config::LlmConfig;
use crate::error::LlmError;
use crate::types::{
    ChatMessage, ChatRequestOptions, ChatResponse, EmbedRequestOptions, EmbeddingResponse,
};
use crate::{EmbeddingClient, LlmClient, StreamChunkReceiver};

/// User-facing handle. `Arc`-backed, cheap to clone, safe to share
/// across tasks.
#[derive(Clone)]
pub struct LlmHandle {
    inner: Arc<Inner>,
}

struct Inner {
    backends: HashMap<&'static str, Arc<dyn Backend>>,
    default_chat_model: String,
    default_embedding_model: String,
    default_temperature: f32,
    default_max_tokens: Option<u32>,
    op_timeout: Duration,
}

impl LlmHandle {
    /// Build a new handle from a config + a pre-built backend set.
    /// Typically called from `LlmProvider::init`.
    pub(crate) fn new(cfg: &LlmConfig, backends: Vec<Arc<dyn Backend>>) -> Self {
        let map = backends.into_iter().map(|b| (b.key(), b)).collect();
        Self {
            inner: Arc::new(Inner {
                backends: map,
                default_chat_model: cfg.default_chat_model.clone(),
                default_embedding_model: cfg.default_embedding_model.clone(),
                default_temperature: cfg.default_temperature,
                default_max_tokens: cfg.default_max_tokens,
                op_timeout: cfg.resilience.op_timeout(),
            }),
        }
    }

    /// `op_timeout_ms` value the handle was built with.
    pub fn op_timeout(&self) -> Duration {
        self.inner.op_timeout
    }

    /// Default chat model, used when `ChatRequestOptions::model` is None.
    pub fn default_chat_model(&self) -> &str {
        &self.inner.default_chat_model
    }

    /// Default embedding model, used when `EmbedRequestOptions::model`
    /// is None.
    pub fn default_embedding_model(&self) -> &str {
        &self.inner.default_embedding_model
    }

    /// Set of backend keys actually wired (e.g.
    /// `["anthropic", "openai"]`). For observability / startup logs.
    pub fn backend_keys(&self) -> Vec<&'static str> {
        let mut v: Vec<_> = self.inner.backends.keys().copied().collect();
        v.sort_unstable();
        v
    }

    fn resolve_chat<'a>(
        &'a self,
        opts: &'a ChatRequestOptions,
    ) -> Result<(Arc<dyn Backend>, String, ChatRequestOptions), LlmError> {
        let full = opts
            .model
            .clone()
            .unwrap_or_else(|| self.inner.default_chat_model.clone());
        if full.is_empty() {
            return Err(LlmError::InvalidRequest(
                "no model specified and no default_chat_model configured".into(),
            ));
        }
        let (prefix, name) = match split_model(&full) {
            Some(parts) => (parts.0.to_string(), parts.1.to_string()),
            None => {
                return Err(LlmError::InvalidRequest(format!(
                    "model '{full}' must include a provider prefix (e.g. 'openai/gpt-4o')"
                )));
            }
        };
        let backend = self
            .inner
            .backends
            .get(prefix.as_str())
            .cloned()
            .ok_or_else(|| LlmError::UnknownProvider {
                prefix: prefix.clone(),
                known: self.backend_keys().join(", "),
            })?;
        let mut resolved = opts.clone();
        if resolved.temperature.is_none() {
            resolved.temperature = Some(self.inner.default_temperature);
        }
        if resolved.max_tokens.is_none() {
            resolved.max_tokens = self.inner.default_max_tokens;
        }
        Ok((backend, name, resolved))
    }

    fn resolve_embed(
        &self,
        opts: &EmbedRequestOptions,
    ) -> Result<(Arc<dyn Backend>, String), LlmError> {
        let full = opts
            .model
            .clone()
            .unwrap_or_else(|| self.inner.default_embedding_model.clone());
        if full.is_empty() {
            return Err(LlmError::InvalidRequest(
                "no model specified and no default_embedding_model configured".into(),
            ));
        }
        let (prefix, name) = match split_model(&full) {
            Some(parts) => (parts.0.to_string(), parts.1.to_string()),
            None => {
                return Err(LlmError::InvalidRequest(format!(
                    "model '{full}' must include a provider prefix"
                )));
            }
        };
        let backend = self
            .inner
            .backends
            .get(prefix.as_str())
            .cloned()
            .ok_or_else(|| LlmError::UnknownProvider {
                prefix,
                known: self.backend_keys().join(", "),
            })?;
        Ok((backend, name))
    }
}

#[async_trait]
impl LlmClient for LlmHandle {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<ChatResponse, LlmError> {
        let (backend, model, opts) = self.resolve_chat(&opts)?;
        backend.chat(&model, messages, opts).await
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<StreamChunkReceiver, LlmError> {
        let (backend, model, opts) = self.resolve_chat(&opts)?;
        backend.chat_stream(&model, messages, opts).await
    }
}

#[async_trait]
impl EmbeddingClient for LlmHandle {
    async fn embed(
        &self,
        texts: Vec<String>,
        opts: EmbedRequestOptions,
    ) -> Result<EmbeddingResponse, LlmError> {
        if texts.is_empty() {
            return Err(LlmError::InvalidRequest("texts must be non-empty".into()));
        }
        let (backend, model) = self.resolve_embed(&opts)?;
        backend.embed(&model, texts, opts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LlmConfig;

    fn cfg_with_model(model: &str) -> LlmConfig {
        let mut c = LlmConfig::default();
        c.default_chat_model = model.into();
        c
    }

    #[test]
    fn resolve_chat_errors_when_no_prefix() {
        let h = LlmHandle::new(&cfg_with_model("claude"), Vec::new());
        match h.resolve_chat(&ChatRequestOptions::default()) {
            Err(LlmError::InvalidRequest(_)) => {}
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn resolve_chat_errors_when_prefix_unknown() {
        let h = LlmHandle::new(&cfg_with_model("anthropic/claude-3-5-sonnet"), Vec::new());
        match h.resolve_chat(&ChatRequestOptions::default()) {
            Err(LlmError::UnknownProvider { .. }) => {}
            other => panic!("expected UnknownProvider, got {other:?}"),
        }
    }

    #[test]
    fn resolve_chat_errors_when_default_is_empty() {
        let h = LlmHandle::new(&LlmConfig::default(), Vec::new());
        match h.resolve_chat(&ChatRequestOptions::default()) {
            Err(LlmError::InvalidRequest(msg)) => {
                assert!(msg.contains("default_chat_model"), "msg: {msg}")
            }
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }
}
