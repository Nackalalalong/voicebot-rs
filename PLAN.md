# PLAN — voicebot-rs

> Production telephony platform: multi-tenant, campaign-driven, horizontally scalable.

---

## Part 1 — Voice Engine (Complete)

> 89 tests passing + 16 ignored integration tests · 10 crates implemented

### Milestone Status

| # | Milestone | Status | Tests | Notes |
| --- | --- | --- | --: | --- |
| 1 | **common** — types, traits, errors, config, retry | ✅ Done | 19 | `AudioFrame`, `PipelineEvent`, provider traits, `SessionConfig`, env-var substitution, `with_retry`, `TestAudioStream` |
| 2 | **vad** — energy-threshold VAD + Speaches | ✅ Done | 14 | `rms_energy`, `is_voiced`, `FrameChunker`, `VadComponent` state machine, `SpeachesVadClient` batch VAD |
| 3 | **core** — orchestrator + session + stubs | ✅ Done | 13+2i | `Orchestrator` with provider triggering, sentence-boundary TTS streaming, cooperative barge-in, partial assistant history retention, `PipelineSession` with per-utterance ASR fanout |
| 4 | **transport/websocket** — WS server + protocol | ✅ Done | 6 | Axum handler, dual router, `ClientMessage`/`ServerMessage` JSON, bidirectional bridge |
| 5 | **asr** — Speaches OpenAI-compatible provider | ✅ Done | 2+3i | `SpeachesAsrProvider` multipart POST + SSE streaming. Skipped: Realtime WS (Speaches bug) |
| 6 | **agent** — OpenAI-compatible provider + tool loop | 🟡 Partial | 8 | `OpenAiProvider` SSE streaming, `AgentCore` (max 5 tool iters), `ConversationMemory`, `Tool` trait. Missing: integration test, concrete tool impls |
| 7 | **tts** — Speaches OpenAI-compatible provider | ✅ Done | 2+4i | `SpeachesTtsProvider` streaming PCM, cancel support, sentence-boundary streaming |
| 8 | **Integration** — end-to-end with interrupt | ✅ Done | 8 | E2E stub tests, barge-in interrupt, sentence-boundary, ASR interrupt regression |
| 9 | **transport/asterisk** — ARI adapter | 🟡 Partial | 4i | ARI WS event loop, bridge lifecycle, DTMF → terminate. Current runtime path uses RTP externalMedia + `ulaw`; AudioSocket+slin16 path exists in crate but is not the active transport |
| 10 | **Observability** — metrics, config, fallbacks | ✅ Done | 2 | Prometheus (9 metrics), binary entry point, graceful shutdown, fallback provider wiring |
| 11 | **loadtest** — virtual phone load harness | ✅ Done | 12+3i | xphone SIP backend, outbound+inbound, campaign scheduler, stutter scoring, per-call artifacts |

### Remaining Voice Engine Items

- [ ] Agent: concrete tool impls (at least one working tool)
- [ ] Agent: integration test with real LLM
- [ ] FinalTranscript never-drop backpressure test
- [ ] Asterisk: choose one production media path and finish it end-to-end. Either wire AudioSocket+slin16 as planned, or update the plan/docs/config to bless RTP externalMedia + `ulaw` and prove it with a live-call smoke test

### Infrastructure

| Component | Status |
| --- | --- |
| Speaches server | ✅ `system/speaches/compose.cpu.yaml` |
| Asterisk ARI | 🟡 Compose exists in `system/asterisk/docker-compose.yaml`, but runtime path needs re-validation against the current transport implementation |
| Web demo | ✅ `system/voicebot-core-demo/` |
| Monitoring | ✅ `system/monitoring/` (Prometheus + Grafana) |

### Provider Strategy

All providers use OpenAI-compatible APIs via `base_url`.

| Component | Default Provider | API Endpoint |
| --- | --- | --- |
| ASR | Speaches | `POST /v1/audio/transcriptions` (SSE) |
| TTS | Speaches | `POST /v1/audio/speech` (streaming PCM) |
| LLM | Any OpenAI-compatible | `POST /v1/chat/completions` (SSE) |

---

## Part 2 — Production Platform

### Vision

Transform the single-session voicebot engine into a **scalable SaaS telephony platform**:

- Multiple **customer accounts** (tenants) self-serve through a dashboard
- Each tenant manages **campaigns** (inbound, outbound, or hybrid)
- Each campaign has its own system prompt, instructions, tools, voices, and metrics
- Voicebot core is **stateless** — durable state in PostgreSQL, hot state in Redis
- Horizontal scaling via N voicebot-core instances behind a load balancer
- **Next.js dashboard** for campaign lifecycle control and real-time analytics
- **Microservice architecture** — Management API, Voicebot Core, and Campaign Scheduler are separate deployable services

