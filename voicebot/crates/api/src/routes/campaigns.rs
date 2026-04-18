use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use auth::AuthUser;

use crate::{
    error::{ApiError, ApiResult},
    pagination::{Page, PaginationParams},
    state::AppState,
};

async fn cache_and_publish_campaign(redis: &cache::RedisPool, campaign: &db::models::Campaign) {
    let update = cache::campaign::CampaignConfigUpdate {
        campaign_id: campaign.id,
        tenant_id: campaign.tenant_id,
        status: campaign.status.clone(),
        system_prompt: campaign.system_prompt.clone(),
        custom_metrics: campaign.custom_metrics.clone(),
    };

    let mut conn = redis.clone();
    if let Err(error) = cache::campaign::set_config(&mut conn, campaign.id, campaign).await {
        tracing::warn!(campaign_id = %campaign.id, error = %error, "failed to refresh campaign cache");
        return;
    }

    if let Err(error) = cache::campaign::publish_update(&mut conn, &update).await {
        tracing::warn!(campaign_id = %campaign.id, error = %error, "failed to publish campaign update");
    }
}

pub async fn list_campaigns(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<Page<db::models::Campaign>>> {
    let limit = params.limit_clamped();
    let offset = params.offset();
    let (items, total) = tokio::try_join!(
        db::queries::campaigns::list(&state.db, user.tenant_id, limit, offset),
        db::queries::campaigns::count(&state.db, user.tenant_id),
    )?;
    Ok(Json(Page::new(items, total, &params)))
}

#[derive(Deserialize)]
pub struct CreateCampaignRequest {
    pub name: String,
    pub system_prompt: Option<String>,
    pub language: Option<String>,
    pub voice_id: Option<String>,
    pub asr_provider: Option<String>,
    pub tts_provider: Option<String>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub max_call_duration_secs: Option<i32>,
    pub recording_enabled: Option<bool>,
    pub tools_config: Option<serde_json::Value>,
    pub custom_metrics: Option<serde_json::Value>,
    pub schedule_config: Option<serde_json::Value>,
}

pub async fn create_campaign(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCampaignRequest>,
) -> ApiResult<(StatusCode, Json<db::models::Campaign>)> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let campaign = db::queries::campaigns::create(
        &state.db,
        db::queries::campaigns::CreateCampaign {
            tenant_id: user.tenant_id,
            name: &req.name,
            system_prompt: req.system_prompt.as_deref().unwrap_or(""),
            language: req.language.as_deref().unwrap_or("en"),
            voice_id: req.voice_id.as_deref(),
            asr_provider: req.asr_provider.as_deref().unwrap_or("whisper"),
            tts_provider: req.tts_provider.as_deref().unwrap_or("kokoro"),
            llm_provider: req.llm_provider.as_deref().unwrap_or("openai"),
            llm_model: req.llm_model.as_deref().unwrap_or("gpt-4o-mini"),
            max_call_duration_secs: req.max_call_duration_secs.unwrap_or(300),
            recording_enabled: req.recording_enabled.unwrap_or(true),
            tools_config: req.tools_config.unwrap_or(serde_json::json!([])),
            custom_metrics: req.custom_metrics.unwrap_or(serde_json::json!({})),
            schedule_config: req.schedule_config.unwrap_or(serde_json::json!({})),
        },
    )
    .await?;
    cache_and_publish_campaign(&state.redis, &campaign).await;
    Ok((StatusCode::CREATED, Json(campaign)))
}

pub async fn get_campaign(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<db::models::Campaign>> {
    let campaign = db::queries::campaigns::get_by_id(&state.db, user.tenant_id, id).await?;
    Ok(Json(campaign))
}

#[derive(Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
}

pub async fn update_campaign_status(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateStatusRequest>,
) -> ApiResult<Json<db::models::Campaign>> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let valid_statuses = ["draft", "active", "paused", "completed", "archived"];
    if !valid_statuses.contains(&req.status.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "invalid status: {}",
            req.status
        )));
    }
    let campaign =
        db::queries::campaigns::update_status(&state.db, user.tenant_id, id, &req.status).await?;
    cache_and_publish_campaign(&state.redis, &campaign).await;
    Ok(Json(campaign))
}

#[derive(Deserialize)]
pub struct UpdatePromptRequest {
    pub system_prompt: String,
    pub change_note: Option<String>,
}

pub async fn update_campaign_prompt(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePromptRequest>,
) -> ApiResult<Json<db::models::Campaign>> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let campaign =
        db::queries::campaigns::update_prompt(&state.db, user.tenant_id, id, &req.system_prompt)
            .await?;
    cache_and_publish_campaign(&state.redis, &campaign).await;
    Ok(Json(campaign))
}

pub async fn delete_campaign(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    db::queries::campaigns::delete(&state.db, user.tenant_id, id).await?;
    let mut redis = state.redis.clone();
    let _ = cache::campaign::invalidate(&mut redis, id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Issue a short-lived WebSocket session token for a campaign.
pub async fn issue_session_token(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    // Verify campaign belongs to tenant
    db::queries::campaigns::get_by_id(&state.db, user.tenant_id, id).await?;
    let token = auth::issue_ws_session_token(&state.jwt_secret, user.user_id, user.tenant_id, id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "token": token, "expires_in_secs": 300 }),
    ))
}

/// GET /campaigns/:id/analytics
pub async fn get_campaign_analytics(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    db::queries::campaigns::get_by_id(&state.db, user.tenant_id, id).await?;
    let (stats, sentiment) = tokio::try_join!(
        db::queries::call_records::analytics_for_campaign(&state.db, user.tenant_id, id),
        db::queries::call_records::sentiment_breakdown(&state.db, user.tenant_id, id),
    )?;
    Ok(Json(serde_json::json!({
        "total_calls": stats.total_calls,
        "completed_calls": stats.completed_calls,
        "avg_duration_secs": stats.avg_duration_secs,
        "answer_rate": stats.answer_rate,
        "sentiment_breakdown": sentiment,
    })))
}

/// GET /campaigns/:id/calls
pub async fn list_campaign_calls(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<Page<db::models::CallRecord>>> {
    db::queries::campaigns::get_by_id(&state.db, user.tenant_id, id).await?;
    let limit = params.limit_clamped();
    let offset = params.offset();
    let (items, total) = tokio::try_join!(
        db::queries::call_records::list(&state.db, user.tenant_id, Some(id), limit, offset),
        db::queries::call_records::count(&state.db, user.tenant_id, Some(id)),
    )?;
    Ok(Json(Page::new(items, total, &params)))
}

/// PUT /campaigns/:id/metrics
#[derive(serde::Deserialize)]
pub struct UpdateMetricsRequest {
    pub custom_metrics_config: serde_json::Value,
}

pub async fn update_campaign_metrics(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateMetricsRequest>,
) -> ApiResult<Json<db::models::Campaign>> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let campaign = db::queries::campaigns::update_metrics(
        &state.db,
        user.tenant_id,
        id,
        req.custom_metrics_config,
    )
    .await?;
    cache_and_publish_campaign(&state.redis, &campaign).await;
    Ok(Json(campaign))
}
