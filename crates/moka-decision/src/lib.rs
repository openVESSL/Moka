//! Decides which releases are acceptable for a film, an episode or an album, and which one is best.
//!
//! Pure functions: no I/O. Every rejection carries a plain-language reason,
//! and the ranking keeps the criterion that separated the winner from the
//! runner-up so the UI can explain the choice.

use std::cmp::Ordering;
use std::collections::HashMap;

use moka_core::{AudioQuality, EbookFormat, Modifier, Quality, Revision};
use moka_indexers::{Protocol, Release};
use moka_parser::{ParsedEpisode, ParsedRelease, normalize_title, parse_episode, parse_movie};
use serde::{Deserialize, Serialize};

mod album;
mod book;
pub use album::{AlbumTarget, AudioProfile, evaluate_album};
pub use book::{BookProfile, BookTarget, evaluate_book};

/// Runtime assumed when the catalog does not know it yet.
pub const DEFAULT_RUNTIME_MINUTES: u32 = 110;
/// Episode runtime assumed when the catalog does not know it yet.
pub const DEFAULT_EPISODE_MINUTES: u32 = 45;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct SizeLimit {
    /// MB per minute of runtime.
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub preferred: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Term {
    pub term: String,
    pub score: i32,
}

#[derive(Debug, Clone)]
pub struct Profile {
    /// Allowed qualities, best first.
    pub qualities: Vec<Quality>,
    /// Once a file at or above this quality exists, stop upgrading.
    pub cutoff: Quality,
    pub upgrades_allowed: bool,
    pub min_seeders: u32,
    /// Keyed by quality key (`webdl-1080p`).
    pub size_limits: HashMap<String, SizeLimit>,
    pub preferred_terms: Vec<Term>,
}

impl Profile {
    /// Position in the profile; lower is better. `None` if not allowed.
    pub fn rank_of(&self, q: &Quality) -> Option<usize> {
        self.qualities.iter().position(|p| p == q)
    }
}

/// The episodes being searched for, when the target is a show.
#[derive(Debug, Clone, Default)]
pub struct EpisodeTarget {
    pub season: u32,
    /// Episodes that may be grabbed. Empty means the whole season, as one pack.
    pub episodes: Vec<u32>,
    /// Episodes in the season, to size a season pack.
    pub season_episodes: u32,
    /// Air date (`YYYY-MM-DD`) of each episode in the season, for daily shows.
    pub air_dates: Vec<(u32, String)>,
    /// Absolute number of each episode in the season (anime), counted from the
    /// first episode of season 1.
    pub absolute: Vec<(u32, u32)>,
}

/// The film or show being searched for. For a show, `runtime_minutes` is
/// the length of one episode.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<u16>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<i64>,
    pub runtime_minutes: Option<u32>,
    /// Quality of the file already in the library, if any.
    pub existing: Option<Quality>,
    pub episodes: Option<EpisodeTarget>,
}

