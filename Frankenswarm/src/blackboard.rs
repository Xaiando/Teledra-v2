//! SQLite blackboard with atomic leases and an explicit task state machine.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id),
  name TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'queued', description TEXT,
  attempts INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS task_dependencies (
  task_id TEXT NOT NULL REFERENCES tasks(id),
  depends_on_task_id TEXT NOT NULL REFERENCES tasks(id),
  PRIMARY KEY(task_id, depends_on_task_id), CHECK(task_id <> depends_on_task_id)
);
CREATE TABLE IF NOT EXISTS agent_leases (
  task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
  agent_role TEXT NOT NULL, acquired_at TEXT NOT NULL, expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS artifacts (
  id TEXT PRIMARY KEY, task_id TEXT REFERENCES tasks(id), path TEXT NOT NULL,
  sha256 TEXT NOT NULL, mime_type TEXT, size_bytes INTEGER NOT NULL,
  producer_role TEXT, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS claims (
  id TEXT PRIMARY KEY, task_id TEXT REFERENCES tasks(id), statement TEXT NOT NULL,
  evidence TEXT, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY, task_id TEXT REFERENCES tasks(id), event_type TEXT NOT NULL,
  payload TEXT, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_entries (
  id TEXT PRIMARY KEY, project_id TEXT REFERENCES projects(id), content TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS eval_runs (
  id TEXT PRIMARY KEY, model_name TEXT NOT NULL, config_hash TEXT, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS eval_results (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES eval_runs(id), task_id TEXT NOT NULL,
  score REAL NOT NULL CHECK(score >= 0.0 AND score <= 1.0), details TEXT
);
CREATE INDEX IF NOT EXISTS tasks_project_status ON tasks(project_id, status);
CREATE INDEX IF NOT EXISTS events_task ON events(task_id, created_at);
"#;

#[derive(Debug, Error)]
pub enum BlackboardError {
    #[error("blackboard database failed: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("project does not exist: {0}")]
    MissingProject(String),
    #[error("task does not exist: {0}")]
    MissingTask(String),
    #[error("task is not claimable: {0}")]
    NotClaimable(String),
    #[error("task has incomplete dependencies: {0}")]
    Dependencies(String),
    #[error("invalid task transition: {0} -> {1}")]
    Transition(String, String),
    #[error("name/content must contain 1-4096 characters")]
    InvalidText,
}

#[derive(Clone, Debug)]
pub struct Blackboard {
    path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub status: String,
    pub description: Option<String>,
    pub attempts: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Lease {
    pub task_id: String,
    pub agent_role: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Blackboard {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, BlackboardError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        }
        let board = Self { path };
        let connection = board.connect()?;
        connection.execute_batch(SCHEMA)?;
        Ok(board)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn create_project(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Project, BlackboardError> {
        validate_text(name)?;
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.trim().chars().take(256).collect(),
            description: description.map(|text| text.chars().take(4096).collect()),
            created_at: Utc::now().to_rfc3339(),
        };
        self.connect()?.execute(
            "INSERT INTO projects(id,name,description,created_at) VALUES(?1,?2,?3,?4)",
            params![
                project.id,
                project.name,
                project.description,
                project.created_at
            ],
        )?;
        Ok(project)
    }

    pub fn get_project(&self, id: &str) -> Result<Project, BlackboardError> {
        self.connect()?
            .query_row(
                "SELECT id,name,description,created_at FROM projects WHERE id=?1",
                [id],
                |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| BlackboardError::MissingProject(id.into()))
    }

    pub fn create_task(
        &self,
        project: &str,
        name: &str,
        description: Option<&str>,
        dependencies: &[String],
    ) -> Result<Task, BlackboardError> {
        validate_text(name)?;
        self.get_project(project)?;
        let now = Utc::now().to_rfc3339();
        let task = Task {
            id: Uuid::new_v4().to_string(),
            project_id: project.into(),
            name: name.trim().chars().take(256).collect(),
            status: "queued".into(),
            description: description.map(|text| text.chars().take(4096).collect()),
            attempts: 0,
            created_at: now.clone(),
            updated_at: now,
        };
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("INSERT INTO tasks(id,project_id,name,status,description,attempts,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,0,?6,?6)", params![task.id, task.project_id, task.name, task.status, task.description, task.created_at])?;
        for dependency in dependencies {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND project_id=?2)",
                params![dependency, project],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(BlackboardError::MissingTask(dependency.clone()));
            }
            transaction.execute(
                "INSERT INTO task_dependencies(task_id,depends_on_task_id) VALUES(?1,?2)",
                params![task.id, dependency],
            )?;
        }
        transaction.commit()?;
        Ok(task)
    }

    pub fn get_task(&self, id: &str) -> Result<Task, BlackboardError> {
        self.connect()?.query_row("SELECT id,project_id,name,status,description,attempts,created_at,updated_at FROM tasks WHERE id=?1", [id], row_task).optional()?.ok_or_else(|| BlackboardError::MissingTask(id.into()))
    }

    pub fn claim_task(
        &self,
        id: &str,
        role: &str,
        duration: Duration,
    ) -> Result<Lease, BlackboardError> {
        validate_text(role)?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        reap_expired_in(&transaction)?;
        let status = transaction
            .query_row("SELECT status FROM tasks WHERE id=?1", [id], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .ok_or_else(|| BlackboardError::MissingTask(id.into()))?;
        if status != "queued" {
            return Err(BlackboardError::NotClaimable(id.into()));
        }
        let incomplete: i64 = transaction.query_row("SELECT COUNT(*) FROM task_dependencies d JOIN tasks t ON t.id=d.depends_on_task_id WHERE d.task_id=?1 AND t.status <> 'done'", [id], |row| row.get(0))?;
        if incomplete > 0 {
            return Err(BlackboardError::Dependencies(id.into()));
        }
        let acquired = Utc::now();
        let seconds = i64::try_from(duration.as_secs().clamp(1, 86_400)).unwrap_or(86_400);
        let expires = acquired + TimeDelta::seconds(seconds);
        let changed = transaction.execute("UPDATE tasks SET status='leased', attempts=attempts+1, updated_at=?2 WHERE id=?1 AND status='queued'", params![id, acquired.to_rfc3339()])?;
        if changed != 1 {
            return Err(BlackboardError::NotClaimable(id.into()));
        }
        transaction.execute("INSERT INTO agent_leases(task_id,agent_role,acquired_at,expires_at) VALUES(?1,?2,?3,?4)", params![id, role, acquired.to_rfc3339(), expires.to_rfc3339()])?;
        event(
            &transaction,
            Some(id),
            "task_claimed",
            &serde_json::json!({"role": role, "expires_at": expires}),
        )?;
        transaction.commit()?;
        Ok(Lease {
            task_id: id.into(),
            agent_role: role.into(),
            acquired_at: acquired,
            expires_at: expires,
        })
    }

    pub fn transition_task(&self, id: &str, next: &str) -> Result<Task, BlackboardError> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row("SELECT status FROM tasks WHERE id=?1", [id], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .ok_or_else(|| BlackboardError::MissingTask(id.into()))?;
        if !allowed_transition(&current, next) {
            return Err(BlackboardError::Transition(current, next.into()));
        }
        transaction.execute(
            "UPDATE tasks SET status=?2,updated_at=?3 WHERE id=?1",
            params![id, next, Utc::now().to_rfc3339()],
        )?;
        if matches!(next, "done" | "failed" | "blocked" | "queued") {
            transaction.execute("DELETE FROM agent_leases WHERE task_id=?1", [id])?;
        }
        event(
            &transaction,
            Some(id),
            "task_transition",
            &serde_json::json!({"from": current, "to": next}),
        )?;
        transaction.commit()?;
        self.get_task(id)
    }

    pub fn reap_expired(&self) -> Result<usize, BlackboardError> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = reap_expired_in(&transaction)?;
        transaction.commit()?;
        Ok(count)
    }

    fn connect(&self) -> Result<Connection, rusqlite::Error> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;",
        )?;
        Ok(connection)
    }
}

fn row_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        status: row.get(3)?,
        description: row.get(4)?,
        attempts: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn validate_text(text: &str) -> Result<(), BlackboardError> {
    let length = text.trim().chars().count();
    if length == 0 || length > 4_096 {
        Err(BlackboardError::InvalidText)
    } else {
        Ok(())
    }
}

fn allowed_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("leased", "running")
            | ("leased", "queued")
            | ("leased", "failed")
            | ("running", "review")
            | ("running", "done")
            | ("running", "failed")
            | ("running", "blocked")
            | ("review", "running")
            | ("review", "done")
            | ("review", "failed")
            | ("failed", "queued")
            | ("blocked", "queued")
    )
}

