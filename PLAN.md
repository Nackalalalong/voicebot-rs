# PLAN — voicebot-rs

> 89 tests passing + 16 ignored integration tests · 10 crates implemented · Milestones 1–5 and 7–11 complete, 6 partial

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
| 11 | **loadtest** — virtual phone load harness | ✅ Done | 12+3i | `voicebot-loadtest` crate: Asterisk external-media + xphone native SIP backends, outbound + inbound modes, campaign scheduler (concurrency, ramp-up, rate-limit, soak), stutter scoring, per-call artifacts, Markdown + JSON reports, 3 ignored xphone integration tests |

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

### Priority 6 — Phone call loadtesting (M11)

- [x] **New package `crates/loadtest`** — library + CLI for campaign execution (`cargo run -p voicebot-loadtest -- ...`)
- [x] **Phone backend abstraction** — keep signaling/media backend behind a trait; do not hardwire the first implementation into core or `transport/asterisk`
- [x] **Outbound mode** — Asterisk external-media (Phase 1) and xphone native SIP (Phase 2) backends both support outbound calls with WAV playback, RX recording, and summary output
- [x] **Inbound mode** — register virtual phones, wait for inbound INVITE via `on_incoming` callback, answer automatically, run the same scripted conversation flow; `campaign.mode = "inbound"` config, `Phase1InboundRequest`, xphone-only
- [x] **Campaign scheduler** — `run_campaign`: bounded concurrency, ramp-up, rate-limiting (`call_rate_per_second`), soak mode (`soak_duration_secs`); per-call subdirectory artifacts
- [x] **Audio scoring** — `stutter_count` added to `CallAnalysis`; counts short inter-voiced-region gaps (< `stutter_gap_ms`)
- [x] **Artifacts + reports** — per-call rx WAV in `calls/{N:04}/rx.wav`; `campaign.json` (JSON) + `report.md` (Markdown) with P50/P90/P99 first-response, avg gap, stutter totals
- [x] **Phase 2 native SIP backend** — `XphoneBackend` registers with Asterisk via `xphone` crate, places outbound calls with paced PCM playback, records RX audio, 8k↔16k resampling, spawn_blocking tokio bridge, 2 `#[ignore]` integration tests

---

## Proposed Milestone 11 — Phone Call Load Testing

### Goal

Add a dedicated load-testing package that behaves like many virtual phones against the existing Asterisk + voicebot stack. The harness should be able to:

- register as SIP endpoints
- place outbound calls into the voicebot
- receive inbound calls from the voicebot/Asterisk side
- send prepared WAV files as caller speech at scripted points in the conversation
- record the audio received by the virtual phone
- score responsiveness and smoothness of that received audio
- summarize failures, latency, and quality across a whole campaign

This is a transport-side test harness. It must stay outside the production pipeline and must not couple `core` to SIP or load-test concerns.

### Research-Based Recommendation

The repo already has the right server-side pieces for this work:

- Asterisk ARI transport is implemented, with inbound call handling and per-call AudioSocket bridging.
- ARI REST already supports originate/hangup, which is enough to drive inbound-to-phone scenarios once virtual endpoints exist.
- Asterisk test SIP endpoint config already exists (`voicebot` / extension `1000`) and accepts registrations.
- Canonical internal audio is already stable: 16 kHz mono i16 via `AudioFrame`.
- Session and latency metrics already exist on the voicebot side.

What does not exist yet is a client-side phone emulator. Because Rust SIP user-agent support is much riskier than the current ARI side, the package should be designed around a backend abstraction from day one:

- Phase 1 recommendation: implement a pragmatic backend that can be supervised reliably for registration, call answer/dial, WAV playback, and RX recording.
- Do not bake a native SIP stack assumption into the package API.
- Keep the backend replaceable so a future native Rust UA can be added later without rewriting campaign logic, scoring, or reporting.

After researching the Rust telephony ecosystem, Phase 2 should explicitly target `xphone` as the native SIP backend:

- `xphone` is the strongest fit for the next milestone because it already exposes SIP registration, inbound/outbound call handling, DTMF, and PCM media channels in Rust.
- `fakepbx` is useful as a test helper for SIP integration, but not as the runtime phone backend.
- `rvoip` and C-library bindings (`pjsip`, `sofia-sip`) add more integration risk than value for this repo at the current stage.
- Low-level crates such as `sip-uri`, `sdp`, `rtp`, and `stun` remain optional helpers, not Phase 2 foundations.

### Package Shape

Add a new workspace package:

```text
voicebot/crates/loadtest/
  src/
    lib.rs
    cli.rs
    config.rs
    campaign.rs
    scheduler.rs
    phone/
      mod.rs
      backend.rs
      session.rs
      media.rs
    flow/
      mod.rs
      step.rs
      runner.rs
    analysis/
      mod.rs
      gaps.rs
      stutter.rs
      smoothness.rs
      summary.rs
    report/
      mod.rs
      json.rs
      markdown.rs
      html.rs
  tests/
    campaign_smoke.rs
    asterisk_integration.rs
```

Recommended dependency boundaries:

- `loadtest` may depend on `common` for shared audio utilities and config types where useful.
- `loadtest` must not require `core` internals.
- If ARI originate helpers are needed, either expose a tiny shared ARI helper crate later or duplicate the minimal REST client instead of coupling to `transport/asterisk` internals.

### Core Concepts

#### 1. Campaign

A campaign is the top-level load run. It owns:

- run ID and seed
- target environment
- number of virtual phones
- ramp policy
- concurrency limit
- scenario assignment
- artifact directory
- global stop conditions

Example stop conditions:

- total call attempts reached
- duration exceeded
- registration failure ratio too high
- active call error ratio above threshold
- manual stop signal

#### 2. Virtual Phone

A virtual phone is a long-lived client identity with its own SIP credentials and media state.

Each phone should have:

- phone ID
- SIP username / password / extension
- registration state
- current call state
- media sender for scripted WAV playback
- media receiver/recorder for incoming audio
- per-call statistics

The scheduler should keep a pool of phones so campaigns can model either:

- many short-lived anonymous callers, or
- a stable fleet of registered handsets receiving and placing calls repeatedly

#### 3. Call Session

A call session is one executed scenario on one phone. It should emit a detailed timeline:

- register start / success / failure
- dial start or inbound INVITE received
- answer timestamp
- each script step start / end
- each WAV playback start / end
- first received audio timestamp
- silence-gap windows
- hangup cause
- final result and score

### Backend Abstraction

The phone emulator must not assume one signaling/media implementation. Define a narrow trait boundary such as:

```rust
trait PhoneBackend {
    async fn register(&mut self, phone: &PhoneIdentity) -> Result<(), PhoneError>;
    async fn unregister(&mut self, phone_id: &str) -> Result<(), PhoneError>;
    async fn dial(&mut self, request: OutboundCallRequest) -> Result<ActiveCall, PhoneError>;
    async fn wait_for_inbound(&mut self, phone_id: &str, timeout_ms: u64) -> Result<InboundCall, PhoneError>;
    async fn answer(&mut self, call_id: &str) -> Result<(), PhoneError>;
    async fn play_wav(&mut self, call_id: &str, path: &Path) -> Result<PlaybackHandle, PhoneError>;
    async fn start_recording(&mut self, call_id: &str, path: &Path) -> Result<(), PhoneError>;
    async fn stop_recording(&mut self, call_id: &str) -> Result<(), PhoneError>;
    async fn send_dtmf(&mut self, call_id: &str, digits: &str) -> Result<(), PhoneError>;
    async fn hangup(&mut self, call_id: &str) -> Result<(), PhoneError>;
}
```

That keeps campaign logic independent from the first backend implementation.

### Conversation Flow Interface

The flow layer should support both declarative scenarios and custom Rust extensions.

#### Declarative scenario file

Use TOML for the first version so it matches the rest of the repo.

```toml
[campaign]
name = "basic-outbound"
mode = "outbound"
concurrency = 20
ramp_calls_per_sec = 2

[target]
dial_string = "PJSIP/1000"

[[steps]]
type = "play_wav"
path = "tests/fixtures/audio/hello.wav"

[[steps]]
type = "expect_audio"
within_ms = 1500
min_duration_ms = 500

[[steps]]
type = "wait_for_silence"
max_ms = 8000

[[steps]]
type = "hangup"
```

#### Extensible step runner

Support a Rust trait for advanced or generated flows:

```rust
trait FlowStep {
    async fn run(&self, ctx: &mut CallFlowContext) -> Result<StepOutcome, FlowError>;
}
```

Initial built-in steps:

- `register`
- `wait_for_inbound`
- `dial`
- `answer`
- `play_wav`
- `play_sequence`
- `send_dtmf`
- `wait`
- `expect_audio`
- `wait_for_silence`
- `assert_max_gap`
- `assert_call_duration`
- `hangup`

### Supported Test Modes

#### Outbound load test

Virtual phones initiate calls toward the system under test.

Use this to measure:

- call setup throughput
- answer latency
- first-response latency after caller speech
- end-to-end voice smoothness under concurrency

#### Inbound load test

Virtual phones register first, then wait for the voicebot side to call them.

Use this to measure:

- outbound dialer behavior from the system side
- answer and media establishment success rate
- first media delivery latency after answer
- conversation quality when the system originates many simultaneous calls

For inbound mode, the loadtest package should support two trigger styles:

- external trigger: wait for some other system to place calls
- ARI-assisted trigger: optionally use Asterisk originate to call the registered virtual phones during test setup

### Audio Handling

The media path must be explicit and deterministic.

- Normalize every input WAV before playback.
- Store the normalized copy as a run artifact so analysis can refer to the exact transmitted audio.
- Record received audio per call as WAV.
- Track timestamps for TX start, TX end, RX first non-silent frame, RX silence windows, and hangup.

Normalization rules for version 1:

- accept mono or stereo WAV input
- resample to a backend-supported format before sending
- keep an analysis copy in 16 kHz mono i16 for consistent scoring
- reject unsupported files at campaign startup, not mid-call

### Quality Metrics

The first version should produce simple, explainable scores instead of a fake MOS number.