---

### Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Next.js Dashboard (Thin BFF)                     │
│  Next.js 15 App Router · TypeScript · Tailwind v4 · shadcn/ui      │
│  TanStack Query (data) · SSE (live metrics) · httpOnly cookie auth  │
└────────────────────────────┬────────────────────────────────────────┘
                             │ HTTPS / WSS
┌────────────────────────────▼────────────────────────────────────────┐
│                      API Gateway / Load Balancer                    │
│                     (nginx / traefik / cloud LB)                    │
└───────┬──────────────────────────────────┬──────────────────────────┘
        │                                  │
┌───────▼──────────────┐        ┌──────────▼─────────────────────────┐
│  Management API       │        │   Voicebot Core (N instances)      │
│  (Rust / Axum)        │        │   Stateless real-time pipeline     │
│  SEPARATE BINARY      │        │   SEPARATE BINARY                  │
│                       │        │                                    │
│  • Auth (JWT)         │        │   • WS transport (token auth)      │
│  • Tenant CRUD        │        │   • Asterisk ARI transport         │
│  • Campaign CRUD      │        │   • VAD → ASR → Agent → TTS       │
│  • Analytics / CDR    │        │   • Session state → Redis          │
│  • SSE live metrics   │        │   • CDR write → PostgreSQL         │
│  • Usage tracking     │        │   • Campaign config from Redis     │
│  • OpenAPI (utoipa)   │        │   • Recording upload → RustFS/S3   │
└───────┬──────────────┘        └──────────┬─────────────────────────┘
        │                                  │
┌───────▼──────────────────────────────────▼──────────────────────────┐
│   PostgreSQL                    │   Redis                           │
│                                 │                                   │
│   • tenants, users              │   • session state (TTL)           │
│   • campaigns, prompt_versions  │   • campaign config cache         │
│   • call_records (CDR)          │   • active session sets           │
│   • contacts, contact_lists     │   • rate limiters                 │
│   • usage_records               │   • pub/sub config reload         │
│   • phone_numbers               │                                   │
└─────────────────────────────────┴───────────────────────────────────┘
        │                                  │
┌───────▼──────────────┐        ┌──────────▼──────────────────────────┐
│  Campaign Scheduler   │        │   Object Storage (RustFS / MinIO)  │
│  (Rust / apalis)      │        │   S3-compatible API                │
│  SEPARATE BINARY      │        │                                    │
│                       │        │   • Call recordings (configurable)  │
│  • Outbound dialer    │        │   • Contact list imports           │
│  • Post-call analysis │        │   • Export artifacts               │
│  • Retry / backoff    │        │                                    │
│  • Schedule windows   │        │   Config: on/off per tenant/camp.  │
│  • Rate limiting      │        │   Dev: RustFS · Prod: MinIO/S3     │
└───────────────────────┘        └────────────────────────────────────┘
```

### Design Decisions

| # | Decision | Rationale |
|---|---|---|
| 1 | **Microservices** — Management API, Voicebot Core, and Scheduler are separate binaries | Independent scaling, independent deploy cycles; API handles CRUD while core handles real-time audio |
| 2 | **Stateless core** — conversation memory in Redis, CDR flushed to PG | Any core instance handles any call; no session affinity needed |
| 3 | **Redis hot path, PG durable** — campaign config cached in Redis on activate/update | Sub-ms config reads on session start; config changes propagate via pub/sub |
| 4 | **Phone number provisioning** — Phase 1: manual Asterisk config; Phase 2: SIP trunk provider API (Twilio/Telnyx) | Get working fast, then automate |
| 5 | **Call recording** — configurable per tenant/campaign; S3-compatible object storage (RustFS for dev, MinIO/S3 for prod) | Not all campaigns need recordings; `aws-sdk-s3` client works with any S3-compatible store |
| 6 | **Usage tracking** — track call minutes, concurrent sessions, recording storage per tenant | Foundation for future billing; needed for plan limits enforcement |
| 7 | **Post-call analysis** — async job via scheduler; LLM analyzes transcript for sentiment, summarization, custom metric extraction | Powerful for custom metrics that are hard to capture in real-time |
| 8 | **WS auth** — `wss://host/ws?campaign_id=X&token=Y`; short-lived token validated on connect, campaign resolved from token claims | Simple, stateless; no session cookie needed for WS voice sessions |
| 9 | **Next.js thin BFF** — server components for SSR, API routes proxy to Rust backend, httpOnly cookie auth | Hides Rust API from browser, secure cookie handling, fast initial paint |

