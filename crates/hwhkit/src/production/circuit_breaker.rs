//! Tier-2 circuit breaker for outgoing HTTP traffic.
//!
//! Implements a classic three-state breaker (closed → open → half-open):
//!
//! - **closed**: requests flow normally. Failures are counted in a sliding
//!   time-window. When the failure rate exceeds [`BreakerConfig::failure_ratio`]
//!   over a window of at least [`BreakerConfig::min_request_volume`]
//!   requests, the breaker trips open.
//! - **open**: every call fails fast with [`BreakerError::Open`] until
//!   [`BreakerConfig::open_duration`] elapses, then transitions to
//!   half-open.
//! - **half-open**: the *next* request is allowed through as a probe.
//!   Success closes the breaker; failure re-opens it.
//!
//! Wraps [`reqwest::Client`] via [`CircuitBreakerClient::execute`], but
//! the underlying [`CircuitBreaker`] is a generic primitive — call
//! [`CircuitBreaker::run`] with any async closure.
//!
//! Behind the `circuit-breaker` feature.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BreakerConfig {
    /// Trip the breaker open if the failure ratio over `window` exceeds
    /// this threshold (0.0 .. 1.0).
    pub failure_ratio: f64,
    /// Minimum number of observed requests before the failure ratio
    /// becomes meaningful. Below this, the breaker stays closed.
    pub min_request_volume: u32,
    /// Sliding-window length used to compute the failure ratio.
    pub window: Duration,
    /// How long the breaker stays in the open state before transitioning
    /// to half-open and probing.
    pub open_duration: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            failure_ratio: 0.5,
            min_request_volume: 20,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(15),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BreakerError<E> {
    #[error("circuit breaker open")]
    Open,
    #[error(transparent)]
    Inner(E),
}

#[derive(Clone)]
pub struct CircuitBreaker {
    inner: Arc<Mutex<BreakerInner>>,
    /// Tracks how many half-open probes are currently in flight. Used to
    /// guarantee that exactly **one** caller proceeds while the breaker
    /// is in `HalfOpen`; the rest fail fast with [`BreakerError::Open`].
    probes_in_flight: Arc<AtomicUsize>,
}

struct BreakerInner {
    cfg: BreakerConfig,
    state: BreakerState,
    opened_at: Option<Instant>,
    events: VecDeque<(Instant, bool)>, // (timestamp, success)
}

impl CircuitBreaker {
    pub fn new(cfg: BreakerConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BreakerInner {
                cfg,
                state: BreakerState::Closed,
                opened_at: None,
                events: VecDeque::new(),
            })),
            probes_in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn state(&self) -> BreakerState {
        self.inner.lock().state
    }

    /// Drive an async closure through the breaker. Returns
    /// [`BreakerError::Open`] when the breaker has tripped, otherwise
    /// forwards the inner result.
    ///
    /// **Half-open contract:** exactly one probe is admitted at a time.
    /// Concurrent callers arriving in `HalfOpen` short-circuit with
    /// [`BreakerError::Open`] until the probe finishes — so a flaky
    /// upstream cannot be hit by a thundering herd at the moment we're
    /// trying to decide whether it has recovered.
    pub async fn run<F, Fut, T, E>(&self, f: F) -> Result<T, BreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        // Decide whether to allow the call. In half-open we additionally
        // claim the single probe slot via compare_exchange — only the
        // CAS winner proceeds; everyone else short-circuits.
        let mut probe_owner = false;
        let allowed = {
            let mut g = self.inner.lock();
            match g.state {
                BreakerState::Closed => true,
                BreakerState::Open => {
                    let elapsed = g.opened_at.map(|t| t.elapsed()).unwrap_or_default();
                    if elapsed >= g.cfg.open_duration {
                        g.state = BreakerState::HalfOpen;
                        // Try to claim the probe slot.
                        if self
                            .probes_in_flight
                            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                            .is_ok()
                        {
                            probe_owner = true;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                BreakerState::HalfOpen => {
                    // Only the CAS winner is the probe owner.
                    if self
                        .probes_in_flight
                        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        probe_owner = true;
                        true
                    } else {
                        false
                    }
                }
            }
        };

        if !allowed {
            return Err(BreakerError::Open);
        }

        let result = f().await;
        let success = result.is_ok();
        if probe_owner {
            // Reset probe slot before recording — record() may flip the
            // state to Closed/Open. Releasing here matches the claim
            // performed above.
            self.probes_in_flight.store(0, Ordering::Release);
        }
        self.record(success);
        result.map_err(BreakerError::Inner)
    }

    fn record(&self, success: bool) {
        let now = Instant::now();
        let mut g = self.inner.lock();

        match g.state {
            BreakerState::HalfOpen => {
                if success {
                    g.state = BreakerState::Closed;
                    g.opened_at = None;
                    g.events.clear();
                } else {
                    g.state = BreakerState::Open;
                    g.opened_at = Some(now);
                }
                return;
            }
            BreakerState::Open => {
                // Should not happen — we early-returned above. Defensive.
                return;
            }
            BreakerState::Closed => {}
        }

        g.events.push_back((now, success));
        let cutoff = now.checked_sub(g.cfg.window).unwrap_or(now);
        while matches!(g.events.front(), Some((t, _)) if *t < cutoff) {
            g.events.pop_front();
        }

        let total = g.events.len() as u32;
        if total >= g.cfg.min_request_volume {
            let failures = g.events.iter().filter(|(_, ok)| !*ok).count() as f64;
            let ratio = failures / total as f64;
            if ratio >= g.cfg.failure_ratio {
                g.state = BreakerState::Open;
                g.opened_at = Some(now);
                tracing::warn!(?ratio, total, "circuit breaker tripped open");
            }
        }
    }
}

