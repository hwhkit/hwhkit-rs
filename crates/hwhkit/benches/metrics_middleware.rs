//! Bench just the HTTP RED metrics middleware in isolation. Goal: confirm
//! the F22/M1 fix that pre-canonicalises the method label to `&'static str`
//! and only allocates two `String`s per request really did reduce
//! allocation pressure (the bench is a measurement vehicle — there's no
//! assertion).

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use axum::Router;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use hwhkit::production::metrics::HttpMetricsLayer;
use tower::{Service, ServiceExt};

fn bench_metrics_layer(c: &mut Criterion) {
    let app: Router = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(HttpMetricsLayer::new());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("metrics_layer/single_request", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut svc = app.clone();
                let resp = ServiceExt::<Request<Body>>::ready(&mut svc)
                    .await
                    .unwrap()
                    .call(Request::builder().uri("/ping").body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                black_box(resp.status());
            });
        });
    });
}

fn bench_raw_for_comparison(c: &mut Criterion) {
    let app: Router = Router::new().route("/ping", get(|| async { "pong" }));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("metrics_layer/raw_baseline", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut svc = app.clone();
                let resp = ServiceExt::<Request<Body>>::ready(&mut svc)
                    .await
                    .unwrap()
                    .call(Request::builder().uri("/ping").body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                black_box(resp.status());
            });
        });
    });
}

criterion_group!(benches, bench_metrics_layer, bench_raw_for_comparison);
criterion_main!(benches);
