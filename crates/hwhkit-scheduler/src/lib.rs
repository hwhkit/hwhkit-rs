//! Persistent background scheduler for hwhkit services.
//!
//! Provides cron + one-shot scheduling with durable storage so jobs
//! survive process restarts. Designed to run on multiple nodes with
//! mutual exclusion (via Postgres `SELECT ... FOR UPDATE SKIP LOCKED` or
//! the optional Redis leadership lock under the `redis-leader` feature).
//!
//! # Quick start
//!
//! ```ignore
//! use hwhkit_scheduler::{Scheduler, JobSpec, Schedule};
//! use serde::{Serialize, Deserialize};
//! use chrono::Utc;
//!
//! #[derive(Serialize, Deserialize, Clone)]
//! enum MyJob { SyncMarket, RebuildIndex }
//!
//! let store = hwhkit_scheduler::storage::PostgresJobStore::new(pool.clone());
//! let scheduler = Scheduler::<MyJob, _>::new(store, |job| async move {
//!     match job {
//!         MyJob::SyncMarket => do_sync().await,
//!         MyJob::RebuildIndex => do_rebuild().await,
//!     }
//! });
//! scheduler.schedule(JobSpec::new(MyJob::SyncMarket, Schedule::Cron("0 */1 * * *".into()))).await?;
//! scheduler.run(shutdown).await;
//! ```
//!
//! # Storage backends
//!
//! - [`storage::PostgresJobStore`] (default, behind `postgres-store`):
//!   single-node-safe via `SELECT FOR UPDATE SKIP LOCKED` claim semantics.
//! - [`storage::InMemoryJobStore`]: zero-dependency fallback for local
//!   development and tests; jobs do **not** survive restart.
//!
//! Both implement [`storage::JobStore`].

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use hwhkit_core::ShutdownToken;
use parking_lot::RwLock;
use serde::{de::DeserializeOwned, Serialize};
use tracing::Instrument;
use uuid::Uuid;

pub mod cron;
pub mod storage;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("invalid cron expression: {0}")]
    Cron(String),
    #[error("serialization error: {0}")]
    Ser(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// How a job recurs (or doesn't).
///
/// **Cron parser caveat:** the parser accepts cron specs leniently —
/// out-of-range numeric tokens are silently truncated to the nearest
/// in-range value rather than rejected. Validate inputs (or pre-parse via
/// [`cron::parse`]) when you need strict checking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Schedule {
    /// One-shot job that fires at the given UTC time.
    Once(DateTime<Utc>),
    /// Recurring schedule expressed as a 5-field cron string
    /// (`min hour dom month dow`). Supports `*`, `*/N`, comma lists, and
    /// numeric ranges. See [`cron`] for the day-of-month / day-of-week
    /// (Vixie OR) semantics.
    Cron(String),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct JobSpec<K> {
    pub kind: K,
    pub schedule: Schedule,
    /// Optional human-readable name. Defaults to `K`'s Debug repr.
    pub name: Option<String>,
}

impl<K> JobSpec<K> {
    pub fn new(kind: K, schedule: Schedule) -> Self {
        Self {
            kind,
            schedule,
            name: None,
        }
    }
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// A persisted job record.
///
/// Stored verbatim by [`storage::JobStore`] implementations and
/// reconstructed on read. Marked `#[non_exhaustive]` so future fields
/// (e.g. priority, retry counters) can be added in a minor release —
/// construct fresh instances via [`StoredJob::new`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct StoredJob {
    /// Stable identifier (UUIDv7 for natural time-ordering).
    pub id: Uuid,
    /// Serialized application-defined `Kind` payload.
    pub kind_json: serde_json::Value,
    /// Recurrence policy — see [`Schedule`].
    pub schedule: Schedule,
    /// Optional human-readable label, used for log spans.
    pub name: Option<String>,
    /// UTC instant of the next firing.
    pub next_run_at: DateTime<Utc>,
    /// UTC instant of the most recent successful run, if any.
    pub last_run_at: Option<DateTime<Utc>>,
    /// UTC instant the job was first inserted.
    pub created_at: DateTime<Utc>,
}

impl StoredJob {
    /// Construct a `StoredJob` from its component parts. Prefer this over
    /// struct-literal syntax — `StoredJob` is `#[non_exhaustive]` so
    /// future fields wouldn't be reachable from outside the crate
    /// otherwise.
    ///
    /// `last_run_at` and `name` start as `None` and are populated by the
    /// scheduler / store.
    pub fn new(
        id: Uuid,
        kind_json: serde_json::Value,
        schedule: Schedule,
        name: Option<String>,
        next_run_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            kind_json,
            schedule,
            name,
            next_run_at,
            last_run_at: None,
            created_at,
        }
    }
}

