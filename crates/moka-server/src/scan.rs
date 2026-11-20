//! Brings an existing film or show folder into the library: each film folder
//! or file, or each show folder, is recognized (TMDB id in the name, else
//! title + year search) and added as available. Files stay where they are.

use std::path::{Path, PathBuf};
use std::time::Duration;

use moka_core::{TitleKind, TitleState};
use moka_db::titles::NewTitle;
use serde::Serialize;
use utoipa::ToSchema;

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
pub fn start(state: &AppState, root: PathBuf, shows: bool) -> bool {
    {
        let mut s = state.scan.lock().expect("scan lock");
        if s.running {
            return false;
        }
        *s = ScanStatus { running: true, folder: Some(root.display().to_string()), ..Default::default() };
    }
    let state = state.clone();
    tokio::spawn(async move {
        let done = if shows { run_shows(&state, &root).await } else { run(&state, &root).await };
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
}
