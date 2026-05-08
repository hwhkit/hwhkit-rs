//! Tower middleware that extracts the tenant id from a request header
//! and inserts a [`hwhkit_core::TenantId`] into the request extensions
//! for downstream handlers.
//!
//! # Trust boundary
//!
//! The header configured here (`X-Tenant-Id` by default) is **untrusted**
//! unless the layer is composed with an authentication mechanism that
//! validates the caller's identity *and* their authorization to operate
//! as the requested tenant — JWT, mTLS, signed gateway header, …
//!
//! In particular, do NOT use this header alone for tenant isolation in
//! a multi-tenant SaaS deployment: anyone able to reach the service
//! could otherwise impersonate any tenant by setting the header. The
//! proper composition is to verify a JWT *first*, extract the tenant
//! claim from the verified token, and only then trust the value.
//!
//! Behind the `multi-tenant` feature.

use std::convert::Infallible;
use std::task::{Context, Poll};

use axum::http::header::HeaderName;
use axum::http::{Request, Response};
use futures::future::BoxFuture;
use hwhkit_core::TenantId;
use tower::{Layer, Service};

/// Default header read by [`TenantExtractorLayer`].
pub const DEFAULT_TENANT_HEADER: &str = "x-tenant-id";

/// Maximum tenant id length the extractor is willing to accept. The
/// value is kept short on purpose — tenant ids end up in cache keys
/// (idempotency, rate limit) and metrics labels, so unbounded length
/// would translate to memory amplification.
pub const MAX_TENANT_ID_LEN: usize = 128;

#[derive(Clone)]
pub struct TenantExtractorLayer {
    header_name: HeaderName,
    max_len: usize,
}

impl TenantExtractorLayer {
    pub fn new(header: &str) -> Self {
        let header_name = HeaderName::from_bytes(header.as_bytes())
            .unwrap_or_else(|_| HeaderName::from_static(DEFAULT_TENANT_HEADER));
        Self {
            header_name,
            max_len: MAX_TENANT_ID_LEN,
        }
    }

    /// Override the upper bound on accepted tenant id length. Values
    /// above the cap are rejected (the request continues without a
    /// `TenantId` extension being inserted).
    pub fn with_max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }
}

impl Default for TenantExtractorLayer {
    fn default() -> Self {
        Self::new(DEFAULT_TENANT_HEADER)
    }
}

impl<S> Layer<S> for TenantExtractorLayer {
    type Service = TenantExtractor<S>;
    fn layer(&self, inner: S) -> Self::Service {
        TenantExtractor {
            inner,
            header_name: self.header_name.clone(),
            max_len: self.max_len,
        }
    }
}

#[derive(Clone)]
pub struct TenantExtractor<S> {
    inner: S,
    header_name: HeaderName,
    max_len: usize,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for TenantExtractor<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        // Validate the header before stamping the extension:
        //
        //  - reject non-UTF-8 (HTTP allows arbitrary bytes; we don't
        //    want to propagate non-text into log fields and cache keys).
        //  - reject empty strings.
        //  - reject anything longer than `max_len` to bound the blast
        //    radius of a maliciously-large header.
        //  - reject values containing control characters or whitespace
        //    other than the leading/trailing space we trim.
        // Snapshot the validated value before touching extensions_mut
        // so the borrow checker is happy.
        let validated = req
            .headers()
            .get(&self.header_name)
            .and_then(|raw| match raw.to_str() {
                Ok(s) => Some(s.trim().to_string()),
                Err(_) => {
                    tracing::debug!("tenant id header rejected (non-utf-8 bytes)");
                    None
                }
            })
            .filter(|trimmed| {
                if trimmed.is_empty() {
                    return false;
                }
                if trimmed.len() > self.max_len {
                    tracing::debug!(
                        len = trimmed.len(),
                        max_len = self.max_len,
                        "tenant id header rejected (too long)"
                    );
                    return false;
                }
                if !trimmed
                    .chars()
                    .all(|c| !c.is_control() && !c.is_whitespace())
                {
                    tracing::debug!("tenant id header rejected (contains control or whitespace)");
                    return false;
                }
                true
            });
        if let Some(value) = validated {
            req.extensions_mut().insert(TenantId::new(value));
        }

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move { inner.call(req).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode};
    use std::convert::Infallible;
    use tower::{Service, ServiceExt};

    /// Stub inner service that pulls the TenantId out of extensions and
    /// echoes it back via a status code.
    #[derive(Clone)]
    struct EchoTenantPresence;

    impl Service<Request<Body>> for EchoTenantPresence {
        type Response = Response<Body>;
        type Error = Infallible;
        type Future = futures::future::BoxFuture<'static, Result<Self::Response, Self::Error>>;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: Request<Body>) -> Self::Future {
            let has_tenant = req.extensions().get::<TenantId>().is_some();
            Box::pin(async move {
                Ok(Response::builder()
                    .status(if has_tenant {
                        StatusCode::OK
                    } else {
                        StatusCode::NO_CONTENT
                    })
                    .body(Body::empty())
                    .unwrap())
            })
        }
    }

    fn build() -> TenantExtractor<EchoTenantPresence> {
        let layer = TenantExtractorLayer::default().with_max_len(8);
        tower::Layer::layer(&layer, EchoTenantPresence)
    }

    #[tokio::test]
    async fn accepts_normal_tenant() {
        let mut svc = build();
        let req = Request::builder()
            .header(DEFAULT_TENANT_HEADER, "acme")
            .body(Body::empty())
            .unwrap();
        let r = ServiceExt::<Request<Body>>::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_overlong() {
        let mut svc = build();
        let req = Request::builder()
            .header(DEFAULT_TENANT_HEADER, "012345678") // > 8 chars
            .body(Body::empty())
            .unwrap();
        let r = ServiceExt::<Request<Body>>::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn rejects_control_chars() {
        let mut svc = build();
        let req = Request::builder()
            .header(DEFAULT_TENANT_HEADER, "ac\tme")
            .body(Body::empty())
            .unwrap();
        let r = ServiceExt::<Request<Body>>::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
    }
}
