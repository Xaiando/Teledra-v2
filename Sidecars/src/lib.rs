//! Typed, bounded ownership of optional Python/device/network sidecars.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SidecarKind {
    Microphone,
    Vision,
    Somatic,
    StreamChat,
    Outreach,
    Treasury,
    Mcp,
    YouTube,
}

impl SidecarKind {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::Vision => "vision",
            Self::Somatic => "somatic",
            Self::StreamChat => "stream_chat",
            Self::Outreach => "outreach",
            Self::Treasury => "treasury",
            Self::Mcp => "mcp",
            Self::YouTube => "youtube",
        }
    }

    fn script(self) -> &'static str {
        match self {
            Self::Microphone => "microphone.py",
            Self::Vision => "vision.py",
            Self::Somatic => "somatic.py",
            Self::StreamChat => "stream_chat.py",
            Self::Outreach => "outreach.py",
            Self::Treasury => "treasury.py",
            Self::Mcp => "mcp.py",
            Self::YouTube => "youtube.py",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SidecarResponse {
    pub ok: bool,
    pub capability: String,
    pub message: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

#[derive(Debug, Error)]
pub enum SidecarError {
    #[error("{0} sidecar is unavailable: {1}")]
    Unavailable(String, String),
    #[error("could not start {0} sidecar: {1}")]
    Start(String, std::io::Error),
    #[error("{0} sidecar timed out")]
    Timeout(String),
    #[error("{0} sidecar I/O failed: {1}")]
    Io(String, std::io::Error),
    #[error("{0} sidecar returned invalid JSON: {1}")]
    Protocol(String, serde_json::Error),
    #[error("{0} sidecar failed: {1}")]
    Failed(String, String),
}

#[derive(Clone, Debug)]
pub struct SidecarManager {
    python: PathBuf,
    scripts: PathBuf,
    timeout: Duration,
    active: Arc<Mutex<HashSet<u32>>>,
    data_root: Option<PathBuf>,
}

impl SidecarManager {
    pub fn bundled(python: impl Into<PathBuf>) -> Self {
        Self {
            python: python.into(),
            scripts: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),
            timeout: Duration::from_secs(30),
            active: Arc::new(Mutex::new(HashSet::new())),
            data_root: None,
        }
    }

    pub fn with_data_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.data_root = Some(root.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.max(Duration::from_secs(1));
        self
    }

    pub fn available(&self, kind: SidecarKind) -> bool {
        (self.python.exists() || self.python.components().count() == 1)
            && self.scripts.join(kind.script()).is_file()
    }

    pub fn run(
        &self,
        kind: SidecarKind,
        arguments: &[String],
        input: Option<&serde_json::Value>,
    ) -> Result<SidecarResponse, SidecarError> {
        let script = self.scripts.join(kind.script());
        if !script.is_file() {
            return Err(SidecarError::Unavailable(
                kind.slug().to_owned(),
                format!("{} is missing", script.display()),
            ));
        }
        let mut command = Command::new(&self.python);
        command.arg(&script).args(arguments);
        if let Some(root) = &self.data_root {
            command.env("TELEDRA_DATA_DIR", root);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| SidecarError::Start(kind.slug().to_owned(), error))?;
        let _active = ActiveProcess::new(child.id(), Arc::clone(&self.active));
        if let (Some(value), Some(mut stdin)) = (input, child.stdin.take()) {
            serde_json::to_writer(&mut stdin, value)
                .map_err(|error| SidecarError::Protocol(kind.slug().to_owned(), error))?;
            stdin
                .flush()
                .map_err(|error| SidecarError::Io(kind.slug().to_owned(), error))?;
        }
        drop(child.stdin.take());
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let out = thread::spawn(move || read_bounded(stdout));
        let err = thread::spawn(move || read_bounded(stderr));
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child
                .try_wait()
                .map_err(|error| SidecarError::Io(kind.slug().to_owned(), error))?
            {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out.join();
                    let _ = err.join();
                    return Err(SidecarError::Timeout(kind.slug().to_owned()));
                }
                None => thread::sleep(Duration::from_millis(20)),
            }
        };
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        drop(_active);
        if !status.success() {
            if let Ok(response) = serde_json::from_slice::<SidecarResponse>(&stdout) {
                return Err(SidecarError::Failed(
                    kind.slug().to_owned(),
                    response.message,
                ));
            }
            return Err(SidecarError::Failed(
                kind.slug().to_owned(),
                String::from_utf8_lossy(&stderr).trim().to_owned(),
            ));
        }
        let response: SidecarResponse = serde_json::from_slice(&stdout)
            .map_err(|error| SidecarError::Protocol(kind.slug().to_owned(), error))?;
        if !response.ok {
            return Err(SidecarError::Failed(
                kind.slug().to_owned(),
                response.message,
            ));
        }
        Ok(response)
    }

    pub fn scripts(&self) -> &Path {
        &self.scripts
    }

    /// Terminates every sidecar still in flight. The application calls this
    /// during shutdown so optional Python integrations cannot become orphans.
    pub fn shutdown(&self) {
        let pids = self
            .active
            .lock()
            .map(|active| active.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for pid in pids {
            terminate_process_tree(pid);
        }
    }
}

struct ActiveProcess {
    pid: u32,
    active: Arc<Mutex<HashSet<u32>>>,
}

impl ActiveProcess {
    fn new(pid: u32, active: Arc<Mutex<HashSet<u32>>>) -> Self {
        if let Ok(mut active) = active.lock() {
            active.insert(pid);
        }
        Self { pid, active }
    }
}

impl Drop for ActiveProcess {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.pid);
        }
    }
}

#[cfg(windows)]
fn terminate_process_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = Command::new("taskkill.exe")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(windows))]
fn terminate_process_tree(pid: u32) {
    let _ = Command::new("kill").arg(pid.to_string()).status();
}

fn read_bounded(mut reader: impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    let _ = reader
        .by_ref()
        .take(MAX_OUTPUT_BYTES as u64)
        .read_to_end(&mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_capability_has_a_stable_slug_and_script() {
        for kind in [
            SidecarKind::Microphone,
            SidecarKind::Vision,
            SidecarKind::Somatic,
            SidecarKind::StreamChat,
            SidecarKind::Outreach,
            SidecarKind::Treasury,
            SidecarKind::Mcp,
            SidecarKind::YouTube,
        ] {
            assert!(!kind.slug().is_empty());
            assert!(kind.script().ends_with(".py"));
        }
    }
}
