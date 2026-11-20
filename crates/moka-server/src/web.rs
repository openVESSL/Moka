//! Serves the built web app embedded in the binary. Unknown paths fall back to
//! `index.html` so client-side routes work on reload.

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(res) = file(path) {
        return res;
    }
    // asset-looking paths that do not exist are real 404s, not the SPA
    if path.starts_with("assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    file("index.html").unwrap_or_else(|| {
        (StatusCode::NOT_FOUND, "web app not built: run `pnpm --dir web build`, then rebuild the server")
            .into_response()
    })
}

fn file(path: &str) -> Option<Response> {
    let path = if path.is_empty() { "index.html" } else { path };
    let asset = Assets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // hashed build assets never change; index.html must always be revalidated
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    Some(
        ([(header::CONTENT_TYPE, mime.as_ref()), (header::CACHE_CONTROL, cache)], asset.data).into_response(),
    )
}
