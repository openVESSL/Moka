//! Database access. The only crate that writes SQL.

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub mod connectors;
pub mod grabs;
pub mod library;
pub mod requests;
pub mod settings;
pub mod titles;
pub mod users;

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
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(options).await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

/// Fresh in-memory database with the schema applied. For tests.
pub async fn memory() -> Pool {
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
