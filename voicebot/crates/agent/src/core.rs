use std::sync::Arc;

use common::error::LlmError;
use common::events::PipelineEvent;
use common::traits::LlmProvider;
use common::types::{Message, ToolCall, ToolDefinition};
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;

use crate::error::AgentError;
use crate::memory::ConversationMemory;
use crate::tool::Tool;

const MAX_TOOL_ITERATIONS: u32 = 5;

pub struct AgentCore {
    llm: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    memory: ConversationMemory,
    cancel_token: CancellationToken,
}

enum LlmCompletionOutcome {
    Completed,
    Cancelled,
}

impl AgentCore {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        system_prompt: Option<String>,
        cancel_token: CancellationToken,
    ) -> Self {
        let mut memory = ConversationMemory::new(20);
        if let Some(prompt) = system_prompt {
            memory.push(Message::system(&prompt));
        }
        Self {
            llm,
            tools,
            memory,
            cancel_token,
        }
    }

    pub async fn handle_turn(
        &mut self,
        transcript: String,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AgentError> {
        self.memory.push(Message::user(&transcript));

        let mut iterations = 0;

        // Build tool definitions once — wrap in Arc so only the refcount is cloned per loop iteration
        let tool_defs: Arc<Vec<ToolDefinition>> =
            Arc::new(self.tools.iter().map(|t| t.definition()).collect());

        loop {
            if iterations >= MAX_TOOL_ITERATIONS {
                tracing::warn!("max tool iterations reached");
                break;
            }

            // Stream completion from LLM
            let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<PipelineEvent>(20);
            let messages = self.memory.as_slice().to_vec();
            let tool_defs = Arc::clone(&tool_defs);

            let llm = self.llm.clone();
            let llm_token = self.cancel_token.child_token();
            let llm_handle = tokio::spawn(async move {
                tokio::select! {
                    result = llm.stream_completion(&messages, tool_defs.as_slice(), response_tx) => result,
                    _ = llm_token.cancelled() => Err(LlmError::Cancelled),
                }
            });

            // Collect response
            let mut full_text = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();

            while let Some(event) = response_rx.recv().await {
                match event {
                    PipelineEvent::AgentPartialResponse { text } => {
                        full_text.push_str(&text);
                        let _ = tx.send(PipelineEvent::AgentPartialResponse { text }).await;
                    }
                    PipelineEvent::AgentFinalResponse {
                        text,
                        tool_calls: tc,
                    } => {
                        full_text = text;
                        tool_calls = tc;
                    }
                    _ => {}
                }
            }

            // Wait for LLM task
            let completion = match llm_handle.await {
                Ok(Ok(())) => LlmCompletionOutcome::Completed,
                Ok(Err(LlmError::Cancelled)) => LlmCompletionOutcome::Cancelled,
                Ok(Err(e)) => return Err(AgentError::LlmError(e.to_string())),
                Err(e) => return Err(AgentError::Internal(e.to_string())),
            };

            if matches!(completion, LlmCompletionOutcome::Cancelled) {
                if !full_text.trim().is_empty() {
                    self.memory.push(Message::assistant(&full_text));
                }
                return Err(AgentError::Cancelled);
            }

            // No tool calls = done
            if tool_calls.is_empty() {
                self.memory.push(Message::assistant(&full_text));
                let _ = tx
                    .send(PipelineEvent::AgentFinalResponse {
                        text: full_text,
                        tool_calls: vec![],
                    })
                    .await;
                return Ok(());
            }

            // Execute tools
            self.memory
                .push(Message::assistant_with_tool_calls(&full_text, &tool_calls));
            for tc in &tool_calls {
                let result =
                    if let Some(tool) = self.tools.iter().find(|t| t.name() == tc.function.name) {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(30),
                            tool.execute(tc.function.arguments.clone()),
                        )
                        .await
                        {
                            Ok(Ok(r)) => r,
                            Ok(Err(e)) => format!("Tool error: {}", e),
                            Err(_) => "Tool execution timed out".into(),
                        }
                    } else {
                        format!("Unknown tool: {}", tc.function.name)
                    };
                self.memory.push(Message::tool_result(&tc.id, &result));
            }

            iterations += 1;
        }

        Ok(())
    }

    pub fn memory(&self) -> &ConversationMemory {
        &self.memory
    }

    /// Replace the cancellation token used for the next LLM call.
    /// Called by the orchestrator before each turn so barge-in cancels only
    /// the current LLM request, not the whole session.
    pub fn set_cancel_token(&mut self, token: CancellationToken) {
        self.cancel_token = token;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use common::types::Role;
    use tokio::sync::mpsc;

    struct PartialThenPendingLlmProvider;

    #[async_trait]
    impl LlmProvider for PartialThenPendingLlmProvider {
        async fn stream_completion(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            tx: Sender<PipelineEvent>,
        ) -> Result<(), LlmError> {
            tx.send(PipelineEvent::AgentPartialResponse {
                text: "partial answer".into(),
            })
            .await
            .map_err(|_| LlmError::StreamError("channel closed".into()))?;

            std::future::pending::<()>().await;
            Ok(())
        }

        async fn cancel(&self) {}
    }

    #[tokio::test]
    async fn test_cancelled_turn_memory_contains_partial_assistant_message() {
        let cancel_token = CancellationToken::new();
        let mut agent = AgentCore::new(
            Arc::new(PartialThenPendingLlmProvider),
            vec![],
            None,
            cancel_token.clone(),
        );
        let (tx, mut rx) = mpsc::channel::<PipelineEvent>(8);

        let turn = tokio::spawn(async move {
            let result = agent.handle_turn("hello".into(), tx).await;
            (agent, result)
        });

        let _ = rx.recv().await;
        cancel_token.cancel();

        let (agent, result) = turn.await.expect("agent task join");
        assert!(matches!(result, Err(AgentError::Cancelled)));

        let messages = agent.memory().as_slice();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content.as_deref(), Some("hello"));
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content.as_deref(), Some("partial answer"));
    }
}
