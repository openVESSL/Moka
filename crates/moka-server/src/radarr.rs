//! Moka in front of an existing Radarr: approved films are handed to Radarr,
//! which searches and downloads; Moka follows them and lists Radarr's films
//! in its library so people can request and watch in one place.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, bail};
use moka_core::{Quality, TitleKind, TitleState};
use moka_db::titles::{NewTitle, Title};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::AppState;

/// Stored under `integrations.radarr`. The API key is sealed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RadarrSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub api_key: String,
    /// 0 picks Radarr's first profile.
    #[serde(default)]
    pub quality_profile_id: i64,
    /// Empty picks Radarr's first root folder.
    #[serde(default)]
    pub root_folder: String,
    /// When Moka sees Radarr's files under another path, replace this
    /// prefix of Radarr's paths...
    #[serde(default)]
    pub path_from: String,
    /// ...with this one.
    #[serde(default)]
    pub path_to: String,
}

impl RadarrSettings {
    pub const KEY: &'static str = "integrations.radarr";

    pub async fn load(db: &moka_db::Pool) -> Result<Self, moka_db::Error> {
        let mut s: Self = moka_db::settings::get(db, Self::KEY).await?.unwrap_or_default();
        s.api_key = moka_db::secret::open(&s.api_key);
        Ok(s)
    }

    pub async fn save(&self, db: &moka_db::Pool) -> Result<(), moka_db::Error> {
        let sealed = Self { api_key: moka_db::secret::seal(&self.api_key), ..self.clone() };
        moka_db::settings::set(db, Self::KEY, &sealed).await
    }

    pub fn usable(&self) -> bool {
        self.enabled && !self.url.is_empty() && !self.api_key.is_empty()
    }

    /// Radarr's path as Moka sees it.
    pub fn map_path(&self, path: &str) -> String {
        match path.strip_prefix(self.path_from.as_str()) {
            Some(rest) if !self.path_from.is_empty() => format!("{}{rest}", self.path_to),
            _ => path.to_owned(),
        }
    }
}

pub struct Client<'a> {
    http: &'a reqwest::Client,
    s: &'a RadarrSettings,
}

impl<'a> Client<'a> {
    pub fn new(http: &'a reqwest::Client, s: &'a RadarrSettings) -> Self {
        Self { http, s }
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> anyhow::Result<Value> {
        let url = format!("{}/api/v3{path}", self.s.url.trim_end_matches('/'));
        let mut req = self
            .http
            .request(method, url)
            .header("X-Api-Key", &self.s.api_key)
            .timeout(Duration::from_secs(30));
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await?;
        match res.status().as_u16() {
            200..=299 => Ok(res.json().await.unwrap_or(Value::Null)),
            401 | 403 => bail!("Radarr rejected the API key"),
            s => bail!("Radarr returned HTTP {s}"),
        }
    }

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.call(reqwest::Method::GET, path, None).await
    }

    pub async fn version(&self) -> anyhow::Result<String> {
        let v = self.get("/system/status").await?;
        Ok(v["version"].as_str().unwrap_or("").to_owned())
    }

