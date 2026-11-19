//! Native HTTP API under `/api/v1`.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{Json, Router, routing::get};
use serde::Serialize;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use utoipa::{OpenApi, ToSchema};

use crate::auth::CurrentUser;
use crate::error::ApiError;
use crate::state::AppState;

pub mod auth;

#[derive(OpenApi)]
#[openapi(
    info(title = "Koda API", version = "1"),
    paths(health, auth::status, auth::setup, auth::login, auth::logout, auth::me),
    components(schemas(Health, ApiErrorBody, auth::AuthStatus, auth::Credentials, auth::Me))
)]
pub struct ApiDoc;

type ApiErrorBody = crate::error::ErrorBody;

pub fn router() -> Router<AppState> {
    let api = Router::new()
        .route("/health", get(health))
        .route("/events", get(events))
        .route("/openapi.json", get(|| async { Json(ApiDoc::openapi()) }))
        .merge(auth::router())
        .fallback(|| async { ApiError::NotFound("endpoint") });
    Router::new().nest("/api/v1", api)
}

#[derive(Serialize, ToSchema)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

#[utoipa::path(get, path = "/api/v1/health", tag = "system", responses((status = 200, body = Health)))]
pub async fn health() -> Json<Health> {
    Json(Health { status: "ok", version: env!("CARGO_PKG_VERSION") })
}

/// Live updates for the signed-in user (Server-Sent Events).
pub async fn events(
    _user: CurrentUser,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(|msg| {
        // a lagging client just misses some progress ticks
        let event = msg.ok()?;
        Event::default().json_data(&event).ok().map(Ok)
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}
