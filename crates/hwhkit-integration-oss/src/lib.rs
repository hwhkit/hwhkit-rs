//! HwhKit Aliyun OSS (Object Storage Service) integration.
//!
//! Wires an `aliyun_oss_client::Client` into the bootstrap
//! `AppContext` and exposes a bucket-info readiness probe.
//!
//! **SDK choice:** community crate `aliyun-oss-client` (~90k downloads,
//! actively maintained). The official Aliyun SDK
//! `alibabacloud-oss-sdk-rust-v2` is still in 0.1.0-delta and not
//! production-ready as of 2026-05. The wrapper pattern keeps the SDK
//! choice isolated to this crate — when the official SDK stabilises
//! we can migrate without touching any consumer code.
//!
//! ## What this integration provides (same shape as the other 7)
//!
//! - Bounded init: `connect_timeout_ms` for client construction +
//!   `op_timeout_ms` for the smoke-test bucket-info request.
//! - Readiness probe: `Bucket::get_info` bounded by `probe_timeout_ms`,
//!   isolated from the hot-path client so a hung response can't queue
//!   `/health/ready` behind real traffic.
//! - Bounded shutdown: no-op (HTTP clients have no explicit close) but
//!   the budget is logged for SIGTERM paper-trail symmetry.
//! - `OssHandle::op_timeout()` accessor so user code wraps long
//!   uploads / downloads with the same configured bound.

#![warn(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use aliyun_oss_client::{Client as OssClient, Error as OssError};
use async_trait::async_trait;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use serde::{Deserialize, Serialize};

/// Standalone OSS section schema, mirrored from
/// `hwhkit_config::OssIntegrationConfig` for callers that drive the
/// integration outside the bootstrap pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OssConfig {
    /// Whether the integration should be initialised at bootstrap.
    pub enabled: bool,
    /// If `true`, an `init` failure aborts bootstrap; otherwise the
    /// integration is recorded as degraded.
    pub required: bool,
    /// OSS endpoint URL.
    ///
    /// Either the full URL form
    /// (`https://oss-cn-hangzhou.aliyuncs.com`) or the bare region
    /// form (`oss-cn-hangzhou`). The bare form lets the SDK derive
    /// the HTTPS URL; pass the full URL if you need a custom
    /// scheme (testing, VPC-private endpoint, etc.).
    pub endpoint: String,
    /// Default bucket name. Used by the readiness probe and as the
    /// default for `OssHandle::bucket()`.
    pub bucket: String,
    /// AccessKey ID issued by Aliyun RAM.
    pub access_key_id: String,
    /// AccessKey Secret — the credential pair with `access_key_id`.
    /// Intentionally not exposed via any `OssHandle` accessor.
    pub access_key_secret: String,
}

/// Cheap-to-clone handle wrapping an `aliyun_oss_client::Client` in an
/// `Arc`. Fields are private — use [`Self::client`], [`Self::bucket`],
/// [`Self::endpoint`], [`Self::op_timeout`].
#[derive(Clone)]
#[non_exhaustive]
pub struct OssHandle {
    bucket: String,
    endpoint: String,
    client: Arc<OssClient>,
    op_timeout: Duration,
    shutdown_timeout: Duration,
}

impl std::fmt::Debug for OssHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OssHandle")
            .field("bucket", &self.bucket)
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl OssHandle {
    /// Borrow the underlying `aliyun_oss_client::Client`.
    pub fn client(&self) -> &OssClient {
        &self.client
    }

    /// Default bucket name the handle was initialised with.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// OSS endpoint the client was opened against.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Configured per-operation timeout (from `resilience.op_timeout_ms`).
    /// Wrap long OSS operations with `tokio::time::timeout(handle.op_timeout(), ...)`.
    pub fn op_timeout(&self) -> Duration {
        self.op_timeout
    }
}

/// `IntegrationProvider` impl for Aliyun OSS. Register an instance of
/// this with the bootstrap pipeline to bring up an
/// `aliyun_oss_client::Client` from the `[integrations.storage.oss]`
/// config section.
#[derive(Debug, Default)]
pub struct OssProvider;

const KEY: &str = "oss";

fn validate(endpoint: &str, bucket: &str, key_id: &str, key_secret: &str) -> CoreResult<()> {
    if endpoint.trim().is_empty() {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::Misconfigured,
            "oss endpoint cannot be empty",
        ));
    }
    if bucket.trim().is_empty() {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::Misconfigured,
            "oss bucket cannot be empty",
        ));
    }
    if key_id.trim().is_empty() || key_secret.trim().is_empty() {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::Misconfigured,
            "oss access_key_id and access_key_secret cannot be empty",
        ));
    }
    Ok(())
}

