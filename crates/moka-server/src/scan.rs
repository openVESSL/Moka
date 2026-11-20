//! Brings an existing film or show folder into the library: each film folder
//! or file, or each show folder, is recognized (TMDB id in the name, else
//! title + year search) and added as available. Albums are found as
//! `Artist/Album (Year)` and matched on MusicBrainz. Files stay where they are.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use moka_core::{TitleKind, TitleState};
use moka_db::titles::NewTitle;
use moka_metadata::music::AlbumSummary;
use regex::Regex;
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::requests::RequestKind;
use crate::pipeline::{VIDEO_EXTENSIONS, main_video};
use crate::state::AppState;

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct ScanStatus {
    pub running: bool,
    pub folder: Option<String>,
    pub total: usize,
    pub done: usize,
    pub added: usize,
    pub already_known: usize,
    /// Names that could not be matched to a film, for the user to fix.
    pub unmatched: Vec<String>,
    pub error: Option<String>,
}

/// One candidate film on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Folder or file name used to recognize the film.
    pub name: String,
    pub video: PathBuf,
}

/// Film folders and loose video files directly under `root`.
pub fn find_films(root: &Path) -> std::io::Result<Vec<Found>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(root)?.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            if let Some(video) = main_video(&p) {
                out.push(Found { name, video });
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| VIDEO_EXTENSIONS.contains(&x.to_ascii_lowercase().as_str()))
        {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name);
            out.push(Found { name: stem, video: p });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Starts a scan in the background. Returns false if one is already running.
pub fn start(state: &AppState, root: PathBuf, kind: RequestKind) -> bool {
    {
        let mut s = state.scan.lock().expect("scan lock");
        if s.running {
            return false;
        }
        *s = ScanStatus { running: true, folder: Some(root.display().to_string()), ..Default::default() };
    }
    let state = state.clone();
    tokio::spawn(async move {
        let done = match kind {
            RequestKind::Movie => run(&state, &root).await,
            RequestKind::Show => run_shows(&state, &root).await,
            RequestKind::Album => run_albums(&state, &root).await,
        };
        if let Err(e) = done {
            state.scan.lock().expect("scan lock").error = Some(e.to_string());
        }
        state.scan.lock().expect("scan lock").running = false;
    });
    true
}

