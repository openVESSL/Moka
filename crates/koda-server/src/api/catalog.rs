//! Film and show catalog from TMDB, annotated with what is already in the library.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::{Json, Router, routing::get};
use koda_core::{TitleKind, TitleState};
use koda_metadata::{List, MovieSummary, ShowSummary};
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
        .route("/catalog/shows/discover/{list}", get(discover_shows))
        .route("/catalog/shows/search", get(search_shows))
        .route("/catalog/show/{tmdb_id}", get(show))
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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogSeason {
    pub season_number: i64,
    pub name: String,
    pub episode_count: i64,
    pub air_date: Option<String>,
    pub poster_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogShow {
    pub tmdb_id: i64,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<i64>,
    pub title: String,
    pub original_title: Option<String>,
    pub tagline: Option<String>,
    pub overview: Option<String>,
    pub year: Option<i64>,
    pub first_air_date: Option<String>,
    /// e.g. "Returning Series", "Ended".
    pub status: Option<String>,
    pub runtime_minutes: Option<i64>,
    pub genres: Vec<String>,
    pub cast: Vec<Person>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: f64,
    pub trailer_youtube_key: Option<String>,
    pub seasons: Vec<CatalogSeason>,
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
    let result = state.cached(key, Duration::from_secs(600), || tmdb.search(query, page)).await?;
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
        .cached(format!("recs:{tmdb_id}"), Duration::from_secs(6 * 3600), || tmdb.recommendations(tmdb_id))
        .await
        .map(|p| p.results)
        .unwrap_or_default();
    let recommendations = annotate(&state, 1, 1, recs.into_iter().take(18).collect()).await?.results;
    let local = local_states(&state, TitleKind::Movie, &[tmdb_id]).await?.remove(&tmdb_id);

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

#[utoipa::path(get, path = "/api/v1/catalog/shows/discover/{list}", tag = "catalog",
    params(("list" = String, Path, description = "trending | popular | now_playing | top_rated"), PageQuery),
    responses((status = 200, body = CatalogPage), (status = 409, description = "TMDB key missing")))]
pub async fn discover_shows(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(list): Path<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<CatalogPage>> {
    let kind = parse_list(&list).ok_or(ApiError::NotFound("list"))?;
    let page = q.page.unwrap_or(1).min(500);
    let tmdb = state.tmdb();
    let result = state
        .cached(format!("shows:{list}:{page}"), Duration::from_secs(3600), || tmdb.show_list(kind, page))
        .await?;
    Ok(Json(annotate_shows(&state, result.page, result.total_pages, result.results).await?))
}

#[utoipa::path(get, path = "/api/v1/catalog/shows/search", tag = "catalog", params(SearchQuery),
    responses((status = 200, body = CatalogPage)))]
pub async fn search_shows(
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
    let key = format!("search-shows:{}:{page}", query.to_lowercase());
    let result = state.cached(key, Duration::from_secs(600), || tmdb.search_shows(query, page)).await?;
    Ok(Json(annotate_shows(&state, result.page, result.total_pages, result.results).await?))
}

#[utoipa::path(get, path = "/api/v1/catalog/show/{tmdb_id}", tag = "catalog",
    params(("tmdb_id" = i64, Path)),
    responses((status = 200, body = CatalogShow), (status = 404)))]
pub async fn show(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(tmdb_id): Path<i64>,
) -> ApiResult<Json<CatalogShow>> {
    let d = show_details(&state, tmdb_id).await?;
    let local = local_states(&state, TitleKind::Show, &[tmdb_id]).await?.remove(&tmdb_id);
    let mut cast = d.credits.cast.clone();
    cast.sort_by_key(|c| c.order);
    let trailer =
        d.videos.results.iter().find(|v| v.site == "YouTube" && v.kind == "Trailer").map(|v| v.key.clone());
    Ok(Json(CatalogShow {
        tmdb_id: d.id,
        imdb_id: d.external_ids.imdb_id.clone().filter(|s| !s.is_empty()),
        tvdb_id: d.external_ids.tvdb_id,
        year: d.year(),
        runtime_minutes: d.runtime().filter(|r| *r > 0),
        genres: d.genres.iter().map(|g| g.name.clone()).collect(),
        cast: cast
            .into_iter()
            .take(12)
            .map(|c| Person { id: c.id, name: c.name, role: c.character, profile_path: c.profile_path })
            .collect(),
        seasons: d
            .seasons
            .iter()
            .map(|s| CatalogSeason {
                season_number: s.season_number,
                name: s.name.clone(),
                episode_count: s.episode_count,
                air_date: s.air_date.clone(),
                poster_path: s.poster_path.clone(),
            })
            .collect(),
        trailer_youtube_key: trailer,
        title: d.name,
        original_title: d.original_name,
        tagline: d.tagline.filter(|t| !t.is_empty()),
        overview: d.overview,
        first_air_date: d.first_air_date,
        status: d.status,
        poster_path: d.poster_path,
        backdrop_path: d.backdrop_path,
        vote_average: d.vote_average,
        local,
    }))
}

