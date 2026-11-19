use std::net::SocketAddr;
use std::path::PathBuf;

/// Startup configuration from environment variables. Everything else is set
/// in the UI and stored in the database.
#[derive(Debug, Clone)]
pub struct Config {
    pub config_dir: PathBuf,
    pub listen: SocketAddr,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let config_dir = PathBuf::from(env_or("KODA_CONFIG_DIR", "./config"));
        let port: u16 = env_or("KODA_PORT", "8484").parse()?;
        let host = env_or("KODA_HOST", "0.0.0.0");
        let listen = format!("{host}:{port}").parse()?;
        let database_url = std::env::var("KODA_DATABASE_URL").unwrap_or_else(|_| {
            format!("sqlite://{}", config_dir.join("koda.db").display())
        });
        Ok(Self { config_dir, listen, database_url })
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.config_dir.join("cache")
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}
