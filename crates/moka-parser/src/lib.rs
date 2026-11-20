//! Release and file name parsing.
//!
//! Instead of one large pattern per naming style, the name is split into
//! tokens, every token is classified on its own (year, resolution, source,
//! codec, language, edition...), and the title is whatever comes before the
//! technical part. Each decision is a small rule that can be read and tested.

use std::sync::LazyLock;

use moka_core::{Modifier, Quality, Resolution, Revision, Source};
use regex::Regex;
use serde::Serialize;

mod book;
mod episode;
mod music;
mod words;

pub use book::{ParsedBook, book_matches, parse_book};
pub use episode::{ParsedEpisode, parse_episode};
pub use music::{ParsedAlbum, audio_quality, parse_album};

use words::{Word, classify};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedRelease {
    /// Main title, cleaned for display.
    pub title: String,
    /// Other titles given with "AKA" or " / ".
    pub alt_titles: Vec<String>,
    pub year: Option<u16>,
    pub quality: Quality,
    pub revision: Revision,
    pub group: Option<String>,
    pub edition: Option<String>,
    /// ISO 639-1 codes, plus "multi" when the release says so.
    pub languages: Vec<String>,
    pub hardcoded_subs: bool,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<i64>,
}

static VIDEO_EXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\.(mkv|mp4|m4v|avi|wmv|mov|ts|m2ts|mpg|mpeg|webm|iso|img|nzb|torrent|divx|xvid|ogm|flv)$",
    )
    .unwrap()
});
static WEBSITE_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    // a bracketed domain, a www. domain, or a bare domain followed by " - "
    Regex::new(
        r"(?i)^\s*(?:\[\s*(?:www\.)?[\w-]+\.\w{2,6}\s*\]|www\.[\w-]+\.\w{2,6}\b|[\w-]+\.(?:com|org|net|to|tc|cc|me|io|info|ru|xyz|club)\s+[-–:])\s*[-–:]?\s*",
    )
    .unwrap()
});
static LEADING_GROUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\[([^\]]{1,40})\][-_. ]*").unwrap());
static TRAILING_HASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\[[0-9A-Fa-f]{8}\]\s*$").unwrap());
static TRAILING_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\[[\w.\- ]{1,30}\]\s*$").unwrap());
static DOTTED_CODEC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b([hx])\.(26[45])\b").unwrap());
static IMDB_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btt(\d{7,8})\b").unwrap());
static TMDB_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\btmdb(?:id)?-(\d{1,9})\b").unwrap());
static HASHED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[0-9a-zA-Z]{32}|[a-z0-9]{24})$").unwrap());
static AKA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s+(?:aka|a\.k\.a\.)\s+|\s+/\s+").unwrap());
static BRACKET_AKA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*\(\s*aka\s+([^)]+)\)?").unwrap());
static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][\w@&]{0,30}$").unwrap());

#[derive(Debug, Clone)]
struct Token<'a> {
    text: &'a str,
    /// Byte offset just past the token in the cleaned name.
    end: usize,
    word: Word,
}

/// Parses a film release or file name. Returns `None` for names that carry
/// no usable title (hashes, obfuscated names).
pub fn parse_movie(name: &str) -> Option<ParsedRelease> {
    let original = name.trim();
    let (s, group) = prepare(original)?;
    let tokens = tokenize(&s);
    let (title_end, year_idx) = locate_title(&tokens);
    let r = release(original, &s, &tokens, title_end, year_idx, group);
    (!r.title.is_empty()).then_some(r)
}

/// Strips extension, site names, hashes and a leading `[Group]`. Returns the
/// cleaned name and that group, or `None` for hashed or empty names.
fn prepare(original: &str) -> Option<(String, Option<String>)> {
    if !original.chars().any(char::is_alphanumeric) {
        return None;
    }
    let mut s = VIDEO_EXT.replace(original, "").into_owned();
    if HASHED.is_match(&s) {
        return None;
    }
    s = s.replace(['【', '「'], "[").replace(['】', '」'], "]");
    s = WEBSITE_PREFIX.replace(&s, "").into_owned();

    let mut group = None;
    if let Some(c) = LEADING_GROUP.captures(&s) {
        group = Some(c[1].trim().to_owned());
        s = s[c.get(0).unwrap().end()..].to_owned();
    }
    s = TRAILING_HASH.replace(&s, "").into_owned();
    s = DOTTED_CODEC.replace_all(&s, "$1$2").into_owned();
    let s = s.trim_matches(|c: char| c == '-' || c == '_' || c.is_whitespace()).to_owned();
    Some((s, group))
}

