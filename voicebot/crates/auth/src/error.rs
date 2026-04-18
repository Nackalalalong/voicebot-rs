use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("token expired")]
    TokenExpired,

    #[error("invalid token: {0}")]
    InvalidToken(String),

    #[error("missing token")]
    MissingToken,

    #[error("insufficient permissions")]
    Forbidden,

    #[error("hashing error: {0}")]
    Hashing(String),

    #[error("database error: {0}")]
    Db(#[from] db::DbError),
}

pub type Result<T> = std::result::Result<T, AuthError>;
