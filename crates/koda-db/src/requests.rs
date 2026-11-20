//! Requests: who asked for which title, and the decision on it.

use serde::Serialize;

use crate::{Error, Pool, now};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Approved,
    Declined,
}

impl RequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestStatus::Pending => "pending",
            RequestStatus::Approved => "approved",
            RequestStatus::Declined => "declined",
        }
    }
}

/// A request joined with what the list view needs.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RequestRow {
    pub id: i64,
    pub title_id: i64,
    pub user_id: i64,
    pub username: String,
    pub status: String,
    pub decided_by: Option<i64>,
    pub decided_at: Option<i64>,
    pub note: Option<String>,
    pub created_at: i64,
    pub kind: String,
    pub tmdb_id: Option<i64>,
    pub title: String,
    pub year: Option<i64>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub title_state: String,
}

macro_rules! select_requests {
    ($rest:literal) => {
        concat!(
            "SELECT r.id, r.title_id, r.user_id, u.username, r.status, r.decided_by, r.decided_at, \
             r.note, r.created_at, t.kind, t.tmdb_id, t.title, t.year, t.poster_path, t.backdrop_path, \
             t.state AS title_state \
             FROM requests r JOIN users u ON u.id = r.user_id JOIN titles t ON t.id = r.title_id ",
            $rest
        )
    };
}

/// Inserts a request, or returns the existing one if this user already asked
/// for this title.
pub async fn create(
    pool: &Pool,
    title_id: i64,
    user_id: i64,
    status: RequestStatus,
) -> Result<RequestRow, Error> {
    let decided_at = (status != RequestStatus::Pending).then(now);
    let decided_by = decided_at.map(|_| user_id);
    sqlx::query(
        "INSERT INTO requests (title_id, user_id, status, decided_by, decided_at)
         VALUES (?, ?, ?, ?, ?) ON CONFLICT(title_id, user_id) DO NOTHING",
    )
    .bind(title_id)
    .bind(user_id)
    .bind(status.as_str())
    .bind(decided_by)
    .bind(decided_at)
    .execute(pool)
    .await?;
    sqlx::query_as(select_requests!("WHERE r.title_id = ? AND r.user_id = ?"))
        .bind(title_id)
        .bind(user_id)
        .fetch_one(pool)
        .await
}

pub async fn by_id(pool: &Pool, id: i64) -> Result<Option<RequestRow>, Error> {
    sqlx::query_as(select_requests!("WHERE r.id = ?")).bind(id).fetch_optional(pool).await
}

/// Newest first. `user_id` limits to one requester.
pub async fn list(
    pool: &Pool,
    status: Option<RequestStatus>,
    user_id: Option<i64>,
) -> Result<Vec<RequestRow>, Error> {
    sqlx::query_as(select_requests!(
        "WHERE (?1 IS NULL OR r.status = ?1) AND (?2 IS NULL OR r.user_id = ?2)
         ORDER BY r.created_at DESC, r.id DESC LIMIT 500"
    ))
    .bind(status.map(RequestStatus::as_str))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn decide(
    pool: &Pool,
    id: i64,
    status: RequestStatus,
    decided_by: i64,
    note: Option<&str>,
) -> Result<(), Error> {
    sqlx::query("UPDATE requests SET status = ?, decided_by = ?, decided_at = ?, note = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(decided_by)
        .bind(now())
        .bind(note)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Requests that still keep a title alive (pending or approved).
pub async fn open_count_for_title(pool: &Pool, title_id: i64) -> Result<i64, Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM requests WHERE title_id = ? AND status IN ('pending', 'approved')",
    )
    .bind(title_id)
    .fetch_one(pool)
    .await
}

/// Requests a user made in the last `days` days, declined ones excluded.
pub async fn count_recent_for_user(pool: &Pool, user_id: i64, days: i64) -> Result<i64, Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM requests WHERE user_id = ? AND status != 'declined' AND created_at > ?",
    )
    .bind(user_id)
    .bind(now() - days * 86_400)
    .fetch_one(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::titles::{self, NewTitle};
    use koda_core::{Permissions, TitleState};

    #[tokio::test]
    async fn request_lifecycle() {
        let pool = crate::memory().await;
        let ana = crate::users::create(&pool, "ana", "h", Permissions::ADMIN).await.unwrap();
        let bob = crate::users::create(&pool, "bob", "h", Permissions::DEFAULT_USER).await.unwrap();
        let t = titles::upsert_movie(
            &pool,
            &NewTitle { tmdb_id: 603, title: "The Matrix".into(), ..Default::default() },
            TitleState::Requested,
        )
        .await
        .unwrap();

        let r = create(&pool, t.id, bob.id, RequestStatus::Pending).await.unwrap();
        let again = create(&pool, t.id, bob.id, RequestStatus::Pending).await.unwrap();
        assert_eq!(r.id, again.id, "one request per user and title");
        assert_eq!(r.username, "bob");
        assert_eq!(r.title, "The Matrix");

        assert_eq!(list(&pool, Some(RequestStatus::Pending), None).await.unwrap().len(), 1);
        assert_eq!(count_recent_for_user(&pool, bob.id, 7).await.unwrap(), 1);

        decide(&pool, r.id, RequestStatus::Declined, ana.id, Some("not now")).await.unwrap();
        assert_eq!(open_count_for_title(&pool, t.id).await.unwrap(), 0);
        assert_eq!(count_recent_for_user(&pool, bob.id, 7).await.unwrap(), 0);
        let r = by_id(&pool, r.id).await.unwrap().unwrap();
        assert_eq!(r.status, "declined");
        assert_eq!(r.note.as_deref(), Some("not now"));
    }
}
