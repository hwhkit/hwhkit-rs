//! Idempotency-key middleware.
//!
//! Honors the `Idempotency-Key` request header for mutating verbs
//! (POST/PUT/PATCH/DELETE) and replays the cached response on a hit.
//! Cached responses are stored in Redis and expire after a configurable
//! window (default 24h).
//!
//! ## Body-fingerprint guard
//!
//! The cache key is `(Idempotency-Key, hash(method || path || body))`.
//! When the **same** `Idempotency-Key` arrives with a **different**
//! method/path/body, the layer treats it as a misuse of the
//! idempotency contract and returns **HTTP 409 Conflict** with an
//! RFC 7807 `application/problem+json` body — the cached response is
//! NOT replayed. This protects against accidental key reuse across
//! distinct operations and makes silent corruption impossible.
//!
//! ## Tenant prefixing
//!
//! When the request carries a `hwhkit_core::TenantId` extension (e.g.
//! inserted by `TenantExtractorLayer`), the cached entries are
//! namespaced per tenant so different tenants reusing the same client
//! id never collide.
//!
//! Example:
//!
//! ```ignore
//! use std::time::Duration;
//! use hwhkit::production::idempotency::IdempotencyLayer;
//!
//! let layer = IdempotencyLayer::new(redis_handle).with_ttl(Duration::from_secs(86400));
//! let app = router.layer(layer);
//! ```
//!
//! Behind the `idempotency` feature.

use std::convert::Infallible;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Method, Request, Response, StatusCode};
use base64::Engine;
use futures::future::BoxFuture;
use http_body_util::BodyExt;
#[cfg(feature = "multi-tenant")]
use hwhkit_core::TenantId;
use hwhkit_integration_redis::RedisHandle;
use redis_client::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tower::{Layer, Service};

const HEADER_IDEMPOTENCY_KEY: &str = "idempotency-key";
const NAMESPACE: &str = "hwhkit:idem";

#[derive(Clone)]
pub struct IdempotencyLayer {
    handle: RedisHandle,
    ttl: Duration,
    namespace: String,
}

impl IdempotencyLayer {
    pub fn new(handle: RedisHandle) -> Self {
        Self {
            handle,
            ttl: Duration::from_secs(24 * 60 * 60),
            namespace: NAMESPACE.to_string(),
        }
    }

    #[must_use]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    #[must_use]
    pub fn with_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = ns.into();
        self
    }
}

impl<S> Layer<S> for IdempotencyLayer {
    type Service = Idempotency<S>;
    fn layer(&self, inner: S) -> Self::Service {
        Idempotency {
            inner,
            handle: self.handle.clone(),
            ttl: self.ttl,
            namespace: self.namespace.clone(),
        }
    }
}

#[derive(Clone)]
pub struct Idempotency<S> {
    inner: S,
    handle: RedisHandle,
    ttl: Duration,
    namespace: String,
}

#[derive(Serialize, Deserialize)]
struct CachedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    /// Base64 (standard, padded) encoded response body. Renamed from the
    /// historical `body_b64` field which was actually hex-encoded.
    body_b64: String,
    /// SHA-256 fingerprint of the original `(method, path, body)` tuple,
    /// hex-encoded. Replays are required to match this fingerprint
    /// exactly — mismatches surface as HTTP 409.
    fingerprint: String,
}

