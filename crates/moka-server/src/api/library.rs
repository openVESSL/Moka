//! Library scan: add films, shows and albums that are already on disk.

use axum::extract::State;
use axum::{Json, Router, routing::get};
use moka_core::Permissions;
use serde::Deserialize;
use utoipa::ToSchema;

use super::requests::RequestKind;
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::scan::ScanStatus;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/library/scan", get(status).post(start))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ScanRequest {
    /// Folder to scan. Defaults to the library's films, shows or music folder.
    pub path: Option<String>,
    /// `movie` (default), `show` or `album`. Episodes of a new show that
    /// aired but are not on disk are not searched; new episodes are. Albums
    /// are read as `Artist/Album (Year)`.
    #[serde(default)]
    pub kind: super::requests::RequestKind,
}

#[utoipa::path(get, path = "/api/v1/library/scan", operation_id = "scan_status", tag = "library", responses((status = 200, body = ScanStatus)))]
pub async fn status(user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<ScanStatus>> {
    user.require(Permissions::MANAGE_LIBRARY)?;
    Ok(Json(s.scan.lock().expect("scan lock").clone()))
}

/// Starts scanning in the background; poll the GET for progress.
#[utoipa::path(post, path = "/api/v1/library/scan", operation_id = "start_scan", tag = "library", request_body = ScanRequest,
    responses((status = 200, body = ScanStatus), (status = 409, description = "already running")))]
pub async fn start(
    user: CurrentUser,
    State(s): State<AppState>,
    body: Option<Json<ScanRequest>>,
) -> ApiResult<Json<ScanStatus>> {
    user.require(Permissions::MANAGE_LIBRARY)?;
    let body = body.map(|Json(b)| b).unwrap_or_default();
    if matches!(body.kind, RequestKind::Ebook | RequestKind::Audiobook) {
        return Err(ApiError::BadRequest("books cannot be scanned yet".into()));
    }
    let path = match body.path.filter(|p| !p.trim().is_empty()) {
        Some(p) => p.trim().to_owned(),
        None => {
            let lib: crate::pipeline::LibrarySettings =
                moka_db::settings::get(&s.db, crate::pipeline::LibrarySettings::KEY)
                    .await?
                    .unwrap_or_default();
            match body.kind {
                RequestKind::Movie => lib.movies_path,
                RequestKind::Show => lib.shows_path,
                RequestKind::Album => lib.music_path,
                RequestKind::Ebook | RequestKind::Audiobook => String::new(),
            }
        }
    };
    let dir = std::path::PathBuf::from(&path);
    if path.is_empty() || !dir.is_dir() {
        return Err(ApiError::BadRequest("choose an existing folder to scan".into()));
    }
    if !crate::scan::start(&s, dir, body.kind) {
        return Err(ApiError::Conflict("a scan is already running".into()));
    }
    Ok(Json(s.scan.lock().expect("scan lock").clone()))
}
