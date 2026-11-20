//! MusicBrainz client: artists, their albums, and the tracks of an album.
//!
//! MusicBrainz asks for at most one request per second per client, so every
//! call waits its turn. No key is needed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::Error;

pub const DEFAULT_MUSIC_URL: &str = "https://musicbrainz.org/ws/2";
pub const COVER_ART_URL: &str = "https://coverartarchive.org";
const SPACING: Duration = Duration::from_millis(1100);

#[derive(Debug, Clone)]
pub struct MusicBrainz {
    http: reqwest::Client,
    base_url: String,
    last: Arc<Mutex<Option<Instant>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "sort-name")]
    pub sort_name: Option<String>,
    /// Person, Group, Orchestra...
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    /// Tells apart artists with the same name.
    #[serde(default)]
    pub disambiguation: Option<String>,
}

/// An album as a release group: every edition of it together.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlbumSummary {
    pub id: String,
    pub title: String,
    /// Album, EP, Single...
    #[serde(default, rename = "primary-type")]
    pub primary_type: Option<String>,
    /// Compilation, Live, Soundtrack...
    #[serde(default, rename = "secondary-types")]
    pub secondary_types: Vec<String>,
    #[serde(default, rename = "first-release-date")]
    pub first_release_date: Option<String>,
}

impl AlbumSummary {
    pub fn year(&self) -> Option<i64> {
        self.first_release_date.as_deref()?.get(..4)?.parse().ok()
    }

