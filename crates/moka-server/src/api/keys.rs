//! API keys for other programs (e.g. an indexer manager or a request app)
//! that talk to Moka. The key is shown once; only its hash is stored.

use axum::extract::{Path, State};
use axum::{Json, Router, routing::delete, routing::get};
use moka_core::Permissions;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{CurrentUser, new_token, token_hash};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api-keys", get(list_keys).post(create_key))
        .route("/api-keys/{id}", delete(delete_key))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyView {
    pub id: i64,
    pub name: String,
    pub user_id: i64,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

impl From<moka_db::users::ApiKey> for ApiKeyView {
    fn from(k: moka_db::users::ApiKey) -> Self {
        Self {
            id: k.id,
            name: k.name,
            user_id: k.user_id,
            created_at: k.created_at,
            last_used_at: k.last_used_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewApiKey {
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedApiKey {
    #[serde(flatten)]
    pub info: ApiKeyView,
    /// Shown only now. Moka keeps just a hash of it.
    pub key: String,
}

#[utoipa::path(get, path = "/api/v1/api-keys", tag = "users", responses((status = 200, body = [ApiKeyView])))]
pub async fn list_keys(user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<Vec<ApiKeyView>>> {
    user.require(Permissions::ADMIN)?;
    Ok(Json(moka_db::users::api_keys(&s.db).await?.into_iter().map(Into::into).collect()))
}

/// Creates a key that acts as the signed-in admin.
#[utoipa::path(post, path = "/api/v1/api-keys", tag = "users", request_body = NewApiKey,
    responses((status = 200, body = CreatedApiKey)))]
pub async fn create_key(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(body): Json<NewApiKey>,
) -> ApiResult<Json<CreatedApiKey>> {
    user.require(Permissions::ADMIN)?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("give the key a name".into()));
    }
    let key = new_token()?;
    let info = moka_db::users::create_api_key(&s.db, user.0.id, name, &token_hash(&key)).await?;
    Ok(Json(CreatedApiKey { info: info.into(), key }))
}

#[utoipa::path(delete, path = "/api/v1/api-keys/{id}", tag = "users", params(("id" = i64, Path)),
    responses((status = 200)))]
pub async fn delete_key(user: CurrentUser, State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<()> {
    user.require(Permissions::ADMIN)?;
    moka_db::users::delete_api_key(&s.db, id).await?;
    Ok(())
}
