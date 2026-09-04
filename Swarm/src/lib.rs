//! Durable, bounded mission and task execution contracts.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_OBJECTIVE_CHARS: usize = 16 * 1024;
const MAX_RESULT_CHARS: usize = 64 * 1024;
const MAX_OWNER_CHARS: usize = 80;
static MISSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Task {
    pub id: u64,
    pub owner: String,
    pub instruction: String,
    pub acceptance: Vec<String>,
    pub attempts: u8,
    pub maximum_attempts: u8,
    pub status: TaskStatus,
    pub result: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Mission {
    pub id: u128,
    pub objective: String,
    pub created_ms: u128,
    pub tasks: VecDeque<Task>,
    pub cancelled: bool,
}

#[derive(Debug, Error)]
pub enum SwarmError {
    #[error("mission objective is empty")]
    EmptyObjective,
    #[error("mission I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("mission JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("task {0} is not pending")]
    NotPending(u64),
}

pub trait TaskExecutor {
    fn execute(&mut self, task: &Task) -> Result<String, String>;
}

impl Mission {
    pub fn new(objective: impl Into<String>) -> Result<Self, SwarmError> {
        Self::new_for("queen", objective)
    }

    pub fn new_for(
        owner: impl Into<String>,
        objective: impl Into<String>,
    ) -> Result<Self, SwarmError> {
        let objective: String = objective
            .into()
            .trim()
            .chars()
            .take(MAX_OBJECTIVE_CHARS)
            .collect();
        if objective.is_empty() {
            return Err(SwarmError::EmptyObjective);
        }
        let owner: String = owner.into().trim().chars().take(MAX_OWNER_CHARS).collect();
        let owner = if owner.is_empty() {
            "queen".to_owned()
        } else {
            owner
        };
        let created_ms = now_ms();
        Ok(Self {
            id: (created_ms << 32) | u128::from(MISSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)),
            objective: objective.clone(),
            created_ms,
            tasks: VecDeque::from([Task {
                id: 1,
                owner,
                instruction: objective,
                acceptance: vec!["A concrete result is recorded".to_owned()],
                attempts: 0,
                maximum_attempts: 3,
                status: TaskStatus::Pending,
                result: None,
            }]),
            cancelled: false,
        })
    }

    pub fn add_task(
        &mut self,
        owner: impl Into<String>,
        instruction: impl Into<String>,
        acceptance: Vec<String>,
    ) -> u64 {
        let id = self.tasks.back().map_or(1, |task| task.id + 1);
        self.tasks.push_back(Task {
            id,
            owner: owner.into(),
            instruction: instruction.into(),
            acceptance,
            attempts: 0,
            maximum_attempts: 3,
            status: TaskStatus::Pending,
            result: None,
        });
        id
    }

    pub fn run_next(&mut self, executor: &mut impl TaskExecutor) -> Result<bool, SwarmError> {
        if self.cancelled {
            return Ok(false);
        }
        let Some(task) = self
            .tasks
            .iter_mut()
            .find(|task| task.status == TaskStatus::Pending)
        else {
            return Ok(false);
        };
        task.status = TaskStatus::Running;
        task.attempts += 1;
        match executor.execute(task) {
            Ok(result) if !result.trim().is_empty() => {
                task.result = Some(result.chars().take(MAX_RESULT_CHARS).collect());
                task.status = TaskStatus::Complete;
            }
            Ok(_) => {
                task.result = Some("executor returned no evidence".to_owned());
                task.status = if task.attempts >= task.maximum_attempts {
                    TaskStatus::Failed
                } else {
                    TaskStatus::Pending
                };
            }
            Err(error) => {
                task.result = Some(error.chars().take(MAX_RESULT_CHARS).collect());
                task.status = if task.attempts >= task.maximum_attempts {
                    TaskStatus::Failed
                } else {
                    TaskStatus::Pending
                };
            }
        }
        Ok(true)
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
        for task in &mut self.tasks {
            if matches!(task.status, TaskStatus::Pending | TaskStatus::Running) {
                task.status = TaskStatus::Cancelled;
            }
        }
    }

    pub fn complete(&self) -> bool {
        !self.cancelled
            && self
                .tasks
                .iter()
                .all(|task| task.status == TaskStatus::Complete)
    }

    pub fn complete_next_with(&mut self, result: impl Into<String>) -> bool {
        let Some(task) = self
            .tasks
            .iter_mut()
            .find(|task| matches!(task.status, TaskStatus::Pending | TaskStatus::Running))
        else {
            return false;
        };
        task.status = TaskStatus::Complete;
        task.result = Some(result.into().chars().take(MAX_RESULT_CHARS).collect());
        true
    }
}

#[derive(Clone, Debug)]
pub struct MissionStore {
    directory: PathBuf,
}

impl MissionStore {
    pub fn new(directory: impl Into<PathBuf>) -> Result<Self, SwarmError> {
        let directory = directory.into();
        fs::create_dir_all(&directory)?;
        Ok(Self { directory })
    }

    pub fn save(&self, mission: &Mission) -> Result<PathBuf, SwarmError> {
        let path = self.directory.join(format!("mission-{}.json", mission.id));
        atomic_json(&path, mission)?;
        Ok(path)
    }

    pub fn load(&self, id: u128) -> Result<Mission, SwarmError> {
        Ok(serde_json::from_slice(&fs::read(
            self.directory.join(format!("mission-{id}.json")),
        )?)?)
    }

    pub fn latest(&self) -> Result<Option<Mission>, SwarmError> {
        let mut latest: Option<Mission> = None;
        for entry in fs::read_dir(&self.directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = fs::read(path) else {
                continue;
            };
            let mission: Mission = match serde_json::from_slice(&bytes) {
                Ok(mission) => mission,
                Err(_) => continue,
            };
            if latest
                .as_ref()
                .is_none_or(|current| mission.created_ms > current.created_ms)
            {
                latest = Some(mission);
            }
        }
        Ok(latest)
    }
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), SwarmError> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
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

    struct Executor;
    impl TaskExecutor for Executor {
        fn execute(&mut self, task: &Task) -> Result<String, String> {
            Ok(format!("evidence for {}", task.instruction))
        }
    }

    #[test]
    fn mission_completes_only_with_recorded_evidence() {
        let mut mission = Mission::new("Build a useful thing").unwrap();
        mission.run_next(&mut Executor).unwrap();
        assert!(mission.complete());
        assert!(
            mission.tasks[0]
                .result
                .as_deref()
                .unwrap()
                .contains("evidence")
        );
    }

    #[test]
    fn a_specialist_can_own_the_root_task() {
        let mission = Mission::new_for("alchemist", "Build a bounded experiment").unwrap();
        assert_eq!(mission.tasks[0].owner, "alchemist");
    }
}
