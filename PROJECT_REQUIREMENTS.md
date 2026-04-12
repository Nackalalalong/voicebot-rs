Architecture

# 📜 Voicebot System — Agent Coding Requirements (v0.2)

## 0. Core Objective

Design and implement a **real-time, streaming voicebot system in Rust** with:

- High performance and low latency (< 500ms perceived)
- Full pipeline control (VAD → ASR → Agent → TTS)
- Support for concurrent sessions
- Strict separation between core pipeline and transport layers

---

## 1. Shared Types (`voicebot/crates/common`)

**This crate is built FIRST. All other crates depend on it.**

### 1.1 AudioFrame

```rust
#[derive(Clone, Debug)]
pub struct AudioFrame {
    pub data: Arc<[i16]>,       // PCM samples, zero-copy shared ownership
    pub sample_rate: u32,       // always 16000 Hz internally
    pub channels: u8,           // always 1 (mono) internally
    pub timestamp_ms: u64,      // monotonic ms since session start
}
```

All adapters MUST convert to/from this canonical format before touching the pipeline.

### 1.2 PipelineEvent

```rust
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    // Audio
    AudioFrame(AudioFrame),

    // VAD
    SpeechStarted { timestamp_ms: u64 },
    SpeechEnded   { timestamp_ms: u64 },

    // ASR
    PartialTranscript { text: String, confidence: f32 },
    FinalTranscript   { text: String, language: String },

    // Agent
    AgentPartialResponse { text: String },
    AgentFinalResponse   { text: String, tool_calls: Vec<ToolCall> },

    // TTS
    TtsAudioChunk { frame: AudioFrame, sequence: u32 },
    TtsComplete,

    // Control signals
    Interrupt,   // user spoke during TTS — stop TTS, restart VAD+ASR
    Cancel,      // drop current turn entirely, no response
    Flush,       // drain buffers, emit what we have so far
    Replace,     // cancel + start a new response with different content

    // Lifecycle
    SessionStart { session_id: Uuid, config: SessionConfig },
    SessionEnd   { session_id: Uuid, reason: EndReason },

    // Errors
    ComponentError { component: Component, error: String, recoverable: bool },
}
```

### 1.3 SessionConfig

```rust
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub session_id: Uuid,
    pub language: Language,           // Language::Thai | Language::English | Language::Auto
    pub asr_provider: AsrProvider,
    pub tts_provider: TtsProvider,
    pub llm_provider: LlmProvider,
    pub vad_config: VadConfig,
}

#[derive(Debug, Clone)]
pub struct VadConfig {
    pub silence_ms: u32,              // ms of silence before SpeechEnded fires; default 800
    pub min_speech_ms: u32,          // minimum speech duration to count; default 200
    pub energy_threshold: f32,       // 0.0–1.0, default 0.02
}
```

### 1.4 Core Traits

```rust
pub trait AudioInputStream: Send {
    async fn recv(&mut self) -> Option<AudioFrame>;
}

pub trait AudioOutputStream: Send {
    async fn send(&mut self, frame: AudioFrame) -> Result<(), SendError>;
}

pub trait AsrProvider: Send + Sync {
    async fn stream(&self, audio: impl AudioInputStream, tx: Sender<PipelineEvent>)
        -> Result<(), AsrError>;
    async fn cancel(&self);
}

pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, text_rx: Receiver<String>, tx: Sender<PipelineEvent>)
        -> Result<(), TtsError>;
    async fn cancel(&self);
}

pub trait LlmProvider: Send + Sync {
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: Sender<PipelineEvent>,
    ) -> Result<(), LlmError>;
    async fn cancel(&self);
}
```

### 1.5 Error Types

Each crate defines its own concrete error type. All errors MUST implement:

```rust
pub trait ComponentError: std::error::Error + Send + Sync {
    fn component(&self) -> Component;
    fn is_recoverable(&self) -> bool;    // true = retry; false = end session
    fn retry_after_ms(&self) -> Option<u64>;
}
```

### 1.6 Crate Dependency Rules

```
common   → (no internal deps)
vad      → common
asr      → common
agent    → common
tts      → common
core     → common, vad, asr, agent, tts
transport/asterisk  → common, core
transport/websocket → common, core
```

