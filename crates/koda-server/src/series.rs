//! Shows in the background loop: which episodes to search, as a season pack
//! or one by one, and how a finished download is split into episode files.
//!
//! A grab belongs to one title (an episode, or the season for a pack) and
//! lists the other episodes it holds; `pipeline` moves them all together.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use koda_core::{Quality, TitleState};
use koda_db::titles::Title;
use koda_decision::{EpisodeTarget, Target};
use koda_indexers::TvQuery;

use crate::pipeline::{ImportError, LibrarySettings, Query, move_to, now, retry_delay, sanitize};
use crate::state::AppState;

/// Seasons searched per pass, like films: indexers have request limits.
const SEASONS_PER_PASS: usize = 3;

/// Runs forever: refreshes every show from the catalog twice a day, so new
/// episodes and air dates arrive and get searched.
pub async fn run(state: AppState) {
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    loop {
        let shows = koda_db::titles::shows(&state.db).await.unwrap_or_default();
        for show in shows.iter().filter(|s| s.state() != TitleState::Requested) {
            let Some(tmdb_id) = show.tmdb_id else { continue };
            let fresh = match crate::api::catalog::new_show(&state, tmdb_id).await {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(show = %show.title, error = %e, "show refresh failed");
                    continue;
                }
            };
            if let Err(e) = koda_db::titles::upsert_show(&state.db, &fresh, TitleState::Wanted).await {
                tracing::warn!(show = %show.title, error = %e, "show refresh failed");
            }
        }
        state.wake.notify_one();
        tokio::time::sleep(std::time::Duration::from_secs(12 * 3600)).await;
    }
}

pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    let due = koda_db::titles::due_episodes(&state.db, 50).await?;
    let mut seen = HashSet::new();
    for ep in due {
        let Some(season_id) = ep.parent_id else { continue };
        if !seen.insert(season_id) {
            continue;
        }
        if seen.len() > SEASONS_PER_PASS {
            break;
        }
        if let Err(e) = search_season(state, season_id, &ep).await {
            tracing::warn!(title = %ep.title, error = ?e, "episode search failed");
        }
    }
    Ok(())
}

/// (season, episode, absolute number) of every regular episode of a show,
/// counted from the first episode of season 1, as anime releases number them.
fn absolute_numbers(all: &[Title]) -> Vec<(i64, i64, u32)> {
    let mut regular: Vec<(i64, i64)> = all
        .iter()
        .filter_map(|t| Some((t.season_number?, t.episode_number?)))
        .filter(|(s, _)| *s > 0)
        .collect();
    regular.sort();
    regular.into_iter().zip(1..).map(|((s, e), n)| (s, e, n)).collect()
}

/// (episode, air date) and (absolute number, episode) pairs of a season.
type SeasonNumbers = (Vec<(u32, String)>, Vec<(u32, u32)>);

/// Air dates and absolute numbers of a season's episodes, for the decision.
fn season_numbers(all: &[Title], season: i64) -> SeasonNumbers {
    let dates = all
        .iter()
        .filter(|t| t.season_number == Some(season))
        .filter_map(|t| Some((t.episode_number? as u32, t.release_date.clone()?)))
        .collect();
    let absolute = absolute_numbers(all)
        .into_iter()
        .filter(|(s, _, _)| *s == season)
        .map(|(_, e, n)| (n, e as u32))
        .collect();
    (dates, absolute)
}

/// Daily shows are searched by air date, anime by absolute number too.
fn number_query(query: &mut TvQuery, numbering: &str, ep: &Title, all: &[Title]) {
    match numbering {
        "daily" => query.air_date = ep.release_date.clone(),
        "anime" => {
            query.absolute = absolute_numbers(all)
                .into_iter()
                .find(|(s, e, _)| Some(*s) == ep.season_number && Some(*e) == ep.episode_number)
                .map(|(_, _, n)| n)
        }
        _ => {}
    }
}

