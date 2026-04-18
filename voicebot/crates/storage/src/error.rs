use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("s3 error: {0}")]
    S3(String),

    #[error("object not found: {0}")]
    NotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;
