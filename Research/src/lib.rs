//! Bounded public-web research with durable source provenance.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use reqwest::header::LOCATION;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_EXCERPT_CHARS: usize = 4_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResearchSource {
    pub id: String,
    pub url: String,
    pub domain: String,
    pub source_kind: String,
    pub quality: f32,
    pub title: String,
    pub excerpt: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResearchBrief {
    pub unix_ms: u128,
    pub query: String,
    pub sources: Vec<ResearchSource>,
}

#[derive(Debug, Error)]
pub enum ResearchError {
    #[error("research HTTP failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("research I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("research JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("research query is empty")]
    Empty,
    #[error("research response exceeded the 2 MiB safety limit")]
    TooLarge,
    #[error("research host could not be resolved: {0}")]
    Unresolvable(String),
    #[error("research URL is not a permitted public HTTP(S) target: {0}")]
    InvalidUrl(String),
    #[error("research redirect limit exceeded")]
    RedirectLimit,
}

#[derive(Clone, Debug)]
pub struct ResearchService {
    client: Client,
    archive: PathBuf,
}

impl ResearchService {
    pub fn new(archive: impl Into<PathBuf>) -> Result<Self, ResearchError> {
        let archive = archive.into();
        if let Some(parent) = archive.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .user_agent("Teledra-V2/0.1 court research")
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            archive,
        })
    }

    pub fn research(&self, query: &str) -> Result<ResearchBrief, ResearchError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(ResearchError::Empty);
        }
        let sources = if is_url(query) {
            vec![self.fetch(query)?]
        } else {
            self.search(query)?
        };
        let brief = ResearchBrief {
            unix_ms: now_ms(),
            query: query.to_owned(),
            sources,
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.archive)?;
        serde_json::to_writer(&mut file, &brief)?;
        file.write_all(b"\n")?;
        Ok(brief)
    }

    fn search(&self, query: &str) -> Result<Vec<ResearchSource>, ResearchError> {
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            percent_encode(query)
        );
        let html = self.get_bounded(&url)?;
        let mut sources = Vec::new();
        for block in html.split("result__body").skip(1).take(5) {
            let Some(raw_url) = attribute_after(block, "result__a", "href=") else {
                continue;
            };
            let title = text_inside_first(block, "result__a").unwrap_or_default();
            let excerpt = text_inside_first(block, "result__snippet").unwrap_or_default();
            let url = unwrap_search_url(&html_decode(&raw_url));
            let id = format!("S{}", sources.len() + 1);
            if is_url(&url)
                && let Ok(mut source) = self.fetch(&url)
            {
                source.id = id;
                sources.push(source);
            } else {
                let provenance = source_provenance(&url);
                sources.push(ResearchSource {
                    id,
                    domain: provenance.0,
                    source_kind: "search_index".to_owned(),
                    quality: 0.2,
                    url,
                    title: clean_text(&title),
                    excerpt: clean_text(&excerpt)
                        .chars()
                        .take(MAX_EXCERPT_CHARS)
                        .collect(),
                });
            }
        }
        if sources.is_empty() {
            sources.push(ResearchSource {
                id: "S1".to_owned(),
                url,
                domain: "duckduckgo.com".to_owned(),
                source_kind: "search_index".to_owned(),
                quality: 0.2,
                title: format!("Search results for {query}"),
                excerpt: clean_text(&strip_tags(&html))
                    .chars()
                    .take(MAX_EXCERPT_CHARS)
                    .collect(),
            });
        }
        Ok(sources)
    }

    fn fetch(&self, url: &str) -> Result<ResearchSource, ResearchError> {
        let html = self.get_bounded(url)?;
        let title = text_between(&html, "<title", "</title>")
            .and_then(|value| value.split_once('>').map(|(_, value)| value))
            .map(clean_text)
            .unwrap_or_else(|| url.to_owned());
        let excerpt = clean_text(&strip_tags(&html))
            .chars()
            .take(MAX_EXCERPT_CHARS)
            .collect();
        let provenance = source_provenance(url);
        Ok(ResearchSource {
            id: "S1".to_owned(),
            url: url.to_owned(),
            domain: provenance.0,
            source_kind: provenance.1,
            quality: provenance.2,
            title,
            excerpt,
        })
    }

    fn get_bounded(&self, url: &str) -> Result<String, ResearchError> {
        let mut url = public_url(url)?;
        for _ in 0..=5 {
            let response = self.client.get(url.clone()).send()?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| ResearchError::InvalidUrl("redirect had no location".into()))?;
                url = public_url(
                    url.join(location)
                        .map_err(|error| ResearchError::InvalidUrl(error.to_string()))?
                        .as_str(),
                )?;
                continue;
            }
            if response.status() == StatusCode::NO_CONTENT {
                return Ok(String::new());
            }
            response.error_for_status_ref()?;
            if response
                .content_length()
                .is_some_and(|size| size as usize > MAX_RESPONSE_BYTES)
            {
                return Err(ResearchError::TooLarge);
            }
            let mut bytes = Vec::new();
            response
                .take(MAX_RESPONSE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_RESPONSE_BYTES {
                return Err(ResearchError::TooLarge);
            }
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
        Err(ResearchError::RedirectLimit)
    }
}