**Circular imports are forbidden.** `core` MUST NOT import from transport crates.

All crate source lives under `voicebot/crates/`.

---

## 2. Architecture

### 2.1 Per-Session Pipeline

Each inbound call/connection spawns one `PipelineSession`. Sessions share no state.

```rust
pub struct PipelineSession {
    pub id: Uuid,
    pub config: SessionConfig,
    pub state: SessionState,       // Starting | Active | Terminating | Terminated
    pub event_tx: Sender<PipelineEvent>,
    pub event_rx: Receiver<PipelineEvent>,
    // ... component handles
}
```

Session lifecycle:

- `Starting` — components initializing, no audio processed
- `Active` — normal operation
- `Terminating` — received `SessionEnd`, draining components
- `Terminated` — all tasks joined, resources freed

Sessions MUST be fully cleaned up within 5 seconds of `SessionEnd`.

### 2.2 Orchestrator (CRITICAL — specify carefully)

The orchestrator is the brain. It owns the `PipelineEvent` channel and drives state transitions.

**Orchestrator state machine:**

```
Idle  ──SpeechStarted──►  Listening
Listening ──SpeechEnded──►  Transcribing
Transcribing ──SpeechStarted──► Listening   (cancel prior ASR turn, start new utterance)
Transcribing ──FinalTranscript──►  AgentThinking
AgentThinking ──AgentFinalResponse──►  Speaking
Speaking ──TtsComplete──►  Idle
AgentThinking ──SpeechStarted──► Listening  (cancel prior LLM turn, preserve streamed partial text)
Speaking ──SpeechStarted──►  Listening      (cancel prior TTS/LLM turn, start new utterance)
Speaking ──Interrupt──►  Idle
Any state ──Cancel──►  Idle
```

**Interrupt handling MUST:**

1. Cancel the in-flight ASR turn if the previous utterance is still transcribing when a new `SpeechStarted` arrives
2. Cancel the in-flight LLM turn cooperatively; do not rely on `JoinHandle::abort()` as the primary mechanism
3. Persist only the already streamed assistant text into conversation history when the LLM turn is interrupted
4. Cancel the in-flight TTS turn cooperatively and stop audio emission immediately
5. Drop buffered TTS text/audio for the interrupted turn
6. Transition to `Listening` and continue processing the new utterance

**Interrupt vs Cancel vs Flush vs Replace:**

- `Interrupt` — user is speaking. Stop output, go listen.
- `Cancel` — abort current turn. Don't respond. Return to Idle.
- `Flush` — emit whatever partial output exists, then stop. No new content.
- `Replace` — cancel current response, begin a new one (e.g. corrected tool result).

### 2.3 Channel Sizing

All channels are bounded. Default sizes:

```
audio ingress   → vad:         capacity 50  (drop oldest on overflow)
vad → asr:                     capacity 100
asr → agent:                   capacity 10
agent → tts:                   capacity 20  (text chunks)
tts → egress:                  capacity 50  (audio frames)
orchestrator event bus:         capacity 200
```

On channel full: log a warning metric and drop the _oldest_ frame (not newest), except for `FinalTranscript` and `AgentFinalResponse` which must never be dropped.

---

## 3. Component Specifications

### 3.1 VAD (`voicebot/crates/vad`)

**Algorithm:** Use `webrtc-vad` crate as primary. Energy threshold as fallback.

Required behavior:

- Emit `SpeechStarted` when consecutive voiced frames exceed `min_speech_ms`
- Emit `SpeechEnded` when silence exceeds `silence_ms` after speech
- MUST NOT emit `SpeechEnded` without a preceding `SpeechStarted`
- Frame size: 10ms, 20ms, or 30ms windows (30ms default)

```rust
pub struct VadComponent {
    config: VadConfig,
    tx: Sender<PipelineEvent>,
}
impl VadComponent {
    pub async fn run(&mut self, mut audio_rx: impl AudioInputStream) { ... }
}
```

### 3.2 ASR (`voicebot/crates/asr`)

Must support:

- Speaches / OpenAI-compatible server via `POST /v1/audio/transcriptions`
- Whisper via `whisper-rs` (local fallback)
- Cooperative cancellation of the current utterance so a new `SpeechStarted` can discard an older transcription still in progress

