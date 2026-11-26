//! The store's clock rules (D13 in `docs/decisions.md`): received time,
//! service time and the millisecond format writers emit. See
//! `spec/store-0.md`, "Two clocks".
//!
//! Nothing here reads a clock. A writer reads its clock once per transaction
//! and passes the reading in, so these rules can be replayed with a scripted
//! clock and give the same bytes every time.
//!
//! [`Timestamp`]'s fields are public, so a caller can build one whose `nanos`
//! is a second or more. Every function here first carries those whole seconds
//! into `secs`, so such a value means the instant it adds up to, and none of
//! them can overflow or panic. [`time::parse`] never returns one.

use evidence_core::time::{self, Timestamp};

const NANOS_PER_MILLI: u32 = 1_000_000;
const NANOS_PER_SEC: u32 = 1_000_000_000;

/// The same instant with `nanos` below one second: whole seconds in `nanos`
/// are carried into `secs` (saturating at the ends of `i64`).
fn normalize(t: Timestamp) -> Timestamp {
    Timestamp {
        secs: t.secs.saturating_add(i64::from(t.nanos / NANOS_PER_SEC)),
        nanos: t.nanos % NANOS_PER_SEC,
    }
}

/// Drop the digits after the milliseconds. Never rounds up.
pub fn truncate_millis(t: Timestamp) -> Timestamp {
    let t = normalize(t);
    Timestamp {
        secs: t.secs,
        nanos: t.nanos - t.nanos % NANOS_PER_MILLI,
    }
}

/// The first whole millisecond at or after `t`.
///
/// A time the store wrote itself is always a whole millisecond, so this
/// leaves it unchanged. It only matters for an earlier time with more than 3
/// fraction digits (an imported log, or an object a writer dated): the result
/// is then never less than that time, yet still exact in 3 digits.
fn ceil_millis(t: Timestamp) -> Timestamp {
    let t = normalize(t);
    let down = truncate_millis(t);
    if down == t {
        return t;
    }
    // Both below one second, so the sum fits in a u32.
    let nanos = down.nanos + NANOS_PER_MILLI;
    if nanos >= NANOS_PER_SEC {
        Timestamp {
            secs: t.secs.saturating_add(1),
            nanos: nanos - NANOS_PER_SEC,
        }
    } else {
        Timestamp {
            secs: t.secs,
            nanos,
        }
    }
}

/// `received_at` for the next entry: the clock reading truncated to
/// milliseconds, or the last entry's `received_at` rounded **up** to a whole
/// millisecond, if that is later. A clock that steps backwards therefore
/// repeats the last value rather than going below it.
///
/// The last value is rounded up, not used as it is, because a writer emits 3
/// fraction digits and a last value read from an imported log may have up to
/// 9: `…05.123456789Z` gives `…05.124Z`, never `…05.123Z`. A whole-millisecond
/// last value, which is everything a writer emits, is unchanged. See
/// `spec/store-0.md`, "Two clocks" ("Earlier times finer than a millisecond").
pub fn next_received(now: Timestamp, last: Option<Timestamp>) -> Timestamp {
    let now = truncate_millis(now);
    match last.map(ceil_millis) {
        Some(last) if last > now => last,
        _ => now,
    }
}

