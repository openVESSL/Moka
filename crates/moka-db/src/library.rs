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

/// Every file in the library, newest first.
pub async fn all_files(pool: &Pool) -> Result<Vec<MediaFile>, Error> {
    sqlx::query_as(
        "SELECT id, title_id, path, size_bytes, quality, release_group, duration_seconds, added_at
         FROM media_files ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await
}

/// What one user did with one title.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct UserData {
    pub title_id: i64,
    pub position_seconds: f64,
    pub played: bool,
    pub play_count: i64,
    pub favorite: bool,
    pub last_played_at: Option<i64>,
}

/// Everything a user has started, finished or marked, most recent first.
pub async fn user_data(pool: &Pool, user_id: i64) -> Result<Vec<UserData>, Error> {
    sqlx::query_as(
        "SELECT title_id, position_seconds, played, play_count, favorite, last_played_at
         FROM user_title_data WHERE user_id = ? ORDER BY last_played_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Marks or unmarks a title as a favorite for one user.
pub async fn set_favorite(pool: &Pool, user_id: i64, title_id: i64, favorite: bool) -> Result<(), Error> {
    sqlx::query(
        "INSERT INTO user_title_data (user_id, title_id, favorite) VALUES (?1, ?2, ?3)
         ON CONFLICT(user_id, title_id) DO UPDATE SET favorite = ?3",
    )
    .bind(user_id)
    .bind(title_id)
    .bind(favorite)
    .execute(pool)
    .await?;
    Ok(())
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

/// Playback position for one user, in seconds.
pub async fn progress(pool: &Pool, user_id: i64, title_id: i64) -> Result<(f64, bool), Error> {
    let row: Option<(f64, bool)> = sqlx::query_as(
        "SELECT position_seconds, played FROM user_title_data WHERE user_id = ? AND title_id = ?",
    )
    .bind(user_id)
    .bind(title_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.unwrap_or((0.0, false)))
}

/// Where a user stopped in a book: an EPUB location, or the audiobook part.
pub async fn bookmark(pool: &Pool, user_id: i64, title_id: i64) -> Result<Option<String>, Error> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT bookmark FROM user_title_data WHERE user_id = ? AND title_id = ?")
            .bind(user_id)
            .bind(title_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.flatten())
}

/// Saves a bookmark next to the position (see `set_progress`, which must run first).
pub async fn set_bookmark(pool: &Pool, user_id: i64, title_id: i64, bookmark: &str) -> Result<(), Error> {
    sqlx::query("UPDATE user_title_data SET bookmark = ? WHERE user_id = ? AND title_id = ?")
        .bind(bookmark)
        .bind(user_id)
        .bind(title_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Saves where a user is in a film. Marking it played counts one more play.
pub async fn set_progress(
    pool: &Pool,
    user_id: i64,
    title_id: i64,
    position: f64,
    played: bool,
) -> Result<(), Error> {
    sqlx::query(
        "INSERT INTO user_title_data (user_id, title_id, position_seconds, played, play_count, last_played_at)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5)
         ON CONFLICT(user_id, title_id) DO UPDATE SET position_seconds = ?3,
            play_count = play_count + (CASE WHEN ?4 AND NOT played THEN 1 ELSE 0 END),
            played = ?4, last_played_at = ?5",
    )
    .bind(user_id)
    .bind(title_id)
    .bind(position.max(0.0))
    .bind(played)
    .bind(crate::now())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_profile(pool: &Pool, p: &QualityProfile) -> Result<(), Error> {
    sqlx::query(
        "UPDATE quality_profiles SET name = ?, qualities = ?, cutoff = ?, upgrades_allowed = ?, min_seeders = ?,
         size_limits = ?, preferred_terms = ? WHERE id = ?",
    )
    .bind(&p.name)
    .bind(&p.qualities)
    .bind(&p.cutoff)
    .bind(p.upgrades_allowed)
    .bind(p.min_seeders)
    .bind(&p.size_limits)
    .bind(&p.preferred_terms)
    .bind(p.id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Cached technical details of a file (FFprobe output, our JSON shape).
pub async fn probe_json(pool: &Pool, file_id: i64) -> Result<Option<String>, Error> {
    sqlx::query_scalar("SELECT probe_json FROM media_files WHERE id = ?").bind(file_id).fetch_one(pool).await
}

pub async fn set_probe(pool: &Pool, file_id: i64, json: &str, duration: f64) -> Result<(), Error> {
    sqlx::query("UPDATE media_files SET probe_json = ?, duration_seconds = ? WHERE id = ?")
        .bind(json)
        .bind(duration)
        .bind(file_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn progress_round_trip() {
        use moka_core::{Permissions, TitleState};
        let pool = crate::memory().await;
        let u = crate::users::create(&pool, "a", "h", Permissions::ADMIN).await.unwrap();
        let t = crate::titles::upsert_movie(
            &pool,
            &crate::titles::NewTitle { tmdb_id: 1, title: "X".into(), ..Default::default() },
            TitleState::Available,
        )
        .await
        .unwrap();
        assert_eq!(super::progress(&pool, u.id, t.id).await.unwrap(), (0.0, false));
        super::set_progress(&pool, u.id, t.id, 120.5, false).await.unwrap();
        super::set_progress(&pool, u.id, t.id, 5000.0, true).await.unwrap();
        assert_eq!(super::progress(&pool, u.id, t.id).await.unwrap(), (5000.0, true));
        assert_eq!(super::bookmark(&pool, u.id, t.id).await.unwrap(), None);
        super::set_bookmark(&pool, u.id, t.id, "epubcfi(/6/4!/4/2)").await.unwrap();
        super::set_progress(&pool, u.id, t.id, 10.0, false).await.unwrap();
        assert_eq!(super::bookmark(&pool, u.id, t.id).await.unwrap().as_deref(), Some("epubcfi(/6/4!/4/2)"));
        super::set_favorite(&pool, u.id, t.id, true).await.unwrap();
        let d = super::user_data(&pool, u.id).await.unwrap();
        assert_eq!((d.len(), d[0].favorite, d[0].played, d[0].position_seconds), (1, true, false, 10.0));
    }

    #[tokio::test]
    async fn default_profile_exists() {
        let pool = crate::memory().await;
        let p = super::profile(&pool, None).await.unwrap().unwrap();
        assert_eq!(p.cutoff, "bluray-1080p");
        assert!(p.qualities.contains("webdl-1080p"));
    }
}

/// A track of an album in the library, with its newest file if it has one.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AlbumTrack {
    pub title_id: i64,
    pub disc: Option<i64>,
    /// Position across the album, from 1.
    pub number: Option<i64>,
    pub title: String,
    pub state: String,
    pub file_id: Option<i64>,
}

pub async fn album_tracks(pool: &Pool, album_id: i64) -> Result<Vec<AlbumTrack>, Error> {
    sqlx::query_as(
        "SELECT t.id AS title_id, t.season_number AS disc, t.episode_number AS number, t.title, t.state,
            (SELECT m.id FROM media_files m WHERE m.title_id = t.id ORDER BY m.id DESC LIMIT 1) AS file_id
         FROM titles t WHERE t.parent_id = ? AND t.kind = 'track'
         ORDER BY t.season_number, t.episode_number",
    )
    .bind(album_id)
    .fetch_all(pool)
    .await
}
