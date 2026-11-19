//! Indexers and download clients the user connected.

use serde::Serialize;

use crate::{Error, Pool, now};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Indexer {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub api_path: String,
    #[serde(skip)]
    pub api_key: Option<String>,
    /// JSON array of category ids.
    pub categories: String,
    pub priority: i64,
    pub enabled: bool,
    /// JSON capabilities from the last check.
    pub capabilities: Option<String>,
    pub failure_level: i64,
    pub disabled_until: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IndexerInput {
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub api_path: String,
    /// `None` keeps the stored key on update.
    pub api_key: Option<String>,
    pub categories: Vec<u32>,
    pub priority: i64,
    pub enabled: bool,
}

macro_rules! select_indexers {
    ($rest:literal) => {
        concat!(
            "SELECT i.id, i.name, i.kind, i.base_url, i.api_path, i.api_key, i.categories, i.priority, \
             i.enabled, i.capabilities, COALESCE(s.failure_level, 0) AS failure_level, s.disabled_until, \
             s.last_error FROM indexers i LEFT JOIN indexer_status s ON s.indexer_id = i.id ",
            $rest
        )
    };
}

pub async fn indexers(pool: &Pool) -> Result<Vec<Indexer>, Error> {
    let mut rows: Vec<Indexer> =
        sqlx::query_as(select_indexers!("ORDER BY i.priority, i.name")).fetch_all(pool).await?;
    for r in &mut rows {
        r.api_key = crate::secret::open_opt(r.api_key.take());
    }
    Ok(rows)
}

pub async fn indexer(pool: &Pool, id: i64) -> Result<Option<Indexer>, Error> {
    let row: Option<Indexer> =
        sqlx::query_as(select_indexers!("WHERE i.id = ?")).bind(id).fetch_optional(pool).await?;
    Ok(row.map(|mut r| {
        r.api_key = crate::secret::open_opt(r.api_key.take());
        r
    }))
}

pub async fn create_indexer(pool: &Pool, i: &IndexerInput) -> Result<i64, Error> {
    let cats = serde_json::to_string(&i.categories).unwrap_or_else(|_| "[]".into());
    sqlx::query_scalar(
        "INSERT INTO indexers (name, kind, base_url, api_path, api_key, categories, priority, enabled)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&i.name)
    .bind(&i.kind)
    .bind(&i.base_url)
    .bind(&i.api_path)
    .bind(crate::secret::seal_opt(i.api_key.as_deref()))
    .bind(cats)
    .bind(i.priority)
    .bind(i.enabled)
    .fetch_one(pool)
    .await
}

pub async fn update_indexer(pool: &Pool, id: i64, i: &IndexerInput) -> Result<(), Error> {
    let cats = serde_json::to_string(&i.categories).unwrap_or_else(|_| "[]".into());
    sqlx::query(
        "UPDATE indexers SET name = ?, kind = ?, base_url = ?, api_path = ?, api_key = COALESCE(?, api_key),
         categories = ?, priority = ?, enabled = ? WHERE id = ?",
    )
    .bind(&i.name)
    .bind(&i.kind)
    .bind(&i.base_url)
    .bind(&i.api_path)
    .bind(crate::secret::seal_opt(i.api_key.as_deref()))
    .bind(cats)
    .bind(i.priority)
    .bind(i.enabled)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_indexer(pool: &Pool, id: i64) -> Result<(), Error> {
    sqlx::query("DELETE FROM indexers WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

pub async fn set_capabilities(pool: &Pool, id: i64, caps_json: &str) -> Result<(), Error> {
    sqlx::query("UPDATE indexers SET capabilities = ? WHERE id = ?")
        .bind(caps_json)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn indexer_ok(pool: &Pool, id: i64) -> Result<(), Error> {
    sqlx::query("DELETE FROM indexer_status WHERE indexer_id = ?").bind(id).execute(pool).await?;
    Ok(())
}

/// Records a failure. `pause_seconds` decides when the indexer is used again;
/// `None` means "until the user fixes it".
pub async fn indexer_failed(
    pool: &Pool,
    id: i64,
    error: &str,
    pause_seconds: Option<i64>,
) -> Result<i64, Error> {
    let until = pause_seconds.map(|s| now() + s).unwrap_or(i64::MAX);
    sqlx::query_scalar(
        "INSERT INTO indexer_status (indexer_id, failure_level, disabled_until, last_error, last_failure_at)
         VALUES (?1, 1, ?2, ?3, ?4)
         ON CONFLICT(indexer_id) DO UPDATE SET failure_level = failure_level + 1, disabled_until = ?2,
            last_error = ?3, last_failure_at = ?4
         RETURNING failure_level",
    )
    .bind(id)
    .bind(until)
    .bind(error)
    .bind(now())
    .fetch_one(pool)
    .await
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DownloadClient {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub username: Option<String>,
    #[serde(skip)]
    pub password: Option<String>,
    pub category: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct DownloadClientInput {
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub username: Option<String>,
    /// `None` keeps the stored password on update.
    pub password: Option<String>,
    pub category: String,
    pub enabled: bool,
}

pub async fn download_clients(pool: &Pool) -> Result<Vec<DownloadClient>, Error> {
    sqlx::query_as(
        "SELECT id, name, kind, base_url, username, password, category, enabled FROM download_clients ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map(|rows: Vec<DownloadClient>| rows.into_iter().map(open_client).collect())
}

fn open_client(mut c: DownloadClient) -> DownloadClient {
    c.password = crate::secret::open_opt(c.password.take());
    c
}

pub async fn download_client(pool: &Pool, id: i64) -> Result<Option<DownloadClient>, Error> {
    sqlx::query_as(
        "SELECT id, name, kind, base_url, username, password, category, enabled FROM download_clients WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map(|row: Option<DownloadClient>| row.map(open_client))
}

pub async fn create_download_client(pool: &Pool, c: &DownloadClientInput) -> Result<i64, Error> {
    sqlx::query_scalar(
        "INSERT INTO download_clients (name, kind, base_url, username, password, category, enabled)
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&c.name)
    .bind(&c.kind)
    .bind(&c.base_url)
    .bind(&c.username)
    .bind(crate::secret::seal_opt(c.password.as_deref()))
    .bind(&c.category)
    .bind(c.enabled)
    .fetch_one(pool)
    .await
}

pub async fn update_download_client(pool: &Pool, id: i64, c: &DownloadClientInput) -> Result<(), Error> {
    sqlx::query(
        "UPDATE download_clients SET name = ?, kind = ?, base_url = ?, username = ?,
         password = COALESCE(?, password), category = ?, enabled = ? WHERE id = ?",
    )
    .bind(&c.name)
    .bind(&c.kind)
    .bind(&c.base_url)
    .bind(&c.username)
    .bind(crate::secret::seal_opt(c.password.as_deref()))
    .bind(&c.category)
    .bind(c.enabled)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_download_client(pool: &Pool, id: i64) -> Result<(), Error> {
    sqlx::query("DELETE FROM download_clients WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> IndexerInput {
        IndexerInput {
            name: "Local".into(),
            kind: "torznab".into(),
            base_url: "http://idx".into(),
            api_path: "/api".into(),
            api_key: Some("k1".into()),
            categories: vec![2000, 2040],
            priority: 25,
            enabled: true,
        }
    }

    #[tokio::test]
    async fn indexer_update_keeps_key_and_tracks_failures() {
        let pool = crate::memory().await;
        let id = create_indexer(&pool, &input()).await.unwrap();
        update_indexer(&pool, id, &IndexerInput { api_key: None, name: "Renamed".into(), ..input() })
            .await
            .unwrap();
        let i = indexer(&pool, id).await.unwrap().unwrap();
        assert_eq!(i.name, "Renamed");
        assert_eq!(i.api_key.as_deref(), Some("k1"));
        let raw: String = sqlx::query_scalar("SELECT api_key FROM indexers WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(raw.starts_with("enc1:") && !raw.contains("k1"), "stored sealed: {raw}");
        assert_eq!(i.categories, "[2000,2040]");

        assert_eq!(indexer_failed(&pool, id, "timeout", Some(60)).await.unwrap(), 1);
        assert_eq!(indexer_failed(&pool, id, "timeout", Some(300)).await.unwrap(), 2);
        let i = indexer(&pool, id).await.unwrap().unwrap();
        assert_eq!((i.failure_level, i.last_error.as_deref()), (2, Some("timeout")));
        indexer_ok(&pool, id).await.unwrap();
        assert_eq!(indexer(&pool, id).await.unwrap().unwrap().failure_level, 0);
    }
}
