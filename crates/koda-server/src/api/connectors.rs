//! Indexers and download clients: add, edit, test, remove.

use axum::extract::{Path, State};
use axum::{Json, Router, routing::get, routing::post};
use koda_core::Permissions;
use koda_db::connectors::{DownloadClientInput, IndexerInput};
use koda_download::{ClientConfig, Kind};
use koda_indexers::{IndexerConfig, IndexerKind};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/indexers", get(list_indexers).post(create_indexer))
        .route("/indexers/test", post(test_indexer))
        .route("/indexers/import/prowlarr", post(import_prowlarr))
        .route("/indexers/{id}", axum::routing::put(update_indexer).delete(delete_indexer))
        .route("/download-clients", get(list_clients).post(create_client))
        .route("/download-clients/test", post(test_client))
        .route("/download-clients/{id}", axum::routing::put(update_client).delete(delete_client))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Indexer {
    pub id: i64,
    pub name: String,
    /// torznab | newznab
    pub kind: String,
    pub base_url: String,
    pub api_path: String,
    pub api_key_set: bool,
    pub categories: Vec<u32>,
    pub priority: i64,
    pub enabled: bool,
    /// Unix time until which the indexer is paused after errors.
    pub paused_until: Option<i64>,
    pub last_error: Option<String>,
}

impl From<koda_db::connectors::Indexer> for Indexer {
    fn from(i: koda_db::connectors::Indexer) -> Self {
        Self {
            id: i.id,
            name: i.name,
            kind: i.kind,
            base_url: i.base_url,
            api_path: i.api_path,
            api_key_set: i.api_key.as_deref().is_some_and(|k| !k.is_empty()),
            categories: serde_json::from_str(&i.categories).unwrap_or_default(),
            priority: i.priority,
            enabled: i.enabled,
            paused_until: i.disabled_until,
            last_error: i.last_error,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct IndexerBody {
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub api_path: Option<String>,
    /// Omit when editing to keep the stored key.
    pub api_key: Option<String>,
    pub categories: Option<Vec<u32>>,
    pub priority: Option<i64>,
    pub enabled: Option<bool>,
}

impl IndexerBody {
    fn validate(self) -> ApiResult<IndexerInput> {
        if !matches!(self.kind.as_str(), "torznab" | "newznab") {
            return Err(ApiError::BadRequest("kind must be torznab or newznab".into()));
        }
        let base_url = check_url(&self.base_url)?;
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ApiError::BadRequest("give the indexer a name".into()));
        }
        Ok(IndexerInput {
            name: name.to_owned(),
            kind: self.kind,
            base_url,
            api_path: self.api_path.filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "/api".into()),
            api_key: self.api_key.filter(|k| !k.trim().is_empty()).map(|k| k.trim().to_owned()),
            categories: self.categories.filter(|c| !c.is_empty()).unwrap_or_else(|| vec![2000]),
            priority: self.priority.unwrap_or(25).clamp(1, 50),
            enabled: self.enabled.unwrap_or(true),
        })
    }
}

pub(crate) fn check_url(u: &str) -> ApiResult<String> {
    let u = u.trim().trim_end_matches('/');
    match reqwest::Url::parse(u) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => {
            koda_net::check_url(&url).map_err(|e| ApiError::BadRequest(e.to_string()))?;
            Ok(u.to_owned())
        }
        _ => Err(ApiError::BadRequest("address must start with http:// or https://".into())),
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

#[utoipa::path(get, path = "/api/v1/indexers", tag = "connectors", responses((status = 200, body = [Indexer])))]
pub async fn list_indexers(user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<Vec<Indexer>>> {
    user.require(Permissions::ADMIN)?;
    Ok(Json(koda_db::connectors::indexers(&s.db).await?.into_iter().map(Indexer::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/indexers", tag = "connectors", request_body = IndexerBody,
    responses((status = 200, body = Indexer)))]
pub async fn create_indexer(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(body): Json<IndexerBody>,
) -> ApiResult<Json<Indexer>> {
    user.require(Permissions::ADMIN)?;
    let id = koda_db::connectors::create_indexer(&s.db, &body.validate()?).await?;
    one_indexer(&s, id).await
}

#[utoipa::path(put, path = "/api/v1/indexers/{id}", tag = "connectors", params(("id" = i64, Path)),
    request_body = IndexerBody, responses((status = 200, body = Indexer)))]
pub async fn update_indexer(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<IndexerBody>,
) -> ApiResult<Json<Indexer>> {
    user.require(Permissions::ADMIN)?;
    koda_db::connectors::indexer(&s.db, id).await?.ok_or(ApiError::NotFound("indexer"))?;
    koda_db::connectors::update_indexer(&s.db, id, &body.validate()?).await?;
    // settings changed: forget cached capabilities and any pause
    koda_db::connectors::set_capabilities(&s.db, id, "").await?;
    koda_db::connectors::indexer_ok(&s.db, id).await?;
    one_indexer(&s, id).await
}

#[utoipa::path(delete, path = "/api/v1/indexers/{id}", tag = "connectors", params(("id" = i64, Path)),
    responses((status = 200)))]
