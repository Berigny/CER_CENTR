//! The writer's clock. See `spec/store-0.md`, "Two clocks".
//!
//! A writer reads its clock exactly once per transaction (creating the store,
//! one [`commit`](crate::api::LogWrite::commit), one
//! [`checkpoint`](crate::api::LogWrite::checkpoint)) and passes that reading
//! to the pure rules in `evidence_log::clock`: `next_received` for every
//! entry's `received_at` and `service_at` for the `at` of objects the store
//! authors. Clocks are injected, so a [`ScriptClock`] replays a log byte for
//! byte.
//!
//! - [`SystemClock`]: the operating system's wall clock.
//! - [`ScriptClock`]: a list of readings consumed in order, for tests and for
//!   replaying `inputs.json` scripts. Clones share one queue of readings, so
//!   a script survives a writer being closed and opened again.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use evidence_core::time::{self, Timestamp};

/// A source of the current time. Read once per transaction.
///
/// A reading may go backwards or repeat: the clock rules in
/// `evidence_log::clock` keep `received_at` from decreasing and keep
/// store-authored `at` values from breaking the prev ordering rule.
pub trait Clock: Send {
    /// The current time. Any precision; the store truncates it to
    /// milliseconds.
    fn now(&self) -> Timestamp;
}

/// The operating system's wall clock (`std::time::SystemTime`), with the
/// precision the platform gives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        timestamp_of(SystemTime::now())
    }
}

/// The instant `t` as a [`Timestamp`], before 1970 included. Saturates at
/// the ends of `i64` seconds rather than overflowing.
fn timestamp_of(t: SystemTime) -> Timestamp {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => Timestamp {
            secs: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            nanos: d.subsec_nanos(),
        },
        Err(before) => {
            // `before` is how far `t` lies before the epoch: -(s + n/1e9)
            // seconds, which is -(s + 1) seconds plus (1e9 − n) nanoseconds
            // when n > 0.
            let d = before.duration();
            let secs = i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
            match d.subsec_nanos() {
                0 => Timestamp {
                    secs: secs.saturating_neg(),
                    nanos: 0,
                },
                n => Timestamp {
                    secs: secs.saturating_neg().saturating_sub(1),
                    nanos: 1_000_000_000 - n,
                },
            }
        }
    }
}

/// A scripted clock: readings consumed in order, one per [`Clock::now`].
///
/// This is a test clock. [`Clock::now`] panics when no reading is left, and
/// [`ScriptClock::new`] panics on a reading that is not a timestamp, so a
/// script with too few or malformed readings fails its test at once.
///
/// Clones share one queue. A test that closes and reopens a writer hands
/// each writer a clone and keeps one itself, and the script carries on where
/// it stopped; [`ScriptClock::remaining`] then tells the test whether every
/// reading was used, as `inputs.json` requires.
#[derive(Clone, Debug, Default)]
pub struct ScriptClock {
    readings: Arc<Mutex<VecDeque<Timestamp>>>,
}

impl ScriptClock {
    /// A clock that gives these readings, in order. Each must be an
    /// evidence/0 timestamp (`YYYY-MM-DDTHH:MM:SS[.fraction]Z`, at most 9
    /// fraction digits, a real calendar instant).
    ///
    /// # Panics
    ///
    /// If a reading is not a timestamp.
    pub fn new(times: &[&str]) -> ScriptClock {
        ScriptClock::from_timestamps(times.iter().map(|text| {
            time::parse(text)
                .unwrap_or_else(|_| panic!("clock reading {text:?} is not a timestamp"))
        }))
    }

    /// A clock that gives these readings, in order.
    pub fn from_timestamps(times: impl IntoIterator<Item = Timestamp>) -> ScriptClock {
        ScriptClock {
            readings: Arc::new(Mutex::new(times.into_iter().collect())),
        }
    }

    /// Add a reading after the last one. Every clone sees it.
    pub fn push(&self, time: Timestamp) {
        self.queue().push_back(time);
    }

    /// The readings not yet used, across every clone.
    pub fn remaining(&self) -> usize {
        self.queue().len()
    }

