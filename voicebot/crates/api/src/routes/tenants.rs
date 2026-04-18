use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use uuid::Uuid;

use auth::AuthUser;

use crate::{
    error::{ApiError, ApiResult},
    pagination::{Page, PaginationParams},
    state::AppState,
};

pub async fn list_tenants(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<Page<db::models::Tenant>>> {
    if !user.is_superadmin() {
        return Err(ApiError::Forbidden);
    }
    let limit = params.limit_clamped();
    let offset = params.offset();
    let (items, total) = tokio::try_join!(
        db::queries::tenants::list(&state.db, limit, offset),
        db::queries::tenants::count(&state.db),
    )?;
    Ok(Json(Page::new(items, total, &params)))
}

#[derive(Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub slug: String,
    pub plan: Option<String>,
}

pub async fn create_tenant(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTenantRequest>,
) -> ApiResult<(StatusCode, Json<db::models::Tenant>)> {
    if !user.is_superadmin() {
        return Err(ApiError::Forbidden);
    }
    let tenant = db::queries::tenants::create(
        &state.db,
        &req.name,
        &req.slug,
        req.plan.as_deref().unwrap_or("starter"),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(tenant)))
}

pub async fn get_tenant(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<db::models::Tenant>> {
    // Users can only see their own tenant unless superadmin
    if !user.is_superadmin() && user.tenant_id != id {
        return Err(ApiError::Forbidden);
    }
    let tenant = db::queries::tenants::get_by_id(&state.db, id).await?;
    Ok(Json(tenant))
}

pub async fn delete_tenant(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if !user.is_superadmin() {
        return Err(ApiError::Forbidden);
    }
    db::queries::tenants::deactivate(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct UsageQuery {
    /// Number of days to look back (default 30)
    pub days: Option<i64>,
}

pub async fn get_usage(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Query(q): Query<UsageQuery>,
) -> ApiResult<Json<db::queries::usage::UsageSummary>> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let to = Utc::now();
    let from = to - Duration::days(days);
    let summary = db::queries::usage::aggregate(&state.db, user.tenant_id, from, to).await?;
    Ok(Json(summary))
}
