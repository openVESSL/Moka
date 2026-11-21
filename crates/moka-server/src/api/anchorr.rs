//! The Discord bot connection: the admin turns it on with a bot token; the bot
//! (with its own key) reads who is linked to Discord and what arrived lately.

use axum::extract::{Query, State};
use axum::{Json, Router, routing::get};
use moka_core::Permissions;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::anchorr::{AnchorrSettings, BotStatus, TitleEvent};
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/anchorr", get(get_anchorr).put(put_anchorr))
        .route("/anchorr/connect", axum::routing::post(connect))
        .route("/anchorr/links", get(links))
        .route("/anchorr/available", get(available))
}

/// The connection as the UI sees it. The bot token and key never come back.
#[derive(Debug, Serialize, ToSchema)]
pub struct AnchorrView {
    pub enabled: bool,
    pub url: String,
    pub public_url: String,
    /// Anchorr connected itself from its own dashboard (no address to call back):
    /// it asks Moka for new titles instead of being told.
    pub from_anchorr: bool,
    /// What Anchorr reports, when it answers.
    pub bot: Option<BotStatus>,
    /// Why Anchorr could not be asked, in plain words.
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AnchorrUpdate {
    pub enabled: bool,
    /// Where Moka reaches Anchorr, e.g. `http://anchorr:8282`.
    pub url: String,
    /// Moka's address for links in Discord; empty for none.
    #[serde(default)]
    pub public_url: String,
    /// The Discord bot token. Needed the first time; omit to keep it.
    pub discord_token: Option<String>,
    /// The Discord server to work in, when the bot is in several.
    pub guild_id: Option<String>,
}

fn http_url(raw: &str, what: &str) -> ApiResult<String> {
    let url = raw.trim().trim_end_matches('/').to_owned();
    match reqwest::Url::parse(&url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => {
            moka_net::check_url(&u).map_err(|e| ApiError::BadRequest(e.to_string()))?;
            Ok(url)
        }
        _ => Err(ApiError::BadRequest(format!("{what} must start with http:// or https://"))),
    }
}

async fn view(state: &AppState, s: AnchorrSettings) -> AnchorrView {
    let from_anchorr = s.enabled && s.url.is_empty();
    let (bot, error) = if s.enabled && !from_anchorr {
        match crate::anchorr::status(state, &s).await {
            Ok(b) => (Some(b), None),
            Err(e) => (None, Some(e)),
        }
    } else {
        (None, None)
    };
    AnchorrView { enabled: s.enabled, url: s.url, public_url: s.public_url, from_anchorr, bot, error }
}

#[utoipa::path(get, path = "/api/v1/anchorr", tag = "settings", responses((status = 200, body = AnchorrView)))]
pub async fn get_anchorr(user: CurrentUser, State(state): State<AppState>) -> ApiResult<Json<AnchorrView>> {
    user.require(Permissions::ADMIN)?;
    let s = AnchorrSettings::load(&state.db).await?;
    Ok(Json(view(&state, s).await))
}

/// Turns the bot on (creating its account and key and handing them to Anchorr),
/// changes its token or server, or turns it off.
#[utoipa::path(put, path = "/api/v1/anchorr", tag = "settings", request_body = AnchorrUpdate,
    responses((status = 200, body = AnchorrView), (status = 400, description = "Anchorr refused or is unreachable")))]
