# hwhkit-integration-s3

S3-compatible object storage (AWS S3 / MinIO) integration for [`hwhkit`](https://crates.io/crates/hwhkit)
services. Wires a connected client into the bootstrap `AppContext`
and registers a readiness probe.

This crate is normally pulled in via a feature flag on the `hwhkit`
facade — depend on `hwhkit` with `features = ["s3"]` instead of
adding this crate directly.

## License

MIT OR Apache-2.0
