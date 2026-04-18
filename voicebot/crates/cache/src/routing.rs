use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{client::RedisPool, error::Result};

const PHONE_ROUTING_PREFIX: &str = "phone_routing:";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhoneRoute {
    pub tenant_id: Uuid,
    pub campaign_id: Uuid,
}

/// Store a phone-number → campaign mapping in Redis.
/// Called by the API when a phone number is assigned to a campaign.
pub async fn set_route(pool: &mut RedisPool, phone_number: &str, route: &PhoneRoute) -> Result<()> {
    let key = format!("{PHONE_ROUTING_PREFIX}{phone_number}");
    let json = serde_json::to_string(route)?;
    // No TTL — routing entries are permanent until explicitly removed.
    pool.set::<_, _, ()>(&key, json).await?;
    Ok(())
}

/// Look up the campaign for an incoming phone number.
/// Returns None if no mapping is configured.
pub async fn get_route(pool: &mut RedisPool, phone_number: &str) -> Result<Option<PhoneRoute>> {
    let key = format!("{PHONE_ROUTING_PREFIX}{phone_number}");
    let raw: Option<String> = pool.get(&key).await?;
    match raw {
        Some(s) => Ok(Some(serde_json::from_str(&s)?)),
        None => Ok(None),
    }
}

/// Remove a phone-number routing entry.
/// Called when a phone number is unassigned or deleted.
pub async fn del_route(pool: &mut RedisPool, phone_number: &str) -> Result<()> {
    let key = format!("{PHONE_ROUTING_PREFIX}{phone_number}");
    pool.del::<_, ()>(&key).await?;
    Ok(())
}
