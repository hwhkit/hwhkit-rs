//! Benchmarks for `AppContext` insert/get round-trips and `TenantScope`
//! contention behaviour.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use hwhkit_core::AppContext;
use std::sync::Arc;

trait Service: Send + Sync + 'static {
    fn id(&self) -> u32;
}

struct ConcreteService(u32);
impl Service for ConcreteService {
    fn id(&self) -> u32 {
        self.0
    }
}

#[derive(Clone)]
struct Counter(u64);

fn bench_typed_round_trip(c: &mut Criterion) {
    c.bench_function("app_context/insert_get_typed", |b| {
        b.iter(|| {
            let mut ctx = AppContext::default();
            ctx.insert(Counter(black_box(42)));
            let v = ctx.get::<Counter>().unwrap();
            black_box(v.0);
        });
    });
}

fn bench_dyn_round_trip(c: &mut Criterion) {
    c.bench_function("app_context/insert_get_dyn", |b| {
        b.iter(|| {
            let mut ctx = AppContext::default();
            ctx.insert_dyn::<dyn Service>(Arc::new(ConcreteService(black_box(7))));
            let v = ctx.get_dyn::<dyn Service>().unwrap();
            black_box(v.id());
        });
    });
}

#[cfg(feature = "multi-tenant")]
mod tenant_bench {
    use super::*;
    use hwhkit_core::{TenantId, TenantScope};
    use std::sync::Barrier;
    use std::thread;

    pub fn bench_tenant_scope_contention(c: &mut Criterion) {
        c.bench_function("tenant_scope/get_or_insert_with_10_threads", |b| {
            b.iter(|| {
                let scope: Arc<TenantScope<u64>> = Arc::new(TenantScope::new());
                let barrier = Arc::new(Barrier::new(10));
                let id = TenantId::new("hot");
                let mut handles = Vec::with_capacity(10);
                for _ in 0..10 {
                    let scope = Arc::clone(&scope);
                    let barrier = Arc::clone(&barrier);
                    let id = id.clone();
                    handles.push(thread::spawn(move || {
                        barrier.wait();
                        scope.get_or_insert_with(&id, || Arc::new(123u64))
                    }));
                }
                for h in handles {
                    let _ = h.join().unwrap();
                }
            });
        });
    }
}

#[cfg(feature = "multi-tenant")]
fn bench_tenant_scope_contention(c: &mut Criterion) {
    tenant_bench::bench_tenant_scope_contention(c);
}

#[cfg(not(feature = "multi-tenant"))]
fn bench_tenant_scope_contention(_: &mut Criterion) {}

criterion_group!(
    benches,
    bench_typed_round_trip,
    bench_dyn_round_trip,
    bench_tenant_scope_contention,
);
criterion_main!(benches);
