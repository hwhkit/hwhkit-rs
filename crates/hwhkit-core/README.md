# hwhkit-core

Bootstrap pipeline + core runtime abstractions for
[`hwhkit`](https://crates.io/crates/hwhkit) services.

This crate is normally pulled in transitively when you depend on
`hwhkit`. Use it directly when you need the lower-level traits to
build your own application bootstrap.

## What this crate provides

- `Application` — the user-supplied entry point that produces an
  `axum::Router`.
- `IntegrationProvider` — pluggable connector for an external resource
  (Postgres, Redis, …). Open trait; new methods must ship with
  default impls so adding one is not a breaking change downstream.
- `AppContext` — type-keyed slot map handed to handlers (insert
  concrete types or `dyn Trait` handles).
- `ApiError` / `ProblemDetails` — RFC 7807 error model.
- `Error` / `IntegrationFailureKind` — hybrid typed-or-boxed error
  model. `IntegrationFailureKind::is_transient()` is the canonical
  retry hint.
- `ShutdownToken` — graceful-shutdown propagation.
- `HealthCheck` / `HealthRegistry` — readiness probe machinery.
- `TenantId` / `TenantScope` — per-tenant primitives (under the
  `multi-tenant` feature, on by default).
- `JwtVerifier` — JWKS-aware JWT verifier (under the `jwt` feature).

## License

MIT OR Apache-2.0