On `SpeechEnded`: flush remaining buffer and emit `FinalTranscript`. On provider error: retry up to 3 times with 200ms backoff; on failure emit `ComponentError { recoverable: false }`.

Emit `PartialTranscript` at minimum every 500ms during active speech.

### 3.3 Agent Core (`voicebot/crates/agent`)

**No LangChain. No external agent frameworks.**

Structure:

```rust
pub struct AgentCore {
    llm: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    memory: ConversationMemory,    // sliding window, max 20 turns
}
```

Tool calling loop:

1. Receive `FinalTranscript`
2. Call `llm.stream_completion()` — stream partial tokens as `AgentPartialResponse`
3. If response contains tool calls: execute tools, append results, loop back to step 2
4. When done: emit `AgentFinalResponse`
5. Max 5 tool call iterations per turn (prevent infinite loops)

Conversation memory: keep last N turns where N is configurable (default 20). Trim oldest turns first.

### 3.4 TTS (`voicebot/crates/tts`)

Must support:

- Speaches / OpenAI-compatible server via `POST /v1/audio/speech`
- Coqui TTS (local fallback)

**Streaming:** begin synthesis as soon as a sentence boundary is detected in partial agent output (`.`, `?`, `!`, or ~80 characters). Do not wait for `AgentFinalResponse`.

**Cancellation:** on `TtsProvider::cancel()`, immediately stop the WebSocket connection and return.

Emit `TtsComplete` after the final audio chunk.

---

## 4. Transport Adapters

### 4.1 WebSocket Protocol (MANDATORY — define the framing)

**Connection:** `ws://<host>/session`

**Client → Server:**

```json
// Control message (text frame)
{ "type": "session_start", "language": "th", "asr": "speaches", "tts": "speaches" }
{ "type": "session_end" }
```

Audio frames are sent as **binary frames**: raw PCM i16 little-endian, 16kHz mono, 320 samples per frame (20ms).

**Server → Client:**

```json
// Text frames
{ "type": "transcript_partial", "text": "สวัสดี" }
{ "type": "transcript_final",   "text": "สวัสดีครับ" }
{ "type": "agent_partial",      "text": "ผม" }
{ "type": "agent_final",        "text": "ผมช่วยอะไรได้บ้าง" }
{ "type": "error",              "code": "asr_timeout", "recoverable": true }
```

TTS audio sent as **binary frames**: same format as input.

### 4.2 Asterisk Adapter (`voicebot/crates/transport/asterisk`)

Connects via AMI or ARI (choose ARI for WebSocket-based RTP bridging). Handles:

- μ-law / A-law → PCM i16 conversion
- RTP jitter buffering (50ms)
- DTMF → `PipelineEvent::Cancel` mapping

The adapter owns `AudioInputStream` and `AudioOutputStream` impls. Core pipeline never sees RTP.

### 4.3 No Leakage Rule

Adapters MUST:

- Convert all audio to `AudioFrame` before passing to core
- Translate `PipelineEvent` into transport-native signals (not the reverse)
- Never import from `voicebot/crates/core` internals — only from `voicebot/crates/common`

---

## 5. Configuration

### 5.1 Config File (`config.toml`)

```toml
[server]
host = "0.0.0.0"
port = 8080

[session_defaults]
language = "auto"
asr_provider = "speaches"
tts_provider = "speaches"
llm_provider = "openai"

[vad]
silence_ms = 800
min_speech_ms = 200
energy_threshold = 0.02

[asr.speaches]
base_url = "http://localhost:8000"
model = "Systran/faster-whisper-large-v3"
language = "th"

[asr.whisper]
model_path = "./models/ggml-base.bin"

[llm.openai]
api_key = "${OPENAI_API_KEY}"
model = "gpt-4o"
max_tokens = 512
temperature = 0.7

[llm.anthropic]
api_key = "${ANTHROPIC_API_KEY}"
model = "claude-opus-4-6"

[tts.speaches]
base_url = "http://localhost:8000"
model = "kokoro"
voice = "af_heart"

[tts.coqui]
model_path = "./models/tts_model.pth"

[channels]
audio_ingress_capacity = 50
event_bus_capacity = 200
```

