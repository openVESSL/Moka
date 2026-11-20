//! Background work that moves a film (or an episode, see `series`, or an album, see `music`)
//! from "wanted" to "available":
//! search indexers, pick a release, hand it to the download client, follow
//! the download, and import the finished file into the library.
//!
//! Every step reads its input from the database and writes its result back,
//! so a restart at any point just resumes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use moka_core::{Quality, TitleState};
use moka_db::titles::Title;
use moka_decision::{Candidate, Evaluated, Profile, SizeLimit, Target, Term};
use moka_download::{ClientConfig, Kind, Status};
use moka_indexers::{
    Capabilities, IndexerConfig, IndexerKind, MovieQuery, MusicQuery, TvQuery, torrent::Payload,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::state::{AppState, LiveEvent};

/// Library settings stored under the `library` key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibrarySettings {
    /// Folder films are imported into.
    #[serde(default)]
    pub movies_path: String,
    /// Folder shows are imported into.
    #[serde(default)]
    pub shows_path: String,
    /// Folder albums are imported into.
    #[serde(default)]
    pub music_path: String,
}

impl LibrarySettings {
    pub const KEY: &'static str = "library";
}

/// Result of the last search for a title.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub at: i64,
    pub candidates: Vec<Candidate>,
    pub evaluated: Vec<Evaluated>,
    /// Indexer name and what went wrong with it.
    pub indexer_errors: Vec<(String, String)>,
}

pub const VIDEO_EXTENSIONS: &[&str] =
    &["mkv", "mp4", "m4v", "avi", "mov", "wmv", "ts", "m2ts", "webm", "mpg"];
const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup"];

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Runs forever: a pass every few seconds while something is moving, every
/// minute when idle, and right away when woken.
pub async fn run(state: AppState) {
    loop {
        let busy = match tick(&state).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = ?e, "background pass failed");
                false
            }
        };
        let wait = if busy { Duration::from_secs(5) } else { Duration::from_secs(60) };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = state.wake.notified() => {}
        }
    }
}

async fn tick(state: &AppState) -> anyhow::Result<bool> {
    let tracking = track_downloads(state).await?;
    let importing = import_pending(state).await?;
    // with Radarr in front, it searches for films; shows and music stay here
    if !crate::radarr::tick(state).await? {
        for title in moka_db::titles::due_for_search(&state.db, 3).await? {
            if let Err(e) = search_and_grab(state, &title).await {
                tracing::warn!(title = %title.title, error = ?e, "search failed");
            }
        }
    }
    if let Err(e) = crate::series::tick(state).await {
        tracing::warn!(error = ?e, "episode search failed");
    }
    if let Err(e) = crate::music::tick(state).await {
        tracing::warn!(error = ?e, "album search failed");
    }
    Ok(tracking || importing)
}

/// One background pass, for tests in other modules.
#[cfg(test)]
pub(crate) async fn tick_for_tests(state: &AppState) {
    tick(state).await.unwrap();
}

pub(crate) fn publish_state(state: &AppState, t: &Title) {
    state.publish(LiveEvent::TitleState { title_id: t.id, state: t.state(), reason: t.state_reason.clone() });
}

pub(crate) async fn move_to(
    state: &AppState,
    id: i64,
    to: TitleState,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    let t = moka_db::titles::transition(&state.db, id, to, reason).await?;
    publish_state(state, &t);
    Ok(())
}

/// Moves a grab's title and the other episodes it holds.
async fn move_grab(
    state: &AppState,
    g: &moka_db::grabs::Grab,
    to: TitleState,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    move_to(state, g.title_id, to, reason).await?;
    for id in moka_db::grabs::covered(&state.db, g.id).await? {
        // an episode moved by hand meanwhile keeps its own state
        let _ = move_to(state, id, to, reason).await;
    }
    Ok(())
}

/// Wait before searching again after `attempts` fruitless searches.
pub(crate) fn retry_delay(attempts: i64) -> i64 {
    const STEPS: [i64; 6] = [15 * 60, 3600, 3 * 3600, 6 * 3600, 12 * 3600, 24 * 3600];
    STEPS[(attempts.max(0) as usize).min(STEPS.len() - 1)]
}

pub async fn load_profile(state: &AppState, id: Option<i64>) -> anyhow::Result<Profile> {
    let p = moka_db::library::profile(&state.db, id).await?.ok_or_else(|| anyhow!("no quality profile"))?;
    let keys: Vec<String> = serde_json::from_str(&p.qualities)?;
    Ok(Profile {
        qualities: keys.iter().filter_map(|k| Quality::from_key(k)).collect(),
        cutoff: Quality::from_key(&p.cutoff).unwrap_or_default(),
        upgrades_allowed: p.upgrades_allowed,
        min_seeders: p.min_seeders.max(0) as u32,
        size_limits: serde_json::from_str::<HashMap<String, SizeLimit>>(&p.size_limits).unwrap_or_default(),
        preferred_terms: serde_json::from_str::<Vec<Term>>(&p.preferred_terms).unwrap_or_default(),
    })
}

