---
name: Error Handling and Fault Tolerance
---

# Skill: Error Handling and Fault Tolerance

Use this whenever writing error types, retry logic, or fallback handling.

## Error type conventions

Each crate defines its own error enum. Every error MUST implement `ComponentError`.

```rust
// In crates/asr/src/error.rs
#[derive(Debug, thiserror::Error)]
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
}

impl ComponentError for AsrError {
    fn component(&self) -> Component { Component::Asr }

    fn is_recoverable(&self) -> bool {
        match self {
            AsrError::ConnectionFailed => true,   // retry
            AsrError::Timeout(_) => true,          // retry
            AsrError::InvalidResponse(_) => false, // bug — don't retry
            AsrError::ChannelClosed => false,      // session is gone
            AsrError::ProviderUnavailable(_) => true, // try fallback
        }
    }

    fn retry_after_ms(&self) -> Option<u64> {
        match self {
            AsrError::ConnectionFailed => Some(200),
            AsrError::Timeout(_) => Some(500),
            AsrError::ProviderUnavailable(_) => Some(1000),
            _ => None,
        }
    }
}
```

## Converting errors to PipelineEvent

Components do NOT emit `PipelineEvent::ComponentError` themselves. They return `Err(...)` to the orchestrator, which decides what to emit.

```rust
// Wrong — component emits the error event directly
tx.send(PipelineEvent::ComponentError { ... }).await;

// Correct — component returns an Err
return Err(AsrError::Timeout(30_000));

// Orchestrator receives it and decides
match asr_result {
    Err(e) if e.is_recoverable() => {
        tracing::warn!("ASR error, retrying: {:?}", e);
        retry_asr(session, attempt + 1).await;
    }
    Err(e) => {
        tx.send(PipelineEvent::ComponentError {
            component: e.component(),
            error: e.to_string(),
            recoverable: false,
        }).await.ok();
        session.terminate().await;
    }
    Ok(_) => {}
}
```

## Retry matrix

| Component | Max retries | Base delay | Strategy    |
| --------- | ----------- | ---------- | ----------- |
| ASR       | 3           | 200ms      | linear      |
| LLM       | 2           | 500ms      | exponential |
| TTS       | 2           | 300ms      | linear      |
| WebSocket | 5           | 1000ms     | exponential |

Use the `with_retry` helper from `common::retry` (see provider-integration skill).

## Fallback provider pattern

```rust
pub struct FallbackAsrProvider {
    primary: Box<dyn AsrProvider>,
    fallback: Option<Box<dyn AsrProvider>>,
}

impl AsrProvider for FallbackAsrProvider {
    async fn stream(
        &self,
        audio_rx: impl AudioInputStream,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError> {
        match self.primary.stream(audio_rx, tx.clone()).await {
            Ok(v) => Ok(v),
            Err(e) if e.is_recoverable() => {
                tracing::warn!("Primary ASR failed ({:?}), trying fallback", e);
                match &self.fallback {
                    Some(fb) => fb.stream(audio_rx, tx).await,
                    None => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }
}
```

## Graceful degradation levels

When components fail beyond recovery:

| Failure | Degradation response |
| --- | --- |
| TTS fails | Send transcript text as `{ "type": "agent_final", "text": "..." }` JSON to egress |
| ASR fails | Send `{ "type": "error", "code": "asr_unavailable" }` to client, keep session alive |
| LLM fails | Send `{ "type": "error", "code": "llm_unavailable" }`, attempt with fallback provider |
| Both ASR+LLM | Terminate session gracefully with error code |

## Session termination on error

```rust
impl PipelineSession {
    pub async fn terminate_on_error(&mut self, error: impl ComponentError) {
        tracing::error!(
            session_id = %self.id,
            component = ?error.component(),
            recoverable = error.is_recoverable(),
            "session terminating due to unrecoverable error"
        );

        // Cancel all component tokens
        self.cancel_token.cancel();

        // Wait for all tasks to finish (max 5s)
        let _ = timeout(
            Duration::from_secs(5),
            futures::future::join_all(self.task_handles.drain(..))
        ).await;

        self.state = SessionState::Terminated;

        self.event_tx.send(PipelineEvent::SessionEnd {
            session_id: self.id,
            reason: EndReason::Error(error.to_string()),
        }).await.ok();
    }
}
```

## What NOT to do

```rust
// Never panic in production
panic!("ASR failed");  // ← forbidden

// Never ignore errors silently
let _ = tx.send(event).await;  // ← only OK for best-effort events (partial transcripts)
                                //   NEVER for FinalTranscript or AgentFinalResponse

// Never use .unwrap() in non-test code
let config = file.read().unwrap();  // ← forbidden; use ?

// Never swallow errors with empty match arms
match result {
    Ok(_) => {}
    Err(_) => {}  // ← forbidden; always log at minimum
}
```
