//! Browsing: libraries (views), lists of films and shows, a show's seasons
//! and episodes, what to resume and what is next, search, and artwork.
//!
//! Only titles with a file are shown: apps list what can be played.

use std::cmp::Ordering;
use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use moka_db::library::{MediaFile, UserData};
use moka_db::titles::Title;
use serde_json::{Value, json};

use super::{AppUser, IdKind, Q, guid, iso, parse_guid, server_id, ticks};
use crate::error::ApiResult;
use crate::state::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/userviews", get(views))
        .route("/users/{uid}/views", get(views))
        .route("/library/mediafolders", get(views))
        .route("/items", get(items))
        .route("/users/{uid}/items", get(items))
        .route("/items/latest", get(latest))
        .route("/users/{uid}/items/latest", get(latest))
        .route("/useritems/resume", get(resume))
        .route("/users/{uid}/items/resume", get(resume))
        .route("/items/{id}", get(item))
        .route("/users/{uid}/items/{id}", get(user_item))
        .route("/shows/nextup", get(next_up))
        .route("/shows/{id}/seasons", get(seasons))
        .route("/shows/{id}/episodes", get(episodes))
        .route("/search/hints", get(search_hints))
        .route("/artists", get(artists))
        .route("/artists/albumartists", get(artists))
        .route("/items/{id}/images/{kind}", get(image).head(image))
        .route("/items/{id}/images/{kind}/{index}", get(image_at).head(image_at))
        .route("/items/{id}/similar", get(empty_result))
        .route("/items/{id}/intros", get(empty_result))
        .route("/users/{uid}/items/{id}/intros", get(empty_result))
        .route("/items/{id}/specialfeatures", get(empty_list))
        .route("/users/{uid}/items/{id}/specialfeatures", get(empty_list))
        .route("/items/{id}/localtrailers", get(empty_list))
        .route("/users/{uid}/items/{id}/localtrailers", get(empty_list))
        .route("/items/{id}/ancestors", get(empty_list))
        .route("/items/{id}/thememedia", get(theme_media))
        .route("/genres", get(empty_result))
        .route("/studios", get(empty_result))
        .route("/persons", get(empty_result))
        .route(
            "/items/filters",
            get(|| async { Json(json!({ "Genres": [], "Tags": [], "OfficialRatings": [], "Years": [] })) }),
        )
        .route("/items/filters2", get(|| async { Json(json!({ "Genres": [], "Tags": [] })) }))
}

const FILMS_VIEW: i64 = 1;
const SHOWS_VIEW: i64 = 2;
const MUSIC_VIEW: i64 = 3;

async fn empty_result() -> Json<Value> {
    Json(json!({ "Items": [], "TotalRecordCount": 0, "StartIndex": 0 }))
}

async fn empty_list() -> Json<Value> {
    Json(json!([]))
}

async fn theme_media() -> Json<Value> {
    let empty = json!({ "Items": [], "TotalRecordCount": 0, "StartIndex": 0, "OwnerId": "", "OwnerID": "" });
    Json(json!({ "ThemeVideosResult": empty, "ThemeSongsResult": empty, "SoundtrackSongsResult": empty }))
}

/// Everything one request needs, read once: video titles, their files and
/// Everything one request needs, read once: titles, their files, what this
/// user did with them, and who holds whom (with a file somewhere below).
pub(super) struct Library {
    pub titles: HashMap<i64, Title>,
    pub files: HashMap<i64, Vec<MediaFile>>,
    pub data: HashMap<i64, UserData>,
    pub server_id: String,
    /// Episodes with a file, in order: all, by season, by show.
    episodes_all: Vec<i64>,
    episodes_by_season: HashMap<i64, Vec<i64>>,
    episodes_by_show: HashMap<i64, Vec<i64>>,
    /// Tracks with a file, in order: all, by album.
    tracks_all: Vec<i64>,
    tracks_by_album: HashMap<i64, Vec<i64>>,
    /// Albums with a track file, by title: all, by artist.
    albums_all: Vec<i64>,
    albums_by_artist: HashMap<i64, Vec<i64>>,
    /// When each title arrived: its newest file, or the newest below it.
    added_at: HashMap<i64, i64>,
    /// A track's number on its disc (positions count across the album).
    track_numbers: HashMap<i64, i64>,
}

impl Library {
    pub async fn load(state: &AppState, user_id: i64) -> ApiResult<Self> {
        let titles: HashMap<i64, Title> =
            moka_db::titles::app_titles(&state.db).await?.into_iter().map(|t| (t.id, t)).collect();
        let mut files: HashMap<i64, Vec<MediaFile>> = HashMap::new();
        for f in moka_db::library::all_files(&state.db).await? {
            files.entry(f.title_id).or_default().push(f);
        }
        let data = moka_db::library::user_data(&state.db, user_id)
            .await?
            .into_iter()
            .map(|d| (d.title_id, d))
            .collect();
        let mut lib = Self {
            titles,
            files,
            data,
            server_id: server_id(state).await?,
            episodes_all: Vec::new(),
            episodes_by_season: HashMap::new(),
            episodes_by_show: HashMap::new(),
            tracks_all: Vec::new(),
            tracks_by_album: HashMap::new(),
            albums_all: Vec::new(),
            albums_by_artist: HashMap::new(),
            added_at: HashMap::new(),
            track_numbers: HashMap::new(),
        };
        lib.index();
        Ok(lib)
    }