---

### Data Model

#### Tenants

```sql
CREATE TABLE tenants (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    slug            TEXT NOT NULL UNIQUE,
    status          TEXT NOT NULL DEFAULT 'active',  -- active | suspended | trial
    plan            TEXT NOT NULL DEFAULT 'basic',   -- basic | pro | enterprise
    max_concurrent  INT NOT NULL DEFAULT 10,
    settings        JSONB DEFAULT '{}'::jsonb,       -- provider defaults, recording prefs
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

#### Users

```sql
CREATE TABLE users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id),
    email           TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    role            TEXT NOT NULL DEFAULT 'viewer',   -- owner | admin | editor | viewer
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

#### Campaigns

```sql
CREATE TABLE campaigns (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id),
    name            TEXT NOT NULL,
    description     TEXT,
    status          TEXT NOT NULL DEFAULT 'draft',   -- draft | active | paused | completed | archived
    direction       TEXT NOT NULL DEFAULT 'inbound', -- inbound | outbound | both

    -- Voice pipeline config
    system_prompt   TEXT NOT NULL,
    instructions    TEXT,
    language        TEXT NOT NULL DEFAULT 'auto',
    asr_model       TEXT,
    tts_model       TEXT,
    tts_voice       TEXT,
    llm_model       TEXT,
    llm_temperature REAL DEFAULT 0.7,
    llm_max_tokens  INT DEFAULT 512,

    -- Provider overrides (null = use tenant defaults)
    asr_base_url    TEXT,
    tts_base_url    TEXT,
    llm_base_url    TEXT,

    -- Tools / function calling
    tools_config    JSONB DEFAULT '[]'::jsonb,

    -- Outbound dialer config
    caller_id       TEXT,
    dial_list_id    UUID,
    max_concurrent  INT DEFAULT 5,
    call_rate_per_minute INT DEFAULT 10,
    retry_attempts  INT DEFAULT 2,
    retry_delay_min INT DEFAULT 30,
    schedule_start  TIME,
    schedule_end    TIME,
    schedule_tz     TEXT DEFAULT 'UTC',

    -- Recording config
    recording_enabled BOOLEAN DEFAULT false,

    -- Custom metrics definition
    custom_metrics  JSONB DEFAULT '[]'::jsonb,

    -- Versioning
    version         INT NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_campaigns_tenant ON campaigns(tenant_id);
CREATE INDEX idx_campaigns_status ON campaigns(tenant_id, status);
```

#### Prompt Versions

```sql
CREATE TABLE prompt_versions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id     UUID NOT NULL REFERENCES campaigns(id),
    version         INT NOT NULL,
    system_prompt   TEXT NOT NULL,
    instructions    TEXT,
    tools_config    JSONB DEFAULT '[]'::jsonb,
    created_by      UUID REFERENCES users(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(campaign_id, version)
);
```

#### Contact Lists

```sql
CREATE TABLE contact_lists (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id),
    campaign_id     UUID REFERENCES campaigns(id),
    name            TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE contacts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    list_id         UUID NOT NULL REFERENCES contact_lists(id),
    phone_number    TEXT NOT NULL,
    name            TEXT,
    metadata        JSONB DEFAULT '{}'::jsonb,
    status          TEXT NOT NULL DEFAULT 'pending', -- pending | called | completed | failed | dnc
    last_attempt_at TIMESTAMPTZ,
    attempt_count   INT DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_contacts_list_status ON contacts(list_id, status);
```

#### Call Records (CDR)