    pub async fn profiles(&self) -> anyhow::Result<Vec<(i64, String)>> {
        let v = self.get("/qualityprofile").await?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p["id"].as_i64()?, p["name"].as_str()?.to_owned())))
            .collect())
    }

    pub async fn root_folders(&self) -> anyhow::Result<Vec<String>> {
        let v = self.get("/rootfolder").await?;
        Ok(v.as_array().into_iter().flatten().filter_map(|r| r["path"].as_str().map(str::to_owned)).collect())
    }

    /// Radarr's film for this TMDB id, if it has one.
    pub async fn movie(&self, tmdb_id: i64) -> anyhow::Result<Option<Value>> {
        let v = self.get(&format!("/movie?tmdbId={tmdb_id}")).await?;
        Ok(v.as_array().and_then(|a| a.first().cloned()))
    }

    pub async fn movies(&self) -> anyhow::Result<Vec<Value>> {
        Ok(self.get("/movie").await?.as_array().cloned().unwrap_or_default())
    }

    /// Radarr film ids currently in its download queue.
    pub async fn queued(&self) -> anyhow::Result<Vec<i64>> {
        let v = self.get("/queue?pageSize=1000").await?;
        Ok(v["records"].as_array().into_iter().flatten().filter_map(|r| r["movieId"].as_i64()).collect())
    }

    /// Makes Radarr want this film and search for it.
    pub async fn want(&self, t: &Title) -> anyhow::Result<()> {
        let tmdb_id = t.tmdb_id.context("film has no TMDB id")?;
        if let Some(mut m) = self.movie(tmdb_id).await? {
            if m["hasFile"] == true {
                return Ok(());
            }
            if m["monitored"] != true {
                m["monitored"] = true.into();
                let id = m["id"].as_i64().unwrap_or(0);
                self.call(reqwest::Method::PUT, &format!("/movie/{id}"), Some(m)).await?;
            }
            let id = self.movie(tmdb_id).await?.and_then(|m| m["id"].as_i64()).unwrap_or(0);
            self.call(
                reqwest::Method::POST,
                "/command",
                Some(json!({ "name": "MoviesSearch", "movieIds": [id] })),
            )
            .await?;
            return Ok(());
        }
        let profile = match self.s.quality_profile_id {
            0 => self.profiles().await?.first().map(|p| p.0).context("Radarr has no quality profile")?,
            id => id,
        };
        let root = match self.s.root_folder.as_str() {
            "" => self.root_folders().await?.into_iter().next().context("Radarr has no root folder")?,
            r => r.to_owned(),
        };
        self.call(
            reqwest::Method::POST,
            "/movie",
            Some(json!({
                "title": t.title,
                "tmdbId": tmdb_id,
                "year": t.year.unwrap_or(0),
                "qualityProfileId": profile,
                "rootFolderPath": root,
                "monitored": true,
                "minimumAvailability": "released",
                "tags": [],
                "addOptions": { "searchForMovie": true },
            })),
        )
        .await?;
        Ok(())
    }
}

/// `Bluray-1080p` -> `bluray-1080p` when Moka knows that quality.
fn quality_key(m: &Value) -> Option<String> {
    let name = m["movieFile"]["quality"]["quality"]["name"].as_str()?.to_ascii_lowercase();
    Quality::from_key(&name).map(|q| q.key())
}

/// `https://image.tmdb.org/t/p/original/abc.jpg` -> `/abc.jpg`
fn tmdb_image(m: &Value, kind: &str) -> Option<String> {
    m["images"].as_array()?.iter().find(|i| i["coverType"] == kind).and_then(|i| {
        let url = i["remoteUrl"].as_str()?;
        url.contains("image.tmdb.org").then(|| url.rsplit('/').next().map(|f| format!("/{f}")))?
    })
}

/// Records Radarr's file for a Moka title and marks it available.
async fn take_file(state: &AppState, s: &RadarrSettings, title_id: i64, m: &Value) -> anyhow::Result<bool> {
    let Some(path) = m["movieFile"]["path"].as_str() else { return Ok(false) };
    let path = s.map_path(path);
    let quality = quality_key(m);
    moka_db::library::add_file(
        &state.db,
        title_id,
        &path,
        m["movieFile"]["size"].as_i64().unwrap_or(0),
        quality.as_deref(),
        m["movieFile"]["releaseGroup"].as_str(),
    )
    .await?;
    moka_db::titles::mark_available(&state.db, title_id).await?;
    Ok(true)
}

