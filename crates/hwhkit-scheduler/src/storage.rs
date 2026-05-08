//! Pluggable storage layer for the scheduler.
//!
//! The default backend is [`PostgresJobStore`] (under `postgres-store`),
//! which uses `SELECT ... FOR UPDATE SKIP LOCKED` to provide multi-node
//! safe job claiming with a heartbeat-style **lease**: every claim writes
//! `claimed_at` + `worker_id`, and any job whose lease expires before the
//! claiming worker calls [`JobStore::complete`] is requeued by
//! [`JobStore::requeue_stale`]. [`InMemoryJobStore`] is a process-local
//! fallback useful for testing and single-node deployments.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::{Error, Result, StoredJob};

/// Pluggable durable store for [`StoredJob`]s.
///
/// **Project policy:** the trait is intentionally **open**. Future methods
/// must ship with default implementations so existing impls keep
/// compiling without churn. Wrap implementations in `Arc<dyn JobStore>`
/// at construction time so the same handle can be cloned cheaply across
/// async tasks.
#[async_trait]
pub trait JobStore: Send + Sync {
    async fn insert(&self, job: StoredJob) -> Result<()>;
    /// Atomically claim up to `max` due jobs. The store records
    /// `claimed_at = now()`, `worker_id = self.worker_id`, and increments
    /// `attempts`. Each returned job becomes the caller's responsibility
    /// to either complete (via [`Self::complete`]) or release back to the
    /// queue when execution finishes.
    async fn claim_due(
        &self,
        worker_id: Uuid,
        now: DateTime<Utc>,
        max: u32,
    ) -> Result<Vec<StoredJob>>;
    async fn complete(
        &self,
        id: Uuid,
        finished: DateTime<Utc>,
        next: Option<DateTime<Utc>>,
    ) -> Result<()>;
    /// Reset jobs whose lease (`claimed_at`) is older than
    /// `now - lease_ttl` AND aren't already complete (i.e., are still
    /// claimed by a presumably-dead worker). Returns the number of jobs
    /// requeued. This is the safety net that keeps stuck claims from
    /// monopolising a slot forever — the scheduler runtime calls it on
    /// a timer (every `lease_ttl / 2` by default).
    async fn requeue_stale(&self, now: DateTime<Utc>, lease_ttl: Duration) -> Result<u64>;
}

/// Process-local store suitable for tests and single-node deployments.
#[derive(Default, Clone)]
pub struct InMemoryJobStore {
    inner: Arc<Mutex<Vec<InMemoryJob>>>,
}

#[derive(Clone)]
struct InMemoryJob {
    job: StoredJob,
    claimed_at: Option<DateTime<Utc>>,
    worker_id: Option<Uuid>,
    attempts: i32,
}

impl InMemoryJobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl JobStore for InMemoryJobStore {
    async fn insert(&self, job: StoredJob) -> Result<()> {
        self.inner.lock().push(InMemoryJob {
            job,
            claimed_at: None,
            worker_id: None,
            attempts: 0,
        });
        Ok(())
    }

    async fn claim_due(
        &self,
        worker_id: Uuid,
        now: DateTime<Utc>,
        max: u32,
    ) -> Result<Vec<StoredJob>> {
        let mut g = self.inner.lock();
        let mut claimed = Vec::new();
        for j in g.iter_mut() {
            if claimed.len() as u32 >= max {
                break;
            }
            // Only claim jobs that are due AND not currently leased by
            // someone else. (`claimed_at` clearing is the responsibility
            // of `complete` or `requeue_stale`.)
            if j.job.next_run_at <= now && j.claimed_at.is_none() {
                j.claimed_at = Some(now);
                j.worker_id = Some(worker_id);
                j.attempts += 1;
                claimed.push(j.job.clone());
                // Push next_run_at into the future so we don't double-claim
                // before completion arrives.
                j.job.next_run_at = now + chrono::Duration::days(365);
            }
        }
        Ok(claimed)
    }

