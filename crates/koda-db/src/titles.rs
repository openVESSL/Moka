//! Titles and their lifecycle state.

use koda_core::{TitleKind, TitleState};
use serde::Serialize;

use crate::{Error, Pool, now};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Title {
    pub id: i64,
    pub kind: String,
    /// Season for an episode, show for a season.
    pub parent_id: Option<i64>,
    pub season_number: Option<i64>,
    pub episode_number: Option<i64>,
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

/// A show with the seasons and episodes to create under it.
#[derive(Debug, Clone, Default)]
pub struct NewShow {
    pub show: NewTitle,
    pub tvdb_id: Option<i64>,
    pub seasons: Vec<NewSeason>,
}

#[derive(Debug, Clone, Default)]
pub struct NewSeason {
    /// TMDB id of the season, number, name, overview, air date and poster.
    pub title: NewTitle,
    pub number: i64,
    pub episodes: Vec<NewEpisode>,
}

#[derive(Debug, Clone, Default)]
pub struct NewEpisode {
    /// TMDB id of the episode, name, overview, air date, runtime and still.
    pub title: NewTitle,
    pub number: i64,
}

/// `SELECT <title columns> FROM titles <rest>` as a static string.
macro_rules! select_titles {
    ($rest:literal) => {
        concat!(
            "SELECT id, kind, parent_id, season_number, episode_number, tmdb_id, imdb_id, title, original_title, year, overview, \
             runtime_minutes, release_date, poster_path, backdrop_path, state, state_reason, \
             quality_profile_id, monitored, next_search_at, last_search_at, created_at, updated_at \
             FROM titles ",
            $rest
        )
    };
}

/// Inserts a movie, or returns the existing one with the same TMDB id.
pub async fn upsert_movie(pool: &Pool, t: &NewTitle, state: TitleState) -> Result<Title, Error> {
    let profile: Option<i64> =
        sqlx::query_scalar("SELECT MIN(id) FROM quality_profiles").fetch_one(pool).await?;
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
    by_tmdb(pool, TitleKind::Movie, t.tmdb_id).await?.ok_or(Error::RowNotFound)
}

/// Inserts a show with its seasons and episodes. Parts that already exist
/// keep their state, so calling it again only adds new seasons and episodes.
/// Specials (season 0) are not monitored.
pub async fn upsert_show(pool: &Pool, s: &NewShow, state: TitleState) -> Result<Title, Error> {
    let profile: Option<i64> =
        sqlx::query_scalar("SELECT MIN(id) FROM quality_profiles").fetch_one(pool).await?;
    let mut tx = pool.begin().await?;
    let show = insert_part(&mut tx, TitleKind::Show, None, &s.show, None, None, state, profile, true).await?;
    sqlx::query("UPDATE titles SET tvdb_id = COALESCE(?, tvdb_id) WHERE id = ?")
        .bind(s.tvdb_id)
        .bind(show)
        .execute(&mut *tx)
        .await?;
    for season in &s.seasons {
        let monitored = season.number > 0;
        let sid = insert_part(
            &mut tx,
            TitleKind::Season,
            Some(show),
            &season.title,
            Some(season.number),
            None,
            state,
            profile,
            monitored,
        )
        .await?;
        for e in &season.episodes {
            insert_part(
                &mut tx,
                TitleKind::Episode,
                Some(sid),
                &e.title,
                Some(season.number),
                Some(e.number),
                state,
                profile,
                monitored,
            )
            .await?;
        }
    }
    tx.commit().await?;
    by_id(pool, show).await?.ok_or(Error::RowNotFound)
}

#[allow(clippy::too_many_arguments)]
async fn insert_part(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    kind: TitleKind,
    parent: Option<i64>,
    t: &NewTitle,
    season: Option<i64>,
    episode: Option<i64>,
    state: TitleState,
    profile: Option<i64>,
    monitored: bool,
) -> Result<i64, Error> {
    sqlx::query(
        "INSERT INTO titles (kind, parent_id, season_number, episode_number, tmdb_id, imdb_id, title,
            original_title, year, overview, runtime_minutes, release_date, poster_path, backdrop_path,
            metadata_json, state, quality_profile_id, monitored, next_search_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(kind, tmdb_id) WHERE tmdb_id IS NOT NULL DO NOTHING",
    )
    .bind(kind.as_str())
    .bind(parent)
    .bind(season)
    .bind(episode)
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
    .bind(monitored)
    .bind((state == TitleState::Wanted).then(now))
    .execute(&mut **tx)
    .await?;
    sqlx::query_scalar("SELECT id FROM titles WHERE kind = ? AND tmdb_id = ?")
        .bind(kind.as_str())
        .bind(t.tmdb_id)
        .fetch_one(&mut **tx)
        .await
}

/// Aired, monitored episodes the background loop should search now.
pub async fn due_episodes(pool: &Pool, limit: i64) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE kind = 'episode' AND state = 'wanted' AND monitored = 1
           AND release_date IS NOT NULL AND release_date <= date('now')
           AND (next_search_at IS NULL OR next_search_at <= ?)
         ORDER BY next_search_at, season_number, episode_number LIMIT ?"
    ))
    .bind(now())
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// TheTVDB id of a show, when the catalog gave one.
pub async fn tvdb_id(pool: &Pool, id: i64) -> Result<Option<i64>, Error> {
    sqlx::query_scalar("SELECT tvdb_id FROM titles WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map(Option::flatten)
}

/// Every title waiting to be imported: films, episodes and season packs.
pub async fn importing(pool: &Pool) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE state = 'importing' ORDER BY updated_at")).fetch_all(pool).await
}

