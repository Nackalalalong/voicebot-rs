use thiserror::Error;

#[derive(Debug, Error)]
pub enum AsrError {
    #[error("ASR channel closed")]
    ChannelClosed,
}
