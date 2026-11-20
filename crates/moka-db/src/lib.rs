//! Database access. The only crate that writes SQL.

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub mod connectors;
pub mod grabs;
pub mod library;
pub mod requests;
pub mod secret;
pub mod settings;
pub mod titles;
pub mod users;
pub mod webhooks;

pub type Pool = sqlx::SqlitePool;
pub use sqlx::Error;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Opens (creating if needed) the database and applies pending migrations.
pub async fn connect(url: &str) -> Result<Pool, Error> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let file = options.get_filename().to_path_buf();
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(options).await?;
    if let Some(backup) = backup_before_migrating(&pool, &file).await? {
        // this crate has no logger; stderr ends up in the service log
        eprintln!("database backed up to {} before upgrading its schema", backup.display());
    }
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

/// Migrations this binary knows but the database has not applied yet.
async fn pending_migrations(pool: &Pool) -> Result<usize, Error> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await?;
    let applied: Vec<i64> = if exists > 0 {
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = 1").fetch_all(pool).await?
    } else {
        Vec::new()
    };
    if applied.is_empty() {
        // a brand-new database has nothing to protect
        return Ok(0);
    }
    Ok(MIGRATOR.iter().filter(|m| !applied.contains(&m.version)).count())
}

/// Copies the database next to itself before a schema upgrade, so a bad
/// migration never costs data. Returns the backup path if one was made.
async fn backup_before_migrating(
    pool: &Pool,
    file: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, Error> {
    if file.as_os_str().is_empty() || file.to_string_lossy().contains(":memory:") {
        return Ok(None);
    }
    if pending_migrations(pool).await? == 0 {
        return Ok(None);
    }
    let backup = file.with_extension(format!("db.bak-{}", now()));
    sqlx::query("VACUUM INTO ?").bind(backup.to_string_lossy().into_owned()).execute(pool).await?;
    Ok(Some(backup))
}

/// Fresh in-memory database with the schema applied. For tests.
pub async fn memory() -> Pool {
    secret::init([1; 32]);
    let options = SqliteConnectOptions::from_str("sqlite::memory:").expect("valid url").foreign_keys(true);
    // one connection: every connection to :memory: is a separate database
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open in-memory sqlite");
    MIGRATOR.run(&pool).await.expect("migrations apply");
    pool
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn backup_only_when_upgrading_an_existing_database() {
        let dir = std::env::temp_dir().join(format!("moka-mig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}", dir.join("moka.db").display());

        // fresh database: no backup
        let pool = super::connect(&url).await.unwrap();
        pool.close().await;
        let backups = || {
            std::fs::read_dir(&dir)
                .unwrap()
                .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains(".bak-"))
                .count()
        };
        assert_eq!(backups(), 0);

        // pretend the last migration was never applied: next start backs up first
        let pool = super::connect(&url).await.unwrap();
        let last = super::MIGRATOR.iter().map(|m| m.version).max().unwrap();
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?")
            .bind(last)
            .execute(&pool)
            .await
            .unwrap();
        let file = dir.join("moka.db");
        // and an older one was: the database exists and is behind
        sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (0, 'old', 1, x'00', 0)").execute(&pool).await.unwrap();
        let made = super::backup_before_migrating(&pool, &file).await.unwrap();
        assert!(made.is_some() && backups() == 1);
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