/// A search result plus what we know about where it came from.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub release: Release,
    pub indexer_id: i64,
    pub indexer_name: String,
    /// Lower is preferred.
    pub indexer_priority: i32,
    pub blocklisted: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Rejection {
    pub code: &'static str,
    pub message: String,
    /// False when trying again later could change the outcome.
    pub permanent: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluated {
    pub candidate_index: usize,
    pub title: String,
    pub parsed: Option<ParsedRelease>,
    pub quality: Option<Quality>,
    /// Audio quality, for an album or an audiobook.
    pub audio: Option<AudioQuality>,
    /// File format, for an ebook.
    pub ebook: Option<EbookFormat>,
    pub score: i32,
    /// Episodes the release holds, for a show.
    pub covers: Vec<u32>,
    pub rejections: Vec<Rejection>,
    /// Why this release ranks above the next accepted one.
    pub beats_next_by: Option<&'static str>,
}

impl Evaluated {
    pub fn accepted(&self) -> bool {
        self.rejections.is_empty()
    }

    /// The quality as stored with a grab: `webdl-1080p` for video, `flac` for
    /// audio, `epub` for an ebook.
    pub fn quality_key(&self) -> Option<String> {
        self.quality
            .map(|q| q.key())
            .or_else(|| self.audio.map(|a| a.to_string()))
            .or_else(|| self.ebook.map(|f| f.to_string()))
    }

    /// The quality as shown to people.
    pub fn quality_label(&self) -> Option<String> {
        self.quality
            .map(|q| q.to_string())
            .or_else(|| self.audio.map(|a| a.label().to_owned()))
            .or_else(|| self.ebook.map(|f| f.label().to_owned()))
    }
}

fn reject(code: &'static str, message: impl Into<String>, permanent: bool) -> Rejection {
    Rejection { code, message: message.into(), permanent }
}

/// Evaluates every candidate and returns them best first: accepted ones in
/// ranking order, then rejected ones.
pub fn evaluate(target: &Target, profile: &Profile, candidates: &[Candidate]) -> Vec<Evaluated> {
    let default = if target.episodes.is_some() { DEFAULT_EPISODE_MINUTES } else { DEFAULT_RUNTIME_MINUTES };
    let runtime = target.runtime_minutes.filter(|m| *m > 0).unwrap_or(default);
    let mut out: Vec<Evaluated> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (parsed, episode) = match &target.episodes {
                Some(_) => match parse_episode(&c.release.title) {
                    Some(e) => (Some(e.release.clone()), Some(e)),
                    None => (None, None),
                },
                None => (parse_movie(&c.release.title), None),
            };
            let mut covers = Vec::new();
            let rejections = match (&parsed, &target.episodes) {
                (None, Some(_)) => {
                    vec![reject(
                        "unparseable",
                        "Could not read a season or episode from the release name",
                        true,
                    )]
                }
                (None, None) => {
                    vec![reject("unparseable", "Could not read a film title from the release name", true)]
                }
                (Some(p), None) => rules(target, profile, runtime, c, p),
                (Some(p), Some(want)) => {
                    let e = episode.as_ref().expect("parsed with episode");
                    let (held, mut out) = episode_rules(want, e);
                    covers = held;
                    let minutes = runtime * (covers.len().max(1) as u32);
                    out.extend(rules(target, profile, minutes, c, p));
                    out
                }
            };
            let score = parsed.as_ref().map(|_| term_score(profile, &c.release.title)).unwrap_or(0);
            Evaluated {
                candidate_index: i,
                title: c.release.title.clone(),
                quality: parsed.as_ref().map(|p| p.quality),
                audio: None,
                ebook: None,
                parsed,
                score,
                covers,
                rejections,
                beats_next_by: None,
            }
        })
        .collect();

    out.sort_by(|a, b| match (a.accepted(), b.accepted()) {
        (true, true) => compare(profile, runtime, candidates, a, b).0,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => Ordering::Equal,
    });
    for i in 0..out.len().saturating_sub(1) {
        if out[i].accepted() && out[i + 1].accepted() {
            out[i].beats_next_by = compare(profile, runtime, candidates, &out[i], &out[i + 1]).1;
        }
    }
    out
}

