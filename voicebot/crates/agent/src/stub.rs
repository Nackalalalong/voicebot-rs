use async_trait::async_trait;
use common::error::LlmError;
use common::events::PipelineEvent;
use common::traits::LlmProvider;
use common::types::{Message, ToolDefinition};
use tokio::sync::mpsc::Sender;
use tracing::debug;

pub struct StubLlmProvider;

#[async_trait]
impl LlmProvider for StubLlmProvider {
    async fn stream_completion(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        tx: Sender<PipelineEvent>,
    ) -> Result<(), LlmError> {
        debug!("StubLlmProvider: emitting AgentFinalResponse");
        tx.send(PipelineEvent::AgentFinalResponse {
            text: "stub response".into(),
            tool_calls: vec![],
        })
        .await
        .map_err(|_| LlmError::StreamError("channel closed".into()))?;

        Ok(())
    }

    async fn cancel(&self) {
        // no-op
    }
}
