//! Graceful-shutdown coordination.
//!
//! [`ShutdownToken`] is a thin wrapper around `tokio_util::sync::CancellationToken`
//! that is inserted into [`crate::AppContext`] during bootstrap. Background
//! tasks should call `token.cancelled().await` (or `is_cancelled()`) to
//! observe shutdown intent so they can drain cleanly.

use tokio_util::sync::CancellationToken;

/// Cheap-to-clone shutdown signal. Cloning shares the same underlying
/// cancellation state.
#[derive(Clone, Debug, Default)]
pub struct ShutdownToken {
    inner: CancellationToken,
}

impl ShutdownToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.inner.clone()
    }

    /// Trigger shutdown for everyone observing this token.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    /// Resolve once shutdown has been requested.
    pub async fn cancelled(&self) {
        self.inner.cancelled().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn cancellation_propagates_to_clones() {
        let token = ShutdownToken::new();
        let observed = Arc::new(AtomicBool::new(false));

        let observer = token.clone();
        let observed_inner = observed.clone();
        let join = tokio::spawn(async move {
            observer.cancelled().await;
            observed_inner.store(true, Ordering::SeqCst);
        });

        token.cancel();
        join.await.unwrap();
        assert!(observed.load(Ordering::SeqCst));
        assert!(token.is_cancelled());
    }
}
