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

/// Generic heartbeat SSE — kept for backward compatibility.
pub async fn stream_events(
    Extension(user): Extension<AuthUser>,
    State(_state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let tenant_id = user.tenant_id;

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

/// `GET /metrics/live` — emits `active_calls` events with `{"count": N}` every 5 s.
/// Used by the Overview page stat card.
pub async fn stream_metrics_live(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let db = state.db.clone();

    let tick_stream = tokio_stream::wrappers::IntervalStream::new(
        tokio::time::interval(Duration::from_secs(5)),
    );

    let event_stream = tick_stream.then(move |_| {
        let db = db.clone();
        let tenant_id = user.tenant_id;
        async move {
            let count = db::queries::call_records::count_active(&db, tenant_id)
                .await
                .unwrap_or(0);
            let event = Event::default()
                .event("active_calls")
                .data(serde_json::json!({"count": count}).to_string());
            Ok::<_, Infallible>(event)
        }
    });

    Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// `GET /sessions/live` — emits `active_calls` events with `{"calls": [...]}` every 5 s.
/// Used by the Live Monitor page table.
pub async fn stream_sessions_live(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let db = state.db.clone();

    let tick_stream = tokio_stream::wrappers::IntervalStream::new(
        tokio::time::interval(Duration::from_secs(5)),
    );

    let event_stream = tick_stream.then(move |_| {
        let db = db.clone();
        let tenant_id = user.tenant_id;
        async move {
            let rows = db::queries::call_records::list_active(&db, tenant_id)
                .await
                .unwrap_or_default();

            // Augment with a `duration_secs` calculated server-side.
            let calls: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|r| {
                    let duration_secs = r
                        .started_at
                        .map(|s| {
                            (chrono::Utc::now() - s).num_seconds().max(0) as u64
                        })
                        .unwrap_or(0);
                    serde_json::json!({
                        "session_id": r.session_id,
                        "phone_number": r.phone_number,
                        "direction": r.direction,
                        "campaign_id": r.campaign_id,
                        "started_at": r.started_at.map(|t| t.to_rfc3339()),
                        "duration_secs": duration_secs,
                    })
                })
                .collect();

            let event = Event::default()
                .event("active_calls")
                .data(serde_json::json!({"calls": calls}).to_string());
            Ok::<_, Infallible>(event)
        }
    });

    Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
