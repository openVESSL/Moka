//! Playback for apps: what a file holds and how to play it (the file as is,
//! or a converted HLS stream when the app's profile cannot play it), the
//! file itself, subtitles as WebVTT, and where each user stopped.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use moka_db::library::MediaFile;
use moka_db::titles::Title;
use serde_json::{Value, json};

use super::items::Library;
use super::{AppUser, IdKind, Q, guid, parse_guid, seconds, ticks};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::streaming::{ClientCaps, Probe};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/items/{id}/playbackinfo", get(playback_info).post(playback_info))
        .route("/videos/{id}/master.m3u8", get(master))
        .route("/videos/{id}/main.m3u8", get(master))
        .route("/videos/{id}/hls/{session}/{name}", get(hls))
        .route("/videos/{id}/{file}", get(stream).head(stream))
        .route("/videos/{id}/{source}/subtitles/{index}/{file}", get(subtitle))
        .route("/videos/{id}/{source}/subtitles/{index}/{start}/{file}", get(subtitle_from))
        .route("/sessions/playing", post(progress))
        .route("/sessions/playing/progress", post(progress))
        .route("/sessions/playing/stopped", post(stopped))
        .route("/sessions/playing/ping", post(|| async { StatusCode::NO_CONTENT }))
        .route("/userplayeditems/{id}", post(mark_played).delete(mark_unplayed))
        .route("/users/{uid}/playeditems/{id}", post(user_mark_played).delete(user_mark_unplayed))
        .route("/userfavoriteitems/{id}", post(favorite).delete(unfavorite))
        .route("/users/{uid}/favoriteitems/{id}", post(user_favorite).delete(user_unfavorite))
}

/// HLS sessions started for apps, by the app's play session id, so a new
/// start or a stop ends the previous conversion.
static APP_STREAMS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

fn split(list: Option<&str>) -> Vec<String> {
    list.map(|l| l.split(',').map(|x| x.trim().to_ascii_lowercase()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

/// A field of the app's JSON body, whatever its casing.
fn field<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    v.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)
}

fn container_names(c: &str) -> &'static [&'static str] {
    match c {
        "mp4" => &["mp4", "m4v", "mov"],
        "mkv" => &["mkv", "matroska", "webm"],
        "webm" => &["webm"],
        "avi" => &["avi"],
        "ts" => &["ts", "mpegts", "m2ts"],
        _ => &[],
    }
}

fn codec_names(c: &str) -> Vec<String> {
    let c = c.to_ascii_lowercase();
    let mut out = vec![c.clone()];
    match c.as_str() {
        "hevc" => out.push("h265".into()),
        "h264" => out.push("avc".into()),
        _ => {}
    }
    out
}

/// Whether the app's device profile plays this file as it is. Without a
/// profile the app is trusted to know.
fn plays_directly(
    profile: Option<&Value>,
    container: &str,
    video: Option<&str>,
    audio: Option<&str>,
) -> bool {
    let Some(list) = profile.and_then(|p| field(p, "DirectPlayProfiles")).and_then(Value::as_array) else {
        return true;
    };
    let names = container_names(container);
    list.iter().any(|p| {
        let get = |n: &str| split(field(p, n).and_then(Value::as_str));
        let kind = field(p, "Type").and_then(Value::as_str).unwrap_or("Video");
        if !kind.eq_ignore_ascii_case("video") {
            return false;
        }
        let (containers, videos, audios) = (get("Container"), get("VideoCodec"), get("AudioCodec"));
        let ok = |allowed: &[String], have: Option<&str>| {
            allowed.is_empty() || have.is_some_and(|h| codec_names(h).iter().any(|c| allowed.contains(c)))
        };
        (containers.is_empty() || containers.iter().any(|c| names.contains(&c.as_str())))
            && ok(&videos, video)
            && (audio.is_none() || ok(&audios, audio))
    })
}

/// What a converted stream may use, from the app's transcoding profile.
fn transcode_caps(profile: Option<&Value>) -> (Vec<String>, Vec<String>) {
    let first =
        profile.and_then(|p| field(p, "TranscodingProfiles")).and_then(Value::as_array).and_then(|l| {
            l.iter().find(|p| {
                field(p, "Type").and_then(Value::as_str).is_none_or(|t| t.eq_ignore_ascii_case("video"))
            })
        });
    let get = |n: &str| split(first.and_then(|p| field(p, n)).and_then(Value::as_str));
    let mut video: Vec<String> =
        get("VideoCodec").into_iter().filter(|c| c == "h264" || c == "hevc").collect();
    let mut audio: Vec<String> = get("AudioCodec")
        .into_iter()
        .filter(|c| matches!(c.as_str(), "aac" | "mp3" | "ac3" | "eac3" | "opus" | "flac"))
        .collect();
    if video.is_empty() {
        video.push("h264".into());
    }
    if audio.is_empty() {
        audio.push("aac".into());
    }
    (video, audio)
}