async fn run(state: &AppState, root: &Path) -> anyhow::Result<()> {
    let r = root.to_path_buf();
    let films = tokio::task::spawn_blocking(move || find_films(&r)).await??;
    state.scan.lock().expect("scan lock").total = films.len();
    let tmdb = state.tmdb();

    for f in films {
        let outcome = add_one(state, &tmdb, &f).await;
        {
            let mut s = state.scan.lock().expect("scan lock");
            s.done += 1;
            match outcome {
                Ok(true) => s.added += 1,
                Ok(false) => s.already_known += 1,
                Err(_) => s.unmatched.push(f.name.clone()),
            }
        }
        // stay well under TMDB's rate limit
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    Ok(())
}

/// Ok(true) added, Ok(false) already known, Err not recognized.
async fn add_one(state: &AppState, tmdb: &moka_metadata::Tmdb, f: &Found) -> anyhow::Result<bool> {
    let path = f.video.to_string_lossy().into_owned();
    let parsed = moka_parser::parse_movie(&f.name).ok_or_else(|| anyhow::anyhow!("unreadable name"))?;

    let tmdb_id = match parsed.tmdb_id {
        Some(id) => id,
        None => {
            let page = tmdb.search(&parsed.title, 1).await?;
            let year = parsed.year.map(i64::from);
            page.results
                .iter()
                .find(|m| year.is_none() || m.year().zip(year).is_some_and(|(a, b)| (a - b).abs() <= 1))
                .map(|m| m.id)
                .ok_or_else(|| anyhow::anyhow!("no match"))?
        }
    };

    if let Some(existing) = moka_db::titles::by_tmdb(&state.db, TitleKind::Movie, tmdb_id).await?
        && moka_db::library::files_for_title(&state.db, existing.id).await?.iter().any(|m| m.path == path)
    {
        return Ok(false);
    }

    let d = tmdb.movie(tmdb_id).await?;
    let title = moka_db::titles::upsert_movie(
        &state.db,
        &NewTitle {
            tmdb_id: d.id,
            imdb_id: d.imdb().map(str::to_owned),
            title: d.title.clone(),
            original_title: d.original_title.clone(),
            year: d.year(),
            overview: d.overview.clone(),
            runtime_minutes: d.runtime.filter(|r| *r > 0),
            release_date: d.release_date.clone(),
            poster_path: d.poster_path.clone(),
            backdrop_path: d.backdrop_path.clone(),
            metadata_json: serde_json::to_string(&d).ok(),
        },
        TitleState::Available,
    )
    .await?;
    moka_db::titles::mark_available(&state.db, title.id).await?;

    let file_name = f.video.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (quality, _) = moka_parser::parse_quality(&file_name);
    let quality = (!quality.is_unknown()).then(|| quality.key());
    let size = tokio::fs::metadata(&f.video).await.map(|m| m.len() as i64).unwrap_or(0);
    moka_db::library::add_file(&state.db, title.id, &path, size, quality.as_deref(), parsed.group.as_deref())
        .await?;
    Ok(true)
}

/// Show folders directly under `root`.
pub fn find_shows(root: &Path) -> std::io::Result<Vec<Found>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(root)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') && e.path().is_dir() {
            out.push(Found { name, video: e.path() });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

async fn run_shows(state: &AppState, root: &Path) -> anyhow::Result<()> {
    let r = root.to_path_buf();
    let shows = tokio::task::spawn_blocking(move || find_shows(&r)).await??;
    state.scan.lock().expect("scan lock").total = shows.len();
    let tmdb = state.tmdb();
    for f in shows {
        let outcome = add_show(state, &tmdb, &f).await;
        {
            let mut s = state.scan.lock().expect("scan lock");
            s.done += 1;
            match outcome {
                Ok(true) => s.added += 1,
                Ok(false) => s.already_known += 1,
                Err(_) => s.unmatched.push(f.name.clone()),
            }
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    Ok(())
}

/// Ok(true) a new show, Ok(false) a known one (new files still added),
/// Err not recognized. `f.video` is the show folder.
async fn add_show(state: &AppState, tmdb: &moka_metadata::Tmdb, f: &Found) -> anyhow::Result<bool> {
    let parsed = moka_parser::parse_movie(&f.name).ok_or_else(|| anyhow::anyhow!("unreadable name"))?;
    let tmdb_id = match parsed.tmdb_id {
        Some(id) => id,
        None => {
            let page = tmdb.search_shows(&parsed.title, 1).await?;
            let year = parsed.year.map(i64::from);
            page.results
                .iter()
                .find(|m| year.is_none() || m.year().zip(year).is_some_and(|(a, b)| (a - b).abs() <= 1))
                .map(|m| m.id)
                .ok_or_else(|| anyhow::anyhow!("no match"))?
        }
    };
    let known = moka_db::titles::by_tmdb(&state.db, TitleKind::Show, tmdb_id).await?.is_some();
    let new = crate::api::catalog::new_show(state, tmdb_id).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let show = moka_db::titles::upsert_show(&state.db, &new, TitleState::Wanted).await?;
    let episodes = moka_db::titles::episodes_of_show(&state.db, show.id).await?;

    let dir = f.video.clone();
    let files = tokio::task::spawn_blocking(move || crate::pipeline::videos(&dir)).await?;
    for (size, video) in files {
        let path = video.to_string_lossy().into_owned();
        let name = video.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(p) = moka_parser::parse_episode(&name) else { continue };
        let season = i64::from(p.seasons.first().copied().unwrap_or(0));
        let found: Vec<&moka_db::titles::Title> = episodes
            .iter()
            .filter(|e| e.season_number == Some(season))
            .filter(|e| e.episode_number.is_some_and(|n| p.episodes.iter().any(|x| i64::from(*x) == n)))
            .collect();
        let Some(first) = found.first() else { continue };
        let quality = (!p.release.quality.is_unknown()).then(|| p.release.quality.key());
        moka_db::library::add_file(
            &state.db,
            first.id,
            &path,
            size as i64,
            quality.as_deref(),
            p.release.group.as_deref(),
        )
        .await?;
        for e in found {
            moka_db::titles::mark_available(&state.db, e.id).await?;
        }
    }
    if !known {
        moka_db::titles::unmonitor_missing(&state.db, show.id).await?;
    }
    Ok(!known)
}

/// One album folder on disk, `Artist/Album (Year)`.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundAlbum {
    /// `Artist/Album folder`, as the scan reports it.
    pub name: String,
    pub artist: String,
    pub album: String,
    pub year: Option<i64>,
    pub dir: PathBuf,
}

/// "2001 - Album" puts the year first.
static LEADING_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^((?:19|20)\d{2})\s*[-.]\s*(.+)$").unwrap());

/// Album title and year from a folder name: "Album (2001) [FLAC]",
/// "2001 - Album" or just "Album".
fn album_name(folder: &str) -> (String, Option<i64>) {
    if let Some(c) = LEADING_YEAR.captures(folder) {
        let (title, year) = album_name(&c[2]);
        return (title, year.or_else(|| c[1].parse().ok()));
    }
    match moka_parser::parse_album(&format!("x - {folder}")) {
        Some(p) => (p.album, p.year.map(i64::from)),
        None => (folder.trim().to_owned(), None),
    }
}

/// Album folders with audio files, two levels under `root`.
pub fn find_albums(root: &Path) -> std::io::Result<Vec<FoundAlbum>> {
    let dirs = |p: &Path| -> std::io::Result<Vec<(String, PathBuf)>> {
        Ok(std::fs::read_dir(p)?
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .filter(|(n, p)| !n.starts_with('.') && p.is_dir())
            .collect())
    };
    let mut out = Vec::new();
    for (artist, artist_dir) in dirs(root)? {
        for (folder, dir) in dirs(&artist_dir).unwrap_or_default() {
            if crate::music::audio_files(&dir).is_empty() {
                continue;
            }
            let (album, year) = album_name(&folder);
            out.push(FoundAlbum {
                name: format!("{artist}/{folder}"),
                artist: artist.clone(),
                album,
                year,
                dir,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

async fn run_albums(state: &AppState, root: &Path) -> anyhow::Result<()> {
    let r = root.to_path_buf();
    let albums = tokio::task::spawn_blocking(move || find_albums(&r)).await??;
    state.scan.lock().expect("scan lock").total = albums.len();
    let mb = state.music();
    // artists MusicBrainz did not know, so their other albums are not asked again
    let mut unknown = std::collections::HashSet::new();
    for f in albums {
        let outcome = if unknown.contains(&f.artist) {
            Err(anyhow::anyhow!("unknown artist"))
        } else {
            match artist_albums(state, &mb, &f.artist).await {
                Ok(albums) => add_album(state, &albums, &f).await,
                Err(e) => {
                    unknown.insert(f.artist.clone());
                    Err(e)
                }
            }
        };
        let mut s = state.scan.lock().expect("scan lock");
        s.done += 1;
        match outcome {
            Ok(true) => s.added += 1,
            Ok(false) => s.already_known += 1,
            Err(_) => s.unmatched.push(f.name.clone()),
        }
    }
    Ok(())
}

/// The albums of the artist whose name matches the folder exactly
/// (ignoring case and punctuation). MusicBrainz is asked at most once a
/// second by its client, and answers are cached like catalog pages.
async fn artist_albums(
    state: &AppState,
    mb: &moka_metadata::music::MusicBrainz,
    name: &str,
) -> anyhow::Result<Vec<AlbumSummary>> {
    let wanted = moka_parser::normalize_title(name);
    let found = state
        .cached(format!("music-search:{}", name.to_lowercase()), Duration::from_secs(600), || {
            mb.search_artists(name)
        })
        .await?;
    let artist = found
        .into_iter()
        .find(|a| {
            moka_parser::normalize_title(&a.name) == wanted
                || a.sort_name.as_deref().is_some_and(|s| moka_parser::normalize_title(s) == wanted)
        })
        .ok_or_else(|| anyhow::anyhow!("no artist named {name}"))?;
    let day = Duration::from_secs(24 * 3600);
    Ok(state.cached(format!("music-albums:{}", artist.id), day, || mb.albums(&artist.id)).await?)
}

/// Same title; the same year and a studio album are preferred, since
/// reissues and live records share titles.
fn pick_album<'a>(albums: &'a [AlbumSummary], f: &FoundAlbum) -> Option<&'a AlbumSummary> {
    let wanted = moka_parser::normalize_title(&f.album);
    albums
        .iter()
        .filter(|a| moka_parser::normalize_title(&a.title) == wanted)
        .min_by_key(|a| (f.year.is_some() && a.year() != f.year, !a.is_studio()))
}

/// Ok(true) new track files added, Ok(false) all already known, Err not
/// recognized. Found tracks become available; the album too, with a reason
/// when tracks are missing. A new album with no matching file is not kept.
async fn add_album(state: &AppState, albums: &[AlbumSummary], f: &FoundAlbum) -> anyhow::Result<bool> {
    let db = &state.db;
    let summary = pick_album(albums, f).ok_or_else(|| anyhow::anyhow!("no album named {}", f.album))?;
    let known = moka_db::titles::by_mbid(db, TitleKind::Album, &summary.id).await?.is_some();
    let new = crate::api::music::new_album(state, &summary.id).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let album = moka_db::titles::upsert_album(db, &new, TitleState::Wanted).await?;
    let tracks: Vec<moka_db::titles::Title> =
        moka_db::titles::children(db, album.id).await?.into_iter().filter(|t| t.kind == "track").collect();

    let dir = f.dir.clone();
    let files = tokio::task::spawn_blocking(move || crate::music::audio_files(&dir)).await?;
    let matched = crate::music::match_tracks(&files, &tracks);
    if matched.is_empty() {
        if !known {
            moka_db::titles::delete(db, album.id).await?;
            if let Some(artist) = album.parent_id
                && moka_db::titles::children(db, artist).await?.is_empty()
            {
                moka_db::titles::delete(db, artist).await?;
            }
        }
        anyhow::bail!("no file matches the album's tracks");
    }

    let folder = f.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut added = false;
    for (file, track) in &matched {
        let path = file.to_string_lossy().into_owned();
        if moka_db::library::files_for_title(db, track.id).await?.iter().any(|m| m.path == path) {
            continue;
        }
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        let quality = moka_parser::audio_quality(&format!("{folder} {ext}"));
        let quality = (quality != moka_core::AudioQuality::Unknown).then(|| quality.to_string());
        let size = tokio::fs::metadata(file).await.map(|m| m.len() as i64).unwrap_or(0);
        moka_db::library::add_file(db, track.id, &path, size, quality.as_deref(), None).await?;
        moka_db::titles::mark_available(db, track.id).await?;
        added = true;
    }
    if added {
        moka_db::titles::mark_available(db, album.id).await?;
        let reason = (matched.len() < tracks.len())
            .then(|| format!("{} of {} tracks found on disk", matched.len(), tracks.len()));
        moka_db::titles::set_reason(db, album.id, reason.as_deref()).await?;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::get};

    #[tokio::test]
    async fn scans_folders_and_loose_files() {
        let root = std::env::temp_dir().join(format!("moka-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Arrival (2016)")).unwrap();
        std::fs::write(root.join("Arrival (2016)/Arrival.2016.1080p.BluRay.x264.mkv"), b"x").unwrap();
        std::fs::create_dir_all(root.join("Known {tmdb-77}")).unwrap();
        std::fs::write(root.join("Known {tmdb-77}/k.mp4"), b"x").unwrap();
        std::fs::write(root.join("Nothing Matches 1999.avi"), b"x").unwrap();
        std::fs::create_dir_all(root.join("Empty folder")).unwrap();

        let found = find_films(&root).unwrap();
        assert_eq!(
            found.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["Arrival (2016)", "Known {tmdb-77}", "Nothing Matches 1999"]
        );

        let tmdb = Router::new()
            .route(
                "/search/movie",
                get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    let results = if q["query"] == "Arrival" {
                        serde_json::json!([{ "id": 329865, "title": "Arrival", "release_date": "2016-11-10" }])
                    } else {
                        serde_json::json!([])
                    };
                    Json(serde_json::json!({ "page": 1, "total_pages": 1, "total_results": 1, "results": results }))
                }),
            )
            .route("/movie/329865", get(|| async { Json(serde_json::json!({ "id": 329865, "title": "Arrival", "release_date": "2016-11-10" })) }))
            .route("/movie/77", get(|| async { Json(serde_json::json!({ "id": 77, "title": "Memento", "release_date": "2000-10-11" })) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, tmdb).await.unwrap() });

        let db = moka_db::memory().await;
        let config = crate::config::Config {
            config_dir: root.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        state.set_tmdb(moka_metadata::Tmdb::with_base_url(&base, "k", "en-US", None));

        run(&state, &root).await.unwrap();
        let s = state.scan.lock().unwrap().clone();
        assert_eq!((s.total, s.added, s.unmatched.clone()), (3, 2, vec!["Nothing Matches 1999".to_owned()]));

        let arrival = moka_db::titles::by_tmdb(&db, TitleKind::Movie, 329865).await.unwrap().unwrap();
        assert_eq!(arrival.state(), TitleState::Available);
        let files = moka_db::library::files_for_title(&db, arrival.id).await.unwrap();
        assert_eq!(files[0].quality.as_deref(), Some("bluray-1080p"));

        // a second scan finds nothing new
        *state.scan.lock().unwrap() = ScanStatus::default();
        run(&state, &root).await.unwrap();
        assert_eq!(state.scan.lock().unwrap().already_known, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn shows_on_disk_keep_what_is_there() {
        let root = std::env::temp_dir().join(format!("moka-scan-shows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let season = root.join("Test Show (2020)/Season 01");
        std::fs::create_dir_all(&season).unwrap();
        std::fs::write(season.join("Test.Show.S01E01.1080p.WEB-DL.mkv"), b"x").unwrap();
        std::fs::create_dir_all(root.join("Unknown Thing")).unwrap();

        let tmdb = Router::new()
            .route(
                "/search/tv",
                get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    let results = if q["query"] == "Test Show" {
                        serde_json::json!([{ "id": 900, "name": "Test Show", "first_air_date": "2020-01-01" }])
                    } else {
                        serde_json::json!([])
                    };
                    Json(serde_json::json!({ "page": 1, "total_pages": 1, "total_results": 1, "results": results }))
                }),
            )
            .route("/tv/900", get(|| async { Json(serde_json::json!({ "id": 900, "name": "Test Show", "first_air_date": "2020-01-01",
                "seasons": [{"id": 901, "season_number": 1, "name": "Season 1", "episode_count": 3}] })) }))
            .route("/tv/900/season/1", get(|| async { Json(serde_json::json!({ "id": 901, "season_number": 1, "name": "Season 1",
                "episodes": [
                    {"id": 911, "season_number": 1, "episode_number": 1, "name": "One", "air_date": "2020-01-01"},
                    {"id": 912, "season_number": 1, "episode_number": 2, "name": "Two", "air_date": "2020-01-08"},
                    {"id": 913, "season_number": 1, "episode_number": 3, "name": "Three", "air_date": "2999-01-01"}] })) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, tmdb).await.unwrap() });

        let db = moka_db::memory().await;
        let config = crate::config::Config {
            config_dir: root.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        state.set_tmdb(moka_metadata::Tmdb::with_base_url(&base, "k", "en-US", None));

        run_shows(&state, &root).await.unwrap();
        let s = state.scan.lock().unwrap().clone();
        assert_eq!((s.total, s.added, s.unmatched.clone()), (2, 1, vec!["Unknown Thing".to_owned()]));

        let show = moka_db::titles::by_tmdb(&db, TitleKind::Show, 900).await.unwrap().unwrap();
        let eps = moka_db::titles::episodes_of_show(&db, show.id).await.unwrap();
        let view: Vec<(TitleState, bool)> = eps.iter().map(|e| (e.state(), e.monitored)).collect();
        assert_eq!(
            view,
            [(TitleState::Available, true), (TitleState::Wanted, false), (TitleState::Wanted, true)],
            "on disk: in the library; aired but missing: not followed; still to air: followed"
        );
        assert_eq!(
            moka_db::library::files_for_title(&db, eps[0].id).await.unwrap()[0].quality.as_deref(),
            Some("webdl-1080p")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn album_folder_names() {
        let n = |s: &str| album_name(s);
        assert_eq!(n("First Light (2001)"), ("First Light".into(), Some(2001)));
        assert_eq!(n("2004 - Second Dark [MP3 320]"), ("Second Dark".into(), Some(2004)));
        assert_eq!(n("Kid A"), ("Kid A".into(), None));
    }

    #[tokio::test]
    async fn albums_on_disk_become_available() {
        let root = std::env::temp_dir().join(format!("moka-scan-music-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |p: &str| {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        write("Test Band/First Light (2001) [FLAC]/01 - Opening.flac");
        write("Test Band/First Light (2001) [FLAC]/02 - Middle.flac");
        write("Test Band/First Light (2001) [FLAC]/cover.jpg");
        write("Test Band/2004 - Second Dark [MP3 320]/01 Dusk.mp3");
        write("Test Band/Not An Album/01 Thing.flac");
        write("Test Band/Scans/front.jpg");
        write("Nobody Known/Some Album/01 Song.flac");

        let found = find_albums(&root).unwrap();
        assert_eq!(
            found.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            [
                "Nobody Known/Some Album",
                "Test Band/2004 - Second Dark [MP3 320]",
                "Test Band/First Light (2001) [FLAC]",
                "Test Band/Not An Album"
            ],
            "folders without audio are skipped"
        );

        let mb = Router::new()
            .route(
                "/artist",
                get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    let artists = if q["query"] == "Test Band" {
                        serde_json::json!([{ "id": "a1", "name": "Test Band" }, { "id": "a2", "name": "Test Band Tribute" }])
                    } else {
                        serde_json::json!([{ "id": "a3", "name": "Somebody Else" }])
                    };
                    Json(serde_json::json!({ "artists": artists }))
                }),
            )
            .route(
                "/release-group",
                get(|| async {
                    Json(serde_json::json!({ "release-groups": [
                        { "id": "rg1", "title": "First Light", "primary-type": "Album", "first-release-date": "2001-03-01" },
                        { "id": "rg1-live", "title": "First Light", "primary-type": "Album", "secondary-types": ["Live"], "first-release-date": "2001-09-01" },
                        { "id": "rg2", "title": "Second Dark", "primary-type": "Album", "first-release-date": "2004-05-01" }] }))
                }),
            )
            .route(
                "/release",
                get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    let tracks = match q["release-group"].as_str() {
                        "rg1" => serde_json::json!([{ "position": 1, "title": "Opening" }, { "position": 2, "title": "Middle" }, { "position": 3, "title": "Closing" }]),
                        "rg2" => serde_json::json!([{ "position": 1, "title": "Dusk" }]),
                        other => panic!("not expected: {other}"),
                    };
                    Json(serde_json::json!({ "releases": [{ "id": "r", "title": if q["release-group"] == "rg1" { "First Light" } else { "Second Dark" },
                        "status": "Official", "date": "2001", "media": [{ "position": 1, "tracks": tracks }],
                        "artist-credit": [{ "name": "Test Band", "artist": { "id": "a1" } }] }] }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, mb).await.unwrap() });

        let db = moka_db::memory().await;
        let config = crate::config::Config {
            config_dir: root.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        state.set_music(moka_metadata::music::MusicBrainz::with_base_url(&base));

        run_albums(&state, &root).await.unwrap();
        let s = state.scan.lock().unwrap().clone();
        assert_eq!(
            (s.total, s.added, s.unmatched.clone()),
            (4, 2, vec!["Nobody Known/Some Album".to_owned(), "Test Band/Not An Album".to_owned()])
        );

        let first = moka_db::titles::by_mbid(&db, TitleKind::Album, "rg1").await.unwrap().unwrap();
        assert_eq!(first.state(), TitleState::Available, "the studio album, not the live one");
        assert_eq!(first.state_reason.as_deref(), Some("2 of 3 tracks found on disk"));
        let tracks = moka_db::titles::children(&db, first.id).await.unwrap();
        assert_eq!(
            tracks.iter().map(|t| t.state()).collect::<Vec<_>>(),
            [TitleState::Available, TitleState::Available, TitleState::Wanted]
        );
        let file = &moka_db::library::files_for_title(&db, tracks[1].id).await.unwrap()[0];
        assert!(file.path.ends_with("02 - Middle.flac"), "{}", file.path);
        assert_eq!(file.quality.as_deref(), Some("flac"));
        let second = moka_db::titles::by_mbid(&db, TitleKind::Album, "rg2").await.unwrap().unwrap();
        let dusk = moka_db::titles::children(&db, second.id).await.unwrap();
        assert_eq!(
            moka_db::library::files_for_title(&db, dusk[0].id).await.unwrap()[0].quality.as_deref(),
            Some("mp3-320")
        );
        assert_eq!(second.state_reason, None);

        // a second scan finds nothing new
        *state.scan.lock().unwrap() = ScanStatus::default();
        run_albums(&state, &root).await.unwrap();
        let s = state.scan.lock().unwrap().clone();
        assert_eq!((s.added, s.already_known), (0, 2));
        let _ = std::fs::remove_dir_all(&root);
    }
}
