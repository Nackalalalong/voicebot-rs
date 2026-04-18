use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, Result},
    models::PhoneNumber,
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
    .fetch_one(pool)
    .await?;
    Ok(pn)
}

pub async fn get_by_id(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<PhoneNumber> {
    sqlx::query_as::<_, PhoneNumber>(
        "SELECT * FROM phone_numbers WHERE id = $1 AND tenant_id = $2",
    )
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
) -> Result<Vec<PhoneNumber>> {
    let rows = sqlx::query_as::<_, PhoneNumber>(
        "SELECT * FROM phone_numbers WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(tenant_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count(pool: &PgPool, tenant_id: Uuid) -> Result<i64> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM phone_numbers WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0)
}

pub async fn delete(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<()> {
    let rows = sqlx::query("DELETE FROM phone_numbers WHERE id = $1 AND tenant_id = $2")
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
