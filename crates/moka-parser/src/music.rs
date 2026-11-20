//! Music release names: `Artist - Album (2020) [FLAC]` and scene style
//! `Artist-Album-WEB-2020-GROUP`, with the audio quality they announce.

use std::sync::LazyLock;

use moka_core::AudioQuality;
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedAlbum {
    pub artist: String,
    pub album: String,
    pub year: Option<u16>,
    pub quality: AudioQuality,
    pub group: Option<String>,
}

static EXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(zip|rar|7z|torrent|nzb|flac|mp3|m4a)$").unwrap());
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(19[0-9]{2}|20[0-9]{2})\b").unwrap());
/// Brackets and parentheses: years, formats, catalogue numbers, editions.
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\[(][^\])]*[\])]").unwrap());
static SCENE_GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-([A-Za-z0-9]{2,12})$").unwrap());

/// What the name says about the audio, from its words.
pub fn audio_quality(name: &str) -> AudioQuality {
    let lower = name.to_ascii_lowercase();
    let words: Vec<&str> =
        lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    let hires = has("24bit")
        || (has("24") && (has("bit") || has("96") || has("192") || has("48")))
        || lower.contains("hi-res")
        || lower.contains("hires")
        || lower.contains("24-bit");
    if has("flac") || has("lossless") {
        return if hires { AudioQuality::Flac24 } else { AudioQuality::Flac };
    }
    if has("alac") {
        return AudioQuality::Alac;
    }
    if has("v0") || lower.contains("vbr v0") {
        return AudioQuality::Mp3V0;
    }
    let mp3 = has("mp3");
    match () {
        _ if has("320") || has("320kbps") => AudioQuality::Mp3_320,
        _ if has("256") || has("256kbps") => {
            if has("aac") || has("m4a") {
                AudioQuality::Aac
            } else {
                AudioQuality::Mp3_256
            }
        }
        _ if has("aac") || has("m4a") => AudioQuality::Aac,
        _ if has("opus") => AudioQuality::Opus,
        _ if has("192") || has("128") || has("160") || has("192kbps") || has("128kbps") => {
            AudioQuality::Mp3Low
        }
        _ if mp3 => AudioQuality::Mp3,
        _ => AudioQuality::Unknown,
    }
}

/// Words that end the album title in a scene name.
fn technical(word: &str) -> bool {
    let w = word.to_ascii_lowercase();
    YEAR.is_match(&w)
        || matches!(
            w.as_str(),
            "web"
                | "cd"
                | "cdr"
                | "vinyl"
                | "flac"
                | "mp3"
                | "aac"
                | "v0"
                | "320"
                | "24bit"
                | "16bit"
                | "proper"
                | "remastered"
                | "deluxe"
                | "advance"
                | "promo"
                | "retail"
                | "ep"
                | "single"
                | "bootleg"
        )
}

/// Reads artist, album, year and quality. `None` when the name has no
/// artist/album split.
pub fn parse_album(name: &str) -> Option<ParsedAlbum> {
    let original = EXT.replace(name.trim(), "").into_owned();
    let quality = audio_quality(&original);
    let year = YEAR.captures_iter(&original).filter_map(|c| c[1].parse::<u16>().ok()).next();

    if let Some((artist, rest)) = original.split_once(" - ") {
        // "Artist - Album (2020) [FLAC]": the album ends at the first bracket
        let album = BRACKETS.replace_all(rest, " ");
        let album = album.split(" - ").next().unwrap_or("");
        let words: Vec<&str> = album.split_whitespace().collect();
        let end = words.iter().position(|w| technical(w)).unwrap_or(words.len());
        let album = words[..end].join(" ");
        let (artist, album) = (artist.trim().to_owned(), album.trim().to_owned());
        return (!artist.is_empty() && !album.is_empty()).then_some(ParsedAlbum {
            artist,
            album,
            year,
            quality,
            group: None,
        });
    }

    // scene: "Artist_Name-Album_Title-(CAT01)-WEB-2020-GROUP"
    let cleaned = original.replace('_', " ");
    let group = SCENE_GROUP.captures(&cleaned).map(|c| c[1].to_owned());
    let parts: Vec<&str> = cleaned.split('-').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.len() < 3 {
        return None;
    }
    let artist = parts[0].to_owned();
    let mut album_parts = Vec::new();
    for p in &parts[1..] {
        if p.starts_with('(') || technical(p) || p.split_whitespace().all(technical) {
            break;
        }
        album_parts.push(*p);
    }
    let album = album_parts.join(" - ");
    if artist.is_empty() || album.is_empty() {
        return None;
    }
    Some(ParsedAlbum { artist, album, year, quality, group })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_names() {
        let cases: &[(&str, &str, &str, Option<u16>, AudioQuality)] = &[
            ("Band - First Light (2001) [FLAC]", "Band", "First Light", Some(2001), AudioQuality::Flac),
            (
                "Band - First Light (2001) [FLAC 24bit-96kHz]",
                "Band",
                "First Light",
                Some(2001),
                AudioQuality::Flac24,
            ),
            ("Band - First Light [2001] [MP3 320]", "Band", "First Light", Some(2001), AudioQuality::Mp3_320),
            (
                "The Band - Second (Deluxe Edition) (2004) MP3 V0",
                "The Band",
                "Second",
                Some(2004),
                AudioQuality::Mp3V0,
            ),
            (
                "Some Artist - Album Name 2019 AAC 256",
                "Some Artist",
                "Album Name",
                Some(2019),
                AudioQuality::Aac,
            ),
            ("Band - Album", "Band", "Album", None, AudioQuality::Unknown),
            (
                "Band_Name-Album_Title-(CAT01)-WEB-2020-GRP",
                "Band Name",
                "Album Title",
                Some(2020),
                AudioQuality::Unknown,
            ),
            (
                "Band_Name-Album_Title-WEB-FLAC-2020-GRP.zip",
                "Band Name",
                "Album Title",
                Some(2020),
                AudioQuality::Flac,
            ),
        ];
        for (name, artist, album, year, q) in cases {
            let p = parse_album(name).unwrap_or_else(|| panic!("{name}: not read"));
            assert_eq!(
                (p.artist.as_str(), p.album.as_str(), p.year, p.quality),
                (*artist, *album, *year, *q),
                "{name}"
            );
        }
        assert_eq!(
            parse_album("Band_Name-Album_Title-WEB-FLAC-2020-GRP").unwrap().group.as_deref(),
            Some("GRP")
        );
        assert!(parse_album("just one word").is_none());
    }
}