/// Approves a requested show: its seasons and episodes become wanted too.
pub async fn approve_show_parts(pool: &Pool, show_id: i64) -> Result<(), Error> {
    sqlx::query(
        "UPDATE titles SET state = 'wanted', next_search_at = ?1, updated_at = ?1
         WHERE state = 'requested'
           AND (parent_id = ?2 OR parent_id IN (SELECT id FROM titles WHERE parent_id = ?2))",
    )
    .bind(now())
    .bind(show_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// How far a show's episodes have come.
#[derive(Debug, Clone, Copy, Default, PartialEq, sqlx::FromRow)]
pub struct ShowProgress {
    pub show_id: i64,
    /// Monitored episodes that have aired.
    pub aired: i64,
    /// Of those, the ones in the library.
    pub available: i64,
    /// Episodes being searched, downloaded or imported.
    pub busy: i64,
}

impl ShowProgress {
    /// The show's state as its episodes make it: available once every aired
    /// episode is, on the way while any is, else its own (requested, wanted).
    pub fn state(&self, own: TitleState) -> TitleState {
        if own == TitleState::Requested {
            own
        } else if self.aired > 0 && self.available >= self.aired {
            TitleState::Available
        } else if self.busy > 0 {
            TitleState::Downloading
        } else {
            own
        }
    }
}

/// Progress of every show that has episodes.
pub async fn show_progress(pool: &Pool) -> Result<Vec<ShowProgress>, Error> {
    sqlx::query_as(
        "SELECT s.parent_id AS show_id,
            COALESCE(SUM(e.monitored = 1 AND e.release_date <= date('now')), 0) AS aired,
            COALESCE(SUM(e.monitored = 1 AND e.release_date <= date('now') AND e.state = 'available'), 0) AS available,
            COALESCE(SUM(e.state IN ('searching', 'grabbed', 'downloading', 'importing')), 0) AS busy
         FROM titles e JOIN titles s ON s.id = e.parent_id
         WHERE e.kind = 'episode' AND s.parent_id IS NOT NULL
         GROUP BY s.parent_id",
    )
    .fetch_all(pool)
    .await
}

/// Shows, newest change first.
pub async fn shows(pool: &Pool) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = 'show' ORDER BY updated_at DESC")).fetch_all(pool).await
}

/// Seasons of a show, or episodes of a season, in order.
pub async fn children(pool: &Pool, parent_id: i64) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE parent_id = ? ORDER BY season_number, episode_number"))
        .bind(parent_id)
        .fetch_all(pool)
        .await
}

