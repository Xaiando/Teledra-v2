//! Best-effort bridge to Kraken's dedicated local Agent Hub.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::queue::{Job, JobStatus};
use crate::skills;

#[derive(Clone, Debug)]
pub struct HubClient {
    endpoint: String,
    client: Client,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Mission {
    pub id: String,
    pub skill: String,
    pub input: String,
}

impl HubClient {
    pub fn from_environment() -> Result<Self, reqwest::Error> {
        Ok(Self {
            endpoint: std::env::var("KRAKEN_HUB_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:3838".into())
                .trim_end_matches('/')
                .into(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()?,
        })
    }

    pub fn available(&self) -> bool {
        self.client
            .get(format!("{}/api/state", self.endpoint))
            .send()
            .is_ok_and(|response| response.status().is_success())
    }

    pub fn register(&self, name: &str) {
        let _ = self
            .client
            .post(format!("{}/api/agents", self.endpoint))
            .json(&serde_json::json!({
                "name": name, "kind": "kraken-worker", "model": "qwen2.5:7b",
                "capabilities": skills::SKILLS.iter().map(|skill| skill.name).collect::<Vec<_>>()
            }))
            .send();
    }

    pub fn signal(&self, name: &str, body: &str) {
        let _ = self
            .client
            .post(format!("{}/api/messages", self.endpoint))
            .json(&serde_json::json!({
                "channel": "agent_signal", "from_agent": name, "body": body
            }))
            .send();
    }

    pub fn missions(&self) -> Vec<Mission> {
        let Ok(response) = self
            .client
            .get(format!("{}/api/state", self.endpoint))
            .send()
        else {
            return Vec::new();
        };
        let Ok(state) = response.json::<Value>() else {
            return Vec::new();
        };
        let mut missions = Vec::new();
        for message in state
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if message.get("channel").and_then(Value::as_str) != Some("mission")
                || message.get("from_agent").and_then(Value::as_str) != Some("human")
            {
                continue;
            }
            let id = message
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("message");
            let body = message
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let mut current: Option<(String, String)> = None;
            let mut parsed = Vec::new();
            for line in body.lines() {
                let start = line
                    .split_once(':')
                    .filter(|(head, _)| skills::skill(head.trim()).is_some());
                if let Some((head, rest)) = start {
                    if let Some(previous) = current.take() {
                        parsed.push(previous);
                    }
                    current = Some((head.trim().into(), rest.trim().into()));
                } else if let Some((_, input)) = current.as_mut() {
                    input.push('\n');
                    input.push_str(line);
                }
            }
            if let Some(previous) = current {
                parsed.push(previous);
            }
            if parsed.is_empty() && !body.is_empty() {
                parsed.push(translate(body));
            }
            for (index, (skill, input)) in parsed
                .into_iter()
                .enumerate()
                .filter(|(_, (_, input))| !input.trim().is_empty())
            {
                missions.push(Mission {
                    id: format!("{id}#{index}"),
                    skill,
                    input,
                });
            }
        }
        missions
    }

    /// Best-effort mirror of a durable Kraken job into the mode hub's task
    /// board. The local job remains authoritative when the optional hub is
    /// offline.
    pub fn mirror_job(&self, root: &Path, job: &Job, worker: &str) {
        let hub = root.join("hub");
        if fs::create_dir_all(&hub).is_err() {
            return;
        }
        let lock = hub.join(".job_tasks.lock");
        let Some(_guard) = acquire_file_lock(&lock, Duration::from_secs(10)) else {
            return;
        };
        let map_path = hub.join("job_tasks.json");
        let mut mapping = fs::read_to_string(&map_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Map<String, Value>>(&text).ok())
            .unwrap_or_default();
        let task_id = if let Some(id) = mapping.get(&job.id).and_then(Value::as_str) {
            id.to_owned()
        } else {
            let created = self
                .client
                .post(format!("{}/api/tasks", self.endpoint))
                .json(&serde_json::json!({
                    "title": format!("{} {}", job.id, job.skill),
                    "description": job.input.replace('\n', " ").chars().take(500).collect::<String>(),
                    "assigned_to": worker,
                    "created_by": worker,
                }))
                .send()
                .ok()
                .and_then(|response| response.json::<Value>().ok())
                .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_owned));
            let Some(id) = created else { return };
            mapping.insert(job.id.clone(), Value::String(id.clone()));
            let temporary = map_path.with_extension("json.tmp");
            if fs::write(
                &temporary,
                serde_json::to_string_pretty(&mapping).unwrap_or_default() + "\n",
            )
            .is_ok()
            {
                let _ = fs::rename(temporary, &map_path);
            }
            id
        };
        let status = match job.status {
            JobStatus::Done => "done",
            JobStatus::Failed | JobStatus::Blocked | JobStatus::Cancelled => "blocked",
            JobStatus::Queued | JobStatus::Running => "in_progress",
        };
        let _ = self
            .client
            .patch(format!("{}/api/tasks/{task_id}", self.endpoint))
            .json(&serde_json::json!({"status":status, "assigned_to":worker}))
            .send();
    }
}

fn translate(body: &str) -> (String, String) {
    let lower = body.to_ascii_lowercase();
    if ["build", "make", "write", "create", "code", "game", "app"]
        .iter()
        .any(|token| lower.contains(token))
    {
        ("code_forge".into(), serde_json::json!({ "task": body, "dir": "translated", "quality": if lower.contains("game") { "beast" } else { "standard" } }).to_string())
    } else if ["latest", "current", "online", "internet"]
        .iter()
        .any(|token| lower.contains(token))
    {
        ("research_web".into(), body.into())
    } else {
        ("research_local".into(), body.into())
    }
}

pub fn claim_mission(root: &Path, mission: &str) -> std::io::Result<bool> {
    let hub = root.join("hub");
    fs::create_dir_all(&hub)?;
    let lock = hub.join(".missions.lock");
    let guard = acquire_file_lock(&lock, Duration::from_secs(10))
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "mission lock busy"))?;
    let path = hub.join("missions_seen.json");
    let mut seen = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<BTreeSet<String>>(&text).ok())
        .unwrap_or_default();
    let claimed = seen.insert(mission.into());
    if claimed {
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_string_pretty(&seen)? + "\n")?;
        fs::rename(temporary, &path)?;
    }
    drop(guard);
    Ok(claimed)
}

struct FileLock {
    path: PathBuf,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_file_lock(path: &Path, timeout: Duration) -> Option<FileLock> {
    let deadline = Instant::now() + timeout;
    loop {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(_) => return Some(FileLock { path: path.into() }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(60));
                if stale {
                    let _ = fs::remove_file(path);
                    continue;
                }
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

pub fn pidfile(root: &Path, name: &str) -> PathBuf {
    root.join("hub").join(format!("{name}.pid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_each_mission_once() {
        let root = tempfile::tempdir().unwrap();
        assert!(claim_mission(root.path(), "m1").unwrap());
        assert!(!claim_mission(root.path(), "m1").unwrap());
    }
}
