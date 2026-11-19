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
    Router::new().route("/titles", get(list)).route("/titles/{id}", get(one))
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
