//! OpenTelemetry instrumentation helpers for the `redis` crate.
//!
//! Wrapping [`redis::aio::ConnectionLike`] directly is fragile because
//! `redis::cmd(...).query_async(&mut conn)` requires `&mut conn` of the
//! concrete type. We instead expose a small helper that opens a
//! `tracing::Span` per command. The caller drives the redis call inside
//! the span (typically via `Span::in_scope`).
//!
//! ```ignore
//! use hwhkit_observability::redis_instrument::redis_span;
//! use tracing::Instrument;
//!
//! let span = redis_span("GET", Some("user:42"));
//! let val: Option<String> = redis::cmd("GET")
//!     .arg("user:42")
//!     .query_async(&mut conn)
//!     .instrument(span)
//!     .await?;
//! ```

use tracing::Span;

/// Open a span describing a single redis command. `cmd_name` is the redis
/// verb (uppercase by convention). `key` is an optional key hint — only
/// the *prefix* (up to the first `:`) is recorded so high-cardinality
/// keys don't blow up the span attribute set.
pub fn redis_span(cmd_name: &str, key: Option<&str>) -> Span {
    let key_prefix = key.map(prefix_of).unwrap_or_default();
    tracing::info_span!(
        "redis.cmd",
        otel.kind = "client",
        db.system = "redis",
        db.operation = cmd_name,
        db.redis.key_prefix = %key_prefix,
    )
}

fn prefix_of(key: &str) -> &str {
    match key.find(':') {
        Some(i) => &key[..i],
        None => key,
    }
}
