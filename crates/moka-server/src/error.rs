use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Error returned by API handlers. Serialized as `{ "error": code, "message": text }`.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("not signed in")]
    Unauthorized,
    #[error("not allowed")]
    Forbidden,
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    NotConfigured(&'static str),
    #[error("{0}")]
    TooManyRequests(String),
    #[error("upstream service failed: {0}")]
    Upstream(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub error: &'static str,
    pub message: String,
}

impl ApiError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            ApiError::NotConfigured(_) => (StatusCode::CONFLICT, "not_configured"),
            ApiError::TooManyRequests(_) => (StatusCode::TOO_MANY_REQUESTS, "quota"),
            ApiError::Upstream(_) => (StatusCode::BAD_GATEWAY, "upstream"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        let message = match &self {
            ApiError::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                "internal error".to_owned()
            }
            other => other.to_string(),
        };
        (status, Json(ErrorBody { error: code, message })).into_response()
    }
}

impl From<moka_db::Error> for ApiError {
    fn from(e: moka_db::Error) -> Self {
        ApiError::Internal(e.into())
    }
}

impl From<moka_metadata::Error> for ApiError {
    fn from(e: moka_metadata::Error) -> Self {
        use moka_metadata::Error as E;
        match e {
            E::MissingKey => ApiError::NotConfigured("add a TMDB API key in Settings to browse the catalog"),
            E::NotFound => ApiError::NotFound("catalog entry"),
            other => ApiError::Upstream(other.to_string()),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
