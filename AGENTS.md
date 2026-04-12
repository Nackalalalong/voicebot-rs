# Agent Notes

## Skills

All agent skills live in `skills/<name>/SKILL.md`. Read the relevant skill before working on that area.

| Skill | When to use |
| --- | --- |
| `audio_dsp` | Audio formats, codec conversion, VAD frame sizing, jitter buffer |
| `rust_async` | Tokio patterns, channels, cancellation, spawn_blocking |
| `rust` | Comprehensive Rust coding guidelines with 179 rules across 14 categories. Use when writing, reviewing, or refactoring Rust code. Covers ownership, error handling, async patterns, API design, memory optimization, performance, testing, and common anti-patterns |
| `robuto` | resample audio in Rust — changing sample rate, normalizing mic input to 16 kHz for ASR, handling variable-rate streams, or converting between 8/16/44.1/48 kHz. Trigger on: rubato, resample, sample rate, 16kHz, 48kHz, SincFixedIn, SincFixedOut, rate conversion, audio format normalization. |
| `error_handling_and_fault_tolerance` | Error types, retry matrix, fallback providers |
| `provider_integration` | OpenAI-compatible API integration (Speaches, vLLM, etc.) |
| `speaches` | Speaches server: ASR (faster-whisper), TTS (Kokoro/Piper), Realtime WS |
| `testing_convention` | Test structure, TestAudioStream, mock providers, fixtures |
| `orchestrator_and_pipeline_session` | State machine, session lifecycle, channel wiring, interrupts |
| `websocket_transport` | WS server, binary/JSON framing, session spawning |
| `agent_tool_calling` | Tool loop (max 5 iters), conversation memory, sentence-boundary TTS |
| `asterisk_ari` | Asterisk ARI Stasis app: WS events, AudioSocket protocol, REST channel control, DTMF |
| `configuration` | config.toml parsing, env var substitution, fail-fast validation |
| `observability` | tracing spans (always include session_id), Prometheus metrics |

## Docker Compose

All `docker compose` commands **must be run from the project root** (`/home/nack/voicebot-rs`) using `-f`:

```bash
# Correct — run from project root
docker compose -f system/asterisk/docker-compose.yaml up -d
docker compose -f system/speaches/compose.cpu.yaml up -d
docker compose -f system/voicebot-core-demo/docker-compose.yaml up -d
```

Volume paths in compose files are relative to the compose file's directory (Docker Compose standard). YAML `extends:` references also use paths relative to the file. Running from a subdirectory breaks `extends` chains — always use `-f` from root.

## Key Rules

- Read `voicebot/README.md` for the crate dependency graph — violations break the build.
- Read `PROJECT_REQUIREMENTS.md` for full specs before implementing any milestone.
- All audio is `AudioFrame` from `common` — 16kHz mono i16. Never redefine.
- All channels bounded. No `unbounded_channel()`. No `unwrap()` in non-test code.
- Build milestones are sequential (1→10). Don't skip ahead.

## Build Order

1. common → 2. vad → 3. core (stubs) → 4. transport/websocket → 5. asr → 6. agent → 7. tts → 8. integration → 9. transport/asterisk → 10. observability
