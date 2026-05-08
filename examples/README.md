# hwhkit examples

Worked examples that double as smoke tests. Each example is its own
workspace member so it builds with `cargo build -p <example-name>`.

| Example | What it shows |
|---|---|
| [`minimal`](./minimal) | The shortest possible hwhkit service — `run_and_serve(MyApp, BootstrapConfig::default())` with one route. |
| [`postgres-rest`](./postgres-rest) | A REST API with the Postgres integration: `IntegrationProvider`, `PostgresHandle::pool()`, an `ApiError`-aware handler. |
| [`full-stack`](./full-stack) | The "OOTB everything" feel — Postgres + Redis + NATS + S3 + JWT + rate-limit + idempotency + OTel feature flags. |

## Running an example

```bash
# Compile every example:
cargo build --workspace --examples
cargo build -p hwhkit-example-minimal

# Run an example (requires a running service for the integrations the
# example uses; see each example's README for the docker-compose or
# `cargo hwhkit dev up` commands):
cargo run -p hwhkit-example-minimal
```

The examples intentionally avoid network calls during `cargo check` — no
example performs an `init` against a real service at compile time. They
*do* connect at startup, so make sure the relevant services are
reachable before `cargo run`.
