//! Titles and their lifecycle state.

use moka_core::{TitleKind, TitleState};
use serde::Serialize;

use crate::{Error, Pool, now};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Title {
    pub id: i64,
    pub kind: String,
    /// Season for an episode, show for a season, album for a track, artist
    /// for an album, author for a book.
    pub parent_id: Option<i64>,
    /// MusicBrainz id, for music.
    pub mbid: Option<String>,
    /// Open Library id, for books and authors.
    pub olid: Option<String>,
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

/// An album with its artist and tracks, from MusicBrainz.
#[derive(Debug, Clone, Default)]
pub struct NewAlbum {
    pub artist_mbid: String,
    pub artist: String,
    pub mbid: String,
    pub title: String,
    pub year: Option<i64>,
    pub release_date: Option<String>,
    pub poster_path: Option<String>,
    pub tracks: Vec<NewTrack>,
}

/// A book with its author, from Open Library. Stored as an ebook or an
/// audiobook: the two are wanted, found and kept apart.
#[derive(Debug, Clone, Default)]
pub struct NewBook {
    pub author_olid: String,
    pub author: String,
    pub olid: String,
    pub title: String,
    pub year: Option<i64>,
    pub overview: Option<String>,
    /// Open Library cover id, kept as text.
    pub poster_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewTrack {
    /// Position across the album, from 1.
    pub number: i64,
    pub disc: i64,
    pub title: String,
    pub length_seconds: Option<i64>,
}

/// `SELECT <title columns> FROM titles <rest>` as a static string.
macro_rules! select_titles {
    ($rest:literal) => {
        concat!(
            "SELECT id, kind, parent_id, mbid, olid, season_number, episode_number, tmdb_id, imdb_id, title, original_title, year, overview, \
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
/// keep their state and get the new names, air dates and images, so calling
/// it again refreshes the show and adds new seasons and episodes.
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
         ON CONFLICT(kind, tmdb_id) WHERE tmdb_id IS NOT NULL DO UPDATE SET
            title = excluded.title, overview = excluded.overview, year = excluded.year,
            runtime_minutes = excluded.runtime_minutes, release_date = excluded.release_date,
            poster_path = COALESCE(excluded.poster_path, poster_path),
            backdrop_path = COALESCE(excluded.backdrop_path, backdrop_path),
            metadata_json = COALESCE(excluded.metadata_json, metadata_json)",
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

/// How a show's releases are numbered: `daily`, `anime` or `standard`.
pub async fn show_numbering(pool: &Pool, id: i64) -> Result<String, Error> {
    let n: Option<String> =
        sqlx::query_scalar("SELECT json_extract(metadata_json, '$.numbering') FROM titles WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .flatten();
    Ok(n.unwrap_or_else(|| "standard".into()))
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

/// Approves a requested show or album: its seasons and episodes, or its
/// tracks, become wanted too.
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

/// Stops following the aired episodes of a show that are not in the
/// library. Used when an existing show folder is brought in: what is on disk
/// is kept, and only episodes airing from now on are searched.
pub async fn unmonitor_missing(pool: &Pool, show_id: i64) -> Result<(), Error> {
    sqlx::query(
        "UPDATE titles SET monitored = 0
         WHERE kind = 'episode' AND state = 'wanted' AND release_date <= date('now')
           AND parent_id IN (SELECT id FROM titles WHERE parent_id = ?)",
    )
    .bind(show_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Follows or stops following an episode, or every episode of a season.
/// Wanted episodes that start being followed are searched right away.
pub async fn set_monitored(pool: &Pool, id: i64, monitored: bool) -> Result<(), Error> {
    sqlx::query(
        "UPDATE titles SET monitored = ?1,
            next_search_at = CASE WHEN ?1 AND state = 'wanted' THEN ?2 ELSE next_search_at END
         WHERE id = ?3 OR (kind = 'episode' AND parent_id = ?3)",
    )
    .bind(monitored)
    .bind(now())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Follows the given seasons of a show (and their episodes). With `only`, the
/// other seasons are left out, for a show requested season by season.
pub async fn follow_seasons(pool: &Pool, show_id: i64, seasons: &[i64], only: bool) -> Result<(), Error> {
    let list = serde_json::to_string(seasons).unwrap_or_else(|_| "[]".into());
    sqlx::query(
        "UPDATE titles SET monitored = CASE
                WHEN season_number IN (SELECT value FROM json_each(?2)) THEN 1
                WHEN ?3 THEN 0 ELSE monitored END,
            next_search_at = CASE
                WHEN season_number IN (SELECT value FROM json_each(?2)) AND state = 'wanted' THEN ?4
                ELSE next_search_at END
         WHERE kind IN ('season', 'episode')
           AND (parent_id = ?1 OR parent_id IN (SELECT id FROM titles WHERE parent_id = ?1))",
    )
    .bind(show_id)
    .bind(list)
    .bind(only)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Films and episodes that arrived through a download since a moment, oldest first.
pub async fn available_since(pool: &Pool, since: i64) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE kind IN ('movie', 'episode') AND state = 'available' AND available_at >= ? ORDER BY available_at, id"
    ))
    .bind(since)
    .fetch_all(pool)
    .await
}

/// Searches a show's wanted, followed episodes on the next pass.
pub async fn search_show_now(pool: &Pool, show_id: i64) -> Result<(), Error> {
    sqlx::query(
        "UPDATE titles SET next_search_at = ? WHERE kind = 'episode' AND state = 'wanted' AND monitored = 1
           AND parent_id IN (SELECT id FROM titles WHERE parent_id = ?)",
    )
    .bind(now())
    .bind(show_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// A show by TheTVDB id.
pub async fn show_by_tvdb(pool: &Pool, tvdb_id: i64) -> Result<Option<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = 'show' AND tvdb_id = ?"))
        .bind(tvdb_id)
        .fetch_optional(pool)
        .await
}

/// Shows, newest change first.
pub async fn shows(pool: &Pool) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = 'show' ORDER BY updated_at DESC")).fetch_all(pool).await
}

/// Inserts an album under its artist (created if new) with its tracks, or
/// returns the existing album. Tracks are keyed by album and number.
pub async fn upsert_album(pool: &Pool, a: &NewAlbum, state: TitleState) -> Result<Title, Error> {
    let profile: Option<i64> =
        sqlx::query_scalar("SELECT MIN(id) FROM quality_profiles").fetch_one(pool).await?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO titles (kind, mbid, title, state, monitored) VALUES ('artist', ?, ?, ?, 1)
         ON CONFLICT(kind, mbid) WHERE mbid IS NOT NULL DO UPDATE SET title = excluded.title",
    )
    .bind(&a.artist_mbid)
    .bind(&a.artist)
    .bind(TitleState::Wanted.as_str())
    .execute(&mut *tx)
    .await?;
    let artist: i64 = sqlx::query_scalar("SELECT id FROM titles WHERE kind = 'artist' AND mbid = ?")
        .bind(&a.artist_mbid)
        .fetch_one(&mut *tx)
        .await?;
    let existing: Option<i64> = sqlx::query_scalar("SELECT id FROM titles WHERE kind = 'album' AND mbid = ?")
        .bind(&a.mbid)
        .fetch_optional(&mut *tx)
        .await?;
    let album = match existing {
        Some(id) => id,
        None => {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO titles (kind, parent_id, mbid, title, year, release_date, poster_path, state,
                    quality_profile_id, monitored, next_search_at)
                 VALUES ('album', ?, ?, ?, ?, ?, ?, ?, ?, 1, ?) RETURNING id",
            )
            .bind(artist)
            .bind(&a.mbid)
            .bind(&a.title)
            .bind(a.year)
            .bind(&a.release_date)
            .bind(&a.poster_path)
            .bind(state.as_str())
            .bind(profile)
            .bind((state == TitleState::Wanted).then(now))
            .fetch_one(&mut *tx)
            .await?;
            for t in &a.tracks {
                sqlx::query(
                    "INSERT INTO titles (kind, parent_id, title, season_number, episode_number, runtime_minutes,
                        state, monitored)
                     VALUES ('track', ?, ?, ?, ?, ?, ?, 1)",
                )
                .bind(id)
                .bind(&t.title)
                .bind(t.disc)
                .bind(t.number)
                // shortcut: track length is kept in whole minutes, rounded up; seconds need their own column
                .bind(t.length_seconds.map(|s| (s + 59) / 60))
                .bind(state.as_str())
                .execute(&mut *tx)
                .await?;
            }
            id
        }
    };
    tx.commit().await?;
    by_id(pool, album).await?.ok_or(Error::RowNotFound)
}

/// A music title by its MusicBrainz id.
pub async fn by_mbid(pool: &Pool, kind: TitleKind, mbid: &str) -> Result<Option<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = ? AND mbid = ?"))
        .bind(kind.as_str())
        .bind(mbid)
        .fetch_optional(pool)
        .await
}

/// Local id and state of each album in `mbids` that is in the library.
pub async fn album_states(pool: &Pool, mbids: &[String]) -> Result<Vec<(String, i64, String)>, Error> {
    let ids = serde_json::to_string(mbids).map_err(|e| Error::Encode(Box::new(e)))?;
    sqlx::query_as(
        "SELECT mbid, id, state FROM titles WHERE kind = 'album' AND mbid IN (SELECT value FROM json_each(?))",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
}

/// Inserts an ebook or audiobook under its author (created if new), or
/// returns the existing one.
pub async fn upsert_book(
    pool: &Pool,
    b: &NewBook,
    kind: TitleKind,
    state: TitleState,
) -> Result<Title, Error> {
    debug_assert!(matches!(kind, TitleKind::Ebook | TitleKind::Audiobook));
    let profile: Option<i64> =
        sqlx::query_scalar("SELECT MIN(id) FROM quality_profiles").fetch_one(pool).await?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO titles (kind, olid, title, state, monitored) VALUES ('author', ?, ?, ?, 1)
         ON CONFLICT(kind, olid) WHERE olid IS NOT NULL DO UPDATE SET title = excluded.title",
    )
    .bind(&b.author_olid)
    .bind(&b.author)
    .bind(TitleState::Wanted.as_str())
    .execute(&mut *tx)
    .await?;
    let author: i64 = sqlx::query_scalar("SELECT id FROM titles WHERE kind = 'author' AND olid = ?")
        .bind(&b.author_olid)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO titles (kind, parent_id, olid, title, year, overview, poster_path, state,
            quality_profile_id, monitored, next_search_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)
         ON CONFLICT(kind, olid) WHERE olid IS NOT NULL DO NOTHING",
    )
    .bind(kind.as_str())
    .bind(author)
    .bind(&b.olid)
    .bind(&b.title)
    .bind(b.year)
    .bind(&b.overview)
    .bind(&b.poster_path)
    .bind(state.as_str())
    .bind(profile)
    .bind((state == TitleState::Wanted).then(now))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    by_olid(pool, kind, &b.olid).await?.ok_or(Error::RowNotFound)
}

/// A book or author by its Open Library id.
pub async fn by_olid(pool: &Pool, kind: TitleKind, olid: &str) -> Result<Option<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = ? AND olid = ?"))
        .bind(kind.as_str())
        .bind(olid)
        .fetch_optional(pool)
        .await
}

/// Open Library id, kind (`ebook` or `audiobook`), local id and state of
/// each book in `olids` that is in the library.
pub async fn book_states(pool: &Pool, olids: &[String]) -> Result<Vec<(String, String, i64, String)>, Error> {
    let ids = serde_json::to_string(olids).map_err(|e| Error::Encode(Box::new(e)))?;
    sqlx::query_as(
        "SELECT olid, kind, id, state FROM titles
         WHERE kind IN ('ebook', 'audiobook') AND olid IN (SELECT value FROM json_each(?))",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
}

/// Wanted ebooks and audiobooks whose next search is due.
pub async fn due_books(pool: &Pool, limit: i64) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE kind IN ('ebook', 'audiobook') AND state = 'wanted' AND monitored = 1
           AND (next_search_at IS NULL OR next_search_at <= ?)
         ORDER BY next_search_at LIMIT ?"
    ))
    .bind(now())
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Ebooks and audiobooks, newest change first.
pub async fn books(pool: &Pool) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind IN ('ebook', 'audiobook') ORDER BY updated_at DESC"))
        .fetch_all(pool)
        .await
}

