//! Image proxy: serves TMDB artwork through the server and keeps a disk cache,
//! so browsers never talk to TMDB directly and repeat views are local.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};
use koda_metadata::{IMAGE_BASE_URL, IMAGE_SIZES, safe_image_name};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/images/{size}/{name}", get(image))
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
