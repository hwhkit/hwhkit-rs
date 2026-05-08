//! Benchmarks for the production middleware stack.
//!
//! Each bench drives a cold/empty handler through a `tower::Service::call`
//! one request at a time. We measure:
//!
//! - the raw axum router (no extra middleware) as a baseline
//! - request-id alone
//! - request-id + metrics
//!
//! This is a *relative* comparison; the absolute numbers depend heavily
//! on the host. The goal is to detect regressions in per-middleware cost.

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use axum::Router;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tower::{Service, ServiceExt};

#[cfg(all(feature = "request-id", not(feature = "metrics")))]
use hwhkit::production::request_id::RequestIdLayer;
#[cfg(all(feature = "request-id", feature = "metrics"))]
use hwhkit::production::{metrics::HttpMetricsLayer, request_id::RequestIdLayer};

fn make_request() -> Request<Body> {
    Request::builder().uri("/ping").body(Body::empty()).unwrap()
}

fn bench_raw(c: &mut Criterion) {
    let app: Router = Router::new().route("/ping", get(|| async { "pong" }));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    c.bench_function("middleware/raw", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut svc = app.clone();
                let resp = ServiceExt::<Request<Body>>::ready(&mut svc)
                    .await
                    .unwrap()
                    .call(make_request())
                    .await
                    .unwrap();
                black_box(resp.status());
            });
        });
    });
}

#[cfg(feature = "request-id")]
fn bench_with_request_id(c: &mut Criterion) {
    let app: Router = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(RequestIdLayer::default());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    c.bench_function("middleware/request_id", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut svc = app.clone();
                let resp = ServiceExt::<Request<Body>>::ready(&mut svc)
                    .await
                    .unwrap()
                    .call(make_request())
                    .await
                    .unwrap();
                black_box(resp.status());
            });
        });
    });
}

#[cfg(all(feature = "request-id", feature = "metrics"))]
fn bench_with_request_id_plus_metrics(c: &mut Criterion) {
    let app: Router = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(HttpMetricsLayer::new())
        .layer(RequestIdLayer::default());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    c.bench_function("middleware/request_id_plus_metrics", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut svc = app.clone();
                let resp = ServiceExt::<Request<Body>>::ready(&mut svc)
                    .await
                    .unwrap()
                    .call(make_request())
                    .await
                    .unwrap();
                black_box(resp.status());
            });
        });
    });
}

#[cfg(not(feature = "request-id"))]
fn bench_with_request_id(_: &mut Criterion) {}

#[cfg(not(all(feature = "request-id", feature = "metrics")))]
fn bench_with_request_id_plus_metrics(_: &mut Criterion) {}

criterion_group!(
    benches,
    bench_raw,
    bench_with_request_id,
    bench_with_request_id_plus_metrics,
);
criterion_main!(benches);
