//! Property-based tests for the idempotency body fingerprint.
//!
//! The production code's `fingerprint_request` is `pub(crate)` and not
//! exposed to integration tests. We re-implement the documented algorithm
//! here (SHA-256 of `method || "\n" || path || "\n" || body`, hex-encoded)
//! and assert the desired *algorithmic* properties:
//!
//! 1. Determinism — same `(method, path, body)` → same fingerprint.
//! 2. Tamper resistance — any single-byte change in any of the three
//!    components yields a different fingerprint.
//! 3. Collision resistance — 1000 random triples have no duplicates.
//!
//! If the production algorithm ever diverges from this reference, the
//! corresponding unit tests in `production::idempotency` will catch the
//! first-order change; the goal here is to lock the *property set* a
//! correct fingerprint must satisfy.

use proptest::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Reference implementation that mirrors `production::idempotency::fingerprint_request`.
fn fingerprint_request(method: &str, path: &str, body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(method.as_bytes());
    h.update(b"\n");
    h.update(path.as_bytes());
    h.update(b"\n");
    h.update(body);
    let digest = h.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn arb_method() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("GET".to_string()),
        Just("POST".to_string()),
        Just("PUT".to_string()),
        Just("PATCH".to_string()),
        Just("DELETE".to_string()),
    ]
}

fn arb_path() -> impl Strategy<Value = String> {
    r"/[a-z]{1,8}(/[a-z]{1,8}){0,3}"
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// Determinism: identical inputs → identical fingerprint.
    #[test]
    fn deterministic(
        m in arb_method(),
        p in arb_path(),
        body in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        let a = fingerprint_request(&m, &p, &body);
        let b = fingerprint_request(&m, &p, &body);
        prop_assert_eq!(a, b);
    }

    /// Method-change tamper sensitivity.
    #[test]
    fn method_change_flips_fingerprint(
        a_method in arb_method(),
        b_method in arb_method(),
        p in arb_path(),
        body in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        prop_assume!(a_method != b_method);
        let a = fingerprint_request(&a_method, &p, &body);
        let b = fingerprint_request(&b_method, &p, &body);
        prop_assert_ne!(a, b);
    }

    /// Path-change tamper sensitivity.
    #[test]
    fn path_change_flips_fingerprint(
        m in arb_method(),
        a_path in arb_path(),
        b_path in arb_path(),
        body in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        prop_assume!(a_path != b_path);
        let a = fingerprint_request(&m, &a_path, &body);
        let b = fingerprint_request(&m, &b_path, &body);
        prop_assert_ne!(a, b);
    }

    /// Body-change tamper sensitivity (any single-byte flip is enough).
    #[test]
    fn single_body_byte_flip_flips_fingerprint(
        m in arb_method(),
        p in arb_path(),
        body in prop::collection::vec(any::<u8>(), 1..256),
        idx in any::<u32>(),
        bit in 0u8..8,
    ) {
        let i = (idx as usize) % body.len();
        let mut tampered = body.clone();
        tampered[i] ^= 1u8 << bit;
        prop_assume!(tampered != body);
        let a = fingerprint_request(&m, &p, &body);
        let b = fingerprint_request(&m, &p, &tampered);
        prop_assert_ne!(a, b);
    }
}

/// Collision resistance smoke check — 1000 random triples should yield
/// 1000 distinct fingerprints. (SHA-256 makes a collision astronomically
/// unlikely — we'd be observing a real bug if this ever failed.)
#[test]
fn no_collisions_in_a_thousand_random_triples() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut seen = HashSet::with_capacity(1000);
    let methods = ["GET", "POST", "PUT", "PATCH", "DELETE"];
    for n in 0..1000u32 {
        // Deterministic pseudo-random body via std hash to avoid pulling
        // in a new dep — the determinism is OK because we're testing
        // collision resistance of *distinct* triples, not of random ones.
        let mut hasher = DefaultHasher::new();
        n.hash(&mut hasher);
        let body = hasher.finish().to_le_bytes().to_vec();
        let path = format!("/r/{}", n);
        let method = methods[(n as usize) % methods.len()];
        let fp = fingerprint_request(method, &path, &body);
        assert!(seen.insert(fp), "fingerprint collision at n={n}");
    }
    assert_eq!(seen.len(), 1000);
}
