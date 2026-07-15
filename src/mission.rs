//! Durable mission and task contracts for Teledra's orchestration layer.
//!
//! This module deliberately has no dependency on the TUI, `Brain`, or court
//! role enum.  The runtime can therefore persist work before it starts an LLM,
//! tool, or speech effect, and can recover that work after a process restart.
//!
//! Integration pattern:
//! 1. Build a [`Mission`] and [`TaskEnvelope`] values.
//! 2. Apply a typed transition (`start_task`, `complete_task`, `fail_task`, ...).
//! 3. Immediately pass the returned [`Transition`] to
//!    [`MissionStore::commit_transition`].
//! 4. On startup call [`MissionStore::load_and_recover`]; eligible tasks that
//!    were `Running` are requeued, while exhausted tasks fail terminally.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MISSION_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn default_schema_version() -> u32 {
    MISSION_SCHEMA_VERSION
}

fn default_max_attempts() -> u32 {
    DEFAULT_MAX_ATTEMPTS
}

fn default_owner() -> String {
    "unassigned".to_string()
}

fn default_role() -> String {
    "unspecified".to_string()
}

fn default_acceptance_criteria() -> Vec<String> {
    vec!["Deliver the stated objective with inspectable evidence.".to_string()]
}

pub fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn trim_nonempty(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let mut output: String = text.chars().take(max_chars - 1).collect();
    output.push('…');
    output
}

#[derive(Debug)]
pub enum MissionError {
    Io(io::Error),
    Json(serde_json::Error),
    Invalid(String),
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
    TaskNotFound(String),
    DuplicateTask(String),
    InvalidTransition {
        task_id: String,
        from: TaskStatus,
        action: &'static str,
    },
    DependencyNotFound {
        task_id: String,
        dependency: String,
    },
    DependenciesIncomplete {
        task_id: String,
        pending: Vec<String>,
    },
    DependencyCycle(Vec<String>),
    AttemptsExhausted {
        task_id: String,
        max_attempts: u32,
    },
    MissingEvidence {
        subject: String,
    },
    FailedEvidenceChecks {
        subject: String,
        checks: Vec<String>,
    },
    MissionNotCompletable {
        pending: Vec<String>,
    },
}

impl fmt::Display for MissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "mission I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "mission JSON failed: {error}"),
            Self::Invalid(detail) => write!(formatter, "invalid mission state: {detail}"),
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "mission schema version {found} is newer than supported version {supported}"
            ),
            Self::TaskNotFound(task_id) => write!(formatter, "task '{task_id}' was not found"),
            Self::DuplicateTask(task_id) => write!(formatter, "task '{task_id}' already exists"),
            Self::InvalidTransition {
                task_id,
                from,
                action,
            } => write!(
                formatter,
                "task '{task_id}' cannot {action} from state {from:?}"
            ),
            Self::DependencyNotFound {
                task_id,
                dependency,
            } => write!(
                formatter,
                "task '{task_id}' refers to missing dependency '{dependency}'"
            ),
            Self::DependenciesIncomplete { task_id, pending } => write!(
                formatter,
                "task '{task_id}' is waiting for dependencies: {}",
                pending.join(", ")
            ),
            Self::DependencyCycle(tasks) => {
                write!(
                    formatter,
                    "task dependency cycle contains: {}",
                    tasks.join(", ")
                )
            }
            Self::AttemptsExhausted {
                task_id,
                max_attempts,
            } => write!(
                formatter,
                "task '{task_id}' exhausted its {max_attempts} permitted attempt(s)"
            ),
            Self::MissingEvidence { subject } => {
                write!(
                    formatter,
                    "'{subject}' cannot complete without positive evidence"
                )
            }
            Self::FailedEvidenceChecks { subject, checks } => write!(
                formatter,
                "'{subject}' cannot complete while checks are failing: {}",
                checks.join(", ")
            ),
            Self::MissionNotCompletable { pending } => write!(
                formatter,
                "mission cannot complete; unfinished tasks: {}",
                pending.join(", ")
            ),
        }
    }
}

impl std::error::Error for MissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for MissionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for MissionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MissionStatus {
    #[default]
    #[serde(rename = "planned", alias = "new", alias = "pending")]
    Planned,
    #[serde(rename = "active", alias = "running", alias = "in_progress")]
    Active,
    #[serde(rename = "completed", alias = "complete", alias = "done")]
    Completed,
    #[serde(rename = "failed", alias = "terminal_failed")]
    Failed,
    #[serde(rename = "cancelled", alias = "canceled")]
    Cancelled,
}

