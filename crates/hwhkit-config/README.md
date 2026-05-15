# hwhkit-config

Configuration loading, layered merging, and strict validation for
[`hwhkit`](https://crates.io/crates/hwhkit) services.

This crate is normally pulled in transitively when you depend on
`hwhkit`. Use it directly only if you need to drive the configuration
pipeline outside the standard bootstrap (e.g. a custom CLI that pre-
validates `default.toml` + env overlays).

See the workspace [README](https://github.com/louishwh/hwhkit-rs) for
the full picture.

## What this crate provides

- `AppConfig` — the strongly-typed configuration root, with
  `validate_strict()` for fail-fast deployment checks.
- `BootstrapConfig` — service-name / environment / config-dir /
  env-prefix wiring read at startup.
- `ConfigLoader` — the standard `default.toml` → `<env>.toml` → env
  vars → remote-patch merge pipeline.
- `RemoteConfigProvider` / `RemotePatchPolicy` — bring your own
  remote source (Consul / etcd / internal API), filtered through a
  strict allow-list.

## License

MIT OR Apache-2.0
