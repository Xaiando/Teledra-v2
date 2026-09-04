//! Reproducible donor compatibility autopsy from immutable model revisions.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::{io::Read, path::PathBuf};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::models::{DownloadManifest, ModelError, ModelLock, read_lock};

const MAX_METADATA_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MetadataReport {
    pub model_type: Option<String>,
    pub hidden_layers: Option<u64>,
    pub hidden_size: Option<u64>,
    pub intermediate_size: Option<u64>,
    pub attention_heads: Option<u64>,
    pub key_value_heads: Option<u64>,
    pub vocab_size: Option<u64>,
    pub rope_config: Option<String>,
    pub norm_config: Option<String>,
    pub tied_embeddings: Option<bool>,
    pub dtype: Option<String>,
    pub special_tokens_hash: Option<String>,
    pub vocab_hash: Option<String>,
    pub chat_template_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DonorAssessment {
    pub name: String,
    pub repo_id: String,
    pub revision: String,
    /// True only for a phase-0 science donor that is permitted in merge configs.
    pub merge_candidate: bool,
    pub compatible: bool,
    pub mismatches: Vec<String>,
    pub metadata: MetadataReport,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutopsyReport {
    /// SHA-256 of the exact immutable lock file inspected by this report.
    pub lock_sha256: String,
    pub base: DonorAssessment,
    pub donors: Vec<DonorAssessment>,
    pub metadata_safe: bool,
    pub merge_safe: bool,
    pub tensor_analysis: String,
    pub conclusion: String,
}

#[derive(Debug, Error)]
pub enum AutopsyError {
    #[error("model lock failed: {0}")]
    Model(#[from] ModelError),
    #[error("autopsy HTTP failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("autopsy I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("autopsy JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model lock has no base model")]
    MissingBase,
    #[error("metadata response exceeded 32 MiB")]
    TooLarge,
    #[error("tensor autopsy failed: {0}")]
    Tensor(String),
}

pub fn run(lock_path: &Path, output: &Path) -> Result<AutopsyReport, AutopsyError> {
    let lock = read_lock(lock_path)?;
    let lock_sha256 = format!("{:x}", Sha256::digest(fs::read(lock_path)?));
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .user_agent("Teledra-FrankenSwarm/2")
        .build()?;
    let (base_name, base_model) = find_base(&lock).ok_or(AutopsyError::MissingBase)?;
    let base_metadata = metadata(&client, &base_model.repo_id, &base_model.revision)?;
    let base = DonorAssessment {
        name: base_name.clone(),
        repo_id: base_model.repo_id.clone(),
        revision: base_model.revision.clone(),
        merge_candidate: false,
        compatible: true,
        mismatches: Vec::new(),
        metadata: base_metadata.clone(),
    };
    let mut donors = Vec::new();
    for (name, model) in &lock.models {
        if name == &base_name {
            continue;
        }
        let candidate = metadata(&client, &model.repo_id, &model.revision)?;
        let mismatches = compare(&base_metadata, &candidate);
        let merge_candidate = model.stage == "phase_0" && model.arm == "science";
        donors.push(DonorAssessment {
            name: name.clone(),
            repo_id: model.repo_id.clone(),
            revision: model.revision.clone(),
            merge_candidate,
            compatible: mismatches.is_empty(),
            mismatches,
            metadata: candidate,
        });
    }
    let merge_donors = donors
        .iter()
        .filter(|donor| donor.merge_candidate)
        .collect::<Vec<_>>();
    let metadata_safe =
        !merge_donors.is_empty() && merge_donors.iter().all(|donor| donor.compatible);
    let conclusion = if metadata_safe {
        "All phase-0 science donors match the immutable base architecture, tokenizer vocabulary, and semantic special-token contract. Tensor deltas still require a streamed local-weight analysis after weights are downloaded."
    } else {
        "At least one phase-0 science donor differs from the base metadata contract. Do not task-vector merge until the recorded mismatches are resolved."
    }.to_owned();
    let report = AutopsyReport {
        lock_sha256,
        base,
        donors,
        metadata_safe,
        merge_safe: false,
        tensor_analysis:
            "not_run: locked safetensors must be downloaded before streamed delta analysis".into(),
        conclusion,
    };
    write_report(&report, output)?;
    Ok(report)
}

#[derive(Deserialize)]
struct TensorSummary {
    compatible: bool,
    tensors: usize,
    donors: usize,
    #[serde(default)]
    reasons: Vec<String>,
}

pub fn run_tensor(
    report: &mut AutopsyReport,
    downloads_path: &Path,
    output: &Path,
) -> Result<(), AutopsyError> {
    if !report.metadata_safe {
        return Err(AutopsyError::Tensor(
            "metadata compatibility must pass before weights are inspected".into(),
        ));
    }
    let downloads: DownloadManifest = serde_json::from_slice(&fs::read(downloads_path)?)?;
    if downloads.lock_sha256 != report.lock_sha256 {
        return Err(AutopsyError::Tensor(
            "download manifest does not match the autopsy model lock".into(),
        ));
    }
    let find = |repo: &str, revision: &str| {
        let model = downloads
            .models
            .values()
            .find(|model| model.repo_id == repo)
            .ok_or_else(|| {
                AutopsyError::Tensor(format!("downloaded snapshot missing for {repo}"))
            })?;
        let path = fs::canonicalize(&model.snapshot_path)?;
        if model.revision != revision
            || path.file_name().and_then(|name| name.to_str()) != Some(revision)
        {
            return Err(AutopsyError::Tensor(format!(
                "downloaded snapshot revision mismatch for {repo}"
            )));
        }
        Ok(path)
    };
    let base = find(&report.base.repo_id, &report.base.revision)?;
    let donors = report
        .donors
        .iter()
        .filter(|donor| donor.merge_candidate)
        .map(|donor| Ok((donor.name.clone(), find(&donor.repo_id, &donor.revision)?)))
        .collect::<Result<Vec<_>, AutopsyError>>()?;
    if donors.is_empty() {
        return Err(AutopsyError::Tensor("no merge donors were selected".into()));
    }
    fs::create_dir_all(output)?;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/tensor_autopsy.py");
    if !script.is_file() {
        return Err(AutopsyError::Tensor(format!(
            "tensor analyzer is missing: {}",
            script.display()
        )));
    }
    let python = std::env::var("FRANKENSWARM_PYTHON")
        .or_else(|_| std::env::var("TELEDRA_PYTHON"))
        .unwrap_or_else(|_| "python".into());
    let summary_path = output.join("tensor_summary.json");
    let stats_path = output.join("tensor_stats.jsonl");
    let mut command = Command::new(python);
    command
        .arg(script)
        .args(["--base", &base.display().to_string()])
        .args(["--output", &stats_path.display().to_string()])
        .args(["--summary", &summary_path.display().to_string()]);
    for (name, path) in &donors {
        command.args(["--donor", &format!("{name}={}", path.display())]);
    }
    let (success, stdout, stderr) = run_bounded(
        command,
        Duration::from_secs(
            std::env::var("FRANKENSWARM_TENSOR_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(21_600)
                .clamp(60, 43_200),
        ),
    )?;
    if !success {
        return Err(AutopsyError::Tensor(format!("{stdout}\n{stderr}")));
    }
    let summary: TensorSummary = serde_json::from_slice(&fs::read(&summary_path)?)?;
    report.tensor_analysis = format!(
        "complete: {} tensors across {} donors",
        summary.tensors, summary.donors
    );
    report.merge_safe = summary.compatible;
    if !summary.compatible {
        report.conclusion = format!(
            "Streamed safetensors analysis rejected the donor set: {}",
            summary.reasons.join("; ")
        );
    } else {
        report.conclusion = "Metadata, tensor names/shapes/dtypes, and streamed task-vector statistics passed for every phase-0 donor. Merge execution remains separately authorization-gated.".into();
    }
    write_report(report, output)?;
    Ok(())
}

fn run_bounded(
    mut command: Command,
    timeout: Duration,
) -> Result<(bool, String, String), AutopsyError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("tensor stdout was piped");
    let stderr = child.stderr.take().expect("tensor stderr was piped");
    let stdout = thread::spawn(move || read_bounded(stdout));
    let stderr = thread::spawn(move || read_bounded(stderr));
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((
                status.success(),
                stdout.join().unwrap_or_default(),
                stderr.join().unwrap_or_default(),
            ));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout.join();
            let _ = stderr.join();
            return Err(AutopsyError::Tensor(format!(
                "tensor analyzer timed out after {} seconds",
                timeout.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn read_bounded(mut reader: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = reader.by_ref().take(256 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn find_base(lock: &ModelLock) -> Option<(String, &crate::models::LockedModel)> {
    lock.models
        .get_key_value("base")
        .map(|(name, model)| (name.clone(), model))
        .or_else(|| {
            lock.models
                .iter()
                .find(|(_, model)| model.stage == "phase_0" && model.arm == "science")
                .map(|(name, model)| (name.clone(), model))
        })
}

fn metadata(client: &Client, repo: &str, revision: &str) -> Result<MetadataReport, AutopsyError> {
    let config = fetch_json(client, repo, revision, "config.json")?;
    let tokenizer = fetch_json(client, repo, revision, "tokenizer.json").ok();
    let tokenizer_config = fetch_json(client, repo, revision, "tokenizer_config.json").ok();
    let special_tokens = fetch_json(client, repo, revision, "special_tokens_map.json").ok();
    let chat_template = tokenizer_config
        .as_ref()
        .and_then(|value| value.get("chat_template"));
    Ok(MetadataReport {
        model_type: string(&config, "model_type"),
        hidden_layers: number(&config, "num_hidden_layers"),
        hidden_size: number(&config, "hidden_size"),
        intermediate_size: number(&config, "intermediate_size"),
        attention_heads: number(&config, "num_attention_heads"),
        key_value_heads: number(&config, "num_key_value_heads"),
        vocab_size: number(&config, "vocab_size"),
        rope_config: canonical(
            config
                .get("rope_scaling")
                .or_else(|| config.get("rope_theta")),
        ),
        norm_config: canonical(
            config
                .get("rms_norm_eps")
                .or_else(|| config.get("layer_norm_eps")),
        ),
        tied_embeddings: config.get("tie_word_embeddings").and_then(Value::as_bool),
        dtype: string(&config, "torch_dtype"),
        special_tokens_hash: special_token_contract_hash(
            special_tokens.as_ref(),
            tokenizer_config.as_ref(),
            tokenizer.as_ref(),
        ),
        vocab_hash: tokenizer_vocab_hash(tokenizer.as_ref()),
        chat_template_hash: hash_value(chat_template),
    })
}

fn fetch_json(
    client: &Client,
    repo: &str,
    revision: &str,
    file: &str,
) -> Result<Value, AutopsyError> {
    let response = client
        .get(format!(
            "https://huggingface.co/{repo}/resolve/{revision}/{file}"
        ))
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length as usize > MAX_METADATA_BYTES)
    {
        return Err(AutopsyError::TooLarge);
    }
    let bytes = response.bytes()?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(AutopsyError::TooLarge);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn compare(base: &MetadataReport, donor: &MetadataReport) -> Vec<String> {
    let base_value = serde_json::to_value(base).unwrap_or_default();
    let donor_value = serde_json::to_value(donor).unwrap_or_default();
    let mut mismatches = Vec::new();
    if let (Some(base), Some(donor)) = (base_value.as_object(), donor_value.as_object()) {
        // Mergekit is configured to use the base tokenizer/chat template and cast
        // the output to bfloat16. Those presentation/storage fields therefore do
        // not determine whether tensor layouts can be merged safely.
        const COMPATIBILITY_FIELDS: [&str; 12] = [
            "model_type",
            "hidden_layers",
            "hidden_size",
            "intermediate_size",
            "attention_heads",
            "key_value_heads",
            "vocab_size",
            "rope_config",
            "norm_config",
            "tied_embeddings",
            "special_tokens_hash",
            "vocab_hash",
        ];
        for field in COMPATIBILITY_FIELDS {
            let expected = base.get(field).unwrap_or(&Value::Null);
            let actual = donor.get(field).unwrap_or(&Value::Null);
            if expected != actual {
                mismatches.push(format!("{field}: base={expected} donor={actual}"));
            }
        }
    }
    mismatches
}

fn write_report(report: &AutopsyReport, output: &Path) -> Result<(), AutopsyError> {
    fs::create_dir_all(output)?;
    fs::write(
        output.join("report.json"),
        serde_json::to_string_pretty(report)? + "\n",
    )?;
    let mut markdown = format!(
        "# Autopsy Report\n\n- Metadata safe: `{}`\n- Merge safe: `{}`\n- Tensor analysis: `{}`\n\n## Conclusion\n\n{}\n\n## Donors\n\n",
        report.metadata_safe, report.merge_safe, report.tensor_analysis, report.conclusion
    );
    for donor in &report.donors {
        markdown.push_str(&format!(
            "- **{}** `{}` ({}): {}",
            donor.name,
            donor.revision,
            if donor.merge_candidate {
                "merge donor"
            } else {
                "baseline"
            },
            if donor.compatible {
                "compatible"
            } else {
                "rejected"
            }
        ));
        if !donor.mismatches.is_empty() {
            markdown.push_str(&format!(" — {}", donor.mismatches.join("; ")));
        }
        markdown.push('\n');
    }
    fs::write(output.join("report.md"), markdown)?;
    if !report.tensor_analysis.starts_with("complete:") {
        fs::write(
            output.join("tensor_stats.jsonl"),
            "{\"status\":\"not_run\",\"reason\":\"download locked safetensors before streamed delta analysis\"}\n",
        )?;
    }
    Ok(())
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}
fn canonical(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| {
        let mut normalized = value.clone();
        normalize_numbers(&mut normalized);
        serde_json::to_string(&normalized).ok()
    })
}
fn hash_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|value| serde_json::to_vec(value).ok())
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

fn tokenizer_vocab(tokenizer: Option<&Value>) -> BTreeMap<String, u64> {
    let mut vocab = BTreeMap::new();
    if let Some(entries) = tokenizer.and_then(|value| value.pointer("/model/vocab")) {
        if let Some(entries) = entries.as_object() {
            for (token, id) in entries {
                if let Some(id) = id.as_u64() {
                    vocab.insert(token.clone(), id);
                }
            }
        } else if let Some(entries) = entries.as_array() {
            // SentencePiece/Unigram tokenizers encode vocabulary as
            // [[token, score], ...]; the stable array index is the token ID.
            for (id, entry) in entries.iter().enumerate() {
                if let Some(token) = entry
                    .as_array()
                    .and_then(|parts| parts.first())
                    .and_then(Value::as_str)
                {
                    vocab.insert(token.to_owned(), id as u64);
                }
            }
        }
    }
    if let Some(entries) = tokenizer
        .and_then(|value| value.get("added_tokens"))
        .and_then(Value::as_array)
    {
        for entry in entries {
            if let (Some(token), Some(id)) = (
                entry.get("content").and_then(Value::as_str),
                entry.get("id").and_then(Value::as_u64),
            ) {
                vocab.insert(token.to_owned(), id);
            }
        }
    }
    vocab
}

fn tokenizer_vocab_hash(tokenizer: Option<&Value>) -> Option<String> {
    let vocab = tokenizer_vocab(tokenizer);
    (!vocab.is_empty()).then(|| {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&vocab).unwrap_or_default())
        )
    })
}

fn token_content(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("content").and_then(Value::as_str))
}

fn special_token_contract_hash(
    special_tokens: Option<&Value>,
    tokenizer_config: Option<&Value>,
    tokenizer: Option<&Value>,
) -> Option<String> {
    let vocab = tokenizer_vocab(tokenizer);
    let mut contract = BTreeMap::<String, (String, Option<u64>)>::new();
    for role in ["bos_token", "eos_token", "pad_token", "unk_token"] {
        let token = special_tokens
            .and_then(|value| value.get(role))
            .and_then(token_content)
            .or_else(|| {
                tokenizer_config
                    .and_then(|value| value.get(role))
                    .and_then(token_content)
            });
        if let Some(token) = token {
            contract.insert(role.into(), (token.into(), vocab.get(token).copied()));
        }
    }
    // Some base tokenizers omit a redundant BOS declaration while donor
    // tokenizers explicitly set BOS=EOS. Treat those encodings identically.
    if !contract.contains_key("bos_token")
        && let Some(eos) = contract.get("eos_token").cloned()
    {
        contract.insert("bos_token".into(), eos);
    }
    (!contract.is_empty()).then(|| {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&contract).unwrap_or_default())
        )
    })
}

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if number.as_u64().is_none() && number.as_i64().is_none() => {
            if let Some(float) = number.as_f64()
                && float.fract() == 0.0
                && float >= i64::MIN as f64
                && float <= i64::MAX as f64
            {
                *value = Value::from(float as i64);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
        Value::Object(values) => values.values_mut().for_each(normalize_numbers),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_detects_one_changed_dimension() {
        let base = MetadataReport {
            model_type: Some("olmo".into()),
            hidden_layers: Some(32),
            hidden_size: Some(4096),
            intermediate_size: None,
            attention_heads: None,
            key_value_heads: None,
            vocab_size: None,
            rope_config: None,
            norm_config: None,
            tied_embeddings: None,
            dtype: None,
            special_tokens_hash: None,
            vocab_hash: None,
            chat_template_hash: None,
        };
        let mut donor = base.clone();
        donor.hidden_size = Some(8192);
        assert!(
            compare(&base, &donor)
                .iter()
                .any(|item| item.starts_with("hidden_size:"))
        );
    }

    #[test]
    fn semantic_special_tokens_ignore_string_object_encoding() {
        let tokenizer = serde_json::json!({
            "model":{"vocab":{"<|endoftext|>":0,"<|pad|>":1}}
        });
        let strings = serde_json::json!({
            "eos_token":"<|endoftext|>", "pad_token":"<|pad|>",
            "unk_token":"<|endoftext|>"
        });
        let objects = serde_json::json!({
            "bos_token":{"content":"<|endoftext|>"},
            "eos_token":{"content":"<|endoftext|>"},
            "pad_token":{"content":"<|pad|>"},
            "unk_token":{"content":"<|endoftext|>"}
        });
        assert_eq!(
            special_token_contract_hash(Some(&strings), None, Some(&tokenizer)),
            special_token_contract_hash(Some(&objects), None, Some(&tokenizer))
        );
    }

    #[test]
    fn canonical_numbers_ignore_integral_float_encoding() {
        assert_eq!(
            canonical(Some(&serde_json::json!(32))),
            canonical(Some(&serde_json::json!(32.0)))
        );
    }
}
