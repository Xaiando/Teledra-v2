//! Sandboxed court experiments, smoke reports, and explicit promotion.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_CODE_BYTES: usize = 40_000;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DraftKind {
    Tool,
    Spawn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Draft {
    pub filename: String,
    pub purpose: String,
    pub value: String,
    pub kind: DraftKind,
    pub code: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Report {
    pub filename: String,
    pub purpose: String,
    pub value: String,
    pub kind: String,
    pub passed: bool,
    pub output: String,
    pub created_ms: u128,
}

#[derive(Debug, Error)]
pub enum WorkshopError {
    #[error("workshop draft is incomplete: {0}")]
    Incomplete(String),
    #[error("workshop rejected unsafe code: {0}")]
    Unsafe(String),
    #[error("workshop I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("workshop JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("workshop smoke run failed: {0}")]
    Smoke(String),
}

#[derive(Clone, Debug)]
pub struct Workshop {
    root: PathBuf,
    python: PathBuf,
    runner: PathBuf,
}

impl Draft {
    pub fn parse(payload: &str) -> Result<Self, WorkshopError> {
        let mut filename = String::new();
        let mut purpose = "Personal workshop experiment.".to_owned();
        let mut value = "Adds a useful creation to the court.".to_owned();
        let mut kind = DraftKind::Tool;
        for line in payload.lines() {
            let trimmed = line.trim();
            let lower = trimmed.to_ascii_lowercase();
            if let Some(rest) = trimmed.split_once(':').map(|(_, rest)| rest.trim()) {
                if lower.starts_with("filename:") || lower.starts_with("file:") {
                    filename = rest.to_owned();
                } else if lower.starts_with("purpose:") {
                    purpose = rest.to_owned();
                } else if lower.starts_with("value:") {
                    value = rest.to_owned();
                } else if lower.starts_with("kind:") {
                    kind = if matches!(
                        rest.to_ascii_lowercase().as_str(),
                        "spawn" | "experience" | "art" | "visual" | "game" | "animation"
                    ) {
                        DraftKind::Spawn
                    } else {
                        DraftKind::Tool
                    };
                }
            } else if filename.is_empty()
                && !trimmed.is_empty()
                && !trimmed.starts_with("```")
                && !lower.starts_with("code")
            {
                filename = trimmed.to_owned();
            }
        }
        let code = fenced_code(payload)
            .or_else(|| {
                payload
                    .find("CODE:")
                    .map(|index| payload[index + 5..].trim().to_owned())
            })
            .ok_or_else(|| WorkshopError::Incomplete("missing code block".to_owned()))?;
        let filename = validate_filename(&filename)?;
        if code.trim().is_empty() {
            return Err(WorkshopError::Incomplete("code is empty".to_owned()));
        }
        Ok(Self {
            filename,
            purpose: clean(&purpose, 500),
            value: clean(&value, 500),
            kind,
            code,
        })
    }
}

impl Workshop {
    pub fn new(
        root: impl Into<PathBuf>,
        python: impl Into<PathBuf>,
        runner: impl Into<PathBuf>,
    ) -> Result<Self, WorkshopError> {
        let root = root.into();
        for directory in ["experiments", "reports", "approved", "broken", "logs"] {
            fs::create_dir_all(root.join(directory))?;
        }
        Ok(Self {
            root,
            python: python.into(),
            runner: runner.into(),
        })
    }

    pub fn create_and_test(&self, draft: &Draft) -> Result<Report, WorkshopError> {
        scan(draft)?;
        let candidate = self.root.join("experiments").join(&draft.filename);
        let previous = fs::read(&candidate).ok();
        fs::write(&candidate, draft.code.as_bytes())?;
        let result = self.verify_candidate(&candidate, draft.kind == DraftKind::Spawn);
        let (passed, output) = match result {
            Ok(output) => (true, output),
            Err(error) => (false, error.to_string()),
        };
        if !passed {
            let broken = self
                .root
                .join("broken")
                .join(format!("{}-{}", now_ms(), draft.filename));
            fs::write(broken, draft.code.as_bytes())?;
            if let Some(previous) = previous {
                fs::write(&candidate, previous)?;
            }
        }
        let report = Report {
            filename: draft.filename.clone(),
            purpose: draft.purpose.clone(),
            value: draft.value.clone(),
            kind: if draft.kind == DraftKind::Spawn {
                "spawn".to_owned()
            } else {
                "tool".to_owned()
            },
            passed,
            output: clean(&output, 4_000),
            created_ms: now_ms(),
        };
        let report_path = self
            .root
            .join("reports")
            .join(format!("{}.report.json", draft.filename));
        atomic_json(&report_path, &report)?;
        append_jsonl(&self.root.join("logs").join("workshop.jsonl"), &report)?;
        if passed {
            Ok(report)
        } else {
            Err(WorkshopError::Smoke(report.output))
        }
    }

    pub fn run_existing(&self, filename: &str) -> Result<String, WorkshopError> {
        let filename = validate_filename(filename)?;
        let path = self.root.join("experiments").join(filename);
        if !path.is_file() {
            return Err(WorkshopError::Incomplete(
                "experiment does not exist".to_owned(),
            ));
        }
        self.verify_candidate(&path, false)
    }

    pub fn promote(&self, filename: &str) -> Result<PathBuf, WorkshopError> {
        let filename = validate_filename(filename)?;
        let report_path = self
            .root
            .join("reports")
            .join(format!("{filename}.report.json"));
        let report: Report = serde_json::from_slice(&fs::read(report_path)?)?;
        if !report.passed {
            return Err(WorkshopError::Incomplete(
                "only a passing experiment can be promoted".to_owned(),
            ));
        }
        let source = self.root.join("experiments").join(&filename);
        let destination = self.root.join("approved").join(&filename);
        fs::copy(source, &destination)?;
        Ok(destination)
    }

    pub fn count(&self) -> usize {
        fs::read_dir(self.root.join("experiments"))
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn verify_candidate(&self, path: &Path, compile_only: bool) -> Result<String, WorkshopError> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("json") => {
                serde_json::from_slice::<serde_json::Value>(&fs::read(path)?)?;
                Ok("JSON parsed successfully".to_owned())
            }
            Some("md" | "txt") => Ok("Document passed bounded content checks".to_owned()),
            Some("py") => self.run_python(path, compile_only),
            _ => Err(WorkshopError::Incomplete(
                "unsupported extension".to_owned(),
            )),
        }
    }

    fn run_python(&self, path: &Path, compile_only: bool) -> Result<String, WorkshopError> {
        if !self.runner.is_file() {
            return Err(WorkshopError::Smoke(format!(
                "sandbox runner is missing: {}",
                self.runner.display()
            )));
        }
        let mut child = Command::new(&self.python)
            .arg(&self.runner)
            .arg(path)
            .args(compile_only.then_some("--compile-only"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let output_reader = thread::spawn(move || read_bounded(stdout));
        let error_reader = thread::spawn(move || read_bounded(stderr));
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = output_reader.join();
                let _ = error_reader.join();
                return Err(WorkshopError::Smoke(
                    "run timed out after 5 seconds".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(20));
        };
        let stdout = output_reader.join().unwrap_or_default();
        let stderr = error_reader.join().unwrap_or_default();
        if status.success() {
            let text = String::from_utf8_lossy(&stdout).trim().to_owned();
            Ok(if text.is_empty() {
                "Python smoke run completed".to_owned()
            } else {
                text
            })
        } else {
            Err(WorkshopError::Smoke(
                String::from_utf8_lossy(&stderr).trim().to_owned(),
            ))
        }
    }
}

fn scan(draft: &Draft) -> Result<(), WorkshopError> {
    if draft.code.len() > MAX_CODE_BYTES || draft.code.trim().len() < 10 {
        return Err(WorkshopError::Unsafe(
            "code must contain 10..=40000 bytes".to_owned(),
        ));
    }
    let lower = draft.code.to_ascii_lowercase();
    const FORBIDDEN: [&str; 20] = [
        "import socket",
        "from socket",
        "import requests",
        "from requests",
        "import urllib",
        "from urllib",
        "import httpx",
        "import subprocess",
        "from subprocess",
        "os.system",
        "popen(",
        "shutil.rmtree",
        "os.remove",
        "os.unlink",
        "os.rmdir",
        "../",
        "..\\",
        "[workshop_tool:",
        "todo",
        "placeholder",
    ];
    if let Some(capability) = FORBIDDEN.iter().find(|needle| lower.contains(**needle)) {
        return Err(WorkshopError::Unsafe((*capability).to_owned()));
    }
    if draft.filename.ends_with(".py") && draft.kind == DraftKind::Tool && !lower.contains("print(")
    {
        return Err(WorkshopError::Unsafe(
            "Python tools must print a concise smoke result".to_owned(),
        ));
    }
    Ok(())
}

fn validate_filename(raw: &str) -> Result<String, WorkshopError> {
    let mut filename = raw
        .trim()
        .trim_matches(|character| matches!(character, '`' | '\'' | '"' | '*'))
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(',')
        .to_owned();
    if !filename.contains('.')
        && filename
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        filename.push_str(".py");
    }
    if filename.is_empty()
        || filename.starts_with('.')
        || filename.contains(['/', '\\', ':'])
        || filename.contains("..")
        || !filename.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
    {
        return Err(WorkshopError::Incomplete("unsafe filename".to_owned()));
    }
    if !matches!(
        Path::new(&filename)
            .extension()
            .and_then(|value| value.to_str()),
        Some("py" | "json" | "md" | "txt")
    ) {
        return Err(WorkshopError::Incomplete(
            "files may only be .py, .json, .md, or .txt".to_owned(),
        ));
    }
    Ok(filename)
}

fn fenced_code(payload: &str) -> Option<String> {
    let start = payload.find("```")? + 3;
    let body = &payload[start..];
    let content = body.find('\n').map_or(body, |line| &body[line + 1..]);
    let end = content.find("```")?;
    Some(content[..end].trim().to_owned())
}

fn clean(value: &str, maximum: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(maximum)
        .collect()
}

fn read_bounded(mut reader: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = reader
        .by_ref()
        .take(MAX_OUTPUT_BYTES as u64)
        .read_to_end(&mut bytes);
    bytes
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), WorkshopError> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn append_jsonl(path: &Path, value: &impl Serialize) -> Result<(), WorkshopError> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
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
    fn upstream_workshop_payload_parses_without_prompt_scaffolding() {
        let draft = Draft::parse(
            "helper.py\nPurpose: prove the runner\nValue: useful evidence\nKind: tool\n```python\nprint('ok')\n```",
        )
        .unwrap();
        assert_eq!(draft.filename, "helper.py");
        assert_eq!(draft.kind, DraftKind::Tool);
        assert_eq!(draft.code, "print('ok')");
    }

    #[test]
    fn network_and_process_code_is_rejected() {
        let draft = Draft {
            filename: "bad.py".to_owned(),
            purpose: "bad".to_owned(),
            value: "none".to_owned(),
            kind: DraftKind::Tool,
            code: "import subprocess\nprint(subprocess.run('bad'))".to_owned(),
        };
        assert!(matches!(scan(&draft), Err(WorkshopError::Unsafe(_))));
    }
}
