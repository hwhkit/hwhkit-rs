use std::sync::Arc;

use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::head_bucket::HeadBucketError;
use aws_sdk_s3::Client;
use hwhkit_config::AppConfig;
use hwhkit_core::{
    AppContext, Error as CoreError, HealthCheck, IntegrationFailureKind, IntegrationProvider,
    Result as CoreResult,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct S3Config {
    pub enabled: bool,
    pub required: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
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
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }
}

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

        let mut s3_builder = S3ConfigBuilder::from(&shared_cfg);
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
                    return Err(CoreError::integration(
                        KEY,
                        IntegrationFailureKind::ConnectionRefused,
                        other,
                    ));
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
        });

        Ok(())
    }

    fn health_check(&self, ctx: &AppContext, cfg: &AppConfig) -> Option<Arc<dyn HealthCheck>> {
        let handle = ctx.get::<S3Handle>()?;
        Some(Arc::new(S3HealthCheck {
            handle: (*handle).clone(),
            required: cfg.integrations.storage.s3.required,
        }))
    }
}

#[derive(Clone)]
struct S3HealthCheck {
    handle: S3Handle,
    required: bool,
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
        match self
            .handle
            .client
            .head_bucket()
            .bucket(&self.handle.bucket)
            .send()
            .await
        {
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
