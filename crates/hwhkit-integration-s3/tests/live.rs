//! Live integration tests for the S3 provider against MinIO.
//!
//! Spins up a MinIO container via `testcontainers` (the bucket of
//! interest is created up-front so the readiness probe succeeds), and
//! runs the provider lifecycle plus a real PUT / GET roundtrip
//! against the S3 SDK.
//!
//! Run with:
//!
//! ```sh
//! cargo test -p hwhkit-integration-s3 -- --ignored
//! ```

use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::{config::Builder as S3ConfigBuilder, primitives::ByteStream, Client};
use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, IntegrationProvider};
use hwhkit_integration_s3::{S3Handle, S3Provider};
use testcontainers_modules::{minio::MinIO, testcontainers::runners::AsyncRunner};

const TEST_BUCKET: &str = "hwhkit-live";
const TEST_KEY: &str = "live-test/object";
const TEST_BODY: &[u8] = b"hwhkit live-test body";

/// MinIO defaults: ROOT_USER=minioadmin / ROOT_PASSWORD=minioadmin.
const MINIO_USER: &str = "minioadmin";
const MINIO_PASS: &str = "minioadmin";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; run with `cargo test -- --ignored`"]
async fn live_s3_full_lifecycle() {
    let container = MinIO::default()
        .start()
        .await
        .expect("start minio container");
    let port = container
        .get_host_port_ipv4(9000)
        .await
        .expect("get container s3 port");
    let endpoint = format!("http://127.0.0.1:{port}");

    // Pre-create the bucket via the AWS SDK — our provider's
    // `head_bucket` readiness probe treats NotFound as a soft success
    // but a real PUT/GET roundtrip requires an existing bucket.
    create_bucket(&endpoint).await;

    let mut cfg = AppConfig::default();
    cfg.integrations.storage.s3.enabled = true;
    cfg.integrations.storage.s3.required = true;
    cfg.integrations.storage.s3.endpoint = endpoint.clone();
    cfg.integrations.storage.s3.region = "us-east-1".into();
    cfg.integrations.storage.s3.bucket = TEST_BUCKET.into();
    cfg.integrations.storage.s3.access_key_id = MINIO_USER.into();
    cfg.integrations.storage.s3.secret_access_key = MINIO_PASS.into();
    // MinIO only supports path-style addressing.
    cfg.integrations.storage.s3.force_path_style = true;

    let provider = S3Provider;
    let mut ctx = AppContext::default();

    // 1. init: builds the client, runs head_bucket, stashes the handle.
    provider
        .init(&mut ctx, &cfg)
        .await
        .expect("provider init against live minio");

    // 2. Handle present with the configured metadata.
    let handle = ctx
        .get::<S3Handle>()
        .expect("S3Handle in AppContext after init");
    assert_eq!(handle.bucket(), TEST_BUCKET);
    assert_eq!(handle.region(), "us-east-1");
    assert_eq!(handle.endpoint(), Some(endpoint.as_str()));

    // 3. Health check.
    let probe = provider
        .health_check(&ctx, &cfg)
        .expect("health_check returns Some after init");
    probe
        .check()
        .await
        .expect("health probe against live minio");
    assert_eq!(probe.name(), "s3");

    // 4. Real PUT / GET roundtrip via the provider's client.
    handle
        .client()
        .put_object()
        .bucket(TEST_BUCKET)
        .key(TEST_KEY)
        .body(ByteStream::from_static(TEST_BODY))
        .send()
        .await
        .expect("put_object");
    let got = handle
        .client()
        .get_object()
        .bucket(TEST_BUCKET)
        .key(TEST_KEY)
        .send()
        .await
        .expect("get_object");
    let body = got.body.collect().await.expect("collect body").into_bytes();
    assert_eq!(&body[..], TEST_BODY);

    // 5. Shutdown.
    provider
        .shutdown(&ctx)
        .await
        .expect("provider shutdown against live minio");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "lives alongside docker tests; run with `cargo test -- --ignored`"]
async fn live_s3_unreachable_endpoint_surfaces_typed_error() {
    use hwhkit_core::{Error as CoreError, IntegrationFailureKind};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    let endpoint = format!("http://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.storage.s3.enabled = true;
    cfg.integrations.storage.s3.required = true;
    cfg.integrations.storage.s3.endpoint = endpoint;
    cfg.integrations.storage.s3.region = "us-east-1".into();
    cfg.integrations.storage.s3.bucket = TEST_BUCKET.into();
    cfg.integrations.storage.s3.access_key_id = MINIO_USER.into();
    cfg.integrations.storage.s3.secret_access_key = MINIO_PASS.into();
    cfg.integrations.storage.s3.force_path_style = true;

    let provider = S3Provider;
    let mut ctx = AppContext::default();

    let err = provider
        .init(&mut ctx, &cfg)
        .await
        .expect_err("init must fail against an unreachable endpoint");

    match err {
        CoreError::Integration { name, kind, .. } => {
            assert_eq!(name, "s3");
            // The AWS SDK retries by default, then surfaces either a
            // DispatchFailure (→ ConnectionRefused in our classifier)
            // or a TimeoutError. Both are correct here.
            assert!(
                matches!(
                    kind,
                    IntegrationFailureKind::ConnectionRefused
                        | IntegrationFailureKind::Timeout
                        | IntegrationFailureKind::Other
                ),
                "expected ConnectionRefused / Timeout / Other, got {kind:?}"
            );
        }
        other => panic!("expected Integration error variant, got {other:?}"),
    }
}

/// Helper: build a one-off SDK client against MinIO and create the
/// bucket the live test will operate on. Kept separate from the
/// provider under test so a failure here is unambiguously a test-fixture
/// problem, not an integration regression.
async fn create_bucket(endpoint: &str) {
    let creds = Credentials::new(MINIO_USER, MINIO_PASS, None, None, "hwhkit-test");
    let shared = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .load()
        .await;
    let s3_cfg = S3ConfigBuilder::from(&shared)
        .endpoint_url(endpoint)
        .force_path_style(true)
        .build();
    let client = Client::from_conf(s3_cfg);
    client
        .create_bucket()
        .bucket(TEST_BUCKET)
        .send()
        .await
        .expect("create_bucket via fixture client");
}
