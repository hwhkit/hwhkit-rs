//! OpenAI-compatible backend (`POST /v1/chat/completions`).
//!
//! Used directly for OpenAI; also serves DeepSeek, Moonshot, Ollama,
//! LiteLLM proxies, and any other host that speaks the OpenAI wire
//! shape. Each is instantiated with its own `key()`, `api_key`, and
//! `base_url`.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{header::HeaderMap, Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::warn;

use super::Backend;
use crate::error::LlmError;
use crate::types::{
    ChatMessage, ChatRequestOptions, ChatResponse, EmbedRequestOptions, EmbeddingResponse, Role,
    StreamChunk, Usage,
};
use crate::StreamChunkReceiver;

/// One backend instance bound to a single host.
#[derive(Debug, Clone)]
pub struct OpenAiCompatBackend {
    key: &'static str,
    client: Client,
    api_key: String,
    base_url: String,
}

impl OpenAiCompatBackend {
    /// Build a new backend. `key` is the routing prefix (e.g.
    /// `"openai"`, `"deepseek"`); `base_url` defaults per known host.
    pub fn new(
        key: &'static str,
        api_key: String,
        base_url: Option<String>,
        op_timeout: Duration,
    ) -> Self {
        let base = base_url
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_base(key).to_string());
        Self {
            key,
            client: Client::builder()
                .timeout(op_timeout)
                .build()
                .expect("reqwest client"),
            api_key,
            base_url: base,
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        if !self.api_key.is_empty() {
            h.insert(
                "authorization",
                format!("Bearer {}", self.api_key).parse().unwrap(),
            );
        }
        h.insert("content-type", "application/json".parse().unwrap());
        h
    }
}

fn default_base(key: &str) -> &'static str {
    match key {
        "openai" => "https://api.openai.com",
        "deepseek" => "https://api.deepseek.com",
        "moonshot" => "https://api.moonshot.cn",
        "ollama" => "http://localhost:11434",
        _ => "https://api.openai.com",
    }
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

#[derive(Serialize)]
struct WireMessage {
    role: &'static str,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

fn build_request(
    model: &str,
    messages: Vec<ChatMessage>,
    opts: &ChatRequestOptions,
    stream: bool,
) -> WireRequest {
    WireRequest {
        model: model.to_string(),
        messages: messages
            .into_iter()
            .map(|m| WireMessage {
                role: role_str(m.role),
                content: m.content,
                name: m.name,
            })
            .collect(),
        temperature: opts.temperature,
        max_tokens: opts.max_tokens,
        stop: opts.stop.clone(),
        stream,
    }
}

#[derive(Deserialize)]
struct WireResponse {
    model: String,
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: WireUsage,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

#[async_trait]
impl Backend for OpenAiCompatBackend {
    fn key(&self) -> &'static str {
        self.key
    }

    async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<ChatResponse, LlmError> {
        let body = build_request(model, messages, &opts, false);
        let url = format!("{}/v1/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await?
            .ok_or_body(self.key)
            .await?;

        let raw: Value = resp.json().await.map_err(|e| LlmError::Decode(e.to_string()))?;
        let parsed: WireResponse = serde_json::from_value(raw.clone())
            .map_err(|e| LlmError::Decode(e.to_string()))?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::Decode("response has no choices".into()))?;
        let content = choice.message.content.unwrap_or_default();
        let finish_reason = choice.finish_reason.unwrap_or_else(|| "stop".into());

        Ok(ChatResponse {
            content,
            model: parsed.model,
            finish_reason,
            usage: Usage {
                input_tokens: parsed.usage.prompt_tokens,
                output_tokens: parsed.usage.completion_tokens,
            },
            raw,
        })
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<StreamChunkReceiver, LlmError> {
        let body = build_request(model, messages, &opts, true);
        let url = format!("{}/v1/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await?
            .ok_or_body(self.key)
            .await?;

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            if let Err(e) = pump_sse(resp, &tx).await {
                let _ = tx.send(Err(e)).await;
            }
        });
        Ok(rx)
    }

    async fn embed(
        &self,
        model: &str,
        texts: Vec<String>,
        _opts: EmbedRequestOptions,
    ) -> Result<EmbeddingResponse, LlmError> {
        #[derive(Serialize)]
        struct Req<'a> {
            model: &'a str,
            input: &'a [String],
        }
        #[derive(Deserialize)]
        struct Resp {
            model: String,
            data: Vec<Item>,
            #[serde(default)]
            usage: WireUsage,
        }
        #[derive(Deserialize)]
        struct Item {
            embedding: Vec<f32>,
        }
        let url = format!("{}/v1/embeddings", self.base_url);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&Req { model, input: &texts })
            .send()
            .await?
            .ok_or_body(self.key)
            .await?
            .json::<Resp>()
            .await
            .map_err(|e| LlmError::Decode(e.to_string()))?;
        Ok(EmbeddingResponse {
            vectors: resp.data.into_iter().map(|d| d.embedding).collect(),
            model: resp.model,
            usage: Usage {
                input_tokens: resp.usage.prompt_tokens,
                output_tokens: resp.usage.completion_tokens,
            },
        })
    }
}

async fn pump_sse(
    resp: reqwest::Response,
    tx: &mpsc::Sender<Result<StreamChunk, LlmError>>,
) -> Result<(), LlmError> {
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(LlmError::from)?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = buf.find("\n\n") {
            let event: String = buf.drain(..idx + 2).collect();
            for line in event.lines() {
                let payload = match line.strip_prefix("data: ") {
                    Some(p) => p,
                    None => continue,
                };
                if payload == "[DONE]" {
                    return Ok(());
                }
                let evt: Value = match serde_json::from_str(payload) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(choice) = evt.get("choices").and_then(|c| c.get(0)) {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(text) = delta.get("content").and_then(|t| t.as_str()) {
                            if tx
                                .send(Ok(StreamChunk::TextDelta {
                                    text: text.to_string(),
                                }))
                                .await
                                .is_err()
                            {
                                return Ok(());
                            }
                        }
                    }
                    if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                        let _ = tx
                            .send(Ok(StreamChunk::Done {
                                finish_reason: reason.to_string(),
                                usage: None,
                            }))
                            .await;
                    }
                }
            }
        }
    }
    Ok(())
}

trait OkOrBody {
    async fn ok_or_body(self, key: &'static str) -> Result<reqwest::Response, LlmError>;
}

impl OkOrBody for reqwest::Response {
    async fn ok_or_body(self, key: &'static str) -> Result<reqwest::Response, LlmError> {
        let status = self.status();
        if status.is_success() {
            Ok(self)
        } else {
            let body = self.text().await.unwrap_or_default();
            warn!(backend = %key, %status, body = %body, "openai-compat backend returned non-2xx");
            Err(LlmError::bad_status(status.as_u16(), body))
        }
    }
}
