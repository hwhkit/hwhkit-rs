# minimal

The shortest possible hwhkit application: one route, all production
defaults pulled in via the default feature set.

```bash
cargo run -p hwhkit-example-minimal
# server listens on 0.0.0.0:3000
curl localhost:3000/           # → "hello from hwhkit"
curl localhost:3000/healthz    # → 200, JSON
curl localhost:3000/version    # → 200, version JSON
```

What you get for free (from the default feature set):

- `/healthz`, `/health/ready`, `/health/live`
- `/metrics` (Prometheus exposition)
- `/version`, `/info`
- request-id middleware (echoes `x-request-id`)
- graceful shutdown on `SIGINT` / `SIGTERM`
- the standard middleware bundle (panic-catcher, sensitive-headers,
  compression, …)

To customise the listen port without touching code, set
`HWHKIT__SERVER__PORT=3001` in the environment.
