//! JWT verification chain with JWKS auto-fetch + caching, multi-algorithm
//! support, and an axum extractor for handler-level deserialization.
//!
//! The verifier is constructed once (typically during bootstrap), shared
//! via [`crate::AppContext`], and consulted from the [`Claims`] extractor
//! on every authenticated request. JWKS are fetched on demand and cached
//! for [`JwtVerifierConfig::cache_ttl`]. Verification supports the most
//! common JWT signing algorithms (RS256, RS384, RS512, ES256, ES384, HS256,
//! HS384, HS512).
//!
//! Use:
//!
//! ```ignore
//! use serde::Deserialize;
//! use hwhkit_core::jwt::{Claims, JwtVerifier, JwtVerifierConfig};
//!
//! #[derive(Deserialize)]
//! struct AppClaims { sub: String }
//!
//! async fn handler(Claims(c): Claims<AppClaims>) -> String { c.sub }
//! ```
//!
//! Behind the `jwt` feature.

#![cfg(feature = "jwt")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::async_trait;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::response::{IntoResponse, Response};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error_response::ApiError;
use crate::AppContext;

/// JWKS-backed JWT verifier. Cheap to clone (Arc inside).
#[derive(Clone)]
pub struct JwtVerifier {
    inner: Arc<JwtVerifierInner>,
}

struct JwtVerifierInner {
    cfg: JwtVerifierConfig,
    cache: RwLock<JwksCache>,
    /// Single-flight gate around `refresh_jwks`. Guarantees that only one
    /// JWKS HTTP fetch is in flight at any moment, even when many verifies
    /// race a cache expiry. Holds nothing — it's purely a mutex.
    refresh_lock: tokio::sync::Mutex<()>,
    /// Optional pre-shared HMAC secret for HS256/384/512. Wrapped in
    /// [`zeroize::Zeroizing`] so the bytes are wiped on drop — defence
    /// in depth against post-mortem core dumps and heap inspection.
    hmac_secret: Option<zeroize::Zeroizing<Vec<u8>>>,
    /// Pluggable JWKS source. Defaults to the production `reqwest` fetcher;
    /// tests inject a counting in-memory implementation to assert
    /// single-flight behaviour.
    fetcher: Arc<dyn JwksFetcher>,
}

/// Pluggable transport for `JwksDoc`. The default is HTTP via `reqwest`;
/// tests can substitute an implementation that counts hits to assert
/// single-flight semantics.
trait JwksFetcher: Send + Sync + 'static {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<JwksDoc, JwtError>> + Send + 'a>>;
}

struct ReqwestJwksFetcher {
    http: reqwest::Client,
}

impl JwksFetcher for ReqwestJwksFetcher {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<JwksDoc, JwtError>> + Send + 'a>>
    {
        Box::pin(async move {
            let resp = self
                .http
                .get(url)
                .send()
                .await
                .map_err(|e| JwtError::Jwks {
                    message: format!("get {url}"),
                    source: Some(Box::new(e)),
                })?;
            if !resp.status().is_success() {
                return Err(JwtError::Jwks {
                    message: format!("{url} returned status {}", resp.status()),
                    source: None,
                });
            }
            resp.json::<JwksDoc>().await.map_err(|e| JwtError::Jwks {
                message: "parse jwks".to_string(),
                source: Some(Box::new(e)),
            })
        })
    }
}

#[derive(Default)]
struct JwksCache {
    keys: Vec<CachedKey>,
    fetched_at: Option<Instant>,
}

struct CachedKey {
    kid: Option<String>,
    alg: Option<String>,
    decoding: DecodingKey,
}

