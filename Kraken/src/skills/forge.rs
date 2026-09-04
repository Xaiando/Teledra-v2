use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::contract::{TaskContract, VerifierProfile, confined_path};

use super::{SkillContext, SkillError, SkillResult, model};

const MAX_CANDIDATE_FILES: usize = 2_000;
const MAX_CANDIDATE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Deserialize)]
struct LegacyRequest {
    task: String,
    #[serde(default = "dot")]
    dir: PathBuf,
    #[serde(default)]
    quality: String,
    #[serde(default)]
    verify_only: bool,
    #[serde(default)]
    required_outputs: Vec<PathBuf>,
    #[serde(default)]
    forbidden_patterns: Vec<String>,
    #[serde(default = "three")]
    max_attempts: u8,
}

fn dot() -> PathBuf {
    PathBuf::from(".")
}

fn three() -> u8 {
    3
}

#[derive(Deserialize)]
struct ForgeEnvelope {
    task: String,
    contract: TaskContract,
    #[serde(default)]
    verify_only: bool,
}

#[derive(Deserialize)]
struct GeneratedFiles {
    files: Vec<GeneratedFile>,
}

#[derive(Deserialize)]
struct GeneratedFile {
    path: PathBuf,
    content: String,
}

pub fn execute(input: &str, context: &SkillContext) -> Result<SkillResult, SkillError> {
    let (task, contract, verify_only) = parse_request(input, context)?;
    contract.validate()?;
    let digest = contract.digest()?;
    let actual = fs::canonicalize(&contract.workspace_root)?;
    let target_root = if verify_only {
        actual.clone()
    } else {
        let candidate = context.workdir.join("candidate");
        let repairing = candidate.exists() && !context.feedback.is_empty();
        if candidate.exists() && !repairing {
            fs::remove_dir_all(&candidate)?;
        }
        if !candidate.exists() {
            fs::create_dir_all(&candidate)?;
            copy_candidate(&actual, &candidate)?;
        }
        candidate
    };
    let protected_before = snapshot_paths(&target_root, &contract.protected_paths)?;

    let mut generated = Vec::new();
    if !verify_only {
        let context_files = candidate_context(&target_root)?;
        let repair_feedback = if context.feedback.is_empty() {
            "This is the first attempt.".to_owned()
        } else {
            format!(
                "REPAIR THIS VERIFIED FAILURE:\n{}",
                context.feedback.join("\n---\n")
            )
        };
        let prompt = format!(
            "TASK:\n{task}\n\nIMMUTABLE CONTRACT SHA256: {digest}\nRequired outputs: {:?}\nForbidden patterns: {:?}\nVerifier: {:?}\n\n{repair_feedback}\n\nCURRENT ISOLATED CANDIDATE EXCERPTS:\n{context_files}\n\nReturn only JSON in this exact shape: {{\"files\":[{{\"path\":\"relative/path\",\"content\":\"complete file text\"}}]}}. Include every file you create or replace. Do not use absolute paths or parent traversal.",
            contract.required_outputs, contract.forbidden_patterns, contract.verifier_profile
        );
        let raw = model::complete(
            "You are Kraken code_forge. Produce complete, runnable files under the immutable contract. Never alter protected paths and never claim tests passed.",
            &prompt,
            if task.to_ascii_lowercase().contains("creative") {
                0.35
            } else {
                0.1
            },
        )?;
        let files = parse_generated(&raw, &contract.required_outputs)?;
        if files.files.is_empty() || files.files.len() > 100 {
            return Err(SkillError::Verification(
                "forge must return 1-100 files".into(),
            ));
        }
        for file in files.files {
            let path = confined_path(&target_root, &file.path)?;
            if contract
                .protected_paths
                .iter()
                .any(|protected| file.path.starts_with(protected))
            {
                return Err(SkillError::Verification(format!(
                    "model attempted protected path: {}",
                    file.path.display()
                )));
            }
            if file.content.len() > 4 * 1024 * 1024 {
                return Err(SkillError::Verification(format!(
                    "generated file is too large: {}",
                    file.path.display()
                )));
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, file.content)?;
            generated.push(file.path);
        }
    }

    if protected_before != snapshot_paths(&target_root, &contract.protected_paths)? {
        return Err(SkillError::Verification(
            "protected paths changed inside candidate".into(),
        ));
    }
    let mut verifier_evidence = Vec::new();
    for pass in 1..=contract.consecutive_passes_required.min(5) {
        verifier_evidence.extend(verify_candidate(&target_root, &contract)?);
        verifier_evidence.push(format!("verification pass {pass} succeeded"));
    }
    if !verify_only {
        generated = changed_files(&actual, &target_root)?;
        for path in &generated {
            let source = confined_path(&target_root, path)?;
            let destination = confined_path(&actual, path)?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let temporary = destination.with_extension(format!(
                "{}.kraken-tmp",
                destination
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
            fs::copy(&source, &temporary)?;
            fs::rename(&temporary, &destination)?;
        }
    }
    let manifest = serde_json::json!({
        "task": task,
        "contract_sha256": digest,
        "verify_only": verify_only,
        "generated": generated,
        "verifier_evidence": verifier_evidence,
    });
    let output = context.write_vault(
        "code-forge.json",
        &(serde_json::to_string_pretty(&manifest)? + "\n"),
    )?;
    Ok(SkillResult {
        summary: format!(
            "code_forge verified {} file(s) under contract {digest}.",
            generated.len()
        ),
        output: Some(output),
        children: Vec::new(),
        evidence: verifier_evidence,
    })
}

fn parse_request(
    input: &str,
    context: &SkillContext,
) -> Result<(String, TaskContract, bool), SkillError> {
    if let Ok(envelope) = serde_json::from_str::<ForgeEnvelope>(input) {
        return Ok((envelope.task, envelope.contract, envelope.verify_only));
    }
    let request: LegacyRequest = serde_json::from_str(input).map_err(|_| {
        SkillError::Unsupported(
            "code_forge input must be JSON with task and dir, or task and contract".into(),
        )
    })?;
    if request.task.trim().is_empty() {
        return Err(SkillError::Empty);
    }
    let workspace_root = context.workspace_path(&request.dir)?;
    fs::create_dir_all(&workspace_root)?;
    let is_game = request.quality.eq_ignore_ascii_case("beast")
        || request.task.to_ascii_lowercase().contains("game");
    let required_outputs = if request.required_outputs.is_empty() {
        vec![PathBuf::from(if is_game {
            "index.html"
        } else {
            "README.md"
        })]
    } else {
        request.required_outputs
    };
    let mut forbidden_patterns = vec!["eval(".into(), "new Function(".into()];
    forbidden_patterns.extend(request.forbidden_patterns);
    Ok((
        request.task,
        TaskContract {
            task_id: context.job_id.clone(),
            start_commit: "working-tree-snapshot".into(),
            workspace_root,
            required_outputs,
            verifier_profile: if is_game {
                VerifierProfile::BehavioralHtml5GameV2
            } else {
                VerifierProfile::GenericFileV1
            },
            max_attempts: request.max_attempts.clamp(1, 5),
            protected_paths: vec![PathBuf::from(".git"), PathBuf::from("tests")],
            forbidden_patterns,
            consecutive_passes_required: if is_game { 2 } else { 1 },
            allow_autostart: false,
            required_start_mode: "user_initiated".into(),
            required_restart_mode: "user_accessible".into(),
        },
        request.verify_only,
    ))
}

pub(super) fn attempt_limit(input: &str) -> u8 {
    if let Ok(envelope) = serde_json::from_str::<ForgeEnvelope>(input) {
        return envelope.contract.max_attempts.clamp(1, 5);
    }
    serde_json::from_str::<LegacyRequest>(input)
        .map(|request| request.max_attempts.clamp(1, 5))
        .unwrap_or(3)
}

fn changed_files(actual: &Path, candidate: &Path) -> Result<Vec<PathBuf>, SkillError> {
    let mut changed = Vec::new();
    for entry in WalkDir::new(candidate)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let relative = entry
            .path()
            .strip_prefix(candidate)
            .map_err(|error| SkillError::Verification(error.to_string()))?;
        let destination = actual.join(relative);
        if !destination.is_file() || fs::read(entry.path())? != fs::read(&destination)? {
            changed.push(relative.to_path_buf());
        }
    }
    changed.sort();
    Ok(changed)
}

fn copy_candidate(source: &Path, destination: &Path) -> Result<(), SkillError> {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_string_lossy().as_ref(),
                ".git" | "target" | "node_modules" | ".venv" | "__pycache__"
            )
        })
    {
        let entry = entry.map_err(|error| SkillError::Io(error.into()))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|error| SkillError::Verification(error.to_string()))?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_symlink() {
            return Err(SkillError::Verification(format!(
                "candidate contains a symlink: {}",
                entry.path().display()
            )));
        }
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        count += 1;
        bytes = bytes.saturating_add(
            entry
                .metadata()
                .map_err(|error| {
                    SkillError::Verification(format!("cannot inspect candidate entry: {error}"))
                })?
                .len(),
        );
        if count > MAX_CANDIDATE_FILES || bytes > MAX_CANDIDATE_BYTES {
            return Err(SkillError::Verification(
                "candidate exceeds 2,000 files or 50 MiB".into(),
            ));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(entry.path(), target)?;
    }
    Ok(())
}

