pub mod error;
pub mod pagination;
pub mod routes;
pub mod state;

use std::sync::Arc;

use axum::{
    extract::State,
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use auth::{auth_middleware, JwtSecret};
use state::AppState;

pub fn create_router(state: Arc<AppState>) -> Router {
    let jwt_secret = JwtSecret(state.jwt_secret.clone());

    // Public routes (no auth)
    let public = Router::new()
        .route("/auth/register", post(routes::auth::register))
        .route("/auth/login", post(routes::auth::login))
        .route("/auth/refresh", post(routes::auth::refresh))
        .route("/healthz", get(healthz))
        .with_state(state.clone());

    // Protected routes (require valid access token)
    let protected = Router::new()
        // SSE
        .route("/events", get(routes::sse::stream_events))
        .route("/metrics/live", get(routes::sse::stream_metrics_live))
        .route("/sessions/live", get(routes::sse::stream_sessions_live))
        // Tenants (superadmin only — enforced inside handlers)
        .route("/tenants", get(routes::tenants::list_tenants))
        .route("/tenants", post(routes::tenants::create_tenant))
        .route("/tenants/:id", get(routes::tenants::get_tenant))
        .route("/tenants/:id", delete(routes::tenants::delete_tenant))
        // Users
        .route("/users", get(routes::users::list_users))
        .route("/users", post(routes::users::create_user))
        .route("/users/:id", get(routes::users::get_user))
        .route("/users/:id/password", put(routes::users::change_password))
        .route("/users/:id", delete(routes::users::delete_user))
        // Campaigns
        .route("/campaigns", get(routes::campaigns::list_campaigns))
        .route("/campaigns", post(routes::campaigns::create_campaign))
        .route("/campaigns/:id", get(routes::campaigns::get_campaign))
        .route("/campaigns/:id/status", put(routes::campaigns::update_campaign_status))
        .route("/campaigns/:id/prompt", put(routes::campaigns::update_campaign_prompt))
        .route("/campaigns/:id", delete(routes::campaigns::delete_campaign))
        .route("/campaigns/:id/session-token", post(routes::campaigns::issue_session_token))
        .route("/campaigns/:id/analytics", get(routes::campaigns::get_campaign_analytics))
        .route("/campaigns/:id/calls", get(routes::campaigns::list_campaign_calls))
        .route("/campaigns/:id/metrics", put(routes::campaigns::update_campaign_metrics))
        // Contacts
        .route("/campaigns/:campaign_id/contacts", get(routes::contacts::list_contacts))
        .route("/campaigns/:campaign_id/contacts", post(routes::contacts::create_contact))
        .route("/campaigns/:campaign_id/contacts/import", post(routes::contacts::bulk_import_contacts))
        // Call records
        .route("/calls", get(routes::calls::list_calls))
        .route("/calls/:id", get(routes::calls::get_call))
        .route("/calls/:id/recording", get(routes::calls::get_recording_url))
        // Usage
        .route("/usage", get(routes::tenants::get_usage))
        // Phone numbers
        .route("/phone-numbers", get(routes::phone_numbers::list_phone_numbers))
        .route("/phone-numbers", post(routes::phone_numbers::provision_phone_number))
        .route("/phone-numbers/:id", delete(routes::phone_numbers::delete_phone_number))
        .route("/phone-numbers/:id/campaign", put(routes::phone_numbers::assign_campaign))
        .route("/phone-numbers/:id/campaign", delete(routes::phone_numbers::unassign_campaign))
        .layer(middleware::from_fn_with_state(jwt_secret, auth_middleware))
        .with_state(state.clone());

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .nest("/api/v1", public)
        .nest("/api/v1", protected)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz(State(state): State<Arc<AppState>>) -> Result<&'static str, axum::http::StatusCode> {
    // Ping DB
    db::health_check(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::SERVICE_UNAVAILABLE)?;
    // Ping Redis
    cache::health_check(&mut state.redis.clone())
        .await
        .map_err(|_| axum::http::StatusCode::SERVICE_UNAVAILABLE)?;
    Ok("ok")
}