    /// Fills the parent and child lists, arrival times and track numbers.
    fn index(&mut self) {
        let parent = |id: Option<i64>| id.and_then(|p| self.titles.get(&p));
        let mut added: HashMap<i64, i64> = HashMap::new();
        let mut bump = |id: i64, at: i64| {
            let e = added.entry(id).or_insert(at);
            *e = (*e).max(at);
        };
        let mut episodes: Vec<&Title> = Vec::new();
        let mut tracks: Vec<&Title> = Vec::new();
        for t in self.titles.values() {
            let Some(f) = self.files.get(&t.id).and_then(|f| f.first()) else { continue };
            bump(t.id, f.added_at);
            match t.kind.as_str() {
                "episode" => episodes.push(t),
                "track" => tracks.push(t),
                _ => {}
            }
            // the season and show, or album and artist, arrived with it
            let up = parent(t.parent_id);
            if let Some(up) = up {
                bump(up.id, f.added_at);
                if let Some(top) = parent(up.parent_id) {
                    bump(top.id, f.added_at);
                }
            }
        }
        episodes.sort_by_key(|e| (e.season_number, e.episode_number, e.id));
        tracks.sort_by_key(|t| (t.parent_id, t.season_number, t.episode_number, t.id));
        let mut by_season: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut by_show: HashMap<i64, Vec<i64>> = HashMap::new();
        for e in &episodes {
            if let Some(season) = parent(e.parent_id) {
                by_season.entry(season.id).or_default().push(e.id);
                if let Some(show) = parent(season.parent_id) {
                    by_show.entry(show.id).or_default().push(e.id);
                }
            }
        }
        let mut by_album: HashMap<i64, Vec<i64>> = HashMap::new();
        for t in &tracks {
            if let Some(a) = t.parent_id {
                by_album.entry(a).or_default().push(t.id);
            }
        }
        let mut albums: Vec<&Title> = by_album.keys().filter_map(|a| self.titles.get(a)).collect();
        albums.sort_by_key(|a| (sort_name(&a.title), a.id));
        let mut by_artist: HashMap<i64, Vec<i64>> = HashMap::new();
        for a in &albums {
            if let Some(artist) = a.parent_id {
                by_artist.entry(artist).or_default().push(a.id);
            }
        }
        // numbers count every track of the album, with a file or not
        let mut all_tracks: Vec<&Title> = self.titles.values().filter(|t| t.kind == "track").collect();
        all_tracks.sort_by_key(|t| (t.parent_id, t.season_number, t.episode_number, t.id));
        let mut numbers = HashMap::new();
        let mut last: Option<(Option<i64>, Option<i64>)> = None;
        let mut n = 0;
        for t in all_tracks {
            let disc = (t.parent_id, t.season_number);
            n = if last == Some(disc) { n + 1 } else { 1 };
            last = Some(disc);
            numbers.insert(t.id, n);
        }

        self.episodes_all = episodes.iter().map(|e| e.id).collect();
        self.tracks_all = tracks.iter().map(|t| t.id).collect();
        self.albums_all = albums.iter().map(|a| a.id).collect();
        self.episodes_by_season = by_season;
        self.episodes_by_show = by_show;
        self.tracks_by_album = by_album;
        self.albums_by_artist = by_artist;
        self.added_at = added;
        self.track_numbers = numbers;
    }

    fn ids(&self, ids: Option<&Vec<i64>>) -> Vec<&Title> {
        ids.map(|l| l.iter().filter_map(|id| self.titles.get(id)).collect()).unwrap_or_default()
    }

    pub fn file(&self, id: i64) -> Option<&MediaFile> {
        self.files.get(&id).and_then(|f| f.first())
    }

    fn kind(&self, kind: &str) -> impl Iterator<Item = &Title> {
        self.titles.values().filter(move |t| t.kind == kind)
    }

    pub fn movies(&self) -> Vec<&Title> {
        self.kind("movie").filter(|t| self.file(t.id).is_some()).collect()
    }

    pub fn parent(&self, t: &Title) -> Option<&Title> {
        t.parent_id.and_then(|p| self.titles.get(&p))
    }

    /// Episodes with a file, in season and episode order.
    pub fn episodes(&self, show: Option<i64>, season: Option<i64>) -> Vec<&Title> {
        match (show, season) {
            (_, Some(s)) => {
                let list = self.ids(self.episodes_by_season.get(&s));
                list.into_iter().filter(|e| show.is_none_or(|sh| self.show_id(e) == Some(sh))).collect()
            }
            (Some(sh), None) => self.ids(self.episodes_by_show.get(&sh)),
            (None, None) => self.ids(Some(&self.episodes_all)),
        }
    }

    fn show_id(&self, episode: &Title) -> Option<i64> {
        self.parent(episode).and_then(|s| s.parent_id)
    }

    /// Seasons with at least one episode file, in order.
    pub fn seasons(&self, show: Option<i64>) -> Vec<&Title> {
        let mut out: Vec<&Title> = self
            .episodes_by_season
            .keys()
            .filter_map(|s| self.titles.get(s))
            .filter(|s| show.is_none_or(|sh| s.parent_id == Some(sh)))
            .collect();
        out.sort_by_key(|s| (s.parent_id, s.season_number));
        out
    }

