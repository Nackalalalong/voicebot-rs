use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("invalid frame size: {0} bytes (expected even)")]
    InvalidFrameSize(usize),

    #[error("invalid sample rate: {0} Hz")]
    InvalidSampleRate(u32),

    #[error("audio resampler error: {0}")]
    AudioResampler(String),

    #[error("invalid JSON message: {0}")]
    InvalidJson(String),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] axum::Error),

    #[error("session start timeout")]
    SessionStartTimeout,

    #[error("session error: {0}")]
    Session(String),
}