pub async fn delete_indexer(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<()> {
    user.require(Permissions::ADMIN)?;
    koda_db::connectors::delete_indexer(&s.db, id).await?;
    Ok(())
}

/// Checks an indexer's settings by asking for its capabilities. Does not save.
#[utoipa::path(post, path = "/api/v1/indexers/test", tag = "connectors", request_body = IndexerBody,
    responses((status = 200, body = TestResult)))]
pub async fn test_indexer(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(body): Json<IndexerBody>,
) -> ApiResult<Json<TestResult>> {
    user.require(Permissions::ADMIN)?;
    let input = body.validate()?;
    let cfg = IndexerConfig {
        kind: if input.kind == "newznab" { IndexerKind::Newznab } else { IndexerKind::Torznab },
        base_url: input.base_url,
        api_path: input.api_path,
        api_key: input.api_key,
        categories: input.categories,
    };
    Ok(Json(match s.indexers.capabilities(&cfg).await {
        Ok(c) if c.movie_search => {
            TestResult { ok: true, message: "Connected. Film search is supported.".into() }
        }
        Ok(_) => TestResult {
            ok: true,
            message: "Connected, but this indexer only supports plain text search.".into(),
        },
        Err(e) => TestResult { ok: false, message: e.to_string() },
    }))
}

async fn one_indexer(s: &AppState, id: i64) -> ApiResult<Json<Indexer>> {
    let i = koda_db::connectors::indexer(&s.db, id).await?.ok_or(ApiError::NotFound("indexer"))?;
    Ok(Json(i.into()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DownloadClient {
    pub id: i64,
    pub name: String,
    /// qbittorrent | transmission | sabnzbd
    pub kind: String,
    pub base_url: String,
    pub username: Option<String>,
    pub password_set: bool,
    pub category: String,
    pub enabled: bool,
}

impl From<koda_db::connectors::DownloadClient> for DownloadClient {
    fn from(c: koda_db::connectors::DownloadClient) -> Self {
        Self {
            id: c.id,
            name: c.name,
            kind: c.kind,
            base_url: c.base_url,
            username: c.username,
            password_set: c.password.as_deref().is_some_and(|p| !p.is_empty()),
            category: c.category,
            enabled: c.enabled,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DownloadClientBody {
    pub name: String,
    /// qbittorrent (default), transmission or sabnzbd
    pub kind: Option<String>,
    pub base_url: String,
    pub username: Option<String>,
    /// Omit when editing to keep the stored password. For SABnzbd: the API key.
    pub password: Option<String>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
}

impl DownloadClientBody {
    fn validate(self) -> ApiResult<DownloadClientInput> {
        let kind_name = self.kind.as_deref().unwrap_or("qbittorrent");
        let kind = Kind::parse(kind_name).ok_or_else(|| {
            ApiError::BadRequest("kind must be qbittorrent, transmission or sabnzbd".into())
        })?;
        let category =
            self.category.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).unwrap_or("koda".into());
        if !category.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) {
            return Err(ApiError::BadRequest("category may only use letters, digits, - _ and .".into()));
        }
        Ok(DownloadClientInput {
            name: Some(self.name.trim().to_owned()).filter(|n| !n.is_empty()).unwrap_or(match kind {
                Kind::Qbittorrent => "qBittorrent".into(),
                Kind::Transmission => "Transmission".into(),
                Kind::Sabnzbd => "SABnzbd".into(),
            }),
            kind: kind_name.to_owned(),
            base_url: check_url(&self.base_url)?,
            username: self.username.filter(|u| !u.is_empty()),
            password: self.password.filter(|p| !p.is_empty()),
            category,
            enabled: self.enabled.unwrap_or(true),
        })
    }
}

#[utoipa::path(get, path = "/api/v1/download-clients", tag = "connectors",
    responses((status = 200, body = [DownloadClient])))]
pub async fn list_clients(
    user: CurrentUser,
    State(s): State<AppState>,
) -> ApiResult<Json<Vec<DownloadClient>>> {
    user.require(Permissions::ADMIN)?;
    Ok(Json(
        koda_db::connectors::download_clients(&s.db).await?.into_iter().map(DownloadClient::from).collect(),
    ))
}

#[utoipa::path(post, path = "/api/v1/download-clients", tag = "connectors", request_body = DownloadClientBody,
    responses((status = 200, body = DownloadClient)))]
pub async fn create_client(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(body): Json<DownloadClientBody>,
) -> ApiResult<Json<DownloadClient>> {
    user.require(Permissions::ADMIN)?;
    let id = koda_db::connectors::create_download_client(&s.db, &body.validate()?).await?;
    one_client(&s, id).await
}

