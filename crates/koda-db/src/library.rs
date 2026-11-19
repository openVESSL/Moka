//! Quality profiles and media files in the library.

use serde::Serialize;

use crate::{Error, Pool};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct QualityProfile {
    pub id: i64,
    pub name: String,
    /// JSON array of quality keys, best first.
    pub qualities: String,
    pub cutoff: String,
    pub upgrades_allowed: bool,
    pub min_seeders: i64,
    /// JSON object: quality key -> size limit (MB/min).
    pub size_limits: String,
    /// JSON array of { term, score }.
    pub preferred_terms: String,
}

pub async fn profile(pool: &Pool, id: Option<i64>) -> Result<Option<QualityProfile>, Error> {
    sqlx::query_as(
        "SELECT * FROM quality_profiles WHERE id = COALESCE(?, (SELECT MIN(id) FROM quality_profiles))",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MediaFile {
    pub id: i64,
    pub title_id: i64,
    pub path: String,
    pub size_bytes: i64,
    pub quality: Option<String>,
    pub release_group: Option<String>,
    pub duration_seconds: Option<f64>,
    pub added_at: i64,
}

pub async fn add_file(
    pool: &Pool,
    title_id: i64,
    path: &str,
    size_bytes: i64,
    quality: Option<&str>,
    release_group: Option<&str>,
) -> Result<i64, Error> {
    sqlx::query_scalar(
        "INSERT INTO media_files (title_id, path, size_bytes, quality, release_group) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(path) DO UPDATE SET title_id = excluded.title_id, size_bytes = excluded.size_bytes,
            quality = excluded.quality, release_group = excluded.release_group
         RETURNING id",
    )
    .bind(title_id)
    .bind(path)
    .bind(size_bytes)
    .bind(quality)
    .bind(release_group)
    .fetch_one(pool)
    .await
}

pub async fn files_for_title(pool: &Pool, title_id: i64) -> Result<Vec<MediaFile>, Error> {
    sqlx::query_as(
        "SELECT id, title_id, path, size_bytes, quality, release_group, duration_seconds, added_at
         FROM media_files WHERE title_id = ? ORDER BY id DESC",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
}

pub async fn file(pool: &Pool, id: i64) -> Result<Option<MediaFile>, Error> {
    sqlx::query_as(
        "SELECT id, title_id, path, size_bytes, quality, release_group, duration_seconds, added_at
         FROM media_files WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn default_profile_exists() {
        let pool = crate::memory().await;
        let p = super::profile(&pool, None).await.unwrap().unwrap();
        assert_eq!(p.cutoff, "bluray-1080p");
        assert!(p.qualities.contains("webdl-1080p"));
    }
}