```sql
CREATE TABLE call_records (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id),
    campaign_id     UUID NOT NULL REFERENCES campaigns(id),
    session_id      UUID NOT NULL UNIQUE,
    contact_id      UUID REFERENCES contacts(id),

    direction       TEXT NOT NULL,                    -- inbound | outbound
    caller_number   TEXT,
    callee_number   TEXT,
    status          TEXT NOT NULL,                    -- completed | failed | no_answer | busy | error

    started_at      TIMESTAMPTZ NOT NULL,
    answered_at     TIMESTAMPTZ,
    ended_at        TIMESTAMPTZ,
    duration_secs   REAL,

    -- Transcript
    transcript      JSONB,                           -- [{role, text, timestamp_ms}]

    -- Technical metrics
    first_response_ms   INT,
    turn_count          INT,
    interrupt_count     INT,
    asr_latency_avg_ms  INT,
    llm_latency_avg_ms  INT,
    tts_latency_avg_ms  INT,

    -- Custom metrics (campaign-defined)
    custom_metrics  JSONB DEFAULT '{}'::jsonb,

    -- Recording
    recording_path  TEXT,                            -- S3 key if recording enabled

    -- Disposition
    disposition     TEXT,
    disposition_notes TEXT,

    -- Post-call analysis
    analysis        JSONB,                           -- {summary, sentiment, extracted_metrics}

    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_cdr_tenant ON call_records(tenant_id);
CREATE INDEX idx_cdr_campaign ON call_records(campaign_id);
CREATE INDEX idx_cdr_session ON call_records(session_id);
CREATE INDEX idx_cdr_time ON call_records(tenant_id, started_at);
```

#### Phone Numbers

```sql
CREATE TABLE phone_numbers (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id),
    number          TEXT NOT NULL UNIQUE,
    campaign_id     UUID REFERENCES campaigns(id),  -- null = unassigned
    provider        TEXT NOT NULL DEFAULT 'manual',  -- manual | twilio | telnyx
    provider_sid    TEXT,                            -- external provider ID
    direction       TEXT NOT NULL DEFAULT 'both',    -- inbound | outbound | both
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_phone_numbers_tenant ON phone_numbers(tenant_id);
CREATE INDEX idx_phone_numbers_number ON phone_numbers(number);
```

#### Usage Tracking

```sql
CREATE TABLE usage_records (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id),
    campaign_id     UUID REFERENCES campaigns(id),
    record_type     TEXT NOT NULL,                   -- call_minutes | concurrent_peak | recording_bytes | post_call_analysis
    quantity        REAL NOT NULL,
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_usage_tenant_time ON usage_records(tenant_id, recorded_at);
CREATE INDEX idx_usage_type ON usage_records(tenant_id, record_type, recorded_at);
```

#### Custom Metrics Definition (JSONB in campaigns.custom_metrics)

```json
[
    {
        "key": "callee_interested",
        "type": "boolean",
        "label": "Callee Interested",
        "description": "Whether the callee expressed interest",
        "collection": "agent_tool",
        "tool_name": "record_interest"
    },
    {
        "key": "campaign_outcome",
        "type": "enum",
        "label": "Campaign Outcome",
        "options": ["sale", "callback", "not_interested", "wrong_number"],
        "collection": "agent_tool",
        "tool_name": "set_outcome"
    },
    {
        "key": "sentiment_score",
        "type": "number",
        "label": "Sentiment Score",
        "description": "Overall sentiment 1-5",
        "collection": "post_call_analysis"
    }
]
```

**Metric types:** boolean, number, enum, text

**Collection methods:**
- `agent_tool` — real-time via LLM tool calling during the call (auto-generated tool defs)
- `post_call_analysis` — async LLM analysis of transcript after call ends
- `manual` — supervisor fills in via dashboard

---

### Making Voicebot Core Stateless

#### Current State → Target State

| State | Current | Target | Storage |
|---|---|---|---|
| Conversation memory | In-memory Vec | Redis hash | `session:{id}:memory`, TTL = session + 1h |
| Orchestrator FSM | In-memory | In-memory (per-connection) | Ephemeral, tied to TCP/WS connection |
| Campaign config | config.toml | Redis cache | `campaign:{id}:config`, updated on activate |
| Active sessions | Not tracked | Redis set | `tenant:{id}:sessions`, for concurrent limits |
| CDR accumulator | Not tracked | In-memory → flush to PG | Batch write on session end |
| Call recording | Not tracked | Stream to S3 during call | Configurable on/off |

#### Stateless Session Lifecycle

```
1. Call arrives (WS with ?campaign_id=X&token=Y, or Asterisk with phone number mapping)
2. Core validates token / resolves campaign from phone number
3. Core fetches campaign config from Redis (cache hit) or PG (miss → populate)
4. Core checks tenant concurrent limit: SCARD + SADD atomic on tenant:{id}:sessions
5. Core creates PipelineSession with campaign config (system_prompt, tools, etc.)
6. During call: conversation memory synced to Redis every turn
7. If recording_enabled: audio streamed to S3 via multipart upload
8. On call end:
   a. Flush CDR to PostgreSQL
   b. Remove session from Redis active set
   c. Record usage (call_minutes, recording_bytes)
   d. If post-call analysis configured: enqueue analysis job
   e. Conversation memory TTL expires naturally
```

#### Backward Compatibility

