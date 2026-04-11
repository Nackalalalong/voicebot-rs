# Skill: Orchestrator and Pipeline Session

Use this whenever working on the orchestrator state machine, session lifecycle, channel wiring, event routing, or interrupt/cancel handling in `/crates/core`.

## Orchestrator state machine

The orchestrator is the brain of the pipeline. It owns the event channel and drives all state transitions.

```
Idle  ──SpeechStarted──►  Listening
Listening ──SpeechEnded──►  Transcribing
Transcribing ──FinalTranscript──►  AgentThinking
AgentThinking ──AgentFinalResponse──►  Speaking
Speaking ──TtsComplete──►  Idle
Speaking ──Interrupt──►  Idle      (cancel TTS + agent immediately)
Any state ──Cancel──►  Idle
```

### State enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestratorState {
    Idle,
    Listening,
    Transcribing,
    AgentThinking,
    Speaking,
}
```

### Transition implementation

```rust
impl Orchestrator {
    async fn handle_event(&mut self, event: PipelineEvent) {
        match (&self.state, &event) {
            // Valid transitions
            (OrchestratorState::Idle, PipelineEvent::SpeechStarted { .. }) => {
                self.state = OrchestratorState::Listening;
            }
            (OrchestratorState::Listening, PipelineEvent::SpeechEnded { .. }) => {
                self.state = OrchestratorState::Transcribing;
            }
            (OrchestratorState::Transcribing, PipelineEvent::FinalTranscript { .. }) => {
                self.state = OrchestratorState::AgentThinking;
                // Forward transcript to agent
                self.agent_tx.send(event).await.ok();
            }
            (OrchestratorState::AgentThinking, PipelineEvent::AgentFinalResponse { .. }) => {
                self.state = OrchestratorState::Speaking;
            }
            (OrchestratorState::Speaking, PipelineEvent::TtsComplete) => {
                self.state = OrchestratorState::Idle;
            }

            // Interrupt — only valid during Speaking
            (OrchestratorState::Speaking, PipelineEvent::Interrupt) => {
                self.handle_interrupt().await;
            }

            // Cancel — valid from any state
            (_, PipelineEvent::Cancel) => {
                self.handle_cancel().await;
            }

            // Ignore invalid transitions, log for debugging
            (state, event) => {
                tracing::debug!(
                    ?state, ?event,
                    "ignoring event in current state"
                );
            }
        }
    }
}
```

## Interrupt vs Cancel vs Flush vs Replace

| Signal | Trigger | Action | Next state |
| --- | --- | --- | --- |
| `Interrupt` | User spoke during TTS playback | Cancel TTS + LLM, drop buffered audio, flush ASR | Idle |
| `Cancel` | Explicit abort from client | Abort current turn entirely, no response | Idle |
| `Flush` | Drain request | Emit whatever partial output exists, then stop | Idle |
| `Replace` | Corrected tool result | Cancel current response, begin a new one with new content | AgentThinking |

### Interrupt handler (CRITICAL — get the order right)

```rust
async fn handle_interrupt(&mut self) {
    tracing::info!(session_id = %self.session_id, "interrupt: cancelling TTS and agent");

    // 1. Cancel TTS immediately — stop sending audio
    self.tts_provider.cancel().await;

    // 2. Cancel LLM — stop generating tokens
    self.llm_provider.cancel().await;

    // 3. Drop all buffered TtsAudioChunk events from the channel
    while self.tts_rx.try_recv().is_ok() {}

    // 4. Flush ASR buffers (send silence tail)
    // ASR will emit any remaining partial as a discard

    // 5. Return to Idle
    self.state = OrchestratorState::Idle;
}
```

## PipelineSession — per-connection lifecycle

Each inbound connection (WebSocket or Asterisk) spawns one `PipelineSession`. Sessions share NO state.

```rust
pub struct PipelineSession {
    pub id: Uuid,
    pub config: SessionConfig,
    pub state: SessionState,
    pub cancel_token: CancellationToken,
    pub task_handles: Vec<JoinHandle<()>>,
    pub event_tx: Sender<PipelineEvent>,
    pub event_rx: Receiver<PipelineEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Starting,      // components initializing, no audio processed
    Active,        // normal operation
    Terminating,   // received SessionEnd, draining components
    Terminated,    // all tasks joined, resources freed
}
```

### Session lifecycle

```
Starting ──components ready──► Active
Active ──SessionEnd / error──► Terminating
Terminating ──all tasks joined──► Terminated
```

Sessions MUST be fully cleaned up within **5 seconds** of `SessionEnd`.

### Spawning a session

```rust
impl PipelineSession {
    pub async fn start(config: SessionConfig) -> Result<Self, SessionError> {
        let cancel_token = CancellationToken::new();
        let (event_tx, event_rx) = mpsc::channel::<PipelineEvent>(200);

        // Create channels with correct capacities
        let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
        let (vad_tx, vad_rx) = mpsc::channel::<AudioFrame>(100);
        let (asr_tx, asr_rx) = mpsc::channel::<PipelineEvent>(10);
        let (agent_tx, agent_rx) = mpsc::channel::<PipelineEvent>(20);
        let (tts_tx, tts_rx) = mpsc::channel::<PipelineEvent>(50);

        let mut handles = Vec::new();

        // Spawn components — each gets its cancel token child
        let vad_token = cancel_token.child_token();
        handles.push(tokio::spawn(async move {
            vad_component.run(audio_rx, vad_token).await;
        }));

        let asr_token = cancel_token.child_token();
        handles.push(tokio::spawn(async move {
            asr_provider.stream(vad_rx, asr_tx, asr_token).await;
        }));

        // ... agent, tts similarly

        Ok(Self {
            id: config.session_id,
            config,
            state: SessionState::Active,
            cancel_token,
            task_handles: handles,
            event_tx,
            event_rx,
        })
    }
}
```

### Session cleanup

```rust
impl PipelineSession {
    pub async fn terminate(&mut self) {
        if self.state == SessionState::Terminated {
            return;
        }
        self.state = SessionState::Terminating;

        // Signal all components to stop
        self.cancel_token.cancel();

        // Wait for all tasks to finish (max 5 seconds)
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            futures::future::join_all(self.task_handles.drain(..)),
        ).await;

