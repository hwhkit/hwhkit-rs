//! Benchmarks for the cron parser and `next_after` evaluation.

use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use hwhkit_scheduler::cron::{next_after, next_after_spec, parse};

/// Five representative cron expressions covering the common production
/// shapes: every-minute, every-N-minute, hourly, daily, weekday-only.
const COMMON_EXPRS: &[&str] = &[
    "* * * * *",
    "*/5 * * * *",
    "0 * * * *",
    "0 9 * * *",
    "0 9 * * 1-5",
];

fn bench_parse(c: &mut Criterion) {
    let mut g = c.benchmark_group("cron/parse");
    for expr in COMMON_EXPRS {
        g.bench_with_input(*expr, expr, |b, e| {
            b.iter(|| {
                let spec = parse(black_box(e)).unwrap();
                black_box(spec);
            })
        });
    }
    g.finish();
}

fn bench_next_after_uncached(c: &mut Criterion) {
    let mut g = c.benchmark_group("cron/next_after_uncached");
    let now = Utc.with_ymd_and_hms(2026, 5, 7, 12, 0, 0).unwrap();
    for expr in COMMON_EXPRS {
        g.bench_with_input(*expr, expr, |b, e| {
            b.iter(|| {
                let n = next_after(black_box(e), black_box(now)).unwrap();
                black_box(n);
            })
        });
    }
    g.finish();
}

fn bench_next_after_cached(c: &mut Criterion) {
    let mut g = c.benchmark_group("cron/next_after_cached");
    let now = Utc.with_ymd_and_hms(2026, 5, 7, 12, 0, 0).unwrap();
    for expr in COMMON_EXPRS {
        let spec = parse(expr).unwrap();
        g.bench_with_input(*expr, expr, |b, e| {
            b.iter_batched(
                || (spec.clone(), now),
                |(s, t)| {
                    let n = next_after_spec(black_box(&s), black_box(t), e).unwrap();
                    black_box(n);
                },
                BatchSize::SmallInput,
            )
        });
    }
    g.finish();
}

/// "Walk a year" — pay the price of repeated calls. Mostly here so the
/// numbers in the report stay sane (one call should be sub-microsecond
/// post-cache).
fn bench_walk_year(c: &mut Criterion) {
    let spec = parse("0 9 * * 1-5").unwrap();
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    c.bench_function("cron/walk_one_week_weekday_9am", |b| {
        b.iter(|| {
            let mut t = start;
            for _ in 0..7 {
                t = next_after_spec(black_box(&spec), black_box(t), "0 9 * * 1-5").unwrap();
            }
            black_box(t + ChronoDuration::seconds(0))
        })
    });
}

criterion_group!(
    benches,
    bench_parse,
    bench_next_after_uncached,
    bench_next_after_cached,
    bench_walk_year,
);
criterion_main!(benches);
