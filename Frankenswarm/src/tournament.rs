//! Deterministic merge tournament planning with optional explicit mergekit execution.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::autopsy::AutopsyReport;
use crate::models::{DownloadManifest, ModelError, read_lock};

const ALLOWED_METHODS: [&str; 4] = ["ram", "ramplus_tl", "task_arithmetic", "ties"];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MergeConfig {
    pub merge_method: String,
    pub base_model: String,
    pub models: Vec<MergeModel>,
    #[serde(default)]
    pub parameters: Value,
    #[serde(default)]
    pub tokenizer: Value,
    #[serde(default)]
    pub chat_template: Value,
    pub dtype: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MergeModel {
    pub model: String,
    #[serde(default)]
    pub parameters: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CandidateManifest {
    pub id: String,
    pub method: String,
    pub seed: u64,
    pub config: PathBuf,
    pub config_sha256: String,
    pub lock_sha256: String,
    pub output: PathBuf,
    pub status: String,
    pub command: Vec<String>,
}

#[derive(Debug, Error)]
pub enum TournamentError {
    #[error("tournament I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("tournament YAML failed: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("tournament JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model lock failed: {0}")]
    Model(#[from] ModelError),
    #[error("invalid merge configuration: {0}")]
    Invalid(String),
    #[error("mergekit failed: {0}")]
    Merge(String),
    #[error("merge authorization denied: {0}")]
    Authorization(String),
}

pub fn plan(
    config_dir: &Path,
    lock_path: &Path,
    artifacts: &Path,
    seeds: &[u64],
) -> Result<Vec<CandidateManifest>, TournamentError> {
    let lock = read_lock(lock_path)?;
    let locked_repos = lock
        .models
        .values()
        .map(|model| model.repo_id.as_str())
        .collect::<BTreeSet<_>>();
    let lock_sha256 = format!("{:x}", Sha256::digest(fs::read(lock_path)?));
    let mut configs = fs::read_dir(config_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| {
                matches!(extension.to_string_lossy().as_ref(), "yaml" | "yml")
            })
        })
        .collect::<Vec<_>>();
    configs.sort();
    let mut candidates = Vec::new();
    for path in configs {
        let bytes = fs::read(&path)?;
        let config: MergeConfig = serde_yaml::from_slice(&bytes)?;
        validate(&config, &locked_repos)?;
        let config_hash = format!("{:x}", Sha256::digest(&bytes));
        for seed in seeds.iter().copied().take(32) {
            let id = format!("{}-{}-s{seed}", config.merge_method, &config_hash[..8]);
            let output = artifacts.join("candidates").join(&id).join("model");
            let command = vec![
                "mergekit-yaml".into(),
                path.display().to_string(),
                output.display().to_string(),
                "--seed".into(),
                seed.to_string(),
            ];
            candidates.push(CandidateManifest {
                id,
                method: config.merge_method.clone(),
                seed,
                config: path.clone(),
                config_sha256: config_hash.clone(),
                lock_sha256: lock_sha256.clone(),
                output,
                status: "planned".into(),
                command,
            });
        }
    }
    fs::create_dir_all(artifacts.join("tournament"))?;
    fs::write(
        artifacts.join("tournament").join("manifest.json"),
        serde_json::to_string_pretty(&candidates)? + "\n",
    )?;
    Ok(candidates)
}

pub fn build(
    mut candidate: CandidateManifest,
    execute: bool,
    autopsy_path: &Path,
    downloads_path: &Path,
) -> Result<CandidateManifest, TournamentError> {
    if !execute {
        return Ok(candidate);
    }
    if std::env::var("FRANKENSWARM_ENABLE_MERGE").ok().as_deref() != Some("1") {
        return Err(TournamentError::Invalid(
            "set FRANKENSWARM_ENABLE_MERGE=1 to authorize the large local merge".into(),
        ));
    }
    let autopsy = authorize(&candidate, autopsy_path)?;
    if let Some(parent) = candidate.output.parent() {
        fs::create_dir_all(parent)?;
    }
    let expected_revisions = std::iter::once((&autopsy.base.repo_id, &autopsy.base.revision))
        .chain(
            autopsy
                .donors
                .iter()
                .map(|donor| (&donor.repo_id, &donor.revision)),
        )
        .map(|(repo, revision)| (repo.clone(), revision.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let locked_config = render_locked_config(&candidate, downloads_path, &expected_revisions)?;
    candidate.command[1] = locked_config.display().to_string();
    let executable =
        std::env::var("FRANKENSWARM_MERGEKIT").unwrap_or_else(|_| candidate.command[0].clone());
    let output = Command::new(executable)
        .args(&candidate.command[1..])
        .output()?;
    if !output.status.success() {
        return Err(TournamentError::Merge(format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    candidate.status = "built".into();
    let manifest = candidate
        .output
        .parent()
        .unwrap_or(&candidate.output)
        .join("candidate.json");
    fs::write(manifest, serde_json::to_string_pretty(&candidate)? + "\n")?;
    Ok(candidate)
}

fn render_locked_config(
    candidate: &CandidateManifest,
    downloads_path: &Path,
    expected_revisions: &std::collections::BTreeMap<String, String>,
) -> Result<PathBuf, TournamentError> {
    let downloads: DownloadManifest =
        serde_json::from_slice(&fs::read(downloads_path).map_err(|error| {
            TournamentError::Authorization(format!(
                "run `frankenswarm download-models` first ({}: {error})",
                downloads_path.display()
            ))
        })?)?;
    if downloads.lock_sha256 != candidate.lock_sha256 {
        return Err(TournamentError::Authorization(
            "download manifest model-lock digest does not match the candidate plan".into(),
        ));
    }
    let by_repo = downloads
        .models
        .values()
        .map(|model| (model.repo_id.as_str(), model))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut config: MergeConfig = serde_yaml::from_slice(&fs::read(&candidate.config)?)?;
    config.base_model = locked_snapshot(&by_repo, expected_revisions, &config.base_model)?;
    for model in &mut config.models {
        model.model = locked_snapshot(&by_repo, expected_revisions, &model.model)?;
    }
    let directory = candidate
        .output
        .parent()
        .ok_or_else(|| TournamentError::Invalid("candidate output has no parent".into()))?;
    fs::create_dir_all(directory)?;
    let output = directory.join("merge-config.locked.yaml");
    fs::write(&output, serde_yaml::to_string(&config)?)?;
    Ok(output)
}

fn locked_snapshot(
    downloads: &std::collections::BTreeMap<&str, &crate::models::DownloadedModel>,
    expected_revisions: &std::collections::BTreeMap<String, String>,
    repo: &str,
) -> Result<String, TournamentError> {
    let model = downloads.get(repo).ok_or_else(|| {
        TournamentError::Authorization(format!("locked snapshot is missing for {repo}"))
    })?;
    let expected = expected_revisions.get(repo).ok_or_else(|| {
        TournamentError::Authorization(format!("autopsy has no immutable revision for {repo}"))
    })?;
    if &model.revision != expected {
        return Err(TournamentError::Authorization(format!(
            "downloaded revision for {repo} does not match autopsy"
        )));
    }
    let canonical = fs::canonicalize(&model.snapshot_path).map_err(|error| {
        TournamentError::Authorization(format!(
            "locked snapshot is unavailable for {repo}: {error}"
        ))
    })?;
    if canonical.file_name().and_then(|name| name.to_str()) != Some(model.revision.as_str()) {
        return Err(TournamentError::Authorization(format!(
            "snapshot directory for {repo} is not the immutable revision {}",
            model.revision
        )));
    }
    if !canonical.join("config.json").is_file()
        || !fs::read_dir(&canonical)?
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("safetensors"))
            })
    {
        return Err(TournamentError::Authorization(format!(
            "snapshot for {repo} has no config.json and safetensors weights"
        )));
    }
    Ok(canonical.display().to_string())
}

fn authorize(
    candidate: &CandidateManifest,
    autopsy_path: &Path,
) -> Result<AutopsyReport, TournamentError> {
    let report: AutopsyReport =
        serde_json::from_slice(&fs::read(autopsy_path).map_err(|error| {
            TournamentError::Authorization(format!(
                "run `frankenswarm autopsy` first ({}: {error})",
                autopsy_path.display()
            ))
        })?)?;
    check_authorization(
        &candidate.lock_sha256,
        &report.lock_sha256,
        report.merge_safe,
        &report.tensor_analysis,
    )?;
    Ok(report)
}

fn check_authorization(
    candidate_lock: &str,
    report_lock: &str,
    merge_safe: bool,
    tensor_analysis: &str,
) -> Result<(), TournamentError> {
    if report_lock != candidate_lock {
        return Err(TournamentError::Authorization(
            "autopsy model-lock digest does not match the candidate plan; rerun autopsy".into(),
        ));
    }
    if !merge_safe {
        return Err(TournamentError::Authorization(
            "phase-0 donor metadata is incompatible; inspect autopsy/report.md".into(),
        ));
    }
    if !tensor_analysis.starts_with("complete:") {
        return Err(TournamentError::Authorization(
            "streamed safetensors analysis is not complete".into(),
        ));
    }
    Ok(())
}

fn validate(config: &MergeConfig, locked: &BTreeSet<&str>) -> Result<(), TournamentError> {
    if !ALLOWED_METHODS.contains(&config.merge_method.as_str()) {
        return Err(TournamentError::Invalid(format!(
            "unsupported merge method: {}",
            config.merge_method
        )));
    }
    if !locked.contains(config.base_model.as_str()) {
        return Err(TournamentError::Invalid(format!(
            "base model is not locked: {}",
            config.base_model
        )));
    }
    if config.models.is_empty() || config.models.len() > 16 {
        return Err(TournamentError::Invalid(
            "candidate must have 1-16 donors".into(),
        ));
    }
    for model in &config.models {
        if !locked.contains(model.model.as_str()) {
            return Err(TournamentError::Invalid(format!(
                "donor is not locked: {}",
                model.model
            )));
        }
    }
    if config.dtype != "bfloat16" {
        return Err(TournamentError::Invalid(
            "only bfloat16 candidates are allowed".into(),
        ));
    }
    if config.tokenizer.get("source").and_then(Value::as_str) != Some("base") {
        return Err(TournamentError::Invalid(
            "tokenizer source must be base".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unlocked_donor() {
        let config = MergeConfig {
            merge_method: "ties".into(),
            base_model: "a/base".into(),
            models: vec![MergeModel {
                model: "evil/donor".into(),
                parameters: Value::Null,
            }],
            parameters: Value::Null,
            tokenizer: serde_json::json!({"source":"base"}),
            chat_template: Value::Null,
            dtype: "bfloat16".into(),
        };
        assert!(validate(&config, &BTreeSet::from(["a/base"])).is_err());
    }

    #[test]
    fn authorization_fails_closed_without_tensor_analysis() {
        let error = check_authorization("same", "same", true, "not_run: weights missing")
            .unwrap_err()
            .to_string();
        assert!(error.contains("safetensors analysis is not complete"));
        assert!(check_authorization("same", "same", true, "complete: 12 tensors").is_ok());
    }

    #[test]
    fn execution_config_uses_exact_downloaded_snapshots() {
        let root = tempfile::tempdir().unwrap();
        let base_revision = "a".repeat(40);
        let donor_revision = "b".repeat(40);
        let base = root.path().join(&base_revision);
        let donor = root.path().join(&donor_revision);
        for directory in [&base, &donor] {
            fs::create_dir_all(directory).unwrap();
            fs::write(directory.join("config.json"), "{}").unwrap();
            fs::write(directory.join("model.safetensors"), "fixture").unwrap();
        }
        let config_path = root.path().join("merge.yaml");
        fs::write(
            &config_path,
            "merge_method: ties\nbase_model: a/base\nmodels:\n  - model: a/donor\ntokenizer: {source: base}\ndtype: bfloat16\n",
        )
        .unwrap();
        let candidate = CandidateManifest {
            id: "candidate".into(),
            method: "ties".into(),
            seed: 1,
            config: config_path,
            config_sha256: "config".into(),
            lock_sha256: "lock".into(),
            output: root.path().join("candidate/model"),
            status: "planned".into(),
            command: Vec::new(),
        };
        let manifest = crate::models::DownloadManifest {
            generated_at: String::new(),
            lock_sha256: "lock".into(),
            models: std::collections::BTreeMap::from([
                (
                    "base".into(),
                    crate::models::DownloadedModel {
                        repo_id: "a/base".into(),
                        revision: base_revision.clone(),
                        snapshot_path: base.clone(),
                    },
                ),
                (
                    "donor".into(),
                    crate::models::DownloadedModel {
                        repo_id: "a/donor".into(),
                        revision: donor_revision.clone(),
                        snapshot_path: donor.clone(),
                    },
                ),
            ]),
        };
        let manifest_path = root.path().join("downloads.json");
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let expected = std::collections::BTreeMap::from([
            ("a/base".into(), base_revision),
            ("a/donor".into(), donor_revision),
        ]);
        let rendered = render_locked_config(&candidate, &manifest_path, &expected).unwrap();
        let rendered: MergeConfig = serde_yaml::from_slice(&fs::read(rendered).unwrap()).unwrap();
        assert_eq!(
            rendered.base_model,
            fs::canonicalize(base).unwrap().display().to_string()
        );
        assert_eq!(
            rendered.models[0].model,
            fs::canonicalize(donor).unwrap().display().to_string()
        );
    }
}
