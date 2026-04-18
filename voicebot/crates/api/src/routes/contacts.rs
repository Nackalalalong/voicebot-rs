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

#[derive(Deserialize)]
pub struct ContactsQuery {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub status: Option<String>,
}

pub async fn list_contacts(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(campaign_id): Path<Uuid>,
    Query(params): Query<ContactsQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    // verify campaign belongs to tenant
    db::queries::campaigns::get_by_id(&state.db, user.tenant_id, campaign_id).await?;

    let pagination = crate::pagination::PaginationParams {
        page: params.page,
        limit: params.limit,
    };
    let limit = pagination.limit_clamped();
    let offset = pagination.offset();

    let (items, total) = tokio::try_join!(
        db::queries::contacts::list_by_campaign(
            &state.db,
            user.tenant_id,
            campaign_id,
            params.status.as_deref(),
            limit,
            offset,
        ),
        db::queries::contacts::count_by_campaign(&state.db, user.tenant_id, campaign_id),
    )?;

    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "page": pagination.page.unwrap_or(1),
        "limit": limit,
    })))
}

#[derive(Deserialize)]
pub struct CreateContactRequest {
    pub phone_number: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

pub async fn create_contact(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(campaign_id): Path<Uuid>,
    Json(req): Json<CreateContactRequest>,
) -> ApiResult<(StatusCode, Json<db::models::Contact>)> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    db::queries::campaigns::get_by_id(&state.db, user.tenant_id, campaign_id).await?;
    let contact = db::queries::contacts::create(
        &state.db,
        db::queries::contacts::CreateContact {
            tenant_id: user.tenant_id,
            campaign_id,
            phone_number: &req.phone_number,
            first_name: req.first_name.as_deref(),
            last_name: req.last_name.as_deref(),
            metadata: req.metadata.unwrap_or(serde_json::json!({})),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(contact)))
}

#[derive(Deserialize)]
pub struct BulkImportRequest {
    pub contacts: Vec<CreateContactRequest>,
}

pub async fn bulk_import_contacts(
    Extension(user): Extension<AuthUser>,
    State(state): State<Arc<AppState>>,
    Path(campaign_id): Path<Uuid>,
    Json(req): Json<BulkImportRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    if !user.is_admin() {
        return Err(ApiError::Forbidden);
    }
    if req.contacts.len() > 10_000 {
        return Err(ApiError::BadRequest("max 10,000 contacts per import".into()));
    }
    db::queries::campaigns::get_by_id(&state.db, user.tenant_id, campaign_id).await?;
    let contacts: Vec<_> = req
        .contacts
        .iter()
        .map(|c| db::queries::contacts::CreateContact {
            tenant_id: user.tenant_id,
            campaign_id,
            phone_number: &c.phone_number,
            first_name: c.first_name.as_deref(),
            last_name: c.last_name.as_deref(),
            metadata: c.metadata.clone().unwrap_or(serde_json::json!({})),
        })
        .collect();
    let count =
        db::queries::contacts::bulk_create(&state.db, user.tenant_id, campaign_id, contacts).await?;
    Ok(Json(serde_json::json!({ "imported": count })))
}
