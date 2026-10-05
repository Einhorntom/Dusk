//! Monitor-memory protection (SPEC-WR): a quiet-period buffer that keeps only
//! the latest target per control, and a per-control write-rate limiter. Both
//! are pure state machines; callers pass the current time.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::time::{Duration, Instant};

use crate::ControlValue;

/// At most one write per control per second (SPEC-WR-6).
pub const MIN_WRITE_INTERVAL: Duration = Duration::from_secs(1);
/// At most this many writes per control in any 60 s window (SPEC-WR-6).
pub const MAX_WRITES_PER_MINUTE: usize = 30;
const MINUTE: Duration = Duration::from_secs(60);

/// A buffered target that has become due.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DueTarget<K> {
    pub key: K,
    pub value: ControlValue,
    /// True when the rate limiter already deferred it once, so callers report
    /// the deferral only the first time.
    pub deferred: bool,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    value: ControlValue,
    due: Instant,
    deferred: bool,
}

/// Latest-wins targets that become due after a quiet period (SPEC-WR-1..3).
#[derive(Debug)]
pub struct WriteBuffer<K> {
    pending: HashMap<K, Pending>,
}

impl<K> Default for WriteBuffer<K> {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash> WriteBuffer<K> {
    /// Replaces the key's target. Every new target restarts the quiet period,
    /// so a value is written only once adjusting stops.
    pub fn set_target(&mut self, key: K, value: ControlValue, now: Instant, quiet: Duration) {
        self.pending.insert(
            key,
            Pending {
                value,
                due: now + quiet,
                deferred: false,
            },
        );
    }

    /// The pending target, if any.
    pub fn target(&self, key: &K) -> Option<ControlValue> {
        self.pending.get(key).map(|pending| pending.value)
    }

    /// Drops a pending target, e.g. because an explicit write supersedes it.
    pub fn discard(&mut self, key: &K) {
        self.pending.remove(key);
    }

    /// Removes and returns the earliest target that is due at `now`.
    pub fn take_due(&mut self, now: Instant) -> Option<DueTarget<K>> {
        let key = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.due <= now)
            .min_by_key(|(_, pending)| pending.due)
            .map(|(key, _)| key.clone())?;
        let pending = self.pending.remove(&key)?;
        Some(DueTarget {
            key,
            value: pending.value,
            deferred: pending.deferred,
        })
    }

    /// Puts a due target back until `until` because the rate limit refused
    /// it. A newer target set in the meantime wins.
    pub fn defer(&mut self, target: DueTarget<K>, until: Instant) {
        self.pending.entry(target.key).or_insert(Pending {
            value: target.value,
            due: until,
            deferred: true,
        });
    }

    /// Time until the next target is due (zero if one is due now).
    pub fn next_due(&self, now: Instant) -> Option<Duration> {
        self.pending
            .values()
            .map(|pending| pending.due.saturating_duration_since(now))
            .min()
    }
}

/// Per-key write-rate limits (SPEC-WR-6).
#[derive(Debug)]
pub struct RateLimiter<K> {
    history: HashMap<K, VecDeque<Instant>>,
}

impl<K> Default for RateLimiter<K> {
    fn default() -> Self {
        Self {
            history: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash> RateLimiter<K> {
    /// Records a write at `now` if the limits allow it; otherwise returns the
    /// earliest time a write will be allowed and records nothing.
    pub fn try_reserve(&mut self, key: &K, now: Instant) -> Result<(), Instant> {
        let writes = self.history.entry(key.clone()).or_default();
        while writes
            .front()
            .is_some_and(|time| now.duration_since(*time) >= MINUTE)
        {
            writes.pop_front();
        }
        let after_interval = writes.back().map(|time| *time + MIN_WRITE_INTERVAL);
        let after_window = (writes.len() >= MAX_WRITES_PER_MINUTE)
            .then(|| writes.front().map(|time| *time + MINUTE))
            .flatten();
        match after_interval.into_iter().chain(after_window).max() {
            Some(next) if next > now => Err(next),
            _ => {
                writes.push_back(now);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: Duration = Duration::from_millis(400);

    fn percent(value: u32) -> ControlValue {
        ControlValue::Normalized(value)
    }

    #[test]
    fn the_latest_target_wins_and_each_change_restarts_the_quiet_period() {
        let start = Instant::now();
        let mut buffer = WriteBuffer::default();
        buffer.set_target("b", percent(60), start, QUIET);
        buffer.set_target("b", percent(70), start + Duration::from_millis(300), QUIET);
        assert_eq!(buffer.target(&"b"), Some(percent(70)));
        assert_eq!(buffer.take_due(start + QUIET), None);
        assert_eq!(
            buffer.next_due(start + QUIET),
            Some(Duration::from_millis(300))
        );
        let due = buffer.take_due(start + Duration::from_millis(700)).unwrap();
        assert_eq!(
            (due.key, due.value, due.deferred),
            ("b", percent(70), false)
        );
        assert_eq!(buffer.next_due(start), None);
    }

    #[test]
    fn the_earliest_due_target_comes_first_and_discarded_ones_never() {
        let start = Instant::now();
        let mut buffer = WriteBuffer::default();
        buffer.set_target("late", percent(1), start + Duration::from_millis(50), QUIET);
        buffer.set_target("early", percent(2), start, QUIET);
        buffer.set_target("gone", percent(3), start, QUIET);
        buffer.discard(&"gone");
        let later = start + Duration::from_secs(1);
        assert_eq!(buffer.take_due(later).unwrap().key, "early");
        assert_eq!(buffer.take_due(later).unwrap().key, "late");
        assert_eq!(buffer.take_due(later), None);
    }

    #[test]
    fn a_deferred_target_is_marked_and_does_not_replace_a_newer_one() {
        let start = Instant::now();
        let mut buffer = WriteBuffer::default();
        buffer.set_target("b", percent(60), start, QUIET);
        let due = buffer.take_due(start + QUIET).unwrap();
        let retry = start + Duration::from_secs(1);
        buffer.defer(due.clone(), retry);
        let again = buffer.take_due(retry).unwrap();
        assert!(again.deferred);

        buffer.set_target("b", percent(80), retry, QUIET);
        buffer.defer(again, retry);
        assert_eq!(buffer.target(&"b"), Some(percent(80)));
    }

    #[test]
    fn writes_are_spaced_by_one_second() {
        let start = Instant::now();
        let mut limiter = RateLimiter::default();
        assert_eq!(limiter.try_reserve(&"b", start), Ok(()));
        let next = start + MIN_WRITE_INTERVAL;
        assert_eq!(
            limiter.try_reserve(&"b", start + Duration::from_millis(400)),
            Err(next)
        );
        assert_eq!(limiter.try_reserve(&"b", next), Ok(()));
        assert_eq!(limiter.try_reserve(&"other", next), Ok(()));
    }

    #[test]
    fn at_most_thirty_writes_fit_in_any_minute() {
        let start = Instant::now();
        let mut limiter = RateLimiter::default();
        let mut now = start;
        for _ in 0..MAX_WRITES_PER_MINUTE {
            assert_eq!(limiter.try_reserve(&"b", now), Ok(()));
            now += MIN_WRITE_INTERVAL;
        }
        assert_eq!(limiter.try_reserve(&"b", now), Err(start + MINUTE));
        assert_eq!(limiter.try_reserve(&"b", start + MINUTE), Ok(()));
    }
}
