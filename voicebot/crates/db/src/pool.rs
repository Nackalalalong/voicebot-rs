use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::info;

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
