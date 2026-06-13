# hwhkit-integration-llm

Multi-provider LLM client (chat + embeddings) wired into the
[HwhKit](../../) bootstrap pipeline.

## What it gives you

- A uniform `LlmHandle` exposed via `AppContext`, no SDK lock-in.
- Backends dispatched by **model prefix**:
  `anthropic/claude-3-5-sonnet-20241022`,
  `openai/gpt-4o-mini`, `deepseek/deepseek-chat`,
  `moonshot/moonshot-v1-128k`, `ollama/llama3.1:8b`.
- Streaming chat via `tokio::mpsc::Receiver<Result<StreamChunk, _>>`.
- Embeddings on the OpenAI-compatible backends (single-batch).
- Resilience: per-call `op_timeout`, body-truncated error context,
  no health-check ping (LLM API calls cost money).

## What it does NOT do

- **No SDK.** We speak `reqwest` against the wire shape directly.
  Anthropic and OpenAI ship their own clients that lag the API and
  balloon the dep graph; we choose the four-deps-deep path.
- **No prompt-caching / function-calling / vision** yet — the wire
  surface is "text chat + embeddings". The roadmap (in CHANGELOG)
  layers those on top of the same `LlmHandle`.
- **No automatic key sourcing from environment.** Keep secrets in the
  config layer; the framework reads them from
  `cfg.integrations.llm.providers.*.api_key`.

## Config

```toml
[integrations.llm]
enabled = true
default_chat_model      = "anthropic/claude-3-5-sonnet-20241022"
default_embedding_model = "openai/text-embedding-3-small"
default_temperature     = 0.7
default_max_tokens      = 4096

[integrations.llm.resilience]
op_timeout_ms = 30000

[integrations.llm.providers.anthropic]
api_key = "${ANTHROPIC_API_KEY}"

[integrations.llm.providers.openai]
api_key = "${OPENAI_API_KEY}"

[integrations.llm.providers.deepseek]
api_key  = "${DEEPSEEK_API_KEY}"
# base_url defaults to https://api.deepseek.com

[integrations.llm.providers.ollama]
base_url = "http://localhost:11434"
# no api_key needed for local Ollama
```

## Usage

```rust
use hwhkit_integration_llm::{ChatMessage, LlmClient, LlmHandle, ChatRequestOptions};

async fn handle(ctx: hwhkit_core::AppContext) -> Result<String, Box<dyn std::error::Error>> {
    let llm = ctx
        .get::<LlmHandle>()
        .ok_or("llm integration not wired")?;
    let resp = llm
        .chat(
            vec![
                ChatMessage::system("You are a concise assistant."),
                ChatMessage::user("Hello?"),
            ],
            ChatRequestOptions::default(),
        )
        .await?;
    Ok(resp.content)
}
```

Streaming:

```rust
let mut rx = llm.chat_stream(messages, opts).await?;
while let Some(chunk) = rx.recv().await {
    match chunk? {
        StreamChunk::TextDelta { text } => print!("{text}"),
        StreamChunk::Done { finish_reason, .. } => println!("\n[{finish_reason}]"),
    }
}
```

## Tests

Single-process tests run on every `cargo test` via
[`wiremock`](https://docs.rs/wiremock) — no real API keys required:

```sh
cargo test -p hwhkit-integration-llm
```

For live tests against real APIs (Anthropic / OpenAI / …), wire up a
binary in your own project; this crate doesn't ship `#[ignore]` live
tests because they'd burn credits in CI.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) — same as the rest of the workspace.
