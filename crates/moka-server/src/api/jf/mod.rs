//! The API phone and TV apps made for Jellyfin already speak, mounted at
//! `/jellyfin`, so those apps can browse and play a Moka library. Add the
//! server in the app as `http://host:8484/jellyfin`.
//!
//! It is an adapter over the same data as `/api/v1`: ids become 32-digit hex
//! GUIDs, seconds become ticks (1 s = 10,000,000), names are PascalCase.
//! Paths are matched without regard to case, as the apps expect.

#[cfg(test)]
mod flow;
mod items;
mod play;

use std::collections::HashMap;

use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use moka_core::Permissions;
use serde_json::{Value, json};

use crate::auth::{new_token, token_hash};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The API version the apps are told; they refuse servers that are too old.
pub const VERSION: &str = "10.11.0";
/// App tokens last long: apps sign in once and keep the token.
const TOKEN_TTL_SECONDS: i64 = 365 * 24 * 3600;
pub const TICKS_PER_SECOND: f64 = 10_000_000.0;

/// Kinds of ids, in their first 8 hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Title = 1,
    View = 2,
    User = 3,
    File = 4,
}

pub fn guid(kind: IdKind, id: i64) -> String {
    format!("{:08x}{:024x}", kind as u32, id)
}

/// Reads a GUID in either form (with or without dashes).
pub fn parse_guid(s: &str) -> Option<(IdKind, i64)> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let kind = match u32::from_str_radix(&hex[..8], 16).ok()? {
        1 => IdKind::Title,
        2 => IdKind::View,
        3 => IdKind::User,
        4 => IdKind::File,
        _ => return None,
    };
    let id = i64::from_str_radix(&hex[8..], 16).ok()?;
    Some((kind, id))
}

pub fn ticks(seconds: f64) -> i64 {
    (seconds * TICKS_PER_SECOND) as i64
}

pub fn seconds(ticks: i64) -> f64 {
    ticks as f64 / TICKS_PER_SECOND
}

/// The router, as a service that lowercases paths first: the apps use
/// PascalCase paths, and some send other casings.
pub fn service(
    state: AppState,
) -> impl tower::Service<
    Request<axum::body::Body>,
    Response = Response,
    Error = std::convert::Infallible,
    Future = impl Send,
> + Clone
+ Send
+ 'static {
    let router = routes().with_state(state);
    tower::ServiceBuilder::new().map_request(lowercase_path).service(router)
}

fn lowercase_path(mut req: Request<axum::body::Body>) -> Request<axum::body::Body> {
    let uri = req.uri();
    let lower = uri.path().to_ascii_lowercase();
    if lower != uri.path() {
        let rebuilt = match uri.query() {
            Some(q) => format!("{lower}?{q}"),
            None => lower,
        };
        if let Ok(u) = rebuilt.parse() {
            *req.uri_mut() = u;
        }
    }
    req
}

fn routes() -> Router<AppState> {
    Router::new()
        .route("/system/info/public", get(public_info))
        .route("/system/info", get(info))
        .route("/system/ping", get(ping).post(ping))
        .route("/system/endpoint", get(endpoint))
        .route("/branding/configuration", get(branding))
        .route("/branding/css", get(|| async { "" }))
        .route("/branding/css.css", get(|| async { "" }))
        .route("/quickconnect/enabled", get(|| async { Json(false) }))
        .route("/users/public", get(|| async { Json(json!([])) }))
        .route("/users/authenticatebyname", post(authenticate))
        .route("/users/me", get(me))
        .route("/users/{id}", get(user))
        .route("/sessions/logout", post(logout))
        .route("/sessions/capabilities", post(no_content))
        .route("/sessions/capabilities/full", post(no_content))
        .route("/displaypreferences/{id}", get(display_preferences).post(no_content))
        .route("/plugins", get(|| async { Json(json!([])) }))
        .route("/localization/cultures", get(|| async { Json(json!([])) }))
        .route("/localization/countries", get(|| async { Json(json!([])) }))
        .route("/socket", get(|| async { StatusCode::NOT_FOUND }))
        .merge(items::routes())
        .merge(play::routes())
        .fallback(|| async { StatusCode::NOT_FOUND })
}

/// Query parameters with lowercased names: apps send `ParentId`,
/// `parentId` or `parentid`.
#[derive(Debug, Default)]
pub struct Q(HashMap<String, String>);

