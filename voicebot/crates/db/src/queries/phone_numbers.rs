use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, Result},
    models::PhoneNumber,
    pool::begin_tenant_tx,
};

pub async fn create(
    pool: &PgPool,
    tenant_id: Uuid,
    number: &str,
    provider: &str,
    provider_number_id: Option<&str>,
    capabilities: serde_json::Value,
    monthly_cost_usd_cents: Option<i64>,
) -> Result<PhoneNumber> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let pn = sqlx::query_as::<_, PhoneNumber>(
        r#"
        INSERT INTO phone_numbers (
            id, tenant_id, number, provider, provider_number_id,
            status, capabilities, monthly_cost_usd_cents, created_at, updated_at
        ) VALUES (
            gen_random_uuid(), $1, $2, $3, $4, 'active', $5, $6, now(), now()
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(number)
    .bind(provider)
    .bind(provider_number_id)
    .bind(capabilities)
    .bind(monthly_cost_usd_cents)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(pn)
}

pub async fn get_by_id(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<PhoneNumber> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let phone_number = sqlx::query_as::<_, PhoneNumber>("SELECT * FROM phone_numbers WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DbError::NotFound)?;
    tx.commit().await?;
    Ok(phone_number)
}

pub async fn list(
    pool: &PgPool,
    tenant_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<PhoneNumber>> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let rows = sqlx::query_as::<_, PhoneNumber>(
        "SELECT * FROM phone_numbers WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(tenant_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn count(pool: &PgPool, tenant_id: Uuid) -> Result<i64> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM phone_numbers WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row.0)
}

pub async fn delete(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<()> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let rows = sqlx::query("DELETE FROM phone_numbers WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if rows == 0 {
        return Err(DbError::NotFound);
    }
    tx.commit().await?;
    Ok(())
}

/// Assign a phone number to a campaign (also returns the updated row).
pub async fn assign_campaign(
    pool: &PgPool,
    tenant_id: Uuid,
    phone_number_id: Uuid,
    campaign_id: Uuid,
) -> Result<PhoneNumber> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let phone_number = sqlx::query_as::<_, PhoneNumber>(
        "UPDATE phone_numbers SET campaign_id = $1, updated_at = now()
         WHERE id = $2 AND tenant_id = $3
         RETURNING *",
    )
    .bind(campaign_id)
    .bind(phone_number_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    tx.commit().await?;
    Ok(phone_number)
}

/// Remove a phone number's campaign assignment.
pub async fn unassign_campaign(
    pool: &PgPool,
    tenant_id: Uuid,
    phone_number_id: Uuid,
) -> Result<PhoneNumber> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let phone_number = sqlx::query_as::<_, PhoneNumber>(
        "UPDATE phone_numbers SET campaign_id = NULL, updated_at = now()
         WHERE id = $1 AND tenant_id = $2
         RETURNING *",
    )
    .bind(phone_number_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    tx.commit().await?;
    Ok(phone_number)
}
