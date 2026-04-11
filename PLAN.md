# PLAN — voicebot-rs

> 69 tests passing · 9 crates · Milestones 1–4 complete, 5–7 scaffolded with Speaches providers, 8+10 partial

---

## Milestone Status

| # | Milestone | Status | Tests | Notes |
| --- | --- | --- | --: | --- |
| 1 | **common** — types, traits, errors, config, retry | ✅ Done | 19 | `AudioFrame`, `PipelineEvent`, provider traits, `SessionConfig`, env-var substitution, `with_retry`, `TestAudioStream` |
| 2 | **vad** — energy-threshold VAD + Speaches | ✅ Done | 14 | `rms_energy`, `is_voiced`, `FrameChunker`, `VadComponent` state machine, `SpeachesVadClient` batch VAD via `/v1/audio/speech/timestamps` |
| 3 | **core** — orchestrator + session + stubs | ✅ Done | 11 | `Orchestrator` with provider triggering, `PipelineSession` with audio fanout, 7 integration tests (3 E2E) |
| 4 | **transport/websocket** — WS server + protocol | ✅ Done | 6 | Axum handler, `ClientMessage`/`ServerMessage` JSON, bidirectional bridge, audio frame parsing |
| 5 | **asr** — Deepgram + Speaches providers | 🟡 Partial | 6 | `DeepgramProvider` WS streaming, `SpeachesAsrProvider` multipart POST to `/v1/audio/transcriptions`. **Missing:** integration test with real audio, streaming SSE mode |
| 6 | **agent** — OpenAI provider + tool loop | 🟡 Partial | 7 | `OpenAiProvider` SSE streaming, `AgentCore` (max 5 tool iters, 30s timeout), `ConversationMemory`, `Tool` trait. **Missing:** integration test, concrete tool impls, Anthropic fallback |
| 7 | **tts** — ElevenLabs + Speaches providers | 🟡 Partial | 6 | `ElevenLabsProvider` WS streaming, `SpeachesTtsProvider` streaming PCM from `/v1/audio/speech`, cancel support. **Missing:** integration test, sentence-boundary streaming wiring |
| 8 | **Integration** — end-to-end with interrupt | 🟡 Partial | 3 | E2E stub tests (full flow + explicit providers + terminate). **Missing:** interrupt E2E test, backpressure test, audio fixtures |
| 9 | **transport/asterisk** — ARI adapter | ❌ Not started | 0 | `lib.rs` is empty. Needs μ-law/A-law codec, RTP jitter buffer, DTMF mapping |
| 10 | **Observability** — metrics, config validation, fallbacks | 🟡 Partial | 0 | Prometheus metrics (9 metrics), `init_metrics()`, binary entry point, graceful shutdown. **Missing:** fallback provider wiring, session metrics in session.rs |

---

## Infrastructure

| Component | Status | Notes |
| --- | --- | --- |
| **Speaches server** | ✅ Running | `system/speaches/compose.cpu.yaml`, CPU mode, HF cache bind-mounted to host |
| **Speaches skill** | ✅ Done | `skills/speaches/SKILL.md` — integration patterns for ASR, TTS, VAD, Realtime WS |
| **API reference** | ✅ Done | `docs/speaches/api-reference.md` — full endpoint reference from source |

---

## What's Next

### Priority 1 — Wire Speaches providers end-to-end (M5, M7, M8)

Speaches replaces the need for external API keys during development. Focus on getting a working local loop.

- [ ] **ASR integration test** — `#[ignore]` test hitting local Speaches `/v1/audio/transcriptions` with a WAV fixture
- [ ] **TTS integration test** — `#[ignore]` test hitting local Speaches `/v1/audio/speech`, verify PCM output
- [ ] **Audio fixtures** — add short WAV/PCM files to `tests/fixtures/audio/` for deterministic testing
- [ ] **Provider factory** — wire `SpeachesAsrProvider` and `SpeachesTtsProvider` into `core::session` based on config
- [ ] **Sentence-boundary TTS** — wire `extract_sentence()` from agent output to TTS `text_rx` in the pipeline

### Priority 2 — Streaming ASR via Speaches (M5)

The current `SpeachesAsrProvider` does batch transcription (collect all audio, then POST). For real-time use, add streaming support.

- [ ] **SSE streaming ASR** — use `stream: true` on `/v1/audio/transcriptions` to emit `PartialTranscript` events
- [ ] **Realtime WebSocket ASR** — use Speaches `/v1/realtime` for full-duplex audio streaming (lower latency)

### Priority 3 — End-to-end integration (M8)

- [x] E2E test with stubs — full pipeline: audio → VAD → ASR → Agent → TTS → egress
- [x] E2E test with explicit providers — verify provider injection works
- [x] Terminate cancels tasks — verify session cleanup
- [ ] **E2E with Speaches** — full pipeline using local Speaches for ASR+TTS (no external API keys)
- [ ] **E2E interrupt test** — verify interrupt cancels TTS+LLM, returns to Idle
- [ ] **FinalTranscript never-drop test** — verify backpressure doesn't lose transcripts

### Priority 4 — Agent improvements (M6)

- [ ] **Anthropic fallback** — `AnthropicProvider` implementing `LlmProvider`
- [ ] **Concrete tool impls** — at least one working tool (e.g. time lookup, weather stub)
- [ ] **Agent integration test** — `#[ignore]` test with real LLM call

### Priority 5 — Asterisk transport (M9)

- [ ] **ARI WebSocket adapter** — connect to Asterisk ARI
- [ ] **Codec conversion** — μ-law/A-law ↔ PCM i16
- [ ] **RTP jitter buffer** — 50ms buffer for out-of-order packets
- [ ] **DTMF mapping** — DTMF digits → `PipelineEvent::Cancel`

### Priority 6 — Observability + hardening (M10)

- [x] Prometheus metrics — 9 metrics instrumented
- [x] Config fail-fast — validate all required keys at startup
- [x] Binary entry point + graceful shutdown
- [ ] **Fallback provider wiring** — primary/fallback config in `core::session`, auto-switch on retry exhaustion
- [ ] **Session metrics** — `session_started()`/`session_ended()` calls in `PipelineSession`

---

## Crate Map

```
voicebot/crates/
  common/          19 tests   ← AudioFrame, PipelineEvent, traits, errors, config
  vad/             14 tests   ← energy VAD, FrameChunker, VadComponent, SpeachesVadClient
  asr/              6 tests   ← DeepgramProvider, SpeachesAsrProvider, StubAsrProvider
  agent/            7 tests   ← OpenAiProvider, AgentCore, ConversationMemory, Tool trait
  tts/              6 tests   ← ElevenLabsProvider, SpeachesTtsProvider, StubTtsProvider
  core/            11 tests   ← Orchestrator, PipelineSession, observability (Prometheus)
  transport/
    websocket/      6 tests   ← Axum WS handler, JSON protocol
    asterisk/       0 tests   ← (empty)
  server/           0 tests   ← Binary entry point (main.rs)
```

## Build & Test

```bash
cd voicebot
cargo test --workspace                    # all 69 tests
cargo test --workspace -- --ignored       # integration tests (need Speaches running or API keys)
cargo run -p voicebot-server              # start server (needs config.toml + env vars)

# Start local Speaches (CPU mode)
cd system/speaches
docker compose --env-file .env.override -f compose.cpu.yaml up -d
```
