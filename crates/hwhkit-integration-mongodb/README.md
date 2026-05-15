# hwhkit-integration-mongodb

MongoDB integration for [`hwhkit`](https://crates.io/crates/hwhkit)
services. Wires a connected client into the bootstrap `AppContext`
and registers a readiness probe.

This crate is normally pulled in via a feature flag on the `hwhkit`
facade — depend on `hwhkit` with `features = ["mongodb"]` instead of
adding this crate directly.

## License

MIT OR Apache-2.0