    /// Shows with at least one episode file.
    pub fn shows(&self) -> Vec<&Title> {
        self.episodes_by_show.keys().filter_map(|s| self.titles.get(s)).collect()
    }

    /// Tracks with a file, by album, disc and position.
    pub fn tracks(&self, album: Option<i64>) -> Vec<&Title> {
        match album {
            Some(a) => self.ids(self.tracks_by_album.get(&a)),
            None => self.ids(Some(&self.tracks_all)),
        }
    }

    /// Albums with at least one track file, by title.
    pub fn albums(&self, artist: Option<i64>) -> Vec<&Title> {
        match artist {
            Some(a) => self.ids(self.albums_by_artist.get(&a)),
            None => self.ids(Some(&self.albums_all)),
        }
    }

    /// Artists with at least one album in the library.
    pub fn artists(&self) -> Vec<&Title> {
        self.albums_by_artist.keys().filter_map(|a| self.titles.get(a)).collect()
    }

    fn track_number(&self, t: &Title) -> Option<i64> {
        self.track_numbers.get(&t.id).copied()
    }

    /// Whether apps may see this title.
    pub fn visible(&self, t: &Title) -> bool {
        match t.kind.as_str() {
            "movie" | "episode" | "track" => self.file(t.id).is_some(),
            "season" => self.episodes_by_season.contains_key(&t.id),
            "show" => self.episodes_by_show.contains_key(&t.id),
            "album" => self.tracks_by_album.contains_key(&t.id),
            "artist" => self.albums_by_artist.contains_key(&t.id),
            _ => false,
        }
    }

    fn played(&self, id: i64) -> bool {
        self.data.get(&id).is_some_and(|d| d.played)
    }

    /// Watched: the title itself, or every episode of a show or season.
    pub fn watched(&self, t: &Title) -> bool {
        let all = |eps: Vec<&Title>| !eps.is_empty() && eps.iter().all(|e| self.played(e.id));
        match t.kind.as_str() {
            "show" => all(self.episodes(Some(t.id), None)),
            "season" => all(self.episodes(None, Some(t.id))),
            _ => self.played(t.id),
        }
    }

    /// When the title arrived: its newest file, or the newest below it.
    fn added(&self, t: &Title) -> i64 {
        self.added_at.get(&t.id).copied().unwrap_or(t.created_at)
    }

    fn runtime_seconds(&self, t: &Title) -> Option<f64> {
        self.file(t.id)
            .and_then(|f| f.duration_seconds)
            .filter(|d| *d > 0.0)
            .or_else(|| t.runtime_minutes.map(|m| m as f64 * 60.0))
    }

    fn user_data(&self, t: &Title) -> Value {
        let d = self.data.get(&t.id).cloned().unwrap_or_default();
        let mut v = json!({
            "PlaybackPositionTicks": ticks(d.position_seconds),
            "PlayCount": d.play_count,
            "IsFavorite": d.favorite,
            "Played": d.played,
            "Key": guid(IdKind::Title, t.id),
            "ItemId": guid(IdKind::Title, t.id),
        });
        if let Some(at) = d.last_played_at {
            v["LastPlayedDate"] = json!(iso(at));
        }
        if let Some(run) = self.runtime_seconds(t).filter(|r| *r > 0.0 && d.position_seconds > 0.0) {
            v["PlayedPercentage"] = json!((d.position_seconds / run * 100.0).min(100.0));
        }
        let unplayed = |eps: Vec<&Title>| {
            let left = eps.iter().filter(|e| !self.played(e.id)).count();
            (left, eps.len())
        };
        let counts = match t.kind.as_str() {
            "show" => Some(unplayed(self.episodes(Some(t.id), None))),
            "season" => Some(unplayed(self.episodes(None, Some(t.id)))),
            _ => None,
        };
        if let Some((left, all)) = counts {
            v["UnplayedItemCount"] = json!(left);
            v["Played"] = json!(all > 0 && left == 0);
        }
        v
    }

