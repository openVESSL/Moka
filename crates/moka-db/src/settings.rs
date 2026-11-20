//! Global key/value settings.

use serde::{Serialize, de::DeserializeOwned};

use crate::{Error, Pool};

pub async fn get<T: DeserializeOwned>(pool: &Pool, key: &str) -> Result<Option<T>, Error> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?").bind(key).fetch_optional(pool).await?;
    Ok(raw.and_then(|v| serde_json::from_str(&v).ok()))
}

pub async fn set<T: Serialize>(pool: &Pool, key: &str, value: &T) -> Result<(), Error> {
    let json = serde_json::to_string(value).map_err(|e| Error::Encode(Box::new(e)))?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn set_then_get() {
        let pool = crate::memory().await;
        assert_eq!(super::get::<String>(&pool, "tmdb.language").await.unwrap(), None);
        super::set(&pool, "tmdb.language", &"ro-RO").await.unwrap();
        super::set(&pool, "tmdb.language", &"en-US").await.unwrap();
        assert_eq!(super::get::<String>(&pool, "tmdb.language").await.unwrap().as_deref(), Some("en-US"));
    }
}