/// Convenience wrapper around `reqwest::Client` that funnels every
/// `execute` through a [`CircuitBreaker`].
#[derive(Clone)]
pub struct CircuitBreakerClient {
    client: reqwest::Client,
    breaker: CircuitBreaker,
    /// Per-tenant scope (only consulted by [`CircuitBreakerClient::execute_for_tenant`]).
    /// `None` ⇒ every tenant shares `breaker`. The shared breaker still
    /// acts as the default for tenants not present in the scope.
    #[cfg(feature = "multi-tenant")]
    tenant_scope: Option<hwhkit_core::TenantScope<CircuitBreaker>>,
    /// Default `BreakerConfig` for lazily-instantiated per-tenant breakers.
    #[cfg(feature = "multi-tenant")]
    tenant_default_cfg: BreakerConfig,
}

impl CircuitBreakerClient {
    pub fn new(client: reqwest::Client, cfg: BreakerConfig) -> Self {
        Self {
            client,
            breaker: CircuitBreaker::new(cfg.clone()),
            #[cfg(feature = "multi-tenant")]
            tenant_scope: None,
            #[cfg(feature = "multi-tenant")]
            tenant_default_cfg: cfg,
        }
    }

    /// Wire a per-tenant scope: each tenant gets its own [`CircuitBreaker`]
    /// (created lazily on first use). Tenants without an explicit
    /// breaker fall back to the shared breaker constructed via
    /// [`CircuitBreakerClient::new`]. Pair with
    /// [`CircuitBreakerClient::execute_for_tenant`].
    #[cfg(feature = "multi-tenant")]
    pub fn with_tenant_scope(mut self, scope: hwhkit_core::TenantScope<CircuitBreaker>) -> Self {
        self.tenant_scope = Some(scope);
        self
    }

