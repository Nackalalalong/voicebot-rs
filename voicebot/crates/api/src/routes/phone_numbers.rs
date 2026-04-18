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
    db::queries::phone_numbers::delete(&state.db, user.tenant_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
