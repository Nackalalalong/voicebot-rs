use crate::audio::AudioFrame;
use crate::error::{AsrError, LlmError, TtsError};
use crate::events::PipelineEvent;
use crate::types::{Message, ToolDefinition};
use async_trait::async_trait;
use tokio::sync::mpsc::Sender;

/// Trait for receiving audio frames.
#[async_trait]
pub trait AudioInputStream: Send {
    /// Receive the next audio frame, or None if the stream ended.
    async fn recv(&mut self) -> Option<AudioFrame>;
}

/// Trait for sending audio frames.
#[async_trait]
pub trait AudioOutputStream: Send {
    /// Send an audio frame. Returns error if the channel is closed.
    async fn send(&mut self, frame: AudioFrame) -> Result<(), crate::error::SendError>;
}

/// ASR provider trait — streams audio in, emits transcript events.
#[async_trait]
pub trait AsrProvider: Send + Sync {
    /// Stream audio and emit PartialTranscript/FinalTranscript events.
    async fn stream(
        &self,
        audio: Box<dyn AudioInputStream>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError>;
}

/// TTS provider trait — receives text chunks, emits audio events.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    /// Synthesize text chunks into audio.
    async fn synthesize(
        &self,
        text_rx: tokio::sync::mpsc::Receiver<String>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), TtsError>;

    /// Cancel ongoing synthesis immediately.
    async fn cancel(&self);
}

/// LLM provider trait — streaming chat completion.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stream a chat completion, emitting partial/final response events.
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: Sender<PipelineEvent>,
    ) -> Result<(), LlmError>;

    /// Cancel ongoing completion.
    async fn cancel(&self);
}