#### Raw per-call metrics

- registration latency ms
- call setup latency ms
- answer latency ms
- first-response latency ms
- total received audio duration ms
- received silence duration ms
- longest silence gap ms
- count of silence gaps over threshold
- count of likely stutter/repeat events
- count of clipping windows
- call completion result
- SIP / backend error code

#### Gap / processing metrics

Measure perceived dead air from the caller perspective:

- `first_response_ms`: from end of the scripted caller utterance to first non-silent received audio
- `inter_chunk_gap_ms`: max and p95 silent gap between consecutive non-silent received regions
- `turn_gap_ms`: per conversation turn delay between caller playback end and bot response start

These are the core "processing time" metrics the summary should highlight.

#### Stutter heuristics

Keep the first version heuristic-based and transparent:

- repeated short audio windows in RX that are near-identical and adjacent
- unnatural alternation of very short speech and silence windows
- repeated transcript fragments if optional ASR-on-RX is enabled later

Output:

- `stutter_events`
- `stutter_ms_total`
- `stutter_score` from 0 to 100

#### Smoothness score

Define a composite score from transparent penalties:

```text
smooth_score = 100
  - gap_penalty
  - stutter_penalty
  - clipping_penalty
  - underrun_penalty
```

Where penalties are based on thresholds committed in config, not hard-coded magic in the report.

The report should always show both the composite score and its component penalties.

### Optional Phase-2 Analysis

Once the basics work, extend the analyzer with optional checks:

- run ASR on received audio and compare transcripts across calls
- keyword / phrase hit rate for scripted prompts
- semantic drift detection for repeated campaigns
- waveform similarity versus a golden baseline prompt

These are useful, but they should not block the first implementation.

### Reporting and Artifacts

Each campaign should write a deterministic artifact tree such as:

```text
artifacts/loadtest/<run_id>/
  config.resolved.toml
  summary.json
  summary.md
  summary.html
  metrics.ndjson
  calls/
    <call_id>/
      events.ndjson
      tx/
        001_hello.normalized.wav
      rx/
        received.wav
      analysis.json
```

Campaign summary must include:

- attempted / connected / completed / failed call counts
- registration success rate
- answer success rate
- p50 / p95 / p99 first-response latency
- p50 / p95 longest silence gap
- average and worst smoothness score
- top error classes
- slowest calls and most degraded calls

### Config Surface

Add a dedicated loadtest config file rather than stretching `voicebot/config.toml`.

Suggested top-level sections:

- `[backend]`
- `[campaign]`
- `[target]`
- `[phones]`
- `[media]`
- `[analysis]`
- `[report]`

Important config fields:

- backend type
- registrar / SIP target / dial string
- credentials template
- phone count
- concurrency
- ramp-up calls/sec
- max active calls
- retries and backoff
- artifact directory
- silence threshold ms
- stutter window ms
- score thresholds

### Failure Model

The loadtest harness should classify failures instead of collapsing everything into "call failed":

- registration failed
- registration timed out
- outbound dial failed
- inbound call not received
- answer failed
- media playback failed
- receive recording failed
- no bot audio returned
- excessive silence gap
- stutter threshold exceeded
- abnormal hangup / backend crash

This matters because scaling bugs and media-quality bugs are different classes of regressions.

### Execution Phases

#### Phase 1 — single-call MVP

- one virtual phone
- outbound mode only
- play one WAV, record RX, write summary JSON
- no scoring beyond first-response and silence-gap metrics

#### Phase 2 — xphone-backed virtual phones

- add an `xphone` backend for native SIP registration and call control
- support reusable virtual-phone identities with register / unregister lifecycle
- add inbound and outbound call execution on top of the same flow runner
- keep the existing Asterisk external-media backend as a simpler fallback backend

#### Phase 3 — inbound mode

- campaign scheduler, phone pool, concurrency, retries, and stop conditions
- same flow runner reused across Asterisk-backed and `xphone`-backed sessions
- artifact tree growth from single-call outputs to campaign outputs

#### Phase 4 — scoring and quality analytics

- stutter heuristics
- composite smoothness score
- per-turn and per-call degradation ranking

#### Phase 5 — hardening

- soak tests
- failure-injection scenarios
- deterministic fixtures and golden summaries
- optional ASR-on-RX analysis

### Test Strategy

- unit tests for config parsing, flow runner, gap detection, and scoring
- fixture-driven tests for WAV normalization and audio analysis
- ignored Asterisk integration tests for register/dial/answer/record flows
- one small smoke campaign in CI-safe mode using a mocked backend

### Non-Goals for the First Cut

- building a full SIP stack inside `core`
- modifying the voicebot pipeline to know about load tests
- real-time dashboards before artifact generation works
- black-box "AI quality score" with no transparent inputs

### Deliverable Standard for the Later Implementation Prompt

The eventual implementation should not stop at "it can make a call". A usable first deliverable must include all of the following together:

- repeatable campaign config
- at least one working outbound scenario
- recorded RX audio artifact per call
- per-call latency metrics
- a campaign summary file
- integration coverage for the happy path and at least one failure path

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
