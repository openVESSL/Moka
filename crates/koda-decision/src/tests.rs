use super::*;
use koda_core::{Resolution, Source};

fn q(source: Source, res: Resolution) -> Quality {
    Quality::new(source, res, Modifier::None)
}

fn profile() -> Profile {
    let mut size_limits = HashMap::new();
    size_limits.insert(
        "bluray-1080p".into(),
        SizeLimit { min: Some(10.0), max: Some(200.0), preferred: Some(95.0) },
    );
    size_limits
        .insert("webdl-1080p".into(), SizeLimit { min: Some(5.0), max: Some(120.0), preferred: Some(60.0) });
    Profile {
        qualities: vec![
            q(Source::Bluray, Resolution::R1080),
            q(Source::Webdl, Resolution::R1080),
            q(Source::Bluray, Resolution::R720),
        ],
        cutoff: q(Source::Bluray, Resolution::R1080),
        upgrades_allowed: true,
        min_seeders: 1,
        size_limits,
        preferred_terms: vec![Term { term: "atmos".into(), score: 10 }],
    }
}

fn matrix() -> Target {
    Target {
        title: "The Matrix".into(),
        year: Some(1999),
        tmdb_id: Some(603),
        imdb_id: Some("tt0133093".into()),
        runtime_minutes: Some(136),
        ..Default::default()
    }
}

const GB: u64 = 1_073_741_824;

fn cand(title: &str, size_gb: f64, seeders: u32) -> Candidate {
    Candidate {
        release: Release {
            title: title.into(),
            guid: title.into(),
            protocol: Protocol::Torrent,
            download_url: Some("https://x/1.torrent".into()),
            magnet_url: None,
            info_hash: None,
            size_bytes: Some((size_gb * GB as f64) as u64),
            published_at: None,
            seeders: Some(seeders),
            peers: None,
            grabs: None,
            imdb_id: None,
            tmdb_id: None,
            categories: vec![],
            freeleech: false,
            info_url: None,
        },
        indexer_id: 1,
        indexer_name: "test".into(),
        indexer_priority: 25,
        blocklisted: false,
    }
}

fn codes(e: &Evaluated) -> Vec<&'static str> {
    e.rejections.iter().map(|r| r.code).collect()
}

#[test]
fn picks_best_quality_then_explains() {
    let cands = vec![
        cand("The.Matrix.1999.1080p.WEB-DL.DD5.1.H264-GRP", 6.0, 50),
        cand("The.Matrix.1999.1080p.BluRay.x264-GRP", 12.0, 30),
        cand("The.Matrix.1999.720p.BluRay.x264-GRP", 5.0, 200),
    ];
    let out = evaluate(&matrix(), &profile(), &cands);
    assert!(out.iter().all(Evaluated::accepted), "{out:#?}");
    assert_eq!(out[0].title, cands[1].release.title);
    assert_eq!(out[0].beats_next_by, Some("quality"));
}

#[test]
fn rejections_have_reasons() {
    let cands = vec![
        cand("The.Matrix.Reloaded.2003.1080p.BluRay.x264-GRP", 12.0, 30),
        cand("The.Matrix.1999.2160p.UHD.BluRay.x265-GRP", 30.0, 30),
        cand("The.Matrix.1999.1080p.BluRay.x264-GRP", 0.5, 30),
        cand("The.Matrix.1999.1080p.BluRay.x264-GRP", 12.0, 0),
        cand("The.Matrix.1999.COMPLETE.BLURAY-GRP", 40.0, 30),
        cand("The.Matrix.1999.1080p.KORSUB.WEBRip.x264-GRP", 5.0, 30),
    ];
    let out = evaluate(&matrix(), &profile(), &cands);
    let by_title = |i: usize| out.iter().find(|e| e.candidate_index == i).unwrap();
    assert_eq!(codes(by_title(0)), vec!["wrong_film"]);
    assert_eq!(codes(by_title(1)), vec!["quality"]);
    assert_eq!(codes(by_title(2)), vec!["too_small"]);
    assert_eq!(codes(by_title(3)), vec!["seeders"]);
    assert!(!by_title(3).rejections[0].permanent, "seeders can change");
    assert_eq!(codes(by_title(4)), vec!["disc"]);
    assert!(codes(by_title(5)).contains(&"hardcoded_subs"));
}

#[test]
fn indexer_ids_override_titles() {
    let mut c = cand("Some.Other.Name.1999.1080p.BluRay.x264-GRP", 12.0, 30);
    c.release.tmdb_id = Some(603);
    assert!(evaluate(&matrix(), &profile(), &[c.clone()])[0].accepted());
    c.release.tmdb_id = Some(604);
    assert_eq!(codes(&evaluate(&matrix(), &profile(), &[c])[0]), vec!["wrong_film"]);
}

#[test]
fn year_off_by_one_is_fine_more_is_not() {
    let ok = cand("The.Matrix.2000.1080p.BluRay.x264-GRP", 12.0, 30);
    let bad = cand("The.Matrix.2021.1080p.BluRay.x264-GRP", 12.0, 30);
    let out = evaluate(&matrix(), &profile(), &[ok, bad]);
    assert!(out[0].accepted());
    assert_eq!(codes(&out[1]), vec!["wrong_film"]);
}

