//! An app signs in, browses a film and a show, plays, and leaves its place.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use moka_core::{Permissions, TitleState};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{IdKind, guid};
use crate::state::AppState;

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header(
            "x-emby-authorization",
            format!(r#"MediaBrowser Client="Test", Device="Pc", DeviceId="d1", Version="1", Token="{t}""#),
        );
    }
    let body = if body.is_null() { Body::empty() } else { Body::from(body.to_string()) };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn an_app_browses_plays_and_resumes() {
    let root = std::env::temp_dir().join(format!("moka-jf-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let film_file = root.join("Arrival.mkv");
    std::fs::write(&film_file, b"film bytes").unwrap();

    let db = moka_db::memory().await;
    let pw = crate::auth::hash_password("password1").unwrap();
    moka_db::users::create(&db, "ana", &pw, Permissions::DEFAULT_USER).await.unwrap();
    let film = moka_db::titles::upsert_movie(
        &db,
        &moka_db::titles::NewTitle {
            tmdb_id: 329865,
            title: "Arrival".into(),
            year: Some(2016),
            runtime_minutes: Some(116),
            poster_path: Some("/poster.jpg".into()),
            ..Default::default()
        },
        TitleState::Available,
    )
    .await
    .unwrap();
    moka_db::library::add_file(&db, film.id, &film_file.to_string_lossy(), 10, Some("bluray-1080p"), None)
        .await
        .unwrap();
    // a film still wanted is not shown
    moka_db::titles::upsert_movie(
        &db,
        &moka_db::titles::NewTitle { tmdb_id: 1, title: "Not Here".into(), ..Default::default() },
        TitleState::Wanted,
    )
    .await
    .unwrap();
    let episode = |n: i64| moka_db::titles::NewEpisode {
        title: moka_db::titles::NewTitle {
            tmdb_id: 900 + n,
            title: format!("Episode {n}"),
            ..Default::default()
        },
        number: n,
    };
    let show = moka_db::titles::upsert_show(
        &db,
        &moka_db::titles::NewShow {
            show: moka_db::titles::NewTitle { tmdb_id: 90, title: "Test Show".into(), ..Default::default() },
            tvdb_id: None,
            seasons: vec![moka_db::titles::NewSeason {
                title: moka_db::titles::NewTitle {
                    tmdb_id: 91,
                    title: "Season 1".into(),
                    ..Default::default()
                },
                number: 1,
                episodes: vec![episode(1), episode(2)],
            }],
        },
        TitleState::Wanted,
    )
    .await
    .unwrap();
    let eps = moka_db::titles::episodes_of_show(&db, show.id).await.unwrap();
    for e in &eps {
        moka_db::library::add_file(
            &db,
            e.id,
            &root.join(format!("e{}.mkv", e.id)).to_string_lossy(),
            5,
            None,
            None,
        )
        .await
        .unwrap();
    }

    let config = crate::config::Config {
        config_dir: root.clone(),
        listen: "127.0.0.1:0".parse().unwrap(),
        database_url: String::new(),
    };
    let app = crate::app(AppState::new(db.clone(), config, Default::default()));

    let (s, info) = send(&app, "GET", "/jellyfin/System/Info/Public", None, Value::Null).await;
    assert_eq!((s, info["Version"].as_str()), (StatusCode::OK, Some(super::VERSION)));
    assert_eq!(send(&app, "GET", "/jellyfin/system/info/public", None, Value::Null).await.0, StatusCode::OK);
    assert_eq!(send(&app, "GET", "/jellyfin/Items", None, Value::Null).await.0, StatusCode::UNAUTHORIZED);

    let (s, _) = send(
        &app,
        "POST",
        "/jellyfin/Users/AuthenticateByName",
        None,
        json!({ "Username": "ana", "Pw": "nope" }),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, auth) = send(
        &app,
        "POST",
        "/jellyfin/Users/AuthenticateByName",
        None,
        json!({ "Username": "ana", "Pw": "password1" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let token = auth["AccessToken"].as_str().unwrap().to_owned();
    let t = Some(token.as_str());
    let user = auth["User"]["Id"].as_str().unwrap().to_owned();
    assert_eq!(auth["User"]["Policy"]["IsAdministrator"], false);

    let (_, views) = send(&app, "GET", &format!("/jellyfin/Users/{user}/Views"), t, Value::Null).await;
    assert_eq!(views["TotalRecordCount"], 2);
    let films_view = views["Items"][0]["Id"].as_str().unwrap().to_owned();

    let (_, films) = send(
        &app,
        "GET",
        &format!("/jellyfin/Items?ParentId={films_view}&Recursive=true&SortBy=SortName"),
        t,
        Value::Null,
    )
    .await;
    assert_eq!(films["TotalRecordCount"], 1, "only films with a file");
    let item = &films["Items"][0];
    assert_eq!((item["Type"].as_str(), item["Name"].as_str()), (Some("Movie"), Some("Arrival")));
    assert_eq!(item["ImageTags"]["Primary"], "poster");
    assert_eq!(item["RunTimeTicks"], 116 * 60 * 10_000_000i64);
    let film_id = guid(IdKind::Title, film.id);

    let show_id = guid(IdKind::Title, show.id);
    let (_, series) =
        send(&app, "GET", "/jellyfin/Items?IncludeItemTypes=Series&Recursive=true", t, Value::Null).await;
    assert_eq!(series["Items"][0]["Id"], show_id.as_str());
    assert_eq!(series["Items"][0]["UserData"]["UnplayedItemCount"], 2);
    let (_, seasons) = send(&app, "GET", &format!("/jellyfin/Shows/{show_id}/Seasons"), t, Value::Null).await;
    assert_eq!(seasons["TotalRecordCount"], 1);
    let (_, episodes) =
        send(&app, "GET", &format!("/jellyfin/Shows/{show_id}/Episodes"), t, Value::Null).await;
    let names: Vec<&str> =
        episodes["Items"].as_array().unwrap().iter().map(|e| e["Name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Episode 1", "Episode 2"]);
    assert_eq!(episodes["Items"][0]["SeriesId"], show_id.as_str());

    // half way through the film, then the first episode watched
    let (s, _) = send(
        &app,
        "POST",
        "/jellyfin/Sessions/Playing/Progress",
        t,
        json!({ "ItemId": film_id, "PositionTicks": 600 * 10_000_000i64 }),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, resume) = send(&app, "GET", "/jellyfin/UserItems/Resume", t, Value::Null).await;
    assert_eq!(resume["Items"][0]["UserData"]["PlaybackPositionTicks"], 600 * 10_000_000i64);
    let ep1 = episodes["Items"][0]["Id"].as_str().unwrap().to_owned();
    let (_, data) = send(&app, "POST", &format!("/jellyfin/UserPlayedItems/{ep1}"), t, Value::Null).await;
    assert_eq!(data["Played"], true);
    let (_, next) = send(&app, "GET", "/jellyfin/Shows/NextUp", t, Value::Null).await;
    assert_eq!(next["Items"][0]["Name"], "Episode 2");

    // the file itself, with the token in the query as players send it
    let res = app
        .clone()
        .oneshot(
            Request::get(format!("/jellyfin/Videos/{film_id}/stream?static=true&api_key={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(&res.into_body().collect().await.unwrap().to_bytes()[..], b"film bytes");

    let (s, _) = send(&app, "POST", "/jellyfin/Sessions/Logout", t, Value::Null).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "GET", "/jellyfin/Users/Me", t, Value::Null).await.0, StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(&root);
}

/// A real file: an app that cannot play MKV gets a converted HLS stream.
#[tokio::test]
#[ignore]
async fn ffmpeg_apps_get_a_converted_stream() {
    let root = std::env::temp_dir().join(format!("moka-jf-ffmpeg-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let clip = root.join("clip.mkv");
    let ok = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=6:size=320x240:rate=25",
        ])
        .args(["-f", "lavfi", "-i", "sine=duration=6", "-c:v", "libx264", "-c:a", "ac3", "-shortest"])
        .arg(&clip)
        .status()
        .await
        .unwrap();
    assert!(ok.success());

    let db = moka_db::memory().await;
    let pw = crate::auth::hash_password("password1").unwrap();
    moka_db::users::create(&db, "ana", &pw, Permissions::DEFAULT_USER).await.unwrap();
    let film = moka_db::titles::upsert_movie(
        &db,
        &moka_db::titles::NewTitle { tmdb_id: 5, title: "Clip".into(), ..Default::default() },
        TitleState::Available,
    )
    .await
    .unwrap();
    moka_db::library::add_file(&db, film.id, &clip.to_string_lossy(), 1000, None, None).await.unwrap();
    let config = crate::config::Config {
        config_dir: root.clone(),
        listen: "127.0.0.1:0".parse().unwrap(),
        database_url: String::new(),
    };
    let app = crate::app(AppState::new(db, config, Default::default()));
    let (_, auth) = send(
        &app,
        "POST",
        "/jellyfin/Users/AuthenticateByName",
        None,
        json!({ "Username": "ana", "Pw": "password1" }),
    )
    .await;
    let token = auth["AccessToken"].as_str().unwrap().to_owned();
    let t = Some(token.as_str());
    let id = guid(IdKind::Title, film.id);

    let profile = json!({ "DeviceProfile": {
        "DirectPlayProfiles": [{ "Type": "Video", "Container": "mp4", "VideoCodec": "h264", "AudioCodec": "aac" }],
        "TranscodingProfiles": [{ "Type": "Video", "Container": "ts", "Protocol": "hls", "VideoCodec": "h264", "AudioCodec": "aac" }] } });
    let (s, info) = send(&app, "POST", &format!("/jellyfin/Items/{id}/PlaybackInfo"), t, profile).await;
    assert_eq!(s, StatusCode::OK, "{info}");
    let source = &info["MediaSources"][0];
    assert_eq!(
        (source["Container"].as_str(), source["SupportsDirectPlay"].as_bool()),
        (Some("mkv"), Some(false))
    );
    let streams: Vec<&str> =
        source["MediaStreams"].as_array().unwrap().iter().map(|s| s["Type"].as_str().unwrap()).collect();
    assert_eq!(streams, ["Video", "Audio"]);
    let url = source["TranscodingUrl"].as_str().unwrap();

    let get = |uri: String| {
        let app = app.clone();
        async move {
            let res = app.oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
            let status = res.status();
            (
                status,
                String::from_utf8_lossy(&res.into_body().collect().await.unwrap().to_bytes()).into_owned(),
            )
        }
    };
    let (s, master) = get(format!("/jellyfin{url}")).await;
    assert_eq!(s, StatusCode::OK, "{master}");
    let variant = master.lines().find(|l| l.starts_with("hls/")).unwrap().to_owned();
    let (s, playlist) = get(format!("/jellyfin/videos/{id}/{variant}")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(playlist.contains("init.mp4"), "{playlist}");
    let init = variant.replace("index.m3u8", "init.mp4");
    assert_eq!(get(format!("/jellyfin/videos/{id}/{init}")).await.0, StatusCode::OK);

    let play = info["PlaySessionId"].as_str().unwrap();
    let (s, _) = send(
        &app,
        "POST",
        "/jellyfin/Sessions/Playing/Stopped",
        t,
        json!({ "ItemId": id, "PlaySessionId": play, "PositionTicks": 0 }),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        get(format!("/jellyfin/videos/{id}/{variant}")).await.0,
        StatusCode::NOT_FOUND,
        "stopping ends the conversion"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn music_apps_list_artists_albums_and_tracks() {
    let root = std::env::temp_dir().join(format!("moka-jf-music-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let db = moka_db::memory().await;
    let pw = crate::auth::hash_password("password1").unwrap();
    moka_db::users::create(&db, "ana", &pw, Permissions::DEFAULT_USER).await.unwrap();
    let album = moka_db::titles::NewAlbum {
        artist_mbid: "artist-1".into(),
        artist: "Test Band".into(),
        mbid: "11111111-2222-3333-4444-555555555555".into(),
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
    let a = moka_db::titles::upsert_album(&db, &album, TitleState::Available).await.unwrap();
    let tracks = moka_db::titles::children(&db, a.id).await.unwrap();
    for t in &tracks[..2] {
        let f = root.join(format!("{}.flac", t.id));
        std::fs::write(&f, b"flac").unwrap();
        moka_db::library::add_file(&db, t.id, &f.to_string_lossy(), 4, Some("flac"), None).await.unwrap();
    }
    let config = crate::config::Config {
        config_dir: root.clone(),
        listen: "127.0.0.1:0".parse().unwrap(),
        database_url: String::new(),
    };
    let app = crate::app(AppState::new(db, config, Default::default()));
    let (_, auth) = send(
        &app,
        "POST",
        "/jellyfin/Users/AuthenticateByName",
        None,
        json!({ "Username": "ana", "Pw": "password1" }),
    )
    .await;
    let token = auth["AccessToken"].as_str().unwrap().to_owned();
    let t = Some(token.as_str());

    let (_, views) = send(&app, "GET", "/jellyfin/UserViews", t, Value::Null).await;
    assert_eq!(views["Items"][2]["CollectionType"], "music");
    let music = views["Items"][2]["Id"].as_str().unwrap().to_owned();

    let (_, artists) = send(&app, "GET", "/jellyfin/Artists/AlbumArtists", t, Value::Null).await;
    assert_eq!(artists["Items"][0]["Name"], "Test Band");
    let artist = artists["Items"][0]["Id"].as_str().unwrap().to_owned();
    let (_, albums) = send(
        &app,
        "GET",
        &format!("/jellyfin/Items?IncludeItemTypes=MusicAlbum&Recursive=true&AlbumArtistIds={artist}"),
        t,
        Value::Null,
    )
    .await;
    let al = &albums["Items"][0];
    assert_eq!((al["Type"].as_str(), al["AlbumArtist"].as_str()), (Some("MusicAlbum"), Some("Test Band")));
    assert_eq!(al["ImageTags"]["Primary"], "11111111222233334444555555555555");
    let (_, latest) =
        send(&app, "GET", &format!("/jellyfin/Items/Latest?ParentId={music}"), t, Value::Null).await;
    assert_eq!(latest[0]["Name"], "First Light");

    let album_id = al["Id"].as_str().unwrap().to_owned();
    let (_, songs) = send(&app, "GET", &format!("/jellyfin/Items?ParentId={album_id}"), t, Value::Null).await;
    let view: Vec<(&str, i64)> = songs["Items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["Name"].as_str().unwrap(), s["IndexNumber"].as_i64().unwrap()))
        .collect();
    assert_eq!(view, [("Opening", 1), ("Middle", 2)], "only tracks with a file");
    assert_eq!(
        (songs["Items"][0]["Type"].as_str(), songs["Items"][0]["MediaType"].as_str()),
        (Some("Audio"), Some("Audio"))
    );

    let song = songs["Items"][0]["Id"].as_str().unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::get(format!("/jellyfin/Audio/{song}/universal?api_key={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let _ = std::fs::remove_dir_all(&root);
}
