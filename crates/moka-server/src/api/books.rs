//! Book catalog from Open Library: search, a book's page and an author's
//! books, annotated with which ebooks and audiobooks are in the library.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::{Json, Router, routing::get};
use moka_metadata::books::{BookSummary, valid_id};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::catalog::LocalState;
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/catalog/books/search", get(search))
        .route("/catalog/books/book/{olid}", get(book))
        .route("/catalog/books/author/{olid}", get(author))
}

const DAY: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Serialize, ToSchema)]
pub struct Book {
    /// Open Library work id, `OL27482W`.
    pub olid: String,
    pub title: String,
    pub authors: Vec<String>,
    /// Open Library author ids, in the order of `authors`.
    pub author_olids: Vec<String>,
    pub year: Option<i64>,
    /// Cover for `/api/v1/images/book/{cover_id}`.
    pub cover_id: Option<i64>,
    /// The ebook, when it is in the library or requested.
    pub ebook: Option<LocalState>,
    /// The audiobook, when it is in the library or requested.
    pub audiobook: Option<LocalState>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BookPage {
    #[serde(flatten)]
    pub book: Book,
    pub description: Option<String>,
    pub subjects: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BookAuthor {
    pub olid: String,
    pub name: String,
    pub bio: Option<String>,
    pub birth_date: Option<String>,
    pub death_date: Option<String>,
    /// Photo for `/api/v1/images/author/{photo_id}`.
    pub photo_id: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BookAuthorPage {
    pub author: BookAuthor,
    pub books: Vec<Book>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    pub q: String,
}

/// Ebook and audiobook states of these books, by Open Library id.
async fn local_books(
    state: &AppState,
    books: &[BookSummary],
) -> ApiResult<HashMap<String, (Option<LocalState>, Option<LocalState>)>> {
    let ids: Vec<String> = books.iter().map(|b| b.id.clone()).collect();
    let mut out: HashMap<String, (Option<LocalState>, Option<LocalState>)> = HashMap::new();
    for (olid, kind, title_id, s) in moka_db::titles::book_states(&state.db, &ids).await? {
        let Ok(s) = s.parse() else { continue };
        let slot = out.entry(olid).or_default();
        let local = Some(LocalState { title_id, state: s });
        if kind == "audiobook" {
            slot.1 = local;
        } else {
            slot.0 = local;
        }
    }
    Ok(out)
}

async fn annotate(state: &AppState, books: Vec<BookSummary>) -> ApiResult<Vec<Book>> {
    let mut local = local_books(state, &books).await?;
    Ok(books
        .into_iter()
        .map(|b| {
            let (ebook, audiobook) = local.remove(&b.id).unwrap_or_default();
            Book {
                olid: b.id,
                title: b.title,
                authors: b.authors,
                author_olids: b.author_ids,
                year: b.year,
                cover_id: b.cover_id,
                ebook,
                audiobook,
            }
        })
        .collect())
}

#[utoipa::path(get, path = "/api/v1/catalog/books/search", operation_id = "search_books", tag = "catalog", params(SearchQuery),
    responses((status = 200, body = [Book])))]
pub async fn search(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<Book>>> {
    let query = q.q.trim();
    if query.is_empty() {
        return Ok(Json(vec![]));
    }
    if query.len() > 200 {
        return Err(ApiError::BadRequest("search text is too long".into()));
    }
    let ol = state.books();
    let found = state
        .cached(format!("books-search:{}", query.to_lowercase()), Duration::from_secs(600), || {
            ol.search(query)
        })
        .await?;
    Ok(Json(annotate(&state, found).await?))
}

pub(crate) async fn book_details(
    state: &AppState,
    olid: &str,
) -> ApiResult<moka_metadata::books::BookDetails> {
    if !valid_id(olid, 'W') {
        return Err(ApiError::NotFound("book"));
    }
    let ol = state.books();
    Ok(state.cached(format!("book:{olid}"), DAY, || ol.book(olid)).await?)
}

#[utoipa::path(get, path = "/api/v1/catalog/books/book/{olid}", operation_id = "book_page", tag = "catalog",
    params(("olid" = String, Path)), responses((status = 200, body = BookPage), (status = 404)))]
pub async fn book(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(olid): Path<String>,
) -> ApiResult<Json<BookPage>> {
    let d = book_details(&state, &olid).await?;
    let book = annotate(&state, vec![d.summary]).await?.remove(0);
    Ok(Json(BookPage { book, description: d.description, subjects: d.subjects }))
}

#[utoipa::path(get, path = "/api/v1/catalog/books/author/{olid}", operation_id = "book_author", tag = "catalog",
    params(("olid" = String, Path)), responses((status = 200, body = BookAuthorPage), (status = 404)))]
pub async fn author(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(olid): Path<String>,
) -> ApiResult<Json<BookAuthorPage>> {
    if !valid_id(&olid, 'A') {
        return Err(ApiError::NotFound("author"));
    }
    let ol = state.books();
    let a = state.cached(format!("book-author:{olid}"), DAY, || ol.author(&olid)).await?;
    let books = state.cached(format!("book-author-works:{olid}"), DAY, || ol.author_books(&olid)).await?;
    Ok(Json(BookAuthorPage {
        author: BookAuthor {
            olid: a.id,
            name: a.name,
            bio: a.bio,
            birth_date: a.birth_date,
            death_date: a.death_date,
            photo_id: a.photo_id,
        },
        books: annotate(&state, books).await?,
    }))
}

/// A book with its first author, ready to store.
pub(crate) async fn new_book(state: &AppState, olid: &str) -> ApiResult<moka_db::titles::NewBook> {
    let d = book_details(state, olid).await?;
    let b = d.summary;
    let author = b.authors.first().cloned().unwrap_or_else(|| "Unknown author".to_owned());
    Ok(moka_db::titles::NewBook {
        author_olid: b.author_ids.first().cloned().unwrap_or_else(|| format!("unknown:{author}")),
        author,
        olid: b.id,
        title: b.title,
        year: b.year,
        overview: d.description,
        poster_path: b.cover_id.map(|c| c.to_string()),
    })
}