#[test]
fn ties_break_on_revision_terms_and_size() {
    let plain = cand("The.Matrix.1999.1080p.BluRay.x264-GRP", 12.0, 30);
    let proper = cand("The.Matrix.1999.PROPER.1080p.BluRay.x264-GRP", 12.0, 30);
    let out = evaluate(&matrix(), &profile(), &[plain.clone(), proper]);
    assert_eq!(out[0].candidate_index, 1);
    assert_eq!(out[0].beats_next_by, Some("revision"));

    let atmos = cand("The.Matrix.1999.1080p.BluRay.Atmos.x264-GRP", 12.0, 30);
    let out = evaluate(&matrix(), &profile(), &[plain.clone(), atmos]);
    assert_eq!(out[0].beats_next_by, Some("preferred words"));

    // preferred size for 136 min at 95 MB/min is about 12.6 GB
    let near = cand("The.Matrix.1999.1080p.BluRay.x264-AAA", 12.5, 30);
    let far = cand("The.Matrix.1999.1080p.BluRay.x264-BBB", 25.0, 30);
    let out = evaluate(&matrix(), &profile(), &[far, near]);
    assert_eq!(out[0].title, "The.Matrix.1999.1080p.BluRay.x264-AAA");
    assert_eq!(out[0].beats_next_by, Some("size"));
}

#[test]
fn upgrades_respect_cutoff() {
    let mut t = matrix();
    t.existing = Some(q(Source::Bluray, Resolution::R720));
    let web = cand("The.Matrix.1999.1080p.WEB-DL.H264-GRP", 6.0, 30);
    let bd720 = cand("The.Matrix.1999.720p.BluRay.x264-GRP", 5.0, 30);
    let out = evaluate(&t, &profile(), &[web.clone(), bd720]);
    assert!(out[0].accepted(), "1080p WEB-DL beats existing 720p");
    assert_eq!(codes(&out[1]), vec!["not_upgrade"]);

    t.existing = Some(q(Source::Bluray, Resolution::R1080));
    assert_eq!(codes(&evaluate(&t, &profile(), &[web])[0]), vec!["not_upgrade"], "already at cutoff");
}

fn thrones(episodes: &[u32]) -> Target {
    Target {
        title: "Thrones".into(),
        runtime_minutes: Some(55),
        episodes: Some(EpisodeTarget { season: 1, episodes: episodes.to_vec(), season_episodes: 10 }),
        ..Default::default()
    }
}

#[test]
fn episodes_match_season_and_number() {
    let cands = [
        cand("Thrones.S01E02.1080p.WEB-DL.x264-GRP", 2.0, 50),
        cand("Thrones.S01E01E02.1080p.WEB-DL.x264-GRP", 4.0, 50),
        cand("Thrones.S01E05.1080p.WEB-DL.x264-GRP", 2.0, 50),
        cand("Thrones.S02E02.1080p.WEB-DL.x264-GRP", 2.0, 50),
        cand("Thrones.S01.1080p.WEB-DL.x264-GRP", 20.0, 50),
        cand("Other.Show.S01E02.1080p.WEB-DL.x264-GRP", 2.0, 50),
        cand("Thrones.2024.01.15.1080p.WEB-DL.x264-GRP", 2.0, 50),
    ];
    let out = evaluate(&thrones(&[1, 2]), &profile(), &cands);
    let by = |t: &str| out.iter().find(|e| e.title.starts_with(t)).unwrap();
    assert!(by("Thrones.S01E02").accepted());
    assert_eq!(by("Thrones.S01E01E02").covers, [1, 2]);
    assert!(by("Thrones.S01E01E02").accepted());
    assert_eq!(codes(by("Thrones.S01E05")), ["wrong_episode"]);
    assert_eq!(codes(by("Thrones.S02E02")), ["wrong_season"]);
    assert_eq!(codes(by("Thrones.S01.1080p")), ["season_pack"]);
    assert_eq!(codes(by("Other.Show")), ["wrong_film"]);
    assert_eq!(codes(by("Thrones.2024")), ["numbering"]);
}

#[test]
fn season_packs_are_sized_for_the_season() {
    let cands = [
        cand("Thrones.S01.1080p.WEB-DL.x264-GRP", 20.0, 50),
        // 10 x 55 min at most 120 MB/min is ~64 GB; one episode's worth is too small for a season
        cand("Thrones.S01.720p.BluRay.x264-GRP", 0.2, 50),
        cand("Thrones.S01E01.1080p.WEB-DL.x264-GRP", 2.0, 50),
    ];
    let out = evaluate(&thrones(&[]), &profile(), &cands);
    assert!(out[0].accepted() && out[0].title.starts_with("Thrones.S01.1080p"));
    assert_eq!(out[0].covers.len(), 10);
    let by = |t: &str| out.iter().find(|e| e.title.starts_with(t)).unwrap();
    assert_eq!(codes(by("Thrones.S01E01")), ["not_season_pack"]);
}
