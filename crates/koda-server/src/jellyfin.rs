//! Working alongside a Jellyfin server: tell it about newly imported films
//! (targeted refresh, not a full library scan) and read back what people
//! have watched there, matched to Koda accounts by user name.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Stored under the `integrations.jellyfin` setting. The API key is sealed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JellyfinSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub api_key: String,
    /// When Jellyfin sees the library under a different path (another
    /// container), replace this prefix of Koda's paths...
    #[serde(default)]
    pub path_from: String,
    /// ...with this one.
    #[serde(default)]
    pub path_to: String,
    #[serde(default = "yes")]
    pub sync_watched: bool,
}

fn yes() -> bool {
    true
}

impl JellyfinSettings {
    pub const KEY: &'static str = "integrations.jellyfin";

    pub async fn load(db: &koda_db::Pool) -> Result<Self, koda_db::Error> {
        let mut s: Self = koda_db::settings::get(db, Self::KEY).await?.unwrap_or_default();
        s.api_key = koda_db::secret::open(&s.api_key);
        Ok(s)
    }

    pub async fn save(&self, db: &koda_db::Pool) -> Result<(), koda_db::Error> {
        let sealed = Self { api_key: koda_db::secret::seal(&self.api_key), ..self.clone() };
        koda_db::settings::set(db, Self::KEY, &sealed).await
    }

    fn usable(&self) -> bool {
        self.enabled && !self.url.is_empty() && !self.api_key.is_empty()
    }

    /// Koda's path as Jellyfin sees it.
    pub fn map_path(&self, path: &str) -> String {
        match path.strip_prefix(self.path_from.as_str()) {
            Some(rest) if !self.path_from.is_empty() => format!("{}{rest}", self.path_to),
            _ => path.to_owned(),
        }
    }
}

pub struct Client<'a> {
    http: &'a reqwest::Client,
    s: &'a JellyfinSettings,
}

impl<'a> Client<'a> {
    pub fn new(http: &'a reqwest::Client, s: &'a JellyfinSettings) -> Self {
        Self { http, s }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.s.url.trim_end_matches('/'));
        // the legacy X-Emby-Token header is off by default on new servers
        let auth = format!(
            "MediaBrowser Client=\"Koda\", Device=\"Server\", DeviceId=\"koda-server\", Version=\"{}\", Token=\"{}\"",
            env!("CARGO_PKG_VERSION"),
            self.s.api_key
        );
        self.http
            .request(method, url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .timeout(Duration::from_secs(15))
    }

    async fn ok(res: reqwest::Response) -> anyhow::Result<reqwest::Response> {
        match res.status().as_u16() {
            200..=299 => Ok(res),
            401 | 403 => anyhow::bail!("Jellyfin rejected the API key"),
            s => anyhow::bail!("Jellyfin returned HTTP {s}"),
        }
    }

    /// Server name and version, to check the connection.
    pub async fn info(&self) -> anyhow::Result<(String, String)> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Info {
            server_name: String,
            version: String,
        }
        let res = Self::ok(self.req(reqwest::Method::GET, "/System/Info").send().await?).await?;
        let i: Info = res.json().await?;
        Ok((i.server_name, i.version))
    }

    /// Tells Jellyfin a file appeared, so it refreshes just that folder.
    pub async fn media_added(&self, koda_path: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({ "Updates": [{ "Path": self.s.map_path(koda_path), "UpdateType": "Created" }] });
        Self::ok(self.req(reqwest::Method::POST, "/Library/Media/Updated").json(&body).send().await?).await?;
        Ok(())
    }

    /// Jellyfin users as (id, name).
    pub async fn users(&self) -> anyhow::Result<Vec<(String, String)>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct User {
            id: String,
            name: String,
        }
        let res = Self::ok(self.req(reqwest::Method::GET, "/Users").send().await?).await?;
        let users: Vec<User> = res.json().await?;
        Ok(users.into_iter().map(|u| (u.id, u.name)).collect())
    }

    /// TMDB ids of films this Jellyfin user has watched.
    pub async fn watched_tmdb_ids(&self, user_id: &str) -> anyhow::Result<Vec<i64>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Items {
            items: Vec<Item>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Item {
            #[serde(default)]
            provider_ids: std::collections::HashMap<String, String>,
        }
        let path = format!(
            "/Items?userId={user_id}&IsPlayed=true&Recursive=true&IncludeItemTypes=Movie&Fields=ProviderIds&EnableUserData=false"
        );
        let res = Self::ok(self.req(reqwest::Method::GET, &path).send().await?).await?;
        let items: Items = res.json().await?;
        Ok(items
            .items
            .iter()
            .filter_map(|i| {
                i.provider_ids
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("tmdb"))
                    .and_then(|(_, v)| v.parse().ok())
            })
            .collect())
    }
}

/// After an import: ask Jellyfin to pick the file up. Errors are logged only.
pub fn announce_import(state: &AppState, path: String) {
    let state = state.clone();
    tokio::spawn(async move {
        let Ok(s) = JellyfinSettings::load(&state.db).await else { return };
        if !s.usable() {
            return;
        }
        if let Err(e) = Client::new(&state.http, &s).media_added(&path).await {
            tracing::warn!(error = %e, "could not tell Jellyfin about a new film");
        }
    });
}

