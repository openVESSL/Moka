//! Albums in the background loop: which ones to search, and how a finished
//! download is split into track files under `Artist/Album (Year)/`.
//!
//! The album is the unit of search and download, like a film; its tracks are
//! marked available as their files are found.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use moka_core::{AudioQuality, TitleState};
use moka_db::titles::Title;
use moka_decision::{AlbumTarget, AudioProfile};
use moka_indexers::MusicQuery;
use regex::Regex;

use crate::pipeline::{ImportError, LibrarySettings, Query, SearchResult, move_to, now, sanitize};
use crate::state::AppState;

/// Settings key of the music profile.
pub const PROFILE_KEY: &str = "music_profile";
/// Albums searched per pass: indexers have request limits.
const ALBUMS_PER_PASS: i64 = 3;
pub const AUDIO_EXTENSIONS: &[&str] = &["flac", "mp3", "m4a", "ogg", "opus", "wav", "aac"];

pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    for album in moka_db::titles::due_albums(&state.db, ALBUMS_PER_PASS).await? {
        if let Err(e) = crate::pipeline::search_and_grab(state, &album).await {
            tracing::warn!(album = %album.title, error = ?e, "album search failed");
        }
    }
    Ok(())
}

pub async fn profile(state: &AppState) -> anyhow::Result<AudioProfile> {
    Ok(moka_db::settings::get(&state.db, PROFILE_KEY).await?.unwrap_or_default())
}

async fn artist_of(state: &AppState, album: &Title) -> anyhow::Result<Title> {
    let id = album.parent_id.ok_or_else(|| anyhow::anyhow!("The album has no artist"))?;
    moka_db::titles::by_id(&state.db, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("The album's artist was removed"))
}

/// Asks every usable indexer for an album and evaluates what comes back.
pub async fn search_album(state: &AppState, album: &Title) -> anyhow::Result<SearchResult> {
    let artist = artist_of(state, album).await?;
    let query = MusicQuery {
        artist: artist.title.clone(),
        album: album.title.clone(),
        year: album.year.and_then(|y| u16::try_from(y).ok()),
    };
    let (candidates, indexer_errors) = crate::pipeline::gather(state, album.id, &Query::Music(query)).await?;
    let target = AlbumTarget { artist: artist.title, album: album.title.clone() };
    let evaluated = moka_decision::evaluate_album(&target, &profile(state).await?, &candidates);
    let result = SearchResult { at: now(), candidates, evaluated, indexer_errors };
    state.searches.lock().expect("searches lock").insert(album.id, result.clone());
    Ok(result)
}

/// Every audio file in a download, in name order.
fn audio_files(path: &Path) -> Vec<PathBuf> {
    let is_audio = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    };
    if path.is_file() {
        return if is_audio(path) { vec![path.to_path_buf()] } else { vec![] };
    }
    let mut found = Vec::new();
    let mut stack = vec![(path.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                stack.push((p, depth + 1));
            } else if is_audio(&p) {
                found.push(p);
            }
        }
    }
    found.sort();
    found
}

/// "01 - Title", "01. Title", "1-01 Title", "101 Title".
static LEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:(\d{1,2})[-.](\d{1,3})|(\d{1,3}))(?:\s*[-._)]\s*|\s+)").unwrap());
/// "Artist - 01 - Title".
static INNER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s-\s(\d{1,3})(?:\s*[-.]\s*|\s+)").unwrap());
/// A "CD2" or "Disc 2" folder.
static DISC_DIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:cd|disc|disk)\s*0*(\d{1,2})\b").unwrap());

/// Disc (when the name or folder says it) and track number read from a file.
fn track_number(path: &Path) -> (Option<u32>, Option<u32>) {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let folder_disc = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .and_then(|n| DISC_DIR.captures(n))
        .and_then(|c| c[1].parse().ok());
    if let Some(c) = LEAD.captures(stem) {
        if let (Some(d), Some(n)) = (c.get(1), c.get(2)) {
            return (d.as_str().parse().ok(), n.as_str().parse().ok());
        }
        return (folder_disc, c.get(3).and_then(|n| n.as_str().parse().ok()));
    }
    let n = INNER.captures(stem).and_then(|c| c[1].parse().ok());
    (folder_disc, n)
}

