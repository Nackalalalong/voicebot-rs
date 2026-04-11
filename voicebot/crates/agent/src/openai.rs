use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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

        // Build request body
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "max_tokens": 1024
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

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|_| LlmError::ConnectionFailed)?;

        if !response.status().is_success() {
            return Err(LlmError::ConnectionFailed);
        }

        let mut stream = response.bytes_stream();
        let mut full_text = String::new();
        // (id, name, arguments_json)
        let mut accumulated_tool_calls: Vec<(String, String, String)> = Vec::new();
        let mut line_buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(LlmError::Cancelled);
            }

            let chunk = chunk_result.map_err(|e| LlmError::StreamError(e.to_string()))?;
            let text = String::from_utf8_lossy(&chunk);
            line_buffer.push_str(&text);

            // Process complete lines
            while let Some(newline_pos) = line_buffer.find('\n') {
                let line = line_buffer[..newline_pos].trim().to_string();
                line_buffer = line_buffer[newline_pos + 1..].to_string();

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
                        tracing::warn!("failed to parse SSE chunk: {}", e);
                        continue;
                    }
                };

                for choice in &parsed.choices {
                    // Handle content delta
                    if let Some(ref content) = choice.delta.content {
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
        }

        // Build final tool calls
        let tool_calls: Vec<ToolCall> = accumulated_tool_calls
            .into_iter()
            .filter(|(id, name, _)| !id.is_empty() && !name.is_empty())
            .map(|(id, name, args_json)| {
                let arguments: serde_json::Value = serde_json::from_str(&args_json)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                ToolCall {
                    id,
                    function: FunctionCall { name, arguments },
                }
            })
            .collect();

        let _ = tx
            .send(PipelineEvent::AgentFinalResponse {
                text: full_text,
                tool_calls,
            })
            .await;

        Ok(())
    }

    async fn cancel(&self) {
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
