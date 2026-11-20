//! Info hashes from magnet links and `.torrent` files. The hash is how a
//! download is tracked, so it is worked out before anything is sent.

use sha1::{Digest, Sha1};

#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    Magnet(String),
    File(Vec<u8>),
}

impl Payload {
    pub fn info_hash(&self) -> Option<String> {
        match self {
            Payload::Magnet(m) => magnet_hash(m),
            Payload::File(b) => torrent_hash(b),
        }
    }
}

/// `xt=urn:btih:<40 hex | 32 base32>` → lowercase hex.
pub fn magnet_hash(magnet: &str) -> Option<String> {
    let start = magnet.to_ascii_lowercase().find("xt=urn:btih:")? + "xt=urn:btih:".len();
    let raw: String = magnet[start..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    match raw.len() {
        40 if raw.bytes().all(|b| b.is_ascii_hexdigit()) => Some(raw.to_ascii_lowercase()),
        32 => base32_to_hex(&raw),
        _ => None,
    }
}

fn base32_to_hex(s: &str) -> Option<String> {
    let mut bits: u64 = 0;
    let mut nbits = 0;
    let mut out = Vec::with_capacity(20);
    for c in s.bytes() {
        let v = match c.to_ascii_uppercase() {
            b @ b'A'..=b'Z' => b - b'A',
            b @ b'2'..=b'7' => b - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | u64::from(v);
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    (out.len() == 20).then(|| hex(&out))
}

/// SHA-1 of the bencoded `info` dictionary of a `.torrent` file.
pub fn torrent_hash(bytes: &[u8]) -> Option<String> {
    if bytes.first() != Some(&b'd') {
        return None;
    }
    let mut i = 1;
    while bytes.get(i)? != &b'e' {
        let (key, after_key) = read_string(bytes, i)?;
        let value_end = skip_value(bytes, after_key)?;
        if key == b"info" && bytes[after_key] == b'd' {
            return Some(hex(&Sha1::digest(&bytes[after_key..value_end])));
        }
        i = value_end;
    }
    None
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Reads `<len>:<bytes>` at `i`; returns the bytes and the index after them.
fn read_string(b: &[u8], i: usize) -> Option<(&[u8], usize)> {
    let colon = i + b[i..].iter().position(|&c| c == b':')?;
    let len: usize = std::str::from_utf8(&b[i..colon]).ok()?.parse().ok()?;
    let start = colon + 1;
    let end = start.checked_add(len)?;
    (end <= b.len()).then(|| (&b[start..end], end))
}

/// Index just past the bencoded value starting at `i`.
fn skip_value(b: &[u8], i: usize) -> Option<usize> {
    match b.get(i)? {
        b'i' => Some(i + b[i..].iter().position(|&c| c == b'e')? + 1),
        b'l' | b'd' => {
            let mut j = i + 1;
            while *b.get(j)? != b'e' {
                j = skip_value(b, j)?;
            }
            Some(j + 1)
        }
        b'0'..=b'9' => read_string(b, i).map(|(_, end)| end),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnet_hex_and_base32() {
        let hex = "magnet:?xt=urn:btih:C12FE1C06BBA254A9DC9F519B335AA7C1367A88A&dn=x";
        assert_eq!(magnet_hash(hex).as_deref(), Some("c12fe1c06bba254a9dc9f519b335aa7c1367a88a"));
        let b32 = "magnet:?xt=urn:btih:YEX6DQDLXISUVHOJ6UM3GNNKPQJWPKEK";
        assert_eq!(magnet_hash(b32).as_deref(), Some("c12fe1c06bba254a9dc9f519b335aa7c1367a88a"));
        assert_eq!(magnet_hash("magnet:?dn=nohash"), None);
    }

    #[test]
    fn torrent_file_hash_covers_info_dict_only() {
        let info = b"d6:lengthi1024e4:name8:film.mkv12:piece lengthi16384e6:pieces0:e";
        let mut file = b"d8:announce14:http://t.local7:comment2:hi4:info".to_vec();
        file.extend_from_slice(info);
        file.push(b'e');
        let expected = hex(&Sha1::digest(info));
        assert_eq!(torrent_hash(&file), Some(expected));
        assert_eq!(torrent_hash(b"not a torrent"), None);
        assert_eq!(torrent_hash(b"d4:infoi1ee"), None, "info must be a dictionary");
    }
}
