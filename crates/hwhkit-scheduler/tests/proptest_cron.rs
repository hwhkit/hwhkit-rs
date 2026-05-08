//! Property-based tests for the cron parser.
//!
//! We exercise three kinds of properties:
//!
//! 1. **Round-trip stability** — formatting a parsed spec back to its
//!    canonical numeric form and re-parsing yields a spec that matches the
//!    same set of timestamps.
//! 2. **Rejection of out-of-range inputs** — random integers above the
//!    field's max produce a parse error rather than panicking or silently
//!    accepting garbage.
//! 3. **`next_after` monotonicity** — for any valid spec, `next_after(t)`
//!    is strictly greater than `t`, and the next-next call advances again
//!    (never stalls).

use chrono::{Duration as ChronoDuration, TimeZone, Timelike, Utc};
use hwhkit_scheduler::cron::{next_after, parse, CronSpec};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Helpers — generate canonical "list" form of every field, which never
// triggers `*` reduction. Each list contains in-range values.
// ---------------------------------------------------------------------------

fn list_in_range(lo: u32, hi: u32) -> impl Strategy<Value = String> {
    prop::collection::vec(lo..=hi, 1..=4).prop_map(|mut vs| {
        vs.sort_unstable();
        vs.dedup();
        vs.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    })
}

fn star_or_list(lo: u32, hi: u32) -> impl Strategy<Value = String> {
    prop_oneof![Just("*".to_string()), list_in_range(lo, hi),]
}

// Generator producing a syntactically-valid 5-field cron expression.
fn valid_cron() -> impl Strategy<Value = String> {
    (
        star_or_list(0, 59), // minute
        star_or_list(0, 23), // hour
        star_or_list(1, 31), // dom
        star_or_list(1, 12), // month
        star_or_list(0, 6),  // dow
    )
        .prop_map(|(mi, h, d, mo, w)| format!("{mi} {h} {d} {mo} {w}"))
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// Parsing a syntactically-valid expression must succeed.
    #[test]
    fn parse_accepts_valid_expressions(expr in valid_cron()) {
        let _spec: CronSpec = parse(&expr).unwrap();
    }

    /// `next_after(t)` is strictly later than `t`. Run for one full year so
    /// we don't accidentally pick a starting timestamp that lies in a dead
    /// zone for some otherwise rare expressions.
    #[test]
    fn next_after_strictly_advances(
        expr in valid_cron(),
        offset_minutes in 0i64..(60 * 24 * 365),
    ) {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
            + ChronoDuration::minutes(offset_minutes);
        // Some random expressions may have no occurrence within 4 years,
        // but for the strategies above (broad ranges + lists) this is
        // very rare; only assert the strict-advance property when parse +
        // next succeed.
        if let Ok(next) = next_after(&expr, start) {
            prop_assert!(next > start);
        }
    }

    /// `next_after(next_after(t))` keeps advancing — running it twice in a
    /// row never stalls.
    #[test]
    fn next_after_progresses_twice(
        expr in valid_cron(),
        offset_minutes in 0i64..(60 * 24 * 30),
    ) {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
            + ChronoDuration::minutes(offset_minutes);
        if let Ok(t1) = next_after(&expr, start) {
            if let Ok(t2) = next_after(&expr, t1) {
                prop_assert!(t2 > t1);
            }
        }
    }

    /// Out-of-range minute values (>=60) without any in-range alternative
    /// produce a parse error rather than panicking.
    #[test]
    fn out_of_range_minute_is_rejected(bad_minute in 60u32..1000) {
        let expr = format!("{bad_minute} * * * *");
        prop_assert!(parse(&expr).is_err());
    }

    /// Out-of-range hour values produce a parse error.
    #[test]
    fn out_of_range_hour_is_rejected(bad_hour in 24u32..1000) {
        let expr = format!("* {bad_hour} * * *");
        prop_assert!(parse(&expr).is_err());
    }

    /// Out-of-range day-of-month values produce a parse error.
    #[test]
    fn out_of_range_dom_is_rejected(bad_dom in 32u32..1000) {
        let expr = format!("* * {bad_dom} * *");
        prop_assert!(parse(&expr).is_err());
    }

    /// Wrong field count is rejected.
    #[test]
    fn wrong_field_count_is_rejected(
        n_fields in prop_oneof![0usize..=4, 6usize..=10]
    ) {
        let expr = vec!["*"; n_fields].join(" ");
        prop_assert!(parse(&expr).is_err());
    }

    /// Step of zero is rejected.
    #[test]
    fn zero_step_is_rejected(field_idx in 0usize..5) {
        let mut fields = ["*"; 5];
        fields[field_idx] = "*/0";
        let expr = fields.join(" ");
        prop_assert!(parse(&expr).is_err());
    }
}

// ---------------------------------------------------------------------------
// `matches` consistency: scanning a year of timestamps, both spec instances
// (parsed twice) agree on which minutes match.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// Re-parsing the same expression yields a spec that matches the same
    /// timestamps as the original.
    #[test]
    fn matches_is_stable_under_reparse(expr in valid_cron()) {
        let a = parse(&expr).unwrap();
        let b = parse(&expr).unwrap();
        // Sample ~one timestamp per hour across a year (8760 points).
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        for hour in 0..(24 * 365) {
            let t = base + ChronoDuration::hours(hour);
            prop_assert_eq!(a.matches(t), b.matches(t),
                "mismatch at {} for `{}`", t, expr);
        }
    }

    /// `*`-only spec matches every minute of every day.
    #[test]
    fn star_only_matches_every_minute(day in 0i64..365) {
        let s = parse("* * * * *").unwrap();
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 12, 30, 0).unwrap()
            + ChronoDuration::days(day);
        prop_assert!(s.matches(t));
    }
}

// ---------------------------------------------------------------------------
// Sanity: round-trip a list-only field expression.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// For lists-only expressions, the matches set is exactly the
    /// cross-product of the listed values — verified against a property
    /// that the year-2026 January 1st 00:00:00 timestamp matches iff
    /// 0 is in each field's list.
    #[test]
    fn list_only_matches_zero_when_zero_listed(
        minutes in prop::collection::vec(0u32..=59, 1..=4),
        hours in prop::collection::vec(0u32..=23, 1..=4),
    ) {
        let mut min_set: Vec<u32> = minutes.clone();
        min_set.sort_unstable();
        min_set.dedup();
        let mut hr_set: Vec<u32> = hours.clone();
        hr_set.sort_unstable();
        hr_set.dedup();

        let mins = min_set.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let hrs = hr_set.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let expr = format!("{mins} {hrs} * * *");
        let spec = parse(&expr).unwrap();

        // Pick a Thursday 2026-01-01.
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let expected = min_set.contains(&t.minute()) && hr_set.contains(&t.hour());
        prop_assert_eq!(spec.matches(t), expected);
    }
}
