//! What individual words in a release name mean.

use moka_core::{Resolution, Source};

/// Meaning of one token, lowercased and with hyphens removed.
#[derive(Debug, Clone, PartialEq)]
pub enum Word {
    Year(u16),
    Resolution(Resolution),
    Source(Source),
    /// Source and resolution glued together, e.g. `bluray1080p`, `bd720p`.
    SourceResolution(Source, Resolution),
    Remux,
    /// Untouched DVD (DVD-R): DVD source, remux modifier.
    DvdR,
    /// Full disc structure or image.
    Disc,
    Screener,
    Regional,
    /// Video codec. `encode` is true for encoder names (x264) as opposed to stream formats (AVC).
    Codec {
        encode: bool,
        lossy_old: bool,
        stream: bool,
    },
    Audio,
    Hdr,
    Language(&'static str),
    Proper,
    Repack(u8),
    Version(u8),
    HardcodedSubs,
    /// Scene and tracker tags with no meaning for us (internal, limited, amzn...).
    Tag,
    /// Edition word that stands on its own (extended, unrated, imax...).
    Edition,
    /// Edition word that only counts when followed by cut/edition/version.
    EditionPrefix,
    /// cut, edition, version
    EditionSuffix,
    Plain,
}

impl Word {
    /// Ends the title wherever it appears (after the first word).
    pub fn is_strong(&self) -> bool {
        matches!(
            self,
            Word::Resolution(_)
                | Word::SourceResolution(..)
                | Word::Version(_)
                | Word::Remux
                | Word::DvdR
                | Word::Disc
                | Word::Codec { .. }
                | Word::Audio
                | Word::Hdr
        ) || matches!(self, Word::Source(s) if !matches!(s, Source::Telesync | Source::Telecine | Source::Cam | Source::Workprint))
    }

