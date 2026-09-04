//! Crash-tolerant, append-only job queue.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_DEPTH: u8 = 3;
pub const MAX_CHILDREN: usize = 5;
pub const MAX_ATTEMPTS: u8 = 2;
const MAX_SKILL_CHARS: usize = 80;
const MAX_INPUT_CHARS: usize = 64 * 1024;
const MAX_FEEDBACK: usize = 32;
const LOCK_STALE_AFTER: Duration = Duration::from_secs(300);
static JOB_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Blocked,
    Cancelled,
}

impl JobStatus {
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Blocked | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Job {
    pub id: String,
    pub skill: String,
    pub input: String,
    pub status: JobStatus,
    pub parent: Option<String>,
    pub depth: u8,
    pub attempts: u8,
    #[serde(default)]
    pub feedback: Vec<String>,
    pub created_ms: u128,
    pub updated_ms: u128,
    #[serde(default)]
    pub output: Option<String>,
}

impl Job {
    pub fn add_feedback(&mut self, feedback: impl Into<String>) {
        self.feedback
            .push(feedback.into().chars().take(2_000).collect());
        if self.feedback.len() > MAX_FEEDBACK {
            self.feedback.drain(..self.feedback.len() - MAX_FEEDBACK);
        }
    }
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("queue I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("queue JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("queue lock remained busy")]
    Busy,
    #[error("skill name is empty")]
    EmptySkill,
    #[error("job input is empty")]
    EmptyInput,
    #[error("job depth {0} exceeds the maximum of {MAX_DEPTH}")]
    Depth(u8),
    #[error("parent job does not exist: {0}")]
    MissingParent(String),
    #[error("parent job already has the maximum of {MAX_CHILDREN} children: {0}")]
    TooManyChildren(String),
    #[error("job does not exist: {0}")]
    MissingJob(String),
    #[error("job is not running: {0}")]
    NotRunning(String),
}

#[derive(Clone, Debug)]
pub struct JobQueue {
    root: PathBuf,
    jobs: PathBuf,
    journal: PathBuf,
    lock: PathBuf,
}

impl JobQueue {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, QueueError> {
        let root = root.into();
        let jobs = root.join("jobs");
        fs::create_dir_all(&jobs)?;
        let journal = jobs.join("jobs.jsonl");
        let lock = jobs.join(".lock");
        Ok(Self {
            root,
            jobs,
            journal,
            lock,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn add(
        &self,
        skill: impl Into<String>,
        input: impl Into<String>,
        parent: Option<&str>,
        depth: u8,
    ) -> Result<Job, QueueError> {
        let skill: String = skill.into().trim().chars().take(MAX_SKILL_CHARS).collect();
        let input: String = input.into().trim().chars().take(MAX_INPUT_CHARS).collect();
        if skill.is_empty() {
            return Err(QueueError::EmptySkill);
        }
        if input.is_empty() {
            return Err(QueueError::EmptyInput);
        }
        if depth > MAX_DEPTH {
            return Err(QueueError::Depth(depth));
        }
        let _guard = self.acquire_lock(Duration::from_secs(10))?;
        let jobs = self.read_map_unlocked()?;
        if let Some(parent) = parent {
            if !jobs.contains_key(parent) {
                return Err(QueueError::MissingParent(parent.to_owned()));
            }
            let children = jobs
                .values()
                .filter(|job| job.parent.as_deref() == Some(parent))
                .count();
            if children >= MAX_CHILDREN {
                return Err(QueueError::TooManyChildren(parent.to_owned()));
            }
        }
        let now = now_ms();
        let sequence = JOB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let job = Job {
            id: format!(
                "k-{now}-{:x}-{:06x}",
                std::process::id(),
                sequence & 0xff_ffff
            ),
            skill,
            input,
            status: JobStatus::Queued,
            parent: parent.map(str::to_owned),
            depth,
            attempts: 0,
            feedback: Vec::new(),
            created_ms: now,
            updated_ms: now,
            output: None,
        };
        self.append_unlocked(&job)?;
        Ok(job)
    }

    pub fn claim(&self) -> Result<Option<Job>, QueueError> {
        let _guard = self.acquire_lock(Duration::from_secs(10))?;
        let mut queued = self
            .read_map_unlocked()?
            .into_values()
            .filter(|job| job.status == JobStatus::Queued)
            .collect::<Vec<_>>();
        queued.sort_by(|left, right| {
            left.created_ms
                .cmp(&right.created_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        let Some(mut job) = queued.into_iter().next() else {
            return Ok(None);
        };
        job.status = JobStatus::Running;
        job.updated_ms = now_ms();
        self.append_unlocked(&job)?;
        Ok(Some(job))
    }

    pub fn update(&self, mut job: Job) -> Result<(), QueueError> {
        let _guard = self.acquire_lock(Duration::from_secs(10))?;
        if !self.read_map_unlocked()?.contains_key(&job.id) {
            return Err(QueueError::MissingJob(job.id));
        }
        job.updated_ms = now_ms();
        self.append_unlocked(&job)
    }

    pub fn finish(
        &self,
        mut job: Job,
        status: JobStatus,
        output: Option<String>,
    ) -> Result<Job, QueueError> {
        if job.status != JobStatus::Running {
            return Err(QueueError::NotRunning(job.id));
        }
        job.status = status;
        job.output = output.map(|value| value.chars().take(MAX_INPUT_CHARS).collect());
        job.updated_ms = now_ms();
        self.update(job.clone())?;
        Ok(job)
    }

    pub fn all(&self) -> Result<Vec<Job>, QueueError> {
        let mut jobs = self.read_map()?.into_values().collect::<Vec<_>>();
        jobs.sort_by(|left, right| {
            left.created_ms
                .cmp(&right.created_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(jobs)
    }

    pub fn get(&self, id: &str) -> Result<Option<Job>, QueueError> {
        Ok(self.read_map()?.remove(id))
    }

    pub fn children(&self, parent: &str) -> Result<Vec<Job>, QueueError> {
        Ok(self
            .all()?
            .into_iter()
            .filter(|job| job.parent.as_deref() == Some(parent))
            .collect())
    }

    pub fn workdir(&self, job: &Job) -> Result<PathBuf, QueueError> {
        let path = self.jobs.join(&job.id);
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    pub fn reap_stale(&self, maximum_age: Duration) -> Result<usize, QueueError> {
        let _guard = self.acquire_lock(Duration::from_secs(10))?;
        let cutoff = now_ms().saturating_sub(maximum_age.as_millis());
        let mut changed = 0;
        for mut job in self.read_map_unlocked()?.into_values() {
            if job.status == JobStatus::Running && job.updated_ms < cutoff {
                job.status = JobStatus::Queued;
                job.add_feedback("reaped: worker stopped while the job was running");
                job.updated_ms = now_ms();
                self.append_unlocked(&job)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    pub fn compact(&self) -> Result<(), QueueError> {
        let _guard = self.acquire_lock(Duration::from_secs(10))?;
        let mut jobs = self.read_map_unlocked()?.into_values().collect::<Vec<_>>();
        jobs.sort_by_key(|job| job.created_ms);
        let temporary = self.journal.with_extension("jsonl.tmp");
        let backup = self.journal.with_extension("jsonl.bak");
        {
            let mut file = File::create(&temporary)?;
            for job in jobs {
                serde_json::to_writer(&mut file, &job)?;
                file.write_all(b"\n")?;
            }
            file.flush()?;
            file.sync_all()?;
        }
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        if self.journal.exists() {
            fs::rename(&self.journal, &backup)?;
        }
        if let Err(error) = fs::rename(&temporary, &self.journal) {
            if backup.exists() {
                let _ = fs::rename(&backup, &self.journal);
            }
            return Err(QueueError::Io(error));
        }
        if backup.exists() {
            fs::remove_file(backup)?;
        }
        Ok(())
    }

    fn read_map(&self) -> Result<HashMap<String, Job>, QueueError> {
        let _guard = self.acquire_lock(Duration::from_secs(10))?;
        self.read_map_unlocked()
    }

    fn read_map_unlocked(&self) -> Result<HashMap<String, Job>, QueueError> {
        let jobs = HashMap::new();
        let path = if self.journal.exists() {
            &self.journal
        } else {
            let backup = self.journal.with_extension("jsonl.bak");
            if !backup.exists() {
                return Ok(jobs);
            }
            return Self::read_path(&backup);
        };
        Self::read_path(path)
    }

    fn read_path(path: &Path) -> Result<HashMap<String, Job>, QueueError> {
        let mut jobs = HashMap::new();
        for line in BufReader::new(File::open(path)?).lines() {
            let Ok(line) = line else { continue };
            let Ok(job) = serde_json::from_str::<Job>(&line) else {
                continue;
            };
            if !job.id.trim().is_empty() {
                jobs.insert(job.id.clone(), job);
            }
        }
        Ok(jobs)
    }

    fn append_unlocked(&self, job: &Job) -> Result<(), QueueError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal)?;
        serde_json::to_writer(&mut file, job)?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }

    fn acquire_lock(&self, timeout: Duration) -> Result<QueueGuard, QueueError> {
        let deadline = SystemTime::now() + timeout;
        loop {
            let token = format!("{}-{}", std::process::id(), now_ms());
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.lock)
            {
                Ok(mut file) => {
                    file.write_all(token.as_bytes())?;
                    file.flush()?;
                    return Ok(QueueGuard {
                        path: self.lock.clone(),
                        token,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&self.lock) {
                        let _ = fs::remove_file(&self.lock);
                        continue;
                    }
                    if SystemTime::now() >= deadline {
                        return Err(QueueError::Busy);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(QueueError::Io(error)),
            }
        }
    }
}

struct QueueGuard {
    path: PathBuf,
    token: String,
}

impl Drop for QueueGuard {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).ok().as_deref() == Some(self.token.as_str()) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn lock_is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > LOCK_STALE_AFTER)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_valid_append_wins_and_a_torn_line_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let queue = JobQueue::new(root.path()).unwrap();
        let mut job = queue.add("research_local", "question", None, 0).unwrap();
        job.status = JobStatus::Done;
        queue.update(job.clone()).unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .open(root.path().join("jobs/jobs.jsonl"))
            .unwrap();
        file.write_all(b"{torn\n").unwrap();

        let loaded = queue.get(&job.id).unwrap().unwrap();
        assert_eq!(loaded.status, JobStatus::Done);
    }

    #[test]
    fn claiming_is_fifo_and_atomic_at_the_state_boundary() {
        let root = tempfile::tempdir().unwrap();
        let queue = JobQueue::new(root.path()).unwrap();
        let first = queue.add("one", "first", None, 0).unwrap();
        let second = queue.add("two", "second", None, 0).unwrap();
        assert_eq!(queue.claim().unwrap().unwrap().id, first.id);
        assert_eq!(queue.claim().unwrap().unwrap().id, second.id);
        assert!(queue.claim().unwrap().is_none());
    }

    #[test]
    fn recursion_is_bounded_by_depth_and_child_count() {
        let root = tempfile::tempdir().unwrap();
        let queue = JobQueue::new(root.path()).unwrap();
        let parent = queue.add("fanout", "parent", None, 0).unwrap();
        for index in 0..MAX_CHILDREN {
            queue
                .add("child", format!("child {index}"), Some(&parent.id), 1)
                .unwrap();
        }
        assert!(matches!(
            queue.add("child", "overflow", Some(&parent.id), 1),
            Err(QueueError::TooManyChildren(_))
        ));
        assert!(matches!(
            queue.add("child", "too deep", None, MAX_DEPTH + 1),
            Err(QueueError::Depth(_))
        ));
    }

    #[test]
    fn compaction_preserves_latest_state() {
        let root = tempfile::tempdir().unwrap();
        let queue = JobQueue::new(root.path()).unwrap();
        let mut job = queue.add("test", "compact me", None, 0).unwrap();
        job.add_feedback("evidence");
        queue.update(job.clone()).unwrap();
        queue.compact().unwrap();
        let loaded = queue.get(&job.id).unwrap().unwrap();
        assert_eq!(loaded.feedback, vec!["evidence"]);
    }
}
