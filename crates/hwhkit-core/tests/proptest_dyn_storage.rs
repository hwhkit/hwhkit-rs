//! Property-based tests for [`AppContext`]'s dynamic-typed storage.
//!
//! The dyn-storage is keyed by `TypeId::of::<dyn Trait>()`. Two interesting
//! invariants:
//!
//! 1. A round-trip insert/get returns a clone of the same trait object.
//! 2. Inserting a second value under the same trait type overwrites the
//!    first (last-write-wins).
//!
//! We also check the typed `insert<T>/get<T>` round-trip invariant.

use hwhkit_core::AppContext;
use proptest::prelude::*;
use std::sync::Arc;

trait Labeller: Send + Sync + 'static {
    fn label(&self) -> String;
}

struct Fixed(String);
impl Labeller for Fixed {
    fn label(&self) -> String {
        self.0.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Counter(u64);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tagged(String);

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// `insert_dyn` then `get_dyn` returns a handle whose label matches
    /// the inserted value.
    #[test]
    fn dyn_round_trip(label in r"[a-z0-9]{1,32}") {
        let mut ctx = AppContext::default();
        ctx.insert_dyn::<dyn Labeller>(Arc::new(Fixed(label.clone())));
        let svc = ctx.get_dyn::<dyn Labeller>().expect("should be present");
        prop_assert_eq!(svc.label(), label);
    }

    /// Last-write-wins for the same trait type.
    #[test]
    fn dyn_last_write_wins(
        a in r"[a-z]{1,8}",
        b in r"[a-z]{1,8}",
    ) {
        let mut ctx = AppContext::default();
        ctx.insert_dyn::<dyn Labeller>(Arc::new(Fixed(a.clone())));
        ctx.insert_dyn::<dyn Labeller>(Arc::new(Fixed(b.clone())));
        let svc = ctx.get_dyn::<dyn Labeller>().unwrap();
        prop_assert_eq!(svc.label(), b);
    }

    /// Typed insert/get (concrete-type slot) round-trips.
    #[test]
    fn typed_round_trip(n in any::<u64>()) {
        let mut ctx = AppContext::default();
        ctx.insert(Counter(n));
        let got = ctx.get::<Counter>().unwrap();
        prop_assert_eq!(got.0, n);
    }

    /// Distinct concrete types live in distinct slots — inserting one
    /// does not evict the other.
    #[test]
    fn distinct_types_coexist(
        n in any::<u64>(),
        s in r"[a-z]{1,8}",
    ) {
        let mut ctx = AppContext::default();
        ctx.insert(Counter(n));
        ctx.insert(Tagged(s.clone()));
        prop_assert_eq!(ctx.get::<Counter>().unwrap().0, n);
        prop_assert_eq!(ctx.get::<Tagged>().unwrap().0.clone(), s);
    }

    /// `get_dyn` for an absent trait returns `None`, never panics.
    #[test]
    fn get_dyn_missing_returns_none(_seed in 0u32..1000) {
        let ctx = AppContext::default();
        prop_assert!(ctx.get_dyn::<dyn Labeller>().is_none());
        prop_assert!(ctx.get::<Counter>().is_none());
    }
}