    /// A title as apps expect it.
    pub fn dto(&self, t: &Title) -> Value {
        let id = guid(IdKind::Title, t.id);
        let kind = match t.kind.as_str() {
            "movie" => "Movie",
            "show" => "Series",
            "season" => "Season",
            "artist" => "MusicArtist",
            "album" => "MusicAlbum",
            "track" => "Audio",
            _ => "Episode",
        };
        let folder = matches!(t.kind.as_str(), "show" | "season" | "artist" | "album");
        let media = match t.kind.as_str() {
            _ if folder => "Unknown",
            "track" => "Audio",
            _ => "Video",
        };
        let mut v = json!({
            "Name": t.title,
            "SortName": sort_name(&t.title),
            "ServerId": self.server_id,
            "Id": id,
            "Etag": format!("{:x}", t.updated_at),
            "DateCreated": iso(self.added(t)),
            "CanDelete": false,
            "CanDownload": !folder,
            "Type": kind,
            "IsFolder": folder,
            "LocationType": "FileSystem",
            "MediaType": media,
            "UserData": self.user_data(t),
            "ImageTags": {},
            "BackdropImageTags": [],
            "ImageBlurHashes": {},
            "Genres": [],
            "GenreItems": [],
            "Studios": [],
            "People": [],
            "Taglines": [],
            "Tags": [],
            "ExternalUrls": [],
            "RemoteTrailers": [],
            "LockedFields": [],
            "LockData": false,
            "ProviderIds": provider_ids(t),
        });
        if let Some(o) = &t.overview {
            v["Overview"] = json!(o);
        }
        if let Some(y) = t.year {
            v["ProductionYear"] = json!(y);
        }
        if let Some(d) = t.release_date.as_deref().filter(|d| d.len() >= 10) {
            v["PremiereDate"] = json!(format!("{}T00:00:00.0000000Z", &d[..10]));
        }
        if let Some(run) = self.runtime_seconds(t) {
            v["RunTimeTicks"] = json!(ticks(run));
        }
        if let Some(tag) = t.poster_path.as_deref().and_then(image_tag) {
            v["ImageTags"] = json!({ "Primary": tag });
            v["PrimaryImageAspectRatio"] = json!(if t.kind == "episode" { 16.0 / 9.0 } else { 2.0 / 3.0 });
        }
        if let Some(tag) = t.backdrop_path.as_deref().and_then(image_tag) {
            v["BackdropImageTags"] = json!([tag]);
        }
        if let Some(f) = self.file(t.id) {
            v["Container"] = json!(extension(&f.path));
            if media == "Video" {
                v["VideoType"] = json!("VideoFile");
            }
            v["Path"] = json!(f.path.rsplit('/').next().unwrap_or(&f.path));
            v["MediaSourceCount"] = json!(1);
        }
        match t.kind.as_str() {
            "show" => {
                let seasons = self.seasons(Some(t.id));
                v["ChildCount"] = json!(seasons.len());
                v["RecursiveItemCount"] = json!(self.episodes(Some(t.id), None).len());
                v["Status"] = json!("Continuing");
                v["AirDays"] = json!([]);
                v["DisplayOrder"] = json!("");
            }
            "season" => {
                let show = self.parent(t);
                v["IndexNumber"] = json!(t.season_number);
                v["ChildCount"] = json!(self.episodes(None, Some(t.id)).len());
                self.series_fields(&mut v, show);
                v["ParentId"] = json!(show.map(|s| guid(IdKind::Title, s.id)));
                if v["ImageTags"].get("Primary").is_none()
                    && let Some(tag) = show.and_then(|s| s.poster_path.as_deref()).and_then(image_tag)
                {
                    // apps show the series poster for a season without one
                    v["ImageTags"] = json!({});
                    v["SeriesPrimaryImageTag"] = json!(tag);
                }
            }
            "episode" => {
                let season = self.parent(t);
                let show = season.and_then(|s| self.parent(s));
                v["IndexNumber"] = json!(t.episode_number);
                v["ParentIndexNumber"] = json!(t.season_number);
                v["SeasonId"] = json!(season.map(|s| guid(IdKind::Title, s.id)));
                v["SeasonName"] = json!(season.map(|s| s.title.clone()));
                v["ParentId"] = json!(season.map(|s| guid(IdKind::Title, s.id)));
                self.series_fields(&mut v, show);
            }
            "artist" => {
                v["ChildCount"] = json!(self.albums(Some(t.id)).len());
            }
            "album" => {
                v["ChildCount"] = json!(self.tracks(Some(t.id)).len());
                self.album_fields(&mut v, Some(t));
            }
            "track" => {
                let album = self.parent(t);
                v["IndexNumber"] = json!(self.track_number(t));
                v["ParentIndexNumber"] = json!(t.season_number);
                v["Album"] = json!(album.map(|a| a.title.clone()));
                v["AlbumId"] = json!(album.map(|a| guid(IdKind::Title, a.id)));
                v["ParentId"] = json!(album.map(|a| guid(IdKind::Title, a.id)));
                self.album_fields(&mut v, album);
            }
            _ => {}
        }
        v
    }

    /// Artist and cover of an album, on the album and on its tracks.
    fn album_fields(&self, v: &mut Value, album: Option<&Title>) {
        let Some(album) = album else { return };
        if let Some(artist) = self.parent(album) {
            let item = json!([{ "Name": artist.title, "Id": guid(IdKind::Title, artist.id) }]);
            v["AlbumArtist"] = json!(artist.title);
            v["AlbumArtists"] = item.clone();
            v["ArtistItems"] = item;
            v["Artists"] = json!([artist.title]);
        }
        if let Some(tag) = album.mbid.as_deref().map(|m| m.replace('-', "")) {
            v["ImageTags"] = json!({ "Primary": tag });
            v["AlbumPrimaryImageTag"] = json!(tag);
            v["PrimaryImageAspectRatio"] = json!(1.0);
        }
    }

    fn series_fields(&self, v: &mut Value, show: Option<&Title>) {
        let Some(show) = show else { return };
        v["SeriesId"] = json!(guid(IdKind::Title, show.id));
        v["SeriesName"] = json!(show.title);
        if let Some(tag) = show.poster_path.as_deref().and_then(image_tag) {
            v["SeriesPrimaryImageTag"] = json!(tag);
        }
        if let Some(tag) = show.backdrop_path.as_deref().and_then(image_tag) {
            v["ParentBackdropItemId"] = json!(guid(IdKind::Title, show.id));
            v["ParentBackdropImageTags"] = json!([tag]);
        }
    }
}