/// One pass: hand wanted films to Radarr and follow the ones it is working on.
/// Returns false when Radarr is not in use, so the built-in search runs.
pub async fn tick(state: &AppState) -> anyhow::Result<bool> {
    let s = RadarrSettings::load(&state.db).await?;
    if !s.usable() {
        return Ok(false);
    }
    let client = Client::new(&state.http, &s);

    for t in moka_db::titles::due_for_search(&state.db, 10).await? {
        match client.want(&t).await {
            Ok(()) => {
                let t = moka_db::titles::transition(
                    &state.db,
                    t.id,
                    TitleState::Searching,
                    Some("Sent to Radarr"),
                )
                .await?;
                crate::pipeline::publish_state(state, &t);
            }
            Err(e) => {
                tracing::warn!(title = %t.title, error = %e, "could not hand film to Radarr");
                moka_db::titles::schedule_search(
                    &state.db,
                    t.id,
                    time::OffsetDateTime::now_utc().unix_timestamp() + 15 * 60,
                    Some(&e.to_string()),
                )
                .await?;
            }
        }
    }

    let mut following: Vec<Title> = Vec::new();
    for st in [TitleState::Searching, TitleState::Grabbed, TitleState::Downloading] {
        following.extend(moka_db::titles::in_state(&state.db, st).await?);
    }
    if following.is_empty() {
        return Ok(true);
    }
    let queued = client.queued().await?;
    for t in following {
        // a grab Moka made itself is followed by the built-in tracker
        if moka_db::grabs::latest_for_title(&state.db, t.id)
            .await?
            .is_some_and(|g| g.status != "imported" && g.status != "failed")
        {
            continue;
        }
        let Some(tmdb_id) = t.tmdb_id else { continue };
        let Some(m) = client.movie(tmdb_id).await? else {
            let t =
                moka_db::titles::transition(&state.db, t.id, TitleState::Wanted, Some("Removed from Radarr"))
                    .await?;
            crate::pipeline::publish_state(state, &t);
            continue;
        };
        if m["hasFile"] == true && take_file(state, &s, t.id, &m).await? {
            if let Some(t) = moka_db::titles::by_id(&state.db, t.id).await? {
                crate::pipeline::publish_state(state, &t);
                let info = crate::notify::TitleInfo::from_title(&t, None);
                crate::notify::send(state, crate::notify::Event::Available, Some(info), None);
            }
            continue;
        }
        let in_queue = m["id"].as_i64().is_some_and(|id| queued.contains(&id));
        if in_queue && t.state() == TitleState::Searching {
            moka_db::titles::transition(&state.db, t.id, TitleState::Grabbed, None).await?;
            let t = moka_db::titles::transition(
                &state.db,
                t.id,
                TitleState::Downloading,
                Some("Downloading in Radarr"),
            )
            .await?;
            crate::pipeline::publish_state(state, &t);
        }
    }
    Ok(true)
}

/// Lists Radarr's downloaded films in Moka's library. Returns how many were added.
pub async fn import_library(state: &AppState) -> anyhow::Result<usize> {
    let s = RadarrSettings::load(&state.db).await?;
    if !s.usable() {
        return Ok(0);
    }
    let mut added = 0;
    let known: HashMap<i64, Title> = moka_db::titles::list(&state.db, None)
        .await?
        .into_iter()
        .filter(|t| t.kind == TitleKind::Movie.as_str())
        .filter_map(|t| Some((t.tmdb_id?, t)))
        .collect();
    for m in Client::new(&state.http, &s).movies().await? {
        let (Some(tmdb_id), true) = (m["tmdbId"].as_i64(), m["hasFile"] == true) else { continue };
        if known.get(&tmdb_id).is_some_and(|t| t.state() == TitleState::Available) {
            continue;
        }
        let title = match known.get(&tmdb_id) {
            Some(t) => t.clone(),
            None => {
                let new = NewTitle {
                    tmdb_id,
                    imdb_id: m["imdbId"].as_str().map(str::to_owned),
                    title: m["title"].as_str().unwrap_or("").to_owned(),
                    original_title: m["originalTitle"].as_str().map(str::to_owned),
                    year: m["year"].as_i64().filter(|y| *y > 0),
                    overview: m["overview"].as_str().map(str::to_owned),
                    runtime_minutes: m["runtime"].as_i64().filter(|r| *r > 0),
                    release_date: None,
                    poster_path: tmdb_image(&m, "poster"),
                    backdrop_path: tmdb_image(&m, "fanart"),
                    metadata_json: None,
                };
                moka_db::titles::upsert_movie(&state.db, &new, TitleState::Available).await?
            }
        };
        if take_file(state, &s, title.id, &m).await? {
            added += 1;
        }
    }
    Ok(added)
}

