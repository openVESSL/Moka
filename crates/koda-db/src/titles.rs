//! Titles and their lifecycle state.

use koda_core::{TitleKind, TitleState};
use serde::Serialize;

use crate::{Error, Pool, now};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Title {
    pub id: i64,
    pub kind: String,
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i64>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i64>,
    pub release_date: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub state: String,
    pub state_reason: Option<String>,
    pub quality_profile_id: Option<i64>,
    pub monitored: bool,
    pub next_search_at: Option<i64>,
    pub last_search_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Title {
    pub fn state(&self) -> TitleState {
        // only values written through this module end up here
        self.state.parse().unwrap_or(TitleState::Wanted)
    }
}

/// Metadata needed to create a title. Filled from the catalog.
#[derive(Debug, Clone, Default)]
pub struct NewTitle {
    pub tmdb_id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i64>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i64>,
    pub release_date: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub metadata_json: Option<String>,
}

/// `SELECT <title columns> FROM titles <rest>` as a static string.
macro_rules! select_titles {
    ($rest:literal) => {
        concat!(
            "SELECT id, kind, tmdb_id, imdb_id, title, original_title, year, overview, \
             runtime_minutes, release_date, poster_path, backdrop_path, state, state_reason, \
             quality_profile_id, monitored, next_search_at, last_search_at, created_at, updated_at \
             FROM titles ",
            $rest
        )
    };
}

/// Inserts a movie, or returns the existing one with the same TMDB id.
pub async fn upsert_movie(pool: &Pool, t: &NewTitle, state: TitleState) -> Result<Title, Error> {
    let profile: Option<i64> = sqlx::query_scalar("SELECT MIN(id) FROM quality_profiles")
        .fetch_one(pool)
        .await?;
    sqlx::query(
        "INSERT INTO titles (kind, tmdb_id, imdb_id, title, original_title, year, overview,
            runtime_minutes, release_date, poster_path, backdrop_path, metadata_json, state,
            quality_profile_id, next_search_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(kind, tmdb_id) WHERE tmdb_id IS NOT NULL DO NOTHING",
    )
    .bind(TitleKind::Movie.as_str())
    .bind(t.tmdb_id)
    .bind(&t.imdb_id)
    .bind(&t.title)
    .bind(&t.original_title)
    .bind(t.year)
    .bind(&t.overview)
    .bind(t.runtime_minutes)
    .bind(&t.release_date)
    .bind(&t.poster_path)
    .bind(&t.backdrop_path)
    .bind(&t.metadata_json)
    .bind(state.as_str())
    .bind(profile)
    .bind((state == TitleState::Wanted).then(now))
    .execute(pool)
    .await?;
    by_tmdb(pool, TitleKind::Movie, t.tmdb_id)
        .await?
        .ok_or(Error::RowNotFound)
}

pub async fn by_id(pool: &Pool, id: i64) -> Result<Option<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn by_tmdb(pool: &Pool, kind: TitleKind, tmdb_id: i64) -> Result<Option<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = ? AND tmdb_id = ?"))
        .bind(kind.as_str())
        .bind(tmdb_id)
        .fetch_optional(pool)
        .await
}

pub async fn list(pool: &Pool, state: Option<TitleState>) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE (?1 IS NULL OR state = ?1) ORDER BY updated_at DESC"
    ))
        .bind(state.map(TitleState::as_str))
        .fetch_all(pool)
        .await
}

/// Titles the background loop should search now.
pub async fn due_for_search(pool: &Pool, limit: i64) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE state = 'wanted' AND monitored = 1 AND (next_search_at IS NULL OR next_search_at <= ?)
         ORDER BY next_search_at LIMIT ?"
    ))
    .bind(now())
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[derive(Debug, thiserror::Error)]
pub enum TransitionError {
    #[error("title not found")]
    NotFound,
    #[error("cannot move from {from} to {to}")]
    NotAllowed { from: TitleState, to: TitleState },
    #[error("title changed concurrently")]
    Conflict,
    #[error(transparent)]
    Db(#[from] Error),
}

/// Moves a title to a new state if the transition is valid and nobody else
/// moved it meanwhile (compare-and-set on the current state).
pub async fn transition(
    pool: &Pool,
    id: i64,
    to: TitleState,
    reason: Option<&str>,
) -> Result<Title, TransitionError> {
    let current = by_id(pool, id).await?.ok_or(TransitionError::NotFound)?;
    let from = current.state();
    if !from.can_move_to(to) {
        return Err(TransitionError::NotAllowed { from, to });
    }
    let updated = sqlx::query(
        "UPDATE titles SET state = ?, state_reason = ?, updated_at = ? WHERE id = ? AND state = ?",
    )
    .bind(to.as_str())
    .bind(reason)
    .bind(now())
    .bind(id)
    .bind(from.as_str())
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(TransitionError::Conflict);
    }
    Ok(by_id(pool, id).await?.ok_or(TransitionError::NotFound)?)
}

/// Records a finished search and when to try again.
pub async fn schedule_search(
    pool: &Pool,
    id: i64,
    next_search_at: i64,
    reason: Option<&str>,
) -> Result<(), Error> {
    sqlx::query(
        "UPDATE titles SET last_search_at = ?, next_search_at = ?, search_attempts = search_attempts + 1,
            state_reason = ?, updated_at = ? WHERE id = ?",
    )
    .bind(now())
    .bind(next_search_at)
    .bind(reason)
    .bind(now())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix() -> NewTitle {
        NewTitle { tmdb_id: 603, title: "The Matrix".into(), year: Some(1999), ..Default::default() }
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_gets_default_profile() {
        let pool = crate::memory().await;
        let a = upsert_movie(&pool, &matrix(), TitleState::Wanted).await.unwrap();
        let b = upsert_movie(&pool, &matrix(), TitleState::Requested).await.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.state(), TitleState::Wanted, "existing title keeps its state");
        assert!(a.quality_profile_id.is_some());
        assert!(a.next_search_at.is_some(), "wanted titles are searched right away");
    }

    #[tokio::test]
    async fn transitions_follow_the_state_machine() {
        let pool = crate::memory().await;
        let t = upsert_movie(&pool, &matrix(), TitleState::Requested).await.unwrap();
        assert!(matches!(
            transition(&pool, t.id, TitleState::Available, None).await,
            Err(TransitionError::NotAllowed { .. })
        ));
        let t = transition(&pool, t.id, TitleState::Wanted, Some("approved")).await.unwrap();
        assert_eq!(t.state(), TitleState::Wanted);
        assert_eq!(t.state_reason.as_deref(), Some("approved"));
        assert_eq!(list(&pool, Some(TitleState::Wanted)).await.unwrap().len(), 1);
        assert_eq!(list(&pool, Some(TitleState::Available)).await.unwrap().len(), 0);
        assert_eq!(due_for_search(&pool, 10).await.unwrap().len(), 1);
    }
}