fn provider_ids(t: &Title) -> Value {
    let mut ids = serde_json::Map::new();
    if let Some(id) = t.tmdb_id.filter(|_| matches!(t.kind.as_str(), "movie" | "show")) {
        ids.insert("Tmdb".into(), json!(id.to_string()));
    }
    if let Some(id) = &t.imdb_id {
        ids.insert("Imdb".into(), json!(id));
    }
    Value::Object(ids)
}

/// "The Matrix" sorts as "matrix".
fn sort_name(title: &str) -> String {
    let lower = title.to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article) {
            return rest.to_owned();
        }
    }
    lower
}

/// A TMDB image path (`/abc.jpg`) as an image tag (`abc`).
fn image_tag(path: &str) -> Option<String> {
    let tag = path.trim_start_matches('/').split('.').next()?;
    (!tag.is_empty()).then(|| tag.to_owned())
}

fn extension(path: &str) -> String {
    path.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn view(server_id: &str, n: i64) -> Value {
    let (name, collection) = match n {
        FILMS_VIEW => ("Films", "movies"),
        SHOWS_VIEW => ("Shows", "tvshows"),
        _ => ("Music", "music"),
    };
    json!({
        "Name": name,
        "ServerId": server_id,
        "Id": guid(IdKind::View, n),
        "Etag": "1",
        "DateCreated": iso(0),
        "CanDelete": false,
        "CanDownload": false,
        "SortName": name.to_lowercase(),
        "Type": "CollectionFolder",
        "CollectionType": collection,
        "IsFolder": true,
        "LocationType": "FileSystem",
        "MediaType": "Unknown",
        "ImageTags": {},
        "BackdropImageTags": [],
        "UserData": { "PlaybackPositionTicks": 0, "PlayCount": 0, "IsFavorite": false, "Played": false, "Key": guid(IdKind::View, n) },
        "ChildCount": 0,
    })
}

fn result(items: Vec<Value>, total: usize, start: usize) -> Json<Value> {
    Json(json!({ "Items": items, "TotalRecordCount": total, "StartIndex": start }))
}

/// Films and shows always; music once there is some.
async fn views(u: AppUser, State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let mut list = vec![view(&lib.server_id, FILMS_VIEW), view(&lib.server_id, SHOWS_VIEW)];
    if !lib.tracks(None).is_empty() {
        list.push(view(&lib.server_id, MUSIC_VIEW));
    }
    let n = list.len();
    Ok(result(list, n, 0))
}

/// Type names apps filter by, as Moka kinds.
fn kind_of(app_type: &str) -> Option<&'static str> {
    match app_type {
        "movie" => Some("movie"),
        "series" => Some("show"),
        "season" => Some("season"),
        "episode" => Some("episode"),
        "musicartist" => Some("artist"),
        "musicalbum" => Some("album"),
        "audio" => Some("track"),
        _ => None,
    }
}

/// The titles a `/Items` query selects, before filters and paging.
fn select<'a>(lib: &'a Library, q: &Q) -> Vec<&'a Title> {
    let types: Vec<&str> = q.list("includeitemtypes").iter().filter_map(|t| kind_of(t)).collect();
    let wants = |k: &str| types.contains(&k);
    let recursive = q.flag("recursive").unwrap_or(false);
    let ids: Vec<i64> = q
        .list("ids")
        .iter()
        .filter_map(|i| parse_guid(i))
        .filter(|(k, _)| *k == IdKind::Title)
        .map(|(_, id)| id)
        .collect();
    if !ids.is_empty() {
        return ids.iter().filter_map(|id| lib.titles.get(id)).filter(|t| lib.visible(t)).collect();
    }
    let by_types = |lib: &'a Library| -> Vec<&'a Title> {
        let mut out = Vec::new();
        if types.is_empty() || wants("movie") {
            out.extend(lib.movies());
        }
        if types.is_empty() || wants("show") {
            out.extend(lib.shows());
        }
        if wants("season") {
            out.extend(lib.seasons(None));
        }
        if wants("episode") {
            out.extend(lib.episodes(None, None));
        }
        if wants("artist") {
            out.extend(lib.artists());
        }
        if wants("album") {
            out.extend(lib.albums(None));
        }
        if wants("track") {
            out.extend(lib.tracks(None));
        }
        out
    };
    // music apps ask for an artist's albums or an album's tracks this way
    let artist = q.get("albumartistids").or_else(|| q.get("artistids")).and_then(parse_guid).map(|(_, n)| n);
    if let Some(a) = artist {
        return if wants("track") {
            lib.albums(Some(a)).into_iter().flat_map(|al| lib.tracks(Some(al.id))).collect()
        } else {
            lib.albums(Some(a))
        };
    }
    if let Some((_, al)) = q.get("albumids").and_then(parse_guid) {
        return lib.tracks(Some(al));
    }
    match q.get("parentid").and_then(parse_guid) {
        Some((IdKind::View, FILMS_VIEW)) => lib.movies(),
        Some((IdKind::View, SHOWS_VIEW)) => {
            if wants("episode") {
                lib.episodes(None, None)
            } else if wants("season") {
                lib.seasons(None)
            } else {
                lib.shows()
            }
        }
        Some((IdKind::View, MUSIC_VIEW)) => {
            if wants("track") {
                lib.tracks(None)
            } else if wants("artist") {
                lib.artists()
            } else {
                lib.albums(None)
            }
        }
        Some((IdKind::Title, id)) => match lib.titles.get(&id).map(|t| t.kind.as_str()) {
            Some("artist") if wants("track") => {
                lib.albums(Some(id)).into_iter().flat_map(|al| lib.tracks(Some(al.id))).collect()
            }
            Some("artist") => lib.albums(Some(id)),
            Some("album") => lib.tracks(Some(id)),
            Some("show") if recursive || wants("episode") => lib.episodes(Some(id), None),
            Some("show") => lib.seasons(Some(id)),
            Some("season") => lib.episodes(None, Some(id)),
            _ => vec![],
        },
        Some(_) => vec![],
        None if types.is_empty() && !recursive => {
            let mut all = lib.movies();
            all.extend(lib.shows());
            all
        }
        None => by_types(lib),
    }
}

