//! The capture source's restart loop (R5b), written for Longhand: spend the
//! budget on attempts separated by the source backoff, and report whether
//! capture came back, ran out of budget, or was called off. Restarts count as
//! consecutive failures; the count clears only after capture recorded for the
//! budget's `reset_after` since it came back. The clock is injected so the
//! budget is tested at its real durations without sleeping.

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

/// The restarts of a source across a session: the count of consecutive
/// failed restarts, and when capture last came back.
#[derive(Default)]
pub struct SourceRestarts {
    tracker: RestartTracker,
    came_back: Option<Instant>,
}

impl SourceRestarts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture came back at `at` with no restart, on another input: the
    /// period that makes the source healthy starts again.
    pub fn came_back_at(&mut self, at: Instant) {
        self.came_back = Some(at);
    }
}

/// Restart after a loss already recorded against nothing: each restart,
/// including the first, is counted, waited for, then attempted. The restarts
/// are the caller's, so a source that fails again soon after coming back
/// spends the same budget (AE15). A source that recorded for the budget's
/// `reset_after` since it came back was healthy, and gets a full budget.
pub fn recover<T, E: Display>(
    restarts: &mut SourceRestarts,
    budget: &RestartBudget,
    clock: &dyn Clock,
    mut attempt: impl FnMut() -> Result<T, E>,
) -> Recovery<T> {
    if let (Some(came_back), Some(healthy_after)) = (restarts.came_back, budget.reset_after)
        && clock.now().duration_since(came_back) >= healthy_after
    {
        restarts.tracker.reset();
        restarts.came_back = None;
    }
    let mut last = None;
    loop {
        if !restarts.tracker.record_consecutive_restart(budget) {
            return Recovery::Exhausted(last);
        }
        if !clock.sleep(source_restart_delay(restarts.tracker.count())) {
            return Recovery::Cancelled;
        }
        match attempt() {
            Ok(value) => {
                restarts.came_back = Some(clock.now());
                return Recovery::Resumed(value);
            }
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
        let mut restarts = SourceRestarts::new();
        let out = recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
            Ok::<_, String>(7)
        });
        assert_eq!(out, Recovery::Resumed(7));
        assert_eq!(*clock.slept.borrow(), [1]);
    }

    #[test]
    fn a_source_that_stays_gone_gets_three_attempts_then_the_budget_runs_out() {
        let clock = Fake::new();
        let mut restarts = SourceRestarts::new();
        let mut attempts = 0;
        let out = recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
            attempts += 1;
            Err::<(), _>("no input device")
        });
        assert_eq!(out, Recovery::Exhausted(Some("no input device".into())));
        assert_eq!(attempts, 3);
        assert_eq!(*clock.slept.borrow(), [1, 2, 4]);
    }

    #[test]
    fn slow_failed_attempts_still_run_out_the_budget() {
        // Each reopen fails at about the 5 s start limit, so the third
        // failure ends past the source window. From the tenth attempt the
        // source opens: a loop that lost count resumes instead of stopping.
        let clock = Fake::new();
        let mut restarts = SourceRestarts::new();
        let mut attempts = 0;
        let out = recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
            attempts += 1;
            clock.advance(Duration::from_millis(5_500));
            if attempts < 10 {
                Err("the microphone did not start")
            } else {
                Ok(())
            }
        });
        assert_eq!(
            out,
            Recovery::Exhausted(Some("the microphone did not start".into()))
        );
        assert_eq!(attempts, 3);
        assert_eq!(*clock.slept.borrow(), [1, 2, 4]);
    }

    #[test]
    fn a_flapping_source_spends_one_budget_across_its_comebacks() {
        // AE15: it reconnects each time and drops again four seconds later.
        let clock = Fake::new();
        let mut restarts = SourceRestarts::new();
        let mut outcomes = Vec::new();
        for _ in 0..4 {
            let out = recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
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

    /// Three losses that each come back spend the budget; then the clock
    /// moves `after` before the next loss of a source that stays gone.
    fn attempts_after_a_spent_budget(after: Duration) -> usize {
        let clock = Fake::new();
        let mut restarts = SourceRestarts::new();
        for _ in 0..3 {
            let out = recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
                Ok::<_, String>(())
            });
            assert_eq!(out, Recovery::Resumed(()));
        }
        clock.advance(after);
        let mut attempts = 0;
        recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
            attempts += 1;
            Err::<(), _>("gone")
        });
        attempts
    }

    #[test]
    fn a_source_healthy_for_the_reset_period_gets_a_full_budget_again() {
        assert_eq!(attempts_after_a_spent_budget(Duration::from_secs(30)), 3);
    }

    #[test]
    fn a_source_that_fails_again_before_it_was_healthy_gets_no_new_budget() {
        assert_eq!(attempts_after_a_spent_budget(Duration::from_secs(4)), 0);
    }

    #[test]
    fn a_comeback_on_another_input_starts_the_health_period_again() {
        // The spent source comes back at 0 s, moves to the default input at
        // 29 s, and the default fails at 31 s.
        let clock = Fake::new();
        let mut restarts = SourceRestarts::new();
        for _ in 0..3 {
            recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
                Ok::<_, String>(())
            });
        }
        clock.advance(Duration::from_secs(29));
        restarts.came_back_at(clock.now());
        clock.advance(Duration::from_secs(2));
        let mut attempts = 0;
        let out = recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
            attempts += 1;
            Err::<(), _>("gone")
        });
        assert_eq!(out, Recovery::Exhausted(None));
        assert_eq!(attempts, 0);
    }

    #[test]
    fn stopping_during_the_backoff_calls_the_attempt_off() {
        let clock = Fake {
            cancel_after: Some(1),
            ..Fake::new()
        };
        let mut restarts = SourceRestarts::new();
        let mut attempts = 0;
        let out = recover(&mut restarts, &SOURCE_RESTART_BUDGET, &clock, || {
            attempts += 1;
            Err::<(), _>("gone")
        });
        assert_eq!(out, Recovery::Cancelled);
        assert_eq!(attempts, 1);
    }
}
