//! Film catalog from TMDB, annotated with what is already in the library.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::{Json, Router, routing::get};
use koda_core::{TitleKind, TitleState};
use koda_metadata::{List, MovieSummary};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/catalog/discover/{list}", get(discover))
        .route("/catalog/search", get(search))
        .route("/catalog/movie/{tmdb_id}", get(movie))
}

/// Where a catalog film stands locally, if it is known at all.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LocalState {
    pub title_id: i64,
    #[schema(value_type = String, example = "wanted")]
    pub state: TitleState,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogItem {
    pub tmdb_id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: f64,
    pub local: Option<LocalState>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogPage {
    pub page: i64,
    pub total_pages: i64,
    pub results: Vec<CatalogItem>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PageQuery {
    pub page: Option<u32>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    pub q: String,
    pub page: Option<u32>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Person {
    pub id: i64,
    pub name: String,
    pub role: Option<String>,
    pub profile_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogMovie {
    pub tmdb_id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub tagline: Option<String>,
    pub overview: Option<String>,
    pub year: Option<i64>,
    pub release_date: Option<String>,
    pub runtime_minutes: Option<i64>,
    pub genres: Vec<String>,
    pub directors: Vec<String>,
    pub cast: Vec<Person>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: f64,
    pub trailer_youtube_key: Option<String>,
    pub recommendations: Vec<CatalogItem>,
    pub local: Option<LocalState>,
}

fn parse_list(s: &str) -> Option<List> {
    Some(match s {
        "trending" => List::Trending,
        "popular" => List::Popular,
        "upcoming" => List::Upcoming,
        "now_playing" => List::NowPlaying,
        "top_rated" => List::TopRated,
        _ => return None,
    })
}

#[utoipa::path(get, path = "/api/v1/catalog/discover/{list}", tag = "catalog",
    params(("list" = String, Path, description = "trending | popular | upcoming | now_playing | top_rated"), PageQuery),
    responses((status = 200, body = CatalogPage), (status = 409, description = "TMDB key missing")))]
pub async fn discover(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(list): Path<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<CatalogPage>> {
    let kind = parse_list(&list).ok_or(ApiError::NotFound("list"))?;
    let page = q.page.unwrap_or(1).min(500);
    let tmdb = state.tmdb();
    let result = state
        .cached(format!("list:{list}:{page}"), Duration::from_secs(3600), || tmdb.list(kind, page))
        .await?;
    Ok(Json(annotate(&state, result.page, result.total_pages, result.results).await?))
}

#[utoipa::path(get, path = "/api/v1/catalog/search", tag = "catalog", params(SearchQuery),
    responses((status = 200, body = CatalogPage)))]
pub async fn search(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<CatalogPage>> {
    let query = q.q.trim();
    if query.is_empty() {
        return Ok(Json(CatalogPage { page: 1, total_pages: 0, results: vec![] }));
    }
    if query.len() > 200 {
        return Err(ApiError::BadRequest("search text is too long".into()));
    }
    let page = q.page.unwrap_or(1).min(500);
    let tmdb = state.tmdb();
    let key = format!("search:{}:{page}", query.to_lowercase());
    let result = state
        .cached(key, Duration::from_secs(600), || tmdb.search(query, page))
        .await?;
    Ok(Json(annotate(&state, result.page, result.total_pages, result.results).await?))
}

#[utoipa::path(get, path = "/api/v1/catalog/movie/{tmdb_id}", tag = "catalog",
    params(("tmdb_id" = i64, Path)),
    responses((status = 200, body = CatalogMovie), (status = 404)))]
pub async fn movie(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(tmdb_id): Path<i64>,
) -> ApiResult<Json<CatalogMovie>> {
    let tmdb = state.tmdb();
    let d = state
        .cached(format!("movie:{tmdb_id}"), Duration::from_secs(6 * 3600), || tmdb.movie(tmdb_id))
        .await?;
    let recs = state
        .cached(format!("recs:{tmdb_id}"), Duration::from_secs(6 * 3600), || {
            tmdb.recommendations(tmdb_id)
        })
        .await
        .map(|p| p.results)
        .unwrap_or_default();
    let recommendations = annotate(&state, 1, 1, recs.into_iter().take(18).collect()).await?.results;
    let local = local_states(&state, &[tmdb_id]).await?.remove(&tmdb_id);

    let mut cast = d.credits.cast.clone();
    cast.sort_by_key(|c| c.order);
    Ok(Json(CatalogMovie {
        tmdb_id: d.id,
        imdb_id: d.imdb().map(str::to_owned),
        year: d.year(),
        directors: d.directors().into_iter().map(str::to_owned).collect(),
        trailer_youtube_key: d.trailer_key().map(str::to_owned),
        genres: d.genres.iter().map(|g| g.name.clone()).collect(),
        cast: cast
            .into_iter()
            .take(12)
            .map(|c| Person { id: c.id, name: c.name, role: c.character, profile_path: c.profile_path })
            .collect(),
        title: d.title,
        original_title: d.original_title,
        tagline: d.tagline.filter(|t| !t.is_empty()),
        overview: d.overview,
        release_date: d.release_date,
        runtime_minutes: d.runtime.filter(|r| *r > 0),
        poster_path: d.poster_path,
        backdrop_path: d.backdrop_path,
        vote_average: d.vote_average,
        recommendations,
        local,
    }))
}

async fn local_states(state: &AppState, ids: &[i64]) -> ApiResult<HashMap<i64, LocalState>> {
    let rows = koda_db::titles::states_for_tmdb(&state.db, TitleKind::Movie, ids).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(tmdb_id, title_id, s)| {
            Some((tmdb_id, LocalState { title_id, state: s.parse().ok()? }))
        })
        .collect())
}

async fn annotate(
    state: &AppState,
    page: i64,
    total_pages: i64,
    results: Vec<MovieSummary>,
) -> ApiResult<CatalogPage> {
    let ids: Vec<i64> = results.iter().map(|m| m.id).collect();
    let mut local = local_states(state, &ids).await?;
    let results = results
        .into_iter()
        .map(|m| CatalogItem {
            tmdb_id: m.id,
            year: m.year(),
            local: local.remove(&m.id),
            title: m.title,
            overview: m.overview,
            poster_path: m.poster_path,
            backdrop_path: m.backdrop_path,
            vote_average: m.vote_average,
        })
        .collect();
    Ok(CatalogPage { page, total_pages: total_pages.min(500), results })
}