fn keep(lib: &Library, q: &Q, t: &Title) -> bool {
    let d = lib.data.get(&t.id);
    let played = lib.watched(t);
    let resumable = d.is_some_and(|d| d.position_seconds > 0.0 && !d.played);
    let favorite = d.is_some_and(|d| d.favorite);
    let filters = q.list("filters");
    let term = q.get("searchterm").or_else(|| q.get("namestartswith")).map(str::to_lowercase);
    term.is_none_or(|s| t.title.to_lowercase().contains(&s))
        && (!filters.iter().any(|f| f == "isresumable") || resumable)
        && (!filters.iter().any(|f| f == "isplayed") || played)
        && (!filters.iter().any(|f| f == "isunplayed") || !played)
        && (!filters.iter().any(|f| f == "isfavorite") || favorite)
        && q.flag("isplayed").is_none_or(|p| p == played)
        && q.flag("isfavorite").is_none_or(|f| f == favorite)
}

fn sort(lib: &Library, q: &Q, list: &mut [&Title]) {
    let by = q.list("sortby");
    let key = by.first().map(String::as_str).unwrap_or("");
    let order = |a: &&Title, b: &&Title| -> Ordering {
        match key {
            "datecreated" | "datelastcontentadded" => lib.added(a).cmp(&lib.added(b)),
            "premieredate" | "productionyear" => (a.year, &a.release_date).cmp(&(b.year, &b.release_date)),
            "dateplayed" | "datelastplayed" => {
                let at = |t: &Title| lib.data.get(&t.id).and_then(|d| d.last_played_at);
                at(a).cmp(&at(b))
            }
            "random" => ((a.id * 2_654_435_761) % 1_000_003).cmp(&((b.id * 2_654_435_761) % 1_000_003)),
            "indexnumber" | "parentindexnumber" | "airedepisodeorder" => {
                (a.season_number, a.episode_number).cmp(&(b.season_number, b.episode_number))
            }
            _ if a.kind == "track" && b.kind == "track" => (a.parent_id, a.season_number, a.episode_number)
                .cmp(&(b.parent_id, b.season_number, b.episode_number)),
            _ if a.kind == "episode" && b.kind == "episode" => {
                (a.season_number, a.episode_number).cmp(&(b.season_number, b.episode_number))
            }
            _ => sort_name(&a.title).cmp(&sort_name(&b.title)),
        }
    };
    list.sort_by(order);
    let descending = q.list("sortorder").first().is_some_and(|o| o == "descending");
    if descending {
        list.reverse();
    }
}

fn page(lib: &Library, q: &Q, mut list: Vec<&Title>) -> Json<Value> {
    list.retain(|t| keep(lib, q, t));
    sort(lib, q, &mut list);
    let total = list.len();
    let start = q.int("startindex").unwrap_or(0).max(0) as usize;
    let limit = q.int("limit").filter(|l| *l > 0).map(|l| l as usize).unwrap_or(usize::MAX);
    let items = list.into_iter().skip(start).take(limit).map(|t| lib.dto(t)).collect();
    result(items, total, start)
}

async fn items(u: AppUser, State(state): State<AppState>, q: Q) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let list = select(&lib, &q);
    Ok(page(&lib, &q, list))
}

async fn artists(u: AppUser, State(state): State<AppState>, q: Q) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let list = lib.artists();
    Ok(page(&lib, &q, list))
}

async fn latest(u: AppUser, State(state): State<AppState>, q: Q) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let types: Vec<&str> = q.list("includeitemtypes").iter().filter_map(|t| kind_of(t)).collect();
    let mut list: Vec<&Title> = match q.get("parentid").and_then(parse_guid) {
        Some((IdKind::View, FILMS_VIEW)) => lib.movies(),
        Some((IdKind::View, SHOWS_VIEW)) => lib.shows(),
        Some((IdKind::View, MUSIC_VIEW)) => lib.albums(None),
        Some((IdKind::Title, id)) => lib.episodes(Some(id), None),
        _ if types.contains(&"album") || types.contains(&"track") => lib.albums(None),
        _ if types.contains(&"movie") && !types.contains(&"show") => lib.movies(),
        _ if types.contains(&"show") || types.contains(&"episode") => lib.shows(),
        _ => {
            let mut all = lib.movies();
            all.extend(lib.shows());
            all
        }
    };
    if q.flag("isplayed") == Some(false) {
        list.retain(|t| !lib.watched(t));
    }
    list.sort_by_key(|t| std::cmp::Reverse(lib.added(t)));
    let limit = q.int("limit").filter(|l| *l > 0).unwrap_or(16) as usize;
    Ok(Json(Value::Array(list.into_iter().take(limit).map(|t| lib.dto(t)).collect())))
}