    async fn complete(
        &self,
        id: Uuid,
        finished: DateTime<Utc>,
        next: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let mut g = self.inner.lock();
        if let Some(next) = next {
            for j in g.iter_mut() {
                if j.job.id == id {
                    j.job.last_run_at = Some(finished);
                    j.job.next_run_at = next;
                    j.claimed_at = None;
                    j.worker_id = None;
                    return Ok(());
                }
            }
        } else {
            g.retain(|j| j.job.id != id);
        }
        Ok(())
    }

    async fn requeue_stale(&self, now: DateTime<Utc>, lease_ttl: Duration) -> Result<u64> {
        let mut g = self.inner.lock();
        let cutoff =
            now - chrono::Duration::from_std(lease_ttl).unwrap_or(chrono::Duration::seconds(60));
        let mut count = 0u64;
        for j in g.iter_mut() {
            if let Some(t) = j.claimed_at {
                if t < cutoff {
                    j.claimed_at = None;
                    j.worker_id = None;
                    // Move the next_run_at back to "now" so the requeued
                    // job gets reclaimed on the next scheduler tick.
                    j.job.next_run_at = now;
                    count += 1;
                }
            }
        }
        Ok(count)
    }
}

#[cfg(feature = "postgres-store")]
pub use postgres::PostgresJobStore;

#[cfg(feature = "postgres-store")]
mod postgres {
    use super::*;
    use sqlx::PgPool;

    /// Postgres-backed durable scheduler store.
    ///
    /// Schema (run once via [`PostgresJobStore::ensure_schema`]):
    ///
    /// ```sql
    /// CREATE TABLE IF NOT EXISTS hwhkit_jobs (
    ///     id UUID PRIMARY KEY,
    ///     kind_json JSONB NOT NULL,
    ///     schedule JSONB NOT NULL,
    ///     name TEXT,
    ///     next_run_at TIMESTAMPTZ NOT NULL,
    ///     last_run_at TIMESTAMPTZ,
    ///     created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ///     claimed_at TIMESTAMPTZ NULL,
    ///     worker_id UUID NULL,
    ///     attempts INT NOT NULL DEFAULT 0
    /// );
    /// CREATE INDEX IF NOT EXISTS hwhkit_jobs_next_run_idx
    ///     ON hwhkit_jobs (next_run_at);
    /// CREATE INDEX IF NOT EXISTS hwhkit_jobs_claimed_idx
    ///     ON hwhkit_jobs (claimed_at);
    /// ```
    ///
    /// `claim_due` uses `SELECT ... FOR UPDATE SKIP LOCKED` so multiple
    /// scheduler nodes never run the same job twice. `requeue_stale`
    /// rescues jobs whose `claimed_at` is older than the lease TTL — i.e.
    /// the worker that claimed them is presumed dead.
    #[derive(Clone)]
    pub struct PostgresJobStore {
        pool: PgPool,
        table: String,
    }

    impl PostgresJobStore {
        pub fn new(pool: PgPool) -> Self {
            Self {
                pool,
                table: "hwhkit_jobs".to_string(),
            }
        }

        pub fn with_table(mut self, table: impl Into<String>) -> Self {
            self.table = table.into();
            self
        }

