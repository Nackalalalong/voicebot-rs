use std::fmt::Debug;
use std::future::Future;
use std::time::Duration;

/// Retry an async operation with exponential backoff.
///
/// - `max_attempts`: total attempts (1 = no retry)
/// - `base_delay_ms`: initial delay between retries
/// - `f`: closure producing the future to retry
pub async fn with_retry<F, Fut, T, E>(max_attempts: u32, base_delay_ms: u64, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: Debug,
{
    let mut attempt = 0u32;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt + 1 >= max_attempts => return Err(e),
            Err(e) => {
                tracing::warn!(attempt, error = ?e, "retrying after error");
                let delay = base_delay_ms * 2u64.pow(attempt);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_with_retry_succeeds_first_try() {
        let result: Result<i32, &str> = with_retry(3, 10, || async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_with_retry_succeeds_after_failures() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result: Result<i32, &str> = with_retry(3, 10, move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err("not yet")
                } else {
                    Ok(99)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 99);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_with_retry_exhausted() {
        let result: Result<i32, &str> = with_retry(2, 10, || async { Err("fail") }).await;
        assert!(result.is_err());
    }
}
