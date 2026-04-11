use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session error: {0}")]
    Internal(String),
}
