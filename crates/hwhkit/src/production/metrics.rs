//! Prometheus `/metrics` endpoint plus a tower middleware that records
//! HTTP RED metrics (rate / errors / duration) keyed by route, method,
//! and status. The exporter handle is shared between the middleware and
//! the route so a single recorder serves both.

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::http::{Method, Request, Response};
use axum::response::IntoResponse;
use axum::{routing::get, Router};
use futures::future::BoxFuture;
use hwhkit_buildinfo::build_info;
use hwhkit_config::MetricsConfig;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct MetricsState {
    pub handle: PrometheusHandle,
}

/// Install a process-wide Prometheus recorder. Returns a handle that can
/// be cloned freely; the same handle is used by both the route and the
/// middleware. Calling more than once returns the new handle but retains
/// the previously installed recorder.
pub fn install_recorder() -> Result<PrometheusHandle, String> {
    let builder = PrometheusBuilder::new();
    let handle = builder
        .install_recorder()
        .map_err(|e| format!("failed to install prometheus recorder: {e}"))?;

    // Build-info gauge
    let info = build_info!();
    metrics::gauge!(
        "hwhkit_build_info",
        "git_sha" => info.git_sha,
        "rust_version" => info.rust_version,
        "version" => info.cargo_version
    )
    .set(1.0);

    Ok(handle)
}

async fn metrics_handler(state: axum::extract::State<Arc<MetricsState>>) -> impl IntoResponse {
    let body = state.handle.render();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
}

/// Router exposing the configured `/metrics` path.
pub fn router(cfg: &MetricsConfig, handle: PrometheusHandle) -> Router {
    let state = Arc::new(MetricsState { handle });
    Router::new()
        .route(&cfg.path, get(metrics_handler))
        .with_state(state)
}

/// HTTP RED middleware: records `http_requests_total` (counter) and
/// `http_request_duration_seconds` (histogram) labelled by method, path,
/// and status code.
#[derive(Clone, Default)]
pub struct HttpMetricsLayer;

impl HttpMetricsLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for HttpMetricsLayer {
    type Service = HttpMetrics<S>;
    fn layer(&self, inner: S) -> Self::Service {
        HttpMetrics { inner }
    }
}

#[derive(Clone)]
pub struct HttpMetrics<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for HttpMetrics<S>
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

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // Method label: pre-canonicalise to a `&'static str` for the
        // common verbs so we never allocate. Cardinality is bounded by
        // HTTP itself, so this matches operationally; uncommon methods
        // collapse to "OTHER" rather than emitting a per-method series.
        let method_label: &'static str = method_to_static(req.method());

        // Path label: `MatchedPath` is request-scoped, so we have to
        // build an owned `String` once per request — the histogram!/
        // counter! macros require a `'static`-or-owned label and the
        // path string can't outlive the request extensions.
        let path_owned: String = req
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|m| m.as_str().to_owned())
            .unwrap_or_else(|| String::from("unmatched"));
        let started = Instant::now();
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let response = inner.call(req).await?;
            let status_u16 = response.status().as_u16();
            let mut status_buf = itoa::Buffer::new();
            let status_owned: String = status_buf.format(status_u16).to_owned();
            let elapsed = started.elapsed().as_secs_f64();

            // We hold two owned `String`s (path + status) and clone each
            // once for the counter so the histogram can take ownership of
            // the originals — 4 String allocations total per request
            // (2 owned + 2 clones), down from 5 in the previous
            // implementation that went through an intermediate `Arc<str>`.
            metrics::counter!(
                "http_requests_total",
                "method" => method_label,
                "path" => path_owned.clone(),
                "status" => status_owned.clone(),
            )
            .increment(1);
            metrics::histogram!(
                "http_request_duration_seconds",
                "method" => method_label,
                "path" => path_owned,
                "status" => status_owned,
            )
            .record(elapsed);

            Ok(response)
        })
    }
}

/// Map a `Method` to a `&'static str` for the metrics label without
/// allocating. Uncommon methods are bucketed into `OTHER` to keep
/// cardinality bounded.
fn method_to_static(m: &Method) -> &'static str {
    match m.as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        "TRACE" => "TRACE",
        "CONNECT" => "CONNECT",
        _ => "OTHER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::{Service, ServiceExt};

    /// Drive the layered service through three back-to-back calls to
    /// confirm the `mem::replace` pattern keeps `poll_ready`-correct
    /// services in flight (no "service not ready" panic under repeated
    /// invocation). Regression for F9.
    #[tokio::test]
    async fn http_metrics_layer_handles_repeated_calls() {
        let app: Router = Router::new()
            .route("/ping", get(|| async { "pong" }))
            .layer(HttpMetricsLayer::new());

        for _ in 0..5 {
            let mut svc = app.clone();
            let response = ServiceExt::<Request<Body>>::ready(&mut svc)
                .await
                .expect("ready")
                .call(Request::builder().uri("/ping").body(Body::empty()).unwrap())
                .await
                .expect("call");
            assert_eq!(response.status(), StatusCode::OK);
        }
    }
}