Single-instance dev mode still works: if no Redis/PG configured, fall back to in-memory state + config.toml. Feature-flagged at startup.

---

### Component Breakdown

#### New Rust Crates

| Crate | Purpose | Key Dependencies |
|---|---|---|
| `crates/db` | PostgreSQL — sqlx queries, migrations, connection pool | `sqlx`, `uuid`, `chrono` |
| `crates/cache` | Redis — session state, config cache, pub/sub | `redis` |
| `crates/auth` | JWT middleware, password hashing, tenant extraction | `jsonwebtoken`, `argon2` |
| `crates/storage` | S3-compatible object storage abstraction | `aws-sdk-s3` |
| `crates/api` | Management REST API (Axum) | `axum`, `utoipa`, all above |
| `crates/scheduler` | Campaign scheduler — dialer, post-call, retry | `apalis`, `db`, `cache` |

#### New Frontend Package

| Package | Purpose |
|---|---|
| `dashboard/` | Next.js 15 thin BFF — TypeScript, Tailwind v4, shadcn/ui |

#### Modified Existing Crates

| Crate | Changes |
|---|---|
| `crates/common` | Add `TenantId`, `CampaignId` types; `CampaignConfig` struct |
| `crates/core` | Redis-backed `ConversationMemory`; campaign config at session start; CDR emit; recording stream |
| `crates/server` | Split into `voicebot-core` and `voicebot-api` binaries |
| `crates/agent` | Auto-generate tools from campaign custom metric definitions |

#### Dependency Graph

```
common → (no deps)
db → common
cache → common
storage → common
auth → common, db
vad → common
asr → common
agent → common
tts → common
core → common, vad, asr, agent, tts, cache, storage
api → common, db, cache, auth, storage
scheduler → common, db, cache, storage
```

---

### Management API Endpoints

#### Auth
| Method | Path | Description |
|---|---|---|
| POST | `/api/auth/register` | Create tenant + owner user |
| POST | `/api/auth/login` | Email + password → JWT (also sets httpOnly cookie) |
| POST | `/api/auth/refresh` | Refresh token |
| POST | `/api/auth/logout` | Clear cookie |

#### Tenants
| Method | Path | Description |
|---|---|---|
| GET | `/api/tenants/me` | Current tenant profile |
| PUT | `/api/tenants/me` | Update tenant settings (provider defaults, recording prefs) |
| GET | `/api/tenants/me/usage` | Usage summary (call minutes, concurrent peak, storage) |

#### Users
| Method | Path | Description |
|---|---|---|
| GET | `/api/users` | List tenant users |
| POST | `/api/users` | Invite user |
| PUT | `/api/users/:id` | Update role |
| DELETE | `/api/users/:id` | Remove user |

#### Campaigns
| Method | Path | Description |
|---|---|---|
| GET | `/api/campaigns` | List campaigns (filterable) |
| POST | `/api/campaigns` | Create campaign |
| GET | `/api/campaigns/:id` | Get campaign details |
| PUT | `/api/campaigns/:id` | Update campaign |
| DELETE | `/api/campaigns/:id` | Archive campaign |
| POST | `/api/campaigns/:id/activate` | Activate → publish config to Redis |
| POST | `/api/campaigns/:id/pause` | Pause campaign |
| POST | `/api/campaigns/:id/duplicate` | Clone campaign |
| GET | `/api/campaigns/:id/versions` | Prompt version history |

#### Contacts / Dial Lists
| Method | Path | Description |
|---|---|---|
| POST | `/api/campaigns/:id/contacts` | Upload contact list (CSV) |
| GET | `/api/campaigns/:id/contacts` | List contacts with status |
| PUT | `/api/campaigns/:id/contacts/:cid` | Update contact status |
| DELETE | `/api/campaigns/:id/contacts/:cid` | Remove contact |

#### Call Records / Analytics
| Method | Path | Description |
|---|---|---|
| GET | `/api/campaigns/:id/calls` | Paginated call records |
| GET | `/api/campaigns/:id/calls/:cid` | Call detail + transcript + recording URL |
| GET | `/api/campaigns/:id/analytics` | Aggregated campaign analytics |
| GET | `/api/campaigns/:id/analytics/stream` | SSE live metrics |

#### Phone Numbers
| Method | Path | Description |
|---|---|---|
| GET | `/api/phone-numbers` | List tenant's phone numbers |
| POST | `/api/phone-numbers` | Add number (manual or provision via provider) |
| PUT | `/api/phone-numbers/:id` | Map number → campaign |
| DELETE | `/api/phone-numbers/:id` | Remove number |

