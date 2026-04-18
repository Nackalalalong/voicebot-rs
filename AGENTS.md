# Agent Notes

## Skills

All agent skills live in `skills/<name>/SKILL.md`. Read the relevant skill before working on that area.

> **⚠️ `skills/rust/SKILL.md` is MANDATORY for ALL Rust code changes.** Read it before writing, reviewing, or refactoring any Rust code. It contains 179 rules across 14 categories (ownership, error handling, async, API design, memory, performance, testing, anti-patterns). Non-compliance causes recurring bugs.

| Skill | When to use |
| --- | --- |
| **`rust`** | **ALWAYS when writing/reviewing/refactoring Rust code.** 179 rules: ownership, error handling, async patterns, API design, memory optimization, performance, testing, anti-patterns. Read FIRST before any Rust work. |
| `rust_async` | Tokio patterns, channels, cancellation, spawn_blocking |
| `audio_dsp` | Audio formats, codec conversion, VAD frame sizing, jitter buffer |
| `robuto` | resample audio in Rust — changing sample rate, normalizing mic input to 16 kHz for ASR, handling variable-rate streams, or converting between 8/16/44.1/48 kHz. Trigger on: rubato, resample, sample rate, 16kHz, 48kHz, SincFixedIn, SincFixedOut, rate conversion, audio format normalization. |
| `error_handling_and_fault_tolerance` | Error types, retry matrix, fallback providers |
| `provider_integration` | OpenAI-compatible API integration (Speaches, vLLM, etc.) |
| `speaches` | Speaches server: ASR (faster-whisper), TTS (Kokoro/Piper), Realtime WS |
| `testing_convention` | Test structure, TestAudioStream, mock providers, fixtures |
| `orchestrator_and_pipeline_session` | State machine, session lifecycle, channel wiring, interrupts |
| `websocket_transport` | WS server, binary/JSON framing, session spawning |
| `xphone` | Native SIP virtual phone for loadtest: xphone crate API, registration, inbound/outbound calls, PCM audio I/O, tokio bridge, Asterisk endpoint config |
| `agent_tool_calling` | Tool loop (max 5 iters), conversation memory, sentence-boundary TTS |
| `asterisk_ari` | Asterisk ARI Stasis app: WS events, AudioSocket protocol, REST channel control, DTMF |
| `configuration` | config.toml parsing, env var substitution, fail-fast validation |
| `observability` | tracing spans (always include session_id), Prometheus metrics |
| `database` | sqlx patterns, migrations, compile-time queries, transactions, PG RLS, connection pool |
| `redis_cache` | Redis async patterns, session state, config cache, pub/sub, TTL, rate limiting |
| `rest_api` | Axum REST patterns, middleware stack, extractors, error responses, pagination, OpenAPI/utoipa |
| `auth_multi_tenant` | JWT auth, multi-tenant isolation, RBAC, password hashing, tenant context, RLS |
| `nextjs_dashboard` | Next.js 15 App Router, thin BFF, server/client components, TanStack Query, shadcn/ui, SSE, httpOnly cookie auth |
| `campaign_management` | Campaign lifecycle, custom metrics, contact lists, outbound dialing, scheduling, post-call analysis |
| `object_storage` | S3-compatible storage, aws-sdk-s3, multipart upload, presigned URLs, RustFS/MinIO |

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

- Read `PLAN.md` for the full project plan and current status before starting work.
- Read `voicebot/README.md` for the crate dependency graph — violations break the build.
- All audio is `AudioFrame` from `common` — 16kHz mono i16. Never redefine.
- All channels bounded. No `unbounded_channel()`. No `unwrap()` in non-test code.
- **Microservice architecture**: Management API, Voicebot Core, and Campaign Scheduler are separate binaries.
- **Rust-first** for all backend services. Frontend is Next.js (dashboard/).
- **Multi-tenant**: every DB query must scope to tenant_id. RLS as defense-in-depth.
- **Stateless core**: session state in Redis, CDR flushed to PostgreSQL, recordings to S3.

## Service Architecture

| Service | Binary/Package | Purpose |
| --- | --- | --- |
| **Management API** | `crates/api` (Axum) | Auth, CRUD, analytics, SSE |
| **Voicebot Core** | `crates/server` (existing) | Real-time audio pipeline, WS + ARI transports |
| **Campaign Scheduler** | `crates/scheduler` (apalis) | Outbound dialer, post-call analysis, retry |
| **Dashboard** | `dashboard/` (Next.js 15) | Customer-facing UI, thin BFF |

## Build Order (Voice Engine — Complete)

1. common → 2. vad → 3. core → 4. transport/websocket → 5. asr → 6. agent → 7. tts → 8. integration → 9. transport/asterisk → 10. observability → 11. loadtest

## Build Order (Production Platform — Next)

A. Data Layer (db, cache, auth, storage) → B. Management API → C. Stateless Core → D. Scheduler ∥ E. Dashboard → F. Integration
