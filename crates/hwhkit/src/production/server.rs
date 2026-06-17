//! HTTP server runner that wires together every Tier-1 production
//! capability:
//!
//! - mounts `/health`, `/health/ready`, `/metrics`, `/version`, `/info`
//! - applies the standard middleware bundle (tracing, CORS, compression,
//!   timeouts, body-limit, panic catcher, sensitive-header redaction)
//! - injects a request-id middleware
//! - serves the resulting router with `axum::serve` + graceful shutdown
//!   driven by the [`hwhkit_core::ShutdownToken`] in `AppContext`
//!
//! Users who want a richer runtime (multi-listener, HTTPS, custom
//! shutdown order) can call [`hwhkit_core::BuiltApplication::router`] /
//! [`hwhkit_core::BuiltApplication::shutdown`] and drive `axum::serve`
//! themselves.

use std::net::{AddrParseError, SocketAddr};
#[cfg(feature = "metrics")]
use std::sync::Arc;
#[cfg(feature = "graceful-shutdown")]
use std::time::Duration;

use axum::Router;
use hwhkit_core::BuiltApplication;
use tokio::net::TcpListener;

use super::*;

/// Errors emitted by [`run`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServeError {
    #[error("invalid server.host/port: {0}")]
    InvalidAddr(#[from] AddrParseError),
    #[error("bind {addr} failed")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("axum serve error")]
    Serve(#[source] std::io::Error),
}

/// Run a [`BuiltApplication`] with the OOTB production runtime. Binds
/// the listener using `server.host`/`server.port` from config. Returns
/// when the server has fully drained or `max_drain_secs` elapses.
pub async fn run(built: BuiltApplication) -> Result<(), ServeError> {
    let cfg = built.config().clone();
    let addr: SocketAddr = format!("{}:{}", cfg.server.host, cfg.server.port).parse()?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| ServeError::Bind { addr, source: e })?;
    tracing::info!(%addr, "hwhkit server listening");
    run_with_listener(built, listener).await
}

/// Run a [`BuiltApplication`] on a pre-bound [`TcpListener`].
///
/// This is the lower-level entry point: it mounts the OOTB production
/// endpoints + middleware bundle, installs SIGINT/SIGTERM handlers, and
/// serves until shutdown completes — but uses the listener you pass
/// instead of binding from config.
///
/// Useful for:
///
/// - **Tests** that need an ephemeral port
///   (`TcpListener::bind("127.0.0.1:0")`).
/// - **systemd socket activation** / **inherited fds** where the
///   listener is owned by the supervisor, not the application.
/// - **Multi-listener** setups where the caller binds multiple
///   addresses and decides which one this `BuiltApplication` services.
pub async fn run_with_listener(
    mut built: BuiltApplication,
    listener: TcpListener,
) -> Result<(), ServeError> {
    let cfg = built.config().clone();
    let mut router = built.router().clone();

    // Mount health/version/metrics first so they sit alongside the user
    // routes inside the standard middleware bundle.
    #[cfg(feature = "health-endpoints")]
    if cfg.runtime.health.enabled {
        router = router.merge(health::router(&cfg.runtime.health, built.health()));
    }

    #[cfg(feature = "version-endpoints")]
    if cfg.runtime.info.enabled {
        let version = version::default_version();
        let info = version::InfoResponse {
            service_name: cfg.observability.service_name.clone(),
            environment: format!("{:?}", cfg.observability.environment).to_lowercase(),
            build: hwhkit_buildinfo::current(),
            initialized_integrations: built.initialized_integrations().to_vec(),
            degraded_integrations: built.degraded_integrations().to_vec(),
        };
        router = router.merge(version::router(&cfg.runtime.info, info, version));
    }

    #[cfg(feature = "metrics")]
    if cfg.runtime.metrics.enabled {
        match metrics::install_recorder() {
            Ok(handle) => {
                router = router.merge(metrics::router(&cfg.runtime.metrics, handle.clone()));
                router = router.layer(metrics::HttpMetricsLayer::new());
                // Park the handle on `BuiltApplication` so its lifetime
                // matches the surrounding application, not just the
                // local scope of this function.
                built.set_metrics_handle(Arc::new(handle));
            }
            Err(err) => {
                tracing::warn!(error = %err, "metrics recorder install failed");
            }
        }
    }

    // Spawn the process-metrics sampler once a recorder is installed.
    #[cfg(feature = "process-metrics")]
    {
        if cfg.runtime.metrics.enabled {
            process_metrics::spawn(built.shutdown());
        }
    }

    #[cfg(feature = "request-id")]
    if cfg.runtime.request_id.enabled {
        router = router.layer(request_id::RequestIdLayer::new(
            &cfg.runtime.request_id.header,
        ));
    }

    #[cfg(feature = "middleware-bundle")]
    {
        router = middleware::apply(router, &cfg.runtime.middleware);
    }

    let serve_result = serve(listener, router, &built).await;

    // Drain providers in reverse-init order so consumers shut down
    // before producers (e.g. the HTTP handler is gone before we close
    // the database pool it was reading from).
    drain_providers(&built).await;

    serve_result
}

