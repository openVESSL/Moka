//! Compatibility layer: the `/api/v3` film-manager API that indexer managers
//! and request apps already speak, mapped onto Koda. Authenticated with a
//! Koda API key (`X-Api-Key` header or `apikey` query parameter) of an admin.

use std::collections::HashMap;

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::{Json, Router, middleware, routing::get, routing::post};
use koda_core::Permissions;
use koda_db::connectors::IndexerInput;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Version reported to clients; they refuse servers older than 4.x.
pub const COMPAT_VERSION: &str = "5.26.2.10099";

/// Stands in for stored secrets; sent back unchanged means "keep it".
const MASK: &str = "********";

pub fn router() -> Router<AppState> {
    let v3 = Router::new()
        .route("/system/status", get(status))
        .route("/indexer", get(list_indexers).post(create_indexer))
        .route("/indexer/schema", get(indexer_schema))
        .route("/indexer/test", post(test_indexer))
        .route("/indexer/{id}", get(one_indexer).put(update_indexer).delete(delete_indexer))
        .fallback(|| async { ApiError::NotFound("endpoint") })
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-application-version"),
            HeaderValue::from_static(COMPAT_VERSION),
        ))
        .layer(middleware::map_request(key_from_query));
    Router::new().nest("/api/v3", v3)
}

/// Some clients send the key as `?apikey=`: treat it like the header.
async fn key_from_query(mut req: Request) -> Request {
    if !req.headers().contains_key("x-api-key")
        && let Ok(Query(q)) = Query::<HashMap<String, String>>::try_from_uri(req.uri())
        && let Some(key) = q.iter().find(|(k, _)| k.eq_ignore_ascii_case("apikey")).map(|(_, v)| v)
        && let Ok(v) = HeaderValue::from_str(key)
    {
        req.headers_mut().insert("x-api-key", v);
    }
    req
}

fn admin(user: &CurrentUser) -> ApiResult<()> {
    user.require(Permissions::ADMIN)
}

async fn status(user: CurrentUser) -> ApiResult<Json<Value>> {
    admin(&user)?;
    Ok(Json(json!({
        "appName": "Radarr",
        "instanceName": "Koda",
        "version": COMPAT_VERSION,
        "urlBase": "",
        "isProduction": true,
        "authentication": "external",
        "branch": "master",
    })))
}

#[derive(Debug, Serialize, Deserialize)]
struct Field {
    name: String,
    #[serde(default)]
    value: Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexerResource {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    implementation: String,
    #[serde(default)]
    implementation_name: String,
    #[serde(default)]
    config_contract: String,
    #[serde(default)]
    protocol: String,
    #[serde(default = "yes")]
    enable_rss: bool,
    #[serde(default = "yes")]
    enable_automatic_search: bool,
    #[serde(default = "yes")]
    enable_interactive_search: bool,
    #[serde(default = "yes")]
    supports_rss: bool,
    #[serde(default = "yes")]
    supports_search: bool,
    #[serde(default = "default_priority")]
    priority: i64,
    #[serde(default)]
    download_client_id: i64,
    #[serde(default)]
    tags: Vec<i64>,
    #[serde(default)]
    fields: Vec<Field>,
}

fn yes() -> bool {
    true
}

fn default_priority() -> i64 {
    25
}

fn field(name: &str, value: Value) -> Field {
    Field { name: name.into(), value }
}

/// Empty resource for one kind; `fields` lists every setting clients fill in.
fn blank(kind: &str) -> IndexerResource {
    let torrent = kind == "torznab";
    let (implementation, protocol) = if torrent { ("Torznab", "torrent") } else { ("Newznab", "usenet") };
    let mut fields = vec![
        field("baseUrl", json!("")),
        field("apiPath", json!("/api")),
        field("apiKey", json!("")),
        field("categories", json!([2000])),
    ];
    if torrent {
        fields.push(field("minimumSeeders", json!(1)));
        fields.push(field("seedCriteria.seedRatio", Value::Null));
        fields.push(field("seedCriteria.seedTime", Value::Null));
    }
    IndexerResource {
        id: 0,
        name: String::new(),
        implementation: implementation.into(),
        implementation_name: implementation.into(),
        config_contract: format!("{implementation}Settings"),
        protocol: protocol.into(),
        enable_rss: true,
        enable_automatic_search: true,
        enable_interactive_search: true,
        supports_rss: true,
        supports_search: true,
        priority: default_priority(),
        download_client_id: 0,
        tags: vec![],
        fields,
    }
}

impl From<koda_db::connectors::Indexer> for IndexerResource {
    fn from(i: koda_db::connectors::Indexer) -> Self {
        let mut r = blank(&i.kind);
        let categories: Vec<u32> = serde_json::from_str(&i.categories).unwrap_or_default();
        for f in &mut r.fields {
            f.value = match f.name.as_str() {
                "baseUrl" => json!(i.base_url),
                "apiPath" => json!(i.api_path),
                "apiKey" if i.api_key.as_deref().is_some_and(|k| !k.is_empty()) => json!(MASK),
                "categories" => json!(categories),
                _ => continue,
            };
        }
        r.id = i.id;
        r.name = i.name;
        r.priority = i.priority;
        r.enable_rss = i.enabled;
        r.enable_automatic_search = i.enabled;
        r.enable_interactive_search = i.enabled;
        r
    }
}

impl IndexerResource {
    fn value(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|f| f.name == name).map(|f| &f.value)
    }

