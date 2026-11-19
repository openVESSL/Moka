//! SABnzbd client (Usenet). Uses the JSON API with the API key; the
//! configured password field holds that key.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::{ClientConfig, Download, Error, Status};

pub struct Sabnzbd {
    cfg: ClientConfig,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct QueueSlot {
    nzo_id: String,
    filename: String,
    status: String,
    #[serde(default)]
    percentage: String,
    #[serde(default)]
    mb: String,
    #[serde(default)]
    cat: String,
    #[serde(default)]
    timeleft: String,
}

#[derive(Deserialize)]
struct HistorySlot {
    nzo_id: String,
    name: String,
    status: String,
    #[serde(default)]
    storage: String,
    #[serde(default)]
    fail_message: String,
    #[serde(default)]
    bytes: u64,
    #[serde(default)]
    category: String,
}

impl Sabnzbd {
    pub fn new(cfg: ClientConfig) -> Self {
        let http =
            koda_net::client_builder().timeout(Duration::from_secs(30)).build().expect("http client builds");
        Self { cfg, http }
    }

    fn api(&self, mode: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/api", self.cfg.base_url.trim_end_matches('/'));
        self.http.get(url).query(&[
            ("mode", mode),
            ("output", "json"),
            ("apikey", self.cfg.password.as_deref().unwrap_or("")),
        ])
    }

    async fn json(req: reqwest::RequestBuilder) -> Result<Value, Error> {
        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(Error::Status(res.status().as_u16()));
        }
        let v: Value = res.json().await?;
        // SABnzbd answers 200 with {"status": false, "error": "API Key Incorrect"}
        if v["status"] == false {
            let err = v["error"].as_str().unwrap_or("");
            return Err(if err.to_ascii_lowercase().contains("key") { Error::Auth } else { Error::Rejected });
        }
        Ok(v)
    }

    pub async fn test(&self) -> Result<String, Error> {
        // "version" needs no key, "queue" does: use it to check the key too
        Self::json(self.api("queue")).await?;
        let v = Self::json(self.api("version")).await?;
        Ok(v["version"].as_str().unwrap_or("").to_owned())
    }

    /// Uploads an .nzb and returns SABnzbd's job id.
    pub async fn add_nzb(&self, bytes: &[u8], name: &str) -> Result<String, Error> {
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(format!("{}.nzb", name.replace(['/', '\\'], "_")))
            .mime_str("application/x-nzb")
            .expect("valid mime");
        let form = reqwest::multipart::Form::new().part("name", part);
        let url = format!("{}/api", self.cfg.base_url.trim_end_matches('/'));
        let req = self
            .http
            .post(url)
            .query(&[
                ("mode", "addfile"),
                ("output", "json"),
                ("cat", self.cfg.category.as_str()),
                ("apikey", self.cfg.password.as_deref().unwrap_or("")),
            ])
            .multipart(form);
        let v = Self::json(req).await?;
        v["nzo_ids"][0].as_str().map(str::to_owned).ok_or(Error::Rejected)
    }

    pub async fn list(&self) -> Result<Vec<Download>, Error> {
        let q = Self::json(self.api("queue")).await?;
        let h = Self::json(self.api("history").query(&[("limit", "100")])).await?;
        let queue: Vec<QueueSlot> = serde_json::from_value(q["queue"]["slots"].clone()).unwrap_or_default();
        let history: Vec<HistorySlot> =
            serde_json::from_value(h["history"]["slots"].clone()).unwrap_or_default();
        let ours = |c: &str| c.eq_ignore_ascii_case(&self.cfg.category);

        let mut out: Vec<Download> = queue
            .into_iter()
            .filter(|s| ours(&s.cat))
            .map(|s| {
                let progress = s.percentage.parse::<f32>().unwrap_or(0.0) / 100.0;
                let size = (s.mb.parse::<f64>().unwrap_or(0.0) * 1_048_576.0) as u64;
                Download {
                    id: s.nzo_id,
                    name: s.filename,
                    status: match s.status.as_str() {
                        "Downloading" | "Fetching" | "Grabbing" => Status::Downloading,
                        "Paused" => Status::Paused,
                        _ => Status::Queued,
                    },
                    progress,
                    size_bytes: size,
                    eta_seconds: parse_hms(&s.timeleft),
                    ratio: 0.0,
                    content_path: None,
                    can_move_files: false,
                    message: None,
                }
            })
            .collect();
        out.extend(history.into_iter().filter(|s| ours(&s.category)).map(|s| {
            let (status, message) = match s.status.as_str() {
                "Completed" => (Status::Completed, None),
                "Failed" => (Status::Failed, Some(s.fail_message.clone()).filter(|m| !m.is_empty())),
                // verifying, repairing, extracting: not ready yet
                _ => (Status::Queued, Some(format!("{} in SABnzbd", s.status.to_lowercase()))),
            };
            Download {
                id: s.nzo_id,
                name: s.name,
                status,
                progress: if status == Status::Completed { 1.0 } else { 0.99 },
                size_bytes: s.bytes,
                eta_seconds: None,
                ratio: 0.0,
                content_path: (!s.storage.is_empty()).then_some(s.storage),
                // Usenet downloads do not seed: the files can always be moved
                can_move_files: true,
                message,
            }
        }));
        Ok(out)
    }

