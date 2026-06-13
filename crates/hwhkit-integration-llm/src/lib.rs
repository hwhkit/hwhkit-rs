//! HwhKit LLM integration.
//!
//! Wires a uniform `LlmHandle` (chat + embeddings) into the bootstrap
//! `AppContext` so handlers can pull it out via
//! `ctx.get::<LlmHandle>()`. The handle dispatches to the right
//! backend at call time based on the **model prefix**:
//!
//! | Prefix | Backend |
//! |--------|---------|
//! | `anthropic/...` | Anthropic Messages API |
//! | `openai/...` | OpenAI Chat Completions |
//! | `deepseek/...` | OpenAI-compatible (DeepSeek host) |
//! | `moonshot/...` | OpenAI-compatible (Moonshot host) |
//! | `ollama/...` | OpenAI-compatible (Ollama host) |
//! | `<no prefix>` | The configured default backend |
//!
//! ## Why not pull in an SDK
//!
//! Anthropic and OpenAI ship hand-written clients that lag the wire
//! API by weeks and balloon the dep graph. This crate speaks the
//! HTTPS shape directly via `reqwest`, which keeps it 4 deps deep
//! and lets us hold our own resilience guarantees.
//!
//! ## Quick start
//!
//! ```toml
//! [integrations.llm]
//! enabled = true
//! default_chat_model = "anthropic/claude-3-5-sonnet-20241022"
//!
//! [integrations.llm.providers.anthropic]
//! api_key = "${ANTHROPIC_API_KEY}"
//!
//! [integrations.llm.providers.openai]
//! api_key = "${OPENAI_API_KEY}"
//! ```
//!
//! ```ignore
//! use hwhkit_integration_llm::{ChatMessage, LlmHandle, Role};
//!
//! async fn answer(ctx: hwhkit_core::AppContext, q: &str) -> String {
//!     let llm = ctx.get::<LlmHandle>().expect("llm not wired");
//!     let resp = llm
//!         .chat(vec![ChatMessage::user(q)], Default::default())
//!         .await
//!         .unwrap();
//!     resp.content
//! }
//! ```

#![warn(missing_docs)]

pub mod backend;
pub mod config;
mod error;
mod handle;
mod provider;
mod types;

pub use config::{LlmConfig, ProviderCredentials, ProvidersConfig};
pub use error::LlmError;
pub use handle::LlmHandle;
pub use provider::LlmProvider;
pub use types::{
    ChatMessage, ChatRequestOptions, ChatResponse, EmbedRequestOptions, EmbeddingResponse, Role,
    StreamChunk, Usage,
};

use async_trait::async_trait;

/// Chat client contract. Implemented by `LlmHandle` and by individual
/// backends.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// One-shot chat completion. Returns the full `ChatResponse`.
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<ChatResponse, LlmError>;

    /// Streaming chat completion. Yields content deltas as they arrive.
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<StreamChunkReceiver, LlmError>;
}

/// Embedding client contract.
#[async_trait]
pub trait EmbeddingClient: Send + Sync {
    /// Encode `texts` into vectors. Returns one vector per input.
    async fn embed(
        &self,
        texts: Vec<String>,
        opts: EmbedRequestOptions,
    ) -> Result<EmbeddingResponse, LlmError>;
}

/// Stream receiver yielded by `chat_stream`. Wraps a `tokio::mpsc::Receiver`
/// so callers can `.recv().await` chunks one at a time.
pub type StreamChunkReceiver = tokio::sync::mpsc::Receiver<Result<StreamChunk, LlmError>>;
