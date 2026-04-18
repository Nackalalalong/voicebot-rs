use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("internal error")]
    Internal(String),
}

impl From<db::DbError> for ApiError {
    fn from(e: db::DbError) -> Self {
        match e {
            db::DbError::NotFound => ApiError::NotFound,
            db::DbError::Duplicate(msg) => ApiError::Conflict(msg),
            e => ApiError::Internal(e.to_string()),
        }
    }
}

impl From<auth::AuthError> for ApiError {
    fn from(e: auth::AuthError) -> Self {
        match e {
            auth::AuthError::InvalidCredentials => ApiError::Unauthorized("invalid credentials".into()),
            auth::AuthError::TokenExpired => ApiError::Unauthorized("token expired".into()),
            auth::AuthError::MissingToken => ApiError::Unauthorized("missing token".into()),
            auth::AuthError::Forbidden => ApiError::Forbidden,
            e => ApiError::Unauthorized(e.to_string()),
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".into()),        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;
