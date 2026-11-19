use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::string_enum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Unknown,
    Cam,
    Telesync,
    Telecine,
    Workprint,
    Dvd,
    Tv,
    Webrip,
    Webdl,
    Bluray,
}

string_enum!(Source, "source" {
    Unknown => "unknown",
    Cam => "cam",
    Telesync => "telesync",
    Telecine => "telecine",
    Workprint => "workprint",
    Dvd => "dvd",
    Tv => "tv",
    Webrip => "webrip",
    Webdl => "webdl",
    Bluray => "bluray",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Resolution {
    #[default]
    #[serde(rename = "unknown")]
    Unknown,
    #[serde(rename = "480p")]
    R480,
    #[serde(rename = "576p")]
    R576,
    #[serde(rename = "720p")]
    R720,
    #[serde(rename = "1080p")]
    R1080,
    #[serde(rename = "2160p")]
    R2160,
}

string_enum!(Resolution, "resolution" {
    Unknown => "unknown",
    R480 => "480p",
    R576 => "576p",
    R720 => "720p",
    R1080 => "1080p",
    R2160 => "2160p",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    #[default]
    None,
    Remux,
    /// Full disc structure or image. Not playable as a single file.
    Disc,
    Screener,
    Regional,
    Raw,
}

string_enum!(Modifier, "modifier" {
    None => "none",
    Remux => "remux",
    Disc => "disc",
    Screener => "screener",
    Regional => "regional",
    Raw => "raw",
});

/// What a release or file is, technically. Ordering between qualities is not
/// built in: it comes from the quality profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Quality {
    pub source: Source,
    pub resolution: Resolution,
    pub modifier: Modifier,
}

impl Quality {
    pub const fn new(source: Source, resolution: Resolution, modifier: Modifier) -> Self {
        Self { source, resolution, modifier }
    }

    pub fn is_unknown(&self) -> bool {
        self.source == Source::Unknown && self.resolution == Resolution::Unknown
    }

    /// Stable short key used in profiles and the database, e.g. `webdl-1080p`, `bluray-2160p-remux`.
    pub fn key(&self) -> String {
        let mut s = format!("{}-{}", self.source.as_str(), self.resolution.as_str());
        if self.modifier != Modifier::None {
            s.push('-');
            s.push_str(self.modifier.as_str());
        }
        s
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let mut parts = key.split('-');
        let source = parts.next()?.parse().ok()?;
        let resolution = parts.next()?.parse().ok()?;
        let modifier = match parts.next() {
            Some(m) => m.parse().ok()?,
            None => Modifier::None,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Self { source, resolution, modifier })
    }
}

impl fmt::Display for Quality {
    /// Human label: `WEB-DL 1080p`, `Blu-ray 2160p Remux`, `CAM`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match self.source {
            Source::Unknown => "Unknown",
            Source::Cam => "CAM",
            Source::Telesync => "Telesync",
            Source::Telecine => "Telecine",
            Source::Workprint => "Workprint",
            Source::Dvd => "DVD",
            Source::Tv => "TV",
            Source::Webrip => "WEBRip",
            Source::Webdl => "WEB-DL",
            Source::Bluray => "Blu-ray",
        };
        f.write_str(source)?;
        if self.resolution != Resolution::Unknown {
            write!(f, " {}", self.resolution.as_str())?;
        }
        match self.modifier {
            Modifier::None => Ok(()),
            Modifier::Remux => f.write_str(" Remux"),
            Modifier::Disc => f.write_str(" Disc"),
            Modifier::Screener => f.write_str(" Screener"),
            Modifier::Regional => f.write_str(" Regional"),
            Modifier::Raw => f.write_str(" Raw"),
        }
    }
}

/// Repacks and propers. Higher wins when the quality is the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Revision {
    /// 1 for the original release, 2+ for proper/repack/v2.
    pub version: u8,
    /// Count of "REAL" tags: a fixed release of a broken release.
    pub real: u8,
}

impl Default for Revision {
    fn default() -> Self {
        Self { version: 1, real: 0 }
    }
}

impl PartialOrd for Revision {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Revision {
    fn cmp(&self, other: &Self) -> Ordering {
        self.real.cmp(&other.real).then(self.version.cmp(&other.version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_round_trip() {
        let q = Quality::new(Source::Bluray, Resolution::R2160, Modifier::Remux);
        assert_eq!(q.key(), "bluray-2160p-remux");
        assert_eq!(Quality::from_key(&q.key()), Some(q));
        let q = Quality::new(Source::Webdl, Resolution::R1080, Modifier::None);
        assert_eq!(q.key(), "webdl-1080p");
        assert_eq!(Quality::from_key("webdl-1080p"), Some(q));
        assert_eq!(Quality::from_key("webdl"), None);
        assert_eq!(Quality::from_key("webdl-1080p-remux-x"), None);
    }

    #[test]
    fn labels() {
        assert_eq!(
            Quality::new(Source::Webdl, Resolution::R1080, Modifier::None).to_string(),
            "WEB-DL 1080p"
        );
        assert_eq!(
            Quality::new(Source::Bluray, Resolution::R2160, Modifier::Remux).to_string(),
            "Blu-ray 2160p Remux"
        );
        assert_eq!(Quality::new(Source::Cam, Resolution::Unknown, Modifier::None).to_string(), "CAM");
    }

    #[test]
    fn revision_order() {
        let original = Revision::default();
        let proper = Revision { version: 2, real: 0 };
        let real = Revision { version: 1, real: 1 };
        assert!(proper > original);
        assert!(real > proper);
    }
}
