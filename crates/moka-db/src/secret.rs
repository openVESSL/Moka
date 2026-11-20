//! Encryption for secrets at rest (indexer keys, download client passwords,
//! webhook addresses, the TMDB key). A leaked database file alone does not
//! reveal them; the key lives in a separate file next to it.
//!
//! Stored form: `enc1:` + base64(24-byte nonce || ciphertext). Values without
//! the prefix are legacy plaintext and are read as is, then re-sealed.

use std::sync::OnceLock;

use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::{Error, Pool};

const PREFIX: &str = "enc1:";
static KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Sets the process-wide key. Later calls are ignored.
pub fn init(key: [u8; 32]) {
    let _ = KEY.set(key);
}

pub fn new_key() -> [u8; 32] {
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).expect("system random generator works");
    k
}

fn cipher() -> XChaCha20Poly1305 {
    let key = KEY.get().expect("secret key initialized before database use");
    XChaCha20Poly1305::new_from_slice(key).expect("32-byte key")
}

pub fn is_sealed(value: &str) -> bool {
    value.starts_with(PREFIX)
}

pub fn seal(plain: &str) -> String {
    if plain.is_empty() || is_sealed(plain) {
        return plain.to_owned();
    }
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).expect("system random generator works");
    let ct = cipher().encrypt(&XNonce::from(nonce), plain.as_bytes()).expect("encryption cannot fail");
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    format!("{PREFIX}{}", B64.encode(out))
}

/// Decrypts a sealed value. Plaintext passes through. A value sealed with a
/// different key comes back empty, so the user is asked to enter it again.
pub fn open(stored: &str) -> String {
    let Some(b64) = stored.strip_prefix(PREFIX) else { return stored.to_owned() };
    let Ok(bytes) = B64.decode(b64) else { return String::new() };
    if bytes.len() < 24 {
        return String::new();
    }
    let (nonce, ct) = bytes.split_at(24);
    let nonce: [u8; 24] = nonce.try_into().expect("24 bytes");
    cipher()
        .decrypt(&XNonce::from(nonce), ct)
        .ok()
        .and_then(|p| String::from_utf8(p).ok())
        .unwrap_or_default()
}

pub fn seal_opt(v: Option<&str>) -> Option<String> {
    v.map(seal)
}

pub fn open_opt(v: Option<String>) -> Option<String> {
    v.map(|s| open(&s))
}

/// Seals secrets written by older versions in plain text. Safe to run on every start.
pub async fn seal_legacy(pool: &Pool) -> Result<u64, Error> {
    let mut changed = 0;
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, api_key FROM indexers WHERE api_key IS NOT NULL AND api_key != ''")
            .fetch_all(pool)
            .await?;
    for (id, v) in rows.into_iter().filter(|(_, v)| !is_sealed(v)) {
        sqlx::query("UPDATE indexers SET api_key = ? WHERE id = ?")
            .bind(seal(&v))
            .bind(id)
            .execute(pool)
            .await?;
        changed += 1;
    }
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, password FROM download_clients WHERE password IS NOT NULL AND password != ''",
    )
    .fetch_all(pool)
    .await?;
    for (id, v) in rows.into_iter().filter(|(_, v)| !is_sealed(v)) {
        sqlx::query("UPDATE download_clients SET password = ? WHERE id = ?")
            .bind(seal(&v))
            .bind(id)
            .execute(pool)
            .await?;
        changed += 1;
    }
    let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, url FROM webhooks").fetch_all(pool).await?;
    for (id, v) in rows.into_iter().filter(|(_, v)| !is_sealed(v)) {
        sqlx::query("UPDATE webhooks SET url = ? WHERE id = ?").bind(seal(&v)).bind(id).execute(pool).await?;
        changed += 1;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_legacy() {
        init([7; 32]);
        let s = seal("hunter2");
        assert!(s.starts_with(PREFIX));
        assert_ne!(seal("hunter2"), s, "fresh nonce every time");
        assert_eq!(open(&s), "hunter2");
        assert_eq!(open("plain-legacy"), "plain-legacy");
        assert_eq!(seal(""), "");
        assert_eq!(open("enc1:not-base64!!"), "");
        // tampering is detected, not silently accepted
        let mut tampered = s.clone();
        tampered.pop();
        tampered.push(if s.ends_with('A') { 'B' } else { 'A' });
        assert_eq!(open(&tampered), "");
    }
}