/// Tunables for [`JwtVerifier`].
///
/// Six independent optional fields → use the [`JwtVerifierConfigBuilder`]
/// returned by [`JwtVerifierConfig::builder`] for ergonomic construction.
/// Marked `#[non_exhaustive]` so future fields don't break callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct JwtVerifierConfig {
    /// Issuer claim required on every token (`iss`). Empty → not enforced.
    pub issuer: Option<String>,
    /// Audience claim required on every token (`aud`). Empty → not enforced.
    pub audience: Option<String>,
    /// JWKS URL to refresh public keys from. When set, the verifier will
    /// fetch + cache the JSON Web Key Set on first use and refresh after
    /// `cache_ttl`. May be `None` for symmetric (HS) configurations using
    /// a static secret instead.
    pub jwks_url: Option<String>,
    /// Cache lifetime for fetched JWKS. Default 1 hour.
    pub cache_ttl: Duration,
    /// Algorithms accepted by the verifier. Defaults to RS256.
    pub algorithms: Vec<String>,
    /// Leeway tolerance for the `exp`/`nbf` claims, in seconds.
    pub leeway_secs: u64,
}

impl Default for JwtVerifierConfig {
    fn default() -> Self {
        Self {
            issuer: None,
            audience: None,
            jwks_url: None,
            cache_ttl: Duration::from_secs(3600),
            algorithms: vec!["RS256".to_string()],
            leeway_secs: 30,
        }
    }
}

impl JwtVerifierConfig {
    /// Start a [`JwtVerifierConfigBuilder`] seeded with default values.
    #[must_use = "builder does nothing until consumed via .build()"]
    pub fn builder() -> JwtVerifierConfigBuilder {
        JwtVerifierConfigBuilder::default()
    }
}

/// Fluent builder for [`JwtVerifierConfig`].
///
/// All fields are optional; unset fields fall back to the
/// [`JwtVerifierConfig::default`] value. Construct via
/// [`JwtVerifierConfig::builder`].
#[derive(Debug, Default, Clone)]
#[must_use = "builder does nothing until consumed via .build()"]
pub struct JwtVerifierConfigBuilder {
    inner: JwtVerifierConfig,
}

impl JwtVerifierConfigBuilder {
    /// Require this issuer on every verified token.
    pub fn issuer(mut self, iss: impl Into<String>) -> Self {
        self.inner.issuer = Some(iss.into());
        self
    }

    /// Require this audience on every verified token.
    pub fn audience(mut self, aud: impl Into<String>) -> Self {
        self.inner.audience = Some(aud.into());
        self
    }

    /// Set the JWKS endpoint to fetch signing keys from.
    pub fn jwks_url(mut self, url: impl Into<String>) -> Self {
        self.inner.jwks_url = Some(url.into());
        self
    }

