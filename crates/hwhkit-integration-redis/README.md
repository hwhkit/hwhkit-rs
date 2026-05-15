# hwhkit-integration-redis

Redis (or Dragonfly) integration for [`hwhkit`](https://crates.io/crates/hwhkit)
services. Wires a connected client into the bootstrap `AppContext`
and registers a readiness probe.

This crate is normally pulled in via a feature flag on the `hwhkit`
facade — depend on `hwhkit` with `features = ["redis"]` instead of
adding this crate directly.

## License

MIT OR Apache-2.0
