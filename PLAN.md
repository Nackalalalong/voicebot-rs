# PLAN — voicebot-rs

> 63 tests passing + 9 ignored integration tests · 9 crates · Milestones 1–7 complete, 8+10 partial

---

## Milestone Status

| # | Milestone | Status | Tests | Notes |
| --- | --- | --- | --: | --- |
| 1 | **common** — types, traits, errors, config, retry | ✅ Done | 19 | `AudioFrame`, `PipelineEvent`, provider traits, `SessionConfig`, env-var substitution, `with_retry`, `TestAudioStream` |
| 2 | **vad** — energy-threshold VAD + Speaches | ✅ Done | 14 | `rms_energy`, `is_voiced`, `FrameChunker`, `VadComponent` state machine, `SpeachesVadClient` batch VAD via `/v1/audio/speech/timestamps` |
| 3 | **core** — orchestrator + session + stubs | ✅ Done | 13+2i | `Orchestrator` with provider triggering, sentence-boundary TTS streaming, barge-in interrupt, `PipelineSession` with audio fanout, 7 integration tests (3 E2E), `build_providers()` factory, `start_with_config()`. 2 ignored Speaches tests |
| 4 | **transport/websocket** — WS server + protocol | ✅ Done | 6 | Axum handler, dual router (`router()` stubs, `router_with_config()` real), `ClientMessage`/`ServerMessage` JSON, bidirectional bridge |
| 5 | **asr** — Speaches OpenAI-compatible provider | ✅ Done | 2+3i | `SpeachesAsrProvider` multipart POST to `/v1/audio/transcriptions` with SSE streaming (`stream: true`), partial transcript events. 3 ignored Speaches integration tests. **Missing:** Realtime WS mode |
| 6 | **agent** — OpenAI-compatible provider + tool loop | 🟡 Partial | 7 | `OpenAiProvider` SSE streaming with configurable `base_url` (works with any OpenAI-compatible server), `AgentCore` (max 5 tool iters, 30s timeout), `ConversationMemory`, `Tool` trait. **Missing:** integration test, concrete tool impls |
| 7 | **tts** — Speaches OpenAI-compatible provider | ✅ Done | 2+4i | `SpeachesTtsProvider` streaming PCM from `/v1/audio/speech`, cancel support, sentence-boundary streaming wired in orchestrator. 4 ignored Speaches integration tests |
| 8 | **Integration** — end-to-end with interrupt | ✅ Done | 7 | E2E stub tests (full flow + explicit providers + terminate + VAD + backpressure), barge-in interrupt test, sentence-boundary test |
| 9 | **transport/asterisk** — ARI adapter | ❌ Not started | 0 | `lib.rs` is empty. Needs μ-law/A-law codec, RTP jitter buffer, DTMF mapping |
| 10 | **Observability** — metrics, config validation, fallbacks | 🟡 Partial | 0 | Prometheus metrics (9 metrics), `init_metrics()`, binary entry point, graceful shutdown. **Missing:** fallback provider wiring, session metrics in session.rs |

---

## Infrastructure

| Component | Status | Notes |
| --- | --- | --- |
| **Speaches server** | ✅ Running | `system/speaches/compose.cpu.yaml`, CPU mode, HF cache bind-mounted to host |
| **Speaches skill** | ✅ Done | `skills/speaches/SKILL.md` — integration patterns for ASR, TTS, VAD, Realtime WS |
| **API reference** | ✅ Done | `docs/speaches/api-reference.md` — full endpoint reference from source |
| **Audio fixtures** | ✅ Done | `tests/fixtures/audio/` — `sine_440hz_1s.wav`, `silence_1s.wav` (16kHz mono i16) |
| **Web demo** | ✅ Done | `system/voicebot-core-demo/` — browser mic → WS → chat UI + TTS playback |

---

## Provider Strategy

All providers use OpenAI-compatible APIs. Speaches implements these APIs locally; any OpenAI-compatible server (vLLM, Ollama, LiteLLM, etc.) can be swapped in via `base_url`.

| Component | Provider | API Endpoint | Base URL (default) |
| --- | --- | --- | --- |
| **ASR** | Speaches | `POST /v1/audio/transcriptions` | `http://localhost:8000` |
| **TTS** | Speaches | `POST /v1/audio/speech` | `http://localhost:8000` |
| **LLM** | OpenAI-compatible | `POST /v1/chat/completions` (SSE stream) | `http://localhost:8000` |

