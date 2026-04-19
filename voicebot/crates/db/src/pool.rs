use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};
use tracing::info;
use uuid::Uuid;

pub async fn connect(database_url: &str) -> crate::Result<PgPool> {
    info!("connecting to postgres");
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .min_connections(2)
        .connect(database_url)
        .await?;
    Ok(pool)
}

pub async fn run_migrations(pool: &PgPool) -> crate::Result<()> {
    info!("running database migrations");
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

pub async fn begin_tenant_tx(
    pool: &PgPool,
    tenant_id: Uuid,
) -> crate::Result<Transaction<'_, Postgres>> {
    let mut tx = pool.begin().await?;
    set_tenant_context(&mut tx, tenant_id).await?;
    Ok(tx)
}

pub async fn set_tenant_context(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> crate::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
