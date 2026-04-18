pub mod campaign;
pub mod client;
pub mod error;
pub mod rate_limit;
pub mod routing;
pub mod session;

pub use client::{connect, RedisPool};
pub use error::{CacheError, Result};
pub use routing::PhoneRoute;

/// Ping Redis — returns Err if unreachable.
pub async fn health_check(pool: &mut RedisPool) -> Result<()> {
    let _: String = redis::cmd("PING").query_async(pool).await?;
    Ok(())
}
