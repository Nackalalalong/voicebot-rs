use thiserror::Error;

#[derive(Debug, Error)]
pub enum AriError {
    #[error("ARI REST error {status}: {url}")]
    Rest { status: u16, url: String },
    #[error("ARI WebSocket: {0}")]
    WebSocket(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Protocol: {0}")]
    Protocol(String),
    #[error("HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("session error: {0}")]
    Session(String),
    #[error("AudioSocket connection timed out")]
    Timeout,
}
