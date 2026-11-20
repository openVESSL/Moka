//! Which releases are acceptable for an album, and which one is best.

use std::cmp::Ordering;

use moka_core::AudioQuality;
use moka_indexers::Protocol;
use moka_parser::{ParsedAlbum, normalize_title, parse_album};
use serde::{Deserialize, Serialize};

use crate::{Candidate, Evaluated, Rejection, reject};

/// Audio qualities accepted for music, best first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioProfile {
    pub qualities: Vec<AudioQuality>,
    #[serde(default = "default_min_seeders")]
    pub min_seeders: u32,
}

fn default_min_seeders() -> u32 {
    1
}

impl Default for AudioProfile {
    fn default() -> Self {
        Self {
            qualities: vec![AudioQuality::Flac, AudioQuality::Mp3_320, AudioQuality::Mp3V0],
            min_seeders: default_min_seeders(),
        }
    }
}

impl AudioProfile {
    /// Position in the profile; lower is better. `None` if not allowed.
    pub fn rank_of(&self, q: AudioQuality) -> Option<usize> {
        self.qualities.iter().position(|p| *p == q)
    }
}

/// The album being searched for.
#[derive(Debug, Clone, Default)]
pub struct AlbumTarget {
    pub artist: String,
    pub album: String,
}

/// Evaluates every candidate for an album and returns them best first:
/// accepted ones in ranking order, then rejected ones.
pub fn evaluate_album(
    target: &AlbumTarget,
    profile: &AudioProfile,
    candidates: &[Candidate],
) -> Vec<Evaluated> {
    let mut out: Vec<Evaluated> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let parsed = parse_album(&c.release.title);
            let rejections = match &parsed {
                None => vec![reject(
                    "unparseable",
                    "Could not read an artist and album from the release name",
                    true,
                )],
                Some(p) => rules(target, profile, c, p),
            };
            Evaluated {
                candidate_index: i,
                title: c.release.title.clone(),
                parsed: None,
                quality: None,
                audio: parsed.as_ref().map(|p| p.quality),
                score: 0,
                covers: Vec::new(),
                rejections,
                beats_next_by: None,
            }
        })
        .collect();

    out.sort_by(|a, b| match (a.accepted(), b.accepted()) {
        (true, true) => compare(profile, candidates, a, b).0,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => Ordering::Equal,
    });
    for i in 0..out.len().saturating_sub(1) {
        if out[i].accepted() && out[i + 1].accepted() {
            out[i].beats_next_by = compare(profile, candidates, &out[i], &out[i + 1]).1;
        }
    }
    out
}

fn rules(target: &AlbumTarget, profile: &AudioProfile, c: &Candidate, p: &ParsedAlbum) -> Vec<Rejection> {
    let r = &c.release;
    let mut out = Vec::new();
    if normalize_title(&p.artist) != normalize_title(&target.artist) {
        out.push(reject("wrong_artist", format!("Artist \"{}\" does not match", p.artist), true));
    } else if !same_album(&target.album, &p.album) {
        out.push(reject("wrong_album", format!("Album \"{}\" does not match", p.album), true));
    }
    match profile.rank_of(p.quality) {
        Some(_) => {}
        None if p.quality == AudioQuality::Unknown => {
            out.push(reject("unknown_quality", "The release name does not say the audio format", true))
        }
        None => {
            out.push(reject("quality", format!("{} is not in the music profile", p.quality.label()), true))
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

/// Release names drop what is in brackets ("(Deluxe Edition)"), so the
/// catalog title is compared with and without it.
fn same_album(wanted: &str, found: &str) -> bool {
    let found = normalize_title(found);
    let bare: String = {
        let mut depth = 0u32;
        wanted
            .chars()
            .filter(|c| match c {
                '(' | '[' => {
                    depth += 1;
                    false
                }
                ')' | ']' => {
                    depth = depth.saturating_sub(1);
                    false
                }
                _ => depth == 0,
            })
            .collect()
    };
    found == normalize_title(wanted) || found == normalize_title(&bare)
}

/// Orders two accepted releases (Less = `a` is better) and names the criterion that decided.
fn compare(
    profile: &AudioProfile,
    candidates: &[Candidate],
    a: &Evaluated,
    b: &Evaluated,
) -> (Ordering, Option<&'static str>) {
    let (ca, cb) = (&candidates[a.candidate_index], &candidates[b.candidate_index]);
    let rank = |e: &Evaluated| e.audio.and_then(|q| profile.rank_of(q)).unwrap_or(usize::MAX);
    let seed_bucket =
        |c: &Candidate| c.release.seeders.map(|s| (f64::from(s.max(1))).log10().round() as i32).unwrap_or(0);
    let steps: [(&'static str, Ordering); 3] = [
        ("quality", rank(a).cmp(&rank(b))),
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
                size_bytes: Some(300_000_000),
                published_at: None,
                seeders: Some(seeders),
                peers: None,
                grabs: None,
                imdb_id: None,
                tmdb_id: None,
                categories: vec![3040],
                freeleech: false,
                info_url: None,
            },
            indexer_id: 1,
            indexer_name: "Idx".into(),
            indexer_priority: 25,
            blocklisted: false,
        }
    }

    fn target() -> AlbumTarget {
        AlbumTarget { artist: "The Band".into(), album: "First Light (Remastered)".into() }
    }

    #[test]
    fn picks_the_best_allowed_format() {
        let list = [
            cand("The Band - First Light (2001) [MP3 320]", 200),
            cand("The Band - First Light (2001) [FLAC]", 12),
            cand("Band - First Light 2001 FLAC 24bit", 50),
            cand("The Band - First Light (2001) [MP3 128]", 500),
        ];
        let out = evaluate_album(&target(), &AudioProfile::default(), &list);
        assert_eq!(
            out[0].title, "The Band - First Light (2001) [FLAC]",
            "FLAC first, even with fewer seeders"
        );
        assert_eq!(out[0].beats_next_by, Some("quality"));
        assert_eq!(out[1].audio, Some(AudioQuality::Mp3_320));
        let rejected: Vec<&str> =
            out.iter().filter(|e| !e.accepted()).map(|e| e.rejections[0].code).collect();
        assert_eq!(rejected, ["quality", "quality"], "24-bit and 128 kbps are not in the default profile");
        assert_eq!(out[0].quality_key().as_deref(), Some("flac"));
    }

    #[test]
    fn rejects_other_albums_and_unclear_names() {
        let list = [
            cand("The Band - Second Light (2003) [FLAC]", 10),
            cand("Other Band - First Light (2001) [FLAC]", 10),
            cand("The Band - First Light (2001)", 10),
            cand("TheBandFirstLight", 10),
            cand("The Band - First Light (2001) [FLAC]", 0),
        ];
        let out = evaluate_album(&target(), &AudioProfile::default(), &list);
        let codes: Vec<&str> = out.iter().map(|e| e.rejections[0].code).collect();
        assert_eq!(codes, ["wrong_album", "wrong_artist", "unknown_quality", "unparseable", "seeders"]);
        assert!(!out[4].rejections[0].permanent, "seeders can come back");
    }
}
