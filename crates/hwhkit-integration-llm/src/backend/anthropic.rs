//! Anthropic Messages API backend (`POST /v1/messages`).
//!
//! Wire reference: <https://docs.anthropic.com/en/api/messages>.

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
    ChatMessage, ChatRequestOptions, ChatResponse, Role, StreamChunk, Usage,
};
use crate::StreamChunkReceiver;

/// Anthropic Messages backend.
#[derive(Debug, Clone)]
pub struct AnthropicBackend {
    client: Client,
    api_key: String,
    base_url: String,
    op_timeout: Duration,
}

impl AnthropicBackend {
    /// Build a new backend.
    pub fn new(api_key: String, base_url: Option<String>, op_timeout: Duration) -> Self {
        let base = base_url
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://api.anthropic.com".to_string());
        Self {
            client: Client::builder()
                .timeout(op_timeout)
                .build()
                .expect("reqwest client"),
            api_key,
            base_url: base,
            op_timeout,
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", self.api_key.parse().unwrap());
        h.insert("anthropic-version", "2023-06-01".parse().unwrap());
        h.insert("content-type", "application/json".parse().unwrap());
        h
    }
}

/// Anthropic splits the system prompt out of `messages`. Walk the
/// caller's messages and produce the wire shape.
fn split_system(messages: Vec<ChatMessage>) -> (Option<String>, Vec<WireMessage>) {
    let mut system: Option<String> = None;
    let mut out: Vec<WireMessage> = Vec::with_capacity(messages.len());
    for m in messages {
        match m.role {
            Role::System => {
                let next = m.content;
                system = Some(match system {
                    Some(prev) => format!("{prev}\n\n{next}"),
                    None => next,
                });
            }
            Role::User | Role::Tool => out.push(WireMessage {
                role: "user".to_string(),
                content: m.content,
            }),
            Role::Assistant => out.push(WireMessage {
                role: "assistant".to_string(),
                content: m.content,
            }),
        }
    }
    (system, out)
}

#[derive(Serialize)]
struct WireMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

fn build_request(model: &str, messages: Vec<ChatMessage>, opts: &ChatRequestOptions, stream: bool) -> WireRequest {
    let (system, msgs) = split_system(messages);
    WireRequest {
        model: model.to_string(),
        messages: msgs,
        system,
        temperature: opts.temperature,
        // Anthropic requires `max_tokens`. Cap at 4096 if caller didn't
        // specify (a soft default that keeps a single response under
        // ~$0.06 on Sonnet).
        max_tokens: opts.max_tokens.unwrap_or(4096),
        stop_sequences: opts.stop.clone(),
        stream,
    }
}

#[derive(Deserialize)]
struct WireResponse {
    model: String,
    content: Vec<WireContentBlock>,
    stop_reason: Option<String>,
    usage: WireUsage,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireContentBlock {
    Text { text: String },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct WireUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[async_trait]
impl Backend for AnthropicBackend {
    fn key(&self) -> &'static str {
        "anthropic"
    }

    async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        opts: ChatRequestOptions,
    ) -> Result<ChatResponse, LlmError> {
        let body = build_request(model, messages, &opts, false);
        let url = format!("{}/v1/messages", self.base_url);
        let raw: Value = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await?
            .error_for_status_or_body()
            .await?
            .json()
            .await
            .map_err(|e| LlmError::Decode(e.to_string()))?;

        let parsed: WireResponse = serde_json::from_value(raw.clone())
            .map_err(|e| LlmError::Decode(e.to_string()))?;

        let content = parsed
            .content
            .into_iter()
            .filter_map(|b| match b {
                WireContentBlock::Text { text } => Some(text),
                WireContentBlock::Other => None,
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(ChatResponse {
            content,
            model: parsed.model,
            finish_reason: parsed.stop_reason.unwrap_or_else(|| "stop".into()),
            usage: Usage {
                input_tokens: parsed.usage.input_tokens,
                output_tokens: parsed.usage.output_tokens,
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
        let url = format!("{}/v1/messages", self.base_url);

        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await?
            .error_for_status_or_body()
            .await?;

        let (tx, rx) = mpsc::channel(64);
        let op_timeout = self.op_timeout;
        tokio::spawn(async move {
            if let Err(e) = pump_sse(resp, &tx, op_timeout).await {
                let _ = tx.send(Err(e)).await;
            }
        });
        Ok(rx)
    }
}

async fn pump_sse(
    resp: reqwest::Response,
    tx: &mpsc::Sender<Result<StreamChunk, LlmError>>,
    _op_timeout: Duration,
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
                let ty = evt.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match ty {
                    "content_block_delta" => {
                        if let Some(text) = evt
                            .get("delta")
                            .and_then(|d| d.get("text"))
                            .and_then(|t| t.as_str())
                        {
                            if tx.send(Ok(StreamChunk::TextDelta { text: text.to_string() }))
                                .await
                                .is_err()
                            {
                                return Ok(()); // consumer dropped
                            }
                        }
                    }
                    "message_delta" => {
                        if let Some(reason) = evt
                            .get("delta")
                            .and_then(|d| d.get("stop_reason"))
                            .and_then(|s| s.as_str())
                        {
                            let usage = evt
                                .get("usage")
                                .and_then(|u| serde_json::from_value::<WireUsage>(u.clone()).ok())
                                .map(|u| Usage {
                                    input_tokens: u.input_tokens,
                                    output_tokens: u.output_tokens,
                                });
                            let _ = tx
                                .send(Ok(StreamChunk::Done {
                                    finish_reason: reason.to_string(),
                                    usage,
                                }))
                                .await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// Helper extension: convert a non-2xx response into `LlmError` with
/// truncated body, otherwise return the response unchanged.
trait ErrorForStatusOrBody {
    async fn error_for_status_or_body(self) -> Result<reqwest::Response, LlmError>;
}

impl ErrorForStatusOrBody for reqwest::Response {
    async fn error_for_status_or_body(self) -> Result<reqwest::Response, LlmError> {
        let status = self.status();
        if status.is_success() {
            Ok(self)
        } else {
            let body = self.text().await.unwrap_or_default();
            warn!(%status, body = %body, "anthropic backend returned non-2xx");
            Err(LlmError::bad_status(status.as_u16(), body))
        }
    }
}
