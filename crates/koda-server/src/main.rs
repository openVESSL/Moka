use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

mod api;
mod auth;
mod config;
mod error;
mod jellyfin;
mod notify;
mod pipeline;
mod radarr;
mod scan;
mod series;
mod state;
mod streaming;
mod subtitles;
mod web;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `koda openapi` prints the API document (used to generate the web client).
    if std::env::args().nth(1).as_deref() == Some("openapi") {
        use utoipa::OpenApi;
        println!("{}", api::ApiDoc::openapi().to_pretty_json()?);
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn")),
        )
        .init();

    let config = Config::from_env()?;
    std::fs::create_dir_all(&config.config_dir)?;
    std::fs::create_dir_all(config.cache_dir())?;
    // leftovers from a previous run are never resumed
    let _ = std::fs::remove_dir_all(config.transcode_dir());
    std::fs::create_dir_all(config.transcode_dir())?;

    koda_db::secret::init(config::secret_key(&config.config_dir)?);
    let db = koda_db::connect(&config.database_url).await?;
    let sealed = koda_db::secret::seal_legacy(&db).await?;
    if sealed > 0 {
        tracing::info!(count = sealed, "encrypted secrets stored by an older version");
    }
    let catalog = state::CatalogSettings::load(&db).await?;
    // re-save so a key stored in plain text by an older version gets encrypted
    catalog.save(&db).await?;
    let listen = config.listen;
    let state = AppState::new(db, config, catalog);
    tokio::spawn(pipeline::run(state.clone()));
    tokio::spawn(state.streams.clone().reap());
    tokio::spawn(jellyfin::run(state.clone()));
    tokio::spawn(radarr::run(state.clone()));

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!("listening on http://{listen}");
    axum::serve(listener, app(state)).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
}

