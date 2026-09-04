//! Bounded verification profiles for code and generated artifacts.

pub mod contract;
pub mod hub;
pub mod queue;
pub mod runner;
pub mod skills;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Check {
    pub name: String,
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: u128,
}

#[derive(Debug, Error)]
pub enum KrakenError {
    #[error("verification program does not exist: {0}")]
    MissingProgram(PathBuf),
    #[error("verification working directory does not exist: {0}")]
    MissingDirectory(PathBuf),
    #[error("could not start verification: {0}")]
    Start(#[from] std::io::Error),
    #[error("verification timed out: {0}")]
    Timeout(String),
}

pub fn verify(check: &Check) -> Result<CheckResult, KrakenError> {
    if check.program.components().count() > 1 && !check.program.is_file() {
        return Err(KrakenError::MissingProgram(check.program.clone()));
    }
    if !check.working_directory.is_dir() {
        return Err(KrakenError::MissingDirectory(
            check.working_directory.clone(),
        ));
    }
    let started = Instant::now();
    let mut child = Command::new(&check.program)
        .args(&check.arguments)
        .current_dir(&check.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout = std::thread::spawn(move || read_bounded(stdout));
    let stderr = std::thread::spawn(move || read_bounded(stderr));
    let timeout = Duration::from_secs(check.timeout_seconds.clamp(1, 900));
    loop {
        if let Some(status) = child.try_wait()? {
            let stdout = stdout.join().unwrap_or_default();
            let stderr = stderr.join().unwrap_or_default();
            return Ok(CheckResult {
                name: check.name.clone(),
                success: status.success(),
                exit_code: status.code(),
                stdout,
                stderr,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout.join();
            let _ = stderr.join();
            return Err(KrakenError::Timeout(check.name.clone()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn rust_workspace(root: &Path, cargo: impl Into<PathBuf>) -> Vec<Check> {
    let cargo = cargo.into();
    [
        ("format", vec!["fmt", "--all", "--", "--check"]),
        ("tests", vec!["test", "--workspace"]),
        (
            "clippy",
            vec![
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
    ]
    .into_iter()
    .map(|(name, arguments)| Check {
        name: name.to_owned(),
        program: cargo.clone(),
        arguments: arguments.into_iter().map(str::to_owned).collect(),
        working_directory: root.to_path_buf(),
        timeout_seconds: 600,
    })
    .collect()
}

fn read_bounded(mut reader: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = reader.by_ref().take(256 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_profile_has_the_three_release_gates() {
        let checks = rust_workspace(Path::new("."), "cargo");
        assert_eq!(checks.len(), 3);
        assert_eq!(checks[0].name, "format");
    }
}