/// Brings Radarr's library over every 30 minutes.
pub async fn run(state: AppState) {
    loop {
        match import_library(&state).await {
            Ok(n) if n > 0 => tracing::info!(count = n, "films listed from Radarr"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "Radarr library sync failed"),
        }
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Query, State};
    use axum::{Json, Router, routing::get, routing::post};
    use std::sync::{Arc, Mutex};

    type Movies = Arc<Mutex<Vec<Value>>>;

    fn fake_radarr(movies: Movies) -> Router {
        Router::new()
            .route("/api/v3/system/status", get(|| async { Json(json!({ "version": "5.26.2.10099" })) }))
            .route("/api/v3/qualityprofile", get(|| async { Json(json!([{ "id": 4, "name": "HD-1080p" }])) }))
            .route("/api/v3/rootfolder", get(|| async { Json(json!([{ "id": 1, "path": "/movies" }])) }))
            .route("/api/v3/queue", get(|| async { Json(json!({ "records": [] })) }))
            .route("/api/v3/command", post(|| async { Json(json!({ "id": 1 })) }))
            .route(
                "/api/v3/movie",
                get(|State(m): State<Movies>, Query(q): Query<HashMap<String, i64>>| async move {
                    let all = m.lock().unwrap().clone();
                    Json(json!(match q.get("tmdbId") {
                        Some(id) => all.into_iter().filter(|x| x["tmdbId"] == *id).collect::<Vec<_>>(),
                        None => all,
                    }))
                })
                .post(|State(m): State<Movies>, Json(mut b): Json<Value>| async move {
                    let mut all = m.lock().unwrap();
                    b["id"] = (all.len() as i64 + 1).into();
                    b["hasFile"] = false.into();
                    all.push(b.clone());
                    Json(b)
                }),
            )
            .with_state(movies)
    }

    #[test]
    fn paths_and_images() {
        let s = RadarrSettings {
            path_from: "/movies".into(),
            path_to: "/data/films".into(),
            ..Default::default()
        };
        assert_eq!(s.map_path("/movies/Dune (2021)/Dune.mkv"), "/data/films/Dune (2021)/Dune.mkv");
        let m = json!({ "images": [{ "coverType": "poster", "remoteUrl": "https://image.tmdb.org/t/p/original/abc.jpg" }],
                        "movieFile": { "quality": { "quality": { "name": "WEBDL-1080p" } } } });
        assert_eq!(tmdb_image(&m, "poster").as_deref(), Some("/abc.jpg"));
        assert_eq!(tmdb_image(&m, "fanart"), None);
        assert_eq!(quality_key(&m).as_deref(), Some("webdl-1080p"));
    }

    #[tokio::test]
    async fn films_go_to_radarr_and_come_back_available() {
        let movies: Movies = Arc::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = fake_radarr(movies.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let db = moka_db::memory().await;
        let config = crate::config::Config {
            config_dir: std::env::temp_dir(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        assert!(!tick(&state).await.unwrap(), "off until configured");
        RadarrSettings {
            enabled: true,
            url: base,
            api_key: "k".into(),
            path_from: "/movies".into(),
            path_to: "/data".into(),
            ..Default::default()
        }
        .save(&db)
        .await
        .unwrap();

        let t = moka_db::titles::upsert_movie(
            &db,
            &NewTitle { tmdb_id: 603, title: "The Matrix".into(), year: Some(1999), ..Default::default() },
            TitleState::Wanted,
        )
        .await
        .unwrap();
        assert!(tick(&state).await.unwrap());
        let sent = movies.lock().unwrap()[0].clone();
        assert_eq!((sent["tmdbId"].as_i64(), sent["qualityProfileId"].as_i64()), (Some(603), Some(4)));
        assert_eq!(sent["rootFolderPath"], "/movies");
        assert_eq!(moka_db::titles::by_id(&db, t.id).await.unwrap().unwrap().state(), TitleState::Searching);

        // Radarr finishes the download
        {
            let mut all = movies.lock().unwrap();
            all[0]["hasFile"] = true.into();
            all[0]["movieFile"] = json!({ "path": "/movies/The Matrix (1999)/The Matrix.mkv", "size": 5,
                                          "quality": { "quality": { "name": "Bluray-1080p" } } });
            all.push(json!({ "id": 9, "tmdbId": 604, "title": "Other", "year": 2003, "hasFile": true,
                             "movieFile": { "path": "/movies/Other/Other.mkv", "size": 7 } }));
        }
        tick(&state).await.unwrap();
        assert_eq!(moka_db::titles::by_id(&db, t.id).await.unwrap().unwrap().state(), TitleState::Available);
        let files = moka_db::library::files_for_title(&db, t.id).await.unwrap();
        assert_eq!(files[0].path, "/data/The Matrix (1999)/The Matrix.mkv");
        assert_eq!(files[0].quality.as_deref(), Some("bluray-1080p"));

        assert_eq!(import_library(&state).await.unwrap(), 1, "only the film Moka did not have");
        let other = moka_db::titles::by_tmdb(&db, TitleKind::Movie, 604).await.unwrap().unwrap();
        assert_eq!(other.state(), TitleState::Available);
        assert_eq!(import_library(&state).await.unwrap(), 0);
    }
}