/// Type-erased async job handler. Receives the deserialized job kind.
pub type Runner<K> =
    Arc<dyn Fn(K) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static>;

/// Default lease TTL applied to claimed jobs. After this many seconds
/// without a `complete()` call the job is presumed orphaned and is
/// requeued on the next `requeue_stale` sweep.
const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(60);

/// Tickable scheduler.
pub struct Scheduler<K>
where
    K: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    store: Arc<dyn storage::JobStore + Send + Sync>,
    runner: Runner<K>,
    poll_interval: Duration,
    lease_ttl: Duration,
    worker_id: Uuid,
    /// Cache of parsed [`cron::CronSpec`] keyed by expression string. The
    /// scheduler tick re-evaluates the same handful of cron strings every
    /// poll; reparsing each one allocates a fresh `Vec<u32>` × 5 — this
    /// cache amortises that to one parse per unique expression.
    cron_cache: Arc<RwLock<HashMap<String, cron::CronSpec>>>,
}

impl<K> Scheduler<K>
where
    K: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    pub fn new<S, F, Fut>(store: S, runner: F) -> Self
    where
        S: storage::JobStore + Send + Sync + 'static,
        F: Fn(K) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self::from_arc(Arc::new(store), runner)
    }

    /// Construct a scheduler from an already-wrapped `Arc<dyn JobStore>`.
    /// Useful when callers want to share the store with other components.
    pub fn from_arc<F, Fut>(store: Arc<dyn storage::JobStore + Send + Sync>, runner: F) -> Self
    where
        F: Fn(K) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let runner: Runner<K> = Arc::new(move |k| Box::pin(runner(k)));
        Self {
            store,
            runner,
            poll_interval: Duration::from_secs(2),
            lease_ttl: DEFAULT_LEASE_TTL,
            worker_id: Uuid::now_v7(),
            cron_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    #[must_use]
    pub fn with_poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }

    /// Override the per-claim lease TTL. Jobs whose claim is older than
    /// this without a [`storage::JobStore::complete`] call are requeued
    /// by the scheduler's reaper sweep. Default 60s.
    #[must_use]
    pub fn with_lease_ttl(mut self, d: Duration) -> Self {
        self.lease_ttl = d;
        self
    }

    /// Override the worker id used to stamp claims (default: random
    /// `Uuid::now_v7()` per scheduler instance). Useful for tests that
    /// want a deterministic id, or for binding the worker id to a stable
    /// node identifier.
    #[must_use]
    pub fn with_worker_id(mut self, id: Uuid) -> Self {
        self.worker_id = id;
        self
    }

    /// Persist a new job. Returns the assigned id.
    pub async fn schedule(&self, spec: JobSpec<K>) -> Result<Uuid> {
        let now = Utc::now();
        let next = match &spec.schedule {
            Schedule::Once(t) => *t,
            Schedule::Cron(expr) => self.next_cron(expr, now)?,
        };
        let kind_json = serde_json::to_value(&spec.kind).map_err(|e| Error::Ser(e.to_string()))?;
        let job = StoredJob {
            id: Uuid::now_v7(),
            kind_json,
            schedule: spec.schedule,
            name: spec.name,
            next_run_at: next,
            last_run_at: None,
            created_at: now,
        };
        self.store.insert(job.clone()).await?;
        Ok(job.id)
    }

    /// Drive the scheduler loop until the shutdown token fires. Runs two
    /// timers concurrently:
    ///   - **claim tick** (every `poll_interval`): pulls due jobs and
    ///     spawns runners.
    ///   - **reaper tick** (every `lease_ttl / 2`): sweeps abandoned
    ///     leases and requeues them so dead workers don't take their
    ///     pending work to the grave.
    pub async fn run(self, shutdown: ShutdownToken) {
        let reaper_interval = (self.lease_ttl / 2).max(Duration::from_secs(1));
        let mut last_reap = std::time::Instant::now();

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("scheduler stopping");
                    break;
                }
                _ = tokio::time::sleep(self.poll_interval) => {}
            }

            // Reaper tick: rescue stale leases.
            if last_reap.elapsed() >= reaper_interval {
                last_reap = std::time::Instant::now();
                match self.store.requeue_stale(Utc::now(), self.lease_ttl).await {
                    Ok(n) if n > 0 => {
                        tracing::warn!(requeued = n, "scheduler: requeued stale jobs");
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(error = %err, "scheduler: requeue_stale failed");
                    }
                }
            }

            let now = Utc::now();
            let claimed = match self.store.claim_due(self.worker_id, now, 16).await {
                Ok(jobs) => jobs,
                Err(err) => {
                    tracing::warn!(error = %err, "claim_due failed");
                    continue;
                }
            };

            for job in claimed {
                let runner = self.runner.clone();
                let store = self.store.clone();
                let cron_cache = self.cron_cache.clone();
                // Span per spawned job so traces propagate from the
                // scheduler tick into the runner. (N15.)
                let span = tracing::info_span!(
                    "scheduler.job",
                    job_id = %job.id,
                    job_name = job.name.as_deref().unwrap_or("<unnamed>"),
                    worker_id = %self.worker_id,
                );
                tokio::spawn(
                    async move {
                        match serde_json::from_value::<K>(job.kind_json.clone()) {
                            Ok(k) => {
                                runner(k).await;
                                // Compute next run + persist completion.
                                let next = match &job.schedule {
                                    Schedule::Once(_) => None,
                                    Schedule::Cron(expr) => {
                                        next_cron_cached(&cron_cache, expr, Utc::now()).ok()
                                    }
                                };
                                if let Err(err) = store.complete(job.id, Utc::now(), next).await {
                                    tracing::warn!(error = %err, "scheduler: complete() failed");
                                }
                            }
                            Err(err) => {
                                // The persisted Kind is no longer
                                // deserializable (likely a Kind variant
                                // was removed/renamed in a deploy). Move
                                // it out of the queue so the lease isn't
                                // wasted; treat it as terminal — for
                                // recurring jobs this drops the next-run
                                // slot but the operator will see the
                                // warning and re-create the schedule.
                                tracing::warn!(
                                    job = %job.id,
                                    error = %err,
                                    "scheduler: failed to deserialize job kind; \
                                     marking complete to release lease"
                                );
                                if let Err(err) = store.complete(job.id, Utc::now(), None).await {
                                    tracing::warn!(error = %err, "scheduler: complete() failed");
                                }
                            }
                        }
                    }
                    .instrument(span),
                );
            }
        }
    }

    /// Compute the next firing of a cron expression, using the per-scheduler
    /// parse cache so we re-parse each unique expression at most once.
    fn next_cron(&self, expr: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        next_cron_cached(&self.cron_cache, expr, now)
    }
}

