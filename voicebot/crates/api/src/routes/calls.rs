use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query, State},
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use auth::AuthUser;

use crate::{
    error::ApiResult,
    pagination::PaginationParams,
    state::AppState,
};

#[derive(Deserialize)]
pub struct CallsQuery {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub campaign_id: Option<Uuid>,
}

pub async fn list_calls(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Query(params): Query<CallsQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let pagination = PaginationParams {
        page: params.page,
        limit: params.limit,
    };
    let limit = pagination.limit_clamped();
    let offset = pagination.offset();

    let (items, total) = tokio::try_join!(
        db::queries::call_records::list(&state.db, user.tenant_id, params.campaign_id, limit, offset),
        db::queries::call_records::count(&state.db, user.tenant_id, params.campaign_id),
    )?;

    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "page": pagination.page.unwrap_or(1),
        "limit": limit,
    })))
}

pub async fn get_call(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<db::models::CallRecord>> {
    let record = db::queries::call_records::get_by_id(&state.db, user.tenant_id, id).await?;
    Ok(Json(record))
}

pub async fn get_recording_url(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let record = db::queries::call_records::get_by_id(&state.db, user.tenant_id, id).await?;
    let recording_key = match &record.recording_url {
        Some(key) => key.clone(),
        None => return Err(crate::error::ApiError::NotFound),
    };
    let url = state
        .storage
        .presign_get(&recording_key, std::time::Duration::from_secs(3600))
        .await
        .map_err(|e| crate::error::ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "url": url, "expires_in_secs": 3600 })))
}
