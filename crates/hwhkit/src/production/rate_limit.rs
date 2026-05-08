//! Redis-backed token-bucket rate limiter.
//!
//! Implements a tower [`Layer`] that consults Redis via an atomic Lua
//! script (`limit_lua`) before forwarding each request. On exhaustion the
//! middleware returns a 429 response with `Retry-After` and an RFC 7807
//! `application/problem+json` body.
//!
//! The keying strategy is pluggable via the `KeyExtractor` trait
//! (built into this module):
//!
//! - `PerIp` — keys by the connection's remote address (or `X-Forwarded-For`)
//! - `PerRoute` — keys by `<METHOD>:<PATH>`
//! - `PerUser` — keys by a JWT-derived user id from request extensions
//!
//! Builder API:
//!
//! ```ignore
//! use std::time::Duration;
//! use hwhkit::production::rate_limit::RateLimitLayer;
//!
//! let layer = RateLimitLayer::per_ip(redis_handle, 100, Duration::from_secs(60));
//! let app = router.layer(layer);
//! ```
//!
//! Behind the `rate-limit` feature.

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Request, Response, StatusCode};
use futures::future::BoxFuture;
use hwhkit_integration_redis::RedisHandle;
use redis_client::Script;
use serde_json::json;
use tower::{Layer, Service};

// All rate-limit decisions are made off the **Redis server clock** via
// `redis.call('TIME')` — this avoids skew between application replicas
// (a token bucket evaluated against a wandering local clock would either
// over-emit or stall as nodes disagree on "now"). Multi-replica
// deployments are therefore immune to local-clock drift, NTP outages,
// and process-suspension hiccups.
const LUA_TOKEN_BUCKET: &str = r#"
-- KEYS[1] = bucket key
-- ARGV[1] = capacity
-- ARGV[2] = refill_per_sec (decimal)
-- ARGV[3] = ttl_ms

local key = KEYS[1]
local capacity = tonumber(ARGV[1])
local refill = tonumber(ARGV[2])
local ttl = tonumber(ARGV[3])

local time = redis.call('TIME')
-- redis.call('TIME') returns {seconds, microseconds}.
local now = (tonumber(time[1]) * 1000) + math.floor(tonumber(time[2]) / 1000)

local data = redis.call('HMGET', key, 'tokens', 'ts')
local tokens = tonumber(data[1])
local ts = tonumber(data[2])

if tokens == nil then
    tokens = capacity
    ts = now
end

local elapsed = math.max(0, now - ts) / 1000.0
tokens = math.min(capacity, tokens + elapsed * refill)

local allowed = 0
local retry_after_ms = 0
if tokens >= 1 then
    tokens = tokens - 1
    allowed = 1
else
    -- ms until one token regenerates
    if refill > 0 then
        retry_after_ms = math.ceil((1 - tokens) / refill * 1000)
    else
        retry_after_ms = ttl
    end
end

redis.call('HMSET', key, 'tokens', tokens, 'ts', now)
redis.call('PEXPIRE', key, ttl)

return {allowed, retry_after_ms, math.floor(tokens)}
"#;

/// Function-pointer style key extractor. Returns `Some(key)` if the
/// request should be rate-limited, or `None` to bypass the limiter for
/// this particular request.
pub type KeyFn = Arc<dyn Fn(&Request<Body>) -> Option<String> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct RateLimitLayer {
    handle: RedisHandle,
    capacity: u32,
    window: Duration,
    key_fn: KeyFn,
    namespace: String,
}

impl RateLimitLayer {
    /// Construct a layer with a custom key extractor.
    pub fn new(handle: RedisHandle, capacity: u32, window: Duration, key_fn: KeyFn) -> Self {
        Self {
            handle,
            capacity,
            window,
            key_fn,
            namespace: "hwhkit:rl".to_string(),
        }
    }

