use redis::{aio::ConnectionManager, Client};
use tracing::info;

pub type RedisPool = ConnectionManager;

pub async fn connect(redis_url: &str) -> crate::Result<RedisPool> {
    info!("connecting to redis");
    let client = Client::open(redis_url)?;
    let manager = ConnectionManager::new(client).await?;
    Ok(manager)
}