/// Every episode of a show, in order.
pub async fn episodes_of_show(pool: &Pool, show_id: i64) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE kind = 'episode' AND parent_id IN (SELECT id FROM titles WHERE parent_id = ?)
         ORDER BY season_number, episode_number"
    ))
    .bind(show_id)
    .fetch_all(pool)
    .await
}

pub async fn by_id(pool: &Pool, id: i64) -> Result<Option<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE id = ?")).bind(id).fetch_optional(pool).await
}

pub async fn by_tmdb(pool: &Pool, kind: TitleKind, tmdb_id: i64) -> Result<Option<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = ? AND tmdb_id = ?"))
        .bind(kind.as_str())
        .bind(tmdb_id)
        .fetch_optional(pool)
        .await
}

/// Films, newest change first (shows are listed on their own).
pub async fn list(pool: &Pool, state: Option<TitleState>) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE kind = 'movie' AND (?1 IS NULL OR state = ?1) ORDER BY updated_at DESC"
    ))
    .bind(state.map(TitleState::as_str))
    .fetch_all(pool)
    .await
}

/// Local id and state for each TMDB id that exists in the library, used to
/// annotate catalog results.
pub async fn states_for_tmdb(
    pool: &Pool,
    kind: TitleKind,
    tmdb_ids: &[i64],
) -> Result<Vec<(i64, i64, String)>, Error> {
    let ids = serde_json::to_string(tmdb_ids).map_err(|e| Error::Encode(Box::new(e)))?;
    sqlx::query_as(
        "SELECT tmdb_id, id, state FROM titles
         WHERE kind = ? AND tmdb_id IN (SELECT value FROM json_each(?))",
    )
    .bind(kind.as_str())
    .bind(ids)
    .fetch_all(pool)
    .await
}

