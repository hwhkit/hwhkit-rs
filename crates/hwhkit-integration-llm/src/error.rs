//! Error model for the LLM integration.

use thiserror::Error;

/// Errors surfaced by `LlmHandle` and the underlying backends.
#[derive(Debug, Error)]
pub enum LlmError {
    /// No backend matches the given model prefix.
    #[error("unknown model prefix: '{prefix}' — known: {known}")]
    UnknownProvider {
        /// Prefix that was looked up (e.g. `"anthropic"`).
        prefix: String,
        /// Comma-separated list of known prefixes.
        known: String,
    },
    /// The matched backend has no API key configured.
    #[error("backend '{backend}' is not configured (missing api_key)")]
    NotConfigured {
        /// Backend that was selected.
        backend: &'static str,
    },
    /// HTTP transport error (DNS, TLS, connect, IO).
    #[error("http transport error: {0}")]
    Transport(#[source] reqwest::Error),
    /// Backend returned a non-2xx status.
    #[error("backend returned {status}: {body}")]
    BadStatus {
        /// HTTP status code.
        status: u16,
        /// Truncated response body for diagnostics.
        body: String,
    },
    /// Backend returned a body we couldn't parse.
    #[error("backend returned malformed response: {0}")]
    Decode(String),
    /// Request exceeded the configured op_timeout.
    #[error("request exceeded op_timeout")]
    Timeout,
    /// Caller violated a precondition we want to surface clearly.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

impl LlmError {
    /// Convenience: build a `BadStatus` with body truncated to 1 KiB.
    pub fn bad_status(status: u16, body: impl Into<String>) -> Self {
        let mut body = body.into();
        if body.len() > 1024 {
            body.truncate(1024);
            body.push_str("... [truncated]");
        }
        Self::BadStatus { status, body }
    }
}

impl From<reqwest::Error> for LlmError {
    fn from(value: reqwest::Error) -> Self {
        if value.is_timeout() {
            Self::Timeout
        } else {
            Self::Transport(value)
        }
    }
}
