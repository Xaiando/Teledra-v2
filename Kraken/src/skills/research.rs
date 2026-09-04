use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use walkdir::{DirEntry, WalkDir};

use super::{ChildRequest, SkillContext, SkillError, SkillResult, model, skill};

const MAX_LOCAL_FILES: usize = 600;
const MAX_SOURCE_CHARS: usize = 24_000;

pub fn local(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let terms = query_terms(input);
    let mut candidates = Vec::new();
    let mut roots = vec![context.root.clone(), context.workspace.clone()];
    if let Some(extra) = std::env::var_os("TELEDRA_KNOWLEDGE_DIR") {
        roots.push(PathBuf::from(extra));
    }
    roots.sort();
    roots.dedup();

    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(wanted_entry)
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .take(MAX_LOCAL_FILES)
        {
            let path = entry.into_path();
            let Some(text) = read_bounded(&path) else {
                continue;
            };
            let haystack = format!(
                "{} {}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                text
            )
            .to_ascii_lowercase();
            let score = terms
                .iter()
                .filter(|term| haystack.contains(term.as_str()))
                .count();
            if score > 0 {
                candidates.push((score, path, text));
            }
        }
    }
    let minimum_score = if terms.len() >= 4 { 2 } else { 1 };
    candidates.retain(|(score, _, _)| *score >= minimum_score);
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    candidates.truncate(12);
    if candidates.is_empty() {
        return Ok(SkillResult {
            summary: "No useful local evidence was found; queued a bounded web-research fallback."
                .into(),
            output: None,
            children: vec![ChildRequest {
                skill: "research_web".into(),
                input: input.into(),
            }],
            evidence: Vec::new(),
        });
    }

    let mut evidence = Vec::new();
    let mut material = String::new();
    for (index, (_, path, text)) in candidates.iter().enumerate() {
        let label = format!("L{}", index + 1);
        evidence.push(format!("[{label}] {}", path.display()));
        material.push_str(&format!("\n[{label}] {}\n{}\n", path.display(), text));
    }
    let prompt = format!(
        "Question: {input}\n\nLocal evidence:{material}\n\nAnswer only from this evidence. Cite every material claim with [L#]. State uncertainty plainly."
    );
    let report = complete_grounded(
        "You are Kraken's local research synthesizer. Do not invent sources, facts, or citations.",
        &prompt,
        &evidence,
    )?;
    let output = context.write_vault("research-local.md", &report)?;
    Ok(SkillResult {
        summary: report,
        output: Some(output),
        children: Vec::new(),
        evidence,
    })
}