        self.state = SessionState::Terminated;

        tracing::info!(session_id = %self.id, "session terminated, all resources freed");
    }
}
```

## Channel wiring and capacities

| Channel      | Capacity | Overflow policy                             |
| ------------ | -------- | ------------------------------------------- |
| audio → vad  | 50       | drop oldest (`try_send`, discard on `Full`) |
| vad → asr    | 100      | drop oldest                                 |
| asr → agent  | 10       | **block** (never drop `FinalTranscript`)    |
| agent → tts  | 20       | **block** (never drop `AgentFinalResponse`) |
| tts → egress | 50       | drop oldest                                 |
| event bus    | 200      | drop oldest, log warn                       |

### Drop-oldest pattern (audio channels)

```rust
// For audio channels where dropping is acceptable
match tx.try_send(frame) {
    Ok(_) => {}
    Err(TrySendError::Full(_new_frame)) => {
        // Drop oldest by receiving one, then retry
        let _ = rx_shadow.try_recv(); // discard oldest
        let _ = tx.try_send(_new_frame);
        tracing::warn!("audio channel full, dropped oldest frame");
    }
    Err(TrySendError::Closed(_)) => {
        return Err(ComponentError::ChannelClosed);
    }
}
```

### Block pattern (critical events)

```rust
// For FinalTranscript and AgentFinalResponse — MUST NOT drop
if let Err(e) = tx.send(event).await {
    tracing::error!("critical event channel closed: {:?}", e);
    return Err(ComponentError::ChannelClosed);
}
```

## Event routing table

| Event                  | Producer     | Consumer     | Droppable? |
| ---------------------- | ------------ | ------------ | ---------- |
| `AudioFrame`           | transport    | VAD          | Yes        |
| `SpeechStarted`        | VAD          | orchestrator | No         |
| `SpeechEnded`          | VAD          | orchestrator | No         |
| `PartialTranscript`    | ASR          | orchestrator | Yes        |
| `FinalTranscript`      | ASR          | orchestrator | **Never**  |
| `AgentPartialResponse` | agent        | orchestrator | Yes        |
| `AgentFinalResponse`   | agent        | orchestrator | **Never**  |
| `TtsAudioChunk`        | TTS          | transport    | Yes        |
| `TtsComplete`          | TTS          | orchestrator | No         |
| `Interrupt`            | transport    | orchestrator | No         |
| `Cancel`               | transport    | orchestrator | No         |
| `ComponentError`       | orchestrator | transport    | No         |

## Orchestrator event loop

```rust
impl Orchestrator {
    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    tracing::info!(session_id = %self.session_id, "orchestrator cancelled");
                    break;
                }
                Some(event) = self.event_rx.recv() => {
                    self.handle_event(event).await;
                }
                else => break,
            }
        }
    }
}
```

## What NOT to do

```rust
// Never skip the state check — always validate transitions
self.state = OrchestratorState::Speaking; // ← wrong: no guard

// Never use unbounded channels
let (tx, rx) = mpsc::unbounded_channel(); // ← forbidden

// Never drop FinalTranscript or AgentFinalResponse
let _ = tx.try_send(PipelineEvent::FinalTranscript { .. }); // ← forbidden: use .send().await

// Never hold locks across await points in the orchestrator loop
let guard = self.state_lock.lock().await;
self.do_async_work().await; // ← forbidden: lock held across await
drop(guard);

// Never share state between sessions
static SESSION_MAP: Lazy<Mutex<HashMap<...>>> = ...; // ← forbidden: sessions are independent
```
