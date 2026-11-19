//! Download clients. qBittorrent first; the [`Download`] shape is what any
//! client gets normalized to.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Where a download stands, in our terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Paused,
    Downloading,
    /// All data is on disk. It may still be seeding.
    Completed,
    /// Something needs attention (stalled, missing files, client error).
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Download {
    /// Lowercase info hash.
    pub hash: String,
    pub name: String,
    pub status: Status,
    /// 0.0 to 1.0
    pub progress: f32,
    pub size_bytes: u64,
    pub eta_seconds: Option<i64>,
    pub ratio: f32,
    /// Folder or file holding the content, as the client sees it.
    pub content_path: Option<String>,
    /// The client has stopped seeding, so files may be moved instead of copied.
    pub can_move_files: bool,
    pub message: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("qBittorrent rejected the username or password")]
    Auth,
    #[error("qBittorrent banned this address after too many failed logins; wait and retry")]
    Banned,
    #[error("qBittorrent did not accept the torrent")]
    Rejected,
    #[error("qBittorrent returned HTTP {0}")]
    Status(u16),
    #[error("could not reach qBittorrent: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Debug, Clone)]
pub struct QbitConfig {
    pub base_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub category: String,
}

/// qBittorrent Web API v2 client. Keeps its login cookie and logs in again
/// when the session expires.
pub struct Qbittorrent {
    cfg: QbitConfig,
    http: reqwest::Client,
    logged_in: Mutex<bool>,
}

#[derive(Deserialize)]
struct Torrent {
    hash: String,
    name: String,
    state: String,
    progress: f32,
    size: u64,
    #[serde(default)]
    eta: i64,
    #[serde(default)]
    ratio: f32,
    #[serde(default)]
    save_path: String,
    #[serde(default)]
    content_path: String,
    #[serde(default)]
    root_path: Option<String>,
}

impl Qbittorrent {
    pub fn new(cfg: QbitConfig) -> Self {
        let http = koda_net::client_builder()
            .cookie_store(true)
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http client builds");
        Self { cfg, http, logged_in: Mutex::new(false) }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v2/{path}", self.cfg.base_url.trim_end_matches('/'))
    }

    async fn login(&self) -> Result<(), Error> {
        let form = [
            ("username", self.cfg.username.clone().unwrap_or_default()),
            ("password", self.cfg.password.clone().unwrap_or_default()),
        ];
        // qBittorrent checks Referer/Origin against its own host (CSRF protection)
        let res = self
            .http
            .post(self.url("auth/login"))
            .header(reqwest::header::REFERER, &self.cfg.base_url)
            .form(&form)
            .send()
            .await?;
        match res.status().as_u16() {
            200 => {}
            403 => return Err(Error::Banned),
            s => return Err(Error::Status(s)),
        }
        // wrong credentials still answer 200, with "Fails."
        if res.text().await?.trim() != "Ok." {
            return Err(Error::Auth);
        }
        *self.logged_in.lock().await = true;
        Ok(())
    }

    /// Sends a request built by `make`, logging in first if needed and once
    /// more if the session expired (403).
    async fn send(&self, make: impl Fn() -> reqwest::RequestBuilder) -> Result<reqwest::Response, Error> {
        if !*self.logged_in.lock().await {
            self.login().await?;
        }
        let res = make().header(reqwest::header::REFERER, &self.cfg.base_url).send().await?;
        if res.status().as_u16() != 403 {
            return Ok(res);
        }
        self.login().await?;
        Ok(make().header(reqwest::header::REFERER, &self.cfg.base_url).send().await?)
    }

    /// Checks the connection and returns the client version.
    pub async fn test(&self) -> Result<String, Error> {
        let res = self.send(|| self.http.get(self.url("app/version"))).await?;
        if !res.status().is_success() {
            return Err(Error::Status(res.status().as_u16()));
        }
        Ok(res.text().await?.trim().to_owned())
    }

