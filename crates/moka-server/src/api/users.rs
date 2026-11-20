//! Accounts: the admin creates people, sets what they may do, resets passwords.

use axum::extract::{Path, State};
use axum::{Json, Router, routing::get, routing::put};
use moka_core::Permissions;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{CurrentUser, hash_password, verify_password};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", get(list).post(create))
        .route("/users/{id}", put(update).delete(remove))
        .route("/me/password", put(change_own_password))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserView {
    pub id: i64,
    pub username: String,
    /// Bitmask: 1 admin, 2 manage requests, 4 request, 8 auto-approve, 16 manage library, 32 play.
    pub permissions: u32,
    pub created_at: i64,
}

impl From<moka_db::users::User> for UserView {
    fn from(u: moka_db::users::User) -> Self {
        Self {
            id: u.id,
            username: u.username.clone(),
            permissions: u.permissions().0,
            created_at: u.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewUser {
    pub username: String,
    pub password: String,
    /// Defaults to request + play.
    pub permissions: Option<u32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UserUpdate {
    pub permissions: Option<u32>,
    /// Sets a new password and signs the user out of every device.
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PasswordChange {
    pub current: String,
    pub new: String,
}

const ALL: u32 = 0b11_1111;

fn check_password(p: &str) -> ApiResult<()> {
    if p.chars().count() < 8 {
        return Err(ApiError::BadRequest("password must be at least 8 characters".into()));
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/users", operation_id = "list_users", tag = "users", responses((status = 200, body = [UserView])))]
pub async fn list(user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<Vec<UserView>>> {
    user.require(Permissions::ADMIN)?;
    Ok(Json(moka_db::users::list(&s.db).await?.into_iter().map(UserView::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/users", operation_id = "create_user", tag = "users", request_body = NewUser,
    responses((status = 200, body = UserView), (status = 409, description = "name taken")))]
pub async fn create(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(b): Json<NewUser>,
) -> ApiResult<Json<UserView>> {
    user.require(Permissions::ADMIN)?;
    let name = b.username.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::BadRequest("username must be 1-64 characters".into()));
    }
    check_password(&b.password)?;
    if moka_db::users::by_username(&s.db, name).await?.is_some() {
        return Err(ApiError::Conflict(format!("{name} already exists")));
    }
    let perms = Permissions(b.permissions.unwrap_or(Permissions::DEFAULT_USER.0) & ALL);
    let u = moka_db::users::create(&s.db, name, &hash_password(&b.password)?, perms).await?;
    Ok(Json(u.into()))
}

#[utoipa::path(put, path = "/api/v1/users/{id}", operation_id = "update_user", tag = "users", params(("id" = i64, Path)), request_body = UserUpdate,
    responses((status = 200, body = UserView)))]
pub async fn update(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<UserUpdate>,
) -> ApiResult<Json<UserView>> {
    user.require(Permissions::ADMIN)?;
    let target = moka_db::users::by_id(&s.db, id).await?.ok_or(ApiError::NotFound("user"))?;
    if let Some(p) = b.permissions {
        let p = Permissions(p & ALL);
        let losing_admin = target.permissions().has(Permissions::ADMIN) && p.0 & Permissions::ADMIN.0 == 0;
        if losing_admin && moka_db::users::admin_count(&s.db).await? <= 1 {
            return Err(ApiError::Conflict("there must be at least one admin".into()));
        }
        moka_db::users::set_permissions(&s.db, id, p).await?;
    }
    if let Some(pw) = b.password {
        check_password(&pw)?;
        moka_db::users::set_password(&s.db, id, &hash_password(&pw)?).await?;
    }
    let u = moka_db::users::by_id(&s.db, id).await?.ok_or(ApiError::NotFound("user"))?;
    Ok(Json(u.into()))
}

#[utoipa::path(delete, path = "/api/v1/users/{id}", operation_id = "delete_user", tag = "users", params(("id" = i64, Path)),
    responses((status = 200)))]
pub async fn remove(user: CurrentUser, State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<()> {
    user.require(Permissions::ADMIN)?;
    if id == user.0.id {
        return Err(ApiError::Conflict("you cannot delete your own account".into()));
    }
    moka_db::users::delete(&s.db, id).await?;
    Ok(())
}

#[utoipa::path(put, path = "/api/v1/me/password", tag = "users", request_body = PasswordChange,
    responses((status = 200), (status = 400, description = "current password wrong")))]
pub async fn change_own_password(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(b): Json<PasswordChange>,
) -> ApiResult<()> {
    if !verify_password(&b.current, &user.0.password_hash) {
        return Err(ApiError::BadRequest("the current password is not right".into()));
    }
    check_password(&b.new)?;
    moka_db::users::set_password(&s.db, user.0.id, &hash_password(&b.new)?).await?;
    Ok(())
}