/// Everything read from a name once we know where its title ends. The title
/// is empty when there is none (`title_end == 0`).
fn release(
    original: &str,
    s: &str,
    tokens: &[Token],
    title_end: usize,
    year_idx: Option<usize>,
    mut group: Option<String>,
) -> ParsedRelease {
    let (title, alt_titles) = match title_end {
        0 => Default::default(),
        _ => clean_title(&s[..tokens[title_end - 1].end]).unwrap_or_default(),
    };
    let tail = &tokens[title_end..];
    let tail_str = match title_end {
        0 => s,
        _ => &s[tokens[title_end - 1].end..],
    };
    let (mut quality, revision) = quality_of(tail, tail_str);
    if original.to_ascii_lowercase().ends_with(".iso") && quality.source == Source::Bluray {
        quality.modifier = Modifier::Disc;
    }

    let mut languages: Vec<String> = Vec::new();
    for t in tail {
        let code = match t.word {
            Word::Language(l) => l,
            Word::Tag if t.text.eq_ignore_ascii_case("multi") => "multi",
            _ => continue,
        };
        if !languages.iter().any(|l| l == code) {
            languages.push(code.to_owned());
        }
    }

    if group.is_none() {
        group = trailing_group(tail_str);
    }

    ParsedRelease {
        title,
        alt_titles,
        year: year_idx.and_then(|i| match tokens[i].word {
            Word::Year(y) => Some(y),
            _ => None,
        }),
        quality,
        revision,
        group,
        edition: edition(tokens, title_end),
        languages,
        hardcoded_subs: tail.iter().any(|t| t.word == Word::HardcodedSubs),
        imdb_id: IMDB_ID.captures(original).map(|c| format!("tt{}", &c[1])),
        tmdb_id: TMDB_ID.captures(original).and_then(|c| c[1].parse().ok()),
    }
}

/// Quality of any name (release or file), without needing the title.
pub fn parse_quality(name: &str) -> (Quality, Revision) {
    let s = VIDEO_EXT.replace(name.trim(), "");
    let s = DOTTED_CODEC.replace_all(&s, "$1$2");
    let tokens = tokenize(&s);
    // the first word is usually the title; skip it so a film called "Cam" is not a cam
    let skip = usize::from(tokens.len() > 1);
    quality_of(&tokens[skip..], &s)
}

