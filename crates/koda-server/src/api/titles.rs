//! Titles in the library and in the pipeline.

use axum::extract::{Path, Query, State};
use axum::{Json, Router, routing::get};
use koda_core::TitleState;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/titles", get(list))
        .route("/titles/{id}", get(one))
        .route("/titles/{id}/search", axum::routing::post(search_now))
        .route("/titles/{id}/releases", get(releases))
        .route("/titles/{id}/grab", axum::routing::post(grab))
        .route("/titles/{id}/files", get(files))
        .route("/files/{id}/stream", get(stream))
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
    /// Quality key, e.g. `webdl-1080p`.
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
                    quality: e.quality.map(|q| q.key()),
                    quality_label: e.quality.map(|q| q.to_string()),
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
    user.require(koda_core::Permissions::MANAGE_LIBRARY)?;
    let t = koda_db::titles::by_id(&state.db, id).await?.ok_or(ApiError::NotFound("title"))?;
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
    user.require(koda_core::Permissions::MANAGE_LIBRARY)?;
    let result = state.searches.lock().expect("searches lock").get(&id).cloned();
    let result = result.ok_or_else(|| ApiError::BadRequest("search first, then pick a release".into()))?;
    let cand = result.candidates.get(body.index).ok_or(ApiError::NotFound("release"))?.clone();
    let quality = result.evaluated.iter().find(|e| e.candidate_index == body.index).and_then(|e| e.quality);
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
    let rows = koda_db::library::files_for_title(&state.db, id).await?;
    Ok(Json(
        rows.into_iter()
            .map(|f| MediaFile {
                quality_label: f
                    .quality
                    .as_deref()
                    .and_then(koda_core::Quality::from_key)
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
    user.require(koda_core::Permissions::PLAY)?;
    let f = koda_db::library::file(&state.db, id).await?.ok_or(ApiError::NotFound("file"))?;
    let res = tower_http::services::ServeFile::new(&f.path)
        .oneshot(req)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serve file: {e}")))?;
    Ok(res.map(axum::body::Body::new))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Title {
    pub id: i64,
    pub kind: String,
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

impl From<koda_db::titles::Title> for Title {
    fn from(t: koda_db::titles::Title) -> Self {
        Self {
            id: t.id,
            kind: t.kind,
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
    let rows = koda_db::titles::list(&state.db, filter).await?;
    Ok(Json(rows.into_iter().map(Title::from).collect()))
}

#[utoipa::path(get, path = "/api/v1/titles/{id}", tag = "titles", params(("id" = i64, Path)),
    responses((status = 200, body = Title), (status = 404)))]
pub async fn one(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Title>> {
    koda_db::titles::by_id(&state.db, id).await?.map(|t| Json(t.into())).ok_or(ApiError::NotFound("title"))
}
