//! Catalog clients: TMDB for film and show lists, search and details
//! (here), MusicBrainz for artists and albums (`music`).
//!
//! Knows nothing about the database. Takes a base URL so tests can point it
//! at a local mock server.

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub mod music;

pub const DEFAULT_BASE_URL: &str = "https://api.themoviedb.org/3";
pub const IMAGE_BASE_URL: &str = "https://image.tmdb.org/t/p";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("TMDB API key is not set")]
    MissingKey,
    #[error("TMDB rejected the API key")]
    Unauthorized,
    #[error("not found in the catalog")]
    NotFound,
    #[error("the catalog service is busy or rate limiting requests, retry shortly")]
    RateLimited,
    #[error("catalog request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("the catalog service returned status {0}")]
    Status(u16),
}

#[derive(Debug, Clone)]
pub struct Tmdb {
    http: reqwest::Client,
    base_url: String,
    key: String,
    language: String,
    region: Option<String>,
}

/// One film in a list or search result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MovieSummary {
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub release_date: Option<String>,
    #[serde(default)]
    pub poster_path: Option<String>,
    #[serde(default)]
    pub backdrop_path: Option<String>,
    #[serde(default)]
    pub vote_average: f64,
    #[serde(default)]
    pub vote_count: i64,
    #[serde(default)]
    pub original_language: Option<String>,
}

