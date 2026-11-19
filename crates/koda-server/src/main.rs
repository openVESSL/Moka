use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

mod api;
mod auth;
mod config;
mod error;
mod notify;
mod pipeline;
mod scan;
mod state;
mod streaming;
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

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!("listening on http://{listen}");
    axum::serve(listener, app(state)).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
}

pub fn app(state: AppState) -> axum::Router {
    api::router()
        .fallback(web::serve)
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
    async fn unknown_api_route_is_json_404() {
        let res = test_app().await.oneshot(json("GET", "/api/v1/nope", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