impl ResearchBrief {
    pub fn prompt_context(&self) -> String {
        let mut text = format!("Research query: {}\n", self.query);
        for source in &self.sources {
            text.push_str(&format!(
                "\nSOURCE {}\nTitle: {}\nURL: {}\nExcerpt: {}\n",
                source.id, source.title, source.url, source.excerpt
            ));
        }
        text.push_str(
            "\nEVIDENCE CONTRACT: Every durable factual claim must copy one complete punctuation-delimited source sentence exactly, preserving negation, modal verbs, actors, quantities, units, signs, dates, and word order. Append it privately as [FACT: SOURCE_ID|exact source sentence]. Put questions and unresolved material in prose as uncertainty; never convert a search snippet or absence of an answer into a verified fact. If sources genuinely oppose the same proposition, quote both positions and identify the conflict.\n",
        );
        text
    }

    pub fn verify_exact_claim(&self, source_id: &str, statement: &str) -> bool {
        let claim = canonical_clause(statement);
        if semantic_tokens(&claim).len() < 2 || claim.contains('?') {
            return false;
        }
        self.sources
            .iter()
            .find(|source| source.id.eq_ignore_ascii_case(source_id.trim()))
            .is_some_and(|source| {
                source.source_kind != "search_index"
                    && evidence_clauses(&source.excerpt)
                        .into_iter()
                        .any(|clause| clause == claim)
            })
    }
}

fn source_provenance(raw_url: &str) -> (String, String, f32) {
    let Ok(url) = Url::parse(raw_url) else {
        return (String::new(), "unknown".to_owned(), 0.0);
    };
    let domain = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    let (kind, quality) = if domain.ends_with(".gov")
        || domain.ends_with(".edu")
        || domain.ends_with(".ac.uk")
        || domain.ends_with("europa.eu")
        || domain == "arxiv.org"
        || domain == "doi.org"
        || domain == "pubmed.ncbi.nlm.nih.gov"
        || domain.starts_with("docs.")
        || path.contains("/docs/")
        || path.contains("/reference/")
        || path.contains("/spec")
    {
        ("primary", 0.88)
    } else if domain.ends_with("wikipedia.org") {
        ("tertiary", 0.42)
    } else if domain.contains("duckduckgo.com") {
        ("search_index", 0.2)
    } else {
        ("secondary", 0.55)
    };
    (domain, kind.to_owned(), quality)
}

