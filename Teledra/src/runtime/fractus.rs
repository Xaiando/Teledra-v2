use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use super::ToolReply;

pub(super) struct FractusStudio {
    python: PathBuf,
    script: PathBuf,
    root: PathBuf,
    command_path: PathBuf,
    status_path: PathBuf,
    generation: u64,
    child: Option<Child>,
}

impl FractusStudio {
    pub(super) fn new(python: PathBuf, data_root: &Path) -> Self {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest.parent().unwrap_or_else(|| Path::new("."));
        let root = data_root.join("fractus");
        Self {
            python,
            script: workspace.join("Fractus").join("studio.py"),
            command_path: root.join(".fractus-studio-command.json"),
            status_path: root.join(".fractus-studio-status.json"),
            root,
            generation: 0,
            child: None,
        }
    }

    pub(super) fn show(
        &mut self,
        path: &Path,
        live: bool,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) -> io::Result<()> {
        let root = canonical_directory(&self.root)?;
        let path = path.canonicalize()?;
        if root != path && !path.starts_with(&root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Fractus artifact is outside the managed gallery",
            ));
        }
        match path.extension().and_then(|extension| extension.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("png") => {}
            Some(extension) if extension.eq_ignore_ascii_case("gif") => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Fractus Studio accepts PNG and GIF artifacts only",
                ));
            }
        }
        self.ensure_running()?;
        self.generation = next_command_id(self.generation);
        let presentation_path = presentation_path(&path);
        let command = serde_json::json!({
            "action": "show",
            "command_id": self.generation,
            "path": presentation_path,
            "live": live,
        });
        write_mailbox(&self.command_path, &command)?;
        await_status(self.status_path.clone(), self.generation, tools.clone());
        Ok(())
    }

    fn ensure_running(&mut self) -> io::Result<()> {
        let stopped = self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_some());
        if stopped {
            self.stop();
        }
        if self.child.is_some() {
            return Ok(());
        }
        if !self.script.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Fractus Studio is missing: {}", self.script.display()),
            ));
        }
        fs::create_dir_all(&self.root)?;
        let _ = fs::remove_file(&self.command_path);
        let _ = fs::remove_file(&self.status_path);
        let mut command = Command::new(&self.python);
        command
            .arg(&self.script)
            .arg("--root")
            .arg(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut command);
        self.child = Some(command.spawn()?);
        Ok(())
    }

    fn stop(&mut self) {
        if self.child.is_some() {
            self.generation = next_command_id(self.generation);
            let command = serde_json::json!({
                "action": "shutdown",
                "command_id": self.generation,
            });
            let _ = write_mailbox(&self.command_path, &command);
        }
        if let Some(mut child) = self.child.take() {
            if !wait_for_exit(&mut child) {
                kill_process_tree(&mut child);
            }
            let _ = child.wait();
        }
    }
}

fn write_mailbox(path: &Path, command: &serde_json::Value) -> io::Result<()> {
    let encoded = serde_json::to_vec(command).map_err(io::Error::other)?;
    fs::write(path, encoded)
}

fn presentation_path(path: &Path) -> String {
    let displayed = path.to_string_lossy();
    #[cfg(windows)]
    if let Some(normal) = displayed.strip_prefix(r"\\?\") {
        return normal.to_owned();
    }
    displayed.into_owned()
}

fn next_command_id(previous: u64) -> u64 {
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64;
    clock.max(previous.saturating_add(1))
}

fn await_status(status_path: PathBuf, command_id: u64, tools: mpsc::UnboundedSender<ToolReply>) {
    std::thread::spawn(move || {
        for _ in 0..240 {
            std::thread::sleep(Duration::from_millis(50));
            let Ok(raw) = fs::read_to_string(&status_path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            if value.get("command_id").and_then(serde_json::Value::as_u64) != Some(command_id) {
                continue;
            }
            let event = value
                .get("event")
                .and_then(|value| value.as_str())
                .unwrap_or("error");
            let result = if event == "loaded" {
                let path = value
                    .get("path")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Fractus artifact");
                let frames = value
                    .get("frames")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                Ok(format!("Displayed {path} with {frames} frame(s)"))
            } else {
                Err(value
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown presentation error")
                    .to_owned())
            };
            let _ = tools.send(ToolReply {
                source: "Fractus Studio",
                result,
                court_prompt: None,
                effect: None,
            });
            return;
        }
        let _ = tools.send(ToolReply {
            source: "Fractus Studio",
            result: Err("Studio did not acknowledge the rendered art within 12 seconds".to_owned()),
            court_prompt: None,
            effect: None,
        });
    });
}

impl Drop for FractusStudio {
    fn drop(&mut self) {
        self.stop();
    }
}

fn canonical_directory(path: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(path)?;
    path.canonicalize()
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
