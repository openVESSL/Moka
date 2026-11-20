//! The quality profile: which qualities are wanted, in what order, and when to stop upgrading.

use std::collections::HashMap;

use axum::extract::State;
use axum::{Json, Router, routing::get};
use moka_core::{Modifier, Permissions, Quality, Resolution, Source};
use moka_decision::{SizeLimit, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/quality-profile", get(get_profile).put(put_profile))
        .route("/qualities", get(qualities))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct QualityOption {
    pub key: String,
    pub label: String,
}

/// Every quality the parser can recognize, roughly worst to best.
pub fn known_qualities() -> Vec<Quality> {
    use Modifier as M;
    use Resolution as R;
    use Source as S;
    let mut out = vec![
        Quality::new(S::Cam, R::Unknown, M::None),
        Quality::new(S::Telesync, R::Unknown, M::None),
        Quality::new(S::Telecine, R::Unknown, M::None),
        Quality::new(S::Workprint, R::Unknown, M::None),
        Quality::new(S::Dvd, R::R480, M::Screener),
        Quality::new(S::Dvd, R::R480, M::Regional),
        Quality::new(S::Tv, R::R480, M::None),
        Quality::new(S::Dvd, R::R480, M::None),
        Quality::new(S::Dvd, R::R480, M::Remux),
    ];
    for r in [R::R480, R::R576, R::R720, R::R1080, R::R2160] {
        if r != R::R576 {
            out.push(Quality::new(S::Tv, r, M::None));
            out.push(Quality::new(S::Webrip, r, M::None));
            out.push(Quality::new(S::Webdl, r, M::None));
        }
        out.push(Quality::new(S::Bluray, r, M::None));
    }
    out.push(Quality::new(S::Bluray, R::R1080, M::Remux));
    out.push(Quality::new(S::Bluray, R::R2160, M::Remux));
    out.push(Quality::new(S::Bluray, R::R1080, M::Disc));
    out.push(Quality::new(S::Bluray, R::R2160, M::Disc));
    out.dedup();
    out
}

#[utoipa::path(get, path = "/api/v1/qualities", tag = "library", responses((status = 200, body = [QualityOption])))]
pub async fn qualities(_user: CurrentUser) -> Json<Vec<QualityOption>> {
    Json(
        known_qualities().into_iter().map(|q| QualityOption { key: q.key(), label: q.to_string() }).collect(),
    )
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QualityProfile {
    pub name: String,
    /// Wanted quality keys, best first.
    pub qualities: Vec<String>,
    /// Stop upgrading once a file of this quality (or better) exists.
    pub cutoff: String,
    pub upgrades_allowed: bool,
    pub min_seeders: i64,
    /// Per quality key, in MB per minute of runtime.
    #[schema(value_type = Object)]
    pub size_limits: HashMap<String, SizeLimit>,
    #[schema(value_type = Vec<Object>)]
    pub preferred_terms: Vec<Term>,
}

#[utoipa::path(get, path = "/api/v1/quality-profile", tag = "library", responses((status = 200, body = QualityProfile)))]
pub async fn get_profile(_user: CurrentUser, State(s): State<AppState>) -> ApiResult<Json<QualityProfile>> {
    let p = moka_db::library::profile(&s.db, None).await?.ok_or(ApiError::NotFound("quality profile"))?;
    Ok(Json(QualityProfile {
        name: p.name,
        qualities: serde_json::from_str(&p.qualities).unwrap_or_default(),
        cutoff: p.cutoff,
        upgrades_allowed: p.upgrades_allowed,
        min_seeders: p.min_seeders,
        size_limits: serde_json::from_str(&p.size_limits).unwrap_or_default(),
        preferred_terms: serde_json::from_str(&p.preferred_terms).unwrap_or_default(),
    }))
}

#[utoipa::path(put, path = "/api/v1/quality-profile", tag = "library", request_body = QualityProfile,
    responses((status = 200, body = QualityProfile)))]
pub async fn put_profile(
    user: CurrentUser,
    State(s): State<AppState>,
    Json(b): Json<QualityProfile>,
) -> ApiResult<Json<QualityProfile>> {
    user.require(Permissions::ADMIN)?;
    if b.qualities.is_empty() {
        return Err(ApiError::BadRequest("pick at least one quality".into()));
    }
    if let Some(bad) = b.qualities.iter().find(|k| Quality::from_key(k).is_none()) {
        return Err(ApiError::BadRequest(format!("unknown quality {bad}")));
    }
    if !b.qualities.contains(&b.cutoff) {
        return Err(ApiError::BadRequest("the upgrade stop must be one of the chosen qualities".into()));
    }
    let bad_size = b.size_limits.values().any(|l| {
        [l.min, l.max, l.preferred].iter().flatten().any(|v| !v.is_finite() || *v < 0.0)
            || matches!((l.min, l.max), (Some(a), Some(z)) if a > z)
    });
    if bad_size {
        return Err(ApiError::BadRequest("size limits must be positive and min below max".into()));
    }
    let current =
        moka_db::library::profile(&s.db, None).await?.ok_or(ApiError::NotFound("quality profile"))?;
    moka_db::library::update_profile(
        &s.db,
        &moka_db::library::QualityProfile {
            id: current.id,
            name: Some(b.name.trim()).filter(|n| !n.is_empty()).unwrap_or("Default").to_owned(),
            qualities: serde_json::to_string(&b.qualities).unwrap_or_default(),
            cutoff: b.cutoff.clone(),
            upgrades_allowed: b.upgrades_allowed,
            min_seeders: b.min_seeders.clamp(0, 10_000),
            size_limits: serde_json::to_string(&b.size_limits).unwrap_or_default(),
            preferred_terms: serde_json::to_string(&b.preferred_terms).unwrap_or_default(),
        },
    )
    .await?;
    get_profile(user, State(s)).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_qualities_round_trip_keys() {
        let all = super::known_qualities();
        assert!(all.len() > 20);
        for q in all {
            assert_eq!(moka_core::Quality::from_key(&q.key()), Some(q));
        }
    }
}
