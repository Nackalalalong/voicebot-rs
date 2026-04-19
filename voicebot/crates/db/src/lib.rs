pub mod error;
pub mod models;
pub mod pool;
pub mod queries;

pub use error::{DbError, Result};
pub use pool::{begin_tenant_tx, connect, run_migrations, set_tenant_context};
pub use sqlx::PgPool;

/// Ping the database — returns Err if unreachable.
pub async fn health_check(pool: &PgPool) -> Result<()> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
