use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::Value;
use walkdir::WalkDir;

use super::{SkillContext, SkillError, SkillResult, model};

const ELIGIBLE: [&str; 6] = ["md", "txt", "jsonl", "json", "log", "toml"];

pub fn digest(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let (raw_path, maximum) = parse_digest_input(input);
    let folder = context.root_or_workspace_path(Path::new(raw_path))?;
    if !folder.is_dir() {
        return Err(SkillError::Unsupported(format!(
            "digest input is not a folder: {}",
            folder.display()
        )));
    }
    let mut files = fs::read_dir(&folder)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && eligible(path))
        .collect::<Vec<_>>();
    files.sort();
    files.truncate(maximum);

    let mut source_lines = Vec::new();
    let mut excerpts = String::new();
    for path in files {
        let bytes = fs::read(&path)?;
        if bytes.is_empty() || bytes.iter().take(1_024).any(|byte| *byte == 0) {
            continue;
        }
        let text = bounded_ends(&String::from_utf8_lossy(&bytes), 8_000);
        source_lines.push(format!("- `{}` ({} bytes)", path.display(), bytes.len()));
        excerpts.push_str(&format!("\n=== {} ===\n{text}\n", path.display()));
    }
    let body = if excerpts.is_empty() {
        "## Summary\n\nNo digestible content found.\n\n## Themes\n\n- None\n\n## Action Items\n\n- None".into()
    } else {
        model::complete(
            "Create a factual, terse productivity brief. Use exactly: ## Summary, ## Themes, ## Action Items. Do not invent facts.",
            &excerpts,
            0.1,
        )?
    };
    let sources = if source_lines.is_empty() {
        "(none)".into()
    } else {
        source_lines.join("\n")
    };
    let report = format!(
        "# Daily Brief — {}\n\n{body}\n\n## Sources\n\n{sources}\n",
        folder.display()
    );
    let output = context.write_vault("digest.md", &report)?;
    Ok(SkillResult {
        summary: report,
        output: Some(output),
        children: Vec::new(),
        evidence: source_lines,
    })
}

pub fn vault(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let raw = Path::new(input.trim().trim_matches('"'));
    let source = if raw.is_absolute() {
        fs::canonicalize(raw)?
    } else {
        fs::canonicalize(context.root.join(raw))?
    };
    let vault = fs::canonicalize(&context.vault)?;
    if !source.starts_with(&vault) || !source.is_file() {
        return Err(SkillError::UnsafePath(source));
    }
    let text = fs::read_to_string(&source)?;
    let body = model::complete(
        "Distill the report into an evergreen note using exactly: ## Evergreen, ## Key Facts, ## References. Preserve uncertainty and never invent facts.",
        &bounded_ends(&text, 24_000),
        0.1,
    )?;
    let report = format!(
        "# Evergreen — {}\n\n{body}\n\n## Source\n\n- `{}`\n",
        source.file_stem().unwrap_or_default().to_string_lossy(),
        source.display()
    );
    let output = context.write_vault("evergreen.md", &report)?;
    Ok(SkillResult {
        summary: report,
        output: Some(output),
        children: Vec::new(),
        evidence: vec![source.display().to_string()],
    })
}

pub fn introspect(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let focus = input.trim();
    let journal = context.root.join("journal");
    let mut outcomes: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    if journal.is_dir() {
        for entry in WalkDir::new(&journal)
            .max_depth(1)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !path.is_file()
                || path
                    .extension()
                    .is_none_or(|extension| !extension.eq_ignore_ascii_case("jsonl"))
            {
                continue;
            }
            for line in fs::read_to_string(path).unwrap_or_default().lines() {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let skill = value
                    .get("skill")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                if focus != "all" && !focus.is_empty() && skill != focus {
                    continue;
                }
                let verdict = value
                    .get("verdict")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("status").and_then(Value::as_str))
                    .unwrap_or("unknown");
                *outcomes
                    .entry(skill.into())
                    .or_default()
                    .entry(verdict.into())
                    .or_default() += 1;
                total += 1;
                if let Some(items) = value.get("reasons").and_then(Value::as_array) {
                    for reason in items.iter().filter_map(Value::as_str) {
                        *reasons.entry(normalize_reason(reason)).or_default() += 1;
                    }
                }
            }
        }
    }
    let evidence =
        format!("Verdicts: {total}\nOutcomes: {outcomes:#?}\nFailure signatures: {reasons:#?}");
    let backlog = if total == 0 || !model::available() {
        "1. Run representative production jobs to establish measured pass/fail history.\n2. Keep task contracts immutable and verifier evidence durable.\n3. Review repeated failures only after evidence exists.".into()
    } else {
        model::complete(
            "From only this operational evidence, produce a ranked improvement backlog of at most five concrete items. Do not invent files or failures.",
            &evidence,
            0.1,
        )?
    };
    let report = format!(
        "# Introspection — {focus}\n\n## Improvement backlog (ranked)\n\n{backlog}\n\n## Evidence\n\n```text\n{evidence}\n```\n"
    );
    let output = context.write_vault("introspection.md", &report)?;
    Ok(SkillResult {
        summary: report,
        output: Some(output),
        children: Vec::new(),
        evidence: vec![format!("{total} journal verdicts")],
    })
}

fn parse_digest_input(input: &str) -> (&str, usize) {
    let mut parts = input.split_whitespace();
    let path = parts.next().unwrap_or(".");
    let maximum = parts
        .find_map(|part| part.strip_prefix("max_files="))
        .and_then(|number| number.parse().ok())
        .unwrap_or(12usize)
        .clamp(1, 100);
    (path, maximum)
}

fn eligible(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        ELIGIBLE
            .iter()
            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
    })
}

fn bounded_ends(text: &str, maximum: usize) -> String {
    if text.chars().count() <= maximum {
        return text.into();
    }
    let half = maximum / 2;
    let start: String = text.chars().take(half).collect();
    let end: String = text
        .chars()
        .rev()
        .take(half)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{start}\n\n[... truncated ...]\n\n{end}")
}

fn normalize_reason(reason: &str) -> String {
    reason
        .lines()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .chars()
        .take(120)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_text_preserves_both_ends() {
        let text = "a".repeat(100) + &"z".repeat(100);
        let bounded = bounded_ends(&text, 40);
        assert!(bounded.starts_with("aaaaaaaa"));
        assert!(bounded.ends_with("zzzzzzzz"));
    }
}