    pub async fn ensure_category(&self) -> Result<(), Error> {
        let form = [("category", self.cfg.category.as_str()), ("savePath", "")];
        let res = self.send(|| self.http.post(self.url("torrents/createCategory")).form(&form)).await?;
        // 409: it already exists
        match res.status().as_u16() {
            200 | 409 => Ok(()),
            s => Err(Error::Status(s)),
        }
    }

    pub async fn add_magnet(&self, magnet: &str) -> Result<(), Error> {
        self.ensure_category().await?;
        let make = || {
            let form = reqwest::multipart::Form::new()
                .text("urls", magnet.to_owned())
                .text("category", self.cfg.category.clone());
            self.http.post(self.url("torrents/add")).multipart(form)
        };
        check_added(self.send(make).await?).await
    }

    pub async fn add_torrent_file(&self, bytes: &[u8], name: &str) -> Result<(), Error> {
        self.ensure_category().await?;
        let make = || {
            let part = reqwest::multipart::Part::bytes(bytes.to_vec())
                .file_name(format!("{}.torrent", safe_file_name(name)))
                .mime_str("application/x-bittorrent")
                .expect("valid mime");
            let form = reqwest::multipart::Form::new()
                .part("torrents", part)
                .text("category", self.cfg.category.clone());
            self.http.post(self.url("torrents/add")).multipart(form)
        };
        check_added(self.send(make).await?).await
    }

    /// Torrents in our category.
    pub async fn list(&self) -> Result<Vec<Download>, Error> {
        let res = self
            .send(|| {
                self.http.get(self.url("torrents/info")).query(&[("category", self.cfg.category.as_str())])
            })
            .await?;
        if !res.status().is_success() {
            return Err(Error::Status(res.status().as_u16()));
        }
        let torrents: Vec<Torrent> = res.json().await?;
        Ok(torrents.into_iter().map(normalize).collect())
    }

    pub async fn remove(&self, hash: &str, delete_files: bool) -> Result<(), Error> {
        let form = [("hashes", hash.to_owned()), ("deleteFiles", delete_files.to_string())];
        let res = self.send(|| self.http.post(self.url("torrents/delete")).form(&form)).await?;
        if res.status().is_success() { Ok(()) } else { Err(Error::Status(res.status().as_u16())) }
    }
}

async fn check_added(res: reqwest::Response) -> Result<(), Error> {
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    match (status, body.trim()) {
        (200, "Fails.") => Err(Error::Rejected),
        (200, _) => Ok(()),
        (415, _) => Err(Error::Rejected),
        (s, _) => Err(Error::Status(s)),
    }
}

fn safe_file_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    cleaned.chars().take(120).collect()
}