fn indexer_config(i: &moka_db::connectors::Indexer) -> IndexerConfig {
    IndexerConfig {
        kind: if i.kind == "newznab" { IndexerKind::Newznab } else { IndexerKind::Torznab },
        base_url: i.base_url.clone(),
        api_path: i.api_path.clone(),
        api_key: i.api_key.clone(),
        categories: serde_json::from_str(&i.categories).unwrap_or_default(),
    }
}

/// What to ask indexers for.
#[derive(Debug, Clone)]
pub(crate) enum Query {
    Movie(MovieQuery),
    Tv(TvQuery),
    Music(MusicQuery),
}

/// Asks every usable indexer for a film and evaluates what comes back.
pub async fn search(state: &AppState, title: &Title) -> anyhow::Result<SearchResult> {
    match title.kind.as_str() {
        "movie" => {}
        "episode" => return crate::series::search_episode(state, title).await,
        "album" => return crate::music::search_album(state, title).await,
        _ => bail!("Search a single episode, or let the background search find the season"),
    }
    let query = MovieQuery {
        title: title.title.clone(),
        year: title.year.and_then(|y| u16::try_from(y).ok()),
        imdb_id: title.imdb_id.clone(),
        tmdb_id: title.tmdb_id,
    };
    let target = Target {
        title: title.title.clone(),
        original_title: None,
        year: query.year,
        imdb_id: title.imdb_id.clone(),
        tmdb_id: title.tmdb_id,
        runtime_minutes: title.runtime_minutes.and_then(|m| u32::try_from(m).ok()),
        existing: existing_quality(state, title.id).await?,
        episodes: None,
    };
    search_with(state, title.id, title.quality_profile_id, Query::Movie(query), target).await
}

/// Searches every usable indexer and evaluates the releases for `target`.
/// The result is kept under `unit_id` for the releases view.
pub(crate) async fn search_with(
    state: &AppState,
    unit_id: i64,
    profile_id: Option<i64>,
    query: Query,
    target: Target,
) -> anyhow::Result<SearchResult> {
    let profile = load_profile(state, profile_id).await?;
    let (candidates, indexer_errors) = gather(state, unit_id, &query).await?;
    let evaluated = moka_decision::evaluate(&target, &profile, &candidates);
    let result = SearchResult { at: now(), candidates, evaluated, indexer_errors };
    state.searches.lock().expect("searches lock").insert(unit_id, result.clone());
    Ok(result)
}

/// Asks every usable indexer and returns what they found (with blocklisted
/// releases marked) and which indexers failed.
pub(crate) async fn gather(
    state: &AppState,
    unit_id: i64,
    query: &Query,
) -> anyhow::Result<(Vec<Candidate>, Vec<(String, String)>)> {
    let all = moka_db::connectors::indexers(&state.db).await?;
    let usable: Vec<_> = all
        .into_iter()
        .filter(|i| i.enabled && i.disabled_until.is_none_or(|until| until <= now()))
        .collect();
    if usable.is_empty() {
        bail!("No indexer is available. Add one in Settings, or wait for paused ones to come back.");
    }

    let mut jobs = JoinSet::new();
    for idx in usable {
        let client = state.indexers.clone();
        let db = state.db.clone();
        let query = query.clone();
        jobs.spawn(async move {
            let result = async {
                let cfg = indexer_config(&idx);
                // capabilities saved before TV or music search was read lack it: ask again
                let knows = |field: &str| idx.capabilities.as_deref().is_some_and(|c| c.contains(field));
                let stale = match query {
                    Query::Movie(_) => false,
                    Query::Tv(_) => !knows("tv_search"),
                    Query::Music(_) => !knows("music_search"),
                };
                let caps = match idx
                    .capabilities
                    .as_deref()
                    .filter(|_| !stale)
                    .and_then(|c| serde_json::from_str::<Capabilities>(c).ok())
                {
                    Some(c) => c,
                    None => {
                        let c = client.capabilities(&cfg).await?;
                        if let Ok(json) = serde_json::to_string(&c) {
                            let _ = moka_db::connectors::set_capabilities(&db, idx.id, &json).await;
                        }
                        c
                    }
                };
                match &query {
                    Query::Movie(q) => client.search_movie(&cfg, &caps, q).await,
                    Query::Tv(q) => client.search_tv(&cfg, &caps, q).await,
                    Query::Music(q) => client.search_music(&cfg, &caps, q).await,
                }
            }
            .await;
            (idx, result)
        });
    }

    let blocked = moka_db::grabs::blocklisted(&state.db, unit_id).await?;
    let mut candidates = Vec::new();
    let mut indexer_errors = Vec::new();
    while let Some(joined) = jobs.join_next().await {
        let Ok((idx, result)) = joined else { continue };
        match result {
            Ok(releases) => {
                moka_db::connectors::indexer_ok(&state.db, idx.id).await?;
                for r in releases {
                    let blocklisted = blocked.iter().any(|(t, h)| {
                        *t == r.title || (h.is_some() && h.as_deref() == r.info_hash.as_deref())
                    });
                    candidates.push(Candidate {
                        release: r,
                        indexer_id: idx.id,
                        indexer_name: idx.name.clone(),
                        indexer_priority: idx.priority as i32,
                        blocklisted,
                    });
                }
            }
            Err(e) => {
                indexer_failed(state, &idx, &e).await?;
                indexer_errors.push((idx.name.clone(), e.to_string()));
            }
        }
    }

    Ok((candidates, indexer_errors))
}