/// Copies "watched" from Jellyfin to Koda for users with the same name.
/// Returns how many films were newly marked.
pub async fn sync_watched(state: &AppState) -> anyhow::Result<usize> {
    let s = JellyfinSettings::load(&state.db).await?;
    if !s.usable() || !s.sync_watched {
        return Ok(0);
    }
    let client = Client::new(&state.http, &s);
    let koda_users = koda_db::users::list(&state.db).await?;
    let mut marked = 0;
    for (jf_id, jf_name) in client.users().await? {
        let Some(user) = koda_users.iter().find(|u| u.username.eq_ignore_ascii_case(&jf_name)) else {
            continue;
        };
        for tmdb_id in client.watched_tmdb_ids(&jf_id).await? {
            let Some(t) = koda_db::titles::by_tmdb(&state.db, koda_core::TitleKind::Movie, tmdb_id).await?
            else {
                continue;
            };
            let (pos, played) = koda_db::library::progress(&state.db, user.id, t.id).await?;
            if !played {
                koda_db::library::set_progress(&state.db, user.id, t.id, pos.min(0.0), true).await?;
                marked += 1;
            }
        }
    }
    Ok(marked)
}

/// Runs the watched sync every 30 minutes.
pub async fn run(state: AppState) {
    loop {
        match sync_watched(&state).await {
            Ok(n) if n > 0 => tracing::info!(count = n, "marked films watched from Jellyfin"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "Jellyfin watched sync failed"),
        }
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::HeaderMap, http::StatusCode, routing::get, routing::post};
    use std::sync::{Arc, Mutex};

    #[test]
    fn paths_are_mapped_for_jellyfin() {
        let s = JellyfinSettings {
            path_from: "/data/movies".into(),
            path_to: "/media/films".into(),
            ..Default::default()
        };
        assert_eq!(s.map_path("/data/movies/Dune (2021)/Dune.mkv"), "/media/films/Dune (2021)/Dune.mkv");
        assert_eq!(s.map_path("/other/x.mkv"), "/other/x.mkv");
        assert_eq!(JellyfinSettings::default().map_path("/a"), "/a");
    }

    #[tokio::test]
    async fn refresh_and_watched_sync() {
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let s2 = seen.clone();
        let authed = |h: &HeaderMap| {
            h.get("authorization").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("Token=\"k\""))
        };
        let app = Router::new()
            .route(
                "/System/Info",
                get(move |h: HeaderMap| async move {
                    if !authed(&h) {
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                    Ok(Json(serde_json::json!({ "ServerName": "Home", "Version": "10.11.2" })))
                }),
            )
            .route(
                "/Library/Media/Updated",
                post(move |Json(b): Json<serde_json::Value>| {
                    let s2 = s2.clone();
                    async move {
                        s2.lock().unwrap().push(b["Updates"][0]["Path"].as_str().unwrap().to_owned());
                        StatusCode::NO_CONTENT
                    }
                }),
            )
            .route("/Users", get(|| async { Json(serde_json::json!([{ "Id": "u1", "Name": "Ana" }, { "Id": "u2", "Name": "Nobody" }])) }))
            .route(
                "/Items",
                get(|| async {
                    Json(serde_json::json!({ "Items": [{ "ProviderIds": { "Tmdb": "603" } }, { "ProviderIds": { "Imdb": "tt1" } }] }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let db = koda_db::memory().await;
        let config = crate::config::Config {
            config_dir: std::env::temp_dir(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        let settings = JellyfinSettings {
            enabled: true,
            url: base,
            api_key: "k".into(),
            path_from: "/data".into(),
            path_to: "/media".into(),
            sync_watched: true,
        };
        settings.save(&db).await.unwrap();
        let raw: serde_json::Value =
            koda_db::settings::get(&db, JellyfinSettings::KEY).await.unwrap().unwrap();
        assert!(raw["api_key"].as_str().unwrap().starts_with("enc1:"), "key stored sealed");

        let loaded = JellyfinSettings::load(&db).await.unwrap();
        let c = Client::new(&state.http, &loaded);
        assert_eq!(c.info().await.unwrap().1, "10.11.2");
        c.media_added("/data/Film (2020)/Film.mkv").await.unwrap();
        assert_eq!(seen.lock().unwrap().as_slice(), ["/media/Film (2020)/Film.mkv"]);

        let ana =
            koda_db::users::create(&db, "ana", "h", koda_core::Permissions::DEFAULT_USER).await.unwrap();
        let t = koda_db::titles::upsert_movie(
            &db,
            &koda_db::titles::NewTitle { tmdb_id: 603, title: "The Matrix".into(), ..Default::default() },
            koda_core::TitleState::Available,
        )
        .await
        .unwrap();
        assert_eq!(sync_watched(&state).await.unwrap(), 1);
        assert!(koda_db::library::progress(&db, ana.id, t.id).await.unwrap().1);
        assert_eq!(sync_watched(&state).await.unwrap(), 0, "already marked");
    }
}
