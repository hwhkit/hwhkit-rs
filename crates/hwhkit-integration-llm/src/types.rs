//! Public wire types shared across backends.

use serde::{Deserialize, Serialize};

/// Conversation role. Mirrors OpenAI's `role` field; backends translate
/// to their native vocabulary (Anthropic merges System into the
/// top-level `system` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System / developer instruction.
    System,
    /// User message.
    User,
    /// Assistant message.
    Assistant,
    /// Tool result message (OpenAI: `"tool"`).
    Tool,
}

/// One turn in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Speaker role.
    pub role: Role,
    /// UTF-8 text content. Multi-part content (images, tool calls) is
    /// not yet modelled at this layer; use the raw backend if you need
    /// it. Roadmap: see CHANGELOG.
    pub content: String,
    /// Optional speaker name (rare; mostly OpenAI tool messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    /// Build a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            name: None,
        }
    }
    /// Build a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            name: None,
        }
    }
    /// Build an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            name: None,
        }
    }
}

/// Optional knobs for a chat call. `Default` is a safe choice on every
/// backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChatRequestOptions {
    /// Model identifier, including provider prefix
    /// (`anthropic/claude-3-5-sonnet-20241022`). If `None`, the
    /// `default_chat_model` from config is used.
    pub model: Option<String>,
    /// Sampling temperature in `[0.0, 2.0]`.
    pub temperature: Option<f32>,
    /// Hard cap on output tokens.
    pub max_tokens: Option<u32>,
    /// Optional stop sequences.
    #[serde(default)]
    pub stop: Vec<String>,
}

impl Default for ChatRequestOptions {
    fn default() -> Self {
        Self {
            model: None,
            temperature: None,
            max_tokens: None,
            stop: Vec::new(),
        }
    }
}

/// Successful (non-streaming) chat response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Concatenated text content.
    pub content: String,
    /// Model that actually served the request (post-dispatch).
    pub model: String,
    /// Provider-reported finish reason (`stop`, `length`, …).
    pub finish_reason: String,
    /// Token usage, if reported.
    pub usage: Usage,
    /// Raw backend response JSON, for callers that need provider-
    /// specific fields. Always populated.
    pub raw: serde_json::Value,
}

/// Token usage. Zero values mean "not reported by the backend".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens in the prompt (input).
    pub input_tokens: u32,
    /// Tokens in the completion (output).
    pub output_tokens: u32,
}

impl Usage {
    /// `input_tokens + output_tokens`.
    pub fn total(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// One frame of a streaming chat response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamChunk {
    /// Text delta. Appended to the running assistant message.
    TextDelta {
        /// Content fragment to append.
        text: String,
    },
    /// Terminal chunk: backend reported a finish reason.
    Done {
        /// Provider-reported finish reason.
        finish_reason: String,
        /// Token usage if the backend included it in the final frame.
        usage: Option<Usage>,
    },
}

/// Options for an `embed()` call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EmbedRequestOptions {
    /// Model identifier (`openai/text-embedding-3-small` etc.). If
    /// `None`, the `default_embedding_model` from config is used.
    pub model: Option<String>,
}

/// Embedding response. `vectors[i]` corresponds to `inputs[i]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// One vector per input text.
    pub vectors: Vec<Vec<f32>>,
    /// Model the backend used (post-dispatch).
    pub model: String,
    /// Total token usage for the batch.
    pub usage: Usage,
}