fn candidate_context(root: &Path) -> Result<String, SkillError> {
    let mut report = String::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .take(80)
    {
        let bytes = fs::read(entry.path())?;
        if bytes.len() > 512 * 1024 || bytes.iter().take(1_024).any(|byte| *byte == 0) {
            continue;
        }
        let excerpt: String = String::from_utf8_lossy(&bytes)
            .chars()
            .take(4_000)
            .collect();
        report.push_str(&format!(
            "\n--- {} ---\n{}\n",
            entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .display(),
            excerpt
        ));
        if report.len() > 48_000 {
            break;
        }
    }
    Ok(report)
}

fn parse_generated(raw: &str, required: &[PathBuf]) -> Result<GeneratedFiles, SkillError> {
    if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}'))
        && let Ok(files) = serde_json::from_str::<GeneratedFiles>(&raw[start..=end])
    {
        return Ok(files);
    }
    if required.len() == 1
        && let Some(start) = raw.find("```")
    {
        let after = &raw[start + 3..];
        let content_start = after.find('\n').map(|index| index + 1).unwrap_or(0);
        if let Some(end) = after[content_start..].find("```") {
            return Ok(GeneratedFiles {
                files: vec![GeneratedFile {
                    path: required[0].clone(),
                    content: after[content_start..content_start + end].to_owned(),
                }],
            });
        }
    }
    Err(SkillError::Model(
        "code_forge returned neither valid file JSON nor one required-output code fence".into(),
    ))
}

