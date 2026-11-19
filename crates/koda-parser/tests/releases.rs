//! Real-world release names and what we expect to read from them.

use koda_parser::{normalize_title, parse_movie, parse_quality};

/// (release name, title, year, quality key, group)
type Case = (&'static str, &'static str, Option<u16>, &'static str, Option<&'static str>);

const CASES: &[Case] = &[
    // plain scene names
    ("Movie.2018.1080p.AMZN.WEB-DL.DD5.1.H.264-NTG", "Movie", Some(2018), "webdl-1080p", Some("NTG")),
    (
        "The.Movie.from.U.N.C.L.E.2015.1080p.BluRay.x264-SPARKS",
        "The Movie from U.N.C.L.E.",
        Some(2015),
        "bluray-1080p",
        Some("SPARKS"),
    ),
    ("R.I.P.D.2013.720p.BluRay.x264-SPARKS", "R.I.P.D.", Some(2013), "bluray-720p", Some("SPARKS")),
    ("V.H.S.2.2013.LIMITED.720p.BluRay.x264-GECKOS", "V.H.S. 2", Some(2013), "bluray-720p", Some("GECKOS")),
    (
        "To.Live.and.Movie.in.L.A.1985.1080p.BluRay",
        "To Live and Movie in L.A.",
        Some(1985),
        "bluray-1080p",
        None,
    ),
    ("A.I.Artificial.Movie.(2001)", "A.I. Artificial Movie", Some(2001), "unknown-unknown", None),
    ("A.Movie.Name.(1998)", "A Movie Name", Some(1998), "unknown-unknown", None),
    (
        "G.I.Movie.Movie.2013.THEATRiCAL.COMPLETE.BLURAY-GLiMMER",
        "G.I. Movie Movie",
        Some(2013),
        "bluray-1080p-disc",
        Some("GLiMMER"),
    ),
    (
        "Das.A.Team.Der.Film.Extended.Cut.German.720p.BluRay.x264-ANCIENT",
        "Das A Team Der Film",
        None,
        "bluray-720p",
        Some("ANCIENT"),
    ),
    (
        "Die.fantastische.Reise.des.Dr.Dolittle.2020.German.DL.LD.1080p.WEBRip.x264-PRD",
        "Die fantastische Reise des Dr. Dolittle",
        Some(2020),
        "webrip-1080p",
        Some("PRD"),
    ),
    // years inside titles
    ("1776.1979.EXTENDED.720p.BluRay.X264-AMIABLE", "1776", Some(1979), "bluray-720p", Some("AMIABLE")),
    (
        "Blade.Runner.2049.2017.2160p.UHD.BluRay.REMUX.HDR.HEVC.Atmos-EPSiLON",
        "Blade Runner 2049",
        Some(2017),
        "bluray-2160p-remux",
        Some("EPSiLON"),
    ),
    (
        "2001.A.Space.Odyssey.1968.1080p.BluRay.x264-AMIABLE",
        "2001 A Space Odyssey",
        Some(1968),
        "bluray-1080p",
        Some("AMIABLE"),
    ),
    ("1917.2019.1080p.WEBRip.x264-RARBG", "1917", Some(2019), "webrip-1080p", Some("RARBG")),
    ("8.2019.GERMAN.720p.BluRay.x264-UNiVERSUM", "8", Some(2019), "bluray-720p", Some("UNiVERSUM")),
    (
        "Movie.Klasse.von.1999.1990.German.720p.HDTV.x264-NORETAiL",
        "Movie Klasse von 1999",
        Some(1990),
        "tv-720p",
        Some("NORETAiL"),
    ),
    (
        "[Inka-Subs] Rokudenashi Blues 1993 (1993) [VHS]",
        "Rokudenashi Blues 1993",
        Some(1993),
        "unknown-unknown",
        Some("Inka-Subs"),
    ),
    (
        "Der.Movie.James.German.Bluray.FuckYou.Pso.Why.cant.you.follow.scene.rules.1998",
        "Der Movie James",
        Some(1998),
        "bluray-720p",
        None,
    ),
    // brackets, spaces and punctuation
    (
        "MY MOVIE (2016) [R][Action, Horror][720p.WEB-DL.AVC.8Bit.6ch.AC3].mkv",
        "MY MOVIE",
        Some(2016),
        "webdl-720p",
        None,
    ),
    (
        "This Is A Movie (1999) [IMDB #] <Genre, Genre, Genre> {ACTORS} !DIRECTOR +MORE_SILLY_STUFF_NO_ONE_NEEDS ?",
        "This Is A Movie",
        Some(1999),
        "unknown-unknown",
        None,
    ),
    ("We Are the Movie!.2013.720p.H264.mkv", "We Are the Movie!", Some(2013), "tv-720p", None),
    (
        "(500).Days.Of.Movie.(2009).DTS.1080p.BluRay.x264.NLsubs",
        "(500) Days Of Movie",
        Some(2009),
        "bluray-1080p",
        None,
    ),
    ("Movie: The Movie World 2013", "Movie: The Movie World", Some(2013), "unknown-unknown", None),
    ("Movie.The.Final.Chapter.2016", "Movie The Final Chapter", Some(2016), "unknown-unknown", None),
    ("Leaving Movie by Movie (1897) [DVD].mp4", "Leaving Movie by Movie", Some(1897), "dvd-480p", None),
    ("World Movie Z Movie [2023]", "World Movie Z Movie", Some(2023), "unknown-unknown", None),
    (
        "The.French.Movie.2013.720p.BluRay.x264 - ROUGH[PublicHD]",
        "The French Movie",
        Some(2013),
        "bluray-720p",
        Some("ROUGH"),
    ),
    (
        "The.Good.German.2006.720p.BluRay.x264-RlsGrp",
        "The Good German",
        Some(2006),
        "bluray-720p",
        Some("RlsGrp"),
    ),
    (
        "Kick.Ass.2.2013.German.DTS.DL.720p.BluRay.x264-Pate_",
        "Kick Ass 2",
        Some(2013),
        "bluray-720p",
        Some("Pate"),
    ),
    ("Movie.Movie.2000.FRENCH..BluRay.-AiRLiNE", "Movie Movie", Some(2000), "bluray-720p", Some("AiRLiNE")),
    ("Valana la Movie TRUEFRENCH BluRay 720p 2016 kjhlj", "Valana la Movie", Some(2016), "bluray-720p", None),
    ("Movie.German.DL.AC3.Dubbed..BluRay.x264-PsO", "Movie", None, "bluray-720p", Some("PsO")),
    ("Cars.2.German.DL.720p.BluRay.x264-EmpireHD", "Cars 2", None, "bluray-720p", Some("EmpireHD")),
    // editions
    (
        "Movie.Title.Imax.2018.1080p.AMZN.WEB-DL.DD5.1.H.264-NTG",
        "Movie Title",
        Some(2018),
        "webdl-1080p",
        Some("NTG"),
    ),
    (
        "World.Movie.Z.EXTENDED.2013.German.DL.1080p.BluRay.AVC-XANOR",
        "World Movie Z",
        Some(2013),
        "bluray-1080p-disc",
        Some("XANOR"),
    ),
    (
        "Movie.War.Q.EXTENDED.CUT.2013.German.DL.1080p.BluRay.x264-HQX",
        "Movie War Q",
        Some(2013),
        "bluray-1080p",
        Some("HQX"),
    ),
    (
        "Movie.Squad.2016.EXTENDED.German.DL.AC3.BDRip.x264-hqc",
        "Movie Squad",
        Some(2016),
        "bluray-720p",
        Some("hqc"),
    ),
    // websites and anime groups
    (
        "www.Torrenting.com - Movie.2008.720p.X264-DIMENSION",
        "Movie",
        Some(2008),
        "tv-720p",
        Some("DIMENSION"),
    ),
    (
        "www.5MovieRulz.tc - Movie (2000) Malayalam HQ HDRip - x264 - AAC - 700MB.mkv",
        "Movie",
        Some(2000),
        "unknown-unknown",
        None,
    ),
    (
        "[MTBB] Kimi no Na wa. (2016) v2 [97681524].mkv",
        "Kimi no Na wa",
        Some(2016),
        "unknown-unknown",
        Some("MTBB"),
    ),
    (
        "[sam] Toward the Terra (1980) [BD 1080p TrueHD].mkv",
        "Toward the Terra",
        Some(1980),
        "bluray-1080p",
        Some("sam"),
    ),
    (
        "[Arid] Cowboy Bebop - Knockin' on Heaven's Door v2 [00F4CDA0].mkv",
        "Cowboy Bebop - Knockin' on Heaven's Door",
        None,
        "unknown-unknown",
        Some("Arid"),
    ),
    (
        "[Arid] 5 Centimeters per Second (BDRip 1920x1080 Hi10 FLAC) [FD8B6FF2].mkv",
        "5 Centimeters per Second",
        None,
        "bluray-1080p",
        Some("Arid"),
    ),
    // quality edge cases
    (
        "Movie.Title.2016.REMUX.1080p.BluRay.AVC.DTS-HD.MA.5.1-iFT",
        "Movie Title",
        Some(2016),
        "bluray-1080p-remux",
        Some("iFT"),
    ),
    (
        "Movie.1993.720p.BluRay.REMUX.AVC.FLAC.2.0-BLURANiUM",
        "Movie",
        Some(1993),
        "bluray-720p",
        Some("BLURANiUM"),
    ),
    (
        "Movie.Name.2008.BDREMUX.1080p.Bluray.AVC.DTS-HR.MA.5.1-LEGi0N",
        "Movie Name",
        Some(2008),
        "bluray-1080p-remux",
        Some("LEGi0N"),
    ),
    (
        "Movie.Title.1980.2160p.UHD.BluRay.Remux.HDR.HEVC.DTS-HD.MA.5.1-PmP.mkv",
        "Movie Title",
        Some(1980),
        "bluray-2160p-remux",
        Some("PmP"),
    ),
    ("Movie.Title.2013.BDISO", "Movie Title", Some(2013), "bluray-1080p-disc", None),
    (
        "Movie.Title.2012.Bluray.1080p.3D.AVC.DTS-HD.MA.5.1.iso",
        "Movie Title",
        Some(2012),
        "bluray-1080p-disc",
        None,
    ),
    ("Movie.Name.2004.576p.BDRip.x264-HANDJOB", "Movie Name", Some(2004), "bluray-576p", Some("HANDJOB")),
    ("Movie.Name.2011.LIMITED.BluRay.360p.H264-20-40", "Movie Name", Some(2011), "bluray-480p", Some("40")),
    (
        "Some.Movie.Magic.Rainbow.2007.DVDR.NTSC",
        "Some Movie Magic Rainbow",
        Some(2007),
        "dvd-480p-remux",
        None,
    ),
    ("Life.Movie.2014.German.DL.PAL.DVDR-ETM", "Life Movie", Some(2014), "dvd-480p-remux", Some("ETM")),
    (
        "Movie.Name.2016.03.14.2160p.WEB.PROPER.h264-spamTV",
        "Movie Name",
        Some(2016),
        "webdl-2160p",
        Some("spamTV"),
    ),
    (
        "Movie Name 2020 WEB-DL 4K H265 10bit HDR DDP5.1 Atmos-PTerWEB",
        "Movie Name",
        Some(2020),
        "webdl-2160p",
        Some("PTerWEB"),
    ),
    ("Movie.Title.2019.1080p.AMZN.WEB-Rip.DDP.5.1.HEVC", "Movie Title", Some(2019), "webrip-1080p", None),
    (
        "Movie.Title.3.2017.720p.TSRip.x264.AAC-Ozlem",
        "Movie Title 3",
        Some(2017),
        "telesync-unknown",
        Some("Ozlem"),
    ),
    (
        "Movie Name 2018 NEW PROPER 720p HD-CAM X264 HQ-CPG",
        "Movie Name",
        Some(2018),
        "cam-unknown",
        Some("CPG"),
    ),
    (
        "Movie Name (2022) 1080p HQCAM ENG x264 AAC - QRips",
        "Movie Name",
        Some(2022),
        "cam-unknown",
        Some("QRips"),
    ),
    ("Movie.Name.2010.DVDSCR.XviD-GROUP", "Movie Name", Some(2010), "dvd-480p-screener", Some("GROUP")),
    ("Movie.Name.2009.R5.LiNE.XviD-GROUP", "Movie Name", Some(2009), "dvd-480p-regional", Some("GROUP")),
    ("Movie.Name.1999.DVDRip.XviD-GROUP", "Movie Name", Some(1999), "dvd-480p", Some("GROUP")),
    ("Movie.Name.2015.1080p.HDTV.x264-GROUP", "Movie Name", Some(2015), "tv-1080p", Some("GROUP")),
    ("Movie.Name.2015.BluRay.XviD-GROUP", "Movie Name", Some(2015), "bluray-480p", Some("GROUP")),
    ("Movie.Name.2016.Bluray1080p.mkv", "Movie Name", Some(2016), "bluray-1080p", None),
];