    pub async fn remove(&self, id: &str, delete_files: bool) -> Result<(), Error> {
        for mode in ["queue", "history"] {
            Self::json(self.api(mode).query(&[
                ("name", "delete"),
                ("value", id),
                ("del_files", if delete_files { "1" } else { "0" }),
            ]))
            .await?;
        }
        Ok(())
    }
}

/// "0:12:34" -> 754 seconds.
fn parse_hms(s: &str) -> Option<i64> {
    let parts: Vec<i64> = s.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let secs = parts.iter().fold(0, |acc, p| acc * 60 + p);
    (secs > 0).then_some(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Query, routing::get, routing::post};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn time_left() {
        assert_eq!(parse_hms("0:12:34"), Some(754));
        assert_eq!(parse_hms("0:00:00"), None);
        assert_eq!(parse_hms("soon"), None);
    }

    #[tokio::test]
    async fn add_and_follow_jobs() {
        let api = |Query(q): Query<HashMap<String, String>>| async move {
            if q.get("apikey").map(String::as_str) != Some("key") {
                return Json(json!({ "status": false, "error": "API Key Incorrect" }));
            }
            Json(match q["mode"].as_str() {
                "version" => json!({ "version": "4.4.0" }),
                "addfile" => json!({ "status": true, "nzo_ids": ["SABnzbd_nzo_1"] }),
                "queue" => json!({ "queue": { "slots": [
                    { "nzo_id": "SABnzbd_nzo_1", "filename": "Film", "status": "Downloading", "percentage": "40",
                      "mb": "1000", "cat": "koda", "timeleft": "0:05:00" },
                    { "nzo_id": "x", "filename": "Other", "status": "Queued", "cat": "tv" } ] } }),
                "history" => json!({ "history": { "slots": [
                    { "nzo_id": "SABnzbd_nzo_0", "name": "Old Film", "status": "Completed", "storage": "/dl/Old Film",
                      "bytes": 5, "category": "koda" },
                    { "nzo_id": "SABnzbd_nzo_9", "name": "Broken", "status": "Failed", "fail_message": "Unpacking failed",
                      "category": "koda" } ] } }),
                _ => json!({ "status": true }),
            })
        };
        let app = Router::new().route("/api", get(api).merge(post(api)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let cfg = |key: &str| ClientConfig {
            base_url: base.clone(),
            username: None,
            password: Some(key.into()),
            category: "koda".into(),
        };
        assert!(matches!(Sabnzbd::new(cfg("bad")).test().await, Err(Error::Auth)));
        let s = Sabnzbd::new(cfg("key"));
        assert_eq!(s.test().await.unwrap(), "4.4.0");
        assert_eq!(s.add_nzb(b"<nzb/>", "Film").await.unwrap(), "SABnzbd_nzo_1");
        let list = s.list().await.unwrap();
        assert_eq!(list.len(), 3, "other categories are ignored");
        assert_eq!((list[0].status, list[0].eta_seconds), (Status::Downloading, Some(300)));
        assert_eq!(
            (list[1].status, list[1].content_path.as_deref()),
            (Status::Completed, Some("/dl/Old Film"))
        );
        assert_eq!(list[2].status, Status::Failed);
        assert_eq!(list[2].message.as_deref(), Some("Unpacking failed"));
    }
}
