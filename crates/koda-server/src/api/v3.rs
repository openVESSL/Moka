//! Compatibility layer: the `/api/v3` film-manager API that indexer managers
//! and request apps already speak, mapped onto Koda. Authenticated with a
//! Koda API key (`X-Api-Key` header or `apikey` query parameter) of an admin.

use std::collections::HashMap;

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::{Json, Router, middleware, routing::get, routing::post};
use koda_core::{Permissions, TitleKind, TitleState};
use koda_db::connectors::IndexerInput;
use koda_db::titles::Title;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
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
        // request apps
        .route("/qualityprofile", get(quality_profiles))
        .route("/qualityProfile", get(quality_profiles))
        .route("/rootfolder", get(root_folders))
        .route("/rootFolder", get(root_folders))
        .route("/tag", get(tags).post(create_tag))
        .route("/movie", get(list_movies).post(add_movie).put(update_movie_body))
        .route("/movie/lookup", get(lookup_movie))
        .route("/movie/{id}", get(one_movie).put(update_movie))
        .route("/command", post(command))
        .route("/queue", get(queue))
        // subtitle apps
        .route("/history", get(history))
        .fallback(|| async { ApiError::NotFound("endpoint") })
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-application-version"),
            HeaderValue::from_static(COMPAT_VERSION),
        ))
        .layer(middleware::map_request(key_from_query));
    Router::new()
        .nest("/api/v3", v3)
        // older clients ask here first for the version
        .route("/api/system/status", get(status).layer(middleware::map_request(key_from_query)))
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

// ---- request apps -------------------------------------------------------

fn date(unix: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(unix)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_default()
}

fn slug(title: &str, tmdb_id: i64) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    format!("{}-{tmdb_id}", words.join("-"))
}

/// 2160, 1080, 720, 576, 480 or 0, read from a quality key such as `bluray-1080p`.
fn resolution(quality: &str) -> u32 {
    [2160, 1080, 720, 576, 480].into_iter().find(|r| quality.contains(&r.to_string())).unwrap_or(0)
}

async fn movies_root(s: &AppState) -> ApiResult<String> {
    let lib: crate::pipeline::LibrarySettings =
        koda_db::settings::get(&s.db, crate::pipeline::LibrarySettings::KEY).await?.unwrap_or_default();
    Ok(lib.movies_path)
}

/// A Koda title as a movie resource.
async fn movie(s: &AppState, t: &Title, root: &str) -> ApiResult<Value> {
    let tmdb_id = t.tmdb_id.unwrap_or(0);
    let file = koda_db::library::files_for_title(&s.db, t.id).await?.into_iter().next();
    let folder = file
        .as_ref()
        .and_then(|f| std::path::Path::new(&f.path).parent().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| format!("{}/{}", root.trim_end_matches(['/', '\\']), t.title));
    // the release name helps subtitle apps find matching subtitles
    let scene_name = match &file {
        Some(_) => koda_db::grabs::latest_for_title(&s.db, t.id)
            .await?
            .filter(|g| g.status == "imported")
            .map(|g| g.release_title),
        None => None,
    };
    let movie_file = file.as_ref().map(|f| {
        let q = f.quality.clone().unwrap_or_default();
        json!({
            "id": f.id,
            "movieId": t.id,
            "path": f.path,
            "sceneName": scene_name,
            // shortcut: audio languages are not reported; subtitle apps read them from the file
            "languages": [],
            "relativePath": std::path::Path::new(&f.path).file_name().map(|n| n.to_string_lossy().into_owned()),
            "size": f.size_bytes,
            "dateAdded": date(f.added_at),
            "releaseGroup": f.release_group,
            "quality": { "quality": { "id": 0, "name": q, "resolution": resolution(&q) },
                         "revision": { "version": 1, "real": 0, "isRepack": false } },
        })
    });
    Ok(json!({
        "id": t.id,
        "title": t.title,
        "originalTitle": t.original_title,
        "sortTitle": t.title.to_lowercase(),
        "tmdbId": tmdb_id,
        "imdbId": t.imdb_id,
        "year": t.year.unwrap_or(0),
        "titleSlug": slug(&t.title, tmdb_id),
        "overview": t.overview,
        "runtime": t.runtime_minutes.unwrap_or(0),
        "status": "released",
        "minimumAvailability": "released",
        "monitored": t.monitored && t.state() != TitleState::Requested,
        "isAvailable": true,
        "hasFile": movie_file.is_some(),
        "movieFile": movie_file,
        "sizeOnDisk": file.as_ref().map_or(0, |f| f.size_bytes),
        "qualityProfileId": t.quality_profile_id.unwrap_or(1),
        "rootFolderPath": root,
        "path": folder,
        "added": date(t.created_at),
        "tags": [],
        "images": [],
    }))
}