async fn resume(u: AppUser, State(state): State<AppState>, q: Q) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let mut list: Vec<(&Title, i64)> = lib
        .data
        .values()
        .filter(|d| d.position_seconds > 0.0 && !d.played)
        .filter_map(|d| lib.titles.get(&d.title_id).map(|t| (t, d.last_played_at.unwrap_or(0))))
        .filter(|(t, _)| matches!(t.kind.as_str(), "movie" | "episode") && lib.visible(t))
        .collect();
    let types: Vec<&str> = q.list("includeitemtypes").iter().filter_map(|t| kind_of(t)).collect();
    if !types.is_empty() {
        list.retain(|(t, _)| types.contains(&t.kind.as_str()));
    }
    list.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    let total = list.len();
    let limit = q.int("limit").filter(|l| *l > 0).unwrap_or(12) as usize;
    Ok(result(list.into_iter().take(limit).map(|(t, _)| lib.dto(t)).collect(), total, 0))
}

async fn item(u: AppUser, State(state): State<AppState>, Path(id): Path<String>) -> Response {
    one(u, state, id).await
}

async fn user_item(
    u: AppUser,
    State(state): State<AppState>,
    Path((_, id)): Path<(String, String)>,
) -> Response {
    one(u, state, id).await
}

async fn one(u: AppUser, state: AppState, id: String) -> Response {
    let run = async {
        let sid = server_id(&state).await?;
        let found = match parse_guid(&id) {
            Some((IdKind::View, n)) if (FILMS_VIEW..=MUSIC_VIEW).contains(&n) => Some(view(&sid, n)),
            Some((IdKind::Title, n)) => {
                let lib = Library::load(&state, u.user.id).await?;
                match lib.titles.get(&n).filter(|t| lib.visible(t)) {
                    Some(t) => {
                        let mut v = lib.dto(t);
                        if let Some(f) = lib.file(t.id) {
                            // the details page shows the tracks; playback asks again
                            if let Ok(source) =
                                super::play::media_source(&state, &lib, t, f, &u.token, None).await
                            {
                                v["MediaSources"] = json!([source.clone()]);
                                v["MediaStreams"] = source["MediaStreams"].clone();
                            }
                        }
                        Some(v)
                    }
                    None => None,
                }
            }
            _ => None,
        };
        ApiResult::Ok(found)
    };
    match run.await {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn seasons(
    u: AppUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let show = parse_guid(&id).filter(|(k, _)| *k == IdKind::Title).map(|(_, n)| n);
    let list: Vec<Value> =
        show.map(|s| lib.seasons(Some(s))).unwrap_or_default().into_iter().map(|t| lib.dto(t)).collect();
    let n = list.len();
    Ok(result(list, n, 0))
}

async fn episodes(
    u: AppUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    q: Q,
) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let Some((IdKind::Title, show)) = parse_guid(&id) else { return Ok(result(vec![], 0, 0)) };
    let season = q.get("seasonid").and_then(parse_guid).map(|(_, n)| n);
    let number = q.int("season");
    let mut list: Vec<&Title> = lib.episodes(Some(show), season);
    if let Some(n) = number {
        list.retain(|e| e.season_number == Some(n));
    }
    if let Some(start) = q.get("startitemid").and_then(parse_guid).map(|(_, n)| n)
        && let Some(at) = list.iter().position(|e| e.id == start)
    {
        list.drain(..at);
    }
    let total = list.len();
    let start = q.int("startindex").unwrap_or(0).max(0) as usize;
    let limit = q.int("limit").filter(|l| *l > 0).map(|l| l as usize).unwrap_or(usize::MAX);
    Ok(result(list.into_iter().skip(start).take(limit).map(|t| lib.dto(t)).collect(), total, start))
}

/// For each show this user has started: the first unwatched episode after
/// the last one watched.
async fn next_up(u: AppUser, State(state): State<AppState>, q: Q) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let only = q.get("seriesid").and_then(parse_guid).map(|(_, n)| n);
    let mut found: Vec<(&Title, i64)> = Vec::new();
    for show in lib.shows() {
        if only.is_some_and(|o| o != show.id) {
            continue;
        }
        let eps = lib.episodes(Some(show.id), None);
        let touched = |e: &Title| lib.data.get(&e.id).is_some_and(|d| d.played || d.position_seconds > 0.0);
        let Some(last) = eps.iter().rposition(|e| lib.played(e.id)) else {
            // a show in progress whose first episode is not finished yet
            if let Some(e) = eps.iter().find(|e| touched(e)) {
                found.push((e, lib.data.get(&e.id).and_then(|d| d.last_played_at).unwrap_or(0)));
            }
            continue;
        };
        let when = lib.data.get(&eps[last].id).and_then(|d| d.last_played_at).unwrap_or(0);
        if let Some(next) = eps[last + 1..].iter().find(|e| !lib.played(e.id)) {
            found.push((next, when));
        }
    }
    found.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    let total = found.len();
    let limit = q.int("limit").filter(|l| *l > 0).unwrap_or(24) as usize;
    Ok(result(found.into_iter().take(limit).map(|(t, _)| lib.dto(t)).collect(), total, 0))
}