fn next_cron_cached(
    cache: &Arc<RwLock<HashMap<String, cron::CronSpec>>>,
    expr: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    if let Some(spec) = cache.read().get(expr).cloned() {
        return cron::next_after_spec(&spec, now, expr);
    }
    let spec = cron::parse(expr)?;
    cache.write().insert(expr.to_string(), spec.clone());
    cron::next_after_spec(&spec, now, expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    enum NewKind {
        Foo,
    }

    /// A persisted job whose Kind enum has changed (`OldKind` → `NewKind`)
    /// must be marked complete after the deserialize failure rather than
    /// left in claimed state until the lease expires.
    #[tokio::test]
    async fn deserialize_failure_marks_complete() {
        use crate::storage::{InMemoryJobStore, JobStore};

        let store = InMemoryJobStore::new();
        let bad = StoredJob {
            id: Uuid::now_v7(),
            kind_json: serde_json::json!({"DefinitelyNotInNewKind": null}),
            schedule: Schedule::Once(Utc::now() - chrono::Duration::seconds(1)),
            name: Some("legacy".into()),
            next_run_at: Utc::now() - chrono::Duration::seconds(1),
            last_run_at: None,
            created_at: Utc::now(),
        };
        store.insert(bad.clone()).await.unwrap();

        let scheduler = Scheduler::<NewKind>::new(store.clone(), |_| async {})
            .with_poll_interval(Duration::from_millis(50));
        // Drive the scheduler for a few ticks then shut down.
        let shutdown = ShutdownToken::new();
        let shutdown2 = shutdown.clone();
        let h = tokio::spawn(async move { scheduler.run(shutdown2).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        shutdown.cancel();
        let _ = h.await;

        // The store should have either dropped the job (Once + complete-with-None)
        // or at least not be sitting on a perpetually-claimed entry.
        let worker = Uuid::now_v7();
        let claimed = store
            .claim_due(worker, Utc::now() + chrono::Duration::days(2), 10)
            .await
            .unwrap();
        assert!(
            claimed.is_empty(),
            "deserialize-failed job should have been completed, not left claimed"
        );
    }
}
