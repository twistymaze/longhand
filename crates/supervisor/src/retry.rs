use std::future::Future;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RetryStrategy {
    pub max_attempts: u32,
    pub base_delay: Duration,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
        }
    }
}

/// Run `attempt_fn` with exponential-backoff retries.
/// Returns `Some(value)` on first success, `None` if all attempts fail.
pub async fn retry_with_backoff<T, E, F, Fut>(strategy: &RetryStrategy, attempt_fn: F) -> Option<T>
where
    E: std::fmt::Debug,
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    for attempt in 0..strategy.max_attempts {
        let delay = strategy.base_delay * 2u32.pow(attempt);
        tokio::time::sleep(delay).await;

        match attempt_fn().await {
            Ok(value) => {
                log::info!("retry_succeeded attempt={attempt}");
                return Some(value);
            }
            Err(e) => {
                log::warn!("retry_failed attempt={attempt} error={e:?}");
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[tokio::test(start_paused = true)]
    async fn zero_attempts_never_calls_attempt_fn() {
        let strategy = RetryStrategy {
            max_attempts: 0,
            base_delay: Duration::from_millis(5),
        };
        let calls = Arc::new(AtomicU64::new(0));
        let calls2 = calls.clone();

        let result: Option<()> = retry_with_backoff(&strategy, move || {
            calls2.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), &str>("must not be called") }
        })
        .await;

        assert!(result.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_until_success() {
        let attempts = Arc::new(AtomicU64::new(0));
        let attempts2 = attempts.clone();
        let strategy = RetryStrategy {
            max_attempts: 4,
            base_delay: Duration::from_millis(10),
        };

        let start = tokio::time::Instant::now();
        let result = retry_with_backoff(&strategy, move || {
            let attempts = attempts2.clone();
            async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    Err("not yet")
                } else {
                    Ok(attempt)
                }
            }
        })
        .await;

        assert_eq!(result, Some(3));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        // 10 + 20 + 40 ms of backoff precede the third attempt.
        assert!(start.elapsed() >= Duration::from_millis(70));
    }

    #[tokio::test(start_paused = true)]
    async fn returns_none_after_exhausting_attempts() {
        let attempts = Arc::new(AtomicU64::new(0));
        let attempts2 = attempts.clone();
        let strategy = RetryStrategy {
            max_attempts: 3,
            base_delay: Duration::from_millis(10),
        };

        let result: Option<()> = retry_with_backoff(&strategy, move || {
            let attempts = attempts2.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err("always")
            }
        })
        .await;

        assert!(result.is_none());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
