//! Health-check registry consumed by the `/health/ready` endpoint.
//!
//! Each [`IntegrationProvider`] can register a [`HealthCheck`] during init.
//! At runtime, the readiness probe runs every check concurrently and reports
//! per-integration status. Required failures yield 503; optional failures
//! mark the service "degraded" but still return 200.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use tokio::time::timeout;

/// Outcome of a single health probe.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HealthStatus {
    Up,
    Down,
    Degraded,
}

/// Per-integration probe result.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct HealthCheckResult {
    pub name: String,
    pub status: HealthStatus,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub duration_ms: u128,
}

/// Asynchronous health check. Implementors should make the probe cheap
/// (a single round-trip is plenty) so it can run frequently from
/// orchestrators like Kubernetes.
///
/// **Project policy:** the trait is intentionally **open**. Future
/// methods must ship with default implementations so existing impls
/// keep compiling without churn.
#[async_trait]
pub trait HealthCheck: Send + Sync {
    fn name(&self) -> &str;
    fn required(&self) -> bool {
        true
    }
    /// Default timeout applied to a single probe (override per-check if
    /// you need a longer/shorter cap).
    fn timeout(&self) -> Duration {
        Duration::from_secs(2)
    }
    async fn check(&self) -> Result<(), String>;
}

/// Registry of [`HealthCheck`]s. Cloning is cheap (Arc-backed).
#[derive(Clone, Default)]
pub struct HealthRegistry {
    checks: Arc<parking_lot::Mutex<Vec<Arc<dyn HealthCheck>>>>,
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an owned [`HealthCheck`]. The registry takes ownership and
    /// wraps it in an `Arc` internally. Prefer this when the caller owns
    /// the value outright; use [`Self::register_arc`] when you already
    /// have an `Arc<dyn HealthCheck>` (e.g. a shared probe also stashed in
    /// [`crate::AppContext`]).
    pub fn register<C: HealthCheck + 'static>(&self, check: C) {
        self.checks.lock().push(Arc::new(check));
    }

    /// Register an already-shared [`HealthCheck`]. Use this when the same
    /// probe is held in multiple places (e.g. an integration that
    /// registers a probe and *also* stashes it in [`crate::AppContext`]
    /// for handlers to inspect).
    pub fn register_arc(&self, check: Arc<dyn HealthCheck>) {
        self.checks.lock().push(check);
    }

    pub fn snapshot(&self) -> Vec<Arc<dyn HealthCheck>> {
        self.checks.lock().clone()
    }

    /// Run all registered probes concurrently and return their results.
    pub async fn run_all(&self) -> Vec<HealthCheckResult> {
        use futures::future::join_all;

        let checks = self.snapshot();
        let futures = checks.into_iter().map(|c| async move {
            let started = std::time::Instant::now();
            let to = c.timeout();
            let outcome = timeout(to, c.check()).await;
            let duration_ms = started.elapsed().as_millis();

            let (status, message) = match outcome {
                Ok(Ok(())) => (HealthStatus::Up, None),
                Ok(Err(msg)) => (HealthStatus::Down, Some(msg)),
                Err(_) => (
                    HealthStatus::Down,
                    Some(format!("timeout after {}ms", to.as_millis())),
                ),
            };

            HealthCheckResult {
                name: c.name().to_string(),
                required: c.required(),
                status,
                message,
                duration_ms,
            }
        });

        join_all(futures).await
    }
}

impl std::fmt::Debug for HealthRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthRegistry")
            .field("count", &self.checks.lock().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysUp;

    #[async_trait]
    impl HealthCheck for AlwaysUp {
        fn name(&self) -> &str {
            "always_up"
        }
        async fn check(&self) -> Result<(), String> {
            Ok(())
        }
    }

    struct AlwaysDown;

    #[async_trait]
    impl HealthCheck for AlwaysDown {
        fn name(&self) -> &str {
            "always_down"
        }
        fn required(&self) -> bool {
            false
        }
        async fn check(&self) -> Result<(), String> {
            Err("nope".to_string())
        }
    }

    #[tokio::test]
    async fn registry_runs_concurrent_checks() {
        let reg = HealthRegistry::new();
        reg.register(AlwaysUp);
        reg.register(AlwaysDown);
        let results = reg.run_all().await;
        assert_eq!(results.len(), 2);
        let up = results.iter().find(|r| r.name == "always_up").unwrap();
        assert_eq!(up.status, HealthStatus::Up);
        let down = results.iter().find(|r| r.name == "always_down").unwrap();
        assert_eq!(down.status, HealthStatus::Down);
        assert!(!down.required);
    }
}
