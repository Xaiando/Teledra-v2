//! Durable inbox for isolated/remote resident reports.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResidentReport {
    pub resident: String,
    pub report_id: String,
    pub created_ms: u128,
    pub summary: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Error)]
pub enum TowerError {
    #[error("tower I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("tower report JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tower rejected an unsafe report id")]
    UnsafeId,
    #[error("tower report is too large")]
    TooLarge,
    #[error("tower report already exists")]
    AlreadyExists,
}

#[derive(Clone, Debug)]
pub struct TowerInbox {
    root: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportSummary {
    pub imported: usize,
    pub duplicates: usize,
    pub rejected: usize,
}

impl TowerInbox {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, TowerError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn receive(&self, report: &ResidentReport) -> Result<PathBuf, TowerError> {
        if !safe_component(&report.resident) || !safe_component(&report.report_id) {
            return Err(TowerError::UnsafeId);
        }
        if report.summary.chars().count() > 32 * 1024 || report.artifacts.len() > 128 {
            return Err(TowerError::TooLarge);
        }
        let directory = self.root.join(&report.resident);
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{}.json", report.report_id));
        let bytes = serde_json::to_vec_pretty(report)?;
        if bytes.len() > 1024 * 1024 {
            return Err(TowerError::TooLarge);
        }
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    TowerError::AlreadyExists
                } else {
                    TowerError::Io(error)
                }
            })?;
        file.write_all(&bytes)?;
        file.flush()?;
        Ok(path)
    }

    pub fn unread(&self) -> Result<Vec<(PathBuf, ResidentReport)>, TowerError> {
        let mut reports = Vec::new();
        visit_json(&self.root, 0, &mut |path| {
            if reports.len() < 500
                && fs::metadata(path).is_ok_and(|metadata| metadata.len() <= 1024 * 1024)
                && let Ok(bytes) = fs::read(path)
                && let Ok(report) = serde_json::from_slice::<ResidentReport>(&bytes)
            {
                reports.push((path.to_path_buf(), report));
            }
        })?;
        reports.sort_by_key(|(_, report)| report.created_ms);
        Ok(reports)
    }

    pub fn archive(&self, path: &Path) -> Result<PathBuf, TowerError> {
        let canonical_root = self.root.canonicalize()?;
        let canonical_path = path.canonicalize()?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(TowerError::UnsafeId);
        }
        let archive = self.root.join("archive");
        let relative = canonical_path
            .strip_prefix(&canonical_root)
            .map_err(|_| TowerError::UnsafeId)?;
        let destination = archive.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(canonical_path, &destination)?;
        Ok(destination)
    }

    pub fn import_jsonl(&self, path: &Path) -> Result<ImportSummary, TowerError> {
        let metadata = fs::metadata(path)?;
        if metadata.len() > 16 * 1024 * 1024 {
            return Err(TowerError::TooLarge);
        }
        let content = fs::read_to_string(path)?;
        let mut summary = ImportSummary::default();
        for line in content.lines().take(5_000) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value = match serde_json::from_str::<serde_json::Value>(line) {
                Ok(value) => value,
                Err(_) => {
                    summary.rejected += 1;
                    continue;
                }
            };
            let report = match flexible_report(value, line) {
                Some(report) => report,
                None => {
                    summary.rejected += 1;
                    continue;
                }
            };
            match self.receive(&report) {
                Ok(_) => summary.imported += 1,
                Err(TowerError::AlreadyExists) => summary.duplicates += 1,
                Err(TowerError::UnsafeId | TowerError::TooLarge) => summary.rejected += 1,
                Err(error) => return Err(error),
            }
        }
        Ok(summary)
    }
}

fn flexible_report(value: serde_json::Value, raw: &str) -> Option<ResidentReport> {
    if let Ok(report) = serde_json::from_value::<ResidentReport>(value.clone()) {
        return Some(report);
    }
    let resident = value
        .get("resident")
        .and_then(|item| item.as_str())
        .unwrap_or("wizard")
        .to_ascii_lowercase()
        .replace(|character: char| !character.is_ascii_alphanumeric(), "-")
        .trim_matches('-')
        .chars()
        .take(80)
        .collect::<String>();
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    let generated = format!("import-{:016x}", hasher.finish());
    let report_id = value
        .get("report_id")
        .or_else(|| value.get("id"))
        .or_else(|| value.get("cycle"))
        .and_then(|item| {
            item.as_str()
                .map(str::to_owned)
                .or_else(|| item.as_u64().map(|number| format!("cycle-{number}")))
        })
        .unwrap_or(generated)
        .replace(|character: char| !character.is_ascii_alphanumeric(), "-")
        .trim_matches('-')
        .chars()
        .take(80)
        .collect::<String>();
    let summary = value
        .get("summary")
        .and_then(|item| item.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| summarize_legacy(&value));
    if resident.is_empty() || report_id.is_empty() || summary.trim().is_empty() {
        return None;
    }
    let artifacts = value
        .get("artifacts")
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .take(128)
                .collect()
        })
        .unwrap_or_default();
    Some(ResidentReport {
        resident,
        report_id,
        created_ms: value
            .get("created_ms")
            .and_then(|item| item.as_u64())
            .map(u128::from)
            .unwrap_or_default(),
        summary: summary.chars().take(32 * 1024).collect(),
        artifacts,
    })
}

fn summarize_legacy(value: &serde_json::Value) -> String {
    let cycle = value.get("cycle").map_or_else(
        || "?".to_owned(),
        |item| {
            item.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| item.to_string())
        },
    );
    let topic = value
        .get("topic")
        .and_then(|item| item.as_str())
        .unwrap_or("unknown study focus");
    let tool = value
        .pointer("/tool/filename")
        .and_then(|item| item.as_str())
        .unwrap_or("no tool");
    let status = value
        .pointer("/tool/status")
        .and_then(|item| item.as_str())
        .unwrap_or("unreported");
    let finding = value
        .pointer("/findings/0/title")
        .and_then(|item| item.as_str())
        .unwrap_or("no outside finding");
    format!("Cycle {cycle}: studied {topic}; tool {tool} {status}; lead: {finding}")
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn visit_json(
    root: &Path,
    depth: u8,
    visitor: &mut impl FnMut(&Path),
) -> Result<(), std::io::Error> {
    if depth > 4 {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "archive") {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            visit_json(&path, depth + 1, visitor)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            visitor(&path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_resident_paths_are_rejected() {
        assert!(!safe_component("../escape"));
        assert!(safe_component("wizard-alpha"));
    }

    #[test]
    fn legacy_wizard_jsonl_is_imported_and_deduplicated() {
        let root = std::env::temp_dir().join(format!("teledra-tower-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let inbox = TowerInbox::new(root.join("inbox")).unwrap();
        let archive = root.join("wizard.jsonl");
        fs::write(
            &archive,
            r#"{"cycle":7,"topic":"MCP tools","tool":{"filename":"probe.py","status":"passed"},"findings":[{"title":"A public spec"}]}"#,
        )
        .unwrap();
        assert_eq!(inbox.import_jsonl(&archive).unwrap().imported, 1);
        assert_eq!(inbox.import_jsonl(&archive).unwrap().duplicates, 1);
        assert_eq!(inbox.unread().unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