fn canonical_clause(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn semantic_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || matches!(character, '\'' | '’' | '+' | '-' | '%' | '=')
        {
            current.push(character);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn evidence_clauses(text: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let characters = text.chars().collect::<Vec<_>>();
    for (index, character) in characters.iter().copied().enumerate() {
        current.push(character);
        if matches!(character, '"' | '“' | '”') {
            quoted = !quoted;
        }
        let decimal = character == '.'
            && index > 0
            && characters[index - 1].is_ascii_digit()
            && characters.get(index + 1).is_some_and(char::is_ascii_digit);
        if !quoted && !decimal && matches!(character, '.' | '!' | '?' | ';' | '\n' | '\r') {
            let clause = canonical_clause(&current);
            if !clause.is_empty() {
                clauses.push(clause);
            }
            current.clear();
        }
    }
    let tail = canonical_clause(&current);
    if !tail.is_empty() {
        clauses.push(tail);
    }
    clauses
}

fn is_url(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://")
}

fn public_url(value: &str) -> Result<Url, ResearchError> {
    let url = Url::parse(value).map_err(|error| ResearchError::InvalidUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ResearchError::InvalidUrl(value.to_owned()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ResearchError::InvalidUrl(value.to_owned()))?;
    let host_lower = host.to_ascii_lowercase();
    if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".local")
    {
        return Err(ResearchError::InvalidUrl(value.to_owned()));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| ResearchError::InvalidUrl(value.to_owned()))?;
    // A name that does not resolve is not the same as a target that is
    // refused. Reporting both as "not a permitted public HTTP(S) target" told
    // the operator their policy had blocked a request that DNS had simply
    // failed to look up.
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| ResearchError::Unresolvable(format!("{host}: {error}")))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(ResearchError::Unresolvable(host.to_owned()));
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(ResearchError::InvalidUrl(value.to_owned()));
    }
    Ok(url)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_unspecified()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && ip.octets()[0] != 0
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && !ip.is_multicast()
        }
    }
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            b' ' => "+".to_owned(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn unwrap_search_url(value: &str) -> String {
    let normalized = html_decode(value);
    let Some(start) = normalized.find("uddg=") else {
        return normalized;
    };
    let encoded = &normalized[start + 5..];
    let encoded = encoded.split('&').next().unwrap_or(encoded);
    percent_decode(encoded).unwrap_or(normalized)
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = hex(bytes[index + 1])?;
                let low = hex(bytes[index + 2])?;
                decoded.push(high * 16 + low);
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn attribute_after(block: &str, class: &str, attribute: &str) -> Option<String> {
    let start = block.find(class)?;
    let tail = &block[start..];
    let attribute = tail.find(attribute)? + attribute.len();
    let tail = tail[attribute..].trim_start();
    let quote = tail.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let value = &tail[quote.len_utf8()..];
    Some(value[..value.find(quote)?].to_owned())
}

fn text_inside_first(block: &str, class: &str) -> Option<String> {
    let start = block.find(class)?;
    let tail = &block[start..];
    let content = tail.find('>')? + 1;
    Some(tail[content..tail[content..].find('<')? + content].to_owned())
}

fn text_between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start = text.find(start)?;
    let tail = &text[start..];
    Some(&tail[..tail.find(end)?])
}

fn strip_tags(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut inside = false;
    for character in html.chars() {
        match character {
            '<' => inside = true,
            '>' => {
                inside = false;
                text.push(' ');
            }
            _ if !inside => text.push(character),
            _ => {}
        }
    }
    text
}

fn clean_text(value: &str) -> String {
    html_decode(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn html_decode(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encoding_is_safe() {
        assert_eq!(percent_encode("rust & music"), "rust+%26+music");
    }

    #[test]
    fn duckduckgo_redirects_are_unwrapped_before_fetching() {
        assert_eq!(
            unwrap_search_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdocs&rut=x"),
            "https://example.com/docs"
        );
    }

    #[test]
    fn markup_is_reduced_to_readable_text() {
        assert_eq!(
            clean_text(&strip_tags("<p>Royal <b>court</b></p>")),
            "Royal court"
        );
    }

    #[test]
    fn local_research_targets_are_rejected() {
        assert!(public_url("http://127.0.0.1/private").is_err());
        assert!(public_url("http://localhost/private").is_err());
        assert!(public_url("file:///private").is_err());
    }

    #[test]
    fn exact_grounding_preserves_negation_numbers_and_actors() {
        let brief = ResearchBrief {
            unix_ms: 0,
            query: "decision".to_owned(),
            sources: vec![ResearchSource {
                id: "S1".to_owned(),
                url: "https://agency.gov/decision".to_owned(),
                domain: "agency.gov".to_owned(),
                source_kind: "primary".to_owned(),
                quality: 0.95,
                title: "Decision".to_owned(),
                excerpt:
                    "The agency has never approved Project Orion. The trial enrolled 12 adults."
                        .to_owned(),
            }],
        };
        assert!(brief.verify_exact_claim("S1", "The agency has never approved Project Orion."));
        assert!(!brief.verify_exact_claim("S1", "The agency approved Project Orion."));
        assert!(!brief.verify_exact_claim("S1", "The trial enrolled 120 adults."));
    }
}
