//! Releases sent to a download client, and what became of them.

use serde::Serialize;

use crate::{Error, Pool, now};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Grab {
    pub id: i64,
    pub title_id: i64,
    pub indexer_id: Option<i64>,
    pub download_client_id: Option<i64>,
    pub release_title: String,
    pub size_bytes: Option<i64>,
    pub quality: Option<String>,
    pub info_hash: Option<String>,
    /// sent | downloading | completed | imported | failed
    pub status: String,
    pub error: Option<String>,
    pub output_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewGrab {
    pub title_id: i64,
    pub indexer_id: Option<i64>,
    pub download_client_id: i64,
    pub release_title: String,
    pub size_bytes: Option<i64>,
    pub quality: Option<String>,
    pub info_hash: String,
}

pub async fn create(pool: &Pool, g: &NewGrab) -> Result<i64, Error> {
    sqlx::query_scalar(
        "INSERT INTO grabs (title_id, indexer_id, download_client_id, release_title, size_bytes, quality, info_hash, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'sent') RETURNING id",
    )
    .bind(g.title_id)
    .bind(g.indexer_id)
    .bind(g.download_client_id)
    .bind(&g.release_title)
    .bind(g.size_bytes)
    .bind(&g.quality)
    .bind(&g.info_hash)
    .fetch_one(pool)
    .await
}

/// Grabs still in flight (not imported, not failed).
pub async fn active(pool: &Pool) -> Result<Vec<Grab>, Error> {
    sqlx::query_as("SELECT * FROM grabs WHERE status IN ('sent', 'downloading', 'completed') ORDER BY id")
        .fetch_all(pool)
        .await
}

pub async fn latest_for_title(pool: &Pool, title_id: i64) -> Result<Option<Grab>, Error> {
    sqlx::query_as("SELECT * FROM grabs WHERE title_id = ? ORDER BY id DESC LIMIT 1")
        .bind(title_id)
        .fetch_optional(pool)
        .await
}

pub async fn set_status(
    pool: &Pool,
    id: i64,
    status: &str,
    error: Option<&str>,
    output_path: Option<&str>,
) -> Result<(), Error> {
    sqlx::query(
        "UPDATE grabs SET status = ?, error = ?, output_path = COALESCE(?, output_path), updated_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(error)
    .bind(output_path)
    .bind(now())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn blocklist(
    pool: &Pool,
    title_id: i64,
    release_title: &str,
    info_hash: Option<&str>,
    reason: &str,
) -> Result<(), Error> {
    sqlx::query("INSERT INTO blocklist (title_id, release_title, info_hash, reason) VALUES (?, ?, ?, ?)")
        .bind(title_id)
        .bind(release_title)
        .bind(info_hash)
        .bind(reason)
        .execute(pool)
        .await?;
    Ok(())
}

/// Release titles and hashes blocklisted for a title.
pub async fn blocklisted(pool: &Pool, title_id: i64) -> Result<Vec<(String, Option<String>)>, Error> {
    sqlx::query_as("SELECT release_title, info_hash FROM blocklist WHERE title_id = ?")
        .bind(title_id)
        .fetch_all(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::titles::{NewTitle, upsert_movie};
    use koda_core::TitleState;

    #[tokio::test]
    async fn grab_lifecycle() {
        let pool = crate::memory().await;
        let t = upsert_movie(
            &pool,
            &NewTitle { tmdb_id: 1, title: "X".into(), ..Default::default() },
            TitleState::Wanted,
        )
        .await
        .unwrap();
        let client = sqlx::query_scalar::<_, i64>(
            "INSERT INTO download_clients (name, kind, base_url) VALUES ('q', 'qbittorrent', 'http://q') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let id = create(
            &pool,
            &NewGrab {
                title_id: t.id,
                indexer_id: None,
                download_client_id: client,
                release_title: "X.2020.1080p".into(),
                size_bytes: Some(10),
                quality: Some("webdl-1080p".into()),
                info_hash: "abc".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(active(&pool).await.unwrap().len(), 1);
        set_status(&pool, id, "failed", Some("stalled"), None).await.unwrap();
        assert!(active(&pool).await.unwrap().is_empty());
        blocklist(&pool, t.id, "X.2020.1080p", Some("abc"), "stalled").await.unwrap();
        assert_eq!(
            blocklisted(&pool, t.id).await.unwrap(),
            vec![("X.2020.1080p".into(), Some("abc".into()))]
        );
        assert_eq!(latest_for_title(&pool, t.id).await.unwrap().unwrap().error.as_deref(), Some("stalled"));
    }
}
