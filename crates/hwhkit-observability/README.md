# hwhkit-observability

Logging, tracing, and metrics initialization for
[`hwhkit`](https://crates.io/crates/hwhkit) services.

This crate is normally pulled in transitively when you depend on
`hwhkit`. Use it directly when you want to drive logging /
OpenTelemetry initialization outside the standard bootstrap.

## What this crate provides

- Default tracing layer (auto JSON in production, pretty in dev) based
  on `tracing-subscriber`.
- OpenTelemetry OTLP exporter (gRPC) under the `otel` feature.
- Client-side instrumentation wrappers under the optional
  `otel-sqlx` / `otel-redis` / `otel-reqwest` features so the
  resulting traces propagate across HTTP / DB / cache boundaries.

## License

MIT OR Apache-2.0
