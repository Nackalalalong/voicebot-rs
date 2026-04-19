use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, Result},
    models::{Tenant, User},
    pool::set_tenant_context,
};

pub async fn create(pool: &PgPool, name: &str, slug: &str, plan: &str) -> Result<Tenant> {
    let tenant = sqlx::query_as::<_, Tenant>(
        r#"
        INSERT INTO tenants (id, name, slug, plan, is_active, created_at, updated_at)
        VALUES (gen_random_uuid(), $1, $2, $3, true, now(), now())
        RETURNING *
        "#,
    )
    .bind(name)
    .bind(slug)
    .bind(plan)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
            DbError::Duplicate(format!("tenant slug '{slug}' already exists"))
        }
        e => DbError::Sqlx(e),
    })?;
    Ok(tenant)
}

/// Create a tenant and its first owner user in a single transaction.
pub async fn create_with_owner(
    pool: &PgPool,
    org_name: &str,
    org_slug: &str,
    email: &str,
    password_hash: &str,
    display_name: &str,
) -> Result<(Tenant, User)> {
    let mut tx = pool.begin().await?;

    let tenant = sqlx::query_as::<_, Tenant>(
        r#"
        INSERT INTO tenants (id, name, slug, plan, is_active, created_at, updated_at)
        VALUES (gen_random_uuid(), $1, $2, 'starter', true, now(), now())
        RETURNING *
        "#,
    )
    .bind(org_name)
    .bind(org_slug)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
            DbError::Duplicate(format!("organisation slug '{org_slug}' already exists"))
        }
        e => DbError::Sqlx(e),
    })?;

    set_tenant_context(&mut tx, tenant.id).await?;

    let user = sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (id, tenant_id, email, password_hash, display_name, role, is_active, created_at, updated_at)
        VALUES (gen_random_uuid(), $1, $2, $3, $4, 'owner', true, now(), now())
        RETURNING *
        "#,
    )
    .bind(tenant.id)
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
            DbError::Duplicate(format!("email '{email}' already registered"))
        }
        e => DbError::Sqlx(e),
    })?;

    tx.commit().await?;
    Ok((tenant, user))
}

pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Tenant> {
    sqlx::query_as::<_, Tenant>("SELECT * FROM tenants WHERE id = $1 AND is_active = true")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)
}

pub async fn get_by_slug(pool: &PgPool, slug: &str) -> Result<Tenant> {
    sqlx::query_as::<_, Tenant>("SELECT * FROM tenants WHERE slug = $1 AND is_active = true")
        .bind(slug)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)
}

pub async fn list(pool: &PgPool, limit: i64, offset: i64) -> Result<Vec<Tenant>> {
    let rows = sqlx::query_as::<_, Tenant>(
        "SELECT * FROM tenants WHERE is_active = true ORDER BY created_at DESC LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count(pool: &PgPool) -> Result<i64> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tenants WHERE is_active = true")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

pub async fn deactivate(pool: &PgPool, id: Uuid) -> Result<()> {
    let rows = sqlx::query(
        "UPDATE tenants SET is_active = false, updated_at = now() WHERE id = $1 AND is_active = true",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    if rows == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
