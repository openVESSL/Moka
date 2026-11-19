//! Torznab / Newznab indexer client.
//!
//! Knows nothing about the database. Turns indexer responses into plain
//! [`Release`] values and leaves decisions to the caller.

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub mod torrent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Torrent,
    Usenet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexerKind {
    Torznab,
    Newznab,
}

impl IndexerKind {
    pub fn protocol(self) -> Protocol {
        match self {
            IndexerKind::Torznab => Protocol::Torrent,
            IndexerKind::Newznab => Protocol::Usenet,
        }
    }
}

/// Connection settings for one indexer.
#[derive(Debug, Clone)]
pub struct IndexerConfig {
    pub kind: IndexerKind,
    pub base_url: String,
    pub api_path: String,
    pub api_key: Option<String>,
    pub categories: Vec<u32>,
}

/// What the indexer says it can do (`t=caps`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Capabilities {
    pub search: bool,
    pub movie_search: bool,
    /// Parameters accepted by movie search, e.g. `q`, `imdbid`, `tmdbid`.
    pub movie_params: Vec<String>,
    pub limit_max: Option<u32>,
    pub categories: Vec<Category>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Category {
    pub id: u32,
    pub name: String,
}

/// One search result, normalized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub title: String,
    pub guid: String,
    pub protocol: Protocol,
    /// `.torrent`/`.nzb` link (may redirect to a magnet).
    pub download_url: Option<String>,
    pub magnet_url: Option<String>,
    /// Lowercase hex BitTorrent v1 info hash, when the indexer gives it.
    pub info_hash: Option<String>,
    pub size_bytes: Option<u64>,
    /// Unix seconds.
    pub published_at: Option<i64>,
    pub seeders: Option<u32>,
    pub peers: Option<u32>,
    pub grabs: Option<u32>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<i64>,
    pub categories: Vec<u32>,
    pub freeleech: bool,
    pub info_url: Option<String>,
}

