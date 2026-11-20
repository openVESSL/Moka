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
        .route("/settings/jellyfin", get(get_jellyfin).put(put_jellyfin))
        .route("/settings/radarr", get(get_radarr).put(put_radarr))
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

/// Jellyfin connection as the UI sees it. The key is never returned.
#[derive(Debug, Serialize, ToSchema)]
pub struct JellyfinView {
    pub enabled: bool,
    pub url: String,
    pub api_key_set: bool,
    pub path_from: String,
    pub path_to: String,
    pub sync_watched: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct JellyfinUpdate {
    pub enabled: bool,
    pub url: String,
    /// Omit to keep the stored key.
    pub api_key: Option<String>,
    #[serde(default)]
    pub path_from: String,
    #[serde(default)]
    pub path_to: String,
    #[serde(default = "default_true")]
    pub sync_watched: bool,
}

fn default_true() -> bool {
    true
}

fn jellyfin_view(s: crate::jellyfin::JellyfinSettings) -> JellyfinView {
    JellyfinView {
        enabled: s.enabled,
        api_key_set: !s.api_key.is_empty(),
        url: s.url,
        path_from: s.path_from,
        path_to: s.path_to,
        sync_watched: s.sync_watched,
    }
}

#[utoipa::path(get, path = "/api/v1/settings/jellyfin", tag = "settings",
    responses((status = 200, body = JellyfinView)))]
pub async fn get_jellyfin(user: CurrentUser, State(state): State<AppState>) -> ApiResult<Json<JellyfinView>> {
    user.require(Permissions::ADMIN)?;
    Ok(Json(jellyfin_view(crate::jellyfin::JellyfinSettings::load(&state.db).await?)))
}

/// Saves the Jellyfin connection after checking it answers with the given key.
#[utoipa::path(put, path = "/api/v1/settings/jellyfin", tag = "settings", request_body = JellyfinUpdate,
    responses((status = 200, body = JellyfinView), (status = 400, description = "cannot connect")))]
pub async fn put_jellyfin(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(b): Json<JellyfinUpdate>,
) -> ApiResult<Json<JellyfinView>> {
    user.require(Permissions::ADMIN)?;
    let current = crate::jellyfin::JellyfinSettings::load(&state.db).await?;
    let url = b.url.trim().trim_end_matches('/').to_owned();
    if !url.is_empty() {
        match reqwest::Url::parse(&url) {
            Ok(u) if matches!(u.scheme(), "http" | "https") => {
                koda_net::check_url(&u).map_err(|e| ApiError::BadRequest(e.to_string()))?
            }
            _ => return Err(ApiError::BadRequest("address must start with http:// or https://".into())),
        }
    }
    let next = crate::jellyfin::JellyfinSettings {
        enabled: b.enabled,
        url,
        api_key: b.api_key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()).unwrap_or(current.api_key),
        path_from: b.path_from.trim().to_owned(),
        path_to: b.path_to.trim().to_owned(),
        sync_watched: b.sync_watched,
    };
    if next.enabled {
        crate::jellyfin::Client::new(&state.http, &next)
            .info()
            .await
            .map_err(|e| ApiError::BadRequest(format!("could not connect to Jellyfin: {e}")))?;
    }
    next.save(&state.db).await?;
    Ok(Json(jellyfin_view(next)))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RadarrProfile {
    pub id: i64,
    pub name: String,
}

/// Radarr connection as the UI sees it, with Radarr's profiles and folders
/// to choose from once connected. The key is never returned.
#[derive(Debug, Serialize, ToSchema)]
pub struct RadarrView {
    pub enabled: bool,
    pub url: String,
    pub api_key_set: bool,
    pub quality_profile_id: i64,
    pub root_folder: String,
    pub path_from: String,
    pub path_to: String,
    pub profiles: Vec<RadarrProfile>,
    pub root_folders: Vec<String>,
    /// Why Radarr could not be reached, when it could not.
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RadarrUpdate {
    pub enabled: bool,
    pub url: String,
    /// Omit to keep the stored key.
    pub api_key: Option<String>,
    #[serde(default)]
    pub quality_profile_id: i64,
    #[serde(default)]
    pub root_folder: String,
    #[serde(default)]
    pub path_from: String,
    #[serde(default)]
    pub path_to: String,
}

async fn radarr_view(state: &AppState, s: crate::radarr::RadarrSettings) -> RadarrView {
    let mut v = RadarrView {
        enabled: s.enabled,
        url: s.url.clone(),
        api_key_set: !s.api_key.is_empty(),
        quality_profile_id: s.quality_profile_id,
        root_folder: s.root_folder.clone(),
        path_from: s.path_from.clone(),
        path_to: s.path_to.clone(),
        profiles: vec![],
        root_folders: vec![],
        error: None,
    };
    if s.usable() {
        let c = crate::radarr::Client::new(&state.http, &s);
        match tokio::try_join!(c.profiles(), c.root_folders()) {
            Ok((p, r)) => {
                v.profiles = p.into_iter().map(|(id, name)| RadarrProfile { id, name }).collect();
                v.root_folders = r;
            }
            Err(e) => v.error = Some(e.to_string()),
        }
    }
    v
}

#[utoipa::path(get, path = "/api/v1/settings/radarr", tag = "settings",
    responses((status = 200, body = RadarrView)))]
pub async fn get_radarr(user: CurrentUser, State(state): State<AppState>) -> ApiResult<Json<RadarrView>> {
    user.require(Permissions::ADMIN)?;
    let s = crate::radarr::RadarrSettings::load(&state.db).await?;
    Ok(Json(radarr_view(&state, s).await))
}

/// Saves the Radarr connection after checking it answers. While enabled,
/// approved films go to Radarr instead of Koda's own search.
#[utoipa::path(put, path = "/api/v1/settings/radarr", tag = "settings", request_body = RadarrUpdate,
    responses((status = 200, body = RadarrView), (status = 400, description = "cannot connect")))]
pub async fn put_radarr(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(b): Json<RadarrUpdate>,
) -> ApiResult<Json<RadarrView>> {
    user.require(Permissions::ADMIN)?;
    let current = crate::radarr::RadarrSettings::load(&state.db).await?;
    let url = b.url.trim().trim_end_matches('/').to_owned();
    if !url.is_empty() {
        match reqwest::Url::parse(&url) {
            Ok(u) if matches!(u.scheme(), "http" | "https") => {
                koda_net::check_url(&u).map_err(|e| ApiError::BadRequest(e.to_string()))?
            }
            _ => return Err(ApiError::BadRequest("address must start with http:// or https://".into())),
        }
    }
    let next = crate::radarr::RadarrSettings {
        enabled: b.enabled,
        url,
        api_key: b.api_key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()).unwrap_or(current.api_key),
        quality_profile_id: b.quality_profile_id.max(0),
        root_folder: b.root_folder.trim().to_owned(),
        path_from: b.path_from.trim().to_owned(),
        path_to: b.path_to.trim().to_owned(),
    };
    if next.enabled {
        crate::radarr::Client::new(&state.http, &next)
            .version()
            .await
            .map_err(|e| ApiError::BadRequest(format!("could not connect to Radarr: {e}")))?;
    }
    next.save(&state.db).await?;
    if next.usable() {
        let s = state.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::radarr::import_library(&s).await {
                tracing::warn!(error = %e, "Radarr library sync failed");
            }
        });
        state.wake.notify_one();
    }
    Ok(Json(radarr_view(&state, next).await))
}
