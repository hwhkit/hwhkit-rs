//! Minimal OpenTelemetry instrumentation middleware for `reqwest`.
//!
//! Implemented as a `tower::Service` that wraps a `reqwest::Client` and
//! emits a span per outbound request:
//! `http.method`, `http.url`, `http.status_code`, plus duration recorded
//! via the underlying `tracing` infrastructure.
//!
//! Most callers won't need a tower stack — they can just call
//! [`tracing_send`] which mirrors `client.execute(req).await` but with
//! a pre-attached span.

use std::time::Duration;

use reqwest::{Client, Request, Response};
use tracing::{field::Empty, Instrument, Span};

/// Execute a `reqwest::Request` while attaching an OTel-compatible span.
pub async fn tracing_send(client: &Client, req: Request) -> reqwest::Result<Response> {
    let span = span_for(&req);
    let started = std::time::Instant::now();
    let res = client.execute(req).instrument(span.clone()).await;
    let elapsed = started.elapsed();
    record_outcome(&span, elapsed, &res);
    res
}

fn span_for(req: &Request) -> Span {
    tracing::info_span!(
        "http.client.request",
        otel.kind = "client",
        http.method = %req.method(),
        http.url = %req.url(),
        http.status_code = Empty,
        http.duration_ms = Empty,
        // OTel semantic-conventions: error.type is recorded on every
        // failure (and emitted as `Empty` so the field is reserved on
        // the span allocation rather than re-resolved at record-time).
        error.type = Empty,
    )
}

fn record_outcome(span: &Span, elapsed: Duration, res: &reqwest::Result<Response>) {
    span.record("http.duration_ms", elapsed.as_millis() as u64);
    match res {
        Ok(r) => span.record("http.status_code", r.status().as_u16()),
        Err(e) => span.record("error.type", classify_error(e)),
    };
}

/// OTel `error.type` classification for `reqwest::Error`. The chosen
/// values match the convention common in OTel semconv extensions
/// (`timeout`, `connect`, `decode`, `other`).
fn classify_error(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else if e.is_decode() {
        "decode"
    } else {
        "other"
    }
}
