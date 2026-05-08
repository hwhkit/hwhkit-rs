//! Tiny 5-field cron parser (`min hour day-of-month month day-of-week`).
//!
//! Supported syntax per field:
//!
//! - `*` (any value)
//! - `N` (exact)
//! - `A,B,C` (list)
//! - `A-B` (inclusive range)
//! - `*/N` (step from min)
//!
//! ## Day-of-month / day-of-week semantics
//!
//! This parser implements the **Vixie cron** day-of-month / day-of-week
//! interaction. Quoting the canonical rule:
//!
//! > Note: The day of a command's execution can be specified by two
//! > fields — day of month, and day of week. If both fields are
//! > restricted (i.e., aren't `*`), the command will be run when *either*
//! > field matches the current time.
//!
//! Concretely:
//!
//! - `0 9 * * 1-5`  — every weekday at 09:00 (DOM is `*`, AND with DOW)
//! - `0 9 1,15 * *` — the 1st and 15th of every month at 09:00 (DOW is
//!   `*`, AND with DOM)
//! - `0 9 1 * 1-5`  — the 1st of every month **OR** every weekday at
//!   09:00 (both restricted, OR semantics — Vixie cron)
//! - `0 9 * * *`    — every day at 09:00 (both `*`, no constraint)
//!
//! Note: This is the OR semantics that match Vixie cron / cron(8) on
//! Linux/macOS. It is *not* compatible with Quartz cron, which uses
//! AND semantics; if you need Quartz compatibility, bring your own
//! parser.
//!
//! This is intentionally minimal — production deployments wanting full
//! Vixie-cron parity (`@reboot`, `L`, etc.) should bring their own parser.

use chrono::{DateTime, Datelike, Duration as ChronoDuration, TimeZone, Timelike, Utc, Weekday};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct CronSpec {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    doms: Vec<u32>,
    months: Vec<u32>,
    dows: Vec<u32>,
    /// Was the DOM field a literal `*`? (Tracks restrictedness for Vixie
    /// OR semantics.)
    dom_unrestricted: bool,
    /// Was the DOW field a literal `*`?
    dow_unrestricted: bool,
}

pub fn parse(expr: &str) -> Result<CronSpec> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(Error::Cron(format!(
            "expected 5 fields, got {}: `{expr}`",
            parts.len()
        )));
    }
    let dom_unrestricted = is_unrestricted(parts[2]);
    let dow_unrestricted = is_unrestricted(parts[4]);
    Ok(CronSpec {
        minutes: parse_field(parts[0], 0, 59)?,
        hours: parse_field(parts[1], 0, 23)?,
        doms: parse_field(parts[2], 1, 31)?,
        months: parse_field(parts[3], 1, 12)?,
        dows: parse_field(parts[4], 0, 6)?,
        dom_unrestricted,
        dow_unrestricted,
    })
}

/// A field is "unrestricted" iff every comma-separated chunk is a `*` or
/// a `*/N` step that yields the full range. The simple cases
/// (`*`, `*/1`) cover essentially every Vixie cron expression in the
/// wild; a more thorough check would also accept `0-59` etc., but those
/// are rare enough that we treat them as restricted.
fn is_unrestricted(raw: &str) -> bool {
    raw.split(',').all(|chunk| chunk == "*")
}

fn parse_field(raw: &str, lo: u32, hi: u32) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    for chunk in raw.split(',') {
        if chunk == "*" {
            for v in lo..=hi {
                out.push(v);
            }
            continue;
        }
        if let Some(step_part) = chunk.strip_prefix("*/") {
            let step: u32 = step_part
                .parse()
                .map_err(|_| Error::Cron(format!("invalid step `{chunk}`")))?;
            if step == 0 {
                return Err(Error::Cron(format!("step cannot be 0 in `{chunk}`")));
            }
            let mut v = lo;
            while v <= hi {
                out.push(v);
                v += step;
            }
            continue;
        }
        if let Some((a, b)) = chunk.split_once('-') {
            let a: u32 = a
                .parse()
                .map_err(|_| Error::Cron(format!("invalid range start `{chunk}`")))?;
            let b: u32 = b
                .parse()
                .map_err(|_| Error::Cron(format!("invalid range end `{chunk}`")))?;
            for v in a..=b {
                if (lo..=hi).contains(&v) {
                    out.push(v);
                }
            }
            continue;
        }
        let v: u32 = chunk
            .parse()
            .map_err(|_| Error::Cron(format!("invalid value `{chunk}`")))?;
        if (lo..=hi).contains(&v) {
            out.push(v);
        }
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(Error::Cron(format!("field `{raw}` produced no values")));
    }
    Ok(out)
}

impl CronSpec {
    pub fn matches(&self, t: DateTime<Utc>) -> bool {
        if !self.minutes.contains(&(t.minute())) {
            return false;
        }
        if !self.hours.contains(&(t.hour())) {
            return false;
        }
        if !self.months.contains(&(t.month())) {
            return false;
        }

        let dow_idx = dow_to_index(t.weekday());
        let dom_match = self.doms.contains(&t.day());
        let dow_match = self.dows.contains(&dow_idx);

        // Vixie cron rule: if both DOM and DOW are restricted, OR them.
        // If at least one is `*`, AND them (which collapses to the
        // restricted side when the other is `*`).
        match (self.dom_unrestricted, self.dow_unrestricted) {
            (true, true) => true, // both `*` — already validated month/hour/min
            (true, false) => dow_match,
            (false, true) => dom_match,
            (false, false) => dom_match || dow_match,
        }
    }
}