/// Pairs audio files with the album's tracks: by number first, then by title.
/// Tracks hold their disc in `season_number` and their position across the
/// album in `episode_number`; releases number tracks per disc.
fn match_tracks<'a>(files: &[PathBuf], tracks: &'a [Title]) -> Vec<(PathBuf, &'a Title)> {
    let mut sorted: Vec<&Title> = tracks.iter().collect();
    sorted.sort_by_key(|t| (t.season_number.unwrap_or(1), t.episode_number.unwrap_or(0)));
    let discs: HashSet<i64> = sorted.iter().map(|t| t.season_number.unwrap_or(1)).collect();
    let multi = discs.len() > 1;
    // (disc, number on the disc, track)
    let mut numbered = Vec::new();
    for t in &sorted {
        let disc = t.season_number.unwrap_or(1);
        let n = numbered.iter().filter(|(d, _, _): &&(i64, u32, &Title)| *d == disc).count() as u32 + 1;
        numbered.push((disc, n, *t));
    }
    let by_number = |disc: Option<u32>, n: u32| -> Option<&'a Title> {
        let on_disc =
            |d: i64, k: u32| numbered.iter().find(|(x, y, _)| *x == d && *y == k).map(|(_, _, t)| *t);
        match disc {
            Some(d) => on_disc(i64::from(d), n),
            None if multi && n >= 100 => on_disc(i64::from(n / 100), n % 100),
            None if multi => sorted.iter().find(|t| t.episode_number == Some(i64::from(n))).copied(),
            None => on_disc(*discs.iter().next().unwrap_or(&1), n),
        }
    };

    let mut used = HashSet::new();
    let mut out = Vec::new();
    let mut unmatched = Vec::new();
    for f in files {
        let found = match track_number(f) {
            (disc, Some(n)) => by_number(disc, n),
            _ => None,
        };
        match found {
            Some(t) if used.insert(t.id) => out.push((f.clone(), t)),
            _ => unmatched.push(f),
        }
    }
    for f in unmatched {
        let name = moka_parser::normalize_title(f.file_stem().and_then(|s| s.to_str()).unwrap_or(""));
        let hits: Vec<&Title> = sorted
            .iter()
            .filter(|t| !used.contains(&t.id))
            .filter(|t| {
                let title = moka_parser::normalize_title(&t.title);
                !title.is_empty() && name.contains(&title)
            })
            .copied()
            .collect();
        if let [t] = hits.as_slice() {
            used.insert(t.id);
            out.push((f.clone(), *t));
        }
    }
    if out.is_empty()
        && let ([f], [t]) = (files, tracks)
    {
        out.push((f.clone(), t));
    }
    out
}

/// `Artist/Album (Year)`
fn album_dir(root: &Path, artist: &Title, album: &Title) -> PathBuf {
    let name = match album.year {
        Some(y) => format!("{} ({y})", album.title),
        None => album.title.clone(),
    };
    root.join(sanitize(&artist.title)).join(sanitize(&name))
}

/// `01 - Title.flac`, or `2-01 - Title.flac` on a multi-disc album.
fn track_file_name(track: &Title, number: u32, disc: Option<i64>, ext: &str) -> String {
    let n = match disc {
        Some(d) => format!("{d}-{number:02}"),
        None => format!("{number:02}"),
    };
    format!("{}.{ext}", sanitize(&format!("{n} - {}", track.title)))
}