    /// Override the JWKS cache TTL (default 1 hour).
    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        self.inner.cache_ttl = ttl;
        self
    }

    /// Replace the accepted algorithm list (default `["RS256"]`).
    pub fn algorithms(mut self, algs: Vec<String>) -> Self {
        self.inner.algorithms = algs;
        self
    }

    /// Override the `exp` / `nbf` leeway in seconds (default 30s).
    pub fn leeway_secs(mut self, secs: u64) -> Self {
        self.inner.leeway_secs = secs;
        self
    }

    /// Materialize the final [`JwtVerifierConfig`].
    pub fn build(self) -> JwtVerifierConfig {
        self.inner
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JwtError {
    #[error("missing Authorization header")]
    MissingHeader,
    #[error("malformed Authorization header (expected `Bearer <token>`)")]
    Malformed,
    #[error("token header parse failed: {message}")]
    HeaderParse {
        message: String,
        #[source]
        source: Option<crate::error::BoxError>,
    },
    #[error("no signing key matched (kid={kid:?}, alg={alg:?})")]
    NoMatchingKey {
        kid: Option<String>,
        alg: Option<String>,
    },
    #[error("verification failed: {message}")]
    Verify {
        message: String,
        #[source]
        source: Option<crate::error::BoxError>,
    },
    #[error("jwks fetch failed: {message}")]
    Jwks {
        message: String,
        #[source]
        source: Option<crate::error::BoxError>,
    },
    #[error("verifier not configured: {0}")]
    Config(String),
}

impl JwtError {
    fn into_api_error(self) -> ApiError {
        ApiError::unauthorized(self.to_string())
    }
}

impl JwtVerifier {
    /// Create a verifier that fetches keys from a JWKS endpoint.
    #[must_use = "verifier construction is non-trivial; assign or pass it to the application"]
    pub fn from_jwks(cfg: JwtVerifierConfig) -> Result<Self, JwtError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| JwtError::Config(format!("reqwest client init failed: {e}")))?;
        let fetcher: Arc<dyn JwksFetcher> = Arc::new(ReqwestJwksFetcher { http });
        Ok(Self {
            inner: Arc::new(JwtVerifierInner {
                cfg,
                cache: RwLock::new(JwksCache::default()),
                refresh_lock: tokio::sync::Mutex::new(()),
                hmac_secret: None,
                fetcher,
            }),
        })
    }

    /// Create a verifier that uses a single shared HMAC secret. Suitable
    /// for closed-system tokens (e.g. internal microservices). Algorithms
    /// must be HS-family.
    #[must_use = "verifier construction is non-trivial; assign or pass it to the application"]
    pub fn from_hmac(cfg: JwtVerifierConfig, secret: impl Into<Vec<u8>>) -> Self {
        let fetcher: Arc<dyn JwksFetcher> = Arc::new(ReqwestJwksFetcher {
            http: reqwest::Client::new(),
        });
        Self {
            inner: Arc::new(JwtVerifierInner {
                cfg,
                cache: RwLock::new(JwksCache::default()),
                refresh_lock: tokio::sync::Mutex::new(()),
                hmac_secret: Some(zeroize::Zeroizing::new(secret.into())),
                fetcher,
            }),
        }
    }

    fn algorithms(&self) -> Vec<Algorithm> {
        self.inner
            .cfg
            .algorithms
            .iter()
            .filter_map(|a| parse_alg(a))
            .collect()
    }

    /// Verify a Bearer token string (without the leading `Bearer ` prefix).
    /// Returns the decoded claims of type `T`.
    pub async fn verify<T: DeserializeOwned>(&self, token: &str) -> Result<T, JwtError> {
        let header = decode_header(token).map_err(|e| JwtError::HeaderParse {
            message: e.to_string(),
            source: Some(Box::new(e)),
        })?;
        let kid = header.kid.clone();
        let alg = header.alg;

        let mut validation = Validation::new(alg);
        if let Some(iss) = &self.inner.cfg.issuer {
            validation.set_issuer(&[iss]);
        }
        if let Some(aud) = &self.inner.cfg.audience {
            validation.set_audience(&[aud]);
        } else {
            validation.validate_aud = false;
        }
        let allowed = self.algorithms();
        if !allowed.is_empty() {
            validation.algorithms = allowed;
        }
        validation.leeway = self.inner.cfg.leeway_secs;

        if let Some(secret) = &self.inner.hmac_secret {
            let key = DecodingKey::from_secret(secret);
            return decode::<T>(token, &key, &validation)
                .map(|d| d.claims)
                .map_err(|e| JwtError::Verify {
                    message: e.to_string(),
                    source: Some(Box::new(e)),
                });
        }

        // JWKS path
        let key =
            self.key_for(kid.as_deref(), alg)
                .await?
                .ok_or_else(|| JwtError::NoMatchingKey {
                    kid: kid.clone(),
                    alg: Some(format!("{alg:?}")),
                })?;

        decode::<T>(token, &key, &validation)
            .map(|d| d.claims)
            .map_err(|e| JwtError::Verify {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })
    }

    async fn key_for(
        &self,
        kid: Option<&str>,
        alg: Algorithm,
    ) -> Result<Option<DecodingKey>, JwtError> {
        let alg_str = format!("{alg:?}");

        // Refresh if missing or stale (single-flight gated).
        let needs_refresh = self.cache_is_stale();
        if needs_refresh {
            self.refresh_jwks_singleflight().await?;
        }

        if let Some(key) = self.lookup_key(kid, &alg_str) {
            return Ok(Some(key));
        }

        // Cache miss for the requested kid. The provider may have rotated
        // its key; trigger a single synchronous refresh (forcing the fetch
        // even though the cache is fresh) and try again. Skip if we just
        // refreshed above — no point in two back-to-back fetches.
        if !needs_refresh {
            self.force_refresh_jwks_singleflight(/*pre_fetched_at=*/ self.cache_fetched_at())
                .await?;
            if let Some(key) = self.lookup_key(kid, &alg_str) {
                return Ok(Some(key));
            }
        }

        Ok(None)
    }

    fn cache_fetched_at(&self) -> Option<Instant> {
        self.inner.cache.read().fetched_at
    }

    fn cache_is_stale(&self) -> bool {
        let cache = self.inner.cache.read();
        match cache.fetched_at {
            None => true,
            Some(t) => t.elapsed() > self.inner.cfg.cache_ttl,
        }
    }

    fn lookup_key(&self, kid: Option<&str>, alg_str: &str) -> Option<DecodingKey> {
        let cache = self.inner.cache.read();
        cache
            .keys
            .iter()
            .find(|k| match (kid, &k.kid) {
                (Some(req), Some(stored)) => req == stored,
                (None, _) => k.alg.as_deref() == Some(alg_str) || k.alg.is_none(),
                _ => false,
            })
            .map(|k| k.decoding.clone())
    }

    /// Single-flight wrapper around [`Self::refresh_jwks`]. Holds the
    /// `refresh_lock` for the duration of the fetch so concurrent callers
    /// piggy-back on the in-flight result instead of stampeding the JWKS
    /// endpoint. After acquiring the lock we re-check the cache freshness
    /// — if a peer already refreshed while we waited, we skip the fetch.
    async fn refresh_jwks_singleflight(&self) -> Result<(), JwtError> {
        let _guard = self.inner.refresh_lock.lock().await;
        if !self.cache_is_stale() {
            // A concurrent caller already refreshed; nothing to do.
            return Ok(());
        }
        self.refresh_jwks().await
    }

    /// Single-flight refresh that ignores TTL freshness — used for the
    /// kid-miss retry path, where the cache may be fresh but missing the
    /// just-rotated key. We still de-duplicate concurrent callers: if the
    /// `fetched_at` timestamp moved while we waited for the lock, the
    /// peer beat us to it and we skip.
    async fn force_refresh_jwks_singleflight(
        &self,
        baseline: Option<Instant>,
    ) -> Result<(), JwtError> {
        let _guard = self.inner.refresh_lock.lock().await;
        if self.cache_fetched_at() != baseline {
            // A concurrent caller already refreshed while we waited.
            return Ok(());
        }
        self.refresh_jwks().await
    }

    async fn refresh_jwks(&self) -> Result<(), JwtError> {
        let url = self
            .inner
            .cfg
            .jwks_url
            .as_deref()
            .ok_or_else(|| JwtError::Config("jwks_url not configured".to_string()))?;

        let body = self.inner.fetcher.fetch(url).await?;

        let mut cached = Vec::new();
        for k in body.keys {
            if let Some(decoding) = build_decoding_key(&k) {
                cached.push(CachedKey {
                    kid: k.kid,
                    alg: k.alg,
                    decoding,
                });
            }
        }

        let mut guard = self.inner.cache.write();
        guard.keys = cached;
        guard.fetched_at = Some(Instant::now());
        Ok(())
    }
}