fn aired(t: &Title) -> bool {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    t.release_date.as_deref().is_some_and(|d| d <= today.as_str())
}

/// One search for a season: the whole season as a pack when every episode
/// has aired and none is in the library yet, else the first due episode.
async fn search_season(state: &AppState, season_id: i64, first: &Title) -> anyhow::Result<()> {
    let db = &state.db;
    let Some(season) = koda_db::titles::by_id(db, season_id).await? else { return Ok(()) };
    let Some(show) = koda_db::titles::by_id(db, season.parent_id.unwrap_or(0)).await? else { return Ok(()) };
    let episodes = koda_db::titles::children(db, season_id).await?;
    let monitored: Vec<&Title> = episodes.iter().filter(|e| e.monitored).collect();
    let wanted: Vec<&Title> =
        monitored.iter().copied().filter(|e| e.state() == TitleState::Wanted && aired(e)).collect();
    let number = season.season_number.unwrap_or(0) as u32;
    let all = koda_db::titles::episodes_of_show(db, show.id).await?;
    let numbering = koda_db::titles::show_numbering(db, show.id).await?;
    let (air_dates, absolute) = season_numbers(&all, i64::from(number));
    let mut target = Target {
        title: show.title.clone(),
        year: show.year.and_then(|y| u16::try_from(y).ok()),
        imdb_id: show.imdb_id.clone(),
        tmdb_id: show.tmdb_id,
        runtime_minutes: show.runtime_minutes.and_then(|m| u32::try_from(m).ok()),
        episodes: Some(EpisodeTarget {
            season: number,
            episodes: vec![],
            season_episodes: episodes.len() as u32,
            air_dates,
            absolute,
        }),
        ..Default::default()
    };
    let tvdb_id = koda_db::titles::tvdb_id(db, show.id).await?;
    let mut query = TvQuery {
        title: show.title.clone(),
        tvdb_id,
        tmdb_id: show.tmdb_id,
        imdb_id: show.imdb_id.clone(),
        season: number,
        episode: None,
        ..Default::default()
    };

    // a daily show's "season" is a year of episodes: never searched as one pack
    let whole = numbering != "daily"
        && wanted.len() > 1
        && wanted.len() == monitored.len()
        && season.state() == TitleState::Wanted
        && season.next_search_at.is_none_or(|t| t <= now());
    if whole {
        let ids: Vec<i64> = wanted.iter().map(|e| e.id).collect();
        if try_unit(state, &season, &show, Query::Tv(query.clone()), target.clone(), &ids, &episodes).await? {
            return Ok(());
        }
    }

    // one episode; a release may still hold a few wanted ones together
    query.episode = first.episode_number.map(|n| n as u32);
    number_query(&mut query, &numbering, first, &all);
    if let Some(e) = target.episodes.as_mut() {
        e.episodes = wanted.iter().filter_map(|w| w.episode_number.map(|n| n as u32)).collect();
    }
    try_unit(state, first, &show, Query::Tv(query), target, &[first.id], &episodes).await?;
    Ok(())
}