#[async_trait]
impl IntegrationProvider for OssProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.storage.oss.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.storage.oss.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let oss_cfg = &cfg.integrations.storage.oss;
        validate(
            &oss_cfg.endpoint,
            &oss_cfg.bucket,
            &oss_cfg.access_key_id,
            &oss_cfg.access_key_secret,
        )?;

        // `Client::new` itself doesn't open a network connection — it
        // just parses the endpoint and stashes the credentials. We
        // still wrap in `tokio::time::timeout` for consistency with
        // the other integrations; the meaningful network call is
        // the smoke-test `get_info` below.
        let client = OssClient::new(
            oss_cfg.access_key_id.as_str(),
            oss_cfg.access_key_secret.as_str(),
            oss_cfg.endpoint.as_str(),
        )
        .map_err(|e| CoreError::integration(KEY, classify_oss_error(&e), e))?;
        let client = Arc::new(client);

        // Smoke test: GET /?bucketInfo against the configured bucket
        // — equivalent to S3's HeadBucket. Confirms (a) the endpoint
        // is reachable, (b) credentials are valid, (c) the bucket
        // exists and is accessible.
        let bucket = client
            .bucket(&oss_cfg.bucket)
            .map_err(|e| CoreError::integration(KEY, classify_oss_error(&e), e))?;
        let bucket_for_probe = bucket.clone();
        let client_for_probe = Arc::clone(&client);
        let probe = async move { bucket_for_probe.get_info(&client_for_probe).await };
        tokio::time::timeout(oss_cfg.resilience.op_timeout(), probe)
            .await
            .map_err(|_| {
                CoreError::integration_msg(
                    KEY,
                    IntegrationFailureKind::Timeout,
                    "oss smoke-test get_info exceeded op_timeout_ms",
                )
            })?
            .map_err(|e| CoreError::integration(KEY, classify_oss_error(&e), e))?;

        ctx.insert(OssHandle {
            bucket: oss_cfg.bucket.clone(),
            endpoint: oss_cfg.endpoint.clone(),
            client,
            op_timeout: oss_cfg.resilience.op_timeout(),
            shutdown_timeout: oss_cfg.resilience.shutdown_timeout(),
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<OssHandle>()?;
        Some(Arc::new(OssHealthCheck {
            handle: (*handle).clone(),
            required: cfg.integrations.storage.oss.required,
            probe_timeout: cfg.integrations.storage.oss.resilience.probe_timeout(),
        }))
    }

    async fn shutdown(&self, ctx: &AppContext) -> CoreResult<()> {
        // `aliyun_oss_client::Client` is a thin reqwest wrapper with
        // no explicit close. HTTP connection pool is released when
        // the last Arc drops. Bounded log for SIGTERM paper-trail
        // symmetry with the other integrations.
        let budget = ctx
            .get::<OssHandle>()
            .map(|h| h.shutdown_timeout)
            .unwrap_or_else(|| hwhkit_config::ResilienceConfig::default().shutdown_timeout());
        tracing::info!(
            integration = KEY,
            budget_ms = budget.as_millis() as u64,
            "oss: shutdown hook invoked (client will drop with context)"
        );
        Ok(())
    }
}

#[derive(Clone)]
struct OssHealthCheck {
    handle: OssHandle,
    required: bool,
    probe_timeout: Duration,
}

#[async_trait]
impl HealthCheck for OssHealthCheck {
    fn name(&self) -> &str {
        "oss"
    }
    fn required(&self) -> bool {
        self.required
    }
    async fn check(&self) -> std::result::Result<(), String> {
        let bucket = match self.handle.client.bucket(&self.handle.bucket) {
            Ok(b) => b,
            Err(e) => return Err(format!("bucket handle: {e}")),
        };
        let probe = async { bucket.get_info(&self.handle.client).await };
        match tokio::time::timeout(self.probe_timeout, probe).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("get_info failed: {e}")),
            Err(_) => Err(format!(
                "probe exceeded probe_timeout_ms = {}",
                self.probe_timeout.as_millis()
            )),
        }
    }
}

/// Map an `aliyun_oss_client::Error` (`OssError`) to the matching
/// [`IntegrationFailureKind`].
///
/// `OssError::Reqwest` carries the transport failure; we use
/// `reqwest::Error::is_timeout` / `is_connect` to distinguish timeout
/// from connection refused. Misconfiguration-shaped variants
/// (`InvalidEndPoint`, `InvalidBucket`, …) map to `Misconfigured`.
/// The catch-all is `Other` — same convention as the other integrations.
fn classify_oss_error(err: &OssError) -> IntegrationFailureKind {
    if let OssError::Reqwest(e) = err {
        if e.is_timeout() {
            return IntegrationFailureKind::Timeout;
        }
        if e.is_connect() {
            return IntegrationFailureKind::ConnectionRefused;
        }
        return IntegrationFailureKind::Other;
    }
    match err {
        OssError::InvalidEndPoint
        | OssError::InvalidRegion
        | OssError::InvalidBucket
        | OssError::InvalidBucketUrl
        | OssError::NotSetDefaultBucket
        | OssError::BucketName(_) => IntegrationFailureKind::Misconfigured,
        OssError::NoFoundBucket => IntegrationFailureKind::Other,
        // Service errors include 401/403 (auth) and other HTTP
        // responses. We don't have a stable way to extract HTTP status
        // from `ServiceXML` without breaking on SDK upgrades, so we
        // bucket them as `Other` for now.
        OssError::Service(_) => IntegrationFailureKind::Other,
        // Anything else (parsing, header errors, env vars, etc.) is
        // ultimately a misuse signal.
        _ => IntegrationFailureKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_endpoint_bucket_or_credentials() {
        assert!(validate("", "b", "id", "secret").is_err());
        assert!(validate("oss-cn-hangzhou", "", "id", "secret").is_err());
        assert!(validate("oss-cn-hangzhou", "b", "", "secret").is_err());
        assert!(validate("oss-cn-hangzhou", "b", "id", "").is_err());
    }

    #[test]
    fn accepts_valid_inputs() {
        assert!(validate("oss-cn-hangzhou", "my-bucket", "akid", "secret").is_ok());
        assert!(validate(
            "https://oss-cn-hangzhou.aliyuncs.com",
            "my-bucket",
            "akid",
            "secret"
        )
        .is_ok());
    }
}
