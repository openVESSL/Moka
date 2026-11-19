use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use koda_metadata::Tmdb;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::config::Config;

/// Shared state handed to every request handler and background task.
#[derive(Clone)]
pub struct AppState {
    pub db: koda_db::Pool,
    pub config: Arc<Config>,
    pub events: broadcast::Sender<LiveEvent>,
    pub http: reqwest::Client,
    pub indexers: Arc<koda_indexers::Client>,
    /// Last search per title, kept so the UI can list releases and grab one by hand.
    pub searches: Arc<Mutex<HashMap<i64, crate::pipeline::SearchResult>>>,
    /// Wakes the background loop early (new request, manual action).
    pub wake: Arc<tokio::sync::Notify>,
    /// Download client connections by id, kept so login sessions are reused.
    pub download_clients: Arc<Mutex<HashMap<i64, Arc<koda_download::Qbittorrent>>>>,
    /// Failed logins per account name: count and time of the last failure.
    pub login_failures: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
    /// Progress of the running or last library scan.
    pub scan: Arc<Mutex<crate::scan::ScanStatus>>,
    pub streams: crate::streaming::Sessions,
    tmdb: Arc<RwLock<Tmdb>>,
    cache: Arc<Mutex<HashMap<String, (Instant, serde_json::Value)>>>,
}

/// Catalog settings stored under the `catalog` key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalogSettings {
    #[serde(default)]
    pub tmdb_key: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub region: Option<String>,
}

fn default_language() -> String {
    "en-US".into()
}

impl CatalogSettings {
    pub const KEY: &'static str = "catalog";

    pub fn client(&self) -> Tmdb {
        Tmdb::new(self.tmdb_key.clone(), self.language.clone(), self.region.clone())
    }
}

impl AppState {
    pub fn new(db: koda_db::Pool, config: Config, catalog: CatalogSettings) -> Self {
        let (events, _) = broadcast::channel(256);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("koda/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client builds");
        Self {
            db,
            config: Arc::new(config),
            events,
            http,
            indexers: Arc::new(koda_indexers::Client::new()),
            searches: Arc::default(),
            wake: Arc::default(),
            download_clients: Arc::default(),
            login_failures: Arc::default(),
            scan: Arc::default(),
            streams: Default::default(),
            tmdb: Arc::new(RwLock::new(catalog.client())),
            cache: Arc::default(),
        }
    }

    pub fn tmdb(&self) -> Tmdb {
        self.tmdb.read().expect("tmdb lock").clone()
    }

    pub fn set_tmdb(&self, client: Tmdb) {
        *self.tmdb.write().expect("tmdb lock") = client;
        self.cache.lock().expect("cache lock").clear();
    }

    /// Returns a cached JSON value younger than `ttl`, or computes and stores it.
    // shortcut: in-memory only and unbounded by count; entries expire by age. Move to SQLite if memory matters on small devices.
    pub async fn cached<T, F, Fut, E>(&self, key: String, ttl: Duration, f: F) -> Result<T, E>
    where
        T: Serialize + serde::de::DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        {
            let mut cache = self.cache.lock().expect("cache lock");
            cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(6 * 3600));
            if let Some((at, value)) = cache.get(&key)
                && at.elapsed() < ttl
                && let Ok(v) = serde_json::from_value(value.clone())
            {
                return Ok(v);
            }
        }
        let value = f().await?;
        if let Ok(json) = serde_json::to_value(&value) {
            self.cache.lock().expect("cache lock").insert(key, (Instant::now(), json));
        }
        Ok(value)
    }

    /// Fire-and-forget: no listeners is fine.
    pub fn publish(&self, event: LiveEvent) {
        let _ = self.events.send(event);
    }
}

/// Pushed to browsers over SSE. Not persisted.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    TitleState { title_id: i64, state: koda_core::TitleState, reason: Option<String> },
    DownloadProgress { title_id: i64, progress: f32, eta_seconds: Option<i64> },
    RequestCreated { request_id: i64, title_id: i64 },
    RequestDecided { request_id: i64, title_id: i64, approved: bool },
}
