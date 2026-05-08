//! Standard tower middleware bundle applied automatically by
//! [`crate::run_v2`]: tracing/spans, CORS, gzip+br compression,
//! request timeout, body size limit, panic catcher (returns
//! `application/problem+json`), and sensitive-header redaction.

use std::time::Duration;

use axum::body::Body;
use axum::http::{header::AUTHORIZATION, HeaderName, HeaderValue, Method, Response, StatusCode};
use axum::Router;
use hwhkit_config::MiddlewareConfig;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::sensitive_headers::SetSensitiveHeadersLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

/// Apply the standard middleware bundle to a [`Router`]. Order matters
/// (outermost first): tracing/spans → request timeout → body limit →
/// compression → CORS → catch-panic → sensitive header redaction.
pub fn apply(router: Router, cfg: &MiddlewareConfig) -> Router {
    if !cfg.enabled {
        return router;
    }

    let cors = if cfg.cors.enabled {
        let mut layer = CorsLayer::new()
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_credentials(cfg.cors.allow_credentials);

        if cfg.cors.allow_origins.iter().any(|o| o == "*") {
            layer = layer.allow_origin(tower_http::cors::Any);
        } else {
            let origins: Vec<HeaderValue> = cfg
                .cors
                .allow_origins
                .iter()
                .filter_map(|o| HeaderValue::from_str(o).ok())
                .collect();
            layer = layer.allow_origin(AllowOrigin::list(origins));
        }
        Some(layer)
    } else {
        None
    };

    let mut router = router
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(Duration::from_secs(cfg.timeout_secs)))
        .layer(RequestBodyLimitLayer::new(cfg.body_limit_bytes))
        .layer(SetSensitiveHeadersLayer::new(std::iter::once(
            AUTHORIZATION,
        )));

    if cfg.compression {
        router = router.layer(CompressionLayer::new().gzip(true).br(true));
    }
    if let Some(cors_layer) = cors {
        router = router.layer(cors_layer);
    }
    if cfg.catch_panic {
        router = router.layer(CatchPanicLayer::custom(panic_handler));
    }

    router
}

/// Compose individual layers as a [`ServiceBuilder`] for users who want
/// to attach the bundle to a custom router.
pub fn standard_middleware_layer(
    cfg: &MiddlewareConfig,
) -> ServiceBuilder<
    tower::layer::util::Stack<
        TraceLayer<
            tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
        >,
        tower::layer::util::Identity,
    >,
> {
    let _ = cfg; // reserved for future tuning
    ServiceBuilder::new().layer(TraceLayer::new_for_http())
}

fn panic_handler(err: Box<dyn std::any::Any + Send + 'static>) -> Response<Body> {
    // Try the two stdlib payload types first (panic!("...")). Any other
    // payload type loses its content in the conversion to `Any`, so we
    // emit a structured log line at error level so operators can still
    // correlate the panic with stack traces from the panic hook.
    let detail = if let Some(s) = err.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else {
        let payload_type = std::any::type_name_of_val(&*err);
        tracing::error!(
            payload_type = %payload_type,
            "panic with non-string payload caught by middleware"
        );
        "internal server error".to_string()
    };

    let body = serde_json::json!({
        "type": "about:blank",
        "title": "Internal Server Error",
        "status": 500,
        "detail": format!("panic: {detail}")
    });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/problem+json"),
        )
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}
