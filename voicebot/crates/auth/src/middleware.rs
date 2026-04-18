use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

use crate::{
    error::AuthError,
    jwt::{validate_token, Claims},
};

/// Axum extension injected by the auth middleware.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub role: String,
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin" || self.role == "superadmin"
    }

    pub fn is_superadmin(&self) -> bool {
        self.role == "superadmin"
    }
}

/// Auth state expected by the middleware.
#[derive(Clone)]
pub struct JwtSecret(pub String);

pub async fn auth_middleware(
    State(secret): State<JwtSecret>,
    mut request: Request,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    let token = extract_bearer_token(&request)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "missing authorization header".into()))?;

    let claims = validate_token(&secret.0, token)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

    if claims.token_type != "access" {
        return Err((StatusCode::UNAUTHORIZED, "invalid token type".into()));
    }

    let user = auth_user_from_claims(&claims)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}

fn extract_bearer_token(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
}

fn auth_user_from_claims(claims: &Claims) -> Result<AuthUser, AuthError> {
    Ok(AuthUser {
        user_id: claims.user_id()?,
        tenant_id: claims.tenant_id()?,
        email: claims.email.clone(),
        role: claims.role.clone(),
    })
}

/// Extractor for requiring specific roles.
pub struct RequireRole(pub &'static str);

impl RequireRole {
    pub fn check(&self, user: &AuthUser) -> Result<(), AuthError> {
        match self.0 {
            "admin" if !user.is_admin() => Err(AuthError::Forbidden),
            "superadmin" if !user.is_superadmin() => Err(AuthError::Forbidden),
            _ => Ok(()),
        }
    }
}
