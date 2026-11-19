//! Outgoing webhooks: list, add, remove, send a test.

use axum::extract::{Path, State};
use axum::{Json, Router, routing::get, routing::post};
use koda_core::Permissions;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::connectors::TestResult;
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::notify::{Event, Payload};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/webhooks", get(list).post(create))
        .route("/webhooks/{id}", axum::routing::delete(remove))
        .route("/webhooks/{id}/test", post(test))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Webhook {
    pub id: i64,
    pub name: String,
    /// Scheme and host only; the full address may contain a token.
    pub url: String,
    pub events: Vec<String>,
    pub enabled: bool,
}

impl From<koda_db::webhooks::Webhook> for Webhook {
    fn from(w: koda_db::webhooks::Webhook) -> Self {
        Self {
            id: w.id,
            events: serde_json::from_str(&w.events).unwrap_or_default(),
            name: w.name,
            url: mask(&w.url),
            enabled: w.enabled,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewWebhook {
    pub name: String,
    pub url: String,
    /// Any of: requested, approved, declined, grabbed, available, failed. Defaults to all.
    pub events: Option<Vec<String>>,
}

#[utoipa::path(get, path = "/api/v1/webhooks", operation_id = "list_webhooks", tag = "webhooks",
    responses((status = 200, body = [Webhook])))]
pub async fn list(user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<Vec<Webhook>>> {
    user.require(Permissions::ADMIN)?;
    Ok(Json(koda_db::webhooks::list(&s.db).await?.into_iter().map(Webhook::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/webhooks", operation_id = "create_webhook", tag = "webhooks",
    request_body = NewWebhook, responses((status = 200, body = Webhook)))]
pub async fn create(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(b): Json<NewWebhook>,
) -> ApiResult<Json<Webhook>> {
    user.require(Permissions::ADMIN)?;
    let url = b.url.trim();
    if !matches!(reqwest::Url::parse(url).map(|u| u.scheme().to_owned()).as_deref(), Ok("http" | "https")) {
        return Err(ApiError::BadRequest("address must start with http:// or https://".into()));
    }
    let events = b.events.unwrap_or_else(|| Event::ALL.iter().map(|e| (*e).to_owned()).collect());
    if let Some(bad) = events.iter().find(|e| !Event::ALL.contains(&e.as_str())) {
        return Err(ApiError::BadRequest(format!("unknown event {bad}")));
    }
    let name = Some(b.name.trim()).filter(|n| !n.is_empty()).unwrap_or("Webhook");
    let id = koda_db::webhooks::create(&s.db, name, url, &events).await?;
    let w = koda_db::webhooks::get(&s.db, id).await?.ok_or(ApiError::NotFound("webhook"))?;
    Ok(Json(w.into()))
}

#[utoipa::path(delete, path = "/api/v1/webhooks/{id}", operation_id = "delete_webhook", tag = "webhooks",
    params(("id" = i64, Path)), responses((status = 200)))]
pub async fn remove(user: CurrentUser, State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<()> {
    user.require(Permissions::ADMIN)?;
    koda_db::webhooks::delete(&s.db, id).await?;
    Ok(())
}

#[utoipa::path(post, path = "/api/v1/webhooks/{id}/test", operation_id = "test_webhook", tag = "webhooks",
    params(("id" = i64, Path)), responses((status = 200, body = TestResult)))]
pub async fn test(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<TestResult>> {
    user.require(Permissions::ADMIN)?;
    let w = koda_db::webhooks::get(&s.db, id).await?.ok_or(ApiError::NotFound("webhook"))?;
    let payload = Payload {
        event: Event::Test,
        occurred_at: 0,
        message: "Test notification".into(),
        title: None,
        user: Some(user.0.username.clone()),
    };
    Ok(Json(match crate::notify::deliver(&s.http, &w.url, &payload).await {
        Ok(()) => TestResult { ok: true, message: "Delivered.".into() },
        Err(e) => TestResult { ok: false, message: e },
    }))
}

/// `https://discord.com/api/webhooks/123/token` → `https://discord.com/…`
fn mask(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => format!("{}://{}/…", u.scheme(), u.host_str().unwrap_or("?")),
        Err(_) => "…".into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn addresses_are_masked() {
        assert_eq!(super::mask("https://discord.com/api/webhooks/1/SECRET"), "https://discord.com/…");
    }
}