/// Pauses a failing indexer: growing waits for passing trouble, until fixed
/// for a bad key or address, exactly what the server asks for on rate limits.
pub async fn indexer_failed(
    state: &AppState,
    idx: &moka_db::connectors::Indexer,
    e: &moka_indexers::Error,
) -> anyhow::Result<()> {
    tracing::warn!(indexer = %idx.name, error = %e, "indexer failed");
    let pause = match e {
        _ if e.is_config_problem() => None,
        moka_indexers::Error::RateLimited { retry_after: Some(d) } => Some(d.as_secs() as i64),
        _ => Some(moka_indexers::backoff(idx.failure_level as u32 + 1).as_secs() as i64),
    };
    moka_db::connectors::indexer_failed(&state.db, idx.id, &e.to_string(), pause).await?;
    Ok(())
}

async fn existing_quality(state: &AppState, title_id: i64) -> anyhow::Result<Option<Quality>> {
    let files = moka_db::library::files_for_title(&state.db, title_id).await?;
    Ok(files.first().and_then(|f| f.quality.as_deref()).and_then(Quality::from_key))
}

/// Automatic search: grabs the best acceptable release, or schedules a retry.
pub async fn search_and_grab(state: &AppState, title: &Title) -> anyhow::Result<()> {
    move_to(state, title.id, TitleState::Searching, None).await?;
    let attempts = moka_db::titles::search_attempts(&state.db, title.id).await.unwrap_or(0);
    let outcome = async {
        let res = search(state, title).await?;
        let best = res.evaluated.iter().find(|e| e.accepted()).cloned();
        match best {
            Some(e) => {
                let cand = res.candidates[e.candidate_index].clone();
                grab(state, title.id, &cand, e.quality_key()).await.map(|_| None)
            }
            None => {
                let found = res.candidates.len();
                let msg = if found == 0 {
                    "No releases found yet".to_owned()
                } else {
                    format!("{found} releases found, none fits the quality profile")
                };
                Ok(Some(msg))
            }
        }
    }
    .await;

    let reason = match outcome {
        Ok(None) => return Ok(()),
        Ok(Some(msg)) => msg,
        Err(e) => e.to_string(),
    };
    move_to(state, title.id, TitleState::Wanted, Some(&reason)).await?;
    moka_db::titles::schedule_search(&state.db, title.id, now() + retry_delay(attempts), Some(&reason))
        .await?;
    Ok(())
}

pub fn download_client(
    state: &AppState,
    c: &moka_db::connectors::DownloadClient,
) -> Arc<moka_download::Client> {
    let mut clients = state.download_clients.lock().expect("clients lock");
    clients
        .entry(c.id)
        .or_insert_with(|| {
            let kind = Kind::parse(&c.kind).unwrap_or(Kind::Qbittorrent);
            Arc::new(moka_download::Client::new(
                kind,
                ClientConfig {
                    base_url: c.base_url.clone(),
                    username: c.username.clone(),
                    password: c.password.clone(),
                    category: c.category.clone(),
                },
            ))
        })
        .clone()
}

/// Sends a release to the download client and records the grab. `quality`
/// is the key stored with the grab (`webdl-1080p`, `flac`).
pub async fn grab(
    state: &AppState,
    title_id: i64,
    cand: &Candidate,
    quality: Option<String>,
) -> anyhow::Result<()> {
    grab_covering(state, title_id, &[], cand, quality).await
}

