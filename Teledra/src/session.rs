//! Read-only snapshots shared with presentation-only TUI views.

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SnapshotTurn {
    pub user: String,
    pub assistant: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SnapshotMessage {
    pub sender: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SnapshotEvent {
    pub source: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SessionSnapshot {
    pub active_slug: String,
    pub active_title: String,
    pub notice: String,
    pub pending: Option<String>,
    pub turns: Vec<SnapshotTurn>,
    #[serde(default)]
    pub messages: Vec<SnapshotMessage>,
    #[serde(default)]
    pub private_events: Vec<SnapshotEvent>,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub night_desk: bool,
    #[serde(default)]
    pub pending_proposals: usize,
    #[serde(default)]
    pub workshop_experiments: usize,
    /// What the console granted this session, in plain words.
    #[serde(default)]
    pub terminal: String,
}

#[derive(Debug)]
pub struct SnapshotPublisher {
    path: PathBuf,
    last: RefCell<Vec<u8>>,
}

impl SnapshotPublisher {
    pub fn for_current_process() -> Self {
        Self {
            path: std::env::temp_dir()
                .join(format!("teledra-v2-session-{}.json", std::process::id())),
            last: RefCell::new(Vec::new()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn publish(&self, snapshot: &SessionSnapshot) -> io::Result<()> {
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
        if *self.last.borrow() == bytes {
            return Ok(());
        }
        fs::write(&temporary, &bytes)?;
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::rename(temporary, &self.path)?;
        *self.last.borrow_mut() = bytes;
        Ok(())
    }
}

impl Drop for SnapshotPublisher {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(self.path.with_extension("json.tmp"));
    }
}

pub fn read_snapshot(path: &Path) -> io::Result<SessionSnapshot> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

#[cfg(windows)]
pub fn launch_vertical_view(snapshot: &Path) -> io::Result<()> {
    launch_view(snapshot, "--vertical-view")
}

#[cfg(windows)]
pub fn launch_dashboard_view(snapshot: &Path) -> io::Result<()> {
    launch_view(snapshot, "--dashboard-view")
}

#[cfg(windows)]
pub fn launch_work_view(snapshot: &Path) -> io::Result<()> {
    launch_view(snapshot, "--work-view")
}

#[cfg(windows)]
fn launch_view(snapshot: &Path, flag: &str) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let conhost = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("conhost.exe");
    std::process::Command::new(conhost)
        .arg(executable)
        .arg(flag)
        .arg(snapshot)
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
pub fn launch_vertical_view(_snapshot: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "automatic second-terminal launch is currently supported on Windows",
    ))
}

#[cfg(not(windows))]
pub fn launch_dashboard_view(_snapshot: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "automatic dashboard launch is currently supported on Windows",
    ))
}

#[cfg(not(windows))]
pub fn launch_work_view(_snapshot: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "automatic work-board launch is currently supported on Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_survives_json_round_trip() {
        let snapshot = SessionSnapshot {
            active_slug: "queen".to_owned(),
            active_title: "Teledra".to_owned(),
            notice: "Court ready".to_owned(),
            pending: None,
            turns: vec![SnapshotTurn {
                user: "hello".to_owned(),
                assistant: "Welcome.".to_owned(),
            }],
            messages: vec![SnapshotMessage {
                sender: "Teledra".to_owned(),
                text: "Welcome.".to_owned(),
            }],
            private_events: vec![SnapshotEvent {
                source: "Mission".to_owned(),
                text: "Task one is running".to_owned(),
            }],
            mode: "Streamer".to_owned(),
            night_desk: true,
            pending_proposals: 2,
            workshop_experiments: 3,
            terminal: String::new(),
        };
        let json = serde_json::to_vec(&snapshot).unwrap();
        let restored: SessionSnapshot = serde_json::from_slice(&json).unwrap();
        assert_eq!(restored.turns[0].assistant, "Welcome.");
    }
}