#[derive(Deserialize)]
struct JwksDoc {
    keys: Vec<JwkEntry>,
}

#[derive(Deserialize)]
struct JwkEntry {
    kty: String,
    kid: Option<String>,
    alg: Option<String>,
    #[serde(rename = "use")]
    _use: Option<String>,
    n: Option<String>,
    e: Option<String>,
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
    k: Option<String>,
}

fn build_decoding_key(k: &JwkEntry) -> Option<DecodingKey> {
    match k.kty.as_str() {
        "RSA" => {
            let n = k.n.as_deref()?;
            let e = k.e.as_deref()?;
            DecodingKey::from_rsa_components(n, e).ok()
        }
        "EC" => {
            let _ = k.crv.as_deref()?;
            let x = k.x.as_deref()?;
            let y = k.y.as_deref()?;
            DecodingKey::from_ec_components(x, y).ok()
        }
        "oct" => {
            let raw = k.k.as_deref()?;
            // Base64url-decoded raw secret.
            use base64::Engine;
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(raw)
                .ok()?;
            Some(DecodingKey::from_secret(&bytes))
        }
        _ => None,
    }
}

fn parse_alg(s: &str) -> Option<Algorithm> {
    match s {
        "HS256" => Some(Algorithm::HS256),
        "HS384" => Some(Algorithm::HS384),
        "HS512" => Some(Algorithm::HS512),
        "RS256" => Some(Algorithm::RS256),
        "RS384" => Some(Algorithm::RS384),
        "RS512" => Some(Algorithm::RS512),
        "ES256" => Some(Algorithm::ES256),
        "ES384" => Some(Algorithm::ES384),
        "PS256" => Some(Algorithm::PS256),
        "PS384" => Some(Algorithm::PS384),
        "PS512" => Some(Algorithm::PS512),
        "EdDSA" => Some(Algorithm::EdDSA),
        _ => None,
    }
}

