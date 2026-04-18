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

pub async fn list_phone_numbers(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<Page<db::models::PhoneNumber>>> {
    let limit = params.limit_clamped();
    let offset = params.offset();
    let (items, total) = tokio::try_join!(
        db::queries::phone_numbers::list(&state.db, user.tenant_id, limit, offset),
        db::queries::phone_numbers::count(&state.db, user.tenant_id),
    )?;
    Ok(Json(Page::new(items, total, &params)))
}

#[derive(Deserialize)]
pub struct ProvisionPhoneNumberRequest {
    pub number: String,
    pub provider: String,
    pub provider_number_id: Option<String>,
    pub capabilities: Option<serde_json::Value>,
    pub monthly_cost_usd_cents: Option<i64>,
}

pub async fn provision_phone_number(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Json(req): Json<ProvisionPhoneNumberRequest>,
) -> ApiResult<(StatusCode, Json<db::models::PhoneNumber>)> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let capabilities = req
        .capabilities
        .unwrap_or(serde_json::json!({"voice": true, "sms": false}));
    let pn = db::queries::phone_numbers::create(
        &state.db,
        user.tenant_id,
        &req.number,
        &req.provider,
        req.provider_number_id.as_deref(),
        capabilities,
        req.monthly_cost_usd_cents,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(pn)))
}

pub async fn delete_phone_number(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let pn = db::queries::phone_numbers::get_by_id(&state.db, user.tenant_id, id).await?;
    db::queries::phone_numbers::delete(&state.db, user.tenant_id, id).await?;

    let mut conn = state.redis.clone();
    if let Err(e) = cache::routing::del_route(&mut conn, &pn.number).await {
        tracing::warn!(number = %pn.number, error = %e, "failed to remove Redis routing cache entry");
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AssignCampaignRequest {
    pub campaign_id: Uuid,
}

/// PUT /phone-numbers/:id/campaign — assign a phone number to a campaign.
/// Also updates the Redis routing table so ARI calls are routed immediately.
pub async fn assign_campaign(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AssignCampaignRequest>,
) -> ApiResult<Json<db::models::PhoneNumber>> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let pn = db::queries::phone_numbers::assign_campaign(
        &state.db,
        user.tenant_id,
        id,
        req.campaign_id,
    )
    .await?;

    // Update Redis routing cache so ARI transport routes immediately.
    let mut conn = state.redis.clone();
    let route = cache::PhoneRoute {
        tenant_id: user.tenant_id,
        campaign_id: req.campaign_id,
    };
    if let Err(e) = cache::routing::set_route(&mut conn, &pn.number, &route).await {
        tracing::warn!(number = %pn.number, error = %e, "failed to update Redis routing cache");
    }

    Ok(Json(pn))
}

/// DELETE /phone-numbers/:id/campaign — unassign a phone number from its campaign.
pub async fn unassign_campaign(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<db::models::PhoneNumber>> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let pn = db::queries::phone_numbers::unassign_campaign(
        &state.db,
        user.tenant_id,
        id,
    )
    .await?;

    // Remove from Redis routing cache.
    let mut conn = state.redis.clone();
    if let Err(e) = cache::routing::del_route(&mut conn, &pn.number).await {
        tracing::warn!(number = %pn.number, error = %e, "failed to remove Redis routing cache entry");
    }

    Ok(Json(pn))
}
