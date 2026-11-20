//! Which releases are acceptable for an ebook or an audiobook, and which
//! one is best.

use std::cmp::Ordering;

use moka_core::{AudioQuality, EbookFormat};
use moka_indexers::Protocol;
use moka_parser::{audio_quality, book_matches, parse_book};
use serde::{Deserialize, Serialize};

use crate::{Candidate, Evaluated, Rejection, reject};

/// Ebook formats accepted, best first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookProfile {
    pub formats: Vec<EbookFormat>,
    #[serde(default = "default_min_seeders")]
    pub min_seeders: u32,
}

fn default_min_seeders() -> u32 {
    1
}

impl Default for BookProfile {
    fn default() -> Self {
        Self {
            formats: vec![EbookFormat::Epub, EbookFormat::Azw3, EbookFormat::Mobi],
            min_seeders: default_min_seeders(),
        }
    }
}

impl BookProfile {
    /// Position in the profile; lower is better. `None` if not allowed.
    pub fn rank_of(&self, f: EbookFormat) -> Option<usize> {
        self.formats.iter().position(|p| *p == f)
    }
}

/// The book being searched for.
#[derive(Debug, Clone, Default)]
pub struct BookTarget {
    pub author: String,
    pub title: String,
    pub audiobook: bool,
}

fn words(name: &str) -> Vec<String> {
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The name announces audio: "Audiobook", "M4B", "MP3", "Unabridged".
fn sounds_like_audio(name: &str) -> bool {
    words(name).iter().any(|w| {
        matches!(
            w.as_str(),
            "audiobook" | "audiobooks" | "audible" | "m4b" | "m4a" | "mp3" | "unabridged" | "narrated"
        )
    })
}

/// M4B keeps chapters in one file: preferred for audiobooks.
fn is_m4b(name: &str) -> bool {
    words(name).iter().any(|w| w == "m4b")
}

/// Evaluates every candidate for a book and returns them best first:
/// accepted ones in ranking order, then rejected ones.
pub fn evaluate_book(target: &BookTarget, profile: &BookProfile, candidates: &[Candidate]) -> Vec<Evaluated> {
    let mut out: Vec<Evaluated> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let name = &c.release.title;
            let parsed = parse_book(name);
            let audio = audio_quality(name);
            Evaluated {
                candidate_index: i,
                title: name.clone(),
                parsed: None,
                quality: None,
                audio: (target.audiobook && audio != AudioQuality::Unknown).then_some(audio),
                ebook: (!target.audiobook && parsed.format != EbookFormat::Unknown).then_some(parsed.format),
                score: 0,
                covers: Vec::new(),
                rejections: rules(target, profile, c, parsed.format),
                beats_next_by: None,
            }
        })
        .collect();

    out.sort_by(|a, b| match (a.accepted(), b.accepted()) {
        (true, true) => compare(target, profile, candidates, a, b).0,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => Ordering::Equal,
    });
    for i in 0..out.len().saturating_sub(1) {
        if out[i].accepted() && out[i + 1].accepted() {
            out[i].beats_next_by = compare(target, profile, candidates, &out[i], &out[i + 1]).1;
        }
    }
    out
}