fn language_name(code: Option<&str>) -> String {
    match code.unwrap_or("") {
        "eng" | "en" => "English".into(),
        "rum" | "ron" | "ro" => "Romanian".into(),
        "fre" | "fra" | "fr" => "French".into(),
        "ger" | "deu" | "de" => "German".into(),
        "spa" | "es" => "Spanish".into(),
        "ita" | "it" => "Italian".into(),
        "jpn" | "ja" => "Japanese".into(),
        "" | "und" => "Unknown".into(),
        other => other.to_owned(),
    }
}

fn channels_label(n: u32) -> &'static str {
    match n {
        1 => "Mono",
        2 => "Stereo",
        6 => "5.1",
        8 => "7.1",
        _ => "",
    }
}

/// Streams as apps number them: video 0, then audio, then subtitles.
fn media_streams(
    item: &str,
    source: &str,
    token: &str,
    probe: &Probe,
    subs: &[crate::subtitles::Track],
) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(v) = &probe.video {
        let ten_bit = v.pix_fmt.as_deref().is_some_and(|f| f.contains("10"));
        let res = match v.height {
            h if h >= 2000 => "4K",
            h if h >= 1000 => "1080p",
            h if h >= 700 => "720p",
            _ => "SD",
        };
        out.push(json!({
            "Index": 0, "Type": "Video", "Codec": v.codec, "Profile": v.profile, "Width": v.width, "Height": v.height,
            "PixelFormat": v.pix_fmt, "BitDepth": if ten_bit { 10 } else { 8 }, "VideoRange": "SDR", "VideoRangeType": "SDR",
            "IsDefault": true, "IsForced": false, "IsInterlaced": false, "IsExternal": false, "IsHearingImpaired": false, "IsOriginal": false, "AudioSpatialFormat": "None",
            "IsTextSubtitleStream": false, "SupportsExternalStream": false, "IsAVC": v.codec == "h264",
            "DisplayTitle": format!("{res} {}", v.codec.to_uppercase()),
        }));
    }
    for (i, a) in probe.audio.iter().enumerate() {
        let lang = language_name(a.language.as_deref());
        let title: Vec<String> = [lang, a.codec.to_uppercase(), channels_label(a.channels).to_owned()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        out.push(json!({
            "Index": 1 + i, "Type": "Audio", "Codec": a.codec, "Channels": a.channels, "Language": a.language,
            "ChannelLayout": channels_label(a.channels).to_lowercase(), "IsDefault": a.default, "IsForced": false,
            "IsInterlaced": false, "IsExternal": false, "IsHearingImpaired": false, "IsOriginal": false, "AudioSpatialFormat": "None", "IsTextSubtitleStream": false,
            "SupportsExternalStream": false,
            "DisplayTitle": title.join(" - "),
        }));
    }
    let first = 1 + probe.audio.len();
    for (i, s) in subs.iter().enumerate() {
        let index = first + i;
        let mut v = json!({
            "Index": index, "Type": "Subtitle", "Codec": s.codec, "Language": s.language, "Title": s.label,
            "IsDefault": s.default, "IsForced": s.forced, "IsInterlaced": false, "IsExternal": s.external,
            "IsHearingImpaired": false, "IsOriginal": false, "AudioSpatialFormat": "None", "IsTextSubtitleStream": s.text, "SupportsExternalStream": s.text,
            "DisplayTitle": format!("{} - {}", language_name(s.language.as_deref()), s.codec.to_uppercase()),
            "DeliveryMethod": if s.text { "External" } else { "Encode" },
        });
        if s.text {
            v["DeliveryUrl"] =
                json!(format!("/Videos/{item}/{source}/Subtitles/{index}/0/Stream.vtt?api_key={token}"));
            v["IsExternalUrl"] = json!(false);
        }
        out.push(v);
    }
    out
}

