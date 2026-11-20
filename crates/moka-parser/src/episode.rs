//! Episode release and file names: `S01E02`, `1x02`, season packs, daily
//! shows (`2024.01.15`) and anime numbering (`Show - 05`).

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::words::Word;
use crate::{ParsedRelease, Token, prepare, release, tokenize};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedEpisode {
    /// Show title, year, quality and the rest, read as for films. The title
    /// is empty for bare file names like `S01E02.mkv`.
    pub release: ParsedRelease,
    /// Seasons covered; more than one for multi-season packs.
    pub seasons: Vec<u32>,
    /// Episode numbers inside the season; empty for a whole season.
    pub episodes: Vec<u32>,
    /// Absolute episode numbers (anime), for names without a season.
    pub absolute: Vec<u32>,
    /// Air date of daily shows, `YYYY-MM-DD`.
    pub air_date: Option<String>,
}

impl ParsedEpisode {
    /// A whole season (or several) in one release.
    pub fn is_season_pack(&self) -> bool {
        !self.seasons.is_empty() && self.episodes.is_empty()
    }
}

/// A range longer than this is a misread, not a release.
const MAX_RANGE: u32 = 200;

static SEASON_EPISODES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^s(\d{1,2})((?:[-_]?e\d{1,3})+)(?:-(\d{1,3}))?$").unwrap());
static EPISODE_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)e(\d{1,3})").unwrap());
static SEASONS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^s(\d{1,2})-s?(\d{1,2})$").unwrap());
static SEASON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^s(\d{1,2})$").unwrap());
static CROSS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(\d{1,2})x(\d{2,3})(?:-(?:\d{1,2}x)?(\d{2,3}))?$").unwrap());
static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^((?:19|20)\d{2})-(\d{2})-(\d{2})$").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(\d{1,4})(?:v\d)?$").unwrap());
static NUMBER_RANGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,4})-(\d{1,4})$").unwrap());

#[derive(Default)]
struct Marker {
    seasons: Vec<u32>,
    episodes: Vec<u32>,
    absolute: Vec<u32>,
    air_date: Option<String>,
}

/// Parses an episode or season release name, or an episode file name.
/// Returns `None` when the name has no season, episode or air date.
pub fn parse_episode(name: &str) -> Option<ParsedEpisode> {
    let original = name.trim();
    let (s, group) = prepare(original)?;
    let tokens = tokenize(&s);
    let (at, marker) = (0..tokens.len()).find_map(|i| marker(&s, &tokens, i).map(|m| (i, m)))?;
    // "Show.2019.S01E02": the year belongs to the show
    let year = at.checked_sub(1).filter(|&i| i > 0 && matches!(tokens[i].word, Word::Year(_)));
    let title_end = year.unwrap_or(at);
    let mut release = release(original, &s, &tokens, title_end, year, group);
    // "01x05 - Pilot": after a spaced dash comes the episode title, not a group
    if release.group.as_ref().is_some_and(|g| s.ends_with(&format!(" - {g}"))) {
        release.group = None;
    }
    Some(ParsedEpisode {
        release,
        seasons: marker.seasons,
        episodes: marker.episodes,
        absolute: marker.absolute,
        air_date: marker.air_date,
    })
}

fn num(s: &str) -> u32 {
    s.parse().unwrap_or(0)
}

/// `a..=b` when it is a sane range, else just `a`.
fn range(a: u32, b: u32) -> Vec<u32> {
    if b > a && b - a <= MAX_RANGE { (a..=b).collect() } else { vec![a] }
}

/// Text between the previous token and this one, e.g. `" - "`.
fn gap<'a>(s: &'a str, tokens: &[Token], i: usize) -> &'a str {
    let start = tokens[i].text.as_ptr() as usize - s.as_ptr() as usize;
    let prev = if i == 0 { 0 } else { tokens[i - 1].end };
    s.get(prev..start).unwrap_or("")
}

fn marker(s: &str, tokens: &[Token], i: usize) -> Option<Marker> {
    let t = tokens[i].text;
    let next = tokens.get(i + 1).map(|t| t.text);

    if let Some(c) = SEASON_EPISODES.captures(t) {
        let mut episodes: Vec<u32> = EPISODE_NUMBER.captures_iter(&c[2]).map(|e| num(&e[1])).collect();
        let last = c.get(3).map(|m| num(m.as_str())).or(if episodes.len() > 1 && t.contains('-') {
            episodes.last().copied()
        } else {
            None
        });
        // "E01-E03" and "E01-03" are ranges; "E01E02" is a list
        if let Some(last) = last {
            episodes = range(episodes[0], last);
        }
        return Some(Marker { seasons: vec![num(&c[1])], episodes, ..Default::default() });
    }
    if let Some(c) = SEASONS.captures(t) {
        return Some(Marker { seasons: range(num(&c[1]), num(&c[2])), ..Default::default() });
    }
    if let Some(c) = SEASON.captures(t) {
        let mut m = Marker { seasons: vec![num(&c[1])], ..Default::default() };
        // "Show S2 - 07": an anime episode inside season 2
        if let Some(e) = next.and_then(|n| NUMBER.captures(n))
            && gap(s, tokens, i + 1).contains('-')
        {
            m.episodes = vec![num(&e[1])];
        }
        return Some(m);
    }
    if let Some(c) = CROSS.captures(t) {
        let first = num(&c[2]);
        let episodes = c.get(3).map_or(vec![first], |l| range(first, num(l.as_str())));
        return Some(Marker { seasons: vec![num(&c[1])], episodes, ..Default::default() });
    }
    if t.eq_ignore_ascii_case("season") || t.eq_ignore_ascii_case("seasons") {
        let n = next?;
        let seasons = match (NUMBER_RANGE.captures(n), n.len() <= 2 && n.bytes().all(|b| b.is_ascii_digit()))
        {
            (Some(r), _) => range(num(&r[1]), num(&r[2])),
            (None, true) => vec![num(n)],
            _ => return None,
        };
        return Some(Marker { seasons, ..Default::default() });
    }
    if let Some(c) = DATE.captures(t) {
        return date(&c[1], &c[2], &c[3]);
    }
    if matches!(tokens[i].word, Word::Year(_))
        && let (Some(m), Some(d)) = (next, tokens.get(i + 2).map(|t| t.text))
        && m.len() == 2
        && d.len() == 2
        && let Some(found) = date(t, m, d)
    {
        return Some(found);
    }
    // anime: "Show - 05", "Show - 01-12" after a title
    if i > 0 && gap(s, tokens, i).contains('-') {
        if let Some(c) = NUMBER_RANGE.captures(t) {
            return Some(Marker { absolute: range(num(&c[1]), num(&c[2])), ..Default::default() });
        }
        if let Some(c) = NUMBER.captures(t)
            && !matches!(tokens[i].word, Word::Year(_))
        {
            return Some(Marker { absolute: vec![num(&c[1])], ..Default::default() });
        }
    }
    None
}

fn date(y: &str, m: &str, d: &str) -> Option<Marker> {
    let (mn, dn) = (num(m), num(d));
    ((1..=12).contains(&mn) && (1..=31).contains(&dn))
        .then(|| Marker { air_date: Some(format!("{y}-{m}-{d}")), ..Default::default() })
}