/// `at` for an object the store authors in a chain (a checkpoint, or the
/// genesis with no prev): the clock reading truncated to milliseconds, or the
/// latest `at` among its `prev` objects, each rounded **up** to a whole
/// millisecond, if that is later. A clock that steps backwards therefore
/// cannot break evidence/0's prev ordering rule, even for a prev dated with
/// more than 3 fraction digits. See `spec/store-0.md`, "Two clocks".
pub fn service_at(now: Timestamp, prev_ats: &[Timestamp]) -> Timestamp {
    prev_ats
        .iter()
        .copied()
        .map(ceil_millis)
        .fold(truncate_millis(now), Timestamp::max)
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ`: exactly 3 fraction digits, truncating any
/// further digits. For an instant outside the years 0000 to 9999, which no
/// parsed timestamp is, the text is not a valid timestamp.
pub fn format_millis(t: Timestamp) -> String {
    let t = normalize(t);
    let secs = time::format_secs(t.secs);
    let whole = secs.strip_suffix('Z').unwrap_or(&secs);
    format!("{whole}.{:03}Z", t.nanos / NANOS_PER_MILLI)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Timestamp {
        time::parse(s).unwrap()
    }

    fn text(x: Timestamp) -> String {
        format_millis(x)
    }

    #[test]
    fn format_has_exactly_three_digits_and_truncates() {
        assert_eq!(text(t("2026-11-26T00:00:00Z")), "2026-11-26T00:00:00.000Z");
        assert_eq!(
            text(t("2026-11-26T00:00:00.5Z")),
            "2026-11-26T00:00:00.500Z"
        );
        assert_eq!(
            text(t("2026-11-26T00:00:00.12Z")),
            "2026-11-26T00:00:00.120Z"
        );
        assert_eq!(
            text(t("2026-11-26T23:59:59.999999999Z")),
            "2026-11-26T23:59:59.999Z"
        );
        assert_eq!(
            text(t("2026-11-26T00:00:00.0009Z")),
            "2026-11-26T00:00:00.000Z"
        );
        assert_eq!(text(t("1970-01-01T00:00:00Z")), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            text(t("1969-12-31T23:59:59.001Z")),
            "1969-12-31T23:59:59.001Z"
        );
        assert_eq!(
            text(t("2024-02-29T12:34:56.789Z")),
            "2024-02-29T12:34:56.789Z"
        );
        assert_eq!(text(t("0000-01-01T00:00:00Z")), "0000-01-01T00:00:00.000Z");
        assert_eq!(
            text(t("9999-12-31T23:59:59.999Z")),
            "9999-12-31T23:59:59.999Z"
        );
        // What it writes is a valid timestamp that reads back as the
        // truncated time.
        let x = t("2026-11-26T08:09:10.123456789Z");
        assert_eq!(t(&text(x)), truncate_millis(x));
    }

    #[test]
    fn truncation_never_rounds_up() {
        let x = t("2026-11-26T00:00:00.999999999Z");
        assert_eq!(truncate_millis(x), t("2026-11-26T00:00:00.999Z"));
        assert_eq!(
            truncate_millis(t("2026-11-26T00:00:00.0004Z")),
            t("2026-11-26T00:00:00Z")
        );
        let whole = t("2026-11-26T00:00:00.123Z");
        assert_eq!(truncate_millis(whole), whole);
    }

    #[test]
    fn ceil_reaches_the_next_whole_millisecond() {
        let whole = t("2026-11-26T00:00:00.123Z");
        assert_eq!(ceil_millis(whole), whole);
        assert_eq!(
            ceil_millis(t("2026-11-26T00:00:00.1230001Z")),
            t("2026-11-26T00:00:00.124Z")
        );
        assert_eq!(
            ceil_millis(t("2026-11-26T23:59:59.9995Z")),
            t("2026-11-27T00:00:00Z")
        );
    }

    #[test]
    fn received_time_never_decreases() {
        let first = next_received(t("2026-11-26T00:00:01.234567Z"), None);
        assert_eq!(text(first), "2026-11-26T00:00:01.234Z");

        // The clock moves forward: the reading wins.
        let later = next_received(t("2026-11-26T00:00:02.5Z"), Some(first));
        assert_eq!(text(later), "2026-11-26T00:00:02.500Z");

        // The clock steps back an hour: the last value repeats.
        let back = next_received(t("2026-11-25T23:00:02.5Z"), Some(later));
        assert_eq!(back, later);

        // Within the same millisecond, or exactly equal: the same value.
        assert_eq!(
            next_received(t("2026-11-26T00:00:02.5009Z"), Some(later)),
            later
        );
        assert_eq!(next_received(later, Some(later)), later);

        // A scripted run with backwards steps stays monotone.
        let readings = [
            "2026-11-26T00:00:00Z",
            "2026-11-26T00:00:05Z",
            "2026-11-26T00:00:03Z",
            "2026-11-25T00:00:00Z",
            "2026-11-26T00:00:05.0001Z",
            "2026-11-26T00:00:06Z",
        ];
        let mut last = None;
        let mut out = Vec::new();
        for r in readings {
            let next = next_received(t(r), last);
            if let Some(prev) = last {
                assert!(next >= prev);
            }
            out.push(text(next));
            last = Some(next);
        }
        assert_eq!(
            out,
            [
                "2026-11-26T00:00:00.000Z",
                "2026-11-26T00:00:05.000Z",
                "2026-11-26T00:00:05.000Z",
                "2026-11-26T00:00:05.000Z",
                "2026-11-26T00:00:05.000Z",
                "2026-11-26T00:00:06.000Z",
            ]
        );
    }

    #[test]
    fn received_time_after_a_finer_last_value_is_still_not_less_when_written() {
        // An imported entry may carry 9 fraction digits. With the clock
        // behind it, the next value is the next whole millisecond, so the
        // 3-digit text is not less than the last value.
        let last = t("2026-11-26T00:00:05.123456789Z");
        let next = next_received(t("2026-11-26T00:00:01Z"), Some(last));
        assert_eq!(text(next), "2026-11-26T00:00:05.124Z");
        assert!(t(&text(next)) >= last);
        // With the clock ahead, the clock wins as usual.
        let next = next_received(t("2026-11-26T00:00:07.9999Z"), Some(last));
        assert_eq!(text(next), "2026-11-26T00:00:07.999Z");
    }

    #[test]
    fn nanos_of_a_second_or_more_carry_into_secs() {
        // Hand-built values only: time::parse never returns one.
        let over = Timestamp {
            secs: 0,
            nanos: 1_999_999_999,
        };
        assert_eq!(format_millis(over), "1970-01-01T00:00:01.999Z");
        assert_eq!(truncate_millis(over), t("1970-01-01T00:00:01.999Z"));
        assert_eq!(ceil_millis(over), t("1970-01-01T00:00:02Z"));

        let max = Timestamp {
            secs: 0,
            nanos: u32::MAX,
        };
        // u32::MAX ns is 4.294967295 s.
        let now = t("1970-01-01T00:00:01Z");
        assert_eq!(
            text(next_received(now, Some(max))),
            "1970-01-01T00:00:04.295Z"
        );
        assert_eq!(text(service_at(now, &[max])), "1970-01-01T00:00:04.295Z");
        assert_eq!(text(next_received(max, None)), "1970-01-01T00:00:04.294Z");

        // At the far end of secs, nothing overflows.
        let end = Timestamp {
            secs: i64::MAX,
            nanos: u32::MAX,
        };
        assert_eq!(truncate_millis(end).secs, i64::MAX);
        assert_eq!(ceil_millis(end).secs, i64::MAX);
        assert_eq!(next_received(end, Some(end)).secs, i64::MAX);
        assert_eq!(service_at(end, &[end]).secs, i64::MAX);
    }

    #[test]
    fn service_time_is_the_latest_of_clock_and_prevs() {
        // The genesis: no prev, so the clock truncated to milliseconds.
        assert_eq!(
            text(service_at(t("2026-11-26T00:00:00.0005Z"), &[])),
            "2026-11-26T00:00:00.000Z"
        );
        // A checkpoint after a backwards step takes the previous one's at.
        let prev = t("2026-11-26T10:00:00.000Z");
        assert_eq!(service_at(t("2026-11-26T09:00:00Z"), &[prev]), prev);
        // With the clock ahead, the clock.
        assert_eq!(
            text(service_at(t("2026-11-26T11:00:00.123999Z"), &[prev])),
            "2026-11-26T11:00:00.123Z"
        );
        // Several prevs: the latest of them all.
        let prevs = [t("2026-11-26T08:00:00Z"), t("2026-11-26T12:00:00.5Z"), prev];
        assert_eq!(
            text(service_at(t("2026-11-26T11:00:00Z"), &prevs)),
            "2026-11-26T12:00:00.500Z"
        );
        // A prev with finer digits than milliseconds: the written at is still
        // not before it, so E_PREV_ORDER cannot follow.
        let fine = t("2026-11-26T12:00:00.0000001Z");
        let at = service_at(t("2026-11-26T11:00:00Z"), &[fine]);
        assert!(t(&text(at)) >= fine);
        assert_eq!(text(at), "2026-11-26T12:00:00.001Z");
    }
}