/// What to look for.
#[derive(Debug, Clone, Default)]
pub struct MovieQuery {
    pub title: String,
    pub year: Option<u16>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the indexer rejected the API key")]
    Auth,
    #[error("the indexer's request limit is reached")]
    RateLimited { retry_after: Option<Duration> },
    #[error("indexer error {code}: {description}")]
    Api { code: u32, description: String },
    #[error("the indexer answered with something that is not a Torznab/Newznab feed")]
    NotAFeed,
    #[error("indexer returned HTTP {0}")]
    Status(u16),
    #[error("{0}")]
    Blocked(String),
    #[error("could not reach the indexer: {0}")]
    Http(#[from] reqwest::Error),
}

impl Error {
    /// Retrying will not help until the user changes something.
    pub fn is_config_problem(&self) -> bool {
        matches!(self, Error::Auth | Error::Status(401 | 403 | 404))
    }
}

pub struct Client {
    http: reqwest::Client,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        let http = koda_net::client_builder()
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("koda/", env!("CARGO_PKG_VERSION")))
            // redirects are followed by hand so a redirect to magnet: can be caught
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client builds");
        Self { http }
    }

    fn url(&self, cfg: &IndexerConfig, params: &[(&str, String)]) -> Result<reqwest::Url, Error> {
        let base = format!("{}{}", cfg.base_url.trim_end_matches('/'), normalize_path(&cfg.api_path));
        let mut url = reqwest::Url::parse(&base).map_err(|_| Error::Status(400))?;
        koda_net::check_url(&url).map_err(|e| Error::Blocked(e.to_string()))?;
        {
            let mut q = url.query_pairs_mut();
            for (k, v) in params {
                q.append_pair(k, v);
            }
            if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
                q.append_pair("apikey", key);
            }
        }
        Ok(url)
    }

    async fn get_xml(&self, url: reqwest::Url) -> Result<String, Error> {
        let res = self.http.get(url).send().await?;
        let status = res.status().as_u16();
        if status == 429 {
            let retry_after = res
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(Error::RateLimited { retry_after });
        }
        let body = res.text().await?;
        // indexers often answer errors with 200 + <error/>, and sometimes the reverse
        if let Some(err) = parse_error(&body) {
            return Err(err);
        }
        if !(200..300).contains(&status) {
            return Err(Error::Status(status));
        }
        Ok(body)
    }

    pub async fn capabilities(&self, cfg: &IndexerConfig) -> Result<Capabilities, Error> {
        let body = self.get_xml(self.url(cfg, &[("t", "caps".into())])?).await?;
        parse_caps(&body)
    }

    /// Searches for a film, using ids when the indexer supports them and
    /// falling back to "title year".
    pub async fn search_movie(
        &self,
        cfg: &IndexerConfig,
        caps: &Capabilities,
        q: &MovieQuery,
    ) -> Result<Vec<Release>, Error> {
        let has = |p: &str| caps.movie_params.iter().any(|x| x == p);
        let mut params: Vec<(&str, String)> = Vec::new();
        let mut by_id = false;
        if caps.movie_search {
            params.push(("t", "movie".into()));
            if let Some(id) = q.tmdb_id.filter(|_| has("tmdbid")) {
                params.push(("tmdbid", id.to_string()));
                by_id = true;
            }
            if let Some(id) = q.imdb_id.as_deref().filter(|_| has("imdbid")) {
                params.push(("imdbid", id.trim_start_matches("tt").to_owned()));
                by_id = true;
            }
        } else {
            params.push(("t", "search".into()));
        }
        if !by_id {
            let text = match q.year {
                Some(y) => format!("{} {y}", q.title),
                None => q.title.clone(),
            };
            params.push(("q", search_text(&text)));
        }
        if !cfg.categories.is_empty() {
            let cats: Vec<String> = cfg.categories.iter().map(u32::to_string).collect();
            params.push(("cat", cats.join(",")));
        }
        params.push(("limit", caps.limit_max.unwrap_or(100).min(100).to_string()));
        params.push(("extended", "1".into()));

        let body = self.get_xml(self.url(cfg, &params)?).await?;
        parse_feed(&body, cfg.kind.protocol())
    }

    /// Downloads what a release's link points at: a `.torrent` file, or the
    /// magnet link the indexer redirects to.
    pub async fn fetch(&self, url: &str) -> Result<torrent::Payload, Error> {
        let mut url = url.to_owned();
        for _ in 0..6 {
            if url.starts_with("magnet:") {
                return Ok(torrent::Payload::Magnet(url));
            }
            if let Ok(parsed) = reqwest::Url::parse(&url) {
                koda_net::check_url(&parsed).map_err(|e| Error::Blocked(e.to_string()))?;
            }
            let res = self.http.get(&url).send().await?;
            if res.status().is_redirection() {
                let next = res
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or(Error::Status(res.status().as_u16()))?;
                url = res.url().join(next).map(|u| u.to_string()).unwrap_or_else(|_| next.to_owned());
                continue;
            }
            if !res.status().is_success() {
                return Err(Error::Status(res.status().as_u16()));
            }
            let bytes = res.bytes().await?;
            return Ok(torrent::Payload::File(bytes.to_vec()));
        }
        Err(Error::Status(310))
    }
}

fn normalize_path(p: &str) -> String {
    let p = p.trim();
    if p.is_empty() {
        "/api".into()
    } else if p.starts_with('/') {
        p.to_owned()
    } else {
        format!("/{p}")
    }
}