        /// Apply the default schema (idempotent). Adds the lease columns
        /// (`claimed_at`, `worker_id`, `attempts`) to existing
        /// installations via `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`.
        pub async fn ensure_schema(&self) -> Result<()> {
            let sql = format!(
                "CREATE TABLE IF NOT EXISTS {0} (\n  id UUID PRIMARY KEY,\n  kind_json JSONB NOT NULL,\n  schedule JSONB NOT NULL,\n  name TEXT,\n  next_run_at TIMESTAMPTZ NOT NULL,\n  last_run_at TIMESTAMPTZ,\n  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),\n  claimed_at TIMESTAMPTZ NULL,\n  worker_id UUID NULL,\n  attempts INT NOT NULL DEFAULT 0\n);\nALTER TABLE {0} ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ NULL;\nALTER TABLE {0} ADD COLUMN IF NOT EXISTS worker_id UUID NULL;\nALTER TABLE {0} ADD COLUMN IF NOT EXISTS attempts INT NOT NULL DEFAULT 0;\nCREATE INDEX IF NOT EXISTS {0}_next_run_idx ON {0} (next_run_at);\nCREATE INDEX IF NOT EXISTS {0}_claimed_idx ON {0} (claimed_at);",
                self.table
            );
            sqlx::query(&sql)
                .execute(&self.pool)
                .await
                .map_err(|e| Error::Storage(e.to_string()))?;
            Ok(())
        }
    }

    #[async_trait]
    impl JobStore for PostgresJobStore {
        async fn insert(&self, job: StoredJob) -> Result<()> {
            let sql = format!(
                "INSERT INTO {} (id, kind_json, schedule, name, next_run_at, last_run_at, created_at)\n VALUES ($1, $2, $3, $4, $5, $6, $7)",
                self.table
            );
            let schedule_json =
                serde_json::to_value(&job.schedule).map_err(|e| Error::Ser(e.to_string()))?;
            sqlx::query(&sql)
                .bind(job.id)
                .bind(&job.kind_json)
                .bind(&schedule_json)
                .bind(job.name.as_ref())
                .bind(job.next_run_at)
                .bind(job.last_run_at)
                .bind(job.created_at)
                .execute(&self.pool)
                .await
                .map_err(|e| Error::Storage(e.to_string()))?;
            Ok(())
        }

        async fn claim_due(
            &self,
            worker_id: Uuid,
            now: DateTime<Utc>,
            max: u32,
        ) -> Result<Vec<StoredJob>> {
            // Atomically lock + push next_run_at forward by one polling
            // window to suppress double-claims; the executor will write
            // the real next_run_at via complete() once the job finishes.
            // Also stamp `claimed_at`/`worker_id` so `requeue_stale` can
            // rescue work abandoned by dead workers.
            let sql = format!(
                "WITH due AS (\n   SELECT id FROM {0}\n   WHERE next_run_at <= $1 AND claimed_at IS NULL\n   ORDER BY next_run_at\n   FOR UPDATE SKIP LOCKED\n   LIMIT $2\n )\n UPDATE {0} t\n SET next_run_at = $1 + interval '1 day',\n     claimed_at = $1,\n     worker_id = $3,\n     attempts = t.attempts + 1\n FROM due\n WHERE t.id = due.id\n RETURNING t.id, t.kind_json, t.schedule, t.name, t.next_run_at, t.last_run_at, t.created_at",
                self.table
            );
            let rows = sqlx::query_as::<
                _,
                (
                    Uuid,
                    serde_json::Value,
                    serde_json::Value,
                    Option<String>,
                    DateTime<Utc>,
                    Option<DateTime<Utc>>,
                    DateTime<Utc>,
                ),
            >(&sql)
            .bind(now)
            .bind(max as i64)
            .bind(worker_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Storage(e.to_string()))?;

            let mut out = Vec::with_capacity(rows.len());
            for (id, kind_json, schedule_json, name, next, last, created) in rows {
                let schedule: crate::Schedule =
                    serde_json::from_value(schedule_json).map_err(|e| Error::Ser(e.to_string()))?;
                out.push(StoredJob {
                    id,
                    kind_json,
                    schedule,
                    name,
                    next_run_at: next,
                    last_run_at: last,
                    created_at: created,
                });
            }
            Ok(out)
        }