/// `grab` for a release that also holds other episodes (`covered`).
pub(crate) async fn grab_covering(
    state: &AppState,
    title_id: i64,
    covered: &[i64],
    cand: &Candidate,
    quality: Option<String>,
) -> anyhow::Result<()> {
    // never two downloads in flight for one title
    if moka_db::grabs::active(&state.db).await?.iter().any(|g| g.title_id == title_id) {
        bail!("This is already downloading");
    }
    let r = &cand.release;
    let torrent = r.protocol == moka_indexers::Protocol::Torrent;
    let clients = moka_db::connectors::download_clients(&state.db).await?;
    let client_row = clients
        .into_iter()
        .find(|c| c.enabled && Kind::parse(&c.kind).is_some_and(|k| k.is_torrent() == torrent))
        .ok_or_else(|| {
            if torrent {
                anyhow!("No torrent client is connected. Add qBittorrent or Transmission in Settings.")
            } else {
                anyhow!("No Usenet client is connected. Add SABnzbd in Settings.")
            }
        })?;
    let fetched = match (&r.magnet_url, &r.download_url) {
        (Some(m), _) if torrent => Payload::Magnet(m.clone()),
        (_, Some(url)) => state.indexers.fetch(url).await.context("could not download the release file")?,
        _ => bail!("The release has no download link"),
    };
    let client = download_client(state, &client_row);

    let id = if torrent {
        let hash = fetched
            .info_hash()
            .or_else(|| r.info_hash.clone())
            .ok_or_else(|| anyhow!("Could not work out the torrent's hash, so it could not be tracked"))?;
        // after a crash between sending and recording, the torrent may already be there
        let already = client.list().await.map(|l| l.iter().any(|d| d.id == hash)).unwrap_or(false);
        if already {
            hash
        } else {
            let payload = match &fetched {
                Payload::Magnet(m) => moka_download::Payload::Magnet(m),
                Payload::File(b) => moka_download::Payload::Torrent(b),
            };
            let returned = client.add(payload, &r.title, Some(&hash)).await?;
            if returned.is_empty() { hash } else { returned }
        }
    } else {
        let Payload::File(nzb) = &fetched else {
            bail!("The indexer returned a magnet link for a Usenet release")
        };
        client.add(moka_download::Payload::Nzb(nzb), &r.title, None).await?
    };
    let grab_id = moka_db::grabs::create(
        &state.db,
        &moka_db::grabs::NewGrab {
            title_id,
            indexer_id: Some(cand.indexer_id),
            download_client_id: client_row.id,
            release_title: r.title.clone(),
            size_bytes: r.size_bytes.and_then(|s| i64::try_from(s).ok()),
            quality,
            info_hash: id,
        },
    )
    .await?;
    let sent = format!("Sent {} to {}", r.title, client_row.name);
    move_to(state, title_id, TitleState::Grabbed, Some(&sent)).await?;
    let others: Vec<i64> = covered.iter().copied().filter(|id| *id != title_id).collect();
    moka_db::grabs::add_covered(&state.db, grab_id, &others).await?;
    for id in others {
        let _ = move_to(state, id, TitleState::Grabbed, Some(&sent)).await;
    }
    if let Some(t) = moka_db::titles::by_id(&state.db, title_id).await? {
        let info = crate::notify::TitleInfo::from_title(&t, Some(r.title.clone()));
        crate::notify::send(state, crate::notify::Event::Grabbed, Some(info), None);
    }
    state.wake.notify_one();
    Ok(())
}

/// Polls download clients for every grab in flight. Returns true if any.
async fn track_downloads(state: &AppState) -> anyhow::Result<bool> {
    let active = moka_db::grabs::active(&state.db).await?;
    if active.is_empty() {
        return Ok(false);
    }
    let mut by_client: HashMap<i64, Vec<moka_db::grabs::Grab>> = HashMap::new();
    for g in active {
        by_client.entry(g.download_client_id.unwrap_or(0)).or_default().push(g);
    }
    for (client_id, grabs) in by_client {
        let Some(row) = moka_db::connectors::download_client(&state.db, client_id).await? else {
            for g in grabs {
                fail_grab(state, &g, "The download client was removed").await?;
            }
            continue;
        };
        let list = match download_client(state, &row).list().await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(client = %row.name, error = %e, "could not poll download client");
                continue;
            }
        };
        for g in grabs {
            let hash = g.info_hash.clone().unwrap_or_default();
            match list.iter().find(|d| d.id == hash) {
                Some(d) if d.status == Status::Failed => {
                    let why = d
                        .message
                        .clone()
                        .unwrap_or_else(|| "The download client gave up on this download".into());
                    fail_grab(state, &g, &why).await?;
                }
                Some(d) if d.status == Status::Completed => {
                    if g.status != "completed" {
                        moka_db::grabs::set_status(
                            &state.db,
                            g.id,
                            "completed",
                            None,
                            d.content_path.as_deref(),
                        )
                        .await?;
                        move_grab(state, &g, TitleState::Importing, None).await?;
                    }
                }
                Some(d) => {
                    if g.status == "sent" {
                        moka_db::grabs::set_status(&state.db, g.id, "downloading", None, None).await?;
                        move_grab(state, &g, TitleState::Downloading, None).await?;
                    }
                    moka_db::titles::set_reason(&state.db, g.title_id, d.message.as_deref()).await?;
                    state.publish(LiveEvent::DownloadProgress {
                        title_id: g.title_id,
                        progress: d.progress,
                        eta_seconds: d.eta_seconds,
                    });
                }
                // qBittorrent can take a moment to list a new magnet
                None if g.status == "sent" && now() - g.created_at < 15 * 60 => {}
                None => fail_grab(state, &g, "The download is no longer in the download client").await?,
            }
        }
    }
    Ok(true)
}