async fn drain_providers(built: &BuiltApplication) {
    for provider in built.providers().iter().rev() {
        let key = provider.key();
        match provider.shutdown(built.context()).await {
            Ok(_) => tracing::info!(integration = %key, "shutdown ok"),
            Err(err) => tracing::warn!(integration = %key, error = %err, "shutdown error"),
        }
    }
}

#[cfg(feature = "graceful-shutdown")]
async fn serve(
    listener: TcpListener,
    router: Router,
    built: &BuiltApplication,
) -> Result<(), ServeError> {
    let shutdown = built.shutdown();
    shutdown::install(shutdown.clone());

    let drain = Duration::from_secs(built.config().runtime.shutdown.max_drain_secs);

    // `with_graceful_shutdown` stops accepting new connections the moment its
    // future resolves (on cancellation), then lets inflight requests finish.
    //
    // The `drain` timeout must bound ONLY the post-cancellation inflight drain
    // — NOT the whole serve. Wrapping the entire `serve_fut` in
    // `timeout(drain, …)` (as a previous version did) caps total server uptime
    // to `max_drain_secs`: with no SIGTERM the future never resolves, the
    // timeout fires after `drain`, and the process exits — a self-restart loop.
    //
    // So: serve until shutdown is requested, and only THEN apply the timeout.
    let trigger = shutdown.clone();
    // `WithGracefulShutdown` is `IntoFuture`, not `Future` — materialise the
    // future so we can poll it by reference under `select!` / `timeout`.
    let serve_fut = std::future::IntoFuture::into_future(
        axum::serve(listener, router).with_graceful_shutdown(async move {
            trigger.cancelled().await;
        }),
    );
    tokio::pin!(serve_fut);

    tokio::select! {
        // Server ended on its own (bind/accept error, or drained with no signal).
        res = &mut serve_fut => res.map_err(ServeError::Serve),
        // Shutdown requested: stop accepting, give inflight up to `drain`, then force.
        _ = shutdown.cancelled() => {
            tracing::info!(?drain, "shutdown signalled; bounding inflight drain");
            match tokio::time::timeout(drain, &mut serve_fut).await {
                Ok(res) => res.map_err(ServeError::Serve),
                Err(_) => {
                    tracing::warn!(?drain, "drain deadline elapsed; forcing shutdown");
                    Ok(())
                }
            }
        }
    }
}

#[cfg(not(feature = "graceful-shutdown"))]
async fn serve(
    listener: TcpListener,
    router: Router,
    _built: &BuiltApplication,
) -> Result<(), ServeError> {
    axum::serve(listener, router)
        .await
        .map_err(ServeError::Serve)
}
