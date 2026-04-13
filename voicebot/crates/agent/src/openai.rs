use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Maximum number of parallel tool calls we accept in a single LLM stream response.
/// Guards against a misbehaving backend sending an unbounded `index` field.
const MAX_PARALLEL_TOOL_CALLS: usize = 32;

use async_trait::async_trait;
use common::error::LlmError;
use common::events::PipelineEvent;
use common::traits::LlmProvider;
use common::types::{FunctionCall, Message, ToolCall, ToolDefinition};
use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc::Sender;

pub struct OpenAiProvider {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
    cancelled: Arc<AtomicBool>,
}

impl OpenAiProvider {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("failed to build reqwest client"); // OK: constructor, not async path
        Self {
            base_url,
            api_key,
            model,
            client,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCall {
    index: Option<usize>,
    id: Option<String>,
    function: Option<StreamFunction>,
}

#[derive(Debug, Deserialize)]
struct StreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: Sender<PipelineEvent>,
    ) -> Result<(), LlmError> {
        self.cancelled.store(false, Ordering::Relaxed);

        tracing::debug!(
            model = %self.model,
            message_count = messages.len(),
            tool_count = tools.len(),
            "starting LLM stream_completion"
        );

        // Build request body
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "max_tokens": 1024,
            "reasoning_effort": "none",
        });

        if !tools.is_empty() {
            let tools_json: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tools_json);
        }

        let url = format!("{}/v1/chat/completions", self.base_url);
        tracing::debug!(url = %url, "sending chat completions request");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(url = %url, error = %e, "LLM connection failed");
                LlmError::ConnectionFailed
            })?;

        let status = response.status();
        tracing::debug!(status = %status, "received LLM response");

        if !status.is_success() {
            tracing::error!(status = %status, url = %url, "LLM returned non-success status");
            return Err(LlmError::ConnectionFailed);
        }

        let mut stream = response.bytes_stream();
        let mut full_text = String::new();
        // (id, name, arguments_json)
        let mut accumulated_tool_calls: Vec<(String, String, String)> = Vec::new();
        let mut line_buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
            if self.cancelled.load(Ordering::Relaxed) {
                tracing::debug!("LLM stream cancelled");
                return Err(LlmError::Cancelled);
            }

            let chunk = chunk_result.map_err(|e| LlmError::StreamError(e.to_string()))?;
            let text = String::from_utf8_lossy(&chunk);
            line_buffer.push_str(&text);

            // Process complete lines — index-based to avoid a String allocation per newline
            let mut consumed = 0;
            while let Some(rel) = line_buffer[consumed..].find('\n') {
                let abs = consumed + rel;
                let line = line_buffer[consumed..abs].trim();
                consumed = abs + 1;

                if line.is_empty() || !line.starts_with("data: ") {
                    continue;
                }

                let data = &line["data: ".len()..];
                if data == "[DONE]" {
                    continue;
                }

                let parsed: StreamChunk = match serde_json::from_str(data) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(raw = %data, error = %e, "failed to parse SSE chunk");
                        continue;
                    }
                };

                for choice in &parsed.choices {
                    // Handle content delta
                    if let Some(ref content) = choice.delta.content {
                        tracing::trace!(content = %content, "LLM partial content delta");
                        full_text.push_str(content);
                        let _ = tx
                            .send(PipelineEvent::AgentPartialResponse {
                                text: content.clone(),
                            })
                            .await;
                    }

                    // Handle tool call deltas
                    if let Some(ref tcs) = choice.delta.tool_calls {
                        for tc in tcs {
                            let index = tc.index.unwrap_or(0);
                            tracing::trace!(index = index, id = ?tc.id, "LLM tool call delta");

                            if index >= MAX_PARALLEL_TOOL_CALLS {
                                tracing::warn!(index = index, max = MAX_PARALLEL_TOOL_CALLS, "LLM tool call index out of bounds; skipping");
                                continue;
                            }

                            // Grow the accumulator if needed
                            while accumulated_tool_calls.len() <= index {
                                accumulated_tool_calls.push((
                                    String::new(),
                                    String::new(),
                                    String::new(),
                                ));
                            }

                            if let Some(ref id) = tc.id {
                                accumulated_tool_calls[index].0 = id.clone();
                            }
                            if let Some(ref func) = tc.function {
                                if let Some(ref name) = func.name {
                                    accumulated_tool_calls[index].1 = name.clone();
                                }
                                if let Some(ref args) = func.arguments {
                                    accumulated_tool_calls[index].2.push_str(args);
                                }
                            }
                        }
                    }
                }
            }
            // Drop the processed prefix in one O(remaining) pass per HTTP chunk
            line_buffer.drain(..consumed);
        }

        // Build final tool calls
        tracing::debug!(
            text_len = full_text.len(),
            tool_call_count = accumulated_tool_calls
                .iter()
                .filter(|(id, name, _)| !id.is_empty() && !name.is_empty())
                .count(),
            "LLM stream complete"
        );

        let tool_calls: Vec<ToolCall> = accumulated_tool_calls
            .into_iter()
            .filter(|(id, name, _)| !id.is_empty() && !name.is_empty())
            .map(|(id, name, args_json)| {
                tracing::debug!(id = %id, name = %name, args = %args_json, "LLM tool call finalized");
                let arguments: serde_json::Value = serde_json::from_str(&args_json)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                ToolCall {
                    id,
                    function: FunctionCall { name, arguments },
                }
            })
            .collect();

        tracing::debug!(tool_calls = tool_calls.len(), "sending AgentFinalResponse");
        let _ = tx
            .send(PipelineEvent::AgentFinalResponse {
                text: full_text,
                tool_calls,
            })
            .await;

        Ok(())
    }

    async fn cancel(&self) {
        tracing::debug!("LLM provider cancel requested");
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_stream_chunk_with_content() {
        let json = r#"{"choices":[{"delta":{"content":"Hello"}}]}"#;
        let chunk: StreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hello"));
    }

    #[test]
    fn test_parse_stream_chunk_with_tool_call() {
        let json = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_123","function":{"name":"get_weather","arguments":"{\"city\":"}}]}}]}"#;
        let chunk: StreamChunk = serde_json::from_str(json).unwrap();
        let tc = &chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.id.as_deref(), Some("call_123"));
    }

    #[test]
    fn test_provider_cancel() {
        let provider = OpenAiProvider::new(
            "https://api.openai.com".into(),
            "test-key".into(),
            "gpt-4".into(),
        );
        assert!(!provider.cancelled.load(Ordering::Relaxed));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { provider.cancel().await });
        assert!(provider.cancelled.load(Ordering::Relaxed));
    }
}