/// Imports a finished album download: audio files are matched to tracks,
/// placed under the artist's folder with the cover, and found tracks become
/// available. Tracks the download lacks are named in the album's reason.
pub async fn import(state: &AppState, album: &Title, g: &moka_db::grabs::Grab) -> Result<(), ImportError> {
    let db = &state.db;
    let err = |e: moka_db::Error| ImportError::Wait(e.to_string());
    let lib: LibrarySettings =
        moka_db::settings::get(db, LibrarySettings::KEY).await.map_err(err)?.unwrap_or_default();
    if lib.music_path.trim().is_empty() {
        return Err(ImportError::Wait("Choose a music folder in Settings to finish importing".into()));
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
    let artist = artist_of(state, album).await.map_err(|e| ImportError::Failed(e.to_string()))?;
    let tracks: Vec<Title> = moka_db::titles::children(db, album.id)
        .await
        .map_err(err)?
        .into_iter()
        .filter(|t| t.kind == "track")
        .collect();
    if tracks.is_empty() {
        return Err(ImportError::Failed("The album has no tracks to match the files with".into()));
    }

    let src = source.clone();
    let files = tokio::task::spawn_blocking(move || audio_files(&src))
        .await
        .map_err(|e| ImportError::Wait(e.to_string()))?;
    let matched = match_tracks(&files, &tracks);
    if matched.is_empty() {
        return Err(ImportError::Failed("No audio file in the download matches the album's tracks".into()));
    }

    let dir = album_dir(Path::new(lib.music_path.trim()), &artist, album);
    let multi = tracks.iter().map(|t| t.season_number.unwrap_or(1)).collect::<HashSet<_>>().len() > 1;
    let group = moka_parser::parse_album(&g.release_title).and_then(|p| p.group);
    for (file, track) in &matched {
        let disc = track.season_number.unwrap_or(1);
        let number = tracks
            .iter()
            .filter(|t| t.season_number.unwrap_or(1) == disc && t.episode_number <= track.episode_number)
            .count() as u32;
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("flac").to_ascii_lowercase();
        let dest = dir.join(track_file_name(track, number, multi.then_some(disc), &ext));
        let (f, d, dd) = (file.clone(), dest.clone(), dir.clone());
        let size = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
            std::fs::create_dir_all(&dd)?;
            crate::pipeline::place(&f, &d)?;
            Ok(std::fs::metadata(&d)?.len())
        })
        .await
        .map_err(|e| ImportError::Wait(e.to_string()))?
        .map_err(|e| ImportError::Wait(format!("Could not write to the music folder: {e}")))?;
        moka_db::library::add_file(
            db,
            track.id,
            &dest.to_string_lossy(),
            size as i64,
            g.quality.as_deref(),
            group.as_deref(),
        )
        .await
        .map_err(err)?;
        moka_db::titles::mark_available(db, track.id).await.map_err(err)?;
    }
    save_cover(state, album, &dir).await;

    moka_db::grabs::set_status(db, g.id, "imported", None, None).await.map_err(err)?;
    let missing = tracks.len() - matched.len();
    let reason =
        (missing > 0).then(|| format!("{} of {} tracks found in the download", matched.len(), tracks.len()));
    move_to(state, album.id, TitleState::Available, reason.as_deref()).await?;
    let detail =
        g.quality.as_deref().and_then(|q| q.parse::<AudioQuality>().ok()).map(|q| q.label().to_owned());
    crate::notify::send(
        state,
        crate::notify::Event::Available,
        Some(crate::notify::TitleInfo::from_title(album, detail)),
        None,
    );
    tracing::info!(album = %album.title, tracks = matched.len(), path = %dir.display(), "imported");
    crate::jellyfin::announce_import(state, dir.to_string_lossy().into_owned());
    Ok(())
}