fn tokenize(s: &str) -> Vec<Token<'_>> {
    fn push<'a>(out: &mut Vec<Token<'a>>, s: &'a str, a: usize, b: usize) {
        let text = s[a..b].trim_matches(|c: char| c == '-' || c == '!' || c == '?');
        if text.is_empty() {
            return;
        }
        let mut word = classify(text);
        // "x264-GROUP", "BLURAY-GROUP": the part before the group still counts
        if word == Word::Plain
            && let Some((head, _)) = text.rsplit_once('-')
        {
            word = classify(head);
        }
        out.push(Token { text, end: b, word });
    }
    let mut out = Vec::new();
    let mut start = None;
    let is_sep = |c: char| matches!(c, ' ' | '.' | '_' | '[' | ']' | '(' | ')' | '{' | '}' | '+' | '|' | ',');
    for (i, c) in s.char_indices() {
        if is_sep(c) {
            if let Some(a) = start.take() {
                push(&mut out, s, a, i);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(a) = start {
        push(&mut out, s, a, s.len());
    }
    out
}

/// Returns (index where the title ends, index of the year token).
fn locate_title(tokens: &[Token]) -> (usize, Option<usize>) {
    let n = tokens.len();
    if n == 0 {
        return (0, None);
    }
    let first_strong = (1..n).find(|&i| tokens[i].word.is_strong()).unwrap_or(n);

    // The year is the last plausible year before the technical part, never
    // the very first word (that is a title like "1917" or "2012").
    let years: Vec<usize> = (1..n).filter(|&i| matches!(tokens[i].word, Word::Year(_))).collect();
    let year = years
        .iter()
        .copied()
        .rfind(|&i| i < first_strong)
        .or_else(|| years.iter().copied().find(|&i| i > first_strong));

    let mut end = year.unwrap_or(n).min(first_strong);

    // weak technical words and editions end the title only when backed up by
    // more technical words (or the year) right after them
    for i in 1..end {
        let w = &tokens[i].word;
        let backed = |lookahead: usize, allow_year: bool| {
            tokens[i + 1..n.min(i + 1 + lookahead)].iter().any(|t| {
                t.word.is_strong()
                    || t.word.is_weak()
                    || matches!(t.word, Word::EditionSuffix)
                    || (allow_year && matches!(t.word, Word::Year(_)))
            })
        };
        let ends = match w {
            // "The.French.Movie" keeps its word; "Movie.German.DL" / "German.BluRay" do not
            Word::Language(_) => tokens.get(i + 1).is_some_and(|t| {
                t.word.is_strong() || (t.word.is_weak() && !matches!(t.word, Word::Edition))
            }),
            Word::Edition => backed(3, true) || i + 1 == n,
            Word::EditionPrefix => matches!(tokens.get(i + 1).map(|t| &t.word), Some(Word::EditionSuffix)),
            w if w.is_weak() => backed(2, true),
            _ => false,
        };
        if ends {
            end = i;
            break;
        }
    }
    (end, year.filter(|&y| y >= end))
}

/// Turns the raw title slice into display text and alternative titles.
fn clean_title(raw: &str) -> Option<(String, Vec<String>)> {
    // split on separators but remember them, to rebuild acronyms like U.N.C.L.E.
    let mut words: Vec<(String, char)> = Vec::new();
    let mut cur = String::new();
    for c in raw.chars() {
        if matches!(c, '.' | '_' | ' ') {
            if !cur.is_empty() {
                words.push((std::mem::take(&mut cur), c));
            } else if let Some(last) = words.last_mut()
                && last.1 != '.'
            {
                last.1 = c;
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        words.push((cur, ' '));
    }

    let single = |w: &str| w.chars().count() == 1 && w.chars().all(char::is_alphabetic);
    let mut out = String::new();
    let mut i = 0;
    while i < words.len() {
        // a run of 2+ dotted single letters is an acronym
        let mut j = i;
        if single(&words[i].0) && words[i].1 == '.' {
            j = i + 1;
            while j < words.len() && single(&words[j].0) && words[j - 1].1 == '.' {
                j += 1;
            }
        }
        if !out.is_empty() {
            out.push(' ');
        }
        if j - i >= 2 {
            for (w, _) in &words[i..j] {
                out.push_str(w);
                out.push('.');
            }
            i = j;
            continue;
        }
        let (w, sep) = &words[i];
        out.push_str(w);
        let abbr = matches!(w.to_ascii_lowercase().as_str(), "dr" | "mr" | "mrs" | "ms" | "st" | "jr" | "vs");
        if abbr && *sep == '.' {
            out.push('.');
        }
        i += 1;
    }

    let unbracketed = BRACKET_AKA.replace(&out, " AKA $1").into_owned();
    // opening brackets are kept at the start ("(500) Days...") but dropped at the end ("Title (")
    let trim = |t: &str| {
        t.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–'))
            .trim_end_matches(|c: char| {
                c.is_whitespace() || matches!(c, '-' | '–' | '(' | '[' | '{' | ',' | ':')
            })
            .to_owned()
    };
    let mut titles = AKA.split(&unbracketed).map(trim).filter(|t| !t.is_empty());
    let title = titles.next()?;
    let alts = titles.filter(|t| *t != title).collect();
    Some((title, alts))
}

fn quality_of(tokens: &[Token], tail: &str) -> (Quality, Revision) {
    let mut resolution = Resolution::Unknown;
    let mut source = Source::Unknown;
    let mut remux = false;
    let mut disc = false;
    let mut dvdr = false;
    let mut screener = false;
    let mut regional = false;
    let mut encoded = false;
    let mut old_codec = false;
    let mut stream_codec = false;
    let mut complete = false;
    let mut revision = Revision::default();

    for t in tokens {
        match &t.word {
            Word::Resolution(r) => resolution = *r,
            Word::SourceResolution(s, r) => {
                source = *s;
                resolution = *r;
            }
            // the last source mentioned wins: "DVD.Rip...BluRay" is usually a BluRay re-encode
            Word::Source(s) => source = *s,
            Word::Remux => {
                remux = true;
                if t.text.to_ascii_lowercase().starts_with("uhd") {
                    resolution = Resolution::R2160;
                }
            }
            Word::DvdR => dvdr = true,
            Word::Disc => disc = true,
            Word::Screener => screener = true,
            Word::Regional => regional = true,
            Word::Codec { encode, lossy_old, stream } => {
                encoded |= encode;
                old_codec |= lossy_old;
                stream_codec |= stream;
            }
            Word::Proper => revision.version = revision.version.max(2),
            Word::Repack(v) | Word::Version(v) => revision.version = revision.version.max(*v),
            Word::Tag if t.text.eq_ignore_ascii_case("complete") => complete = true,
            _ => {}
        }
        // "REAL" in capitals marks a fixed re-release; lowercase is just a word
        if t.text == "REAL" {
            revision.real = revision.real.saturating_add(1);
        }
    }
    let rip = tail.to_ascii_lowercase().contains("rip");
    // a Blu-ray named only by its stream format (AVC, VC-1) with no encoder is the untouched disc
    disc |= source == Source::Bluray && (complete || (stream_codec && !encoded));

    let quality = if dvdr {
        Quality::new(Source::Dvd, Resolution::R480, Modifier::Remux)
    } else if screener {
        Quality::new(Source::Dvd, Resolution::R480, Modifier::Screener)
    } else if regional {
        Quality::new(Source::Dvd, Resolution::R480, Modifier::Regional)
    } else if source == Source::Bluray || (source == Source::Unknown && (remux || disc)) {
        bluray(resolution, remux, disc && !remux && !encoded && !rip, old_codec)
    } else {
        match source {
            Source::Webdl | Source::Webrip | Source::Tv => {
                let r = if resolution == Resolution::Unknown { Resolution::R480 } else { resolution };
                Quality::new(source, if r == Resolution::R576 { Resolution::R480 } else { r }, Modifier::None)
            }
            Source::Dvd => Quality::new(Source::Dvd, Resolution::R480, Modifier::None),
            Source::Cam | Source::Telesync | Source::Telecine | Source::Workprint => {
                Quality::new(source, Resolution::Unknown, Modifier::None)
            }
            // a resolution with no source: most likely a TV capture
            _ if resolution != Resolution::Unknown => {
                let r = if resolution == Resolution::R576 { Resolution::R480 } else { resolution };
                Quality::new(Source::Tv, r, Modifier::None)
            }
            _ => Quality::default(),
        }
    };
    (quality, revision)
}

fn bluray(resolution: Resolution, remux: bool, disc: bool, old_codec: bool) -> Quality {
    if disc {
        let r = if resolution == Resolution::R2160 { Resolution::R2160 } else { Resolution::R1080 };
        return Quality::new(Source::Bluray, r, Modifier::Disc);
    }
    if old_codec {
        return Quality::new(Source::Bluray, Resolution::R480, Modifier::None);
    }
    match (resolution, remux) {
        // a 720p remux is not a real remux; treat it as an encode
        (Resolution::R720, _) => Quality::new(Source::Bluray, Resolution::R720, Modifier::None),
        (Resolution::Unknown, true) => Quality::new(Source::Bluray, Resolution::R1080, Modifier::Remux),
        (Resolution::Unknown, false) => Quality::new(Source::Bluray, Resolution::R720, Modifier::None),
        (r @ (Resolution::R1080 | Resolution::R2160), true) => {
            Quality::new(Source::Bluray, r, Modifier::Remux)
        }
        (r, _) => Quality::new(Source::Bluray, r, Modifier::None),
    }
}

/// `...x264-GROUP` or `... - GROUP[site]`: the word after the last hyphen.
fn trailing_group(tail: &str) -> Option<String> {
    let tail = TRAILING_TAG.replace(tail, "");
    let (_, after) = tail.rsplit_once('-')?;
    let g = after.trim().trim_matches('_');
    if !GROUP.is_match(g) {
        return None;
    }
    // "WEB-DL", "DTS-HD", "Blu-ray" end in technical words, not groups
    match classify(g) {
        Word::Plain => Some(g.to_owned()),
        _ => None,
    }
}

fn edition(tokens: &[Token], title_end: usize) -> Option<String> {
    let mut i = title_end;
    while i < tokens.len() {
        let w = &tokens[i].word;
        let starts = matches!(w, Word::Edition)
            || (matches!(w, Word::EditionPrefix)
                && matches!(tokens.get(i + 1).map(|t| &t.word), Some(Word::EditionSuffix)))
            || is_anniversary(tokens, i);
        if starts {
            let mut j = i + 1;
            while j < tokens.len()
                && matches!(tokens[j].word, Word::Edition | Word::EditionPrefix | Word::EditionSuffix)
            {
                j += 1;
            }
            let text: Vec<String> = tokens[i..j].iter().map(|t| title_case(t.text)).collect();
            return Some(text.join(" "));
        }
        i += 1;
    }
    None
}

fn is_anniversary(tokens: &[Token], i: usize) -> bool {
    let t = tokens[i].text.to_ascii_lowercase();
    let ordinal = t.ends_with("th") && t.len() > 2 && t[..t.len() - 2].bytes().all(|b| b.is_ascii_digit());
    ordinal && tokens.get(i + 1).is_some_and(|n| n.text.eq_ignore_ascii_case("anniversary"))
}

fn title_case(w: &str) -> String {
    let lower = w.to_lowercase();
    match lower.as_str() {
        "imax" => "IMAX".into(),
        "directors" => "Director's".into(),
        "collectors" => "Collector's".into(),
        _ => {
            let mut c = lower.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

/// Lowercase, accent-free, punctuation-free form used to compare titles.
/// Leading articles and "and"/"&" differences do not matter.
pub fn normalize_title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = fold_accent(c);
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == '&' {
            out.push_str(" and ");
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace()
        .enumerate()
        .filter(|(i, w)| {
            !(matches!(*w, "a" | "an" | "the" | "and" | "of") && (*i == 0 || *w == "and" || *w == "of"))
        })
        .map(|(_, w)| w)
        .collect::<Vec<_>>()
        .join(" ")
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ă' | 'ą' => 'a',
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ă' | 'Ą' => 'A',
        'è' | 'é' | 'ê' | 'ë' | 'ę' | 'ě' => 'e',
        'È' | 'É' | 'Ê' | 'Ë' | 'Ę' | 'Ě' => 'E',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ő' => 'o',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ő' => 'O',
        'ù' | 'ú' | 'û' | 'ü' | 'ů' | 'ű' => 'u',
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ů' | 'Ű' => 'U',
        'ç' | 'ć' | 'č' => 'c',
        'Ç' | 'Ć' | 'Č' => 'C',
        'ñ' | 'ń' | 'ň' => 'n',
        'Ñ' | 'Ń' | 'Ň' => 'N',
        'ș' | 'ş' | 'ś' | 'š' => 's',
        'Ș' | 'Ş' | 'Ś' | 'Š' => 'S',
        'ț' | 'ţ' | 'ť' => 't',
        'Ț' | 'Ţ' | 'Ť' => 'T',
        'ý' | 'ÿ' => 'y',
        'ž' | 'ź' | 'ż' => 'z',
        'Ž' | 'Ź' | 'Ż' => 'Z',
        'ł' => 'l',
        'Ł' => 'L',
        'ß' => 's',
        other => other,
    }
}
