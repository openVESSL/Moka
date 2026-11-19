//! Transmission RPC client (`/transmission/rpc`).

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{ClientConfig, Download, Error, Status};

pub struct Transmission {
    cfg: ClientConfig,
    http: reqwest::Client,
    /// CSRF token Transmission hands out on the first 409.
    session: Mutex<Option<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Torrent {
    hash_string: String,
    name: String,
    status: i64,
    percent_done: f32,
    total_size: u64,
    #[serde(default)]
    eta: i64,
    #[serde(default)]
    upload_ratio: f32,
    #[serde(default)]
    download_dir: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    error: i64,
    #[serde(default)]
    error_string: String,
    #[serde(default)]
    is_finished: bool,
}

impl Transmission {
    pub fn new(cfg: ClientConfig) -> Self {
        let http =
            koda_net::client_builder().timeout(Duration::from_secs(20)).build().expect("http client builds");
        Self { cfg, http, session: Mutex::new(None) }
    }

    fn url(&self) -> String {
        let base = self.cfg.base_url.trim_end_matches('/');
        if base.ends_with("/rpc") { base.to_owned() } else { format!("{base}/transmission/rpc") }
    }

    async fn call(&self, method: &str, arguments: Value) -> Result<Value, Error> {
        let body = json!({ "method": method, "arguments": arguments });
        for _ in 0..2 {
            let mut req = self.http.post(self.url()).json(&body);
            if let Some(user) = self.cfg.username.as_deref().filter(|u| !u.is_empty()) {
                req = req.basic_auth(user, self.cfg.password.as_deref());
            }
            if let Some(s) = self.session.lock().await.clone() {
                req = req.header("X-Transmission-Session-Id", s);
            }
            let res = req.send().await?;
            match res.status().as_u16() {
                // first contact or expired token: take the new one and retry once
                409 => {
                    let token = res
                        .headers()
                        .get("X-Transmission-Session-Id")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    *self.session.lock().await = token;
                }
                401 | 403 => return Err(Error::Auth),
                200 => {
                    let v: Value = res.json().await?;
                    if v["result"] != "success" {
                        return Err(Error::Rejected);
                    }
                    return Ok(v["arguments"].clone());
                }
                s => return Err(Error::Status(s)),
            }
        }
        Err(Error::Status(409))
    }

    pub async fn test(&self) -> Result<String, Error> {
        let v = self.call("session-get", json!({ "fields": ["version"] })).await?;
        Ok(v["version"].as_str().unwrap_or("").to_owned())
    }

    /// Adds a magnet or a .torrent and returns its info hash.
    pub async fn add(&self, magnet: Option<&str>, torrent: Option<&[u8]>) -> Result<String, Error> {
        let mut args = json!({ "labels": [self.cfg.category] });
        match (magnet, torrent) {
            (Some(m), _) => args["filename"] = m.into(),
            (None, Some(bytes)) => {
                args["metainfo"] = base64::engine::general_purpose::STANDARD.encode(bytes).into()
            }
            (None, None) => return Err(Error::Rejected),
        }
        let v = self.call("torrent-add", args).await?;
        let t = v.get("torrent-added").or_else(|| v.get("torrent-duplicate")).ok_or(Error::Rejected)?;
        Ok(t["hashString"].as_str().unwrap_or("").to_ascii_lowercase())
    }

    pub async fn list(&self) -> Result<Vec<Download>, Error> {
        let fields = [
            "hashString",
            "name",
            "status",
            "percentDone",
            "totalSize",
            "eta",
            "uploadRatio",
            "downloadDir",
            "labels",
            "error",
            "errorString",
            "isFinished",
        ];
        let v = self.call("torrent-get", json!({ "fields": fields })).await?;
        let torrents: Vec<Torrent> = serde_json::from_value(v["torrents"].clone()).unwrap_or_default();
        Ok(torrents
            .into_iter()
            .filter(|t| t.labels.iter().any(|l| l == &self.cfg.category))
            .map(normalize)
            .collect())
    }

    pub async fn remove(&self, hash: &str, delete_files: bool) -> Result<(), Error> {
        self.call("torrent-remove", json!({ "ids": [hash], "delete-local-data": delete_files }))
            .await
            .map(|_| ())
    }
}

fn normalize(t: Torrent) -> Download {
    // 0 stopped, 1-2 checking, 3 queued, 4 downloading, 5 queued to seed, 6 seeding
    let done = t.percent_done >= 1.0;
    let (status, message) = if t.error != 0 {
        (Status::Warning, Some(t.error_string.clone()).filter(|s| !s.is_empty()))
    } else {
        match (t.status, done) {
            (4, _) => (Status::Downloading, None),
            (5 | 6, _) | (0, true) => (Status::Completed, None),
            (0, false) => (Status::Paused, None),
            _ => (Status::Queued, None),
        }
    };
    Download {
        id: t.hash_string.to_ascii_lowercase(),
        content_path: (!t.download_dir.is_empty())
            .then(|| format!("{}/{}", t.download_dir.trim_end_matches('/'), t.name)),
        name: t.name,
        status,
        progress: t.percent_done.clamp(0.0, 1.0),
        size_bytes: t.total_size,
        eta_seconds: (t.eta > 0).then_some(t.eta),
        ratio: t.upload_ratio.max(0.0),
        can_move_files: t.status == 0 && t.is_finished,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::HeaderMap, http::StatusCode, response::IntoResponse, routing::post};

    #[tokio::test]
    async fn session_handshake_add_and_list() {
        let app = Router::new().route(
            "/transmission/rpc",
            post(|h: HeaderMap, Json(b): Json<Value>| async move {
                if h.get("x-transmission-session-id").and_then(|v| v.to_str().ok()) != Some("tok") {
                    return (StatusCode::CONFLICT, [("X-Transmission-Session-Id", "tok")]).into_response();
                }
                let args = match b["method"].as_str().unwrap() {
                    "torrent-add" => {
                        assert_eq!(b["arguments"]["labels"][0], "koda");
                        json!({ "torrent-added": { "hashString": "ABCDEF", "name": "Film" } })
                    }
                    "torrent-get" => json!({ "torrents": [
                        { "hashString": "ABCDEF", "name": "Film", "status": 4, "percentDone": 0.5, "totalSize": 100,
                          "eta": 60, "downloadDir": "/dl", "labels": ["koda"] },
                        { "hashString": "123", "name": "Done", "status": 6, "percentDone": 1.0, "totalSize": 9,
                          "downloadDir": "/dl", "labels": ["koda"] },
                        { "hashString": "999", "name": "Not ours", "status": 4, "percentDone": 0.1, "totalSize": 1,
                          "labels": ["other"] }
                    ]}),
                    _ => json!({}),
                };
                Json(json!({ "result": "success", "arguments": args })).into_response()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let t = Transmission::new(ClientConfig {
            base_url: base,
            username: None,
            password: None,
            category: "koda".into(),
        });
        assert_eq!(t.add(Some("magnet:?xt=urn:btih:abcdef"), None).await.unwrap(), "abcdef");
        let list = t.list().await.unwrap();
        assert_eq!(list.len(), 2, "only our label");
        assert_eq!(
            (list[0].status, list[0].content_path.as_deref()),
            (Status::Downloading, Some("/dl/Film"))
        );
        assert_eq!(list[1].status, Status::Completed);
    }
}
