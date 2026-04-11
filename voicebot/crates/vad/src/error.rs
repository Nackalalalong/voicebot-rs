use thiserror::Error;

#[derive(Debug, Error)]
pub enum VadError {
    #[error("VAD channel closed")]
    ChannelClosed,
}
