//! Episode and season names and what we expect to read from them.

use koda_parser::parse_episode;

/// (name, show title, seasons, episodes, absolute, quality key, group)
type Case = (
    &'static str,
    &'static str,
    &'static [u32],
    &'static [u32],
    &'static [u32],
    &'static str,
    Option<&'static str>,
);

const CASES: &[Case] = &[
    (
        "Show.Name.S01E02.1080p.WEB-DL.DDP5.1.H.264-NTb",
        "Show Name",
        &[1],
        &[2],
        &[],
        "webdl-1080p",
        Some("NTb"),
    ),
    (
        "Show.Name.S01E01E02.1080p.BluRay.x264-ROVERS",
        "Show Name",
        &[1],
        &[1, 2],
        &[],
        "bluray-1080p",
        Some("ROVERS"),
    ),
    ("Show.Name.S01E01-E03.720p.WEB.h264-GRP", "Show Name", &[1], &[1, 2, 3], &[], "webdl-720p", Some("GRP")),
    ("Show.Name.S02E05-07.720p.HDTV.x264-GRP", "Show Name", &[2], &[5, 6, 7], &[], "tv-720p", Some("GRP")),
    ("Show.Name.S00E01.Special.720p.HDTV.x264", "Show Name", &[0], &[1], &[], "tv-720p", None),
    ("Show Name 1x05 HDTV XviD", "Show Name", &[1], &[5], &[], "tv-480p", None),
    ("Show Name - S01E05 - Episode Title.mkv", "Show Name", &[1], &[5], &[], "unknown-unknown", None),
    ("S01E05.mkv", "", &[1], &[5], &[], "unknown-unknown", None),
    ("01x05 - Pilot.mkv", "", &[1], &[5], &[], "unknown-unknown", None),
    // season packs
    (
        "Show.Name.S03.1080p.BluRay.x264-SHORTBREHD",
        "Show Name",
        &[3],
        &[],
        &[],
        "bluray-1080p",
        Some("SHORTBREHD"),
    ),
    ("Show Name Season 2 Complete 720p WEB-DL", "Show Name", &[2], &[], &[], "webdl-720p", None),
    (
        "Show.Name.S01-S03.1080p.WEB-DL.x264-GRP",
        "Show Name",
        &[1, 2, 3],
        &[],
        &[],
        "webdl-1080p",
        Some("GRP"),
    ),
    // anime
    (
        "[SubsPlease] Show Name - 05 (1080p) [ABCD1234].mkv",
        "Show Name",
        &[],
        &[],
        &[5],
        "tv-1080p",
        Some("SubsPlease"),
    ),
    ("[Group] Show Name S2 - 07 [720p].mkv", "Show Name", &[2], &[7], &[], "tv-720p", Some("Group")),
    (
        "[Group] Show Name - 01-12 [1080p]",
        "Show Name",
        &[],
        &[],
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        "tv-1080p",
        Some("Group"),
    ),
];

#[test]
fn episode_names() {
    for (name, title, seasons, episodes, absolute, quality, group) in CASES {
        let p = parse_episode(name).unwrap_or_else(|| panic!("{name}: not read"));
        assert_eq!(p.release.title, *title, "{name}: title");
        assert_eq!(p.seasons, *seasons, "{name}: seasons");
        assert_eq!(p.episodes, *episodes, "{name}: episodes");
        assert_eq!(p.absolute, *absolute, "{name}: absolute");
        assert_eq!(p.release.quality.key(), *quality, "{name}: quality");
        assert_eq!(p.release.group.as_deref(), *group, "{name}: group");
    }
}

#[test]
fn years_and_dates() {
    let p = parse_episode("Show.Name.2019.S02E10.720p.HDTV.x264-KILLERS").unwrap();
    assert_eq!((p.release.title.as_str(), p.release.year), ("Show Name", Some(2019)));
    let p = parse_episode("Show Name (2019) - S01E02 - Title [1080p]").unwrap();
    assert_eq!(
        (p.release.title.as_str(), p.release.year, p.episodes.as_slice()),
        ("Show Name", Some(2019), &[2][..])
    );

    let p = parse_episode("The.Daily.Show.2024.01.15.Guest.Name.720p.WEB.h264-GRP").unwrap();
    assert_eq!(p.release.title, "The Daily Show");
    assert_eq!(p.air_date.as_deref(), Some("2024-01-15"));
    assert!(p.seasons.is_empty() && !p.is_season_pack());
    let p = parse_episode("Late Show 2024-03-09 1080p WEB").unwrap();
    assert_eq!(p.air_date.as_deref(), Some("2024-03-09"));
}

#[test]
fn films_are_not_episodes() {
    for name in [
        "Movie.2018.1080p.AMZN.WEB-DL.DD5.1.H.264-NTG",
        "Movie.Name.1920x1080.BluRay",
        "Blade Runner 2049 2017 2160p UHD BluRay",
        "Movie.2013.13.Hours",
    ] {
        assert!(parse_episode(name).is_none(), "{name}");
    }
    assert!(parse_episode("Show.Name.S03.1080p.BluRay").unwrap().is_season_pack());
}
