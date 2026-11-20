//! Playback decision and HLS delivery.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get, routing::post};
use moka_core::Permissions;
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
        .route("/files/{id}/subtitles", get(subtitles))
        .route("/files/{id}/subtitles/{track}", get(subtitle_track))
        .route("/files/{id}/fonts", get(fonts))
        .route("/files/{id}/fonts/{n}", get(font))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PlayRequest {
    pub caps: ClientCaps,
    /// Where to start, in seconds from the beginning of the film.
    pub start_seconds: Option<f64>,
    /// Image subtitle track to draw into the picture (`e<N>` from the
    /// subtitle list). Forces conversion.
    pub burn_subtitle: Option<String>,
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

pub(crate) async fn probe_cached(s: &AppState, file_id: i64, path: &std::path::Path) -> ApiResult<Probe> {
    if let Some(p) = moka_db::library::probe_json(&s.db, file_id)
        .await?
        .and_then(|j| serde_json::from_str::<Probe>(&j).ok())
        .filter(|p| p.duration_seconds > 0.0 && p.fonts.is_some())
    {
        return Ok(p);
    }
    let p = crate::streaming::probe(path).await.map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if let Ok(j) = serde_json::to_string(&p) {
        moka_db::library::set_probe(&s.db, file_id, &j, p.duration_seconds).await?;
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
    let f = moka_db::library::file(&s.db, id).await?.ok_or(ApiError::NotFound("file"))?;
    let path = std::path::PathBuf::from(&f.path);
    if !path.exists() {
        return Err(ApiError::NotFound("file on disk"));
    }
    let probe = probe_cached(&s, id, &path).await?;
    let mut decision = crate::streaming::decide(&probe, &req.caps);
    if let Some(track) = req.burn_subtitle.as_deref() {
        let n = track
            .strip_prefix('e')
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| *n < probe.subtitles.len())
            .ok_or(ApiError::NotFound("subtitle track"))?;
        decision = decision.burn(n);
    }
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
    let hw: crate::streaming::HwAccel =
        moka_db::settings::get(&s.db, crate::streaming::HwAccel::KEY).await?.unwrap_or_default();
    let dir = s.config.transcode_dir();
    let session = match s.streams.start(&dir, &path, &decision, start, max_height, &hw).await {
        Ok(id) => id,
        // a graphics card that is missing or busy should not stop playback
        Err(e) if hw.encoder != crate::streaming::Encoder::Software && !decision.copy_video => {
            tracing::warn!(error = %e, "hardware conversion failed, using the processor");
            s.streams
                .start(&dir, &path, &decision, start, max_height, &Default::default())
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?
        }
        Err(e) => return Err(ApiError::BadRequest(e.to_string())),
    };
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

async fn media_path(s: &AppState, id: i64) -> ApiResult<(moka_db::library::MediaFile, std::path::PathBuf)> {
    let f = moka_db::library::file(&s.db, id).await?.ok_or(ApiError::NotFound("file"))?;
    let path = std::path::PathBuf::from(&f.path);
    if !path.exists() {
        return Err(ApiError::NotFound("file on disk"));
    }
    Ok((f, path))
}

/// Subtitle tracks inside the file and subtitle files next to it.
#[utoipa::path(get, path = "/api/v1/files/{id}/subtitles", tag = "play", params(("id" = i64, Path)),
    responses((status = 200, body = [crate::subtitles::Track])))]
pub async fn subtitles(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<crate::subtitles::Track>>> {
    user.require(Permissions::PLAY)?;
    let (_, path) = media_path(&s, id).await?;
    let probe = probe_cached(&s, id, &path).await?;
    Ok(Json(crate::subtitles::tracks(&path, &probe)))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SubtitleQuery {
    /// Where the stream starts, in seconds; WebVTT cues are moved to match.
    pub offset: Option<f64>,
    /// `vtt` (default) or `ass` for styled tracks, sent with their styling.
    pub format: Option<String>,
}

/// One subtitle track as WebVTT, or as ASS for styled tracks.
#[utoipa::path(get, path = "/api/v1/files/{id}/subtitles/{track}", tag = "play",
    params(("id" = i64, Path), ("track" = String, Path), SubtitleQuery),
    responses((status = 200, content_type = "text/vtt", body = String)))]
