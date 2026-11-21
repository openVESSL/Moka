//! Users, login sessions and API keys. Password and token hashing happens in
//! the server; this module only stores hashes.

use moka_core::Permissions;
use serde::Serialize;

use crate::{Error, Pool, now};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip)]
    pub password_hash: String,
    #[sqlx(try_from = "i64")]
    pub permissions: PermissionBits,
    pub created_at: i64,
    /// Discord account linked to this user, as its numeric id.
    pub discord_id: Option<String>,
}

/// Column adapter: SQLite stores the bitmask as a signed integer.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(transparent)]
pub struct PermissionBits(pub Permissions);

impl TryFrom<i64> for PermissionBits {
    type Error = std::num::TryFromIntError;
    fn try_from(v: i64) -> Result<Self, Self::Error> {
        Ok(Self(Permissions(u32::try_from(v)?)))
    }
}

impl User {
    pub fn permissions(&self) -> Permissions {
        self.permissions.0
    }
}

pub async fn count(pool: &Pool) -> Result<i64, Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(pool).await
}

pub async fn create(
    pool: &Pool,
    username: &str,
    password_hash: &str,
    permissions: Permissions,
) -> Result<User, Error> {
    sqlx::query_as("INSERT INTO users (username, password_hash, permissions) VALUES (?, ?, ?) RETURNING *")
        .bind(username)
        .bind(password_hash)
        .bind(i64::from(permissions.0))
        .fetch_one(pool)
        .await
}

pub async fn by_username(pool: &Pool, username: &str) -> Result<Option<User>, Error> {
    sqlx::query_as("SELECT * FROM users WHERE username = ?").bind(username).fetch_optional(pool).await
}

pub async fn by_id(pool: &Pool, id: i64) -> Result<Option<User>, Error> {
    sqlx::query_as("SELECT * FROM users WHERE id = ?").bind(id).fetch_optional(pool).await
}

pub async fn create_session(
    pool: &Pool,
    token_hash: &str,
    user_id: i64,
    ttl_seconds: i64,
    user_agent: Option<&str>,
) -> Result<(), Error> {
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at, user_agent) VALUES (?, ?, ?, ?)")
        .bind(token_hash)
        .bind(user_id)
        .bind(now() + ttl_seconds)
        .bind(user_agent)
        .execute(pool)
        .await?;
    Ok(())
}

/// User owning a live session. Expired sessions are treated as missing.
pub async fn by_session(pool: &Pool, token_hash: &str) -> Result<Option<User>, Error> {
    sqlx::query_as(
        "SELECT u.* FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = ? AND s.expires_at > ?",
    )
    .bind(token_hash)
    .bind(now())
    .fetch_optional(pool)
    .await
}

pub async fn delete_session(pool: &Pool, token_hash: &str) -> Result<(), Error> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?").bind(token_hash).execute(pool).await?;
    Ok(())
}

pub async fn delete_expired_sessions(pool: &Pool) -> Result<u64, Error> {
    let r = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?").bind(now()).execute(pool).await?;
    Ok(r.rows_affected())
}

pub async fn by_api_key(pool: &Pool, token_hash: &str) -> Result<Option<User>, Error> {
    let user =
        sqlx::query_as("SELECT u.* FROM api_keys k JOIN users u ON u.id = k.user_id WHERE k.token_hash = ?")
            .bind(token_hash)
            .fetch_optional(pool)
            .await?;
    if user.is_some() {
        sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE token_hash = ?")
            .bind(now())
            .bind(token_hash)
            .execute(pool)
            .await?;
    }
    Ok(user)
}

pub async fn list(pool: &Pool) -> Result<Vec<User>, Error> {
    sqlx::query_as("SELECT * FROM users ORDER BY username").fetch_all(pool).await
}

pub async fn set_permissions(pool: &Pool, id: i64, permissions: Permissions) -> Result<(), Error> {
    sqlx::query("UPDATE users SET permissions = ? WHERE id = ?")
        .bind(i64::from(permissions.0))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Changes a password and signs the user out everywhere else.
pub async fn set_password(pool: &Pool, id: i64, password_hash: &str) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(password_hash)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await
}

/// Links a Discord account to the user, or unlinks it with `None`.
pub async fn set_discord_id(pool: &Pool, id: i64, discord_id: Option<&str>) -> Result<(), Error> {
    sqlx::query("UPDATE users SET discord_id = ? WHERE id = ?")
        .bind(discord_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &Pool, id: i64) -> Result<(), Error> {
    sqlx::query("DELETE FROM users WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

pub async fn admin_count(pool: &Pool) -> Result<i64, Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE permissions & 1 = 1").fetch_one(pool).await
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ApiKey {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

pub async fn create_api_key(
    pool: &Pool,
    user_id: i64,
    name: &str,
    token_hash: &str,
) -> Result<ApiKey, Error> {
    sqlx::query_as(
        "INSERT INTO api_keys (user_id, name, token_hash) VALUES (?, ?, ?)
         RETURNING id, user_id, name, created_at, last_used_at",
    )
    .bind(user_id)
    .bind(name)
    .bind(token_hash)
    .fetch_one(pool)
    .await
}

pub async fn api_keys(pool: &Pool) -> Result<Vec<ApiKey>, Error> {
    sqlx::query_as("SELECT id, user_id, name, created_at, last_used_at FROM api_keys ORDER BY id")
        .fetch_all(pool)
        .await
}

pub async fn delete_api_key(pool: &Pool, id: i64) -> Result<(), Error> {
    sqlx::query("DELETE FROM api_keys WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sessions_resolve_to_users_until_expiry() {
        let pool = crate::memory().await;
        let u = create(&pool, "Ana", "hash", Permissions::ADMIN).await.unwrap();
        assert!(by_username(&pool, "ana").await.unwrap().is_some(), "case-insensitive");
        assert!(u.permissions().has(Permissions::PLAY));

        create_session(&pool, "live", u.id, 3600, None).await.unwrap();
        create_session(&pool, "dead", u.id, -1, None).await.unwrap();
        assert_eq!(by_session(&pool, "live").await.unwrap().unwrap().id, u.id);
        assert!(by_session(&pool, "dead").await.unwrap().is_none());
        assert_eq!(delete_expired_sessions(&pool).await.unwrap(), 1);

        delete_session(&pool, "live").await.unwrap();
        assert!(by_session(&pool, "live").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn duplicate_username_is_rejected() {
        let pool = crate::memory().await;
        create(&pool, "ana", "h", Permissions::NONE).await.unwrap();
        assert!(create(&pool, "ANA", "h", Permissions::NONE).await.is_err());
    }
}
