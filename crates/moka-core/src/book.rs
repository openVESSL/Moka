use serde::{Deserialize, Serialize};

use crate::string_enum;

/// File format of an ebook release, worst to best for reading on any
/// device: PDF keeps a fixed page, Kindle formats reflow, EPUB reflows and
/// opens everywhere (and in the browser reader).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EbookFormat {
    #[default]
    Unknown,
    Pdf,
    Mobi,
    Azw3,
    Epub,
}

string_enum!(EbookFormat, "ebook format" {
    Unknown => "unknown",
    Pdf => "pdf",
    Mobi => "mobi",
    Azw3 => "azw3",
    Epub => "epub",
});

impl EbookFormat {
    pub fn label(self) -> &'static str {
        match self {
            EbookFormat::Unknown => "Unknown",
            EbookFormat::Pdf => "PDF",
            EbookFormat::Mobi => "MOBI",
            EbookFormat::Azw3 => "AZW3",
            EbookFormat::Epub => "EPUB",
        }
    }

    /// From a file extension, any case.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "epub" | "kepub" => EbookFormat::Epub,
            "azw3" | "kf8" | "azw" => EbookFormat::Azw3,
            "mobi" | "prc" => EbookFormat::Mobi,
            "pdf" => EbookFormat::Pdf,
            _ => EbookFormat::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_and_names() {
        assert!(EbookFormat::Epub > EbookFormat::Azw3 && EbookFormat::Mobi > EbookFormat::Pdf);
        assert_eq!("azw3".parse::<EbookFormat>().unwrap(), EbookFormat::Azw3);
        assert_eq!(EbookFormat::from_extension("EPUB"), EbookFormat::Epub);
        assert_eq!(EbookFormat::from_extension("txt"), EbookFormat::Unknown);
    }
}
