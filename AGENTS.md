# Agent Notes

## Skills

All agent skills live in `skills/<name>/<name>.md`. Read the relevant skill before working on that area.

| Skill | When to use |
| --- | --- |
| `audio_dsp` | Audio formats, codec conversion, VAD frame sizing, jitter buffer |
| `rust_async` | Tokio patterns, channels, cancellation, spawn_blocking |
| `error_handling_and_fault_tolerance` | Error types, retry matrix, fallback providers |
| `provider_integration` | Deepgram, OpenAI, ElevenLabs API integration |
| `testing_convention` | Test structure, TestAudioStream, mock providers, fixtures |
| `orchestrator_and_pipeline_session` | State machine, session lifecycle, channel wiring, interrupts |
| `websocket_transport` | WS server, binary/JSON framing, session spawning |
| `agent_tool_calling` | Tool loop (max 5 iters), conversation memory, sentence-boundary TTS |
| `configuration` | config.toml parsing, env var substitution, fail-fast validation |
| `observability` | tracing spans (always include session_id), Prometheus metrics |

## Key Rules

- Read `voicebot/README.md` for the crate dependency graph — violations break the build.
- Read `PROJECT_REQUIREMENTS.md` for full specs before implementing any milestone.
- All audio is `AudioFrame` from `common` — 16kHz mono i16. Never redefine.
- All channels bounded. No `unbounded_channel()`. No `unwrap()` in non-test code.
- Build milestones are sequential (1→10). Don't skip ahead.

## Build Order

1. common → 2. vad → 3. core (stubs) → 4. transport/websocket → 5. asr → 6. agent → 7. tts → 8. integration → 9. transport/asterisk → 10. observability