#### Voice Session Tokens
| Method | Path | Description |
|---|---|---|
| POST | `/api/campaigns/:id/session-token` | Generate short-lived WS auth token |

---

### Dashboard Pages

#### Overview / Home
- Active campaigns summary cards
- Today's call volume chart
- Active calls count (real-time via SSE)
- Quick stats: total calls, avg duration, success rate
- Usage meter (call minutes vs plan limit)

#### Campaigns List
- Table: name, status, direction, call count, success rate, actions
- Filters: status, direction, date range
- Create campaign button

#### Campaign Detail
- **Config tab** — system prompt editor with version history, instructions, voice/model selection, tool configuration
- **Metrics tab** — define custom metrics (boolean, number, enum, text), link to collection method
- **Contacts tab** (outbound) — CSV upload, contact list with status, progress bar
- **Schedule tab** (outbound) — calling windows, timezone, rate limits, retry policy
- **Analytics tab** — call volume over time, avg duration, disposition breakdown, custom metric aggregations, technical metrics P50/P90/P99
- **Calls tab** — paginated call log with search, expand for transcript + metrics + recording playback

#### Live Monitor
- Active calls table (real-time SSE)
- Per-call: session ID, campaign, duration, current state
- Future: live audio monitoring button

#### Settings
- Tenant profile
- User management (invite, roles)
- Provider configuration (ASR/TTS/LLM endpoints, API keys)
- Phone number management
- Recording storage preferences
- Usage / billing summary

---

### Technology Stack

#### Backend (Rust)

| Component | Crate | Version |
|---|---|---|
| HTTP framework | `axum` | 0.8+ |
| PostgreSQL | `sqlx` | 0.8+ |
| Redis | `redis` | 1.2+ |
| JWT auth | `jsonwebtoken` | 10+ |
| Password hashing | `argon2` | 0.5+ |
| Object storage | `aws-sdk-s3` | latest |
| Job queue | `apalis` + `apalis-sql` | 0.7+ |
| OpenAPI | `utoipa` + `utoipa-axum` | 5+ |
| API testing | `axum-test` | 20+ |
| Serialization | `serde` + `serde_json` | 1.x |
| Async runtime | `tokio` | 1.x |

#### Frontend (Next.js)

| Component | Tool | Version |
|---|---|---|
| Framework | Next.js (App Router, thin BFF) | 15+ |
| Language | TypeScript | 5.7+ |
| Styling | Tailwind CSS | 4+ |
| Components | shadcn/ui (Radix primitives) | latest |
| Data fetching | TanStack Query | 5+ |
| Forms | React Hook Form + Zod | latest |
| Tables | TanStack Table | 8+ |
| Charts | Recharts | 2+ |
| JWT validation | jose | 6+ |
| Icons | lucide-react | latest |
| Package manager | pnpm | latest |

#### Infrastructure

| Component | Tool |
|---|---|
| Database | PostgreSQL 16+ |
| Cache | Redis 7+ |
| Object storage | RustFS (dev) / MinIO or S3 (prod) |
| Telephony | Asterisk 20+ (ARI) |
| ASR/TTS | Speaches (or any OpenAI-compatible) |
| LLM | Any OpenAI-compatible (vLLM, Ollama, etc.) |
| Reverse proxy | nginx or traefik |
| Containers | Docker Compose (dev) → Kubernetes (prod) |
| Monitoring | Prometheus + Grafana (existing) |

---

### Implementation Phases

#### Phase A — Data Layer & Auth (Foundation)

| # | Task | Status |
|---|---|---|
| A1 | Create `crates/db` — sqlx setup, PgPool, DATABASE_URL config | ✅ Done |
| A2 | Write migrations: tenants, users, campaigns, prompt_versions, contacts, contact_lists, call_records, phone_numbers, usage_records | ✅ Done |
| A3 | DB query functions — CRUD for all entities, pagination helpers | ✅ Done |
| A4 | Create `crates/cache` — Redis MultiplexedConnection, session state read/write, config cache get/set, pub/sub | ✅ Done |
| A5 | Create `crates/auth` — JWT issue/validate, argon2 password hashing, Axum middleware (extract tenant_id + user_id from token) | ✅ Done |
| A6 | Create `crates/storage` — S3 client abstraction, upload/download/presigned-url, bucket init | ✅ Done |
| A7 | PostgreSQL RLS policies as defense-in-depth (SET LOCAL app.tenant_id per transaction) | ✅ Done |
| A8 | Integration tests for DB + cache + storage layers | 🟡 Partial — ignored db/cache/storage roundtrip tests added; broader scenario coverage still missing |

