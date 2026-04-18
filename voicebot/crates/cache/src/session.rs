use redis::AsyncCommands;
use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::{client::RedisPool, error::Result};

const SESSION_TTL_SECS: u64 = 3600; // 1 hour
const SESSION_PREFIX: &str = "session:";

pub async fn set<T: Serialize>(
    pool: &mut RedisPool,
    session_id: Uuid,
    value: &T,
    ttl_secs: Option<u64>,
) -> Result<()> {
    let key = format!("{SESSION_PREFIX}{session_id}");
    let json = serde_json::to_string(value)?;
    let ttl = ttl_secs.unwrap_or(SESSION_TTL_SECS);
    pool.set_ex::<_, _, ()>(&key, json, ttl).await?;
    Ok(())
}

pub async fn get<T: DeserializeOwned>(pool: &mut RedisPool, session_id: Uuid) -> Result<Option<T>> {
    let key = format!("{SESSION_PREFIX}{session_id}");
    let raw: Option<String> = pool.get(&key).await?;
    match raw {
        Some(s) => Ok(Some(serde_json::from_str(&s)?)),
        None => Ok(None),
    }
}

pub async fn del(pool: &mut RedisPool, session_id: Uuid) -> Result<()> {
    let key = format!("{SESSION_PREFIX}{session_id}");
    pool.del::<_, ()>(&key).await?;
    Ok(())
}

pub async fn extend_ttl(pool: &mut RedisPool, session_id: Uuid, ttl_secs: u64) -> Result<()> {
    let key = format!("{SESSION_PREFIX}{session_id}");
    pool.expire::<_, ()>(&key, ttl_secs as i64).await?;
    Ok(())
}

pub async fn set_field<T: Serialize>(
    pool: &mut RedisPool,
    session_id: Uuid,
    field: &str,
    value: &T,
) -> Result<()> {
    let key = format!("{SESSION_PREFIX}{session_id}");
    let json = serde_json::to_string(value)?;
    pool.hset::<_, _, _, ()>(&key, field, json).await?;
    Ok(())
}

pub async fn get_field<T: DeserializeOwned>(
    pool: &mut RedisPool,
    session_id: Uuid,
    field: &str,
) -> Result<Option<T>> {
    let key = format!("{SESSION_PREFIX}{session_id}");
    let raw: Option<String> = pool.hget(&key, field).await?;
    match raw {
        Some(s) => Ok(Some(serde_json::from_str(&s)?)),
        None => Ok(None),
    }
}

pub async fn del_field(pool: &mut RedisPool, session_id: Uuid, field: &str) -> Result<()> {
    let key = format!("{SESSION_PREFIX}{session_id}");
    pool.hdel::<_, _, ()>(&key, field).await?;
    Ok(())
}
