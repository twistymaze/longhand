//! The capture source's restart loop (R5b), written for Longhand: spend the
//! budget on attempts separated by the source backoff, and report whether
//! capture came back, ran out of budget, or was called off. The clock is
//! injected so the budget is tested at its real durations without sleeping.

use std::fmt::Display;
use std::time::{Duration, Instant};

use crate::{RestartBudget, RestartTracker, source_restart_delay};

/// Time as the loop sees it.
pub trait Clock {
    fn now(&self) -> Instant;
    /// Wait `duration`; false when the wait was called off.
    fn sleep(&self, duration: Duration) -> bool;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Recovery<T> {
    /// An attempt succeeded.
    Resumed(T),
    /// The budget ran out; the last attempt's error, if one was made.
    Exhausted(Option<String>),
    /// Called off while waiting.
    Cancelled,
}

/// Restart after a loss already recorded against nothing: each restart,
/// including the first, is counted, waited for, then attempted. The tracker
/// is the caller's, so a source that fails again soon after coming back
/// spends the same budget (AE15).
pub fn recover<T, E: Display>(
    tracker: &mut RestartTracker,
    budget: &RestartBudget,
    clock: &dyn Clock,
    mut attempt: impl FnMut() -> Result<T, E>,
) -> Recovery<T> {
    let mut last = None;
    loop {
        tracker.maybe_reset_at(budget, clock.now());
        if !tracker.record_restart_at(budget, clock.now()) {
            return Recovery::Exhausted(last);
        }
        if !clock.sleep(source_restart_delay(tracker.count())) {
            return Recovery::Cancelled;
        }
        match attempt() {
            Ok(value) => return Recovery::Resumed(value),
            Err(e) => last = Some(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::SOURCE_RESTART_BUDGET;

    /// Time moves only when slept through.
    struct Fake {
        t0: Instant,
        at: Cell<Duration>,
        slept: RefCell<Vec<u64>>,
        cancel_after: Option<usize>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                t0: Instant::now(),
                at: Cell::new(Duration::ZERO),
                slept: RefCell::new(Vec::new()),
                cancel_after: None,
            }
        }
        fn advance(&self, d: Duration) {
            self.at.set(self.at.get() + d);
        }
    }

    impl Clock for Fake {
        fn now(&self) -> Instant {
            self.t0 + self.at.get()
        }
        fn sleep(&self, d: Duration) -> bool {
            if self.cancel_after == Some(self.slept.borrow().len()) {
                return false;
            }
            self.slept.borrow_mut().push(d.as_secs());
            self.advance(d);
            true
        }
    }

    #[test]
    fn a_source_that_fails_once_comes_back_after_one_second() {
        let clock = Fake::new();
        let mut tracker = RestartTracker::new_at(clock.now());
        let out = recover(&mut tracker, &SOURCE_RESTART_BUDGET, &clock, || {
            Ok::<_, String>(7)
        });
        assert_eq!(out, Recovery::Resumed(7));
        assert_eq!(*clock.slept.borrow(), [1]);
    }

    #[test]
    fn a_source_that_stays_gone_gets_three_attempts_then_the_budget_runs_out() {
        let clock = Fake::new();
        let mut tracker = RestartTracker::new_at(clock.now());
        let mut attempts = 0;
        let out = recover(&mut tracker, &SOURCE_RESTART_BUDGET, &clock, || {
            attempts += 1;
            Err::<(), _>("no input device")
        });
        assert_eq!(out, Recovery::Exhausted(Some("no input device".into())));
        assert_eq!(attempts, 3);
        assert_eq!(*clock.slept.borrow(), [1, 2, 4]);
    }

    #[test]
    fn a_flapping_source_spends_one_budget_across_its_comebacks() {
        // AE15: it reconnects each time and drops again four seconds later.
        let clock = Fake::new();
        let mut tracker = RestartTracker::new_at(clock.now());
        let mut outcomes = Vec::new();
        for _ in 0..4 {
            let out = recover(&mut tracker, &SOURCE_RESTART_BUDGET, &clock, || {
                Ok::<_, String>(())
            });
            let resumed = matches!(out, Recovery::Resumed(()));
            outcomes.push(resumed);
            if !resumed {
                break;
            }
            clock.advance(Duration::from_secs(4));
        }
        assert_eq!(outcomes, [true, true, true, false]);
    }

    #[test]
    fn a_source_quiet_for_the_reset_period_gets_a_full_budget_again() {
        let clock = Fake::new();
        let mut tracker = RestartTracker::new_at(clock.now());
        for _ in 0..3 {
            recover(&mut tracker, &SOURCE_RESTART_BUDGET, &clock, || {
                Ok::<_, String>(())
            });
        }
        clock.advance(Duration::from_secs(40));
        let out = recover(&mut tracker, &SOURCE_RESTART_BUDGET, &clock, || {
            Ok::<_, String>(())
        });
        assert_eq!(out, Recovery::Resumed(()));
    }

    #[test]
    fn stopping_during_the_backoff_calls_the_attempt_off() {
        let clock = Fake {
            cancel_after: Some(1),
            ..Fake::new()
        };
        let mut tracker = RestartTracker::new_at(clock.now());
        let mut attempts = 0;
        let out = recover(&mut tracker, &SOURCE_RESTART_BUDGET, &clock, || {
            attempts += 1;
            Err::<(), _>("gone")
        });
        assert_eq!(out, Recovery::Cancelled);
        assert_eq!(attempts, 1);
    }
}
