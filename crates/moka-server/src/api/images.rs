//! Image proxy: serves TMDB artwork through the server and keeps a disk cache,
//! so browsers never talk to TMDB directly and repeat views are local.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};
use moka_metadata::{IMAGE_BASE_URL, IMAGE_SIZES, safe_image_name};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/images/{size}/{name}", get(image))
        .route("/images/cover/{mbid}", get(cover))
        .route("/images/book/{id}", get(book_cover))
        .route("/images/author/{id}", get(author_photo))
}

/// Book cover from Open Library, cached like other artwork.
pub async fn book_cover(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> ApiResult<Response> {
    open_library_image(&state, "b", id).await
}

/// Author photo from Open Library, cached like other artwork.
pub async fn author_photo(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> ApiResult<Response> {
    open_library_image(&state, "a", id).await
}

/// `kind` is `b` (book) or `a` (author); ids are numbers, so they are safe
/// as file names.
async fn open_library_image(state: &AppState, kind: &str, id: u64) -> ApiResult<Response> {
    let dir = state.config.cache_dir().join("images").join(format!("openlibrary-{kind}"));
    let path = dir.join(format!("{id}.jpg"));
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => {
            // default=false: a 404 instead of a blank placeholder
            let url = format!("{}/{kind}/id/{id}-L.jpg?default=false", moka_metadata::books::COVERS_URL);
            let res = state.http.get(&url).send().await.map_err(|e| ApiError::Upstream(e.to_string()))?;
            if res.status().as_u16() == 404 {
                return Err(ApiError::NotFound("image"));
            }
            if !res.status().is_success() {
                return Err(ApiError::Upstream(format!("cover host returned {}", res.status())));
            }
            let bytes = res.bytes().await.map_err(|e| ApiError::Upstream(e.to_string()))?.to_vec();
            tokio::fs::create_dir_all(&dir).await.map_err(|e| ApiError::Internal(e.into()))?;
            let tmp = dir.join(format!(".{id}.part"));
            if tokio::fs::write(&tmp, &bytes).await.is_ok() {
                let _ = tokio::fs::rename(&tmp, &path).await;
            }
            bytes
        }
    };
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg".to_owned()),
            (header::CACHE_CONTROL, "private, max-age=604800".to_owned()),
        ],
        bytes,
    )
        .into_response())
}

/// Album cover from the Cover Art Archive, cached like other artwork.
pub async fn cover(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(mbid): Path<String>,
) -> ApiResult<Response> {
    let valid = mbid.len() == 36 && mbid.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    if !valid {
        return Err(ApiError::NotFound("cover"));
    }
    let bytes = cover_bytes(&state, &mbid).await?.ok_or(ApiError::NotFound("cover"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg".to_owned()),
            (header::CACHE_CONTROL, "private, max-age=604800".to_owned()),
        ],
        bytes,
    )
        .into_response())
}

/// An album's front cover, from the cache or fetched and cached. `None` when
/// the Cover Art Archive has none.
pub(crate) async fn cover_bytes(state: &AppState, mbid: &str) -> ApiResult<Option<Vec<u8>>> {
    let dir = state.config.cache_dir().join("images").join("covers");
    let path = dir.join(format!("{mbid}.jpg"));
    if let Ok(b) = tokio::fs::read(&path).await {
        return Ok(Some(b));
    }
    let url = moka_metadata::music::cover_url(mbid, 500);
    let res = state.http.get(&url).send().await.map_err(|e| ApiError::Upstream(e.to_string()))?;
    if res.status().as_u16() == 404 {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(ApiError::Upstream(format!("cover host returned {}", res.status())));
    }
    let bytes = res.bytes().await.map_err(|e| ApiError::Upstream(e.to_string()))?.to_vec();
    tokio::fs::create_dir_all(&dir).await.map_err(|e| ApiError::Internal(e.into()))?;
    let tmp = dir.join(format!(".{mbid}.part"));
    if tokio::fs::write(&tmp, &bytes).await.is_ok() {
        let _ = tokio::fs::rename(&tmp, &path).await;
    }
    Ok(Some(bytes))
}

pub async fn image(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path((size, name)): Path<(String, String)>,
) -> ApiResult<Response> {
    if !IMAGE_SIZES.contains(&size.as_str()) {
        return Err(ApiError::NotFound("image size"));
    }
    let name = safe_image_name(&name).ok_or(ApiError::NotFound("image"))?;
    let dir = state.config.cache_dir().join("images").join(&size);
    let path = dir.join(name);

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => {
            let url = format!("{IMAGE_BASE_URL}/{size}/{name}");
            let res = state.http.get(&url).send().await.map_err(|e| ApiError::Upstream(e.to_string()))?;
            if res.status().as_u16() == 404 {
                return Err(ApiError::NotFound("image"));
            }
            if !res.status().is_success() {
                return Err(ApiError::Upstream(format!("image host returned {}", res.status())));
            }
            let bytes = res.bytes().await.map_err(|e| ApiError::Upstream(e.to_string()))?.to_vec();
            // write to a temp file then rename, so a crash never leaves half an image
            tokio::fs::create_dir_all(&dir).await.map_err(|e| ApiError::Internal(e.into()))?;
            let tmp = dir.join(format!(".{name}.part"));
            if tokio::fs::write(&tmp, &bytes).await.is_ok() {
                let _ = tokio::fs::rename(&tmp, &path).await;
            }
            bytes
        }
    };

    let mime = mime_guess::from_path(name).first_or_octet_stream();
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            // TMDB image paths are content-addressed: a changed image gets a new name
            (header::CACHE_CONTROL, "private, max-age=2592000, immutable".to_owned()),
        ],
        bytes,
    )
        .into_response())
}
