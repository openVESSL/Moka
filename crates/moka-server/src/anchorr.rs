//! The Discord bot (Anchorr) as a built-in connection. Moka gives the bot its
//! own account and API key, hands them to Anchorr with the bot token, and tells
//! Anchorr when a film or episodes arrive. Everything Discord-specific (channels,
//! roles, embeds) stays in Anchorr's own dashboard.

use std::time::Duration;

use moka_core::Permissions;
use moka_db::titles::Title;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::state::AppState;

/// The bot's own account: it acts for Discord members and decides requests
/// from Anchorr's dashboard, nothing else. Nobody signs in with it.
pub const SERVICE_USERNAME: &str = "anchorr-bot";
const SERVICE_PERMISSIONS: Permissions =
    Permissions(Permissions::ACT_AS_USER.0 | Permissions::MANAGE_REQUESTS.0);
/// Discord members without a linked account request as this user.
pub const GUEST_USERNAME: &str = "Discord (Anchorr)";

/// Stored under the `integrations.anchorr` setting. The API key is sealed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnchorrSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Where Moka reaches Anchorr, e.g. `http://anchorr:8282`.
    #[serde(default)]
    pub url: String,
    /// Moka's address for links in Discord (empty: no links).
    #[serde(default)]
    pub public_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_key_id: i64,
    #[serde(default)]
    pub service_user_id: i64,
    #[serde(default)]
    pub guest_user_id: i64,
}

impl AnchorrSettings {
    pub const KEY: &'static str = "integrations.anchorr";

    pub async fn load(db: &moka_db::Pool) -> Result<Self, moka_db::Error> {
        let mut s: Self = moka_db::settings::get(db, Self::KEY).await?.unwrap_or_default();
        s.api_key = moka_db::secret::open(&s.api_key);
        Ok(s)
    }

    pub async fn save(&self, db: &moka_db::Pool) -> Result<(), moka_db::Error> {
        let sealed = Self { api_key: moka_db::secret::seal(&self.api_key), ..self.clone() };
        moka_db::settings::set(db, Self::KEY, &sealed).await
    }

    fn link(&self, t: &Title) -> Option<String> {
        let base = self.public_url.trim_end_matches('/');
        let path = match t.kind.as_str() {
            "movie" => "movie",
            "show" => "show",
            _ => return None,
        };
        (!base.is_empty()).then(|| t.tmdb_id.map(|id| format!("{base}/{path}/{id}"))).flatten()
    }
}

/// Creates the bot's account, the guest account and the bot's API key when
/// missing, and returns the settings that point at them.
pub async fn ensure_accounts(state: &AppState, mut s: AnchorrSettings) -> anyhow::Result<AnchorrSettings> {
    let db = &state.db;
    let service = account(db, s.service_user_id, SERVICE_USERNAME, SERVICE_PERMISSIONS).await?;
    // keep its rights to exactly these, whatever was changed by hand
    moka_db::users::set_permissions(db, service.id, SERVICE_PERMISSIONS).await?;
    let guest = account(db, s.guest_user_id, GUEST_USERNAME, Permissions::REQUEST).await?;

    let key_works = !s.api_key.is_empty()
        && moka_db::users::by_api_key(db, &crate::auth::token_hash(&s.api_key))
            .await?
            .is_some_and(|u| u.id == service.id);
    if !key_works {
        if s.api_key_id > 0 {
            moka_db::users::delete_api_key(db, s.api_key_id).await?;
        }
        let key = crate::auth::new_token()?;
        let row =
            moka_db::users::create_api_key(db, service.id, "Anchorr", &crate::auth::token_hash(&key)).await?;
        s.api_key = key;
        s.api_key_id = row.id;
    }
    s.service_user_id = service.id;
    s.guest_user_id = guest.id;
    Ok(s)
}

/// The account with this id, else the one with this name, else a new one with
/// a random password nobody knows.
async fn account(
    db: &moka_db::Pool,
    id: i64,
    name: &str,
    permissions: Permissions,
) -> anyhow::Result<moka_db::users::User> {
    if id > 0
        && let Some(u) = moka_db::users::by_id(db, id).await?
    {
        return Ok(u);
    }
    if let Some(u) = moka_db::users::by_username(db, name).await? {
        return Ok(u);
    }
    let password = crate::auth::hash_password(&crate::auth::new_token()?)?;
    Ok(moka_db::users::create(db, name, &password, permissions).await?)
}

