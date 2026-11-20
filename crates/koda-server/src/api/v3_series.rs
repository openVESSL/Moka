//! Shows in the `/api/v3` compatibility layer, in the shape TV request apps
//! and subtitle apps expect from a show manager: series, episodes and
//! episode files. Ids are Koda title ids.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use koda_core::{TitleKind, TitleState};
use koda_db::titles::Title;
use serde::Deserialize;
use serde_json::{Value, json};

use super::v3::{admin, date, resolution, slug};
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

async fn shows_root(s: &AppState) -> ApiResult<String> {
    let lib: crate::pipeline::LibrarySettings =
        koda_db::settings::get(&s.db, crate::pipeline::LibrarySettings::KEY).await?.unwrap_or_default();
    Ok(lib.shows_path)
}

fn today() -> String {
    time::OffsetDateTime::now_utc().date().to_string()
}

fn stats(eps: &[&Title], files: &HashMap<i64, (i64, i64)>) -> Value {
    let today = today();
    let with_file = eps.iter().filter(|e| e.state() == TitleState::Available).count();
    let aired = eps.iter().filter(|e| e.release_date.as_deref().is_some_and(|d| d <= today.as_str())).count();
    let size: i64 = eps.iter().filter_map(|e| files.get(&e.id)).map(|(_, size)| size).sum();
    let total = eps.len();
    json!({
        "episodeFileCount": with_file,
        "episodeCount": aired.max(with_file),
        "totalEpisodeCount": total,
        "sizeOnDisk": size,
        "percentOfEpisodes": if total == 0 { 0.0 } else { with_file as f64 * 100.0 / total as f64 },
    })
}

/// File id and size of each episode that has a file.
async fn episode_files(s: &AppState, eps: &[Title]) -> ApiResult<HashMap<i64, (i64, i64)>> {
    let mut out = HashMap::new();
    for e in eps.iter().filter(|e| e.state() == TitleState::Available) {
        if let Some(f) = koda_db::library::files_for_title(&s.db, e.id).await?.into_iter().next() {
            out.insert(e.id, (f.id, f.size_bytes));
        }
    }
    Ok(out)
}

/// A show in the library as a series resource.
async fn series(s: &AppState, show: &Title, root: &str) -> ApiResult<Value> {
    let eps = koda_db::titles::episodes_of_show(&s.db, show.id).await?;
    let seasons = koda_db::titles::children(&s.db, show.id).await?;
    let files = episode_files(s, &eps).await?;
    let all: Vec<&Title> = eps.iter().filter(|e| e.season_number != Some(0)).collect();
    let season_list: Vec<Value> = seasons
        .iter()
        .map(|season| {
            let in_season: Vec<&Title> =
                eps.iter().filter(|e| e.season_number == season.season_number).collect();
            json!({
                "seasonNumber": season.season_number.unwrap_or(0),
                "monitored": season.monitored && in_season.iter().any(|e| e.monitored),
                "statistics": stats(&in_season, &files),
            })
        })
        .collect();
    let tmdb_id = show.tmdb_id.unwrap_or(0);
    let tvdb_id = koda_db::titles::tvdb_id(&s.db, show.id).await?.unwrap_or(0);
    let numbering = koda_db::titles::show_numbering(&s.db, show.id).await?;
    let mut statistics = stats(&all, &files);
    statistics["seasonCount"] = json!(seasons.iter().filter(|s| s.season_number != Some(0)).count());
    Ok(json!({
        "id": show.id,
        "title": show.title,
        "sortTitle": show.title.to_lowercase(),
        "tvdbId": tvdb_id,
        "tmdbId": tmdb_id,
        "imdbId": show.imdb_id,
        "year": show.year.unwrap_or(0),
        "overview": show.overview,
        "firstAired": show.release_date,
        "status": "continuing",
        "seriesType": numbering,
        "titleSlug": slug(&show.title, tmdb_id),
        "monitored": show.state() != TitleState::Requested,
        "monitorNewItems": "all",
        "seasonFolder": true,
        "useSceneNumbering": false,
        "runtime": show.runtime_minutes.unwrap_or(0),
        "qualityProfileId": show.quality_profile_id.unwrap_or(1),
        "languageProfileId": 1,
        "rootFolderPath": root,
        "path": format!("{}/{}", root.trim_end_matches(['/', '\\']), crate::pipeline::library_folder_name(show)),
        "added": date(show.created_at),
        "seasonCount": statistics["seasonCount"],
        "seasons": season_list,
        "statistics": statistics,
        "tags": [],
        "genres": [],
        "images": poster(show.poster_path.as_deref()),
        "ratings": { "votes": 0, "value": 0.0 },
    }))
}

