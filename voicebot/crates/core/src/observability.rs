/// Initialize structured tracing with JSON output.
pub fn init_tracing() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,voicebot_core=debug"));

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
}

/// Record a component error as a metric.
pub fn record_error(component: &str, recoverable: bool) {
    tracing::warn!(
        component = component,
        recoverable = recoverable,
        "component error recorded"
    );
}

/// Record an interrupt event.
pub fn record_interrupt() {
    tracing::info!("interrupt recorded");
}