/// Searches for one episode by hand (nothing is grabbed): only releases
/// holding this episode alone are acceptable.
pub async fn search_episode(state: &AppState, ep: &Title) -> anyhow::Result<crate::pipeline::SearchResult> {
    let db = &state.db;
    let season = koda_db::titles::by_id(db, ep.parent_id.unwrap_or(0)).await?;
    let show_id = season.as_ref().and_then(|s| s.parent_id).unwrap_or(0);
    let show =
        koda_db::titles::by_id(db, show_id).await?.ok_or_else(|| anyhow::anyhow!("The show was removed"))?;
    let siblings = koda_db::titles::children(db, ep.parent_id.unwrap_or(0)).await?.len() as u32;
    let (number, episode) = (ep.season_number.unwrap_or(0) as u32, ep.episode_number.unwrap_or(0) as u32);
    let all = koda_db::titles::episodes_of_show(db, show.id).await?;
    let (air_dates, absolute) = season_numbers(&all, i64::from(number));
    let files = koda_db::library::files_for_title(db, ep.id).await?;
    let target = Target {
        title: show.title.clone(),
        year: show.year.and_then(|y| u16::try_from(y).ok()),
        imdb_id: show.imdb_id.clone(),
        tmdb_id: show.tmdb_id,
        runtime_minutes: ep.runtime_minutes.or(show.runtime_minutes).and_then(|m| u32::try_from(m).ok()),
        existing: files.first().and_then(|f| f.quality.as_deref()).and_then(Quality::from_key),
        episodes: Some(EpisodeTarget {
            season: number,
            episodes: vec![episode],
            season_episodes: siblings,
            air_dates,
            absolute,
        }),
        ..Default::default()
    };
    let mut query = TvQuery {
        title: show.title.clone(),
        tvdb_id: koda_db::titles::tvdb_id(db, show.id).await?,
        tmdb_id: show.tmdb_id,
        imdb_id: show.imdb_id.clone(),
        season: number,
        episode: Some(episode),
        ..Default::default()
    };
    number_query(&mut query, &koda_db::titles::show_numbering(db, show.id).await?, ep, &all);
    crate::pipeline::search_with(state, ep.id, show.quality_profile_id, Query::Tv(query), target).await
}

/// Searches for `unit` (a season or an episode) and grabs the best release.
/// Returns false when nothing fitted; the unit is then scheduled to retry.
async fn try_unit(
    state: &AppState,
    unit: &Title,
    show: &Title,
    query: Query,
    target: Target,
    wanted: &[i64],
    episodes: &[Title],
) -> anyhow::Result<bool> {
    move_to(state, unit.id, TitleState::Searching, None).await?;
    let attempts = koda_db::titles::search_attempts(&state.db, unit.id).await.unwrap_or(0);
    let outcome = async {
        let res =
            crate::pipeline::search_with(state, unit.id, show.quality_profile_id, query, target).await?;
        let Some(best) = res.evaluated.iter().find(|e| e.accepted()).cloned() else {
            let found = res.candidates.len();
            return Ok(Some(if found == 0 {
                "No releases found yet".to_owned()
            } else {
                format!("{found} releases found, none fits the quality profile")
            }));
        };
        let covered: Vec<i64> = episodes
            .iter()
            .filter(|e| e.episode_number.is_some_and(|n| best.covers.contains(&(n as u32))))
            .map(|e| e.id)
            .filter(|id| wanted.contains(id) || unit.kind == "season")
            .collect();
        let cand = res.candidates[best.candidate_index].clone();
        crate::pipeline::grab_covering(state, unit.id, &covered, &cand, best.quality).await.map(|_| None)
    }
    .await;
    let reason = match outcome {
        Ok(None) => return Ok(true),
        Ok(Some(msg)) => msg,
        Err(e) => format!("{e}"),
    };
    move_to(state, unit.id, TitleState::Wanted, Some(&reason)).await?;
    koda_db::titles::schedule_search(&state.db, unit.id, now() + retry_delay(attempts), Some(&reason))
        .await?;
    Ok(false)
}

/// `Show (Year) {tmdb-ID}/Season 01/Show - S01E02 - Name - Quality.ext`
pub fn episode_path(
    root: &Path,
    show: &Title,
    season: i64,
    eps: &[&Title],
    quality: Option<Quality>,
    ext: &str,
) -> PathBuf {
    let numbers = match eps {
        [one] => format!("S{season:02}E{:02}", one.episode_number.unwrap_or(0)),
        [first, .., last] => format!(
            "S{season:02}E{:02}-E{:02}",
            first.episode_number.unwrap_or(0),
            last.episode_number.unwrap_or(0)
        ),
        [] => format!("S{season:02}"),
    };
    let mut name = format!("{} - {numbers}", show.title);
    if let [one] = eps {
        name.push_str(&format!(" - {}", one.title));
    }
    if let Some(q) = quality {
        name.push_str(&format!(" - {q}"));
    }
    root.join(crate::pipeline::library_folder_name(show))
        .join(format!("Season {season:02}"))
        .join(format!("{}.{ext}", sanitize(&name)))
}

