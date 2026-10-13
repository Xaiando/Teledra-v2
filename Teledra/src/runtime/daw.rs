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
    PythonMusic,
    ToggleAlternation,
    Keep,
    Reject,
    SetCadence { seconds: u64 },
    SetTrackGain { track: usize, gain: f32 },
    SetTrackPan { track: usize, pan: f32 },
}

pub(super) struct OrganistStudio {
    pub(super) session: u64,
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
            session: 0,
            python,
            script: workspace.join("Music").join("daw.py"),
            child: None,
            stdin: None,
            dismissed: false,
        }
    }

    #[cfg(test)]
    pub(super) fn show(
        &mut self,
        score: &Score,
        transport: &str,
        cadence_seconds: u64,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) -> io::Result<()> {
        self.show_at(score, transport, cadence_seconds, 0.0, tools)
    }

    pub(super) fn show_at(
        &mut self,
        score: &Score,
        transport: &str,
        cadence_seconds: u64,
        position_seconds: f64,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) -> io::Result<()> {
        self.ensure_running(tools)?;
        if self.dismissed {
            return Ok(());
        }
        self.send(&serde_json::json!({
            "action": "score",
            "score": score,
            "transport": transport,
            "cadence_seconds": cadence_seconds,
            "position_seconds": position_seconds,
            "analysis": score.composition_report(),
            "theory": score.theory_report(),
            "form_timeline": score.form_timeline(),
            "harmonic_timeline": score.harmonic_timeline(),
            "instrument_catalog": teledra_music::Instrument::ALL.iter()
                .map(|instrument| instrument.info()).collect::<Vec<_>>(),
        }))
    }

    pub(super) fn set_status(&mut self, message: &str) -> io::Result<()> {
        if self.child.is_none() {
            return Ok(());
        }
        self.send(&serde_json::json!({"action": "status", "message": message}))
    }

    pub(super) fn set_alternating(&mut self, enabled: bool) -> io::Result<()> {
        if self.child.is_none() {
            return Ok(());
        }
        self.send(&serde_json::json!({"action": "alternating", "enabled": enabled}))
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
        // A window the operator closed and a window that crashed look the same
        // from here — both are a child that has exited — but they mean opposite
        // things. Closing is a request to stay closed. A crash is a fault, and
        // treating it as a dismissal made the DAW vanish silently: stderr is
        // discarded, nothing was reported, and the studio never came back
        // until someone typed /Music without knowing why it had gone.
        let exit = self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten());
        if let Some(status) = exit {
            if status.success() {
                self.dismissed = true;
            } else {
                let _ = tools.send(ToolReply {
                    source: "Organist Studio",
                    result: Ok(format!(
                        "Organist Studio exited unexpectedly ({status}); it will reopen on the next score"
                    )),
                    court_prompt: None,
                    effect: None,
                });
            }
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
        self.session = self.session.wrapping_add(1);
        let session = self.session;
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
                    effect: Some(ToolEffect::StudioAction { session, action }),
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

    #[cfg(test)]
    pub(super) fn use_probe(&mut self, directory: &Path) -> PathBuf {
        let state = directory.join("daw-state.json");
        let wrapper = directory.join("daw-probe.py");
        std::fs::write(
            &wrapper,
            format!(
                "import sys\nsys.stderr = open({}, 'w', encoding='utf-8', buffering=1)\nsys.path.insert(0, {})\nfrom probe_daw import main\nmain({})\n",
                serde_json::to_string(&state.with_extension("stderr.log")).unwrap(),
                serde_json::to_string(&self.script.parent().unwrap()).unwrap(),
                serde_json::to_string(&state).unwrap(),
            ),
        )
        .unwrap();
        self.script = wrapper;
        state
    }

    pub(super) fn stop(&mut self) {
        self.session = self.session.wrapping_add(1);
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
pub(super) fn kill_process_tree(child: &mut Child) {
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
pub(super) fn kill_process_tree(child: &mut Child) {
    let _ = child.kill();
}

#[cfg(windows)]
pub(super) fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(super) fn hide_console(_command: &mut Command) {}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[track_caller]
    pub(crate) fn wait_state(
        path: &Path,
        predicate: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut last_state = serde_json::Value::Null;
        loop {
            if let Ok(raw) = std::fs::read(path)
                && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw)
            {
                if predicate(&value) {
                    return value;
                }
                last_state = serde_json::json!({
                    "pid": value["pid"],
                    "transport": value["transport"],
                    "commands": value["commands"],
                    "position": value["position"],
                    "title": value["score"]["title"],
                    "first_track_gain": value["score"]["tracks"][0]["gain"],
                });
            }
            assert!(
                std::time::Instant::now() < deadline,
                "DAW did not acknowledge expected state: {}; last state: {last_state}; stderr: {}",
                path.display(),
                std::fs::read_to_string(path.with_extension("stderr.log")).unwrap_or_default()
            );
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }

    #[test]
    #[ignore = "requires installed Python and Tk; opens a hidden workstation"]
    fn operational_studio_hot_reload_dismissal_restart_and_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let mut studio = OrganistStudio::new(crate::python_executable());
        let state = studio.use_probe(directory.path());
        let (tools, _replies) = mpsc::unbounded_channel();
        let mut score = teledra_music::default_score();
        studio.show(&score, "paused", 600, &tools).unwrap();
        let first = wait_state(&state, |s| s["cadence"] == 600);
        score.title = "Hot reload".to_owned();
        studio.show(&score, "muted", 120, &tools).unwrap();
        let second = wait_state(&state, |s| s["score"]["title"] == "Hot reload");
        assert_eq!(first["pid"], second["pid"]);
        assert_eq!(second["transport"], "muted");
        studio
            .send(&serde_json::json!({"action": "probe_close"}))
            .unwrap();
        assert!(wait_for_exit(studio.child.as_mut().unwrap()));
        studio.show(&score, "playing", 120, &tools).unwrap();
        assert!(studio.dismissed && studio.child.is_none());
        studio.reopen();
        studio.show(&score, "paused", 300, &tools).unwrap();
        let reopened = wait_state(&state, |s| s["pid"] != first["pid"]);
        studio
            .send(&serde_json::json!({"action": "probe_crash"}))
            .unwrap();
        assert!(wait_for_exit(studio.child.as_mut().unwrap()));
        studio.show(&score, "paused", 600, &tools).unwrap();
        wait_state(&state, |s| s["pid"] != reopened["pid"]);
        studio.stop();
        assert!(studio.child.is_none());
    }

    #[test]
    fn protocol_accepts_only_typed_bounded_shapes() {
        assert_eq!(
            serde_json::from_str::<DawAction>(r#"{"action":"python_music"}"#).unwrap(),
            DawAction::PythonMusic
        );
        assert_eq!(
            serde_json::from_str::<DawAction>(r#"{"action":"toggle_alternation"}"#).unwrap(),
            DawAction::ToggleAlternation
        );
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

    #[test]
    #[ignore = "requires installed Python; uses a hidden protocol child without audio or a window"]
    fn child_actions_keep_their_session_when_a_window_is_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("studio-session-probe.py");
        std::fs::write(&script,
            "import json,sys\nprint(json.dumps({'action':'python_music'}),flush=True)\nfor line in sys.stdin:\n if json.loads(line).get('action')=='shutdown': break\n").unwrap();
        let mut studio = OrganistStudio::new(crate::python_executable());
        studio.script = script;
        let (tools, mut replies) = mpsc::unbounded_channel();
        studio
            .show(&teledra_music::default_score(), "paused", 300, &tools)
            .unwrap();
        let first_session = studio.session;
        let first = replies.blocking_recv().unwrap();
        assert!(matches!(first.effect, Some(ToolEffect::StudioAction {
            session, action: DawAction::PythonMusic
        }) if session == first_session));
        studio.stop();
        assert_ne!(studio.session, first_session);
        studio
            .show(&teledra_music::default_score(), "paused", 300, &tools)
            .unwrap();
        let second = replies.blocking_recv().unwrap();
        assert!(matches!(second.effect, Some(ToolEffect::StudioAction {
            session, action: DawAction::PythonMusic
        }) if session == studio.session && session != first_session));
        studio.stop();
    }
}
