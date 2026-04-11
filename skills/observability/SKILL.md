---
name: Observability
---

# Skill: Observability

Use this whenever adding metrics, structured logging, tracing spans, or Prometheus instrumentation to any crate.

## Structured logging with `tracing`

Use the `tracing` crate exclusively. Every span MUST include `session_id`.

### Subscriber setup (in main.rs)

```rust
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing() {
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env()        // RUST_LOG=info,voicebot=debug
            .add_directive("voicebot=debug".parse().unwrap()))
        .with(fmt::layer()
            .json()                                 // structured JSON output
            .with_target(true)
            .with_thread_ids(true)
            .with_span_events(fmt::format::FmtSpan::CLOSE))
        .init();
}
```

### Log levels — strict conventions

| Level | When to use | Example |
| --- | --- | --- |
| `ERROR` | Unrecoverable failures that end a session | LLM retries exhausted, session terminating |
| `WARN` | Retries, dropped frames, channel overflow | Audio channel full, ASR retry attempt 2 |
| `INFO` | Lifecycle events (session start/end, state transitions) | Session started, TTS complete |
| `DEBUG` | Per-frame events, partial transcripts | VAD frame energy=0.03, partial transcript |

### Span patterns

```rust
// Session-level span — wraps entire session lifetime
let _session_span = tracing::info_span!("session",
    session_id = %session_id,
    language = %config.language,
).entered();

// Component-level span — nested inside session
let _vad_span = tracing::info_span!("vad", session_id = %session_id).entered();

// Event logging within a span
tracing::info!(session_id = %self.session_id, event = "started", language = %config.language);
tracing::debug!(session_id = %self.session_id, event = "speech_started");
tracing::warn!(session_id = %self.session_id, event = "channel_full", channel = "audio_ingress");
tracing::error!(session_id = %self.session_id, event = "session_terminated", reason = %error);
```

### Async span propagation

```rust
use tracing::Instrument;

// Attach a span to a spawned task
let span = tracing::info_span!("asr", session_id = %session_id);
tokio::spawn(async move {
    asr_provider.stream(audio_rx, event_tx).await;
}.instrument(span));
```

## Metrics with `metrics` crate (Prometheus-compatible)

### Setup (in main.rs)

```rust
use metrics_exporter_prometheus::PrometheusBuilder;

pub fn init_metrics() {
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9090))  // GET /metrics
        .install()
        .expect("failed to install Prometheus metrics exporter");
}
```

### Required metrics — exact names and types

```rust
// Gauges
metrics::gauge!("voicebot_sessions_active").set(active_count as f64);

// Histograms (latency)
metrics::histogram!("voicebot_session_duration_ms").record(duration_ms as f64);
metrics::histogram!("voicebot_vad_latency_ms", "session_id" => session_id.to_string())
    .record(latency_ms as f64);
metrics::histogram!("voicebot_asr_latency_ms", "provider" => provider_name)
    .record(latency_ms as f64);
metrics::histogram!("voicebot_llm_first_token_ms", "provider" => provider_name)
    .record(first_token_ms as f64);
metrics::histogram!("voicebot_llm_total_ms")
    .record(total_ms as f64);
metrics::histogram!("voicebot_tts_first_chunk_ms", "provider" => provider_name)
    .record(first_chunk_ms as f64);

// Counters
metrics::counter!("voicebot_interrupts_total").increment(1);
metrics::counter!("voicebot_errors_total",
    "component" => component_name,
    "recoverable" => recoverable.to_string(),
).increment(1);
```

### Metric reference table

| Metric name                    | Type      | Labels                     | Emitted by      |
| ------------------------------ | --------- | -------------------------- | --------------- |
| `voicebot_sessions_active`     | gauge     | —                          | session manager |
| `voicebot_session_duration_ms` | histogram | —                          | session cleanup |
| `voicebot_vad_latency_ms`      | histogram | `session_id`               | VAD component   |
| `voicebot_asr_latency_ms`      | histogram | `provider`                 | ASR component   |
| `voicebot_llm_first_token_ms`  | histogram | `provider`                 | agent           |
| `voicebot_llm_total_ms`        | histogram | —                          | agent           |
| `voicebot_tts_first_chunk_ms`  | histogram | `provider`                 | TTS component   |
| `voicebot_interrupts_total`    | counter   | —                          | orchestrator    |
| `voicebot_errors_total`        | counter   | `component`, `recoverable` | orchestrator    |

### Instrumentation patterns

```rust
// Measure latency with Instant
let start = std::time::Instant::now();
let result = asr_provider.stream(audio_rx, event_tx).await;
let latency = start.elapsed().as_millis() as f64;
metrics::histogram!("voicebot_asr_latency_ms", "provider" => "deepgram")
    .record(latency);

// Track active sessions with increment/decrement
metrics::gauge!("voicebot_sessions_active").increment(1.0);
// ... on session end:
metrics::gauge!("voicebot_sessions_active").decrement(1.0);

// LLM first-token latency
let llm_start = Instant::now();
let mut first_token_recorded = false;
while let Some(event) = response_rx.recv().await {
    if !first_token_recorded {
        metrics::histogram!("voicebot_llm_first_token_ms", "provider" => "openai")
            .record(llm_start.elapsed().as_millis() as f64);
        first_token_recorded = true;
    }
    // ... handle event
}
```

### Error counting

```rust
// In orchestrator error handler
fn record_component_error(component: Component, recoverable: bool) {
    metrics::counter!(
        "voicebot_errors_total",
        "component" => format!("{:?}", component),
        "recoverable" => recoverable.to_string(),
    ).increment(1);
}
```

## What NOT to do

```rust
// Never use println! in library code
println!("session started"); // ← forbidden; use tracing::info!

// Never omit session_id from spans
tracing::info!("session started"); // ← missing session_id

// Never log at wrong level
tracing::info!(event = "audio_frame", energy = 0.03); // ← should be DEBUG (per-frame)
tracing::debug!(event = "session_started"); // ← should be INFO (lifecycle)

// Never use high-cardinality labels on counters (e.g., session_id on a counter)
metrics::counter!("voicebot_errors_total", "session_id" => id.to_string()); // ← cardinality explosion

// Never log secrets
tracing::info!(api_key = %key); // ← forbidden

// Never skip instrumenting spawned tasks
tokio::spawn(async move { ... }); // ← missing .instrument(span)
```