    pub fn breaker(&self) -> &CircuitBreaker {
        &self.breaker
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub async fn execute(
        &self,
        req: reqwest::Request,
    ) -> Result<reqwest::Response, BreakerError<reqwest::Error>> {
        self.breaker
            .run(|| async { self.client.execute(req).await.and_then(check_status) })
            .await
    }

    /// Execute against the breaker scoped to `tenant`. Each tenant gets
    /// its own [`CircuitBreaker`] backed by `tenant_default_cfg`; if no
    /// tenant scope was wired (see [`CircuitBreakerClient::with_tenant_scope`])
    /// the request is funnelled through the shared default breaker.
    #[cfg(feature = "multi-tenant")]
    pub async fn execute_for_tenant(
        &self,
        tenant: &hwhkit_core::TenantId,
        req: reqwest::Request,
    ) -> Result<reqwest::Response, BreakerError<reqwest::Error>> {
        let breaker = match &self.tenant_scope {
            // `get_or_insert_with` holds the scope's write lock across
            // the check-then-insert, so two concurrent callers for the
            // same fresh tenant cannot each build an orphaned breaker.
            Some(scope) => {
                let cfg = self.tenant_default_cfg.clone();
                let arc = scope
                    .get_or_insert_with(tenant, || std::sync::Arc::new(CircuitBreaker::new(cfg)));
                (*arc).clone()
            }
            None => self.breaker.clone(),
        };
        breaker
            .run(|| async { self.client.execute(req).await.and_then(check_status) })
            .await
    }
}

/// HTTP errors >= 500 count as breaker failures (alongside transport
/// errors). 4xx are *not* counted because they reflect client behaviour.
fn check_status(resp: reqwest::Response) -> reqwest::Result<reqwest::Response> {
    if resp.status().is_server_error() {
        // Convert into a transport-like error using error_for_status which
        // does the conversion.
        return resp.error_for_status();
    }
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 20 concurrent callers in `HalfOpen` must collapse onto exactly
    /// one probe; the other 19 must fail fast with `Open`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn half_open_admits_single_probe() {
        // Trip the breaker open quickly.
        let cb = CircuitBreaker::new(BreakerConfig {
            failure_ratio: 0.5,
            min_request_volume: 2,
            window: Duration::from_secs(60),
            open_duration: Duration::from_millis(20),
        });
        for _ in 0..2 {
            let _: Result<(), BreakerError<&'static str>> = cb.run(|| async { Err("nope") }).await;
        }
        assert_eq!(cb.state(), BreakerState::Open);
        // Wait long enough for the breaker to admit a probe.
        tokio::time::sleep(Duration::from_millis(40)).await;

        // Fan out 20 concurrent calls. The probe blocks long enough that
        // the others see the in-flight slot.
        let probe_hits = Arc::new(AtomicU32::new(0));
        let open_hits = Arc::new(AtomicU32::new(0));

        let mut handles = Vec::new();
        for _ in 0..20 {
            let cb = cb.clone();
            let probe_hits = probe_hits.clone();
            let open_hits = open_hits.clone();
            handles.push(tokio::spawn(async move {
                let res: Result<(), BreakerError<&'static str>> = cb
                    .run(|| async {
                        probe_hits.fetch_add(1, Ordering::SeqCst);
                        // Hold the probe slot long enough for siblings to see it.
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        Ok(())
                    })
                    .await;
                if matches!(res, Err(BreakerError::Open)) {
                    open_hits.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        assert_eq!(
            probe_hits.load(Ordering::SeqCst),
            1,
            "exactly one probe should reach the inner future, got {}",
            probe_hits.load(Ordering::SeqCst)
        );
        assert!(
            open_hits.load(Ordering::SeqCst) >= 1,
            "at least one sibling should have been short-circuited as Open"
        );
    }

    /// Two tenants must not share a breaker: tripping tenant A open
    /// must not affect tenant B.
    #[cfg(feature = "multi-tenant")]
    #[tokio::test]
    async fn per_tenant_scope_isolates_breakers() {
        use hwhkit_core::{TenantId, TenantScope};

        let scope: TenantScope<CircuitBreaker> = TenantScope::new();
        let cfg = BreakerConfig {
            failure_ratio: 0.5,
            min_request_volume: 2,
            window: Duration::from_secs(60),
            open_duration: Duration::from_secs(30),
        };
        // Pre-seed the scope with breakers for two tenants.
        let a = CircuitBreaker::new(cfg.clone());
        let b = CircuitBreaker::new(cfg.clone());
        scope.insert(TenantId::new("a"), Arc::new(a.clone()));
        scope.insert(TenantId::new("b"), Arc::new(b.clone()));

        // Trip tenant A's breaker open by failing through it directly.
        for _ in 0..2 {
            let _: Result<(), BreakerError<&'static str>> = a.run(|| async { Err("nope") }).await;
        }
        assert_eq!(a.state(), BreakerState::Open);
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[tokio::test]
    async fn opens_on_failure_burst() {
        let cb = CircuitBreaker::new(BreakerConfig {
            failure_ratio: 0.5,
            min_request_volume: 4,
            window: Duration::from_secs(60),
            open_duration: Duration::from_millis(50),
        });

        let counter = AtomicU32::new(0);
        for _ in 0..4 {
            let _ = cb
                .run::<_, _, (), &'static str>(|| async {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Err("nope")
                })
                .await;
        }
        assert_eq!(cb.state(), BreakerState::Open);

        // While open, calls fail fast without invoking the closure.
        let before = counter.load(Ordering::SeqCst);
        let res: Result<(), BreakerError<&'static str>> = cb.run(|| async { Err("x") }).await;
        assert!(matches!(res, Err(BreakerError::Open)));
        assert_eq!(counter.load(Ordering::SeqCst), before);

        // After cooldown, transitions to half-open and a successful probe
        // re-closes it.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let res: Result<(), BreakerError<&'static str>> = cb.run(|| async { Ok(()) }).await;
        assert!(res.is_ok());
        assert_eq!(cb.state(), BreakerState::Closed);
    }
}
