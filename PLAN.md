# PLAN — voicebot-rs

> 62 tests passing · 9 crates · Milestones 1–4 complete, 5–7 scaffolded, 8+10 partial

---

## Milestone Status

| # | Milestone | Status | Tests | Notes |
| --- | --- | --- | --: | --- |
| 1 | **common** — types, traits, errors, config, retry | ✅ Done | 19 | `AudioFrame`, `PipelineEvent`, provider traits, `SessionConfig`, env-var substitution, `with_retry`, `TestAudioStream` |
| 2 | **vad** — energy-threshold VAD | ✅ Done | 11 | `rms_energy`, `is_voiced`, `FrameChunker`, `VadComponent` state machine |
| 3 | **core** — orchestrator + session + stubs | ✅ Done | 11 | `Orchestrator` with provider triggering, `PipelineSession` with audio fanout, 7 integration tests (3 E2E) |
| 4 | **transport/websocket** — WS server + protocol | ✅ Done | 6 | Axum handler, `ClientMessage`/`ServerMessage` JSON, bidirectional bridge, audio frame parsing |
| 5 | **asr** — Deepgram provider | 🟡 Partial | 4 | `DeepgramProvider` WS streaming impl, JSON response parsing. **Missing:** integration test with real audio, Whisper fallback |
| 6 | **agent** — OpenAI provider + tool loop | 🟡 Partial | 7 | `OpenAiProvider` SSE streaming, `AgentCore` (max 5 tool iters, 30s timeout), `ConversationMemory`, `Tool` trait. **Missing:** integration test, concrete tool impls, Anthropic fallback |
| 7 | **tts** — ElevenLabs provider | 🟡 Partial | 4 | `ElevenLabsProvider` WS streaming, base64 decode, cancel. **Missing:** integration test, sentence-boundary streaming wiring, Coqui fallback |
| 8 | **Integration** — end-to-end with interrupt | 🟡 Partial | 3 | E2E stub tests (full flow + explicit providers + terminate). **Missing:** interrupt E2E test, backpressure test, audio fixtures |
| 9 | **transport/asterisk** — ARI adapter | ❌ Not started | 0 | `lib.rs` is empty. Needs μ-law/A-law codec, RTP jitter buffer, DTMF mapping |
| 10 | **Observability** — metrics, config validation, fallbacks | 🟡 Partial | 0 | Prometheus metrics (9 metrics), `init_metrics()`, binary entry point, graceful shutdown. **Missing:** fallback provider wiring, session metrics in session.rs |

---

## What's Next

### Priority 1 — Complete provider integration (M5-7)

- [ ] **ASR: Deepgram integration test** — `#[ignore]` test with real audio fixture + API key
- [ ] **ASR: Whisper fallback** — local `whisper-rs` provider
- [ ] **Agent: Anthropic fallback** — `AnthropicProvider` implementing `LlmProvider`
- [ ] **Agent: integration test** — `#[ignore]` test with real OpenAI call
- [ ] **TTS: sentence-boundary streaming** — wire `extract_sentence()` to TTS in the pipeline
- [ ] **TTS: Coqui fallback** — local TTS provider
- [ ] **TTS: integration test** — `#[ignore]` test with real ElevenLabs streaming

### Priority 2 — End-to-end integration (M8)

- [x] **E2E test with stubs** — full pipeline: audio → VAD → ASR → Agent → TTS → egress
- [x] **E2E test with explicit providers** — verify provider injection works
- [x] **Terminate cancels tasks** — verify session cleanup
- [ ] **E2E test with interrupt** — verify interrupt cancels TTS+LLM, returns to Idle
- [ ] **Audio fixtures** — add WAV files to `tests/fixtures/audio/`
- [ ] **FinalTranscript never-drop test** — verify backpressure doesn't lose transcripts

### Priority 3 — Asterisk transport (M9)

- [ ] **ARI WebSocket adapter** — connect to Asterisk ARI
- [ ] **Codec conversion** — μ-law/A-law ↔ PCM i16 (use `audiopus` or manual conversion)
- [ ] **RTP jitter buffer** — 50ms buffer for out-of-order packets
- [ ] **DTMF mapping** — DTMF digits → `PipelineEvent::Cancel`

### Priority 4 — Observability + hardening (M10)

- [x] **Prometheus metrics** — `metrics` + `metrics-exporter-prometheus`, 9 metrics instrumented
- [x] **Config fail-fast** — validate all required keys present at startup, error on missing `${VAR}`
- [x] **Binary entry point** — `voicebot-server` crate with `main.rs`, loads config, inits tracing+metrics, starts Axum server
- [x] **Graceful shutdown** — `SIGTERM`/`SIGINT` handler via `axum::serve::with_graceful_shutdown`
- [ ] **Fallback provider wiring** — primary/fallback config in `core::session`, auto-switch on retry exhaustion
- [ ] **Session metrics** — `session_started()`/`session_ended()` calls in `PipelineSession`

---

## Crate Map

```
voicebot/crates/
  common/          19 tests   ← AudioFrame, PipelineEvent, traits, errors, config
  vad/             11 tests   ← energy VAD, FrameChunker, VadComponent
  asr/              4 tests   ← DeepgramProvider, StubAsrProvider
  agent/            7 tests   ← OpenAiProvider, AgentCore, ConversationMemory, Tool trait
  tts/              4 tests   ← ElevenLabsProvider, StubTtsProvider
  core/            11 tests   ← Orchestrator (with provider triggering), PipelineSession, observability (Prometheus)
  transport/
    websocket/      6 tests   ← Axum WS handler, JSON protocol
    asterisk/       0 tests   ← (empty)
  server/           0 tests   ← Binary entry point (main.rs)
```

## Build & Test

```bash
cd voicebot
source "$HOME/.cargo/env"
cargo test --workspace                    # all 62 tests
cargo test --workspace -- --ignored       # integration tests (need API keys)
cargo run -p voicebot-server              # start server (needs config.toml + env vars)
```