fn normalize(t: Torrent) -> Download {
    let (status, message) = match t.state.as_str() {
        "downloading" | "forcedDL" | "moving" => (Status::Downloading, None),
        "uploading" | "stalledUP" | "queuedUP" | "forcedUP" | "pausedUP" | "stoppedUP" | "checkingUP" => {
            (Status::Completed, None)
        }
        "pausedDL" | "stoppedDL" => (Status::Paused, None),
        "queuedDL" | "checkingDL" | "checkingResumeData" | "allocating" => (Status::Queued, None),
        "metaDL" | "forcedMetaDL" => (Status::Queued, Some("fetching metadata".to_owned())),
        "stalledDL" => (Status::Warning, Some("stalled: no peers are sending data".to_owned())),
        "missingFiles" => (Status::Warning, Some("files are missing on disk".to_owned())),
        "error" => (Status::Warning, Some("qBittorrent reported an error".to_owned())),
        other => (Status::Warning, Some(format!("unknown state {other}"))),
    };
    // checkingUP means "verifying after completion": not ready to import yet
    let status = if t.state == "checkingUP" { Status::Queued } else { status };
    let content_path = t
        .root_path
        .filter(|p| !p.is_empty() && *p != t.save_path)
        .or_else(|| (!t.content_path.is_empty()).then_some(t.content_path));
    Download {
        hash: t.hash.to_ascii_lowercase(),
        name: t.name,
        progress: t.progress.clamp(0.0, 1.0),
        size_bytes: t.size,
        // 8640000 is qBittorrent's "infinity"
        eta_seconds: (t.eta > 0 && t.eta < 8_640_000).then_some(t.eta),
        ratio: t.ratio,
        can_move_files: matches!(t.state.as_str(), "pausedUP" | "stoppedUP"),
        content_path,
        status,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Form, Router,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn mock(logins: Arc<AtomicUsize>) -> String {
        let l = logins.clone();
        let app = Router::new()
            .route(
                "/api/v2/auth/login",
                post(move |Form(f): Form<HashMap<String, String>>| {
                    let l = l.clone();
                    async move {
                        l.fetch_add(1, Ordering::SeqCst);
                        if f.get("password").map(String::as_str) == Some("secret") {
                            ([("set-cookie", "SID=abc; path=/")], "Ok.").into_response()
                        } else {
                            "Fails.".into_response()
                        }
                    }
                }),
            )
            .route(
                "/api/v2/app/version",
                get(|h: HeaderMap| async move {
                    let ok = h.get("cookie").and_then(|c| c.to_str().ok()).is_some_and(|c| c.contains("SID=abc"));
                    if ok { "v5.0.1".into_response() } else { StatusCode::FORBIDDEN.into_response() }
                }),
            )
            .route("/api/v2/torrents/createCategory", post(|| async { StatusCode::CONFLICT }))
            .route("/api/v2/torrents/add", post(|| async { "Ok." }))
            .route(
                "/api/v2/torrents/info",
                get(|| async {
                    axum::Json(serde_json::json!([
                        { "hash": "ABCDEF", "name": "Film", "state": "downloading", "progress": 0.42,
                          "size": 1000, "eta": 120, "ratio": 0.0, "save_path": "/dl", "content_path": "/dl/Film" },
                        { "hash": "123456", "name": "Done", "state": "stoppedUP", "progress": 1.0,
                          "size": 2000, "eta": 8640000, "ratio": 1.2, "save_path": "/dl", "content_path": "/dl/Done.mkv",
                          "root_path": "" },
                        { "hash": "777777", "name": "Stuck", "state": "stalledDL", "progress": 0.1, "size": 10 }
                    ]))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn client(base: &str, password: &str) -> Qbittorrent {
        Qbittorrent::new(QbitConfig {
            base_url: base.into(),
            username: Some("admin".into()),
            password: Some(password.into()),
            category: "koda".into(),
        })
    }

    #[tokio::test]
    async fn logs_in_once_and_lists() {
        let logins = Arc::new(AtomicUsize::new(0));
        let base = mock(logins.clone()).await;
        let q = client(&base, "secret");
        assert_eq!(q.test().await.unwrap(), "v5.0.1");
        q.add_magnet("magnet:?xt=urn:btih:abc").await.unwrap();
        let list = q.list().await.unwrap();
        assert_eq!(logins.load(Ordering::SeqCst), 1, "session cookie is reused");

        assert_eq!(list[0].hash, "abcdef");
        assert_eq!(list[0].status, Status::Downloading);
        assert_eq!(list[0].eta_seconds, Some(120));
        assert!(!list[0].can_move_files);

        assert_eq!(list[1].status, Status::Completed);
        assert!(list[1].can_move_files);
        assert_eq!(list[1].eta_seconds, None);
        assert_eq!(list[1].content_path.as_deref(), Some("/dl/Done.mkv"));

        assert_eq!(list[2].status, Status::Warning);
        assert!(list[2].message.as_deref().unwrap().contains("stalled"));
    }

    #[tokio::test]
    async fn wrong_password_is_auth_error() {
        let base = mock(Arc::new(AtomicUsize::new(0))).await;
        assert!(matches!(client(&base, "nope").test().await, Err(Error::Auth)));
    }
}
