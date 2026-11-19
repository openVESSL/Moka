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
        let database_url = std::env::var("KODA_DATABASE_URL")
            .unwrap_or_else(|_| format!("sqlite://{}", config_dir.join("koda.db").display()));
        Ok(Self { config_dir, listen, database_url })
    }

    pub fn transcode_dir(&self) -> PathBuf {
        self.config_dir.join("transcodes")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.config_dir.join("cache")
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

/// Key that encrypts secrets in the database. Taken from `KODA_SECRET_KEY`
/// (base64, 32 bytes) or from `secret.key` in the config folder, created on
/// first start. Losing it means re-entering indexer keys and passwords.
pub fn secret_key(config_dir: &std::path::Path) -> anyhow::Result<[u8; 32]> {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    let decode = |s: &str| -> anyhow::Result<[u8; 32]> {
        let bytes = STANDARD.decode(s.trim())?;
        bytes.try_into().map_err(|_| anyhow::anyhow!("secret key must be 32 bytes (base64)"))
    };
    if let Ok(v) = std::env::var("KODA_SECRET_KEY") {
        return decode(&v);
    }
    let path = config_dir.join("secret.key");
    if path.exists() {
        return decode(&std::fs::read_to_string(&path)?);
    }
    let key = koda_db::secret::new_key();
    write_private(&path, &STANDARD.encode(key))?;
    tracing::info!(path = %path.display(), "created a new secret key; back it up with the database");
    Ok(key)
}

/// Writes a file only the current user can read (Unix); on Windows the
/// config folder's own permissions apply.
fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(contents.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn key_is_created_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("koda-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("secret.key"));
        let a = super::secret_key(&dir).unwrap();
        let b = super::secret_key(&dir).unwrap();
        assert_eq!(a, b);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