fn poster(path: Option<&str>) -> Value {
    match path {
        Some(p) => {
            json!([{ "coverType": "poster", "remoteUrl": format!("{}/original{p}", koda_metadata::IMAGE_BASE_URL) }])
        }
        None => json!([]),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SeriesQuery {
    tvdb_id: Option<i64>,
}

pub(super) async fn list_series(
    user: CurrentUser,
    State(s): State<AppState>,
    Query(q): Query<SeriesQuery>,
) -> ApiResult<Json<Vec<Value>>> {
    admin(&user)?;
    let root = shows_root(&s).await?;
    let shows = match q.tvdb_id {
        Some(id) => koda_db::titles::show_by_tvdb(&s.db, id).await?.into_iter().collect(),
        None => koda_db::titles::shows(&s.db).await?,
    };
    let mut out = Vec::with_capacity(shows.len());
    for show in &shows {
        out.push(series(&s, show, &root).await?);
    }
    Ok(Json(out))
}

pub(super) async fn one_series(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    admin(&user)?;
    let show = show(&s, id).await?;
    Ok(Json(series(&s, &show, &shows_root(&s).await?).await?))
}

async fn show(s: &AppState, id: i64) -> ApiResult<Title> {
    koda_db::titles::by_id(&s.db, id)
        .await?
        .filter(|t| t.kind == TitleKind::Show.as_str())
        .ok_or(ApiError::NotFound("series"))
}

#[derive(Deserialize)]
pub(super) struct Lookup {
    term: String,
}

/// `tvdb:ID` (what request apps send) or plain text.
pub(super) async fn lookup_series(
    user: CurrentUser,
    State(s): State<AppState>,
    Query(q): Query<Lookup>,
) -> ApiResult<Json<Vec<Value>>> {
    admin(&user)?;
    let root = shows_root(&s).await?;
    let tmdb = s.tmdb();
    let tmdb_ids: Vec<i64> =
        match q.term.trim().strip_prefix("tvdb:").and_then(|i| i.trim().parse::<i64>().ok()) {
            Some(tvdb) => {
                if let Some(known) = koda_db::titles::show_by_tvdb(&s.db, tvdb).await? {
                    return Ok(Json(vec![series(&s, &known, &root).await?]));
                }
                vec![tmdb.show_by_tvdb(tvdb).await?]
            }
            None => {
                tmdb.search_shows(q.term.trim(), 1).await?.results.into_iter().take(5).map(|r| r.id).collect()
            }
        };
    let mut out = Vec::new();
    for id in tmdb_ids {
        if let Some(known) = koda_db::titles::by_tmdb(&s.db, TitleKind::Show, id).await? {
            out.push(series(&s, &known, &root).await?);
            continue;
        }
        let d = tmdb.show(id).await?;
        let seasons: Vec<Value> = d
            .seasons
            .iter()
            .map(|x| json!({ "seasonNumber": x.season_number, "monitored": false,
                "statistics": { "episodeFileCount": 0, "episodeCount": 0, "totalEpisodeCount": x.episode_count,
                                "sizeOnDisk": 0, "percentOfEpisodes": 0.0 } }))
            .collect();
        out.push(json!({
            "title": d.name,
            "sortTitle": d.name.to_lowercase(),
            "tvdbId": d.external_ids.tvdb_id.unwrap_or(0),
            "tmdbId": d.id,
            "imdbId": d.external_ids.imdb_id,
            "year": d.year().unwrap_or(0),
            "overview": d.overview,
            "firstAired": d.first_air_date,
            "status": if d.status.as_deref() == Some("Ended") { "ended" } else { "continuing" },
            "seriesType": d.numbering(),
            "titleSlug": slug(&d.name, d.id),
            "seasonFolder": true,
            "monitored": false,
            "runtime": d.runtime().unwrap_or(0),
            "seasons": seasons,
            "seasonCount": d.seasons.iter().filter(|x| x.season_number > 0).count(),
            "images": poster(d.poster_path.as_deref()),
            "genres": d.genres.iter().map(|g| g.name.clone()).collect::<Vec<_>>(),
            "tags": [],
            "ratings": { "votes": 0, "value": d.vote_average },
        }));
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SeasonFlag {
    season_number: i64,
    #[serde(default)]
    monitored: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NewSeries {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    tvdb_id: i64,
    #[serde(default)]
    tmdb_id: Option<i64>,
    #[serde(default)]
    seasons: Vec<SeasonFlag>,
}

/// Follows exactly the seasons marked monitored; others are left alone.
async fn apply_seasons(s: &AppState, show_id: i64, flags: &[SeasonFlag]) -> ApiResult<()> {
    let seasons = koda_db::titles::children(&s.db, show_id).await?;
    for f in flags {
        if let Some(season) = seasons.iter().find(|x| x.season_number == Some(f.season_number)) {
            koda_db::titles::set_monitored(&s.db, season.id, f.monitored).await?;
        }
    }
    s.wake.notify_one();
    Ok(())
}

/// Adds a show (approved, as for an admin request), following the seasons
/// asked for. A show already in the library just has those seasons followed.
pub(super) async fn add_series(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(b): Json<NewSeries>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    admin(&user)?;
    let tmdb_id = match b.tmdb_id.filter(|id| *id > 0) {
        Some(id) => id,
        None => s.tmdb().show_by_tvdb(b.tvdb_id).await?,
    };
    let new = super::catalog::new_show(&s, tmdb_id).await?;
    let show = koda_db::titles::upsert_show(&s.db, &new, TitleState::Wanted).await?;
    if show.state() == TitleState::Requested {
        koda_db::titles::transition(&s.db, show.id, TitleState::Wanted, None)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        koda_db::titles::approve_show_parts(&s.db, show.id).await?;
    }
    if !b.seasons.is_empty() {
        apply_seasons(&s, show.id, &b.seasons).await?;
    }
    koda_db::requests::create(&s.db, show.id, user.0.id, koda_db::requests::RequestStatus::Approved).await?;
    s.wake.notify_one();
    Ok((StatusCode::CREATED, Json(series(&s, &show, &shows_root(&s).await?).await?)))
}

pub(super) async fn update_series(
    user: CurrentUser,
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<NewSeries>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    admin(&user)?;
    let show = show(&s, id).await?;
    apply_seasons(&s, show.id, &b.seasons).await?;
    Ok((StatusCode::ACCEPTED, Json(series(&s, &show, &shows_root(&s).await?).await?)))
}

pub(super) async fn update_series_body(
    user: CurrentUser,
    state: State<AppState>,
    Json(b): Json<NewSeries>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let id = b.id.ok_or_else(|| ApiError::BadRequest("id is missing".into()))?;
    update_series(user, state, Path(id), Json(b)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BySeries {
    series_id: i64,
}

pub(super) async fn list_episodes(
    user: CurrentUser,
    State(s): State<AppState>,
    Query(q): Query<BySeries>,
) -> ApiResult<Json<Vec<Value>>> {
    admin(&user)?;
    let show = show(&s, q.series_id).await?;
    let eps = koda_db::titles::episodes_of_show(&s.db, show.id).await?;
    let files = episode_files(&s, &eps).await?;
    let mut abs = 0;
    let out = eps
        .iter()
        .map(|e| {
            let regular = e.season_number.is_some_and(|n| n > 0);
            abs += i64::from(regular);
            json!({
                "id": e.id,
                "seriesId": show.id,
                "episodeFileId": files.get(&e.id).map_or(0, |(id, _)| *id),
                "seasonNumber": e.season_number.unwrap_or(0),
                "episodeNumber": e.episode_number.unwrap_or(0),
                "absoluteEpisodeNumber": if regular { Some(abs) } else { None },
                "title": e.title,
                "airDate": e.release_date,
                "airDateUtc": e.release_date.as_ref().map(|d| format!("{d}T00:00:00Z")),
                "overview": e.overview,
                "hasFile": files.contains_key(&e.id),
                "monitored": e.monitored,
                "unverifiedSceneNumbering": false,
            })
        })
        .collect();
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Monitor {
    episode_ids: Vec<i64>,
    monitored: bool,
}

pub(super) async fn monitor_episodes(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(b): Json<Monitor>,
) -> ApiResult<StatusCode> {
    admin(&user)?;
    for id in b.episode_ids {
        if koda_db::titles::by_id(&s.db, id).await?.is_some_and(|t| t.kind == TitleKind::Episode.as_str()) {
            koda_db::titles::set_monitored(&s.db, id, b.monitored).await?;
        }
    }
    s.wake.notify_one();
    Ok(StatusCode::ACCEPTED)
}

/// Episode files with their paths and release names, for subtitle apps.
pub(super) async fn list_episode_files(
    user: CurrentUser,
    State(s): State<AppState>,
    Query(q): Query<BySeries>,
) -> ApiResult<Json<Vec<Value>>> {
    admin(&user)?;
    let show = show(&s, q.series_id).await?;
    let mut out = Vec::new();
    for e in koda_db::titles::episodes_of_show(&s.db, show.id).await? {
        for f in koda_db::library::files_for_title(&s.db, e.id).await? {
            let q = f.quality.clone().unwrap_or_default();
            let scene = koda_db::grabs::latest_for_title(&s.db, e.id)
                .await?
                .filter(|g| g.status == "imported")
                .map(|g| g.release_title);
            out.push(json!({
                "id": f.id,
                "seriesId": show.id,
                "seasonNumber": e.season_number.unwrap_or(0),
                "path": f.path,
                "relativePath": std::path::Path::new(&f.path).file_name().map(|n| n.to_string_lossy().into_owned()),
                "size": f.size_bytes,
                "dateAdded": date(f.added_at),
                "sceneName": scene,
                "releaseGroup": f.release_group,
                // shortcut: audio languages are not reported; subtitle apps read them from the file
                "languages": [],
                "quality": { "quality": { "id": 0, "name": q, "resolution": resolution(&q) },
                             "revision": { "version": 1, "real": 0, "isRepack": false } },
            }));
        }
    }
    Ok(Json(out))
}

/// Koda has no language profiles; clients on older versions ask for one.
pub(super) async fn language_profiles(user: CurrentUser) -> ApiResult<Json<Value>> {
    admin(&user)?;
    Ok(Json(json!([{ "id": 1, "name": "Any" }])))
}

/// `SeriesSearch` / `MissingEpisodeSearch`: the show's wanted episodes are searched now.
pub(super) async fn search_series(s: &AppState, series_id: i64) -> ApiResult<()> {
    koda_db::titles::search_show_now(&s.db, series_id).await?;
    s.wake.notify_one();
    Ok(())
}

/// Queue fields for a grab of an episode or a season pack.
pub(super) async fn queue_fields(s: &AppState, title_id: i64) -> ApiResult<Option<Value>> {
    let Some(t) = koda_db::titles::by_id(&s.db, title_id).await? else { return Ok(None) };
    let season_id = match t.kind.as_str() {
        "episode" => t.parent_id.unwrap_or(0),
        "season" => t.id,
        _ => return Ok(None),
    };
    let show_id = koda_db::titles::by_id(&s.db, season_id).await?.and_then(|x| x.parent_id).unwrap_or(0);
    let mut v = json!({ "seriesId": show_id, "seasonNumber": t.season_number.unwrap_or(0) });
    if t.kind == "episode" {
        v["episodeId"] = json!(t.id);
        v["episode"] = json!({
            "id": t.id,
            "seriesId": show_id,
            "seasonNumber": t.season_number.unwrap_or(0),
            "episodeNumber": t.episode_number.unwrap_or(0),
            "title": t.title,
        });
    }
    Ok(Some(v))
}
