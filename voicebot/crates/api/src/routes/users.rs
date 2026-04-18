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

pub async fn list_users(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<Page<db::models::User>>> {
    let limit = params.limit_clamped();
    let offset = params.offset();
    let (items, total) = tokio::try_join!(
        db::queries::users::list(&state.db, user.tenant_id, limit, offset),
        db::queries::users::count(&state.db, user.tenant_id),
    )?;
    // Strip password hashes from response
    let items: Vec<_> = items
        .into_iter()
        .map(|mut u| {
            u.password_hash = String::new();
            u
        })
        .collect();
    Ok(Json(Page::new(items, total, &params)))
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub email: String,
    pub password: String,
    pub display_name: String,
    pub role: Option<String>,
}

pub async fn create_user(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateUserRequest>,
) -> ApiResult<(StatusCode, Json<db::models::User>)> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let role = req.role.as_deref().unwrap_or("operator");
    let hash = auth::hash_password(&req.password)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut new_user =
        db::queries::users::create(&state.db, user.tenant_id, &req.email, &hash, &req.display_name, role).await?;
    new_user.password_hash = String::new();
    Ok((StatusCode::CREATED, Json(new_user)))
}

pub async fn get_user(
    Extension(auth_user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<db::models::User>> {
    let mut user = db::queries::users::get_by_id(&state.db, auth_user.tenant_id, id).await?;
    user.password_hash = String::new();
    Ok(Json(user))
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    Extension(auth_user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    // Users can only change their own password
    if auth_user.user_id != id && !auth_user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    let user = db::queries::users::get_by_id(&state.db, auth_user.tenant_id, id).await?;
    let valid = auth::verify_password(&req.current_password, &user.password_hash)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !valid {
        return Err(ApiError::Unauthorized("invalid current password".into()));
    }
    let new_hash = auth::hash_password(&req.new_password)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::queries::users::update_password(&state.db, auth_user.tenant_id, id, &new_hash).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_user(
    Extension(auth_user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if !auth_user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    db::queries::users::deactivate(&state.db, auth_user.tenant_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
