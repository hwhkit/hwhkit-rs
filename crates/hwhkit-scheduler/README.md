# hwhkit-scheduler

Persistent background scheduler for [`hwhkit`](https://crates.io/crates/hwhkit)
services.

## Features

- Cron + one-shot scheduling
- Durable: jobs survive process restarts (Postgres backend)
- Multi-node safe via `SELECT FOR UPDATE SKIP LOCKED`
- Pluggable `JobStore` (in-memory + Postgres included)
- Type-safe job kinds — caller defines a `Serialize + DeserializeOwned`
  enum and provides one async dispatcher

## Cargo features

| Feature          | What it pulls in                              | Default |
|------------------|-----------------------------------------------|---------|
| `postgres-store` | `sqlx` + `PostgresJobStore`                   | yes     |
| `redis-leader`   | optional Redis leader-election lock           | no      |

## Schema

When using `PostgresJobStore`, run the SQL DDL once (or call
`store.ensure_schema().await?` on startup):

```sql
CREATE TABLE IF NOT EXISTS hwhkit_jobs (
    id UUID PRIMARY KEY,
    kind_json JSONB NOT NULL,
    schedule JSONB NOT NULL,
    name TEXT,
    next_run_at TIMESTAMPTZ NOT NULL,
    last_run_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS hwhkit_jobs_next_run_idx ON hwhkit_jobs (next_run_at);
```

## Quick start

```rust,ignore
use hwhkit_scheduler::{Scheduler, JobSpec, Schedule, storage::InMemoryJobStore};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
enum MyJob { Hello, Tick }

#[tokio::main]
async fn main() {
    let store = InMemoryJobStore::new();
    let sched = Scheduler::<MyJob, _>::new(store, |job| async move {
        println!("running {job:?}");
    });
    sched.schedule(JobSpec::new(MyJob::Tick, Schedule::Cron("*/5 * * * *".into())))
        .await
        .unwrap();
    sched.run(hwhkit_core::ShutdownToken::new()).await;
}
```