#[utoipa::path(put, path = "/api/v1/download-clients/{id}", tag = "connectors", params(("id" = i64, Path)),
    request_body = DownloadClientBody, responses((status = 200, body = DownloadClient)))]
pub async fn update_client(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<DownloadClientBody>,
) -> ApiResult<Json<DownloadClient>> {
    user.require(Permissions::ADMIN)?;
    koda_db::connectors::download_client(&s.db, id).await?.ok_or(ApiError::NotFound("download client"))?;
    koda_db::connectors::update_download_client(&s.db, id, &body.validate()?).await?;
    s.download_clients.lock().expect("clients lock").remove(&id);
    one_client(&s, id).await
}

#[utoipa::path(delete, path = "/api/v1/download-clients/{id}", tag = "connectors", params(("id" = i64, Path)),
    responses((status = 200)))]
pub async fn delete_client(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<()> {
    user.require(Permissions::ADMIN)?;
    koda_db::connectors::delete_download_client(&s.db, id).await?;
    s.download_clients.lock().expect("clients lock").remove(&id);
    Ok(())
}

/// Logs in to the client with these settings. Does not save.
#[utoipa::path(post, path = "/api/v1/download-clients/test", tag = "connectors", request_body = DownloadClientBody,
    responses((status = 200, body = TestResult)))]
pub async fn test_client(
    user: CurrentUser,
    Json(body): Json<DownloadClientBody>,
) -> ApiResult<Json<TestResult>> {
    user.require(Permissions::ADMIN)?;
    let c = body.validate()?;
    let kind = Kind::parse(&c.kind).unwrap_or(Kind::Qbittorrent);
    let client = koda_download::Client::new(
        kind,
        ClientConfig {
            base_url: c.base_url,
            username: c.username,
            password: c.password,
            category: c.category,
        },
    );
    Ok(Json(match client.test().await {
        Ok(v) => TestResult { ok: true, message: format!("Connected to {} {v}.", c.name) },
        Err(e) => TestResult { ok: false, message: e.to_string() },
    }))
}

async fn one_client(s: &AppState, id: i64) -> ApiResult<Json<DownloadClient>> {
    let c = koda_db::connectors::download_client(&s.db, id)
        .await?
        .ok_or(ApiError::NotFound("download client"))?;
    Ok(Json(c.into()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProwlarrImport {
    /// Prowlarr address, e.g. http://192.168.1.10:9696
    pub url: String,
    pub api_key: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProwlarrImportResult {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    /// Indexer name and why it was left out.
    pub skipped: Vec<String>,
}

/// Adds every film-capable indexer from Prowlarr (torrent and Usenet), searched through
/// Prowlarr's own Torznab endpoints. Running it again updates them instead
/// of adding duplicates.
#[utoipa::path(post, path = "/api/v1/indexers/import/prowlarr", tag = "connectors", request_body = ProwlarrImport,
    responses((status = 200, body = ProwlarrImportResult), (status = 400, description = "cannot read Prowlarr")))]
pub async fn import_prowlarr(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(b): Json<ProwlarrImport>,
) -> ApiResult<Json<ProwlarrImportResult>> {
    user.require(Permissions::ADMIN)?;
    let base = check_url(&b.url)?;
    let key = b.api_key.trim();
    if key.is_empty() {
        return Err(ApiError::BadRequest("enter the Prowlarr API key (Settings → General)".into()));
    }
    let list = s
        .indexers
        .prowlarr_indexers(&base, key)
        .await
        .map_err(|e| ApiError::BadRequest(format!("could not read Prowlarr: {e}")))?;
    let existing = koda_db::connectors::indexers(&s.db).await?;
    let mut out = ProwlarrImportResult { added: vec![], updated: vec![], skipped: vec![] };
    for p in list {
        if p.movie_categories.is_empty() {
            out.skipped.push(format!("{}: offers no film categories", p.name));
            continue;
        }
        let input = IndexerInput {
            name: format!("{} (Prowlarr)", p.name),
            kind: if p.protocol == koda_indexers::Protocol::Torrent { "torznab" } else { "newznab" }.into(),
            base_url: format!("{base}/{}", p.id),
            api_path: "/api".into(),
            api_key: Some(key.to_owned()),
            categories: p.movie_categories,
            priority: 25,
            enabled: p.enabled,
        };
        match existing.iter().find(|i| i.base_url == input.base_url) {
            Some(i) => {
                let keep = IndexerInput { priority: i.priority, ..input };
                koda_db::connectors::update_indexer(&s.db, i.id, &keep).await?;
                koda_db::connectors::set_capabilities(&s.db, i.id, "").await?;
                out.updated.push(p.name);
            }
            None => {
                koda_db::connectors::create_indexer(&s.db, &input).await?;
                out.added.push(p.name);
            }
        }
    }
    Ok(Json(out))
}