/// Axum extractor: deserialize claims of type `T` from the verified
/// `Authorization: Bearer <token>` header.
///
/// Requires `JwtVerifier` to be present in the application state — either
/// inserted directly (via `Router::with_state(verifier)`) or via the
/// [`AppContext`] (the bootstrap inserts it automatically when the JWT
/// verifier was configured).
#[derive(Debug, Clone)]
pub struct Claims<T>(pub T);

#[async_trait]
impl<S, T> FromRequestParts<S> for Claims<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send + Sync + 'static,
    JwtVerifier: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let verifier = JwtVerifier::from_ref(state);
        let token = extract_bearer(parts).map_err(|e| e.into_api_error().into_response())?;
        let claims = verifier
            .verify::<T>(&token)
            .await
            .map_err(|e| e.into_api_error().into_response())?;
        Ok(Claims(claims))
    }
}

/// Convenience extractor that doesn't require putting [`JwtVerifier`] in
/// the typed state — it pulls the verifier out of an [`AppContext`] held
/// in the request extensions instead. The bootstrap layer inserts the
/// context automatically.
#[derive(Debug, Clone)]
pub struct CtxClaims<T>(pub T);

#[async_trait]
impl<S, T> FromRequestParts<S> for CtxClaims<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send + Sync + 'static,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<AppContext>()
            .cloned()
            .ok_or_else(|| {
                ApiError::internal("AppContext missing from request extensions").into_response()
            })?;
        let verifier = ctx.get::<JwtVerifier>().ok_or_else(|| {
            ApiError::internal("JwtVerifier not registered in AppContext").into_response()
        })?;
        let token = extract_bearer(parts).map_err(|e| e.into_api_error().into_response())?;
        let claims = (*verifier)
            .verify::<T>(&token)
            .await
            .map_err(|e| e.into_api_error().into_response())?;
        Ok(CtxClaims(claims))
    }
}

fn extract_bearer(parts: &Parts) -> Result<String, JwtError> {
    let header = parts
        .headers
        .get(AUTHORIZATION)
        .ok_or(JwtError::MissingHeader)?;
    let s = header.to_str().map_err(|_| JwtError::Malformed)?;
    let s = s.trim();
    if let Some(rest) = s
        .strip_prefix("Bearer ")
        .or_else(|| s.strip_prefix("bearer "))
    {
        Ok(rest.trim().to_string())
    } else {
        Err(JwtError::Malformed)
    }
}