pub fn web(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let archive = context.root.join("journal").join("research.jsonl");
    let brief = teledra_research::ResearchService::new(archive)?.research(input)?;
    let evidence = brief
        .sources
        .iter()
        .filter(|source| source.source_kind != "search_index")
        .map(|source| format!("[{}] {} — {}", source.id, source.title, source.url))
        .collect::<Vec<_>>();
    let direct_url = input.starts_with("https://") || input.starts_with("http://");
    if evidence.len() < if direct_url { 1 } else { 3 } {
        return Err(SkillError::Verification(format!(
            "web research found only {} fetched source(s); {} required",
            evidence.len(),
            if direct_url { 1 } else { 3 }
        )));
    }
    let labelled_sources = brief
        .sources
        .iter()
        .filter(|source| source.source_kind != "search_index")
        .map(|source| {
            format!(
                "[{}] {} — {}\n{}",
                source.id, source.title, source.url, source.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let prompt = format!(
        "{}\n\nCITATION-LABELLED SOURCES:\n{}\n\nWrite a concise answer to the research query. Cite sources with [S#]. Search-index snippets are leads, not verified facts; state conflicts and uncertainty.",
        brief.prompt_context(),
        labelled_sources
    );
    let report = complete_grounded(
        "You are Kraken's evidence-bound public-web researcher. Never fabricate citations.",
        &prompt,
        &evidence,
    )?;
    let output = context.write_vault("research-web.md", &report)?;
    Ok(SkillResult {
        summary: report,
        output: Some(output),
        children: Vec::new(),
        evidence,
    })
}

#[derive(Deserialize)]
struct SynthesisRequest {
    question: String,
    sources: Vec<SynthesisSource>,
}

#[derive(Deserialize)]
struct SynthesisSource {
    path: PathBuf,
    #[serde(default)]
    title: String,
}

pub fn synthesize(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let Ok(request) = serde_json::from_str::<SynthesisRequest>(input) else {
        return web(input, context);
    };
    if request.question.trim().is_empty() || request.sources.is_empty() {
        return Err(SkillError::Unsupported(
            "research_synth requires a question and sources".into(),
        ));
    }
    let mut material = String::new();
    let mut evidence = Vec::new();
    for (index, source) in request.sources.iter().take(16).enumerate() {
        let path = context.root_or_workspace_path(&source.path)?;
        let text = read_bounded(&path).ok_or_else(|| {
            SkillError::Unsupported(format!("unsupported source: {}", path.display()))
        })?;
        let label = format!("S{}", index + 1);
        let title = if source.title.trim().is_empty() {
            path.display().to_string()
        } else {
            source.title.clone()
        };
        evidence.push(format!("[{label}] {title} — {}", path.display()));
        material.push_str(&format!("\n[{label}] {title}\n{text}\n"));
    }
    let report = complete_grounded(
        "Synthesize only the supplied sources. Cite material claims with [S#] and identify disagreements.",
        &format!("Question: {}\nSources:{material}", request.question),
        &evidence,
    )?;
    let output = context.write_vault("research-synthesis.md", &report)?;
    Ok(SkillResult {
        summary: report,
        output: Some(output),
        children: Vec::new(),
        evidence,
    })
}

pub fn fanout(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    if input.starts_with("SYNTH:") {
        return Err(SkillError::Unsupported(
            "fanout join jobs are assembled by the runner from completed child outputs".into(),
        ));
    }
    let prompt = format!(
        "Break this research question into 2 to 4 independent, non-overlapping subquestions. Return only a JSON array of strings.\n\n{input}"
    );
    let raw = model::complete("You design bounded research plans.", &prompt, 0.1)?;
    let questions = recover_string_array(&raw).unwrap_or_else(|| deterministic_questions(input));
    let mut children = questions
        .into_iter()
        .take(4)
        .map(|question| ChildRequest {
            skill: "research_local".into(),
            input: question,
        })
        .collect::<Vec<_>>();
    children.push(ChildRequest {
        skill: "research_fanout".into(),
        input: format!("SYNTH:{}", context.job_id),
    });
    Ok(SkillResult {
        summary: format!(
            "Planned {} bounded research branches plus a synthesis join.",
            children.len() - 1
        ),
        output: None,
        children,
        evidence: Vec::new(),
    })
}

#[derive(Deserialize)]
struct ArchitectureItem {
    skill: String,
    input: String,
}

pub fn architect(input: &str, _context: &SkillContext) -> Result<SkillResult, SkillError> {
    let raw = model::complete(
        "Plan a bounded Kraken workflow using only registered skill names. Return only JSON: [{\"skill\":\"...\",\"input\":\"...\"}]. Maximum five items.",
        input,
        0.1,
    )?;
    let json = recover_json(&raw)
        .ok_or_else(|| SkillError::Model("architect returned no JSON array".into()))?;
    let items: Vec<ArchitectureItem> = serde_json::from_str(json)?;
    let mut children = Vec::new();
    for item in items.into_iter().take(5) {
        let spec = skill(&item.skill).ok_or_else(|| SkillError::Unknown(item.skill.clone()))?;
        if spec.name == "swarm_architect" {
            return Err(SkillError::Unsupported(
                "architect cannot recursively schedule itself".into(),
            ));
        }
        children.push(ChildRequest {
            skill: item.skill,
            input: item.input,
        });
    }
    if children.is_empty() {
        return Err(SkillError::Unsupported("architect produced no jobs".into()));
    }
    Ok(SkillResult {
        summary: format!("Validated {} typed child jobs.", children.len()),
        output: None,
        children,
        evidence: Vec::new(),
    })
}

fn wanted_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
    !matches!(
        name.as_str(),
        ".git" | "target" | ".venv" | "node_modules" | "jobs" | "vault" | "assets" | "output"
    )
}

fn complete_grounded(
    system: &str,
    prompt: &str,
    evidence: &[String],
) -> Result<String, SkillError> {
    let first = model::complete(system, prompt, 0.1)?;
    if citations_are_valid(&first, evidence) {
        return Ok(first);
    }
    let labels = evidence
        .iter()
        .filter_map(|item| item.split_once(']').map(|(label, _)| format!("{label}]")))
        .collect::<Vec<_>>()
        .join(", ");
    let correction = format!(
        "Your draft failed the citation gate. Rewrite the complete answer. Every factual paragraph or bullet must end with one or more of these exact source labels: {labels}. Do not create any other labels. If the evidence does not support a claim, omit it or mark it uncertain.\n\nFAILED DRAFT:\n{first}\n\nORIGINAL QUESTION AND EVIDENCE:\n{prompt}"
    );
    let revised = model::complete(system, &correction, 0.0)?;
    if citations_are_valid(&revised, evidence) {
        Ok(revised)
    } else {
        Ok(deterministic_grounding(prompt, evidence))
    }
}

fn deterministic_grounding(prompt: &str, evidence: &[String]) -> String {
    let query = prompt
        .lines()
        .find_map(|line| {
            line.strip_prefix("Question: ")
                .or_else(|| line.strip_prefix("Research query: "))
        })
        .unwrap_or_default();
    let terms = query_terms(query);
    let prefer_constants = terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "constant"
                | "constants"
                | "exact"
                | "maximum"
                | "limit"
                | "limits"
                | "value"
                | "values"
        ) || term.starts_with("max_")
    });
    let identifier_terms = terms
        .iter()
        .filter(|term| term.contains('_'))
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for (evidence_index, item) in evidence.iter().take(12).enumerate() {
        let Some((raw_label, _)) = item.strip_prefix('[').and_then(|item| item.split_once(']'))
        else {
            continue;
        };
        let label = format!("[{raw_label}]");
        // Match the complete generated evidence header rather than the short
        // label. Source code and tests can legitimately contain strings such
        // as `[L1]`; treating those as boundaries selects the wrong source.
        let header = format!("\n{item}\n");
        let Some(start) = prompt.find(&header).map(|start| start + header.len()) else {
            continue;
        };
        let end = evidence
            .iter()
            .skip(evidence_index + 1)
            .filter_map(|next| prompt[start..].find(&format!("\n{next}\n")))
            .min()
            .map_or(prompt.len(), |offset| start + offset);
        let mut lines = prompt[start..end]
            .lines()
            .enumerate()
            .map(|(index, line)| (index, line.trim()))
            .filter(|(_, line)| !line.is_empty())
            .map(|(index, line)| {
                let lower = line.to_ascii_lowercase();
                let mut score = terms
                    .iter()
                    .filter(|term| lower.contains(term.as_str()))
                    .count();
                let identifier_match = identifier_terms
                    .iter()
                    .any(|term| lower.contains(term.as_str()));
                let declaration = lower.starts_with("const ") || lower.starts_with("pub const ");
                if prefer_constants
                    && declaration
                    && lower.chars().any(|character| character.is_ascii_digit())
                    && (identifier_terms.is_empty() && score > 0 || identifier_match)
                {
                    score += 4;
                }
                (score, index, line)
            })
            .collect::<Vec<_>>();
        lines.sort_by_key(|item| (std::cmp::Reverse(item.0), item.1));
        let best_score = lines.first().map_or(0, |line| line.0);
        let excerpt = lines
            .into_iter()
            .filter(|line| line.0 > 0 || terms.is_empty())
            .take(5)
            .map(|(_, _, line)| line)
            .collect::<Vec<_>>()
            .join(" ");
        if !excerpt.is_empty() {
            candidates.push((best_score, evidence_index, label, excerpt));
        }
    }
    candidates.sort_by_key(|candidate| (std::cmp::Reverse(candidate.0), candidate.1));
    let minimum_score = candidates.first().map_or(1, |candidate| candidate.0.max(1));

    let mut report = "## Evidence-bound fallback\n".to_owned();
    for (_, _, label, excerpt) in candidates
        .into_iter()
        .filter(|candidate| candidate.0 >= minimum_score)
        .take(6)
    {
        report.push_str(&format!(
            "\n- {} {label}",
            excerpt.chars().take(700).collect::<String>()
        ));
    }
    report.push('\n');
    report
}