fn snapshot_paths(root: &Path, paths: &[PathBuf]) -> Result<BTreeMap<PathBuf, String>, SkillError> {
    let mut snapshot = BTreeMap::new();
    for relative in paths {
        let path = confined_path(root, relative)?;
        if path.is_file() {
            snapshot.insert(relative.clone(), hash_file(&path)?);
        } else if path.is_dir() {
            for entry in WalkDir::new(&path)
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
            {
                let nested = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(entry.path())
                    .to_path_buf();
                snapshot.insert(nested, hash_file(entry.path())?);
            }
        }
    }
    Ok(snapshot)
}

fn hash_file(path: &Path) -> Result<String, SkillError> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

fn verify_candidate(root: &Path, contract: &TaskContract) -> Result<Vec<String>, SkillError> {
    let mut evidence = Vec::new();
    for relative in &contract.required_outputs {
        let path = confined_path(root, relative)?;
        let metadata = fs::metadata(&path).map_err(|_| {
            SkillError::Verification(format!(
                "required output is missing: {}",
                relative.display()
            ))
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(SkillError::Verification(format!(
                "required output is empty: {}",
                relative.display()
            )));
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        for pattern in &contract.forbidden_patterns {
            if text.contains(pattern) {
                return Err(SkillError::Verification(format!(
                    "{} contains forbidden pattern {pattern:?}",
                    relative.display()
                )));
            }
        }
        evidence.push(format!(
            "{}: {} bytes sha256={}",
            relative.display(),
            metadata.len(),
            hash_file(&path)?
        ));
    }
    match contract.verifier_profile {
        VerifierProfile::GenericFileV1 => {}
        VerifierProfile::PythonModuleV1 => run_gate(
            root,
            "python",
            &["-m", "py_compile"],
            &contract.required_outputs,
        )?,
        VerifierProfile::StrictDomGameV1 | VerifierProfile::BehavioralHtml5GameV2 => {
            verify_html_game(root, contract)?
        }
        VerifierProfile::KrakenBlackboxJudgeV02 => {
            if root.join("Cargo.toml").is_file() {
                run_simple(root, "cargo", &["test", "--all-targets"], 600)?;
            } else if root.join("tests").is_dir() {
                run_simple(root, "python", &["-m", "unittest", "discover"], 180)?;
            }
        }
    }
    Ok(evidence)
}

fn verify_html_game(root: &Path, contract: &TaskContract) -> Result<(), SkillError> {
    let html = contract
        .required_outputs
        .iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
        })
        .ok_or_else(|| {
            SkillError::Verification("game contract has no required HTML output".into())
        })?;
    let source = fs::read_to_string(confined_path(root, html)?)?;
    for (needle, reason) in [
        ("<canvas", "canvas"),
        ("<script", "script"),
        ("requestAnimationFrame", "animation loop"),
    ] {
        if !source.contains(needle) {
            return Err(SkillError::Verification(format!(
                "game is missing {reason}"
            )));
        }
    }
    if !contract.allow_autostart
        && !source.to_ascii_lowercase().contains("start")
        && !source.to_ascii_lowercase().contains("play")
    {
        return Err(SkillError::Verification(
            "game has no discoverable user-initiated start control".into(),
        ));
    }
    if contract.required_restart_mode != "none" && !source.to_ascii_lowercase().contains("restart")
    {
        return Err(SkillError::Verification(
            "game has no discoverable restart control".into(),
        ));
    }
    Ok(())
}