### OpenAI STT API (used by ASR)

- **Endpoint:** `POST /v1/audio/transcriptions`
- **Input:** multipart form — `file` (audio), `model`, `language`, `response_format`
- **Models:** `whisper-1`, `gpt-4o-transcribe`, `gpt-4o-mini-transcribe`
- **Formats:** `json`, `text`, `verbose_json`, `srt`, `vtt`
- **Streaming:** `stream: true` → SSE with `transcript.text.delta` / `transcript.text.done` events
- **Realtime:** `wss://…/v1/realtime?intent=transcription` for full-duplex audio streaming

### OpenAI TTS API (used by TTS)

- **Endpoint:** `POST /v1/audio/speech`
- **Input:** JSON — `model`, `voice`, `input`, `response_format`, `instructions`
- **Models:** `gpt-4o-mini-tts`, `tts-1`, `tts-1-hd`
- **Output formats:** `mp3` (default), `opus`, `aac`, `flac`, `wav`, `pcm` (24kHz 16-bit LE raw)
- **Streaming:** chunked transfer encoding — audio plays before full generation
- **Best latency:** use `wav` or `pcm` format

---

## What's Next

### Priority 1 — Realtime WS ASR (M5)

- [x] **SSE streaming ASR** — use `stream: true` on `/v1/audio/transcriptions` to emit `PartialTranscript` events
- [ ] **Realtime WebSocket ASR** — use Speaches `/v1/realtime` for full-duplex audio streaming (lower latency)
- [x] **Sentence-boundary TTS** — orchestrator extracts sentences from agent partial responses, sends each to TTS immediately
- [x] **Barge-in interrupt** — SpeechStarted during Speaking cancels TTS and returns to Listening

### Priority 2 — End-to-end integration (M8)

- [x] E2E test with stubs — full pipeline: audio → VAD → ASR → Agent → TTS → egress
- [x] E2E test with explicit providers — verify provider injection works
- [x] Terminate cancels tasks — verify session cleanup
- [x] Audio fixtures — WAV files for deterministic testing
- [x] ASR integration tests — 3 `#[ignore]` tests with Speaches
- [x] TTS integration tests — 4 `#[ignore]` tests with Speaches
- [x] E2E pipeline integration test — `#[ignore]` test using Speaches ASR+TTS
- [x] **E2E interrupt test** — barge-in during speaking cancels TTS, returns to Listening
- [x] **Sentence boundary test** — verifies sentence extraction and flushing logic
- [ ] **FinalTranscript never-drop test** — verify backpressure doesn't lose transcripts

### Priority 3 — Agent improvements (M6)

- [x] Configurable `base_url` for OpenAI-compatible servers (Speaches, vLLM, Ollama, etc.)
- [ ] **Concrete tool impls** — at least one working tool (e.g. time lookup, weather stub)
- [ ] **Agent integration test** — `#[ignore]` test with real LLM call

### Priority 4 — Asterisk transport (M9)

- [ ] **ARI WebSocket adapter** — connect to Asterisk ARI
- [ ] **Codec conversion** — μ-law/A-law ↔ PCM i16
- [ ] **RTP jitter buffer** — 50ms buffer for out-of-order packets
- [ ] **DTMF mapping** — DTMF digits → `PipelineEvent::Cancel`

### Priority 5 — Observability + hardening (M10)

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
  asr/              6+3i      ← SpeachesAsrProvider, StubAsrProvider
  agent/            7 tests   ← OpenAiProvider (base_url), AgentCore, ConversationMemory, Tool trait
  tts/              6+4i      ← SpeachesTtsProvider, StubTtsProvider
  core/            11+2i      ← Orchestrator, PipelineSession, build_providers(), observability
  transport/
    websocket/      6 tests   ← Axum WS handler (stubs + config), JSON protocol
    asterisk/       0 tests   ← (empty)
  server/           0 tests   ← Binary entry point (main.rs)
```

## Build & Test

```bash
cd voicebot
cargo test --workspace                    # all unit tests
cargo test --workspace -- --include-ignored  # +integration tests (need Speaches running)
cargo run -p voicebot-server              # start server (needs config.toml + env vars)

# Start local Speaches (CPU mode)
cd system/speaches
docker compose --env-file .env.override -f compose.cpu.yaml up -d
```