// ----- talking to Anchorr -----

/// What Anchorr reports about the bot.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct BotStatus {
    #[serde(default)]
    pub bot_online: bool,
    #[serde(default)]
    pub bot_name: Option<String>,
    /// Link that adds the bot to a Discord server with the rights it needs.
    #[serde(default)]
    pub invite_url: Option<String>,
    /// Servers the bot is in.
    #[serde(default)]
    pub guilds: Vec<Guild>,
    /// The server the bot works in.
    #[serde(default)]
    pub guild_id: Option<String>,
    /// Whether new titles have a channel to be announced in.
    #[serde(default)]
    pub announce_channel_set: bool,
    /// Anchorr's own dashboard, for channels, roles and messages.
    #[serde(default)]
    pub dashboard_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Guild {
    pub id: String,
    pub name: String,
}

/// One call to Anchorr with the bot's key. Errors are short, readable sentences.
async fn call(
    state: &AppState,
    s: &AnchorrSettings,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let url = format!("{}{path}", s.url.trim_end_matches('/'));
    moka_net::check_str(&url).map_err(|e| e.to_string())?;
    let mut req = state.http.request(method, &url).bearer_auth(&s.api_key).timeout(Duration::from_secs(20));
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await.map_err(|e| format!("Anchorr could not be reached at {}: {e}", s.url))?;
    let status = res.status();
    let value: serde_json::Value = res.json().await.unwrap_or_default();
    if status.is_success() {
        return Ok(value);
    }
    let said = value.get("message").or_else(|| value.get("error")).and_then(|m| m.as_str());
    Err(said.map(str::to_owned).unwrap_or_else(|| format!("Anchorr answered HTTP {status}")))
}

/// Hands Anchorr the key, the guest account and the bot token. Anchorr checks
/// the key against Moka before it accepts anything.
pub async fn pair(
    state: &AppState,
    s: &AnchorrSettings,
    discord_token: Option<&str>,
    guild_id: Option<&str>,
) -> Result<BotStatus, String> {
    let body = serde_json::json!({
        "api_key": s.api_key,
        "guest_user_id": s.guest_user_id,
        "public_url": s.public_url,
        "discord_token": discord_token,
        "guild_id": guild_id,
    });
    let v = call(state, s, reqwest::Method::POST, "/api/moka/pair", Some(body)).await?;
    serde_json::from_value(v).map_err(|e| format!("unexpected answer from Anchorr: {e}"))
}

pub async fn status(state: &AppState, s: &AnchorrSettings) -> Result<BotStatus, String> {
    let v = call(state, s, reqwest::Method::GET, "/api/moka/status", None).await?;
    serde_json::from_value(v).map_err(|e| format!("unexpected answer from Anchorr: {e}"))
}

pub async fn unpair(state: &AppState, s: &AnchorrSettings) -> Result<(), String> {
    call(state, s, reqwest::Method::DELETE, "/api/moka/pair", None).await.map(|_| ())
}

// ----- new titles -----