pub async fn put_anchorr(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(b): Json<AnchorrUpdate>,
) -> ApiResult<Json<AnchorrView>> {
    user.require(Permissions::ADMIN)?;
    let current = AnchorrSettings::load(&state.db).await?;
    // turning it off needs no address (Anchorr may have connected itself)
    let url = if b.url.trim().is_empty() && !b.enabled {
        String::new()
    } else {
        http_url(&b.url, "Anchorr's address")?
    };
    let public_url = if b.public_url.trim().is_empty() {
        String::new()
    } else {
        http_url(&b.public_url, "The public address")?
    };

    if !b.enabled {
        if current.enabled
            && let Err(e) = crate::anchorr::unpair(&state, &current).await
        {
            tracing::warn!(error = %e, "Anchorr did not confirm it stopped");
        }
        // turning it off changes nothing else
        let public_url = if public_url.is_empty() { current.public_url.clone() } else { public_url };
        let next = AnchorrSettings { enabled: false, url, public_url, ..current };
        next.save(&state.db).await?;
        return Ok(Json(view(&state, next).await));
    }

    let token = b.discord_token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if !current.enabled && token.is_none() {
        return Err(ApiError::BadRequest("enter the Discord bot token".into()));
    }
    let guild = b.guild_id.as_deref().map(str::trim).filter(|g| !g.is_empty());
    let current_enabled = current.enabled;
    let next = AnchorrSettings { enabled: true, url, public_url, ..current };
    let next = crate::anchorr::ensure_accounts(&state, next).await?;
    // kept even if Anchorr refuses, so a retry reuses the accounts and key instead of adding keys
    AnchorrSettings { enabled: current_enabled, ..next.clone() }.save(&state.db).await?;
    let bot = crate::anchorr::pair(&state, &next, token, guild).await.map_err(ApiError::BadRequest)?;
    next.save(&state.db).await?;
    Ok(Json(AnchorrView {
        from_anchorr: false,
        enabled: true,
        url: next.url,
        public_url: next.public_url,
        bot: Some(bot),
        error: None,
    }))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct AnchorrConnect {
    /// Moka's address for links in Discord; empty keeps the saved one.
    #[serde(default)]
    pub public_url: String,
}

/// What Anchorr keeps after connecting itself. The key is shown only here.
#[derive(Debug, Serialize, ToSchema)]
pub struct AnchorrKeys {
    pub api_key: String,
    pub guest_user_id: i64,
    pub public_url: String,
}

/// Anchorr connecting itself from its own dashboard, signed in as a Moka admin:
/// creates (or reuses) the bot's account, guest account and key and returns them.
#[utoipa::path(post, path = "/api/v1/anchorr/connect", tag = "settings", request_body = AnchorrConnect,
    responses((status = 200, body = AnchorrKeys)))]
pub async fn connect(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(b): Json<AnchorrConnect>,
) -> ApiResult<Json<AnchorrKeys>> {
    user.require(Permissions::ADMIN)?;
    let current = AnchorrSettings::load(&state.db).await?;
    let public_url = if b.public_url.trim().is_empty() {
        current.public_url.clone()
    } else {
        http_url(&b.public_url, "The public address")?
    };
    let next = crate::anchorr::ensure_accounts(
        &state,
        AnchorrSettings { enabled: true, url: String::new(), public_url, ..current },
    )
    .await?;
    next.save(&state.db).await?;
    Ok(Json(AnchorrKeys {
        api_key: next.api_key,
        guest_user_id: next.guest_user_id,
        public_url: next.public_url,
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DiscordLinkView {
    pub user_id: i64,
    pub username: String,
    pub discord_id: String,
}

/// Users with a linked Discord account, for the bot to request in their name.
#[utoipa::path(get, path = "/api/v1/anchorr/links", tag = "settings", responses((status = 200, body = [DiscordLinkView])))]
pub async fn links(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<DiscordLinkView>>> {
    user.require(Permissions::ACT_AS_USER)?;
    let users = moka_db::users::list(&state.db).await?;
    Ok(Json(
        users
            .into_iter()
            .filter_map(|u| {
                u.discord_id.map(|d| DiscordLinkView { user_id: u.id, username: u.username, discord_id: d })
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SinceQuery {
    /// Unix time in seconds.
    pub since: i64,
}

/// Films and episodes that arrived since a moment, for the bot to catch up
/// after being offline and to sum up the week.
#[utoipa::path(get, path = "/api/v1/anchorr/available", tag = "settings", params(SinceQuery),
    responses((status = 200, body = [TitleEvent])))]
pub async fn available(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(q): Query<SinceQuery>,
) -> ApiResult<Json<Vec<TitleEvent>>> {
    user.require(Permissions::ACT_AS_USER)?;
    Ok(Json(crate::anchorr::available_since(&state, q.since).await?))
}
