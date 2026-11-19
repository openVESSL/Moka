//! Outgoing notification targets.

use serde::Serialize;

use crate::{Error, Pool};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Webhook {
    pub id: i64,
    pub name: String,
    pub url: String,
    /// JSON array of event names.
    pub events: String,
    pub enabled: bool,
}

impl Webhook {
    pub fn wants(&self, event: &str) -> bool {
        serde_json::from_str::<Vec<String>>(&self.events)
            .map(|e| e.iter().any(|x| x == event))
            .unwrap_or(false)
    }
}

// Webhook addresses often embed a token (Discord), so they are stored sealed.
fn open(mut w: Webhook) -> Webhook {
    w.url = crate::secret::open(&w.url);
    w
}

pub async fn list(pool: &Pool) -> Result<Vec<Webhook>, Error> {
    let rows: Vec<Webhook> =
        sqlx::query_as("SELECT id, name, url, events, enabled FROM webhooks ORDER BY id")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(open).collect())
}

pub async fn get(pool: &Pool, id: i64) -> Result<Option<Webhook>, Error> {
    let row: Option<Webhook> =
        sqlx::query_as("SELECT id, name, url, events, enabled FROM webhooks WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(open))
}

pub async fn create(pool: &Pool, name: &str, url: &str, events: &[String]) -> Result<i64, Error> {
    let events = serde_json::to_string(events).map_err(|e| Error::Encode(Box::new(e)))?;
    sqlx::query_scalar("INSERT INTO webhooks (name, url, events) VALUES (?, ?, ?) RETURNING id")
        .bind(name)
        .bind(crate::secret::seal(url))
        .bind(events)
        .fetch_one(pool)
        .await
}

pub async fn delete(pool: &Pool, id: i64) -> Result<(), Error> {
    sqlx::query("DELETE FROM webhooks WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn filters_by_event() {
        let pool = crate::memory().await;
        let id = super::create(&pool, "bot", "http://x", &["available".into()]).await.unwrap();
        let w = super::get(&pool, id).await.unwrap().unwrap();
        assert!(w.wants("available"));
        assert!(!w.wants("grabbed"));
    }
}
