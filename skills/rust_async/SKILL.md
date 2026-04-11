---
name: Rust Async
---

# Skill: Rust Async Patterns

Use this whenever writing async code in this project.

## Runtime

Always use Tokio. Never mix async runtimes.

```rust
// Correct — in main.rs
#[tokio::main]
async fn main() -> anyhow::Result<()> { ... }

// Correct — in tests
#[tokio::test]
async fn test_something() { ... }
```

## Channel patterns

All channels in this project are bounded. Use `tokio::sync::mpsc`.

```rust
use tokio::sync::mpsc;

// Creating a bounded channel
let (tx, rx) = mpsc::channel::<PipelineEvent>(200);

// Sending — handle backpressure explicitly
match tx.try_send(event) {
    Ok(_) => {}
    Err(TrySendError::Full(dropped)) => {
        tracing::warn!(dropped = ?dropped, "channel full, dropping event");
    }
    Err(TrySendError::Closed(_)) => {
        return Err(ComponentError::ChannelClosed);
    }
}

// Receiving in a select loop
loop {
    tokio::select! {
        Some(frame) = audio_rx.recv() => { /* handle */ }
        Some(event) = event_rx.recv() => { /* handle */ }
        else => break,
    }
}
```

## Cancellation

Use `tokio_util::CancellationToken` for component shutdown. Never use raw flags.

```rust
use tokio_util::sync::CancellationToken;

pub struct VadComponent {
    cancel: CancellationToken,
}

impl VadComponent {
    pub async fn run(&self, mut audio_rx: mpsc::Receiver<AudioFrame>) {
        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    tracing::info!("VAD cancelled, shutting down");
                    break;
                }
                Some(frame) = audio_rx.recv() => {
                    self.process(frame).await;
                }
                else => break,
            }
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}
```

## CPU-heavy work

Offload to a blocking thread pool. Never block the Tokio runtime.

```rust
// Wrong — blocks the async runtime
let result = heavy_cpu_work(data);

// Correct — offload to blocking pool
let result = tokio::task::spawn_blocking(move || {
    heavy_cpu_work(data)
}).await?;
```

VAD inference, audio format conversion, and codec operations MUST use `spawn_blocking`.

## Shared state across tasks

Prefer message passing over shared state. When shared state is unavoidable:

```rust
// For read-heavy config (set once, read many)
use std::sync::Arc;
let config: Arc<SessionConfig> = Arc::new(config);

// For mutable shared state (rare — prefer channels instead)
use tokio::sync::RwLock;
let state: Arc<RwLock<SessionState>> = Arc::new(RwLock::new(SessionState::Idle));

// Never use std::sync::Mutex in async code — use tokio::sync::Mutex
use tokio::sync::Mutex;  // OK in async context
```

## Task spawning

Always name your tasks and handle their JoinHandles.

```rust
let handle = tokio::spawn(async move {
    vad.run(audio_rx).await
});

// In session cleanup — join all handles
tokio::join!(vad_handle, asr_handle, agent_handle, tts_handle);
```

## Timeouts

Always wrap external I/O with a timeout.

```rust
use tokio::time::{timeout, Duration};

let result = timeout(
    Duration::from_secs(30),
    deepgram_client.send_audio(frame)
).await
.map_err(|_| AsrError::Timeout)?;
```

## Arc<[i16]> for AudioFrame data

```rust
// Creating — from Vec
let data: Arc<[i16]> = samples.into();

// Cloning is cheap — just increments refcount, no copy
let frame2 = frame.clone();  // Arc<[i16]> clone = O(1)

// Slicing without copy
let slice: &[i16] = &frame.data[..160];
```
