use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

use crate::{error::ApiResult, state::AppState};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub tenant_slug: String,
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user: UserInfo,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub tenant_id: String,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> ApiResult<(StatusCode, Json<LoginResponse>)> {
    let tenant = db::queries::tenants::get_by_slug(&state.db, &req.tenant_slug).await?;
    let user = db::queries::users::get_by_email(&state.db, tenant.id, &req.email).await?;

    let valid = auth::verify_password(&req.password, &user.password_hash)
        .await
        .map_err(|e| crate::error::ApiError::Unauthorized(e.to_string()))?;

    if !valid {
        return Err(crate::error::ApiError::Unauthorized("invalid credentials".into()));
    }

    let access_token = auth::issue_access_token(
        &state.jwt_secret,
        user.id,
        tenant.id,
        &user.email,
        &user.role,
    )
    .map_err(|e| crate::error::ApiError::Internal(e.to_string()))?;

    let refresh_token = auth::issue_refresh_token(
        &state.jwt_secret,
        user.id,
        tenant.id,
        &user.email,
        &user.role,
    )
    .map_err(|e| crate::error::ApiError::Internal(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(LoginResponse {
            access_token,
            refresh_token,
            user: UserInfo {
                id: user.id.to_string(),
                email: user.email,
                display_name: user.display_name,
                role: user.role,
                tenant_id: tenant.id.to_string(),
            },
        }),
    ))
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

pub async fn refresh(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RefreshRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let claims = auth::validate_token(&state.jwt_secret, &req.refresh_token)
        .map_err(|e| crate::error::ApiError::Unauthorized(e.to_string()))?;

    if claims.token_type != "refresh" {
        return Err(crate::error::ApiError::Unauthorized("invalid token type".into()));
    }

    let user_id = claims.user_id().map_err(|e| crate::error::ApiError::Unauthorized(e.to_string()))?;
    let tenant_id = claims.tenant_id().map_err(|e| crate::error::ApiError::Unauthorized(e.to_string()))?;

    let access_token = auth::issue_access_token(
        &state.jwt_secret,
        user_id,
        tenant_id,
        &claims.email,
        &claims.role,
    )
    .map_err(|e| crate::error::ApiError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "access_token": access_token })))
}