    fn text(&self, name: &str) -> Option<String> {
        self.value(name).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
    }

    fn into_input(self) -> ApiResult<IndexerInput> {
        let kind = self.implementation.to_ascii_lowercase();
        if !matches!(kind.as_str(), "torznab" | "newznab") {
            return Err(ApiError::BadRequest("only Torznab and Newznab indexers are supported".into()));
        }
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ApiError::BadRequest("name is required".into()));
        }
        let base_url =
            self.text("baseUrl").ok_or_else(|| ApiError::BadRequest("baseUrl is required".into()))?;
        super::connectors::check_url(&base_url)?;
        let categories: Vec<u32> = self
            .value("categories")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .filter(|c: &Vec<u32>| !c.is_empty())
            .unwrap_or_else(|| vec![2000]);
        Ok(IndexerInput {
            name: name.to_owned(),
            kind,
            // kept as sent (trailing slash included): clients match on it
            base_url,
            api_path: self.text("apiPath").unwrap_or_else(|| "/api".into()),
            api_key: self.text("apiKey").filter(|k| k != MASK),
            categories,
            priority: self.priority.clamp(1, 50),
            enabled: self.enable_automatic_search || self.enable_interactive_search || self.enable_rss,
        })
    }
}

async fn list_indexers(
    user: CurrentUser,
    State(s): State<AppState>,
) -> ApiResult<Json<Vec<IndexerResource>>> {
    admin(&user)?;
    Ok(Json(koda_db::connectors::indexers(&s.db).await?.into_iter().map(Into::into).collect()))
}

async fn one_indexer(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<IndexerResource>> {
    admin(&user)?;
    let i = koda_db::connectors::indexer(&s.db, id).await?.ok_or(ApiError::NotFound("indexer"))?;
    Ok(Json(i.into()))
}

async fn indexer_schema(user: CurrentUser) -> ApiResult<Json<Vec<IndexerResource>>> {
    admin(&user)?;
    Ok(Json(vec![blank("newznab"), blank("torznab")]))
}

/// Accepts any well-formed indexer: clients test with placeholder ids that
/// only resolve once the indexer is saved.
async fn test_indexer(user: CurrentUser, Json(r): Json<IndexerResource>) -> ApiResult<Json<Value>> {
    admin(&user)?;
    r.into_input()?;
    Ok(Json(json!({})))
}

async fn create_indexer(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(r): Json<IndexerResource>,
) -> ApiResult<(StatusCode, Json<IndexerResource>)> {
    admin(&user)?;
    let id = koda_db::connectors::create_indexer(&s.db, &r.into_input()?).await?;
    let i = koda_db::connectors::indexer(&s.db, id).await?.ok_or(ApiError::NotFound("indexer"))?;
    Ok((StatusCode::CREATED, Json(i.into())))
}

async fn update_indexer(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(r): Json<IndexerResource>,
) -> ApiResult<(StatusCode, Json<IndexerResource>)> {
    admin(&user)?;
    let old = koda_db::connectors::indexer(&s.db, id).await?.ok_or(ApiError::NotFound("indexer"))?;
    let new = r.into_input()?;
    // clients re-send unchanged indexers on every sync: skip those
    let same = new.name == old.name
        && new.kind == old.kind
        && new.base_url == old.base_url
        && new.api_path == old.api_path
        && serde_json::to_string(&new.categories).ok().as_deref() == Some(old.categories.as_str())
        && new.priority == old.priority
        && new.enabled == old.enabled
        && (new.api_key.is_none() || new.api_key == old.api_key);
    if !same {
        koda_db::connectors::update_indexer(&s.db, id, &new).await?;
        koda_db::connectors::set_capabilities(&s.db, id, "").await?;
        koda_db::connectors::indexer_ok(&s.db, id).await?;
    }
    let i = koda_db::connectors::indexer(&s.db, id).await?.ok_or(ApiError::NotFound("indexer"))?;
    Ok((StatusCode::ACCEPTED, Json(i.into())))
}

async fn delete_indexer(user: CurrentUser, State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<()> {
    admin(&user)?;
    koda_db::connectors::indexer(&s.db, id).await?.ok_or(ApiError::NotFound("indexer"))?;
    koda_db::connectors::delete_indexer(&s.db, id).await?;
    Ok(())
}
