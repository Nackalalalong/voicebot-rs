use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("record not found")]
    NotFound,

    #[error("duplicate record: {0}")]
    Duplicate(String),
}

pub type Result<T> = std::result::Result<T, DbError>;