/// Puts `cover.jpg` next to the tracks, from the cover cache (fetched if
/// missing). Players read it; a missing cover is not an error.
async fn save_cover(state: &AppState, album: &Title, dir: &Path) {
    let dest = dir.join("cover.jpg");
    let Some(mbid) = album.mbid.as_deref() else { return };
    if dest.exists() {
        return;
    }
    let cached = state.config.cache_dir().join("images").join("covers").join(format!("{mbid}.jpg"));
    let bytes = match tokio::fs::read(&cached).await {
        Ok(b) => Some(b),
        // tests never reach the Cover Art Archive
        Err(_) if cfg!(test) => None,
        Err(_) => crate::api::images::cover_bytes(state, mbid).await.ok().flatten(),
    };
    if let Some(b) = bytes
        && let Err(e) = tokio::fs::write(&dest, b).await
    {
        tracing::warn!(album = %album.title, error = %e, "could not save the cover");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: i64, disc: i64, pos: i64, name: &str) -> Title {
        Title {
            id,
            kind: "track".into(),
            parent_id: Some(1),
            mbid: None,
            season_number: Some(disc),
            episode_number: Some(pos),
            tmdb_id: None,
            imdb_id: None,
            title: name.into(),
            original_title: None,
            year: None,
            overview: None,
            runtime_minutes: None,
            release_date: None,
            poster_path: None,
            backdrop_path: None,
            state: "wanted".into(),
            state_reason: None,
            quality_profile_id: None,
            monitored: true,
            next_search_at: None,
            last_search_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn numbers_from_file_names() {
        let n = |p: &str| track_number(Path::new(p));
        assert_eq!(n("/dl/01 - Airbag.flac"), (None, Some(1)));
        assert_eq!(n("/dl/02. Paranoid Android.flac"), (None, Some(2)));
        assert_eq!(n("/dl/1-03 Subterranean.flac"), (Some(1), Some(3)));
        assert_eq!(n("/dl/104 Exit Music.mp3"), (None, Some(104)));
        assert_eq!(n("/dl/CD2/05 Let Down.flac"), (Some(2), Some(5)));
        assert_eq!(n("/dl/Radiohead - 06 - Karma Police.mp3"), (None, Some(6)));
        assert_eq!(n("/dl/Karma Police.mp3"), (None, None));
    }

    #[test]
    fn files_pair_with_tracks() {
        let single =
            vec![track(10, 1, 1, "Airbag"), track(11, 1, 2, "Paranoid Android"), track(12, 1, 3, "Lucky")];
        let files: Vec<PathBuf> =
            ["/dl/02 - Paranoid Android.flac", "/dl/Airbag.flac", "/dl/folder.jpg.flac", "/dl/01 - dup.flac"]
                .iter()
                .map(PathBuf::from)
                .collect();
        let ids: Vec<(String, i64)> = match_tracks(&files, &single)
            .into_iter()
            .map(|(f, t)| (f.file_name().unwrap().to_string_lossy().into_owned(), t.id))
            .collect();
        assert_eq!(
            ids,
            [("02 - Paranoid Android.flac".into(), 11), ("01 - dup.flac".into(), 10)],
            "by number first; a title already taken is not matched twice"
        );

        let double = vec![track(20, 1, 1, "One"), track(21, 1, 2, "Two"), track(22, 2, 3, "Three")];
        let files: Vec<PathBuf> = ["/dl/CD2/01 Three.flac", "/dl/102 Two.flac", "/dl/1-01 One.flac"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let ids: Vec<i64> = match_tracks(&files, &double).into_iter().map(|(_, t)| t.id).collect();
        assert_eq!(ids, [22, 21, 20], "disc from the folder, from 102 and from 1-01");

        let dir = album_dir(
            Path::new("/music"),
            &track(1, 1, 1, "AC/DC"),
            &Title { year: Some(1980), ..track(2, 1, 1, "Back in Black") },
        );
        assert_eq!(dir, Path::new("/music/AC DC/Back in Black (1980)"));
        assert_eq!(track_file_name(&double[2], 1, Some(2), "flac"), "2-01 - Three.flac");
        assert_eq!(track_file_name(&double[0], 1, None, "mp3"), "01 - One.mp3");
    }
}

/// A wanted album through a fake indexer and qBittorrent into the music folder.
#[cfg(test)]
mod flow {
    use super::*;
    use axum::{Router, extract::Query, routing::get, routing::post};
    use std::collections::HashMap;

    const HASH: &str = "e12fe1c06bba254a9dc9f519b335aa7c1367a88c";

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn wanted_album_arrives_as_tracks() {
        let root = std::env::temp_dir().join(format!("moka-music-{}-{}", std::process::id(), now()));
        let download = root.join("downloads/Test Band - First Light (2001) [FLAC]");
        std::fs::create_dir_all(&download).unwrap();
        std::fs::write(download.join("01 - Opening.flac"), vec![1u8; 1024]).unwrap();
        std::fs::write(download.join("02 - Middle.flac"), vec![2u8; 1024]).unwrap();
        std::fs::write(download.join("album.log"), b"rip log").unwrap();

        let indexer = serve(Router::new().route(
            "/api",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                match q.get("t").map(String::as_str) {
                    Some("caps") => r#"<caps><searching><music-search available="yes" supportedParams="q,artist,album"/></searching></caps>"#.to_owned(),
                    _ => {
                        assert_eq!(q.get("t").map(String::as_str), Some("music"));
                        assert_eq!(q.get("artist").map(String::as_str), Some("Test Band"));
                        assert_eq!(q.get("cat").map(String::as_str), Some("3000"));
                        format!(r#"<rss><channel>
                        <item><title>Test Band - First Light (2001) [FLAC]</title>
                          <link>magnet:?xt=urn:btih:{HASH}</link><size>300000000</size>
                          <attr name="seeders" value="8"/></item>
                        <item><title>Test Band - First Light (2001) [MP3 128]</title>
                          <link>magnet:?xt=urn:btih:ffffffffffffffffffffffffffffffffffffffff</link><size>60000000</size>
                          <attr name="seeders" value="90"/></item>
                        <item><title>Test Band - Second Dark (2004) [FLAC]</title>
                          <link>magnet:?xt=urn:btih:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee</link><size>300000000</size>
                          <attr name="seeders" value="90"/></item>
                      </channel></rss>"#)
                    }
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
                            axum::Json(serde_json::json!([{ "hash": HASH.to_uppercase(), "name": "Test Band",
                                "state": "stalledUP", "progress": 1.0, "size": 2048,
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
        let album = moka_db::titles::NewAlbum {
            artist_mbid: "artist-1".into(),
            artist: "Test Band".into(),
            mbid: "album-1".into(),
            title: "First Light".into(),
            year: Some(2001),
            tracks: ["Opening", "Middle", "Closing"]
                .iter()
                .zip(1..)
                .map(|(t, n)| moka_db::titles::NewTrack {
                    title: (*t).into(),
                    disc: 1,
                    number: n,
                    length_seconds: Some(200),
                })
                .collect(),
            ..Default::default()
        };
        let a = moka_db::titles::upsert_album(&db, &album, TitleState::Wanted).await.unwrap();
        let lib = LibrarySettings {
            music_path: root.join("music").to_string_lossy().into_owned(),
            ..Default::default()
        };
        moka_db::settings::set(&db, LibrarySettings::KEY, &lib).await.unwrap();

        // pass 1: FLAC wins over the 128 kbps copy and the other album
        crate::pipeline::tick_for_tests(&state).await;
        let found = state.searches.lock().unwrap().get(&a.id).cloned().unwrap();
        let ok: Vec<&str> =
            found.evaluated.iter().filter(|e| e.accepted()).map(|e| e.title.as_str()).collect();
        assert_eq!(ok, ["Test Band - First Light (2001) [FLAC]"]);
        let state_of = || async { moka_db::titles::by_id(&db, a.id).await.unwrap().unwrap() };
        assert_eq!(state_of().await.state(), TitleState::Grabbed);

        // pass 2: complete; pass 3: imported track by track
        crate::pipeline::tick_for_tests(&state).await;
        crate::pipeline::tick_for_tests(&state).await;
        let done = state_of().await;
        assert_eq!(done.state(), TitleState::Available);
        assert_eq!(done.state_reason.as_deref(), Some("2 of 3 tracks found in the download"));
        let tracks = moka_db::titles::children(&db, a.id).await.unwrap();
        let states: Vec<TitleState> = tracks.iter().map(|t| t.state()).collect();
        assert_eq!(states, [TitleState::Available, TitleState::Available, TitleState::Wanted]);
        let file = &moka_db::library::files_for_title(&db, tracks[1].id).await.unwrap()[0];
        assert!(
            Path::new(&file.path).ends_with("Test Band/First Light (2001)/02 - Middle.flac"),
            "{}",
            file.path
        );
        assert_eq!(file.quality.as_deref(), Some("flac"));
        assert_eq!(std::fs::read(&file.path).unwrap(), vec![2u8; 1024]);
        let listed = moka_db::library::album_tracks(&db, a.id).await.unwrap();
        assert_eq!(listed.iter().map(|t| t.file_id.is_some()).collect::<Vec<_>>(), [true, true, false]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