async fn show_details(state: &AppState, tmdb_id: i64) -> ApiResult<koda_metadata::ShowDetails> {
    let tmdb = state.tmdb();
    Ok(state.cached(format!("show:{tmdb_id}"), Duration::from_secs(6 * 3600), || tmdb.show(tmdb_id)).await?)
}

/// A show with every season and episode, ready to store.
pub(crate) async fn new_show(state: &AppState, tmdb_id: i64) -> ApiResult<koda_db::titles::NewShow> {
    use koda_db::titles::{NewEpisode, NewSeason, NewShow, NewTitle};
    let d = show_details(state, tmdb_id).await?;
    let tmdb = state.tmdb();
    let mut seasons = Vec::new();
    for s in &d.seasons {
        let n = s.season_number;
        let full = state
            .cached(format!("season:{tmdb_id}:{n}"), Duration::from_secs(6 * 3600), || {
                tmdb.season(tmdb_id, n)
            })
            .await?;
        seasons.push(NewSeason {
            number: n,
            title: NewTitle {
                tmdb_id: full.id,
                title: full.name.clone(),
                overview: full.overview.clone().filter(|o| !o.is_empty()),
                release_date: full.air_date.clone(),
                year: full.air_date.as_deref().and_then(|a| a.get(..4)?.parse().ok()),
                poster_path: full.poster_path.clone(),
                ..Default::default()
            },
            episodes: full
                .episodes
                .iter()
                .map(|e| NewEpisode {
                    number: e.episode_number,
                    title: NewTitle {
                        tmdb_id: e.id,
                        title: e.name.clone(),
                        overview: e.overview.clone().filter(|o| !o.is_empty()),
                        release_date: e.air_date.clone(),
                        runtime_minutes: e.runtime.filter(|r| *r > 0),
                        backdrop_path: e.still_path.clone(),
                        ..Default::default()
                    },
                })
                .collect(),
        });
    }
    Ok(NewShow {
        tvdb_id: d.external_ids.tvdb_id,
        show: NewTitle {
            tmdb_id: d.id,
            imdb_id: d.external_ids.imdb_id.clone().filter(|s| !s.is_empty()),
            title: d.name.clone(),
            original_title: d.original_name.clone(),
            year: d.year(),
            overview: d.overview.clone(),
            runtime_minutes: d.runtime().filter(|r| *r > 0),
            release_date: d.first_air_date.clone(),
            poster_path: d.poster_path.clone(),
            backdrop_path: d.backdrop_path.clone(),
            metadata_json: Some(serde_json::json!({ "numbering": d.numbering() }).to_string()),
        },
        seasons,
    })
}

async fn local_states(state: &AppState, kind: TitleKind, ids: &[i64]) -> ApiResult<HashMap<i64, LocalState>> {
    let rows = koda_db::titles::states_for_tmdb(&state.db, kind, ids).await?;
    // a show's own row only says requested or wanted; its episodes tell the rest
    let progress: HashMap<i64, koda_db::titles::ShowProgress> = if kind == TitleKind::Show && !rows.is_empty()
    {
        koda_db::titles::show_progress(&state.db).await?.into_iter().map(|p| (p.show_id, p)).collect()
    } else {
        HashMap::new()
    };
    Ok(rows
        .into_iter()
        .filter_map(|(tmdb_id, title_id, s)| {
            let own: TitleState = s.parse().ok()?;
            let state = progress.get(&title_id).map_or(own, |p| p.state(own));
            Some((tmdb_id, LocalState { title_id, state }))
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
    let mut local = local_states(state, TitleKind::Movie, &ids).await?;
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

async fn annotate_shows(
    state: &AppState,
    page: i64,
    total_pages: i64,
    results: Vec<ShowSummary>,
) -> ApiResult<CatalogPage> {
    let ids: Vec<i64> = results.iter().map(|m| m.id).collect();
    let mut local = local_states(state, TitleKind::Show, &ids).await?;
    let results = results
        .into_iter()
        .map(|m| CatalogItem {
            tmdb_id: m.id,
            year: m.year(),
            local: local.remove(&m.id),
            title: m.name,
            overview: m.overview,
            poster_path: m.poster_path,
            backdrop_path: m.backdrop_path,
            vote_average: m.vote_average,
        })
        .collect();
    Ok(CatalogPage { page, total_pages: total_pages.min(500), results })
}