    /// A studio album or EP, not a single, compilation or live recording.
    pub fn is_studio(&self) -> bool {
        matches!(self.primary_type.as_deref(), Some("Album" | "EP")) && self.secondary_types.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlbumTrack {
    /// Position across the whole album, from 1.
    pub number: u32,
    pub disc: u32,
    /// Position on its disc.
    pub position: u32,
    pub title: String,
    pub length_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlbumDetails {
    /// Release group id.
    pub id: String,
    pub title: String,
    pub artist: String,
    /// MusicBrainz id of the first credited artist.
    pub artist_id: Option<String>,
    pub first_release_date: Option<String>,
    /// The edition whose track list is used (the earliest official one).
    pub release_id: String,
    pub tracks: Vec<AlbumTrack>,
}

/// Album cover from the Cover Art Archive; `size` is 250, 500 or 1200.
pub fn cover_url(album_id: &str, size: u32) -> String {
    format!("{COVER_ART_URL}/release-group/{album_id}/front-{size}")
}

impl MusicBrainz {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_MUSIC_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("moka/", env!("CARGO_PKG_VERSION"), " ( https://github.com/openVESSL/Moka )"))
            .build()
            .expect("http client builds");
        Self { http, base_url: base_url.into().trim_end_matches('/').to_owned(), last: Default::default() }
    }

    pub async fn search_artists(&self, query: &str) -> Result<Vec<ArtistSummary>, Error> {
        #[derive(Deserialize)]
        struct Found {
            #[serde(default)]
            artists: Vec<ArtistSummary>,
        }
        let found: Found = self.get("/artist", &[("query", query), ("limit", "15")]).await?;
        Ok(found.artists)
    }

    pub async fn artist(&self, id: &str) -> Result<ArtistSummary, Error> {
        self.get(&format!("/artist/{id}"), &[]).await
    }

    /// Albums and EPs of an artist, oldest first.
    pub async fn albums(&self, artist_id: &str) -> Result<Vec<AlbumSummary>, Error> {
        #[derive(Deserialize)]
        struct Found {
            #[serde(default, rename = "release-groups")]
            groups: Vec<AlbumSummary>,
        }
        let found: Found = self
            .get("/release-group", &[("artist", artist_id), ("type", "album|ep"), ("limit", "100")])
            .await?;
        let mut albums = found.groups;
        albums.sort_by(|a, b| a.first_release_date.cmp(&b.first_release_date));
        Ok(albums)
    }

    /// An album with the track list of its earliest official edition.
    pub async fn album(&self, id: &str) -> Result<AlbumDetails, Error> {
        #[derive(Deserialize)]
        struct CreditArtist {
            id: String,
        }
        #[derive(Deserialize)]
        struct Credit {
            name: String,
            #[serde(default)]
            artist: Option<CreditArtist>,
        }
        #[derive(Deserialize)]
        struct Track {
            #[serde(default)]
            position: u32,
            title: String,
            #[serde(default)]
            length: Option<u64>,
        }
        #[derive(Deserialize)]
        struct Medium {
            #[serde(default)]
            position: u32,
            #[serde(default)]
            tracks: Vec<Track>,
        }
        #[derive(Deserialize)]
        struct Release {
            id: String,
            title: String,
            #[serde(default)]
            date: Option<String>,
            #[serde(default)]
            status: Option<String>,
            #[serde(default)]
            media: Vec<Medium>,
            #[serde(default, rename = "artist-credit")]
            credits: Vec<Credit>,
        }
        #[derive(Deserialize)]
        struct Found {
            #[serde(default)]
            releases: Vec<Release>,
        }
        let found: Found = self
            .get(
                "/release",
                &[("release-group", id), ("inc", "recordings+media+artist-credits"), ("limit", "50")],
            )
            .await?;
        let release = found
            .releases
            .into_iter()
            .filter(|r| r.media.iter().any(|m| !m.tracks.is_empty()))
            // official editions first, then the earliest; undated last
            .min_by_key(|r| {
                (
                    r.status.as_deref() != Some("Official"),
                    r.date.clone().filter(|d| !d.is_empty()).is_none(),
                    r.date.clone(),
                )
            })
            .ok_or(Error::NotFound)?;
        let mut tracks = Vec::new();
        for m in &release.media {
            for t in &m.tracks {
                tracks.push(AlbumTrack {
                    number: tracks.len() as u32 + 1,
                    disc: m.position.max(1),
                    position: t.position,
                    title: t.title.clone(),
                    length_ms: t.length,
                });
            }
        }
        Ok(AlbumDetails {
            id: id.to_owned(),
            title: release.title,
            artist: release.credits.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "),
            artist_id: release.credits.first().and_then(|c| c.artist.as_ref()).map(|a| a.id.clone()),
            first_release_date: release.date.filter(|d| !d.is_empty()),
            release_id: release.id,
            tracks,
        })
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, Error> {
        // MusicBrainz often answers "busy" (503): wait a little longer each time
        for attempt in 0..3u64 {
            {
                // one request at a time, spaced out as MusicBrainz asks
                let mut last = self.last.lock().await;
                let spacing = SPACING * (1 + 2 * attempt as u32);
                if let Some(at) = *last {
                    let wait = spacing.saturating_sub(at.elapsed());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                }
                *last = Some(Instant::now());
            }
            let res = self
                .http
                .get(format!("{}{path}", self.base_url))
                .query(&[("fmt", "json")])
                .query(query)
                .send()
                .await?;
            match res.status().as_u16() {
                200..=299 => return Ok(res.json().await?),
                400 | 404 => return Err(Error::NotFound),
                503 | 429 if attempt < 2 => continue,
                503 | 429 => return Err(Error::RateLimited),
                s => return Err(Error::Status(s)),
            }
        }
        Err(Error::RateLimited)
    }
}

impl Default for MusicBrainz {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::get};

    async fn mock() -> String {
        let app = Router::new()
            .route(
                "/artist",
                get(|| async {
                    Json(serde_json::json!({ "artists": [
                        { "id": "a1", "name": "Band", "sort-name": "Band", "type": "Group", "country": "GB" }] }))
                }),
            )
            .route(
                "/release-group",
                get(|| async {
                    Json(serde_json::json!({ "release-groups": [
                        { "id": "rg2", "title": "Second", "primary-type": "Album", "secondary-types": [], "first-release-date": "2004-05-01" },
                        { "id": "rg1", "title": "First", "primary-type": "Album", "secondary-types": [], "first-release-date": "2001-01-01" },
                        { "id": "rg3", "title": "Live at Home", "primary-type": "Album", "secondary-types": ["Live"], "first-release-date": "2006" }] }))
                }),
            )
            .route(
                "/release",
                get(|| async {
                    Json(serde_json::json!({ "releases": [
                        { "id": "late", "title": "First", "date": "2010-01-01", "status": "Official",
                          "media": [{ "position": 1, "tracks": [{ "position": 1, "title": "Remaster", "length": 1 }] }] },
                        { "id": "orig", "title": "First", "date": "2001-01-01", "status": "Official",
                          "artist-credit": [{ "name": "Band", "artist": { "id": "a1" } }],
                          "media": [{ "position": 1, "tracks": [{ "position": 1, "title": "Opening", "length": 200000 },
                                                                 { "position": 2, "title": "Middle" }] },
                                    { "position": 2, "tracks": [{ "position": 1, "title": "Bonus" }] }] }] }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn artists_albums_and_tracks() {
        let mb = MusicBrainz::with_base_url(mock().await);
        assert_eq!(mb.search_artists("band").await.unwrap()[0].kind.as_deref(), Some("Group"));
        let albums = mb.albums("a1").await.unwrap();
        assert_eq!(albums.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["rg1", "rg2", "rg3"]);
        assert!(albums[0].is_studio() && !albums[2].is_studio());
        assert_eq!(albums[1].year(), Some(2004));

        let started = Instant::now();
        let album = mb.album("rg1").await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(1000), "calls are spaced out");
        assert_eq!((album.release_id.as_str(), album.artist.as_str()), ("orig", "Band"));
        assert_eq!(album.artist_id.as_deref(), Some("a1"));
        let t: Vec<(u32, u32, u32, &str)> =
            album.tracks.iter().map(|t| (t.number, t.disc, t.position, t.title.as_str())).collect();
        assert_eq!(t, [(1, 1, 1, "Opening"), (2, 1, 2, "Middle"), (3, 2, 1, "Bonus")]);
        assert_eq!(cover_url("rg1", 250), "https://coverartarchive.org/release-group/rg1/front-250");
    }
}