/// Imports a finished episode or season download: every video file is
/// matched to an episode by its name, placed under the show's folder, and
/// episodes the download did not hold go back to wanted.
pub async fn import(state: &AppState, unit: &Title, g: &koda_db::grabs::Grab) -> Result<(), ImportError> {
    let db = &state.db;
    let err = |e: koda_db::Error| ImportError::Wait(e.to_string());
    let lib: LibrarySettings =
        koda_db::settings::get(db, LibrarySettings::KEY).await.map_err(err)?.unwrap_or_default();
    if lib.shows_path.trim().is_empty() {
        return Err(ImportError::Wait("Choose a shows folder in Settings to finish importing".into()));
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

    // the show, and every episode this grab is for
    let season_id = if unit.kind == "season" { unit.id } else { unit.parent_id.unwrap_or(0) };
    let season = koda_db::titles::by_id(db, season_id).await.map_err(err)?;
    let show_id = season.as_ref().and_then(|s| s.parent_id).unwrap_or(0);
    let show = koda_db::titles::by_id(db, show_id)
        .await
        .map_err(err)?
        .ok_or_else(|| ImportError::Failed("The show was removed".into()))?;
    let mut ids = koda_db::grabs::covered(db, g.id).await.map_err(err)?;
    if unit.kind == "episode" {
        ids.push(unit.id);
    }
    let mut targets = Vec::new();
    for id in ids {
        if let Some(t) = koda_db::titles::by_id(db, id).await.map_err(err)? {
            targets.push(t);
        }
    }
    let by_number: HashMap<(i64, i64), &Title> =
        targets.iter().filter_map(|t| Some(((t.season_number?, t.episode_number?), t))).collect();
    let all = koda_db::titles::episodes_of_show(db, show.id).await.map_err(err)?;

    let src = source.clone();
    let files = tokio::task::spawn_blocking(move || crate::pipeline::videos(&src))
        .await
        .map_err(|e| ImportError::Wait(e.to_string()))?;
    let quality = g.quality.as_deref().and_then(Quality::from_key);
    let group = koda_parser::parse_episode(&g.release_title).and_then(|p| p.release.group);
    let root = PathBuf::from(lib.shows_path.trim());

    let mut done: HashSet<i64> = HashSet::new();
    let mut episode_files = Vec::new();
    let mut placed = Vec::new();
    for (_, video) in &files {
        let eps = match_file(video, &by_number, &targets, files.len(), &all);
        let eps: Vec<&Title> = eps.into_iter().filter(|t| !done.contains(&t.id)).collect();
        let Some(first) = eps.first() else { continue };
        let ext = video.extension().and_then(|e| e.to_str()).unwrap_or("mkv").to_ascii_lowercase();
        let dest = episode_path(&root, &show, first.season_number.unwrap_or(0), &eps, quality, &ext);
        let (v, d) = (video.clone(), dest.clone());
        let size = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
            std::fs::create_dir_all(d.parent().expect("has a season folder"))?;
            crate::pipeline::place(&v, &d)?;
            if let Some(parent) = v.parent() {
                crate::pipeline::copy_subtitles(parent, &v, &d);
            }
            Ok(std::fs::metadata(&d)?.len())
        })
        .await
        .map_err(|e| ImportError::Wait(e.to_string()))?
        .map_err(|e| ImportError::Wait(format!("Could not write to the shows folder: {e}")))?;
        // shortcut: a file holding several episodes is linked to the first one only; the others show as available without their own file row
        let file_id = koda_db::library::add_file(
            db,
            first.id,
            &dest.to_string_lossy(),
            size as i64,
            g.quality.as_deref(),
            group.as_deref(),
        )
        .await
        .map_err(err)?;
        episode_files.extend(eps.iter().map(|t| (t.id, file_id)));
        done.extend(eps.iter().map(|t| t.id));
        placed.push(dest);
    }
    if done.is_empty() {
        return Err(ImportError::Failed("No file in the download matches the wanted episodes".into()));
    }

    koda_db::grabs::set_status(db, g.id, "imported", None, None).await.map_err(err)?;
    for t in &targets {
        if done.contains(&t.id) {
            let _ = move_to(state, t.id, TitleState::Available, None).await;
        } else {
            let why = "This episode was not in the download";
            if move_to(state, t.id, TitleState::Wanted, Some(why)).await.is_ok() {
                koda_db::titles::schedule_search(db, t.id, now(), Some(why)).await.map_err(err)?;
            }
        }
    }
    if unit.kind == "season" {
        move_to(state, unit.id, TitleState::Available, None).await?;
    }
    let names: Vec<String> = targets
        .iter()
        .filter(|t| done.contains(&t.id))
        .map(|t| format!("S{:02}E{:02}", t.season_number.unwrap_or(0), t.episode_number.unwrap_or(0)))
        .collect();
    let mut info = crate::notify::TitleInfo::from_title(&show, Some(names.join(", ")));
    info.episode_files = episode_files;
    crate::notify::send(state, crate::notify::Event::Available, Some(info), None);
    tracing::info!(show = %show.title, episodes = %names.join(","), "imported");
    for p in placed {
        crate::jellyfin::announce_import(state, p.to_string_lossy().into_owned());
    }
    Ok(())
}