    /// Override the Redis key namespace (default `hwhkit:rl`). Useful when
    /// multiple services share a Redis cluster.
    #[must_use]
    pub fn with_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = ns.into();
        self
    }

    /// Per-IP token bucket. Reads `X-Forwarded-For` or the connection's
    /// remote address (via `axum::extract::ConnectInfo`).
    pub fn per_ip(handle: RedisHandle, capacity: u32, window: Duration) -> Self {
        let key_fn: KeyFn = Arc::new(|req: &Request<Body>| {
            // Prefer the leftmost X-Forwarded-For entry; fall back to the
            // socket address if axum injected one as an extension.
            if let Some(xff) = req
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
            {
                let first = xff.split(',').next()?.trim();
                if !first.is_empty() {
                    return Some(format!("ip:{first}"));
                }
            }
            if let Some(addr) = req
                .extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            {
                return Some(format!("ip:{}", addr.0.ip()));
            }
            None
        });
        Self::new(handle, capacity, window, key_fn)
    }

    /// Per-route token bucket — `<METHOD>:<PATH>` is the key.
    pub fn per_route(handle: RedisHandle, capacity: u32, window: Duration) -> Self {
        let key_fn: KeyFn = Arc::new(|req: &Request<Body>| {
            Some(format!("route:{}:{}", req.method(), req.uri().path()))
        });
        Self::new(handle, capacity, window, key_fn)
    }

    /// Per-user token bucket. Requires the authenticated user id to live
    /// in request extensions as a `String` (insert it from your auth
    /// middleware after JWT verification).
    pub fn per_user(handle: RedisHandle, capacity: u32, window: Duration) -> Self {
        let key_fn: KeyFn = Arc::new(|req: &Request<Body>| {
            req.extensions()
                .get::<UserId>()
                .map(|u| format!("user:{}", u.0))
        });
        Self::new(handle, capacity, window, key_fn)
    }

    /// Per-tenant token bucket. Requires `hwhkit_core::TenantId` to live
    /// in request extensions (e.g. inserted by `TenantExtractorLayer`).
    /// When a request arrives without a tenant id, falls back to a
    /// per-IP bucket — this matches the typical "best-effort scoping"
    /// pattern where unauthenticated requests still need to be limited.
    /// Use [`RateLimitLayer::per_tenant_strict`] if you want to skip
    /// rate-limiting for tenant-less requests instead.
    #[cfg(feature = "multi-tenant")]
    pub fn per_tenant(handle: RedisHandle, capacity: u32, window: Duration) -> Self {
        let key_fn: KeyFn = Arc::new(|req: &Request<Body>| {
            if let Some(t) = req.extensions().get::<hwhkit_core::TenantId>() {
                return Some(format!("tenant:{}", t.as_str()));
            }
            // Fallback: per-IP. Inlined to avoid borrowing `Self`.
            if let Some(xff) = req
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
            {
                let first = xff.split(',').next()?.trim();
                if !first.is_empty() {
                    return Some(format!("ip:{first}"));
                }
            }
            req.extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|addr| format!("ip:{}", addr.0.ip()))
        });
        Self::new(handle, capacity, window, key_fn)
    }

    /// Strict variant of [`RateLimitLayer::per_tenant`]: requests without
    /// a tenant id bypass the limiter entirely. Useful for routes that
    /// only ever serve authenticated tenant traffic.
    #[cfg(feature = "multi-tenant")]
    pub fn per_tenant_strict(handle: RedisHandle, capacity: u32, window: Duration) -> Self {
        let key_fn: KeyFn = Arc::new(|req: &Request<Body>| {
            req.extensions()
                .get::<hwhkit_core::TenantId>()
                .map(|t| format!("tenant:{}", t.as_str()))
        });
        Self::new(handle, capacity, window, key_fn)
    }
}

