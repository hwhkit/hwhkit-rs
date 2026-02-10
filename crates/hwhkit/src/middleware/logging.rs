//! 日志中间件模块

use axum::{extract::Request, middleware::Next, response::Response};
use std::time::Instant;

/// 请求日志中间件
pub async fn request_logging_middleware(request: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let user_agent = request
        .headers()
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("-");

    tracing::info!(
        method = %method,
        uri = %uri,
        user_agent = %user_agent,
        "请求开始"
    );

    let response = next.run(request).await;

    let duration = start.elapsed();
    let status = response.status();

    tracing::info!(
        method = %method,
        uri = %uri,
        status = %status,
        duration = ?duration,
        "请求完成"
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
        middleware,
        response::IntoResponse,
        routing::get,
        Router,
    };
    use tower::util::ServiceExt;

    async fn test_handler() -> impl IntoResponse {
        (StatusCode::OK, "Hello, World!")
    }

    #[tokio::test]
    async fn test_request_logging() {
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn(request_logging_middleware));

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("user-agent", "test-agent")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