/// The episodes a release holds (numbers inside the season), and why it is
/// not the season or episodes wanted, if it is not. Air dates and absolute
/// numbers are turned into episode numbers first.
fn episode_rules(want: &EpisodeTarget, e: &ParsedEpisode) -> (Vec<u32>, Vec<Rejection>) {
    let one = |r: Rejection| (vec![], vec![r]);
    let held: Vec<u32> = if let Some(date) = &e.air_date {
        match want.air_dates.iter().find(|(_, d)| d == date) {
            Some((n, _)) => vec![*n],
            None => {
                return one(reject("wrong_episode", format!("Nothing in this season aired on {date}"), true));
            }
        }
    } else if !e.absolute.is_empty() {
        let mapped: Vec<u32> = e
            .absolute
            .iter()
            .filter_map(|a| want.absolute.iter().find(|(x, _)| x == a).map(|(_, n)| *n))
            .collect();
        if mapped.len() != e.absolute.len() {
            let nums: Vec<String> = e.absolute.iter().map(u32::to_string).collect();
            return one(reject(
                "wrong_season",
                format!("Episode {} is not in season {}", nums.join("-"), want.season),
                true,
            ));
        }
        mapped
    } else if e.seasons != [want.season] {
        let s: Vec<String> = e.seasons.iter().map(|s| format!("S{s:02}")).collect();
        return one(reject("wrong_season", format!("Holds {}, not S{:02}", s.join(", "), want.season), true));
    } else if e.is_season_pack() {
        (1..=want.season_episodes).collect()
    } else {
        e.episodes.clone()
    };
    let pack = e.is_season_pack();
    let rejections = match (want.episodes.is_empty(), pack) {
        (true, false) => vec![reject("not_season_pack", "A single episode, not the whole season", true)],
        (false, true) => {
            vec![reject("season_pack", "The whole season, but only some episodes are wanted", true)]
        }
        (false, false) => match held.iter().find(|n| !want.episodes.contains(n)) {
            Some(n) => vec![reject(
                "wrong_episode",
                format!("Episode S{:02}E{n:02} is not wanted", want.season),
                true,
            )],
            None => vec![],
        },
        (true, true) => vec![],
    };
    (held, rejections)
}

fn rules(
    target: &Target,
    profile: &Profile,
    runtime: u32,
    c: &Candidate,
    p: &ParsedRelease,
) -> Vec<Rejection> {
    let r = &c.release;
    let mut out = Vec::new();

    if let Some(why) = mismatch(target, r, p) {
        out.push(reject("wrong_film", why, true));
    }
    match profile.rank_of(&p.quality) {
        None if p.quality.modifier == Modifier::Disc => {
            out.push(reject("disc", "Full disc images cannot be played as a single file", true))
        }
        None if p.quality.is_unknown() => {
            out.push(reject("unknown_quality", "Quality could not be determined", true))
        }
        None => out.push(reject("quality", format!("{} is not in the quality profile", p.quality), true)),
        Some(rank) => {
            if let Some(existing) = target.existing {
                let current = profile.rank_of(&existing);
                let at_cutoff =
                    current.is_some_and(|cur| profile.rank_of(&profile.cutoff).is_some_and(|cut| cur <= cut));
                if !profile.upgrades_allowed || at_cutoff {
                    out.push(reject(
                        "not_upgrade",
                        "The film is already in the library at the wanted quality",
                        true,
                    ));
                } else if current.is_some_and(|cur| rank >= cur) {
                    out.push(reject("not_upgrade", format!("Not better than the current {existing}"), true));
                }
            }
        }
    }
    if let Some(size) = r.size_bytes {
        let limit = profile.size_limits.get(&p.quality.key()).copied().unwrap_or_default();
        let mb = size as f64 / 1_048_576.0;
        let minutes = f64::from(runtime);
        if let Some(min) = limit.min.filter(|m| *m > 0.0)
            && mb < min * minutes
        {
            out.push(reject(
                "too_small",
                format!("{} is below the {} minimum for this film", human(size), human_mb(min * minutes)),
                true,
            ));
        }
        if let Some(max) = limit.max.filter(|m| *m > 0.0)
            && mb > max * minutes
        {
            out.push(reject(
                "too_large",
                format!("{} is above the {} maximum for this film", human(size), human_mb(max * minutes)),
                true,
            ));
        }
    }
    if r.protocol == Protocol::Torrent && r.seeders.unwrap_or(0) < profile.min_seeders {
        out.push(reject(
            "seeders",
            format!("{} seeders, at least {} needed", r.seeders.unwrap_or(0), profile.min_seeders),
            false,
        ));
    }
    if c.blocklisted {
        out.push(reject("blocklisted", "This release failed before and was blocklisted", true));
    }
    if p.hardcoded_subs {
        out.push(reject("hardcoded_subs", "Subtitles are burned into the picture", true));
    }
    if r.download_url.is_none() && r.magnet_url.is_none() {
        out.push(reject("no_link", "The indexer gave no download link", true));
    }
    out
}