fn reap_expired_in(transaction: &rusqlite::Transaction<'_>) -> Result<usize, rusqlite::Error> {
    let now = Utc::now().to_rfc3339();
    let mut statement =
        transaction.prepare("SELECT task_id FROM agent_leases WHERE expires_at <= ?1")?;
    let ids = statement
        .query_map([&now], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for id in &ids {
        transaction.execute(
            "UPDATE tasks SET status='queued',updated_at=?2 WHERE id=?1 AND status='leased'",
            params![id, now],
        )?;
        transaction.execute("DELETE FROM agent_leases WHERE task_id=?1", [id])?;
        event(
            transaction,
            Some(id),
            "lease_expired",
            &serde_json::json!({}),
        )?;
    }
    Ok(ids.len())
}

fn event(
    transaction: &rusqlite::Transaction<'_>,
    task: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO events(id,task_id,event_type,payload,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![
            Uuid::new_v4().to_string(),
            task,
            kind,
            payload.to_string(),
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> (tempfile::TempDir, Blackboard, Project) {
        let root = tempfile::tempdir().unwrap();
        let board = Blackboard::open(root.path().join("board.db")).unwrap();
        let project = board.create_project("project", None).unwrap();
        (root, board, project)
    }

    #[test]
    fn atomic_claim_rejects_a_second_agent() {
        let (_root, board, project) = board();
        let task = board.create_task(&project.id, "task", None, &[]).unwrap();
        board
            .claim_task(&task.id, "builder", Duration::from_secs(60))
            .unwrap();
        assert!(matches!(
            board.claim_task(&task.id, "critic", Duration::from_secs(60)),
            Err(BlackboardError::NotClaimable(_))
        ));
    }

    #[test]
    fn state_machine_rejects_skipping_the_lease() {
        let (_root, board, project) = board();
        let task = board.create_task(&project.id, "task", None, &[]).unwrap();
        assert!(matches!(
            board.transition_task(&task.id, "done"),
            Err(BlackboardError::Transition(_, _))
        ));
    }

    #[test]
    fn dependencies_must_finish_before_claim() {
        let (_root, board, project) = board();
        let first = board.create_task(&project.id, "first", None, &[]).unwrap();
        let second = board
            .create_task(&project.id, "second", None, std::slice::from_ref(&first.id))
            .unwrap();
        assert!(matches!(
            board.claim_task(&second.id, "builder", Duration::from_secs(60)),
            Err(BlackboardError::Dependencies(_))
        ));
    }
}
