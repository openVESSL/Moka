use std::sync::Arc;

use serde::Serialize;
use tokio::sync::broadcast;

use crate::config::Config;

/// Shared state handed to every request handler and background task.
#[derive(Clone)]
pub struct AppState {
    pub db: koda_db::Pool,
    pub config: Arc<Config>,
    pub events: broadcast::Sender<LiveEvent>,
}

impl AppState {
    pub fn new(db: koda_db::Pool, config: Config) -> Self {
        let (events, _) = broadcast::channel(256);
        Self { db, config: Arc::new(config), events }
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
