use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::RegexBuilder;
use serde::Deserialize;
use walkdir::WalkDir;

use super::{SkillContext, SkillError, SkillResult};

const MAX_READ_BYTES: usize = 80_000;
const MAX_FILES: usize = 1_000;
const MAX_MATCHES: usize = 120;

#[derive(Deserialize)]
struct Request {
    op: String,
    #[serde(default = "dot")]
    path: PathBuf,
    #[serde(default)]
    pattern: String,
    #[serde(default)]
    max_files: Option<usize>,
}

fn dot() -> PathBuf {
    PathBuf::from(".")
}

pub fn execute(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let request = parse(input)?;
    let path = context.root_or_workspace_path(&request.path)?;
    let report = match request.op.as_str() {
        "tree" => tree(&path, request.max_files.unwrap_or(80).clamp(1, MAX_FILES))?,
        "read" => read(&path)?,
        "search" => search(&path, &request.pattern)?,
        "py_compile" => py_compile(&path)?,
        "run_tests" => run_tests(&path)?,
        "git_status" => command_report(&path, "git", &["status", "--short"], 20)?,
        operation => {
            return Err(SkillError::Unsupported(format!(
                "unknown coding_mcp op: {operation}"
            )));
        }
    };
    let output = context.write_vault("coding-mcp.md", &report)?;
    Ok(SkillResult {
        summary: report,
        output: Some(output),
        children: Vec::new(),
        evidence: vec![path.display().to_string()],
    })
}

fn parse(input: &str) -> Result<Request, SkillError> {
    if input.trim_start().starts_with('{') {
        return Ok(serde_json::from_str(input)?);
    }
    let lower = input.to_ascii_lowercase();
    let op = if lower.contains("git") && lower.contains("status") {
        "git_status"
    } else if lower.contains("test") {
        "run_tests"
    } else if lower.contains("compile") {
        "py_compile"
    } else if lower.contains("search") {
        "search"
    } else {
        "tree"
    };
    let pattern = input
        .split_once("search")
        .map(|(_, pattern)| pattern.trim().to_owned())
        .unwrap_or_default();
    Ok(Request {
        op: op.into(),
        path: dot(),
        pattern,
        max_files: None,
    })
}

fn files(path: &Path, maximum: usize) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_string_lossy().as_ref(),
                ".git" | "target" | "node_modules" | ".venv" | "__pycache__"
            )
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .take(maximum)
        .map(|entry| entry.into_path())
        .collect()
}

fn tree(path: &Path, maximum: usize) -> Result<String, SkillError> {
    if !path.is_dir() {
        return Err(SkillError::Unsupported(format!(
            "not a directory: {}",
            path.display()
        )));
    }
    let found = files(path, maximum);
    let mut report = format!("# Tree: {}\n\n", path.display());
    for file in &found {
        report.push_str(&format!("- {}\n", file.display()));
    }
    if found.len() == maximum {
        report.push_str(&format!("- ... capped at {maximum} files\n"));
    }
    Ok(report)
}

fn read(path: &Path) -> Result<String, SkillError> {
    if !path.is_file() {
        return Err(SkillError::Unsupported(format!(
            "not a file: {}",
            path.display()
        )));
    }
    let bytes = fs::read(path)?;
    let truncated = bytes.len() > MAX_READ_BYTES;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_READ_BYTES)]);
    Ok(format!(
        "# Read: {}\n\n```text\n{text}\n```{}",
        path.display(),
        if truncated {
            "\n\n(truncated at 80000 bytes)"
        } else {
            ""
        }
    ))
}

fn search(path: &Path, pattern: &str) -> Result<String, SkillError> {
    if pattern.is_empty() || pattern.len() > 200 {
        return Err(SkillError::Unsupported(
            "search pattern must contain 1-200 characters".into(),
        ));
    }
    if ["++", "**", "??", "{,"]
        .iter()
        .any(|marker| pattern.contains(marker))
    {
        return Err(SkillError::Unsupported(
            "search pattern rejected as unsafe".into(),
        ));
    }
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(256 * 1024)
        .build()
        .map_err(|error| SkillError::Unsupported(error.to_string()))?;
    let mut report = format!("# Search: {pattern}\n\n");
    let mut count = 0;
    'files: for file in files(path, MAX_FILES) {
        let Ok(bytes) = fs::read(&file) else { continue };
        if bytes.len() > 2 * 1024 * 1024 || bytes.iter().take(1_024).any(|byte| *byte == 0) {
            continue;
        }
        for (line, text) in String::from_utf8_lossy(&bytes).lines().enumerate() {
            if regex.is_match(text) {
                report.push_str(&format!(
                    "- {}:{}: {}\n",
                    file.display(),
                    line + 1,
                    text.chars().take(240).collect::<String>()
                ));
                count += 1;
                if count >= MAX_MATCHES {
                    break 'files;
                }
            }
        }
    }
    if count == 0 {
        report.push_str("(no matches)\n");
    }
    Ok(report)
}

fn py_compile(path: &Path) -> Result<String, SkillError> {
    let python = std::env::var("TELEDRA_PYTHON").unwrap_or_else(|_| "python".into());
    let found = files(path, MAX_FILES)
        .into_iter()
        .filter(|file| {
            file.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
        })
        .collect::<Vec<_>>();
    if found.is_empty() {
        return Ok(format!(
            "# py_compile: {}\n\nNo Python files found.",
            path.display()
        ));
    }
    let mut failures = Vec::new();
    for file in &found {
        let output = Command::new(&python)
            .args(["-m", "py_compile"])
            .arg(file)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()?;
        if !output.status.success() {
            failures.push(format!(
                "{}: {}",
                file.display(),
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(2_000)
                    .collect::<String>()
            ));
        }
    }
    if failures.is_empty() {
        Ok(format!(
            "# py_compile: {}\n\nOK ({} Python files)",
            path.display(),
            found.len()
        ))
    } else {
        Err(SkillError::Verification(failures.join("\n")))
    }
}

fn run_tests(path: &Path) -> Result<String, SkillError> {
    let directory = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    if directory.join("Cargo.toml").is_file() {
        command_report(directory, "cargo", &["test", "--all-targets"], 600)
    } else {
        let python = std::env::var("TELEDRA_PYTHON").unwrap_or_else(|_| "python".into());
        command_report(directory, &python, &["-m", "unittest", "discover"], 120)
    }
}

fn command_report(
    path: &Path,
    program: &str,
    arguments: &[&str],
    timeout: u64,
) -> Result<String, SkillError> {
    let directory = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let result = crate::verify(&crate::Check {
        name: format!("{program} {}", arguments.join(" ")),
        program: program.into(),
        arguments: arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect(),
        working_directory: directory.to_path_buf(),
        timeout_seconds: timeout,
    })
    .map_err(|error| SkillError::Verification(error.to_string()))?;
    let report = format!(
        "# {}\n\nexit={:?}\n\n{}\n{}",
        result.name, result.exit_code, result.stdout, result.stderr
    );
    if result.success {
        Ok(report)
    } else {
        Err(SkillError::Verification(report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_language_defaults_to_tree() {
        assert_eq!(parse("show the files").unwrap().op, "tree");
    }

    #[test]
    fn rejects_oversized_pattern() {
        let root = tempfile::tempdir().unwrap();
        assert!(search(root.path(), &"a".repeat(201)).is_err());
    }
}
