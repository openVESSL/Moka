//! Requests: ask for a film, approve or decline.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::{Json, Router, routing::get, routing::post};
use koda_core::{Permissions, TitleState};
use koda_db::requests::{RequestRow, RequestStatus};
use koda_db::titles::NewTitle;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, LiveEvent};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/requests", get(list).post(create))
        .route("/requests/{id}/approve", post(approve))
        .route("/requests/{id}/decline", post(decline))
}

/// Per-user request limit, stored under the `requests.quota` setting.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
pub struct Quota {
    /// Maximum films per period. Missing or zero means unlimited.
    pub limit: Option<i64>,
    pub days: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewRequest {
    pub tmdb_id: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Request {
    pub id: i64,
    pub title_id: i64,
    pub tmdb_id: Option<i64>,
    pub title: String,
    pub year: Option<i64>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    /// pending | approved | declined
    pub status: String,
    /// Lifecycle state of the film itself.
    pub title_state: String,
    pub requested_by: String,
    pub note: Option<String>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

impl From<RequestRow> for Request {
    fn from(r: RequestRow) -> Self {
        Self {
            id: r.id,
            title_id: r.title_id,
            tmdb_id: r.tmdb_id,
            title: r.title,
            year: r.year,
            poster_path: r.poster_path,
            backdrop_path: r.backdrop_path,
            status: r.status,
            title_state: r.title_state,
            requested_by: r.username,
            note: r.note,
            created_at: r.created_at,
            decided_at: r.decided_at,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListQuery {
    /// pending | approved | declined
    pub status: Option<String>,
    /// Only my own requests. Users without request management only ever see their own.
    pub mine: Option<bool>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct Decision {
    pub note: Option<String>,
}

fn parse_status(s: &str) -> ApiResult<RequestStatus> {
    Ok(match s {
        "pending" => RequestStatus::Pending,
        "approved" => RequestStatus::Approved,
        "declined" => RequestStatus::Declined,
        _ => return Err(ApiError::BadRequest("status must be pending, approved or declined".into())),
    })
}

#[utoipa::path(get, path = "/api/v1/requests", operation_id = "list_requests", tag = "requests", params(ListQuery),
    responses((status = 200, body = [Request])))]
pub async fn list(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Request>>> {
    let status = q.status.as_deref().map(parse_status).transpose()?;
    let manager = user.0.permissions().has(Permissions::MANAGE_REQUESTS);
    let only = (!manager || q.mine.unwrap_or(false)).then_some(user.0.id);
    let rows = koda_db::requests::list(&state.db, status, only).await?;
    Ok(Json(rows.into_iter().map(Request::from).collect()))
}

/// Requests a film. Creates the title if it is new; if someone already asked
/// for it, this adds the user to that title instead of failing.
#[utoipa::path(post, path = "/api/v1/requests", tag = "requests", request_body = NewRequest,
    responses((status = 200, body = Request), (status = 403), (status = 429, description = "quota reached")))]
pub async fn create(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(body): Json<NewRequest>,
) -> ApiResult<Json<Request>> {
    user.require(Permissions::REQUEST)?;
    let perms = user.0.permissions();
    let db = &state.db;

    if !perms.has(Permissions::MANAGE_REQUESTS) {
        let quota: Quota = koda_db::settings::get(db, "requests.quota").await?.unwrap_or_default();
        if let Some(limit) = quota.limit.filter(|l| *l > 0) {
            let used = koda_db::requests::count_recent_for_user(db, user.0.id, quota.days.max(1)).await?;
            if used >= limit {
                return Err(ApiError::TooManyRequests(format!(
                    "request limit reached: {limit} films every {} days",
                    quota.days.max(1)
                )));
            }
        }
    }

    let tmdb = state.tmdb();
    let d = state
        .cached(format!("movie:{}", body.tmdb_id), Duration::from_secs(6 * 3600), || tmdb.movie(body.tmdb_id))
        .await?;
    let auto = perms.has(Permissions::AUTO_APPROVE);
    let initial = if auto { TitleState::Wanted } else { TitleState::Requested };
    let new = NewTitle {
        tmdb_id: d.id,
        imdb_id: d.imdb().map(str::to_owned),
        title: d.title.clone(),
        original_title: d.original_title.clone(),
        year: d.year(),
        overview: d.overview.clone(),
        runtime_minutes: d.runtime.filter(|r| *r > 0),
        release_date: d.release_date.clone(),
        poster_path: d.poster_path.clone(),
        backdrop_path: d.backdrop_path.clone(),
        metadata_json: serde_json::to_string(&d).ok(),
    };
    let mut title = koda_db::titles::upsert_movie(db, &new, initial).await?;

    // An auto-approved request moves a title someone else left pending.
    if auto && title.state() == TitleState::Requested {
        title = koda_db::titles::transition(db, title.id, TitleState::Wanted, None)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        koda_db::titles::schedule_search(db, title.id, 0, None).await?;
    }

    let status = if auto { RequestStatus::Approved } else { RequestStatus::Pending };
    let row = koda_db::requests::create(db, title.id, user.0.id, status).await?;
    state.publish(LiveEvent::RequestCreated { request_id: row.id, title_id: title.id });
    state.wake.notify_one();
    let event = if auto { crate::notify::Event::Approved } else { crate::notify::Event::Requested };
    crate::notify::send(
        &state,
        event,
        Some(crate::notify::TitleInfo::from_title(&title, None)),
        Some(user.0.username.clone()),
    );
    if auto {
        state.publish(LiveEvent::TitleState { title_id: title.id, state: title.state(), reason: None });
    }
    Ok(Json(row.into()))
}

#[utoipa::path(post, path = "/api/v1/requests/{id}/approve", tag = "requests",
    params(("id" = i64, Path)), request_body(content = Decision, content_type = "application/json"),
    responses((status = 200, body = Request)))]
pub async fn approve(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    body: Option<Json<Decision>>,
) -> ApiResult<Json<Request>> {
    user.require(Permissions::MANAGE_REQUESTS)?;
    let db = &state.db;
    let req = koda_db::requests::by_id(db, id).await?.ok_or(ApiError::NotFound("request"))?;
    let note = body.and_then(|Json(d)| d.note);
    koda_db::requests::decide(db, id, RequestStatus::Approved, user.0.id, note.as_deref()).await?;

    if req.title_state == TitleState::Requested.as_str() {
        let t = koda_db::titles::transition(db, req.title_id, TitleState::Wanted, None)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        koda_db::titles::schedule_search(db, t.id, 0, None).await?;
        state.publish(LiveEvent::TitleState { title_id: t.id, state: t.state(), reason: None });
    }
    state.publish(LiveEvent::RequestDecided { request_id: id, title_id: req.title_id, approved: true });
    state.wake.notify_one();
    if let Some(t) = koda_db::titles::by_id(db, req.title_id).await? {
        let info = crate::notify::TitleInfo::from_title(&t, None);
        crate::notify::send(&state, crate::notify::Event::Approved, Some(info), Some(req.username.clone()));
    }
    let row = koda_db::requests::by_id(db, id).await?.ok_or(ApiError::NotFound("request"))?;
    Ok(Json(row.into()))
}

#[utoipa::path(post, path = "/api/v1/requests/{id}/decline", tag = "requests",
    params(("id" = i64, Path)), request_body(content = Decision, content_type = "application/json"),
    responses((status = 200, body = Request)))]
pub async fn decline(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    body: Option<Json<Decision>>,
) -> ApiResult<Json<Request>> {
    user.require(Permissions::MANAGE_REQUESTS)?;
    let db = &state.db;
    let req = koda_db::requests::by_id(db, id).await?.ok_or(ApiError::NotFound("request"))?;
    let note = body.and_then(|Json(d)| d.note);
    koda_db::requests::decide(db, id, RequestStatus::Declined, user.0.id, note.as_deref()).await?;
    state.publish(LiveEvent::RequestDecided { request_id: id, title_id: req.title_id, approved: false });
    if let Some(t) = koda_db::titles::by_id(db, req.title_id).await? {
        let info = crate::notify::TitleInfo::from_title(&t, note.clone());
        crate::notify::send(&state, crate::notify::Event::Declined, Some(info), Some(req.username.clone()));
    }
    let row = koda_db::requests::by_id(db, id).await?.ok_or(ApiError::NotFound("request"))?;

    // A title nobody wants any more and that never left "requested" is dropped.
    if req.title_state == TitleState::Requested.as_str()
        && koda_db::requests::open_count_for_title(db, req.title_id).await? == 0
    {
        koda_db::titles::delete(db, req.title_id).await?;
    }
    Ok(Json(row.into()))
}
