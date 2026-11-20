//! Music catalog from MusicBrainz: artists, their albums and album track
//! lists, annotated with what is already in the library.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::catalog::LocalState;
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/catalog/music/search", get(search))
        .route("/catalog/music/artist/{mbid}", get(artist))
        .route("/catalog/music/album/{mbid}", get(album))
}

const DAY: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Serialize, ToSchema)]
pub struct MusicArtist {
    pub mbid: String,
    pub name: String,
    /// Person, Group...
    pub kind: Option<String>,
    pub country: Option<String>,
    pub disambiguation: Option<String>,
}

impl From<koda_metadata::music::ArtistSummary> for MusicArtist {
    fn from(a: koda_metadata::music::ArtistSummary) -> Self {
        Self {
            mbid: a.id,
            name: a.name,
            kind: a.kind,
            country: a.country,
            disambiguation: a.disambiguation.filter(|d| !d.is_empty()),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MusicAlbum {
    pub mbid: String,
    pub title: String,
    pub year: Option<i64>,
    /// Album, EP...
    pub kind: Option<String>,
    /// A studio album or EP (not live, not a compilation).
    pub studio: bool,
    pub local: Option<LocalState>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MusicArtistPage {
    pub artist: MusicArtist,
    pub albums: Vec<MusicAlbum>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MusicTrack {
    pub number: u32,
    pub disc: u32,
    pub title: String,
    pub length_seconds: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MusicAlbumPage {
    pub mbid: String,
    pub title: String,
    pub artist: String,
    pub artist_mbid: Option<String>,
    pub year: Option<i64>,
    pub tracks: Vec<MusicTrack>,
    pub local: Option<LocalState>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    pub q: String,
}

#[utoipa::path(get, path = "/api/v1/catalog/music/search", operation_id = "search_music", tag = "catalog", params(SearchQuery),
    responses((status = 200, body = [MusicArtist])))]
pub async fn search(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<MusicArtist>>> {
    let query = q.q.trim();
    if query.is_empty() {
        return Ok(Json(vec![]));
    }
    if query.len() > 200 {
        return Err(ApiError::BadRequest("search text is too long".into()));
    }
    let mb = state.music();
    let found = state
        .cached(format!("music-search:{}", query.to_lowercase()), Duration::from_secs(600), || {
            mb.search_artists(query)
        })
        .await?;
    Ok(Json(found.into_iter().map(MusicArtist::from).collect()))
}

async fn local_albums(state: &AppState, mbids: &[String]) -> ApiResult<HashMap<String, LocalState>> {
    let rows = koda_db::titles::album_states(&state.db, mbids).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(mbid, title_id, s)| Some((mbid, LocalState { title_id, state: s.parse().ok()? })))
        .collect())
}

#[utoipa::path(get, path = "/api/v1/catalog/music/artist/{mbid}", operation_id = "music_artist", tag = "catalog",
    params(("mbid" = String, Path)), responses((status = 200, body = MusicArtistPage), (status = 404)))]
pub async fn artist(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(mbid): Path<String>,
) -> ApiResult<Json<MusicArtistPage>> {
    let mb = state.music();
    let artist = state.cached(format!("music-artist:{mbid}"), DAY, || mb.artist(&mbid)).await?;
    let albums = state.cached(format!("music-albums:{mbid}"), DAY, || mb.albums(&mbid)).await?;
    let ids: Vec<String> = albums.iter().map(|a| a.id.clone()).collect();
    let mut local = local_albums(&state, &ids).await?;
    let albums = albums
        .into_iter()
        .map(|a| MusicAlbum {
            year: a.year(),
            studio: a.is_studio(),
            local: local.remove(&a.id),
            mbid: a.id,
            title: a.title,
            kind: a.primary_type,
        })
        .collect();
    Ok(Json(MusicArtistPage { artist: artist.into(), albums }))
}

async fn album_details(state: &AppState, mbid: &str) -> ApiResult<koda_metadata::music::AlbumDetails> {
    let mb = state.music();
    Ok(state.cached(format!("music-album:{mbid}"), DAY, || mb.album(mbid)).await?)
}

#[utoipa::path(get, path = "/api/v1/catalog/music/album/{mbid}", operation_id = "music_album", tag = "catalog",
    params(("mbid" = String, Path)), responses((status = 200, body = MusicAlbumPage), (status = 404)))]
pub async fn album(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(mbid): Path<String>,
) -> ApiResult<Json<MusicAlbumPage>> {
    let a = album_details(&state, &mbid).await?;
    let local = local_albums(&state, std::slice::from_ref(&mbid)).await?.remove(&mbid);
    Ok(Json(MusicAlbumPage {
        year: a.first_release_date.as_deref().and_then(|d| d.get(..4)?.parse().ok()),
        tracks: a
            .tracks
            .iter()
            .map(|t| MusicTrack {
                number: t.number,
                disc: t.disc,
                title: t.title.clone(),
                length_seconds: t.length_ms.map(|ms| ms / 1000),
            })
            .collect(),
        mbid: a.id,
        title: a.title,
        artist: a.artist,
        artist_mbid: a.artist_id,
        local,
    }))
}

/// An album with its artist and tracks, ready to store.
pub(crate) async fn new_album(state: &AppState, mbid: &str) -> ApiResult<koda_db::titles::NewAlbum> {
    let a = album_details(state, mbid).await?;
    Ok(koda_db::titles::NewAlbum {
        artist_mbid: a.artist_id.clone().unwrap_or_else(|| format!("unknown:{}", a.artist)),
        artist: a.artist.clone(),
        mbid: a.id.clone(),
        title: a.title.clone(),
        year: a.first_release_date.as_deref().and_then(|d| d.get(..4)?.parse().ok()),
        release_date: a.first_release_date.clone(),
        poster_path: None,
        tracks: a
            .tracks
            .iter()
            .map(|t| koda_db::titles::NewTrack {
                number: i64::from(t.number),
                disc: i64::from(t.disc),
                title: t.title.clone(),
                length_seconds: t.length_ms.map(|ms| (ms / 1000) as i64),
            })
            .collect(),
    })
}
