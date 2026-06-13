//! Backend implementations and routing.

pub mod anthropic;
pub mod openai_compat;

pub use anthropic::AnthropicBackend;
pub use openai_compat::OpenAiCompatBackend;

use async_trait::async_trait;

use crate::error::LlmError;
use crate::types::{
    ChatMessage, ChatRequestOptions, ChatResponse, EmbedRequestOptions, EmbeddingResponse,
};
use crate::StreamChunkReceiver;

/// What every backend has to provide. Wire shape is per-provider; this
/// trait owns the uniform translation.
#[async_trait]
pub trait Backend: Send + Sync + std::fmt::Debug {
    /// Static identifier — must match the `<prefix>/...` in the model
    /// string (e.g. `"anthropic"`).
    fn key(&self) -> &'static str;

    /// One-shot completion. `model` is the prefix-stripped model name.
    async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<ChatResponse, LlmError>;

    /// Streaming completion. `model` is the prefix-stripped model name.
    async fn chat_stream(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<StreamChunkReceiver, LlmError>;

    /// Default-disabled. Override on backends that support embeddings.
    async fn embed(
        &self,
        _model: &str,
        _texts: Vec<String>,
        _opts: EmbedRequestOptions,
    ) -> Result<EmbeddingResponse, LlmError> {
        Err(LlmError::InvalidRequest(format!(
            "backend '{}' does not support embeddings",
            self.key()
        )))
    }
}

/// Split a fully-qualified model id into `(prefix, name)`. Returns
/// `None` for unprefixed ids — caller falls back to the default
/// backend.
pub fn split_model(full: &str) -> Option<(&str, &str)> {
    full.split_once('/')
}
