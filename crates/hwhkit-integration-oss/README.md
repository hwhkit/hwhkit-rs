# hwhkit-integration-oss

Aliyun OSS (Object Storage Service) integration for
[`hwhkit`](https://crates.io/crates/hwhkit) services. Wires a
connected `aliyun_oss_client::Client` into the bootstrap `AppContext`
and registers a bucket-info readiness probe.

This crate is normally pulled in via a feature flag on the `hwhkit`
facade — depend on `hwhkit` with `features = ["oss"]` instead of
adding this crate directly.

## Configuration

```toml
[integrations.storage.oss]
enabled = true
required = true
endpoint = "oss-cn-hangzhou"                       # or full URL form
bucket = "my-bucket"
access_key_id = "<RAM AccessKey ID>"
access_key_secret = "<RAM AccessKey Secret>"

# Optional resilience bounds (defaults shown).
[integrations.storage.oss.resilience]
connect_timeout_ms = 5000
op_timeout_ms      = 5000
probe_timeout_ms   = 500
shutdown_timeout_ms = 5000
```

Endpoint forms accepted:

- Bare region: `oss-cn-hangzhou`, `oss-cn-shenzhen`, …
- Full URL: `https://oss-cn-hangzhou.aliyuncs.com`,
  `https://my-bucket.oss-cn-hangzhou-internal.aliyuncs.com`

## Usage

```rust,ignore
use hwhkit::oss::OssHandle;

async fn handler(ctx: AppContext) -> Result<()> {
    let oss = ctx.get::<OssHandle>().expect("oss enabled");
    let bucket = oss.client().bucket(oss.bucket())?;
    let objects = bucket.get_objects().await?;
    // ...
}
```

## SDK choice

This integration wraps the community crate
[`aliyun-oss-client`](https://crates.io/crates/aliyun-oss-client). It's
the most-adopted Rust OSS SDK (90k+ downloads), actively maintained,
and uses `reqwest` + `rustls` — matching the rest of the workspace.

The official Aliyun SDK
[`alibabacloud-oss-sdk-rust-v2`](https://crates.io/crates/alibabacloud-oss-sdk-rust-v2)
is still in `0.1.0-delta` as of 2026-05 and not production-ready. We
will migrate when it stabilises; the wrapper pattern means consumer
code (`OssHandle::client()` etc.) only changes if the public SDK
surface does.

## License

MIT OR Apache-2.0