async fn search_hints(u: AppUser, State(state): State<AppState>, q: Q) -> ApiResult<Json<Value>> {
    let lib = Library::load(&state, u.user.id).await?;
    let term = q.get("searchterm").unwrap_or("").to_lowercase();
    let types: Vec<&str> = q.list("includeitemtypes").iter().filter_map(|t| kind_of(t)).collect();
    let mut list: Vec<&Title> = lib.movies();
    list.extend(lib.shows());
    list.extend(lib.episodes(None, None));
    list.retain(|t| !term.is_empty() && t.title.to_lowercase().contains(&term));
    if !types.is_empty() {
        list.retain(|t| types.contains(&t.kind.as_str()));
    }
    list.sort_by_key(|t| (t.kind == "episode", sort_name(&t.title)));
    let total = list.len();
    let limit = q.int("limit").filter(|l| *l > 0).unwrap_or(20) as usize;
    let hints: Vec<Value> = list
        .into_iter()
        .take(limit)
        .map(|t| {
            let d = lib.dto(t);
            json!({
                "ItemId": d["Id"], "Id": d["Id"], "Name": d["Name"], "Type": d["Type"],
                "ProductionYear": d["ProductionYear"], "PrimaryImageTag": d["ImageTags"]["Primary"],
                "MediaType": d["MediaType"], "RunTimeTicks": d["RunTimeTicks"], "IsFolder": d["IsFolder"],
                "Series": d["SeriesName"], "IndexNumber": d["IndexNumber"], "ParentIndexNumber": d["ParentIndexNumber"],
            })
        })
        .collect();
    Ok(Json(json!({ "SearchHints": hints, "TotalRecordCount": total })))
}

async fn image(State(state): State<AppState>, Path((id, kind)): Path<(String, String)>, q: Q) -> Response {
    artwork(&state, &id, &kind, &q).await
}

async fn image_at(
    State(state): State<AppState>,
    Path((id, kind, _)): Path<(String, String, String)>,
    q: Q,
) -> Response {
    artwork(&state, &id, &kind, &q).await
}

/// Artwork through the image cache. Apps load images in plain image views
/// that send no token, so artwork of titles in the library is public: the
/// pictures come from public catalogs.
async fn artwork(state: &AppState, id: &str, kind: &str, q: &Q) -> Response {
    let Some((IdKind::Title, n)) = parse_guid(id) else { return StatusCode::NOT_FOUND.into_response() };
    // only what apps can list: requested or wanted titles stay private
    if !moka_db::library::in_library(&state.db, n).await.unwrap_or(false) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(Some(t)) = moka_db::titles::by_id(&state.db, n).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let parent = |id: Option<i64>| async move {
        match id {
            Some(p) => moka_db::titles::by_id(&state.db, p).await.ok().flatten(),
            None => None,
        }
    };
    let show = match t.kind.as_str() {
        "season" => parent(t.parent_id).await,
        "episode" => match parent(t.parent_id).await {
            Some(s) => parent(s.parent_id).await,
            None => None,
        },
        _ => None,
    };
    // albums and their tracks: the album cover
    let album = match t.kind.as_str() {
        "album" => Some(t.clone()),
        "track" => parent(t.parent_id).await,
        _ => None,
    };
    if let Some(album) = album {
        let Some(mbid) = album.mbid.filter(|_| kind == "primary") else {
            return StatusCode::NOT_FOUND.into_response();
        };
        return match crate::api::images::cover_bytes(state, &mbid).await {
            Ok(Some(bytes)) => (
                [(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "public, max-age=604800")],
                bytes,
            )
                .into_response(),
            Ok(None) => StatusCode::NOT_FOUND.into_response(),
            Err(e) => e.into_response(),
        };
    }
    let backdrop = || t.backdrop_path.clone().or_else(|| show.as_ref().and_then(|s| s.backdrop_path.clone()));
    let path = match kind {
        "primary" => t.poster_path.clone().or_else(|| show.as_ref().and_then(|s| s.poster_path.clone())),
        "backdrop" | "thumb" | "art" | "banner" => backdrop(),
        _ => None,
    };
    let Some(path) = path else { return StatusCode::NOT_FOUND.into_response() };
    let wide = kind != "primary" || t.kind == "episode";
    let width = q.int("maxwidth").or_else(|| q.int("fillwidth")).or_else(|| q.int("width")).unwrap_or(0);
    let size = match (wide, width) {
        (false, 1..=200) => "w185",
        (false, 201..=360) => "w342",
        (false, 361..=520) => "w500",
        (false, _) => "w780",
        (true, 1..=800) => "w780",
        (true, _) => "w1280",
    };
    let Some(name) = moka_metadata::safe_image_name(path.trim_start_matches('/')) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match crate::api::images::tmdb_bytes(state, size, name).await {
        Ok(Some(bytes)) => (
            [
                (header::CONTENT_TYPE, mime_guess::from_path(name).first_or_octet_stream().to_string()),
                (header::CACHE_CONTROL, "public, max-age=2592000, immutable".to_owned()),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_tags() {
        assert_eq!(sort_name("The Matrix"), "matrix");
        assert_eq!(image_tag("/abc123.jpg").as_deref(), Some("abc123"));
        assert_eq!(kind_of("series"), Some("show"));
    }
}
