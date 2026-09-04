//! Typed Kraken skills. Generated text never selects executable paths or raw
//! shell commands; each skill owns a bounded input and verifier.

mod coding;
mod forge;
mod games;
mod model;
mod productivity;
mod research;

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contract::{ContractError, confined_path};

pub const MAX_SKILL_INPUT_CHARS: usize = 64 * 1024;
pub const MAX_REPORT_CHARS: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillSpec {
    pub name: &'static str,
    pub timeout_seconds: u64,
    pub max_children: u8,
    pub verifier: Option<&'static str>,
    pub uses_model: bool,
}

pub const SKILLS: [SkillSpec; 12] = [
    SkillSpec {
        name: "research_local",
        timeout_seconds: 240,
        max_children: 1,
        verifier: Some("research"),
        uses_model: true,
    },
    SkillSpec {
        name: "research_web",
        timeout_seconds: 300,
        max_children: 2,
        verifier: Some("research"),
        uses_model: true,
    },
    SkillSpec {
        name: "research_synth",
        timeout_seconds: 240,
        max_children: 1,
        verifier: Some("research"),
        uses_model: true,
    },
    SkillSpec {
        name: "research_fanout",
        timeout_seconds: 300,
        max_children: 5,
        verifier: Some("research"),
        uses_model: true,
    },
    SkillSpec {
        name: "coding_mcp",
        timeout_seconds: 120,
        max_children: 0,
        verifier: Some("coding_mcp"),
        uses_model: false,
    },
    SkillSpec {
        name: "code_forge",
        timeout_seconds: 1_500,
        max_children: 0,
        verifier: Some("code"),
        uses_model: true,
    },
    SkillSpec {
        name: "prod_digest",
        timeout_seconds: 300,
        max_children: 0,
        verifier: Some("digest"),
        uses_model: true,
    },
    SkillSpec {
        name: "prod_vault",
        timeout_seconds: 240,
        max_children: 0,
        verifier: Some("digest"),
        uses_model: true,
    },
    SkillSpec {
        name: "introspect",
        timeout_seconds: 240,
        max_children: 0,
        verifier: Some("introspect"),
        uses_model: true,
    },
    SkillSpec {
        name: "graduate_games",
        timeout_seconds: 600,
        max_children: 0,
        verifier: Some("game_graduation"),
        uses_model: false,
    },
    SkillSpec {
        name: "pixelfork",
        timeout_seconds: 300,
        max_children: 1,
        verifier: Some("pixelfork"),
        uses_model: false,
    },
    SkillSpec {
        name: "swarm_architect",
        timeout_seconds: 300,
        max_children: 5,
        verifier: None,
        uses_model: true,
    },
];

