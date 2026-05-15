//! HwhKit S3-compatible storage integration (AWS S3 / MinIO).
//!
//! Wires an `aws_sdk_s3::Client` into the bootstrap `AppContext` and
//! exposes a `head_bucket` readiness probe.

#![warn(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::{timeout::TimeoutConfig, Builder as S3ConfigBuilder};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::head_bucket::HeadBucketError;
use aws_sdk_s3::Client;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use serde::{Deserialize, Serialize};

/// Standalone S3 section schema, mirrored from
/// `hwhkit_config::S3IntegrationConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct S3Config {
    /// Whether the integration should be initialised at bootstrap.
    pub enabled: bool,
    /// If `true`, an `init` failure aborts bootstrap; otherwise the
    /// integration is recorded as degraded.
    pub required: bool,
    /// Optional endpoint URL — empty string falls back to the AWS
    /// default endpoint resolution. Use for MinIO / LocalStack.
    pub endpoint: String,
    /// AWS region (e.g. `us-east-1`). Required.
    pub region: String,
    /// Bucket name probed by the readiness check.
    pub bucket: String,
    /// Static access key id; falls back to the AWS credential provider
    /// chain when empty.
    pub access_key_id: String,
    /// Static secret access key; paired with `access_key_id`.
    pub secret_access_key: String,
    /// Force path-style addressing (required for MinIO and many
    /// S3-compatible providers).
    pub force_path_style: bool,
}

/// Cheap-to-clone handle wrapping `aws_sdk_s3::Client` (already cheap to
/// clone). Fields are private — use [`Self::client`], [`Self::bucket`],
/// [`Self::region`], [`Self::endpoint`].
#[derive(Clone)]
#[non_exhaustive]
pub struct S3Handle {
    bucket: String,
    region: String,
    endpoint: Option<String>,
    client: Client,
    op_timeout: Duration,
    shutdown_timeout: Duration,
}

impl std::fmt::Debug for S3Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Handle")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl S3Handle {
    /// Borrow the underlying `aws_sdk_s3::Client`.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Bucket name the handle is configured to operate on.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// AWS region the client was built with.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// Configured custom endpoint, if any (e.g. MinIO host). `None`
    /// when using the default AWS endpoint resolution.
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// Configured per-operation timeout (from `resilience.op_timeout_ms`).
    /// The AWS SDK is also configured with the same value via its
    /// native `TimeoutConfig::operation_timeout` so all SDK calls are
    /// bounded at the network layer.
    pub fn op_timeout(&self) -> Duration {
        self.op_timeout
    }
}

/// `IntegrationProvider` impl for S3-compatible storage. Register an
/// instance of this with the bootstrap pipeline to bring up an
/// `aws_sdk_s3::Client` from the `[integrations.storage.s3]` config
/// section.
#[derive(Debug, Default)]
pub struct S3Provider;

const KEY: &str = "s3";

fn validate(bucket: &str, region: &str, endpoint: &str) -> CoreResult<()> {
    if bucket.trim().is_empty() {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::Misconfigured,
            "s3 bucket cannot be empty",
        ));
    }
    if region.trim().is_empty() {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::Misconfigured,
            "s3 region cannot be empty",
        ));
    }
    if !endpoint.is_empty() && !endpoint.starts_with("http://") && !endpoint.starts_with("https://")
    {
        return Err(CoreError::integration_msg(
            KEY,
            IntegrationFailureKind::InvalidUrl,
            "s3 endpoint must start with http:// or https:// when set",
        ));
    }
    Ok(())
}

