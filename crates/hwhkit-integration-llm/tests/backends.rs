//! End-to-end backend tests against a `wiremock` server. No real LLM
//! credentials needed; runs in `cargo test` by default.

use std::time::Duration;

use hwhkit_integration_llm::backend::{AnthropicBackend, Backend, OpenAiCompatBackend};
use hwhkit_integration_llm::{ChatMessage, ChatRequestOptions, StreamChunk};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn anthropic_chat_parses_text_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "claude-3-5-sonnet-20241022",
            "content": [
                { "type": "text", "text": "Hello " },
                { "type": "text", "text": "world." }
            ],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 12, "output_tokens": 4 }
        })))
        .mount(&server)
        .await;

    let backend = AnthropicBackend::new(
        "test-key".into(),
        Some(server.uri()),
        Duration::from_secs(5),
    );
    let resp = backend
        .chat(
            "claude-3-5-sonnet-20241022",
            vec![ChatMessage::user("hi")],
            ChatRequestOptions::default(),
        )
        .await
        .unwrap();

    assert_eq!(resp.content, "Hello world.");
    assert_eq!(resp.finish_reason, "end_turn");
    assert_eq!(resp.usage.input_tokens, 12);
    assert_eq!(resp.usage.output_tokens, 4);
}

#[tokio::test]
async fn anthropic_chat_propagates_bad_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error": {"message": "bad key"}})),
        )
        .mount(&server)
        .await;
    let backend = AnthropicBackend::new(
        "test-key".into(),
        Some(server.uri()),
        Duration::from_secs(5),
    );
    let err = backend
        .chat(
            "claude-3-5-sonnet-20241022",
            vec![ChatMessage::user("hi")],
            ChatRequestOptions::default(),
        )
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("401"), "got {msg}");
}

#[tokio::test]
async fn openai_chat_parses_choice_content() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "gpt-4o-mini",
            "choices": [{
                "message": { "role": "assistant", "content": "hi from gpt" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 9, "completion_tokens": 3, "total_tokens": 12 }
        })))
        .mount(&server)
        .await;
    let backend = OpenAiCompatBackend::new(
        "openai",
        "k".into(),
        Some(server.uri()),
        Duration::from_secs(5),
    );
    let resp = backend
        .chat(
            "gpt-4o-mini",
            vec![ChatMessage::user("hi")],
            ChatRequestOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(resp.content, "hi from gpt");
    assert_eq!(resp.finish_reason, "stop");
    assert_eq!(resp.usage.input_tokens, 9);
    assert_eq!(resp.usage.output_tokens, 3);
}

#[tokio::test]
async fn openai_chat_stream_yields_deltas() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    let backend = OpenAiCompatBackend::new(
        "openai",
        "k".into(),
        Some(server.uri()),
        Duration::from_secs(5),
    );
    let mut rx = backend
        .chat_stream(
            "gpt-4o-mini",
            vec![ChatMessage::user("hi")],
            ChatRequestOptions::default(),
        )
        .await
        .unwrap();
    let mut text = String::new();
    let mut finish = None;
    while let Some(chunk) = rx.recv().await {
        match chunk.unwrap() {
            StreamChunk::TextDelta { text: t } => text.push_str(&t),
            StreamChunk::Done { finish_reason, .. } => finish = Some(finish_reason),
        }
    }
    assert_eq!(text, "hello world");
    assert_eq!(finish.as_deref(), Some("stop"));
}

#[tokio::test]
async fn openai_embed_returns_vectors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "text-embedding-3-small",
            "data": [
                {"embedding": [0.1, 0.2, 0.3]},
                {"embedding": [0.4, 0.5, 0.6]}
            ],
            "usage": {"prompt_tokens": 8, "completion_tokens": 0, "total_tokens": 8}
        })))
        .mount(&server)
        .await;
    let backend = OpenAiCompatBackend::new(
        "openai",
        "k".into(),
        Some(server.uri()),
        Duration::from_secs(5),
    );
    let resp = backend
        .embed(
            "text-embedding-3-small",
            vec!["a".into(), "b".into()],
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(resp.vectors.len(), 2);
    assert_eq!(resp.vectors[0], vec![0.1, 0.2, 0.3]);
}