/// Films, shows, seasons, episodes, artists, albums and tracks, for apps
/// that browse the library.
pub async fn app_titles(pool: &Pool) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE kind IN ('movie', 'show', 'season', 'episode', 'artist', 'album', 'track') ORDER BY id"
    ))
    .fetch_all(pool)
    .await
}

/// Albums, newest change first.
pub async fn albums(pool: &Pool) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!("WHERE kind = 'album' ORDER BY updated_at DESC")).fetch_all(pool).await
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
/// Wanted albums whose next search is due.
pub async fn due_albums(pool: &Pool, limit: i64) -> Result<Vec<Title>, Error> {
    sqlx::query_as(select_titles!(
        "WHERE kind = 'album' AND state = 'wanted' AND monitored = 1 AND (next_search_at IS NULL OR next_search_at <= ?)
         ORDER BY next_search_at LIMIT ?"
    ))
    .bind(now())
    .bind(limit)
    .fetch_all(pool)
    .await
}

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
        "UPDATE titles SET state = ?1, state_reason = ?2, updated_at = ?3,
            available_at = CASE WHEN ?1 = 'available' THEN ?3 ELSE available_at END
         WHERE id = ?4 AND state = ?5",
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
        show.seasons[1].episodes[0].title.release_date = Some("2011-04-17".into());
        let b = upsert_show(&pool, &show, TitleState::Wanted).await.unwrap();
        assert_eq!(a.id, b.id);
        let eps = episodes_of_show(&pool, a.id).await.unwrap();
        assert_eq!(eps.len(), 5);
        assert_eq!(eps[2].state(), TitleState::Searching);
        assert_eq!(eps[2].release_date.as_deref(), Some("2011-04-17"), "air dates are refreshed");
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
        set_monitored(&pool, eps[1].id, false).await.unwrap();
        assert!(!by_id(&pool, eps[1].id).await.unwrap().unwrap().monitored);
        let season = children(&pool, t.id).await.unwrap().remove(0);
        set_monitored(&pool, season.id, true).await.unwrap();
        assert!(
            episodes_of_show(&pool, t.id).await.unwrap().iter().all(|e| e.monitored),
            "a season covers its episodes"
        );
        assert_eq!(
            progress(t.id).await.state(TitleState::Wanted),
            TitleState::Available,
            "the unaired one does not count"
        );
        assert!(list(&pool, None).await.unwrap().is_empty(), "film lists leave shows out");
        assert!(due_for_search(&pool, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn albums_sit_under_their_artist() {
        let pool = crate::memory().await;
        let album = |mbid: &str, title: &str| NewAlbum {
            artist_mbid: "a1".into(),
            artist: "Band".into(),
            mbid: mbid.into(),
            title: title.into(),
            year: Some(2001),
            tracks: (1..=3)
                .map(|n| NewTrack {
                    number: n,
                    disc: 1,
                    title: format!("Song {n}"),
                    length_seconds: Some(200),
                })
                .collect(),
            ..Default::default()
        };
        let a = upsert_album(&pool, &album("rg1", "First"), TitleState::Requested).await.unwrap();
        let b = upsert_album(&pool, &album("rg2", "Second"), TitleState::Wanted).await.unwrap();
        assert_eq!((a.kind.as_str(), a.mbid.as_deref()), ("album", Some("rg1")));
        assert_eq!(a.parent_id, b.parent_id, "one artist row for both");
        let tracks = children(&pool, a.id).await.unwrap();
        assert_eq!(tracks.iter().map(|t| t.episode_number).collect::<Vec<_>>(), [Some(1), Some(2), Some(3)]);
        assert_eq!(tracks[0].runtime_minutes, Some(4));
        assert_eq!(upsert_album(&pool, &album("rg1", "First"), TitleState::Wanted).await.unwrap().id, a.id);
        assert_eq!(by_mbid(&pool, TitleKind::Album, "rg2").await.unwrap().unwrap().id, b.id);
        assert_eq!(album_states(&pool, &["rg1".into(), "nope".into()]).await.unwrap().len(), 1);
        assert_eq!(albums(&pool).await.unwrap().len(), 2);
        assert!(list(&pool, None).await.unwrap().is_empty(), "film lists leave music out");
    }

    #[tokio::test]
    async fn books_sit_under_their_author() {
        let pool = crate::memory().await;
        let hobbit = NewBook {
            author_olid: "OL26320A".into(),
            author: "J.R.R. Tolkien".into(),
            olid: "OL27482W".into(),
            title: "The Hobbit".into(),
            year: Some(1937),
            poster_path: Some("14627509".into()),
            ..Default::default()
        };
        let e = upsert_book(&pool, &hobbit, TitleKind::Ebook, TitleState::Wanted).await.unwrap();
        let a = upsert_book(&pool, &hobbit, TitleKind::Audiobook, TitleState::Requested).await.unwrap();
        assert_ne!(e.id, a.id, "an ebook and an audiobook of one book");
        assert_eq!(e.parent_id, a.parent_id, "one author row for both");
        assert!(e.next_search_at.is_some() && a.next_search_at.is_none());
        assert_eq!(upsert_book(&pool, &hobbit, TitleKind::Ebook, TitleState::Wanted).await.unwrap().id, e.id);
        let author = by_olid(&pool, TitleKind::Author, "OL26320A").await.unwrap().unwrap();
        assert_eq!(author.title, "J.R.R. Tolkien");
        let states = book_states(&pool, &["OL27482W".into(), "OL1W".into()]).await.unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(books(&pool).await.unwrap().len(), 2);
        assert!(list(&pool, None).await.unwrap().is_empty(), "film lists leave books out");
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