#[async_trait]
impl IntegrationProvider for S3Provider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn enabled(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.storage.s3.enabled
    }

    fn required(&self, cfg: &AppConfig) -> bool {
        cfg.integrations.storage.s3.required
    }

    async fn init(&self, ctx: &mut AppContext, cfg: &AppConfig) -> CoreResult<()> {
        let s3_cfg = &cfg.integrations.storage.s3;
        validate(&s3_cfg.bucket, &s3_cfg.region, &s3_cfg.endpoint)?;

        // If explicit credentials are set in config, use them; otherwise fall
        // back to the standard AWS credential provider chain (env, profile,
        // IMDS, etc.).
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(s3_cfg.region.clone()));
        if !s3_cfg.access_key_id.is_empty() && !s3_cfg.secret_access_key.is_empty() {
            let creds = Credentials::new(
                s3_cfg.access_key_id.clone(),
                s3_cfg.secret_access_key.clone(),
                None,
                None,
                "hwhkit-static",
            );
            loader = loader.credentials_provider(creds);
        }
        let shared_cfg = loader.load().await;

        // Wire native AWS SDK timeouts from the resilience config. The
        // SDK's `TimeoutConfig` is the right layer to set these —
        // a wrapping `tokio::time::timeout` would cancel the future but
        // leak the in-flight TCP connection; the SDK's own timeout
        // tears down the request properly.
        let timeouts = TimeoutConfig::builder()
            .connect_timeout(s3_cfg.resilience.connect_timeout())
            .operation_timeout(s3_cfg.resilience.op_timeout())
            .build();
        let mut s3_builder = S3ConfigBuilder::from(&shared_cfg).timeout_config(timeouts);
        if !s3_cfg.endpoint.is_empty() {
            s3_builder = s3_builder.endpoint_url(s3_cfg.endpoint.clone());
        }
        if s3_cfg.force_path_style {
            s3_builder = s3_builder.force_path_style(true);
        }

        let client = Client::from_conf(s3_builder.build());

        // Verify connectivity. `head_bucket` is the cheapest reachability
        // check. We classify the error using the typed SDK error rather
        // than scraping its `Display` text — the latter is locale- and
        // version-dependent.
        if let Err(err) = client.head_bucket().bucket(&s3_cfg.bucket).send().await {
            match err {
                SdkError::ServiceError(svc) => {
                    let inner = svc.into_err();
                    match inner {
                        // Treat "missing bucket" as a soft success — MinIO
                        // startup races commonly hit this and the operator
                        // can create the bucket later.
                        HeadBucketError::NotFound(_) => {}
                        other => {
                            return Err(CoreError::integration(
                                KEY,
                                IntegrationFailureKind::Other,
                                other,
                            ));
                        }
                    }
                }
                other => {
                    let kind = classify_s3_sdk_error(&other);
                    return Err(CoreError::integration(KEY, kind, other));
                }
            }
        }

        let endpoint = if s3_cfg.endpoint.is_empty() {
            None
        } else {
            Some(s3_cfg.endpoint.clone())
        };

        ctx.insert(S3Handle {
            bucket: s3_cfg.bucket.clone(),
            region: s3_cfg.region.clone(),
            endpoint,
            client,
            op_timeout: s3_cfg.resilience.op_timeout(),
            shutdown_timeout: s3_cfg.resilience.shutdown_timeout(),
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<S3Handle>()?;
        Some(Arc::new(S3HealthCheck {
            handle: (*handle).clone(),
            required: cfg.integrations.storage.s3.required,
            probe_timeout: cfg.integrations.storage.s3.resilience.probe_timeout(),
        }))
    }

    async fn shutdown(&self, ctx: &AppContext) -> CoreResult<()> {
        // `aws_sdk_s3::Client` has no explicit close. The HTTP
        // connection pool is dropped with the client.
        let budget = ctx
            .get::<S3Handle>()
            .map(|h| h.shutdown_timeout)
            .unwrap_or_else(|| hwhkit_config::ResilienceConfig::default().shutdown_timeout());
        tracing::info!(
            integration = KEY,
            budget_ms = budget.as_millis() as u64,
            "s3: shutdown hook invoked (client will drop with context)"
        );
        Ok(())
    }
}

#[derive(Clone)]
struct S3HealthCheck {
    handle: S3Handle,
    required: bool,
    probe_timeout: Duration,
}

#[async_trait]
impl HealthCheck for S3HealthCheck {
    fn name(&self) -> &str {
        "s3"
    }
    fn required(&self) -> bool {
        self.required
    }
    async fn check(&self) -> std::result::Result<(), String> {
        let probe = self
            .handle
            .client
            .head_bucket()
            .bucket(&self.handle.bucket)
            .send();
        let result = match tokio::time::timeout(self.probe_timeout, probe).await {
            Ok(r) => r,
            Err(_) => {
                return Err(format!(
                    "probe exceeded probe_timeout_ms = {}",
                    self.probe_timeout.as_millis()
                ));
            }
        };
        match result {
            Ok(_) => Ok(()),
            Err(err) => match err {
                SdkError::ServiceError(svc) => match svc.err() {
                    // Missing-but-reachable bucket → service is up.
                    HeadBucketError::NotFound(_) => Ok(()),
                    // Anything else (Forbidden, signature mismatch, …)
                    // is a real problem that the orchestrator should see.
                    other => Err(format!("head_bucket service error: {other}")),
                },
                // Network / TLS / credential-load failures all surface as
                // non-`ServiceError` `SdkError` variants. Treat them as
                // Down — being unreachable is *not* healthy.
                other => Err(format!("head_bucket transport error: {other}")),
            },
        }
    }
}

/// Map a non-service `SdkError` (network / TLS / credential / timeout)
/// to the corresponding [`IntegrationFailureKind`].
///
/// `SdkError::TimeoutError` is the canonical timeout signal; we also
/// inspect the underlying `DispatchFailure` for an `io::Error` of kind
/// `TimedOut`. Everything else is reported as
/// [`IntegrationFailureKind::ConnectionRefused`] (the previous default).
fn classify_s3_sdk_error<E, R>(err: &SdkError<E, R>) -> IntegrationFailureKind
where
    E: std::error::Error + 'static,
    R: std::fmt::Debug + 'static,
{
    if matches!(err, SdkError::TimeoutError(_)) {
        return IntegrationFailureKind::Timeout;
    }
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::TimedOut {
                return IntegrationFailureKind::Timeout;
            }
        }
        current = e.source();
    }
    IntegrationFailureKind::ConnectionRefused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_bucket_or_region() {
        assert!(validate("", "us-east-1", "").is_err());
        assert!(validate("bucket", "", "").is_err());
    }

    #[test]
    fn rejects_invalid_endpoint() {
        assert!(validate("bucket", "us-east-1", "minio.local:9000").is_err());
        assert!(validate("bucket", "us-east-1", "ftp://minio.local").is_err());
    }

    #[test]
    fn accepts_valid_inputs() {
        assert!(validate("bucket", "us-east-1", "").is_ok());
        assert!(validate("bucket", "us-east-1", "http://localhost:9000").is_ok());
        assert!(validate("bucket", "us-east-1", "https://s3.amazonaws.com").is_ok());
    }
}