pub(crate) async fn fail_grab(
    state: &AppState,
    g: &moka_db::grabs::Grab,
    reason: &str,
) -> anyhow::Result<()> {
    moka_db::grabs::set_status(&state.db, g.id, "failed", Some(reason), None).await?;
    for id in moka_db::grabs::covered(&state.db, g.id).await? {
        if let Ok(t) = moka_db::titles::transition(&state.db, id, TitleState::Wanted, Some(reason)).await {
            moka_db::titles::schedule_search(&state.db, t.id, now(), Some(reason)).await?;
            publish_state(state, &t);
        }
    }
    moka_db::grabs::blocklist(&state.db, g.title_id, &g.release_title, g.info_hash.as_deref(), reason)
        .await?;
    if let Ok(t) = moka_db::titles::transition(&state.db, g.title_id, TitleState::Wanted, Some(reason)).await
    {
        moka_db::titles::schedule_search(&state.db, t.id, now(), Some(reason)).await?;
        crate::notify::send(
            state,
            crate::notify::Event::Failed,
            Some(crate::notify::TitleInfo::from_title(&t, Some(reason.to_owned()))),
            None,
        );
        publish_state(state, &t);
    }
    Ok(())
}

async fn import_pending(state: &AppState) -> anyhow::Result<bool> {
    let titles = moka_db::titles::importing(&state.db).await?;
    let busy = !titles.is_empty();
    for t in titles {
        // episodes inside a season pack are imported with the season's grab
        let Some(g) = moka_db::grabs::latest_for_title(&state.db, t.id).await? else { continue };
        if g.status != "completed" {
            continue;
        }
        let done = match t.kind.as_str() {
            "movie" => import(state, &t, &g).await,
            "album" => crate::music::import(state, &t, &g).await,
            _ => crate::series::import(state, &t, &g).await,
        };
        match done {
            Ok(()) => {}
            Err(ImportError::Wait(reason)) => {
                moka_db::titles::set_reason(&state.db, t.id, Some(&reason)).await?
            }
            Err(ImportError::Failed(reason)) => fail_grab(state, &g, &reason).await?,
        }
    }
    Ok(busy)
}

pub(crate) enum ImportError {
    /// Fixable by the user; try again later.
    Wait(String),
    /// This download is unusable.
    Failed(String),
}

impl From<anyhow::Error> for ImportError {
    fn from(e: anyhow::Error) -> Self {
        ImportError::Wait(e.to_string())
    }
}

async fn import(state: &AppState, t: &Title, g: &moka_db::grabs::Grab) -> Result<(), ImportError> {
    let lib: LibrarySettings = moka_db::settings::get(&state.db, LibrarySettings::KEY)
        .await
        .map_err(anyhow::Error::from)?
        .unwrap_or_default();
    if lib.movies_path.trim().is_empty() {
        return Err(ImportError::Wait("Choose a library folder in Settings to finish importing".into()));
    }
    let source = g.output_path.as_deref().map(PathBuf::from).ok_or_else(|| {
        ImportError::Failed("The download client did not report where the files are".into())
    })?;
    if !source.exists() {
        return Err(ImportError::Wait(format!(
            "Cannot see {} from the server. The download folder must be reachable at the same path.",
            source.display()
        )));
    }

    let src = source.clone();
    let video = tokio::task::spawn_blocking(move || main_video(&src))
        .await
        .map_err(|e| ImportError::Wait(e.to_string()))?
        .ok_or_else(|| ImportError::Failed("No video file in the download".into()))?;

    let quality = g.quality.as_deref().and_then(Quality::from_key);
    let parsed = moka_parser::parse_movie(&g.release_title);
    let folder = library_folder_name(t);
    let ext = video.extension().and_then(|e| e.to_str()).unwrap_or("mkv").to_ascii_lowercase();
    let file_stem = match quality {
        Some(q) => format!("{folder} - {q}"),
        None => folder.clone(),
    };
    let dest_dir = Path::new(lib.movies_path.trim()).join(&folder);
    let dest = dest_dir.join(format!("{}.{ext}", sanitize(&file_stem)));

    let (v, d, dd) = (video.clone(), dest.clone(), dest_dir.clone());
    let size = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
        std::fs::create_dir_all(&dd)?;
        place(&v, &d)?;
        if let Some(parent) = v.parent() {
            copy_subtitles(parent, &v, &d);
        }
        Ok(std::fs::metadata(&d)?.len())
    })
    .await
    .map_err(|e| ImportError::Wait(e.to_string()))?
    .map_err(|e| ImportError::Wait(format!("Could not write to the library folder: {e}")))?;

    moka_db::library::add_file(
        &state.db,
        t.id,
        &dest.to_string_lossy(),
        size as i64,
        g.quality.as_deref(),
        parsed.as_ref().and_then(|p| p.group.as_deref()),
    )
    .await
    .map_err(anyhow::Error::from)?;
    moka_db::grabs::set_status(&state.db, g.id, "imported", None, None).await.map_err(anyhow::Error::from)?;
    move_to(state, t.id, TitleState::Available, None).await?;
    let detail = quality.map(|q| q.to_string());
    crate::notify::send(
        state,
        crate::notify::Event::Available,
        Some(crate::notify::TitleInfo::from_title(t, detail)),
        None,
    );
    tracing::info!(title = %t.title, path = %dest.display(), "imported");
    crate::jellyfin::announce_import(state, dest.to_string_lossy().into_owned());
    Ok(())
}

