use super::*;
use axum::{Router, extract::Query, http::StatusCode, response::IntoResponse, routing::get};
use std::collections::HashMap;

const CAPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <server title="Test"/>
  <limits default="50" max="100"/>
  <searching>
    <search available="yes" supportedParams="q"/>
    <movie-search available="yes" supportedParams="q,imdbid,tmdbid"/>
    <tv-search available="no"/>
  </searching>
  <categories>
    <category id="2000" name="Movies">
      <subcat id="2040" name="Movies/HD"/>
    </category>
  </categories>
</caps>"#;

const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed">
<channel>
  <item>
    <title>The.Matrix.1999.1080p.BluRay.x264-GRP</title>
    <guid>https://idx.local/details/1</guid>
    <link>https://idx.local/dl/1.torrent</link>
    <comments>https://idx.local/details/1#comments</comments>
    <pubDate>Tue, 05 Mar 2024 10:00:00 +0000</pubDate>
    <size>9000000000</size>
    <enclosure url="https://idx.local/dl/1.torrent" length="9000000000" type="application/x-bittorrent"/>
    <torznab:attr name="seeders" value="120"/>
    <torznab:attr name="peers" value="130"/>
    <torznab:attr name="imdbid" value="0133093"/>
    <torznab:attr name="category" value="2040"/>
    <torznab:attr name="downloadvolumefactor" value="0"/>
    <torznab:attr name="infohash" value="C12FE1C06BBA254A9DC9F519B335AA7C1367A88A"/>
  </item>
  <item>
    <title>The Matrix 1999 720p WEB-DL</title>
    <link>magnet:?xt=urn:btih:YEX6DQDLXISUVHOJ6UM3GNNKPQJWPKEK</link>
    <torznab:attr name="size" value="3000000000"/>
  </item>
  <item>
    <title>No link at all</title>
  </item>
</channel>
</rss>"#;

async fn mock() -> String {
    let app = Router::new()
        .route(
            "/api",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                if q.get("apikey").map(String::as_str) != Some("good") {
                    return (StatusCode::OK, r#"<error code="100" description="Invalid API Key"/>"#)
                        .into_response();
                }
                match q.get("t").map(String::as_str) {
                    Some("caps") => CAPS.into_response(),
                    Some("movie") => {
                        assert_eq!(q.get("tmdbid").map(String::as_str), Some("603"));
                        assert_eq!(q.get("imdbid").map(String::as_str), Some("0133093"));
                        assert!(!q.contains_key("q"), "ids replace the text query");
                        FEED.into_response()
                    }
                    _ => StatusCode::BAD_REQUEST.into_response(),
                }
            }),
        )
        .route("/limited", get(|| async { (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "30")], "") }))
        .route(
            "/redirect",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [("location", "magnet:?xt=urn:btih:C12FE1C06BBA254A9DC9F519B335AA7C1367A88A")],
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn cfg(base: &str, key: &str) -> IndexerConfig {
    IndexerConfig {
        kind: IndexerKind::Torznab,
        base_url: base.to_owned(),
        api_path: "api".into(),
        api_key: Some(key.into()),
        categories: vec![2000],
    }
}

#[tokio::test]
async fn caps_then_search_by_ids() {
    let base = mock().await;
    let c = Client::new();
    let caps = c.capabilities(&cfg(&base, "good")).await.unwrap();
    assert!(caps.movie_search);
    assert_eq!(caps.movie_params, vec!["q", "imdbid", "tmdbid"]);
    assert_eq!(caps.limit_max, Some(100));
    assert_eq!(caps.categories.len(), 2);

    let q = MovieQuery {
        title: "The Matrix".into(),
        year: Some(1999),
        imdb_id: Some("tt0133093".into()),
        tmdb_id: Some(603),
    };
    let releases = c.search_movie(&cfg(&base, "good"), &caps, &q).await.unwrap();
    assert_eq!(releases.len(), 2, "items without any link are dropped");

    let r = &releases[0];
    assert_eq!(r.size_bytes, Some(9_000_000_000));
    assert_eq!(r.seeders, Some(120));
    assert_eq!(r.imdb_id.as_deref(), Some("tt0133093"));
    assert_eq!(r.info_hash.as_deref(), Some("c12fe1c06bba254a9dc9f519b335aa7c1367a88a"));
    assert!(r.freeleech);
    assert_eq!(r.published_at, Some(1_709_632_800));
    assert_eq!(r.info_url.as_deref(), Some("https://idx.local/details/1"));

    let m = &releases[1];
    assert!(m.download_url.is_none());
    assert_eq!(m.info_hash.as_deref(), Some("c12fe1c06bba254a9dc9f519b335aa7c1367a88a"));
    assert_eq!(m.size_bytes, Some(3_000_000_000));
}

#[tokio::test]
async fn errors_are_classified() {
    let base = mock().await;
    let c = Client::new();
    let err = c.capabilities(&cfg(&base, "bad")).await.unwrap_err();
    assert!(matches!(err, Error::Auth));
    assert!(err.is_config_problem());

    let mut limited = cfg(&base, "good");
    limited.api_path = "/limited".into();
    let err = c.capabilities(&limited).await.unwrap_err();
    assert!(matches!(err, Error::RateLimited { retry_after: Some(d) } if d.as_secs() == 30));
}

#[tokio::test]
async fn fetch_follows_redirect_to_magnet() {
    let base = mock().await;
    let p = Client::new().fetch(&format!("{base}/redirect")).await.unwrap();
    assert!(matches!(&p, torrent::Payload::Magnet(_)));
    assert_eq!(p.info_hash().as_deref(), Some("c12fe1c06bba254a9dc9f519b335aa7c1367a88a"));
}

#[test]
fn text_search_drops_punctuation() {
    assert_eq!(search_text("Mission: Impossible - Fallout 2018"), "Mission Impossible Fallout 2018");
}

#[test]
fn backoff_grows_and_caps() {
    assert_eq!(backoff(0).as_secs(), 0);
    assert_eq!(backoff(1).as_secs(), 60);
    assert_eq!(backoff(3).as_secs(), 900);
    assert_eq!(backoff(99).as_secs(), 86_400);
}

#[tokio::test]
async fn reads_prowlarr_indexers() {
    use axum::http::HeaderMap;
    let app = Router::new().route(
        "/api/v1/indexer",
        get(|h: HeaderMap| async move {
            if h.get("x-api-key").and_then(|v| v.to_str().ok()) != Some("pk") {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(axum::Json(serde_json::json!([
                { "id": 1, "name": "Tracker A", "enable": true, "protocol": "torrent",
                  "capabilities": { "categories": [ { "id": 2000, "subCategories": [ { "id": 2040 }, { "id": 2045 } ] }, { "id": 5000 } ] } },
                { "id": 2, "name": "Usenet B", "enable": false, "protocol": "usenet" },
                { "id": 3, "name": "Odd", "enable": true, "protocol": "other" }
            ])))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let c = Client::new();
    let list = c.prowlarr_indexers(&base, "pk").await.unwrap();
    assert_eq!(list.len(), 2, "unknown protocols are skipped");
    assert_eq!(list[0].movie_categories, vec![2000, 2040, 2045]);
    assert_eq!(list[1].protocol, Protocol::Usenet);
    assert!(!list[1].enabled);
    assert!(matches!(c.prowlarr_indexers(&base, "bad").await, Err(Error::Auth)));
}
