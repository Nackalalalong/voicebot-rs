use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, Result},
    models::CallRecord,
};

pub struct CreateCallRecord<'a> {
    pub tenant_id: Uuid,
    pub campaign_id: Option<Uuid>,
    pub contact_id: Option<Uuid>,
    pub session_id: &'a str,
    pub direction: &'a str,
    pub phone_number: &'a str,
}

pub async fn create(pool: &PgPool, req: CreateCallRecord<'_>) -> Result<CallRecord> {
    let record = sqlx::query_as::<_, CallRecord>(
        r#"
        INSERT INTO call_records (
            id, tenant_id, campaign_id, contact_id, session_id,
            direction, phone_number, status, custom_metrics, created_at
        ) VALUES (
            gen_random_uuid(), $1, $2, $3, $4, $5, $6, 'initiated', '{}', now()
        )
        RETURNING *
        "#,
    )
    .bind(req.tenant_id)
    .bind(req.campaign_id)
    .bind(req.contact_id)
    .bind(req.session_id)
    .bind(req.direction)
    .bind(req.phone_number)
    .fetch_one(pool)
    .await?;
    Ok(record)
}

pub async fn get_by_id(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<CallRecord> {
    sqlx::query_as::<_, CallRecord>(
        "SELECT * FROM call_records WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn get_by_session_id(pool: &PgPool, session_id: &str) -> Result<Option<CallRecord>> {
    let record = sqlx::query_as::<_, CallRecord>(
        "SELECT * FROM call_records WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(record)
}

pub async fn list(
    pool: &PgPool,
    tenant_id: Uuid,
    campaign_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> Result<Vec<CallRecord>> {
    let rows = match campaign_id {
        Some(cid) => sqlx::query_as::<_, CallRecord>(
            "SELECT * FROM call_records WHERE tenant_id = $1 AND campaign_id = $2 ORDER BY created_at DESC LIMIT $3 OFFSET $4",
        )
        .bind(tenant_id)
        .bind(cid)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?,
        None => sqlx::query_as::<_, CallRecord>(
            "SELECT * FROM call_records WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?,
    };
    Ok(rows)
}

pub async fn count(pool: &PgPool, tenant_id: Uuid, campaign_id: Option<Uuid>) -> Result<i64> {
    let row: (i64,) = match campaign_id {
        Some(cid) => sqlx::query_as(
            "SELECT COUNT(*) FROM call_records WHERE tenant_id = $1 AND campaign_id = $2",
        )
        .bind(tenant_id)
        .bind(cid)
        .fetch_one(pool)
        .await?,
        None => sqlx::query_as("SELECT COUNT(*) FROM call_records WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(pool)
            .await?,
    };
    Ok(row.0)
}

pub async fn finalize(
    pool: &PgPool,
    tenant_id: Uuid,
    session_id: &str,
    status: &str,
    duration_secs: Option<i32>,
    recording_url: Option<&str>,
    transcript: Option<serde_json::Value>,
    custom_metrics: serde_json::Value,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE call_records
        SET status = $1, duration_secs = $2, recording_url = $3,
            transcript = $4, custom_metrics = $5,
            ended_at = now(), started_at = COALESCE(started_at, now())
        WHERE session_id = $6 AND tenant_id = $7
        "#,
    )
    .bind(status)
    .bind(duration_secs)
    .bind(recording_url)
    .bind(transcript)
    .bind(custom_metrics)
    .bind(session_id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_sentiment(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
    sentiment: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE call_records SET sentiment = $1 WHERE id = $2 AND tenant_id = $3",
    )
    .bind(sentiment)
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}
