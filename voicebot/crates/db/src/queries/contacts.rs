use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, Result},
    models::Contact,
};

pub struct CreateContact<'a> {
    pub tenant_id: Uuid,
    pub campaign_id: Uuid,
    pub phone_number: &'a str,
    pub first_name: Option<&'a str>,
    pub last_name: Option<&'a str>,
    pub metadata: serde_json::Value,
}

pub async fn create(pool: &PgPool, req: CreateContact<'_>) -> Result<Contact> {
    let contact = sqlx::query_as::<_, Contact>(
        r#"
        INSERT INTO contacts (
            id, tenant_id, campaign_id, phone_number, first_name, last_name,
            metadata, status, retry_count, created_at, updated_at
        ) VALUES (
            gen_random_uuid(), $1, $2, $3, $4, $5, $6, 'pending', 0, now(), now()
        )
        RETURNING *
        "#,
    )
    .bind(req.tenant_id)
    .bind(req.campaign_id)
    .bind(req.phone_number)
    .bind(req.first_name)
    .bind(req.last_name)
    .bind(req.metadata)
    .fetch_one(pool)
    .await?;
    Ok(contact)
}

pub async fn bulk_create(
    pool: &PgPool,
    tenant_id: Uuid,
    campaign_id: Uuid,
    contacts: Vec<CreateContact<'_>>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let mut count = 0u64;
    for c in contacts {
        sqlx::query(
            r#"
            INSERT INTO contacts (
                id, tenant_id, campaign_id, phone_number, first_name, last_name,
                metadata, status, retry_count, created_at, updated_at
            ) VALUES (
                gen_random_uuid(), $1, $2, $3, $4, $5, $6, 'pending', 0, now(), now()
            )
            ON CONFLICT (tenant_id, campaign_id, phone_number) DO NOTHING
            "#,
        )
        .bind(tenant_id)
        .bind(campaign_id)
        .bind(c.phone_number)
        .bind(c.first_name)
        .bind(c.last_name)
        .bind(c.metadata)
        .execute(&mut *tx)
        .await?;
        count += 1;
    }
    tx.commit().await?;
    Ok(count)
}

pub async fn get_by_id(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<Contact> {
    sqlx::query_as::<_, Contact>("SELECT * FROM contacts WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)
}

pub async fn list_by_campaign(
    pool: &PgPool,
    tenant_id: Uuid,
    campaign_id: Uuid,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Contact>> {
    let rows = match status {
        Some(s) => sqlx::query_as::<_, Contact>(
            r#"SELECT * FROM contacts WHERE tenant_id = $1 AND campaign_id = $2 AND status = $3
               ORDER BY created_at DESC LIMIT $4 OFFSET $5"#,
        )
        .bind(tenant_id)
        .bind(campaign_id)
        .bind(s)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?,
        None => sqlx::query_as::<_, Contact>(
            "SELECT * FROM contacts WHERE tenant_id = $1 AND campaign_id = $2 ORDER BY created_at DESC LIMIT $3 OFFSET $4",
        )
        .bind(tenant_id)
        .bind(campaign_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?,
    };
    Ok(rows)
}

pub async fn count_by_campaign(pool: &PgPool, tenant_id: Uuid, campaign_id: Uuid) -> Result<i64> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM contacts WHERE tenant_id = $1 AND campaign_id = $2")
            .bind(tenant_id)
            .bind(campaign_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0)
}

/// Fetch next pending contact for dialing (used by scheduler).
pub async fn claim_next_pending(
    pool: &PgPool,
    tenant_id: Uuid,
    campaign_id: Uuid,
) -> Result<Option<Contact>> {
    let contact = sqlx::query_as::<_, Contact>(
        r#"
        UPDATE contacts SET status = 'claimed', updated_at = now()
        WHERE id = (
            SELECT id FROM contacts
            WHERE tenant_id = $1 AND campaign_id = $2
              AND status = 'pending'
              AND (next_attempt_at IS NULL OR next_attempt_at <= now())
            ORDER BY next_attempt_at ASC NULLS FIRST
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(campaign_id)
    .fetch_optional(pool)
    .await?;
    Ok(contact)
}

pub async fn update_status(pool: &PgPool, tenant_id: Uuid, id: Uuid, status: &str) -> Result<()> {
    let rows = sqlx::query(
        "UPDATE contacts SET status = $1, updated_at = now() WHERE id = $2 AND tenant_id = $3",
    )
    .bind(status)
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

pub async fn mark_failed_retry(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
    next_attempt_at: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE contacts
        SET status = 'pending', retry_count = retry_count + 1,
            last_attempt_at = now(), next_attempt_at = $1, updated_at = now()
        WHERE id = $2 AND tenant_id = $3
        "#,
    )
    .bind(next_attempt_at)
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}
