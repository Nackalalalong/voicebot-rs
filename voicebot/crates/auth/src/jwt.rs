use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AuthError, Result};

const ACCESS_TOKEN_TTL_MINUTES: i64 = 60;
const REFRESH_TOKEN_TTL_DAYS: i64 = 30;
const SHORT_LIVED_TOKEN_MINUTES: i64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,          // user id
    pub tenant_id: String,
    pub email: String,
    pub role: String,
    pub exp: i64,
    pub iat: i64,
    pub token_type: String,   // "access" | "refresh" | "ws_session"
}

impl Claims {
    pub fn user_id(&self) -> Result<Uuid> {
        Uuid::parse_str(&self.sub)
            .map_err(|_| AuthError::InvalidToken("invalid user id in claims".into()))
    }

    pub fn tenant_id(&self) -> Result<Uuid> {
        Uuid::parse_str(&self.tenant_id)
            .map_err(|_| AuthError::InvalidToken("invalid tenant id in claims".into()))
    }
}

pub fn issue_access_token(
    secret: &str,
    user_id: Uuid,
    tenant_id: Uuid,
    email: &str,
    role: &str,
) -> Result<String> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id.to_string(),
        tenant_id: tenant_id.to_string(),
        email: email.to_string(),
        role: role.to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::minutes(ACCESS_TOKEN_TTL_MINUTES)).timestamp(),
        token_type: "access".into(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AuthError::InvalidToken(e.to_string()))
}

pub fn issue_refresh_token(
    secret: &str,
    user_id: Uuid,
    tenant_id: Uuid,
    email: &str,
    role: &str,
) -> Result<String> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id.to_string(),
        tenant_id: tenant_id.to_string(),
        email: email.to_string(),
        role: role.to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::days(REFRESH_TOKEN_TTL_DAYS)).timestamp(),
        token_type: "refresh".into(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AuthError::InvalidToken(e.to_string()))
}

/// Short-lived token for WebSocket session initiation.
pub fn issue_ws_session_token(
    secret: &str,
    user_id: Uuid,
    tenant_id: Uuid,
    campaign_id: Uuid,
) -> Result<String> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id.to_string(),
        tenant_id: tenant_id.to_string(),
        email: campaign_id.to_string(), // reuse field to carry campaign_id
        role: "ws_session".into(),
        iat: now.timestamp(),
        exp: (now + Duration::minutes(SHORT_LIVED_TOKEN_MINUTES)).timestamp(),
        token_type: "ws_session".into(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AuthError::InvalidToken(e.to_string()))
}

pub fn validate_token(secret: &str, token: &str) -> Result<Claims> {
    let mut validation = Validation::default();
    validation.validate_exp = true;

    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
        _ => AuthError::InvalidToken(e.to_string()),
    })
}
