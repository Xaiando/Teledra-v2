use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::IpAddr;

pub const RESEARCH_BRIEFS_PATH: &str = "knowledge/research_briefs.jsonl";

fn default_schema_version() -> u32 {
    1
}

fn clamp01(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn compact(text: &str, max_chars: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
struct SourceProvenance {
    url: String,
    domain: String,
    source_kind: String,
    quality: f32,
}

fn host_is_or_subdomain(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn is_public_domain(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || !host.contains('.')
        || host.parse::<IpAddr>().is_ok()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".invalid")
        || host.ends_with(".test")
    {
        return false;
    }
    true
}

fn derive_source_provenance(raw_url: &str) -> Option<SourceProvenance> {
    let mut parsed = reqwest::Url::parse(raw_url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let domain = parsed
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !is_public_domain(&domain) {
        return None;
    }

    const SEARCH_INTERMEDIARIES: &[&str] = &[
        "bing.com",
        "duckduckgo.com",
        "google.com",
        "googleusercontent.com",
    ];
    if SEARCH_INTERMEDIARIES
        .iter()
        .any(|candidate| host_is_or_subdomain(&domain, candidate))
    {
        return None;
    }

    let path = parsed.path().to_ascii_lowercase();
    let (source_kind, mut quality) = if domain.ends_with(".gov")
        || host_is_or_subdomain(&domain, "gov.uk")
        || host_is_or_subdomain(&domain, "gov.au")
        || host_is_or_subdomain(&domain, "gc.ca")
        || host_is_or_subdomain(&domain, "europa.eu")
    {
        ("primary", 0.95)
    } else if domain.ends_with(".edu")
        || host_is_or_subdomain(&domain, "ac.uk")
        || host_is_or_subdomain(&domain, "edu.au")
    {
        ("primary", 0.88)
    } else if ["arxiv.org", "doi.org", "pubmed.ncbi.nlm.nih.gov"]
        .iter()
        .any(|candidate| host_is_or_subdomain(&domain, candidate))
    {
        ("primary", 0.90)
    } else if host_is_or_subdomain(&domain, "wikipedia.org") {
        ("tertiary", 0.42)
    } else if domain.starts_with("docs.")
        || domain.starts_with("developer.")
        || domain.starts_with("standards.")
    {
        ("primary", 0.84)
    } else if ["/docs/", "/documentation/", "/reference/", "/spec"]
        .iter()
        .any(|marker| path.contains(marker))
    {
        ("primary", 0.80)
    } else {
        ("secondary", 0.55)
    };
    if parsed.scheme() == "http" {
        quality = (quality - 0.08_f32).max(0.0);
    }

    parsed.set_fragment(None);
    Some(SourceProvenance {
        url: parsed.to_string(),
        domain,
        source_kind: source_kind.to_string(),
        quality,
    })
}

fn canonical_clause(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_interrogative_statement(statement: &str) -> bool {
    // Questions belong in `unknowns`, never in the verified-claim channel.
    // This also catches quoted or FAQ-wrapped questions whose marker is not
    // the final character of the clause.
    statement.contains('?')
}

fn semantic_token_sequence(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        let signed_number = matches!(character, '+' | '-' | '−')
            && characters
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_digit());
        if character.is_ascii_digit() || signed_number {
            let mut token = String::new();
            if signed_number {
                token.push(if character == '−' { '-' } else { character });
                index += 1;
            }
            while index < characters.len() {
                let current = characters[index];
                if current.is_ascii_digit() {
                    token.push(current);
                    index += 1;
                    continue;
                }
                if matches!(current, '.' | ',')
                    && characters
                        .get(index + 1)
                        .is_some_and(|next| next.is_ascii_digit())
                {
                    token.push(current);
                    index += 1;
                    continue;
                }
                break;
            }
            tokens.push(token);
            continue;
        }
        if character.is_alphanumeric() {
            let mut token = String::new();
            while index < characters.len() {
                let current = characters[index];
                if current.is_alphanumeric() {
                    token.push(current);
                    index += 1;
                    continue;
                }
                if matches!(current, '\'' | '’')
                    && !token.is_empty()
                    && characters
                        .get(index + 1)
                        .is_some_and(|next| next.is_alphanumeric())
                {
                    token.push('\'');
                    index += 1;
                    continue;
                }
                break;
            }
            tokens.push(token);
            continue;
        }
        if !character.is_whitespace() {
            tokens.push(character.to_string());
        }
        index += 1;
    }
    tokens
}

fn dispute_key(statement: &str) -> String {
    let mut tokens = semantic_token_sequence(statement);
    let closing = |token: &str| matches!(token, "\"" | "'" | "”" | "’" | ")" | "]" | "}");
    let mut core_end = tokens.len();
    while core_end > 0 && closing(&tokens[core_end - 1]) {
        core_end -= 1;
    }
    while core_end > 0 && matches!(tokens[core_end - 1].as_str(), "." | "!") {
        tokens.remove(core_end - 1);
        core_end -= 1;
    }
    tokens
        .into_iter()
        .map(|token| {
            let acronym = token.chars().any(char::is_alphabetic)
                && token
                    .chars()
                    .all(|character| !character.is_alphabetic() || character.is_uppercase())
                && token.chars().count() <= 12;
            if acronym {
                format!("@{token}")
            } else {
                token.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

fn evidence_clause_sequences(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut clauses = Vec::new();
    let mut current = String::new();
    let mut pending_terminal = false;
    let mut inside_double_quote = false;
    let mut inside_single_quote = false;
    let mut parenthesis_depth = 0_u16;
    let mut bracket_depth = 0_u16;
    let mut brace_depth = 0_u16;

    let flush = |current: &mut String, clauses: &mut Vec<String>| {
        let clause = canonical_clause(current);
        if !clause.is_empty() {
            clauses.push(clause);
        }
        current.clear();
    };

    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if pending_terminal {
            if matches!(character, '.' | '!' | '?') {
                current.push(character);
                index += 1;
                continue;
            }
            if matches!(character, '"' | '\'' | '”' | '’' | ')' | ']' | '}') {
                current.push(character);
                index += 1;
                continue;
            }
            if character.is_whitespace() {
                flush(&mut current, &mut clauses);
                pending_terminal = false;
                index += 1;
                continue;
            }
            flush(&mut current, &mut clauses);
            pending_terminal = false;
            continue;
        }

        let inside_group = parenthesis_depth > 0 || bracket_depth > 0 || brace_depth > 0;
        if matches!(character, ';' | '\n' | '\r')
            && !inside_double_quote
            && !inside_single_quote
            && !inside_group
        {
            flush(&mut current, &mut clauses);
            index += 1;
            continue;
        }
        if character == '"' {
            inside_double_quote = !inside_double_quote;
            current.push(character);
            index += 1;
            continue;
        }
        if character == '“' {
            inside_double_quote = true;
            current.push(character);
            index += 1;
            continue;
        }
        if character == '”' {
            inside_double_quote = false;
            current.push(character);
            index += 1;
            continue;
        }
        if matches!(character, '\'' | '‘' | '’') {
            let previous_is_alphanumeric = index
                .checked_sub(1)
                .and_then(|prior| characters.get(prior))
                .is_some_and(|previous| previous.is_alphanumeric());
            let next_is_alphanumeric = characters
                .get(index + 1)
                .is_some_and(|next| next.is_alphanumeric());
            if !(previous_is_alphanumeric && next_is_alphanumeric) {
                inside_single_quote = match character {
                    '‘' => true,
                    '’' => false,
                    _ => !inside_single_quote,
                };
            }
            current.push(character);
            index += 1;
            continue;
        }
        match character {
            '(' => parenthesis_depth = parenthesis_depth.saturating_add(1),
            ')' => parenthesis_depth = parenthesis_depth.saturating_sub(1),
            '[' => bracket_depth = bracket_depth.saturating_add(1),
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth = brace_depth.saturating_add(1),
            '}' => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
        let inside_quote = inside_double_quote || inside_single_quote;
        let inside_group = parenthesis_depth > 0 || bracket_depth > 0 || brace_depth > 0;
        let inline_bang_or_question = matches!(character, '!' | '?')
            && characters
                .get(index + 1)
                .is_some_and(|next| next.is_alphanumeric() || *next == '=');
        let period_boundary = if character == '.' && !inside_quote && !inside_group {
            let previous = index.checked_sub(1).and_then(|prior| characters.get(prior));
            let next = characters.get(index + 1);
            let decimal_or_inline = previous.is_some_and(|value| value.is_ascii_digit())
                && next.is_some_and(|value| value.is_ascii_digit())
                || next.is_some_and(|value| value.is_alphanumeric() || *value == '.')
                || previous.is_some_and(|value| *value == '.');
            let before_period = current.trim_end_matches(|value| {
                matches!(value, ')' | ']' | '}' | '"' | '\'' | '”' | '’')
            });
            let token_start = before_period
                .char_indices()
                .rev()
                .find_map(|(offset, value)| {
                    (!value.is_alphanumeric()).then_some(offset + value.len_utf8())
                })
                .unwrap_or(0);
            let preceding_token = &before_period[token_start..];
            let dotted_abbreviation =
                preceding_token.chars().count() == 1 && before_period[..token_start].ends_with('.');
            let known_abbreviation = matches!(
                preceding_token.to_ascii_lowercase().as_str(),
                "dr" | "etc" | "jr" | "mr" | "mrs" | "ms" | "prof" | "sr" | "st" | "vs"
            );
            !decimal_or_inline && !dotted_abbreviation && !known_abbreviation
        } else {
            false
        };

        current.push(character);
        if (matches!(character, '!' | '?')
            && !inline_bang_or_question
            && !inside_quote
            && !inside_group)
            || period_boundary
        {
            pending_terminal = true;
        }
        index += 1;
    }
    flush(&mut current, &mut clauses);
    clauses
}

fn bounded_source_excerpt(text: &str, max_chars: usize) -> String {
    let collapsed = canonical_clause(text);
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let mut bounded = String::new();
    for clause in evidence_clause_sequences(&collapsed) {
        let clause_len = clause.chars().count();
        if clause_len > max_chars {
            continue;
        }
        let separator = usize::from(!bounded.is_empty());
        if bounded.chars().count() + separator + clause_len > max_chars {
            break;
        }
        if separator == 1 {
            bounded.push(' ');
        }
        bounded.push_str(&clause);
    }
    bounded
}

fn source_supports_statement(statement: &str, source: &ResearchSource) -> bool {
    let claim = canonical_clause(statement);
    if semantic_token_sequence(&claim).len() < 2 || is_interrogative_statement(&claim) {
        return false;
    }

    // Only a complete punctuation-delimited source clause can ground a claim.
    // This deliberately rejects embedded substrings such as "it is not true
    // that [claim]" and attribution/debunking shells whose outer relation would
    // otherwise be lost. Equality is intentionally lexical (apart from
    // collapsed whitespace), so punctuation, operators, modal verbs,
    // contractions, actors, signs, units, grouping, and quantities survive.
    evidence_clause_sequences(&source.excerpt)
        .into_iter()
        .any(|clause| clause == claim)
}

fn claim_supported_by_sources(
    statement: &str,
    source_ids: &[String],
    sources: &[ResearchSource],
) -> bool {
    if source_ids.is_empty() {
        return false;
    }
    source_ids.iter().any(|id| {
        sources
            .iter()
            .find(|source| &source.id == id)
            .is_some_and(|source| source_supports_statement(statement, source))
    })
}

fn positions_show_opposition(positions: &[ResearchPosition]) -> bool {
    let polarity_pairs: &[(&[&str], &[&str])] = &[
        (
            &[
                "approve",
                "approved",
                "accept",
                "accepted",
                "support",
                "supported",
            ],
            &["reject", "rejected", "deny", "denied", "oppose", "opposed"],
        ),
        (&["stable"], &["unstable"]),
        (
            &["increase", "increased", "higher", "rise", "rose"],
            &["decrease", "decreased", "lower", "fall", "fell"],
        ),
        (&["required", "mandatory"], &["optional"]),
        (&["success", "successful", "passed"], &["failure", "failed"]),
        (&["true"], &["false"]),
    ];

    fn differs_only_by_opposing_token(
        left: &[String],
        right: &[String],
        positive: &[&str],
        negative: &[&str],
    ) -> bool {
        if left.len() != right.len() || left.len() < 3 {
            return false;
        }
        let differences: Vec<(usize, &str, &str)> = left
            .iter()
            .zip(right.iter())
            .enumerate()
            .filter(|(_, (left_token, right_token))| left_token != right_token)
            .map(|(index, (left_token, right_token))| {
                (index, left_token.as_str(), right_token.as_str())
            })
            .collect();
        let is_member = |token: &str, candidates: &[&str]| {
            candidates
                .iter()
                .any(|candidate| token.eq_ignore_ascii_case(candidate))
        };
        let predicate_role = |sequence: &[String], index: usize| {
            let previous = index
                .checked_sub(1)
                .and_then(|prior| sequence.get(prior))
                .map(String::as_str);
            let follows_copula = previous.is_some_and(|value| {
                [
                    "appear", "appeared", "appears", "are", "be", "became", "become", "becomes",
                    "been", "being", "is", "remain", "remained", "remains", "seem", "seemed",
                    "seems", "was", "were",
                ]
                .iter()
                .any(|copula| value.eq_ignore_ascii_case(copula))
            });
            follows_copula
        };
        differences.len() == 1
            && predicate_role(left, differences[0].0)
            && predicate_role(right, differences[0].0)
            && ((is_member(differences[0].1, positive) && is_member(differences[0].2, negative))
                || (is_member(differences[0].1, negative) && is_member(differences[0].2, positive)))
    }

    fn differs_only_by_one_negator(longer: &[String], shorter: &[String]) -> bool {
        const NEGATORS: &[&str] = &["no", "not", "never", "none"];
        if longer.len() != shorter.len() + 1 {
            return false;
        }
        let negator_indices: Vec<usize> = longer
            .iter()
            .enumerate()
            .filter_map(|(index, token)| {
                NEGATORS
                    .iter()
                    .any(|negator| token.eq_ignore_ascii_case(negator))
                    .then_some(index)
            })
            .collect();
        if negator_indices.len() != 1 {
            return false;
        }
        let index = negator_indices[0];
        let previous = index
            .checked_sub(1)
            .and_then(|prior| longer.get(prior))
            .map(String::as_str);
        let follows_auxiliary = previous.is_some_and(|value| {
            [
                "am", "are", "be", "been", "being", "did", "do", "does", "had", "has", "have",
                "is", "must", "shall", "should", "was", "were", "will", "would",
            ]
            .iter()
            .any(|auxiliary| value.eq_ignore_ascii_case(auxiliary))
        });
        if !follows_auxiliary {
            return false;
        }
        longer[..index] == shorter[..index] && longer[index + 1..] == shorter[index..]
    }

    fn numeric_value(token: &str) -> Option<f64> {
        let normalized = token.replace(',', "");
        normalized
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
    }

    fn differs_only_by_one_numeric_value(left: &[String], right: &[String]) -> bool {
        if left.len() != right.len() || left.len() < 3 {
            return false;
        }
        let left_numbers: Vec<usize> = left
            .iter()
            .enumerate()
            .filter_map(|(index, token)| numeric_value(token).map(|_| index))
            .collect();
        let right_numbers: Vec<usize> = right
            .iter()
            .enumerate()
            .filter_map(|(index, token)| numeric_value(token).map(|_| index))
            .collect();
        if left_numbers.len() != 1 || right_numbers.len() != 1 || left_numbers != right_numbers {
            return false;
        }
        let index = left_numbers[0];
        if numeric_value(&left[index]) == numeric_value(&right[index]) {
            return false;
        }
        const VALUE_INTRODUCERS: &[&str] = &[
            "are",
            "contained",
            "contains",
            "enrolled",
            "equaled",
            "equals",
            "had",
            "has",
            "is",
            "measured",
            "reached",
            "recorded",
            "reported",
            "showed",
            "shows",
            "totaled",
            "was",
            "were",
        ];
        if !index
            .checked_sub(1)
            .and_then(|prior| left.get(prior))
            .is_some_and(|token| {
                VALUE_INTRODUCERS
                    .iter()
                    .any(|introducer| token.eq_ignore_ascii_case(introducer))
            })
        {
            return false;
        }
        if left[index + 1..].iter().any(|token| {
            [
                "+",
                "<",
                ">",
                "above",
                "about",
                "approximately",
                "around",
                "below",
                "fewer",
                "greater",
                "higher",
                "least",
                "less",
                "lower",
                "maximum",
                "minimum",
                "more",
                "most",
                "over",
                "roughly",
                "under",
                "up",
                "≤",
                "≥",
            ]
            .iter()
            .any(|bound| token.eq_ignore_ascii_case(bound))
        }) {
            return false;
        }
        left.iter()
            .zip(right.iter())
            .enumerate()
            .all(|(token_index, (left_token, right_token))| {
                token_index == index || left_token == right_token
            })
    }

    for left_index in 0..positions.len() {
        for right_index in (left_index + 1)..positions.len() {
            if is_interrogative_statement(&positions[left_index].statement)
                || is_interrogative_statement(&positions[right_index].statement)
            {
                continue;
            }
            let left = semantic_token_sequence(&positions[left_index].statement);
            let right = semantic_token_sequence(&positions[right_index].statement);
            if polarity_pairs.iter().any(|(positive, negative)| {
                differs_only_by_opposing_token(&left, &right, positive, negative)
            }) {
                return true;
            }
            if differs_only_by_one_negator(&left, &right)
                || differs_only_by_one_negator(&right, &left)
            {
                return true;
            }
            if differs_only_by_one_numeric_value(&left, &right) {
                return true;
            }
        }
    }
    false
}

fn normalize_sources(sources: Vec<ResearchSource>) -> Vec<ResearchSource> {
    let mut seen_ids = HashSet::new();
    let mut seen_urls = HashSet::new();
    sources
        .into_iter()
        .filter_map(|mut source| {
            source.id = compact(&source.id, 20);
            source.title = compact(&source.title, 300);
            source.url = compact(&source.url, 1_500);
            source.excerpt = bounded_source_excerpt(&source.excerpt, 5_000);
            let provenance = derive_source_provenance(&source.url)?;
            source.url = provenance.url;
            source.domain = provenance.domain;
            source.source_kind = provenance.source_kind;
            source.quality = provenance.quality;
            if source.id.is_empty()
                || source.title.is_empty()
                || source.excerpt.len() < 30
                || !seen_ids.insert(source.id.clone())
                || !seen_urls.insert(source.url.clone())
            {
                None
            } else {
                Some(source)
            }
        })
        .take(8)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSource {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default = "default_source_kind")]
    pub source_kind: String,
    #[serde(default)]
    pub quality: f32,
    #[serde(default)]
    pub excerpt: String,
}

fn default_source_kind() -> String {
    "unknown".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResearchBundle {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub sources: Vec<ResearchSource>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub raw_text: String,
}

impl BrowserResearchBundle {
    pub fn from_json(raw: &str, expected_query: &str) -> Result<Self, String> {
        let mut bundle: Self = serde_json::from_str(raw)
            .map_err(|error| format!("invalid browser evidence JSON: {error}"))?;
        if bundle.schema_version != 1 {
            return Err(format!(
                "unsupported browser evidence schema {}",
                bundle.schema_version
            ));
        }

        bundle.query = compact(expected_query, 240);
        bundle.raw_text = compact(&bundle.raw_text, 24_000);
        bundle.diagnostics = bundle
            .diagnostics
            .iter()
            .map(|item| compact(item, 500))
            .filter(|item| !item.is_empty())
            .take(12)
            .collect();

        bundle.sources = normalize_sources(bundle.sources);

        Ok(bundle)
    }

    pub fn synthesis_context(&self) -> String {
        let mut out = format!("QUERY: {}\n", self.query);
        for source in &self.sources {
            out.push_str(&format!(
                "\n[{}] {}\nURL: {}\nTYPE: {}\nQUALITY PRIOR: {:.2}\nEXCERPT: {}\n",
                source.id,
                source.title,
                source.url,
                source.source_kind,
                source.quality,
                source.excerpt
            ));
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchClaim {
    pub statement: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchPosition {
    pub source_id: String,
    pub statement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchContradiction {
    pub statement: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub positions: Vec<ResearchPosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelResearchSynthesis {
    #[serde(default)]
    pub claims: Vec<ResearchClaim>,
    #[serde(default)]
    pub contradictions: Vec<ResearchContradiction>,
    #[serde(default)]
    pub unknowns: Vec<String>,
    #[serde(default)]
    pub overall_confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchBrief {
    pub schema_version: u32,
    pub timestamp: String,
    pub query: String,
    pub sources: Vec<ResearchSource>,
    pub claims: Vec<ResearchClaim>,
    pub contradictions: Vec<ResearchContradiction>,
    pub unknowns: Vec<String>,
    pub overall_confidence: f32,
    pub usable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

impl ResearchBrief {
    pub fn from_model_json(
        timestamp: String,
        bundle: BrowserResearchBundle,
        raw: &str,
    ) -> Result<Self, String> {
        let json = extract_json_object(raw)
            .ok_or_else(|| "research synthesizer returned no JSON object".to_string())?;
        let synthesis: ModelResearchSynthesis = serde_json::from_str(json)
            .map_err(|error| format!("invalid research synthesis JSON: {error}"))?;
        Ok(Self::from_synthesis(timestamp, bundle, synthesis))
    }

    pub fn from_synthesis(
        timestamp: String,
        mut bundle: BrowserResearchBundle,
        synthesis: ModelResearchSynthesis,
    ) -> Self {
        bundle.sources = normalize_sources(bundle.sources);
        let valid_ids: HashSet<String> = bundle
            .sources
            .iter()
            .map(|source| source.id.clone())
            .collect();
        let source_quality = |ids: &[String]| -> f32 {
            let qualities: Vec<f32> = ids
                .iter()
                .filter_map(|id| {
                    bundle
                        .sources
                        .iter()
                        .find(|source| &source.id == id)
                        .map(|source| source.quality)
                })
                .collect();
            if qualities.is_empty() {
                0.0
            } else {
                qualities.iter().sum::<f32>() / qualities.len() as f32
            }
        };

        let mut seen_claims = HashSet::new();
        let mut claims: Vec<ResearchClaim> = synthesis
            .claims
            .into_iter()
            .filter_map(|mut claim| {
                claim.statement = compact(&claim.statement, 900);
                claim.source_ids.retain(|id| valid_ids.contains(id));
                claim.source_ids.retain(|id| {
                    bundle
                        .sources
                        .iter()
                        .find(|source| &source.id == id)
                        .is_some_and(|source| source_supports_statement(&claim.statement, source))
                });
                claim.source_ids.sort();
                claim.source_ids.dedup();
                let key = claim.statement.to_ascii_lowercase();
                if claim.statement.len() < 25
                    || claim.source_ids.is_empty()
                    || !claim_supported_by_sources(
                        &claim.statement,
                        &claim.source_ids,
                        &bundle.sources,
                    )
                    || !seen_claims.insert(key)
                {
                    return None;
                }
                // A weak source cannot become a high-confidence claim merely because
                // the language model sounded certain.
                claim.confidence = clamp01(claim.confidence).min(source_quality(&claim.source_ids));
                (claim.confidence > f32::EPSILON).then_some(claim)
            })
            .take(6)
            .collect();

        let contradictions: Vec<ResearchContradiction> = synthesis
            .contradictions
            .into_iter()
            .filter_map(|mut item| {
                item.positions = item
                    .positions
                    .into_iter()
                    .filter_map(|mut position| {
                        position.source_id = compact(&position.source_id, 20);
                        position.statement = compact(&position.statement, 700);
                        bundle
                            .sources
                            .iter()
                            .find(|source| source.id == position.source_id)
                            .filter(|source| source_supports_statement(&position.statement, source))
                            .map(|_| position)
                    })
                    .take(4)
                    .collect();
                item.positions
                    .sort_by(|left, right| left.source_id.cmp(&right.source_id));
                item.positions
                    .dedup_by(|left, right| left.source_id == right.source_id);
                item.source_ids = item
                    .positions
                    .iter()
                    .map(|position| position.source_id.clone())
                    .collect();
                if item.positions.len() < 2 || !positions_show_opposition(&item.positions) {
                    return None;
                }
                item.statement = compact(
                    &item
                        .positions
                        .iter()
                        .map(|position| format!("{}: {}", position.source_id, position.statement))
                        .collect::<Vec<_>>()
                        .join(" | "),
                    700,
                );
                Some(item)
            })
            .take(4)
            .collect();

        let disputed_statements: HashSet<String> = contradictions
            .iter()
            .flat_map(|contradiction| contradiction.positions.iter())
            .map(|position| dispute_key(&position.statement))
            .collect();
        claims.retain(|claim| !disputed_statements.contains(&dispute_key(&claim.statement)));

        let mut seen_unknowns = HashSet::new();
        let mut unknowns: Vec<String> = synthesis
            .unknowns
            .iter()
            .map(|item| compact(item, 500))
            .filter(|item| item.len() >= 12)
            .filter(|item| seen_unknowns.insert(item.to_ascii_lowercase()))
            .take(6)
            .collect();
        if unknowns.is_empty() {
            unknowns.push(
                "The available source excerpts may omit context needed to test these claims."
                    .to_string(),
            );
        }

        let evidence_confidence = if claims.is_empty() {
            0.0
        } else {
            claims.iter().map(|claim| claim.confidence).sum::<f32>() / claims.len() as f32
        };
        let overall_confidence = clamp01(synthesis.overall_confidence).min(evidence_confidence);
        let usable =
            !claims.is_empty() && !bundle.sources.is_empty() && overall_confidence > f32::EPSILON;

        Self {
            schema_version: 1,
            timestamp,
            query: bundle.query,
            sources: bundle.sources,
            claims,
            contradictions,
            unknowns,
            overall_confidence,
            usable,
            failure: None,
        }
    }

    pub fn failed(timestamp: String, mut bundle: BrowserResearchBundle, reason: &str) -> Self {
        bundle.sources = normalize_sources(bundle.sources);
        Self {
            schema_version: 1,
            timestamp,
            query: bundle.query,
            sources: bundle.sources,
            claims: Vec::new(),
            contradictions: Vec::new(),
            unknowns: vec![
                "No grounded synthesis was produced; the preserved sources require review."
                    .to_string(),
            ],
            overall_confidence: 0.0,
            usable: false,
            failure: Some(compact(reason, 800)),
        }
    }

    pub fn best_fact(&self) -> Option<String> {
        if !self.usable || self.overall_confidence <= f32::EPSILON {
            return None;
        }
        let disputed_statements: HashSet<String> = self
            .contradictions
            .iter()
            .flat_map(|contradiction| contradiction.positions.iter())
            .map(|position| dispute_key(&position.statement))
            .collect();
        let claim = self
            .claims
            .iter()
            .filter(|claim| {
                claim.confidence > f32::EPSILON
                    && !disputed_statements.contains(&dispute_key(&claim.statement))
                    && claim_supported_by_sources(
                        &claim.statement,
                        &claim.source_ids,
                        &self.sources,
                    )
            })
            .max_by(|a, b| a.confidence.total_cmp(&b.confidence))?;
        let domains: Vec<String> = claim
            .source_ids
            .iter()
            .filter_map(|id| {
                self.sources
                    .iter()
                    .find(|source| &source.id == id)
                    .and_then(|source| derive_source_provenance(&source.url))
                    .map(|provenance| provenance.domain)
            })
            .filter(|domain| !domain.is_empty())
            .collect();
        if domains.is_empty() {
            Some(claim.statement.clone())
        } else {
            Some(format!(
                "{} (Sources: {})",
                claim.statement,
                domains.join(", ")
            ))
        }
    }

    pub fn status_summary(&self) -> String {
        format!(
            "{} grounded claim(s), {} source(s), {} contradiction(s), {} open unknown(s), confidence {:.0}%",
            self.claims.len(),
            self.sources.len(),
            self.contradictions.len(),
            self.unknowns.len(),
            self.overall_confidence * 100.0
        )
    }
}

fn extract_json_object(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        None
    } else {
        Some(&raw[start..=end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> BrowserResearchBundle {
        BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "query": "ignored",
                "sources": [
                    {"id":"S1","title":"Official docs","url":"https://docs.example.com/spec#browser-fragment","domain":"attacker.gov","source_kind":"primary","quality":1.0,"excerpt":"The official specification defines a stable and inspectable behavior. The source documents concrete requirements for this grounded test claim. The measured system behavior is stable under the inspectable test conditions."},
                    {"id":"S2","title":"Study","url":"https://example.edu/paper","domain":"attacker.gov","source_kind":"tertiary","quality":0.01,"excerpt":"This independent study documents a disagreement. The measured system behavior is unstable under the inspectable test conditions."},
                    {"id":"BAD","title":"Local","url":"file:///secret","domain":"","source_kind":"unknown","quality":1.0,"excerpt":"This must never survive source validation even if its quality is declared as perfect."}
                ]
            }"#,
            "real query",
        )
        .unwrap()
    }

    #[test]
    fn browser_bundle_rejects_non_http_sources() {
        let bundle = bundle();
        assert_eq!(bundle.query, "real query");
        assert_eq!(bundle.sources.len(), 2);
        assert_eq!(bundle.sources[0].domain, "docs.example.com");
        assert_eq!(bundle.sources[0].source_kind, "primary");
        assert_eq!(bundle.sources[0].quality, 0.84);
        assert_eq!(bundle.sources[0].url, "https://docs.example.com/spec");
        assert_eq!(bundle.sources[1].domain, "example.edu");
        assert_eq!(bundle.sources[1].source_kind, "primary");
        assert_eq!(bundle.sources[1].quality, 0.88);
    }

    #[test]
    fn synthesis_drops_ungrounded_claims_and_invalid_contradictions() {
        let synthesis = ModelResearchSynthesis {
            claims: vec![
                ResearchClaim {
                    statement:
                        "The official specification defines a stable and inspectable behavior."
                            .into(),
                    source_ids: vec!["S1".into()],
                    confidence: 1.0,
                },
                ResearchClaim {
                    statement: "This hallucinated claim cites a source that was never retrieved."
                        .into(),
                    source_ids: vec!["S99".into()],
                    confidence: 1.0,
                },
                ResearchClaim {
                    statement: "Quantum teleportation was commercially deployed in every major city in 2042."
                        .into(),
                    source_ids: vec!["S1".into()],
                    confidence: 1.0,
                },
            ],
            contradictions: vec![
                ResearchContradiction {
                    statement: "The two retrieved sources disagree about the measured behavior."
                        .into(),
                    source_ids: vec!["S1".into(), "S2".into()],
                    positions: vec![
                        ResearchPosition {
                            source_id: "S1".into(),
                            statement: "The measured system behavior is stable under the inspectable test conditions.".into(),
                        },
                        ResearchPosition {
                            source_id: "S2".into(),
                            statement: "The measured system behavior is unstable under the inspectable test conditions.".into(),
                        },
                    ],
                },
                ResearchContradiction {
                    statement: "One source alone is not a contradiction.".into(),
                    source_ids: vec!["S1".into()],
                    positions: vec![ResearchPosition {
                        source_id: "S1".into(),
                        statement: "The measured system behavior is stable under the inspectable test conditions.".into(),
                    }],
                },
            ],
            unknowns: vec!["No replication data was included in either excerpt.".into()],
            overall_confidence: 1.0,
        };
        let brief = ResearchBrief::from_synthesis("1".into(), bundle(), synthesis);
        assert_eq!(brief.claims.len(), 1);
        assert_eq!(brief.contradictions.len(), 1);
        assert_eq!(brief.overall_confidence, 0.84);
        assert!(brief.best_fact().unwrap().contains("docs.example.com"));
    }

    #[test]
    fn fenced_model_json_is_accepted() {
        let raw = r#"```json
        {"claims":[{"statement":"The official specification defines a stable and inspectable behavior.","source_ids":["S1"],"confidence":0.7}],"contradictions":[],"unknowns":["Long-term behavior is not described."],"overall_confidence":0.7}
        ```"#;
        let brief = ResearchBrief::from_model_json("1".into(), bundle(), raw).unwrap();
        assert!(brief.usable);
        assert_eq!(brief.claims.len(), 1);
    }

    #[test]
    fn browser_cannot_spoof_provenance_or_smuggle_local_sources() {
        let bundle = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [
                    {"id":"S1","title":"Ordinary article","url":"https://news.example.net/report","domain":"agency.gov","source_kind":"primary","quality":1.0,"excerpt":"The ordinary article contains enough source text to survive structural validation safely."},
                    {"id":"S2","title":"Loopback","url":"http://127.0.0.1/private","domain":"agency.gov","source_kind":"primary","quality":1.0,"excerpt":"A local endpoint must never become accepted evidence despite browser supplied provenance."},
                    {"id":"S3","title":"Search intermediary","url":"https://google.com/search?q=claim","domain":"agency.gov","source_kind":"primary","quality":1.0,"excerpt":"A search result intermediary is not the retrieved primary source behind a factual claim."},
                    {"id":"S4","title":"Credential URL","url":"https://user:secret@example.org/report","domain":"agency.gov","source_kind":"primary","quality":1.0,"excerpt":"Credentials embedded in a source URL are rejected by deterministic provenance policy."}
                ]
            }"#,
            "provenance audit",
        )
        .unwrap();

        assert_eq!(bundle.sources.len(), 1);
        let source = &bundle.sources[0];
        assert_eq!(source.id, "S1");
        assert_eq!(source.domain, "news.example.net");
        assert_eq!(source.source_kind, "secondary");
        assert_eq!(source.quality, 0.55);
    }

    #[test]
    fn valid_citation_id_does_not_ground_unrelated_model_prose() {
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement:
                    "Quantum teleportation was commercially deployed in every major city in 2042."
                        .into(),
                source_ids: vec!["S1".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec![
                "Commercial deployment remains unknown from the supplied excerpts.".into(),
            ],
            overall_confidence: 1.0,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), bundle(), synthesis);

        assert!(brief.claims.is_empty());
        assert!(!brief.usable);
        assert_eq!(brief.overall_confidence, 0.0);
        assert_eq!(
            brief.unknowns,
            vec!["Commercial deployment remains unknown from the supplied excerpts."]
        );
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn identical_vocabulary_cannot_hide_a_reversed_relation() {
        let evidence = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [{
                    "id":"S1",
                    "title":"Decision record",
                    "url":"https://docs.example.org/decisions",
                    "excerpt":"Alice approved Project Alder after review, while Bob rejected Project Birch after review."
                }]
            }"#,
            "project decisions",
        )
        .unwrap();
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement:
                    "Alice rejected Project Birch while Bob approved Project Alder after review."
                        .into(),
                source_ids: vec!["S1".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["No other decision record was supplied.".into()],
            overall_confidence: 1.0,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);

        assert!(brief.claims.is_empty());
        assert!(!brief.usable);
        assert!(brief.best_fact().is_none());

        let mut differently_styled_bundle = bundle();
        differently_styled_bundle.sources.push(ResearchSource {
            id: "S3".into(),
            title: "Independent lowercase record".into(),
            url: "https://docs.example.org/lowercase-record".into(),
            domain: String::new(),
            source_kind: String::new(),
            quality: 0.0,
            excerpt: "the measured system behavior is stable under the inspectable test conditions"
                .into(),
        });
        let differently_styled = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement:
                    "the measured system behavior is stable under the inspectable test conditions"
                        .into(),
                source_ids: vec!["S3".into()],
                confidence: 0.9,
            }],
            contradictions: vec![ResearchContradiction {
                statement: "The measurements disagree.".into(),
                source_ids: vec!["S1".into(), "S2".into()],
                positions: vec![
                    ResearchPosition {
                        source_id: "S1".into(),
                        statement: "The measured system behavior is stable under the inspectable test conditions.".into(),
                    },
                    ResearchPosition {
                        source_id: "S2".into(),
                        statement: "The measured system behavior is unstable under the inspectable test conditions.".into(),
                    },
                ],
            }],
            unknowns: vec!["The disagreement remains unresolved.".into()],
            overall_confidence: 0.9,
        };
        let brief = ResearchBrief::from_synthesis(
            "1".into(),
            differently_styled_bundle,
            differently_styled,
        );
        assert!(brief.claims.is_empty());
        assert!(!brief.usable);
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn contradiction_requires_exact_per_source_opposing_positions() {
        let unsupported = ModelResearchSynthesis {
            claims: Vec::new(),
            contradictions: vec![ResearchContradiction {
                statement: "The sources disagree.".into(),
                source_ids: vec!["S1".into(), "S2".into()],
                positions: Vec::new(),
            }],
            unknowns: vec!["The disagreement requires quoted source positions.".into()],
            overall_confidence: 0.5,
        };
        let brief = ResearchBrief::from_synthesis("1".into(), bundle(), unsupported);
        assert!(brief.contradictions.is_empty());

        let supported = ModelResearchSynthesis {
            claims: Vec::new(),
            contradictions: vec![ResearchContradiction {
                statement: "ignored model summary".into(),
                source_ids: vec!["S1".into(), "S2".into()],
                positions: vec![
                    ResearchPosition {
                        source_id: "S1".into(),
                        statement: "The measured system behavior is stable under the inspectable test conditions.".into(),
                    },
                    ResearchPosition {
                        source_id: "S2".into(),
                        statement: "The measured system behavior is unstable under the inspectable test conditions.".into(),
                    },
                ],
            }],
            unknowns: vec!["The full methods remain outside the excerpts.".into()],
            overall_confidence: 0.5,
        };
        let brief = ResearchBrief::from_synthesis("1".into(), bundle(), supported);
        assert_eq!(brief.contradictions.len(), 1);
        assert!(brief.contradictions[0].statement.starts_with("S1:"));
        assert_eq!(brief.contradictions[0].positions.len(), 2);
    }

    #[test]
    fn contested_claims_are_quarantined_from_verified_fact_memory() {
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement: "The measured system behavior is stable under the inspectable test conditions.".into(),
                source_ids: vec!["S1".into()],
                confidence: 0.9,
            }],
            contradictions: vec![ResearchContradiction {
                statement: "The measurements disagree.".into(),
                source_ids: vec!["S1".into(), "S2".into()],
                positions: vec![
                    ResearchPosition {
                        source_id: "S1".into(),
                        statement: "The measured system behavior is stable under the inspectable test conditions.".into(),
                    },
                    ResearchPosition {
                        source_id: "S2".into(),
                        statement: "The measured system behavior is unstable under the inspectable test conditions.".into(),
                    },
                ],
            }],
            unknowns: vec!["The disagreement remains unresolved.".into()],
            overall_confidence: 0.9,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), bundle(), synthesis);
        assert_eq!(brief.contradictions.len(), 1);
        assert!(brief.claims.is_empty());
        assert!(!brief.usable);
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn zero_confidence_and_wrapped_questions_never_become_verified_facts() {
        let zero_confidence = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement: "The official specification defines a stable and inspectable behavior."
                    .into(),
                source_ids: vec!["S1".into()],
                confidence: 0.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["Confidence is explicitly absent.".into()],
            overall_confidence: 1.0,
        };
        let brief = ResearchBrief::from_synthesis("1".into(), bundle(), zero_confidence);
        assert!(brief.claims.is_empty());
        assert!(!brief.usable);
        assert!(brief.best_fact().is_none());

        let question_bundle = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [{
                    "id":"S1",
                    "title":"Trial FAQ",
                    "url":"https://docs.example.org/trial-faq",
                    "excerpt":"\"Did the trial enroll 120 adults?\" [FAQ]"
                }]
            }"#,
            "trial enrollment",
        )
        .unwrap();
        let question = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement: "\"Did the trial enroll 120 adults?\" [FAQ]".into(),
                source_ids: vec!["S1".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["The FAQ asks but does not answer the question.".into()],
            overall_confidence: 1.0,
        };
        let brief = ResearchBrief::from_synthesis("1".into(), question_bundle, question);
        assert!(brief.claims.is_empty());
        assert!(!brief.usable);
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn excerpt_bounds_drop_unterminated_tail_fragments() {
        let safe = "The complete evidence sentence preserves its full relation.";
        let input = format!("{} {}", safe, "unfinished qualifier ".repeat(400));
        assert_eq!(bounded_source_excerpt(&input, 5_000), safe);
    }

    #[test]
    fn fabricated_number_fails_support_despite_high_token_overlap() {
        let evidence = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [{
                    "id":"TRIAL",
                    "title":"Trial report",
                    "url":"https://example.edu/trial",
                    "domain":"spoofed.gov",
                    "source_kind":"primary",
                    "quality":1.0,
                    "excerpt":"The clinical trial enrolled 12 adults and reported improved outcomes after treatment."
                }]
            }"#,
            "trial enrollment",
        )
        .unwrap();
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement:
                    "The clinical trial enrolled 120 adults and reported improved outcomes after treatment."
                        .into(),
                source_ids: vec!["TRIAL".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["The exact participant count requires source review.".into()],
            overall_confidence: 1.0,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);

        assert!(brief.claims.is_empty());
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn unsupported_negation_is_not_hidden_by_high_token_overlap() {
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement:
                    "The official specification does not define stable and inspectable behavior."
                        .into(),
                source_ids: vec!["S1".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec![
                "Whether the specification negates this behavior remains unknown.".into(),
            ],
            overall_confidence: 1.0,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), bundle(), synthesis);

        assert!(brief.claims.is_empty());
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn source_negation_cannot_be_omitted_from_a_supported_claim() {
        let evidence = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [{
                    "id":"S1",
                    "title":"Agency decision",
                    "url":"https://docs.example.org/agency-decision",
                    "excerpt":"The agency has never approved Project Orion for deployment."
                }]
            }"#,
            "agency decision",
        )
        .unwrap();
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement: "The agency approved Project Orion for deployment.".into(),
                source_ids: vec!["S1".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["Approval remains unsupported.".into()],
            overall_confidence: 1.0,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);
        assert!(brief.claims.is_empty());
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn outer_scope_negation_cannot_ground_an_embedded_positive_clause() {
        let evidence = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [{
                    "id":"S1",
                    "title":"Agency correction",
                    "url":"https://docs.example.org/agency-correction",
                    "excerpt":"It is not true that the agency approved Project Orion for deployment."
                }]
            }"#,
            "agency decision",
        )
        .unwrap();
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement: "The agency approved Project Orion for deployment.".into(),
                source_ids: vec!["S1".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["Approval remains unsupported.".into()],
            overall_confidence: 1.0,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);
        assert!(brief.claims.is_empty());
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn exact_negative_clause_is_supported_without_contaminating_the_next_sentence() {
        let evidence = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [{
                    "id":"S1",
                    "title":"Agency correction",
                    "url":"https://docs.example.org/agency-correction",
                    "excerpt":"It is not true that the agency approved Project Orion. The agency approved Project Atlas."
                }]
            }"#,
            "agency decisions",
        )
        .unwrap();
        let synthesis = ModelResearchSynthesis {
            claims: vec![
                ResearchClaim {
                    statement: "It is not true that the agency approved Project Orion.".into(),
                    source_ids: vec!["S1".into()],
                    confidence: 0.8,
                },
                ResearchClaim {
                    statement: "The agency approved Project Atlas.".into(),
                    source_ids: vec!["S1".into()],
                    confidence: 0.8,
                },
            ],
            contradictions: Vec::new(),
            unknowns: vec!["No other projects were described.".into()],
            overall_confidence: 0.8,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);
        assert_eq!(brief.claims.len(), 2);
    }

    #[test]
    fn short_actor_tokens_cannot_be_dropped_to_reverse_a_relation() {
        let evidence = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [{
                    "id":"S1",
                    "title":"Joint decision record",
                    "url":"https://docs.example.org/joint-decision",
                    "excerpt":"The US approved Project Orion while the EU rejected Project Orion."
                }]
            }"#,
            "joint decision",
        )
        .unwrap();
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement: "The EU approved Project Orion while the US rejected Project Orion."
                    .into(),
                source_ids: vec!["S1".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["The reversed assignment is unsupported.".into()],
            overall_confidence: 1.0,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);
        assert!(brief.claims.is_empty());
        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn exact_clause_grounding_preserves_operators_symbols_and_single_character_actors() {
        let source = ResearchSource {
            id: "S1".into(),
            title: "Semantic contract".into(),
            url: "https://docs.example.org/semantic-contract".into(),
            domain: "docs.example.org".into(),
            source_kind: "primary".into(),
            quality: 0.8,
            excerpt: concat!(
                "The experimental method may be unsafe under extreme load. ",
                "Treatment reduced fever or nausea in the monitored cohort. ",
                "Mortality fell and vaccination increased during the trial. ",
                "Group A received treatment while Group B received placebo. ",
                "The measured change was -5 percent after calibration. ",
                "Clinical risk was 5% after calibration. ",
                "The system can't support Project Orion under load. ",
                "!Enabled means the feature remains disabled. ",
                "The threshold was <=5% after calibration. ",
                "The rule requires A and (B or C). ",
                "The treatment is effective... not. ",
                "The experimental treatment is effective? ",
                "\"Project Orion is approved for deployment.\""
            )
            .into(),
        };

        assert!(source_supports_statement(
            "The experimental method may be unsafe under extreme load.",
            &source
        ));
        assert!(source_supports_statement(
            "!Enabled means the feature remains disabled.",
            &source
        ));
        assert!(
            source_supports_statement("\"Project Orion is approved for deployment.\"", &source),
            "clauses: {:?}",
            evidence_clause_sequences(&source.excerpt)
        );
        assert!(!source_supports_statement(
            "The experimental treatment is effective?",
            &source
        ));
        let quoted_denial = ResearchSource {
            excerpt:
                "The report quoted \"Project Orion is approved.\" but called the quotation false."
                    .into(),
            ..source.clone()
        };
        assert!(!source_supports_statement(
            "\"Project Orion is approved.\"",
            &quoted_denial
        ));
        let single_quoted_denial = ResearchSource {
            excerpt: "'Project Orion is approved.' was explicitly marked false.".into(),
            ..source.clone()
        };
        assert!(!source_supports_statement(
            "'Project Orion is approved.'",
            &single_quoted_denial
        ));
        let parenthetical_denial = ResearchSource {
            excerpt: "(Project Orion is approved.) was explicitly marked false.".into(),
            ..source.clone()
        };
        assert!(!source_supports_statement(
            "(Project Orion is approved.)",
            &parenthetical_denial
        ));
        for altered in [
            "The experimental method is unsafe under extreme load.",
            "Treatment reduced fever and nausea in the monitored cohort.",
            "Mortality fell because vaccination increased during the trial.",
            "Group B received treatment while Group A received placebo.",
            "The measured change was 5 percent after calibration.",
            "Clinical risk was 5 after calibration.",
            "The system can support Project Orion under load.",
            "Enabled means the feature remains disabled.",
            "The threshold was <5% after calibration.",
            "The rule requires (A and B) or C.",
            "The treatment is effective.",
            "Project Orion is approved for deployment.",
            "The experimental treatment is effective.",
        ] {
            assert!(
                !source_supports_statement(altered, &source),
                "altered clause was incorrectly grounded: {altered}"
            );
        }
    }

    #[test]
    fn mixed_polarities_are_not_a_bound_contradiction() {
        let evidence = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [
                    {"id":"S1","title":"First report","url":"https://docs.example.org/first","excerpt":"Project Alder passed verification while Project Birch failed verification."},
                    {"id":"S2","title":"Second report","url":"https://docs.example.org/second","excerpt":"Project Alder failed verification while Project Birch passed verification."}
                ]
            }"#,
            "verification reports",
        )
        .unwrap();
        let synthesis = ModelResearchSynthesis {
            claims: Vec::new(),
            contradictions: vec![ResearchContradiction {
                statement: "The reports disagree.".into(),
                source_ids: vec!["S1".into(), "S2".into()],
                positions: vec![
                    ResearchPosition {
                        source_id: "S1".into(),
                        statement: "Project Alder passed verification while Project Birch failed verification.".into(),
                    },
                    ResearchPosition {
                        source_id: "S2".into(),
                        statement: "Project Alder failed verification while Project Birch passed verification.".into(),
                    },
                ],
            }],
            unknowns: vec!["Each project needs its own normalized position.".into()],
            overall_confidence: 0.5,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);
        assert!(brief.contradictions.is_empty());
    }

    #[test]
    fn different_actors_do_not_form_a_contradiction_about_one_proposition() {
        let evidence = BrowserResearchBundle::from_json(
            r#"{
                "schema_version": 1,
                "sources": [
                    {"id":"S1","title":"Alice record","url":"https://docs.example.org/alice","excerpt":"Alice approved Project Orion after committee review."},
                    {"id":"S2","title":"Bob record","url":"https://docs.example.org/bob","excerpt":"Bob rejected Project Orion after committee review."}
                ]
            }"#,
            "committee decisions",
        )
        .unwrap();
        let synthesis = ModelResearchSynthesis {
            claims: Vec::new(),
            contradictions: vec![ResearchContradiction {
                statement: "The records disagree about Project Orion.".into(),
                source_ids: vec!["S1".into(), "S2".into()],
                positions: vec![
                    ResearchPosition {
                        source_id: "S1".into(),
                        statement: "Alice approved Project Orion after committee review.".into(),
                    },
                    ResearchPosition {
                        source_id: "S2".into(),
                        statement: "Bob rejected Project Orion after committee review.".into(),
                    },
                ],
            }],
            unknowns: vec!["The records describe decisions by different actors.".into()],
            overall_confidence: 0.5,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);
        assert!(brief.contradictions.is_empty());
    }

    #[test]
    fn numeric_contradictions_keep_context_numbers_bound_to_the_proposition() {
        let different_years = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "Acme revenue was 5 million in 2020.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "Acme revenue was 6 million in 2021.".into(),
            },
        ];
        assert!(!positions_show_opposition(&different_years));

        let same_context = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "Acme quarterly revenue was 5 million.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "Acme quarterly revenue was 6 million.".into(),
            },
        ];
        assert!(positions_show_opposition(&same_context));

        let different_year_only = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The audited record covers 2020.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The audited record covers 2021.".into(),
            },
        ];
        assert!(!positions_show_opposition(&different_year_only));

        let different_models = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "Model 5 passed verification.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "Model 6 passed verification.".into(),
            },
        ];
        assert!(!positions_show_opposition(&different_models));

        let compatible_bounds = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "Revenue exceeded 5 million.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "Revenue exceeded 6 million.".into(),
            },
        ];
        assert!(!positions_show_opposition(&compatible_bounds));

        let copular_bounds = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "Revenue was 5 million or more.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "Revenue was 6 million or more.".into(),
            },
        ];
        assert!(!positions_show_opposition(&copular_bounds));

        let higher_bounds = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "Score was 5 or higher.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "Score was 6 or higher.".into(),
            },
        ];
        assert!(!positions_show_opposition(&higher_bounds));

        let four_digit_counts = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The trial enrolled 2024 participants.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The trial enrolled 2025 participants.".into(),
            },
        ];
        assert!(positions_show_opposition(&four_digit_counts));
    }

    #[test]
    fn polarity_words_in_subject_descriptors_are_not_predicate_contradictions() {
        let different_cohorts = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The stable cohort improved after treatment.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The unstable cohort improved after treatment.".into(),
            },
        ];
        assert!(!positions_show_opposition(&different_cohorts));

        let same_subject = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The measured system behavior is stable after treatment.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The measured system behavior is unstable after treatment.".into(),
            },
        ];
        assert!(positions_show_opposition(&same_subject));

        let negated_descriptor = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The approved proposal advanced to review.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The not approved proposal advanced to review.".into(),
            },
        ];
        assert!(!positions_show_opposition(&negated_descriptor));

        let negated_predicate = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The proposal was approved after review.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The proposal was not approved after review.".into(),
            },
        ];
        assert!(positions_show_opposition(&negated_predicate));

        let epistemic_modal = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The treatment may be effective.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The treatment may not be effective.".into(),
            },
        ];
        assert!(!positions_show_opposition(&epistemic_modal));

        let temporal_descriptors = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The period before treatment lasted five days.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The period after treatment lasted five days.".into(),
            },
        ];
        assert!(!positions_show_opposition(&temporal_descriptors));

        let question_and_assertion = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The measured system behavior is stable?".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The measured system behavior is unstable.".into(),
            },
        ];
        assert!(!positions_show_opposition(&question_and_assertion));

        let two_questions = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "The measured system behavior is stable?".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "The measured system behavior is unstable?".into(),
            },
        ];
        assert!(!positions_show_opposition(&two_questions));

        let participial_descriptors = vec![
            ResearchPosition {
                source_id: "S1".into(),
                statement: "Agency-approved proposals passed review.".into(),
            },
            ResearchPosition {
                source_id: "S2".into(),
                statement: "Agency-rejected proposals passed review.".into(),
            },
        ];
        assert!(!positions_show_opposition(&participial_descriptors));
    }

    #[test]
    fn synthesis_boundary_rederives_provenance_for_manually_built_bundle() {
        let mut evidence = bundle();
        evidence.sources[0].domain = "forged.gov".into();
        evidence.sources[0].source_kind = "tertiary".into();
        evidence.sources[0].quality = 1.0;
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement: "The official specification defines a stable and inspectable behavior."
                    .into(),
                source_ids: vec!["S1".into()],
                confidence: 1.0,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["Long-term behavior remains unknown.".into()],
            overall_confidence: 1.0,
        };

        let brief = ResearchBrief::from_synthesis("1".into(), evidence, synthesis);

        assert_eq!(brief.sources[0].domain, "docs.example.com");
        assert_eq!(brief.sources[0].source_kind, "primary");
        assert_eq!(brief.sources[0].quality, 0.84);
        assert_eq!(brief.overall_confidence, 0.84);
    }

    #[test]
    fn best_fact_rechecks_excerpt_support_for_loaded_or_mutated_briefs() {
        let synthesis = ModelResearchSynthesis {
            claims: vec![ResearchClaim {
                statement: "The official specification defines a stable and inspectable behavior."
                    .into(),
                source_ids: vec!["S1".into()],
                confidence: 0.8,
            }],
            contradictions: Vec::new(),
            unknowns: vec!["Long-term behavior remains unknown.".into()],
            overall_confidence: 0.8,
        };
        let mut brief = ResearchBrief::from_synthesis("1".into(), bundle(), synthesis);
        assert!(brief.best_fact().is_some());

        brief.claims[0].statement =
            "Quantum teleportation was commercially deployed worldwide in 2042.".into();

        assert!(brief.best_fact().is_none());
    }

    #[test]
    fn failed_brief_preserves_sources_failure_and_explicit_unknown() {
        let brief = ResearchBrief::failed("1".into(), bundle(), "synthesizer unavailable");

        assert_eq!(brief.sources.len(), 2);
        assert!(brief.claims.is_empty());
        assert!(!brief.usable);
        assert_eq!(brief.failure.as_deref(), Some("synthesizer unavailable"));
        assert_eq!(brief.unknowns.len(), 1);
        assert!(brief.best_fact().is_none());
    }
}
