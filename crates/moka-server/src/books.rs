//! Ebooks and audiobooks in the background loop: which ones to search, and
//! how a finished download is placed under `Author/Title (Year)/`.
//!
//! An ebook keeps one file, the best format the download holds. An
//! audiobook keeps every audio file, in order, under their own names.

use std::path::{Path, PathBuf};

use moka_core::{EbookFormat, TitleState};
use moka_db::titles::Title;
use moka_decision::{BookProfile, BookTarget};
use moka_indexers::BookQuery;

use crate::pipeline::{ImportError, LibrarySettings, Query, SearchResult, move_to, now, sanitize};
use crate::state::AppState;

/// Settings key of the ebook profile.
pub const PROFILE_KEY: &str = "book_profile";
/// Books searched per pass: indexers have request limits.
const BOOKS_PER_PASS: i64 = 3;
pub const EBOOK_EXTENSIONS: &[&str] = &["epub", "kepub", "azw3", "azw", "kf8", "mobi", "prc", "pdf"];
pub const AUDIOBOOK_EXTENSIONS: &[&str] = &["m4b", "m4a", "mp3", "aac", "ogg", "opus", "flac", "wav"];

pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    for book in moka_db::titles::due_books(&state.db, BOOKS_PER_PASS).await? {
        if let Err(e) = crate::pipeline::search_and_grab(state, &book).await {
            tracing::warn!(book = %book.title, error = ?e, "book search failed");
        }
    }
    Ok(())
}

pub async fn profile(state: &AppState) -> anyhow::Result<BookProfile> {
    Ok(moka_db::settings::get(&state.db, PROFILE_KEY).await?.unwrap_or_default())
}

async fn author_of(state: &AppState, book: &Title) -> anyhow::Result<Title> {
    let id = book.parent_id.ok_or_else(|| anyhow::anyhow!("The book has no author"))?;
    moka_db::titles::by_id(&state.db, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("The book's author was removed"))
}

/// Asks every usable indexer for a book and evaluates what comes back.
pub async fn search_book(state: &AppState, book: &Title) -> anyhow::Result<SearchResult> {
    let author = author_of(state, book).await?;
    let audiobook = book.kind == "audiobook";
    let query = BookQuery { author: author.title.clone(), title: book.title.clone(), audiobook };
    let (candidates, indexer_errors) = crate::pipeline::gather(state, book.id, &Query::Book(query)).await?;
    let target = BookTarget { author: author.title, title: book.title.clone(), audiobook };
    let evaluated = moka_decision::evaluate_book(&target, &profile(state).await?, &candidates);
    let result = SearchResult { at: now(), candidates, evaluated, indexer_errors };
    state.searches.lock().expect("searches lock").insert(book.id, result.clone());
    Ok(result)
}

/// `Author/Title (Year)`
fn book_dir(root: &Path, author: &Title, book: &Title) -> PathBuf {
    let name = match book.year {
        Some(y) => format!("{} ({y})", book.title),
        None => book.title.clone(),
    };
    root.join(sanitize(&author.title)).join(sanitize(&name))
}

