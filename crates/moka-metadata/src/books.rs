//! Open Library client: books (works, every edition together), their
//! authors and covers. No key is needed; calls are spaced out to stay polite.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::Error;

pub const DEFAULT_BOOKS_URL: &str = "https://openlibrary.org";
pub const COVERS_URL: &str = "https://covers.openlibrary.org";
const SPACING: Duration = Duration::from_millis(400);
/// Fields asked of a search, so answers stay small.
const FIELDS: &str = "key,title,author_name,author_key,first_publish_year,cover_i,edition_count";

#[derive(Debug, Clone)]
pub struct OpenLibrary {
    http: reqwest::Client,
    base_url: String,
    last: Arc<Mutex<Option<Instant>>>,
}

/// A book as a work: every edition and translation together.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BookSummary {
    /// Work id, `OL27482W`.
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    /// Author ids, `OL26320A`, in the order of `authors`.
    pub author_ids: Vec<String>,
    pub year: Option<i64>,
    /// Open Library cover id, for `cover_url`.
    pub cover_id: Option<i64>,
    pub editions: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BookDetails {
    #[serde(flatten)]
    pub summary: BookSummary,
    pub description: Option<String>,
    pub subjects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Author {
    /// Author id, `OL26320A`.
    pub id: String,
    pub name: String,
    pub bio: Option<String>,
    pub birth_date: Option<String>,
    pub death_date: Option<String>,
    pub photo_id: Option<i64>,
}

/// A work (`OL…W`) or author (`OL…A`) id. Anything else is refused before a
/// request is made, since ids go into the request path.
pub fn valid_id(id: &str, kind: char) -> bool {
    id.len() < 20
        && id.starts_with("OL")
        && id.ends_with(kind)
        && id.len() > 3
        && id[2..id.len() - 1].bytes().all(|b| b.is_ascii_digit())
}

/// Book cover; `size` is `S`, `M` or `L`.
pub fn cover_url(cover_id: i64, size: char) -> String {
    format!("{COVERS_URL}/b/id/{cover_id}-{size}.jpg")
}

/// Author photo; `size` is `S`, `M` or `L`.
pub fn photo_url(photo_id: i64, size: char) -> String {
    format!("{COVERS_URL}/a/id/{photo_id}-{size}.jpg")
}

/// Open Library writes text either as a string or as `{ "value": … }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Text {
    Plain(String),
    Typed { value: String },
}

impl Text {
    fn into_string(self) -> String {
        match self {
            Text::Plain(s) | Text::Typed { value: s } => s.trim().to_owned(),
        }
    }
}

#[derive(Deserialize)]
struct SearchDoc {
    key: String,
    title: String,
    #[serde(default)]
    author_name: Vec<String>,
    #[serde(default)]
    author_key: Vec<String>,
    #[serde(default)]
    first_publish_year: Option<i64>,
    #[serde(default)]
    cover_i: Option<i64>,
    #[serde(default)]
    edition_count: Option<i64>,
}

impl From<SearchDoc> for BookSummary {
    fn from(d: SearchDoc) -> Self {
        Self {
            id: d.key.trim_start_matches("/works/").to_owned(),
            title: d.title,
            authors: d.author_name,
            author_ids: d.author_key,
            year: d.first_publish_year,
            cover_id: d.cover_i,
            editions: d.edition_count,
        }
    }
}

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

