use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use serde::Deserialize;
use teledra_music::Score;
use tokio::sync::mpsc;

use super::{ToolEffect, ToolReply};

/// The complete allow-listed control surface emitted by Organist Studio.
/// Free-form commands never cross the Python/Rust boundary.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum DawAction {
    Play,
    Pause,
    Stop,
    Evolve,
    NewMovement,
    Keep,
    Reject,
    SetCadence { seconds: u64 },
    SetTrackGain { track: usize, gain: f32 },
    SetTrackPan { track: usize, pan: f32 },
}

pub(super) struct OrganistStudio {
    python: PathBuf,
    script: PathBuf,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    /// Set when the operator closed the window themselves.
    ///
    /// The studio is refreshed on every accepted variation, and a variation
    /// normally arrives every eighteen to thirty-six seconds. Reopening whenever the child
    /// was missing meant closing the window bought under a minute of peace
    /// before it came back. A window the operator closed stays closed until
    /// they ask for it again.
    dismissed: bool,
}

impl OrganistStudio {
    pub(super) fn new(python: PathBuf) -> Self {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest.parent().unwrap_or_else(|| Path::new("."));
        Self {
            python,
            script: workspace.join("Music").join("daw.py"),
            child: None,
            stdin: None,
            dismissed: false,
        }
    }

    pub(super) fn show(
        &mut self,
        score: &Score,
        transport: &str,
        cadence_seconds: u64,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) -> io::Result<()> {
        self.ensure_running(tools)?;
        self.send(&serde_json::json!({
            "action": "score",
            "score": score,
            "transport": transport,
            "cadence_seconds": cadence_seconds,
            "analysis": score.composition_report(),
        }))
    }

    pub(super) fn set_transport(&mut self, state: &str) -> io::Result<()> {
        if self.child.is_none() {
            return Ok(());
        }
        self.send(&serde_json::json!({
            "action": "transport",
            "state": state,
        }))
    }

    pub(super) fn set_cadence(&mut self, seconds: u64) -> io::Result<()> {
        if self.child.is_none() {
            return Ok(());
        }
        self.send(&serde_json::json!({
            "action": "cadence",
            "seconds": seconds,
        }))
    }

    fn ensure_running(&mut self, tools: &mpsc::UnboundedSender<ToolReply>) -> io::Result<()> {
        let stopped = self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_some());
        if stopped {
            // It exited without being asked to, which means the operator closed
            // the window. Nothing reopens it until they say so.
            self.dismissed = true;
            self.stop();
        }
        if self.child.is_some() {
            return Ok(());
        }
        if self.dismissed {
            return Ok(());
        }
        if !self.script.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Organist Studio is missing: {}", self.script.display()),
            ));
        }
        let mut command = Command::new(&self.python);
        command
            .arg(&self.script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        hide_console(&mut command);
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Organist Studio stdin is unavailable",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Organist Studio stdout is unavailable",
            )
        })?;
        let studio_tools = tools.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(action) = serde_json::from_str::<DawAction>(&line) else {
                    continue;
                };
                let _ = studio_tools.send(ToolReply {
                    source: "Organist Studio",
                    result: Ok(format!("DAW requested {action:?}")),
                    court_prompt: None,
                    effect: Some(ToolEffect::DawAction(action)),
                });
            }
        });
        self.child = Some(child);
        self.stdin = Some(stdin);
        Ok(())
    }

    fn send(&mut self, command: &serde_json::Value) -> io::Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Organist Studio is not connected",
            )
        })?;
        serde_json::to_writer(&mut *stdin, command).map_err(io::Error::other)?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    }

    /// Opens the studio again after the operator closed it.
    pub(super) fn reopen(&mut self) {
        self.dismissed = false;
    }

    fn stop(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = writeln!(stdin, "{{\"action\":\"shutdown\"}}");
            let _ = stdin.flush();
        }
        if let Some(mut child) = self.child.take() {
            if !wait_for_exit(&mut child) {
                kill_process_tree(&mut child);
            }
            let _ = child.wait();
        }
    }
}

impl Drop for OrganistStudio {
    fn drop(&mut self) {
        self.stop();
    }
}

fn wait_for_exit(child: &mut Child) -> bool {
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

#[cfg(windows)]
fn kill_process_tree(child: &mut Child) {
    let mut command = Command::new("taskkill");
    command
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let _ = command.status();
}

#[cfg(not(windows))]
fn kill_process_tree(child: &mut Child) {
    let _ = child.kill();
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::DawAction;

    #[test]
    fn protocol_accepts_only_typed_bounded_shapes() {
        assert_eq!(
            serde_json::from_str::<DawAction>(r#"{"action":"play"}"#).unwrap(),
            DawAction::Play
        );
        assert_eq!(
            serde_json::from_str::<DawAction>(
                r#"{"action":"set_track_gain","track":2,"gain":0.42}"#
            )
            .unwrap(),
            DawAction::SetTrackGain {
                track: 2,
                gain: 0.42
            }
        );
        assert!(
            serde_json::from_str::<DawAction>(r#"{"action":"shell","command":"calc"}"#).is_err()
        );
    }
}
