//! Brings an existing film folder into the library: each film folder or file
//! is recognized (TMDB id in the name, else title + year search) and added
//! as available. Files stay where they are.

use std::path::{Path, PathBuf};
use std::time::Duration;

use koda_core::{TitleKind, TitleState};
use koda_db::titles::NewTitle;
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
pub fn start(state: &AppState, root: PathBuf) -> bool {
    {
        let mut s = state.scan.lock().expect("scan lock");
        if s.running {
            return false;
        }
        *s = ScanStatus { running: true, folder: Some(root.display().to_string()), ..Default::default() };
    }
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = run(&state, &root).await {
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
async fn add_one(state: &AppState, tmdb: &koda_metadata::Tmdb, f: &Found) -> anyhow::Result<bool> {
    let path = f.video.to_string_lossy().into_owned();
    let parsed = koda_parser::parse_movie(&f.name).ok_or_else(|| anyhow::anyhow!("unreadable name"))?;

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

    if let Some(existing) = koda_db::titles::by_tmdb(&state.db, TitleKind::Movie, tmdb_id).await?
        && koda_db::library::files_for_title(&state.db, existing.id).await?.iter().any(|m| m.path == path)
    {
        return Ok(false);
    }

    let d = tmdb.movie(tmdb_id).await?;
    let title = koda_db::titles::upsert_movie(
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
    koda_db::titles::mark_available(&state.db, title.id).await?;

    let file_name = f.video.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (quality, _) = koda_parser::parse_quality(&file_name);
    let quality = (!quality.is_unknown()).then(|| quality.key());
    let size = tokio::fs::metadata(&f.video).await.map(|m| m.len() as i64).unwrap_or(0);
    koda_db::library::add_file(&state.db, title.id, &path, size, quality.as_deref(), parsed.group.as_deref())
        .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::get};

    #[tokio::test]
    async fn scans_folders_and_loose_files() {
        let root = std::env::temp_dir().join(format!("koda-scan-{}", std::process::id()));
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

        let db = koda_db::memory().await;
        let config = crate::config::Config {
            config_dir: root.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        state.set_tmdb(koda_metadata::Tmdb::with_base_url(&base, "k", "en-US", None));

        run(&state, &root).await.unwrap();
        let s = state.scan.lock().unwrap().clone();
        assert_eq!((s.total, s.added, s.unmatched.clone()), (3, 2, vec!["Nothing Matches 1999".to_owned()]));

        let arrival = koda_db::titles::by_tmdb(&db, TitleKind::Movie, 329865).await.unwrap().unwrap();
        assert_eq!(arrival.state(), TitleState::Available);
        let files = koda_db::library::files_for_title(&db, arrival.id).await.unwrap();
        assert_eq!(files[0].quality.as_deref(), Some("bluray-1080p"));

        // a second scan finds nothing new
        *state.scan.lock().unwrap() = ScanStatus::default();
        run(&state, &root).await.unwrap();
        assert_eq!(state.scan.lock().unwrap().already_known, 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
