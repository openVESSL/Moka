//! Book release names. They follow no common order ("Author - Title",
//! "Title - Author", "Title by Author", scene `Author.-.Title.2012.RETAIL.ePub.eBook-GRP`),
//! so a name is not split into author and title: it is checked for a given
//! author and title instead, and read for format, year and edition marks.

use std::sync::LazyLock;

use moka_core::EbookFormat;
use regex::Regex;
use serde::Serialize;

use crate::normalize_title;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedBook {
    pub year: Option<u16>,
    /// The best format the name announces; packs often hold several.
    pub format: EbookFormat,
    /// Made from the publisher's file, not scanned or converted.
    pub retail: bool,
    pub group: Option<String>,
}

static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(1[5-9][0-9]{2}|20[0-9]{2})\b").unwrap());
static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bebook-([A-Za-z0-9]{2,15})$").unwrap());

fn words(name: &str) -> Vec<String> {
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn parse_book(name: &str) -> ParsedBook {
    let w = words(name);
    let format = w.iter().map(|x| EbookFormat::from_extension(x)).max().unwrap_or_default();
    ParsedBook {
        year: YEAR.captures_iter(name).filter_map(|c| c[1].parse().ok()).next(),
        format,
        retail: w.iter().any(|x| x == "retail"),
        group: GROUP.captures(name.trim()).map(|c| c[1].to_owned()),
    }
}

/// Whether a release name is about this book: it holds the whole title
/// (the part before a subtitle or ", or" is enough) and the author's last
/// name.
pub fn book_matches(name: &str, author: &str, title: &str) -> bool {
    let hay = format!(" {} ", normalize_title(name));
    let has = |needle: &str| !needle.is_empty() && hay.contains(&format!(" {needle} "));
    let surname = author_surname(author);
    let main = title.split([':', ';']).next().unwrap_or(title);
    let main = main.split(", or ").next().unwrap_or(main);
    let titled = has(&normalize_title(title)) || has(&normalize_title(main));
    titled && (surname.is_empty() || has(&surname))
}

/// Last word of the author's name, "Tolkien" from "J.R.R. Tolkien" or
/// "Tolkien, J.R.R.".
fn author_surname(author: &str) -> String {
    let name = match author.split_once(',') {
        Some((last, _)) => last,
        None => author,
    };
    normalize_title(name).split_whitespace().last().unwrap_or("").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_names() {
        let p = parse_book("J.R.R.Tolkien.-.The.Hobbit.2012.RETAIL.ePub.eBook-BitBook");
        assert_eq!(
            p,
            ParsedBook {
                year: Some(2012),
                format: EbookFormat::Epub,
                retail: true,
                group: Some("BitBook".into())
            }
        );
        let p = parse_book("Tolkien, J.R.R. - The Hobbit (1937) [mobi, azw3, pdf]");
        assert_eq!((p.year, p.format, p.retail, p.group), (Some(1937), EbookFormat::Azw3, false, None));
        assert_eq!(parse_book("The Hobbit by J R R Tolkien").format, EbookFormat::Unknown);
    }

    #[test]
    fn names_about_a_book() {
        let yes = |n: &str| book_matches(n, "J.R.R. Tolkien", "The Hobbit, or There and Back Again");
        assert!(yes("J.R.R.Tolkien.-.The.Hobbit.2012.RETAIL.ePub.eBook-BitBook"));
        assert!(yes("The Hobbit - Tolkien (epub)"));
        let hobbit = |n: &str| book_matches(n, "Tolkien, J.R.R.", "The Hobbit");
        assert!(hobbit("Hobbit by J R R Tolkien [EPUB]"), "leading article and comma form");
        assert!(!hobbit("The Hobbit - Some Other Writer (epub)"), "wrong author");
        assert!(!hobbit("Tolkien - The Hobbitses (epub)"), "whole words only");
        assert!(book_matches("Dune - Frank Herbert", "Frank Herbert", "Dune: Deluxe Edition"));
    }
}
