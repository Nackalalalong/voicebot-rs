use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Extension, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use tokio_stream::StreamExt;

use auth::AuthUser;

use crate::state::AppState;

/// Server-Sent Events endpoint for real-time campaign and call updates.
/// Emits heartbeats every 15 s; future versions will fan-out Redis pub/sub events.
pub async fn stream_events(
    Extension(user): Extension<AuthUser>,
    State(_state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let tenant_id = user.tenant_id;

    // Build an infinite stream of interval ticks then map to SSE events.
    let tick_stream = tokio_stream::wrappers::IntervalStream::new(
        tokio::time::interval(Duration::from_secs(15)),
    );

    let event_stream = tick_stream.map(move |_| {
        let event = Event::default()
            .event("heartbeat")
            .data(
                serde_json::json!({
                    "tenant_id": tenant_id,
                    "ts": chrono::Utc::now().to_rfc3339(),
                })
                .to_string(),
            );
        Ok::<_, Infallible>(event)
    });

    Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
