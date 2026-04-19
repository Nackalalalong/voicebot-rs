use std::env;

use uuid::Uuid;

#[tokio::test]
#[ignore = "requires REDIS_URL"]
async fn session_and_campaign_cache_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let redis_url = env::var("REDIS_URL")?;
    let mut redis = cache::connect(&redis_url).await?;
    let session_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();

    let payload = serde_json::json!({"hello": "world"});
    cache::session::set(&mut redis, session_id, &payload, Some(60)).await?;
    let session_value: Option<serde_json::Value> = cache::session::get(&mut redis, session_id).await?;
    assert_eq!(session_value, Some(payload.clone()));

    cache::campaign::set_config(&mut redis, campaign_id, &payload).await?;
    let campaign_value: Option<serde_json::Value> = cache::campaign::get_config(&mut redis, campaign_id).await?;
    assert_eq!(campaign_value, Some(payload));

    cache::session::del(&mut redis, session_id).await?;
    cache::campaign::invalidate(&mut redis, campaign_id).await?;

    Ok(())
}