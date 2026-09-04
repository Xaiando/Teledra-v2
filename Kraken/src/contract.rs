//! Immutable execution contracts and workspace path confinement.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierProfile {
    GenericFileV1,
    PythonModuleV1,
    StrictDomGameV1,
    #[default]
    BehavioralHtml5GameV2,
    KrakenBlackboxJudgeV02,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskContract {
    pub task_id: String,
    pub start_commit: String,
    pub workspace_root: PathBuf,
    #[serde(default)]
    pub required_outputs: Vec<PathBuf>,
    #[serde(default)]
    pub verifier_profile: VerifierProfile,
    pub max_attempts: u8,
    #[serde(default)]
    pub protected_paths: Vec<PathBuf>,
    #[serde(default)]
    pub forbidden_patterns: Vec<String>,
    pub consecutive_passes_required: u8,
    #[serde(default)]
    pub allow_autostart: bool,
    #[serde(default = "user_initiated")]
    pub required_start_mode: String,
    #[serde(default = "user_accessible")]
    pub required_restart_mode: String,
}

fn user_initiated() -> String {
    "user_initiated".to_owned()
}

fn user_accessible() -> String {
    "user_accessible".to_owned()
}

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("task id is empty")]
    EmptyTask,
    #[error("workspace root is not a directory: {0}")]
    MissingWorkspace(PathBuf),
    #[error("maximum attempts must be at least one")]
    Attempts,
    #[error("consecutive passes must be at least one")]
    Passes,
    #[error("invalid start mode: {0}")]
    StartMode(String),
    #[error("invalid restart mode: {0}")]
    RestartMode(String),
    #[error("unsafe relative path: {0}")]
    UnsafeRelative(PathBuf),
    #[error("path escapes workspace {root}: {target}")]
    Escape { root: PathBuf, target: PathBuf },
    #[error("path I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("contract JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

impl TaskContract {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.task_id.trim().is_empty() {
            return Err(ContractError::EmptyTask);
        }
        if !self.workspace_root.is_dir() {
            return Err(ContractError::MissingWorkspace(self.workspace_root.clone()));
        }
        if self.max_attempts == 0 {
            return Err(ContractError::Attempts);
        }
        if self.consecutive_passes_required == 0 {
            return Err(ContractError::Passes);
        }
        if !matches!(
            self.required_start_mode.as_str(),
            "user_initiated" | "autostart" | "any"
        ) {
            return Err(ContractError::StartMode(self.required_start_mode.clone()));
        }
        if !matches!(
            self.required_restart_mode.as_str(),
            "user_accessible" | "strict_button" | "none"
        ) {
            return Err(ContractError::RestartMode(
                self.required_restart_mode.clone(),
            ));
        }
        for path in self
            .required_outputs
            .iter()
            .chain(self.protected_paths.iter())
        {
            validate_relative(path)?;
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, ContractError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

pub fn validate_relative(path: &Path) -> Result<(), ContractError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ContractError::UnsafeRelative(path.to_path_buf()));
    }
    Ok(())
}

/// Resolve a workspace-relative path and reject lexical, canonical, and
/// existing-symlink escapes. The target itself may not exist yet.
pub fn confined_path(root: &Path, relative: &Path) -> Result<PathBuf, ContractError> {
    validate_relative(relative)?;
    let canonical_root =
        fs::canonicalize(root).map_err(|_| ContractError::MissingWorkspace(root.to_path_buf()))?;
    let target = canonical_root.join(relative);

    let mut existing = target.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| ContractError::Escape {
            root: canonical_root.clone(),
            target: target.clone(),
        })?;
    }
    let canonical_existing = fs::canonicalize(existing)?;
    if !canonical_existing.starts_with(&canonical_root) {
        return Err(ContractError::Escape {
            root: canonical_root,
            target,
        });
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(root: &Path) -> TaskContract {
        TaskContract {
            task_id: "job-1".to_owned(),
            start_commit: "HEAD".to_owned(),
            workspace_root: root.to_path_buf(),
            required_outputs: vec![PathBuf::from("game/index.html")],
            verifier_profile: VerifierProfile::BehavioralHtml5GameV2,
            max_attempts: 3,
            protected_paths: vec![PathBuf::from("tests")],
            forbidden_patterns: vec!["eval(".to_owned()],
            consecutive_passes_required: 2,
            allow_autostart: false,
            required_start_mode: user_initiated(),
            required_restart_mode: user_accessible(),
        }
    }

    #[test]
    fn contract_digest_is_deterministic() {
        let root = tempfile::tempdir().unwrap();
        let first = contract(root.path()).digest().unwrap();
        let second = contract(root.path()).digest().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn traversal_and_absolute_paths_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        assert!(confined_path(root.path(), Path::new("safe/file.txt")).is_ok());
        assert!(confined_path(root.path(), Path::new("../escape.txt")).is_err());
        assert!(confined_path(root.path(), Path::new("C:/escape.txt")).is_err());
    }
}