fn citations_are_valid(report: &str, evidence: &[String]) -> bool {
    let labels = evidence
        .iter()
        .filter_map(|item| {
            item.strip_prefix('[')?
                .split_once(']')
                .map(|(label, _)| label)
        })
        .collect::<BTreeSet<_>>();
    super::research_citation_contract(report, &labels).is_ok()
}

fn read_bounded(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "md" | "txt" | "json" | "jsonl" | "rs" | "py" | "toml" | "yaml" | "yml"
    ) {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.len() > 2 * 1024 * 1024 || bytes.iter().take(1024).any(|byte| *byte == 0) {
        return None;
    }
    Some(
        String::from_utf8_lossy(&bytes)
            .chars()
            .take(MAX_SOURCE_CHARS)
            .collect(),
    )
}

fn query_terms(input: &str) -> Vec<String> {
    let stop = [
        "about", "could", "from", "have", "into", "that", "their", "there", "these", "this",
        "what", "when", "where", "which", "with", "would",
    ];
    input
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() >= 4 && !stop.contains(&term.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn recover_json(raw: &str) -> Option<&str> {
    let start = raw.find('[')?;
    let end = raw.rfind(']')?;
    (end >= start).then_some(&raw[start..=end])
}

fn recover_string_array(raw: &str) -> Option<Vec<String>> {
    let values: Vec<String> = serde_json::from_str(recover_json(raw)?).ok()?;
    let values = values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn deterministic_questions(input: &str) -> Vec<String> {
    vec![
        format!("What are the primary facts and current state of: {input}"),
        format!("What risks, constraints, and counterevidence apply to: {input}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_json_array_from_fenced_response() {
        assert_eq!(
            recover_string_array("```json\n[\"one\",\"two\"]\n```").unwrap(),
            ["one", "two"]
        );
    }

    #[test]
    fn terms_are_normalized_and_stop_words_removed() {
        assert_eq!(
            query_terms("What is THIS Kraken queue contract?"),
            ["contract", "kraken", "queue"]
        );
    }

    #[test]
    fn citation_gate_rejects_missing_and_invented_labels() {
        let evidence = vec!["[L1] queue.rs".to_owned()];
        assert!(!citations_are_valid("Depth is bounded.", &evidence));
        assert!(!citations_are_valid("Depth is bounded [L2].", &evidence));
        assert!(citations_are_valid("Depth is bounded [L1].", &evidence));
    }

    #[test]
    fn deterministic_fallback_keeps_exact_source_labels() {
        let report = deterministic_grounding(
            "Question: What are the depth limit and retry count?\n\n[L1] queue.rs\nMAX_DEPTH is three.\n[L2] runner.rs\nJobs retry twice.",
            &["[L1] queue.rs".into(), "[L2] runner.rs".into()],
        );
        assert!(report.contains("MAX_DEPTH is three. [L1]"));
        assert!(report.contains("Jobs retry twice. [L2]"));
        assert!(citations_are_valid(
            &report,
            &["[L1] queue.rs".into(), "[L2] runner.rs".into()]
        ));
    }

    #[test]
    fn deterministic_fallback_does_not_confuse_labels_inside_sources() {
        let report = deterministic_grounding(
            "Question: What exact values do MAX_DEPTH and MAX_CHILDREN use?\n\n[L1] research.rs\nA test mentions [L2] but has no answer.\n\n[L2] queue.rs\npub const MAX_DEPTH: u8 = 3;\npub const MAX_CHILDREN: usize = 5;",
            &["[L1] research.rs".into(), "[L2] queue.rs".into()],
        );
        assert!(report.contains("pub const MAX_DEPTH: u8 = 3;"));
        assert!(report.contains("pub const MAX_CHILDREN: usize = 5;"));
        assert!(!report.contains("has no answer"));
        assert!(report.contains("[L2]"));
        assert!(citations_are_valid(
            &report,
            &["[L1] research.rs".into(), "[L2] queue.rs".into()]
        ));
    }
}
