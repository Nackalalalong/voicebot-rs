use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;

/// Initialize tracing.
/// - If `LOG_FORMAT=json` or stdout is not a TTY: structured JSON (for log aggregators).
/// - Otherwise: colorized human-readable output for the terminal.
pub fn init_tracing() {
    use std::io::IsTerminal;
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,voicebot_core=debug,asr=debug,tts=debug,agent=debug,transport_websocket=debug",
        )
    });

    let use_json =
        std::env::var("LOG_FORMAT").as_deref() == Ok("json") || !std::io::stdout().is_terminal();

    if use_json {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .json()
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_span_events(fmt::format::FmtSpan::CLOSE),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .pretty()
                    .with_ansi(true)
                    .with_target(true)
                    .with_span_events(fmt::format::FmtSpan::CLOSE),
            )
            .init();
    }
}

/// Install the Prometheus metrics exporter, listening on the given address.
/// Returns the socket address the metrics server is bound to.
pub fn init_metrics(addr: &str) -> Result<std::net::SocketAddr, String> {
    let addr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| format!("bad metrics addr: {}", e))?;
    let builder = PrometheusBuilder::new().with_http_listener(addr);
    builder
        .install()
        .map_err(|e| format!("failed to install prometheus exporter: {}", e))?;
    Ok(addr)
}

// --- Metric recording functions ---

/// Increment active sessions gauge.
pub fn session_started() {
    gauge!("voicebot_sessions_active").increment(1.0);
}

/// Decrement active sessions gauge and record session duration.
pub fn session_ended(duration_ms: f64) {
    gauge!("voicebot_sessions_active").decrement(1.0);
    histogram!("voicebot_session_duration_ms").record(duration_ms);
}

/// Record VAD processing latency.
pub fn record_vad_latency(session_id: &str, latency_ms: f64) {
    histogram!("voicebot_vad_latency_ms", "session_id" => session_id.to_string())
        .record(latency_ms);
}

/// Record ASR transcription latency.
pub fn record_asr_latency(provider: &str, latency_ms: f64) {
    histogram!("voicebot_asr_latency_ms", "provider" => provider.to_string()).record(latency_ms);
}

/// Record LLM time-to-first-token.
pub fn record_llm_first_token(provider: &str, latency_ms: f64) {
    histogram!("voicebot_llm_first_token_ms", "provider" => provider.to_string())
        .record(latency_ms);
}

/// Record total LLM completion time.
pub fn record_llm_total(latency_ms: f64) {
    histogram!("voicebot_llm_total_ms").record(latency_ms);
}

/// Record TTS time-to-first-chunk.
pub fn record_tts_first_chunk(provider: &str, latency_ms: f64) {
    histogram!("voicebot_tts_first_chunk_ms", "provider" => provider.to_string())
        .record(latency_ms);
}

/// Record an interrupt event.
pub fn record_interrupt() {
    counter!("voicebot_interrupts_total").increment(1);
}

/// Record a component error.
pub fn record_error(component: &str, recoverable: bool) {
    counter!(
        "voicebot_errors_total",
        "component" => component.to_string(),
        "recoverable" => recoverable.to_string()
    )
    .increment(1);
}
