//! Playback decision and HLS delivery.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get, routing::post};
use koda_core::Permissions;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::streaming::{ClientCaps, Method, Probe};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/files/{id}/play", post(play))
        .route("/hls/{session}/{name}", get(hls_file))
        .route("/hls/{session}", axum::routing::delete(stop))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PlayRequest {
    pub caps: ClientCaps,
    /// Where to start, in seconds from the beginning of the film.
    pub start_seconds: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlayResponse {
    pub method: Method,
    /// Plain file for direct play, an HLS playlist otherwise.
    pub url: String,
    /// The stream begins at this point of the film (0 for direct play).
    pub start_seconds: f64,
    pub duration_seconds: f64,
    /// Why the file is not played as is.
    pub reasons: Vec<String>,
    /// Session to stop when done (remux/transcode only).
    pub session: Option<String>,
}

async fn probe_cached(s: &AppState, file_id: i64, path: &std::path::Path) -> ApiResult<Probe> {
    if let Some(p) = koda_db::library::probe_json(&s.db, file_id)
        .await?
        .and_then(|j| serde_json::from_str::<Probe>(&j).ok())
        .filter(|p| p.duration_seconds > 0.0)
    {
        return Ok(p);
    }
    let p = crate::streaming::probe(path).await.map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if let Ok(j) = serde_json::to_string(&p) {
        koda_db::library::set_probe(&s.db, file_id, &j, p.duration_seconds).await?;
    }
    Ok(p)
}

/// Decides how this browser plays the file and, if needed, starts converting it.
#[utoipa::path(post, path = "/api/v1/files/{id}/play", tag = "play", params(("id" = i64, Path)),
    request_body = PlayRequest, responses((status = 200, body = PlayResponse)))]
pub async fn play(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<PlayRequest>,
) -> ApiResult<Json<PlayResponse>> {
    user.require(Permissions::PLAY)?;
    let f = koda_db::library::file(&s.db, id).await?.ok_or(ApiError::NotFound("file"))?;
    let path = std::path::PathBuf::from(&f.path);
    if !path.exists() {
        return Err(ApiError::NotFound("file on disk"));
    }
    let probe = probe_cached(&s, id, &path).await?;
    let decision = crate::streaming::decide(&probe, &req.caps);
    let start = req.start_seconds.unwrap_or(0.0).clamp(0.0, (probe.duration_seconds - 5.0).max(0.0));

    if decision.method == Method::Direct {
        return Ok(Json(PlayResponse {
            method: Method::Direct,
            url: format!("/api/v1/files/{id}/stream"),
            start_seconds: 0.0,
            duration_seconds: probe.duration_seconds,
            reasons: vec![],
            session: None,
        }));
    }
    let max_height = req.caps.max_height.unwrap_or(1080).clamp(360, 2160);
    let session = s
        .streams
        .start(&s.config.transcode_dir(), &path, &decision, start, max_height)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(PlayResponse {
        method: decision.method,
        url: format!("/api/v1/hls/{session}/index.m3u8"),
        start_seconds: start,
        duration_seconds: probe.duration_seconds,
        reasons: decision.reasons,
        session: Some(session),
    }))
}

pub async fn hls_file(
    _user: CurrentUser,
    State(s): State<AppState>,
    Path((session, name)): Path<(String, String)>,
) -> ApiResult<Response> {
    if !crate::streaming::valid_hls_name(&name) {
        return Err(ApiError::NotFound("segment"));
    }
    let dir = s.streams.touch(&session).ok_or(ApiError::NotFound("session"))?;
    let bytes = tokio::fs::read(dir.join(&name)).await.map_err(|_| ApiError::NotFound("segment"))?;
    let (mime, cache) = match name.rsplit('.').next() {
        Some("m3u8") => ("application/vnd.apple.mpegurl", "no-cache"),
        Some("mp4") => ("video/mp4", "private, max-age=3600"),
        _ => ("video/iso.segment", "private, max-age=3600"),
    };
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)], bytes).into_response())
}

/// Stops a remux/transcode session and deletes its files.
#[utoipa::path(delete, path = "/api/v1/hls/{session}", operation_id = "stop_stream", tag = "play",
    params(("session" = String, Path)), responses((status = 200)))]
pub async fn stop(
    _user: CurrentUser,
    State(s): State<AppState>,
    Path(session): Path<String>,
) -> ApiResult<()> {
    s.streams.stop(&session).await;
    Ok(())
}
