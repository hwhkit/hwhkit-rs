//! Graceful shutdown signal handling. Installs SIGTERM (Unix) and
//! Ctrl-C handlers that fire the [`ShutdownToken`] which is shared
//! through [`hwhkit_core::AppContext`].

use hwhkit_core::ShutdownToken;

/// Install platform signal handlers and trip `token` when SIGINT or
/// SIGTERM arrives. Returns immediately; the signal listener runs in a
/// background tokio task that *also* observes the token's cancellation
/// so it cannot outlive the application — important when shutdown is
/// triggered programmatically (e.g. by tests or a second binary entry
/// point) instead of by an OS signal.
pub fn install(token: ShutdownToken) {
    let observe = token.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = wait_for_signal() => {
                tracing::info!("shutdown signal received; cancelling shutdown token");
                token.cancel();
            }
            _ = observe.cancelled() => {
                // Token was cancelled by other code; the task exits without
                // re-cancelling so we do not leak it for the lifetime of
                // the runtime.
            }
        }
    });
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            // Fall back to ctrl-c only.
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    let mut int = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
