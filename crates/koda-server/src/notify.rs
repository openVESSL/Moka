//! Outgoing webhooks. Plain JSON for bots and scripts; Discord webhook
//! addresses get a Discord message with the poster instead.

use std::time::Duration;

use serde::Serialize;

use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Requested,
    Approved,
    Declined,
    Grabbed,
    Available,
    Failed,
    Test,
}

impl Event {
    pub const ALL: [&'static str; 6] =
        ["requested", "approved", "declined", "grabbed", "available", "failed"];

    pub fn as_str(self) -> &'static str {
        match self {
            Event::Requested => "requested",
            Event::Approved => "approved",
            Event::Declined => "declined",
            Event::Grabbed => "grabbed",
            Event::Available => "available",
            Event::Failed => "failed",
            Event::Test => "test",
        }
    }

    fn headline(self, title: &str) -> String {
        match self {
            Event::Requested => format!("{title} was requested"),
            Event::Approved => format!("{title} was approved"),
            Event::Declined => format!("{title} was declined"),
            Event::Grabbed => format!("{title} is downloading"),
            Event::Available => format!("{title} is ready to watch"),
            Event::Failed => format!("{title} hit a problem"),
            Event::Test => "Test notification".to_owned(),
        }
    }
}

/// What a webhook receives.
#[derive(Debug, Clone, Serialize)]
pub struct Payload {
    pub event: Event,
    pub occurred_at: i64,
    pub message: String,
    pub title: Option<TitleInfo>,
    /// Who the event is about, when there is one (the requester).
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TitleInfo {
    pub id: i64,
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    pub poster_url: Option<String>,
    pub detail: Option<String>,
}

impl TitleInfo {
    pub fn from_title(t: &koda_db::titles::Title, detail: Option<String>) -> Self {
        Self {
            id: t.id,
            tmdb_id: t.tmdb_id,
            imdb_id: t.imdb_id.clone(),
            title: t.title.clone(),
            year: t.year,
            poster_url: t.poster_path.as_ref().map(|p| format!("{}/w500{p}", koda_metadata::IMAGE_BASE_URL)),
            detail,
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Fires an event at every enabled webhook that wants it. Never blocks or fails the caller.
pub fn send(state: &AppState, event: Event, title: Option<TitleInfo>, user: Option<String>) {
    let state = state.clone();
    tokio::spawn(async move {
        let hooks = match koda_db::webhooks::list(&state.db).await {
            Ok(h) => h,
            Err(e) => return tracing::warn!(error = ?e, "could not load webhooks"),
        };
        let name = title.as_ref().map(|t| t.title.as_str()).unwrap_or("A film");
        let payload =
            Payload { event, occurred_at: now(), message: event.headline(name), title: title.clone(), user };
        for h in hooks.into_iter().filter(|h| h.enabled && h.wants(event.as_str())) {
            if let Err(e) = deliver(&state.http, &h.url, &payload).await {
                tracing::warn!(webhook = %h.name, error = %e, "webhook delivery failed");
            }
        }
    });
}

/// Sends one payload; one retry after a short wait on network or server errors.
pub async fn deliver(http: &reqwest::Client, url: &str, payload: &Payload) -> Result<(), String> {
    let body =
        if is_discord(url) { discord(payload) } else { serde_json::to_value(payload).unwrap_or_default() };
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        match http.post(url).json(&body).timeout(Duration::from_secs(10)).send().await {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) if r.status().is_client_error() => {
                return Err(format!("rejected with HTTP {}", r.status()));
            }
            Ok(r) => last = format!("HTTP {}", r.status()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn is_discord(url: &str) -> bool {
    reqwest::Url::parse(url).ok().is_some_and(|u| {
        matches!(
            u.host_str(),
            Some("discord.com" | "discordapp.com" | "ptb.discord.com" | "canary.discord.com")
        ) && u.path().starts_with("/api/webhooks/")
    })
}

fn discord(p: &Payload) -> serde_json::Value {
    let mut embed = serde_json::json!({ "title": p.message, "timestamp": null });
    if let Some(t) = &p.title {
        let mut desc = Vec::new();
        if let Some(y) = t.year {
            desc.push(y.to_string());
        }
        if let Some(d) = &t.detail {
            desc.push(d.clone());
        }
        if let Some(u) = &p.user {
            desc.push(format!("Requested by {u}"));
        }
        embed["description"] = desc.join(" · ").into();
        if let Some(poster) = &t.poster_url {
            embed["thumbnail"] = serde_json::json!({ "url": poster });
        }
        if let Some(id) = t.tmdb_id {
            embed["url"] = format!("https://www.themoviedb.org/movie/{id}").into();
        }
    }
    serde_json::json!({ "embeds": [embed] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_addresses_are_recognized() {
        assert!(is_discord("https://discord.com/api/webhooks/1/abc"));
        assert!(!is_discord("https://discord.com.evil.example/api/webhooks/1/abc"));
        assert!(!is_discord("http://bot.local/hook"));
    }

    #[test]
    fn discord_message_carries_poster_and_requester() {
        let p = Payload {
            event: Event::Available,
            occurred_at: 0,
            message: Event::Available.headline("Dune"),
            title: Some(TitleInfo {
                id: 1,
                tmdb_id: Some(438631),
                imdb_id: None,
                title: "Dune".into(),
                year: Some(2021),
                poster_url: Some("https://img/p.jpg".into()),
                detail: Some("WEB-DL 1080p".into()),
            }),
            user: Some("ana".into()),
        };
        let v = discord(&p);
        assert_eq!(v["embeds"][0]["title"], "Dune is ready to watch");
        assert_eq!(v["embeds"][0]["description"], "2021 · WEB-DL 1080p · Requested by ana");
        assert_eq!(v["embeds"][0]["thumbnail"]["url"], "https://img/p.jpg");
    }
}
