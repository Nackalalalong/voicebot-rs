use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("channel closed")]
    ChannelClosed,
    #[error("LLM error: {0}")]
    LlmError(String),
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("agent cancelled")]
    Cancelled,
    #[error("internal error: {0}")]
    Internal(String),
}