fn rules(target: &BookTarget, profile: &BookProfile, c: &Candidate, format: EbookFormat) -> Vec<Rejection> {
    let r = &c.release;
    let mut out = Vec::new();
    if !book_matches(&r.title, &target.author, &target.title) {
        out.push(reject(
            "wrong_book",
            format!("The name does not hold \"{}\" by {}", target.title, target.author),
            true,
        ));
    }
    if target.audiobook {
        if format != EbookFormat::Unknown && !sounds_like_audio(&r.title) {
            out.push(reject("ebook", format!("{} is an ebook, not an audiobook", format.label()), true));
        }
    } else if sounds_like_audio(&r.title) {
        out.push(reject("audiobook", "This is an audiobook, not an ebook", true));
    } else {
        match profile.rank_of(format) {
            Some(_) => {}
            None if format == EbookFormat::Unknown => {
                out.push(reject("unknown_quality", "The release name does not say the ebook format", true))
            }
            None => {
                out.push(reject("quality", format!("{} is not in the book profile", format.label()), true))
            }
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
    if r.download_url.is_none() && r.magnet_url.is_none() {
        out.push(reject("no_link", "The indexer gave no download link", true));
    }
    out
}

/// Orders two accepted releases (Less = `a` is better) and names the criterion that decided.
fn compare(
    target: &BookTarget,
    profile: &BookProfile,
    candidates: &[Candidate],
    a: &Evaluated,
    b: &Evaluated,
) -> (Ordering, Option<&'static str>) {
    let (ca, cb) = (&candidates[a.candidate_index], &candidates[b.candidate_index]);
    let rank = |e: &Evaluated| {
        if target.audiobook {
            usize::from(!is_m4b(&e.title))
        } else {
            e.ebook.and_then(|f| profile.rank_of(f)).unwrap_or(usize::MAX)
        }
    };
    let retail = |c: &Candidate| parse_book(&c.release.title).retail;
    let seed_bucket =
        |c: &Candidate| c.release.seeders.map(|s| (f64::from(s.max(1))).log10().round() as i32).unwrap_or(0);
    let steps: [(&'static str, Ordering); 4] = [
        ("format", rank(a).cmp(&rank(b))),
        ("retail", retail(cb).cmp(&retail(ca))),
        ("indexer priority", ca.indexer_priority.cmp(&cb.indexer_priority)),
        ("seeders", seed_bucket(cb).cmp(&seed_bucket(ca))),
    ];
    for (name, ord) in steps {
        if ord != Ordering::Equal {
            return (ord, Some(name));
        }
    }
    (Ordering::Equal, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moka_indexers::Release;

    fn cand(title: &str, seeders: u32) -> Candidate {
        Candidate {
            release: Release {
                title: title.into(),
                guid: title.into(),
                protocol: Protocol::Torrent,
                download_url: Some(format!("https://idx.local/{title}.torrent")),
                magnet_url: None,
                info_hash: None,
                size_bytes: Some(3_000_000),
                published_at: None,
                seeders: Some(seeders),
                peers: None,
                grabs: None,
                imdb_id: None,
                tmdb_id: None,
                categories: vec![7020],
                freeleech: false,
                info_url: None,
            },
            indexer_id: 1,
            indexer_name: "Idx".into(),
            indexer_priority: 25,
            blocklisted: false,
        }
    }

    fn dune(audiobook: bool) -> BookTarget {
        BookTarget { author: "Frank Herbert".into(), title: "Dune".into(), audiobook }
    }

    #[test]
    fn picks_the_best_ebook() {
        let list = [
            cand("Frank Herbert - Dune (1965) [mobi]", 300),
            cand("Frank.Herbert.-.Dune.2005.RETAIL.ePub.eBook-GRP", 5),
            cand("Frank Herbert - Dune (epub)", 50),
            cand("Frank Herbert - Dune (pdf)", 900),
            cand("Frank Herbert - Dune [Unabridged] (m4b)", 900),
            cand("Frank Herbert - Dune Messiah (epub)", 900),
            cand("Dune", 900),
        ];
        let out = evaluate_book(&dune(false), &BookProfile::default(), &list);
        let ok: Vec<&str> = out.iter().filter(|e| e.accepted()).map(|e| e.title.as_str()).collect();
        assert_eq!(
            ok,
            [
                "Frank.Herbert.-.Dune.2005.RETAIL.ePub.eBook-GRP",
                "Frank Herbert - Dune (epub)",
                "Frank Herbert - Dune (1965) [mobi]"
            ],
            "EPUB first, retail before the rest"
        );
        assert_eq!((out[0].beats_next_by, out[1].beats_next_by), (Some("retail"), Some("format")));
        assert_eq!(out[0].quality_key().as_deref(), Some("epub"));
        let codes: Vec<&str> = out.iter().filter(|e| !e.accepted()).map(|e| e.rejections[0].code).collect();
        assert_eq!(codes, ["quality", "audiobook", "wrong_book", "wrong_book"]);
    }

    #[test]
    fn picks_the_best_audiobook() {
        let list = [
            cand("Frank Herbert - Dune [Unabridged] MP3 64kbps", 80),
            cand("Frank Herbert - Dune (Unabridged) [M4B]", 10),
            cand("Frank Herbert - Dune (epub)", 500),
            cand("Frank Herbert - Dune", 0),
        ];
        let out = evaluate_book(&dune(true), &BookProfile::default(), &list);
        assert_eq!(out[0].title, "Frank Herbert - Dune (Unabridged) [M4B]");
        assert_eq!(out[0].beats_next_by, Some("format"));
        assert_eq!(out[1].quality_key().as_deref(), Some("mp3"));
        let codes: Vec<&str> = out.iter().filter(|e| !e.accepted()).map(|e| e.rejections[0].code).collect();
        assert_eq!(codes, ["ebook", "seeders"]);
    }
}