/// `Title (Year) {tmdb-ID}`: the id makes later rescans unambiguous.
pub fn library_folder_name(t: &Title) -> String {
    let mut name = t.title.clone();
    if let Some(y) = t.year {
        name.push_str(&format!(" ({y})"));
    }
    if let Some(id) = t.tmdb_id {
        name.push_str(&format!(" {{tmdb-{id}}}"));
    }
    sanitize(&name)
}

/// Removes characters that are invalid in file names on common systems.
pub fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            ':' => '-',
            '<' | '>' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(['.', ' ']).to_owned()
}

/// The biggest video file in a download, skipping samples.
pub fn main_video(path: &Path) -> Option<PathBuf> {
    videos(path).into_iter().max_by_key(|(s, _)| *s).map(|(_, p)| p)
}

/// Every video file in a download with its size, skipping samples and trailers.
pub(crate) fn videos(path: &Path) -> Vec<(u64, PathBuf)> {
    let is_video = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    };
    if path.is_file() {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        return if is_video(path) { vec![(size, path.to_path_buf())] } else { vec![] };
    }
    let mut found = Vec::new();
    let mut stack = vec![(path.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                stack.push((p, depth + 1));
            } else if is_video(&p) {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
                if name.contains("sample") || name.contains("trailer") {
                    continue;
                }
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                found.push((size, p));
            }
        }
    }
    found
}

/// Hardlink when possible (no extra space, the torrent keeps seeding), copy otherwise.
// shortcut: never moves, even when seeding is done; moving needs the client's "can move" flag carried here.
pub(crate) fn place(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.exists() {
        std::fs::remove_file(to)?;
    }
    if std::fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    let tmp = to.with_extension("partial");
    std::fs::copy(from, &tmp)?;
    std::fs::rename(&tmp, to)
}