    /// Technical, but common enough as a real word that it only ends the
    /// title when more technical words follow closely.
    pub fn is_weak(&self) -> bool {
        matches!(
            self,
            Word::Source(_)
                | Word::Screener
                | Word::Regional
                | Word::Language(_)
                | Word::Proper
                | Word::Repack(_)
                | Word::HardcodedSubs
                | Word::Tag
                | Word::Edition
        )
    }
}

pub fn classify(raw: &str) -> Word {
    let lower = raw.to_ascii_lowercase();
    let w = lower.replace('-', "");
    let w = w.as_str();

    if w.len() == 4
        && w.bytes().all(|b| b.is_ascii_digit())
        && let Ok(y) = w.parse::<u16>()
        && (1880..=2100).contains(&y)
    {
        return Word::Year(y);
    }
    if let Some(r) = resolution(w) {
        return Word::Resolution(r);
    }
    if let Some(sr) = glued_source_resolution(w) {
        return sr;
    }
    if let Some(s) = source(w) {
        return s;
    }
    if let Some(l) = language(w) {
        return Word::Language(l);
    }
    if let Some(v) = w.strip_prefix("repack").or_else(|| w.strip_prefix("rerip")) {
        return match v {
            "" => Word::Repack(2),
            n => n.parse::<u8>().map(|n| Word::Repack(n.saturating_add(1))).unwrap_or(Word::Plain),
        };
    }
    if w.len() == 2
        && w.starts_with('v')
        && let Some(n) = w[1..].parse::<u8>().ok().filter(|n| (2..=9).contains(n))
    {
        return Word::Version(n);
    }
    if is_hardcoded_subs(w) {
        return Word::HardcodedSubs;
    }
    // file sizes like 700MB or 1.4GB
    if let Some(n) = w.strip_suffix("mb").or_else(|| w.strip_suffix("gb"))
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
    {
        return Word::Tag;
    }
    match w {
        "remux" | "bdremux" | "uhdremux" => Word::Remux,
        "bd25" | "bd50" | "bd66" | "bd100" | "bdmv" | "bdiso" | "iso" => Word::Disc,
        "scr" | "screener" | "dvdscr" | "dvdscreener" | "bdscr" => Word::Screener,
        "r5" | "r6" | "regional" => Word::Regional,
        "x264" | "x265" | "h264" | "h265" | "hevc" | "av1" | "vp9" | "hi10p" | "10bit" | "8bit" => {
            Word::Codec {
                encode: matches!(w, "x264" | "x265" | "h264" | "h265"),
                lossy_old: false,
                stream: false,
            }
        }
        "avc" | "vc1" | "mpeg2" | "mvc" => Word::Codec { encode: false, lossy_old: false, stream: true },
        "xvid" | "divx" | "xvidhd" => Word::Codec { encode: true, lossy_old: true, stream: false },
        "aac" | "ac3" | "eac3" | "dts" | "dtshd" | "dtsx" | "dtses" | "truehd" | "atmos" | "flac" | "dd"
        | "ddp" | "dd5" | "ddp5" | "dd2" | "ddp2" | "dd51" | "ddp51" | "aac2" | "aac5" | "opus" | "lpcm"
        | "pcm" | "mp3" | "6ch" | "2ch" | "8ch" | "ac3d" => Word::Audio,
        "hdr" | "hdr10" | "hdr10plus" | "hdr10+" | "dv" | "dovi" | "dolbyvision" | "sdr" | "hlg" => Word::Hdr,
        "proper" => Word::Proper,
        "hc" | "subbed" => Word::HardcodedSubs,
        "extended" | "unrated" | "uncut" | "uncensored" | "remastered" | "theatrical" | "imax"
        | "criterion" | "recut" | "despecialized" => Word::Edition,
        "special" | "final" | "ultimate" | "collectors" | "collector's" | "directors" | "director's"
        | "director" | "diamond" | "signature" | "rogue" | "assembly" | "imperial" | "anniversary"
        | "open" | "fan" => Word::EditionPrefix,
        "cut" | "edition" | "version" | "matte" | "edit" => Word::EditionSuffix,
        "internal" | "limited" | "multi" | "dual" | "dubbed" | "complete" | "retail" | "festival"
        | "amzn" | "nf" | "dsnp" | "hmax" | "atvp" | "pmtp" | "ws" | "fs" | "nfofix" | "readnfo" | "dl"
        | "hq" | "hdrip" | "3d" | "hsbs" | "sbs" | "hou" | "custom" | "pal" | "ntsc" => Word::Tag,
        _ => Word::Plain,
    }
}

fn resolution(w: &str) -> Option<Resolution> {
    Some(match w {
        "360p" | "480p" | "480i" | "540p" | "640x480" | "848x480" | "sd" => Resolution::R480,
        "576p" | "576i" => Resolution::R576,
        "720p" | "720i" | "1280x720" | "960p" => Resolution::R720,
        "1080p" | "1080i" | "1920x1080" | "1440p" | "fhd" | "4kto1080p" => Resolution::R1080,
        "2160p" | "3840x2160" | "4096x2160" | "4k" | "uhd" => Resolution::R2160,
        _ => return None,
    })
}

fn source(w: &str) -> Option<Word> {
    let s = match w {
        "bluray" | "blueray" | "bdrip" | "brrip" | "bd" | "bdlight" | "uhdbluray" | "bluraydisc" => {
            Source::Bluray
        }
        "webdl" | "web" | "webhd" => Source::Webdl,
        "webrip" | "webmux" | "webcap" => Source::Webrip,
        "hdtv" | "pdtv" | "sdtv" | "tvrip" | "dsr" | "dsrip" | "hdtvrip" | "satrip" | "dvbrip" => Source::Tv,
        "dvd" | "dvdrip" | "dvd5" | "dvd9" | "xvidvd" | "2xdvd9" => Source::Dvd,
        "dvdr" => return Some(Word::DvdR),
        "cam" | "camrip" | "hdcam" | "hqcam" | "newcam" | "hdcamrip" => Source::Cam,
        "ts" | "telesync" | "telesynch" | "hdts" | "tsrip" | "hdtsrip" | "pdvd" => Source::Telesync,
        "tc" | "telecine" | "hdtc" => Source::Telecine,
        "workprint" | "wp" => Source::Workprint,
        _ => return None,
    };
    Some(Word::Source(s))
}

/// `bluray1080p`, `bd720p`, `web1080p`, `hdtv720p`.
fn glued_source_resolution(w: &str) -> Option<Word> {
    let split = w.find(|c: char| c.is_ascii_digit())?;
    let (s, r) = w.split_at(split);
    if s.is_empty() {
        return None;
    }
    let res = resolution(r)?;
    match source(s)? {
        Word::Source(src) => Some(Word::SourceResolution(src, res)),
        _ => None,
    }
}

fn is_hardcoded_subs(w: &str) -> bool {
    let stem = w.strip_suffix("subs").or_else(|| w.strip_suffix("sub"));
    // KORSUB, HCSUBS: a language or "hc" glued to "sub(s)". Soft, multi and
    // English ("e") subs are separate tracks, not burned in.
    match stem {
        Some("hc") => true,
        Some(prefix) => {
            (2..=4).contains(&prefix.len())
                && prefix.bytes().all(|b| b.is_ascii_alphabetic())
                && !matches!(prefix, "soft" | "nl" | "no" | "ext" | "eng" | "en")
        }
        None => false,
    }
}

/// Language words that appear in release names, mapped to ISO 639-1 codes.
fn language(w: &str) -> Option<&'static str> {
    Some(match w {
        "english" | "eng" => "en",
        "french" | "truefrench" | "vff" | "vfq" | "vf2" | "vostfr" => "fr",
        "german" | "ger" | "deutsch" => "de",
        "italian" | "ita" => "it",
        "spanish" | "spa" | "esp" | "castellano" | "latino" => "es",
        "portuguese" | "por" | "ptbr" => "pt",
        "russian" | "rus" => "ru",
        "polish" | "pol" | "plsub" => "pl",
        "dutch" | "flemish" => "nl",
        "swedish" | "swe" => "sv",
        "danish" | "dan" => "da",
        "norwegian" | "nor" => "no",
        "finnish" | "fin" => "fi",
        "korean" | "kor" => "ko",
        "japanese" | "jap" | "jpn" => "ja",
        "chinese" | "chs" | "cht" | "mandarin" | "cantonese" => "zh",
        "hindi" => "hi",
        "tamil" => "ta",
        "telugu" => "te",
        "malayalam" => "ml",
        "hungarian" | "hun" => "hu",
        "czech" | "cze" => "cs",
        "romanian" | "rum" => "ro",
        "turkish" | "tur" => "tr",
        "greek" => "el",
        "hebrew" | "heb" => "he",
        "arabic" => "ar",
        "thai" => "th",
        "vietnamese" => "vi",
        "ukrainian" | "ukr" => "uk",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(classify("2019"), Word::Year(2019));
        assert_eq!(classify("1080p"), Word::Resolution(Resolution::R1080));
        assert_eq!(classify("WEB-DL"), Word::Source(Source::Webdl));
        assert_eq!(classify("Blu-ray"), Word::Source(Source::Bluray));
        assert_eq!(classify("Bluray1080p"), Word::SourceResolution(Source::Bluray, Resolution::R1080));
        assert_eq!(classify("REPACK2"), Word::Repack(3));
        assert_eq!(classify("KORSUB"), Word::HardcodedSubs);
        assert_eq!(classify("Softsubs"), Word::Plain);
        assert_eq!(classify("MultiSubs"), Word::Plain);
        assert_eq!(classify("DTS-ES"), Word::Audio);
        assert_eq!(classify("German"), Word::Language("de"));
        assert_eq!(classify("1234"), Word::Plain, "outside the year range");
        assert_eq!(classify("v2"), Word::Version(2));
    }
}
