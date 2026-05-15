//! Live integration tests for the Neo4j provider.
//!
//! Spins up `neo4j:5` (community edition) via `testcontainers` and
//! runs the provider lifecycle plus a real Cypher CREATE + MATCH
//! roundtrip.
//!
//! Run with:
//!
//! ```sh
//! cargo test -p hwhkit-integration-neo4j -- --ignored
//! ```

use hwhkit_config::AppConfig;
use hwhkit_core::{AppContext, IntegrationProvider};
use hwhkit_integration_neo4j::{Neo4jHandle, Neo4jProvider};
use testcontainers_modules::{neo4j::Neo4j, testcontainers::runners::AsyncRunner};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; run with `cargo test -- --ignored`"]
async fn live_neo4j_full_lifecycle() {
    // Neo4j refuses to start with the default `neo4j/neo4j` password
    // because it forces a password change on first login. The
    // testcontainers module sets `NEO4J_AUTH=neo4j/<password>` from
    // the builder; we pick a non-trivial value here so the container
    // boots without prompting.
    let container = Neo4j::default()
        .with_password("hwhkit-live-pw")
        .start()
        .await
        .expect("start neo4j container");
    let port = container
        .get_host_port_ipv4(7687)
        .await
        .expect("get container bolt port");
    let url = format!("bolt://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.neo4j.enabled = true;
    cfg.integrations.neo4j.required = true;
    cfg.integrations.neo4j.url = url.clone();
    cfg.integrations.neo4j.username = "neo4j".into();
    cfg.integrations.neo4j.password = "hwhkit-live-pw".into();

    let provider = Neo4jProvider;
    let mut ctx = AppContext::default();

    // 1. init: opens connection pool, runs `RETURN 1` ping.
    provider
        .init(&mut ctx, &cfg)
        .await
        .expect("provider init against live neo4j");

    // 2. Handle present with correct metadata; password is intentionally
    //    not exposed via any accessor.
    let handle = ctx
        .get::<Neo4jHandle>()
        .expect("Neo4jHandle in AppContext after init");
    assert_eq!(handle.url(), url);
    assert_eq!(handle.username(), "neo4j");

    // 3. Health check (audit F3 — shares pool with hot path today).
    let probe = provider
        .health_check(&ctx, &cfg)
        .expect("health_check returns Some after init");
    probe
        .check()
        .await
        .expect("health probe against live neo4j");
    assert_eq!(probe.name(), "neo4j");

    // 4. Real CREATE + MATCH roundtrip — confirms the pool actually
    //    carries writes and reads, not just the smoke-test query.
    handle
        .graph()
        .run(neo4rs::query("CREATE (n:HwhkitLiveTest {label: $label})").param("label", "smoke"))
        .await
        .expect("CREATE");

    let mut rows = handle
        .graph()
        .execute(
            neo4rs::query("MATCH (n:HwhkitLiveTest {label: $label}) RETURN n.label AS label")
                .param("label", "smoke"),
        )
        .await
        .expect("execute MATCH");
    let row = rows
        .next()
        .await
        .expect("driver yielded a row")
        .expect("no row returned for the node we just created");
    let label: String = row.get("label").expect("label column");
    assert_eq!(label, "smoke");

    // 5. Shutdown (default no-op; assertion will tighten with P0d).
    provider
        .shutdown(&ctx)
        .await
        .expect("provider shutdown against live neo4j");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "lives alongside docker tests; run with `cargo test -- --ignored`"]
async fn live_neo4j_unreachable_url_surfaces_typed_error() {
    use hwhkit_core::{Error as CoreError, IntegrationFailureKind};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    let url = format!("bolt://127.0.0.1:{port}");

    let mut cfg = AppConfig::default();
    cfg.integrations.neo4j.enabled = true;
    cfg.integrations.neo4j.required = true;
    cfg.integrations.neo4j.url = url;
    cfg.integrations.neo4j.username = "neo4j".into();
    cfg.integrations.neo4j.password = "any".into();

    let provider = Neo4jProvider;
    let mut ctx = AppContext::default();

    let err = provider
        .init(&mut ctx, &cfg)
        .await
        .expect_err("init must fail against an unreachable URL");

    match err {
        CoreError::Integration { name, kind, .. } => {
            assert_eq!(name, "neo4j");
            // AuthFailed is also acceptable: the integration maps the
            // post-connect `RETURN 1` failure to AuthFailed
            // generically, and on an unreachable host that path is
            // sometimes the one that fires first.
            assert!(
                matches!(
                    kind,
                    IntegrationFailureKind::ConnectionRefused
                        | IntegrationFailureKind::Timeout
                        | IntegrationFailureKind::AuthFailed
                ),
                "expected ConnectionRefused / Timeout / AuthFailed, got {kind:?}"
            );
        }
        other => panic!("expected Integration error variant, got {other:?}"),
    }
}
