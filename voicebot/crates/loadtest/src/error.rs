use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoadtestError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TOML parse: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("TOML serialize: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("WAV: {0}")]
    Wav(#[from] hound::Error),
    #[error("task join: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("missing required environment variables: {0:?}")]
    MissingEnvVars(Vec<String>),
    #[error("unsupported WAV format at {path}: {reason}")]
    UnsupportedWav { path: PathBuf, reason: String },
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("protocol: {0}")]
    Protocol(String),
}
