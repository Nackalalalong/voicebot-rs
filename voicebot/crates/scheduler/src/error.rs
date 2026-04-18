use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("database error: {0}")]
    Db(#[from] db::DbError),

    #[error("cache error: {0}")]
    Cache(#[from] cache::CacheError),

    #[error("http error: {0}")]
    Http(String),

    #[error("job failed: {0}")]
    JobFailed(String),
}

pub type Result<T> = std::result::Result<T, SchedulerError>;
