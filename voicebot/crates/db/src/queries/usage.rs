use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{error::Result, models::UsageRecord, pool::begin_tenant_tx};

pub async fn record_call(
    pool: &PgPool,
    tenant_id: Uuid,
    campaign_id: Option<Uuid>,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    duration_secs: i64,
    asr_seconds: i64,
    tts_characters: i64,
    llm_tokens: i64,
    cost_usd_cents: i64,
) -> Result<UsageRecord> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let record = sqlx::query_as::<_, UsageRecord>(
        r#"
        INSERT INTO usage_records (
            id, tenant_id, campaign_id, period_start, period_end,
            call_count, total_duration_secs, asr_seconds, tts_characters,
            llm_tokens, cost_usd_cents, created_at
        ) VALUES (
            gen_random_uuid(), $1, $2, $3, $4, 1, $5, $6, $7, $8, $9, now()
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(campaign_id)
    .bind(period_start)
    .bind(period_end)
    .bind(duration_secs)
    .bind(asr_seconds)
    .bind(tts_characters)
    .bind(llm_tokens)
    .bind(cost_usd_cents)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(record)
}

pub async fn aggregate(
    pool: &PgPool,
    tenant_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<UsageSummary> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let row = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(call_count), 0),
            COALESCE(SUM(total_duration_secs), 0),
            COALESCE(SUM(asr_seconds), 0),
            COALESCE(SUM(tts_characters), 0),
            COALESCE(SUM(llm_tokens), 0),
            COALESCE(SUM(cost_usd_cents), 0)
        FROM usage_records
        WHERE tenant_id = $1 AND period_start >= $2 AND period_end <= $3
        "#,
    )
    .bind(tenant_id)
    .bind(from)
    .bind(to)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(UsageSummary {
        call_count: row.0,
        total_duration_secs: row.1,
        asr_seconds: row.2,
        tts_characters: row.3,
        llm_tokens: row.4,
        cost_usd_cents: row.5,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageSummary {
    pub call_count: i64,
    pub total_duration_secs: i64,
    pub asr_seconds: i64,
    pub tts_characters: i64,
    pub llm_tokens: i64,
    pub cost_usd_cents: i64,
}
