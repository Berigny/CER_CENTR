//! UTC timestamps: `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, fraction up to 9 digits.
//! Offsets other than `Z` are not allowed, so ordering never depends on time zones.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// Seconds since 1970-01-01T00:00:00Z.
    pub secs: i64,
    pub nanos: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampError {
    /// Does not match the pattern (a schema error).
    Pattern,
    /// Matches the pattern but is not a real instant, e.g. 30 February.
    Calendar,
}

/// True if `s` matches the schema pattern
/// `^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$`.
pub fn matches_pattern(s: &str) -> bool {
    let b = s.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(|c| c.is_ascii_digit());
    let at = |i: usize, c: u8| b.get(i) == Some(&c);
    if b.len() < 20 {
        return false;
    }
    let fixed = (0..4).all(digit)
        && at(4, b'-')
        && digit(5)
        && digit(6)
        && at(7, b'-')
        && digit(8)
        && digit(9)
        && at(10, b'T')
        && digit(11)
        && digit(12)
        && at(13, b':')
        && digit(14)
        && digit(15)
        && at(16, b':')
        && digit(17)
        && digit(18);
    if !fixed {
        return false;
    }
    match &b[19..] {
        [b'Z'] => true,
        [b'.', rest @ .., b'Z'] => {
            (1..=9).contains(&rest.len()) && rest.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    }
}

pub fn parse(s: &str) -> Result<Timestamp, TimestampError> {
    if !matches_pattern(s) {
        return Err(TimestampError::Pattern);
    }
    let num = |r: std::ops::Range<usize>| s[r].parse::<i64>().expect("digits");
    let (year, month, day) = (num(0..4), num(5..7), num(8..10));
    let (hour, minute, second) = (num(11..13), num(14..16), num(17..19));
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(TimestampError::Calendar);
    }
    let frac = &s[19..s.len() - 1];
    let nanos = if let Some(digits) = frac.strip_prefix('.') {
        let padded = format!("{digits:0<9}");
        padded.parse::<u32>().expect("digits")
    } else {
        0
    };
    let secs = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    Ok(Timestamp { secs, nanos })
}

/// Current time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_secs(secs)
}

pub fn format_secs(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's civil-date algorithms (public domain).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders() {
        let a = parse("2026-11-20T05:12:44Z").unwrap();
        let b = parse("2026-11-20T05:12:44.5Z").unwrap();
        assert!(a < b);
        assert_eq!(parse("1970-01-01T00:00:00Z").unwrap().secs, 0);
        assert_eq!(format_secs(a.secs), "2026-11-20T05:12:44Z");
    }

    #[test]
    fn rejects() {
        assert_eq!(parse("2026-02-30T00:00:00Z"), Err(TimestampError::Calendar));
        assert_eq!(parse("2026-11-20T24:00:00Z"), Err(TimestampError::Calendar));
        assert_eq!(
            parse("2026-11-20T16:12:44+11:00"),
            Err(TimestampError::Pattern)
        );
        assert_eq!(parse("2026-11-20T05:12:44.Z"), Err(TimestampError::Pattern));
        assert!(parse("2024-02-29T00:00:00Z").is_ok());
        assert_eq!(parse("2100-02-29T00:00:00Z"), Err(TimestampError::Calendar));
    }
}
