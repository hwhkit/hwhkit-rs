//! Live integration tests for the MongoDB provider.
//!
//! Spins up a real `mongo:7` container via `testcontainers` and runs
//! the provider lifecycle plus a real insert/find roundtrip.
//!
//! Run with:
//!
//! ```sh
//! cargo test -p hwhkit-integration-mongodb -- --ignored
//! ```

use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, IntegrationProvider};
use hwhkit_integration_mongodb::{MongoDbHandle, MongoDbProvider};
use mongodb::bson::doc;
use testcontainers_modules::{mongo::Mongo, testcontainers::runners::AsyncRunner};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; run with `cargo test -- --ignored`"]
async fn live_mongodb_full_lifecycle() {
    let container = Mongo::default()
        .start()
        .await
        .expect("start mongo container");
    let port = container
        .get_host_port_ipv4(27017)
        .await
        .expect("get container ipv4 port");
    let url = format!("mongodb://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.mongodb.enabled = true;
    cfg.integrations.mongodb.required = true;
    cfg.integrations.mongodb.url = url.clone();
    cfg.integrations.mongodb.database = "hwhkit_live_test".into();

    let provider = MongoDbProvider;
    let mut ctx = AppContext::default();

    // 1. init: opens client, runs admin.ping, registers handle.
    provider
        .init(&mut ctx, &cfg)
        .await
        .expect("provider init against live mongodb");

    // 2. Handle present with the configured database name.
    let handle = ctx
        .get::<MongoDbHandle>()
        .expect("MongoDbHandle in AppContext after init");
    assert_eq!(handle.url(), url);
    assert_eq!(handle.database_name(), "hwhkit_live_test");

    // 3. Health check: another admin.ping. Same client + same path as
    //    hot traffic today (audit finding F3, fixed in P0b).
    let probe = provider
        .health_check(&ctx, &cfg)
        .expect("health_check returns Some after init");
    probe
        .check()
        .await
        .expect("health probe against live mongodb");
    assert_eq!(probe.name(), "mongodb");

    // 4. Real insert + find roundtrip. Confirms the configured DB
    //    accessor works end-to-end.
    let coll = handle
        .database()
        .collection::<mongodb::bson::Document>("smoke");
    coll.insert_one(doc! { "k": "v" }, None)
        .await
        .expect("insert_one");
    let found = coll
        .find_one(doc! { "k": "v" }, None)
        .await
        .expect("find_one")
        .expect("inserted doc must be visible");
    assert_eq!(found.get_str("k").unwrap(), "v");

    // 5. Shutdown (default no-op in current impl; assertion will
    //    tighten when P0d lands).
    provider
        .shutdown(&ctx)
        .await
        .expect("provider shutdown against live mongodb");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "lives alongside docker tests; run with `cargo test -- --ignored`"]
async fn live_mongodb_unreachable_url_surfaces_typed_error() {
    use hwhkit_core::{Error as CoreError, IntegrationFailureKind};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    // serverSelectionTimeoutMS=2000 caps how long the SDK will spend
    // looking for a primary before erroring — without this the
    // mongodb driver waits 30s on unreachable hosts.
    let url = format!("mongodb://127.0.0.1:{port}/?serverSelectionTimeoutMS=2000");

    let mut cfg = AppConfig::default();
    cfg.integrations.mongodb.enabled = true;
    cfg.integrations.mongodb.required = true;
    cfg.integrations.mongodb.url = url;

    let provider = MongoDbProvider;
    let mut ctx = AppContext::default();

    let err = provider
        .init(&mut ctx, &cfg)
        .await
        .expect_err("init must fail against an unreachable URL");

    match err {
        CoreError::Integration { name, kind, .. } => {
            assert_eq!(name, "mongodb");
            assert!(
                matches!(
                    kind,
                    IntegrationFailureKind::ConnectionRefused | IntegrationFailureKind::Timeout
                ),
                "expected ConnectionRefused or Timeout, got {kind:?}"
            );
        }
        other => panic!("expected Integration error variant, got {other:?}"),
    }
}