async fn quality_profiles(user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<Value>> {
    admin(&user)?;
    let p = koda_db::library::profile(&s.db, None).await?;
    Ok(Json(json!(p.map(|p| json!({ "id": p.id, "name": p.name })).into_iter().collect::<Vec<_>>())))
}

async fn root_folders(user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<Value>> {
    admin(&user)?;
    let root = movies_root(&s).await?;
    if root.is_empty() {
        return Ok(Json(json!([])));
    }
    // shortcut: free space is not measured, report it once Koda tracks disk usage
    Ok(Json(json!([{ "id": 1, "path": root, "accessible": true, "freeSpace": 0, "unmappedFolders": [] }])))
}

// shortcut: Koda has no tags; created tags are acknowledged, not stored
async fn tags(user: CurrentUser) -> ApiResult<Json<Value>> {
    admin(&user)?;
    Ok(Json(json!([])))
}

async fn create_tag(user: CurrentUser, Json(b): Json<Value>) -> ApiResult<(StatusCode, Json<Value>)> {
    admin(&user)?;
    Ok((StatusCode::CREATED, Json(json!({ "id": 1, "label": b["label"] }))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MovieQuery {
    tmdb_id: Option<i64>,
}

async fn list_movies(
    user: CurrentUser,
    State(s): State<AppState>,
    Query(q): Query<MovieQuery>,
) -> ApiResult<Json<Vec<Value>>> {
    admin(&user)?;
    let root = movies_root(&s).await?;
    let titles = match q.tmdb_id {
        Some(id) => koda_db::titles::by_tmdb(&s.db, TitleKind::Movie, id).await?.into_iter().collect(),
        None => koda_db::titles::list(&s.db, None).await?,
    };
    let mut out = Vec::with_capacity(titles.len());
    for t in titles.iter().filter(|t| t.kind == TitleKind::Movie.as_str()) {
        out.push(movie(&s, t, &root).await?);
    }
    Ok(Json(out))
}

async fn one_movie(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    admin(&user)?;
    let t = koda_db::titles::by_id(&s.db, id).await?.ok_or(ApiError::NotFound("movie"))?;
    Ok(Json(movie(&s, &t, &movies_root(&s).await?).await?))
}

#[derive(Deserialize)]
struct Lookup {
    term: String,
}

/// `term=tmdb:603` finds one film, in the library or not (then `id` is 0).
async fn lookup_movie(
    user: CurrentUser,
    State(s): State<AppState>,
    Query(q): Query<Lookup>,
) -> ApiResult<Json<Vec<Value>>> {
    admin(&user)?;
    let Some(tmdb_id) = q.term.trim().strip_prefix("tmdb:").and_then(|id| id.trim().parse::<i64>().ok())
    else {
        return Ok(Json(vec![]));
    };
    let root = movies_root(&s).await?;
    if let Some(t) = koda_db::titles::by_tmdb(&s.db, TitleKind::Movie, tmdb_id).await? {
        return Ok(Json(vec![movie(&s, &t, &root).await?]));
    }
    let tmdb = s.tmdb();
    let d = s
        .cached(format!("movie:{tmdb_id}"), std::time::Duration::from_secs(6 * 3600), || tmdb.movie(tmdb_id))
        .await?;
    Ok(Json(vec![json!({
        "id": 0,
        "title": d.title,
        "originalTitle": d.original_title,
        "tmdbId": d.id,
        "imdbId": d.imdb_id,
        "year": d.year().unwrap_or(0),
        "titleSlug": slug(&d.title, d.id),
        "overview": d.overview,
        "runtime": d.runtime.unwrap_or(0),
        "monitored": false,
        "hasFile": false,
        "isAvailable": true,
        "status": "released",
        "rootFolderPath": root,
        "tags": [],
        "images": [],
    })]))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewMovie {
    tmdb_id: i64,
}

/// Adding a film is an approved request by the key's owner: Koda searches
/// for it right away.
async fn add_movie(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(b): Json<NewMovie>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    admin(&user)?;
    let Json(req) = super::requests::create(
        CurrentUser(user.0.clone()),
        State(s.clone()),
        Json(super::requests::NewRequest { tmdb_id: b.tmdb_id, kind: Default::default() }),
    )
    .await?;
    let t = koda_db::titles::by_id(&s.db, req.title_id).await?.ok_or(ApiError::NotFound("movie"))?;
    Ok((StatusCode::CREATED, Json(movie(&s, &t, &movies_root(&s).await?).await?)))
}

#[derive(Deserialize)]
struct MovieId {
    id: i64,
}

/// Edits are not applied (Koda picks folders and quality itself); the
/// current state is returned so the caller carries on.
async fn update_movie(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    admin(&user)?;
    let t = koda_db::titles::by_id(&s.db, id).await?.ok_or(ApiError::NotFound("movie"))?;
    Ok((StatusCode::ACCEPTED, Json(movie(&s, &t, &movies_root(&s).await?).await?)))
}

async fn update_movie_body(
    user: CurrentUser,
    state: State<AppState>,
    Json(b): Json<MovieId>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    update_movie(user, state, Path(b.id)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Command {
    name: String,
    #[serde(default)]
    movie_ids: Vec<i64>,
}

/// `MoviesSearch` searches now for wanted films; other commands are accepted
/// and have nothing to do in Koda.
async fn command(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(c): Json<Command>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    admin(&user)?;
    if c.name.eq_ignore_ascii_case("MoviesSearch") {
        for id in &c.movie_ids {
            if let Some(t) = koda_db::titles::by_id(&s.db, *id).await?
                && t.state() == TitleState::Wanted
            {
                koda_db::titles::schedule_search(&s.db, t.id, 0, None).await?;
            }
        }
        s.wake.notify_one();
    }
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": 1, "name": c.name, "commandName": c.name, "status": "queued", "queued": date(now) }),
        ),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryQuery {
    /// Comma-separated title ids.
    #[serde(default)]
    movie_ids: String,
}

/// Grab history (the release each film came from), newest grab per film.
async fn history(
    user: CurrentUser,
    State(s): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    admin(&user)?;
    let mut records = Vec::new();
    for id in q.movie_ids.split(',').filter_map(|i| i.trim().parse::<i64>().ok()) {
        if let Some(g) = koda_db::grabs::latest_for_title(&s.db, id).await? {
            records.push(json!({
                "id": g.id,
                "movieId": g.title_id,
                "sourceTitle": g.release_title,
                "eventType": "grabbed",
                "date": date(g.created_at),
                "downloadId": g.info_hash,
                "quality": { "quality": { "id": 0, "name": g.quality.clone().unwrap_or_default() } },
                "data": {},
            }));
        }
    }
    Ok(Json(
        json!({ "page": 1, "pageSize": records.len().max(1), "totalRecords": records.len(), "records": records }),
    ))
}

/// Downloads in progress, read live from the download clients.
async fn queue(user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<Value>> {
    admin(&user)?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let mut lists: HashMap<i64, Vec<koda_download::Download>> = HashMap::new();
    let mut records = Vec::new();
    for g in koda_db::grabs::active(&s.db).await? {
        let (Some(client_id), Some(download_id)) = (g.download_client_id, g.info_hash.as_deref()) else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(slot) = lists.entry(client_id) {
            let list = match koda_db::connectors::download_client(&s.db, client_id).await? {
                Some(row) => crate::pipeline::download_client(&s, &row).list().await.unwrap_or_default(),
                None => vec![],
            };
            slot.insert(list);
        }
        let Some(d) = lists[&client_id].iter().find(|d| d.id == download_id) else { continue };
        let left = (d.size_bytes as f64 * (1.0 - f64::from(d.progress))) as u64;
        let eta = d.eta_seconds.unwrap_or(0);
        let status = format!("{:?}", d.status).to_lowercase();
        records.push(json!({
            "id": g.id,
            "movieId": g.title_id,
            "title": g.release_title,
            "size": d.size_bytes,
            "sizeleft": left,
            "timeleft": format!("{:02}:{:02}:{:02}", eta / 3600, eta / 60 % 60, eta % 60),
            "estimatedCompletionTime": date(now + eta),
            "status": status,
            "trackedDownloadStatus": if matches!(status.as_str(), "warning" | "failed") { "warning" } else { "ok" },
            "trackedDownloadState": if status == "completed" { "importPending" } else { "downloading" },
            "downloadId": download_id,
            "errorMessage": d.message,
        }));
    }
    Ok(Json(
        json!({ "page": 1, "pageSize": records.len().max(1), "totalRecords": records.len(), "records": records }),
    ))
}
