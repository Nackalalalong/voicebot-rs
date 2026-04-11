---
name: Agent Tool Calling
---

# Skill: Agent Tool Calling

Use this whenever working on the agent core in `voicebot/crates/agent`, including the tool-calling loop, conversation memory, sentence-boundary streaming to TTS, or the `Tool` trait.

## AgentCore struct

```rust
pub struct AgentCore {
    llm: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    memory: ConversationMemory,
    cancel_token: CancellationToken,
}
```

**No LangChain. No external agent frameworks.** This is a hand-rolled tool loop.

## Tool trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name used in LLM tool definitions (e.g., "get_weather")
    fn name(&self) -> &str;

    /// JSON schema describing the tool's parameters
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given arguments
    async fn execute(&self, args: serde_json::Value) -> Result<String, ToolError>;
}
```

## Tool-calling loop (CRITICAL — max 5 iterations)

```rust
impl AgentCore {
    pub async fn handle_turn(
        &mut self,
        transcript: String,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AgentError> {
        // Add user message to memory
        self.memory.push(Message::user(&transcript));

        let mut iterations = 0;
        const MAX_TOOL_ITERATIONS: u32 = 5;

        loop {
            if iterations >= MAX_TOOL_ITERATIONS {
                tracing::warn!("max tool iterations reached, forcing final response");
                break;
            }

            // Stream completion from LLM
            let (response_tx, mut response_rx) = mpsc::channel::<PipelineEvent>(20);
            let messages = self.memory.messages();
            let tool_defs: Vec<ToolDefinition> = self.tools.iter()
                .map(|t| t.definition())
                .collect();

            // Spawn LLM streaming in background
            let llm = self.llm.clone();
            let llm_token = self.cancel_token.child_token();
            let llm_handle = tokio::spawn(async move {
                tokio::select! {
                    result = llm.stream_completion(&messages, &tool_defs, response_tx) => result,
                    _ = llm_token.cancelled() => Err(LlmError::Cancelled),
                }
            });

            // Forward partial tokens and collect the final response
            let mut sentence_buffer = String::new();
            let mut final_text = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();

            while let Some(event) = response_rx.recv().await {
                match event {
                    PipelineEvent::AgentPartialResponse { text } => {
                        final_text.push_str(&text);
                        sentence_buffer.push_str(&text);

                        // Forward partial to orchestrator
                        tx.send(PipelineEvent::AgentPartialResponse {
                            text: text.clone(),
                        }).await.ok();

                        // Check for sentence boundary → flush to TTS early
                        if let Some(sentence) = extract_sentence(&mut sentence_buffer) {
                            tx.send(PipelineEvent::AgentPartialResponse {
                                text: sentence,
                            }).await.ok();
                        }
                    }
                    PipelineEvent::AgentFinalResponse { text, tool_calls: tc } => {
                        final_text = text;
                        tool_calls = tc;
                    }
                    _ => {}
                }
            }

            llm_handle.await??;

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                self.memory.push(Message::assistant(&final_text));
                tx.send(PipelineEvent::AgentFinalResponse {
                    text: final_text,
                    tool_calls: vec![],
                }).await.ok();
                return Ok(());
            }

            // Execute tool calls
            self.memory.push(Message::assistant_with_tool_calls(&final_text, &tool_calls));

            for tc in &tool_calls {
                let tool = self.tools.iter()
                    .find(|t| t.name() == tc.function.name)
                    .ok_or_else(|| AgentError::ToolNotFound(tc.function.name.clone()))?;

                let result = tool.execute(tc.function.arguments.clone()).await
                    .unwrap_or_else(|e| format!("Tool error: {}", e));

                self.memory.push(Message::tool_result(&tc.id, &result));
            }

            iterations += 1;
            // Loop back to call LLM again with tool results
        }

        Ok(())
    }
}
```

## Sentence boundary detection for early TTS streaming

Begin TTS synthesis as soon as a sentence boundary is detected. Do NOT wait for `AgentFinalResponse`.

```rust
/// Extract a complete sentence from the buffer if a boundary is found.
/// Returns the sentence and leaves the remainder in the buffer.
fn extract_sentence(buffer: &mut String) -> Option<String> {
    // Check for sentence-ending punctuation
    if let Some(pos) = buffer.rfind(|c: char| c == '.' || c == '?' || c == '!') {
        // Include the punctuation
        let sentence = buffer[..=pos].trim().to_string();
        *buffer = buffer[pos + 1..].to_string();
        if !sentence.is_empty() {
            return Some(sentence);
        }
    }

    // Fallback: flush at ~80 characters on a word boundary
    if buffer.len() >= 80 {
        if let Some(pos) = buffer[..80].rfind(' ') {
            let chunk = buffer[..pos].trim().to_string();
            *buffer = buffer[pos..].to_string();
            if !chunk.is_empty() {
                return Some(chunk);
            }
        }
    }

    None
}
```

## Conversation memory

Sliding window of the last N turns, oldest trimmed first.

```rust
pub struct ConversationMemory {
    messages: VecDeque<Message>,
    max_turns: usize, // default 20
}

impl ConversationMemory {
    pub fn new(max_turns: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            max_turns,
        }
    }

    pub fn push(&mut self, msg: Message) {
        self.messages.push_back(msg);
        // Trim oldest turns (keep system message if present)
        while self.messages.len() > self.max_turns * 2 {
            // Skip index 0 if it's a system message
            if self.messages.front().map_or(false, |m| m.role == Role::System) {
                if self.messages.len() > 1 {
                    self.messages.remove(1);
                }
            } else {
                self.messages.pop_front();
            }
        }
    }

    pub fn messages(&self) -> Vec<Message> {
        self.messages.iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }
}
```

## Message types

```rust
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Message {
    pub fn user(text: &str) -> Self {
        Self { role: Role::User, content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn assistant(text: &str) -> Self {
        Self { role: Role::Assistant, content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn assistant_with_tool_calls(text: &str, calls: &[ToolCall]) -> Self {
        Self {
            role: Role::Assistant,
            content: Some(text.into()),
            tool_calls: Some(calls.to_vec()),
            tool_call_id: None,
        }
    }
    pub fn tool_result(tool_call_id: &str, result: &str) -> Self {
        Self {
            role: Role::Tool,
            content: Some(result.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}
```

## Cancellation

When an `Interrupt` arrives mid-generation, the orchestrator cancels the agent's token:

```rust
// In orchestrator interrupt handler
self.agent_cancel_token.cancel(); // cancels the child token held by AgentCore

// In AgentCore — the select! in handle_turn exits via llm_token.cancelled()
```

The agent MUST:

1. Stop the LLM stream immediately
2. NOT emit `AgentFinalResponse`
3. Discard any accumulated partial text
4. Be ready to accept a new `FinalTranscript` for the next turn

## What NOT to do

```rust
// Never use an external agent framework
use langchain::Agent; // ← forbidden

// Never exceed MAX_TOOL_ITERATIONS
loop { llm.stream_completion(...).await; } // ← must have iteration counter

// Never wait for AgentFinalResponse before starting TTS
// Stream sentence chunks to TTS as they arrive

// Never keep unlimited conversation history
self.messages.push(msg); // ← must enforce max_turns

// Never block on tool execution without a timeout
let result = tool.execute(args).await; // ← wrap with tokio::time::timeout
```