/// What Anchorr receives when titles arrive, and what it reads back to catch up.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TitleEvent {
    /// Always `available`.
    pub event: String,
    pub occurred_at: i64,
    pub title: EventTitle,
    /// Users with an open request for the title, to tell them it is here.
    pub requester_user_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EventTitle {
    pub id: i64,
    /// `movie` or `show`.
    pub kind: String,
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i64>,
    /// TMDB image paths (`/abc.jpg`).
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    /// The title's page in Moka, when a public address is set.
    pub link: Option<String>,
    /// For a show: the episodes that arrived.
    pub episodes: Vec<EventEpisode>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EventEpisode {
    pub id: i64,
    pub season: i64,
    pub episode: i64,
    pub name: String,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i64>,
}

fn episode(e: &Title) -> EventEpisode {
    EventEpisode {
        id: e.id,
        season: e.season_number.unwrap_or(0),
        episode: e.episode_number.unwrap_or(0),
        name: e.title.clone(),
        overview: e.overview.clone(),
        runtime_minutes: e.runtime_minutes,
    }
}

async fn event_for(
    state: &AppState,
    s: &AnchorrSettings,
    t: &Title,
    episodes: &[Title],
    at: i64,
) -> anyhow::Result<TitleEvent> {
    Ok(TitleEvent {
        event: "available".into(),
        occurred_at: at,
        title: EventTitle {
            id: t.id,
            kind: t.kind.clone(),
            tmdb_id: t.tmdb_id,
            imdb_id: t.imdb_id.clone(),
            title: t.title.clone(),
            year: t.year,
            overview: t.overview.clone(),
            runtime_minutes: t.runtime_minutes,
            poster_path: t.poster_path.clone(),
            backdrop_path: t.backdrop_path.clone(),
            link: s.link(t),
            episodes: episodes.iter().map(episode).collect(),
        },
        requester_user_ids: moka_db::requests::requester_ids(&state.db, t.id).await?,
    })
}

/// The show an episode belongs to (episode → season → show).
async fn show_of(db: &moka_db::Pool, e: &Title) -> anyhow::Result<Option<Title>> {
    let Some(season) = e.parent_id else { return Ok(None) };
    let Some(season) = moka_db::titles::by_id(db, season).await? else { return Ok(None) };
    let Some(show) = season.parent_id else { return Ok(None) };
    Ok(moka_db::titles::by_id(db, show).await?)
}

/// Tells Anchorr a film or a show's episodes arrived. Never fails the caller.
pub async fn forward(state: &AppState, info: &crate::notify::TitleInfo) {
    let s = match AnchorrSettings::load(&state.db).await {
        Ok(s) if s.enabled && !s.url.is_empty() => s,
        _ => return,
    };
    let event = async {
        let Some(t) = moka_db::titles::by_id(&state.db, info.id).await? else { return Ok(None) };
        if !matches!(t.kind.as_str(), "movie" | "show") {
            return Ok(None);
        }
        let mut episodes = Vec::new();
        for (id, _) in &info.episode_files {
            if let Some(e) = moka_db::titles::by_id(&state.db, *id).await? {
                episodes.push(e);
            }
        }
        anyhow::Ok(Some(event_for(state, &s, &t, &episodes, crate::notify::unix_now()).await?))
    };
    let event = match event.await {
        Ok(Some(e)) => e,
        Ok(None) => return,
        Err(e) => return tracing::warn!(error = %e, "could not build the Anchorr event"),
    };
    let body = serde_json::to_value(&event).unwrap_or_default();
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        match call(state, &s, reqwest::Method::POST, "/api/moka/events", Some(body.clone())).await {
            Ok(_) => return,
            Err(e) => tracing::warn!(error = %e, attempt, "could not tell Anchorr about a new title"),
        }
    }
}

/// Films and shows (with their new episodes) that arrived since a moment, as
/// the events Anchorr would have received.
pub async fn available_since(state: &AppState, since: i64) -> anyhow::Result<Vec<TitleEvent>> {
    let s = AnchorrSettings::load(&state.db).await?;
    let mut events: Vec<TitleEvent> = Vec::new();
    let mut shows: Vec<(Title, Vec<Title>)> = Vec::new();
    for t in moka_db::titles::available_since(&state.db, since).await? {
        if t.kind == "movie" {
            let at = t.updated_at;
            events.push(event_for(state, &s, &t, &[], at).await?);
        } else if let Some(show) = show_of(&state.db, &t).await? {
            match shows.iter_mut().find(|(sh, _)| sh.id == show.id) {
                Some((_, eps)) => eps.push(t),
                None => shows.push((show, vec![t])),
            }
        }
    }
    for (show, eps) in shows {
        let at = eps.iter().map(|e| e.updated_at).max().unwrap_or(show.updated_at);
        events.push(event_for(state, &s, &show, &eps, at).await?);
    }
    events.sort_by_key(|e| e.occurred_at);
    Ok(events)
}