pub fn skill(name: &str) -> Option<&'static SkillSpec> {
    SKILLS.iter().find(|skill| skill.name == name)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChildRequest {
    pub skill: String,
    pub input: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SkillResult {
    pub summary: String,
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub children: Vec<ChildRequest>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

impl SkillResult {
    fn bounded(mut self) -> Self {
        self.summary = self.summary.chars().take(8_000).collect();
        self.children.truncate(5);
        for child in &mut self.children {
            child.skill = child.skill.chars().take(80).collect();
            child.input = child.input.chars().take(MAX_SKILL_INPUT_CHARS).collect();
        }
        self.evidence.truncate(64);
        for item in &mut self.evidence {
            *item = item.chars().take(4_000).collect();
        }
        self
    }
}

#[derive(Clone, Debug)]
pub struct SkillContext {
    pub root: PathBuf,
    pub workspace: PathBuf,
    pub workdir: PathBuf,
    pub vault: PathBuf,
    pub job_id: String,
    /// Bounded verifier/runner feedback from earlier attempts of this job.
    pub feedback: Vec<String>,
}

impl SkillContext {
    pub fn new(
        root: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        workdir: impl Into<PathBuf>,
        job_id: impl Into<String>,
    ) -> Result<Self, SkillError> {
        let root = root.into();
        let workspace = workspace.into();
        let workdir = workdir.into();
        let vault = root.join("vault");
        fs::create_dir_all(&workspace)?;
        fs::create_dir_all(&workdir)?;
        fs::create_dir_all(&vault)?;
        Ok(Self {
            root,
            workspace,
            workdir,
            vault,
            job_id: job_id.into(),
            feedback: Vec::new(),
        })
    }

    pub fn with_feedback(mut self, feedback: Vec<String>) -> Self {
        self.feedback = feedback
            .into_iter()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|item| item.chars().take(2_000).collect())
            .collect();
        self
    }

    pub fn workspace_path(&self, relative: &Path) -> Result<PathBuf, SkillError> {
        Ok(confined_path(&self.workspace, relative)?)
    }

    pub fn root_or_workspace_path(&self, raw: &Path) -> Result<PathBuf, SkillError> {
        if !raw.is_absolute() {
            return self.workspace_path(raw);
        }
        let canonical = fs::canonicalize(raw)?;
        let root = fs::canonicalize(&self.root)?;
        let workspace = fs::canonicalize(&self.workspace)?;
        if canonical.starts_with(root) || canonical.starts_with(workspace) {
            Ok(canonical)
        } else {
            Err(SkillError::UnsafePath(raw.to_path_buf()))
        }
    }

    pub fn write_vault(&self, suffix: &str, content: &str) -> Result<PathBuf, SkillError> {
        let safe_suffix: String = suffix
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                    character
                } else {
                    '-'
                }
            })
            .take(100)
            .collect();
        let path = self.vault.join(format!("{}-{safe_suffix}", self.job_id));
        fs::write(
            &path,
            content.chars().take(MAX_REPORT_CHARS).collect::<String>(),
        )?;
        Ok(path)
    }
}

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("unknown Kraken skill: {0}")]
    Unknown(String),
    #[error("skill input is empty")]
    Empty,
    #[error("skill I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("skill JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("skill HTTP failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("research failed: {0}")]
    Research(#[from] teledra_research::ResearchError),
    #[error("workspace contract failed: {0}")]
    Contract(#[from] ContractError),
    #[error("path is outside Kraken root/workspace: {0}")]
    UnsafePath(PathBuf),
    #[error("model failed: {0}")]
    Model(String),
    #[error("verification failed: {0}")]
    Verification(String),
    #[error("unsupported request: {0}")]
    Unsupported(String),
}

pub fn execute(name: &str, input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let input: String = input.trim().chars().take(MAX_SKILL_INPUT_CHARS).collect();
    if input.is_empty() {
        return Err(SkillError::Empty);
    }
    let result = match name {
        "research_local" => research::local(&input, context),
        "research_web" => research::web(&input, context),
        "research_synth" => research::synthesize(&input, context),
        "research_fanout" => research::fanout(&input, context),
        "coding_mcp" => coding::execute(&input, context),
        "code_forge" => forge::execute(&input, context),
        "prod_digest" => productivity::digest(&input, context),
        "prod_vault" => productivity::vault(&input, context),
        "introspect" => productivity::introspect(&input, context),
        "graduate_games" => games::graduate(&input, context),
        "pixelfork" => games::pixelfork(&input, context),
        "swarm_architect" => research::architect(&input, context),
        _ => Err(SkillError::Unknown(name.to_owned())),
    }?;
    Ok(result.bounded())
}

pub fn attempt_limit(name: &str, input: &str) -> u8 {
    if name == "code_forge" {
        forge::attempt_limit(input)
    } else {
        crate::queue::MAX_ATTEMPTS
    }
}

/// Enforce the declared mechanical verifier before a worker can mark a job
/// done. Skills may perform stronger checks internally; this boundary prevents
/// an implementation from accidentally becoming trust-on-return.
pub fn verify_result(name: &str, result: &SkillResult) -> Result<(), SkillError> {
    let spec = skill(name).ok_or_else(|| SkillError::Unknown(name.to_owned()))?;
    if let Some(path) = &result.output {
        let metadata = fs::metadata(path).map_err(|_| {
            SkillError::Verification(format!(
                "declared output does not exist: {}",
                path.display()
            ))
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(SkillError::Verification(format!(
                "declared output is empty: {}",
                path.display()
            )));
        }
    }
    match spec.verifier {
        Some("research") if result.output.is_some() => verify_research_result(result),
        Some("digest") => {
            if !result.summary.contains("## ")
                || (result.evidence.is_empty()
                    && !result.summary.contains("No digestible content found"))
            {
                return Err(SkillError::Verification(
                    "digest output needs structured sections and source evidence".into(),
                ));
            }
            Ok(())
        }
        Some("coding_mcp" | "code" | "introspect" | "game_graduation" | "pixelfork") => {
            if result.summary.trim().is_empty() {
                Err(SkillError::Verification(
                    "verified result has no summary".into(),
                ))
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

fn verify_research_result(result: &SkillResult) -> Result<(), SkillError> {
    if result.evidence.is_empty() {
        return Err(SkillError::Verification(
            "research report has no source evidence".into(),
        ));
    }
    let labels = result
        .evidence
        .iter()
        .filter_map(|item| {
            item.strip_prefix('[')?
                .split_once(']')
                .map(|(label, _)| label)
        })
        .collect::<std::collections::BTreeSet<_>>();
    research_citation_contract(&result.summary, &labels).map_err(SkillError::Verification)
}

fn research_citation_contract(
    text: &str,
    labels: &std::collections::BTreeSet<&str>,
) -> Result<(), String> {
    let citations = research_citations(text);
    if citations.is_empty() {
        return Err("research report contains no [L#]/[S#] citations".into());
    }
    if let Some(unknown) = citations
        .iter()
        .find(|citation| !labels.contains(citation.as_str()))
    {
        return Err(format!("research report cites unknown source [{unknown}]"));
    }
    let mut paragraph = String::new();
    let verify_unit = |unit: &str| {
        let unit = unit.trim();
        if unit.is_empty() || unit.starts_with('#') || !research_citations(unit).is_empty() {
            Ok(())
        } else {
            Err(format!(
                "research claim lacks a source citation: {}",
                unit.chars().take(160).collect::<String>()
            ))
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("- ") {
            verify_unit(&paragraph)?;
            paragraph.clear();
            if trimmed.starts_with("- ") {
                verify_unit(trimmed)?;
            }
        } else {
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(trimmed);
        }
    }
    verify_unit(&paragraph)
}

fn research_citations(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut citations = Vec::new();
    let mut index = 0;
    while index + 3 < bytes.len() {
        if bytes[index] == b'[' && matches!(bytes[index + 1], b'L' | b'S') {
            let start = index + 1;
            let mut end = index + 2;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > index + 2 && end < bytes.len() && bytes[end] == b']' {
                citations.push(text[start..end].to_owned());
                index = end;
            }
        }
        index += 1;
    }
    citations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_operator_skills_have_typed_specs() {
        for name in [
            "research_local",
            "research_web",
            "research_synth",
            "research_fanout",
            "coding_mcp",
            "code_forge",
            "prod_digest",
            "prod_vault",
            "introspect",
            "graduate_games",
        ] {
            assert!(skill(name).is_some(), "missing skill {name}");
        }
    }

    #[test]
    fn uncited_research_can_never_pass_the_worker_boundary() {
        let result = SkillResult {
            summary: "The queue has a depth limit.".into(),
            output: Some(std::env::current_exe().unwrap()),
            children: Vec::new(),
            evidence: vec!["[L1] queue.rs".into()],
        };
        assert!(verify_result("research_local", &result).is_err());
    }

    #[test]
    fn one_citation_cannot_shield_a_second_uncited_claim() {
        let result = SkillResult {
            summary: "The queue has a depth limit [L1].\n\nRetries are also bounded.".into(),
            output: Some(std::env::current_exe().unwrap()),
            children: Vec::new(),
            evidence: vec!["[L1] queue.rs".into()],
        };
        assert!(verify_result("research_local", &result).is_err());
    }
}
