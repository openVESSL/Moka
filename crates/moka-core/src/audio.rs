use serde::{Deserialize, Serialize};

use crate::string_enum;

/// Quality of a music release, worst to best. Lossy formats are ranked by
/// bitrate; lossless above them, high-resolution lossless at the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioQuality {
    #[default]
    Unknown,
    Mp3Low,
    /// MP3 with no bitrate given.
    Mp3,
    Aac,
    Opus,
    Mp3_256,
    Mp3V0,
    Mp3_320,
    Alac,
    Flac,
    /// 24-bit lossless (hi-res).
    Flac24,
}

string_enum!(AudioQuality, "audio quality" {
    Unknown => "unknown",
    Mp3Low => "mp3-low",
    Mp3 => "mp3",
    Aac => "aac",
    Opus => "opus",
    Mp3_256 => "mp3-256",
    Mp3V0 => "mp3-v0",
    Mp3_320 => "mp3-320",
    Alac => "alac",
    Flac => "flac",
    Flac24 => "flac-24",
});

impl AudioQuality {
    pub fn is_lossless(self) -> bool {
        matches!(self, AudioQuality::Alac | AudioQuality::Flac | AudioQuality::Flac24)
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioQuality::Unknown => "Unknown",
            AudioQuality::Mp3Low => "MP3 (low)",
            AudioQuality::Mp3 => "MP3",
            AudioQuality::Aac => "AAC",
            AudioQuality::Opus => "Opus",
            AudioQuality::Mp3_256 => "MP3 256",
            AudioQuality::Mp3V0 => "MP3 V0",
            AudioQuality::Mp3_320 => "MP3 320",
            AudioQuality::Alac => "ALAC",
            AudioQuality::Flac => "FLAC",
            AudioQuality::Flac24 => "FLAC 24-bit",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_and_names() {
        assert!(AudioQuality::Flac24 > AudioQuality::Flac && AudioQuality::Flac > AudioQuality::Mp3_320);
        assert!(AudioQuality::Mp3V0 > AudioQuality::Mp3_256 && AudioQuality::Mp3 > AudioQuality::Mp3Low);
        assert_eq!("mp3-v0".parse::<AudioQuality>().unwrap(), AudioQuality::Mp3V0);
        assert_eq!(AudioQuality::Flac24.to_string(), "flac-24");
        assert!(AudioQuality::Alac.is_lossless() && !AudioQuality::Mp3_320.is_lossless());
    }
}
