use redis::AsyncCommands;
use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::{client::RedisPool, error::Result};

const CAMPAIGN_CONFIG_TTL_SECS: u64 = 300; // 5 minutes
const CAMPAIGN_PREFIX: &str = "campaign:config:";

/// Cache campaign config to avoid DB lookups on every call.
pub async fn set_config<T: Serialize>(
    pool: &mut RedisPool,
    campaign_id: Uuid,
    config: &T,
) -> Result<()> {
    let key = format!("{CAMPAIGN_PREFIX}{campaign_id}");
    let json = serde_json::to_string(config)?;
    pool.set_ex::<_, _, ()>(&key, json, CAMPAIGN_CONFIG_TTL_SECS)
        .await?;
    Ok(())
}

pub async fn get_config<T: DeserializeOwned>(
    pool: &mut RedisPool,
    campaign_id: Uuid,
) -> Result<Option<T>> {
    let key = format!("{CAMPAIGN_PREFIX}{campaign_id}");
    let raw: Option<String> = pool.get(&key).await?;
    match raw {
        Some(s) => Ok(Some(serde_json::from_str(&s)?)),
        None => Ok(None),
    }
}

pub async fn invalidate(pool: &mut RedisPool, campaign_id: Uuid) -> Result<()> {
    let key = format!("{CAMPAIGN_PREFIX}{campaign_id}");
    pool.del::<_, ()>(&key).await?;
    Ok(())
}

/// Atomic active-call counter per campaign.
pub async fn increment_active_calls(pool: &mut RedisPool, campaign_id: Uuid) -> Result<i64> {
    let key = format!("campaign:active_calls:{campaign_id}");
    let count: i64 = pool.incr(&key, 1).await?;
    pool.expire::<_, ()>(&key, 86400).await?;
    Ok(count)
}

pub async fn decrement_active_calls(pool: &mut RedisPool, campaign_id: Uuid) -> Result<i64> {
    let key = format!("campaign:active_calls:{campaign_id}");
    let count: i64 = pool.decr(&key, 1).await?;
    Ok(count.max(0))
}

pub async fn get_active_calls(pool: &mut RedisPool, campaign_id: Uuid) -> Result<i64> {
    let key = format!("campaign:active_calls:{campaign_id}");
    let count: Option<i64> = pool.get(&key).await?;
    Ok(count.unwrap_or(0))
}
