//! Library scan: add films and shows that are already on disk.

use axum::extract::State;
use axum::{Json, Router, routing::get};
use moka_core::Permissions;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::scan::ScanStatus;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/library/scan", get(status).post(start))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ScanRequest {
    /// Folder to scan. Defaults to the library's films or shows folder.
    pub path: Option<String>,
    /// `movie` (default) or `show`. Episodes of a new show that aired but
    /// are not on disk are not searched; new episodes are.
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
    let shows = body.kind == super::requests::RequestKind::Show;
    let path = match body.path.filter(|p| !p.trim().is_empty()) {
        Some(p) => p.trim().to_owned(),
        None => {
            let lib: crate::pipeline::LibrarySettings =
                moka_db::settings::get(&s.db, crate::pipeline::LibrarySettings::KEY)
                    .await?
                    .unwrap_or_default();
            if shows { lib.shows_path } else { lib.movies_path }
        }
    };
    let dir = std::path::PathBuf::from(&path);
    if path.is_empty() || !dir.is_dir() {
        return Err(ApiError::BadRequest("choose an existing folder to scan".into()));
    }
    if !crate::scan::start(&s, dir, shows) {
        return Err(ApiError::Conflict("a scan is already running".into()));
    }
    Ok(Json(s.scan.lock().expect("scan lock").clone()))
}
