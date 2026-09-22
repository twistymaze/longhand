//! Restart budgeting for the capture path (KTD5, R5b).
//!
//! `restart.rs` is copied from anarlog's `crates/supervisor`; `retry.rs` is
//! that crate's backoff loop with the `ractor` actor types removed; the budget
//! constants are anarlog's `listener-core/.../supervisor/children.rs`. The
//! actor framework itself is not taken. See PROVENANCE.md.

mod restart;
mod retry;

use std::time::Duration;

pub use restart::{RestartBudget, RestartTracker};
pub use retry::{RetryStrategy, retry_with_backoff};

const MAX_RESTARTS: u32 = 3;
const RESTART_FAILURE_WINDOW_SECS: u64 = 15;
const SOURCE_RESTART_BACKOFF_ALLOWANCE_SECS: u64 = 1 + 2 + 4;

/// The capture source: its window also allows for the backoff sleeps between
/// restarts, so three quick failures are judged on failure time alone.
pub const SOURCE_RESTART_BUDGET: RestartBudget = RestartBudget {
    max_restarts: MAX_RESTARTS,
    max_window: Duration::from_secs(
        RESTART_FAILURE_WINDOW_SECS + SOURCE_RESTART_BACKOFF_ALLOWANCE_SECS,
    ),
    reset_after: Some(Duration::from_secs(30)),
};

/// The chunk writer.
pub const RECORDER_RESTART_BUDGET: RestartBudget = RestartBudget {
    max_restarts: MAX_RESTARTS,
    max_window: Duration::from_secs(RESTART_FAILURE_WINDOW_SECS),
    reset_after: Some(Duration::from_secs(30)),
};

pub const RETRY_STRATEGY: RetryStrategy = RetryStrategy {
    max_attempts: 3,
    base_delay: Duration::from_millis(100),
};

/// Delay before the nth restart of the source: 1 s, 2 s, then 4 s thereafter.
pub fn source_restart_delay(restart_count: u32) -> Duration {
    Duration::from_secs(1 << restart_count.saturating_sub(1).min(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_restart_backoff_is_capped() {
        assert_eq!(source_restart_delay(1), Duration::from_secs(1));
        assert_eq!(source_restart_delay(2), Duration::from_secs(2));
        assert_eq!(source_restart_delay(3), Duration::from_secs(4));
        assert_eq!(source_restart_delay(10), Duration::from_secs(4));
    }

    #[test]
    fn source_restart_budget_excludes_backoff() {
        let total_backoff = (1..=SOURCE_RESTART_BUDGET.max_restarts)
            .map(source_restart_delay)
            .sum::<Duration>();
        assert_eq!(
            SOURCE_RESTART_BUDGET.max_window,
            RECORDER_RESTART_BUDGET.max_window + total_backoff
        );
    }
}

#[cfg(test)]
mod budget_tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn source_budget_allows_three_restarts_in_22_seconds_then_refuses() {
        let t0 = Instant::now();
        let mut tracker = RestartTracker::new_at(t0);
        for secs in [0, 10, 21] {
            assert!(
                tracker.record_restart_at(&SOURCE_RESTART_BUDGET, t0 + Duration::from_secs(secs))
            );
        }
        assert!(!tracker.record_restart_at(&SOURCE_RESTART_BUDGET, t0 + Duration::from_secs(22)));
    }

    #[test]
    fn source_budget_clears_after_31_quiet_seconds() {
        let t0 = Instant::now();
        let mut tracker = RestartTracker::new_at(t0);
        for _ in 0..3 {
            tracker.record_restart_at(&SOURCE_RESTART_BUDGET, t0);
        }
        tracker.maybe_reset_at(&SOURCE_RESTART_BUDGET, t0 + Duration::from_secs(31));
        assert_eq!(tracker.count(), 0);
        assert!(tracker.record_restart_at(&SOURCE_RESTART_BUDGET, t0 + Duration::from_secs(31)));
    }
}