pub async fn delete(pool: &Pool, id: i64) -> Result<(), Error> {
    sqlx::query("DELETE FROM titles WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

/// Films the background loop should search now.
pub async fn due_for_search(pool: &Pool, limit: i64) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE kind = 'movie' AND state = 'wanted' AND monitored = 1 AND (next_search_at IS NULL OR next_search_at <= ?)
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
    by_id(pool, id).await?.ok_or(TransitionError::NotFound)
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

/// Updates the explanation shown for the current state without moving it.
pub async fn set_reason(pool: &Pool, id: i64, reason: Option<&str>) -> Result<(), Error> {
    sqlx::query("UPDATE titles SET state_reason = ?, updated_at = ? WHERE id = ?")
        .bind(reason)
        .bind(now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Films in one state, oldest change first.
pub async fn in_state(pool: &Pool, state: TitleState) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = 'movie' AND state = ? ORDER BY updated_at"))
        .bind(state.as_str())
        .fetch_all(pool)
        .await
}

pub async fn search_attempts(pool: &Pool, id: i64) -> Result<i64, Error> {
    sqlx::query_scalar("SELECT search_attempts FROM titles WHERE id = ?").bind(id).fetch_one(pool).await
}

/// Marks a title available because a file for it was found on disk,
/// whatever state it was in (a library scan is ground truth).
pub async fn mark_available(pool: &Pool, id: i64) -> Result<(), Error> {
    sqlx::query("UPDATE titles SET state = 'available', state_reason = NULL, updated_at = ? WHERE id = ?")
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

    #[tokio::test]
    async fn shows_hold_seasons_and_episodes() {
        let pool = crate::memory().await;
        let part = |id: i64, name: &str| NewTitle { tmdb_id: id, title: name.into(), ..Default::default() };
        let episodes =
            |base: i64| (1..=2).map(|n| NewEpisode { title: part(base + n, "Ep"), number: n }).collect();
        let mut show = NewShow {
            show: part(1399, "Thrones"),
            tvdb_id: Some(121361),
            seasons: vec![
                NewSeason { title: part(10, "Specials"), number: 0, episodes: episodes(100) },
                NewSeason { title: part(11, "Season 1"), number: 1, episodes: episodes(110) },
            ],
        };
        let a = upsert_show(&pool, &show, TitleState::Wanted).await.unwrap();
        assert_eq!(a.kind, "show");
        let seasons = children(&pool, a.id).await.unwrap();
        assert_eq!(seasons.iter().map(|s| s.season_number).collect::<Vec<_>>(), [Some(0), Some(1)]);
        let eps = episodes_of_show(&pool, a.id).await.unwrap();
        assert_eq!(eps.len(), 4);
        assert!(!eps[0].monitored && eps[2].monitored, "specials are not monitored");
        assert_eq!((eps[3].season_number, eps[3].episode_number), (Some(1), Some(2)));

        // a new episode later: only it is added, the rest keep their state
        transition(&pool, eps[2].id, TitleState::Searching, None).await.unwrap();
        show.seasons[1].episodes.push(NewEpisode { title: part(113, "Ep 3"), number: 3 });
        let b = upsert_show(&pool, &show, TitleState::Wanted).await.unwrap();
        assert_eq!(a.id, b.id);
        let eps = episodes_of_show(&pool, a.id).await.unwrap();
        assert_eq!(eps.len(), 5);
        assert_eq!(eps[2].state(), TitleState::Searching);
        assert_eq!(shows(&pool).await.unwrap().len(), 1);

        let asked = NewShow {
            show: part(2, "Asked"),
            seasons: vec![NewSeason { title: part(20, "S1"), number: 1, episodes: episodes(200) }],
            ..Default::default()
        };
        let t = upsert_show(&pool, &asked, TitleState::Requested).await.unwrap();
        approve_show_parts(&pool, t.id).await.unwrap();
        let eps = episodes_of_show(&pool, t.id).await.unwrap();
        assert!(eps.iter().all(|e| e.state() == TitleState::Wanted && e.next_search_at.is_some()));
        assert_eq!(children(&pool, t.id).await.unwrap()[0].state(), TitleState::Wanted);
        assert!(due_episodes(&pool, 10).await.unwrap().is_empty(), "episodes without an air date wait");
        let progress = |id: i64| {
            let pool = pool.clone();
            async move { show_progress(&pool).await.unwrap().into_iter().find(|p| p.show_id == id).unwrap() }
        };
        sqlx::query("UPDATE titles SET release_date = '2011-04-17' WHERE id = ?")
            .bind(eps[0].id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE titles SET release_date = '2999-01-01' WHERE id = ?")
            .bind(eps[1].id)
            .execute(&pool)
            .await
            .unwrap();
        let due = due_episodes(&pool, 10).await.unwrap();
        assert_eq!(due.iter().map(|e| e.id).collect::<Vec<_>>(), [eps[0].id], "only aired episodes");
        let p = progress(t.id).await;
        assert_eq!((p.aired, p.available, p.busy), (1, 0, 0));
        assert_eq!(p.state(TitleState::Wanted), TitleState::Wanted);
        mark_available(&pool, eps[0].id).await.unwrap();
        assert_eq!(
            progress(t.id).await.state(TitleState::Wanted),
            TitleState::Available,
            "the unaired one does not count"
        );
        assert!(list(&pool, None).await.unwrap().is_empty(), "film lists leave shows out");
        assert!(due_for_search(&pool, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn states_for_tmdb_returns_only_known_ids() {
        let pool = crate::memory().await;
        let t = upsert_movie(&pool, &matrix(), TitleState::Wanted).await.unwrap();
        let found = states_for_tmdb(&pool, TitleKind::Movie, &[603, 999]).await.unwrap();
        assert_eq!(found, vec![(603, t.id, "wanted".to_owned())]);
        assert!(states_for_tmdb(&pool, TitleKind::Movie, &[]).await.unwrap().is_empty());
    }
}