#[test]
fn parses_known_releases() {
    let mut failures = Vec::new();
    for &(name, title, year, quality, group) in CASES {
        let Some(p) = parse_movie(name) else {
            failures.push(format!("{name}\n    did not parse"));
            continue;
        };
        let got = (p.title.as_str(), p.year, p.quality.key(), p.group.as_deref());
        let want = (title, year, quality.to_owned(), group);
        if got != want {
            failures.push(format!("{name}\n    got  {got:?}\n    want {want:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n")
    );
}

#[test]
fn editions() {
    let cases = [
        ("Movie.War.Q.EXTENDED.CUT.2013.German.DL.1080p.BluRay.x264-HQX", Some("Extended Cut")),
        ("Movie.Title.Imax.2018.1080p.AMZN.WEB-DL.DD5.1.H.264-NTG", Some("IMAX")),
        ("Movie.Directors.Cut.2010.1080p.BluRay.x264-GRP", Some("Director's Cut")),
        ("Movie.2010.25th.Anniversary.Edition.1080p.BluRay.x264-GRP", Some("25th Anniversary Edition")),
        ("Movie.The.Final.Chapter.2016.1080p.WEB-DL-GRP", None),
        ("Sin.Movie.2005.RECUT.EXTENDED.German.DL.1080p.BluRay.x264-DETAiLS", Some("Recut Extended")),
    ];
    for (name, want) in cases {
        assert_eq!(parse_movie(name).unwrap().edition.as_deref(), want, "{name}");
    }
}

#[test]
fn revisions() {
    let r = |n: &str| parse_movie(n).unwrap().revision;
    assert_eq!(r("Movie.2010.1080p.BluRay.x264-GRP").version, 1);
    assert_eq!(r("Movie.2010.PROPER.1080p.BluRay.x264-GRP").version, 2);
    assert_eq!(r("Movie.2010.REPACK.1080p.BluRay.x264-GRP").version, 2);
    assert_eq!(r("Movie.2010.REPACK2.1080p.BluRay.x264-GRP").version, 3);
    assert_eq!(r("Movie.2010.REAL.PROPER.1080p.BluRay.x264-GRP").real, 1);
    assert_eq!(r("Movie.2010.real.1080p.BluRay.x264-GRP").real, 0, "lowercase real is a word");
}

#[test]
fn languages_and_subs() {
    let p = parse_movie("Movie.2016.MULTi.FRENCH.1080p.BluRay.x264-GRP").unwrap();
    assert_eq!(p.languages, vec!["multi", "fr"]);
    assert!(parse_movie("Movie Name (2016) BluRay 1080p DTS-ES AC3 x264-3Li").unwrap().languages.is_empty());
    assert!(parse_movie("The.Good.German.2006.720p.HDTV.x264-TVP").unwrap().languages.is_empty());

    let hc = |n: &str| parse_movie(n).unwrap().hardcoded_subs;
    assert!(hc("Movie.Title.2016.1080p.KORSUB.WEBRip.x264.AAC2.0-GRP"));
    assert!(hc("Movie Title 2017 HC 720p HDRiP DD5 1 x264-LEGi0N"));
    assert!(hc("Movie.Title.2017.720p.SUBBED.HDRip.V2.XViD-26k.avi"));
    assert!(!hc("Movie.Title.2000.1080p.BlueRay.x264.DTS.RoSubbed-playHD"));
    assert!(!hc("Movie Title! 2018 [Web][MKV][h264][480p][AAC 2.0][Softsubs]"));
    assert!(!hc("Movie Title! 2024 [Web][x265][1080p][EAC3][MultiSubs]"));
}

#[test]
fn ids_and_alternative_titles() {
    let p = parse_movie("That Italian Movie 2008 [tt1234567] 720p BluRay X264").unwrap();
    assert_eq!(p.imdb_id.as_deref(), Some("tt1234567"));
    assert_eq!(parse_movie("Movie Name (2016) {tmdbid-43074}").unwrap().tmdb_id, Some(43074));
    assert_eq!(parse_movie("Movie Name (2016) [tmdb-2020]").unwrap().tmdb_id, Some(2020));

    let p = parse_movie("Kjeller.chitai.AKA.Basement.of.Shame.1956.1080p.BluRay.x264.FLAC.1.0.mkv").unwrap();
    assert_eq!(p.title, "Kjeller chitai");
    assert_eq!(p.alt_titles, vec!["Basement of Shame"]);
    let p = parse_movie("Mon cousin a.k.a. My Cousin 2020 1080p Blu-ray DD 5.1 x264.mkv").unwrap();
    assert_eq!((p.title.as_str(), p.alt_titles[0].as_str()), ("Mon cousin", "My Cousin"));
    let p = parse_movie("Movie.Under.Water.(aka.Beneath.the.Code.Freeze).1997.DVDRip.x264.CG-Grzechsin.mkv")
        .unwrap();
    assert_eq!(p.alt_titles, vec!["Beneath the Code Freeze"]);
    assert_eq!(parse_movie("AKA.2002.DVDRip.x264-HANDJOB.mkv").unwrap().title, "AKA");
}

#[test]
fn rejects_junk() {
    assert!(parse_movie("").is_none());
    assert!(parse_movie("----").is_none());
    assert!(parse_movie("b2a1f7c9e8d34a6b9c0d1e2f3a4b5c6d").is_none(), "md5-like hash");
}

#[test]
fn quality_without_title() {
    assert_eq!(parse_quality("The.Movie.2010.1080p.BluRay.x264.mkv").0.key(), "bluray-1080p");
    assert_eq!(parse_quality("Cam.2018.1080p.WEB-DL.mkv").0.key(), "webdl-1080p");
}

#[test]
fn normalized_titles_compare_loosely() {
    assert_eq!(normalize_title("The Matrix"), normalize_title("Matrix"));
    assert_eq!(normalize_title("Amélie"), normalize_title("Amelie"));
    assert_eq!(normalize_title("Fast & Furious"), normalize_title("Fast and Furious"));
    assert_eq!(normalize_title("Mission: Impossible"), "mission impossible");
    assert_ne!(normalize_title("Alien"), normalize_title("Aliens"));
}