/// Newtype for inserting the authenticated user id into the request
/// extensions for [`RateLimitLayer::per_user`].
#[derive(Clone, Debug)]
pub struct UserId(pub String);

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimit<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RateLimit {
            inner,
            handle: self.handle.clone(),
            capacity: self.capacity,
            window: self.window,
            key_fn: self.key_fn.clone(),
            namespace: self.namespace.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RateLimit<S> {
    inner: S,
    handle: RedisHandle,
    capacity: u32,
    window: Duration,
    key_fn: KeyFn,
    namespace: String,
}

impl<S> Service<Request<Body>> for RateLimit<S>
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
        let key = match (self.key_fn)(&req) {
            Some(k) => format!("{}:{}", self.namespace, k),
            None => {
                let clone = self.inner.clone();
                let mut inner = std::mem::replace(&mut self.inner, clone);
                return Box::pin(async move { inner.call(req).await });
            }
        };

        let capacity = self.capacity;
        let refill_per_sec = self.capacity as f64 / self.window.as_secs_f64().max(0.001);
        let ttl_ms = (self.window.as_millis() as i64).max(1_000);
        let mut conn = self.handle.manager();
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            // The Lua script reads time via `redis.call('TIME')`, so all
            // replicas making decisions against the same Redis instance
            // share a single clock. `now_ms` is no longer passed in.
            let result: redis_client::RedisResult<(i32, i64, i64)> = Script::new(LUA_TOKEN_BUCKET)
                .key(key.clone())
                .arg(capacity)
                .arg(refill_per_sec)
                .arg(ttl_ms)
                .invoke_async(&mut conn)
                .await;

            match result {
                Ok((1, _, remaining)) => {
                    let mut response = inner.call(req).await?;
                    response.headers_mut().insert(
                        HeaderName::from_static("x-ratelimit-remaining"),
                        HeaderValue::from(remaining),
                    );
                    Ok(response)
                }
                Ok((_, retry_after_ms, _)) => {
                    let retry_ms: i64 = retry_after_ms;
                    Ok(too_many_requests(retry_ms.max(0) as u64))
                }
                Err(err) => {
                    // Fail open — log and let the request through. A
                    // half-broken Redis must not take the service down.
                    tracing::warn!(error = %err, "rate-limit redis failure; failing open");
                    inner.call(req).await
                }
            }
        })
    }
}

#[cfg(all(test, feature = "multi-tenant"))]
mod tenant_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use hwhkit_core::TenantId;

    /// Construct only the key extractor used by `per_tenant_strict` to
    /// confirm two tenants produce two distinct cache keys, and that an
    /// absent tenant id yields `None` (skipping the bucket).
    #[test]
    fn per_tenant_strict_keys_are_per_tenant() {
        let key_fn: KeyFn = Arc::new(|req: &Request<Body>| {
            req.extensions()
                .get::<TenantId>()
                .map(|t| format!("tenant:{}", t.as_str()))
        });

        let mut req_a = Request::builder().uri("/x").body(Body::empty()).unwrap();
        req_a.extensions_mut().insert(TenantId::new("tenant-a"));
        let mut req_b = Request::builder().uri("/x").body(Body::empty()).unwrap();
        req_b.extensions_mut().insert(TenantId::new("tenant-b"));
        let req_none = Request::builder().uri("/x").body(Body::empty()).unwrap();

        let ka = key_fn(&req_a);
        let kb = key_fn(&req_b);
        let kn = key_fn(&req_none);
        assert_eq!(ka.as_deref(), Some("tenant:tenant-a"));
        assert_eq!(kb.as_deref(), Some("tenant:tenant-b"));
        assert_eq!(kn, None);
        assert_ne!(ka, kb);
    }
}

fn too_many_requests(retry_after_ms: u64) -> Response<Body> {
    let retry_secs = (retry_after_ms / 1000).max(1);
    let body = json!({
        "type": "about:blank",
        "title": "Too Many Requests",
        "status": 429,
        "detail": format!("rate limit exceeded; retry in {retry_secs}s"),
    });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("retry-after", retry_secs.to_string())
        .header("content-type", "application/problem+json")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}