#### Phase B — Management API

| # | Task | Status |
|---|---|---|
| B1 | Create `crates/api` — Axum router on `/api`, shared AppState (PgPool, Redis, S3) | ✅ Done |
| B2 | Auth endpoints: register, login, refresh, logout | ✅ Done |
| B3 | Tenant endpoints: profile, settings, usage | ✅ Done |
| B4 | User management: list, invite, update role, remove | ✅ Done |
| B5 | Campaign CRUD + activate/pause lifecycle (publish config to Redis on activate) | ✅ Done |
| B6 | Contact list upload (CSV parsing) + CRUD | ✅ Done |
| B7 | Call records: paginated queries, single call detail, analytics aggregation | ✅ Done |
| B8 | Phone number management: add, map to campaign, remove | ✅ Done |
| B9 | Session token generation (short-lived JWT for WS auth) | ✅ Done |
| B10 | SSE endpoint for live campaign metrics | ✅ Done (`/metrics/live`, `/sessions/live`) |
| B11 | Usage tracking: record call minutes, concurrent peak, storage bytes | ✅ Done |
| B12 | OpenAPI spec generation with utoipa | ❌ Not done |
| B13 | API integration tests (axum-test) | 🟡 Partial — tenant isolation integration test added; broader route coverage still missing |

#### Phase C — Stateless Core

| # | Task | Status |
|---|---|---|
| C1 | Campaign config resolution: Redis cache → PG fallback on session start | ✅ Done |
| C2 | Redis-backed ConversationMemory (replace in-memory Vec, with fallback) | ✅ Done |
| C3 | Concurrent session limit enforcement via Redis SCARD/SADD | ✅ Done |
| C4 | CDR accumulation during call + async flush to PG on session end | ✅ Done |
| C5 | Auto-generate agent tools from campaign custom_metrics definitions | ✅ Done |
| C6 | Campaign config hot-reload via Redis pub/sub | ✅ Done |
| C7 | Phone number → campaign routing table (Redis lookup) | ✅ Done |
| C8 | WS auth: validate token from query param, extract campaign_id + tenant_id | ✅ Done |
| C9 | Call recording: configurable audio stream to S3 via multipart upload | ✅ Done (WAV buffer → S3 on session end) |
| C10 | Usage metering: emit call_minutes + recording_bytes on session end | ✅ Done |

#### Phase D — Campaign Scheduler

| # | Task | Status |
|---|---|---|
| D1 | Create `crates/scheduler` binary — apalis with PG backend | ✅ Done |
| D2 | Outbound dialer job: pick pending contacts, originate calls via ARI REST | ✅ Done |
| D3 | Retry logic: exponential backoff, max attempts, update contact status | ✅ Done |
| D4 | Schedule enforcement: time windows, timezone-aware (chrono-tz) | ✅ Done |
| D5 | Rate limiting: calls per minute per campaign | ✅ Done |
| D6 | Campaign completion detection: all contacts processed → status = completed | ✅ Done |
| D7 | Post-call analysis job: LLM analysis of transcript → extract custom metrics, sentiment, summary | ✅ Done |
| D8 | Post-call analysis results written to call_records.analysis JSONB | ✅ Done |

#### Phase E — Dashboard Frontend

| # | Task | Status |
|---|---|---|
| E1 | Scaffold Next.js 15 + TS + Tailwind v4 + shadcn/ui + TanStack Query in `dashboard/` | ✅ Done |
| E2 | Auth flow: login page, httpOnly cookie JWT, middleware route protection | ✅ Done |
| E3 | API proxy routes: catch-all `/api/proxy/[...path]` → Rust API | ✅ Done |
| E4 | SSE proxy route: streaming passthrough for live metrics | ✅ Done |
| E5 | Overview page: campaign cards, call volume chart, active calls count (SSE) | ✅ Done |
| E6 | Campaign list page: table with filters, create button | ✅ Done |
| E7 | Campaign detail — config tab: system prompt editor, model/voice selection | ✅ Done |
| E8 | Campaign detail — metrics tab: custom metrics builder | ✅ Done |
| E9 | Campaign detail — contacts tab: CSV upload, contact table | ✅ Done |
| E10 | Campaign detail — analytics tab: charts (recharts), metric aggregations | ✅ Done |
| E11 | Campaign detail — calls tab: paginated log, transcript viewer, recording playback | ✅ Done |
| E12 | Live monitor page: active calls table with SSE | ✅ Done |
| E13 | Settings pages: users, providers, phone numbers, recording prefs, usage | ✅ Done |

