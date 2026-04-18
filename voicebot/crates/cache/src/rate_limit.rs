use redis::AsyncCommands;

use crate::{client::RedisPool, error::Result};

/// Token bucket rate limiter using Redis.
/// Returns `true` if the request is allowed, `false` if rate-limited.
pub async fn check_rate_limit(
    pool: &mut RedisPool,
    key: &str,
    max_requests: i64,
    window_secs: u64,
) -> Result<bool> {
    let redis_key = format!("rate_limit:{key}");
    let count: i64 = pool.incr(&redis_key, 1).await?;
    if count == 1 {
        pool.expire::<_, ()>(&redis_key, window_secs as i64).await?;
    }
    Ok(count <= max_requests)
}

pub async fn get_remaining(
    pool: &mut RedisPool,
    key: &str,
    max_requests: i64,
) -> Result<i64> {
    let redis_key = format!("rate_limit:{key}");
    let count: Option<i64> = pool.get(&redis_key).await?;
    Ok((max_requests - count.unwrap_or(0)).max(0))
}