/// Why a release is not the target film, if it is not.
fn mismatch(target: &Target, r: &Release, p: &ParsedRelease) -> Option<String> {
    // ids from the indexer are the strongest signal both ways
    if let (Some(a), Some(b)) = (r.tmdb_id, target.tmdb_id) {
        return (a != b).then(|| "The indexer says this is a different title".to_owned());
    }
    // for shows the indexer's IMDb id may be the episode's, not the show's
    if let (Some(a), Some(b), None) = (&r.imdb_id, &target.imdb_id, &target.episodes) {
        return (!a.eq_ignore_ascii_case(b)).then(|| "The indexer says this is a different film".to_owned());
    }
    let wanted: Vec<String> = std::iter::once(&target.title)
        .chain(target.original_title.as_ref())
        .map(|t| normalize_title(t))
        .collect();
    let found = std::iter::once(&p.title).chain(&p.alt_titles).map(|t| normalize_title(t));
    let title_ok = found.into_iter().any(|f| wanted.contains(&f));
    if !title_ok {
        return Some(format!("Title \"{}\" does not match", p.title));
    }
    match (p.year, target.year) {
        (Some(a), Some(b)) if a.abs_diff(b) > 1 => Some(format!("Year {a} does not match {b}")),
        _ => None,
    }
}

fn term_score(profile: &Profile, title: &str) -> i32 {
    let lower = title.to_lowercase();
    profile.preferred_terms.iter().filter(|t| lower.contains(&t.term.to_lowercase())).map(|t| t.score).sum()
}

/// Orders two accepted releases (Less = `a` is better) and names the criterion that decided.
fn compare(
    profile: &Profile,
    runtime: u32,
    candidates: &[Candidate],
    a: &Evaluated,
    b: &Evaluated,
) -> (Ordering, Option<&'static str>) {
    let (ca, cb) = (&candidates[a.candidate_index], &candidates[b.candidate_index]);
    let (pa, pb) = (a.parsed.as_ref().expect("accepted"), b.parsed.as_ref().expect("accepted"));
    let rank = |p: &ParsedRelease| profile.rank_of(&p.quality).unwrap_or(usize::MAX);
    let seed_bucket =
        |c: &Candidate| c.release.seeders.map(|s| (f64::from(s.max(1))).log10().round() as i32).unwrap_or(0);
    let size_distance = |p: &ParsedRelease, c: &Candidate| -> i64 {
        let Some(size) = c.release.size_bytes else { return i64::MAX };
        let mb = size as f64 / 1_048_576.0;
        let preferred = profile.size_limits.get(&p.quality.key()).and_then(|l| l.preferred);
        match preferred {
            // closer to the preferred size wins, in 200 MB steps
            Some(pref) => ((mb - pref * f64::from(runtime)).abs() / 200.0).round() as i64,
            // no preference: bigger wins
            None => -((mb / 200.0).round() as i64),
        }
    };
    let steps: [(&'static str, Ordering); 5] = [
        ("quality", rank(pa).cmp(&rank(pb))),
        ("revision", cmp_rev(&pb.revision, &pa.revision)),
        ("preferred words", b.score.cmp(&a.score)),
        ("indexer priority", ca.indexer_priority.cmp(&cb.indexer_priority)),
        ("seeders", seed_bucket(cb).cmp(&seed_bucket(ca))),
    ];
    for (name, ord) in steps {
        if ord != Ordering::Equal {
            return (ord, Some(name));
        }
    }
    match size_distance(pa, ca).cmp(&size_distance(pb, cb)) {
        Ordering::Equal => (Ordering::Equal, None),
        ord => (ord, Some("size")),
    }
}

fn cmp_rev(a: &Revision, b: &Revision) -> Ordering {
    a.cmp(b)
}

fn human(bytes: u64) -> String {
    human_mb(bytes as f64 / 1_048_576.0)
}

fn human_mb(mb: f64) -> String {
    if mb >= 1024.0 { format!("{:.1} GB", mb / 1024.0) } else { format!("{mb:.0} MB") }
}

#[cfg(test)]
mod tests;
