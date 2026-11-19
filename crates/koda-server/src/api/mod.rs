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
pub mod catalog;
pub mod connectors;
pub mod images;
pub mod library;
pub mod profiles;
pub mod requests;
pub mod settings;
pub mod titles;
pub mod users;
pub mod webhooks;

#[derive(OpenApi)]
#[openapi(
    info(title = "Koda API", version = "1"),
    paths(
        health,
        auth::status,
        auth::setup,
        auth::login,
        auth::logout,
        auth::me,
        catalog::discover,
        catalog::search,
        catalog::movie,
        requests::list,
        requests::create,
        requests::approve,
        requests::decline,
        titles::list,
        titles::one,
        titles::search_now,
        titles::releases,
        titles::grab,
        titles::files,
        titles::get_progress,
        titles::put_progress,
        connectors::list_indexers,
        connectors::create_indexer,
        connectors::update_indexer,
        connectors::delete_indexer,
        connectors::test_indexer,
        connectors::list_clients,
        connectors::create_client,
        connectors::update_client,
        connectors::delete_client,
        connectors::test_client,
        settings::get_catalog,
        settings::put_catalog,
        settings::get_requests,
        settings::put_requests,
        users::list,
        users::create,
        users::update,
        users::remove,
        users::change_own_password,
        library::status,
        library::start,
        profiles::qualities,
        profiles::get_profile,
        profiles::put_profile,
        webhooks::list,
        webhooks::create,
        webhooks::remove,
        webhooks::test,
        settings::get_library,
        settings::put_library,
    ),
    components(schemas(Health, crate::error::ErrorBody))
)]
pub struct ApiDoc;

pub fn router() -> Router<AppState> {
    let api = Router::new()
        .route("/health", get(health))
        .route("/events", get(events))
        .route("/openapi.json", get(|| async { Json(ApiDoc::openapi()) }))
        .merge(auth::router())
        .merge(catalog::router())
        .merge(requests::router())
        .merge(titles::router())
        .merge(connectors::router())
        .merge(users::router())
        .merge(webhooks::router())
        .merge(profiles::router())
        .merge(library::router())
        .merge(settings::router())
        .merge(images::router())
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

#[cfg(test)]
mod tests {
    use utoipa::OpenApi;

    /// Generated clients key methods by operation id; duplicates silently mix up types.
    #[test]
    fn operation_ids_are_unique() {
        let doc = serde_json::to_value(super::ApiDoc::openapi()).unwrap();
        let mut ids: Vec<String> = Vec::new();
        for (_, item) in doc["paths"].as_object().unwrap() {
            for (_, op) in item.as_object().unwrap() {
                if let Some(id) = op.get("operationId").and_then(|v| v.as_str()) {
                    assert!(!ids.contains(&id.to_owned()), "duplicate operationId {id}");
                    ids.push(id.to_owned());
                }
            }
        }
        assert!(ids.len() > 20);
    }
}