impl MissionStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TaskStatus {
    #[default]
    #[serde(rename = "queued", alias = "new", alias = "pending", alias = "ready")]
    Queued,
    #[serde(rename = "blocked", alias = "waiting")]
    Blocked,
    #[serde(rename = "running", alias = "in_progress")]
    Running,
    #[serde(rename = "retryable", alias = "retry_pending", alias = "retry")]
    Retryable,
    #[serde(rename = "completed", alias = "complete", alias = "done")]
    Completed,
    #[serde(rename = "failed", alias = "terminal_failed")]
    Failed,
    #[serde(rename = "cancelled", alias = "canceled")]
    Cancelled,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FailureDisposition {
    #[default]
    #[serde(rename = "retryable")]
    Retryable,
    #[serde(rename = "terminal")]
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ArtifactEvidence {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub reference: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub detail: String,
}

impl ArtifactEvidence {
    #[allow(dead_code)] // Public constructor retained for future artifact-producing specialists.
    pub fn verified(kind: impl Into<String>, reference: impl Into<String>) -> Self {
        Self {
            kind: kind.into().trim().to_string(),
            reference: reference.into().trim().to_string(),
            digest: None,
            verified: true,
            detail: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CheckEvidence {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub passed: bool,
    #[serde(default)]
    pub detail: String,
}

impl CheckEvidence {
    pub fn passed(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into().trim().to_string(),
            passed: true,
            detail: detail.into().trim().to_string(),
        }
    }

    #[allow(dead_code)] // Negative checks are persisted by richer specialist integrations.
    pub fn failed(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into().trim().to_string(),
            passed: false,
            detail: detail.into().trim().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceEvidence {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub claim: String,
    #[serde(default)]
    pub accessed_at_ms: u64,
}

impl SourceEvidence {
    #[allow(dead_code)] // Research task wiring can attach source evidence without schema changes.
    pub fn new(url: impl Into<String>, title: impl Into<String>, claim: impl Into<String>) -> Self {
        Self {
            url: url.into().trim().to_string(),
            title: title.into().trim().to_string(),
            claim: claim.into().trim().to_string(),
            accessed_at_ms: current_timestamp_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EvidenceBundle {
    #[serde(default)]
    pub artifacts: Vec<ArtifactEvidence>,
    #[serde(default)]
    pub checks: Vec<CheckEvidence>,
    #[serde(default)]
    pub sources: Vec<SourceEvidence>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub produced_at_ms: Option<u64>,
}

impl EvidenceBundle {
    pub fn positive_evidence_count(&self) -> usize {
        let artifacts = self
            .artifacts
            .iter()
            .filter(|item| item.verified && !item.reference.trim().is_empty())
            .count();
        let checks = self
            .checks
            .iter()
            .filter(|item| item.passed && !item.name.trim().is_empty())
            .count();
        let sources = self
            .sources
            .iter()
            .filter(|item| !item.url.trim().is_empty())
            .count();
        artifacts + checks + sources
    }

    pub fn failed_check_names(&self) -> Vec<String> {
        self.checks
            .iter()
            .filter(|item| !item.passed)
            .map(|item| {
                if item.name.trim().is_empty() {
                    "unnamed check".to_string()
                } else {
                    item.name.trim().to_string()
                }
            })
            .collect()
    }

    pub fn validate_for_completion(&self, subject: &str) -> Result<(), MissionError> {
        let failed = self.failed_check_names();
        if !failed.is_empty() {
            return Err(MissionError::FailedEvidenceChecks {
                subject: subject.to_string(),
                checks: failed,
            });
        }
        if self.positive_evidence_count() == 0 {
            return Err(MissionError::MissingEvidence {
                subject: subject.to_string(),
            });
        }
        Ok(())
    }

    fn normalize(&mut self, now_ms: u64) -> bool {
        let mut changed = false;
        self.notes = trim_nonempty(std::mem::take(&mut self.notes));
        if self.positive_evidence_count() > 0 && self.produced_at_ms.is_none() {
            self.produced_at_ms = Some(now_ms);
            changed = true;
        }
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Handoff {
    #[serde(default)]
    pub from_owner: String,
    #[serde(default)]
    pub from_role: String,
    #[serde(default)]
    pub to_owner: String,
    #[serde(default)]
    pub to_role: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub next_action: String,
    #[serde(default)]
    pub context_refs: Vec<String>,
    #[serde(default)]
    pub at_ms: u64,
}

impl Handoff {
    #[allow(dead_code)] // The handoff contract is durable even before every role emits one.
    pub fn new(
        from_owner: impl Into<String>,
        from_role: impl Into<String>,
        to_owner: impl Into<String>,
        to_role: impl Into<String>,
        summary: impl Into<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            from_owner: from_owner.into().trim().to_string(),
            from_role: from_role.into().trim().to_string(),
            to_owner: to_owner.into().trim().to_string(),
            to_role: to_role.into().trim().to_string(),
            summary: summary.into().trim().to_string(),
            next_action: next_action.into().trim().to_string(),
            context_refs: Vec::new(),
            at_ms: current_timestamp_ms(),
        }
    }

    fn normalize(&mut self, now_ms: u64) -> bool {
        let mut changed = false;
        if self.from_owner.trim().is_empty() {
            self.from_owner = default_owner();
            changed = true;
        }
        if self.from_role.trim().is_empty() {
            self.from_role = default_role();
            changed = true;
        }
        if self.to_owner.trim().is_empty() {
            self.to_owner = default_owner();
            changed = true;
        }
        if self.to_role.trim().is_empty() {
            self.to_role = default_role();
            changed = true;
        }
        self.context_refs = trim_nonempty(std::mem::take(&mut self.context_refs));
        if self.at_ms == 0 {
            self.at_ms = now_ms;
            changed = true;
        }
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskFailure {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub disposition: FailureDisposition,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEnvelope {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub mission_id: String,
    #[serde(default)]
    pub objective: String,
    #[serde(default = "default_acceptance_criteria")]
    pub acceptance_criteria: Vec<String>,
    #[serde(default = "default_owner")]
    pub owner: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub compact_synopsis: String,
    #[serde(default)]
    pub handoff: Option<Handoff>,
    #[serde(default)]
    pub evidence: EvidenceBundle,
    #[serde(default)]
    pub last_failure: Option<TaskFailure>,
    #[serde(default)]
    pub priority: u8,
}

impl TaskEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        mission_id: impl Into<String>,
        objective: impl Into<String>,
        acceptance_criteria: Vec<String>,
        owner: impl Into<String>,
        role: impl Into<String>,
        dependencies: Vec<String>,
        max_attempts: u32,
        compact_synopsis: impl Into<String>,
    ) -> Result<Self, MissionError> {
        let now_ms = current_timestamp_ms();
        let id = id.into().trim().to_string();
        let mission_id = mission_id.into().trim().to_string();
        let objective = objective.into().trim().to_string();
        if id.is_empty() {
            return Err(MissionError::Invalid("task id cannot be empty".to_string()));
        }
        if mission_id.is_empty() {
            return Err(MissionError::Invalid(format!(
                "task '{id}' must name its mission"
            )));
        }
        if objective.is_empty() {
            return Err(MissionError::Invalid(format!(
                "task '{id}' objective cannot be empty"
            )));
        }
        let mut task = Self {
            schema_version: MISSION_SCHEMA_VERSION,
            id,
            mission_id,
            objective,
            acceptance_criteria: trim_nonempty(acceptance_criteria),
            owner: owner.into().trim().to_string(),
            role: role.into().trim().to_string(),
            dependencies: trim_nonempty(dependencies),
            status: TaskStatus::Queued,
            attempt: 0,
            max_attempts: max_attempts.max(1),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            started_at_ms: None,
            finished_at_ms: None,
            compact_synopsis: compact_synopsis.into().trim().to_string(),
            handoff: None,
            evidence: EvidenceBundle::default(),
            last_failure: None,
            priority: 0,
        };
        task.normalize_legacy(now_ms, &task.mission_id.clone())?;
        Ok(task)
    }

    fn normalize_legacy(
        &mut self,
        now_ms: u64,
        parent_mission_id: &str,
    ) -> Result<bool, MissionError> {
        if self.schema_version > MISSION_SCHEMA_VERSION {
            return Err(MissionError::UnsupportedVersion {
                found: self.schema_version,
                supported: MISSION_SCHEMA_VERSION,
            });
        }
        let mut changed = false;
        if self.schema_version != MISSION_SCHEMA_VERSION {
            self.schema_version = MISSION_SCHEMA_VERSION;
            changed = true;
        }
        if self.mission_id.trim().is_empty() {
            self.mission_id = parent_mission_id.to_string();
            changed = true;
        }
        if self.acceptance_criteria.is_empty() {
            self.acceptance_criteria = default_acceptance_criteria();
            changed = true;
        } else {
            self.acceptance_criteria = trim_nonempty(std::mem::take(&mut self.acceptance_criteria));
            if self.acceptance_criteria.is_empty() {
                self.acceptance_criteria = default_acceptance_criteria();
            }
        }
        if self.owner.trim().is_empty() {
            self.owner = default_owner();
            changed = true;
        }
        if self.role.trim().is_empty() {
            self.role = default_role();
            changed = true;
        }
        self.dependencies = trim_nonempty(std::mem::take(&mut self.dependencies));
        self.dependencies.sort();
        self.dependencies.dedup();
        if self.max_attempts == 0 {
            self.max_attempts = DEFAULT_MAX_ATTEMPTS;
            changed = true;
        }
        if self.created_at_ms == 0 {
            self.created_at_ms = now_ms;
            changed = true;
        }
        if self.updated_at_ms == 0 {
            self.updated_at_ms = self.created_at_ms;
            changed = true;
        }
        if self.status == TaskStatus::Running && self.attempt == 0 {
            self.attempt = 1;
            changed = true;
        }
        if self.status.is_terminal() && self.finished_at_ms.is_none() {
            self.finished_at_ms = Some(self.updated_at_ms);
            changed = true;
        }
        if self.compact_synopsis.trim().is_empty() {
            self.compact_synopsis = truncate_chars(self.objective.trim(), 600);
            changed = true;
        }
        if let Some(handoff) = self.handoff.as_mut() {
            changed |= handoff.normalize(now_ms);
        }
        changed |= self.evidence.normalize(now_ms);
        Ok(changed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mission {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub objective: String,
    #[serde(default = "default_acceptance_criteria")]
    pub acceptance_criteria: Vec<String>,
    #[serde(default = "default_owner")]
    pub owner: String,
    #[serde(default = "default_role")]
    pub owner_role: String,
    #[serde(default)]
    pub status: MissionStatus,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub compact_synopsis: String,
    #[serde(default)]
    pub handoff: Option<Handoff>,
    #[serde(default)]
    pub evidence: EvidenceBundle,
    #[serde(default)]
    pub tasks: Vec<TaskEnvelope>,
    #[serde(default)]
    pub revision: u64,
}

impl Mission {
    pub fn new(
        id: impl Into<String>,
        objective: impl Into<String>,
        acceptance_criteria: Vec<String>,
        owner: impl Into<String>,
        owner_role: impl Into<String>,
        compact_synopsis: impl Into<String>,
    ) -> Result<Self, MissionError> {
        let now_ms = current_timestamp_ms();
        let id = id.into().trim().to_string();
        let objective = objective.into().trim().to_string();
        if id.is_empty() {
            return Err(MissionError::Invalid(
                "mission id cannot be empty".to_string(),
            ));
        }
        if objective.is_empty() {
            return Err(MissionError::Invalid(
                "mission objective cannot be empty".to_string(),
            ));
        }
        let mut mission = Self {
            schema_version: MISSION_SCHEMA_VERSION,
            id,
            objective,
            acceptance_criteria: trim_nonempty(acceptance_criteria),
            owner: owner.into().trim().to_string(),
            owner_role: owner_role.into().trim().to_string(),
            status: MissionStatus::Planned,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            started_at_ms: None,
            finished_at_ms: None,
            compact_synopsis: compact_synopsis.into().trim().to_string(),
            handoff: None,
            evidence: EvidenceBundle::default(),
            tasks: Vec::new(),
            revision: 1,
        };
        mission.normalize_legacy(now_ms)?;
        mission.validate()?;
        Ok(mission)
    }

    pub fn creation_event(&self) -> LifecycleEvent {
        self.make_event(
            LifecycleKind::MissionCreated,
            None,
            None,
            None,
            0,
            format!("Mission created: {}", truncate_chars(&self.objective, 500)),
            self.created_at_ms,
        )
    }

    pub fn normalize_legacy(&mut self, now_ms: u64) -> Result<bool, MissionError> {
        if self.schema_version > MISSION_SCHEMA_VERSION {
            return Err(MissionError::UnsupportedVersion {
                found: self.schema_version,
                supported: MISSION_SCHEMA_VERSION,
            });
        }
        let mut changed = false;
        if self.schema_version != MISSION_SCHEMA_VERSION {
            self.schema_version = MISSION_SCHEMA_VERSION;
            changed = true;
        }
        if self.acceptance_criteria.is_empty() {
            self.acceptance_criteria = default_acceptance_criteria();
            changed = true;
        } else {
            self.acceptance_criteria = trim_nonempty(std::mem::take(&mut self.acceptance_criteria));
            if self.acceptance_criteria.is_empty() {
                self.acceptance_criteria = default_acceptance_criteria();
            }
        }
        if self.owner.trim().is_empty() {
            self.owner = default_owner();
            changed = true;
        }
        if self.owner_role.trim().is_empty() {
            self.owner_role = default_role();
            changed = true;
        }
        if self.created_at_ms == 0 {
            self.created_at_ms = now_ms;
            changed = true;
        }
        if self.updated_at_ms == 0 {
            self.updated_at_ms = self.created_at_ms;
            changed = true;
        }
        if self.status.is_terminal() && self.finished_at_ms.is_none() {
            self.finished_at_ms = Some(self.updated_at_ms);
            changed = true;
        }
        if self.compact_synopsis.trim().is_empty() {
            self.compact_synopsis = truncate_chars(self.objective.trim(), 900);
            changed = true;
        }
        if self.revision == 0 {
            self.revision = 1;
            changed = true;
        }
        if let Some(handoff) = self.handoff.as_mut() {
            changed |= handoff.normalize(now_ms);
        }
        changed |= self.evidence.normalize(now_ms);
        for task in &mut self.tasks {
            changed |= task.normalize_legacy(now_ms, &self.id)?;
        }
        Ok(changed)
    }

    pub fn validate(&self) -> Result<(), MissionError> {
        if self.schema_version > MISSION_SCHEMA_VERSION {
            return Err(MissionError::UnsupportedVersion {
                found: self.schema_version,
                supported: MISSION_SCHEMA_VERSION,
            });
        }
        if self.id.trim().is_empty() {
            return Err(MissionError::Invalid(
                "mission id cannot be empty".to_string(),
            ));
        }
        if self.objective.trim().is_empty() {
            return Err(MissionError::Invalid(
                "mission objective cannot be empty".to_string(),
            ));
        }

        let mut ids = BTreeSet::new();
        for task in &self.tasks {
            if task.id.trim().is_empty() {
                return Err(MissionError::Invalid("task id cannot be empty".to_string()));
            }
            if !ids.insert(task.id.clone()) {
                return Err(MissionError::DuplicateTask(task.id.clone()));
            }
            if task.mission_id != self.id {
                return Err(MissionError::Invalid(format!(
                    "task '{}' belongs to mission '{}', not '{}'",
                    task.id, task.mission_id, self.id
                )));
            }
            if task.objective.trim().is_empty() {
                return Err(MissionError::Invalid(format!(
                    "task '{}' objective cannot be empty",
                    task.id
                )));
            }
            if task.max_attempts == 0 || task.attempt > task.max_attempts {
                return Err(MissionError::Invalid(format!(
                    "task '{}' has invalid attempt counter {}/{}",
                    task.id, task.attempt, task.max_attempts
                )));
            }
            if task
                .dependencies
                .iter()
                .any(|dependency| dependency == &task.id)
            {
                return Err(MissionError::DependencyCycle(vec![task.id.clone()]));
            }
            if task.status == TaskStatus::Completed {
                task.evidence.validate_for_completion(&task.id)?;
            }
        }

        for task in &self.tasks {
            for dependency in &task.dependencies {
                if !ids.contains(dependency) {
                    return Err(MissionError::DependencyNotFound {
                        task_id: task.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }
        self.validate_dependency_graph()?;

        if self.status == MissionStatus::Completed {
            let pending: Vec<String> = self
                .tasks
                .iter()
                .filter(|task| task.status != TaskStatus::Completed)
                .map(|task| task.id.clone())
                .collect();
            if !pending.is_empty() {
                return Err(MissionError::MissionNotCompletable { pending });
            }
            self.evidence.validate_for_completion(&self.id)?;
        }
        Ok(())
    }

    fn validate_dependency_graph(&self) -> Result<(), MissionError> {
        let mut indegree: BTreeMap<String, usize> = self
            .tasks
            .iter()
            .map(|task| (task.id.clone(), task.dependencies.len()))
            .collect();
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for task in &self.tasks {
            for dependency in &task.dependencies {
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(task.id.clone());
            }
        }
        let mut ready: Vec<String> = indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut visited = 0usize;
        while let Some(id) = ready.pop() {
            visited += 1;
            if let Some(children) = dependents.get(&id) {
                for child in children {
                    if let Some(count) = indegree.get_mut(child) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            ready.push(child.clone());
                        }
                    }
                }
            }
        }
        if visited != self.tasks.len() {
            let cycle = indegree
                .into_iter()
                .filter(|(_, count)| *count > 0)
                .map(|(id, _)| id)
                .collect();
            return Err(MissionError::DependencyCycle(cycle));
        }
        Ok(())
    }

    pub fn task(&self, task_id: &str) -> Option<&TaskEnvelope> {
        self.tasks.iter().find(|task| task.id == task_id)
    }

    pub fn ready_task_ids(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Queued | TaskStatus::Retryable))
            .filter(|task| self.pending_dependencies(task).is_empty())
            .map(|task| task.id.clone())
            .collect()
    }

    pub fn add_task(&mut self, mut task: TaskEnvelope) -> Result<Transition, MissionError> {
        self.ensure_mission_mutable("add a task")?;
        if self.tasks.iter().any(|existing| existing.id == task.id) {
            return Err(MissionError::DuplicateTask(task.id));
        }
        if task.mission_id.trim().is_empty() {
            task.mission_id = self.id.clone();
        }
        if task.mission_id != self.id {
            return Err(MissionError::Invalid(format!(
                "task '{}' belongs to mission '{}', not '{}'",
                task.id, task.mission_id, self.id
            )));
        }
        let now_ms = current_timestamp_ms();
        task.normalize_legacy(now_ms, &self.id)?;
        if !task.dependencies.is_empty() && !self.dependencies_complete(&task.dependencies) {
            task.status = TaskStatus::Blocked;
        }
        let task_id = task.id.clone();
        let status = task.status;
        self.tasks.push(task);
        self.bump_revision(now_ms);
        let event = self.make_event(
            LifecycleKind::TaskAdded,
            Some(&task_id),
            None,
            Some(status),
            0,
            format!("Task added: {task_id}"),
            now_ms,
        );
        Ok(Transition::single(event))
    }

    pub fn start_task(&mut self, task_id: &str) -> Result<Transition, MissionError> {
        self.ensure_mission_mutable("start a task")?;
        let index = self.task_index(task_id)?;
        let from = self.tasks[index].status;
        if !matches!(from, TaskStatus::Queued | TaskStatus::Retryable) {
            return Err(MissionError::InvalidTransition {
                task_id: task_id.to_string(),
                from,
                action: "start",
            });
        }
        let pending = self.pending_dependencies(&self.tasks[index]);
        if !pending.is_empty() {
            return Err(MissionError::DependenciesIncomplete {
                task_id: task_id.to_string(),
                pending,
            });
        }
        if self.tasks[index].attempt >= self.tasks[index].max_attempts {
            return Err(MissionError::AttemptsExhausted {
                task_id: task_id.to_string(),
                max_attempts: self.tasks[index].max_attempts,
            });
        }
        let now_ms = current_timestamp_ms();
        let attempt;
        {
            let task = &mut self.tasks[index];
            task.attempt += 1;
            attempt = task.attempt;
            task.status = TaskStatus::Running;
            task.started_at_ms = Some(now_ms);
            task.finished_at_ms = None;
            task.updated_at_ms = now_ms;
        }
        if self.status == MissionStatus::Planned {
            self.status = MissionStatus::Active;
            self.started_at_ms = Some(now_ms);
        }
        self.bump_revision(now_ms);
        Ok(Transition::single(self.make_event(
            LifecycleKind::TaskStarted,
            Some(task_id),
            Some(from),
            Some(TaskStatus::Running),
            attempt,
            format!("Task started (attempt {attempt})"),
            now_ms,
        )))
    }

    pub fn complete_task(
        &mut self,
        task_id: &str,
        mut evidence: EvidenceBundle,
        compact_synopsis: impl Into<String>,
    ) -> Result<Transition, MissionError> {
        self.ensure_mission_mutable("complete a task")?;
        evidence.validate_for_completion(task_id)?;
        let index = self.task_index(task_id)?;
        let from = self.tasks[index].status;
        if from != TaskStatus::Running {
            return Err(MissionError::InvalidTransition {
                task_id: task_id.to_string(),
                from,
                action: "complete",
            });
        }
        let now_ms = current_timestamp_ms();
        evidence.normalize(now_ms);
        let synopsis = compact_synopsis.into().trim().to_string();
        let attempt;
        {
            let task = &mut self.tasks[index];
            task.status = TaskStatus::Completed;
            task.evidence = evidence;
            if !synopsis.is_empty() {
                task.compact_synopsis = truncate_chars(&synopsis, 1200);
            }
            task.updated_at_ms = now_ms;
            task.finished_at_ms = Some(now_ms);
            task.last_failure = None;
            attempt = task.attempt;
        }
        self.bump_revision(now_ms);
        let mut transition = Transition::single(self.make_event(
            LifecycleKind::TaskCompleted,
            Some(task_id),
            Some(from),
            Some(TaskStatus::Completed),
            attempt,
            format!("Task completed with evidence: {task_id}"),
            now_ms,
        ));
        transition
            .events
            .extend(self.reconcile_blocked_tasks(now_ms));
        Ok(transition)
    }

    pub fn fail_task(
        &mut self,
        task_id: &str,
        code: impl Into<String>,
        message: impl Into<String>,
        requested: FailureDisposition,
    ) -> Result<Transition, MissionError> {
        self.ensure_mission_mutable("fail a task")?;
        let index = self.task_index(task_id)?;
        let from = self.tasks[index].status;
        if from != TaskStatus::Running {
            return Err(MissionError::InvalidTransition {
                task_id: task_id.to_string(),
                from,
                action: "record failure",
            });
        }
        let now_ms = current_timestamp_ms();
        let attempt = self.tasks[index].attempt;
        let exhausted = attempt >= self.tasks[index].max_attempts;
        let actual = if requested == FailureDisposition::Terminal || exhausted {
            FailureDisposition::Terminal
        } else {
            FailureDisposition::Retryable
        };
        let next = if actual == FailureDisposition::Retryable {
            TaskStatus::Retryable
        } else {
            TaskStatus::Failed
        };
        let failure = TaskFailure {
            code: truncate_chars(code.into().trim(), 160),
            message: truncate_chars(message.into().trim(), 2000),
            disposition: actual,
            attempt,
            at_ms: now_ms,
        };
        {
            let task = &mut self.tasks[index];
            task.status = next;
            task.last_failure = Some(failure.clone());
            task.updated_at_ms = now_ms;
            task.started_at_ms = None;
            if next == TaskStatus::Failed {
                task.finished_at_ms = Some(now_ms);
            }
        }
        if next == TaskStatus::Failed {
            self.status = MissionStatus::Failed;
            self.finished_at_ms = Some(now_ms);
        }
        self.bump_revision(now_ms);
        let kind = if next == TaskStatus::Retryable {
            LifecycleKind::TaskFailedRetryable
        } else {
            LifecycleKind::TaskFailedTerminal
        };
        let mut transition = Transition::single(self.make_event(
            kind,
            Some(task_id),
            Some(from),
            Some(next),
            attempt,
            format!("{}: {}", failure.code, failure.message),
            now_ms,
        ));
        if next == TaskStatus::Failed {
            self.bump_revision(now_ms);
            transition.events.push(self.make_event(
                LifecycleKind::MissionFailed,
                None,
                None,
                None,
                attempt,
                format!("Mission failed because task '{task_id}' failed terminally"),
                now_ms,
            ));
        }
        Ok(transition)
    }

    #[allow(dead_code)] // Reserved for explicit cross-role ownership transfers.
    pub fn record_handoff(
        &mut self,
        task_id: &str,
        mut handoff: Handoff,
    ) -> Result<Transition, MissionError> {
        self.ensure_mission_mutable("record a handoff")?;
        let index = self.task_index(task_id)?;
        if handoff.to_owner.trim().is_empty() && handoff.to_role.trim().is_empty() {
            return Err(MissionError::Invalid(format!(
                "handoff for task '{task_id}' needs a destination owner or role"
            )));
        }
        let now_ms = current_timestamp_ms();
        handoff.normalize(now_ms);
        let from_status = self.tasks[index].status;
        let attempt = self.tasks[index].attempt;
        let summary = handoff.summary.clone();
        {
            let task = &mut self.tasks[index];
            task.owner = handoff.to_owner.clone();
            task.role = handoff.to_role.clone();
            task.handoff = Some(handoff);
            task.updated_at_ms = now_ms;
        }
        self.bump_revision(now_ms);
        Ok(Transition::single(self.make_event(
            LifecycleKind::HandoffRecorded,
            Some(task_id),
            Some(from_status),
            Some(from_status),
            attempt,
            format!("Handoff recorded: {}", truncate_chars(&summary, 700)),
            now_ms,
        )))
    }

    #[allow(dead_code)] // Reserved for future long-running mission compaction passes.
    pub fn update_compact_synopsis(
        &mut self,
        synopsis: impl Into<String>,
    ) -> Result<Transition, MissionError> {
        self.ensure_mission_mutable("update its synopsis")?;
        let synopsis = synopsis.into().trim().to_string();
        if synopsis.is_empty() {
            return Err(MissionError::Invalid(
                "mission synopsis cannot be empty".to_string(),
            ));
        }
        let now_ms = current_timestamp_ms();
        self.compact_synopsis = truncate_chars(&synopsis, 2400);
        self.bump_revision(now_ms);
        Ok(Transition::single(self.make_event(
            LifecycleKind::MissionSynopsisUpdated,
            None,
            None,
            None,
            0,
            "Mission synopsis updated".to_string(),
            now_ms,
        )))
    }

    pub fn complete_mission(
        &mut self,
        mut evidence: EvidenceBundle,
        compact_synopsis: impl Into<String>,
    ) -> Result<Transition, MissionError> {
        self.ensure_mission_mutable("complete")?;
        let pending: Vec<String> = self
            .tasks
            .iter()
            .filter(|task| task.status != TaskStatus::Completed)
            .map(|task| task.id.clone())
            .collect();
        if !pending.is_empty() {
            return Err(MissionError::MissionNotCompletable { pending });
        }
        evidence.validate_for_completion(&self.id)?;
        let now_ms = current_timestamp_ms();
        evidence.normalize(now_ms);
        let synopsis = compact_synopsis.into().trim().to_string();
        self.status = MissionStatus::Completed;
        self.evidence = evidence;
        if !synopsis.is_empty() {
            self.compact_synopsis = truncate_chars(&synopsis, 2400);
        }
        self.finished_at_ms = Some(now_ms);
        self.bump_revision(now_ms);
        Ok(Transition::single(self.make_event(
            LifecycleKind::MissionCompleted,
            None,
            None,
            None,
            0,
            "Mission completed with evidence".to_string(),
            now_ms,
        )))
    }

    pub fn cancel_mission(
        &mut self,
        reason: impl Into<String>,
    ) -> Result<Transition, MissionError> {
        self.ensure_mission_mutable("cancel")?;
        let now_ms = current_timestamp_ms();
        let reason = truncate_chars(reason.into().trim(), 1000);
        let mut events = Vec::new();
        for index in 0..self.tasks.len() {
            let from = self.tasks[index].status;
            if from.is_terminal() {
                continue;
            }
            let task_id = self.tasks[index].id.clone();
            let attempt = self.tasks[index].attempt;
            self.tasks[index].status = TaskStatus::Cancelled;
            self.tasks[index].updated_at_ms = now_ms;
            self.tasks[index].finished_at_ms = Some(now_ms);
            self.bump_revision(now_ms);
            events.push(self.make_event(
                LifecycleKind::TaskCancelled,
                Some(&task_id),
                Some(from),
                Some(TaskStatus::Cancelled),
                attempt,
                reason.clone(),
                now_ms,
            ));
        }
        self.status = MissionStatus::Cancelled;
        self.finished_at_ms = Some(now_ms);
        self.bump_revision(now_ms);
        events.push(self.make_event(
            LifecycleKind::MissionCancelled,
            None,
            None,
            None,
            0,
            reason,
            now_ms,
        ));
        Ok(Transition { events })
    }

    pub fn recover_after_restart(&mut self, now_ms: u64) -> RecoveryReport {
        let mut report = RecoveryReport::default();
        let mut terminal_failure = false;
        for index in 0..self.tasks.len() {
            if self.tasks[index].status != TaskStatus::Running {
                continue;
            }
            let task_id = self.tasks[index].id.clone();
            let attempt = self.tasks[index].attempt.max(1);
            let from = TaskStatus::Running;
            let retryable = attempt < self.tasks[index].max_attempts;
            let next = if retryable {
                TaskStatus::Queued
            } else {
                TaskStatus::Failed
            };
            let disposition = if retryable {
                FailureDisposition::Retryable
            } else {
                FailureDisposition::Terminal
            };
            {
                let task = &mut self.tasks[index];
                task.attempt = attempt;
                task.status = next;
                task.started_at_ms = None;
                task.updated_at_ms = now_ms;
                if next == TaskStatus::Failed {
                    task.finished_at_ms = Some(now_ms);
                }
                task.last_failure = Some(TaskFailure {
                    code: "restart_interrupted".to_string(),
                    message: "Process restarted while the task was running.".to_string(),
                    disposition,
                    attempt,
                    at_ms: now_ms,
                });
            }
            self.bump_revision(now_ms);
            if retryable {
                report.requeued.push(task_id.clone());
            } else {
                terminal_failure = true;
                report.terminal.push(task_id.clone());
            }
            report.events.push(self.make_event(
                if retryable {
                    LifecycleKind::TaskRecoveryRequeued
                } else {
                    LifecycleKind::TaskRecoveryFailed
                },
                Some(&task_id),
                Some(from),
                Some(next),
                attempt,
                if retryable {
                    "Interrupted running task requeued after restart".to_string()
                } else {
                    "Interrupted running task exhausted attempts during restart recovery"
                        .to_string()
                },
                now_ms,
            ));
        }
        if terminal_failure && !self.status.is_terminal() {
            self.status = MissionStatus::Failed;
            self.finished_at_ms = Some(now_ms);
            self.bump_revision(now_ms);
            report.events.push(self.make_event(
                LifecycleKind::MissionFailed,
                None,
                None,
                None,
                0,
                "Mission failed during restart recovery".to_string(),
                now_ms,
            ));
        }
        report
    }

    pub fn render_context(&self, budget: ContextBudget) -> String {
        if budget.max_chars == 0 {
            return String::new();
        }
        let mut output = format!(
            "MISSION v{} id={} status={:?} revision={}\nOBJECTIVE: {}\nSYNOPSIS: {}\nOWNER: {} ({})\n",
            self.schema_version,
            self.id,
            self.status,
            self.revision,
            truncate_chars(&self.objective, 900),
            truncate_chars(&self.compact_synopsis, 900),
            self.owner,
            self.owner_role
        );
        output.push_str("ACCEPTANCE:\n");
        for criterion in self.acceptance_criteria.iter().take(budget.max_criteria) {
            output.push_str(&format!("- {}\n", truncate_chars(criterion, 320)));
        }

        let mut tasks: Vec<&TaskEnvelope> = self.tasks.iter().collect();
        tasks.sort_by_key(|task| {
            let state_rank = match task.status {
                TaskStatus::Running => 0,
                TaskStatus::Retryable => 1,
                TaskStatus::Queued => 2,
                TaskStatus::Blocked => 3,
                TaskStatus::Failed => 4,
                TaskStatus::Cancelled => 5,
                TaskStatus::Completed => 6,
            };
            (
                state_rank,
                std::cmp::Reverse(task.priority),
                task.id.as_str(),
            )
        });
        output.push_str("TASKS:\n");
        for task in tasks.into_iter().take(budget.max_tasks) {
            output.push_str(&format!(
                "- [{}] {} owner={} role={} attempt={}/{} deps=[{}]\n  goal={}\n  synopsis={}\n",
                format!("{:?}", task.status).to_lowercase(),
                task.id,
                task.owner,
                task.role,
                task.attempt,
                task.max_attempts,
                task.dependencies.join(","),
                truncate_chars(&task.objective, 360),
                truncate_chars(&task.compact_synopsis, 360),
            ));
            if let Some(handoff) = &task.handoff {
                output.push_str(&format!(
                    "  handoff={}({}) -> {}({}); next={}\n",
                    handoff.from_owner,
                    handoff.from_role,
                    handoff.to_owner,
                    handoff.to_role,
                    truncate_chars(&handoff.next_action, 280)
                ));
            }
            let evidence_items = task
                .evidence
                .positive_evidence_count()
                .min(budget.max_evidence_items);
            if evidence_items > 0 || !task.evidence.failed_check_names().is_empty() {
                output.push_str(&format!(
                    "  evidence=positive:{} failed_checks:{}\n",
                    evidence_items,
                    task.evidence.failed_check_names().join(",")
                ));
            }
            if let Some(failure) = &task.last_failure {
                output.push_str(&format!(
                    "  last_failure={} ({:?}): {}\n",
                    failure.code,
                    failure.disposition,
                    truncate_chars(&failure.message, 300)
                ));
            }
        }
        truncate_chars(&output, budget.max_chars)
    }

    fn ensure_mission_mutable(&self, action: &'static str) -> Result<(), MissionError> {
        if self.status.is_terminal() {
            return Err(MissionError::Invalid(format!(
                "mission '{}' cannot {action} from terminal state {:?}",
                self.id, self.status
            )));
        }
        Ok(())
    }

    fn task_index(&self, task_id: &str) -> Result<usize, MissionError> {
        self.tasks
            .iter()
            .position(|task| task.id == task_id)
            .ok_or_else(|| MissionError::TaskNotFound(task_id.to_string()))
    }

    fn dependencies_complete(&self, dependencies: &[String]) -> bool {
        dependencies.iter().all(|dependency| {
            self.task(dependency)
                .map(|task| task.status == TaskStatus::Completed)
                .unwrap_or(false)
        })
    }

    fn pending_dependencies(&self, task: &TaskEnvelope) -> Vec<String> {
        task.dependencies
            .iter()
            .filter(|dependency| {
                self.task(dependency)
                    .map(|candidate| candidate.status != TaskStatus::Completed)
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    fn reconcile_blocked_tasks(&mut self, now_ms: u64) -> Vec<LifecycleEvent> {
        let mut events = Vec::new();
        for index in 0..self.tasks.len() {
            let from = self.tasks[index].status;
            if !matches!(from, TaskStatus::Queued | TaskStatus::Blocked) {
                continue;
            }
            let dependencies = self.tasks[index].dependencies.clone();
            let desired = if self.dependencies_complete(&dependencies) {
                TaskStatus::Queued
            } else {
                TaskStatus::Blocked
            };
            if from == desired {
                continue;
            }
            let task_id = self.tasks[index].id.clone();
            self.tasks[index].status = desired;
            self.tasks[index].updated_at_ms = now_ms;
            self.bump_revision(now_ms);
            events.push(self.make_event(
                if desired == TaskStatus::Queued {
                    LifecycleKind::TaskUnblocked
                } else {
                    LifecycleKind::TaskBlocked
                },
                Some(&task_id),
                Some(from),
                Some(desired),
                self.tasks[index].attempt,
                if desired == TaskStatus::Queued {
                    "Task dependencies completed; task queued".to_string()
                } else {
                    "Task blocked on dependencies".to_string()
                },
                now_ms,
            ));
        }
        events
    }

    fn bump_revision(&mut self, now_ms: u64) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at_ms = now_ms;
    }

    #[allow(clippy::too_many_arguments)]
    fn make_event(
        &self,
        kind: LifecycleKind,
        task_id: Option<&str>,
        from_status: Option<TaskStatus>,
        to_status: Option<TaskStatus>,
        attempt: u32,
        summary: String,
        at_ms: u64,
    ) -> LifecycleEvent {
        let subject = task_id.unwrap_or("mission");
        LifecycleEvent {
            schema_version: MISSION_SCHEMA_VERSION,
            event_id: format!(
                "{}:{}:{}:{}",
                self.id,
                subject,
                self.revision,
                kind.as_code()
            ),
            mission_id: self.id.clone(),
            task_id: task_id.map(str::to_string),
            kind,
            at_ms,
            attempt,
            from_status,
            to_status,
            summary: truncate_chars(summary.trim(), 1200),
            mission_revision: self.revision,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub max_chars: usize,
    pub max_tasks: usize,
    pub max_criteria: usize,
    pub max_evidence_items: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_chars: 6_000,
            max_tasks: 12,
            max_criteria: 8,
            max_evidence_items: 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleKind {
    #[serde(rename = "mission_created")]
    MissionCreated,
    #[serde(rename = "mission_synopsis_updated")]
    MissionSynopsisUpdated,
    #[serde(rename = "mission_completed")]
    MissionCompleted,
    #[serde(rename = "mission_failed")]
    MissionFailed,
    #[serde(rename = "mission_cancelled")]
    MissionCancelled,
    #[serde(rename = "task_added")]
    TaskAdded,
    #[serde(rename = "task_blocked")]
    TaskBlocked,
    #[serde(rename = "task_unblocked")]
    TaskUnblocked,
    #[serde(rename = "task_started")]
    TaskStarted,
    #[serde(rename = "task_completed")]
    TaskCompleted,
    #[serde(rename = "task_failed_retryable")]
    TaskFailedRetryable,
    #[serde(rename = "task_failed_terminal")]
    TaskFailedTerminal,
    #[serde(rename = "task_cancelled")]
    TaskCancelled,
    #[serde(rename = "task_recovery_requeued")]
    TaskRecoveryRequeued,
    #[serde(rename = "task_recovery_failed")]
    TaskRecoveryFailed,
    #[serde(rename = "handoff_recorded")]
    HandoffRecorded,
}

impl LifecycleKind {
    fn as_code(self) -> &'static str {
        match self {
            Self::MissionCreated => "mission_created",
            Self::MissionSynopsisUpdated => "mission_synopsis_updated",
            Self::MissionCompleted => "mission_completed",
            Self::MissionFailed => "mission_failed",
            Self::MissionCancelled => "mission_cancelled",
            Self::TaskAdded => "task_added",
            Self::TaskBlocked => "task_blocked",
            Self::TaskUnblocked => "task_unblocked",
            Self::TaskStarted => "task_started",
            Self::TaskCompleted => "task_completed",
            Self::TaskFailedRetryable => "task_failed_retryable",
            Self::TaskFailedTerminal => "task_failed_terminal",
            Self::TaskCancelled => "task_cancelled",
            Self::TaskRecoveryRequeued => "task_recovery_requeued",
            Self::TaskRecoveryFailed => "task_recovery_failed",
            Self::HandoffRecorded => "handoff_recorded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleEvent {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub mission_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub kind: LifecycleKind,
    #[serde(default)]
    pub at_ms: u64,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub from_status: Option<TaskStatus>,
    #[serde(default)]
    pub to_status: Option<TaskStatus>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub mission_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Transition {
    pub events: Vec<LifecycleEvent>,
}

impl Transition {
    fn single(event: LifecycleEvent) -> Self {
        Self {
            events: vec![event],
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecoveryReport {
    pub requeued: Vec<String>,
    pub terminal: Vec<String>,
    pub events: Vec<LifecycleEvent>,
}

impl RecoveryReport {
    pub fn transition(&self) -> Transition {
        Transition {
            events: self.events.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MissionStore {
    snapshot_path: PathBuf,
    lifecycle_path: PathBuf,
}

impl MissionStore {
    pub fn new(snapshot_path: impl Into<PathBuf>, lifecycle_path: impl Into<PathBuf>) -> Self {
        Self {
            snapshot_path: snapshot_path.into(),
            lifecycle_path: lifecycle_path.into(),
        }
    }

    pub fn snapshot_path(&self) -> &Path {
        &self.snapshot_path
    }

    #[allow(dead_code)]
    pub fn lifecycle_path(&self) -> &Path {
        &self.lifecycle_path
    }

    pub fn initialize(&self, mission: &Mission) -> Result<(), MissionError> {
        mission.validate()?;
        self.save_snapshot(mission)?;
        self.append_lifecycle(&mission.creation_event())
    }

    /// Persist a fully applied transition. The atomic snapshot is replaced
    /// first, making current state authoritative even if journal I/O later
    /// fails; lifecycle entries are then appended in transition order.
    pub fn commit_transition(
        &self,
        mission: &Mission,
        transition: &Transition,
    ) -> Result<(), MissionError> {
        mission.validate()?;
        for event in &transition.events {
            if event.mission_id != mission.id {
                return Err(MissionError::Invalid(format!(
                    "event '{}' belongs to mission '{}', not '{}'",
                    event.event_id, event.mission_id, mission.id
                )));
            }
            if event.mission_revision > mission.revision {
                return Err(MissionError::Invalid(format!(
                    "event '{}' revision {} is ahead of mission revision {}",
                    event.event_id, event.mission_revision, mission.revision
                )));
            }
        }
        self.save_snapshot(mission)?;
        for event in &transition.events {
            self.append_lifecycle(event)?;
        }
        Ok(())
    }

    pub fn save_snapshot(&self, mission: &Mission) -> Result<(), MissionError> {
        mission.validate()?;
        let parent = self
            .snapshot_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temp_path = temporary_snapshot_path(&self.snapshot_path);
        let write_result = (|| -> Result<(), MissionError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            serde_json::to_writer_pretty(&mut file, mission)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            replace_file_atomically(&temp_path, &self.snapshot_path)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        write_result
    }

    pub fn append_lifecycle(&self, event: &LifecycleEvent) -> Result<(), MissionError> {
        let parent = self
            .lifecycle_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut encoded = serde_json::to_vec(event)?;
        encoded.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.lifecycle_path)?;
        file.write_all(&encoded)?;
        file.sync_data()?;
        Ok(())
    }

    #[allow(dead_code)] // Useful for read-only tooling; runtime uses load_and_recover.
    pub fn load_snapshot(&self) -> Result<Mission, MissionError> {
        let file = File::open(&self.snapshot_path)?;
        let mut mission: Mission = serde_json::from_reader(BufReader::new(file))?;
        mission.normalize_legacy(current_timestamp_ms())?;
        mission.validate()?;
        Ok(mission)
    }

    pub fn load_and_recover(&self) -> Result<(Mission, RecoveryReport), MissionError> {
        let file = File::open(&self.snapshot_path)?;
        let mut mission: Mission = serde_json::from_reader(BufReader::new(file))?;
        let now_ms = current_timestamp_ms();
        let normalized = mission.normalize_legacy(now_ms)?;
        let report = mission.recover_after_restart(now_ms);
        mission.validate()?;
        if report.events.is_empty() {
            if normalized {
                self.save_snapshot(&mission)?;
            }
        } else {
            self.commit_transition(&mission, &report.transition())?;
        }
        Ok((mission, report))
    }

    #[allow(dead_code)] // Useful for read-only tooling; dashboard reads the bounded journal tail.
    pub fn lifecycle_events(&self) -> Result<Vec<LifecycleEvent>, MissionError> {
        if !self.lifecycle_path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&self.lifecycle_path)?;
        let mut events = Vec::new();
        for (line_index, line) in BufReader::new(file).lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event = serde_json::from_str::<LifecycleEvent>(&line).map_err(|error| {
                MissionError::Invalid(format!(
                    "lifecycle line {} is invalid JSON: {}",
                    line_index + 1,
                    error
                ))
            })?;
            events.push(event);
        }
        Ok(events)
    }
}

fn temporary_snapshot_path(snapshot_path: &Path) -> PathBuf {
    let parent = snapshot_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = snapshot_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mission.json");
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        sequence
    ))
}

#[cfg(windows)]
fn replace_file_atomically(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source_wide: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers that live
    // through the call. Paths are on the same volume because the temp file is
    // created beside the destination.
    let result = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file_atomically(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let sequence = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "teledra_mission_{label}_{}_{}",
                std::process::id(),
                sequence
            ));
            fs::create_dir_all(&path).expect("create isolated test directory");
            Self { path }
        }

        fn store(&self) -> MissionStore {
            MissionStore::new(
                self.path.join("mission.json"),
                self.path.join("lifecycle.jsonl"),
            )
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn mission() -> Mission {
        Mission::new(
            "m-1",
            "Upgrade Teledra without losing the active objective",
            vec!["Every task has inspectable evidence".to_string()],
            "operator",
            "sovereign",
            "Durable orchestration upgrade",
        )
        .unwrap()
    }

    fn task(id: &str, dependencies: Vec<String>, max_attempts: u32) -> TaskEnvelope {
        TaskEnvelope::new(
            id,
            "m-1",
            format!("Complete {id}"),
            vec![format!("{id} passes its verifier")],
            "worker",
            "alchemist",
            dependencies,
            max_attempts,
            format!("Work on {id}"),
        )
        .unwrap()
    }

    fn passing_evidence(name: &str) -> EvidenceBundle {
        EvidenceBundle {
            checks: vec![CheckEvidence::passed(name, "verified")],
            ..EvidenceBundle::default()
        }
    }

    #[test]
    fn task_completion_requires_positive_evidence() {
        let mut mission = mission();
        mission.add_task(task("t1", vec![], 2)).unwrap();
        mission.start_task("t1").unwrap();

        let error = mission
            .complete_task("t1", EvidenceBundle::default(), "done")
            .expect_err("empty evidence must be rejected");
        assert!(matches!(error, MissionError::MissingEvidence { .. }));
        assert_eq!(mission.task("t1").unwrap().status, TaskStatus::Running);

        let evidence = EvidenceBundle {
            sources: vec![SourceEvidence::new(
                "https://example.test/source",
                "Primary source",
                "Supports the task result",
            )],
            checks: vec![CheckEvidence::failed("contract", "still broken")],
            ..EvidenceBundle::default()
        };
        let error = mission
            .complete_task("t1", evidence, "done")
            .expect_err("a failed check must override other evidence");
        assert!(matches!(error, MissionError::FailedEvidenceChecks { .. }));
        assert_eq!(mission.task("t1").unwrap().status, TaskStatus::Running);
    }

    #[test]
    fn dependencies_block_then_unblock_after_verified_completion() {
        let mut mission = mission();
        mission.add_task(task("research", vec![], 2)).unwrap();
        mission
            .add_task(task("build", vec!["research".to_string()], 2))
            .unwrap();
        assert_eq!(mission.task("build").unwrap().status, TaskStatus::Blocked);
        assert!(matches!(
            mission.start_task("build"),
            Err(MissionError::InvalidTransition { .. })
        ));

        mission.start_task("research").unwrap();
        let transition = mission
            .complete_task(
                "research",
                passing_evidence("source contract"),
                "Sources captured",
            )
            .unwrap();
        assert!(
            transition
                .events
                .iter()
                .any(|event| event.kind == LifecycleKind::TaskUnblocked)
        );
        assert_eq!(mission.task("build").unwrap().status, TaskStatus::Queued);
        assert_eq!(mission.ready_task_ids(), vec!["build".to_string()]);
    }

    #[test]
    fn retry_budget_turns_failures_terminal_deterministically() {
        let mut mission = mission();
        mission.add_task(task("fragile", vec![], 2)).unwrap();
        mission.start_task("fragile").unwrap();
        let first = mission
            .fail_task(
                "fragile",
                "timeout",
                "first attempt timed out",
                FailureDisposition::Retryable,
            )
            .unwrap();
        assert_eq!(first.events[0].kind, LifecycleKind::TaskFailedRetryable);
        assert_eq!(
            mission.task("fragile").unwrap().status,
            TaskStatus::Retryable
        );

        mission.start_task("fragile").unwrap();
        let second = mission
            .fail_task(
                "fragile",
                "timeout",
                "second attempt timed out",
                FailureDisposition::Retryable,
            )
            .unwrap();
        assert_eq!(second.events[0].kind, LifecycleKind::TaskFailedTerminal);
        assert_eq!(mission.task("fragile").unwrap().status, TaskStatus::Failed);
        assert_eq!(mission.status, MissionStatus::Failed);
    }

    #[test]
    fn mission_completion_requires_tasks_and_mission_evidence() {
        let mut mission = mission();
        mission.add_task(task("t1", vec![], 1)).unwrap();
        assert!(matches!(
            mission.complete_mission(passing_evidence("mission"), "done"),
            Err(MissionError::MissionNotCompletable { .. })
        ));
        mission.start_task("t1").unwrap();
        mission
            .complete_task("t1", passing_evidence("task"), "task done")
            .unwrap();
        assert!(matches!(
            mission.complete_mission(EvidenceBundle::default(), "done"),
            Err(MissionError::MissingEvidence { .. })
        ));
        mission
            .complete_mission(
                EvidenceBundle {
                    artifacts: vec![ArtifactEvidence::verified(
                        "snapshot",
                        "knowledge/missions/m-1.json",
                    )],
                    ..EvidenceBundle::default()
                },
                "All work verified",
            )
            .unwrap();
        assert_eq!(mission.status, MissionStatus::Completed);
        mission.validate().unwrap();
    }

    #[test]
    fn restart_requeues_eligible_running_tasks_and_persists_recovery() {
        let dir = TestDir::new("recovery");
        let store = dir.store();
        let mut mission = mission();
        store.initialize(&mission).unwrap();
        let transition = mission.add_task(task("running", vec![], 3)).unwrap();
        store.commit_transition(&mission, &transition).unwrap();
        let transition = mission.start_task("running").unwrap();
        store.commit_transition(&mission, &transition).unwrap();

        let (recovered, report) = store.load_and_recover().unwrap();
        assert_eq!(report.requeued, vec!["running".to_string()]);
        assert!(report.terminal.is_empty());
        let recovered_task = recovered.task("running").unwrap();
        assert_eq!(recovered_task.status, TaskStatus::Queued);
        assert_eq!(recovered_task.attempt, 1);
        assert_eq!(
            recovered_task.last_failure.as_ref().unwrap().code,
            "restart_interrupted"
        );
        let journal = store.lifecycle_events().unwrap();
        assert!(
            journal
                .iter()
                .any(|event| event.kind == LifecycleKind::TaskRecoveryRequeued)
        );
    }

    #[test]
    fn restart_marks_exhausted_running_task_terminal() {
        let dir = TestDir::new("recovery_exhausted");
        let store = dir.store();
        let mut mission = mission();
        store.initialize(&mission).unwrap();
        let transition = mission.add_task(task("one-shot", vec![], 1)).unwrap();
        store.commit_transition(&mission, &transition).unwrap();
        let transition = mission.start_task("one-shot").unwrap();
        store.commit_transition(&mission, &transition).unwrap();

        let (recovered, report) = store.load_and_recover().unwrap();
        assert_eq!(report.terminal, vec!["one-shot".to_string()]);
        assert_eq!(
            recovered.task("one-shot").unwrap().status,
            TaskStatus::Failed
        );
        assert_eq!(recovered.status, MissionStatus::Failed);
    }

    #[test]
    fn atomic_snapshot_replaces_previous_state_and_journal_appends() {
        let dir = TestDir::new("atomic");
        let store = dir.store();
        let mut mission = mission();
        store.initialize(&mission).unwrap();
        let transition = mission
            .update_compact_synopsis("A newer durable synopsis")
            .unwrap();
        store.commit_transition(&mission, &transition).unwrap();

        let loaded = store.load_snapshot().unwrap();
        assert_eq!(loaded.compact_synopsis, "A newer durable synopsis");
        assert_eq!(store.lifecycle_events().unwrap().len(), 2);
        let leftovers: Vec<_> = fs::read_dir(&dir.path)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "atomic replacement left temp files");
    }

    #[test]
    fn legacy_json_receives_safe_defaults_and_aliases() {
        let raw = r#"{
            "id": "legacy",
            "objective": "Recover old work",
            "tasks": [{
                "id": "old-task",
                "objective": "Resume it",
                "status": "pending"
            }],
            "unknown_future_field": "ignored"
        }"#;
        let mut mission: Mission = serde_json::from_str(raw).unwrap();
        assert!(mission.normalize_legacy(42).unwrap());
        assert_eq!(mission.schema_version, MISSION_SCHEMA_VERSION);
        assert_eq!(mission.owner, "unassigned");
        assert!(!mission.acceptance_criteria.is_empty());
        let task = mission.task("old-task").unwrap();
        assert_eq!(task.mission_id, "legacy");
        assert_eq!(task.status, TaskStatus::Queued);
        assert_eq!(task.max_attempts, DEFAULT_MAX_ATTEMPTS);
        assert_eq!(task.owner, "unassigned");
        mission.validate().unwrap();
    }

    #[test]
    fn dependency_cycles_are_rejected() {
        let mut mission = mission();
        mission
            .add_task(task("a", vec!["b".to_string()], 2))
            .unwrap();
        mission
            .add_task(task("b", vec!["a".to_string()], 2))
            .unwrap();
        let error = mission.validate().expect_err("cycle must be rejected");
        assert!(matches!(error, MissionError::DependencyCycle(_)));
    }

    #[test]
    fn handoff_updates_owner_and_preserves_bounded_context() {
        let mut mission = mission();
        mission.add_task(task("handoff", vec![], 2)).unwrap();
        let mut handoff = Handoff::new(
            "foreman",
            "planner",
            "artist",
            "geometric_artist",
            "Research is complete; render the selected family.",
            "Render and attach a verified artifact.",
        );
        handoff.context_refs = vec!["knowledge/research/brief.json".to_string()];
        mission.record_handoff("handoff", handoff).unwrap();
        let task = mission.task("handoff").unwrap();
        assert_eq!(task.owner, "artist");
        assert_eq!(task.role, "geometric_artist");

        mission.compact_synopsis = "Mandala geometry ✦ ".repeat(200);
        let context = mission.render_context(ContextBudget {
            max_chars: 220,
            max_tasks: 4,
            max_criteria: 2,
            max_evidence_items: 2,
        });
        assert!(context.chars().count() <= 220);
        assert!(context.starts_with("MISSION v1"));
        assert_eq!(
            mission
                .render_context(ContextBudget {
                    max_chars: 0,
                    ..ContextBudget::default()
                })
                .len(),
            0
        );
    }

    #[test]
    fn unsupported_future_schema_is_rejected() {
        let mut mission = mission();
        mission.schema_version = MISSION_SCHEMA_VERSION + 1;
        assert!(matches!(
            mission.validate(),
            Err(MissionError::UnsupportedVersion { .. })
        ));
    }
}