impl OpenLibrary {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_BOOKS_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("moka/", env!("CARGO_PKG_VERSION"), " ( https://github.com/openVESSL/Moka )"))
            .build()
            .expect("http client builds");
        Self { http, base_url: base_url.into().trim_end_matches('/').to_owned(), last: Default::default() }
    }

    /// Books whose title or author matches, most edited first.
    pub async fn search(&self, query: &str) -> Result<Vec<BookSummary>, Error> {
        let found: Search =
            self.get("/search.json", &[("q", query), ("fields", FIELDS), ("limit", "20")]).await?;
        Ok(found.docs.into_iter().filter(|d| d.key.starts_with("/works/")).map(Into::into).collect())
    }

    /// A book with its description.
    pub async fn book(&self, id: &str) -> Result<BookDetails, Error> {
        if !valid_id(id, 'W') {
            return Err(Error::NotFound);
        }
        #[derive(Deserialize)]
        struct Work {
            #[serde(default)]
            description: Option<Text>,
            #[serde(default)]
            subjects: Vec<String>,
        }
        // the search index holds authors and the first year; the work holds the text
        let key = format!("key:/works/{id}");
        let found: Search =
            self.get("/search.json", &[("q", &key), ("fields", FIELDS), ("limit", "1")]).await?;
        let summary: BookSummary = found.docs.into_iter().next().ok_or(Error::NotFound)?.into();
        let work: Work = self.get(&format!("/works/{id}.json"), &[]).await?;
        Ok(BookDetails {
            summary,
            description: work.description.map(Text::into_string).filter(|d| !d.is_empty()),
            subjects: work.subjects.into_iter().take(12).collect(),
        })
    }

    pub async fn author(&self, id: &str) -> Result<Author, Error> {
        if !valid_id(id, 'A') {
            return Err(Error::NotFound);
        }
        #[derive(Deserialize)]
        struct Found {
            name: String,
            #[serde(default)]
            bio: Option<Text>,
            #[serde(default)]
            birth_date: Option<String>,
            #[serde(default)]
            death_date: Option<String>,
            #[serde(default)]
            photos: Vec<i64>,
        }
        let a: Found = self.get(&format!("/authors/{id}.json"), &[]).await?;
        Ok(Author {
            id: id.to_owned(),
            name: a.name,
            bio: a.bio.map(Text::into_string).filter(|b| !b.is_empty()),
            birth_date: a.birth_date,
            death_date: a.death_date,
            // -1 marks a removed photo
            photo_id: a.photos.into_iter().find(|p| *p > 0),
        })
    }

    /// An author's books, most edited first.
    pub async fn author_books(&self, id: &str) -> Result<Vec<BookSummary>, Error> {
        if !valid_id(id, 'A') {
            return Err(Error::NotFound);
        }
        let found: Search = self
            .get(
                "/search.json",
                &[("author_key", id), ("sort", "editions"), ("fields", FIELDS), ("limit", "100")],
            )
            .await?;
        Ok(found.docs.into_iter().filter(|d| d.key.starts_with("/works/")).map(Into::into).collect())
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, Error> {
        for attempt in 0..3u32 {
            {
                let mut last = self.last.lock().await;
                let spacing = SPACING * (1 + 4 * attempt);
                if let Some(at) = *last {
                    let wait = spacing.saturating_sub(at.elapsed());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                }
                *last = Some(Instant::now());
            }
            let res = self.http.get(format!("{}{path}", self.base_url)).query(query).send().await?;
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

impl Default for OpenLibrary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Query;
    use axum::{Json, Router, routing::get};
    use std::collections::HashMap;

    async fn mock() -> String {
        let app = Router::new()
            .route(
                "/search.json",
                get(|Query(q): Query<HashMap<String, String>>| async move {
                    let docs = if q.get("q").map(String::as_str) == Some("key:/works/OL1W") || q.contains_key("author_key") {
                        serde_json::json!([{ "key": "/works/OL1W", "title": "The Hobbit", "author_name": ["J.R.R. Tolkien"],
                            "author_key": ["OL9A"], "first_publish_year": 1937, "cover_i": 42, "edition_count": 400 }])
                    } else {
                        serde_json::json!([
                            { "key": "/works/OL1W", "title": "The Hobbit", "author_name": ["J.R.R. Tolkien"], "author_key": ["OL9A"] },
                            { "key": "/books/OL5M", "title": "An edition" }])
                    };
                    Json(serde_json::json!({ "docs": docs }))
                }),
            )
            .route(
                "/works/OL1W.json",
                get(|| async {
                    Json(serde_json::json!({ "title": "The Hobbit",
                        "description": { "type": "/type/text", "value": " There and back. " }, "subjects": ["Dragons"] }))
                }),
            )
            .route(
                "/authors/OL9A.json",
                get(|| async {
                    Json(serde_json::json!({ "name": "J.R.R. Tolkien", "bio": "Philologist.", "photos": [-1, 77] }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn books_and_authors() {
        let ol = OpenLibrary::with_base_url(mock().await);
        let found = ol.search("hobbit").await.unwrap();
        assert_eq!(found.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["OL1W"], "works only");

        let book = ol.book("OL1W").await.unwrap();
        assert_eq!((book.summary.year, book.summary.cover_id), (Some(1937), Some(42)));
        assert_eq!(book.description.as_deref(), Some("There and back."));
        assert_eq!(book.subjects, ["Dragons"]);

        let author = ol.author("OL9A").await.unwrap();
        assert_eq!((author.name.as_str(), author.photo_id), ("J.R.R. Tolkien", Some(77)));
        assert_eq!(ol.author_books("OL9A").await.unwrap()[0].editions, Some(400));

        assert!(matches!(ol.book("../admin").await, Err(Error::NotFound)));
        assert!(!valid_id("OL9W", 'A') && !valid_id("OLW", 'W') && valid_id("OL27482W", 'W'));
        assert_eq!(cover_url(42, 'M'), "https://covers.openlibrary.org/b/id/42-M.jpg");
    }
}