    fn queue(&self) -> MutexGuard<'_, VecDeque<Timestamp>> {
        // A panic while the lock is held cannot leave the queue half-changed,
        // so a poisoned lock is still safe to use.
        self.readings.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Clock for ScriptClock {
    /// The next reading.
    ///
    /// # Panics
    ///
    /// If no reading is left.
    fn now(&self) -> Timestamp {
        let next = self.queue().pop_front();
        next.expect("the scripted clock has no reading left")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn t(s: &str) -> Timestamp {
        time::parse(s).unwrap()
    }

    #[test]
    fn script_clock_gives_readings_in_order() {
        let clock = ScriptClock::new(&["2026-11-26T00:00:01Z", "2026-11-26T00:00:00.5Z"]);
        assert_eq!(clock.remaining(), 2);
        assert_eq!(clock.now(), t("2026-11-26T00:00:01Z"));
        assert_eq!(clock.now(), t("2026-11-26T00:00:00.500Z"));
        assert_eq!(clock.remaining(), 0);
    }

    #[test]
    fn clones_share_one_queue() {
        let clock = ScriptClock::new(&[
            "2026-11-26T00:00:00Z",
            "2026-11-26T00:00:01Z",
            "2026-11-26T00:00:02Z",
        ]);
        let writer: Box<dyn Clock> = Box::new(clock.clone());
        assert_eq!(writer.now(), t("2026-11-26T00:00:00Z"));
        drop(writer);
        let reopened: Box<dyn Clock> = Box::new(clock.clone());
        assert_eq!(reopened.now(), t("2026-11-26T00:00:01Z"));
        assert_eq!(clock.remaining(), 1);
        clock.push(t("2026-11-26T00:00:03Z"));
        assert_eq!(clock.remaining(), 2);
        assert_eq!(clock.now(), t("2026-11-26T00:00:02Z"));
        assert_eq!(reopened.now(), t("2026-11-26T00:00:03Z"));
    }

    #[test]
    fn from_timestamps_keeps_nanoseconds() {
        let clock = ScriptClock::from_timestamps([t("2026-11-26T00:00:03.123456789Z")]);
        assert_eq!(clock.now().nanos, 123_456_789);
    }

    #[test]
    #[should_panic(expected = "no reading left")]
    fn script_clock_panics_when_exhausted() {
        let clock = ScriptClock::new(&["2026-11-26T00:00:00Z"]);
        clock.now();
        clock.now();
    }

    #[test]
    #[should_panic(expected = "is not a timestamp")]
    fn script_clock_panics_on_a_bad_reading() {
        ScriptClock::new(&["2026-02-30T00:00:00Z"]);
    }

    #[test]
    fn a_panic_while_reading_does_not_break_other_clones() {
        let clock = ScriptClock::new(&[]);
        let other = clock.clone();
        assert!(std::panic::catch_unwind(move || other.now()).is_err());
        clock.push(t("2026-11-26T00:00:00Z"));
        assert_eq!(clock.now(), t("2026-11-26T00:00:00Z"));
    }

    #[test]
    fn system_time_converts_on_both_sides_of_the_epoch() {
        let after = UNIX_EPOCH + Duration::new(1_795_651_200, 250_000_000);
        assert_eq!(
            timestamp_of(after),
            Timestamp {
                secs: 1_795_651_200,
                nanos: 250_000_000
            }
        );
        assert_eq!(timestamp_of(UNIX_EPOCH), Timestamp { secs: 0, nanos: 0 });
        let before = UNIX_EPOCH - Duration::new(1, 500_000_000);
        assert_eq!(
            timestamp_of(before),
            Timestamp {
                secs: -2,
                nanos: 500_000_000
            }
        );
        let whole = UNIX_EPOCH - Duration::from_secs(86_400);
        assert_eq!(
            timestamp_of(whole),
            Timestamp {
                secs: -86_400,
                nanos: 0
            }
        );
    }

    #[test]
    fn system_clock_reads_a_plausible_time() {
        let now = SystemClock.now();
        assert!(now > t("2026-01-01T00:00:00Z"));
        assert!(now.nanos < 1_000_000_000);
    }
}