        async fn complete(
            &self,
            id: Uuid,
            finished: DateTime<Utc>,
            next: Option<DateTime<Utc>>,
        ) -> Result<()> {
            match next {
                Some(next) => {
                    let sql = format!(
                        "UPDATE {} SET last_run_at = $1, next_run_at = $2, claimed_at = NULL, worker_id = NULL WHERE id = $3",
                        self.table
                    );
                    sqlx::query(&sql)
                        .bind(finished)
                        .bind(next)
                        .bind(id)
                        .execute(&self.pool)
                        .await
                        .map_err(|e| Error::Storage(e.to_string()))?;
                }
                None => {
                    let sql = format!("DELETE FROM {} WHERE id = $1", self.table);
                    sqlx::query(&sql)
                        .bind(id)
                        .execute(&self.pool)
                        .await
                        .map_err(|e| Error::Storage(e.to_string()))?;
                }
            }
            Ok(())
        }

        async fn requeue_stale(&self, now: DateTime<Utc>, lease_ttl: Duration) -> Result<u64> {
            // Reset lease columns and surface the job for the next claim
            // tick by setting next_run_at to now.
            //
            // INVARIANT: the reaper must never fight a live worker. We
            // already gate on `claimed_at < cutoff`, but that predicate
            // alone races with a concurrent `claim()` that just won its
            // own row lock. Wrapping the row selection in a CTE that
            // takes `FOR UPDATE SKIP LOCKED` makes the lease eviction
            // strictly skip rows another transaction is currently
            // acting on — the standard "row-level work queue" pattern.
            let cutoff = now
                - chrono::Duration::from_std(lease_ttl).unwrap_or(chrono::Duration::seconds(60));
            let sql = format!(
                "WITH stale AS (\n  SELECT id FROM {table}\n  WHERE claimed_at IS NOT NULL AND claimed_at < $2\n  FOR UPDATE SKIP LOCKED\n)\nUPDATE {table} SET claimed_at = NULL, worker_id = NULL, next_run_at = $1\nWHERE id IN (SELECT id FROM stale)",
                table = self.table
            );
            let result = sqlx::query(&sql)
                .bind(now)
                .bind(cutoff)
                .execute(&self.pool)
                .await
                .map_err(|e| Error::Storage(e.to_string()))?;
            Ok(result.rows_affected())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schedule;
    use serde_json::json;

    #[tokio::test]
    async fn in_memory_claim_and_complete() {
        let store = InMemoryJobStore::new();
        let job = StoredJob {
            id: Uuid::now_v7(),
            kind_json: json!("hi"),
            schedule: Schedule::Once(Utc::now() - chrono::Duration::seconds(1)),
            name: Some("t".into()),
            next_run_at: Utc::now() - chrono::Duration::seconds(1),
            last_run_at: None,
            created_at: Utc::now(),
        };
        store.insert(job.clone()).await.unwrap();
        let worker = Uuid::now_v7();
        let claimed = store.claim_due(worker, Utc::now(), 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        store.complete(job.id, Utc::now(), None).await.unwrap();
        let after = store.claim_due(worker, Utc::now(), 10).await.unwrap();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn in_memory_requeue_stale_resets_dead_lease() {
        let store = InMemoryJobStore::new();
        let job = StoredJob {
            id: Uuid::now_v7(),
            kind_json: json!("hi"),
            schedule: Schedule::Once(Utc::now()),
            name: Some("t".into()),
            next_run_at: Utc::now(),
            last_run_at: None,
            created_at: Utc::now(),
        };
        store.insert(job.clone()).await.unwrap();
        let worker = Uuid::now_v7();
        // Worker A claims the job …
        let claimed = store.claim_due(worker, Utc::now(), 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        // … then dies. Some time later we sweep for stale leases.
        let later = Utc::now() + chrono::Duration::seconds(120);
        let n = store
            .requeue_stale(later, Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(n, 1);
        // The job is reclaimable.
        let again = store.claim_due(worker, later, 10).await.unwrap();
        assert_eq!(again.len(), 1);
    }
}
