//! Property-based tests for the tenant id extraction layer.
//!
//! The middleware accepts a tenant id iff:
//!
//! - the header is valid UTF-8 (HTTP allows raw bytes; non-UTF-8 is rejected)
//! - after trimming, it is non-empty
//! - after trimming, length <= MAX_TENANT_ID_LEN
//! - after trimming, it contains no control or whitespace characters
//!
//! We assert that any input the middleware accepts also satisfies all four
//! conditions and vice versa. We also assert that no input causes a panic.

#![cfg(feature = "multi-tenant")]

use axum::body::Body;
use axum::http::header::HeaderValue;
use axum::http::{Request, Response, StatusCode};
use futures::future::BoxFuture;
use hwhkit::production::tenant::{TenantExtractorLayer, DEFAULT_TENANT_HEADER, MAX_TENANT_ID_LEN};
use hwhkit_core::TenantId;
use proptest::prelude::*;
use std::convert::Infallible;
use tower::{Layer, Service, ServiceExt};

/// Echo service that reflects whether a `TenantId` made it into the
/// extensions via 200 OK / 204 No Content.
#[derive(Clone)]
struct EchoTenantPresence;

impl Service<Request<Body>> for EchoTenantPresence {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

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

/// Reference implementation of the middleware's acceptance predicate. Must
/// stay in sync with `production::tenant::TenantExtractor::call`.
fn should_accept(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.len() > MAX_TENANT_ID_LEN {
        return false;
    }
    if !trimmed
        .chars()
        .all(|c| !c.is_control() && !c.is_whitespace())
    {
        return false;
    }
    true
}

async fn try_through_layer(header: HeaderValue) -> StatusCode {
    let layer = TenantExtractorLayer::default();
    let mut svc = layer.layer(EchoTenantPresence);
    let req = Request::builder()
        .header(DEFAULT_TENANT_HEADER, header)
        .body(Body::empty())
        .unwrap();
    let r = ServiceExt::<Request<Body>>::ready(&mut svc)
        .await
        .unwrap()
        .call(req)
        .await
        .unwrap();
    r.status()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// For arbitrary printable-ASCII-ish strings, the middleware must
    /// either accept (and stamp `TenantId`) or reject — but never panic.
    /// The accept/reject decision must agree with the documented
    /// acceptance predicate.
    #[test]
    fn accept_iff_predicate_matches(s in r"[ -~]{0,256}") {
        // Drop any byte that wouldn't fit in a header value (HTTP/1.1
        // disallows CR/LF/NUL anyway, but `[ -~]` already rules them out).
        let header = match HeaderValue::from_str(&s) {
            Ok(h) => h,
            Err(_) => return Ok(()),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let status = rt.block_on(try_through_layer(header));
        let predicate = should_accept(&s);
        if predicate {
            prop_assert_eq!(status, StatusCode::OK,
                "expected accept for `{}`", s);
        } else {
            prop_assert_eq!(status, StatusCode::NO_CONTENT,
                "expected reject for `{}`", s);
        }
    }

    /// No header at all is always rejected — never panics.
    #[test]
    fn no_header_is_rejected(_seed in 0u32..32) {
        let layer = TenantExtractorLayer::default();
        let mut svc = layer.layer(EchoTenantPresence);
        let req = Request::builder().body(Body::empty()).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let status = rt.block_on(async {
            ServiceExt::<Request<Body>>::ready(&mut svc)
                .await
                .unwrap()
                .call(req)
                .await
                .unwrap()
                .status()
        });
        prop_assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// Length bound: a string longer than `MAX_TENANT_ID_LEN` must be
    /// rejected even if every character is alphanumeric.
    #[test]
    fn overlong_alpha_is_rejected(extra in 1usize..32) {
        let s = "a".repeat(MAX_TENANT_ID_LEN + extra);
        let header = HeaderValue::from_str(&s).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let status = rt.block_on(try_through_layer(header));
        prop_assert_eq!(status, StatusCode::NO_CONTENT);
    }
}