/// Indexers match words, not punctuation: "Mission: Impossible" → "Mission Impossible".
fn search_text(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_error(body: &str) -> Option<Error> {
    let doc = roxmltree::Document::parse(body.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    if !root.has_tag_name("error") {
        return None;
    }
    let code: u32 = root.attribute("code").and_then(|c| c.parse().ok()).unwrap_or(0);
    let description = root.attribute("description").unwrap_or("").to_owned();
    Some(match code {
        100..=199 => Error::Auth,
        _ if description.to_ascii_lowercase().contains("limit") => Error::RateLimited { retry_after: None },
        _ => Error::Api { code, description },
    })
}

pub fn parse_caps(body: &str) -> Result<Capabilities, Error> {
    let doc = roxmltree::Document::parse(body.trim_start_matches('\u{feff}')).map_err(|_| Error::NotAFeed)?;
    let root = doc.root_element();
    if !root.has_tag_name("caps") {
        return Err(Error::NotAFeed);
    }
    let mut caps = Capabilities::default();
    for n in root.descendants().filter(|n| n.is_element()) {
        let available = n.attribute("available").map(|a| a.eq_ignore_ascii_case("yes")).unwrap_or(false);
        let params = || {
            n.attribute("supportedParams")
                .unwrap_or("q")
                .split(',')
                .map(|p| p.trim().to_ascii_lowercase())
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
        };
        match n.tag_name().name() {
            "limits" => caps.limit_max = n.attribute("max").and_then(|m| m.parse().ok()),
            "search" => caps.search = available,
            "movie-search" if available => {
                caps.movie_search = true;
                caps.movie_params = params();
            }
            "category" | "subcat" => {
                if let (Some(id), Some(name)) =
                    (n.attribute("id").and_then(|i| i.parse().ok()), n.attribute("name"))
                {
                    caps.categories.push(Category { id, name: name.to_owned() });
                }
            }
            _ => {}
        }
    }
    // some indexers list movie-search without params: plain text search still works
    if caps.movie_search && caps.movie_params.is_empty() {
        caps.movie_params.push("q".into());
    }
    Ok(caps)
}

pub fn parse_feed(body: &str, protocol: Protocol) -> Result<Vec<Release>, Error> {
    let doc = roxmltree::Document::parse(body.trim_start_matches('\u{feff}')).map_err(|_| Error::NotAFeed)?;
    if !doc.root_element().has_tag_name("rss") {
        return Err(Error::NotAFeed);
    }
    let mut out = Vec::new();
    for item in doc.descendants().filter(|n| n.has_tag_name("item")) {
        let child = |name: &str| {
            item.children()
                .find(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace().is_none())
                .and_then(|c| c.text())
                .map(str::trim)
                .filter(|t| !t.is_empty())
        };
        let mut attrs: Vec<(String, String)> = Vec::new();
        for a in item.children().filter(|c| c.is_element() && c.tag_name().name() == "attr") {
            if let (Some(k), Some(v)) = (a.attribute("name"), a.attribute("value")) {
                attrs.push((k.to_ascii_lowercase(), v.to_owned()));
            }
        }
        let attr = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let enclosure = item.children().find(|c| c.is_element() && c.has_tag_name("enclosure"));

        let Some(title) = child("title") else { continue };
        let link = child("link").map(str::to_owned);
        let enclosure_url = enclosure.and_then(|e| e.attribute("url")).map(str::to_owned);
        let magnet = attr("magneturl").map(str::to_owned).or_else(|| {
            [&link, &enclosure_url].into_iter().flatten().find(|u| u.starts_with("magnet:")).cloned()
        });
        let download = enclosure_url.or(link).filter(|u| !u.starts_with("magnet:"));
        if download.is_none() && magnet.is_none() {
            continue;
        }
        let info_hash = attr("infohash")
            .map(str::to_ascii_lowercase)
            .filter(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .or_else(|| magnet.as_deref().and_then(torrent::magnet_hash));
        let size = attr("size")
            .and_then(|s| s.parse().ok())
            .or_else(|| child("size").and_then(|s| s.parse().ok()))
            .or_else(|| enclosure.and_then(|e| e.attribute("length")).and_then(|l| l.parse().ok()))
            .filter(|s: &u64| *s > 0);
        let seeders = attr("seeders").and_then(|s| s.parse().ok());
        let peers = attr("peers").and_then(|s| s.parse().ok());
        let imdb = attr("imdbid").or_else(|| attr("imdb")).map(|i| {
            let digits = i.trim_start_matches("tt");
            format!("tt{digits:0>7}")
        });
        let categories =
            attrs.iter().filter(|(k, _)| k == "category").filter_map(|(_, v)| v.parse().ok()).collect();

        out.push(Release {
            title: title.to_owned(),
            guid: child("guid").unwrap_or(title).to_owned(),
            protocol,
            download_url: download,
            magnet_url: magnet,
            info_hash,
            size_bytes: size,
            published_at: child("pubDate").and_then(parse_rfc2822),
            seeders,
            peers,
            grabs: attr("grabs").and_then(|s| s.parse().ok()),
            imdb_id: imdb.filter(|i| i.len() >= 9 && i != "tt0000000"),
            tmdb_id: attr("tmdbid").or_else(|| attr("tmdb")).and_then(|s| s.parse().ok()).filter(|i| *i > 0),
            categories,
            freeleech: attr("downloadvolumefactor").and_then(|s| s.parse::<f32>().ok()) == Some(0.0),
            info_url: child("comments").map(|c| c.trim_end_matches("#comments").to_owned()),
        });
    }
    Ok(out)
}

/// "Tue, 05 Mar 2024 10:00:00 +0000" → unix seconds. Tolerates missing weekday.
fn parse_rfc2822(s: &str) -> Option<i64> {
    let s = s.split_once(',').map(|(_, r)| r).unwrap_or(s).trim();
    let mut it = s.split_whitespace();
    let day: i64 = it.next()?.parse().ok()?;
    let month = match it.next()?.get(..3)?.to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    let year: i64 = it.next()?.parse().ok()?;
    let mut hms = it.next().unwrap_or("00:00:00").split(':').map(|p| p.parse::<i64>().unwrap_or(0));
    let (h, m, sec) = (hms.next()?, hms.next().unwrap_or(0), hms.next().unwrap_or(0));
    let offset = it.next().map(parse_offset).unwrap_or(0);
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + sec - offset)
}

fn parse_offset(z: &str) -> i64 {
    let (sign, digits) = match z.as_bytes().first() {
        Some(b'+') => (1, &z[1..]),
        Some(b'-') => (-1, &z[1..]),
        _ => return 0,
    };
    if digits.len() != 4 {
        return 0;
    }
    let h: i64 = digits[..2].parse().unwrap_or(0);
    let m: i64 = digits[2..].parse().unwrap_or(0);
    sign * (h * 3600 + m * 60)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// One indexer configured in Prowlarr.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProwlarrIndexer {
    pub id: i64,
    pub name: String,
    pub protocol: Protocol,
    pub enabled: bool,
    /// Film categories (2000-2999) it offers.
    pub movie_categories: Vec<u32>,
}

impl Client {
    /// Lists the indexers of a Prowlarr instance (`GET /api/v1/indexer`).
    pub async fn prowlarr_indexers(
        &self,
        base_url: &str,
        api_key: &str,
    ) -> Result<Vec<ProwlarrIndexer>, Error> {
        #[derive(Deserialize)]
        struct Cat {
            id: u32,
            #[serde(default, rename = "subCategories")]
            sub: Vec<Cat>,
        }
        #[derive(Deserialize, Default)]
        struct Caps {
            #[serde(default)]
            categories: Vec<Cat>,
        }
        #[derive(Deserialize)]
        struct Raw {
            id: i64,
            name: String,
            #[serde(default)]
            enable: bool,
            protocol: String,
            #[serde(default)]
            capabilities: Option<Caps>,
        }
        let url = reqwest::Url::parse(&format!("{}/api/v1/indexer", base_url.trim_end_matches('/')))
            .map_err(|_| Error::Status(400))?;
        koda_net::check_url(&url).map_err(|e| Error::Blocked(e.to_string()))?;
        let res = self.http.get(url).header("X-Api-Key", api_key).send().await?;
        match res.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(Error::Auth),
            s => return Err(Error::Status(s)),
        }
        let raw: Vec<Raw> = res.json().await?;
        Ok(raw
            .into_iter()
            .filter_map(|r| {
                let protocol = match r.protocol.as_str() {
                    "torrent" => Protocol::Torrent,
                    "usenet" => Protocol::Usenet,
                    _ => return None,
                };
                let mut cats: Vec<u32> = Vec::new();
                for c in r.capabilities.unwrap_or_default().categories {
                    for id in std::iter::once(c.id).chain(c.sub.iter().map(|s| s.id)) {
                        if (2000..3000).contains(&id) && !cats.contains(&id) {
                            cats.push(id);
                        }
                    }
                }
                Some(ProwlarrIndexer {
                    id: r.id,
                    name: r.name,
                    protocol,
                    enabled: r.enable,
                    movie_categories: cats,
                })
            })
            .collect())
    }
}

/// Wait time before trying an indexer again after `failures` failures in a row.
pub fn backoff(failures: u32) -> Duration {
    const STEPS: [u64; 9] = [60, 300, 900, 1800, 3600, 3 * 3600, 6 * 3600, 12 * 3600, 24 * 3600];
    if failures == 0 {
        return Duration::ZERO;
    }
    Duration::from_secs(STEPS[(failures as usize - 1).min(STEPS.len() - 1)])
}

#[cfg(test)]
mod tests;
