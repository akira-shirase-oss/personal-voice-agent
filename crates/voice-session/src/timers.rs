//! Deadline timers for the session actor. The domain arms and disarms timers by
//! (call, kind); this keeps them ordered by deadline so the actor sleeps until the next one.

use std::collections::{BTreeMap, HashMap};
use tokio::time::Instant;
use voice_domain::Timer;

#[derive(Default)]
pub struct Timers {
    by_deadline: BTreeMap<(Instant, u64), Timer>,
    by_timer: HashMap<Timer, (Instant, u64)>,
    seq: u64,
}

impl Timers {
    pub fn arm(&mut self, timer: Timer, at: Instant) {
        self.disarm(timer);
        self.seq += 1;
        self.by_deadline.insert((at, self.seq), timer);
        self.by_timer.insert(timer, (at, self.seq));
    }

    pub fn disarm(&mut self, timer: Timer) {
        if let Some(key) = self.by_timer.remove(&timer) {
            self.by_deadline.remove(&key);
        }
    }

    pub fn next(&self) -> Option<Instant> {
        self.by_deadline.keys().next().map(|(at, _)| *at)
    }

    /// Removes and returns every timer due at or before `now`, earliest first.
    pub fn pop_due(&mut self, now: Instant) -> Vec<Timer> {
        let mut due = Vec::new();
        while let Some((&(at, seq), &timer)) = self.by_deadline.iter().next() {
            if at > now {
                break;
            }
            self.by_deadline.remove(&(at, seq));
            self.by_timer.remove(&timer);
            due.push(timer);
        }
        due
    }

    pub fn len(&self) -> usize {
        self.by_timer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_timer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use voice_domain::{CallId, TimerKind};

    #[test]
    fn rearming_replaces_and_due_timers_pop_in_order() {
        let t0 = Instant::now();
        let a = Timer {
            call: CallId(1),
            kind: TimerKind::First,
        };
        let b = Timer {
            call: CallId(2),
            kind: TimerKind::Total,
        };
        let mut t = Timers::default();
        t.arm(a, t0 + Duration::from_millis(50));
        t.arm(b, t0 + Duration::from_millis(10));
        t.arm(a, t0 + Duration::from_millis(5));
        assert_eq!(t.len(), 2);
        assert_eq!(t.pop_due(t0 + Duration::from_millis(20)), vec![a, b]);
        assert!(t.is_empty());
        t.arm(a, t0);
        t.disarm(a);
        assert_eq!(t.next(), None);
    }
}
