use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

mod api;
mod auth;
mod config;
mod error;
mod state;
mod web;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn")),
        )
        .init();

    let config = Config::from_env()?;
    std::fs::create_dir_all(&config.config_dir)?;
    std::fs::create_dir_all(config.cache_dir())?;

    let db = koda_db::connect(&config.database_url).await?;
    let listen = config.listen;
    let state = AppState::new(db, config);

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!("listening on http://{listen}");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
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
        app(AppState::new(koda_db::memory().await, config))
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

    #[tokio::test]
    async fn unknown_api_route_is_json_404() {
        let res = test_app().await.oneshot(json("GET", "/api/v1/nope", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
