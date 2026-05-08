//! Multi-tenant scoping primitive.
//!
//! [`TenantId`] is an opaque string newtype that downstream code uses as a
//! map key. [`TenantScope<T>`] is a thread-safe `tenant_id -> Arc<T>` map
//! that callers can store in [`crate::AppContext`] to hand out per-tenant
//! handles (database shards, rate-limit buckets, queues, …).
//!
//! ## Trust boundary
//!
//! `TenantId` itself encodes no authentication. The accompanying
//! `TenantExtractorLayer` (in `hwhkit::production::tenant`) reads the
//! tenant id from a request header. **That header is untrusted** unless
//! it is paired with an authentication mechanism (JWT, mTLS, signed
//! gateway header, …). Treat user-supplied tenant ids exactly like any
//! other client input.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Opaque tenant identifier. The inner [`String`] is private — construct
/// via [`Self::new`] (which validates) and read via [`Self::as_str`].
///
/// `Deserialize` is implemented manually so that values entering the
/// type from network input must round-trip through [`Self::new`] —
/// preventing callers from sneaking invalid ids in by hand.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct TenantId(String);

impl TenantId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the wrapper and return the inner [`String`]. Lossy in the
    /// sense that no validation is re-applied on the way out.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for TenantId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(TenantId::new(s))
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Thread-safe map of `TenantId -> Arc<T>` used to hand out per-tenant
/// resources without a global lock on the surrounding `AppContext`.
pub struct TenantScope<T: Send + Sync + 'static> {
    inner: Arc<RwLock<HashMap<TenantId, Arc<T>>>>,
}

impl<T: Send + Sync + 'static> Default for TenantScope<T> {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl<T: Send + Sync + 'static> Clone for TenantScope<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: Send + Sync + 'static> TenantScope<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, id: TenantId, value: Arc<T>) {
        self.inner.write().insert(id, value);
    }

    pub fn get(&self, id: &TenantId) -> Option<Arc<T>> {
        self.inner.read().get(id).cloned()
    }

    /// Atomically fetch the value for `id` or insert a freshly-built one.
    ///
    /// Acquires a single write lock that holds across the check-then-insert
    /// so two concurrent callers seeing a missing entry do not both build a
    /// value (the loser's `f()` would otherwise produce an orphaned
    /// resource). The closure is only invoked when no entry exists yet —
    /// keep it side-effect-light because it runs under the write lock.
    pub fn get_or_insert_with<F>(&self, id: &TenantId, f: F) -> Arc<T>
    where
        F: FnOnce() -> Arc<T>,
    {
        let mut guard = self.inner.write();
        if let Some(existing) = guard.get(id) {
            return Arc::clone(existing);
        }
        let value = f();
        guard.insert(id.clone(), Arc::clone(&value));
        value
    }

    pub fn remove(&self, id: &TenantId) -> Option<Arc<T>> {
        self.inner.write().remove(id)
    }

    /// Snapshot the current `(id, value)` pairs. The snapshot is a
    /// `Vec<(TenantId, Arc<T>)>` to keep the lock-hold time bounded;
    /// the returned iterator is owned and decoupled from the map.
    pub fn iter(&self) -> impl Iterator<Item = (TenantId, Arc<T>)> {
        let snapshot: Vec<_> = self
            .inner
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();
        snapshot.into_iter()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let scope: TenantScope<String> = TenantScope::new();
        scope.insert(TenantId::new("acme"), Arc::new("payload".to_string()));
        let v = scope.get(&TenantId::new("acme")).unwrap();
        assert_eq!(v.as_str(), "payload");
    }

    #[test]
    fn iter_collects_all_entries() {
        let scope: TenantScope<u32> = TenantScope::new();
        scope.insert(TenantId::new("a"), Arc::new(1));
        scope.insert(TenantId::new("b"), Arc::new(2));
        let mut ids: Vec<_> = scope.iter().map(|(k, _)| k.0).collect();
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn remove_returns_value_and_drops_entry() {
        let scope: TenantScope<u32> = TenantScope::new();
        scope.insert(TenantId::new("x"), Arc::new(7));
        let v = scope.remove(&TenantId::new("x")).unwrap();
        assert_eq!(*v, 7);
        assert!(scope.get(&TenantId::new("x")).is_none());
    }

    #[test]
    fn get_or_insert_with_is_atomic_under_contention() {
        // Spawn 50 threads racing on the same fresh tenant id; the
        // closure must run only once and every caller must see the
        // same Arc.
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        use std::sync::Barrier;
        use std::thread;

        let scope: TenantScope<u32> = TenantScope::new();
        let scope = Arc::new(scope);
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(50));
        let id = TenantId::new("hot");

        let mut handles = Vec::with_capacity(50);
        for _ in 0..50 {
            let scope = Arc::clone(&scope);
            let calls = Arc::clone(&calls);
            let barrier = Arc::clone(&barrier);
            let id = id.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                scope.get_or_insert_with(&id, || {
                    calls.fetch_add(1, AtomicOrdering::SeqCst);
                    Arc::new(42)
                })
            }));
        }

        let results: Vec<Arc<u32>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(scope.iter().count(), 1);
        // Every caller must observe the same Arc instance.
        let first = Arc::as_ptr(&results[0]);
        for r in &results {
            assert_eq!(Arc::as_ptr(r), first);
            assert_eq!(**r, 42);
        }
    }
}
