//! Titles in the library and in the pipeline.

use axum::extract::{Path, Query, State};
use axum::{Json, Router, routing::get};
use moka_core::TitleState;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/titles", get(list))
        .route("/titles/{id}", get(one))
        .route("/titles/{id}/episodes", get(episodes))
        .route("/titles/{id}/tracks", get(tracks))
        .route("/titles/{id}/monitored", axum::routing::put(set_monitored))
        .route("/titles/{id}/search", axum::routing::post(search_now))
        .route("/titles/{id}/releases", get(releases))
        .route("/titles/{id}/grab", axum::routing::post(grab))
        .route("/titles/{id}/files", get(files))
        .route("/files/{id}/stream", get(stream))
        .route("/files/{id}/progress", get(get_progress).put(put_progress))
}

/// One release from the last search, with the verdict on it.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReleaseView {
    /// Position in the last search; pass it to `grab`.
    pub index: usize,
    pub title: String,
    pub indexer: String,
    pub size_bytes: Option<u64>,
    pub seeders: Option<u32>,
    pub published_at: Option<i64>,
    /// Quality key, e.g. `webdl-1080p` or `flac`.
    pub quality: Option<String>,
    pub quality_label: Option<String>,
    pub accepted: bool,
    pub rejections: Vec<String>,
    /// Why it ranks above the next acceptable release.
    pub beats_next_by: Option<String>,
    pub info_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReleasesView {
    /// Unix time of the search; null if never searched since start.
    pub searched_at: Option<i64>,
    pub releases: Vec<ReleaseView>,
    pub indexer_errors: Vec<String>,
}

fn view(r: Option<crate::pipeline::SearchResult>) -> ReleasesView {
    let Some(r) = r else {
        return ReleasesView { searched_at: None, releases: vec![], indexer_errors: vec![] };
    };
    ReleasesView {
        searched_at: Some(r.at),
        indexer_errors: r.indexer_errors.iter().map(|(n, e)| format!("{n}: {e}")).collect(),
        releases: r
            .evaluated
            .iter()
            .map(|e| {
                let c = &r.candidates[e.candidate_index];
                ReleaseView {
                    index: e.candidate_index,
                    title: e.title.clone(),
                    indexer: c.indexer_name.clone(),
                    size_bytes: c.release.size_bytes,
                    seeders: c.release.seeders,
                    published_at: c.release.published_at,
                    quality: e.quality_key(),
                    quality_label: e.quality_label(),
                    accepted: e.accepted(),
                    rejections: e.rejections.iter().map(|x| x.message.clone()).collect(),
                    beats_next_by: e.beats_next_by.map(str::to_owned),
                    info_url: c.release.info_url.clone(),
                }
            })
            .collect(),
    }
}

/// Searches indexers for this film now and returns every release with its verdict.
/// Nothing is grabbed; use `grab` to pick one, or let the background search pick.
#[utoipa::path(post, path = "/api/v1/titles/{id}/search", tag = "titles", params(("id" = i64, Path)),
    responses((status = 200, body = ReleasesView)))]
pub async fn search_now(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<ReleasesView>> {
    user.require(moka_core::Permissions::MANAGE_LIBRARY)?;
    let t = moka_db::titles::by_id(&state.db, id).await?.ok_or(ApiError::NotFound("title"))?;
    let result =
        crate::pipeline::search(&state, &t).await.map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(view(Some(result))))
}

#[utoipa::path(get, path = "/api/v1/titles/{id}/releases", tag = "titles", params(("id" = i64, Path)),
    responses((status = 200, body = ReleasesView)))]