fn extension(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// The ebook file to keep: the best format in the profile, else the best
/// format there is; the largest file breaks ties (samples are small).
fn best_ebook(files: &[(PathBuf, u64)], profile: &BookProfile) -> Option<PathBuf> {
    files
        .iter()
        .min_by_key(|(p, size)| {
            let f = EbookFormat::from_extension(&extension(p));
            (profile.rank_of(f).unwrap_or(usize::MAX), std::cmp::Reverse(f), std::cmp::Reverse(*size))
        })
        .map(|(p, _)| p.clone())
}

/// Copies or links files into `dir`, each under the name given, and returns
/// where each one went with its size.
async fn place_all(files: Vec<(PathBuf, String)>, dir: PathBuf) -> Result<Vec<(PathBuf, u64)>, ImportError> {
    tokio::task::spawn_blocking(move || -> std::io::Result<Vec<(PathBuf, u64)>> {
        std::fs::create_dir_all(&dir)?;
        let mut out = Vec::new();
        for (src, name) in files {
            let dest = dir.join(name);
            crate::pipeline::place(&src, &dest)?;
            let size = std::fs::metadata(&dest)?.len();
            out.push((dest, size));
        }
        Ok(out)
    })
    .await
    .map_err(|e| ImportError::Wait(e.to_string()))?
    .map_err(|e| ImportError::Wait(format!("Could not write to the books folder: {e}")))
}

/// Imports a finished book download into the ebooks or audiobooks folder.
pub async fn import(state: &AppState, book: &Title, g: &moka_db::grabs::Grab) -> Result<(), ImportError> {
    let db = &state.db;
    let err = |e: moka_db::Error| ImportError::Wait(e.to_string());
    let audiobook = book.kind == "audiobook";
    let lib: LibrarySettings =
        moka_db::settings::get(db, LibrarySettings::KEY).await.map_err(err)?.unwrap_or_default();
    let root = if audiobook { lib.audiobooks_path } else { lib.books_path };
    if root.trim().is_empty() {
        let which = if audiobook { "an audiobooks" } else { "an ebooks" };
        return Err(ImportError::Wait(format!("Choose {which} folder in Settings to finish importing")));
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
    let author = author_of(state, book).await.map_err(|e| ImportError::Failed(e.to_string()))?;
    let dir = book_dir(Path::new(root.trim()), &author, book);

    let src = source.clone();
    let exts = if audiobook { AUDIOBOOK_EXTENSIONS } else { EBOOK_EXTENSIONS };
    let found: Vec<(PathBuf, u64)> = tokio::task::spawn_blocking(move || {
        crate::music::files_with(&src, exts)
            .into_iter()
            .map(|p| {
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                (p, size)
            })
            .collect()
    })
    .await
    .map_err(|e| ImportError::Wait(e.to_string()))?;

    let to_place: Vec<(PathBuf, String)> = if audiobook {
        if found.is_empty() {
            return Err(ImportError::Failed("No audio file in the download".into()));
        }
        match found.as_slice() {
            [(only, _)] => vec![(only.clone(), sanitize(&format!("{}.{}", book.title, extension(only))))],
            many => many
                .iter()
                .map(|(p, _)| {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    (p.clone(), sanitize(&name))
                })
                .collect(),
        }
    } else {
        let profile = profile(state).await.map_err(|e| ImportError::Wait(e.to_string()))?;
        let best = best_ebook(&found, &profile)
            .ok_or_else(|| ImportError::Failed("No ebook file in the download".into()))?;
        let name = sanitize(&format!("{}.{}", book.title, extension(&best)));
        vec![(best, name)]
    };

    let placed = place_all(to_place, dir.clone()).await?;
    let group = moka_parser::parse_book(&g.release_title).group;
    for (dest, size) in &placed {
        let quality = if audiobook {
            g.quality.clone()
        } else {
            Some(EbookFormat::from_extension(&extension(dest)).to_string())
        };
        moka_db::library::add_file(
            db,
            book.id,
            &dest.to_string_lossy(),
            *size as i64,
            quality.as_deref(),
            group.as_deref(),
        )
        .await
        .map_err(err)?;
    }
    save_cover(state, book, &dir).await;

    moka_db::grabs::set_status(db, g.id, "imported", None, None).await.map_err(err)?;
    move_to(state, book.id, TitleState::Available, None).await?;
    crate::notify::send(
        state,
        crate::notify::Event::Available,
        Some(crate::notify::TitleInfo::from_title(book, None)),
        None,
    );
    tracing::info!(book = %book.title, files = placed.len(), path = %dir.display(), "imported");
    Ok(())
}

/// Puts `cover.jpg` next to the book, as readers and players expect. A
/// missing cover is not an error.
async fn save_cover(state: &AppState, book: &Title, dir: &Path) {
    let dest = dir.join("cover.jpg");
    let Some(id) = book.poster_path.as_deref().and_then(|p| p.parse::<u64>().ok()) else { return };
    // tests never reach Open Library
    if dest.exists() || cfg!(test) {
        return;
    }
    if let Ok(Some(bytes)) = crate::api::images::open_library_bytes(state, "b", id).await
        && let Err(e) = tokio::fs::write(&dest, bytes).await
    {
        tracing::warn!(book = %book.title, error = %e, "could not save the cover");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_best_ebook_format() {
        let files = |names: &[(&str, u64)]| -> Vec<(PathBuf, u64)> {
            names.iter().map(|(n, s)| (PathBuf::from(n), *s)).collect()
        };
        let profile = BookProfile::default();
        let pick = |f: &[(&str, u64)]| best_ebook(&files(f), &profile).map(|p| p.display().to_string());
        assert_eq!(
            pick(&[("/d/a.mobi", 9), ("/d/a.epub", 5), ("/d/a.pdf", 50)]).as_deref(),
            Some("/d/a.epub")
        );
        assert_eq!(pick(&[("/d/sample.epub", 1), ("/d/book.epub", 5)]).as_deref(), Some("/d/book.epub"));
        assert_eq!(
            pick(&[("/d/scan.pdf", 5)]).as_deref(),
            Some("/d/scan.pdf"),
            "outside the profile, still a book"
        );
        assert_eq!(pick(&[]), None);
    }
}

/// A wanted ebook through a fake indexer and qBittorrent into the books folder.
#[cfg(test)]
mod flow {
    use super::*;
    use axum::{Router, extract::Query, routing::get, routing::post};
    use moka_core::TitleKind;
    use std::collections::HashMap;

    const HASH: &str = "b12fe1c06bba254a9dc9f519b335aa7c1367a88c";

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn wanted_ebook_arrives_as_one_file() {
        let root = std::env::temp_dir().join(format!("moka-books-{}-{}", std::process::id(), now()));
        let download = root.join("downloads/Frank Herbert - Dune (epub, mobi)");
        std::fs::create_dir_all(&download).unwrap();
        std::fs::write(download.join("Dune - Frank Herbert.mobi"), vec![1u8; 2048]).unwrap();
        std::fs::write(download.join("Dune - Frank Herbert.epub"), vec![2u8; 1024]).unwrap();
        std::fs::write(download.join("info.nfo"), b"nfo").unwrap();

        let indexer = serve(Router::new().route(
            "/api",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                match q.get("t").map(String::as_str) {
                    Some("caps") => r#"<caps><searching><book-search available="yes" supportedParams="q,author,title"/></searching></caps>"#.to_owned(),
                    _ => {
                        assert_eq!(q.get("t").map(String::as_str), Some("book"));
                        assert_eq!(q.get("author").map(String::as_str), Some("Frank Herbert"));
                        assert_eq!(q.get("cat").map(String::as_str), Some("7000"));
                        format!(r#"<rss><channel>
                        <item><title>Frank Herbert - Dune (epub, mobi)</title>
                          <link>magnet:?xt=urn:btih:{HASH}</link><size>3000000</size>
                          <attr name="seeders" value="8"/></item>
                        <item><title>Frank Herbert - Dune (pdf)</title>
                          <link>magnet:?xt=urn:btih:ffffffffffffffffffffffffffffffffffffffff</link><size>9000000</size>
                          <attr name="seeders" value="90"/></item>
                        <item><title>Frank Herbert - Dune Messiah (epub)</title>
                          <link>magnet:?xt=urn:btih:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee</link><size>3000000</size>
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
                            axum::Json(serde_json::json!([{ "hash": HASH.to_uppercase(), "name": "Dune",
                                "state": "stalledUP", "progress": 1.0, "size": 3072,
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
        let book = moka_db::titles::NewBook {
            author_olid: "OL79034A".into(),
            author: "Frank Herbert".into(),
            olid: "OL893414W".into(),
            title: "Dune".into(),
            year: Some(1965),
            poster_path: Some("11481354".into()),
            ..Default::default()
        };
        let b = moka_db::titles::upsert_book(&db, &book, TitleKind::Ebook, TitleState::Wanted).await.unwrap();
        let lib = LibrarySettings {
            books_path: root.join("books").to_string_lossy().into_owned(),
            ..Default::default()
        };
        moka_db::settings::set(&db, LibrarySettings::KEY, &lib).await.unwrap();

        // pass 1: the EPUB pack wins over the PDF and the sequel
        crate::pipeline::tick_for_tests(&state).await;
        let found = state.searches.lock().unwrap().get(&b.id).cloned().unwrap();
        let ok: Vec<&str> =
            found.evaluated.iter().filter(|e| e.accepted()).map(|e| e.title.as_str()).collect();
        assert_eq!(ok, ["Frank Herbert - Dune (epub, mobi)"]);
        let state_of = || async { moka_db::titles::by_id(&db, b.id).await.unwrap().unwrap() };
        assert_eq!(state_of().await.state(), TitleState::Grabbed);

        // pass 2: complete; pass 3: imported, the EPUB only
        crate::pipeline::tick_for_tests(&state).await;
        crate::pipeline::tick_for_tests(&state).await;
        assert_eq!(state_of().await.state(), TitleState::Available);
        let files = moka_db::library::files_for_title(&db, b.id).await.unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            Path::new(&files[0].path).ends_with("Frank Herbert/Dune (1965)/Dune.epub"),
            "{}",
            files[0].path
        );
        assert_eq!(files[0].quality.as_deref(), Some("epub"));
        assert_eq!(std::fs::read(&files[0].path).unwrap(), vec![2u8; 1024]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
