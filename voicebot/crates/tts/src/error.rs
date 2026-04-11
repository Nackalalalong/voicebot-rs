use thiserror::Error;

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("channel closed")]
    ChannelClosed,
    #[error("synthesis failed: {0}")]
    SynthesisFailed(String),
}