fn run_gate(
    root: &Path,
    program: &str,
    prefix: &[&str],
    files: &[PathBuf],
) -> Result<(), SkillError> {
    for file in files {
        let mut arguments = prefix
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        arguments.push(file.display().to_string());
        let references = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        run_simple(root, program, &references, 120)?;
    }
    Ok(())
}

fn run_simple(
    root: &Path,
    program: &str,
    arguments: &[&str],
    timeout: u64,
) -> Result<(), SkillError> {
    let result = crate::verify(&crate::Check {
        name: format!("{program} {}", arguments.join(" ")),
        program: program.into(),
        arguments: arguments.iter().map(|value| (*value).to_owned()).collect(),
        working_directory: root.into(),
        timeout_seconds: timeout,
    })
    .map_err(|error| SkillError::Verification(error.to_string()))?;
    if result.success {
        Ok(())
    } else {
        Err(SkillError::Verification(format!(
            "{}\n{}",
            result.stdout, result.stderr
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_fenced_required_output() {
        let files =
            parse_generated("```html\n<h1>ok</h1>\n```", &[PathBuf::from("index.html")]).unwrap();
        assert_eq!(files.files[0].path, PathBuf::from("index.html"));
    }

    #[test]
    fn legacy_forge_keeps_three_repair_attempts() {
        assert_eq!(attempt_limit(r#"{"task":"repair me"}"#), 3);
        assert_eq!(attempt_limit(r#"{"task":"repair me","max_attempts":5}"#), 5);
    }

    #[test]
    fn aggregate_candidate_diff_survives_a_repair_pass() {
        let actual = tempfile::tempdir().unwrap();
        let candidate = tempfile::tempdir().unwrap();
        fs::write(actual.path().join("same.txt"), "same").unwrap();
        fs::write(candidate.path().join("same.txt"), "same").unwrap();
        fs::write(candidate.path().join("first-pass.txt"), "kept").unwrap();
        fs::write(candidate.path().join("repair.txt"), "fixed").unwrap();
        assert_eq!(
            changed_files(actual.path(), candidate.path()).unwrap(),
            [PathBuf::from("first-pass.txt"), PathBuf::from("repair.txt")]
        );
    }
}