No secrets in source. All `${VAR}` tokens MUST be resolved from environment at startup. Fail fast if a required key is missing.

---

## 6. Observability

### 6.1 Metrics (emit via `metrics` crate, Prometheus-compatible)

```
voicebot_sessions_active              gauge
voicebot_session_duration_ms          histogram
voicebot_vad_latency_ms              histogram  (label: session_id)
voicebot_asr_latency_ms              histogram  (label: provider)
voicebot_llm_first_token_ms          histogram  (label: provider)
voicebot_llm_total_ms                histogram
voicebot_tts_first_chunk_ms          histogram  (label: provider)
voicebot_interrupts_total            counter
voicebot_errors_total                counter    (label: component, recoverable)
```

### 6.2 Structured Logging

Use `tracing` crate. Every span MUST include `session_id`.

```
INFO  span=session session_id=abc123 event=started language=th
DEBUG span=vad     session_id=abc123 event=speech_started
DEBUG span=asr     session_id=abc123 event=partial_transcript text="สวัสดี"
INFO  span=agent   session_id=abc123 event=final_transcript text="สวัสดีครับ"
```

---

## 7. Fault Tolerance

### 7.1 Retry Matrix

| Component | Max retries | Backoff | On exhaustion |
| --- | --- | --- | --- |
| ASR | 3 | 200ms linear | `ComponentError { recoverable: false }` → session end |
| LLM | 2 | 500ms exp | Fallback provider if configured, else session end |
| TTS | 2 | 300ms linear | Text-only mode (emit transcript to egress as text frame) |
| WebSocket | 5 | 1s exp | Close session |

### 7.2 Fallback Providers

Configure in `config.toml`:

```toml
[llm]
primary = "openai"
fallback = "anthropic"

[asr]
primary = "speaches"
fallback = "whisper"
```

Fallback is attempted automatically on exhaustion of retries for primary.

### 7.3 Graceful Degradation

- TTS failure: emit transcript text to egress as a text frame instead
- ASR failure: emit error event to client, keep session alive for retry
- LLM failure: emit `{ "type": "error", "code": "llm_unavailable" }` to client

---

## 8. Project Build Order (for agents)

Build in this exact sequence. Do not parallelize across crates until the dependency is ready.

**Milestone 1:** `voicebot/crates/common` — all types, traits, error types. No logic, just contracts.

**Milestone 2:** `voicebot/crates/vad` — VAD with energy threshold only (no webrtc-vad yet). Unit tests with synthetic audio frames.

**Milestone 3:** `voicebot/crates/core` — orchestrator state machine, session struct, channel wiring. Stub out ASR/Agent/TTS with echo implementations. Run a full pipeline with stubs.

**Milestone 4:** `voicebot/crates/transport/websocket` — WebSocket server, session spawning, binary frame parsing. Test with a real WebSocket client sending PCM audio.

**Milestone 5:** `voicebot/crates/asr` — Speaches/OpenAI-compatible ASR provider. Integration test with real audio file.

**Milestone 6:** `voicebot/crates/agent` — LLM provider (OpenAI first), tool calling loop.

**Milestone 7:** `voicebot/crates/tts` — Speaches/OpenAI-compatible TTS provider. Sentence-boundary streaming.

**Milestone 9:** `voicebot/crates/transport/asterisk` — ARI adapter. Codec conversion.

**Milestone 9:** `/crates/transport/asterisk` — ARI adapter. Codec conversion.

**Milestone 10:** Observability wiring, config validation, fallback providers.

---

## 9. Non-Goals (unchanged)

- No UI
- No persistent long-term memory
- No model training
- No telephony optimizations inside core
- No SIP/RTP handling inside core

---

## 10. Key Invariants (agents MUST NOT violate)

1. `AudioFrame.sample_rate` is always 16000. Adapters convert before passing in.
2. Core pipeline never imports transport crates.
3. All channels are bounded. Unbounded channels are forbidden.
4. Every `SpeechStarted` must be followed by exactly one `SpeechEnded`.
5. Session IDs are UUIDs generated by the transport adapter, not the core.
6. No `unwrap()` in production paths. All errors propagate via `Result`.
7. No `std::sync::Mutex` in hot audio paths. Use `tokio::sync` primitives only.
