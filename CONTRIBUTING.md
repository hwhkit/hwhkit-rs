# Contributing to hwhkit

Thanks for the interest. This file documents the conventions the
workspace expects PRs to follow. The bar is "boring and consistent" —
the framework's day job is to be predictable for downstream services.

## Development workflow

```bash
cargo build --workspace --all-features
cargo test  --workspace --all-features
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI runs the same matrix on stable, the workspace MSRV (`Cargo.toml`
`rust-version`), and nightly. Set up `cargo-deny` locally if you're
touching dependencies:

```bash
cargo install cargo-deny
cargo deny check
```

Pre-1.0 we accept breaking changes between minor versions — we do
not accept them between patch versions. Mark `pub` types
`#[non_exhaustive]` so additive changes don't ratchet the
`MAJOR.MINOR` floor.

## Naming conventions

Function- and method-level conventions used throughout the workspace.
Stick to these when adding new APIs.

| Pattern | Use for |
|---|---|
| `Foo::new(...)` | The total / canonical constructor. Should rarely fail; if it does, return `Result`. |
| `Foo::with_<field>(self, value) -> Self` | Fluent setter on a builder; method consumes `self` and returns the modified value. Mark `#[must_use]`. |
| `Foo::from_<source>(source) -> Self` (or `Result<Self>`) | Variant constructor — derive a `Foo` from another type. Common when `From<T>` would be ambiguous. |
| `Foo::try_from_<source>(source) -> Result<Self>` | Like `from_*` but always fallible; pair with `try_into`/`try_from` if the conversion is general. |
| `FooProvider` (unit struct + trait impl) | Convention for `IntegrationProvider` implementations. Keep them zero-sized so they can be `Arc::new`'d as singletons. |
| `*Handle` | Cheap-to-clone wrapper around an integration's connection / pool / client. Fields are private; accessor methods (`pool()`, `client()`, `url()`) are the public interface. |
| `with_*` (free fn `bootstrap_with(...)`) | Lower-level entry point that takes the dependencies the higher-level entry point fills in by default. Pair with a `Default::default()`-friendly outer wrapper. |

Errors:

- All public error enums are `#[non_exhaustive]`.
- Strongly typed for hwhkit's own concerns; boxed
  (`Box<dyn std::error::Error + Send + Sync>`) for opaque third-party
  sources.
- Use `IntegrationFailureKind` to classify integration failures —
  callers retry on `is_transient()`.

## Trait additions

The following traits are intentionally **open** (downstream code is
expected to `impl` them):

- `hwhkit_core::Application`
- `hwhkit_core::IntegrationProvider`
- `hwhkit_core::HealthCheck`
- `hwhkit_scheduler::storage::JobStore`
- `hwhkit_config::ConfigSource`
- `hwhkit_config::RemoteConfigProvider`

**Project policy:** every method added to one of these traits must
ship with a default implementation, so existing impls keep compiling
without churn. The trait rustdoc above each declaration repeats this
rule — keep it in sync if you add a trait.

## Async traits: `#[async_trait]` vs AFIT

The workspace uses **`#[async_trait]`** (not `async fn` in trait, "AFIT") for
every public async trait. This is a deliberate choice:

- `Application`, `IntegrationProvider`, `HealthCheck`, `JobStore`,
  `ConfigSource`, `RemoteConfigProvider` are all expected to be used
  through `Arc<dyn Trait>` somewhere in the bootstrap pipeline. AFIT
  traits are not directly `dyn`-compatible without
  `async-fn-in-trait` workarounds (return-type notation, manual
  `Box<dyn Future>` wrapping). `#[async_trait]` produces the boxed
  future automatically and the trait is `dyn`-compatible by
  construction.
- The runtime cost of one extra heap allocation per call is
  acceptable: trait methods on these types are invoked at most a
  handful of times per request (and typically zero — most are
  bootstrap-only).
- `async-trait` is well-understood, has stable error messages, and
  doesn't require MSRV bumps. We can revisit once stable AFIT is
  `dyn`-compatible end-to-end (currently slated for post-2024
  edition).

Library-internal traits that are **never** used as `dyn Trait` may
use AFIT to avoid the macro overhead. Document the choice in the
trait's rustdoc.

## Public-API hygiene

Before opening a PR that touches a `pub` item, run through this
checklist:

1. Is the item `#[non_exhaustive]`? If it's a struct or enum that may
   grow fields/variants, the answer is "yes".
2. Are fields private? Public fields are an interface contract that's
   hard to evolve — prefer accessor methods returning `&T`.
3. Does the new method have `#[must_use]` if it returns `Self` (a
   builder), `Result`, or any other value the caller is expected to
   use?
4. If you're adding a new trait, does the rustdoc state the
   default-method policy?
5. If you're touching a config struct, did you add a `Default` impl
   that gives an "off by default" value for new fields?

## Documentation

- Public items need rustdoc.
- Internal modules can use `//!` headers.
- Update `MIGRATION.md` when you make a breaking change. Keep the
  before/after pair concrete enough that a downstream reader can
  fix-up their code by analogy.
- Update `CHANGELOG.md` for any user-visible change.

## Testing conventions

See `TESTING.md`. Highlights:

- New invariants on small algorithms (cron parser, idempotency
  fingerprint, tenant-id validator) get a `proptest` block.
- Integration providers' validators get a unit test asserting the
  resulting `IntegrationFailureKind` (the contract callers depend on).
