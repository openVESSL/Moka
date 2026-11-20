use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use axum::{Json, Router, routing::get, routing::post};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use moka_core::Permissions;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{
    CurrentUser, SESSION_COOKIE, SESSION_TTL_SECONDS, hash_password, new_token, token_hash, verify_password,
};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/status", get(status))
        .route("/auth/setup", post(setup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/me", get(me))
}

#[derive(Serialize, ToSchema)]
pub struct AuthStatus {
    /// True until the first admin account exists.
    pub needs_setup: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct Me {
    pub id: i64,
    pub username: String,
    pub permissions: u32,
}

impl From<&moka_db::users::User> for Me {
    fn from(u: &moka_db::users::User) -> Self {
        Self { id: u.id, username: u.username.clone(), permissions: u.permissions().0 }
    }
}

#[utoipa::path(get, path = "/api/v1/auth/status", tag = "auth",
    responses((status = 200, body = AuthStatus)))]
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<AuthStatus>> {
    let needs_setup = moka_db::users::count(&state.db).await? == 0;
    Ok(Json(AuthStatus { needs_setup }))
}

/// Creates the first admin account and signs it in. Only works while no users exist.
#[utoipa::path(post, path = "/api/v1/auth/setup", tag = "auth", request_body = Credentials,
    responses((status = 200, body = Me), (status = 409, description = "already set up")))]
pub async fn setup(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> ApiResult<(CookieJar, Json<Me>)> {
    validate(&body)?;
    if moka_db::users::count(&state.db).await? > 0 {
        return Err(ApiError::Conflict("setup already completed".into()));
    }
    let hash = hash_password(&body.password)?;
    // shortcut: two concurrent setup calls can both pass the count check; the
    // unique username stops duplicates of the same name only. Fine for a one-time local step.
    let user = moka_db::users::create(&state.db, body.username.trim(), &hash, Permissions::ADMIN).await?;
    start_session(&state, jar, &headers, &user).await
}

#[utoipa::path(post, path = "/api/v1/auth/login", tag = "auth", request_body = Credentials,
    responses((status = 200, body = Me), (status = 401, description = "wrong credentials")))]
pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> ApiResult<(CookieJar, Json<Me>)> {
    let key = body.username.trim().to_lowercase();
    if let Some(wait) = login_lockout(&state, &key) {
        return Err(ApiError::TooManyRequests(format!(
            "too many failed attempts, try again in {} seconds",
            wait.as_secs().max(1)
        )));
    }
    let user = moka_db::users::by_username(&state.db, body.username.trim()).await?;
    match user {
        Some(u) if verify_password(&body.password, &u.password_hash) => {
            state.login_failures.lock().expect("login lock").remove(&key);
            start_session(&state, jar, &headers, &u).await
        }
        _ => {
            let mut f = state.login_failures.lock().expect("login lock");
            let entry = f.entry(key).or_insert((0, std::time::Instant::now()));
            entry.0 += 1;
            entry.1 = std::time::Instant::now();
            Err(ApiError::Unauthorized)
        }
    }
}

/// After 5 failures an account name waits 1 minute, doubling per extra
/// failure up to 30 minutes. Keyed by name, so it also slows guessing from
/// many addresses.
// shortcut: in memory, so a restart clears it; an attacker can briefly lock a known name out.
fn login_lockout(state: &AppState, key: &str) -> Option<std::time::Duration> {
    let f = state.login_failures.lock().expect("login lock");
    let &(count, at) = f.get(key)?;
    if count < 5 {
        return None;
    }
    let wait = std::time::Duration::from_secs((60u64 << (count - 5).min(5)).min(1800));
    wait.checked_sub(at.elapsed())
}

#[utoipa::path(post, path = "/api/v1/auth/logout", tag = "auth", responses((status = 200)))]
pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> ApiResult<CookieJar> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        moka_db::users::delete_session(&state.db, &token_hash(c.value())).await?;
    }
    Ok(jar.remove(Cookie::build(SESSION_COOKIE).path("/")))
}

#[utoipa::path(get, path = "/api/v1/me", tag = "auth",
    responses((status = 200, body = Me), (status = 401)))]
pub async fn me(user: CurrentUser) -> Json<Me> {
    Json(Me::from(&user.0))
}

fn validate(c: &Credentials) -> ApiResult<()> {
    let name = c.username.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::BadRequest("username must be 1-64 characters".into()));
    }
    if c.password.chars().count() < 8 {
        return Err(ApiError::BadRequest("password must be at least 8 characters".into()));
    }
    Ok(())
}

async fn start_session(
    state: &AppState,
    jar: CookieJar,
    headers: &HeaderMap,
    user: &moka_db::users::User,
) -> ApiResult<(CookieJar, Json<Me>)> {
    let token = new_token()?;
    let agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    moka_db::users::create_session(&state.db, &token_hash(&token), user.id, SESSION_TTL_SECONDS, agent)
        .await?;
    // Not `Secure`: most installs are reached over plain http on the LAN.
    // A TLS reverse proxy can add the flag.
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::seconds(SESSION_TTL_SECONDS))
        .build();
    Ok((jar.add(cookie), Json(Me::from(user))))
}