pub async fn subtitle_track(
    user: CurrentUser,
    State(s): State<AppState>,
    Path((id, track)): Path<(i64, String)>,
    axum::extract::Query(q): axum::extract::Query<SubtitleQuery>,
) -> ApiResult<Response> {
    user.require(Permissions::PLAY)?;
    let (f, path) = media_path(&s, id).await?;
    let track = track.trim_end_matches(".vtt").trim_end_matches(".ass");
    let ass = q.format.as_deref() == Some("ass");
    let probe = probe_cached(&s, id, &path).await?;
    let info = crate::subtitles::tracks(&path, &probe)
        .into_iter()
        .find(|t| t.id == track)
        .ok_or(ApiError::NotFound("subtitle track"))?;
    if !info.text {
        return Err(ApiError::BadRequest(format!(
            "{} subtitles are images: they are drawn into the picture instead",
            info.codec
        )));
    }
    if ass && !info.styled {
        return Err(ApiError::BadRequest("this track has no styling; ask for WebVTT".into()));
    }
    let n: usize = track.get(1..).and_then(|n| n.parse().ok()).unwrap_or(0);
    let fail = |e: anyhow::Error| ApiError::BadRequest(e.to_string());
    let text = if info.external {
        let files = crate::subtitles::external(&path);
        let (file, _) = files.get(n).ok_or(ApiError::NotFound("subtitle track"))?;
        if ass {
            crate::subtitles::to_ass(file, None).await
        } else {
            crate::subtitles::to_vtt(file, None).await
        }
        .map_err(fail)?
    } else {
        // reading a track means reading the whole film: keep the result
        let ext = if ass { "ass" } else { "vtt" };
        let cache =
            s.config.cache_dir().join("subtitles").join(format!("{id}-{}-{track}.{ext}", f.size_bytes));
        match tokio::fs::read_to_string(&cache).await {
            Ok(v) => v,
            Err(_) => {
                let v = if ass {
                    crate::subtitles::to_ass(&path, Some(n)).await
                } else {
                    crate::subtitles::to_vtt(&path, Some(n)).await
                }
                .map_err(fail)?;
                if let Some(dir) = cache.parent() {
                    let _ = tokio::fs::create_dir_all(dir).await;
                }
                let _ = tokio::fs::write(&cache, &v).await;
                v
            }
        }
    };
    let (body, mime) = if ass {
        // the player shifts styled subtitles itself
        (text, "text/x-ssa; charset=utf-8")
    } else {
        (crate::subtitles::shift_vtt(&text, q.offset.unwrap_or(0.0)), "text/vtt; charset=utf-8")
    };
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache")], body).into_response())
}

/// Fonts attached to the file, for styled subtitles.
#[utoipa::path(get, path = "/api/v1/files/{id}/fonts", tag = "play", params(("id" = i64, Path)),
    responses((status = 200, body = [crate::streaming::Font])))]
pub async fn fonts(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<crate::streaming::Font>>> {
    user.require(Permissions::PLAY)?;
    let (_, path) = media_path(&s, id).await?;
    Ok(Json(probe_cached(&s, id, &path).await?.fonts.unwrap_or_default()))
}

/// One attached font file (`n` from the font list).
#[utoipa::path(get, path = "/api/v1/files/{id}/fonts/{n}", tag = "play",
    params(("id" = i64, Path), ("n" = usize, Path)),
    responses((status = 200, content_type = "font/ttf", body = Vec<u8>)))]
pub async fn font(
    user: CurrentUser,
    State(s): State<AppState>,
    Path((id, n)): Path<(i64, usize)>,
) -> ApiResult<Response> {
    user.require(Permissions::PLAY)?;
    let (f, path) = media_path(&s, id).await?;
    let probe = probe_cached(&s, id, &path).await?;
    let font = probe
        .fonts
        .unwrap_or_default()
        .into_iter()
        .find(|x| x.index == n)
        .ok_or(ApiError::NotFound("font"))?;
    // reading an attachment means reading the whole film: keep the result
    let cache = s.config.cache_dir().join("fonts").join(format!("{id}-{}-{n}.font", f.size_bytes));
    let bytes = match tokio::fs::read(&cache).await {
        Ok(b) => b,
        Err(_) => {
            let b = crate::subtitles::attachment(&path, n)
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            if let Some(dir) = cache.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            let _ = tokio::fs::write(&cache, &b).await;
            b
        }
    };
    let ext = font.name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "otf" => "font/otf",
        "ttc" => "font/collection",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "font/ttf",
    };
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "private, max-age=86400")], bytes)
        .into_response())
}
