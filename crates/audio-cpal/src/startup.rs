//! The microphone start: a time limit, a cancel, and a guard against a
//! second start while an earlier one is blocked in the driver. This follows
//! anarlog's `StartupPermit` and its `recv_timeout` wait; the cancel is
//! Longhand's own. See PROVENANCE.md.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::time::{Duration, Instant};

use audio::Error;

/// How long a microphone start can take (KTD3). anarlog uses the same value.
pub(crate) const START_LIMIT: Duration = Duration::from_secs(5);

/// True while a capture thread is in its start: the device lookup, the
/// config query, the stream build and `play`.
pub(crate) static MIC_STARTING: AtomicBool = AtomicBool::new(false);

/// How often the wait checks the cancel.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// The right to start the microphone (KTD4). Only one start can hold it.
/// The capture thread holds it until its driver calls return, also after
/// the caller stopped its wait.
struct Permit(&'static AtomicBool);

impl Permit {
    fn take(flag: &'static AtomicBool) -> Result<Self, Error> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self(flag))
            .map_err(|_| {
                Error::MicStreamInitializationFailed(
                    "an earlier microphone start has not returned".into(),
                )
            })
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// What the capture thread holds during its start. [`Starting::report`]
/// frees the permit and then sends the result. When the thread ends with
/// no report, the drop frees the permit and the wait gets an error.
pub(crate) struct Starting<T> {
    report: SyncSender<Result<T, Error>>,
    permit: Permit,
}

impl<T> Starting<T> {
    pub(crate) fn report(self, result: Result<T, Error>) {
        let Self { report, permit } = self;
        // Free the permit first: the caller can start again as soon as it
        // has the result.
        drop(permit);
        // A caller that stopped its wait is gone. The send then fails, and
        // the thread ends its capture.
        let _ = report.send(result);
    }
}

/// Run `body` on a new capture thread and wait for its report, for at most
/// `limit`. `cancelled` is checked before the start and during the wait.
/// The thread is not joined on a timeout or a cancel: it still waits for
/// the driver, and it ends when the driver returns.
pub(crate) fn start<T: Send + 'static>(
    flag: &'static AtomicBool,
    limit: Duration,
    cancelled: &dyn Fn() -> bool,
    body: impl FnOnce(Starting<T>) + Send + 'static,
) -> Result<T, Error> {
    if cancelled() {
        return Err(Error::MicStartCancelled);
    }
    let permit = Permit::take(flag)?;
    let (report, result) = sync_channel(1);
    let starting = Starting { report, permit };
    std::thread::Builder::new()
        .name("longhand-capture".into())
        .spawn(move || body(starting))
        .map_err(|e| Error::MicStreamInitializationFailed(e.to_string()))?;

    let deadline = Instant::now() + limit;
    loop {
        if cancelled() {
            return Err(Error::MicStartCancelled);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            let message = format!("the microphone did not start within {limit:?}");
            log::warn!("{message}");
            return Err(Error::MicStreamInitializationFailed(message));
        }
        match result.recv_timeout(left.min(CANCEL_POLL)) {
            Ok(reported) => return reported,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(Error::MicStreamInitializationFailed(
                    "capture thread exited".into(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use super::*;

    const LONG: Duration = Duration::from_secs(5);

    fn never() -> bool {
        false
    }

    /// A start body that blocks until the test sends on the returned
    /// sender, then reports `Ok(1)`.
    fn blocked() -> (
        mpsc::Sender<()>,
        impl FnOnce(Starting<u32>) + Send + 'static,
    ) {
        let (release, wait) = mpsc::channel::<()>();
        let body = move |starting: Starting<u32>| {
            let _ = wait.recv();
            starting.report(Ok(1));
        };
        (release, body)
    }

    #[test]
    fn a_start_that_blocks_before_its_report_times_out() {
        // AE1. The body stands for a device lookup or a stream build that
        // the driver does not return from.
        static FLAG: AtomicBool = AtomicBool::new(false);
        let (release, body) = blocked();
        let started = Instant::now();
        let result = start(&FLAG, Duration::from_millis(100), &never, body);
        let elapsed = started.elapsed();
        assert!(
            matches!(&result, Err(Error::MicStreamInitializationFailed(m)) if m.contains("did not start")),
            "{result:?}"
        );
        assert!(elapsed >= Duration::from_millis(100), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        let _ = release.send(());
    }

    #[test]
    fn a_cancel_ends_the_wait_before_the_limit() {
        // AE1: a Stop during the wait.
        static FLAG: AtomicBool = AtomicBool::new(false);
        let (release, body) = blocked();
        let started = Instant::now();
        let cancelled = move || started.elapsed() >= Duration::from_millis(50);
        let result = start(&FLAG, LONG, &cancelled, body);
        assert_eq!(result, Err(Error::MicStartCancelled));
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = release.send(());
    }

    #[test]
    fn a_second_start_fails_at_once_while_the_first_is_blocked() {
        // AE2.
        static FLAG: AtomicBool = AtomicBool::new(false);
        let (release, body) = blocked();
        let first = start(&FLAG, Duration::from_millis(50), &never, body);
        assert!(matches!(
            first,
            Err(Error::MicStreamInitializationFailed(_))
        ));

        let ran = Arc::new(AtomicBool::new(false));
        let second_ran = Arc::clone(&ran);
        let started = Instant::now();
        let second = start(&FLAG, LONG, &never, move |starting: Starting<u32>| {
            second_ran.store(true, Ordering::SeqCst);
            starting.report(Ok(2));
        });
        assert!(
            matches!(&second, Err(Error::MicStreamInitializationFailed(m)) if m.contains("earlier")),
            "{second:?}"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(!ran.load(Ordering::SeqCst), "the second body never runs");

        // The blocked body returns: the late thread frees the permit, and a
        // new start succeeds.
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let third = loop {
            let attempt = start(&FLAG, LONG, &never, |starting: Starting<u32>| {
                starting.report(Ok(3))
            });
            if attempt.is_ok() || Instant::now() > deadline {
                break attempt;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(third, Ok(3));
    }

    #[test]
    fn a_report_in_time_returns_its_result() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let ok = start(&FLAG, LONG, &never, |starting: Starting<u32>| {
            starting.report(Ok(7))
        });
        assert_eq!(ok, Ok(7));
        let failed = start(&FLAG, LONG, &never, |starting: Starting<u32>| {
            starting.report(Err(Error::MicOpenFailed))
        });
        assert_eq!(failed, Err(Error::MicOpenFailed));
        // A body that ends with no report is an error, and frees the permit.
        let silent = start(&FLAG, LONG, &never, |_: Starting<u32>| {});
        assert!(matches!(
            silent,
            Err(Error::MicStreamInitializationFailed(_))
        ));
        assert!(!FLAG.load(Ordering::SeqCst));
    }

    #[test]
    fn a_cancel_before_the_start_fails_at_once() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let ran = Arc::new(AtomicBool::new(false));
        let body_ran = Arc::clone(&ran);
        let result = start(&FLAG, LONG, &|| true, move |starting: Starting<u32>| {
            body_ran.store(true, Ordering::SeqCst);
            starting.report(Ok(1));
        });
        assert_eq!(result, Err(Error::MicStartCancelled));
        assert!(!ran.load(Ordering::SeqCst));
        assert!(!FLAG.load(Ordering::SeqCst), "no permit is taken");
    }
}
