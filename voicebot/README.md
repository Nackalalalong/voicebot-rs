# Voicebot System — Agent Root Context

## What this project is

A real-time streaming voicebot in Rust. Full pipeline: VAD → ASR → Agent → TTS. Transport-agnostic core. Asterisk and WebSocket as adapter layers. No batch processing. No request-response. Everything streams.

## Read this before touching any file

1. Check which crate you are working in before importing anything.
2. Consult the dependency graph below — violations will break the build.
3. Every audio path uses `AudioFrame` from `common`. Do not invent alternative types.
4. All channels are bounded. `unbounded_channel()` is forbidden.
5. No `unwrap()` in non-test code. Propagate errors with `?`.
6. No `std::sync::Mutex` in hot paths. Use `tokio::sync` only.

## Crate dependency graph (strict)

```
common   ←  (no internal deps)
vad      ← common
asr      ← common
agent    ← common
tts      ← common
core     ← common, vad, asr, agent, tts
transport/asterisk  ← common, core
transport/websocket ← common, core
```

`core` MUST NOT import transport crates. Transport crates MUST NOT import each other.

## Canonical types (always use these, never redefine)

```rust
// AudioFrame — defined in common::audio
pub struct AudioFrame {
    pub data: Arc<[i16]>,    // PCM, always 16kHz mono
    pub sample_rate: u32,    // always 16000
    pub channels: u8,        // always 1
    pub timestamp_ms: u64,
}

// PipelineEvent — defined in common::events
// See crates/common/src/events.rs for the full enum
```

## Orchestrator state machine

```
Idle ──SpeechStarted──► Listening
Listening ──SpeechEnded──► Transcribing
Transcribing ──FinalTranscript──► AgentThinking
AgentThinking ──AgentFinalResponse──► Speaking
Speaking ──TtsComplete──► Idle
Speaking ──Interrupt──► Idle   (cancel TTS + LLM immediately)
Any ──Cancel──► Idle
```

## Channel capacities (do not change without updating docs)

| Channel      | Capacity | Overflow policy       |
| ------------ | -------- | --------------------- |
| audio → vad  | 50       | drop oldest           |
| vad → asr    | 100      | drop oldest           |
| asr → agent  | 10       | block (never drop)    |
| agent → tts  | 20       | block (never drop)    |
| tts → egress | 50       | drop oldest           |
| event bus    | 200      | drop oldest, log warn |

## Build milestones (never skip ahead)

| #   | Crate(s)            | Gate to proceed                        |
| --- | ------------------- | -------------------------------------- |
| 1   | common              | `cargo test -p common` passes          |
| 2   | vad                 | VAD unit tests with synthetic frames   |
| 3   | core (with stubs)   | Full pipeline runs end-to-end w/ stubs |
| 4   | transport/websocket | WS client can send PCM, get events     |
| 5   | asr                 | Deepgram integration test passes       |
| 6   | agent               | OpenAI tool-call loop test passes      |
| 7   | tts                 | ElevenLabs streams audio chunks        |
| 8   | Integration         | Real end-to-end call with interrupt    |
| 9   | transport/asterisk  | ARI adapter passes codec test          |
| 10  | Observability       | All metrics emit, config validates     |

## Error handling rules

- Every component emits `ComponentError { component, error, recoverable }` on failure.
- `recoverable: true` → orchestrator retries (see retry matrix in requirements).
- `recoverable: false` → orchestrator terminates session gracefully.
- Never panic in production paths. Panics in tests are acceptable.

## Configuration

- All secrets come from environment variables.
- Config file: `config.toml` at project root.
- Env vars override TOML values.
- Fail fast at startup if a required key is missing.

## Testing conventions

- Unit tests in `#[cfg(test)]` modules within each file.
- Integration tests in `crates/<name>/tests/`.
- Use `tokio::test` for async tests.
- Audio fixtures live in `tests/fixtures/audio/` — 16kHz mono WAV files.
- Use the `TestAudioStream` helper from `common::testing` for synthetic audio.

## Logging

- Use `tracing` crate. Every span MUST include `session_id`.
- Log levels: ERROR for unrecoverable, WARN for retries/drops, INFO for lifecycle, DEBUG for per-frame events.
- No `println!` in library code.