/// Helper for the rare case of returning a structured 401 explicitly.
pub fn unauthorized(detail: impl Into<String>) -> Response {
    let mut resp = ApiError::unauthorized(detail).into_response();
    *resp.status_mut() = StatusCode::UNAUTHORIZED;
    resp
}

#[cfg(test)]
impl JwtVerifier {
    /// Construct a verifier wired to a caller-supplied [`JwksFetcher`].
    /// Used by integration tests that need to assert on the number of
    /// fetches performed (single-flight, kid-miss retry, …).
    fn with_fetcher(cfg: JwtVerifierConfig, fetcher: Arc<dyn JwksFetcher>) -> Self {
        Self {
            inner: Arc::new(JwtVerifierInner {
                cfg,
                cache: RwLock::new(JwksCache::default()),
                refresh_lock: tokio::sync::Mutex::new(()),
                hmac_secret: None,
                fetcher,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestClaims {
        sub: String,
        exp: usize,
        iss: String,
    }

    #[tokio::test]
    async fn hmac_verify_roundtrip() {
        let secret = b"super-secret";
        let mut cfg = JwtVerifierConfig {
            issuer: Some("hwhkit".to_string()),
            algorithms: vec!["HS256".to_string()],
            ..Default::default()
        };
        cfg.audience = None;
        let verifier = JwtVerifier::from_hmac(cfg, secret.to_vec());

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        let claims = TestClaims {
            sub: "alice".into(),
            exp: now + 600,
            iss: "hwhkit".into(),
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap();

        let decoded: TestClaims = verifier.verify(&token).await.unwrap();
        assert_eq!(decoded, claims);
    }

    /// Counting fetcher for single-flight tests. Returns a configurable
    /// `JwksDoc` after a small delay so racing tasks definitely overlap.
    struct CountingFetcher {
        hits: Arc<AtomicUsize>,
        kids: parking_lot::Mutex<Vec<String>>,
        delay: Duration,
    }

    impl CountingFetcher {
        fn new(initial_kids: Vec<&str>) -> Arc<Self> {
            Arc::new(Self {
                hits: Arc::new(AtomicUsize::new(0)),
                kids: parking_lot::Mutex::new(initial_kids.into_iter().map(String::from).collect()),
                delay: Duration::from_millis(20),
            })
        }
        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
        fn set_kids(&self, kids: Vec<&str>) {
            *self.kids.lock() = kids.into_iter().map(String::from).collect();
        }
    }

    impl JwksFetcher for CountingFetcher {
        fn fetch<'a>(
            &'a self,
            _url: &'a str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<JwksDoc, JwtError>> + Send + 'a>,
        > {
            Box::pin(async move {
                self.hits.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(self.delay).await;
                let keys = self
                    .kids
                    .lock()
                    .iter()
                    .map(|kid| JwkEntry {
                        kty: "oct".into(),
                        kid: Some(kid.clone()),
                        alg: Some("HS256".into()),
                        _use: None,
                        n: None,
                        e: None,
                        crv: None,
                        x: None,
                        y: None,
                        // base64url-encoded shared "secret" (fine for tests
                        // — we never decode the token after the key is
                        // resolved; we're asserting on fetch counts).
                        k: Some("c2VjcmV0".into()),
                    })
                    .collect();
                Ok(JwksDoc { keys })
            })
        }
    }

    /// 20 concurrent verifies hitting an empty cache must collapse into a
    /// single JWKS HTTP fetch (single-flight). The verifies themselves
    /// fail on signature (we don't supply matching keys), but `key_for`
    /// is what we're benchmarking.
    #[tokio::test]
    async fn jwks_refresh_is_single_flight() {
        let fetcher = CountingFetcher::new(vec!["k1"]);
        let cfg = JwtVerifierConfig {
            jwks_url: Some("http://example/jwks".into()),
            algorithms: vec!["HS256".into()],
            cache_ttl: Duration::from_millis(50),
            ..Default::default()
        };
        let verifier = JwtVerifier::with_fetcher(cfg, fetcher.clone());

        let mut handles = Vec::new();
        for _ in 0..20 {
            let v = verifier.clone();
            handles.push(tokio::spawn(async move {
                v.key_for(Some("k1"), Algorithm::HS256).await
            }));
        }
        for h in handles {
            let _ = h.await.unwrap();
        }
        assert_eq!(
            fetcher.hits(),
            1,
            "single-flight failed: {} fetches happened",
            fetcher.hits()
        );
    }

    /// A verify with an unknown kid must trigger one extra refresh and
    /// (after the refresh adds the kid) hit the cache successfully.
    #[tokio::test]
    async fn unknown_kid_triggers_refresh() {
        let fetcher = CountingFetcher::new(vec!["old"]);
        let cfg = JwtVerifierConfig {
            jwks_url: Some("http://example/jwks".into()),
            algorithms: vec!["HS256".into()],
            cache_ttl: Duration::from_secs(3600), // long TTL: cache won't expire
            ..Default::default()
        };
        let verifier = JwtVerifier::with_fetcher(cfg, fetcher.clone());

        // Prime the cache (1 fetch).
        let _ = verifier
            .key_for(Some("old"), Algorithm::HS256)
            .await
            .unwrap();
        assert_eq!(fetcher.hits(), 1);

        // Rotate the upstream JWKS to a new kid the cache hasn't seen.
        fetcher.set_kids(vec!["new"]);

        // First lookup of "new" — cache fresh but kid missing. Implementation
        // must trigger a synchronous refresh and retry once.
        let key = verifier
            .key_for(Some("new"), Algorithm::HS256)
            .await
            .unwrap();
        assert!(key.is_some(), "kid-miss retry did not pick up the new kid");
        assert_eq!(fetcher.hits(), 2, "expected exactly one extra fetch");

        // Repeat lookup — cache now has "new", no further fetch.
        let _ = verifier
            .key_for(Some("new"), Algorithm::HS256)
            .await
            .unwrap();
        assert_eq!(fetcher.hits(), 2);
    }

    /// Confirm a still-unknown kid after the retry resolves to None
    /// (verify will then surface NoMatchingKey).
    #[tokio::test]
    async fn unknown_kid_still_missing_returns_none() {
        let fetcher = CountingFetcher::new(vec!["old"]);
        let cfg = JwtVerifierConfig {
            jwks_url: Some("http://example/jwks".into()),
            algorithms: vec!["HS256".into()],
            cache_ttl: Duration::from_secs(3600),
            ..Default::default()
        };
        let verifier = JwtVerifier::with_fetcher(cfg, fetcher.clone());
        let _ = verifier
            .key_for(Some("old"), Algorithm::HS256)
            .await
            .unwrap();

        let key = verifier
            .key_for(Some("ghost"), Algorithm::HS256)
            .await
            .unwrap();
        assert!(key.is_none());
        // Two fetches: the prime + the kid-miss retry.
        assert_eq!(fetcher.hits(), 2);
    }

    #[tokio::test]
    async fn hmac_rejects_bad_issuer() {
        let secret = b"abc";
        let cfg = JwtVerifierConfig {
            issuer: Some("expected".to_string()),
            algorithms: vec!["HS256".to_string()],
            ..Default::default()
        };
        let verifier = JwtVerifier::from_hmac(cfg, secret.to_vec());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        let claims = TestClaims {
            sub: "x".into(),
            exp: now + 60,
            iss: "wrong".into(),
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap();
        assert!(verifier.verify::<TestClaims>(&token).await.is_err());
    }
}
