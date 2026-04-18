use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, Result},
    models::Campaign,
};

pub struct CreateCampaign<'a> {
    pub tenant_id: Uuid,
    pub name: &'a str,
    pub system_prompt: &'a str,
    pub language: &'a str,
    pub voice_id: Option<&'a str>,
    pub asr_provider: &'a str,
    pub tts_provider: &'a str,
    pub llm_provider: &'a str,
    pub llm_model: &'a str,
    pub max_call_duration_secs: i32,
    pub recording_enabled: bool,
    pub tools_config: serde_json::Value,
    pub custom_metrics: serde_json::Value,
    pub schedule_config: serde_json::Value,
}

pub async fn create(pool: &PgPool, req: CreateCampaign<'_>) -> Result<Campaign> {
    let campaign = sqlx::query_as::<_, Campaign>(
        r#"
        INSERT INTO campaigns (
            id, tenant_id, name, status, system_prompt, language, voice_id,
            asr_provider, tts_provider, llm_provider, llm_model,
            max_call_duration_secs, recording_enabled,
            tools_config, custom_metrics, schedule_config, created_at, updated_at
        ) VALUES (
            gen_random_uuid(), $1, $2, 'draft', $3, $4, $5,
            $6, $7, $8, $9, $10, $11, $12, $13, $14, now(), now()
        )
        RETURNING *
        "#,
    )
    .bind(req.tenant_id)
    .bind(req.name)
    .bind(req.system_prompt)
    .bind(req.language)
    .bind(req.voice_id)
    .bind(req.asr_provider)
    .bind(req.tts_provider)
    .bind(req.llm_provider)
    .bind(req.llm_model)
    .bind(req.max_call_duration_secs)
    .bind(req.recording_enabled)
    .bind(req.tools_config)
    .bind(req.custom_metrics)
    .bind(req.schedule_config)
    .fetch_one(pool)
    .await?;
    Ok(campaign)
}

pub async fn get_by_id(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<Campaign> {
    sqlx::query_as::<_, Campaign>("SELECT * FROM campaigns WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)
}

pub async fn list(
    pool: &PgPool,
    tenant_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<Campaign>> {
    let rows = sqlx::query_as::<_, Campaign>(
        "SELECT * FROM campaigns WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(tenant_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count(pool: &PgPool, tenant_id: Uuid) -> Result<i64> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM campaigns WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

pub async fn update_status(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
    status: &str,
) -> Result<Campaign> {
    sqlx::query_as::<_, Campaign>(
        "UPDATE campaigns SET status = $1, updated_at = now() WHERE id = $2 AND tenant_id = $3 RETURNING *",
    )
    .bind(status)
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn update_prompt(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
    system_prompt: &str,
) -> Result<Campaign> {
    sqlx::query_as::<_, Campaign>(
        "UPDATE campaigns SET system_prompt = $1, updated_at = now() WHERE id = $2 AND tenant_id = $3 RETURNING *",
    )
    .bind(system_prompt)
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn delete(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<()> {
    let rows = sqlx::query("DELETE FROM campaigns WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?
        .rows_affected();
    if rows == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
