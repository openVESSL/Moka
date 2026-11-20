//! Book release names. They follow no common order ("Author - Title",
//! "Title - Author", "Title by Author", scene `Author.-.Title.2012.RETAIL.ePub.eBook-GRP`),
//! so a name is not split into author and title: it is checked for a given
//! author and title instead, and read for format, year and edition marks.

use std::sync::LazyLock;

use moka_core::EbookFormat;
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedBook {
    pub year: Option<u16>,
    /// The best format the name announces; packs often hold several.
    pub format: EbookFormat,
    /// Every format the name announces, best first.
    pub formats: Vec<EbookFormat>,
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
    let mut formats: Vec<EbookFormat> =
        w.iter().map(|x| EbookFormat::from_extension(x)).filter(|f| *f != EbookFormat::Unknown).collect();
    formats.sort_by(|a, b| b.cmp(a));
    formats.dedup();
    let format = formats.first().copied().unwrap_or_default();
    ParsedBook {
        formats,
        year: YEAR.captures_iter(name).filter_map(|c| c[1].parse().ok()).next(),
        format,
        retail: w.iter().any(|x| x == "retail"),
        group: GROUP.captures(name.trim()).map(|c| c[1].to_owned()),
    }
}

/// Words left out when names are compared.
const SMALL: [&str; 5] = ["a", "an", "the", "and", "of"];

/// Words that may follow a title without making it another book.
const AFTER_TITLE: &[&str] = &[
    "by",
    "epub",
    "kepub",
    "mobi",
    "azw",
    "azw3",
    "kf8",
    "pdf",
    "retail",
    "ebook",
    "ebooks",
    "audiobook",
    "audiobooks",
    "unabridged",
    "abridged",
    "m4b",
    "mp3",
    "narrated",
    "read",
    "illustrated",
];

/// Lowercase, accent-free words of a name, each marked when a bracket,
/// dash or comma comes right before it.
fn tokens(s: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut boundary = false;
    let mut flush = |word: &mut String, boundary: &mut bool| {
        let w = std::mem::take(word);
        if !w.is_empty() && !SMALL.contains(&w.as_str()) {
            out.push((w, *boundary));
            *boundary = false;
        }
    };
    for c in s.chars() {
        let c = crate::fold_accent(c);
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else {
            flush(&mut word, &mut boundary);
            if matches!(c, '(' | '[' | '{' | '-' | '–' | '—' | ',' | ':' | ';' | '/') {
                boundary = true;
            }
        }
    }
    flush(&mut word, &mut boundary);
    out
}

/// Whether a release name is about this book: it holds the author's last
/// name and the title (the part before a subtitle or ", or" is enough),
/// not followed by more title words ("Dune" is not "Dune Messiah").
pub fn book_matches(name: &str, author: &str, title: &str) -> bool {
    let hay = tokens(name);
    let author_words: Vec<String> = tokens(author).into_iter().map(|(w, _)| w).collect();
    let surname = author_surname(author);
    if !surname.is_empty() && !hay.iter().any(|(w, _)| *w == surname) {
        return false;
    }
    let main = title.split([':', ';']).next().unwrap_or(title);
    let short = main.split(", or ").next().unwrap_or(main);
    [title, main, short].iter().any(|t| {
        let want: Vec<String> = tokens(t).into_iter().map(|(w, _)| w).collect();
        if want.is_empty() || want.len() > hay.len() {
            return false;
        }
        (0..=hay.len() - want.len()).any(|i| {
            let found = hay[i..i + want.len()].iter().zip(&want).all(|((w, _), x)| w == x);
            found
                && hay.get(i + want.len()).is_none_or(|(next, boundary)| {
                    *boundary
                        || (next.len() == 4 && next.bytes().all(|b| b.is_ascii_digit()))
                        || AFTER_TITLE.contains(&next.as_str())
                        || author_words.contains(next)
                })
        })
    })
}

/// Last word of the author's name, "tolkien" from "J.R.R. Tolkien" or
/// "Tolkien, J.R.R.".
fn author_surname(author: &str) -> String {
    let name = match author.split_once(',') {
        Some((last, _)) => last,
        None => author,
    };
    tokens(name).pop().map(|(w, _)| w).unwrap_or_default()
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
                formats: vec![EbookFormat::Epub],
                retail: true,
                group: Some("BitBook".into())
            }
        );
        let p = parse_book("Tolkien, J.R.R. - The Hobbit (1937) [mobi, azw3, pdf]");
        assert_eq!(
            (p.year, p.format, p.retail, p.group.clone()),
            (Some(1937), EbookFormat::Azw3, false, None)
        );
        assert_eq!(p.formats, [EbookFormat::Azw3, EbookFormat::Mobi, EbookFormat::Pdf]);
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
        let dune = |n: &str| book_matches(n, "Frank Herbert", "Dune");
        assert!(!dune("Frank Herbert - Dune Messiah (epub)"), "a longer title is another book");
        assert!(!dune("Frank.Herbert.-.Dune.Messiah.2005.RETAIL.ePub.eBook-GRP"));
        assert!(dune("Frank.Herbert.-.Dune.2005.RETAIL.ePub.eBook-GRP"));
        assert!(dune("Dune (Dune Chronicles 1) by Frank Herbert"));
        let rings = |n: &str| book_matches(n, "J. R. R. Tolkien", "The Lord of the Rings");
        assert!(rings("Tolkien - Lord of the Rings [epub]") && !rings("Tolkien - Lord of the Flies"));
    }
}
