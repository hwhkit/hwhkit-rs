//! Property-based tests for [`ConfigLoader`] layered merge behaviour.
//!
//! The loader applies sources in declaration order and deep-merges each
//! patch into the running result. Two non-trivial properties hold:
//!
//! 1. **Identity**: applying an empty patch source is a no-op (the final
//!    `AppConfig` equals what you'd get without it).
//! 2. **Last-wins on overlap**: when sources overlap on a leaf, the
//!    *latest* value wins (so the merge is left-associative but **not**
//!    commutative — we document the non-commutativity rather than test it).
//!
//! We avoid touching the filesystem by using in-memory `ConfigSource`s.

use async_trait::async_trait;
use hwhkit_config::{
    AppConfig, BootstrapConfig, ConfigLoader, ConfigPatch, ConfigSource, Environment, Result,
};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::TempDir;

// A tiny in-memory source that yields a fixed JSON patch.
struct StaticSource {
    name: &'static str,
    patch: Value,
}

#[async_trait]
impl ConfigSource for StaticSource {
    fn name(&self) -> &'static str {
        self.name
    }
    async fn load(&self, _bootstrap: &BootstrapConfig) -> Result<ConfigPatch> {
        Ok(ConfigPatch::from_value(self.patch.clone()))
    }
}

/// Build a bootstrap whose `config_dir` points at a tempdir containing a
/// minimal-but-valid `default.toml`. All in-memory sources stack on top.
fn bootstrap_with_defaults(tmp: &TempDir) -> BootstrapConfig {
    std::fs::write(
        tmp.path().join("default.toml"),
        r#"
[server]
host = "127.0.0.1"
port = 3000

[observability]
service_name = "demo"
environment = "dev"
"#,
    )
    .unwrap();
    BootstrapConfig::default()
        .with_config_dir(tmp.path())
        .with_environment(Environment::Dev)
}

// Strategies ---------------------------------------------------------------

fn arb_port() -> impl Strategy<Value = u16> {
    1u16..=65535
}

fn arb_log_level() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("trace".to_string()),
        Just("debug".to_string()),
        Just("info".to_string()),
        Just("warn".to_string()),
        Just("error".to_string()),
    ]
}

// Properties ---------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// Identity: an empty patch source is a no-op.
    #[test]
    fn empty_patch_is_identity(port in arb_port()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let tmp = TempDir::new().unwrap();
            let bootstrap = bootstrap_with_defaults(&tmp);

            let baseline_loader = ConfigLoader::default()
                .with_source(StaticSource {
                    name: "set-port",
                    patch: json!({"server": {"port": port}}),
                });
            let baseline = baseline_loader.load(&bootstrap).await.unwrap();

            let with_empty = ConfigLoader::default()
                .with_source(StaticSource {
                    name: "set-port",
                    patch: json!({"server": {"port": port}}),
                })
                .with_source(StaticSource {
                    name: "empty",
                    patch: json!({}),
                });
            let combined = with_empty.load(&bootstrap).await.unwrap();

            prop_assert_eq!(baseline.config.server.port, combined.config.server.port);
            prop_assert_eq!(
                baseline.config.observability.logging.level,
                combined.config.observability.logging.level
            );
            Ok(())
        }).unwrap();
    }

    /// Last-wins: when two sources both set the same leaf, the later one
    /// wins. This documents the non-commutativity of the merge.
    #[test]
    fn last_source_wins_on_overlap(
        a in arb_log_level(),
        b in arb_log_level(),
    ) {
        prop_assume!(a != b);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let tmp = TempDir::new().unwrap();
            let bootstrap = bootstrap_with_defaults(&tmp);

            let loader = ConfigLoader::default()
                .with_source(StaticSource {
                    name: "a",
                    patch: json!({"observability": {"logging": {"level": a.clone()}}}),
                })
                .with_source(StaticSource {
                    name: "b",
                    patch: json!({"observability": {"logging": {"level": b.clone()}}}),
                });
            let loaded = loader.load(&bootstrap).await.unwrap();
            prop_assert_eq!(loaded.config.observability.logging.level, b);
            Ok(())
        }).unwrap();
    }

    /// Disjoint patches commute — `default ⊕ A ⊕ B == default ⊕ B ⊕ A`
    /// when A and B touch different leaves.
    #[test]
    fn disjoint_patches_commute(
        port in arb_port(),
        level in arb_log_level(),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let tmp = TempDir::new().unwrap();
            let bootstrap = bootstrap_with_defaults(&tmp);

            let order_ab = ConfigLoader::default()
                .with_source(StaticSource {
                    name: "a",
                    patch: json!({"server": {"port": port}}),
                })
                .with_source(StaticSource {
                    name: "b",
                    patch: json!({"observability": {"logging": {"level": level.clone()}}}),
                })
                .load(&bootstrap)
                .await
                .unwrap();
            let order_ba = ConfigLoader::default()
                .with_source(StaticSource {
                    name: "b",
                    patch: json!({"observability": {"logging": {"level": level.clone()}}}),
                })
                .with_source(StaticSource {
                    name: "a",
                    patch: json!({"server": {"port": port}}),
                })
                .load(&bootstrap)
                .await
                .unwrap();

            prop_assert_eq!(
                order_ab.config.server.port,
                order_ba.config.server.port
            );
            prop_assert_eq!(
                order_ab.config.observability.logging.level,
                order_ba.config.observability.logging.level
            );
            Ok(())
        }).unwrap();
    }

    /// `ConfigPatch::set_path` round-trips: setting a nested path then
    /// reading it back returns the inserted value.
    #[test]
    fn config_patch_set_path_roundtrip(
        port in arb_port(),
        host in r"[a-z]{1,8}",
    ) {
        let mut patch = ConfigPatch::empty();
        patch.set_path(&["server", "host"], Value::String(host.clone()));
        patch.set_path(&["server", "port"], json!(port));
        let v = patch.into_value();
        prop_assert_eq!(
            v.get("server").and_then(|s| s.get("host")).and_then(Value::as_str),
            Some(host.as_str())
        );
        prop_assert_eq!(
            v.get("server").and_then(|s| s.get("port")).and_then(Value::as_u64),
            Some(port as u64)
        );
    }
}

// Smoke test that the helper actually still produces a valid config.
#[tokio::test]
async fn smoke_loader_validates() {
    let tmp = TempDir::new().unwrap();
    let bootstrap = bootstrap_with_defaults(&tmp);
    let _: AppConfig = ConfigLoader::default()
        .load(&bootstrap)
        .await
        .unwrap()
        .config;
    // Make sure the directory really existed (paranoia against a stale
    // working dir).
    assert!(PathBuf::from(tmp.path()).exists());
}
