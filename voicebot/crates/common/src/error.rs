use crate::types::Component;
use thiserror::Error;

/// Trait that all component errors must implement.
pub trait ComponentErrorTrait: std::error::Error + Send + Sync {
    /// Which component produced this error.
    fn component(&self) -> Component;
    /// Whether the orchestrator should retry.
    fn is_recoverable(&self) -> bool;
    /// Suggested delay before retry, if applicable.
    fn retry_after_ms(&self) -> Option<u64>;
}

/// Error when sending on a channel.
#[derive(Debug, Error)]
pub enum SendError {
    #[error("channel closed")]
    ChannelClosed,
    #[error("channel full")]
    ChannelFull,
}

/// Error during ASR operation.
#[derive(Debug, Error)]
pub enum AsrError {
    #[error("connection to ASR provider failed")]
    ConnectionFailed,
    #[error("ASR stream timed out after {0}ms")]
    Timeout(u64),
    #[error("ASR provider returned invalid response: {0}")]
    InvalidResponse(String),
    #[error("ASR channel closed unexpectedly")]
    ChannelClosed,
    #[error("ASR provider unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("ASR cancelled")]
    Cancelled,
}

impl ComponentErrorTrait for AsrError {
    fn component(&self) -> Component {
        Component::Asr
    }

    fn is_recoverable(&self) -> bool {
        matches!(
            self,
            AsrError::ConnectionFailed | AsrError::Timeout(_) | AsrError::ProviderUnavailable(_)
        )
    }

    fn retry_after_ms(&self) -> Option<u64> {
        match self {
            AsrError::ConnectionFailed => Some(200),
            AsrError::Timeout(_) => Some(200),
            AsrError::ProviderUnavailable(_) => Some(1000),
            _ => None,
        }
    }
}

/// Error during LLM operation.
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("LLM connection failed")]
    ConnectionFailed,
    #[error("LLM request timed out")]
    Timeout,
    #[error("LLM stream error: {0}")]
    StreamError(String),
    #[error("LLM response parse error: {0}")]
    ParseError(String),
    #[error("LLM provider unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("LLM cancelled")]
    Cancelled,
}

impl ComponentErrorTrait for LlmError {
    fn component(&self) -> Component {
        Component::Agent
    }

    fn is_recoverable(&self) -> bool {
        matches!(
            self,
            LlmError::ConnectionFailed | LlmError::Timeout | LlmError::ProviderUnavailable(_)
        )
    }

    fn retry_after_ms(&self) -> Option<u64> {
        match self {
            LlmError::ConnectionFailed => Some(500),
            LlmError::Timeout => Some(500),
            LlmError::ProviderUnavailable(_) => Some(1000),
            _ => None,
        }
    }
}

/// Error during TTS operation.
#[derive(Debug, Error)]
pub enum TtsError {
    #[error("TTS connection failed")]
    ConnectionFailed,
    #[error("TTS stream timed out")]
    Timeout,
    #[error("TTS synthesis error: {0}")]
    SynthesisError(String),
    #[error("TTS channel closed")]
    ChannelClosed,
    #[error("TTS provider unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("TTS cancelled")]
    Cancelled,
}

impl ComponentErrorTrait for TtsError {
    fn component(&self) -> Component {
        Component::Tts
    }

    fn is_recoverable(&self) -> bool {
        matches!(
            self,
            TtsError::ConnectionFailed | TtsError::Timeout | TtsError::ProviderUnavailable(_)
        )
    }

    fn retry_after_ms(&self) -> Option<u64> {
        match self {
            TtsError::ConnectionFailed => Some(300),
            TtsError::Timeout => Some(300),
            TtsError::ProviderUnavailable(_) => Some(1000),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asr_error_recoverability() {
        assert!(AsrError::ConnectionFailed.is_recoverable());
        assert!(AsrError::Timeout(5000).is_recoverable());
        assert!(!AsrError::InvalidResponse("bad".into()).is_recoverable());
        assert!(!AsrError::ChannelClosed.is_recoverable());
    }

    #[test]
    fn test_llm_error_retry_delay() {
        assert_eq!(LlmError::ConnectionFailed.retry_after_ms(), Some(500));
        assert_eq!(LlmError::Cancelled.retry_after_ms(), None);
    }

    #[test]
    fn test_tts_error_component() {
        assert_eq!(TtsError::Timeout.component(), Component::Tts);
    }
}