impl MovieSummary {
    pub fn year(&self) -> Option<i64> {
        year_of(self.release_date.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub page: i64,
    pub results: Vec<T>,
    pub total_pages: i64,
    pub total_results: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Genre {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastMember {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub character: Option<String>,
    #[serde(default)]
    pub profile_path: Option<String>,
    #[serde(default)]
    pub order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrewMember {
    pub id: i64,
    pub name: String,
    pub job: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credits {
    #[serde(default)]
    pub cast: Vec<CastMember>,
    #[serde(default)]
    pub crew: Vec<CrewMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub key: String,
    pub site: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub official: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Videos {
    #[serde(default)]
    pub results: Vec<Video>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalIds {
    #[serde(default)]
    pub imdb_id: Option<String>,
    /// Shows only.
    #[serde(default)]
    pub tvdb_id: Option<i64>,
}

/// One show in a list or search result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShowSummary {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub original_name: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub first_air_date: Option<String>,
    #[serde(default)]
    pub poster_path: Option<String>,
    #[serde(default)]
    pub backdrop_path: Option<String>,
    #[serde(default)]
    pub vote_average: f64,
    #[serde(default)]
    pub vote_count: i64,
    #[serde(default)]
    pub original_language: Option<String>,
}

impl ShowSummary {
    pub fn year(&self) -> Option<i64> {
        year_of(self.first_air_date.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonSummary {
    pub id: i64,
    pub season_number: i64,
    pub name: String,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub air_date: Option<String>,
    #[serde(default)]
    pub episode_count: i64,
    #[serde(default)]
    pub poster_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowDetails {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub original_name: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub tagline: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub first_air_date: Option<String>,
    /// Typical episode lengths in minutes.
    #[serde(default)]
    pub episode_run_time: Vec<i64>,
    #[serde(default)]
    pub poster_path: Option<String>,
    #[serde(default)]
    pub backdrop_path: Option<String>,
    #[serde(default)]
    pub vote_average: f64,
    #[serde(default)]
    pub genres: Vec<Genre>,
    /// e.g. "Returning Series", "Ended".
    #[serde(default)]
    pub status: Option<String>,
    /// e.g. "Scripted", "Talk Show", "News", "Reality".
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub original_language: Option<String>,
    #[serde(default)]
    pub seasons: Vec<SeasonSummary>,
    #[serde(default)]
    pub credits: Credits,
    #[serde(default)]
    pub videos: Videos,
    #[serde(default)]
    pub external_ids: ExternalIds,
}

impl ShowDetails {
    /// How releases of this show are numbered: `daily` (by air date),
    /// `anime` (often by absolute episode) or `standard` (S01E02).
    pub fn numbering(&self) -> &'static str {
        if matches!(self.kind.as_deref(), Some("Talk Show" | "News")) {
            "daily"
        } else if self.original_language.as_deref() == Some("ja") && self.genres.iter().any(|g| g.id == 16) {
            "anime"
        } else {
            "standard"
        }
    }

    pub fn year(&self) -> Option<i64> {
        year_of(self.first_air_date.as_deref())
    }

    pub fn runtime(&self) -> Option<i64> {
        self.episode_run_time.first().copied()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: i64,
    pub season_number: i64,
    pub episode_number: i64,
    pub name: String,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub air_date: Option<String>,
    #[serde(default)]
    pub runtime: Option<i64>,
    #[serde(default)]
    pub still_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonDetails {
    pub id: i64,
    pub season_number: i64,
    pub name: String,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub air_date: Option<String>,
    #[serde(default)]
    pub poster_path: Option<String>,
    #[serde(default)]
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovieDetails {
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub tagline: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub release_date: Option<String>,
    #[serde(default)]
    pub runtime: Option<i64>,
    #[serde(default)]
    pub poster_path: Option<String>,
    #[serde(default)]
    pub backdrop_path: Option<String>,
    #[serde(default)]
    pub vote_average: f64,
    #[serde(default)]
    pub imdb_id: Option<String>,
    #[serde(default)]
    pub genres: Vec<Genre>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub credits: Credits,
    #[serde(default)]
    pub videos: Videos,
    #[serde(default)]
    pub external_ids: ExternalIds,
}

impl MovieDetails {
    pub fn year(&self) -> Option<i64> {
        year_of(self.release_date.as_deref())
    }

    pub fn imdb(&self) -> Option<&str> {
        self.imdb_id.as_deref().or(self.external_ids.imdb_id.as_deref()).filter(|s| !s.is_empty())
    }

    pub fn directors(&self) -> Vec<&str> {
        self.credits.crew.iter().filter(|c| c.job == "Director").map(|c| c.name.as_str()).collect()
    }

    /// First official YouTube trailer, if any.
    pub fn trailer_key(&self) -> Option<&str> {
        let yt = |v: &&Video| v.site == "YouTube" && v.kind == "Trailer";
        self.videos
            .results
            .iter()
            .filter(yt)
            .find(|v| v.official)
            .or_else(|| self.videos.results.iter().find(yt))
            .map(|v| v.key.as_str())
    }
}

/// Catalog lists the home page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum List {
    Trending,
    Popular,
    Upcoming,
    NowPlaying,
    TopRated,
}

impl List {
    fn path(self) -> &'static str {
        match self {
            List::Trending => "/trending/movie/week",
            List::Popular => "/movie/popular",
            List::Upcoming => "/movie/upcoming",
            List::NowPlaying => "/movie/now_playing",
            List::TopRated => "/movie/top_rated",
        }
    }

    fn show_path(self) -> &'static str {
        match self {
            List::Trending => "/trending/tv/week",
            List::Popular => "/tv/popular",
            // shows have no release window; "on the air" is the closest
            List::Upcoming | List::NowPlaying => "/tv/on_the_air",
            List::TopRated => "/tv/top_rated",
        }
    }
}

impl Tmdb {
    pub fn new(key: impl Into<String>, language: impl Into<String>, region: Option<String>) -> Self {
        Self::with_base_url(DEFAULT_BASE_URL, key, language, region)
    }

    pub fn with_base_url(
        base_url: impl Into<String>,
        key: impl Into<String>,
        language: impl Into<String>,
        region: Option<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("moka/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client builds");
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            key: key.into(),
            language: language.into(),
            region: region.filter(|r| !r.is_empty()),
        }
    }

    pub async fn list(&self, list: List, page: u32) -> Result<Page<MovieSummary>, Error> {
        let page = page.max(1).to_string();
        let mut q = vec![("page", page.as_str())];
        if let Some(r) = &self.region {
            q.push(("region", r));
        }
        self.get(list.path(), &q).await
    }

    pub async fn search(&self, query: &str, page: u32) -> Result<Page<MovieSummary>, Error> {
        let page = page.max(1).to_string();
        self.get("/search/movie", &[("query", query), ("page", &page), ("include_adult", "false")]).await
    }

    pub async fn movie(&self, id: i64) -> Result<MovieDetails, Error> {
        self.get(
            &format!("/movie/{id}"),
            &[("append_to_response", "credits,videos,external_ids,release_dates")],
        )
        .await
    }

    pub async fn recommendations(&self, id: i64) -> Result<Page<MovieSummary>, Error> {
        self.get(&format!("/movie/{id}/recommendations"), &[]).await
    }

    pub async fn show_list(&self, list: List, page: u32) -> Result<Page<ShowSummary>, Error> {
        let page = page.max(1).to_string();
        self.get(list.show_path(), &[("page", page.as_str())]).await
    }

    pub async fn search_shows(&self, query: &str, page: u32) -> Result<Page<ShowSummary>, Error> {
        let page = page.max(1).to_string();
        self.get("/search/tv", &[("query", query), ("page", &page), ("include_adult", "false")]).await
    }

    pub async fn show(&self, id: i64) -> Result<ShowDetails, Error> {
        self.get(&format!("/tv/{id}"), &[("append_to_response", "credits,videos,external_ids")]).await
    }

    /// The TMDB id of the show with this TheTVDB id.
    pub async fn show_by_tvdb(&self, tvdb_id: i64) -> Result<i64, Error> {
        #[derive(Deserialize)]
        struct Found {
            #[serde(default)]
            tv_results: Vec<ShowSummary>,
        }
        let found: Found = self.get(&format!("/find/{tvdb_id}"), &[("external_source", "tvdb_id")]).await?;
        found.tv_results.first().map(|s| s.id).ok_or(Error::NotFound)
    }

    pub async fn season(&self, show_id: i64, season: i64) -> Result<SeasonDetails, Error> {
        self.get(&format!("/tv/{show_id}/season/{season}"), &[]).await
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, Error> {
        if self.key.is_empty() {
            return Err(Error::MissingKey);
        }
        let mut req = self
            .http
            .get(format!("{}{path}", self.base_url))
            .query(&[("language", self.language.as_str())])
            .query(query);
        // v4 read access tokens are JWTs; v3 keys go in the query string
        req = if self.key.starts_with("eyJ") {
            req.bearer_auth(&self.key)
        } else {
            req.query(&[("api_key", self.key.as_str())])
        };
        let res = req.send().await?;
        match res.status().as_u16() {
            200..=299 => Ok(res.json().await?),
            401 => Err(Error::Unauthorized),
            404 => Err(Error::NotFound),
            429 => Err(Error::RateLimited),
            s => Err(Error::Status(s)),
        }
    }
}

fn year_of(date: Option<&str>) -> Option<i64> {
    date?.get(..4)?.parse().ok()
}

fn empty_as_none<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s: Option<String> = Option::deserialize(d)?;
    Ok(s.filter(|s| !s.is_empty()))
}

/// Allowed image sizes for the proxy. Anything else is rejected.
pub const IMAGE_SIZES: &[&str] = &["w92", "w185", "w342", "w500", "w780", "w1280", "original"];

/// Validates an image file name from a TMDB path (`/abc123.jpg` → `abc123.jpg`)
/// so it is safe to use in a URL and as a cache file name.
pub fn safe_image_name(name: &str) -> Option<&str> {
    let name = name.trim_start_matches('/');
    let (stem, ext) = name.rsplit_once('.')?;
    let ok_stem = !stem.is_empty()
        && stem.len() <= 64
        && stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    let ok_ext = matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "svg");
    (ok_stem && ok_ext).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Query, http::StatusCode, routing::get};
    use std::collections::HashMap;

    async fn mock() -> String {
        let app = Router::new()
            .route(
                "/trending/movie/week",
                get(|Query(q): Query<HashMap<String, String>>| async move {
                    if q.get("api_key").map(String::as_str) != Some("good") {
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                    Ok(Json(serde_json::json!({
                        "page": 1, "total_pages": 1, "total_results": 1,
                        "results": [{ "id": 603, "title": "The Matrix", "release_date": "1999-03-30",
                                      "poster_path": "/p.jpg", "vote_average": 8.2, "vote_count": 25000 },
                                    { "id": 1, "title": "Undated", "release_date": "" }]
                    })))
                }),
            )
            .route(
                "/movie/603",
                get(|| async {
                    Json(serde_json::json!({
                        "id": 603, "title": "The Matrix", "release_date": "1999-03-30", "runtime": 136,
                        "imdb_id": "tt0133093", "genres": [{"id": 28, "name": "Action"}],
                        "credits": { "cast": [{"id": 1, "name": "Keanu Reeves", "character": "Neo", "order": 0}],
                                     "crew": [{"id": 2, "name": "Lana Wachowski", "job": "Director"},
                                              {"id": 3, "name": "Someone", "job": "Editor"}] },
                        "videos": { "results": [
                            {"key": "teaser", "site": "YouTube", "type": "Teaser", "official": true},
                            {"key": "fan", "site": "YouTube", "type": "Trailer", "official": false},
                            {"key": "real", "site": "YouTube", "type": "Trailer", "official": true}] }
                    }))
                }),
            );
        let app = app
            .route(
                "/tv/1399",
                get(|| async {
                    Json(serde_json::json!({
                        "id": 1399, "name": "Thrones", "first_air_date": "2011-04-17", "episode_run_time": [60],
                        "status": "Ended", "external_ids": { "imdb_id": "tt0944947", "tvdb_id": 121361 },
                        "seasons": [
                            {"id": 3627, "season_number": 0, "name": "Specials", "episode_count": 2, "air_date": ""},
                            {"id": 3624, "season_number": 1, "name": "Season 1", "episode_count": 10, "air_date": "2011-04-17"}]
                    }))
                }),
            )
            .route(
                "/tv/1399/season/1",
                get(|| async {
                    Json(serde_json::json!({
                        "id": 3624, "season_number": 1, "name": "Season 1",
                        "episodes": [{"id": 63056, "season_number": 1, "episode_number": 1, "name": "Winter",
                                      "air_date": "2011-04-17", "runtime": 62}]
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn lists_and_details_parse() {
        let base = mock().await;
        let tmdb = Tmdb::with_base_url(&base, "good", "en-US", None);
        let page = tmdb.list(List::Trending, 1).await.unwrap();
        assert_eq!(page.results[0].year(), Some(1999));
        assert_eq!(page.results[1].release_date, None, "empty date becomes None");

        let m = tmdb.movie(603).await.unwrap();
        assert_eq!(m.runtime, Some(136));
        assert_eq!(m.imdb(), Some("tt0133093"));
        assert_eq!(m.directors(), vec!["Lana Wachowski"]);
        assert_eq!(m.trailer_key(), Some("real"));

        let show = tmdb.show(1399).await.unwrap();
        assert_eq!((show.year(), show.runtime()), (Some(2011), Some(60)));
        assert_eq!(show.external_ids.tvdb_id, Some(121361));
        assert_eq!(show.seasons[0].air_date, None);
        let season = tmdb.season(1399, 1).await.unwrap();
        assert_eq!((season.episodes[0].episode_number, season.episodes[0].runtime), (1, Some(62)));
    }

    #[tokio::test]
    async fn auth_and_missing_key_errors() {
        let base = mock().await;
        let bad = Tmdb::with_base_url(&base, "bad", "en-US", None);
        assert!(matches!(bad.list(List::Trending, 1).await, Err(Error::Unauthorized)));
        let none = Tmdb::with_base_url(&base, "", "en-US", None);
        assert!(matches!(none.list(List::Trending, 1).await, Err(Error::MissingKey)));
        let missing = Tmdb::with_base_url(&base, "good", "en-US", None);
        assert!(matches!(missing.movie(1).await, Err(Error::NotFound)));
    }

    #[test]
    fn image_names_are_validated() {
        assert_eq!(safe_image_name("/abc_DEF-123.jpg"), Some("abc_DEF-123.jpg"));
        assert_eq!(safe_image_name("../etc/passwd"), None);
        assert_eq!(safe_image_name("a/b.jpg"), None);
        assert_eq!(safe_image_name("x.exe"), None);
        assert_eq!(safe_image_name(".jpg"), None);
    }
}
