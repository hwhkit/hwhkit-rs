# postgres-rest

A REST endpoint backed by Postgres. Demonstrates how an
`IntegrationProvider` injects a typed handle into `AppContext`, and how
to wire that handle into an `axum` handler that uses `ApiError` for
error mapping.

## Run

```bash
# Bring up Postgres with a sensible default schema:
docker run --rm -p 5432:5432 -e POSTGRES_PASSWORD=hwhkit postgres:16

# Point the app at it (this matches the docker run above):
export HWHKIT__INTEGRATIONS__SQL__POSTGRES__ENABLED=true
export HWHKIT__INTEGRATIONS__SQL__POSTGRES__URL="postgres://postgres:hwhkit@localhost:5432/postgres"

cargo run -p hwhkit-example-postgres-rest

curl localhost:3000/db/now
# → {"now":"2026-05-08 11:23:45.123456+00"}
```

## What to look at

- `MyApp::providers()` registers `PostgresProvider`. The bootstrap
  pipeline calls `init` on it before `build_router` runs.
- `PostgresHandle::pool()` exposes the underlying `sqlx::PgPool` —
  field access is private (`#[non_exhaustive]`), so always go through
  the accessor.
- `ApiError::internal(...)` is the canonical way to surface unexpected
  failures; the response is RFC-7807 problem-details JSON.