pub async fn releases(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Json<ReleasesView> {
    Json(view(state.searches.lock().expect("searches lock").get(&id).cloned()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GrabBody {
    /// `index` from the release list.
    pub index: usize,
}

/// Sends one release from the last search to the download client, even if it was rejected.
#[utoipa::path(post, path = "/api/v1/titles/{id}/grab", tag = "titles", params(("id" = i64, Path)),
    request_body = GrabBody, responses((status = 200, body = Title)))]
pub async fn grab(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<GrabBody>,
) -> ApiResult<Json<Title>> {
    user.require(moka_core::Permissions::MANAGE_LIBRARY)?;
    let result = state.searches.lock().expect("searches lock").get(&id).cloned();
    let result = result.ok_or_else(|| ApiError::BadRequest("search first, then pick a release".into()))?;
    let cand = result.candidates.get(body.index).ok_or(ApiError::NotFound("release"))?.clone();
    let quality =
        result.evaluated.iter().find(|e| e.candidate_index == body.index).and_then(|e| e.quality_key());
    crate::pipeline::grab(&state, id, &cand, quality)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    one(user, State(state), Path(id)).await
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MediaFile {
    pub id: i64,
    pub path: String,
    pub size_bytes: i64,
    pub quality: Option<String>,
    pub quality_label: Option<String>,
    pub release_group: Option<String>,
    pub added_at: i64,
}

#[utoipa::path(get, path = "/api/v1/titles/{id}/files", tag = "titles", params(("id" = i64, Path)),
    responses((status = 200, body = [MediaFile])))]
pub async fn files(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<MediaFile>>> {
    let rows = moka_db::library::files_for_title(&state.db, id).await?;
    Ok(Json(
        rows.into_iter()
            .map(|f| MediaFile {
                quality_label: f
                    .quality
                    .as_deref()
                    .and_then(moka_core::Quality::from_key)
                    .map(|q| q.to_string()),
                id: f.id,
                path: f.path,
                size_bytes: f.size_bytes,
                quality: f.quality,
                release_group: f.release_group,
                added_at: f.added_at,
            })
            .collect(),
    ))
}

/// The original file over HTTP with range requests (direct play).
pub async fn stream(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    req: axum::extract::Request,
) -> ApiResult<axum::response::Response> {
    use tower::ServiceExt;
    user.require(moka_core::Permissions::PLAY)?;
    let f = moka_db::library::file(&state.db, id).await?.ok_or(ApiError::NotFound("file"))?;
    let res = tower_http::services::ServeFile::new(&f.path)
        .oneshot(req)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serve file: {e}")))?;
    Ok(res.map(axum::body::Body::new))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Title {
    pub id: i64,
    /// movie | show | season | episode | artist | album | track
    pub kind: String,
    /// Season of an episode, show of a season.
    pub parent_id: Option<i64>,
    /// MusicBrainz id, for music.
    pub mbid: Option<String>,
    pub season_number: Option<i64>,
    pub episode_number: Option<i64>,
    /// Searched for automatically (specials are not, by default).
    pub monitored: bool,
    /// First air date for shows and episodes.
    pub release_date: Option<String>,
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i64>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    /// requested | wanted | searching | grabbed | downloading | importing | available
    pub state: String,
    /// Plain-language reason for the current state, when there is one.
    pub state_reason: Option<String>,
    pub next_search_at: Option<i64>,
    pub last_search_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<moka_db::titles::Title> for Title {
    fn from(t: moka_db::titles::Title) -> Self {
        Self {
            id: t.id,
            kind: t.kind,
            parent_id: t.parent_id,
            mbid: t.mbid,
            season_number: t.season_number,
            episode_number: t.episode_number,
            monitored: t.monitored,
            release_date: t.release_date,
            tmdb_id: t.tmdb_id,
            imdb_id: t.imdb_id,
            title: t.title,
            year: t.year,
            overview: t.overview,
            runtime_minutes: t.runtime_minutes,
            poster_path: t.poster_path,
            backdrop_path: t.backdrop_path,
            state: t.state,
            state_reason: t.state_reason,
            next_search_at: t.next_search_at,
            last_search_at: t.last_search_at,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListQuery {
    pub state: Option<String>,
    /// `movie` (default) or `show`.
    pub kind: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/titles", operation_id = "list_titles", tag = "titles", params(ListQuery),
    responses((status = 200, body = [Title])))]
pub async fn list(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Title>>> {
    let filter = q
        .state
        .as_deref()
        .map(|s| s.parse::<TitleState>().map_err(|e| ApiError::BadRequest(e.to_string())))
        .transpose()?;
    let rows = match q.kind.as_deref() {
        None | Some("movie") => moka_db::titles::list(&state.db, filter).await?,
        Some("show") => {
            let progress: std::collections::HashMap<i64, moka_db::titles::ShowProgress> =
                moka_db::titles::show_progress(&state.db)
                    .await?
                    .into_iter()
                    .map(|p| (p.show_id, p))
                    .collect();
            moka_db::titles::shows(&state.db)
                .await?
                .into_iter()
                .map(|mut t| {
                    if let Some(p) = progress.get(&t.id) {
                        t.state = p.state(t.state()).as_str().to_owned();
                    }
                    t
                })
                .filter(|t| filter.is_none_or(|f| t.state() == f))
                .collect()
        }
        Some("album") => moka_db::titles::albums(&state.db)
            .await?
            .into_iter()
            .filter(|t| filter.is_none_or(|f| t.state() == f))
            .collect(),
        Some(_) => return Err(ApiError::BadRequest("kind must be movie, show or album".into())),
    };
    Ok(Json(rows.into_iter().map(Title::from).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MonitoredBody {
    pub monitored: bool,
}

/// Follows or stops following an episode, or all episodes of a season.
/// Followed episodes that are wanted get searched.
#[utoipa::path(put, path = "/api/v1/titles/{id}/monitored", tag = "titles", params(("id" = i64, Path)),
    request_body = MonitoredBody, responses((status = 200, body = Title), (status = 404)))]
pub async fn set_monitored(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<MonitoredBody>,
) -> ApiResult<Json<Title>> {
    user.require(moka_core::Permissions::MANAGE_LIBRARY)?;
    let t = moka_db::titles::by_id(&state.db, id).await?.ok_or(ApiError::NotFound("title"))?;
    if t.kind != "episode" && t.kind != "season" {
        return Err(ApiError::BadRequest("only episodes and seasons can be followed".into()));
    }
    moka_db::titles::set_monitored(&state.db, id, body.monitored).await?;
    state.wake.notify_one();
    one(user, State(state), Path(id)).await
}

/// Every episode of a show, by season and number, or every track of an album.
#[utoipa::path(get, path = "/api/v1/titles/{id}/episodes", tag = "titles", params(("id" = i64, Path)),
    responses((status = 200, body = [Title]), (status = 404)))]
pub async fn episodes(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<Title>>> {
    let show = moka_db::titles::by_id(&state.db, id).await?.ok_or(ApiError::NotFound("title"))?;
    let rows = match show.kind.as_str() {
        "show" => moka_db::titles::episodes_of_show(&state.db, id).await?,
        "album" => moka_db::titles::children(&state.db, id).await?,
        _ => return Err(ApiError::BadRequest("only shows and albums have episodes or tracks".into())),
    };
    Ok(Json(rows.into_iter().map(Title::from).collect()))
}

/// A track of an album, with the file to play when it is in the library.
#[derive(Debug, Serialize, ToSchema)]
pub struct AlbumTrack {
    pub title_id: i64,
    pub disc: Option<i64>,
    /// Position across the album, from 1.
    pub number: Option<i64>,
    pub title: String,
    pub state: String,
    pub file_id: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/titles/{id}/tracks", tag = "titles", params(("id" = i64, Path)),
    responses((status = 200, body = [AlbumTrack]), (status = 404)))]
pub async fn tracks(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<AlbumTrack>>> {
    let album = moka_db::titles::by_id(&state.db, id).await?.ok_or(ApiError::NotFound("title"))?;
    if album.kind != "album" {
        return Err(ApiError::BadRequest("only albums have tracks".into()));
    }
    let rows = moka_db::library::album_tracks(&state.db, id).await?;
    Ok(Json(
        rows.into_iter()
            .map(|t| AlbumTrack {
                title_id: t.title_id,
                disc: t.disc,
                number: t.number,
                title: t.title,
                state: t.state,
                file_id: t.file_id,
            })
            .collect(),
    ))
}

#[utoipa::path(get, path = "/api/v1/titles/{id}", tag = "titles", params(("id" = i64, Path)),
    responses((status = 200, body = Title), (status = 404)))]
pub async fn one(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Title>> {
    moka_db::titles::by_id(&state.db, id).await?.map(|t| Json(t.into())).ok_or(ApiError::NotFound("title"))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Progress {
    pub position_seconds: f64,
    pub played: bool,
}

#[utoipa::path(get, path = "/api/v1/files/{id}/progress", tag = "titles", params(("id" = i64, Path)),
    responses((status = 200, body = Progress)))]
pub async fn get_progress(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Progress>> {
    let f = moka_db::library::file(&state.db, id).await?.ok_or(ApiError::NotFound("file"))?;
    let (position_seconds, played) = moka_db::library::progress(&state.db, user.0.id, f.title_id).await?;
    Ok(Json(Progress { position_seconds, played }))
}

/// Saves the signed-in user's position in the film this file belongs to.
#[utoipa::path(put, path = "/api/v1/files/{id}/progress", tag = "titles", params(("id" = i64, Path)),
    request_body = Progress, responses((status = 200, body = Progress)))]
pub async fn put_progress(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(p): Json<Progress>,
) -> ApiResult<Json<Progress>> {
    if !p.position_seconds.is_finite() {
        return Err(ApiError::BadRequest("position must be a number of seconds".into()));
    }
    let f = moka_db::library::file(&state.db, id).await?.ok_or(ApiError::NotFound("file"))?;
    moka_db::library::set_progress(&state.db, user.0.id, f.title_id, p.position_seconds, p.played).await?;
    Ok(Json(p))
}
