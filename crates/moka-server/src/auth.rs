//! Password hashing, session tokens and the `CurrentUser` extractor.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use moka_core::Permissions;
use moka_db::users::User;
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "moka_session";
pub const SESSION_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let hash: PasswordHash = Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow::anyhow!("hash password: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

/// Random opaque token (256 bits) for sessions and API keys.
pub fn new_token() -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("rng: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Tokens are stored hashed, so a leaked database does not leak live sessions.
pub fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Header with which an app holding `ACT_AS_USER` names the user it acts for.
pub const ACT_AS_HEADER: &str = "x-moka-user";

/// The user an app acts for: that user exactly, never with more rights than
/// they have, and never as an admin or as another acting app.
async fn act_as(state: &AppState, owner: &User, target: &axum::http::HeaderValue) -> Result<User, ApiError> {
    if !owner.permissions().has(Permissions::ACT_AS_USER) {
        return Err(ApiError::Forbidden);
    }
    let id: i64 = target.to_str().ok().and_then(|v| v.trim().parse().ok()).ok_or(ApiError::Forbidden)?;
    let mut user = moka_db::users::by_id(&state.db, id).await?.ok_or(ApiError::Forbidden)?;
    let stripped = user.permissions().0 & !(Permissions::ADMIN.0 | Permissions::ACT_AS_USER.0);
    user.permissions = moka_db::users::PermissionBits(Permissions(stripped));
    Ok(user)
}

/// The signed-in user. Resolved from `Authorization: Bearer <api key>`,
/// `X-Api-Key: <api key>` or the session cookie. Rejects with 401 when neither is valid.
pub struct CurrentUser(pub User);

impl CurrentUser {
    pub fn require(&self, permission: Permissions) -> Result<(), ApiError> {
        if self.0.permissions().has(permission) { Ok(()) } else { Err(ApiError::Forbidden) }
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(key) = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            // the header other media apps send
            .or_else(|| parts.headers.get("x-api-key").and_then(|v| v.to_str().ok()))
        {
            let owner = moka_db::users::by_api_key(&state.db, &token_hash(key.trim()))
                .await?
                .ok_or(ApiError::Unauthorized)?;
            return match parts.headers.get(ACT_AS_HEADER) {
                None => Ok(CurrentUser(owner)),
                Some(target) => act_as(state, &owner, target).await.map(CurrentUser),
            };
        }

        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(SESSION_COOKIE).ok_or(ApiError::Unauthorized)?;
        moka_db::users::by_session(&state.db, &token_hash(token.value()))
            .await?
            .map(CurrentUser)
            .ok_or(ApiError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let h = hash_password("correct horse").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn tokens_are_unique_and_hash_stably() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_eq!(token_hash(&a), token_hash(&a));
        assert_ne!(token_hash(&a), a);
    }
}
