//! Live audience chat from streaming services.
//!
//! Each service has a small Python sidecar that holds one websocket open and
//! writes one `{author, text[, platform]}` JSON object per chat message. The
//! service's secret (a Restream token, a Social Stream Ninja session ID) is
//! passed on stdin, never on a command line, and is saved only in the court's
//! own data folder so it reconnects on every launch.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use tokio::sync::mpsc;

use super::{ToolEffect, ToolReply};

/// What differs between chat services.
struct Service {
    /// Shown in status events and notices.
    label: &'static str,
    script: &'static str,
    /// File under `<data>/stream/` that keeps the secret between launches.
    secret_file: &'static str,
    /// What the secret is called in messages.
    secret_name: &'static str,
    valid: fn(&str) -> bool,
}

const RESTREAM: Service = Service {
    label: "Restream",
    script: "restream_listener.py",
    secret_file: "restream_token.txt",
    secret_name: "token",
    valid: |token| (8..=2_048).contains(&token.len()) && !token.chars().any(char::is_whitespace),
};

const SOCIAL_STREAM: Service = Service {
    label: "Social Stream",
    script: "socialstream_listener.py",
    secret_file: "socialstream_session.txt",
    secret_name: "session ID",
    valid: valid_social_stream_session,
};

/// SSN session IDs become part of the websocket URL path, so only the
/// characters SSN itself generates are accepted.
pub(crate) fn valid_social_stream_session(session: &str) -> bool {
    (4..=200).contains(&session.len())
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub(super) struct ChatListener {
    service: &'static Service,
    python: PathBuf,
    script: PathBuf,
    secret_path: PathBuf,
    legacy_secret_path: Option<PathBuf>,
    child: Option<Child>,
}

impl ChatListener {
    pub(super) fn restream(python: PathBuf, data_root: &Path) -> Self {
        let mut listener = Self::new(&RESTREAM, python, data_root);
        // Teledra V1 kept the token beside the old application.
        listener.legacy_secret_path = Some(
            workspace()
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("Teledra")
                .join("config")
                .join("restream_token.txt"),
        );
        listener
    }

    pub(super) fn social_stream(python: PathBuf, data_root: &Path) -> Self {
        Self::new(&SOCIAL_STREAM, python, data_root)
    }

    fn new(service: &'static Service, python: PathBuf, data_root: &Path) -> Self {
        Self {
            service,
            python,
            script: workspace()
                .join("Sidecars")
                .join("scripts")
                .join(service.script),
            secret_path: data_root.join("stream").join(service.secret_file),
            legacy_secret_path: None,
            child: None,
        }
    }

    pub(super) fn label(&self) -> &'static str {
        self.service.label
    }

    /// Reconnects with the saved secret; false when none has been saved.
    pub(super) fn connect_saved(
        &mut self,
        tools: &mpsc::UnboundedSender<ToolReply>,
    ) -> io::Result<bool> {
        let secret = fs::read_to_string(&self.secret_path)
            .or_else(|error| match &self.legacy_secret_path {
                Some(legacy) => fs::read_to_string(legacy),
                None => Err(error),
            })
            .unwrap_or_default();
        if secret.trim().is_empty() {
            return Ok(false);
        }
        self.connect(secret.trim(), tools, false)?;
        Ok(true)
    }

    pub(super) fn connect(
        &mut self,
        secret: &str,
        tools: &mpsc::UnboundedSender<ToolReply>,
        persist: bool,
    ) -> io::Result<()> {
        let secret = secret.trim();
        let Service {
            label, secret_name, ..
        } = self.service;
        if !(self.service.valid)(secret) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{label} {secret_name} is absent or invalid"),
            ));
        }
        if !self.script.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{label} listener is missing: {}", self.script.display()),
            ));
        }
        self.stop();
        if persist {
            if let Some(parent) = self.secret_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&self.secret_path, secret)?;
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
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("{label} stdin is unavailable"),
            )
        })?;
        writeln!(stdin, "{secret}")?;
        drop(stdin);

        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("{label} stdout is unavailable"),
            )
        })?;
        let audience_tools = tools.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Some((author, text)) = audience_message(&line) else {
                    continue;
                };
                let _ = audience_tools.send(ToolReply {
                    source: "Audience",
                    result: Ok(format!("{label} message received from {author}")),
                    court_prompt: None,
                    effect: Some(ToolEffect::Audience {
                        messages: vec![(author, text)],
                    }),
                });
            }
        });

        let stderr = child.stderr.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("{label} stderr is unavailable"),
            )
        })?;
        let status_tools = tools.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .take(40)
            {
                let _ = status_tools.send(ToolReply {
                    source: label,
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

impl Drop for ChatListener {
    fn drop(&mut self) {
        self.stop();
    }
}

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

/// One sidecar line as (author, text). A platform, when the service names
/// one, stays with the author so the same name on two platforms is two people.
fn audience_message(line: &str) -> Option<(String, String)> {
    let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let mut author = value
        .get("author")
        .and_then(|value| value.as_str())
        .unwrap_or("Audience")
        .chars()
        .take(80)
        .collect::<String>();
    if let Some(platform) = value
        .get("platform")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|platform| !platform.is_empty())
    {
        let platform: String = platform.chars().take(30).collect();
        author = format!("{author} ({platform})");
    }
    let text = value
        .get("text")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .chars()
        .take(2_000)
        .collect::<String>();
    (!text.is_empty()).then_some((author, text))
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audience_lines_keep_the_platform_with_the_author() {
        assert_eq!(
            audience_message(r#"{"author":"Mira","platform":"twitch","text":"hello court"}"#),
            Some(("Mira (twitch)".into(), "hello court".into()))
        );
        assert_eq!(
            audience_message(r#"{"author":"Mira","text":"from restream"}"#),
            Some(("Mira".into(), "from restream".into()))
        );
        assert_eq!(audience_message(r#"{"author":"Mira","text":""}"#), None);
        assert_eq!(audience_message("not json"), None);
    }

    #[test]
    fn social_stream_sessions_are_url_safe() {
        assert!(valid_social_stream_session("TsSR6Biwd8"));
        assert!(valid_social_stream_session("my-court_session"));
        assert!(!valid_social_stream_session("abc"));
        assert!(!valid_social_stream_session("has space"));
        assert!(!valid_social_stream_session("../escape"));
        assert!(!valid_social_stream_session("a/b/c/d"));
    }

    #[test]
    fn each_service_keeps_its_own_saved_secret() {
        let root = tempfile::tempdir().unwrap();
        let restream = ChatListener::restream("python".into(), root.path());
        let social = ChatListener::social_stream("python".into(), root.path());
        assert_ne!(restream.secret_path, social.secret_path);
        assert!(social.script.ends_with("socialstream_listener.py"));
        assert!(social.script.is_file(), "the sidecar ships with the court");
        let (tools, _) = mpsc::unbounded_channel();
        let mut social = social;
        assert!(!social.connect_saved(&tools).unwrap());
        let error = social.connect("bad id!", &tools, true).unwrap_err();
        assert!(error.to_string().contains("session ID"));
        assert!(!social.secret_path.exists(), "an invalid ID is never saved");
    }
}
