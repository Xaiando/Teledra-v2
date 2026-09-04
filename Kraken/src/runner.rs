//! Durable Kraken worker loop and child-job orchestration.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

use crate::queue::{Job, JobQueue, JobStatus, QueueError};
use crate::skills::{self, SkillContext, SkillError};

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("queue failed: {0}")]
    Queue(#[from] QueueError),
    #[error("runner I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("runner JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct KrakenRunner {
    queue: JobQueue,
    root: PathBuf,
    workspace: PathBuf,
}

impl KrakenRunner {
    pub fn new(
        root: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
    ) -> Result<Self, RunnerError> {
        let root = root.into();
        let workspace = workspace.into();
        fs::create_dir_all(&root)?;
        fs::create_dir_all(&workspace)?;
        Ok(Self {
            queue: JobQueue::new(&root)?,
            root,
            workspace,
        })
    }

    pub fn queue(&self) -> &JobQueue {
        &self.queue
    }

    pub fn run_once(&self) -> Result<Option<Job>, RunnerError> {
        self.queue.reap_stale(Duration::from_secs(30 * 60))?;
        let Some(mut job) = self.queue.claim()? else {
            return Ok(None);
        };

        if job.skill == "research_fanout" && job.input.starts_with("SYNTH:") {
            match self.prepare_fanout_join(&job)? {
                JoinState::Waiting => {
                    job.status = JobStatus::Queued;
                    job.add_feedback("waiting for research descendants");
                    self.queue.update(job.clone())?;
                    return Ok(Some(job));
                }
                JoinState::Ready(input) => job.input = input,
                JoinState::Failed(reason) => {
                    job.add_feedback(reason.clone());
                    return Ok(Some(self.queue.finish(
                        job,
                        JobStatus::Blocked,
                        Some(reason),
                    )?));
                }
            }
        }

        job.attempts = job.attempts.saturating_add(1);
        self.queue.update(job.clone())?;
        let spec = skills::skill(&job.skill);
        let result = if spec.is_none() {
            Err(SkillError::Unknown(job.skill.clone()))
        } else {
            let workdir = self.queue.workdir(&job)?;
            match SkillContext::new(&self.root, &self.workspace, workdir, &job.id) {
                Ok(context) => skills::execute(
                    &job.skill,
                    &job.input,
                    &context.with_feedback(job.feedback.clone()),
                ),
                Err(error) => Err(error),
            }
        };
        match result {
            Ok(result) => {
                if let Err(error) = skills::verify_result(&job.skill, &result) {
                    let reason = error.to_string();
                    job.add_feedback(&reason);
                    if job.attempts < skills::attempt_limit(&job.skill, &job.input) {
                        job.status = JobStatus::Queued;
                        self.queue.update(job.clone())?;
                        self.journal(&job, "retry", Some(&reason))?;
                        return Ok(Some(job));
                    }
                    self.journal(&job, "failed", Some(&reason))?;
                    return Ok(Some(self.queue.finish(
                        job,
                        JobStatus::Failed,
                        Some(reason),
                    )?));
                }
                let allowed_children = spec.map_or(0, |spec| spec.max_children as usize);
                if result.children.len() > allowed_children {
                    let reason = format!(
                        "skill returned {} children but allows {allowed_children}",
                        result.children.len()
                    );
                    job.add_feedback(&reason);
                    self.journal(&job, "failed", Some(&reason))?;
                    return Ok(Some(self.queue.finish(
                        job,
                        JobStatus::Failed,
                        Some(reason),
                    )?));
                }
                let output = result
                    .output
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| result.summary.clone());
                let finished = self.queue.finish(job, JobStatus::Done, Some(output))?;
                for child in result.children {
                    if skills::skill(&child.skill).is_none() {
                        continue;
                    }
                    if let Err(error) = self.queue.add(
                        child.skill,
                        child.input,
                        Some(&finished.id),
                        finished.depth.saturating_add(1),
                    ) {
                        let mut parent = self
                            .queue
                            .get(&finished.id)?
                            .unwrap_or_else(|| finished.clone());
                        parent.add_feedback(format!("child rejected: {error}"));
                        self.queue.update(parent)?;
                    }
                }
                self.journal(&finished, "done", None)?;
                Ok(Some(finished))
            }
            Err(error) => {
                let reason = error.to_string();
                job.add_feedback(&reason);
                if job.attempts < skills::attempt_limit(&job.skill, &job.input) {
                    job.status = JobStatus::Queued;
                    self.queue.update(job.clone())?;
                    self.journal(&job, "retry", Some(&reason))?;
                    Ok(Some(job))
                } else {
                    self.journal(&job, "failed", Some(&reason))?;
                    Ok(Some(self.queue.finish(
                        job,
                        JobStatus::Failed,
                        Some(reason),
                    )?))
                }
            }
        }
    }

    pub fn run(&self, maximum: usize) -> Result<usize, RunnerError> {
        let mut completed = 0;
        let mut idle_or_waiting = 0;
        for _ in 0..maximum.clamp(1, 10_000) {
            let Some(job) = self.run_once()? else {
                break;
            };
            if job.status.terminal() {
                completed += 1;
                idle_or_waiting = 0;
            } else {
                idle_or_waiting += 1;
            }
            if idle_or_waiting > self.queue.all()?.len().saturating_add(1) {
                break;
            }
        }
        Ok(completed)
    }

    fn prepare_fanout_join(&self, join: &Job) -> Result<JoinState, RunnerError> {
        let root_id = join.input.trim_start_matches("SYNTH:").trim();
        let jobs = self.queue.all()?;
        if !jobs.iter().any(|job| job.id == root_id) {
            return Ok(JoinState::Failed(format!(
                "fanout root does not exist: {root_id}"
            )));
        }
        let by_id = jobs
            .iter()
            .map(|job| (job.id.as_str(), job))
            .collect::<HashMap<_, _>>();
        let descendants = jobs
            .iter()
            .filter(|candidate| {
                candidate.id != join.id && is_descendant(candidate, root_id, &by_id)
            })
            .collect::<Vec<_>>();
        if descendants.iter().any(|job| !job.status.terminal()) {
            return Ok(JoinState::Waiting);
        }
        let useful = descendants.iter().filter(|job| job.status == JobStatus::Done).filter_map(|job| {
            let path = job.output.as_ref().map(PathBuf::from)?;
            path.is_file().then(|| serde_json::json!({ "path": path, "title": format!("{} result {}", job.skill, job.id) }))
        }).collect::<Vec<_>>();
        if useful.is_empty() {
            return Ok(JoinState::Failed(
                "fanout descendants produced no verified reports".into(),
            ));
        }
        let question = by_id
            .get(root_id)
            .map(|job| job.input.clone())
            .unwrap_or_default();
        Ok(JoinState::Ready(
            serde_json::json!({ "question": question, "sources": useful }).to_string(),
        ))
    }

    fn journal(&self, job: &Job, verdict: &str, reason: Option<&str>) -> Result<(), RunnerError> {
        #[derive(Serialize)]
        struct Entry<'a> {
            job: &'a str,
            skill: &'a str,
            verdict: &'a str,
            attempts: u8,
            #[serde(skip_serializing_if = "Option::is_none")]
            reason: Option<&'a str>,
        }
        let directory = self.root.join("journal");
        fs::create_dir_all(&directory)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("kraken.jsonl"))?;
        serde_json::to_writer(
            &mut file,
            &Entry {
                job: &job.id,
                skill: &job.skill,
                verdict,
                attempts: job.attempts,
                reason,
            },
        )?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }
}

enum JoinState {
    Waiting,
    Ready(String),
    Failed(String),
}

fn is_descendant<'a>(job: &'a Job, ancestor: &str, by_id: &HashMap<&'a str, &'a Job>) -> bool {
    let mut current = job.parent.as_deref();
    let mut visited = BTreeSet::new();
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        if !visited.insert(id) {
            return false;
        }
        current = by_id.get(id).and_then(|job| job.parent.as_deref());
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_finishes_a_non_model_tree_job() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("hello.txt"), "hello").unwrap();
        let runner = KrakenRunner::new(root.path(), &workspace).unwrap();
        runner
            .queue()
            .add("coding_mcp", r#"{"op":"tree","path":"."}"#, None, 0)
            .unwrap();
        let job = runner.run_once().unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Done);
    }
}