/// Which of the wanted episodes a video file holds. A download with a single
/// video for a single episode needs no name.
fn match_file<'a>(
    video: &Path,
    by_number: &HashMap<(i64, i64), &'a Title>,
    targets: &'a [Title],
    files: usize,
    all: &[Title],
) -> Vec<&'a Title> {
    let name = video.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let parsed = koda_parser::parse_episode(name).or_else(|| {
        // "Show.S01E02.720p/abc123.mkv": the folder carries the name
        video
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .and_then(koda_parser::parse_episode)
    });
    if let Some(p) = parsed {
        // daily and anime names point at an episode through its date or absolute number
        let keys: Vec<(i64, i64)> = if let Some(date) = &p.air_date {
            all.iter()
                .filter(|t| t.release_date.as_deref() == Some(date))
                .filter_map(|t| Some((t.season_number?, t.episode_number?)))
                .collect()
        } else if !p.absolute.is_empty() {
            let abs = absolute_numbers(all);
            p.absolute.iter().filter_map(|a| abs.iter().find(|x| x.2 == *a).map(|x| (x.0, x.1))).collect()
        } else {
            let season = i64::from(p.seasons.first().copied().unwrap_or(0));
            p.episodes.iter().map(|e| (season, i64::from(*e))).collect()
        };
        if !keys.is_empty() {
            return keys.iter().filter_map(|k| by_number.get(k).copied()).collect();
        }
    }
    match (files, targets) {
        (1, [only]) => vec![only],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(id: i64, kind: &str, name: &str, s: Option<i64>, e: Option<i64>) -> Title {
        Title {
            id,
            kind: kind.into(),
            parent_id: None,
            mbid: None,
            season_number: s,
            episode_number: e,
            tmdb_id: Some(id),
            imdb_id: None,
            title: name.into(),
            original_title: None,
            year: Some(2011),
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
    fn episode_files_are_named_and_matched() {
        let show = title(1399, "show", "Thrones: Part", None, None);
        let e1 = title(1, "episode", "Winter", Some(1), Some(1));
        let e2 = title(2, "episode", "Kingsroad", Some(1), Some(2));
        let q = Quality::from_key("webdl-1080p");
        let p = episode_path(Path::new("/tv"), &show, 1, &[&e1], q, "mkv");
        assert!(
            p.ends_with("Thrones- Part (2011) {tmdb-1399}/Season 01/Thrones- Part - S01E01 - Winter - WEB-DL 1080p.mkv"),
            "{}",
            p.display()
        );
        let p = episode_path(Path::new("/tv"), &show, 1, &[&e1, &e2], None, "mkv");
        assert!(p.ends_with("Season 01/Thrones- Part - S01E01-E02.mkv"), "{}", p.display());

        let targets = vec![e1.clone(), e2.clone()];
        let by: HashMap<(i64, i64), &Title> =
            targets.iter().map(|t| ((t.season_number.unwrap(), t.episode_number.unwrap()), t)).collect();
        let ids = |v: Vec<&Title>| v.iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(
            ids(match_file(Path::new("/dl/Thrones.S01E02.1080p.mkv"), &by, &targets, 2, &targets)),
            [2]
        );
        assert_eq!(
            ids(match_file(Path::new("/dl/Thrones.S01E01E02.1080p/x1f3.mkv"), &by, &targets, 1, &targets)),
            [1, 2]
        );
        assert!(match_file(Path::new("/dl/Thrones.S02E01.mkv"), &by, &targets, 2, &targets).is_empty());
        assert_eq!(
            ids(match_file(Path::new("/dl/[Grp] Thrones - 02 [1080p].mkv"), &by, &targets, 2, &targets)),
            [2]
        );
        let mut dated = targets.clone();
        dated[0].release_date = Some("2011-04-17".into());
        let by_d: HashMap<(i64, i64), &Title> =
            dated.iter().map(|t| ((t.season_number.unwrap(), t.episode_number.unwrap()), t)).collect();
        assert_eq!(ids(match_file(Path::new("/dl/Thrones.2011.04.17.mkv"), &by_d, &dated, 2, &dated)), [1]);
        let one = vec![e1.clone()];
        let by1: HashMap<(i64, i64), &Title> = HashMap::from([((1, 1), &one[0])]);
        assert_eq!(
            ids(match_file(Path::new("/dl/abc123.mkv"), &by1, &one, 1, &one)),
            [1],
            "one file, one episode"
        );
    }
}

/// A wanted season through a fake indexer and qBittorrent into the shows folder.
#[cfg(test)]
mod flow {
    use super::*;
    use axum::{Router, extract::Query, routing::get, routing::post};
    use koda_db::titles::{NewEpisode, NewSeason, NewShow, NewTitle};

    const HASH: &str = "d12fe1c06bba254a9dc9f519b335aa7c1367a88b";

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn wanted_season_arrives_as_a_pack() {
        let root = std::env::temp_dir().join(format!("koda-series-{}-{}", std::process::id(), now()));
        let download = root.join("downloads/Test.Show.S01.1080p.WEB-DL.H264-GRP");
        std::fs::create_dir_all(&download).unwrap();
        for e in 1..=2 {
            let name = format!("Test.Show.S01E0{e}.1080p.WEB-DL.H264-GRP.mkv");
            std::fs::write(download.join(name), vec![e as u8; 2048]).unwrap();
        }

        let indexer = serve(Router::new().route(
            "/api",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                match q.get("t").map(String::as_str) {
                    Some("caps") => r#"<caps><searching><tv-search available="yes" supportedParams="q,season,ep,tvdbid"/></searching></caps>"#.to_owned(),
                    _ => {
                        assert_eq!(q.get("t").map(String::as_str), Some("tvsearch"));
                        assert_eq!(q.get("tvdbid").map(String::as_str), Some("777"));
                        format!(r#"<rss><channel>
                        <item><title>Test.Show.S01.1080p.WEB-DL.H264-GRP</title>
                          <link>magnet:?xt=urn:btih:{HASH}</link><size>3221225472</size>
                          <attr name="seeders" value="10"/></item>
                        <item><title>Test.Show.S01E01.1080p.WEB-DL.H264-GRP</title>
                          <link>magnet:?xt=urn:btih:ffffffffffffffffffffffffffffffffffffffff</link><size>1600000000</size>
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
                            axum::Json(serde_json::json!([{ "hash": HASH.to_uppercase(), "name": "Test.Show",
                                "state": "stalledUP", "progress": 1.0, "size": 4096,
                                "save_path": "/elsewhere", "content_path": content }]))
                        }
                    }),
                ),
        )
        .await;

        let db = koda_db::memory().await;
        let config = crate::config::Config {
            config_dir: root.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let state = AppState::new(db.clone(), config, Default::default());
        koda_db::connectors::create_indexer(
            &db,
            &koda_db::connectors::IndexerInput {
                name: "fake".into(),
                kind: "torznab".into(),
                base_url: indexer,
                api_path: "/api".into(),
                api_key: None,
                categories: vec![2000, 5000],
                priority: 25,
                enabled: true,
            },
        )
        .await
        .unwrap();
        koda_db::connectors::create_download_client(
            &db,
            &koda_db::connectors::DownloadClientInput {
                name: "qbit".into(),
                kind: "qbittorrent".into(),
                base_url: qbit,
                username: None,
                password: None,
                category: "koda".into(),
                enabled: true,
            },
        )
        .await
        .unwrap();
        let part = |id: i64, name: &str, date: Option<&str>| NewTitle {
            tmdb_id: id,
            title: name.into(),
            release_date: date.map(str::to_owned),
            ..Default::default()
        };
        let show = NewShow {
            show: NewTitle { year: Some(2020), runtime_minutes: Some(45), ..part(500, "Test Show", None) },
            tvdb_id: Some(777),
            seasons: vec![NewSeason {
                title: part(501, "Season 1", Some("2020-01-01")),
                number: 1,
                episodes: (1..=2)
                    .map(|n| NewEpisode {
                        title: part(510 + n, &format!("Part {n}"), Some("2020-01-01")),
                        number: n,
                    })
                    .collect(),
            }],
        };
        let s = koda_db::titles::upsert_show(&db, &show, TitleState::Wanted).await.unwrap();
        let lib = LibrarySettings {
            shows_path: root.join("tv").to_string_lossy().into_owned(),
            ..Default::default()
        };
        koda_db::settings::set(&db, LibrarySettings::KEY, &lib).await.unwrap();
        let states = || async {
            koda_db::titles::episodes_of_show(&db, s.id)
                .await
                .unwrap()
                .iter()
                .map(|e| e.state())
                .collect::<Vec<_>>()
        };

        // by hand, one episode: only the single-episode release fits
        let ep1 = koda_db::titles::episodes_of_show(&db, s.id).await.unwrap().remove(0);
        let found = crate::pipeline::search(&state, &ep1).await.unwrap();
        let ok: Vec<&str> =
            found.evaluated.iter().filter(|e| e.accepted()).map(|e| e.title.as_str()).collect();
        assert_eq!(ok, ["Test.Show.S01E01.1080p.WEB-DL.H264-GRP"]);
        assert!(crate::pipeline::search(&state, &s).await.is_err(), "a whole show is not searched by hand");

        // pass 1: the whole season is wanted, so the pack wins over the single episode
        crate::pipeline::tick_for_tests(&state).await;
        assert_eq!(states().await, [TitleState::Grabbed, TitleState::Grabbed]);

        // pass 2: complete; pass 3: imported file by file
        crate::pipeline::tick_for_tests(&state).await;
        crate::pipeline::tick_for_tests(&state).await;
        assert_eq!(states().await, [TitleState::Available, TitleState::Available]);
        let eps = koda_db::titles::episodes_of_show(&db, s.id).await.unwrap();
        let file = &koda_db::library::files_for_title(&db, eps[1].id).await.unwrap()[0];
        assert!(
            Path::new(&file.path).ends_with(
                "Test Show (2020) {tmdb-500}/Season 01/Test Show - S01E02 - Part 2 - WEB-DL 1080p.mkv"
            ),
            "{}",
            file.path
        );
        assert_eq!(std::fs::read(&file.path).unwrap(), vec![2u8; 2048]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