/// The one way to play a title's file: as it is when the app can, else as a
/// converted HLS stream.
pub(super) async fn media_source(
    state: &AppState,
    lib: &Library,
    t: &Title,
    f: &MediaFile,
    token: &str,
    request: Option<&Value>,
) -> ApiResult<Value> {
    let path = std::path::PathBuf::from(&f.path);
    if !path.exists() {
        return Err(ApiError::NotFound("file on disk"));
    }
    let probe = crate::api::play::probe_cached(state, f.id, &path).await?;
    let subs = crate::subtitles::tracks(&path, &probe);
    let item = guid(IdKind::Title, t.id);
    let source = guid(IdKind::File, f.id);
    let container = crate::streaming::container_of(&probe.format);
    let audio = probe.audio.iter().find(|a| a.default).or_else(|| probe.audio.first());
    let profile = request.and_then(|r| field(r, "DeviceProfile"));
    let allowed = |name: &str| request.and_then(|r| field(r, name)).and_then(Value::as_bool).unwrap_or(true);
    let direct = allowed("EnableDirectPlay")
        && plays_directly(
            profile,
            container,
            probe.video.as_ref().map(|v| v.codec.as_str()),
            audio.map(|a| a.codec.as_str()),
        );
    let default_audio = probe.audio.iter().position(|a| a.default).unwrap_or(0) + 1;
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut v = json!({
        "Protocol": "File",
        "Id": source,
        "Path": path.file_name().map(|n| n.to_string_lossy().into_owned()),
        "Type": "Default",
        "Container": if container == "other" { path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default() } else { container.to_owned() },
        "Size": f.size_bytes,
        "Name": name,
        "IsRemote": false,
        "ETag": format!("{:x}", f.added_at),
        "RunTimeTicks": ticks(probe.duration_seconds),
        "ReadAtNativeFramerate": false,
        "IgnoreDts": false,
        "IgnoreIndex": false,
        "GenPtsInput": false,
        "SupportsTranscoding": true,
        "SupportsDirectStream": direct,
        "SupportsDirectPlay": direct,
        "IsInfiniteStream": false,
        "UseMostCompatibleTranscodingProfile": false,
        "RequiresOpening": false,
        "RequiresClosing": false,
        "RequiresLooping": false,
        "SupportsProbing": true,
        "VideoType": "VideoFile",
        "MediaStreams": media_streams(&item, &source, token, &probe, &subs),
        "MediaAttachments": [],
        "Formats": [],
        "Bitrate": if probe.duration_seconds > 0.0 { (f.size_bytes as f64 * 8.0 / probe.duration_seconds) as i64 } else { 0 },
        "RequiredHttpHeaders": {},
        "TranscodingSubProtocol": "http",
        "DefaultAudioStreamIndex": default_audio,
        "HasSegments": false,
    });
    if !direct {
        let (video, audio) = transcode_caps(profile);
        let session = request
            .and_then(|r| field(r, "PlaySessionId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| guid(IdKind::File, f.id));
        let audio_index = request
            .and_then(|r| field(r, "AudioStreamIndex"))
            .and_then(Value::as_i64)
            .unwrap_or(default_audio as i64);
        v["TranscodingUrl"] = json!(format!(
            "/videos/{item}/master.m3u8?MediaSourceId={source}&PlaySessionId={session}&api_key={token}&VideoCodec={}&AudioCodec={}&AudioStreamIndex={audio_index}",
            video.join(","),
            audio.join(","),
        ));
        v["TranscodingSubProtocol"] = json!("hls");
        v["TranscodingContainer"] = json!("mp4");
    }
    let _ = lib;
    Ok(v)
}

/// The title (film or episode) and file an id points at: a title id, or a
/// file id from `MediaSourceId`.
async fn resolve(state: &AppState, id: &str, source: Option<&str>) -> ApiResult<(Title, MediaFile)> {
    let file = match source.and_then(parse_guid) {
        Some((IdKind::File, n)) => moka_db::library::file(&state.db, n).await?,
        _ => match parse_guid(id) {
            Some((IdKind::Title, n)) => {
                moka_db::library::files_for_title(&state.db, n).await?.into_iter().next()
            }
            Some((IdKind::File, n)) => moka_db::library::file(&state.db, n).await?,
            _ => None,
        },
    }
    .ok_or(ApiError::NotFound("file"))?;
    let title = moka_db::titles::by_id(&state.db, file.title_id).await?.ok_or(ApiError::NotFound("title"))?;
    if !matches!(title.kind.as_str(), "movie" | "episode") {
        return Err(ApiError::NotFound("video"));
    }
    Ok((title, file))
}

async fn playback_info(
    u: AppUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    q: Q,
    body: axum::body::Bytes,
) -> Response {
    if let Err(r) = u.can_play() {
        return r.into_response();
    }
    let mut request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    if !request.is_object() {
        request = json!({});
    }
    let play_session = {
        let mut b = [0u8; 16];
        let _ = getrandom::fill(&mut b);
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    };
    request["PlaySessionId"] = json!(play_session);
    if let Some(a) = q.int("audiostreamindex") {
        request["AudioStreamIndex"] = json!(a);
    }
    let run = async {
        let source_param = q
            .get("mediasourceid")
            .map(str::to_owned)
            .or_else(|| field(&request, "MediaSourceId").and_then(Value::as_str).map(str::to_owned));
        let (title, file) = resolve(&state, &id, source_param.as_deref()).await?;
        let lib = Library::load(&state, u.user.id).await?;
        let source = media_source(&state, &lib, &title, &file, &u.token, Some(&request)).await?;
        ApiResult::Ok(json!({ "MediaSources": [source], "PlaySessionId": play_session }))
    };
    match run.await {
        Ok(v) => Json(v).into_response(),
        Err(ApiError::NotFound(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

/// The file as it is, with range requests (`/Videos/{id}/stream` and
/// `/Videos/{id}/stream.mkv`).
async fn stream(
    u: AppUser,
    State(state): State<AppState>,
    Path((id, file)): Path<(String, String)>,
    q: Q,
    req: Request,
) -> Response {
    use tower::ServiceExt;
    if let Err(r) = u.can_play() {
        return r.into_response();
    }
    if !file.starts_with("stream") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (_, f) = match resolve(&state, &id, q.get("mediasourceid")).await {
        Ok(x) => x,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    match tower_http::services::ServeFile::new(&f.path).oneshot(req).await {
        Ok(res) => res.map(axum::body::Body::new),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Starts converting the file and answers with a playlist pointing at the
/// stream. Seeking far ahead starts a new conversion from `StartTimeTicks`.
async fn master(u: AppUser, State(state): State<AppState>, Path(id): Path<String>, q: Q) -> Response {
    if let Err(r) = u.can_play() {
        return r.into_response();
    }
    let run = async {
        let (_, f) = resolve(&state, &id, q.get("mediasourceid")).await?;
        let path = std::path::PathBuf::from(&f.path);
        let probe = crate::api::play::probe_cached(&state, f.id, &path).await?;
        let caps = ClientCaps {
            video_codecs: split(q.get("videocodec"))
                .into_iter()
                .filter(|c| c == "h264" || c == "hevc")
                .collect(),
            audio_codecs: split(q.get("audiocodec")),
            containers: vec![],
            max_height: q.int("maxheight").and_then(|h| u32::try_from(h).ok()),
        };
        let caps = ClientCaps {
            video_codecs: if caps.video_codecs.is_empty() { vec!["h264".into()] } else { caps.video_codecs },
            audio_codecs: if caps.audio_codecs.is_empty() { vec!["aac".into()] } else { caps.audio_codecs },
            ..caps
        };
        let mut decision = crate::streaming::decide(&probe, &caps);
        if let Some(n) = q
            .int("audiostreamindex")
            .and_then(|i| usize::try_from(i - 1).ok())
            .filter(|n| *n < probe.audio.len())
        {
            decision.audio_index = Some(n);
        }
        let start = q
            .int("starttimeticks")
            .map(seconds)
            .unwrap_or(0.0)
            .clamp(0.0, (probe.duration_seconds - 5.0).max(0.0));
        let max_height = caps.max_height.unwrap_or(1080).clamp(360, 2160);
        let hw: crate::streaming::HwAccel =
            moka_db::settings::get(&state.db, crate::streaming::HwAccel::KEY).await?.unwrap_or_default();
        let dir = state.config.transcode_dir();
        let session = match state.streams.start(&dir, &path, &decision, start, max_height, &hw).await {
            Ok(s) => s,
            Err(_) if hw.encoder != crate::streaming::Encoder::Software && !decision.copy_video => state
                .streams
                .start(&dir, &path, &decision, start, max_height, &Default::default())
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?,
            Err(e) => return Err(ApiError::BadRequest(e.to_string())),
        };
        if let Some(play) = q.get("playsessionid") {
            let old = APP_STREAMS.lock().expect("app streams").insert(play.to_owned(), session.clone());
            if let Some(old) = old {
                state.streams.stop(&old).await;
            }
        }
        ApiResult::Ok(session)
    };
    match run.await {
        Ok(session) => (
            [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"), (header::CACHE_CONTROL, "no-cache")],
            format!(
                "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-STREAM-INF:BANDWIDTH=12000000\nhls/{session}/index.m3u8\n"
            ),
        )
            .into_response(),
        Err(ApiError::NotFound(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

/// Playlist and segments of a running conversion. Players fetch these with
/// relative links that carry no token; the session id, random and short-lived,
/// stands in for it.
async fn hls(
    State(state): State<AppState>,
    Path((_, session, name)): Path<(String, String, String)>,
) -> Response {
    if !crate::streaming::valid_hls_name(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(dir) = state.streams.touch(&session) else { return StatusCode::NOT_FOUND.into_response() };
    let Ok(bytes) = tokio::fs::read(dir.join(&name)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = match name.rsplit('.').next() {
        Some("m3u8") => "application/vnd.apple.mpegurl",
        Some("mp4") => "video/mp4",
        _ => "video/iso.segment",
    };
    ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache")], bytes).into_response()
}

async fn subtitle(
    u: AppUser,
    State(state): State<AppState>,
    Path((id, source, index, _)): Path<(String, String, String, String)>,
) -> Response {
    subtitle_at(u, state, id, source, index).await
}

async fn subtitle_from(
    u: AppUser,
    State(state): State<AppState>,
    Path((id, source, index, _, _)): Path<(String, String, String, String, String)>,
) -> Response {
    subtitle_at(u, state, id, source, index).await
}

/// A text subtitle track as WebVTT, by its stream index.
async fn subtitle_at(u: AppUser, state: AppState, id: String, source: String, index: String) -> Response {
    if let Err(r) = u.can_play() {
        return r.into_response();
    }
    let run = async {
        let (_, f) = resolve(&state, &id, Some(&source)).await?;
        let path = std::path::PathBuf::from(&f.path);
        let probe = crate::api::play::probe_cached(&state, f.id, &path).await?;
        let n: usize = index.parse().map_err(|_| ApiError::NotFound("subtitle track"))?;
        let track = n
            .checked_sub(1 + probe.audio.len())
            .and_then(|i| crate::subtitles::tracks(&path, &probe).into_iter().nth(i))
            .ok_or(ApiError::NotFound("subtitle track"))?;
        crate::api::play::subtitle_text(&state, f.id, &track.id, false).await
    };
    match run.await {
        Ok(text) => ([(header::CONTENT_TYPE, "text/vtt; charset=utf-8")], text).into_response(),
        Err(ApiError::NotFound(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

/// The title of a progress report, and the position in seconds.
async fn report(state: &AppState, body: &Value) -> Option<(Title, f64)> {
    let id = field(body, "ItemId").and_then(Value::as_str)?;
    let (IdKind::Title, n) = parse_guid(id)? else { return None };
    let t = moka_db::titles::by_id(&state.db, n).await.ok().flatten()?;
    let at = field(body, "PositionTicks").and_then(Value::as_i64).map(seconds).unwrap_or(0.0);
    Some((t, at))
}

async fn progress(u: AppUser, State(state): State<AppState>, body: axum::body::Bytes) -> StatusCode {
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    if let Some((t, at)) = report(&state, &body).await
        && at > 0.0
    {
        let _ = moka_db::library::set_progress(&state.db, u.user.id, t.id, at, false).await;
    }
    StatusCode::NO_CONTENT
}

/// Stopping near the end counts as watched.
async fn stopped(u: AppUser, State(state): State<AppState>, body: axum::body::Bytes) -> StatusCode {
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    if let Some(play) = field(&body, "PlaySessionId").and_then(Value::as_str) {
        let running = APP_STREAMS.lock().expect("app streams").remove(play);
        if let Some(s) = running {
            state.streams.stop(&s).await;
        }
    }
    if let Some((t, at)) = report(&state, &body).await {
        let file =
            moka_db::library::files_for_title(&state.db, t.id).await.ok().and_then(|f| f.into_iter().next());
        let length = file
            .and_then(|f| f.duration_seconds)
            .or_else(|| t.runtime_minutes.map(|m| m as f64 * 60.0))
            .unwrap_or(0.0);
        let done = length > 0.0 && at >= length * 0.9;
        let _ = moka_db::library::set_progress(&state.db, u.user.id, t.id, if done { 0.0 } else { at }, done)
            .await;
    }
    StatusCode::NO_CONTENT
}

/// The episodes a played mark covers: the title itself, or every episode
/// with a file in a show or season.
async fn covered(state: &AppState, user: i64, id: &str) -> ApiResult<(Library, Vec<i64>, i64)> {
    let lib = Library::load(state, user).await?;
    let Some((IdKind::Title, n)) = parse_guid(id) else { return Err(ApiError::NotFound("item")) };
    let t = lib.titles.get(&n).ok_or(ApiError::NotFound("item"))?;
    let ids = match t.kind.as_str() {
        "show" => lib.episodes(Some(n), None).iter().map(|e| e.id).collect(),
        "season" => lib.episodes(None, Some(n)).iter().map(|e| e.id).collect(),
        _ => vec![n],
    };
    Ok((lib, ids, n))
}

async fn set_played(u: AppUser, state: AppState, id: String, played: bool) -> Response {
    let run = async {
        let (_, ids, n) = covered(&state, u.user.id, &id).await?;
        for t in ids {
            moka_db::library::set_progress(&state.db, u.user.id, t, 0.0, played).await?;
        }
        let lib = Library::load(&state, u.user.id).await?;
        let t = lib.titles.get(&n).ok_or(ApiError::NotFound("item"))?;
        ApiResult::Ok(lib.dto(t)["UserData"].clone())
    };
    match run.await {
        Ok(v) => Json(v).into_response(),
        Err(ApiError::NotFound(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn set_favorite(u: AppUser, state: AppState, id: String, favorite: bool) -> Response {
    let run = async {
        let Some((IdKind::Title, n)) = parse_guid(&id) else { return Err(ApiError::NotFound("item")) };
        moka_db::library::set_favorite(&state.db, u.user.id, n, favorite).await?;
        let lib = Library::load(&state, u.user.id).await?;
        let t = lib.titles.get(&n).ok_or(ApiError::NotFound("item"))?;
        ApiResult::Ok(lib.dto(t)["UserData"].clone())
    };
    match run.await {
        Ok(v) => Json(v).into_response(),
        Err(ApiError::NotFound(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn mark_played(u: AppUser, State(s): State<AppState>, Path(id): Path<String>) -> Response {
    set_played(u, s, id, true).await
}
async fn mark_unplayed(u: AppUser, State(s): State<AppState>, Path(id): Path<String>) -> Response {
    set_played(u, s, id, false).await
}
async fn user_mark_played(
    u: AppUser,
    State(s): State<AppState>,
    Path((_, id)): Path<(String, String)>,
) -> Response {
    set_played(u, s, id, true).await
}
async fn user_mark_unplayed(
    u: AppUser,
    State(s): State<AppState>,
    Path((_, id)): Path<(String, String)>,
) -> Response {
    set_played(u, s, id, false).await
}
async fn favorite(u: AppUser, State(s): State<AppState>, Path(id): Path<String>) -> Response {
    set_favorite(u, s, id, true).await
}
async fn unfavorite(u: AppUser, State(s): State<AppState>, Path(id): Path<String>) -> Response {
    set_favorite(u, s, id, false).await
}
async fn user_favorite(
    u: AppUser,
    State(s): State<AppState>,
    Path((_, id)): Path<(String, String)>,
) -> Response {
    set_favorite(u, s, id, true).await
}
async fn user_unfavorite(
    u: AppUser,
    State(s): State<AppState>,
    Path((_, id)): Path<(String, String)>,
) -> Response {
    set_favorite(u, s, id, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_profiles_decide_direct_play() {
        let profile = json!({ "DirectPlayProfiles": [
            { "Type": "Video", "Container": "mp4,m4v", "VideoCodec": "h264,h265", "AudioCodec": "aac,ac3" },
            { "Type": "Audio", "Container": "mkv" }] });
        let p = Some(&profile);
        assert!(plays_directly(p, "mp4", Some("hevc"), Some("aac")), "hevc is h265");
        assert!(!plays_directly(p, "mkv", Some("h264"), Some("aac")), "audio profiles do not count");
        assert!(!plays_directly(p, "mp4", Some("h264"), Some("dts")));
        assert!(plays_directly(None, "avi", Some("mpeg4"), None), "no profile: trust the app");
        let t = json!({ "TranscodingProfiles": [{ "Type": "Video", "VideoCodec": "hevc,h264,vp9", "AudioCodec": "aac,mp3,dts" }] });
        assert_eq!(
            transcode_caps(Some(&t)),
            (vec!["hevc".into(), "h264".into()], vec!["aac".into(), "mp3".into()])
        );
        assert_eq!(transcode_caps(None), (vec!["h264".into()], vec!["aac".into()]));
    }
}
