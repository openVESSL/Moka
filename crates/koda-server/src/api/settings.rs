//! Global settings edited in the UI.

use axum::extract::State;
use axum::{Json, Router, routing::get};
use koda_core::Permissions;
use koda_metadata::{List, Tmdb};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::requests::Quota;
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, CatalogSettings};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings/catalog", get(get_catalog).put(put_catalog))
        .route("/settings/requests", get(get_requests).put(put_requests))
        .route("/settings/library", get(get_library).put(put_library))
}

/// Catalog settings as the UI sees them. The key itself is never returned.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogSettingsView {
    pub tmdb_key_set: bool,
    pub language: String,
    pub region: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CatalogSettingsUpdate {
    /// New key. Omit to keep the current one.
    pub tmdb_key: Option<String>,
    pub language: String,
    pub region: Option<String>,
}

async fn load(state: &AppState) -> ApiResult<CatalogSettings> {
    Ok(CatalogSettings::load(&state.db).await?)
}

#[utoipa::path(get, path = "/api/v1/settings/catalog", tag = "settings",
    responses((status = 200, body = CatalogSettingsView)))]
pub async fn get_catalog(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<CatalogSettingsView>> {
    user.require(Permissions::ADMIN)?;
    let s = load(&state).await?;
    Ok(Json(CatalogSettingsView {
        tmdb_key_set: !s.tmdb_key.is_empty(),
        language: s.language,
        region: s.region,
    }))
}

/// Saves catalog settings. A new key is checked against TMDB before it is stored.
#[utoipa::path(put, path = "/api/v1/settings/catalog", tag = "settings", request_body = CatalogSettingsUpdate,
    responses((status = 200, body = CatalogSettingsView), (status = 400, description = "key rejected")))]
pub async fn put_catalog(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(body): Json<CatalogSettingsUpdate>,
) -> ApiResult<Json<CatalogSettingsView>> {
    user.require(Permissions::ADMIN)?;
    let current = load(&state).await?;
    let language = body.language.trim();
    if !valid_language(language) {
        return Err(ApiError::BadRequest("language must look like en-US".into()));
    }
    let region = body.region.map(|r| r.trim().to_uppercase()).filter(|r| !r.is_empty());
    if region.as_deref().is_some_and(|r| r.len() != 2 || !r.bytes().all(|b| b.is_ascii_uppercase())) {
        return Err(ApiError::BadRequest("region must be a two-letter country code".into()));
    }
    let key = match body.tmdb_key.map(|k| k.trim().to_owned()) {
        Some(k) if !k.is_empty() => {
            let probe = Tmdb::new(k.clone(), language, region.clone());
            match probe.list(List::Trending, 1).await {
                Ok(_) => k,
                Err(koda_metadata::Error::Unauthorized) => {
                    return Err(ApiError::BadRequest("TMDB rejected this key".into()));
                }
                Err(e) => return Err(ApiError::Upstream(e.to_string())),
            }
        }
        _ => current.tmdb_key,
    };
    let next = CatalogSettings { tmdb_key: key, language: language.to_owned(), region };
    next.save(&state.db).await?;
    state.set_tmdb(next.client());
    Ok(Json(CatalogSettingsView {
        tmdb_key_set: !next.tmdb_key.is_empty(),
        language: next.language,
        region: next.region,
    }))
}

fn valid_language(s: &str) -> bool {
    let b = s.as_bytes();
    matches!(b.len(), 2 | 5)
        && b[..2].iter().all(u8::is_ascii_lowercase)
        && (b.len() == 2 || (b[2] == b'-' && b[3..].iter().all(u8::is_ascii_uppercase)))
}

#[utoipa::path(get, path = "/api/v1/settings/requests", tag = "settings", responses((status = 200, body = Quota)))]
pub async fn get_requests(user: CurrentUser, State(state): State<AppState>) -> ApiResult<Json<Quota>> {
    user.require(Permissions::ADMIN)?;
    Ok(Json(
        koda_db::settings::get(&state.db, "requests.quota").await?.unwrap_or(Quota { limit: None, days: 7 }),
    ))
}

#[utoipa::path(put, path = "/api/v1/settings/requests", tag = "settings", request_body = Quota,
    responses((status = 200, body = Quota)))]
pub async fn put_requests(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(q): Json<Quota>,
) -> ApiResult<Json<Quota>> {
    user.require(Permissions::ADMIN)?;
    if q.days < 1 || q.days > 365 || q.limit.is_some_and(|l| l < 0) {
        return Err(ApiError::BadRequest("days must be 1-365 and the limit not negative".into()));
    }
    koda_db::settings::set(&state.db, "requests.quota", &q).await?;
    Ok(Json(q))
}

#[cfg(test)]
mod tests {
    #[test]
    fn language_codes() {
        assert!(super::valid_language("en-US"));
        assert!(super::valid_language("ro"));
        assert!(!super::valid_language("EN-us"));
        assert!(!super::valid_language("english"));
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LibrarySettingsBody {
    /// Folder films are imported into, as the server sees it.
    pub movies_path: String,
}

#[utoipa::path(get, path = "/api/v1/settings/library", tag = "settings",
    responses((status = 200, body = LibrarySettingsBody)))]
pub async fn get_library(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<LibrarySettingsBody>> {
    user.require(Permissions::ADMIN)?;
    let lib: crate::pipeline::LibrarySettings =
        koda_db::settings::get(&state.db, crate::pipeline::LibrarySettings::KEY).await?.unwrap_or_default();
    Ok(Json(LibrarySettingsBody { movies_path: lib.movies_path }))
}

/// Saves the library folder after checking the server can write to it.
#[utoipa::path(put, path = "/api/v1/settings/library", tag = "settings", request_body = LibrarySettingsBody,
    responses((status = 200, body = LibrarySettingsBody), (status = 400, description = "folder not usable")))]
pub async fn put_library(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(body): Json<LibrarySettingsBody>,
) -> ApiResult<Json<LibrarySettingsBody>> {
    user.require(Permissions::ADMIN)?;
    let path = body.movies_path.trim().to_owned();
    let dir = std::path::PathBuf::from(&path);
    if path.is_empty() || !dir.is_absolute() {
        return Err(ApiError::BadRequest("use a full folder path, like /data/movies".into()));
    }
    let probe = dir.join(".koda-write-test");
    let writable = tokio::fs::create_dir_all(&dir).await.is_ok()
        && tokio::fs::write(&probe, b"ok").await.is_ok()
        && tokio::fs::remove_file(&probe).await.is_ok();
    if !writable {
        return Err(ApiError::BadRequest(format!("the server cannot write to {path}")));
    }
    let lib = crate::pipeline::LibrarySettings { movies_path: path.clone() };
    koda_db::settings::set(&state.db, crate::pipeline::LibrarySettings::KEY, &lib).await?;
    state.wake.notify_one();
    Ok(Json(LibrarySettingsBody { movies_path: path }))
}