impl Q {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str).filter(|v| !v.is_empty())
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.get(name)?.parse().ok()
    }

    pub fn flag(&self, name: &str) -> Option<bool> {
        match self.get(name)?.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        }
    }

    /// A comma-separated list, lowercased.
    pub fn list(&self, name: &str) -> Vec<String> {
        self.get(name)
            .map(|v| v.split(',').map(|x| x.trim().to_ascii_lowercase()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Q {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let pairs = parts
            .uri
            .query()
            .map(|q| {
                form_urlencoded::parse(q.as_bytes())
                    .map(|(k, v)| (k.to_ascii_lowercase(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Q(pairs))
    }
}

/// The app's token: `Authorization: MediaBrowser … Token="…"` (also as
/// `X-Emby-Authorization`), `X-Emby-Token`, `X-MediaBrowser-Token`, or
/// `api_key` / `ApiKey` in the query.
fn token(parts: &Parts) -> Option<String> {
    let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok());
    for name in ["authorization", "x-emby-authorization"] {
        if let Some(t) = header(name).and_then(|v| auth_field(v, "token")) {
            return Some(t);
        }
    }
    if let Some(t) = header("x-emby-token").or_else(|| header("x-mediabrowser-token")) {
        return Some(t.to_owned());
    }
    form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k.eq_ignore_ascii_case("api_key") || k.eq_ignore_ascii_case("apikey"))
        .map(|(_, v)| v.into_owned())
}

/// One field of a `MediaBrowser Client="…", Token="…"` header.
fn auth_field(value: &str, name: &str) -> Option<String> {
    let rest = value.trim().strip_prefix("MediaBrowser").or_else(|| value.trim().strip_prefix("Emby"))?;
    rest.split(',')
        .find_map(|part| {
            let (k, v) = part.trim().split_once('=')?;
            k.trim().eq_ignore_ascii_case(name).then(|| {
                let v = v.trim().trim_matches('"');
                form_urlencoded::parse(format!("x={v}").as_bytes())
                    .next()
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_default()
            })
        })
        .filter(|v| !v.is_empty())
}

/// A user signed in from an app (a session token or a Moka API key).
pub struct AppUser {
    pub user: moka_db::users::User,
    pub token: String,
}

impl FromRequestParts<AppState> for AppUser {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let unauthorized = || StatusCode::UNAUTHORIZED.into_response();
        let token = token(parts).ok_or_else(unauthorized)?;
        let hash = token_hash(&token);
        let found = match moka_db::users::by_session(&state.db, &hash).await {
            Ok(Some(u)) => Some(u),
            Ok(None) => moka_db::users::by_api_key(&state.db, &hash).await.ok().flatten(),
            Err(_) => None,
        };
        found.map(|user| AppUser { user, token }).ok_or_else(unauthorized)
    }
}

impl AppUser {
    pub fn can_play(&self) -> Result<(), StatusCode> {
        if self.user.permissions().has(Permissions::PLAY) { Ok(()) } else { Err(StatusCode::FORBIDDEN) }
    }
}

/// The id apps use to tell servers apart, made once and kept.
pub async fn server_id(state: &AppState) -> ApiResult<String> {
    const KEY: &str = "app_api.server_id";
    if let Some(id) = moka_db::settings::get::<String>(&state.db, KEY).await? {
        return Ok(id);
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| ApiError::Internal(anyhow::anyhow!("rng: {e}")))?;
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    moka_db::settings::set(&state.db, KEY, &id).await?;
    Ok(id)
}

fn iso(unix: i64) -> String {
    let t = time::OffsetDateTime::from_unix_timestamp(unix).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    t.format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
}

async fn no_content() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn ping() -> &'static str {
    "Moka"
}

async fn public_info(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    Ok(Json(json!({
        "LocalAddress": format!("http://{host}/jellyfin"),
        "ServerName": "Moka",
        "Version": VERSION,
        "ProductName": "Jellyfin Server",
        "OperatingSystem": "",
        "Id": server_id(&state).await?,
        "StartupWizardCompleted": true,
    })))
}

async fn info(_user: AppUser, state: State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let Json(mut v) = public_info(state, headers).await?;
    let extra = json!({
        "OperatingSystemDisplayName": std::env::consts::OS,
        "HasPendingRestart": false,
        "IsShuttingDown": false,
        "SupportsLibraryMonitor": false,
        "WebSocketPortNumber": 0,
        "CompletedInstallations": [],
        "CanSelfRestart": false,
        "CanLaunchWebBrowser": false,
        "HasUpdateAvailable": false,
        "TranscodingTempPath": "",
        "CachePath": "",
        "LogPath": "",
        "InternalMetadataPath": "",
        "ProgramDataPath": "",
        "ItemsByNamePath": "",
        "WebPath": "",
        "CastReceiverApplications": [],
    });
    if let (Some(a), Some(b)) = (v.as_object_mut(), extra.as_object()) {
        a.extend(b.clone());
    }
    Ok(Json(v))
}

async fn endpoint() -> Json<Value> {
    Json(json!({ "IsLocal": true, "IsInNetwork": true }))
}

async fn branding() -> Json<Value> {
    Json(json!({ "LoginDisclaimer": "", "CustomCss": "", "SplashscreenEnabled": false }))
}

async fn display_preferences(Path(id): Path<String>) -> Json<Value> {
    Json(json!({
        "Id": id,
        "SortBy": "SortName",
        "SortOrder": "Ascending",
        "RememberIndexing": false,
        "PrimaryImageHeight": 250,
        "PrimaryImageWidth": 250,
        "CustomPrefs": {},
        "ScrollDirection": "Horizontal",
        "ShowBackdrop": true,
        "RememberSorting": false,
        "ShowSidebar": false,
        "Client": "emby",
    }))
}

/// A user as apps expect it, with a configuration and a policy.
pub async fn user_dto(state: &AppState, u: &moka_db::users::User) -> ApiResult<Value> {
    let admin = u.permissions().has(Permissions::ADMIN);
    let play = u.permissions().has(Permissions::PLAY);
    let configuration = json!({
        "AudioLanguagePreference": "",
        "PlayDefaultAudioTrack": true,
        "SubtitleLanguagePreference": "",
        "DisplayMissingEpisodes": false,
        "GroupedFolders": [],
        "SubtitleMode": "Default",
        "DisplayCollectionsView": false,
        "EnableLocalPassword": false,
        "OrderedViews": [],
        "LatestItemsExcludes": [],
        "MyMediaExcludes": [],
        "HidePlayedInLatest": true,
        "RememberAudioSelections": true,
        "RememberSubtitleSelections": true,
        "EnableNextEpisodeAutoPlay": true,
        "CastReceiverId": "",
    });
    let mut policy = json!({
        "IsAdministrator": admin,
        "IsHidden": false,
        "IsDisabled": false,
        "EnableCollectionManagement": false,
        "EnableSubtitleManagement": false,
        "EnableLyricManagement": false,
        "EnableUserPreferenceAccess": true,
        "EnableRemoteControlOfOtherUsers": false,
        "EnableSharedDeviceControl": false,
        "EnableRemoteAccess": true,
        "EnableLiveTvManagement": false,
        "EnableLiveTvAccess": false,
        "EnableMediaPlayback": play,
        "EnableAudioPlaybackTranscoding": play,
        "EnableVideoPlaybackTranscoding": play,
        "EnablePlaybackRemuxing": play,
        "ForceRemoteSourceTranscoding": false,
        "EnableContentDeletion": false,
        "EnableContentDownloading": play,
        "EnableSyncTranscoding": false,
        "EnableMediaConversion": false,
        "EnableAllDevices": true,
        "EnableAllChannels": false,
        "EnableAllFolders": true,
        "EnablePublicSharing": false,
    });
    let more = json!({
        "BlockedTags": [],
        "AllowedTags": [],
        "AccessSchedules": [],
        "BlockUnratedItems": [],
        "EnableContentDeletionFromFolders": [],
        "EnabledDevices": [],
        "EnabledChannels": [],
        "EnabledFolders": [],
        "BlockedMediaFolders": [],
        "BlockedChannels": [],
        "InvalidLoginAttemptCount": 0,
        "LoginAttemptsBeforeLockout": -1,
        "MaxActiveSessions": 0,
        "RemoteClientBitrateLimit": 0,
        "AuthenticationProviderId": "Moka",
        "PasswordResetProviderId": "Moka",
        "SyncPlayAccess": "None",
    });
    if let (Some(p), Some(m)) = (policy.as_object_mut(), more.as_object()) {
        p.extend(m.clone());
    }
    Ok(json!({
        "Name": u.username,
        "ServerId": server_id(state).await?,
        "Id": guid(IdKind::User, u.id),
        "HasPassword": true,
        "HasConfiguredPassword": true,
        "HasConfiguredEasyPassword": false,
        "EnableAutoLogin": false,
        "Configuration": configuration,
        "Policy": policy,
    }))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Credentials {
    username: String,
    #[serde(default)]
    pw: String,
}

async fn authenticate(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // some apps send the body with another casing of the field names
    let parsed: Option<Credentials> = serde_json::from_slice::<Value>(&body).ok().and_then(|v| {
        let lower: serde_json::Map<String, Value> = v
            .as_object()?
            .iter()
            .map(|(k, v)| {
                let mut c = k.chars();
                let k = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
                (k, v.clone())
            })
            .collect();
        serde_json::from_value(Value::Object(lower)).ok()
    });
    let Some(c) = parsed else { return StatusCode::BAD_REQUEST.into_response() };
    let result = async {
        let user = crate::api::auth::check_credentials(&state, &c.username, &c.pw).await?;
        let token = new_token()?;
        let device = headers
            .get("authorization")
            .or_else(|| headers.get("x-emby-authorization"))
            .and_then(|v| v.to_str().ok())
            .and_then(|v| auth_field(v, "client"));
        moka_db::users::create_session(&state.db, &token_hash(&token), user.id, TOKEN_TTL_SECONDS, device.as_deref())
            .await?;
        let user_json = user_dto(&state, &user).await?;
        let sid = server_id(&state).await?;
        let field = |name: &str| {
            headers
                .get("authorization")
                .or_else(|| headers.get("x-emby-authorization"))
                .and_then(|v| v.to_str().ok())
                .and_then(|v| auth_field(v, name))
                .unwrap_or_default()
        };
        ApiResult::Ok(json!({
            "User": user_json,
            "SessionInfo": {
                "Id": token_hash(&token).chars().take(32).collect::<String>(),
                "UserId": guid(IdKind::User, user.id),
                "UserName": user.username,
                "Client": field("client"),
                "DeviceId": field("deviceid"),
                "DeviceName": field("device"),
                "ApplicationVersion": field("version"),
                "ServerId": sid,
                "IsActive": true,
                "SupportsMediaControl": false,
                "SupportsRemoteControl": false,
                "HasCustomDeviceName": false,
                "PlayableMediaTypes": [],
                "SupportedCommands": [],
                "AdditionalUsers": [],
                "NowPlayingQueue": [],
                "NowPlayingQueueFullItems": [],
                "LastActivityDate": iso(crate::pipeline::now()),
                "LastPlaybackCheckIn": iso(0),
                "PlayState": { "CanSeek": false, "IsPaused": false, "IsMuted": false, "RepeatMode": "RepeatNone", "PlaybackOrder": "Default" },
                "Capabilities": { "PlayableMediaTypes": [], "SupportedCommands": [], "SupportsMediaControl": false, "SupportsPersistentIdentifier": true },
            },
            "AccessToken": token,
            "ServerId": sid,
        }))
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(ApiError::TooManyRequests(_)) => StatusCode::TOO_MANY_REQUESTS.into_response(),
        Err(ApiError::Unauthorized) => StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn me(u: AppUser, State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(user_dto(&state, &u.user).await?))
}

async fn user(u: AppUser, State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match parse_guid(&id) {
        Some((IdKind::User, n)) if n == u.user.id => match user_dto(&state, &u.user).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => e.into_response(),
        },
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn logout(u: AppUser, State(state): State<AppState>) -> StatusCode {
    let _ = moka_db::users::delete_session(&state.db, &token_hash(&u.token)).await;
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let g = guid(IdKind::Title, 42);
        assert_eq!(g, "0000000100000000000000000000002a");
        assert_eq!(parse_guid(&g), Some((IdKind::Title, 42)));
        assert_eq!(parse_guid("00000003-0000-0000-0000-000000000007"), Some((IdKind::User, 7)));
        assert_eq!(parse_guid("zz"), None);
        assert_eq!(parse_guid("00000009000000000000000000000001"), None);
        assert_eq!(ticks(1.5), 15_000_000);
    }

    #[test]
    fn reads_the_app_header() {
        let h = r#"MediaBrowser Client="Android TV", Device="Shield", DeviceId="abc", Version="0.17", Token="t0k%3D""#;
        assert_eq!(auth_field(h, "token").as_deref(), Some("t0k="));
        assert_eq!(auth_field(h, "client").as_deref(), Some("Android TV"));
        assert_eq!(auth_field("Bearer x", "token"), None);
    }
}