impl<S> Service<Request<Body>> for Idempotency<S>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // Only intercept mutating verbs.
        let mutating = matches!(
            req.method(),
            &Method::POST | &Method::PUT | &Method::PATCH | &Method::DELETE
        );
        let key = req
            .headers()
            .get(HEADER_IDEMPOTENCY_KEY)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        if !mutating || key.is_none() {
            return Box::pin(async move { inner.call(req).await });
        }

        let key = key.expect("verified Some above");

        // Optional tenant id — when present, namespace cache entries per
        // tenant so two tenants reusing the same client id never collide.
        #[cfg(feature = "multi-tenant")]
        let tenant_prefix: String = req
            .extensions()
            .get::<TenantId>()
            .map(|t| format!("t:{}:", t.as_str()))
            .unwrap_or_default();
        #[cfg(not(feature = "multi-tenant"))]
        let tenant_prefix: String = String::new();

        let mut conn = self.handle.manager();
        let ttl_secs = self.ttl.as_secs().max(1);
        let redis_key = format!("{}:{}{}", self.namespace, tenant_prefix, key);

        // Save method + path before consuming the body — we hash both
        // into the fingerprint so distinct operations sharing the same
        // Idempotency-Key get a 409.
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let (parts, body) = req.into_parts();

        Box::pin(async move {
            // Buffer the request body so we can both fingerprint it and
            // pass it on. A failure to buffer is a real I/O problem —
            // surface it as 502 (N19) rather than silently truncating.
            let collected = match body.collect().await {
                Ok(c) => c.to_bytes(),
                Err(err) => {
                    tracing::warn!(error = %err, "idempotency: failed to buffer request body");
                    return Ok(bad_gateway(
                        "failed to read request body for idempotency processing",
                    ));
                }
            };
            let fingerprint = fingerprint_request(&method, &path, &collected);

            // Cache lookup — replay only when the fingerprints match.
            let cached: Option<String> = AsyncCommands::get(&mut conn, &redis_key)
                .await
                .unwrap_or(None);
            if let Some(payload) = cached {
                if let Ok(parsed) = serde_json::from_str::<CachedResponse>(&payload) {
                    if parsed.fingerprint == fingerprint {
                        return Ok(replay(parsed));
                    } else {
                        return Ok(idempotency_conflict());
                    }
                }
            }

            let req = Request::from_parts(parts, Body::from(collected));
            let response = inner.call(req).await?;

            // Re-buffer the body so we can both forward and cache it.
            let (parts, body) = response.into_parts();
            let collected = match body.collect().await {
                Ok(c) => c.to_bytes(),
                Err(err) => {
                    // We already started streaming — fall back to an empty
                    // body for caching but don't fail the live request.
                    tracing::warn!(
                        error = %err,
                        idempotency_key = %key,
                        "failed to buffer response body for cache; serving empty body"
                    );
                    let resp = Response::from_parts(parts, Body::empty());
                    return Ok(resp);
                }
            };

            // Persist the response for future replays.
            let mut headers_vec = Vec::new();
            for (k, v) in parts.headers.iter() {
                if let Ok(val) = v.to_str() {
                    headers_vec.push((k.as_str().to_string(), val.to_string()));
                }
            }
            let cached = CachedResponse {
                status: parts.status.as_u16(),
                headers: headers_vec,
                body_b64: encode_b64(&collected),
                fingerprint,
            };
            if let Ok(serialized) = serde_json::to_string(&cached) {
                let _: redis_client::RedisResult<()> =
                    AsyncCommands::set_ex(&mut conn, &redis_key, serialized, ttl_secs).await;
            }

            Ok(Response::from_parts(parts, Body::from(collected)))
        })
    }
}

fn replay(cached: CachedResponse) -> Response<Body> {
    let mut builder =
        Response::builder().status(StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK));
    for (k, v) in &cached.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::try_from(k.as_str()), HeaderValue::from_str(v))
        {
            builder = builder.header(name, value);
        }
    }
    builder = builder.header("x-idempotent-replay", "1");
    let bytes = decode_b64(&cached.body_b64).unwrap_or_default();
    builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// RFC 7807 problem response for fingerprint mismatch on idempotency
/// replay. The caller likely reused an `Idempotency-Key` across distinct
/// operations and should regenerate the key.
fn idempotency_conflict() -> Response<Body> {
    let body = json!({
        "type": "about:blank",
        "title": "Idempotency-Key Conflict",
        "status": 409,
        "detail": "Idempotency-Key reused across requests with different fingerprints",
    });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    Response::builder()
        .status(StatusCode::CONFLICT)
        .header("content-type", "application/problem+json")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// RFC 7807 problem response for upstream-style errors (e.g. failure to
/// buffer the request body). 502 because the middleware itself acted as
/// the gateway between the client and the inner handler.
fn bad_gateway(detail: &str) -> Response<Body> {
    let body = json!({
        "type": "about:blank",
        "title": "Bad Gateway",
        "status": 502,
        "detail": detail,
    });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header("content-type", "application/problem+json")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// SHA-256 fingerprint of `(method, path, body)`.
///
/// Cached idempotency replays must match the fingerprint of the
/// original request — mismatches surface as HTTP 409. This helper is
/// public so tests and external tooling can verify fingerprints
/// without re-implementing the algorithm.
pub fn fingerprint_request(method: &Method, path: &str, body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(method.as_str().as_bytes());
    h.update(b"\n");
    h.update(path.as_bytes());
    h.update(b"\n");
    h.update(body);
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

fn encode_b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_b64(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_roundtrip() {
        let data = b"hello, world";
        let enc = encode_b64(data);
        let dec = decode_b64(&enc).unwrap();
        assert_eq!(dec, data);
    }

    #[test]
    fn fingerprint_changes_when_body_changes() {
        let a = fingerprint_request(&Method::POST, "/x", b"a");
        let b = fingerprint_request(&Method::POST, "/x", b"b");
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_changes_when_path_changes() {
        let a = fingerprint_request(&Method::POST, "/a", b"x");
        let b = fingerprint_request(&Method::POST, "/b", b"x");
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_changes_when_method_changes() {
        let a = fingerprint_request(&Method::POST, "/x", b"x");
        let b = fingerprint_request(&Method::PUT, "/x", b"x");
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_stable_for_same_inputs() {
        let a = fingerprint_request(&Method::POST, "/x", b"y");
        let b = fingerprint_request(&Method::POST, "/x", b"y");
        assert_eq!(a, b);
    }
}