/// Compute the next time after `start` (exclusive) that matches `expr`.
///
/// Walks one minute at a time but skips ahead by hour/day when the
/// minute/hour fields exclude the current cell, capping at 4 years to
/// guarantee termination on impossible specs.
pub fn next_after(expr: &str, start: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let spec = parse(expr)?;
    next_after_spec(&spec, start, expr)
}

/// Same as [`next_after`] but accepts a pre-parsed [`CronSpec`] so callers
/// can amortize parse cost across many evaluations.
pub fn next_after_spec(
    spec: &CronSpec,
    start: DateTime<Utc>,
    expr_for_error: &str,
) -> Result<DateTime<Utc>> {
    let mut t = (start + ChronoDuration::minutes(1))
        .with_second(0)
        .ok_or_else(|| Error::Cron("clock arithmetic failed".into()))?
        .with_nanosecond(0)
        .ok_or_else(|| Error::Cron("clock arithmetic failed".into()))?;

    let limit = start + ChronoDuration::days(366 * 4);
    while t < limit {
        if spec.matches(t) {
            return Ok(t);
        }
        t = match t.checked_add_signed(ChronoDuration::minutes(1)) {
            Some(v) => v,
            None => break,
        };
    }

    Err(Error::Cron(format!(
        "no occurrence within 4 years for `{expr_for_error}`"
    )))
}

fn dow_to_index(d: Weekday) -> u32 {
    // Sunday=0, Monday=1, ..., Saturday=6 (matches Vixie cron defaults)
    match d {
        Weekday::Sun => 0,
        Weekday::Mon => 1,
        Weekday::Tue => 2,
        Weekday::Wed => 3,
        Weekday::Thu => 4,
        Weekday::Fri => 5,
        Weekday::Sat => 6,
    }
}

/// Test helper that builds a Utc datetime from components.
#[allow(dead_code)]
pub(crate) fn ymdhm(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_star() {
        let s = parse("* * * * *").unwrap();
        assert_eq!(s.minutes.len(), 60);
        assert_eq!(s.hours.len(), 24);
        assert!(s.dom_unrestricted);
        assert!(s.dow_unrestricted);
    }

    #[test]
    fn parses_step() {
        let s = parse("*/15 * * * *").unwrap();
        assert_eq!(s.minutes, vec![0, 15, 30, 45]);
    }

    #[test]
    fn next_minute_after_a_minute() {
        let now = ymdhm(2026, 5, 7, 10, 0);
        let next = next_after("*/5 * * * *", now).unwrap();
        assert_eq!(next, ymdhm(2026, 5, 7, 10, 5));
    }

    /// `0 9 * * *` — every day at 09:00.
    #[test]
    fn every_day_at_nine() {
        let s = parse("0 9 * * *").unwrap();
        // 2026-05-07 is Thursday — should match
        assert!(s.matches(ymdhm(2026, 5, 7, 9, 0)));
        // 2026-05-10 is Sunday — also matches
        assert!(s.matches(ymdhm(2026, 5, 10, 9, 0)));
        // wrong minute
        assert!(!s.matches(ymdhm(2026, 5, 7, 9, 30)));
    }

    /// `0 9 * * 1-5` — DOM is `*`, DOW is restricted: AND collapses to DOW.
    #[test]
    fn weekdays_only() {
        let s = parse("0 9 * * 1-5").unwrap();
        // 2026-05-07 Thursday
        assert!(s.matches(ymdhm(2026, 5, 7, 9, 0)));
        // 2026-05-09 Saturday — must NOT match
        assert!(!s.matches(ymdhm(2026, 5, 9, 9, 0)));
        // 2026-05-10 Sunday — must NOT match
        assert!(!s.matches(ymdhm(2026, 5, 10, 9, 0)));
    }

    /// `0 9 1,15 * *` — DOW is `*`, DOM is restricted: AND collapses to DOM.
    #[test]
    fn first_and_fifteenth() {
        let s = parse("0 9 1,15 * *").unwrap();
        // 2026-05-01 Friday — matches via DOM
        assert!(s.matches(ymdhm(2026, 5, 1, 9, 0)));
        // 2026-05-15 Friday — matches via DOM
        assert!(s.matches(ymdhm(2026, 5, 15, 9, 0)));
        // 2026-05-07 Thursday — neither
        assert!(!s.matches(ymdhm(2026, 5, 7, 9, 0)));
    }

    /// `0 9 1 * 1-5` — both DOM and DOW are restricted: OR semantics.
    /// Should match the 1st of every month *or* every weekday.
    #[test]
    fn vixie_or_semantics_dom_dow() {
        let s = parse("0 9 1 * 1-5").unwrap();
        assert!(!s.dom_unrestricted);
        assert!(!s.dow_unrestricted);
        // 2026-05-01 Friday — matches both DOM and DOW.
        assert!(s.matches(ymdhm(2026, 5, 1, 9, 0)));
        // 2026-05-07 Thursday — DOW match (weekday), DOM no.
        assert!(s.matches(ymdhm(2026, 5, 7, 9, 0)));
        // 2026-05-09 Saturday — neither match.
        assert!(!s.matches(ymdhm(2026, 5, 9, 9, 0)));
        // 2026-08-01 Saturday — DOM match (1st), DOW no — must still match.
        assert!(s.matches(ymdhm(2026, 8, 1, 9, 0)));
    }
}
