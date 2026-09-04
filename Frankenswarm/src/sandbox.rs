//! Host-safe launcher for networkless, capability-free Podman/Docker execution.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SandboxRequest {
    pub workspace: PathBuf,
    pub command: String,
    #[serde(default = "podman")]
    pub runtime: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn podman() -> String {
    "podman".into()
}
fn default_timeout() -> u64 {
    60
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SandboxResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("sandbox I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("workspace is not a directory: {0}")]
    Workspace(PathBuf),
    #[error("sandbox command must contain 1-8192 printable characters")]
    Command,
    #[error("sandbox runtime must be podman or docker")]
    Runtime,
}

pub fn run(request: &SandboxRequest) -> Result<SandboxResult, SandboxError> {
    let workspace = std::fs::canonicalize(&request.workspace)
        .map_err(|_| SandboxError::Workspace(request.workspace.clone()))?;
    if !workspace.is_dir() {
        return Err(SandboxError::Workspace(workspace));
    }
    let length = request.command.chars().count();
    if length == 0
        || length > 8_192
        || request.command.chars().any(|character| {
            character == '\0'
                || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        })
    {
        return Err(SandboxError::Command);
    }
    if !matches!(request.runtime.as_str(), "podman" | "docker") {
        return Err(SandboxError::Runtime);
    }
    let image = std::env::var("FRANKENSWARM_SANDBOX_IMAGE")
        .unwrap_or_else(|_| "frankenswarm-sandbox:local".into());
    let mut command = Command::new(&request.runtime);
    command.args([
        "run",
        "--rm",
        "--network",
        "none",
        "--read-only",
        "--cap-drop",
        "all",
        "--security-opt",
        "no-new-privileges",
        "--pids-limit",
        "256",
        "--memory",
        "4g",
        "--cpus",
        "4",
    ]);
    if request.runtime == "podman" {
        command.args(["--userns", "keep-id"]);
    }
    command
        .args(["--tmpfs", "/tmp:rw,size=512m", "-v"])
        .arg(format!("{}:/workspace:rw,Z", workspace.display()))
        .arg(image)
        .arg(&request.command)
        .current_dir(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("sandbox stdout was piped");
    let stderr = child.stderr.take().expect("sandbox stderr was piped");
    let stdout = std::thread::spawn(move || bounded(stdout));
    let stderr = std::thread::spawn(move || bounded(stderr));
    let timeout = Duration::from_secs(request.timeout_seconds.clamp(1, 600));
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(SandboxResult {
                exit_code: status.code(),
                stdout: stdout.join().unwrap_or_default(),
                stderr: stderr.join().unwrap_or_default(),
                timed_out: false,
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(SandboxResult {
                exit_code: None,
                stdout: stdout.join().unwrap_or_default(),
                stderr: stderr.join().unwrap_or_default(),
                timed_out: true,
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn build_image(project_root: &Path, runtime: &str) -> Result<SandboxResult, SandboxError> {
    if !matches!(runtime, "podman" | "docker") {
        return Err(SandboxError::Runtime);
    }
    let sandbox = project_root.join("sandbox");
    if !sandbox.join("Containerfile").is_file() {
        return Err(SandboxError::Workspace(sandbox));
    }
    let output = Command::new(runtime)
        .args(["build", "-t", "frankenswarm-sandbox:local", "-f"])
        .arg(sandbox.join("Containerfile"))
        .arg(&sandbox)
        .output()?;
    Ok(SandboxResult {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout)
            .chars()
            .take(256 * 1024)
            .collect(),
        stderr: String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(256 * 1024)
            .collect(),
        timed_out: false,
    })
}

fn bounded(mut reader: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = reader.by_ref().take(256 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_host_shell_as_runtime() {
        let root = tempfile::tempdir().unwrap();
        let request = SandboxRequest {
            workspace: root.path().into(),
            command: "echo safe".into(),
            runtime: "powershell".into(),
            timeout_seconds: 1,
        };
        assert!(matches!(run(&request), Err(SandboxError::Runtime)));
    }
}