/// Copies subtitle files next to the video, keeping a language suffix like `.en`.
pub(crate) fn copy_subtitles(dir: &Path, video: &Path, dest: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let dest_stem = dest.with_extension("");
    let video_stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
    for e in entries.flatten() {
        let p = e.path();
        let Some(ext) = p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase) else { continue };
        if !SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // "Film.en.forced.srt" -> ".en.forced"
        let middle =
            name.strip_prefix(video_stem.as_str()).unwrap_or("").trim_end_matches(&format!(".{ext}"));
        let middle = if middle.is_empty() || middle.len() > 20 { String::new() } else { middle.to_owned() };
        let target = PathBuf::from(format!("{}{middle}.{ext}", dest_stem.display()));
        if !target.exists() {
            let _ = place(&p, &target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe() {
        assert_eq!(sanitize("Mission: Impossible / Fallout?"), "Mission- Impossible Fallout");
        assert_eq!(sanitize("Film..."), "Film");
    }

    #[test]
    fn retry_backs_off() {
        assert_eq!(retry_delay(0), 900);
        assert_eq!(retry_delay(2), 3 * 3600);
        assert_eq!(retry_delay(50), 24 * 3600);
    }

    #[test]
    fn picks_biggest_video_and_brings_subtitles() {
        let root = std::env::temp_dir().join(format!("moka-import-{}", now()));
        let dl = root.join("download/Film.2020.1080p");
        std::fs::create_dir_all(dl.join("Sample")).unwrap();
        std::fs::write(dl.join("Film.2020.1080p.mkv"), vec![0u8; 2048]).unwrap();
        std::fs::write(dl.join("Sample/sample.mkv"), vec![0u8; 4096]).unwrap();
        std::fs::write(dl.join("Film.2020.1080p.en.srt"), "1\n").unwrap();
        std::fs::write(dl.join("notes.nfo"), "x").unwrap();

        let video = main_video(&dl).unwrap();
        assert!(video.ends_with("Film.2020.1080p.mkv"), "samples are skipped even when bigger");

        let lib = root.join("library/Film (2020)");
        std::fs::create_dir_all(&lib).unwrap();
        let dest = lib.join("Film (2020) - WEB-DL 1080p.mkv");
        place(&video, &dest).unwrap();
        copy_subtitles(&dl, &video, &dest);
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), 2048);
        assert!(lib.join("Film (2020) - WEB-DL 1080p.en.srt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod flow {
    use super::*;
    use axum::{Router, extract::Query, routing::get, routing::post};
    use moka_db::titles::NewTitle;

    const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    /// Request to library with a fake indexer and a fake qBittorrent.
    #[tokio::test]
    async fn wanted_film_ends_up_in_the_library() {
        let root = std::env::temp_dir().join(format!("moka-flow-{}-{}", std::process::id(), now()));
        let download = root.join("downloads/Test.Film.2020.1080p.WEB-DL.H264-GRP");
        std::fs::create_dir_all(&download).unwrap();
        std::fs::write(download.join("Test.Film.2020.1080p.WEB-DL.H264-GRP.mkv"), vec![7u8; 4096]).unwrap();
        let library = root.join("movies");

        let indexer = serve(Router::new().route(
            "/api",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                match q.get("t").map(String::as_str) {
                    Some("caps") => r#"<caps><searching><movie-search available="yes" supportedParams="q,tmdbid"/></searching></caps>"#.to_owned(),
                    _ => format!(r#"<rss><channel>
                        <item><title>Test.Film.2020.1080p.WEB-DL.H264-GRP</title>
                          <link>magnet:?xt=urn:btih:{HASH}</link><size>3221225472</size>
                          <attr name="seeders" value="10"/></item>
                        <item><title>Test.Film.2020.CAM-BAD</title>
                          <link>magnet:?xt=urn:btih:ffffffffffffffffffffffffffffffffffffffff</link><size>900000000</size>
                          <attr name="seeders" value="900"/></item>
                      </channel></rss>"#),
                }
            }),
        ))
        .await;

        let content = download.to_string_lossy().into_owned();
        let qbit = serve(
            Router::new()
                .route("/api/v2/auth/login", post(|| async { "Ok." }))
                .route("/api/v2/torrents/createCategory", post(|| async { "" }))
                .route("/api/v2/torrents/add", post(|| async { "Ok." }))
                .route(
                    "/api/v2/torrents/info",
                    get(move || {
                        let content = content.clone();
                        async move {
                            axum::Json(serde_json::json!([{ "hash": HASH.to_uppercase(), "name": "Test.Film",
                                "state": "stalledUP", "progress": 1.0, "size": 4096,
                                "save_path": "/elsewhere", "content_path": content }]))
                        }
                    }),
                ),
        )
        .await;

        let db = moka_db::memory().await;
        let config = crate::config::Config {
            config_dir: root.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        moka_db::connectors::create_indexer(
            &db,
            &moka_db::connectors::IndexerInput {
                name: "fake".into(),
                kind: "torznab".into(),
                base_url: indexer,
                api_path: "/api".into(),
                api_key: None,
                categories: vec![2000],
                priority: 25,
                enabled: true,
            },
        )
        .await
        .unwrap();
        moka_db::connectors::create_download_client(
            &db,
            &moka_db::connectors::DownloadClientInput {
                name: "qbit".into(),
                kind: "qbittorrent".into(),
                base_url: qbit,
                username: None,
                password: None,
                category: "moka".into(),
                enabled: true,
            },
        )
        .await
        .unwrap();
        let film = NewTitle {
            tmdb_id: 42,
            title: "Test Film".into(),
            year: Some(2020),
            runtime_minutes: Some(100),
            ..Default::default()
        };
        let t = moka_db::titles::upsert_movie(&db, &film, TitleState::Wanted).await.unwrap();

        // pass 1: search and grab the WEB-DL (the CAM is not in the profile)
        tick(&state).await.unwrap();
        let t = moka_db::titles::by_id(&db, t.id).await.unwrap().unwrap();
        assert_eq!(t.state(), TitleState::Grabbed, "{:?}", t.state_reason);

        // pass 2: client reports it complete; the library folder is not set yet
        tick(&state).await.unwrap();
        let t = moka_db::titles::by_id(&db, t.id).await.unwrap().unwrap();
        assert_eq!(t.state(), TitleState::Importing);
        assert!(t.state_reason.unwrap().contains("library folder"));

        // pass 3: with a library folder the file is placed and the film is available
        let lib =
            LibrarySettings { movies_path: library.to_string_lossy().into_owned(), ..Default::default() };
        moka_db::settings::set(&db, LibrarySettings::KEY, &lib).await.unwrap();
        tick(&state).await.unwrap();
        let t = moka_db::titles::by_id(&db, t.id).await.unwrap().unwrap();
        assert_eq!(t.state(), TitleState::Available);

        let files = moka_db::library::files_for_title(&db, t.id).await.unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            files[0].path.ends_with("Test Film (2020) {tmdb-42} - WEB-DL 1080p.mkv"),
            "{}",
            files[0].path
        );
        assert_eq!(std::fs::metadata(&files[0].path).unwrap().len(), 4096);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod usenet_flow {
    use super::*;
    use axum::{Router, extract::Query, routing::get};
    use moka_db::titles::NewTitle;

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn newznab_release_goes_through_sabnzbd_into_the_library() {
        let root = std::env::temp_dir().join(format!("moka-usenet-{}-{}", std::process::id(), now()));
        let done = root.join("complete/Usenet.Film.2021.1080p.WEB-DL.H264-GRP");
        std::fs::create_dir_all(&done).unwrap();
        std::fs::write(done.join("Usenet.Film.2021.1080p.WEB-DL.H264-GRP.mkv"), vec![1u8; 2048]).unwrap();

        // serves the .nzb the feed links to
        let nzb_host = serve(Router::new().route("/api", get(|| async { "<nzb>fake</nzb>" }))).await;
        // the feed links to the .nzb host
        let indexer_for_link = nzb_host.clone();
        let indexer = serve(Router::new().route("/api", get(move |Query(q): Query<HashMap<String, String>>| {
            let base = indexer_for_link.clone();
            async move {
                match q.get("t").map(String::as_str) {
                    Some("caps") => r#"<caps><searching><movie-search available="yes" supportedParams="q"/></searching></caps>"#.to_owned(),
                    _ => format!(r#"<rss><channel><item><title>Usenet.Film.2021.1080p.WEB-DL.H264-GRP</title>
                        <link>{base}/api?t=get&amp;id=1</link><size>3221225472</size></item></channel></rss>"#),
                }
            }
        })))
        .await;

        let storage = done.to_string_lossy().into_owned();
        let sab = serve(
            Router::new().route(
                "/api",
                get({
                    let storage = storage.clone();
                    move |Query(q): Query<HashMap<String, String>>| {
                        let storage = storage.clone();
                        async move {
                            axum::Json(match q["mode"].as_str() {
                                "queue" => serde_json::json!({ "queue": { "slots": [] } }),
                                "history" => serde_json::json!({ "history": { "slots": [
                                { "nzo_id": "SABnzbd_nzo_7", "name": "Usenet.Film", "status": "Completed",
                                  "storage": storage, "bytes": 2048, "category": "moka" } ] } }),
                                _ => serde_json::json!({ "status": true }),
                            })
                        }
                    }
                })
                .post(|| async {
                    axum::Json(serde_json::json!({ "status": true, "nzo_ids": ["SABnzbd_nzo_7"] }))
                }),
            ),
        )
        .await;

        let db = moka_db::memory().await;
        let config = crate::config::Config {
            config_dir: root.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        moka_db::connectors::create_indexer(
            &db,
            &moka_db::connectors::IndexerInput {
                name: "nz".into(),
                kind: "newznab".into(),
                base_url: indexer,
                api_path: "/api".into(),
                api_key: None,
                categories: vec![2000],
                priority: 25,
                enabled: true,
            },
        )
        .await
        .unwrap();
        moka_db::connectors::create_download_client(
            &db,
            &moka_db::connectors::DownloadClientInput {
                name: "sab".into(),
                kind: "sabnzbd".into(),
                base_url: sab,
                username: None,
                password: Some("key".into()),
                category: "moka".into(),
                enabled: true,
            },
        )
        .await
        .unwrap();
        let lib = LibrarySettings {
            movies_path: root.join("movies").to_string_lossy().into_owned(),
            ..Default::default()
        };
        moka_db::settings::set(&db, LibrarySettings::KEY, &lib).await.unwrap();
        let t = moka_db::titles::upsert_movie(
            &db,
            &NewTitle {
                tmdb_id: 7,
                title: "Usenet Film".into(),
                year: Some(2021),
                runtime_minutes: Some(100),
                ..Default::default()
            },
            TitleState::Wanted,
        )
        .await
        .unwrap();

        tick(&state).await.unwrap();
        let after_grab = moka_db::titles::by_id(&db, t.id).await.unwrap().unwrap();
        assert_eq!(after_grab.state(), TitleState::Grabbed, "{:?}", after_grab.state_reason);
        let g = moka_db::grabs::latest_for_title(&db, t.id).await.unwrap().unwrap();
        assert_eq!(g.info_hash.as_deref(), Some("SABnzbd_nzo_7"), "tracked by the SABnzbd job id");

        tick(&state).await.unwrap(); // completed -> importing
        tick(&state).await.unwrap(); // importing -> available
        let t = moka_db::titles::by_id(&db, t.id).await.unwrap().unwrap();
        assert_eq!(t.state(), TitleState::Available, "{:?}", t.state_reason);
        let _ = std::fs::remove_dir_all(&root);
    }
}
