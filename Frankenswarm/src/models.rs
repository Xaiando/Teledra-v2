//! Immutable Hugging Face model locking and reproducible download plans.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use chrono::Utc;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelsConfig {
    pub models: BTreeMap<String, ModelSpec>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelSpec {
    pub repo_id: String,
    #[serde(default = "yes")]
    pub required: bool,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub arm: String,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelLock {
    pub generated_at: String,
    pub models: BTreeMap<String, LockedModel>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockedModel {
    pub repo_id: String,
    pub revision: String,
    pub license: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub arm: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DownloadedModel {
    pub repo_id: String,
    pub revision: String,
    pub snapshot_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DownloadManifest {
    pub generated_at: String,
    pub lock_sha256: String,
    pub models: BTreeMap<String, DownloadedModel>,
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("model I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("model YAML failed: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("model JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model HTTP failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid Hugging Face repository id: {0}")]
    InvalidRepo(String),
    #[error("Hugging Face returned no immutable revision for {0}")]
    MissingRevision(String),
    #[error("model download command failed for {repo}: {detail}")]
    Download { repo: String, detail: String },
    #[error(
        "neither `hf` nor `huggingface-cli` is available; install huggingface_hub or set FRANKENSWARM_HF_CLI"
    )]
    MissingDownloader,
    #[error("model downloader returned no valid snapshot directory for {0}")]
    MissingSnapshot(String),
}

#[derive(Deserialize)]
struct HubModel {
    sha: Option<String>,
    #[serde(rename = "cardData", default)]
    card_data: serde_json::Value,
}

pub fn read_config(path: &Path) -> Result<ModelsConfig, ModelError> {
    let config: ModelsConfig = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    for model in config.models.values() {
        validate_repo(&model.repo_id)?;
    }
    Ok(config)
}

pub fn lock_models(config_path: &Path, output_path: &Path) -> Result<ModelLock, ModelError> {
    let config = read_config(config_path)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .user_agent("Teledra-FrankenSwarm/2")
        .build()?;
    let mut models = BTreeMap::new();
    for (name, spec) in config.models {
        if spec.enabled == Some(false) {
            continue;
        }
        let response = client
            .get(format!(
                "https://huggingface.co/api/models/{}",
                spec.repo_id
            ))
            .send()?
            .error_for_status()?
            .json::<HubModel>()?;
        let revision = response
            .sha
            .filter(|sha| {
                sha.len() == 40 && sha.chars().all(|character| character.is_ascii_hexdigit())
            })
            .ok_or_else(|| ModelError::MissingRevision(spec.repo_id.clone()))?;
        let license = response
            .card_data
            .get("license")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .chars()
            .take(100)
            .collect();
        models.insert(
            name,
            LockedModel {
                repo_id: spec.repo_id,
                revision,
                license,
                required: spec.required,
                stage: spec.stage,
                arm: spec.arm,
            },
        );
    }
    let lock = ModelLock {
        generated_at: Utc::now().to_rfc3339(),
        models,
    };
    write_atomic(output_path, &(serde_json::to_string_pretty(&lock)? + "\n"))?;
    Ok(lock)
}

pub fn read_lock(path: &Path) -> Result<ModelLock, ModelError> {
    let lock: ModelLock = serde_json::from_str(&fs::read_to_string(path)?)?;
    for model in lock.models.values() {
        validate_repo(&model.repo_id)?;
        if model.revision.len() != 40
            || !model
                .revision
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(ModelError::MissingRevision(model.repo_id.clone()));
        }
    }
    Ok(lock)
}

pub fn lock_digest(lock: &ModelLock) -> Result<String, ModelError> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(lock)?)))
}

pub fn download_models(
    lock_path: &Path,
    cache: Option<&Path>,
) -> Result<DownloadManifest, ModelError> {
    let lock = read_lock(lock_path)?;
    let executable = downloader()?;
    let mut downloaded = BTreeMap::new();
    for (name, model) in &lock.models {
        let mut command = Command::new(&executable);
        command.args([
            "download",
            &model.repo_id,
            "--revision",
            &model.revision,
            "--include",
            "*.safetensors",
            "config.json",
            "tokenizer*",
            "special_tokens_map.json",
            "generation_config.json",
        ]);
        if let Some(cache) = cache {
            command.arg("--cache-dir").arg(cache);
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(ModelError::Download {
                repo: model.repo_id.clone(),
                detail: String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(4_000)
                    .collect(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let path = stdout
            .lines()
            .rev()
            .map(str::trim)
            .map(PathBuf::from)
            .find(|path| path.is_dir())
            .ok_or_else(|| ModelError::MissingSnapshot(model.repo_id.clone()))?;
        downloaded.insert(
            name.clone(),
            DownloadedModel {
                repo_id: model.repo_id.clone(),
                revision: model.revision.clone(),
                snapshot_path: fs::canonicalize(path)?,
            },
        );
    }
    Ok(DownloadManifest {
        generated_at: Utc::now().to_rfc3339(),
        lock_sha256: format!("{:x}", Sha256::digest(fs::read(lock_path)?)),
        models: downloaded,
    })
}

pub fn write_download_manifest(
    manifest: &DownloadManifest,
    output: &Path,
) -> Result<(), ModelError> {
    write_atomic(output, &(serde_json::to_string_pretty(manifest)? + "\n"))?;
    Ok(())
}

fn downloader() -> Result<String, ModelError> {
    if let Ok(executable) = std::env::var("FRANKENSWARM_HF_CLI") {
        return Ok(executable);
    }
    for executable in ["hf", "huggingface-cli"] {
        if Command::new(executable)
            .arg("--help")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Ok(executable.into());
        }
    }
    Err(ModelError::MissingDownloader)
}

fn validate_repo(repo: &str) -> Result<(), ModelError> {
    let mut pieces = repo.split('/');
    let owner = pieces.next().unwrap_or_default();
    let name = pieces.next().unwrap_or_default();
    let safe = |value: &str| {
        !value.is_empty()
            && value.len() <= 100
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    };
    if pieces.next().is_some() || !safe(owner) || !safe(name) {
        Err(ModelError::InvalidRepo(repo.into()))
    } else {
        Ok(())
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_v1_model_config_correctly() {
        let config: ModelsConfig = serde_yaml::from_str(
            "models:\n  base:\n    repo_id: allenai/Olmo-3\n    required: true\n",
        )
        .unwrap();
        assert_eq!(config.models["base"].repo_id, "allenai/Olmo-3");
    }

    #[test]
    fn rejects_branch_names_as_locks() {
        let lock = ModelLock {
            generated_at: String::new(),
            models: BTreeMap::from([(
                "base".into(),
                LockedModel {
                    repo_id: "allenai/Olmo".into(),
                    revision: "main".into(),
                    license: "Apache-2.0".into(),
                    required: true,
                    stage: String::new(),
                    arm: String::new(),
                },
            )]),
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("lock.json");
        fs::write(&path, serde_json::to_string(&lock).unwrap()).unwrap();
        assert!(read_lock(&path).is_err());
    }
}
