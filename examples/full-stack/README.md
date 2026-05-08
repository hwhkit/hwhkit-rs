# full-stack

The "OOTB everything" feel: every shipping integration registered, the
full Tier-1 production-defaults bundle pulled in, and Tier-2 capabilities
(rate-limit, idempotency, circuit-breaker, scheduler, JWT, OTel) gated
behind their feature flags.

This is the example to copy when starting a new service that you know
will eventually need most of the kit.

## Run

The example registers four integrations but only initialises those whose
`[integrations.*].enabled` field is `true`. With nothing configured it
just runs the in-process production defaults, the same as `examples/minimal`.

To bring up the dependencies, use `cargo hwhkit dev up` (it generates a
docker-compose file from `hwhkit.toml`) or your own compose stack.

```bash
cargo run -p hwhkit-example-full-stack
curl localhost:3000/                  # → "hwhkit full-stack example"
curl localhost:3000/healthz           # → 200, JSON
curl localhost:3000/metrics           # → Prometheus exposition
```

## Feature flags spelled out

`Cargo.toml` lists every flag this example uses. Notable groups:

- **Tier-1 (always-on production defaults):** `health-endpoints`,
  `metrics`, `process-metrics`, `version-endpoints`,
  `graceful-shutdown`, `request-id`, `middleware-bundle`.
- **Multi-tenant primitives:** `multi-tenant` (TenantId / TenantScope).
- **Integrations:** `postgres`, `redis`, `nats`, `s3`.
- **Tier-2 capabilities:** `jwt`, `rate-limit`, `idempotency`,
  `circuit-breaker`, `scheduler`.
- **Observability:** `otel` (OTLP/gRPC export wired into `tracing`).

Drop any feature you don't need; the umbrella crate is feature-gated
end to end so the binary will shrink accordingly.