/// Same-origin only. `blob:` is needed for HLS playback, wasm for the subtitle renderer.
const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; \
    script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; font-src 'self' data:; \
    connect-src 'self'; worker-src 'self' blob:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub fn app(state: AppState) -> axum::Router {
    use axum::http::{HeaderName, HeaderValue, header};
    use tower_http::set_header::SetResponseHeaderLayer;
    let set = |name: HeaderName, value: &'static str| {
        SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
    };
    api::router()
        .fallback(web::serve)
        .layer(set(header::CONTENT_SECURITY_POLICY, CSP))
        .layer(set(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(set(header::X_FRAME_OPTIONS, "DENY"))
        .layer(set(header::REFERRER_POLICY, "same-origin"))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    async fn test_app() -> axum::Router {
        let config = Config {
            config_dir: std::env::temp_dir(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: "sqlite::memory:".into(),
        };
        app(AppState::new(koda_db::memory().await, config, Default::default()))
    }

    fn json(method: &str, uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap()
    }

    #[tokio::test]
    async fn setup_login_me_logout() {
        let app = test_app().await;

        let res = app.clone().oneshot(json("GET", "/api/v1/auth/status", "")).await.unwrap();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], br#"{"needs_setup":true}"#);

        let res = app.clone().oneshot(json("GET", "/api/v1/me", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let short = json("POST", "/api/v1/auth/setup", r#"{"username":"ana","password":"short"}"#);
        assert_eq!(app.clone().oneshot(short).await.unwrap().status(), StatusCode::BAD_REQUEST);

        let creds = r#"{"username":"ana","password":"long enough"}"#;
        let res = app.clone().oneshot(json("POST", "/api/v1/auth/setup", creds)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let cookie = res.headers()[header::SET_COOKIE].to_str().unwrap().to_owned();
        assert!(cookie.contains("HttpOnly"));

        let again = app.clone().oneshot(json("POST", "/api/v1/auth/setup", creds)).await.unwrap();
        assert_eq!(again.status(), StatusCode::CONFLICT);

        let bad = r#"{"username":"ana","password":"wrong password"}"#;
        let res = app.clone().oneshot(json("POST", "/api/v1/auth/login", bad)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let session = cookie.split(';').next().unwrap().to_owned();
        let me = Request::builder()
            .uri("/api/v1/me")
            .header(header::COOKIE, &session)
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(me).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains(r#""username":"ana""#));

        let logout = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/logout")
            .header(header::COOKIE, &session)
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(logout).await.unwrap();
        let me = Request::builder()
            .uri("/api/v1/me")
            .header(header::COOKIE, &session)
            .body(Body::empty())
            .unwrap();
        assert_eq!(app.oneshot(me).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    }

    /// Logs a user in through the API and returns the session cookie.
    async fn login(app: &axum::Router, user: &str, pass: &str) -> String {
        let body = format!(r#"{{"username":"{user}","password":"{pass}"}}"#);
        let res = app.clone().oneshot(json("POST", "/api/v1/auth/login", &body)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "login {user}");
        let c = res.headers()[header::SET_COOKIE].to_str().unwrap();
        c.split(';').next().unwrap().to_owned()
    }

    fn authed(method: &str, uri: &str, cookie: &str, body: &str) -> Request<Body> {
        let mut r = json(method, uri, body);
        r.headers_mut().insert(header::COOKIE, cookie.parse().unwrap());
        r
    }

    async fn body_json(res: axum::response::Response) -> serde_json::Value {
        let b = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&b).unwrap()
    }

    #[tokio::test]
    async fn request_needs_approval_then_becomes_wanted() {
        // fake TMDB serving one film
        let tmdb = axum::Router::new().route(
            "/movie/603",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "id": 603, "title": "The Matrix", "release_date": "1999-03-30", "runtime": 136
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, tmdb).await.unwrap() });

        let db = koda_db::memory().await;
        let pw = auth::hash_password("password1").unwrap();
        koda_db::users::create(&db, "admin", &pw, koda_core::Permissions::ADMIN).await.unwrap();
        koda_db::users::create(&db, "guest", &pw, koda_core::Permissions::DEFAULT_USER).await.unwrap();
        let config = Config {
            config_dir: std::env::temp_dir(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: "sqlite::memory:".into(),
        };
        let state = AppState::new(db, config, Default::default());
        state.set_tmdb(koda_metadata::Tmdb::with_base_url(&base, "k", "en-US", None));
        let app = app(state);

        let guest = login(&app, "guest", "password1").await;
        let admin = login(&app, "admin", "password1").await;

        let res = app
            .clone()
            .oneshot(authed("POST", "/api/v1/requests", &guest, r#"{"tmdb_id":603}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let req = body_json(res).await;
        assert_eq!(req["status"], "pending");
        assert_eq!(req["title_state"], "requested");
        let id = req["id"].as_i64().unwrap();

        // guests cannot approve
        let res = app
            .clone()
            .oneshot(authed("POST", &format!("/api/v1/requests/{id}/approve"), &guest, "{}"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        let res = app
            .clone()
            .oneshot(authed("POST", &format!("/api/v1/requests/{id}/approve"), &admin, "{}"))
            .await
            .unwrap();
        let req = body_json(res).await;
        assert_eq!(req["status"], "approved");
        assert_eq!(req["title_state"], "wanted");

        // the guest only sees their own requests; the admin sees all
        let res = app.clone().oneshot(authed("GET", "/api/v1/requests", &guest, "")).await.unwrap();
        assert_eq!(body_json(res).await.as_array().unwrap().len(), 1);

        // the admin asking for the same film joins it instead of failing
        let res = app
            .clone()
            .oneshot(authed("POST", "/api/v1/requests", &admin, r#"{"tmdb_id":603}"#))
            .await
            .unwrap();
        let req = body_json(res).await;
        assert_eq!(req["status"], "approved");
        let res =
            app.clone().oneshot(authed("GET", "/api/v1/titles?state=wanted", &admin, "")).await.unwrap();
        assert_eq!(body_json(res).await.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn shows_are_requested_whole() {
        // fake TMDB serving one show with specials and one season
        let tmdb = axum::Router::new()
            .route(
                "/tv/1399",
                axum::routing::get(|| async {
                    axum::Json(serde_json::json!({
                        "id": 1399, "name": "Thrones", "first_air_date": "2011-04-17",
                        "external_ids": { "tvdb_id": 121361 },
                        "seasons": [{"id": 30, "season_number": 0, "name": "Specials", "episode_count": 1},
                                    {"id": 31, "season_number": 1, "name": "Season 1", "episode_count": 2}]
                    }))
                }),
            )
            .route(
                "/tv/1399/season/{n}",
                axum::routing::get(|axum::extract::Path(n): axum::extract::Path<i64>| async move {
                    let eps: Vec<_> = (1..=if n == 0 { 1 } else { 2 })
                        .map(|e| {
                            serde_json::json!({"id": 300 + n * 10 + e, "season_number": n,
                                                     "episode_number": e, "name": format!("Ep {e}")})
                        })
                        .collect();
                    axum::Json(
                        serde_json::json!({"id": 30 + n, "season_number": n, "name": "S", "episodes": eps}),
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, tmdb).await.unwrap() });

        let db = koda_db::memory().await;
        let pw = auth::hash_password("password1").unwrap();
        koda_db::users::create(&db, "admin", &pw, koda_core::Permissions::ADMIN).await.unwrap();
        koda_db::users::create(&db, "guest", &pw, koda_core::Permissions::DEFAULT_USER).await.unwrap();
        let config = Config {
            config_dir: std::env::temp_dir(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: "sqlite::memory:".into(),
        };
        let state = AppState::new(db, config, Default::default());
        state.set_tmdb(koda_metadata::Tmdb::with_base_url(&base, "k", "en-US", None));
        let app = app(state);
        let guest = login(&app, "guest", "password1").await;
        let admin = login(&app, "admin", "password1").await;

        let res = app
            .clone()
            .oneshot(authed("POST", "/api/v1/requests", &guest, r#"{"tmdb_id":1399,"kind":"show"}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let req = body_json(res).await;
        assert_eq!((req["kind"].as_str(), req["title_state"].as_str()), (Some("show"), Some("requested")));
        let (id, show) = (req["id"].as_i64().unwrap(), req["title_id"].as_i64().unwrap());

        app.clone()
            .oneshot(authed("POST", &format!("/api/v1/requests/{id}/approve"), &admin, "{}"))
            .await
            .unwrap();
        let res = app
            .clone()
            .oneshot(authed("GET", &format!("/api/v1/titles/{show}/episodes"), &guest, ""))
            .await
            .unwrap();
        let eps = body_json(res).await;
        let eps = eps.as_array().unwrap();
        assert_eq!(eps.len(), 3);
        assert!(eps.iter().all(|e| e["state"] == "wanted"));
        assert_eq!((eps[0]["season_number"].as_i64(), eps[0]["monitored"].as_bool()), (Some(0), Some(false)));
        assert_eq!((eps[2]["episode_number"].as_i64(), eps[2]["monitored"].as_bool()), (Some(2), Some(true)));

        // shows are listed apart from films
        let res = app.clone().oneshot(authed("GET", "/api/v1/titles?kind=show", &guest, "")).await.unwrap();
        assert_eq!(body_json(res).await.as_array().unwrap().len(), 1);
        let res = app.clone().oneshot(authed("GET", "/api/v1/titles", &guest, "")).await.unwrap();
        assert!(body_json(res).await.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admin_manages_users_but_keeps_one_admin() {
        let db = koda_db::memory().await;
        let pw = auth::hash_password("password1").unwrap();
        let admin = koda_db::users::create(&db, "admin", &pw, koda_core::Permissions::ADMIN).await.unwrap();
        let config = Config {
            config_dir: std::env::temp_dir(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let app = app(AppState::new(db, config, Default::default()));
        let c = login(&app, "admin", "password1").await;

        let res = app
            .clone()
            .oneshot(authed("POST", "/api/v1/users", &c, r#"{"username":"bob","password":"bob-password"}"#))
            .await
            .unwrap();
        let bob = body_json(res).await;
        assert_eq!(bob["permissions"], 36, "request + play by default");
        login(&app, "bob", "bob-password").await;

        let res = app
            .clone()
            .oneshot(authed("POST", "/api/v1/users", &c, r#"{"username":"BOB","password":"another-one"}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);

        let demote = format!("/api/v1/users/{}", admin.id);
        let res = app.clone().oneshot(authed("PUT", &demote, &c, r#"{"permissions":32}"#)).await.unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT, "last admin cannot be demoted");

        let res = app.clone().oneshot(authed("DELETE", &demote, &c, "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT, "cannot delete yourself");
    }

    #[tokio::test]
    async fn repeated_bad_logins_are_throttled() {
        let db = koda_db::memory().await;
        let pw = auth::hash_password("password1").unwrap();
        koda_db::users::create(&db, "ana", &pw, koda_core::Permissions::ADMIN).await.unwrap();
        let config = Config {
            config_dir: std::env::temp_dir(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
        };
        let app = app(AppState::new(db, config, Default::default()));
        let bad = r#"{"username":"ana","password":"wrong-one"}"#;
        for _ in 0..5 {
            let res = app.clone().oneshot(json("POST", "/api/v1/auth/login", bad)).await.unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
        let good = r#"{"username":"ANA","password":"password1"}"#;
        let res = app.clone().oneshot(json("POST", "/api/v1/auth/login", good)).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS, "locked even with the right password");
    }

    /// Every route except a short public list refuses anonymous callers.
    #[tokio::test]
    async fn every_route_requires_sign_in() {
        use utoipa::OpenApi;
        let app = test_app().await;
        let public = [
            "GET /api/v1/health",
            "GET /api/v1/auth/status",
            "POST /api/v1/auth/setup",
            "POST /api/v1/auth/login",
            "POST /api/v1/auth/logout",
        ];
        let doc = serde_json::to_value(api::ApiDoc::openapi()).unwrap();
        let mut routes: Vec<(String, String)> = Vec::new();
        for (path, item) in doc["paths"].as_object().unwrap() {
            for method in item.as_object().unwrap().keys() {
                routes.push((method.to_uppercase(), path.clone()));
            }
        }
        // routes that are not in the API document
        for extra in [
            "GET /api/v1/events",
            "GET /api/v1/images/w342/abc.jpg",
            "GET /api/v1/files/1/stream",
            "GET /api/v1/hls/abc/index.m3u8",
            "GET /api/v3/system/status",
            "GET /api/v3/indexer",
            "POST /api/v3/indexer",
            "GET /api/v3/indexer/schema",
            "POST /api/v3/indexer/test",
            "GET /api/v3/indexer/1",
            "PUT /api/v3/indexer/1",
            "DELETE /api/v3/indexer/1",
            "GET /api/v3/qualityProfile",
            "GET /api/v3/rootfolder",
            "GET /api/v3/tag",
            "GET /api/v3/movie",
            "POST /api/v3/movie",
            "GET /api/v3/movie/lookup?term=tmdb:1",
            "GET /api/v3/movie/1",
            "PUT /api/v3/movie/1",
            "POST /api/v3/command",
            "GET /api/v3/queue",
            "GET /api/v3/history?movieIds=1",
            "GET /api/system/status",
        ] {
            let (m, p) = extra.split_once(' ').unwrap();
            routes.push((m.into(), p.into()));
        }
        assert!(routes.len() > 40, "found {} routes", routes.len());

        for (method, path) in routes {
            let key = format!("{method} {path}");
            if public.contains(&key.as_str()) {
                continue;
            }
            // fill path parameters with something plausible
            let uri: String = path
                .split('/')
                .map(|seg| if seg.starts_with('{') { "1" } else { seg })
                .collect::<Vec<_>>()
                .join("/");
            let res = app.clone().oneshot(json(&method, &uri, "{}")).await.unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{key} answered without sign-in");
        }
    }

    #[tokio::test]
    async fn secrets_never_come_back_from_the_api() {
        let app = test_app().await;
        let creds = r#"{"username":"ana","password":"long enough"}"#;
        let res = app.clone().oneshot(json("POST", "/api/v1/auth/setup", creds)).await.unwrap();
        let c = res.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();
        let body =
            r#"{"name":"x","kind":"torznab","base_url":"http://idx.local","api_key":"SUPER-SECRET-KEY"}"#;
        let res = app.clone().oneshot(authed("POST", "/api/v1/indexers", &c, body)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let meta = r#"{"name":"m","kind":"torznab","base_url":"http://169.254.169.254/latest"}"#;
        let res = app.clone().oneshot(authed("POST", "/api/v1/indexers", &c, meta)).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "cloud metadata address refused");
        let body = r#"{"name":"q","base_url":"http://q.local","username":"u","password":"SUPER-SECRET-PW"}"#;
        app.clone().oneshot(authed("POST", "/api/v1/download-clients", &c, body)).await.unwrap();
        for uri in ["/api/v1/indexers", "/api/v1/download-clients"] {
            let res = app.clone().oneshot(authed("GET", uri, &c, "")).await.unwrap();
            let text =
                String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
            assert!(!text.contains("SUPER-SECRET"), "{uri} leaked a secret: {text}");
            assert!(text.contains("_set\":true"), "{uri} should say a secret is stored");
        }
    }

    #[tokio::test]
    async fn responses_carry_security_headers() {
        let res = test_app().await.oneshot(json("GET", "/api/v1/health", "")).await.unwrap();
        let h = res.headers();
        assert_eq!(h["x-frame-options"], "DENY");
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert!(h["content-security-policy"].to_str().unwrap().contains("frame-ancestors 'none'"));
    }

    #[tokio::test]
    async fn unknown_api_route_is_json_404() {
        let res = test_app().await.oneshot(json("GET", "/api/v1/nope", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn other_apps_sync_indexers_through_v3() {
        let app = test_app().await;
        let creds = r#"{"username":"ana","password":"long enough"}"#;
        let res = app.clone().oneshot(json("POST", "/api/v1/auth/setup", creds)).await.unwrap();
        let c = res.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();
        let res =
            app.clone().oneshot(authed("POST", "/api/v1/api-keys", &c, r#"{"name":"sync"}"#)).await.unwrap();
        let key = body_json(res).await["key"].as_str().unwrap().to_owned();
        let v3 = |method: &str, uri: &str, body: &str| {
            let mut r = json(method, uri, body);
            r.headers_mut().insert("x-api-key", key.parse().unwrap());
            r
        };

        let res = app.clone().oneshot(json("GET", "/api/v3/system/status", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let res = app
            .clone()
            .oneshot(json("GET", &format!("/api/v3/system/status?apikey={key}"), ""))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "key in the query works too");
        assert!(res.headers()["x-application-version"].to_str().unwrap().starts_with('5'));

        let schema =
            body_json(app.clone().oneshot(v3("GET", "/api/v3/indexer/schema", "")).await.unwrap()).await;
        let mut torznab =
            schema.as_array().unwrap().iter().find(|s| s["implementation"] == "Torznab").unwrap().clone();
        for f in torznab["fields"].as_array_mut().unwrap() {
            match f["name"].as_str().unwrap() {
                "baseUrl" => f["value"] = "http://indexers.local:9696/7/".into(),
                "apiKey" => f["value"] = "THE-KEY".into(),
                "categories" => f["value"] = serde_json::json!([2000, 2040]),
                _ => {}
            }
        }
        torznab["name"] = "Films (sync)".into();
        let res =
            app.clone().oneshot(v3("POST", "/api/v3/indexer/test", &torznab.to_string())).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = app.clone().oneshot(v3("POST", "/api/v3/indexer", &torznab.to_string())).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let created = body_json(res).await;
        let id = created["id"].as_i64().unwrap();
        let text = created.to_string();
        assert!(text.contains("http://indexers.local:9696/7/"), "base url as sent: {text}");
        assert!(!text.contains("THE-KEY"), "key never comes back");

        // the native API sees the same indexer
        let list =
            body_json(app.clone().oneshot(authed("GET", "/api/v1/indexers", &c, "")).await.unwrap()).await;
        assert_eq!(list[0]["name"], "Films (sync)");
        assert_eq!(list[0]["api_key_set"], true);

        // re-sending what came back keeps the key
        let res = app.clone().oneshot(v3("PUT", &format!("/api/v3/indexer/{id}"), &text)).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let list =
            body_json(app.clone().oneshot(authed("GET", "/api/v1/indexers", &c, "")).await.unwrap()).await;
        assert_eq!(list[0]["api_key_set"], true);

        let res = app.clone().oneshot(v3("DELETE", &format!("/api/v3/indexer/{id}"), "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = app.clone().oneshot(v3("GET", &format!("/api/v3/indexer/{id}"), "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn request_apps_add_films_through_v3() {
        let tmdb = axum::Router::new().route(
            "/movie/603",
            axum::routing::get(|| async {
                axum::Json(
                    serde_json::json!({ "id": 603, "title": "The Matrix", "release_date": "1999-03-30" }),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, tmdb).await.unwrap() });

        let db = koda_db::memory().await;
        let admin = koda_db::users::create(&db, "admin", "x", koda_core::Permissions::ADMIN).await.unwrap();
        koda_db::users::create_api_key(&db, admin.id, "requests", &auth::token_hash("K")).await.unwrap();
        let config = Config {
            config_dir: std::env::temp_dir(),
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: "sqlite::memory:".into(),
        };
        let state = AppState::new(db, config, Default::default());
        state.set_tmdb(koda_metadata::Tmdb::with_base_url(&base, "k", "en-US", None));
        let app = app(state);
        let call = |method: &str, uri: &str, body: &str| {
            let mut r = json(method, uri, body);
            r.headers_mut().insert("x-api-key", "K".parse().unwrap());
            app.clone().oneshot(r)
        };

        let found = body_json(call("GET", "/api/v3/movie/lookup?term=tmdb:603", "").await.unwrap()).await;
        assert_eq!((found[0]["id"].as_i64(), found[0]["title"].as_str()), (Some(0), Some("The Matrix")));

        let res = call("POST", "/api/v3/movie", r#"{"tmdbId":603,"qualityProfileId":1,"monitored":true}"#)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let added = body_json(res).await;
        let id = added["id"].as_i64().unwrap();
        assert!(id > 0);
        assert_eq!((added["monitored"].as_bool(), added["hasFile"].as_bool()), (Some(true), Some(false)));
        assert_eq!(added["titleSlug"], "the-matrix-603");

        let found = body_json(call("GET", "/api/v3/movie/lookup?term=tmdb:603", "").await.unwrap()).await;
        assert_eq!(found[0]["id"].as_i64(), Some(id), "now in the library");
        let all = body_json(call("GET", "/api/v3/movie", "").await.unwrap()).await;
        assert_eq!(all.as_array().unwrap().len(), 1);

        let cmd = format!(r#"{{"name":"MoviesSearch","movieIds":[{id}]}}"#);
        assert_eq!(call("POST", "/api/v3/command", &cmd).await.unwrap().status(), StatusCode::CREATED);
        let q = body_json(call("GET", "/api/v3/queue?page=1&pageSize=50", "").await.unwrap()).await;
        assert_eq!(q["totalRecords"], 0);
        let h =
            body_json(call("GET", &format!("/api/v3/history?eventType=1&movieIds={id}"), "").await.unwrap())
                .await;
        assert_eq!(h["totalRecords"], 0, "nothing grabbed yet");
        let legacy = body_json(call("GET", "/api/system/status", "").await.unwrap()).await;
        assert!(legacy["version"].as_str().unwrap().starts_with('5'));
        let p = body_json(call("GET", "/api/v3/qualityProfile", "").await.unwrap()).await;
        assert_eq!(p[0]["name"], "HD 1080p");
    }
}
