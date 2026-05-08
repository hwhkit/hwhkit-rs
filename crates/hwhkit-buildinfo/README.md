# hwhkit-buildinfo

Compile-time build information (git SHA, build time, rustc version, crate
version) for hwhkit services.

The `build.rs` here populates `HWHKIT_GIT_SHA`, `HWHKIT_BUILD_TIME_UNIX`,
and `HWHKIT_RUST_VERSION` env vars that this crate exposes as `pub const`s.
CI builds can override `HWHKIT_GIT_SHA` and `HWHKIT_BUILD_TIME` for
reproducibility.

```rust
use hwhkit_buildinfo::build_info;

let info = build_info!();
println!("{} @ {}", info.cargo_version, info.git_sha);
```

The hwhkit facade uses this crate to power the auto-mounted `/version`
and `/info` endpoints when the `version-endpoints` feature is enabled
(default).
