use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use tokio::sync::mpsc;

use super::{ToolEffect, ToolReply};

pub(super) struct RestreamListener {
    python: PathBuf,
    script: PathBuf,
    token_path: PathBuf,
    legacy_token_path: PathBuf,
    child: Option<Child>,
}

impl RestreamListener {
    pub(super) fn new(python: PathBuf, data_root: &Path) -> Self {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let legacy_token_path = workspace
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("Teledra")
            .join("config")
            .join("restream_token.txt");
        Self {
            python,
            script: workspace
                .join("Sidecars")
                .join("scripts")
                .join("restream_listener.py"),
            token_path: data_root.join("stream").join("restream_token.txt"),
            legacy_token_path,
            child: None,
        }
    }

    pub(super) fn connect_saved(
        &mut self,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) -> io::Result<bool> {
        let token = fs::read_to_string(&self.token_path)
            .or_else(|_| fs::read_to_string(&self.legacy_token_path))
            .unwrap_or_default();
        if token.trim().is_empty() {
            return Ok(false);
        }
        self.connect(token.trim(), tools, false)?;
        Ok(true)
    }

    pub(super) fn connect(
        &mut self,
        token: &str,
        tools: &mpsc::UnboundedSender<ToolReply>,
        persist: bool,
    ) -> io::Result<()> {
        let token = token.trim();
        if token.len() < 8 || token.len() > 2_048 || token.chars().any(char::is_whitespace) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Restream token is absent or invalid",
            ));
        }
        if !self.script.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Restream listener is missing: {}", self.script.display()),
            ));
        }
        self.stop();
        if persist {
            if let Some(parent) = self.token_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&self.token_path, token)?;
        }

        let mut command = Command::new(&self.python);
        command
            .arg(&self.script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_window(&mut command);
        let mut child = command.spawn()?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "Restream stdin is unavailable")
        })?;
        writeln!(stdin, "{token}")?;
        drop(stdin);

        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "Restream stdout is unavailable")
        })?;
        let audience_tools = tools.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                let author = value
                    .get("author")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Audience")
                    .chars()
                    .take(80)
                    .collect::<String>();
                let text = value
                    .get("text")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .chars()
                    .take(2_000)
                    .collect::<String>();
                if text.is_empty() {
                    continue;
                }
                let _ = audience_tools.send(ToolReply {
                    source: "Audience",
                    result: Ok(format!("Restream message received from {author}")),
                    court_prompt: None,
                    effect: Some(ToolEffect::Audience {
                        messages: vec![(author, text)],
                    }),
                });
            }
        });

        let stderr = child.stderr.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "Restream stderr is unavailable")
        })?;
        let status_tools = tools.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .take(40)
            {
                let _ = status_tools.send(ToolReply {
                    source: "Restream",
                    result: Ok(line.chars().take(500).collect()),
                    court_prompt: None,
                    effect: None,
                });
            }
        });
        self.child = Some(child);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for RestreamListener {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}