#### Phase F — Integration & Hardening

| # | Task | Status |
|---|---|---|
| F1 | E2E: create campaign → activate → inbound call → CDR written → appears in dashboard | ❌ Not done |
| F2 | E2E: outbound campaign → scheduler dials → call completes → post-call analysis runs | ❌ Not done |
| F3 | Multi-tenant isolation tests: verify tenant A cannot see tenant B's data (app-level + RLS) | 🟡 Partial — campaign access covered at API layer and raw SQL RLS layer; more entity coverage still missing |
| F4 | Recording E2E: call with recording_enabled → audio in S3 → playback from dashboard | ❌ Not done |
| F5 | Load test: 50 concurrent sessions across 5 tenants | ❌ Not done |
| F6 | Docker Compose: full stack (PG + Redis + RustFS + Speaches + Asterisk + API + Core + Scheduler + Dashboard) | 🟡 Partial — compose files exist, but Asterisk/live telephony path needs a working validation pass |
| F7 | Health check endpoints + readiness probes for all services | ✅ Done |
| F8 | Usage tracking accuracy test: verify call minutes match actual call durations | ❌ Not done |

---

### Phase Dependencies

```
Phase A (Data Layer)  ──→  Phase B (Management API)  ──→  Phase C (Stateless Core)
                                                                  │
                                                                  ├──→  Phase D (Scheduler)
                                                                  │
                                                                  └──→  Phase E (Dashboard)
                                                                               │
                                                                               ▼
                                                                       Phase F (Integration)
```

- **A → B**: API needs DB, cache, auth, storage layers
- **B → C**: Core needs campaign config API to exist (for Redis population)
- **C → D**: Scheduler needs stateless core + CDR for post-call analysis
- **C → E**: Dashboard needs both API and live core for SSE
- **D ∥ E**: Scheduler and Dashboard can proceed in parallel
- **F**: Integration testing after D and E are both functional

---

### Agent Skills

#### Existing (in repo)

| Skill | Domain |
|---|---|
| `rust` | Rust coding guidelines (179 rules) |
| `rust_async` | Tokio patterns, channels, cancellation |
| `audio_dsp` | Audio formats, codec conversion, VAD framing |
| `robuto` | Audio resampling (rubato) |
| `provider_integration` | OpenAI-compatible API patterns |
| `speaches` | Speaches server: ASR, TTS, VAD, Realtime WS |
| `orchestrator_and_pipeline_session` | State machine, session lifecycle, interrupts |
| `websocket_transport` | WS server, binary/JSON framing |
| `asterisk_ari` | ARI Stasis, AudioSocket, REST channel control |
| `agent_tool_calling` | Tool loop, conversation memory, sentence-boundary TTS |
| `configuration` | config.toml parsing, env var substitution |
| `observability` | tracing spans, Prometheus metrics |
| `testing_convention` | Test structure, TestAudioStream, fixtures |
| `error_handling_and_fault_tolerance` | Error types, retry matrix, fallback providers |
| `xphone` | Native SIP virtual phone for loadtest |

#### New Skills to Create

| Skill | Domain | Trigger Words |
|---|---|---|
| `database` | sqlx patterns, migrations, compile-time queries, transactions, connection pool, PG RLS | sqlx, migration, database, postgresql, query, transaction, pool |
| `redis_cache` | Redis async patterns, session state, config cache, pub/sub, TTL, rate limiting | redis, cache, session state, pub/sub, TTL |
| `rest_api` | Axum REST patterns, middleware stack, extractors, error responses, pagination, OpenAPI/utoipa | api, endpoint, REST, middleware, utoipa, pagination |
| `auth_multi_tenant` | JWT auth, multi-tenant isolation, RBAC, password hashing, tenant context, RLS | auth, jwt, tenant, multi-tenant, rbac, login |
| `nextjs_dashboard` | Next.js 15 App Router, thin BFF pattern, server/client components, TanStack Query, shadcn/ui, SSE, httpOnly cookie auth | next, dashboard, frontend, react, shadcn, tanstack |
| `campaign_management` | Campaign lifecycle, custom metrics, contact lists, outbound dialing, scheduling, post-call analysis | campaign, outbound, inbound, dialer, schedule, contact, post-call |
| `object_storage` | S3-compatible storage, aws-sdk-s3, multipart upload, presigned URLs, RustFS/MinIO | storage, s3, recording, upload, minio, rustfs |
