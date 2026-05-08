//! Request-ID middleware: read incoming `x-request-id` header (configurable)
//! or generate a UUID v7. Inject into the current tracing span and echo
//! back to the client in the response header.

use std::convert::Infallible;
use std::task::{Context, Poll};

use axum::http::header::HeaderName;
use axum::http::{HeaderValue, Request, Response};
use futures::future::BoxFuture;
use tower::{Layer, Service};
use uuid::Uuid;

const TRACING_FIELD: &str = "request_id";

#[derive(Clone)]
pub struct RequestIdLayer {
    header_name: HeaderName,
}

impl RequestIdLayer {
    pub fn new(header: &str) -> Self {
        let header_name = HeaderName::from_bytes(header.as_bytes())
            .unwrap_or_else(|_| HeaderName::from_static("x-request-id"));
        Self { header_name }
    }
}

impl Default for RequestIdLayer {
    fn default() -> Self {
        Self::new("x-request-id")
    }
}

impl<S> Layer<S> for RequestIdLayer {
    type Service = RequestIdService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RequestIdService {
            inner,
            header_name: self.header_name.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RequestIdService<S> {
    inner: S,
    header_name: HeaderName,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for RequestIdService<S>
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
        let header_name = self.header_name.clone();
        let id = req
            .headers()
            .get(&header_name)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::now_v7().to_string());

        // Stamp the inbound request so handlers can read it via the same header.
        if let Ok(value) = HeaderValue::from_str(&id) {
            req.headers_mut().insert(header_name.clone(), value);
        }

        // Record on the current tracing span so log lines pick it up.
        tracing::Span::current().record(TRACING_FIELD, tracing::field::display(&id));

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            let mut response = inner.call(req).await?;
            if let Ok(value) = HeaderValue::from_str(&id) {
                response.headers_mut().insert(header_name, value);
            }
            Ok(response)
        })
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

    /// Regression for F9: the layered service must remain ready across
    /// multiple invocations. Three sequential calls is enough to catch
    /// a poll_ready-after-call mistake under the typical tower runtime.
    #[tokio::test]
    async fn request_id_layer_handles_repeated_calls() {
        let app: Router = Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(RequestIdLayer::default());

        for _ in 0..3 {
            let mut svc = app.clone();
            let response = ServiceExt::<Request<Body>>::ready(&mut svc)
                .await
                .expect("ready")
                .call(Request::builder().uri("/x").body(Body::empty()).unwrap())
                .await
                .expect("call");
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.headers().get("x-request-id").is_some());
        }
    }
}
