use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, Result},
    models::User,
    pool::begin_tenant_tx,
};

pub async fn create(
    pool: &PgPool,
    tenant_id: Uuid,
    email: &str,
    password_hash: &str,
    display_name: &str,
    role: &str,
) -> Result<User> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let user = sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (id, tenant_id, email, password_hash, display_name, role, is_active, created_at, updated_at)
        VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, true, now(), now())
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .bind(role)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
            DbError::Duplicate(format!("email '{email}' already exists in this tenant"))
        }
        e => DbError::Sqlx(e),
    })?;
    tx.commit().await?;
    Ok(user)
}

pub async fn get_by_id(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<User> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE id = $1 AND tenant_id = $2 AND is_active = true",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    tx.commit().await?;
    Ok(user)
}

pub async fn get_by_email(pool: &PgPool, tenant_id: Uuid, email: &str) -> Result<User> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE email = $1 AND tenant_id = $2 AND is_active = true",
    )
    .bind(email)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    tx.commit().await?;
    Ok(user)
}

pub async fn list(pool: &PgPool, tenant_id: Uuid, limit: i64, offset: i64) -> Result<Vec<User>> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let rows = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE tenant_id = $1 AND is_active = true ORDER BY created_at DESC LIMIT $2 OFFSET $3",
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
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM users WHERE tenant_id = $1 AND is_active = true")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(row.0)
}

pub async fn update_password(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
    password_hash: &str,
) -> Result<()> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let rows = sqlx::query(
        "UPDATE users SET password_hash = $1, updated_at = now() WHERE id = $2 AND tenant_id = $3 AND is_active = true",
    )
    .bind(password_hash)
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

pub async fn deactivate(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> Result<()> {
    let mut tx = begin_tenant_tx(pool, tenant_id).await?;
    let rows = sqlx::query(
        "UPDATE users SET is_active = false, updated_at = now() WHERE id = $1 AND tenant_id = $2 AND is_active = true",
    )
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
