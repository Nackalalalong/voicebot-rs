# PLAN — voicebot-rs

> 69 tests passing + 13 ignored integration tests · 9 crates · Milestones 1–5 and 7–10 complete, 6 partial

---

## Milestone Status

| # | Milestone | Status | Tests | Notes |
| --- | --- | --- | --: | --- |
| 1 | **common** — types, traits, errors, config, retry | ✅ Done | 19 | `AudioFrame`, `PipelineEvent`, provider traits, `SessionConfig`, env-var substitution, `with_retry`, `TestAudioStream` |
| 2 | **vad** — energy-threshold VAD + Speaches | ✅ Done | 14 | `rms_energy`, `is_voiced`, `FrameChunker`, `VadComponent` state machine, `SpeachesVadClient` batch VAD via `/v1/audio/speech/timestamps` |
| 3 | **core** — orchestrator + session + stubs | ✅ Done | 13+2i | `Orchestrator` with provider triggering, sentence-boundary TTS streaming, cooperative barge-in cancellation for ASR/LLM/TTS, partial assistant history retention on LLM interrupt, `PipelineSession` with per-utterance ASR fanout, 7 integration tests (3 E2E), `build_providers()` factory, `start_with_config()`. 2 ignored Speaches tests |
| 4 | **transport/websocket** — WS server + protocol | ✅ Done | 6 | Axum handler, dual router (`router()` stubs, `router_with_config()` real), `ClientMessage`/`ServerMessage` JSON, bidirectional bridge |
| 5 | **asr** — Speaches OpenAI-compatible provider | ✅ Done | 2+3i | `SpeachesAsrProvider` multipart POST to `/v1/audio/transcriptions` with SSE streaming (`stream: true`), partial transcript events. 3 ignored Speaches integration tests. **Skipped:** Realtime WS mode (Speaches bug) |
| 6 | **agent** — OpenAI-compatible provider + tool loop | 🟡 Partial | 8 | `OpenAiProvider` SSE streaming with configurable `base_url` (works with any OpenAI-compatible server), `AgentCore` (max 5 tool iters, 30s timeout), `ConversationMemory`, streamed-partial retention on cancellation, `Tool` trait. **Missing:** integration test, concrete tool impls |
| 7 | **tts** — Speaches OpenAI-compatible provider | ✅ Done | 2+4i | `SpeachesTtsProvider` streaming PCM from `/v1/audio/speech`, cancel support, sentence-boundary streaming wired in orchestrator. 4 ignored Speaches integration tests |
| 8 | **Integration** — end-to-end with interrupt | ✅ Done | 8 | E2E stub tests (full flow + explicit providers + terminate + VAD + backpressure), barge-in interrupt test, sentence-boundary test, new-speech-cancels-previous-ASR regression |
| 9 | **transport/asterisk** — ARI adapter | ✅ Done | 4i | AudioSocket+slin16 approach (no codec conversion needed). ARI WS event loop, per-call ephemeral TCP port, CancellationToken per call, DTMF → terminate, local Docker Compose verified from project root, ignored integration tests for ARI REST, WebSocket, endpoint status, and originate/hangup. |
| 10 | **Observability** — metrics, config validation, fallbacks | ✅ Done | 2 | Prometheus metrics (9 metrics), `init_metrics()`, binary entry point, graceful shutdown, fallback provider wiring in `core::session`, session lifecycle metrics in `PipelineSession` |

---

## Infrastructure

| Component | Status | Notes |
| --- | --- | --- |
| **Speaches server** | ✅ Running | `system/speaches/compose.cpu.yaml`, CPU mode, HF cache bind-mounted to host |
| **Speaches skill** | ✅ Done | `skills/speaches/SKILL.md` — integration patterns for ASR, TTS, VAD, Realtime WS |
| **API reference** | ✅ Done | `docs/speaches/api-reference.md` — full endpoint reference from source |
| **Asterisk ARI skill** | ✅ Done | `skills/asterisk_ari/SKILL.md` — ARI Stasis lifecycle, AudioSocket wire protocol, REST channel control, event deserialization, DTMF handling |
| **Web demo** | ✅ Done | `system/voicebot-core-demo/` — browser mic → WS → chat UI + TTS playback; `start.sh` boots Speaches + voicebot server + static server in one command |

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
- [~] **Realtime WebSocket ASR** — use Speaches `/v1/realtime` for full-duplex audio streaming (lower latency) _(skipped — Speaches bug)_
- [x] **Sentence-boundary TTS** — orchestrator extracts sentences from agent partial responses, sends each to TTS immediately
- [x] **Barge-in interrupt** — SpeechStarted during Transcribing, AgentThinking, or Speaking cancels the previous turn and returns to Listening
- [x] **Interrupted partial history retention** — keep only the already streamed assistant text when LLM output is cut off by new speech

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
- [x] **ASR interrupt regression** — new speech cancels an older ASR transcription that is still finishing
- [ ] **FinalTranscript never-drop test** — verify backpressure doesn't lose transcripts

### Priority 3 — Agent improvements (M6)

- [x] Configurable `base_url` for OpenAI-compatible servers (Speaches, vLLM, Ollama, etc.)
- [ ] **Concrete tool impls** — at least one working tool (e.g. time lookup, weather stub)
- [ ] **Agent integration test** — `#[ignore]` test with real LLM call

### Priority 4 — Asterisk transport (M9)

- [x] **ARI skill written** — `skills/asterisk_ari/SKILL.md` with full protocol reference
- [x] **AsterisConfig in AppConfig** — `[asterisk]` section parsed from config.toml
- [x] **ARI WebSocket adapter** — connect to Asterisk ARI WS event stream, dispatch StasisStart/End/DTMF
- [x] **AudioSocket TCP server** — per-call ephemeral port, packet encode/decode (0x00/0x01/0x10)
- [x] **slin16 passthrough** — AudioSocket `format=slin16` means no codec conversion needed
- [x] **ARI REST client** — answer, externalMedia, bridge, hangup via reqwest
- [x] **DTMF → cancel** — `#` or `*` cancels session via CancellationToken
- [x] **Bridge cleanup** — destroy bridge and hang up on session end or error
- [x] **Integration test** — `#[ignore]` tests with real Asterisk
- [x] **Docker Compose** — `system/asterisk/docker-compose.yaml` for local testing

### Priority 5 — Observability + hardening (M10)

- [x] Prometheus metrics — 9 metrics instrumented
- [x] Config fail-fast — validate all required keys at startup
- [x] Binary entry point + graceful shutdown
- [x] **Fallback provider wiring** — primary/fallback config in `core::session`, auto-switch on retry exhaustion
- [x] **Session metrics** — `session_started()`/`session_ended()` calls in `PipelineSession`

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
    asterisk/       0+4i      ← ARI REST/WebSocket/originate integration tests
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
