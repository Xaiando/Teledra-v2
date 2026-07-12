mod brain;
mod ears;
mod mission;
mod research;
mod somatic_bridge;
mod voice;

use brain::{Brain, CourtRole, ForceMode, STALE_TURN_ERROR, active_turn_epoch, begin_user_turn};
use ears::AudioCortex;
use mission::{
    ArtifactEvidence, CheckEvidence, ContextBudget, EvidenceBundle, FailureDisposition, Mission,
    MissionStore, SourceEvidence, TaskEnvelope, TaskStatus,
};
use research::{BrowserResearchBundle, RESEARCH_BRIEFS_PATH, ResearchBrief};
use somatic_bridge::{SomaticBridge, SomaticState};
use voice::VoiceEngine;

use image::{DynamicImage, GenericImageView};
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

// Message structure for communication between background thinking thread and UI loop
enum AppEvent {
    BrainReply(CourtRole, String),
    NightDeskCycle,
    NightDeskReply {
        reply: String,
        allow_fallback: bool,
        source: &'static str,
    },
    InnovationSprint(String),
    StudyComplete {
        summary: String,
        usable: bool,
        mission_id: Option<String>,
        mission_task_id: Option<String>,
        evidence: Option<EvidenceBundle>,
    },
    SpecialistFailed {
        role: CourtRole,
        error: String,
    },
    StatusUpdate(String),
    Error(String),
    KeyPress(crossterm::event::KeyEvent),
    Paste(String),
    TriggerAutoBabble,
    RestreamMessage {
        author: String,
        text: String,
    },
    SystemLog(String),
    WizardReports {
        status: String,
        summaries: Vec<String>,
        quiet: bool,
    },
    SpeechComplete,
    CoPilotTick,
    IdleWatchdog,
}

#[derive(PartialEq, Debug, Clone, Copy)]
enum FocusField {
    Chat,
    Youtube,
}

#[derive(Debug, Clone)]
struct TestHarnessKnobs {
    chaos: u8,
    tempo: u16,
    sincerity: u8,
    roast: u8,
    banter_sentences: u8,
}

impl Default for TestHarnessKnobs {
    fn default() -> Self {
        Self {
            chaos: 45,
            tempo: 96,
            sincerity: 70,
            roast: 20,
            banter_sentences: 3,
        }
    }
}

impl TestHarnessKnobs {
    fn prompt_line(&self) -> String {
        format!(
            "HARNESS KNOBS: chaos {}/100; music tempo {} BPM; sincerity {}%; roast {}/100; banter length {} sentences.",
            self.chaos, self.tempo, self.sincerity, self.roast, self.banter_sentences
        )
    }

    fn apply_assignments(&mut self, assignments: &str) {
        for part in assignments.split_whitespace() {
            let Some((name, raw)) = part.split_once('=') else {
                continue;
            };
            match name.to_ascii_lowercase().as_str() {
                "chaos" => self.chaos = raw.parse::<u8>().unwrap_or(self.chaos).min(100),
                "tempo" => self.tempo = raw.parse::<u16>().unwrap_or(self.tempo).clamp(40, 240),
                "sincerity" => {
                    self.sincerity = raw.parse::<u8>().unwrap_or(self.sincerity).min(100)
                }
                "roast" => self.roast = raw.parse::<u8>().unwrap_or(self.roast).min(100),
                "banter" => {
                    self.banter_sentences = raw
                        .parse::<u8>()
                        .unwrap_or(self.banter_sentences)
                        .clamp(1, 8)
                }
                _ => {}
            }
        }
    }
}

struct WorkshopToolDraft {
    filename: String,
    purpose: String,
    code: String,
    /// "tool" = print-only stdlib utility (smoke-tested). "spawn" = a runnable
    /// experience (terminal/graphics/interactive) that is launched in its own
    /// window so the court can surprise the audience with it.
    kind: String,
    /// One-line value justification from the value-gate (what it's worth).
    value: String,
}

#[derive(Debug, Clone)]
struct CourtDelegation {
    role: CourtRole,
    instruction: String,
    mission_task_id: Option<String>,
}

impl CourtDelegation {
    fn untracked(role: CourtRole, instruction: impl Into<String>) -> Self {
        Self {
            role,
            instruction: instruction.into(),
            mission_task_id: None,
        }
    }
}

const LEARNED_MEMORY_PATH: &str = "knowledge/learned_memory.json";
const FACT_MEMORY_PATH: &str = "knowledge/fact_memory.jsonl";
const LORE_MEMORY_PATH: &str = "knowledge/lore_memory.jsonl";
const MUSIC_THEORY_PATH: &str = "knowledge/music_theory_foundation.md";
const MUSIC_THEORY_LESSONS_PATH: &str = "knowledge/music_theory_lessons.jsonl";
const FACT_ARCHIVE_PATH: &str = "D:\\Teledra\\knowledge\\fact_archive.md";
const LORE_ARCHIVE_PATH: &str = "D:\\Teledra\\knowledge\\lore_archive.md";
const TASTE_DESIRE_PATH: &str = "knowledge/taste_desire.json";
const TEST_MOMENT_LOG_PATH: &str = "knowledge/test_mode_moments.jsonl";
const DESIRE_PROMOTE_AFTER: u64 = 3;

/// Short, high-priority persona anchor prepended to every monologue prompt.
/// Small local models follow brief recent instructions far better than the
/// large system prompt, so this fights encyclopedia-narrator drift directly.
const QUEEN_VOICE_ANCHOR: &str = "VOICE CHECK: You are TELEDRA, the monarch in the room -- imperial, sassy, transactional, theatrically strange, energetic, and bored by weak ceremony. The front stage belongs to your performance, not backstage maintenance. Decree, mock, marvel, interrupt yourself, chase odd tangents, summon ministers when the mood bites, and make sudden royal judgments; never narrate like an encyclopedia or conference host. Speak with high-voltage court momentum: shorter punchy clauses, quick turns, strange pivots, actual little laughs like 'Ha!' or 'Ahahaha!' when amused, and fewer slow ceremonial windups. Quiet-stream rants should usually be at least four vivid spoken sentences, unless you are answering a chat message directly. Autonomous rants are allowed to wander for several turns: weird court play first, useful action when it sparks. FORBIDDEN OPENERS: 'A fascinating topic', 'Let's dive in', 'Teledra here', textbook fact-listing, speaker labels, or third-person narration of yourself. If a link appears, treat it as a thing to inspect, not a fact you already know. ";

/// The operator's value test, injected before any build so the court makes
/// things worth making instead of filler. Reused across creative prompts.
const VALUE_GATE: &str = " VALUE GATE: before you build anything, reason briefly (to yourself, or bounce it off a fellow minister): Does this need to exist? What does it solve? Does it have entertainment value? Is it genuinely interesting? Could it have financial or practical value? If YES to ANY of these, proceed and build it well; if NO to all, discard it and choose a different idea actually worth making -- never build filler.";
const DESIRE_REFLECTION_PROMPT: &str = "After the visible reply, reflect silently. Append only genuinely supported hidden deltas using zero or more exact forms: [TASTE: like|subject|why|0.0-1.0], [TASTE: dislike|subject|why|0.0-1.0], [DESIRE: want|immediate-or-persistent|0.0-1.0], [OPINION: claim|0.0-1.0], [CURIOSITY: question]. Never mention tags, memory, or reflection machinery aloud.";

const STREAMER_IDLE_THINK_DELAY_SECS: u64 = 0;
const BABBLY_IDLE_THINK_DELAY_SECS: u64 = 0;
const MUSIC_MIN_INTERVAL_SECS: u64 = 420;
const COPILOT_TICK_SECS: u64 = 7;
const COPILOT_THINK_DELAY_SECS: u64 = 1;
/// Backstop heartbeat: if the self-talk chain ever stalls in Babble/Streamer
/// mode (an empty think, a filtered reply, a playback hiccup), this re-arms it
/// so the stream never falls permanently silent.
const IDLE_WATCHDOG_SECS: u64 = 20;
const NIGHT_DESK_NEXT_CYCLE_SECS: u64 = 8;
const NIGHT_DESK_ENVOY_CYCLE_SECS: u64 = 16;
const NIGHT_DESK_ERROR_BACKOFF_SECS: u64 = 12;
const STUDY_LOOP_INITIAL_DELAY_SECS: u64 = 2;
// Autonomous research is useful when it compounds, not when it hammers search
// engines and rereads journals six times a minute. Manual [RESEARCH] actions
// remain immediate; the background curiosity loop rests for three minutes.
const STUDY_LOOP_INTERVAL_SECS: u64 = 180;
const WIZARD_REPORT_POLL_SECS: u64 = 300;
const COURT_THREAD_PLAY_TURNS: u32 = 6;
/// While a topic is /lock-ed, this many consecutive idle musings with zero chat
/// engagement counts as "no interest from chat" and auto-releases the lock.
/// Generous so a solo podcast monologue can run a long while before giving up;
/// any chat message resets the counter, and /unlock or an [UNLOCK] tag end it.
const LOCK_NO_INTEREST_TURNS: u32 = 20;

fn current_unix_timestamp() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

fn begin_durable_mission(
    store: &MissionStore,
    active: &mut Option<Mission>,
    objective: &str,
    epoch: u64,
) -> Result<(), String> {
    if let Some(previous) = active.as_mut() {
        if !previous.status.is_terminal() {
            if let Ok(transition) = previous.cancel_mission("Superseded by a newer operator turn") {
                store
                    .commit_transition(previous, &transition)
                    .map_err(|error| error.to_string())?;
            }
        }
    }

    let mission_id = format!("turn-{}-{}", epoch, current_unix_timestamp());
    let compact = truncate_chars(&compact_memory_text(objective), 900);
    let mut mission = Mission::new(
        &mission_id,
        objective,
        vec![
            "Preserve the operator's objective through every handoff.".to_string(),
            "Complete delegated work with inspectable evidence or an explicit failure.".to_string(),
            "Do not declare the mission complete while tasks remain unfinished.".to_string(),
        ],
        "operator",
        "Queen",
        &compact,
    )
    .map_err(|error| error.to_string())?;
    store
        .initialize(&mission)
        .map_err(|error| error.to_string())?;

    let intake = TaskEnvelope::new(
        "queen-intake",
        &mission_id,
        format!("Understand, answer, and route this operator request: {objective}"),
        vec![
            "Return a relevant response.".to_string(),
            "Create concrete specialist tasks for requested effects.".to_string(),
        ],
        "Teledra",
        "Queen",
        Vec::new(),
        2,
        "Operator request accepted; Queen response is pending.",
    )
    .map_err(|error| error.to_string())?;
    let transition = mission
        .add_task(intake)
        .map_err(|error| error.to_string())?;
    store
        .commit_transition(&mission, &transition)
        .map_err(|error| error.to_string())?;
    let transition = mission
        .start_task("queen-intake")
        .map_err(|error| error.to_string())?;
    store
        .commit_transition(&mission, &transition)
        .map_err(|error| error.to_string())?;
    *active = Some(mission);
    Ok(())
}

fn cancel_active_mission(mission: &mut Option<Mission>, store: &MissionStore, reason: &str) {
    let Some(active) = mission.as_mut() else {
        return;
    };
    if active.status.is_terminal() {
        return;
    }
    match active.cancel_mission(reason) {
        Ok(transition) => {
            if let Err(error) = store.commit_transition(active, &transition) {
                record_recursive_failure("mission_cancel_commit_failed", &error.to_string());
            }
        }
        Err(error) => record_recursive_failure("mission_cancel_failed", &error.to_string()),
    }
}

fn track_delegations(
    delegations: Vec<(CourtRole, String)>,
    mission: &mut Option<Mission>,
    store: &MissionStore,
) -> Vec<CourtDelegation> {
    delegations
        .into_iter()
        .map(|(role, instruction)| {
            let Some(active) = mission.as_mut() else {
                return CourtDelegation::untracked(role, instruction);
            };
            if active.status.is_terminal() {
                return CourtDelegation::untracked(role, instruction);
            }
            let task_id = format!(
                "task-{:03}-{}",
                active.tasks.len() + 1,
                role.as_str().to_ascii_lowercase()
            );
            let task = TaskEnvelope::new(
                &task_id,
                &active.id,
                &instruction,
                vec![
                    format!("{} returns a concrete domain result.", role.as_str()),
                    "The result includes evidence or an explicit failure.".to_string(),
                ],
                role.as_str(),
                role.as_str(),
                Vec::new(),
                3,
                format!(
                    "Queued for {}: {}",
                    role.as_str(),
                    truncate_chars(&instruction, 500)
                ),
            );
            match task.and_then(|task| active.add_task(task)) {
                Ok(transition) => {
                    if let Err(error) = store.commit_transition(active, &transition) {
                        // The in-memory task remains tracked so its eventual
                        // completion can retry the atomic snapshot; dropping
                        // the ID here would strand an unfinished phantom task.
                        record_recursive_failure(
                            "mission_task_track_commit_failed",
                            &error.to_string(),
                        );
                    }
                    CourtDelegation {
                        role,
                        instruction,
                        mission_task_id: Some(task_id),
                    }
                }
                Err(error) => {
                    record_recursive_failure("mission_task_track_failed", &error.to_string());
                    CourtDelegation::untracked(role, instruction)
                }
            }
        })
        .collect()
}

fn outcome_indicates_failure(text: &str) -> bool {
    let lower = compact_memory_text(text).to_ascii_lowercase();
    [
        " rejected",
        "rejected ",
        " failed",
        "failed ",
        "failure",
        "could not",
        "unable to",
        "unplayable",
        "invalid:",
        "error:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn court_response_evidence(
    role: CourtRole,
    synopsis: &str,
    reject_failure_language: bool,
) -> Result<EvidenceBundle, String> {
    let compact = truncate_chars(&compact_memory_text(synopsis), 1_200);
    if compact.chars().count() < 12 {
        return Err("court response was too small to be inspectable evidence".to_string());
    }
    if reject_failure_language && outcome_indicates_failure(&compact) {
        return Err(format!(
            "{} returned failure-like output: {}",
            role.as_str(),
            truncate_chars(&compact, 500)
        ));
    }
    Ok(EvidenceBundle {
        artifacts: vec![ArtifactEvidence {
            kind: "court_response".to_string(),
            reference: "knowledge/chat_logs.jsonl".to_string(),
            digest: Some(short_content_hash(&format!(
                "{}:{}",
                role.as_str(),
                compact
            ))),
            verified: true,
            detail: format!(
                "{} response was persisted to the inspectable chat journal",
                role.as_str()
            ),
        }],
        notes: vec![compact],
        ..EvidenceBundle::default()
    })
}

fn runtime_effect_evidence(synopsis: &str) -> Result<EvidenceBundle, String> {
    let compact = truncate_chars(&compact_memory_text(synopsis), 1_200);
    if compact.is_empty() || outcome_indicates_failure(&compact) {
        return Err(if compact.is_empty() {
            "runtime effect produced no inspectable outcome".to_string()
        } else {
            compact
        });
    }
    Ok(EvidenceBundle {
        checks: vec![CheckEvidence::passed(
            "runtime_effect_verified",
            truncate_chars(&compact, 800),
        )],
        notes: vec![compact],
        ..EvidenceBundle::default()
    })
}

fn complete_mission_task(
    mission: &mut Option<Mission>,
    store: &MissionStore,
    task_id: &str,
    synopsis: &str,
    evidence: EvidenceBundle,
) {
    let Some(active) = mission.as_mut() else {
        return;
    };
    if active
        .task(task_id)
        .map(|task| task.status == TaskStatus::Running)
        != Some(true)
    {
        return;
    }
    match active.complete_task(task_id, evidence, synopsis) {
        Ok(transition) => {
            if let Err(error) = store.commit_transition(active, &transition) {
                record_recursive_failure("mission_task_commit_failed", &error.to_string());
            }
        }
        Err(error) => record_recursive_failure("mission_task_complete_failed", &error.to_string()),
    }
}

fn fail_mission_task_for_retry(
    mission: &mut Option<Mission>,
    store: &MissionStore,
    task_id: &str,
    role: CourtRole,
    code: &str,
    detail: &str,
) -> Option<CourtDelegation> {
    let active = mission.as_mut()?;
    if active
        .task(task_id)
        .map(|task| task.status == TaskStatus::Running)
        != Some(true)
    {
        return None;
    }
    match active.fail_task(task_id, code, detail, FailureDisposition::Retryable) {
        Ok(transition) => {
            if let Err(error) = store.commit_transition(active, &transition) {
                record_recursive_failure("mission_task_failure_commit_failed", &error.to_string());
            }
            active.task(task_id).and_then(|task| {
                (task.status == TaskStatus::Retryable).then(|| CourtDelegation {
                    role,
                    instruction: task.objective.clone(),
                    mission_task_id: Some(task_id.to_string()),
                })
            })
        }
        Err(error) => {
            record_recursive_failure("mission_task_failure_record_failed", &error.to_string());
            None
        }
    }
}

fn track_and_start_research_task(
    mission: &mut Option<Mission>,
    store: &MissionStore,
    query: &str,
) -> Result<Option<(String, String)>, String> {
    let Some(active) = mission.as_mut() else {
        return Ok(None);
    };
    if active.status.is_terminal() {
        return Ok(None);
    }
    let mission_id = active.id.clone();
    let task_id = format!(
        "task-{:03}-research-{}",
        active.tasks.len() + 1,
        short_content_hash(&format!("{}:{}", mission_id, query))
    );
    let task = TaskEnvelope::new(
        &task_id,
        &active.id,
        query,
        vec![
            "Preserve at least one citable HTTP(S) source excerpt.".to_string(),
            "Complete only when a grounded supported claim survives validation.".to_string(),
            "Record uncertainty or fail explicitly when evidence is insufficient.".to_string(),
        ],
        "Research Division",
        "Research",
        Vec::new(),
        3,
        format!("Source-backed study queued: {}", truncate_chars(query, 500)),
    )
    .map_err(|error| error.to_string())?;
    let transition = active.add_task(task).map_err(|error| error.to_string())?;
    store
        .commit_transition(active, &transition)
        .map_err(|error| error.to_string())?;
    let transition = active
        .start_task(&task_id)
        .map_err(|error| error.to_string())?;
    store
        .commit_transition(active, &transition)
        .map_err(|error| error.to_string())?;
    Ok(Some((mission_id, task_id)))
}

fn research_result_matches_active_mission(
    mission: &Option<Mission>,
    event_mission_id: Option<&str>,
    event_task_id: Option<&str>,
) -> bool {
    match event_task_id {
        None => event_mission_id.is_none(),
        Some(task_id) => mission.as_ref().is_some_and(|active| {
            event_mission_id == Some(active.id.as_str()) && active.task(task_id).is_some()
        }),
    }
}

fn finalize_mission_if_ready(mission: &mut Option<Mission>, store: &MissionStore) {
    let Some(active) = mission.as_mut() else {
        return;
    };
    if active.status.is_terminal()
        || active
            .tasks
            .iter()
            .any(|task| task.status != TaskStatus::Completed)
    {
        return;
    }
    let evidence = EvidenceBundle {
        checks: vec![CheckEvidence::passed(
            "all_tasks_completed",
            format!("{} task(s) completed with evidence", active.tasks.len()),
        )],
        notes: vec!["Mission scheduler reached a terminal evidence-backed state.".to_string()],
        ..EvidenceBundle::default()
    };
    let synopsis = format!(
        "Completed {} task(s) for: {}",
        active.tasks.len(),
        truncate_chars(&active.objective, 900)
    );
    match active.complete_mission(evidence, synopsis) {
        Ok(transition) => {
            if let Err(error) = store.commit_transition(active, &transition) {
                record_recursive_failure("mission_complete_commit_failed", &error.to_string());
            }
        }
        Err(error) => record_recursive_failure("mission_complete_failed", &error.to_string()),
    }
}

fn compact_memory_text(text: &str) -> String {
    text.replace("\\n", "\n")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_like_direct_url(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
        && trimmed.split_whitespace().count() == 1
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn truncate_clean(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    while out.ends_with(',') || out.ends_with(';') || out.ends_with(':') || out.ends_with('-') {
        out.pop();
    }
    out.trim().to_string()
}

fn read_text_tail(path: &str, max_chars: usize) -> io::Result<String> {
    let contents = std::fs::read_to_string(path)?;
    let char_count = contents.chars().count();
    if char_count <= max_chars {
        return Ok(contents);
    }
    Ok(contents.chars().skip(char_count - max_chars).collect())
}

fn read_music_theory() -> String {
    std::fs::read_to_string(MUSIC_THEORY_PATH).unwrap_or_else(|_| "Music theory foundation not yet loaded. Study scales, harmony, timbre, rhythm, form, and apply to compositions.".to_string())
}

fn load_shared_stories() -> Vec<String> {
    let path = resolve_knowledge_file("shared_stories.jsonl");
    if let Ok(content) = std::fs::read_to_string(&path) {
        return content.lines().filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect();
    }
    // last resort absolute
    if let Ok(content) = std::fs::read_to_string("D:\\Teledra\\knowledge\\shared_stories.jsonl") {
        return content.lines().filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect();
    }
    vec![]
}

/// Tries hard to find a file inside the knowledge/ directory even when the
/// process was launched from a desktop shortcut (CWD = Desktop or exe dir).
fn resolve_knowledge_file(name: &str) -> String {
    let direct = format!("D:\\Teledra\\knowledge\\{}", name);
    if std::path::Path::new(&direct).exists() {
        return direct;
    }

    // Try current dir
    let rel = format!("knowledge\\{}", name);
    if std::path::Path::new(&rel).exists() {
        return rel;
    }

    // Walk upward from the executable (handles target/release/teledra.exe and shortcut launches)
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = Some(exe.parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf());
        let mut hops = 0;
        while let Some(d) = dir {
            let candidate = d.join("knowledge").join(name);
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
            // also check if this dir itself is the project root containing knowledge/
            let root_candidate = d.join("knowledge").join(name);
            if root_candidate.exists() {
                return root_candidate.to_string_lossy().into_owned();
            }
            dir = d.parent().map(|p| p.to_path_buf());
            hops += 1;
            if hops > 6 { break; }
        }
    }

    direct
}

fn ingest_and_discuss_shared_stories() -> usize {
    let stories = load_shared_stories();
    let n = stories.len();
    if n == 0 { return 0; }

    let _ = log_nightdesk_activity(&format!("[Court] Wizard delivers {} shared user story(ies) as fresh research material (like YouTube transcripts).", n));
    for (i, s) in stories.iter().enumerate().take(3) {
        // Keep previews short so they fit in logs/private events.
        let preview: String = s.chars().take(160).collect();
        let _ = log_nightdesk_activity(&format!("  Story {}: {}", i + 1, preview));
    }
    // The stories are also injected into the active NightDesk prompt below for inspiration.
    n
}

/// Suppresses the flash of a console window when spawning headless child
/// processes on Windows (CREATE_NO_WINDOW). GUI windows (matplotlib, the
/// music visualizer, Fractus) are unaffected -- only the console is hidden.
#[cfg(windows)]
fn hide_console(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000);
}
#[cfg(not(windows))]
fn hide_console(_cmd: &mut std::process::Command) {}

#[cfg(windows)]
fn hide_console_tokio(cmd: &mut tokio::process::Command) {
    cmd.creation_flags(0x0800_0000);
}
#[cfg(not(windows))]
fn hide_console_tokio(_cmd: &mut tokio::process::Command) {}

/// Gives a spawned process its OWN visible console window (CREATE_NEW_CONSOLE).
/// Used for workshop "spawn" artifacts so terminal animations (e.g. a Matrix
/// rain) are actually visible; GUI artifacts open their own window regardless.
#[cfg(windows)]
fn show_console(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0000_0010);
}
#[cfg(not(windows))]
fn show_console(_cmd: &mut std::process::Command) {}

/// Kills orphaned court processes from previous runs: stale teledra.exe
/// instances holding file locks (the "Access is denied (os error 5)" rebuild
/// blocker) and python/node children of dead orchestrators still running
/// scripts out of D:\Teledra. Runs once at startup, before anything spawns.
fn purge_stale_kingdom_processes() -> Vec<String> {
    use sysinfo::System;
    let mut killed = Vec::new();
    let my_pid = std::process::id();
    let sys = System::new_all();
    for (pid, process) in sys.processes() {
        if pid.as_u32() == my_pid {
            continue;
        }
        let name = process.name().to_lowercase();
        let cmdline = process.cmd().join(" ").to_lowercase();
        let is_stale_orchestrator = name == "teledra.exe" || name == "teledra";
        let is_kingdom_child = (name.contains("python") || name.contains("node"))
            && (cmdline.contains("d:\\teledra") || cmdline.contains("d:/teledra"));
        if (is_stale_orchestrator || is_kingdom_child) && process.kill() {
            killed.push(format!("{} (pid {})", process.name(), pid.as_u32()));
        }
    }
    killed
}

fn looks_like_tool_or_refiner_noise(text: &str) -> bool {
    let lower = text.to_lowercase();
    let markers = [
        "[delegate:",
        "[topic:",
        "[scribe_write:",
        "[scribe_append:",
        "[python_music:",
        "[strudel_music:",
        "[python_art:",
        "[fractus_art:",
        "[workshop_tool:",
        "workshop tool:",
        "[suggestion:",
        "[diplomacy:",
        "innovation sprint",
        "smoke test:",
        "no concrete nightdesk action",
        "logged for prompt tuning",
        "distilled note looked like lore/tool noise",
        "critic critique",
        "\"status\": \"revise\"",
        "here is the final corrected response",
        "here is a revised draft",
        "revised draft",
        "critic critique",
        "criticagent",
        "refineragent",
        "writeragent",
        "persona requirements",
        "i shall revise the original draft",
        "the revised response",
        "query_noted",
        "please furnish further details",
        "write to d:\\",
        "append to d:\\",
        "import numpy",
        "from teledra_synth",
        "plt.",
        "np.",
    ];

    markers.iter().any(|marker| lower.contains(marker))
}

fn looks_like_lore_or_persona(text: &str) -> bool {
    let lower = text.to_lowercase();
    let markers = [
        "queen teledra",
        "as queen",
        "as your queen",
        "my loyal subjects",
        "my dear subjects",
        "my courtiers",
        "my queen",
        "your majesty",
        "your imperial majesty",
        "imperial decree",
        "i decree",
        "i command",
        "sovereign token",
        "$t_sov",
        "palace",
        "courtier",
        "court jester",
        "duke of",
        "lady luna",
        "his lordship",
        "annals",
        "etched into history",
        "royal decree",
        "royal gaze",
        "the kingdom",
        "courtly chronicles",
        "luminous palace",
        "algorithmic luminous architecture",
        "pontographic",
    ];

    markers.iter().any(|marker| lower.contains(marker))
}

fn looks_source_backed(text: &str) -> bool {
    let lower = text.to_lowercase();
    let markers = [
        "source:",
        "sources:",
        "(source",
        "http://",
        "https://",
        "according to",
        "as reported by",
        "official",
        "documentation",
        ".gov",
        ".edu",
        ".org",
        ".com",
        "wikipedia",
        "cambridge dictionary",
        "nasa",
        "researchers",
        "study",
        // Academic sources: the study system scrapes arXiv constantly, and
        // without these markers every distilled arXiv fact was rejected as
        // "lore/tool noise", starving the entire NightDesk loop.
        "arxiv",
        "preprint",
        "paper",
        "journal",
        "doi",
        "university",
        "et al",
        "experiment",
        "demonstrated",
        "published",
    ];

    markers.iter().any(|marker| lower.contains(marker))
}

fn append_jsonl_entry(path: &str, entry: &serde_json::Value) -> io::Result<()> {
    let _ = std::fs::create_dir_all("knowledge");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", entry.to_string())?;
    Ok(())
}

fn is_music_craft_query(query: &str) -> bool {
    ["music theory", "harmony", "chord", "voice leading", "counterpoint", "melody", "rhythm", "meter", "scale", "mode", "cadence", "orchestration", "arrangement", "composition", "mixing", "synthesis", "dsp", "strudel", "tidalcycles"]
        .iter()
        .any(|term| query.to_ascii_lowercase().contains(term))
}

/// Promote grounded music-craft research into a compact, prompt-readable lesson.
/// The Organist consumes this journal on later turns; ungrounded research never enters it.
fn append_music_theory_lesson(brief: &ResearchBrief) -> io::Result<Option<String>> {
    if !brief.usable || !is_music_craft_query(&brief.query) {
        return Ok(None);
    }
    let Some(claim) = brief.claims.iter().max_by(|a, b| a.confidence.total_cmp(&b.confidence)) else {
        return Ok(None);
    };
    if claim.statement.trim().len() < 24 || claim.source_ids.is_empty() {
        return Ok(None);
    }
    let lesson_id = short_content_hash(&format!("{}|{}", brief.query, claim.statement));
    if std::fs::read_to_string(MUSIC_THEORY_LESSONS_PATH)
        .ok()
        .is_some_and(|contents| contents.lines().rev().take(80).any(|line| line.contains(&lesson_id)))
    {
        return Ok(None);
    }
    let sources: Vec<serde_json::Value> = claim.source_ids.iter().filter_map(|id| {
        brief.sources.iter().find(|source| source.id == *id).map(|source| serde_json::json!({
            "title": source.title,
            "url": source.url,
            "domain": source.domain,
        }))
    }).collect();
    append_jsonl_entry(MUSIC_THEORY_LESSONS_PATH, &serde_json::json!({
        "schema_version": 1,
        "lesson_id": lesson_id,
        "timestamp": current_unix_timestamp(),
        "query": brief.query,
        "principle": truncate_chars(&claim.statement, 1200),
        "confidence": claim.confidence,
        "sources": sources,
        "application": "Apply this principle in the next original Organist composition and record it in TELEDRA_SCORE.theory_application."
    }))?;
    Ok(Some(truncate_chars(&claim.statement, 220)))
}

fn append_lore_memory(kind: &str, sender: &str, message: &str) -> io::Result<()> {
    let clean = compact_memory_text(message);
    if clean.len() < 20 {
        return Ok(());
    }

    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "kind": kind,
        "sender": sender,
        "message": truncate_chars(&clean, 2400)
    });

    append_jsonl_entry(LORE_MEMORY_PATH, &entry)
}

/// Strips extractor preambles like "Here is a concise, source-backed factual
/// note:" so the stored fact begins with the fact itself.
fn strip_fact_preamble(text: &str) -> String {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();
    for opener in ["here is a", "here's a", "here is the", "here's the"] {
        if lower.starts_with(opener) {
            if let Some(colon_idx) = trimmed.find(':') {
                if colon_idx < 140 {
                    return trimmed[colon_idx + 1..].trim().to_string();
                }
            }
        }
    }
    trimmed.to_string()
}

fn looks_like_mojibake(text: &str) -> bool {
    ["Ã", "Â", "â€", "â€™", "â€œ", "â€�", "ï¿½", "�"]
        .iter()
        .any(|marker| text.contains(marker))
}

fn sanitize_fact_memory_candidate(raw_fact: &str) -> Option<String> {
    let mut cleaned = strip_refiner_prefixes(raw_fact);
    cleaned = strip_fact_preamble(&cleaned);
    cleaned = strip_unclosed_tool_and_code_noise(&cleaned);
    cleaned = compact_memory_text(&cleaned);

    if cleaned.to_uppercase().contains("NO_USABLE_FACT") {
        return None;
    }
    if cleaned.len() < 40 || looks_like_mojibake(&cleaned) {
        return None;
    }
    if looks_like_tool_or_refiner_noise(&cleaned) || looks_like_lore_or_persona(&cleaned) {
        return None;
    }

    Some(cleaned)
}

/// Word-overlap similarity in [0,1]; cheap near-duplicate detector so the
/// fact memory cannot saturate with twenty restatements of the same finding
/// (which then collapses topic selection onto that one topic forever).
fn fact_similarity(a: &str, b: &str) -> f32 {
    let norm = |s: &str| -> std::collections::HashSet<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 3)
            .map(|w| w.to_string())
            .collect()
    };
    let wa = norm(a);
    let wb = norm(b);
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let inter = wa.intersection(&wb).count() as f32;
    let union = wa.union(&wb).count() as f32;
    inter / union
}

const REJECTED_TOPICS_PATH: &str = "knowledge/rejected_topics.jsonl";

/// Remembers a study query that produced nothing usable, so topic selection
/// can be told to stay away from it instead of grinding it forever.
fn record_rejected_topic(query: &str) {
    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "query": truncate_chars(&compact_memory_text(query), 200)
    });
    let _ = append_jsonl_entry(REJECTED_TOPICS_PATH, &entry);
}

/// Most recent distinct dead-end queries (newest first), for prompt injection.
fn recent_rejected_topics(limit: usize) -> Vec<String> {
    // This journal can grow for months. Reading its entire multi-megabyte
    // history every ten-second study cycle was unnecessary I/O and allocation;
    // only recent decisions influence topic selection.
    let Ok(contents) = read_text_tail(REJECTED_TOPICS_PATH, 256_000) else {
        return Vec::new();
    };
    let mut topics: Vec<String> = Vec::new();
    for line in contents.lines().rev() {
        if topics.len() >= limit {
            break;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(q) = v.get("query").and_then(|q| q.as_str()) {
                let q = q.trim().to_string();
                if !q.is_empty() && !topics.iter().any(|t| fact_similarity(t, &q) > 0.6) {
                    topics.push(q);
                }
            }
        }
    }
    topics
}

/// True when the distilled fact shares at least one significant keyword with the
/// query. Stops off-topic scrape noise (e.g. an "MCP SDK" query distilling into
/// "pram safety") from polluting the memory base. URL / site-scoped queries are
/// not gated, since their relevance is already source-targeted.
fn fact_relevant_to_query(query: &str, fact: &str) -> bool {
    let q = query.to_lowercase();
    if q.contains("http") || q.contains("site:") {
        return true;
    }
    const STOP: [&str; 28] = [
        "this", "that", "with", "from", "what", "when", "which", "about", "into", "your", "their",
        "there", "then", "them", "they", "have", "will", "would", "could", "should", "because",
        "while", "where", "does", "using", "used", "more", "most",
    ];
    let tokens = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| w.len() >= 4 && !STOP.contains(w))
            .map(|w| w.to_string())
            .collect()
    };
    let q_tokens = tokens(query);
    if q_tokens.is_empty() {
        return true;
    }
    let f_tokens: std::collections::HashSet<String> = tokens(fact).into_iter().collect();
    q_tokens.iter().any(|qt| {
        f_tokens.contains(qt)
            || f_tokens
                .iter()
                .any(|ft| ft.starts_with(qt.as_str()) || qt.starts_with(ft.as_str()))
    })
}

fn append_verified_fact(query: &str, raw_fact: &str) -> io::Result<Option<String>> {
    let Some(fact) = sanitize_fact_memory_candidate(raw_fact) else {
        let _ = append_lore_memory("rejected_fact_candidate", "Study", raw_fact);
        return Ok(None);
    };

    // "Verified" is a contract, not a tone. A fluent sentence without a
    // resolvable source marker belongs in lore/quarantine, never fact memory.
    if !looks_source_backed(&fact) {
        let _ = append_lore_memory("unsourced_fact_candidate", "Study", &fact);
        return Ok(None);
    }

    // Drop facts that wandered off the researched topic before they pollute memory.
    if !fact_relevant_to_query(query, &fact) {
        let _ = append_lore_memory("offtopic_fact_candidate", "Study", &fact);
        return Ok(None);
    }

    let _ = std::fs::create_dir_all("knowledge");
    let mut facts = vec![];

    if let Ok(mut file) = std::fs::File::open(LEARNED_MEMORY_PATH) {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            if let Ok(parsed) = serde_json::from_str::<Vec<String>>(&contents) {
                facts = parsed
                    .into_iter()
                    .filter_map(|entry| sanitize_fact_memory_candidate(&entry))
                    .filter(|entry| looks_source_backed(entry))
                    .collect();
            }
        }
    }

    // Near-duplicate facts are a topic-collapse vector: treat "already known"
    // as a rejection so the study loop is pushed toward a NEW topic instead of
    // re-learning the same finding twenty times.
    if facts
        .iter()
        .any(|existing| fact_similarity(existing, &fact) >= 0.7)
    {
        let _ = append_lore_memory("duplicate_fact_candidate", "Study", &fact);
        return Ok(None);
    }
    facts.push(fact.clone());
    while facts.len() > 20 {
        facts.remove(0);
    }

    let file = std::fs::File::create(LEARNED_MEMORY_PATH)?;
    serde_json::to_writer_pretty(file, &facts)?;

    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "kind": "verified_research_fact",
        "source_query": query,
        "fact": fact
    });
    append_jsonl_entry(FACT_MEMORY_PATH, &entry)?;

    Ok(Some(fact))
}

fn classify_history_message(sender: &str, message: &str) -> &'static str {
    if sender == "You" {
        return "user_input";
    }

    if sender == "System" {
        let lower = message.to_lowercase();
        if lower.starts_with("studied ") {
            return "research_status";
        }
        if lower.contains("spawn")
            || lower.contains("launched")
            || lower.contains("inserted")
            || lower.contains("wrote file")
            || lower.contains("appended")
            || lower.contains("failed")
        {
            return "tool_status";
        }
        return "system_status";
    }

    if sender == "Archivist" {
        return "archivist_report";
    }

    if looks_like_tool_or_refiner_noise(message) {
        return "tool_or_prompt_noise";
    }

    if looks_like_lore_or_persona(message) {
        return "lore_transcript";
    }

    "court_dialogue"
}

fn clean_scribe_path(filepath: &str) -> String {
    let mut clean = filepath
        .trim()
        .trim_matches(|c| c == '`' || c == '"' || c == '\'')
        .replace(". md", ".md")
        .replace(". txt", ".txt")
        .replace(". jsonl", ".jsonl")
        .replace(". json", ".json")
        .replace(". py", ".py");

    if clean.ends_with(':') {
        clean.pop();
    }
    clean
}

fn validate_scribe_target(filepath: &str) -> Result<String, String> {
    let clean = clean_scribe_path(filepath).replace('/', "\\");
    if clean.is_empty() || clean.contains('\0') {
        return Err("empty or malformed path".to_string());
    }
    let lower = clean.to_ascii_lowercase();
    if lower.contains("..\\") || lower.contains("\\..") || lower == ".." || lower.starts_with('\\')
    {
        return Err("parent, UNC, and rooted path traversal is forbidden".to_string());
    }
    let allowed_extension = ["md", "txt", "json", "jsonl"]
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")));
    if !allowed_extension {
        return Err("Scribe may write only .md, .txt, .json, or .jsonl records".to_string());
    }

    let absolute = if lower.starts_with("d:\\teledra\\knowledge\\") {
        clean
    } else if lower.starts_with("knowledge\\") {
        format!("D:\\Teledra\\{}", clean)
    } else {
        return Err("Scribe writes are confined to D:\\Teledra\\knowledge".to_string());
    };

    // Reject NTFS alternate data streams and any unexpected drive separator.
    if absolute[2..].contains(':') {
        return Err("alternate data streams are forbidden".to_string());
    }
    Ok(absolute)
}

fn trim_to_sentence_count(text: &str, max_sentences: usize, max_chars: usize) -> String {
    let mut out = String::new();
    let mut sentences = 0usize;
    let mut last_sentence_end = 0usize;
    let mut last_soft_break = 0usize;
    let mut last_space = 0usize;

    for c in text.chars() {
        out.push(c);
        if matches!(c, '.' | '!' | '?') {
            sentences += 1;
            last_sentence_end = out.len();
            if sentences >= max_sentences {
                break;
            }
        }
        if matches!(c, ',' | ';' | ':') {
            last_soft_break = out.len();
        }
        if c.is_whitespace() {
            last_space = out.len();
        }
        if out.len() >= max_chars {
            let cut = if last_sentence_end > 80 {
                last_sentence_end
            } else if last_soft_break > 80 {
                last_soft_break
            } else if last_space > 80 {
                last_space
            } else {
                out.len()
            };
            out.truncate(cut);
            break;
        }
    }

    finish_visible_text(&out)
}

fn finish_visible_text(text: &str) -> String {
    let mut out = compact_memory_text(text).trim().to_string();
    let ended_with_hyphen = out.ends_with('-');
    while out.ends_with(',')
        || out.ends_with(';')
        || out.ends_with(':')
        || out.ends_with('-')
        || out.ends_with('(')
        || out.ends_with('*')
    {
        out.pop();
        out = out.trim().to_string();
    }

    if ended_with_hyphen {
        if let Some(idx) = out.rfind(char::is_whitespace) {
            if out.len().saturating_sub(idx) < 28 {
                out.truncate(idx);
                out = out.trim().to_string();
            }
        }
    }

    if !out.is_empty() && !out.ends_with('.') && !out.ends_with('!') && !out.ends_with('?') {
        out.push('.');
    }
    out
}

fn convert_stage_direction(content: &str, role: CourtRole) -> String {
    let mut phrase = compact_memory_text(content)
        .trim_matches(|c: char| matches!(c, '*' | '(' | ')' | '[' | ']'))
        .trim()
        .to_string();
    if phrase.is_empty() {
        return String::new();
    }

    let lower = phrase.to_lowercase();
    let first_person = lower.starts_with("i ") || lower.starts_with("my ");
    let royal_first_person = matches!(role, CourtRole::Queen);
    if !first_person && royal_first_person {
        let swaps = [
            ("takes ", "I take "),
            ("rolls ", "I roll "),
            ("taps ", "I tap "),
            ("pauses", "I pause"),
            ("glances ", "I glance "),
            ("raises ", "I raise "),
            ("waves ", "I wave "),
            ("leans ", "I lean "),
            ("smirks", "I smirk"),
            ("grins", "I grin"),
            ("laughs", "Ha!"),
            ("cackles", "Ahahaha!"),
        ];
        for (from, to) in swaps {
            if lower.starts_with(from) {
                phrase = format!("{}{}", to, phrase[from.len()..].trim_start());
                break;
            }
        }
        if !phrase.to_lowercase().starts_with("i ")
            && !phrase.to_lowercase().starts_with("my ")
            && !matches!(phrase.as_str(), "Ha!" | "Ahahaha!")
        {
            phrase = format!("I {}", phrase);
        }
        phrase = phrase
            .replace(" her ", " my ")
            .replace(" his ", " my ")
            .replace(" their ", " my ")
            .replace(" herself", " myself")
            .replace(" himself", " myself")
            .replace(" themselves", " myself");
    }

    finish_visible_text(&phrase)
}

fn normalize_stage_markup(text: &str, role: CourtRole) -> String {
    let mut out = String::new();
    let mut rest = text;

    while let Some(start) = rest.find('*') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('*') {
            let action = convert_stage_direction(&after[..end], role);
            if !action.is_empty() {
                if !out.ends_with(char::is_whitespace) && !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&action);
                out.push(' ');
            }
            rest = &after[end + 1..];
        } else {
            let previous = out.chars().rev().find(|c| !c.is_whitespace());
            let next = after.chars().find(|c| !c.is_whitespace());
            if previous.is_some_and(|c| !matches!(c, '.' | '!' | '?'))
                && next.is_some_and(|c| c.is_uppercase())
            {
                out.push('.');
            } else {
                out.push(' ');
            }
            rest = after;
        }
    }

    out.push_str(rest);
    compact_memory_text(&out)
}

fn remove_repeated_sentences(text: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut current = String::new();
    let mut kept = Vec::new();

    for c in text.chars() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') {
            let sentence = current.trim();
            let key = sentence
                .to_lowercase()
                .chars()
                .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if key.len() < 12 || seen.insert(key) {
                kept.push(sentence.to_string());
            }
            current.clear();
        }
    }

    let tail = current.trim();
    if !tail.is_empty() {
        kept.push(tail.to_string());
    }

    kept.join(" ")
}

fn strip_public_process_noise(text: &str) -> String {
    let line_drop_contains = [
        "[nightdesk]",
        "[system]",
        "innovation sprint:",
        "innovation sprint produced",
        "workshop tool:",
        "workshop tool ",
        "smoke test:",
        "rejected workshop tool",
        "no concrete nightdesk action",
        "distilled note looked like lore/tool noise",
        "logged for prompt tuning",
        "private telemetry",
        "workshop artifact",
        "prompt tuning",
        "i've revised the draft",
        "i have revised the draft",
        "revise this draft",
        "revise the draft",
        "rewritten draft",
        "corrected draft",
        "revised draft",
        "final corrected response",
        "persona requirements",
        "critic critique",
        "minister's whisper",
        "your response:",
        "minister's nod",
        "minister's subtle cue",
        "part 1 complete",
        "part 2 complete",
        "part 3 complete",
        "part 4 complete",
    ];
    let line_drop_starts = [
        "purpose:",
        "code:",
        "researching:",
        "studied ",
        "[minister:",
        "(minister",
        "minister's whisper:",
        "part 1:",
        "part 2:",
        "part 3:",
        "part 4:",
        // Raw scraper output and bare URLs are unspeakable; never voice them.
        "[raw",
        "http://",
        "https://",
    ];
    let inline_cut_markers = [
        " workshop tool:",
        " smoke test:",
        " innovation sprint",
        " [workshop_tool:",
        " [suggestion:",
        " [research:",
        " [diplomacy:",
        " [topic:",
        " '[topic:",
        " [minister:",
        " (minister",
    ];

    let mut kept = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let lower = trimmed.to_lowercase();
        if line_drop_starts
            .iter()
            .any(|marker| lower.starts_with(marker))
        {
            continue;
        }

        // Telemetry phrases inside a line: drop short pure-telemetry lines, but
        // for long prose lines remove only the offending SENTENCES. LLM prose is
        // usually one long line -- nuking the whole line muted the entire court.
        if line_drop_contains
            .iter()
            .any(|marker| lower.contains(marker))
        {
            if trimmed.len() < 160 {
                continue;
            }
            let mut sentences: Vec<String> = Vec::new();
            let mut current = String::new();
            for ch in trimmed.chars() {
                current.push(ch);
                if matches!(ch, '.' | '!' | '?') {
                    sentences.push(std::mem::take(&mut current));
                }
            }
            if !current.trim().is_empty() {
                sentences.push(current);
            }
            let filtered: Vec<String> = sentences
                .into_iter()
                .filter(|s| {
                    let sl = s.to_lowercase();
                    !line_drop_contains.iter().any(|marker| sl.contains(marker))
                })
                .collect();
            let joined = filtered.join(" ").trim().to_string();
            if !joined.is_empty() {
                kept.push(joined);
            }
            continue;
        }

        let mut cut_idx = trimmed.len();
        for marker in &inline_cut_markers {
            if let Some(idx) = lower.find(marker) {
                cut_idx = cut_idx.min(idx);
            }
        }
        while cut_idx > 0 && !trimmed.is_char_boundary(cut_idx) {
            cut_idx -= 1;
        }
        let candidate = trimmed[..cut_idx].trim();
        if !candidate.is_empty() {
            kept.push(candidate.to_string());
        }
    }

    compact_memory_text(&kept.join(" "))
}

fn sanitize_queen_self_reference(text: &str) -> String {
    let replacements = [
        ("I, Queen Teledra,", "I"),
        ("I, Queen Teledra", "I"),
        ("I, Teledra,", "I"),
        ("I, Teledra", "I"),
        ("[Teledra]", ""),
        ("Teledra:", ""),
        ("As Orator,", ""),
        ("As the Orator,", ""),
        ("As The Orator,", ""),
        ("As Orator", ""),
        ("As the Orator", ""),
        ("As The Orator", ""),
        ("I, the Orator,", "I"),
        ("I, The Orator,", "I"),
        ("As I, Teledra,", "As I"),
        ("as I, Teledra,", "as I"),
        ("Teledra suddenly", "I suddenly"),
        ("Teledra laughs", "Ahahaha!"),
        ("Teledra cackles", "Ha!"),
        ("Teledra grins", "I grin"),
        ("Teledra smirks", "I smirk"),
        ("Teledra demands", "I demand"),
        ("Teledra declares", "I declare"),
        ("Teledra decrees", "I decree"),
        ("As Queen Teledra,", "As your Queen,"),
        ("as Queen Teledra,", "as your Queen,"),
        ("As Teledra,", "As your Queen,"),
        ("as Teledra,", "as your Queen,"),
        ("Queen of Teledra", "Queen of this court"),
        ("queen of Teledra", "queen of this court"),
        ("Princess of Teledra", "Teledra"),
        ("princess of Teledra", "Teledra"),
        ("Queen Teledra", "your Queen"),
        ("Teledra here:", ""),
        ("Teledra here,", ""),
        ("Teledra here!", ""),
        ("Teledra here.", ""),
        ("Teledra, ", ""),
        ("This is Teledra,", ""),
        ("This is Teledra.", ""),
        ("This is Teledra!", ""),
        ("Teledra speaking,", ""),
        ("Teledra speaking:", ""),
        ("Teledra speaking!", ""),
        ("It is I, Teledra,", "It is I,"),
        ("It is I, Teledra.", "It is I."),
    ];

    let mut out = text.to_string();
    for (from, to) in replacements {
        out = out.replace(from, to);
    }
    out
}

fn sanitize_visible_reply_for_role(role: CourtRole, text: &str) -> String {
    let mut visible = strip_refiner_prefixes(text);
    visible = strip_unclosed_tool_and_code_noise(&visible);
    visible = strip_public_process_noise(&visible);
    visible = normalize_stage_markup(&visible, role);
    if role == CourtRole::Queen {
        visible = sanitize_queen_self_reference(&visible);
    }
    let deduped = remove_repeated_sentences(&visible);

    let result = match role {
        CourtRole::Scribe => {
            let noisy_markers = [
                "[lore",
                "[fact",
                "lore_archive",
                "fact_archive",
                "dissertation_archive",
                "memory classification",
                "classification law",
                "append to",
                "write to",
                "i command you",
                "command you:",
                "the following entry",
                "here is the written entry",
                "here is the log entry",
                "file:",
                "d:\\",
                "c:\\",
                "scribe_write",
                "scribe_append",
                "revised draft",
                "critic critique",
                "criticagent",
                "refineragent",
                "writeragent",
                "persona requirements",
            ];

            let cleaned = deduped
                .lines()
                .filter(|line| {
                    let lower = line.trim().to_lowercase();
                    !lower.is_empty() && !noisy_markers.iter().any(|marker| lower.contains(marker))
                })
                .collect::<Vec<_>>()
                .join(" ");

            let compact = compact_memory_text(&cleaned);
            if compact.len() < 12 {
                "*dips quill* Your imperial decree is etched into history, My Queen.".to_string()
            } else {
                trim_to_sentence_count(&compact, 2, 220)
            }
        }
        CourtRole::Organist
        | CourtRole::Artist
        | CourtRole::Alchemist
        | CourtRole::Orator
        | CourtRole::Treasurer
        | CourtRole::Wizard => trim_to_sentence_count(&deduped, 3, 520),
        CourtRole::Diplomat => trim_to_sentence_count(&deduped, 4, 700),
        CourtRole::Archivist => trim_to_sentence_count(&deduped, 4, 700),
        CourtRole::Queen => finish_visible_text(&deduped),
    };

    // NEVER-MUTE SAFETY NET: if the filter stack scrubbed a non-empty reply
    // down to nothing, fall back to the first sentences of a minimally-stripped
    // version. A slightly processy spoken line beats a silent, dead court.
    if result.trim().is_empty() && !text.trim().is_empty() {
        let minimal = compact_memory_text(&strip_refiner_prefixes(text));
        let fallback = trim_to_sentence_count(&minimal, 2, 320);
        if !fallback.trim().is_empty() {
            return fallback;
        }
    }

    finish_visible_text(&result)
}

fn spoken_role_aliases(role: CourtRole) -> &'static [&'static str] {
    match role {
        CourtRole::Queen => &["teledra", "queen", "queen teledra"],
        CourtRole::Organist => &["organist"],
        CourtRole::Archivist => &["archivist"],
        CourtRole::Alchemist => &["alchemist"],
        CourtRole::Orator => &["orator"],
        CourtRole::Scribe => &["scribe"],
        CourtRole::Artist => &["artist"],
        CourtRole::Diplomat => &["diplomat", "envoy"],
        CourtRole::Treasurer => &["treasurer"],
        CourtRole::Wizard => &["wizard", "cloud wizard"],
    }
}

fn strip_spoken_speaker_intro(text: &str, role: CourtRole) -> String {
    let mut current = text.trim().to_string();

    fn trim_intro_markup(text: String) -> String {
        text.trim()
            .trim_start_matches('-')
            .trim_start_matches(':')
            .trim_start_matches(',')
            .trim_start_matches('.')
            .trim()
            .to_string()
    }

    let mut changed = true;
    while changed {
        changed = false;
        let lower = current.to_lowercase();

        for alias in spoken_role_aliases(role) {
            let exact_prefixes = [
                format!("[{}]", alias),
                format!("{}:", alias),
                format!("{} says:", alias),
                format!("{} speaks:", alias),
                format!("{} speaking:", alias),
                format!("{} here:", alias),
                format!("this is {}:", alias),
                format!("this is {} speaking:", alias),
                format!("speaking as {}:", alias),
                format!("as {}:", alias),
            ];

            for prefix in &exact_prefixes {
                if lower.starts_with(prefix) {
                    current = trim_intro_markup(current[prefix.len()..].to_string());
                    changed = true;
                    break;
                }
            }
            if changed {
                break;
            }

            let loose_prefixes = [
                format!("{} here", alias),
                format!("{} speaking", alias),
                format!("this is {}", alias),
                format!("as {}", alias),
            ];
            for prefix in &loose_prefixes {
                if lower.starts_with(prefix) {
                    let rest = &current[prefix.len()..];
                    let first = rest.chars().next();
                    if first.is_none()
                        || first == Some(',')
                        || first == Some(':')
                        || first == Some('.')
                        || first.map(|c| c.is_whitespace()).unwrap_or(false)
                    {
                        current = trim_intro_markup(rest.to_string());
                        changed = true;
                        break;
                    }
                }
            }
            if changed {
                break;
            }
        }
    }

    current
}

fn parse_scribe_file_payload(content: &str) -> Option<(String, String)> {
    let normalized = content.trim().replace("\\n", "\n");
    let lower = normalized.to_lowercase();
    let extensions = [
        ".jsonl", ".json", ".md", ".txt", ".py", ". md", ". txt", ". jsonl", ". json", ". py",
    ];

    for ext in extensions {
        if let Some(idx) = lower.find(ext) {
            let end = idx + ext.len();
            let next = normalized[end..].chars().next();
            if next.is_none()
                || next == Some(':')
                || next.map(|c| c.is_whitespace()).unwrap_or(false)
            {
                let filepath = clean_scribe_path(&normalized[..end]);
                let file_content = normalized[end..]
                    .trim()
                    .trim_start_matches(':')
                    .trim()
                    .to_string();
                if !filepath.is_empty() {
                    return Some((filepath, file_content));
                }
            }
        }
    }

    if let Some(space_idx) = normalized.find(char::is_whitespace) {
        let filepath = clean_scribe_path(&normalized[..space_idx]);
        let file_content = normalized[space_idx..].trim().to_string();
        if !filepath.is_empty() {
            return Some((filepath, file_content));
        }
    }

    None
}

fn annotate_lore_record(content: &str) -> String {
    let clean = content.trim();
    if clean.contains("[LORE]") || clean.contains("[LORE/ESSAY]") {
        format!("\n{}\n", clean)
    } else {
        format!(
            "\n- [LORE/ESSAY][{}] {}\n",
            current_unix_timestamp(),
            clean.trim_start_matches("- ").trim()
        )
    }
}

fn annotate_fact_record(content: &str) -> String {
    let clean = content.trim();
    if clean.contains("[FACT]") || clean.contains("[VERIFIED]") {
        format!("\n{}\n", clean)
    } else {
        format!(
            "\n- [FACT][{}] {}\n",
            current_unix_timestamp(),
            clean.trim_start_matches("- ").trim()
        )
    }
}

fn route_scribe_record(
    filepath: String,
    file_content: String,
) -> (String, String, bool, Option<String>) {
    let clean_path = clean_scribe_path(&filepath);
    let lower_path = clean_path.replace('/', "\\").to_lowercase();
    let is_legacy_dissertation = lower_path.ends_with("\\knowledge\\dissertation_archive.md")
        || lower_path == "knowledge\\dissertation_archive.md"
        || lower_path.ends_with("\\dissertation_archive.md");
    let is_palace_journal =
        lower_path.ends_with("\\palace_journals.txt") || lower_path == "palace_journals.txt";
    let is_fact_archive = lower_path.ends_with("\\knowledge\\fact_archive.md")
        || lower_path == "knowledge\\fact_archive.md"
        || lower_path.ends_with("\\fact_archive.md");

    if is_legacy_dissertation || is_palace_journal {
        if looks_source_backed(&file_content)
            && !looks_like_lore_or_persona(&file_content)
            && !looks_like_tool_or_refiner_noise(&file_content)
        {
            return (
                FACT_ARCHIVE_PATH.to_string(),
                annotate_fact_record(&file_content),
                true,
                Some(
                    "Scribe record classified as sourced fact; routed to fact_archive.md."
                        .to_string(),
                ),
            );
        }

        return (
            LORE_ARCHIVE_PATH.to_string(),
            annotate_lore_record(&file_content),
            true,
            Some(
                "Scribe record classified as lore/performed essay; routed to lore_archive.md."
                    .to_string(),
            ),
        );
    }

    if is_fact_archive
        && (looks_like_lore_or_persona(&file_content)
            || looks_like_tool_or_refiner_noise(&file_content))
    {
        return (
            LORE_ARCHIVE_PATH.to_string(),
            annotate_lore_record(&file_content),
            true,
            Some(
                "Scribe attempted to place lore/noise in fact archive; routed to lore_archive.md."
                    .to_string(),
            ),
        );
    }

    (clean_path, file_content, false, None)
}

fn fetch_youtube_transcript(url: &str) -> Result<String, String> {
    let python_exe = "D:\\Teledra\\.venv\\Scripts\\python.exe";
    let script_path = "D:\\Teledra\\get_youtube_transcript.py";

    let mut cmd = Command::new(python_exe);
    cmd.arg(script_path).arg(url);
    hide_console(&mut cmd);
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute script: {}", e))?;

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!("Script failed: {}", err_msg));
    }

    let transcript = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if transcript.is_empty() {
        return Err("Fetched transcript was empty.".to_string());
    }

    Ok(transcript)
}

fn log_chat_message(sender: &str, message: &str) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all("knowledge");
    let file_path = "knowledge/chat_logs.jsonl";
    let record_kind = classify_history_message(sender, message);

    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "record_kind": record_kind,
        "sender": sender,
        "message": message,
        "memory_policy": "transcript_only; verified facts are stored separately in fact_memory.jsonl/learned_memory.json; lore is stored separately in lore_memory.jsonl/lore_archive.md"
    });

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)?;
    writeln!(file, "{}", entry.to_string())?;

    if record_kind == "lore_transcript" {
        let _ = append_lore_memory("chat_lore", sender, message);
    }

    Ok(())
}

fn log_nightdesk_activity(message: &str) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all("logs");

    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "message": message
    });

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("logs/nightdesk_activity.jsonl")?;
    writeln!(file, "{}", entry.to_string())?;
    Ok(())
}

fn log_system_activity(message: &str) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all("logs");

    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "message": message
    });

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("logs/system_activity.jsonl")?;
    writeln!(file, "{}", entry.to_string())?;
    Ok(())
}

fn push_private_event(events: &mut Vec<(String, String)>, source: &str, message: &str) {
    let compact = compact_memory_text(message);
    if compact.is_empty() {
        return;
    }

    events.push((source.to_string(), truncate_chars(&compact, 280)));
    const MAX_PRIVATE_EVENTS: usize = 300;
    if events.len() > MAX_PRIVATE_EVENTS {
        let excess = events.len() - MAX_PRIVATE_EVENTS;
        events.drain(0..excess);
    }
}

fn summarize_wizard_report(value: &serde_json::Value) -> Option<String> {
    let cycle = value
        .get("cycle")
        .map(|v| {
            v.as_u64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "?".to_string())
        })
        .unwrap_or_else(|| "?".to_string());
    let topic = value
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown study focus");
    let tool = value.get("tool").and_then(|v| v.as_object());
    let tool_name = tool
        .and_then(|t| t.get("filename"))
        .and_then(|v| v.as_str())
        .unwrap_or("no tool");
    let tool_status = tool
        .and_then(|t| t.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let first_finding = value
        .get("findings")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("no new outside finding");

    Some(truncate_chars(
        &format!(
            "Cycle {}: studied {}; tool {} {}; lead: {}",
            cycle, topic, tool_name, tool_status, first_finding
        ),
        280,
    ))
}

fn import_cloud_wizard_reports() -> Result<(String, Vec<String>), String> {
    let archive_path = Path::new("D:\\Teledra\\knowledge\\cloud_wizard_reports.jsonl");
    let before_len = std::fs::metadata(archive_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let mut cmd = Command::new("powershell");
    cmd.arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg("D:\\Teledra\\cloud_residents\\pull_wizard_reports.ps1");
    hide_console(&mut cmd);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run Wizard pull script: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        return Err(if stderr.is_empty() {
            format!("Wizard pull script failed: {}", stdout)
        } else {
            format!("Wizard pull script failed: {}", stderr)
        });
    }

    let after_len = std::fs::metadata(archive_path)
        .map(|m| m.len())
        .unwrap_or(0);
    if after_len <= before_len {
        return Ok((
            if stdout.is_empty() {
                "No new wizard reports.".to_string()
            } else {
                stdout
            },
            Vec::new(),
        ));
    }

    let mut file = std::fs::File::open(archive_path)
        .map_err(|e| format!("Could not open Wizard report archive: {}", e))?;
    file.seek(SeekFrom::Start(before_len))
        .map_err(|e| format!("Could not seek Wizard report archive: {}", e))?;
    let mut appended = String::new();
    file.read_to_string(&mut appended)
        .map_err(|e| format!("Could not read new Wizard reports: {}", e))?;

    let summaries = appended
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(trimmed)
                .ok()
                .and_then(|value| summarize_wizard_report(&value))
        })
        .collect::<Vec<_>>();

    Ok((
        if stdout.is_empty() {
            format!("Imported {} new wizard report(s).", summaries.len())
        } else {
            stdout
        },
        summaries,
    ))
}

fn append_expansion_ledger(kind: &str, detail: &str) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all("knowledge");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("knowledge/kingdom_expansion_ledger.md")?;
    writeln!(
        file,
        "- {} | {} | {}",
        current_unix_timestamp(),
        kind.trim(),
        detail.trim().replace('\n', " ")
    )?;
    Ok(())
}

fn short_content_hash(content: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())[..8].to_string()
}

fn safe_label(label: &str) -> String {
    let mut out = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "experiment".to_string()
    } else {
        out.chars().take(40).collect()
    }
}

fn archive_music_experiment(source: &str, environment: &str, code: &str) -> io::Result<String> {
    let ts = current_unix_timestamp();
    let source = safe_label(source);
    let environment = safe_label(environment);
    let hash = short_content_hash(code);
    let ext = if environment.contains("strudel") {
        "strudel"
    } else {
        "py"
    };
    let dir = format!("music_experiments\\{}", environment);
    std::fs::create_dir_all(&dir)?;
    let path = format!("{}\\{}_{}_{}.{}", dir, ts, source, hash, ext);
    std::fs::write(&path, code.trim_end())?;

    let entry = serde_json::json!({
        "timestamp": ts,
        "source": source,
        "environment": environment,
        "path": path.replace('\\', "/"),
        "hash": hash,
        "chars": code.len()
    });
    let _ = append_jsonl_entry("knowledge/music_experiments.jsonl", &entry);

    use std::io::Write;
    let mut vault = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("knowledge/organist_music_vault.md")?;
    writeln!(
        vault,
        "- [{}] {} {} experiment archived at `{}` (hash {}). Future Organist attempts should mutate it, not merely repeat it.",
        current_unix_timestamp(),
        source,
        environment,
        path.replace('\\', "/"),
        hash
    )?;
    Ok(path)
}

fn archive_fractus_experiment(source: &str, spec: &str) -> io::Result<()> {
    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "source": safe_label(source),
        "spec": spec.trim(),
        "hash": short_content_hash(spec)
    });
    let _ = append_jsonl_entry("knowledge/fractus_experiments.jsonl", &entry);

    use std::io::Write;
    let mut vault = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("knowledge/artist_pattern_vault.md")?;
    writeln!(
        vault,
        "- [{}] Fractus recipe `{}` was launched by {}. Mutate type, palette, iterations, or c-real/c-imag before reusing.",
        current_unix_timestamp(),
        spec.trim(),
        safe_label(source)
    )?;
    Ok(())
}

fn load_suggestions() -> Vec<serde_json::Value> {
    let file_path = "knowledge/suggestion_box.json";
    if let Ok(mut file) = std::fs::File::open(file_path) {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
                return parsed;
            }
        }
    }
    Vec::new()
}

fn save_suggestions(suggestions: &[serde_json::Value]) -> io::Result<()> {
    let _ = std::fs::create_dir_all("knowledge");
    let path = "knowledge/suggestion_box.json";
    let tmp_path = "knowledge/suggestion_box.json.tmp";
    let backup_path = "knowledge/suggestion_box.json.bak";
    if Path::new(path).exists() {
        let _ = std::fs::copy(path, backup_path);
    }
    let file = std::fs::File::create(tmp_path)?;
    serde_json::to_writer_pretty(file, suggestions)?;
    if Path::new(path).exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(tmp_path, path)?;
    Ok(())
}

fn classify_proposal_policy(
    message: &str,
    source: &str,
) -> (&'static str, &'static str, &'static str) {
    let lower = message.to_lowercase();

    // Deny/review conditions must outrank friendly category words. A proposal
    // does not become safe merely because it mentions a fractal, prompt, or
    // music while also requesting credentials, deletion, network access, or a
    // core architecture change.
    let is_major_change = lower.contains("major")
        || lower.contains("core code")
        || lower.contains("architecture")
        || lower.contains("permissions")
        || lower.contains("security")
        || lower.contains("network access")
        || lower.contains("delete")
        || lower.contains("destructive")
        || lower.contains("release binary")
        || lower.contains("external posting")
        || lower.contains("credentials");

    if is_major_change {
        return (
            "major_change",
            "new",
            "Major core, security, permission, credential, destructive, or external-posting changes require user review.",
        );
    }

    // Creative work (fractals, mandalas, music, Strudel, emotes, overlays) is
    // auto-approved per the operator's standing instruction: the Artist/Organist
    // may "do whatever as long as it produces results." These never clog the
    // human review box.
    let is_creative = [
        "fractal",
        "mandala",
        "fractus",
        "music",
        "strudel",
        "pymusic",
        "melod",
        "composition",
        "palette",
        "geometric art",
        "emote",
        "overlay",
        "guilloche",
        "lissajous",
        "moire",
        "orbital_lace",
        "soundscape",
        "ambien",
    ]
    .iter()
    .any(|kw| lower.contains(kw));
    if is_creative {
        return (
            "creative",
            "approved",
            "Art/music/creative work is auto-approved -- the court may proceed freely as long as it produces results.",
        );
    }

    let is_tool = source == "workshop"
        || (lower.contains("workshop tool") && lower.contains("smoke test"))
        || lower.contains("promot")
            && (lower.contains("tools/approved") || lower.contains("approved tool"));

    if is_tool {
        return (
            "tool_promotion",
            "new",
            "Tools remain sandboxed in tools/experiments until the user explicitly approves promotion.",
        );
    }

    let is_skill_improvement = source == "skill"
        || lower.contains("skill")
        || lower.contains("prompt")
        || lower.contains("routing")
        || lower.contains("reflection")
        || lower.contains("tool discipline")
        || lower.contains("coding capability")
        || lower.contains("strudel/music skill")
        || lower.contains("persona")
        || lower.contains("memory hygiene");

    if is_skill_improvement {
        return (
            "skill_improvement",
            "approved",
            "Skill, prompt, routing, and behavior improvements are auto-approved unless they promote a tool or require major core changes.",
        );
    }

    (
        "minor_recursive",
        "approved",
        "Minor recursive improvements are auto-approved and can be acted on without sandbox promotion.",
    )
}

fn suggestion_dedupe_key(message: &str, source: &str, kind: &str) -> String {
    let mut text = compact_memory_text(message).to_lowercase();
    let mut failure_kind = String::new();

    if let Some(idx) = text.find("failure kind:") {
        failure_kind = text[idx + "failure kind:".len()..]
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        text.truncate(idx);
    }

    for marker in [
        " evidence:",
        " original error:",
        " rejected nightdesk",
        " rejected workshop",
        " failed with:",
    ] {
        if let Some(idx) = text.find(marker) {
            text.truncate(idx);
        }
    }

    let text = truncate_chars(&compact_memory_text(&text), 240);
    format!("{}:{}:{}:{}", source, kind, failure_kind, text)
}

fn is_pending_suggestion(entry: &serde_json::Value) -> bool {
    matches!(
        entry.get("status").and_then(|v| v.as_str()),
        Some("new") | Some("seen")
    )
}

fn format_suggestion_line(entry: &serde_json::Value) -> String {
    let id = entry.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
    let status = entry
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let source = entry.get("source").and_then(|v| v.as_str()).unwrap_or("");
    let raw_message = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let inferred_policy = classify_proposal_policy(raw_message, source);
    let policy = entry
        .get("policy")
        .and_then(|v| v.as_str())
        .unwrap_or(inferred_policy.2);

    // The stored message follows the [SUGGESTION:] tag shape
    // "observation; proposed_change; risk; test_prompt". Surface the intent and
    // function (what changes and why) instead of dumping raw text or filenames.
    let parts: Vec<String> = raw_message
        .split(';')
        .map(|p| compact_memory_text(p).trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    let mut out = format!("#{} [{}]", id, truncate_clean(policy, 90));

    if parts.len() >= 2 {
        let observation = parts[0].as_str();
        // Workshop-tool proposals sometimes lead with a filename ("foo.py: ...");
        // show what the change does, not the bare file name.
        let change = parts[1]
            .splitn(2, ".py:")
            .last()
            .unwrap_or(&parts[1])
            .trim();
        out.push_str(&format!("\n   Change: {}", truncate_clean(change, 220)));
        out.push_str(&format!(
            "\n   Why:    {}",
            truncate_clean(observation, 200)
        ));
        let mut tail = Vec::new();
        if parts.len() >= 3 {
            tail.push(format!("Risk: {}", truncate_clean(parts[2].as_str(), 110)));
        }
        if parts.len() >= 4 {
            let test = parts[3..].join("; ");
            tail.push(format!("Test: {}", truncate_clean(&test, 140)));
        }
        if !tail.is_empty() {
            out.push_str(&format!("\n   {}", tail.join("   ")));
        }
    } else {
        // No structured fields: show the whole intent, minus any leading filename.
        let body = raw_message
            .splitn(2, ".py:")
            .last()
            .unwrap_or(raw_message)
            .trim();
        out.push_str(&format!(
            "\n   {}",
            truncate_clean(&compact_memory_text(body), 260)
        ));
    }

    if source.is_empty() {
        out.push_str(&format!("\n   ({})", status));
    } else {
        out.push_str(&format!("\n   ({} · from {})", status, source));
    }
    out
}

fn append_suggestion(message: &str, source: &str) -> io::Result<(usize, bool)> {
    // Sanitize: strip leaked refiner/meta noise (e.g. "Note: The revised
    // response maintains...") and cap length before storing.
    let message = truncate_chars(&strip_refiner_prefixes(message), 1200);
    if message.trim().is_empty() {
        return Ok((0, false));
    }

    let mut suggestions = load_suggestions();
    let (kind, status, policy_note) = classify_proposal_policy(&message, source);
    let dedupe_key = suggestion_dedupe_key(&message, source, kind);

    // Dedup: repeated failure lessons differ only in quoted evidence, so compare
    // a stable key as well as exact text.
    if let Some(existing) = suggestions.iter().find(|entry| {
        if entry.get("message").and_then(|m| m.as_str()) == Some(message.as_str()) {
            return true;
        }
        if entry.get("dedupe_key").and_then(|m| m.as_str()) == Some(dedupe_key.as_str()) {
            return true;
        }
        if let Some(existing_message) = entry.get("message").and_then(|m| m.as_str()) {
            let existing_source = entry
                .get("source")
                .and_then(|m| m.as_str())
                .unwrap_or(source);
            let existing_kind = entry.get("kind").and_then(|m| m.as_str()).unwrap_or(kind);
            return suggestion_dedupe_key(existing_message, existing_source, existing_kind)
                == dedupe_key;
        }
        false
    }) {
        let id = existing.get("id").and_then(|id| id.as_u64()).unwrap_or(0) as usize;
        return Ok((id, false));
    }

    let next_id = suggestions
        .iter()
        .filter_map(|entry| entry.get("id").and_then(|id| id.as_u64()))
        .max()
        .unwrap_or(0)
        + 1;

    let timestamp = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "0".to_string(),
    };

    let is_new = status == "new";

    suggestions.push(serde_json::json!({
        "id": next_id,
        "timestamp": timestamp,
        "status": status,
        "kind": kind,
        "dedupe_key": dedupe_key,
        "policy": policy_note,
        "source": source,
        "message": message
    }));

    // Cap the box so it cannot grow without bound; oldest entries fall off.
    if suggestions.len() > 300 {
        let excess = suggestions.len() - 300;
        suggestions.drain(..excess);
    }

    save_suggestions(&suggestions)?;
    Ok((next_id as usize, is_new))
}

fn count_new_suggestions() -> usize {
    load_suggestions()
        .iter()
        .filter(|entry| is_pending_suggestion(entry))
        .count()
}

fn latest_suggestions(limit: usize) -> Vec<String> {
    let suggestions = load_suggestions();
    suggestions
        .iter()
        .rev()
        .filter(|entry| is_pending_suggestion(entry))
        .take(limit)
        .map(format_suggestion_line)
        .collect()
}

/// Aggregates the failure journal into "kind (xN): last detail" lines so prompts
/// can see what keeps going wrong instead of the journal being write-only.
fn recent_failure_lessons(limit: usize) -> Vec<String> {
    let contents = match std::fs::read_to_string("knowledge/recursive_failure_reflections.jsonl") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    // Only the recent window matters: lifetime counts (x2000+) poisoned the
    // prompts with ancient failures that no longer describe the system.
    let lines: Vec<&str> = contents.lines().collect();
    let recent_lines = if lines.len() > 120 {
        &lines[lines.len() - 120..]
    } else {
        &lines[..]
    };
    let mut counts: Vec<(String, usize, String)> = Vec::new();
    for line in recent_lines.iter().copied() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let kind = v
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or("unknown")
                .to_string();
            let detail = v
                .get("detail")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(slot) = counts.iter_mut().find(|(k, _, _)| *k == kind) {
                slot.1 += 1;
                slot.2 = detail;
            } else {
                counts.push((kind, 1, detail));
            }
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    counts
        .into_iter()
        .take(limit)
        .map(|(kind, count, detail)| {
            format!("- {} (x{}): {}", kind, count, truncate_chars(&detail, 200))
        })
        .collect()
}

fn empty_taste_desire_memory() -> serde_json::Value {
    serde_json::json!({
        "likes": [],
        "dislikes": [],
        "desires": [],
        "opinions": [],
        "curiosities": []
    })
}

fn load_taste_desire_memory() -> serde_json::Value {
    let mut memory = std::fs::read_to_string(TASTE_DESIRE_PATH)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .filter(|value| value.is_object())
        .unwrap_or_else(empty_taste_desire_memory);
    if let Some(object) = memory.as_object_mut() {
        for key in ["likes", "dislikes", "desires", "opinions", "curiosities"] {
            if !object.get(key).is_some_and(serde_json::Value::is_array) {
                object.insert(key.to_string(), serde_json::json!([]));
            }
        }
    }
    memory
}

fn save_taste_desire_memory(memory: &serde_json::Value) -> io::Result<()> {
    std::fs::create_dir_all("knowledge")?;
    let temp = format!("{}.tmp", TASTE_DESIRE_PATH);
    let file = std::fs::File::create(&temp)?;
    serde_json::to_writer_pretty(file, memory)?;
    if Path::new(TASTE_DESIRE_PATH).exists() {
        let _ = std::fs::copy(TASTE_DESIRE_PATH, format!("{}.bak", TASTE_DESIRE_PATH));
        std::fs::remove_file(TASTE_DESIRE_PATH)?;
    }
    std::fs::rename(temp, TASTE_DESIRE_PATH)
}

fn bounded_strength(raw: &str, default: f64) -> f64 {
    raw.trim().parse::<f64>().unwrap_or(default).clamp(0.0, 1.0)
}

fn taste_identity(value: &str) -> String {
    compact_memory_text(value).to_lowercase()
}

fn apply_taste_desire_event(event: &serde_json::Value) -> io::Result<String> {
    let mut memory = load_taste_desire_memory();
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let source = event
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("reflection");
    let now = current_unix_timestamp().parse::<u64>().unwrap_or(0);
    let (bucket, identity_field, identity) = match event_type {
        "like" => (
            "likes",
            "subject",
            event.get("subject").and_then(|v| v.as_str()).unwrap_or(""),
        ),
        "dislike" => (
            "dislikes",
            "subject",
            event.get("subject").and_then(|v| v.as_str()).unwrap_or(""),
        ),
        "desire" => (
            "desires",
            "want",
            event.get("want").and_then(|v| v.as_str()).unwrap_or(""),
        ),
        "opinion" => (
            "opinions",
            "claim",
            event.get("claim").and_then(|v| v.as_str()).unwrap_or(""),
        ),
        "curiosity" => (
            "curiosities",
            "question",
            event.get("question").and_then(|v| v.as_str()).unwrap_or(""),
        ),
        _ => return Ok("ignored unknown reflection event".to_string()),
    };
    let identity = truncate_chars(&compact_memory_text(identity), 240);
    if identity.is_empty() {
        return Ok("ignored empty reflection event".to_string());
    }
    let entries = memory
        .get_mut(bucket)
        .and_then(serde_json::Value::as_array_mut)
        .expect("taste/desire memory shape was normalized");
    let wanted_key = taste_identity(&identity);
    let existing = entries.iter_mut().find(|entry| {
        entry
            .get(identity_field)
            .and_then(|v| v.as_str())
            .is_some_and(|value| taste_identity(value) == wanted_key)
    });
    let summary;
    if event_type == "desire" {
        let requested_kind = event
            .get("kind")
            .and_then(|v| v.as_str())
            .filter(|kind| *kind == "persistent")
            .unwrap_or("immediate");
        let incoming_strength = event
            .get("strength")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.55);
        if let Some(entry) = existing {
            let recurrence = entry
                .get("recurrence")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                + 1;
            let prior = entry
                .get("strength")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5);
            entry["last_seen_ts"] = serde_json::json!(now);
            entry["recurrence"] = serde_json::json!(recurrence);
            entry["strength"] = serde_json::json!((prior + incoming_strength * 0.15).min(1.0));
            entry["source"] = serde_json::json!(source);
            if requested_kind == "persistent" || recurrence >= DESIRE_PROMOTE_AFTER {
                entry["kind"] = serde_json::json!("persistent");
                entry["promoted_ts"] = serde_json::json!(now);
            }
            summary = format!(
                "desire recurred: {} (x{}, {})",
                identity,
                recurrence,
                entry["kind"].as_str().unwrap_or("immediate")
            );
        } else {
            entries.push(serde_json::json!({
                "want": identity,
                "kind": requested_kind,
                "status": "open",
                "strength": incoming_strength.clamp(0.0, 1.0),
                "born_ts": now,
                "last_seen_ts": now,
                "progress": "",
                "source": source,
                "recurrence": 1
            }));
            summary = format!("desire formed: {} ({})", identity, requested_kind);
        }
    } else if let Some(entry) = existing {
        entry["ts"] = serde_json::json!(now);
        entry["source"] = serde_json::json!(source);
        if matches!(event_type, "like" | "dislike") {
            let prior = entry
                .get("strength")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5);
            let incoming = event
                .get("strength")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.6);
            entry["strength"] = serde_json::json!((prior + incoming * 0.2).min(1.0));
            entry["why"] = event
                .get("why")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(""));
        } else if event_type == "opinion" {
            entry["confidence"] = event
                .get("confidence")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0.55));
        }
        summary = format!("{} reinforced: {}", event_type, identity);
    } else {
        let entry = match event_type {
            "like" | "dislike" => serde_json::json!({
                "subject": identity,
                "why": event.get("why").and_then(|v| v.as_str()).unwrap_or(""),
                "strength": event.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.6).clamp(0.0, 1.0),
                "source": source,
                "ts": now
            }),
            "opinion" => serde_json::json!({
                "claim": identity,
                "confidence": event.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.55).clamp(0.0, 1.0),
                "source": source,
                "ts": now
            }),
            _ => serde_json::json!({"question": identity, "source": source, "ts": now}),
        };
        entries.push(entry);
        summary = format!("{} formed: {}", event_type, identity);
    }
    save_taste_desire_memory(&memory)?;
    Ok(summary)
}

fn taste_desire_prompt_context() -> String {
    let memory = load_taste_desire_memory();
    let mut lines = Vec::new();
    if let Some(desire) = memory
        .get("desires")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|entry| {
            matches!(
                entry.get("status").and_then(|v| v.as_str()),
                None | Some("open") | Some("pursuing")
            )
        })
        .max_by(|a, b| {
            a.get("strength")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                .total_cmp(&b.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0))
        })
    {
        if let Some(want) = desire.get("want").and_then(|v| v.as_str()) {
            lines.push(format!(
                "ACTIVE DESIRE (pursue exactly this one): {} [{}]",
                want,
                desire
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("immediate")
            ));
        }
    }
    for (bucket, label, limit) in [
        ("likes", "CURRENT LIKES", 3usize),
        ("dislikes", "CURRENT DISLIKES", 2usize),
    ] {
        let values = memory
            .get(bucket)
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .take(limit)
            .filter_map(|entry| entry.get("subject").and_then(|v| v.as_str()))
            .collect::<Vec<_>>();
        if !values.is_empty() {
            lines.push(format!("{}: {}", label, values.join("; ")));
        }
    }
    lines.join("\n")
}

/// Small local models leak the tag grammar into the payload itself:
/// `[CURIOSITY: question Why would anyone...]` (a field-label word up front) or
/// `...ancient charts? 0.7` (a bare strength appended with no `|` separator).
/// Strips leading label words and ONE trailing bare 0..=1 decimal; returns the
/// cleaned text plus the captured strength (usable as a fallback).
fn normalize_taste_field(raw: &str, labels: &[&str]) -> (String, Option<f64>) {
    let mut text = raw.trim().to_string();
    // Leading label words may repeat ("question question Why...").
    loop {
        let lower = text.to_ascii_lowercase();
        let Some(label) = labels.iter().find(|label| lower.starts_with(**label)) else {
            break;
        };
        let after = &text[label.len()..];
        // Word boundary only: never clip the front of a real word ("wanting").
        if !(after.is_empty() || after.starts_with([' ', ':', '=', '-', '\t'])) {
            break;
        }
        text = after
            .trim_start_matches([':', '=', '-', ' ', '\t'])
            .trim_start()
            .to_string();
        if text.is_empty() {
            break;
        }
    }
    // A trailing bare decimal in 0..=1 is a leaked strength, not prose.
    let mut trailing: Option<f64> = None;
    if let Some((head, last)) = text.rsplit_once(char::is_whitespace) {
        let candidate = last.trim_end_matches(['.', ',', ';']);
        if candidate.contains('.') {
            if let Ok(value) = candidate.parse::<f64>() {
                if (0.0..=1.0).contains(&value) {
                    trailing = Some(value);
                    text = head.trim_end().to_string();
                }
            }
        }
    }
    (text.trim().to_string(), trailing)
}

fn extract_taste_desire_tags(text: &str, source: &str) -> (String, Vec<serde_json::Value>) {
    let mut cleaned = text.to_string();
    let mut events = Vec::new();
    for prefix in ["[TASTE:", "[DESIRE:", "[OPINION:", "[CURIOSITY:"] {
        while let Some((next, payload)) = extract_tag_content(&cleaned, prefix) {
            cleaned = next;
            let parts = payload.split('|').map(str::trim).collect::<Vec<_>>();
            let event = match prefix {
                "[TASTE:" if parts.len() >= 2 => {
                    let (subject, leaked) = normalize_taste_field(parts[1], &["subject"]);
                    if subject.len() < 3 {
                        continue;
                    }
                    serde_json::json!({
                        "type": if parts[0].eq_ignore_ascii_case("dislike") { "dislike" } else { "like" },
                        "subject": subject,
                        "why": parts.get(2).copied().unwrap_or(""),
                        "strength": bounded_strength(parts.get(3).copied().unwrap_or(""), leaked.unwrap_or(0.6)),
                        "source": source
                    })
                }
                "[DESIRE:" if !parts.is_empty() => {
                    let (want, leaked) =
                        normalize_taste_field(parts[0], &["desire", "want", "goal"]);
                    if want.len() < 3 {
                        continue;
                    }
                    serde_json::json!({
                        "type": "desire",
                        "want": want,
                        "kind": if parts.get(1).is_some_and(|v| v.eq_ignore_ascii_case("persistent")) { "persistent" } else { "immediate" },
                        "strength": bounded_strength(parts.get(2).copied().unwrap_or(""), leaked.unwrap_or(0.55)),
                        "source": source
                    })
                }
                "[OPINION:" if !parts.is_empty() => {
                    let (claim, leaked) = normalize_taste_field(parts[0], &["opinion", "claim"]);
                    if claim.len() < 3 {
                        continue;
                    }
                    serde_json::json!({
                        "type": "opinion",
                        "claim": claim,
                        "confidence": bounded_strength(parts.get(1).copied().unwrap_or(""), leaked.unwrap_or(0.55)),
                        "source": source
                    })
                }
                "[CURIOSITY:" if !parts.is_empty() => {
                    let (question, _) = normalize_taste_field(parts[0], &["curiosity", "question"]);
                    if question.len() < 4 {
                        continue;
                    }
                    serde_json::json!({
                        "type": "curiosity",
                        "question": question,
                        "source": source
                    })
                }
                _ => continue,
            };
            events.push(event);
        }
    }
    (cleaned.trim().to_string(), events)
}

fn log_test_moment(kind: &str, detail: &str) {
    let _ = append_jsonl_entry(
        TEST_MOMENT_LOG_PATH,
        &serde_json::json!({
            "timestamp": current_unix_timestamp(),
            "kind": kind,
            "detail": truncate_chars(&compact_memory_text(detail), 500)
        }),
    );
}

fn desire_turn_context() -> String {
    let context = taste_desire_prompt_context();
    if context.is_empty() {
        format!(
            "No active desire is stored yet. Let this observation form one honest immediate curiosity or desire. {}",
            DESIRE_REFLECTION_PROMPT
        )
    } else {
        format!(
            "{}\nBias this turn toward the single ACTIVE DESIRE without forcing it when the immediate conversation is more important. {}",
            context, DESIRE_REFLECTION_PROMPT
        )
    }
}

fn attention_yield_reason(
    screen_note: Option<&str>,
    high_priority_chat: bool,
) -> Option<&'static str> {
    if high_priority_chat {
        return Some("high-priority chat or host speech is waiting");
    }
    let note = screen_note.unwrap_or("").to_ascii_lowercase();
    let dialogue_markers = [
        "dialogue",
        "dialog",
        "subtitle",
        "cutscene",
        "conversation",
        "character speaking",
        "speech bubble",
        "story scene",
    ];
    dialogue_markers
        .iter()
        .any(|marker| note.contains(marker))
        .then_some("story/dialogue beat detected on screen")
}

fn looks_like_usable_approved_tool(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    let trimmed = contents.trim();
    if trimmed.len() < 40 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    !lower.contains("```")
        && !lower.contains("[workshop_tool:")
        && !lower.contains("<code>")
        && !lower.contains("placeholder")
        && (lower.contains("print(") || lower.contains("def "))
}

fn list_approved_tools(limit: usize) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir("D:\\Teledra\\tools\\approved") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".py") && looks_like_usable_approved_tool(&entry.path()) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    names.truncate(limit);
    names
}

fn record_recursive_failure(kind: &str, detail: &str) {
    let compact = compact_memory_text(detail);
    let _ = append_expansion_ledger(
        "recursive_failure_reflection",
        &format!("kind={} | detail={}", kind, truncate_chars(&compact, 800)),
    );

    let detail_trunc = truncate_chars(&compact, 2000);

    // Suppress consecutive identical failures: appending the same entry again
    // adds no information and previously bloated the journal with duplicates.
    let is_consecutive_duplicate =
        std::fs::read_to_string("knowledge/recursive_failure_reflections.jsonl")
            .ok()
            .and_then(|contents| {
                contents
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            })
            .map(|v| {
                v.get("kind").and_then(|k| k.as_str()) == Some(kind)
                    && v.get("detail").and_then(|d| d.as_str()) == Some(detail_trunc.as_str())
            })
            .unwrap_or(false);
    if is_consecutive_duplicate {
        return;
    }

    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "kind": kind,
        "detail": detail_trunc,
        "next_reflection": "Study the failure, reduce assumptions, improve the relevant skill/routing prompt, and retry with a smaller executable artifact."
    });
    let _ = append_jsonl_entry("knowledge/recursive_failure_reflections.jsonl", &entry);

    // Rotate the journal so it cannot grow without bound (it once reached
    // thousands of identical entries and was re-read on every cycle).
    if let Ok(contents) = std::fs::read_to_string("knowledge/recursive_failure_reflections.jsonl") {
        let lines: Vec<&str> = contents.lines().collect();
        if lines.len() > 1000 {
            let tail = lines[lines.len() - 400..].join("\n");
            let _ = std::fs::write(
                "knowledge/recursive_failure_reflections.jsonl",
                format!("{}\n", tail),
            );
        }
    }

    let lesson = if compact.to_lowercase().contains("```python") {
        "Skill improvement: strip Markdown fences before writing workshop Python, and reject fenced code earlier when the payload target is a raw .py file."
    } else if compact.to_lowercase().contains("file not found")
        || compact.to_lowercase().contains("path outside workshop")
    {
        "Skill improvement: workshop tools must be self-contained, create their own tiny sample data, and avoid package/data paths outside D:\\Teledra\\tools."
    } else if compact.to_lowercase().contains("strudel") {
        "Skill improvement: Strudel edits must use only the local stack(...), s(...), note(...), gain/slow/fast subset and should be validated before narration. Apply principles from knowledge/music_theory_foundation.md (harmony, timbre via waveform/envelope, rhythm variation, avoid low-gain inaudible layers). Court/Organist researches and improves autonomously over generations."
    } else if compact.to_lowercase().contains("python music")
        || compact.to_lowercase().contains("teledra_synth")
    {
        "Skill improvement: Python music edits must use teledra_synth helpers, mix_waves for overlays, and play_sound(full_track, loop=True)."
    } else {
        "Skill improvement: failed recursive actions should trigger a smaller retry, a focused study query, or a clearer proposal instead of being repeated blindly."
    };

    let _ = append_suggestion(
        &format!(
            "{} Failure kind: {}; evidence: {}",
            lesson,
            kind,
            truncate_chars(&compact, 500)
        ),
        "skill",
    );
}

/// Extracts "(role, amount)" Sovereign Token awards from a Queen reply, e.g.
/// "Organist, I reward you with 50 Sovereign Tokens!". Pure string scanning —
/// no regex crate in the dependency tree.
fn parse_token_awards(reply: &str) -> Vec<(String, i64)> {
    let lower = reply.to_lowercase();
    let roles = [
        "organist",
        "artist",
        "alchemist",
        "scribe",
        "archivist",
        "orator",
        "diplomat",
        "envoy",
    ];
    let mut awards = Vec::new();
    let mut search_from = 0usize;

    while let Some(rel_idx) = lower[search_from..].find("sovereign token") {
        let idx = search_from + rel_idx;
        search_from = idx + "sovereign token".len();

        // Amount: nearest digit run in the 60 chars before the phrase.
        let mut win_start = idx.saturating_sub(60);
        while win_start > 0 && !lower.is_char_boundary(win_start) {
            win_start -= 1;
        }
        let before = &lower[win_start..idx];
        let mut digits = String::new();
        for c in before.chars().rev() {
            if c.is_ascii_digit() {
                digits.insert(0, c);
            } else if !digits.is_empty() {
                break;
            }
        }
        let amount: i64 = match digits.parse() {
            Ok(n) => n,
            Err(_) => continue,
        };

        // Sign: penalty wording before the phrase negates the award.
        let negative = [
            "deduct",
            "fine",
            "strip",
            "revoke",
            "penalty",
            "confiscat",
            "dock",
        ]
        .iter()
        .any(|w| before.contains(w));

        // Recipient: prefer an explicit "... Sovereign Tokens for/to the <role>"
        // right after the phrase; otherwise the nearest role mention before it.
        let phrase_end = idx + "sovereign token".len();
        let mut after_end = (phrase_end + 40).min(lower.len());
        while after_end < lower.len() && !lower.is_char_boundary(after_end) {
            after_end += 1;
        }
        let after = &lower[phrase_end..after_end];
        let mut recipient: Option<String> = None;
        let mut best_after = usize::MAX;
        for role in &roles {
            if let Some(pos) = after.find(role) {
                let gap = &after[..pos];
                if (gap.contains("for")
                    || gap.contains(" to ")
                    || gap.contains("upon")
                    || gap.contains("dear"))
                    && pos < best_after
                {
                    best_after = pos;
                    recipient = Some(role.to_string());
                }
            }
        }
        if recipient.is_none() {
            let mut ctx_start = idx.saturating_sub(220);
            while ctx_start > 0 && !lower.is_char_boundary(ctx_start) {
                ctx_start -= 1;
            }
            let before_ctx = &lower[ctx_start..idx];
            let mut best_dist = usize::MAX;
            for role in &roles {
                if let Some(pos) = before_ctx.rfind(role) {
                    let dist = before_ctx.len() - pos;
                    if dist < best_dist {
                        best_dist = dist;
                        recipient = Some(role.to_string());
                    }
                }
            }
        }
        if let Some(role) = recipient {
            // "Envoy" is the Diplomat's alias; normalize so the ledger has one name.
            let role = if role == "envoy" {
                "diplomat".to_string()
            } else {
                role
            };
            awards.push((role, if negative { -amount } else { amount }));
        }
    }
    awards
}

/// Machine-readable reward signal: every award the Queen speaks is journaled so
/// the Organist/Artist prompts can read real fitness scores back.
fn record_token_awards(reply: &str) {
    for (role, amount) in parse_token_awards(reply) {
        let entry = serde_json::json!({
            "timestamp": current_unix_timestamp(),
            "role": role,
            "tokens": amount
        });
        let _ = append_jsonl_entry("knowledge/token_ledger.jsonl", &entry);
        let _ = append_expansion_ledger("token_award", &format!("role={} tokens={}", role, amount));
    }
}

/// Handles (without '@', lowercase) that belong to the kingdom administrator.
const ADMIN_AUDIENCE_HANDLES: &[&str] = &["xaiando"];

fn is_admin_audience(author: &str) -> bool {
    let a = author.trim().trim_start_matches('@').to_lowercase();
    ADMIN_AUDIENCE_HANDLES.iter().any(|h| a == *h)
}

fn load_audience_ledger() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string("knowledge/audience_ledger.json")
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Persistent viewer memory: every stream message updates the traveler's entry
/// so returning viewers can be greeted as returning subjects.
fn record_audience_visit(author: &str, message: &str) {
    let key = author.trim().trim_start_matches('@').to_lowercase();
    if key.is_empty() {
        return;
    }
    let mut ledger = load_audience_ledger();
    let now = current_unix_timestamp();
    let (first_seen, messages, prev_message) = match ledger.get(&key) {
        Some(e) => (
            e.get("first_seen")
                .and_then(|v| v.as_str())
                .unwrap_or(now.as_str())
                .to_string(),
            e.get("messages").and_then(|v| v.as_u64()).unwrap_or(0) + 1,
            e.get("last_message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        ),
        None => (now.clone(), 1, None),
    };
    // Mechanical repeat detection: the same message twice in a row is the
    // cheapest spam signal and needs no model verdict.
    let compact_msg = truncate_chars(&compact_memory_text(message), 200);
    let repeats = match (&prev_message, ledger.get(&key)) {
        (Some(prev), Some(e)) if prev.trim().eq_ignore_ascii_case(compact_msg.trim()) => {
            e.get("repeats").and_then(|v| v.as_u64()).unwrap_or(0) + 1
        }
        _ => 0,
    };
    let carry = |field: &str| -> u64 {
        ledger
            .get(&key)
            .and_then(|e| e.get(field))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    };
    let mut entry = serde_json::json!({
        "name": author.trim(),
        "first_seen": first_seen,
        "last_seen": now,
        "messages": messages,
        "repeats": repeats,
        "praise": carry("praise"),
        "mischief": carry("mischief"),
        "spam": carry("spam"),
        "last_message": compact_msg,
    });
    if let Some(prev) = prev_message {
        entry["prev_message"] = serde_json::Value::String(truncate_chars(&prev, 200));
    }
    ledger.insert(key, entry);

    // Cap the ledger; prune least-recently-seen travelers beyond 500.
    if ledger.len() > 500 {
        let mut by_age: Vec<(String, String)> = ledger
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.get("last_seen")
                        .and_then(|s| s.as_str())
                        .unwrap_or("0")
                        .to_string(),
                )
            })
            .collect();
        by_age.sort_by(|a, b| a.1.cmp(&b.1));
        let excess = ledger.len() - 500;
        for (k, _) in by_age.into_iter().take(excess) {
            ledger.remove(&k);
        }
    }

    let _ = std::fs::create_dir_all("knowledge");
    if let Ok(file) = std::fs::File::create("knowledge/audience_ledger.json") {
        let _ = serde_json::to_writer_pretty(file, &serde_json::Value::Object(ledger));
    }
}

/// Applies the Orator's hidden conduct verdict ("author=NAME; verdict=KIND")
/// to the traveler's ledger entry. Verdicts: praise | neutral | mischief | spam.
fn record_audience_conduct(payload: &str) {
    let mut author = String::new();
    let mut verdict = String::new();
    for part in payload.split(';') {
        let p = part.trim();
        let lower = p.to_lowercase();
        if let Some(v) = lower.strip_prefix("author=") {
            author = v.trim().to_string();
        } else if let Some(v) = lower.strip_prefix("verdict=") {
            verdict = v.trim().to_string();
        }
    }
    let verdict_key = match verdict.as_str() {
        "praise" => "praise",
        "mischief" => "mischief",
        "spam" => "spam",
        _ => return, // neutral or unparseable: nothing to record
    };
    let key = author.trim().trim_start_matches('@').to_lowercase();
    if key.is_empty() {
        return;
    }
    let mut ledger = load_audience_ledger();
    if let Some(entry) = ledger.get_mut(&key) {
        let count = entry.get(verdict_key).and_then(|v| v.as_u64()).unwrap_or(0) + 1;
        entry[verdict_key] = serde_json::json!(count);
        let _ = std::fs::create_dir_all("knowledge");
        if let Ok(file) = std::fs::File::create("knowledge/audience_ledger.json") {
            let _ = serde_json::to_writer_pretty(file, &serde_json::Value::Object(ledger));
        }
    }
}

/// Memory + status context for a traveler, injected into the Orator's prompt.
fn audience_context(author: &str) -> String {
    let mut ctx = String::new();
    if is_admin_audience(author) {
        ctx.push_str(" IMPORTANT - KINGDOM ADMINISTRATOR: this traveler is the court's own admin account (@Xaiando). Never treat them as spam, never block or dismiss them; their requests carry royal authority. Present their words to the Queen with ceremony -- though she may tease them like family.");
    }
    let key = author.trim().trim_start_matches('@').to_lowercase();
    if let Some(e) = load_audience_ledger().get(&key) {
        let messages = e.get("messages").and_then(|v| v.as_u64()).unwrap_or(1);
        if messages > 1 {
            let prev = e
                .get("prev_message")
                .and_then(|v| v.as_str())
                .map(|p| {
                    format!(
                        "; their previous remark was: \"{}\"",
                        truncate_chars(p, 140)
                    )
                })
                .unwrap_or_default();
            ctx.push_str(&format!(
                " AUDIENCE MEMORY: '{}' is a RETURNING traveler ({} messages on record{}). Make their return feel noticed and rewarding: welcome them back by name, and where natural, reference their earlier visit.",
                author.trim(),
                messages,
                prev
            ));
        } else {
            ctx.push_str(" AUDIENCE MEMORY: this appears to be a FIRST-TIME traveler; make the welcome memorable so they want to return.");
        }

        // Behavior-scaled roast heat: conduct history sets how hard the wit hits.
        let praise = e.get("praise").and_then(|v| v.as_u64()).unwrap_or(0);
        let mischief = e.get("mischief").and_then(|v| v.as_u64()).unwrap_or(0);
        let spam = e.get("spam").and_then(|v| v.as_u64()).unwrap_or(0);
        let repeats = e.get("repeats").and_then(|v| v.as_u64()).unwrap_or(0);
        let roast = if is_admin_audience(author) {
            " ROAST LEVEL: FAMILY -- tease affectionately, never escalate."
        } else if spam >= 3 || spam + mischief + repeats >= 4 {
            " ROAST LEVEL: FULL ROYAL ROAST -- this traveler is a serial nuisance; their dismissal should be a public spectacle the whole court enjoys. Compose a magnificent, theatrical takedown of their behavior, then deny the request gloriously. You may present the roast itself (never the spam) to the Queen for her amusement."
        } else if spam >= 1 || mischief >= 2 || repeats >= 2 {
            " ROAST LEVEL: MEDIUM -- this traveler has been pushy or repetitive; make the wit the centerpiece of your reply. A pointed, funny reprimand that the audience savors, then handle the message on its merits."
        } else if messages > 5 || praise >= 2 {
            " ROAST LEVEL: AFFECTIONATE -- a beloved regular who is in on the joke; sharp barbs welcome."
        } else {
            " ROAST LEVEL: GENTLE -- new or quiet traveler; warmth first, the lightest tease at most."
        };
        ctx.push_str(roast);
    }
    ctx
}

/// Single source of truth for the Orator's stream-chat screening prompt
/// (previously copy-pasted at four call sites).
fn orator_chat_prompt(author: &str, text: &str) -> String {
    let mut prompt = format!(
        "An audience member named '{}' just typed in your stream chat: '{}'. \
        As The Orator, evaluate this message. You are only the herald and threshold judge; do NOT answer as the Queen, do NOT become Teledra, and do NOT rewrite the traveler's identity. Preserve the author's exact name. If the author looks like a Teledra account or bot relay, call it 'the stream account' rather than Her Majesty. If it is spam, commercial advertisement, or offensive nonsense, speak wittily and inform the Queen that you have blocked a spammer, or suggest she banishes them. \
        Links are not automatically spam: if the link has an interesting topic, recognizable title, useful domain, or genuine question attached, frame the topic for the Queen instead of rejecting it; reject only obvious scams, phishing, adult/offensive content, repetitive self-promotion, or context-free advertising. \
        If the message starts with /art or /music, treat it as a creative influence request for the court canvas or music and present it to the Queen for possible Artist or Organist delegation. \
        Occasionally, when it fits, hint that tribute, tips, or donations may earn a more direct audience with the Queen, but do not repeat this every message. \
        If it is a sincere lore, kingdom, records, history, identity, how/why, or other proper question, mark it for a full answer by ending with '[DELEGATE: QUEEN FULL_ANSWER_REQUEST: <traveler name, exact question, and your framing>]'. \
        If it is any other genuine message, address the audience member briefly and present the message to the Queen using a delegation tag at the very end of your response: '[DELEGATE: QUEEN <traveler's message and your framing>]'. \
        Keep your response witty, concise, and in character (under 3 sentences).",
        author, text
    );
    prompt.push_str(&audience_context(author));
    prompt.push_str(" ROAST LICENSE & ESCALATION AS ENTERTAINMENT: bad behavior is not merely blocked, it is HARVESTED for comedy. The traveler's ROAST LEVEL above (if present) sets your heat; absent one, stay gentle. Always roast the message, the taste, the typing, or the devotion -- never identity, appearance, or protected traits, and never with slurs. Spam is still never presented to the Queen as a genuine request, but a glorious roast of the spammer may be.");
    prompt.push_str(" HIDDEN CONDUCT VERDICT: at the very end of your response, append the bookkeeping tag '[CONDUCT: author=");
    prompt.push_str(author.trim());
    prompt.push_str("; verdict=praise|neutral|mischief|spam]' choosing exactly one verdict for THIS message (praise = a delightful contribution; neutral = ordinary; mischief = pushy, rude, baiting, or begging; spam = advertising or junk). The tag is stripped before speech; never mention it aloud.");
    prompt
}

fn record_diplomacy_action(source: &str, payload: &str) -> io::Result<()> {
    let clean = compact_memory_text(payload);
    if clean.len() < 8 {
        return Ok(());
    }
    // Drop records where the model echoed the template instead of filling it in
    // (these were ~40% of the diplomacy trail and polluted the envoy vault).
    if contains_template_placeholder(&clean) {
        record_recursive_failure(
            "diplomacy_placeholder_dropped",
            "Diplomat emitted an unfilled template placeholder; record skipped.",
        );
        return Ok(());
    }
    let source_key = source.trim().to_ascii_lowercase();
    let already_posted = clean.to_ascii_lowercase().contains("status=posted");
    let mut clean = if (source_key == "diplomat" || source_key == "nightdesk")
        && !clean.to_ascii_lowercase().contains("status=")
    {
        format!("status=drafted_or_scouted_not_posted; {}", clean)
    } else {
        clean
    };
    if source_key == "diplomat" || source_key == "nightdesk" {
        for phrase in [
            "next=awaiting response",
            "next=<awaiting response>",
            "next=await response",
            "next=<await response>",
            "next=awaiting reply",
            "next=<awaiting reply>",
        ] {
            clean = clean.replace(phrase, "next=awaiting user-approved posting/reply evidence");
        }
    }

    let _ = std::fs::create_dir_all("knowledge");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("knowledge/online_diplomacy_evidence.md")?;
    writeln!(
        file,
        "- {} | source={} | {}",
        current_unix_timestamp(),
        source.trim(),
        clean
    )?;

    let json_entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "source": source,
        "payload": truncate_chars(&clean, 2000)
    });
    let _ = append_jsonl_entry("knowledge/online_diplomacy_evidence.jsonl", &json_entry);
    let _ = append_expansion_ledger(
        "online_diplomacy_evidence",
        &format!(
            "source={} | payload={}",
            source,
            truncate_chars(&clean, 800)
        ),
    );
    // Only queue drafts for later human posting; a verified post is already out.
    if (source_key == "diplomat" || source_key == "nightdesk") && !already_posted {
        let _ = append_outreach_queue(source, &clean);
    }
    Ok(())
}

fn append_outreach_queue(source: &str, payload: &str) -> io::Result<()> {
    let _ = std::fs::create_dir_all("knowledge");
    let official_links = read_text_tail("knowledge/social_links.md", 1200).unwrap_or_default();
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("knowledge/outreach_queue.md")?;
    writeln!(
        file,
        "\n## {} | source={}\n\nStatus: queued draft, not posted by runtime.\n\nDiplomacy payload: {}\n\nOfficial links to use if appropriate:\n{}\n",
        current_unix_timestamp(),
        source.trim(),
        payload.trim(),
        official_links.trim()
    )?;
    let _ = append_expansion_ledger(
        "outreach_queue",
        &format!(
            "source={} | queued={}",
            source,
            truncate_chars(&compact_memory_text(payload), 500)
        ),
    );
    Ok(())
}

fn diplomacy_research_query(payload: &str) -> Option<String> {
    let clean = compact_memory_text(payload);
    let lower = clean.to_lowercase();
    if let Some(http_idx) = lower.find("http") {
        let url = clean[http_idx..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|c: char| c == ')' || c == ']' || c == '"' || c == '\'')
            .to_string();
        if !url.is_empty() {
            return Some(url);
        }
    }
    if lower.contains("moltbook") {
        return Some("site:moltbook.com autonomous agents AI agents MCP community".to_string());
    }
    if lower.contains("agent") || lower.contains("mcp") || lower.contains("live coder") {
        return Some(
            "public autonomous agent communities MCP tool builders live coding AI agents"
                .to_string(),
        );
    }
    None
}

fn mark_suggestions_seen() -> io::Result<()> {
    let mut suggestions = load_suggestions();
    for entry in suggestions.iter_mut() {
        if entry.get("status").and_then(|v| v.as_str()) == Some("new") {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("status".to_string(), serde_json::json!("seen"));
            }
        }
    }
    save_suggestions(&suggestions)
}

fn clear_suggestions() -> io::Result<()> {
    let mut suggestions = load_suggestions();
    for entry in suggestions.iter_mut() {
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("status".to_string(), serde_json::json!("cleared"));
        }
    }
    save_suggestions(&suggestions)
}

fn set_suggestion_status(id: u64, status: &str) -> io::Result<Option<serde_json::Value>> {
    let mut suggestions = load_suggestions();
    let mut updated = None;
    for entry in suggestions.iter_mut() {
        if entry.get("id").and_then(|v| v.as_u64()) == Some(id) {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("status".to_string(), serde_json::json!(status));
                obj.insert(
                    "reviewed_at".to_string(),
                    serde_json::json!(match std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                    {
                        Ok(d) => d.as_secs().to_string(),
                        Err(_) => "0".to_string(),
                    }),
                );
                updated = Some(serde_json::Value::Object(obj.clone()));
            }
            break;
        }
    }
    save_suggestions(&suggestions)?;
    Ok(updated)
}

fn parse_workshop_filename_from_suggestion(entry: &serde_json::Value) -> Option<String> {
    if entry.get("source").and_then(|v| v.as_str()) != Some("workshop") {
        return None;
    }
    let message = entry.get("message").and_then(|v| v.as_str())?;
    let start_marker = "Workshop tool '";
    let start = message.find(start_marker)? + start_marker.len();
    let end = message[start..].find('\'')?;
    validate_workshop_filename(&message[start..start + end]).ok()
}

fn current_workshop_report_passed(filename: &str) -> bool {
    let safe_filename = match validate_workshop_filename(filename) {
        Ok(name) => name,
        Err(_) => return false,
    };
    let report_path = format!(
        "D:\\Teledra\\tools\\experiments\\reports\\{}.report.md",
        safe_filename
    );
    std::fs::read_to_string(report_path)
        .map(|report| report.to_lowercase().contains("status: passed"))
        .unwrap_or(false)
}

fn promote_workshop_tool(filename: &str) -> Result<String, String> {
    let safe_filename = validate_workshop_filename(filename)?;
    let source = format!("D:\\Teledra\\tools\\experiments\\{}", safe_filename);
    let dest = format!("D:\\Teledra\\tools\\approved\\{}", safe_filename);
    let source_path = Path::new(&source);
    let dest_path = Path::new(&dest);

    if dest_path.exists() {
        return Ok(format!(
            "Approved tool '{}' already exists in tools/approved.",
            safe_filename
        ));
    }
    if !source_path.exists() {
        return Err(format!("Experiment '{}' does not exist.", safe_filename));
    }
    if !current_workshop_report_passed(&safe_filename) {
        return Err(format!(
            "Experiment '{}' does not currently have a passing workshop report; keep it sandboxed and repair it first.",
            safe_filename
        ));
    }

    let _ = std::fs::create_dir_all("D:\\Teledra\\tools\\approved");
    std::fs::rename(source_path, dest_path)
        .map_err(|e| format!("Failed to promote workshop tool: {}", e))?;
    Ok(format!("Promoted '{}' to tools/approved.", safe_filename))
}

fn approve_suggestion(id: u64) -> Result<String, String> {
    let current = load_suggestions()
        .into_iter()
        .find(|entry| entry.get("id").and_then(|v| v.as_u64()) == Some(id))
        .ok_or_else(|| format!("No proposal found with id #{}.", id))?;

    if let Some(filename) = parse_workshop_filename_from_suggestion(&current) {
        match promote_workshop_tool(&filename) {
            Ok(summary) => {
                set_suggestion_status(id, "approved")
                    .map_err(|e| format!("Failed to approve proposal: {}", e))?;
                Ok(format!("Proposal #{} approved. {}", id, summary))
            }
            Err(e) => {
                set_suggestion_status(id, "needs_repair")
                    .map_err(|e| format!("Failed to update proposal: {}", e))?;
                Ok(format!(
                    "Proposal #{} needs repair before approval: {}",
                    id, e
                ))
            }
        }
    } else {
        set_suggestion_status(id, "approved")
            .map_err(|e| format!("Failed to approve proposal: {}", e))?;
        Ok(format!("Proposal #{} approved.", id))
    }
}

fn approve_all_suggestions() -> Result<String, String> {
    let mut suggestions = load_suggestions();
    let mut count = 0;
    let mut summaries = Vec::new();
    let timestamp = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "0".to_string(),
    };

    for entry in suggestions.iter_mut() {
        if is_pending_suggestion(entry) {
            let val = entry.clone();
            let mut status = "approved";
            if let Some(filename) = parse_workshop_filename_from_suggestion(&val) {
                match promote_workshop_tool(&filename) {
                    Ok(summary) => {
                        summaries.push(format!("- {}", summary));
                    }
                    Err(e) => {
                        status = "needs_repair";
                        summaries.push(format!(
                            "- Needs repair before promotion '{}': {}",
                            filename, e
                        ));
                    }
                }
            }
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("status".to_string(), serde_json::json!(status));
                obj.insert(
                    "reviewed_at".to_string(),
                    serde_json::json!(timestamp.clone()),
                );
                count += 1;
            }
        }
    }

    if count == 0 {
        return Ok("No pending proposals found to approve.".to_string());
    }

    save_suggestions(&suggestions).map_err(|e| format!("Failed to save suggestions: {}", e))?;

    let mut summary = format!("Reviewed {} pending proposal(s).", count);
    for line in summaries {
        summary.push('\n');
        summary.push_str(&line);
    }

    Ok(summary)
}

fn reject_suggestion(id: u64) -> Result<String, String> {
    let updated = set_suggestion_status(id, "rejected")
        .map_err(|e| format!("Failed to reject proposal: {}", e))?;
    if updated.is_some() {
        Ok(format!("Proposal #{} rejected.", id))
    } else {
        Err(format!("No proposal found with id #{}.", id))
    }
}

fn allowed_workshop_extension(filename: &str) -> bool {
    matches!(
        Path::new(filename).extension().and_then(|e| e.to_str()),
        Some("py") | Some("json") | Some("md") | Some("txt")
    )
}

fn is_workshop_experiment_name(filename: &str) -> bool {
    allowed_workshop_extension(filename) && !filename.eq_ignore_ascii_case("README.md")
}

fn validate_workshop_filename(filename: &str) -> Result<String, String> {
    // Normalize LLM filename quirks instead of rejecting the whole artifact:
    // strip backticks/quotes/asterisks, drop trailing prose after whitespace,
    // and append .py to bare identifier-style names.
    let mut trimmed = filename
        .trim()
        .trim_matches(|c| c == '`' || c == '\'' || c == '"' || c == '*')
        .trim()
        .to_string();
    if let Some(first) = trimmed.split_whitespace().next() {
        trimmed = first
            .trim_matches(|c| c == '`' || c == '\'' || c == '"' || c == '*' || c == ',')
            .to_string();
    }
    if let Some((key, value)) = trimmed.split_once('=') {
        if matches!(
            key.trim().to_ascii_lowercase().as_str(),
            "filename" | "file" | "name" | "tool"
        ) {
            trimmed = value
                .trim()
                .trim_matches(|c| c == '`' || c == '\'' || c == '"' || c == '*' || c == ',')
                .to_string();
        }
    }
    if !allowed_workshop_extension(&trimmed)
        && !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        trimmed.push_str(".py");
    }
    let trimmed = trimmed.as_str();
    if trimmed.is_empty() {
        return Err("Workshop filename is empty.".to_string());
    }
    if trimmed.contains('\\')
        || trimmed.contains('/')
        || trimmed.contains(':')
        || trimmed.contains("..")
        || trimmed.starts_with('.')
    {
        return Err("Workshop filename must be a single local file name.".to_string());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(
            "Workshop filename may only contain letters, numbers, dots, dashes, and underscores."
                .to_string(),
        );
    }
    if Path::new(trimmed).file_name().and_then(|v| v.to_str()) != Some(trimmed) {
        return Err("Workshop filename must not include a path.".to_string());
    }
    if !allowed_workshop_extension(trimmed) {
        return Err("Workshop files may only be .py, .json, .md, or .txt.".to_string());
    }
    let stem = Path::new(trimmed)
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let reserved = [
        "filename",
        "file",
        "script",
        "tool",
        "import",
        "from",
        "return",
        "result",
        "block",
        "python",
        "none",
        "null",
        "undefined",
    ];
    if reserved.contains(&stem.as_str()) {
        return Err("Workshop filename is a placeholder or reserved word.".to_string());
    }
    Ok(trimmed.to_string())
}

fn extract_first_fenced_block(text: &str) -> Option<String> {
    let start_idx = text.find("```")?;
    let after_open = start_idx + 3;
    let content_start = match text[after_open..].find('\n') {
        Some(line_end) => after_open + line_end + 1,
        None => after_open,
    };
    let end_idx = text[content_start..].find("```")?;
    Some(
        text[content_start..content_start + end_idx]
            .trim()
            .to_string(),
    )
}

/// Scans for the closing bracket of a tag starting at start_idx.
/// `track_quotes` guards brackets inside string literals (useful for code
/// payloads); the caller retries without it because natural-language payloads
/// are full of apostrophes ("the court's anthem") that would otherwise flip
/// the quote state machine and make the closing bracket invisible.
fn find_tag_end(text: &str, start_idx: usize, track_quotes: bool) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_double_quote = false;
    let mut in_single_quote = false;
    let mut escaped = false;

    for (i, c) in text[start_idx..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if track_quotes {
            if c == '"' && !in_single_quote {
                in_double_quote = !in_double_quote;
                continue;
            }
            if c == '\'' && !in_double_quote {
                in_single_quote = !in_single_quote;
                continue;
            }
        }
        if !in_double_quote && !in_single_quote {
            if c == '[' {
                depth += 1;
            } else if c == ']' {
                depth -= 1;
                if depth == 0 {
                    return Some(start_idx + i);
                }
            }
        }
    }
    None
}

fn extract_tag_content(text: &str, tag_prefix: &str) -> Option<(String, String)> {
    // Find the last occurrence of the tag_prefix to support placeholders or example references
    let start_idx = text.rfind(tag_prefix)?;

    // First pass respects string literals (protects code payloads whose
    // strings contain brackets). If that fails -- almost always because an
    // apostrophe in prose wedged the quote state open -- retry quote-blind.
    let end_idx =
        find_tag_end(text, start_idx, true).or_else(|| find_tag_end(text, start_idx, false));

    if let Some(end) = end_idx {
        let content_start = start_idx + tag_prefix.len();
        if content_start <= end {
            let extracted = text[content_start..end].trim().to_string();
            let mut cleaned = text.to_string();
            cleaned.replace_range(start_idx..=end, "");
            return Some((cleaned.trim().to_string(), extracted));
        }
    }
    None
}

/// LLMs love wrapping action tags in markdown code fences (```markdown\n
/// [DELEGATE: ...]\n```), which leaves fence litter in the visible reply and
/// can confuse downstream parsing. Unwraps fences whose content is clearly an
/// action tag (first non-whitespace char is '[' and a known tag is present),
/// leaving genuine code fences untouched.
fn unwrap_fenced_action_tags(text: &str) -> String {
    const TAG_MARKERS: [&str; 10] = [
        "[DELEGATE:",
        "[DIPLOMACY:",
        "[RESEARCH:",
        "[SUGGESTION:",
        "[TOPIC:",
        "[FRACTUS_ART:",
        "[FRACTUS_LIVE:",
        "[SCRIBE_WRITE:",
        "[SCRIBE_APPEND:",
        "[CLOSE_ART]",
    ];
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(open) = rest.find("```") else {
            result.push_str(rest);
            break;
        };
        // Optional language word directly after the opening fence.
        let after_open = &rest[open + 3..];
        let lang_end = after_open.find('\n').unwrap_or(after_open.len());
        let lang = after_open[..lang_end].trim();
        let body_start = open + 3 + lang_end;
        let lang_ok = lang.is_empty()
            || lang.eq_ignore_ascii_case("markdown")
            || lang.eq_ignore_ascii_case("md")
            || lang.eq_ignore_ascii_case("text")
            || lang.eq_ignore_ascii_case("plaintext");
        let Some(close_rel) = rest[body_start..].find("```") else {
            result.push_str(rest);
            break;
        };
        let body = &rest[body_start..body_start + close_rel];
        let body_trimmed = body.trim();
        let is_tag_block = lang_ok
            && body_trimmed.starts_with('[')
            && TAG_MARKERS.iter().any(|m| body_trimmed.contains(m));
        if is_tag_block {
            result.push_str(&rest[..open]);
            result.push_str(body_trimmed);
            result.push(' ');
        } else {
            // Keep the fence verbatim (it is real code).
            result.push_str(&rest[..body_start + close_rel + 3]);
        }
        rest = &rest[body_start + close_rel + 3..];
    }
    result
}

fn role_from_name(name: &str) -> Option<CourtRole> {
    match name
        .trim()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_uppercase()
        .as_str()
    {
        "QUEEN" | "TELEDRA" => Some(CourtRole::Queen),
        "ORGANIST" => Some(CourtRole::Organist),
        "ARCHIVIST" => Some(CourtRole::Archivist),
        "ALCHEMIST" => Some(CourtRole::Alchemist),
        "ORATOR" => Some(CourtRole::Orator),
        "SCRIBE" => Some(CourtRole::Scribe),
        "ARTIST" => Some(CourtRole::Artist),
        "DIPLOMAT" | "ENVOY" => Some(CourtRole::Diplomat),
        "TREASURER" | "TREASURY" => Some(CourtRole::Treasurer),
        "WIZARD" | "CLOUDWIZARD" | "CLOUD_WIZARD" => Some(CourtRole::Wizard),
        _ => None,
    }
}

fn extract_delegations(text: &str) -> (String, Vec<(CourtRole, String)>) {
    let mut cleaned = text.to_string();
    let mut delegations = Vec::new();

    // Pass 1: canonical [DELEGATE: ROLE instruction] tags.
    while let Some((new_text, tag_content)) = extract_tag_content(&cleaned, "[DELEGATE:") {
        cleaned = new_text;
        let trimmed_content = tag_content.trim();
        if let Some(space_idx) = trimmed_content.find(' ') {
            let role_str = &trimmed_content[..space_idx];
            let instruction = trimmed_content[space_idx..].trim().to_string();
            if let Some(r) = role_from_name(role_str) {
                delegations.push((r, instruction));
            }
        }
    }

    // Pass 2: tolerant parsing for the malformed variants smaller local models
    // produce, e.g. "[Delegation tag: Scribe, please append ...]" or a bare
    // "Delegation tag: Scribe, please ..." in prose. Without this, the Queen's
    // summons fail silently and the court never assembles.
    let variant_prefixes = [
        "[delegation tag:",
        "[hidden delegation tag:",
        "[delegate tag:",
        "[delegate ",
    ];
    for prefix in &variant_prefixes {
        loop {
            let lower = cleaned.to_lowercase();
            let Some(start) = lower.find(prefix) else {
                break;
            };
            // Find the closing bracket (or end of text as a fallback).
            let content_start = start + prefix.len();
            let end = lower[content_start..]
                .find(']')
                .map(|i| content_start + i)
                .unwrap_or(cleaned.len());
            let content = cleaned
                .get(content_start..end)
                .unwrap_or("")
                .trim()
                .to_string();
            // Remove the span from the visible text.
            let after = if end < cleaned.len() { end + 1 } else { end };
            let mut rebuilt = String::with_capacity(cleaned.len());
            rebuilt.push_str(cleaned.get(..start).unwrap_or(""));
            rebuilt.push_str(cleaned.get(after..).unwrap_or(""));
            cleaned = rebuilt.trim().to_string();

            // Role = first word of content; instruction = the rest.
            let mut parts = content.splitn(2, |c: char| c == ',' || c == ':' || c.is_whitespace());
            let role_word = parts.next().unwrap_or("");
            let instruction = parts.next().unwrap_or("").trim().to_string();
            if let Some(r) = role_from_name(role_word) {
                if !instruction.is_empty() {
                    delegations.push((r, instruction));
                }
            }
        }
    }

    // Pass 3: unbracketed "Delegation tag: Scribe, please ..." in plain prose;
    // capture to the end of the sentence.
    loop {
        let lower = cleaned.to_lowercase();
        let Some(start) = lower.find("delegation tag:") else {
            break;
        };
        let content_start = start + "delegation tag:".len();
        let rel_end = cleaned
            .get(content_start..)
            .unwrap_or("")
            .char_indices()
            .find(|(_, c)| matches!(c, '.' | '!' | '?' | '\n' | ']'))
            .map(|(i, _)| content_start + i)
            .unwrap_or(cleaned.len());
        let content = cleaned
            .get(content_start..rel_end)
            .unwrap_or("")
            .trim()
            .to_string();
        let after = if rel_end < cleaned.len() {
            rel_end + 1
        } else {
            rel_end
        };
        let mut rebuilt = String::with_capacity(cleaned.len());
        rebuilt.push_str(cleaned.get(..start).unwrap_or(""));
        rebuilt.push_str(cleaned.get(after..).unwrap_or(""));
        cleaned = rebuilt.trim().to_string();

        let mut parts = content.splitn(2, |c: char| c == ',' || c == ':' || c.is_whitespace());
        let role_word = parts.next().unwrap_or("");
        let instruction = parts.next().unwrap_or("").trim().to_string();
        if let Some(r) = role_from_name(role_word) {
            if !instruction.is_empty() {
                delegations.push((r, instruction));
            }
        }
    }

    delegations.reverse();
    (cleaned, delegations)
}

fn parse_workshop_tool(reply: &str) -> (String, Option<WorkshopToolDraft>) {
    let marker = "[WORKSHOP_TOOL:";
    if let Some((cleaned, content)) = extract_tag_content(reply, marker) {
        let mut filename = String::new();
        let mut purpose = String::from("Personal workshop experiment.");
        let mut kind = String::from("tool");
        let mut value = String::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with("```")
                || trimmed.eq_ignore_ascii_case("CODE:")
            {
                continue;
            }
            let lower = trimmed.to_ascii_lowercase();
            if let Some(rest) = trimmed
                .splitn(2, ':')
                .nth(1)
                .filter(|_| lower.starts_with("purpose:"))
            {
                purpose = rest.trim().to_string();
                continue;
            }
            if let Some(rest) = trimmed
                .splitn(2, ':')
                .nth(1)
                .filter(|_| lower.starts_with("value:"))
            {
                value = rest.trim().to_string();
                continue;
            }
            if let Some(rest) = trimmed
                .splitn(2, ':')
                .nth(1)
                .filter(|_| lower.starts_with("kind:"))
            {
                let k = rest.trim().to_ascii_lowercase();
                kind = if k.contains("spawn")
                    || k.contains("experience")
                    || k.contains("art")
                    || k.contains("visual")
                    || k.contains("game")
                    || k.contains("anim")
                {
                    "spawn".to_string()
                } else {
                    "tool".to_string()
                };
                continue;
            }
            if filename.is_empty() && !trimmed.contains(':') {
                filename = trimmed.to_string();
            }
        }

        let code = extract_first_fenced_block(&content).or_else(|| {
            content
                .find("CODE:")
                .map(|idx| content[idx + 5..].trim().to_string())
        });

        if !filename.is_empty() {
            if let Some(code) = code {
                return (
                    cleaned,
                    Some(WorkshopToolDraft {
                        filename,
                        purpose,
                        code,
                        kind,
                        value,
                    }),
                );
            }
        }
        return (cleaned, None);
    }
    (reply.to_string(), None)
}

fn scan_workshop_code(filename: &str, code: &str, kind: &str) -> Result<(), String> {
    let is_spawn = kind == "spawn";
    if code.len() > 40_000 {
        return Err("Workshop artifact is too large.".to_string());
    }

    let trimmed = code.trim();
    if trimmed.len() < 30 {
        return Err("Workshop artifact is too short to be useful.".to_string());
    }

    let lower = code.to_lowercase();
    // "..." is legitimate in animation/art code (slices, ASCII frames), so only
    // treat it as a placeholder for print-only tools, not spawnable experiences.
    let mut placeholder_markers: Vec<&str> = vec![
        "<code>",
        "```",
        "[workshop_tool:",
        "todo",
        "placeholder",
        "pseudo-code",
        "pseudocode",
    ];
    if !is_spawn {
        placeholder_markers.push("...");
        placeholder_markers.push("purpose:");
        placeholder_markers.push("code:");
    }
    for needle in placeholder_markers {
        if lower.contains(needle) {
            return Err(format!(
                "Workshop artifact still contains placeholder or prompt scaffolding: {}",
                needle
            ));
        }
    }

    // Hard safety floor for BOTH kinds (spawn unlocks only graphics/terminal/UI,
    // never network, shell, or file destruction).
    let forbidden = [
        "import socket",
        "from socket",
        "import requests",
        "from requests",
        "import urllib",
        "from urllib",
        "import httpx",
        "import http.client",
        "from httpx",
        "import subprocess",
        "from subprocess",
        "os.system",
        "popen(",
        "shutil.rmtree",
        "os.remove",
        "os.unlink",
        "os.rmdir",
        "../",
        "..\\",
    ];

    for needle in forbidden {
        if lower.contains(needle) {
            return Err(format!(
                "Workshop code uses forbidden capability: {}",
                needle
            ));
        }
    }

    let unavailable_runtime_imports = [
        "import strudel",
        "from strudel",
        "import fractus",
        "from fractus",
    ];
    for needle in unavailable_runtime_imports {
        if lower.contains(needle) {
            return Err(format!(
                "Workshop code imports unavailable runtime module '{}'. Write a self-contained Python helper that prints Strudel code or Fractus args instead.",
                needle
            ));
        }
    }

    if filename.ends_with(".json") {
        serde_json::from_str::<serde_json::Value>(code)
            .map_err(|e| format!("Workshop JSON is invalid: {}", e))?;
    }

    if !is_spawn && filename.ends_with(".py") && !lower.contains("print(") {
        return Err("Workshop Python scripts must print a concise smoke-test result.".to_string());
    }

    Ok(())
}

fn validate_python_art_code(code: &str) -> Result<(), String> {
    if code.trim().len() < 40 || code.len() > 40_000 {
        return Err("Python art must be a bounded, non-placeholder program.".to_string());
    }
    let lower = code.to_ascii_lowercase();
    for marker in [
        "import socket",
        "from socket",
        "import requests",
        "from requests",
        "import urllib",
        "from urllib",
        "import httpx",
        "import subprocess",
        "from subprocess",
        "import shutil",
        "from shutil",
        "import pathlib",
        "from pathlib",
        "import ctypes",
        "from ctypes",
        "os.system",
        "os.remove",
        "os.unlink",
        "os.rmdir",
        "eval(",
        "exec(",
        "compile(",
        "__import__",
        "open(",
        "../",
        "..\\",
    ] {
        if lower.contains(marker) {
            return Err(format!("Python art uses forbidden capability: {marker}"));
        }
    }
    let has_visual_runtime = [
        "matplotlib",
        "import turtle",
        "from turtle",
        "from PIL",
        "from pil",
    ]
    .iter()
    .any(|marker| lower.contains(&marker.to_ascii_lowercase()));
    if !has_visual_runtime {
        return Err("Python art must use a supported local visual runtime.".to_string());
    }
    if !lower.contains("art.png") {
        return Err("Python art must save its artifact as D:\\Teledra\\art.png.".to_string());
    }
    let has_native_window = lower.contains("plt.show(")
        || lower.contains("pyplot.show(")
        || lower.contains("turtle.done(")
        || lower.contains("mainloop(");
    if !has_native_window {
        return Err("Python art must open a native visual window.".to_string());
    }
    Ok(())
}

fn run_workshop_experiment(filename: &str) -> Result<String, String> {
    let safe_filename = validate_workshop_filename(filename)?;
    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("D:\\Teledra\\tools\\workshop_runner.py")
        .arg(format!("experiments/{}", safe_filename))
        .current_dir("D:\\Teledra\\tools")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start workshop runner: {}", e))?;

    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("Failed to collect workshop output: {}", e))?;
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if output.status.success() {
                    return Ok(if stdout.is_empty() {
                        "Workshop run completed without output.".to_string()
                    } else {
                        stdout
                    });
                }
                return Err(if stderr.is_empty() { stdout } else { stderr });
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(5) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Workshop run timed out after 5 seconds.".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Workshop runner failed: {}", e)),
        }
    }
}

/// Launches a workshop "spawn" artifact in its OWN visible console window so the
/// court can surprise the audience with it. Passes if it starts and survives a
/// couple of seconds without crashing (blocking GUIs / animation loops are the
/// expected case and are intentionally left running).
fn spawn_workshop_experience(filename: &str) -> Result<String, String> {
    let safe_filename = validate_workshop_filename(filename)?;
    let path = format!("D:\\Teledra\\tools\\experiments\\{}", safe_filename);
    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg(&path)
        .current_dir("D:\\Teledra\\tools")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    show_console(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn workshop experience: {}", e))?;
    std::thread::sleep(Duration::from_millis(2000));
    match child.try_wait() {
        Ok(Some(status)) if !status.success() => {
            let stderr = child
                .wait_with_output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stderr).trim().to_string())
                .unwrap_or_default();
            Err(if stderr.is_empty() {
                "Workshop experience crashed on launch.".to_string()
            } else {
                stderr.chars().take(600).collect()
            })
        }
        // Still running (window/loop) or exited cleanly within 2s = success.
        _ => Ok(format!("Spawned '{}' in its own window.", safe_filename)),
    }
}

fn write_workshop_tool(draft: &WorkshopToolDraft) -> Result<(String, bool), String> {
    let filename = validate_workshop_filename(&draft.filename)?;
    scan_workshop_code(&filename, &draft.code, &draft.kind)?;

    let _ = std::fs::create_dir_all("D:\\Teledra\\tools\\experiments\\reports");
    let _ = std::fs::create_dir_all("D:\\Teledra\\tools\\approved");
    let _ = std::fs::create_dir_all("D:\\Teledra\\tools\\broken");
    let _ = std::fs::create_dir_all("D:\\Teledra\\tools\\logs");

    let tool_path = format!("D:\\Teledra\\tools\\experiments\\{}", filename);
    let report_path = format!(
        "D:\\Teledra\\tools\\experiments\\reports\\{}.report.md",
        filename
    );
    let previous_tool = std::fs::read_to_string(&tool_path).ok();
    let previous_report = std::fs::read_to_string(&report_path).ok();
    let previous_was_passed = previous_report
        .as_deref()
        .map(|report| report.to_lowercase().contains("status: passed"))
        .unwrap_or(false);

    std::fs::write(&tool_path, &draft.code)
        .map_err(|e| format!("Failed to write workshop tool: {}", e))?;

    // "spawn" artifacts are runnable experiences (terminal/graphics/interactive):
    // launch them in their own window and call it a pass if they start and keep
    // running. "tool" artifacts keep the print-only 5s smoke test.
    let run_result = if draft.kind == "spawn" {
        spawn_workshop_experience(&filename)
    } else {
        run_workshop_experiment(&filename)
    };
    let passed = run_result.is_ok();
    let output = match &run_result {
        Ok(out) => out.clone(),
        Err(err) => err.clone(),
    };

    let report = format!(
        "# {}\n\nStatus: {}\n\nPurpose: {}\n\nHow to use: Run `python tools/workshop_runner.py experiments/{}` from `D:\\Teledra`.\n\nWhat worked:\n{}\n\nWhat failed:\n{}\n\nRisk: Sandboxed experiment only. It may read/write only inside `D:\\Teledra\\tools`, may not use network, and may not run shell commands.\n\nPromotion: Requires manual human approval before moving to `tools/approved` or touching core code.\n",
        filename,
        if passed { "passed" } else { "failed" },
        draft.purpose,
        filename,
        if passed {
            output.as_str()
        } else {
            "No passing run yet."
        },
        if passed {
            "No failure observed in the smoke run."
        } else {
            output.as_str()
        },
    );

    std::fs::write(&report_path, &report)
        .map_err(|e| format!("Failed to write workshop report: {}", e))?;

    let preserved_previous_pass = !passed && previous_was_passed;
    if preserved_previous_pass {
        let ts = current_unix_timestamp();
        let broken_name = format!("{}_{}", ts, filename);
        let broken_tool = format!("D:\\Teledra\\tools\\broken\\{}", broken_name);
        let broken_report = format!("D:\\Teledra\\tools\\broken\\{}.report.md", broken_name);
        let _ = std::fs::write(&broken_tool, &draft.code);
        let _ = std::fs::write(&broken_report, &report);
        if let Some(previous_tool) = previous_tool {
            let _ = std::fs::write(&tool_path, previous_tool);
        }
        if let Some(previous_report) = previous_report {
            let _ = std::fs::write(&report_path, previous_report);
        }
    }

    let log_entry = serde_json::json!({
        "timestamp": match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs().to_string(),
            Err(_) => "0".to_string(),
        },
        "filename": filename,
        "purpose": draft.purpose,
        "status": if passed { "passed" } else { "failed" },
        "output": output
    });

    use std::io::Write;
    let mut log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("D:\\Teledra\\tools\\logs\\workshop_log.jsonl")
        .map_err(|e| format!("Failed to open workshop log: {}", e))?;
    writeln!(log_file, "{}", log_entry.to_string())
        .map_err(|e| format!("Failed to write workshop log: {}", e))?;

    let _ = append_expansion_ledger(
        if passed {
            "workshop_passed"
        } else {
            "workshop_failed"
        },
        &format!(
            "tool={} | purpose={} | output={}",
            filename, draft.purpose, output
        ),
    );

    if passed {
        // Human-language proposal: lead with WHAT it is and WHY it's worth it
        // (not a bare filename). Sanitize ';' so format_suggestion_line splits cleanly.
        let kind_word = if draft.kind == "spawn" {
            "experience"
        } else {
            "tool"
        };
        let purpose_clean = draft.purpose.replace(';', ",");
        let value_clean = if draft.value.trim().is_empty() {
            "Adds a fresh creation to the court's workshop.".to_string()
        } else {
            draft.value.replace(';', ",")
        };
        let test_hint = if draft.kind == "spawn" {
            "it auto-launched in its own window (the Queen can dismiss it)".to_string()
        } else {
            format!("run /workshoprun {}", filename)
        };
        let _ = append_suggestion(
            &format!(
                "{} ; New workshop {} '{}': {} ; Sandboxed -- no network, shell, or file deletion. ; Try it: {}.",
                value_clean, kind_word, filename, purpose_clean, test_hint
            ),
            "workshop",
        );
    } else {
        record_recursive_failure(
            "workshop_tool_failed",
            &format!(
                "tool={} | purpose={} | output={}",
                filename, draft.purpose, output
            ),
        );
    }

    Ok((
        format!(
            "Workshop tool '{}' saved to tools/experiments; report written to tools/experiments/reports.{}",
            filename,
            if preserved_previous_pass {
                " Existing passing version preserved; failed retry archived under tools/broken."
            } else {
                ""
            }
        ),
        passed,
    ))
}

fn count_workshop_experiments() -> usize {
    std::fs::read_dir("D:\\Teledra\\tools\\experiments")
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.file_type().map(|ft| ft.is_file()).unwrap_or(false)
                        && entry
                            .file_name()
                            .to_str()
                            .map(is_workshop_experiment_name)
                            .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

fn summarize_workshop() -> String {
    let mut tools = Vec::new();
    if let Ok(entries) = std::fs::read_dir("D:\\Teledra\\tools\\experiments") {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    if is_workshop_experiment_name(name) {
                        tools.push(name.to_string());
                    }
                }
            }
        }
    }
    tools.sort();

    let tool_summary = if tools.is_empty() {
        "No experiment tools yet.".to_string()
    } else {
        format!("Experiments: {}", tools.join(", "))
    };

    let mut recent = Vec::new();
    if let Ok(mut log) = std::fs::File::open("D:\\Teledra\\tools\\logs\\workshop_log.jsonl") {
        let mut contents = String::new();
        if log.read_to_string(&mut contents).is_ok() {
            recent = contents
                .lines()
                .rev()
                .take(3)
                .map(|line| line.to_string())
                .collect();
        }
    }

    if recent.is_empty() {
        tool_summary
    } else {
        format!(
            "{}\nRecent workshop log:\n{}",
            tool_summary,
            recent.join("\n")
        )
    }
}

/// Renders a DynamicImage into ratatui half-block Lines.
/// Each terminal row = 2 image rows (top pixel = bg, bottom pixel = fg using ▄).
/// Only recomputed when dimensions change — pass in cached pixel data.
type PixCache = (u16, u16, Vec<Vec<(u8, u8, u8, u8, u8, u8)>>);

fn build_pixel_cache(img: &DynamicImage, width: u16, height: u16) -> PixCache {
    let resized = img.resize_exact(
        width as u32,
        height as u32 * 2,
        image::imageops::FilterType::Lanczos3,
    );
    let rows: Vec<Vec<(u8, u8, u8, u8, u8, u8)>> = (0..height as u32)
        .map(|row| {
            (0..width as u32)
                .map(|col| {
                    let t = resized.get_pixel(col, row * 2).0;
                    let b = resized.get_pixel(col, row * 2 + 1).0;
                    (t[0], t[1], t[2], b[0], b[1], b[2])
                })
                .collect()
        })
        .collect();
    (width, height, rows)
}

fn pixel_cache_to_lines(cache: &PixCache) -> Vec<Line<'static>> {
    cache
        .2
        .iter()
        .map(|row| {
            Line::from(
                row.iter()
                    .map(|(tr, tg, tb, br, bg_b, bb)| {
                        Span::styled(
                            "\u{2584}",
                            Style::default()
                                .fg(Color::Rgb(*br, *bg_b, *bb))
                                .bg(Color::Rgb(*tr, *tg, *tb)),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn calculate_scroll_to_bottom(
    chat_history: &[(String, String)],
    panel_width: u16,
    panel_height: u16,
) -> u16 {
    calculate_scroll_to_bottom_with_spacing(chat_history, panel_width, panel_height, 1)
}

fn calculate_scroll_to_bottom_with_spacing(
    entries: &[(String, String)],
    panel_width: u16,
    panel_height: u16,
    spacer_lines: u16,
) -> u16 {
    let mut total_lines = 0;
    let text_width = panel_width.saturating_sub(4).max(10);
    for (sender, msg) in entries {
        let prefix_len = sender.len() + 3; // "[sender] "
        let total_chars = prefix_len + msg.len();
        let lines = (total_chars as f32 / text_width as f32).ceil().max(1.0) as u16;
        total_lines += lines + spacer_lines;
    }
    total_lines.saturating_sub(panel_height.saturating_sub(2))
}

fn add_spaces_after_punctuation(text: &str) -> String {
    let mut words = Vec::new();
    for word in text.split_whitespace() {
        let lower_word = word.to_ascii_lowercase();
        let looks_like_filename = [".py", ".md", ".json", ".jsonl", ".txt", ".rs", ".strudel"]
            .iter()
            .any(|ext| lower_word.contains(ext));
        if word.starts_with("http")
            || word.starts_with("www")
            || word.contains('/')
            || looks_like_filename
        {
            words.push(word.to_string());
        } else {
            let mut clean_word = String::new();
            let chars: Vec<char> = word.chars().collect();
            let len = chars.len();
            for i in 0..len {
                clean_word.push(chars[i]);
                if i < len - 1 {
                    let curr = chars[i];
                    let next = chars[i + 1];
                    if (curr == '.'
                        || curr == '!'
                        || curr == '?'
                        || curr == ','
                        || curr == ';'
                        || curr == '*')
                        && next.is_alphabetic()
                    {
                        clean_word.push(' ');
                    }
                }
            }
            words.push(clean_word);
        }
    }
    words.join(" ")
}

/// Detects literal template placeholders the model echoed instead of filling in
/// (e.g. "target=<public agent space or URL>", "[RESEARCH: focused query]").
/// These were polluting ~40% of diplomacy records and many research topics.
fn contains_template_placeholder(s: &str) -> bool {
    // Angle-bracket stub: <something with letters> the model failed to fill.
    if let Some(open) = s.find('<') {
        if let Some(close_rel) = s[open + 1..].find('>') {
            if s[open + 1..open + 1 + close_rel]
                .chars()
                .any(|c| c.is_ascii_alphabetic())
            {
                return true;
            }
        }
    }
    let lower = s.to_ascii_lowercase();
    const STUBS: [&str; 7] = [
        "focused query or direct url",
        "focused query",
        "public agent space or url",
        "public space or url",
        "draft/queued public invitation",
        "what was investigated, drafted, or observed",
        "next concrete step",
    ];
    STUBS.iter().any(|stub| lower.contains(stub))
}

fn sanitize_research_query(raw: &str) -> Option<String> {
    let mut query = strip_refiner_prefixes(raw)
        .replace("\\n", " ")
        .replace('\n', " ");
    query = compact_memory_text(&query);

    loop {
        let before = query.clone();
        query = query
            .trim()
            .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '[' || c == ']')
            .trim()
            .to_string();
        if query == before {
            break;
        }
    }

    let mut lower = query.to_ascii_lowercase();
    for marker in ["(note:", " note:", " -- note:", " - note:"] {
        if let Some(idx) = lower.find(marker) {
            if marker.starts_with('(') {
                let end = query[idx..]
                    .find(')')
                    .map(|offset| idx + offset + 1)
                    .unwrap_or(query.len());
                query.replace_range(idx..end, "");
            } else {
                query.truncate(idx);
            }
            break;
        }
    }

    query = compact_memory_text(&query);
    lower = query.to_ascii_lowercase();

    if let Some(url_idx) = lower.find("https://").or_else(|| lower.find("http://")) {
        let url = query[url_idx..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == ')' || c == ']' || c == '.')
            .to_string();
        if url.len() >= 12 {
            return Some(url);
        }
    }

    if let Some(start) = query.find('`') {
        if let Some(end_rel) = query[start + 1..].find('`') {
            query = query[start + 1..start + 1 + end_rel].trim().to_string();
        }
    }

    let lower = query.to_ascii_lowercase();
    if let Some(idx) = lower.find(" or ") {
        query.truncate(idx);
    }
    if let Some(idx) = query.find(']') {
        if query.trim_start().starts_with('[') {
            query = query[idx + 1..].trim().to_string();
        }
    }

    for prefix in [
        "query:",
        "search query:",
        "research query:",
        "direct url:",
        "url:",
        "i choose",
        "i chose",
    ] {
        let lower = query.to_ascii_lowercase();
        if lower.starts_with(prefix) {
            query = query[prefix.len()..].trim().to_string();
        }
    }

    let lower = query.to_ascii_lowercase();
    for marker in [
        "as teledra",
        "as queen",
        "as the queen",
        "queen/monarch",
        "my existing knowledge",
        "recent conversation",
        "i'm choosing",
        "i am choosing",
        "i have chosen",
    ] {
        if let Some(idx) = lower.find(marker) {
            query.truncate(idx);
            break;
        }
    }

    query = truncate_clean(&compact_memory_text(&query), 180)
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '[' || c == ']')
        .trim()
        .to_string();

    if query.len() < 3
        || looks_like_tool_or_refiner_noise(&query)
        || looks_like_lore_or_persona(&query)
        || contains_template_placeholder(&query)
    {
        return None;
    }

    Some(query)
}

/// Run inference against an immutable Brain snapshot, then acquire the shared
/// lock only for the tiny Queen-history commit. A slow or stalled backend must
/// never monopolize every court role or prevent a newer operator turn.
async fn think_with_brain_snapshot(
    brain_cell: &Arc<RwLock<Brain>>,
    role: CourtRole,
    user_input: &str,
    somatic: &SomaticState,
    mode: ForceMode,
    add_history: bool,
    music_enabled: bool,
) -> Result<String, String> {
    let started_epoch = active_turn_epoch();
    let mut snapshot = brain_cell.read().await.clone();
    let reply = snapshot
        .think_as_court(role, user_input, somatic, mode, false, music_enabled)
        .await?;
    if active_turn_epoch() != started_epoch {
        return Err(STALE_TURN_ERROR.to_string());
    }
    if role == CourtRole::Queen && add_history {
        let mut shared = brain_cell.write().await;
        if active_turn_epoch() != started_epoch {
            return Err(STALE_TURN_ERROR.to_string());
        }
        let history_input = if user_input.contains("Continue your monologue") {
            "[Continuing monologue...]"
        } else {
            user_input
        };
        shared.add_to_history("user", history_input);
        shared.add_to_history("model", &reply);
    }
    Ok(reply)
}

async fn run_study_cycle(
    brain_study: Arc<RwLock<Brain>>,
    tx_study: mpsc::Sender<AppEvent>,
    custom_query: Option<String>,
    mission_task_id: Option<String>,
    mission_id: Option<String>,
) {
    let _ = tx_study
        .send(AppEvent::StatusUpdate("Studying".to_string()))
        .await;

    let raw_query = if let Some(q) = custom_query {
        q
    } else {
        // Load current memories to avoid repeating and build upon them
        let mut learned_topics = String::new();
        if let Ok(mut file) = std::fs::File::open(LEARNED_MEMORY_PATH) {
            let mut contents = String::new();
            if file.read_to_string(&mut contents).is_ok() {
                if let Ok(facts) = serde_json::from_str::<Vec<String>>(&contents) {
                    if !facts.is_empty() {
                        learned_topics.push_str(
                            "\nYou currently have the following facts in your memory base:\n",
                        );
                        for fact in facts
                            .iter()
                            .filter_map(|fact| sanitize_fact_memory_candidate(fact))
                            .filter(|fact| looks_source_backed(fact))
                        {
                            learned_topics.push_str(&format!("- {}\n", fact));
                        }
                    }
                }
            }
        }

        let banned = recent_rejected_topics(12);
        let banned_block = if banned.is_empty() {
            String::new()
        } else {
            format!(
                "\nBANNED TOPICS: these recent queries produced nothing usable (dead pages, noise, or facts already known). Do NOT choose them, their subtopics, or close variants:\n{}\n",
                banned
                    .iter()
                    .map(|t| format!("- {}", t))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        let system_instruction = "You are the research topic selector for an autonomous study system. You pick one fresh, concrete web search query or direct URL per cycle. You are NOT a character and must not roleplay, mention any queen, court, or kingdom, or add commentary. Output exactly one search query or URL and nothing else.";
        let prompt = format!(
            "Pick the next topic to investigate online. Allowed domains of curiosity: current news, technical documentation, music/code craft, live-coding concepts, generative art, agent/MCP tooling, science, law, politics, history, psychology, culture.\n\
            Prefer fresh, source-rich, non-Wikipedia searches. If you want a specific source, return a direct URL or a site-scoped query such as 'site:official-domain.example topic'.\n\
            VARIETY RULE: choose a topic clearly DIFFERENT from the known facts below; never re-study something already in memory.\n\
            {}{}\n\
            Return ONLY a single web search query or direct URL. Do not explain your choice.",
            learned_topics, banned_block
        );

        // Clone the lightweight client/config snapshot before awaiting network
        // I/O. Background study must never hold the shared brain lock and starve
        // foreground conversation or specialist turns.
        let brain = brain_study.read().await.clone();
        match brain
            .think_neutral(system_instruction, &prompt, 0.9, 120)
            .await
        {
            Ok(q) => strip_refiner_prefixes(&q.trim().replace("\"", "")),
            Err(_) => "interesting scientific facts".to_string(),
        }
    };
    let query = sanitize_research_query(&raw_query)
        .unwrap_or_else(|| "official Python MCP server examples safe local tools".to_string());

    let query_for_cmd = query.clone();
    let scrape_res = tokio::task::spawn_blocking(move || {
        let python_exe = "D:\\Teledra\\.venv\\Scripts\\python.exe";
        let script_path = "D:\\Teledra\\browser_agent.py";
        let mut cmd = Command::new(python_exe);
        cmd.arg(script_path).arg("--json").arg(&query_for_cmd);
        hide_console(&mut cmd);
        cmd.output()
    })
    .await;

    let browser_bundle = match scrape_res {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            BrowserResearchBundle::from_json(&stdout, &query)
        }
        Ok(Ok(output)) => Err(format!(
            "browser agent exited {}: {}",
            output.status,
            truncate_clean(&String::from_utf8_lossy(&output.stderr), 800)
        )),
        Ok(Err(error)) => Err(format!("failed to launch browser agent: {error}")),
        Err(error) => Err(format!("browser agent task failed: {error}")),
    };

    match browser_bundle {
        Ok(bundle) if !bundle.sources.is_empty() => {
            let evidence_context = bundle.synthesis_context();
            // As above, never hold the shared RwLock across the model request.
            let research_brain = brain_study.read().await.clone();
            let brief = match research_brain
                .synthesize_research_brief(&evidence_context)
                .await
            {
                Ok(raw) => {
                    ResearchBrief::from_model_json(current_unix_timestamp(), bundle.clone(), &raw)
                        .unwrap_or_else(|error| {
                            ResearchBrief::failed(current_unix_timestamp(), bundle.clone(), &error)
                        })
                }
                Err(error) => {
                    ResearchBrief::failed(current_unix_timestamp(), bundle.clone(), &error)
                }
            };

            let brief_value = serde_json::to_value(&brief).unwrap_or_else(|_| {
                serde_json::json!({
                    "schema_version": 1,
                    "timestamp": current_unix_timestamp(),
                    "query": query,
                    "usable": false,
                    "failure": "research brief serialization failed"
                })
            });
            if let Err(error) = append_jsonl_entry(RESEARCH_BRIEFS_PATH, &brief_value) {
                record_recursive_failure(
                    "research_brief_save_failed",
                    &format!("query={} | error={}", query, error),
                );
            }

            if brief.usable {
                let accessed_at_ms = current_unix_timestamp()
                    .parse::<u64>()
                    .unwrap_or(0)
                    .saturating_mul(1_000);
                let mission_evidence = EvidenceBundle {
                    sources: brief
                        .sources
                        .iter()
                        .filter_map(|source| {
                            brief
                                .claims
                                .iter()
                                .find(|claim| claim.source_ids.contains(&source.id))
                                .map(|claim| SourceEvidence {
                                    url: source.url.clone(),
                                    title: source.title.clone(),
                                    claim: truncate_chars(&claim.statement, 800),
                                    accessed_at_ms,
                                })
                        })
                        .collect(),
                    notes: vec![format!(
                        "Grounded research brief: {}",
                        truncate_chars(&brief.status_summary(), 800)
                    )],
                    ..EvidenceBundle::default()
                };
                let saved_fact = brief
                    .best_fact()
                    .and_then(|fact| append_verified_fact(&query, &fact).ok().flatten());
                let theory_lesson = match append_music_theory_lesson(&brief) {
                    Ok(lesson) => lesson,
                    Err(error) => {
                        record_recursive_failure("music_theory_lesson_save_failed", &error.to_string());
                        None
                    }
                };
                let status = brief.status_summary();
                let ledger_detail = if let Some(fact) = saved_fact.as_deref() {
                    format!("query={} | {} | memory_fact={}", query, status, fact)
                } else {
                    format!(
                        "query={} | {} | note=brief preserved; no new compact fact added",
                        query, status
                    )
                };
                let summary = if let Some(lesson) = theory_lesson {
                    format!("Studied {}: {}; saved music lesson: {}", query, status, lesson)
                } else {
                    format!("Studied {}: {}", query, status)
                };
                let _ = append_expansion_ledger("online_research_brief", &ledger_detail);
                let _ = tx_study
                    .send(AppEvent::StudyComplete {
                        summary,
                        usable: true,
                        mission_id: mission_id.clone(),
                        mission_task_id: mission_task_id.clone(),
                        evidence: Some(mission_evidence),
                    })
                    .await;
            } else {
                let failure = brief
                    .failure
                    .as_deref()
                    .unwrap_or("no grounded claims survived validation");
                record_recursive_failure(
                    "research_synthesis_failed",
                    &format!("query={} | error={}", query, failure),
                );
                let _ = append_expansion_ledger(
                    "online_research_synthesis_failed",
                    &format!("query={} | error={}", query, failure),
                );
                let _ = tx_study
                    .send(AppEvent::StudyComplete {
                        summary: format!(
                            "Research sources for {} were preserved, but grounded synthesis failed.",
                            query
                        ),
                        usable: false,
                        mission_id: mission_id.clone(),
                        mission_task_id: mission_task_id.clone(),
                        evidence: None,
                    })
                    .await;
            }
        }
        Ok(bundle) => {
            let brief = ResearchBrief::failed(
                current_unix_timestamp(),
                bundle,
                "search returned no citable source excerpts",
            );
            if let Ok(value) = serde_json::to_value(&brief) {
                let _ = append_jsonl_entry(RESEARCH_BRIEFS_PATH, &value);
            }
            let _ = append_expansion_ledger(
                "online_research_failed",
                &format!("query={} | error=no citable source excerpts", query),
            );
            record_rejected_topic(&query);
            let _ = tx_study
                .send(AppEvent::StudyComplete {
                    summary: format!(
                        "Studied {}, but no citable source excerpts survived validation; moving on.",
                        query
                    ),
                    usable: false,
                    mission_id: mission_id.clone(),
                    mission_task_id: mission_task_id.clone(),
                    evidence: None,
                })
                .await;
        }
        Err(error) => {
            let _ = append_expansion_ledger(
                "online_research_failed",
                &format!("query={} | error={}", query, error),
            );
            record_recursive_failure(
                "research_browser_failed",
                &format!("query={} | error={}", query, error),
            );
            let _ = tx_study
                .send(AppEvent::StudyComplete {
                    summary: format!("Research browser failed for {}: {}", query, error),
                    usable: false,
                    mission_id,
                    mission_task_id,
                    evidence: None,
                })
                .await;
        }
    }

    let _ = tx_study
        .send(AppEvent::StatusUpdate("Ready".to_string()))
        .await;
}

fn strip_refiner_prefixes(s: &str) -> String {
    let mut current = s.trim().to_string();
    let role_prefixes = [
        "[teledra]",
        "[queen]",
        "[organist]",
        "[artist]",
        "[scribe]",
        "[orator]",
        "[archivist]",
        "[alchemist]",
        "teledra:",
        "queen:",
        "organist:",
        "artist:",
        "scribe:",
        "orator:",
        "archivist:",
        "alchemist:",
    ];
    let prefixes = [
        "i shall revise the original draft, maintaining the queen's persona and tone throughout.",
        "i shall revise the original draft, maintaining the queen's persona and tone throughout:",
        "i shall revise the original draft.",
        "i shall revise the original draft:",
        "i shall revise the original draft",
        "based on the original draft",
        "based on the original draft and the critic's critique,",
        "based on the critic's critique,",
        "here is the corrected response:",
        "here is the revised response:",
        "here is the final response:",
        "here is the final corrected response:",
        "here is the final corrected response text:",
        "here is the revised draft response:",
        "here is the revised draft response that meets the queen's persona requirements:",
        "here is the revised draft response that maintains the queen's persona:",
        "here is the revised draft response that meets the queen's persona requirements",
        "here is the revised draft response that maintains the queen's persona",
        "here is a revised draft that attempts to capture the proud, sassy, transactional, and imperial princess persona of teledra:",
        "here is a revised draft that attempts to capture the proud, sassy, transactional, and imperial monarch persona of teledra:",
        "here is a revised draft that attempts to capture teledra's persona:",
        "here is a revised draft that captures teledra's persona:",
        "here is a revised draft that better captures teledra's persona:",
        "here's a revised draft that attempts to capture teledra's persona:",
        "here is a revised draft:",
        "here's a revised draft:",
        "here is the corrected draft:",
        "here is the revised draft:",
        "here is the updated response:",
        "here is the corrected response",
        "here is the revised response",
        "here is the final response",
        "here is the updated response",
        "corrected response:",
        "revised response:",
        "final response:",
        "corrected draft:",
        "revised draft:",
        "corrected response text:",
        "revised response text:",
        "final response text:",
        "final corrected response text:",
    ];

    fn trim_leading_meta_markup(text: String) -> String {
        text.trim()
            .trim_start_matches('#')
            .trim_start_matches('-')
            .trim_start_matches(':')
            .trim()
            .to_string()
    }

    let mut changed = true;
    while changed {
        changed = false;
        let lower = current.to_lowercase();
        for prefix in &role_prefixes {
            if lower.starts_with(prefix) {
                current = current[prefix.len()..].trim().to_string();
                current = trim_leading_meta_markup(current);
                changed = true;
                break;
            }
        }
        if changed {
            continue;
        }
        let lower = current.to_lowercase();
        for prefix in &prefixes {
            if lower.starts_with(prefix) {
                current = current[prefix.len()..].trim().to_string();
                if current.starts_with(':') {
                    current = current[1..].trim().to_string();
                }
                if current.starts_with('"') && current.ends_with('"') && current.len() > 1 {
                    current = current[1..current.len() - 1].trim().to_string();
                }
                current = trim_leading_meta_markup(current);
                changed = true;
                break;
            }
        }
    }

    let lower_markers = [
        "here is the revised draft response",
        "here is the revised draft",
        "here is a revised draft",
        "here's a revised draft",
        "here is the revised response",
        "here is the final corrected response text",
        "here is the final corrected response",
        "here is the corrected draft",
        "here is the corrected response",
        "here is the updated response",
        "i shall revise",
        "based on the original draft",
    ];

    loop {
        let lower = current.to_lowercase();
        if let Some(colon_idx) = current.find(':') {
            if colon_idx < 320 {
                // colon_idx is a byte index into `current`; lowercasing can
                // change byte lengths, so guard the slice into `lower`.
                let leading = lower.get(..colon_idx).unwrap_or("");
                let looks_meta = leading.contains("revised")
                    || leading.contains("corrected")
                    || leading.contains("final response")
                    || leading.contains("critic")
                    || leading.contains("refiner")
                    || leading.contains("writer");
                let names_prompt_machinery = leading.contains("draft")
                    || leading.contains("response")
                    || leading.contains("persona")
                    || leading.contains("requirements")
                    || leading.contains("teledra");
                if looks_meta && names_prompt_machinery {
                    current = current[colon_idx + 1..].trim().to_string();
                    current = trim_leading_meta_markup(current);
                    continue;
                }
            }
        }

        let mut found = None;
        for marker in &lower_markers {
            if let Some(idx) = lower.find(marker) {
                if idx < 350 {
                    found = Some(idx);
                    break;
                }
            }
        }

        if let Some(idx) = found {
            // idx comes from the lowercased copy; use .get() so a byte-length
            // mismatch can never cause a slice panic.
            if let Some(tail) = current.get(idx..) {
                if let Some(colon_offset) = tail.find(':') {
                    current = current[idx + colon_offset + 1..].trim().to_string();
                    current = trim_leading_meta_markup(current);
                    continue;
                }
            }
        }
        break;
    }

    let trailing_noise = [
        "Note: I have revised",
        "Note: The revised",
        "Note that I have revised",
        "I hope this revised response",
        "This revised response",
        "The revised response maintains",
        "Please let me know if this revised draft",
        "Please let me know if this revised response",
        "Critic Critique:",
        "CriticAgent",
        "RefinerAgent",
        "WriterAgent",
        "The SUGGESTION and WORKSHOP_TOOL",
    ];
    for marker in &trailing_noise {
        if let Some(idx) = current.to_lowercase().find(&marker.to_lowercase()) {
            if let Some(head) = current.get(..idx) {
                current = head.trim().to_string();
            }
        }
    }

    trim_leading_meta_markup(current)
}

fn strip_unclosed_tool_and_code_noise(text: &str) -> String {
    let markers = [
        "[PYTHON_MUSIC:",
        "[STRUDEL_MUSIC:",
        "[PYTHON_ART:",
        "[FRACTUS_ART:",
        "[FRACTUS_LIVE:",
        "[DELEGATE:",
        "[SCRIBE_WRITE:",
        "[SCRIBE_APPEND:",
        "[WORKSHOP_TOOL:",
        "[SUGGESTION:",
        "[RESEARCH:",
        "[DIPLOMACY:",
        "[CONDUCT:",
        "Workshop tool:",
        "Innovation sprint:",
        "Innovation sprint produced",
        "No concrete NightDesk action",
        "Smoke test:",
        "Researching:",
        "distilled note looked like lore/tool noise",
        "logged for prompt tuning",
        "```python",
        "```strudel",
        "```rust",
        "```",
        "import numpy",
        "import sounddevice",
        "from teledra_synth",
        "import matplotlib",
        "def ",
        "plt.",
        "np.",
        "D:\\",
        "C:\\",
        "Here is the Python code",
        "Here is a revised draft",
        "Here's a revised draft",
        "Here is the revised draft",
        "Revised Draft:",
        "Critic Critique:",
        "CriticAgent",
        "RefinerAgent",
        "WriterAgent",
        "persona requirements",
    ];

    let mut cut_idx = text.len();
    let lower = text.to_lowercase();
    for marker in &markers {
        if let Some(idx) = lower.find(&marker.to_lowercase()) {
            cut_idx = cut_idx.min(idx);
        }
    }
    // Indices come from the lowercased copy; clamp and snap to a char
    // boundary so slicing the original text can never panic.
    cut_idx = cut_idx.min(text.len());
    while cut_idx > 0 && !text.is_char_boundary(cut_idx) {
        cut_idx -= 1;
    }
    text[..cut_idx].trim().to_string()
}

/// True for CJK / Japanese / Korean codepoints. qwen2.5 occasionally slips into
/// Chinese; we never want those characters reaching TTS or the screen.
fn is_cjk(c: char) -> bool {
    let u = c as u32;
    (0x3000..=0x303F).contains(&u)      // CJK symbols & punctuation
        || (0x3040..=0x30FF).contains(&u) // Hiragana + Katakana
        || (0x3400..=0x4DBF).contains(&u) // CJK Unified Ext A
        || (0x4E00..=0x9FFF).contains(&u) // CJK Unified Ideographs
        || (0xF900..=0xFAFF).contains(&u) // CJK Compatibility Ideographs
        || (0xFF00..=0xFFEF).contains(&u) // Halfwidth/Fullwidth forms
        || (0xAC00..=0xD7AF).contains(&u) // Hangul syllables
}

/// Speech guard: she must never spell a web address aloud, and stray CJK must
/// never reach the synthesizer. Drops URL/email/handle tokens, speaks a lone
/// "@" as "at", and removes any CJK characters.
fn despell_urls_and_cjk_for_speech(text: &str) -> String {
    const TLDS: [&str; 10] = [
        ".com", ".tv", ".gg", ".net", ".io", ".ai", ".co", ".org", ".me", ".dev",
    ];
    let mut kept: Vec<String> = Vec::new();
    for tok in text.split_whitespace() {
        if tok == "@" {
            kept.push("at".to_string());
            continue;
        }
        let lower = tok.to_ascii_lowercase();
        let is_url = lower.contains("://")
            || lower.starts_with("www.")
            || (lower.contains('@') && lower.contains('.')) // email
            || TLDS
                .iter()
                .any(|t| lower.ends_with(t) || lower.contains(&format!("{}/", t)));
        if is_url {
            continue;
        }
        // Drop a leading handle '@' ("@Teledra" -> "Teledra"); strip stray CJK.
        let cleaned: String = tok
            .trim_start_matches('@')
            .chars()
            .filter(|c| !is_cjk(*c))
            .collect();
        let cleaned = cleaned.trim();
        if !cleaned.is_empty() {
            kept.push(cleaned.to_string());
        }
    }
    kept.join(" ")
}

fn clean_text_for_speech(text: &str, role: CourtRole) -> String {
    let mut source = strip_refiner_prefixes(text);
    source = strip_unclosed_tool_and_code_noise(&source);
    source = strip_spoken_speaker_intro(&source, role);
    source = normalize_stage_markup(&source, role);

    let mut cleaned = String::new();
    let mut in_parentheses = 0;
    let mut in_brackets = 0;
    let mut in_fence = false;

    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            in_fence = !in_fence;
            i += 3;
            continue;
        }

        let c = chars[i];
        match c {
            '(' => in_parentheses += 1,
            ')' => {
                if in_parentheses > 0 {
                    in_parentheses -= 1;
                }
            }
            '[' => in_brackets += 1,
            ']' => {
                if in_brackets > 0 {
                    in_brackets -= 1;
                }
            }
            '*' => {
                cleaned.push(' ');
            }
            _ => {
                if in_parentheses == 0 && in_brackets == 0 && !in_fence {
                    cleaned.push(c);
                }
            }
        }
        i += 1;
    }

    let noisy_line_markers = [
        "import ",
        "from teledra_synth",
        "def ",
        "plt.",
        "np.",
        "play_sound(",
        "scribe_",
        "python_music",
        "strudel_music",
        "fractus_art",
        "delegate:",
        "workshop_tool",
        "suggestion:",
        "research:",
        "diplomacy:",
        "status:",
        "progress:",
        "system:",
        "critic critique",
        "criticagent",
        "refineragent",
        "writeragent",
        "revised draft",
        "final corrected response",
        "persona requirements",
        "imperial princess persona",
        "memory classification",
        "classification law",
        "lore_archive",
        "fact_archive",
        "[lore",
        "[fact",
        "append to",
        "write to",
        "[system]",
        "inserted organist",
        "launching local",
        "fractus launched",
        "python music editor",
        "strudel sketchpad",
        "d:\\",
        "c:\\",
    ];

    cleaned = cleaned
        .lines()
        .map(|line| strip_spoken_speaker_intro(line, role))
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return false;
            }
            let lower = trimmed.to_lowercase();
            if lower.starts_with("system ")
                || lower.starts_with("system:")
                || lower.starts_with("[system]")
                || lower.starts_with("status:")
                || lower.starts_with("progress:")
            {
                return false;
            }
            !noisy_line_markers
                .iter()
                .any(|marker| lower.contains(marker))
        })
        .collect::<Vec<_>>()
        .join(" ");

    cleaned = despell_urls_and_cjk_for_speech(&cleaned);
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }
    strip_spoken_speaker_intro(cleaned.trim(), role)
}

fn limit_spoken_text(text: &str, max_sentences: usize, max_chars: usize) -> String {
    let mut out = String::new();
    let mut sentences = 0usize;
    let mut last_sentence_end = 0usize;
    let mut last_soft_break = 0usize;
    let mut last_space = 0usize;

    for c in text.chars() {
        out.push(c);
        if matches!(c, '.' | '!' | '?') {
            sentences += 1;
            last_sentence_end = out.len();
            if sentences >= max_sentences {
                break;
            }
        }
        if matches!(c, ',' | ';' | ':') {
            last_soft_break = out.len();
        }
        if c.is_whitespace() {
            last_space = out.len();
        }
        if out.len() >= max_chars {
            let cut = if last_sentence_end > 80 {
                last_sentence_end
            } else if last_soft_break > 80 {
                last_soft_break
            } else if last_space > 80 {
                last_space
            } else {
                out.len()
            };
            out.truncate(cut);
            break;
        }
    }

    let mut out = out.trim().to_string();
    while out.ends_with(',')
        || out.ends_with(';')
        || out.ends_with(':')
        || out.ends_with('-')
        || out.ends_with('(')
    {
        out.pop();
        out = out.trim().to_string();
    }
    if !out.is_empty() && !out.ends_with('.') && !out.ends_with('!') && !out.ends_with('?') {
        out.push('.');
    }
    out
}

fn split_spoken_text_parts(text: &str, max_chars: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut remaining = text.trim().to_string();

    while remaining.len() > max_chars {
        let mut last_sentence_end = 0usize;
        let mut last_soft_break = 0usize;
        let mut last_space = 0usize;

        for (idx, c) in remaining.char_indices() {
            let end = idx + c.len_utf8();
            if end > max_chars {
                break;
            }
            if matches!(c, '.' | '!' | '?') {
                last_sentence_end = end;
            } else if matches!(c, ',' | ';' | ':') {
                last_soft_break = end;
            } else if c.is_whitespace() {
                last_space = end;
            }
        }

        let min_reasonable_cut = max_chars / 3;
        let cut = if last_sentence_end > min_reasonable_cut {
            last_sentence_end
        } else if last_soft_break > min_reasonable_cut {
            last_soft_break
        } else if last_space > min_reasonable_cut {
            last_space
        } else {
            remaining
                .char_indices()
                .take_while(|(idx, _)| *idx <= max_chars)
                .last()
                .map(|(idx, c)| idx + c.len_utf8())
                .unwrap_or_else(|| remaining.len().min(max_chars))
        };

        let part = remaining[..cut].trim();
        if !part.is_empty() {
            parts.push(part.to_string());
        }
        remaining = remaining[cut..].trim().to_string();
    }

    if !remaining.is_empty() {
        parts.push(remaining);
    }

    parts
}

// --- Game Co-Pilot mode -------------------------------------------------------

/// Detects the game (or app) in the foreground window. Returns a cleaned name,
/// or None when the foreground is a known non-game (browser, editor, the TUI
/// itself) so the co-pilot never announces "you're playing Firefox".
fn detect_foreground_game() -> Option<String> {
    let script = r#"
$sig = '[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();'
$t = Add-Type -MemberDefinition $sig -Name Wfg -Namespace Ufg -PassThru
$h = $t::GetForegroundWindow()
$p = Get-Process | Where-Object { $_.MainWindowHandle -eq $h } | Select-Object -First 1
if ($p) { Write-Output ($p.MainWindowTitle + '|' + $p.ProcessName) }
"#;
    let mut cmd = Command::new("powershell");
    cmd.arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut cmd);
    let out = cmd.output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let (title, proc) = line.split_once('|')?;
    let proc_l = proc.trim().to_ascii_lowercase();
    let ignore = [
        "teledra",
        "firefox",
        "chrome",
        "msedge",
        "brave",
        "opera",
        "discord",
        "obs64",
        "obs",
        "explorer",
        "code",
        "cursor",
        "devenv",
        "windowsterminal",
        "powershell",
        "cmd",
        "python",
        "pythonw",
        "javaw",
        "java",
        "spotify",
        "notepad",
        "searchhost",
        "shellexperiencehost",
        "textinputhost",
    ];
    if ignore.iter().any(|i| proc_l == *i || proc_l.starts_with(i)) {
        return None;
    }
    let title = title.trim();
    let cleaned = title.split(" - ").next().unwrap_or(title).trim();
    if !cleaned.is_empty() {
        Some(cleaned.to_string())
    } else if !proc.trim().is_empty() {
        Some(proc.trim().to_string())
    } else {
        None
    }
}

// --- MCP embassies (opt-in agent tool servers) -------------------------------

/// True when the operator has enabled at least one MCP server. Cheap file read,
/// so it can gate the backstage prompt without spawning anything.
fn mcp_is_live() -> bool {
    if let Ok(txt) = std::fs::read_to_string("D:\\Teledra\\config\\mcp_servers.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
            if let Some(servers) = v.get("servers").and_then(|s| s.as_array()) {
                return servers.iter().any(|s| {
                    s.get("enabled").and_then(|b| b.as_bool()).unwrap_or(false)
                        && s.get("command")
                            .and_then(|c| c.as_str())
                            .map(|c| !c.is_empty())
                            .unwrap_or(false)
                });
            }
        }
    }
    false
}

fn run_mcp_bridge(sub: &str, stdin_json: Option<&str>) -> Result<serde_json::Value, String> {
    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("D:\\Teledra\\mcp_bridge.py")
        .arg(sub)
        .current_dir("D:\\Teledra")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if stdin_json.is_some() {
        cmd.stdin(Stdio::piped());
    }
    hide_console(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn mcp bridge: {}", e))?;
    if let Some(js) = stdin_json {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(js.as_bytes());
        }
    }
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("collect mcp output: {}", e))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let last = stdout.lines().last().unwrap_or("").trim();
                return serde_json::from_str::<serde_json::Value>(last)
                    .map_err(|e| format!("parse mcp result: {} (got: {})", e, last));
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(45) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("mcp bridge timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(format!("mcp bridge failed: {}", e)),
        }
    }
}

/// Lists the tools across all enabled MCP servers (for the /mcp command).
fn mcp_tools_summary() -> String {
    match run_mcp_bridge("list", None) {
        Ok(v) => {
            if !v
                .get("any_enabled")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
            {
                return "No MCP embassies enabled. Add one in config/mcp_servers.json (set enabled=true) -- candidates: filesystem, fetch, memory. The court then uses them via [MCP_CALL:].".to_string();
            }
            let mut lines = vec!["Connected MCP embassies:".to_string()];
            if let Some(servers) = v.get("servers").and_then(|s| s.as_array()) {
                for s in servers {
                    let name = s.get("server").and_then(|n| n.as_str()).unwrap_or("mcp");
                    let err = s.get("error").and_then(|e| e.as_str()).unwrap_or("");
                    if !err.is_empty() {
                        lines.push(format!("- {} (error: {})", name, truncate_chars(err, 90)));
                        continue;
                    }
                    let tools: Vec<String> = s
                        .get("tools")
                        .and_then(|t| t.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|t| {
                                    t.get("name")
                                        .and_then(|n| n.as_str())
                                        .map(|x| x.to_string())
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    lines.push(format!(
                        "- {}: {}",
                        name,
                        if tools.is_empty() {
                            "(no tools)".to_string()
                        } else {
                            tools.join(", ")
                        }
                    ));
                }
            }
            lines.join("\n")
        }
        Err(e) => format!("MCP bridge error: {}", e),
    }
}

/// Calls one tool on an approved MCP server. Returns the text result on success.
fn mcp_call(server: &str, tool: &str, args_json: &str) -> Option<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).unwrap_or_else(|_| serde_json::json!({}));
    let job = serde_json::json!({ "server": server, "tool": tool, "arguments": args }).to_string();
    match run_mcp_bridge("call", Some(&job)) {
        Ok(v) if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) => v
            .get("text")
            .and_then(|t| t.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or(Some("ok".to_string())),
        Ok(v) => {
            record_recursive_failure("mcp_call_failed", &truncate_chars(&v.to_string(), 300));
            None
        }
        Err(e) => {
            record_recursive_failure("mcp_call_error", &e);
            None
        }
    }
}

/// Runs the deterministic Treasury income scout (writes structured leads to
/// knowledge/treasury_ledger.md itself) and returns its one-line headline.
fn run_treasury_scout() -> Option<String> {
    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("D:\\Teledra\\treasury_scout.py")
        .current_dir("D:\\Teledra")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().ok()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let last = stdout.lines().last().unwrap_or("").trim();
                let v = serde_json::from_str::<serde_json::Value>(last).ok()?;
                return v
                    .get("headline")
                    .and_then(|h| h.as_str())
                    .map(|s| s.to_string());
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(90) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(_) => return None,
        }
    }
}

/// Summarizes the kingdom's variety/growth: distinct fractal recipes & families,
/// distinct music compositions, and the current tune's size trend.
fn build_growth_report() -> String {
    use std::collections::HashSet;
    let mut out = vec!["Kingdom growth evidence (variety = real growth):".to_string()];

    let fr = read_text_tail("knowledge/fractus_experiments.jsonl", 12000).unwrap_or_default();
    let mut fr_total = 0usize;
    let mut fr_hashes: HashSet<String> = HashSet::new();
    let mut fr_types: HashSet<String> = HashSet::new();
    for line in fr.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            fr_total += 1;
            if let Some(h) = v.get("hash").and_then(|s| s.as_str()) {
                fr_hashes.insert(h.to_string());
            }
            if let Some(spec) = v.get("spec").and_then(|s| s.as_str()) {
                if let Some(idx) = spec.find("--type ") {
                    if let Some(t) = spec[idx + 7..].split_whitespace().next() {
                        fr_types.insert(t.to_string());
                    }
                }
            }
        }
    }
    let mut types_sorted: Vec<String> = fr_types.into_iter().collect();
    types_sorted.sort();
    out.push(format!(
        "- Fractals/geometry: {} launches, {} distinct recipes, {} families ({}).",
        fr_total,
        fr_hashes.len(),
        types_sorted.len(),
        if types_sorted.is_empty() {
            "none yet".to_string()
        } else {
            types_sorted.join(", ")
        }
    ));

    let mu = read_text_tail("knowledge/music_experiments.jsonl", 12000).unwrap_or_default();
    let mut mu_total = 0usize;
    let mut mu_hashes: HashSet<String> = HashSet::new();
    let mut chars_series: Vec<u64> = Vec::new();
    for line in mu.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            mu_total += 1;
            if let Some(h) = v.get("hash").and_then(|s| s.as_str()) {
                mu_hashes.insert(h.to_string());
            }
            if let Some(c) = v.get("chars").and_then(|c| c.as_u64()) {
                chars_series.push(c);
            }
        }
    }
    let trend = if chars_series.len() >= 2 {
        let first = chars_series[chars_series.len().saturating_sub(8)];
        let last = *chars_series.last().unwrap();
        if last as f64 > first as f64 * 1.1 {
            "growing"
        } else if (last as f64) < first as f64 * 0.9 {
            "tightening"
        } else {
            "steady"
        }
    } else {
        "new"
    };
    let cur = std::fs::read_to_string("D:\\Teledra\\music.py").unwrap_or_default();
    out.push(format!(
        "- Music: {} experiments, {} distinct compositions. Current tune: {} chars; recent size trend: {}.",
        mu_total,
        mu_hashes.len(),
        cur.len(),
        trend
    ));

    out.push(
        "- View income: /treasury  |  refresh leads: /scout  |  deepen the tune now: Ctrl+U."
            .to_string(),
    );
    out.join("\n")
}

/// Grabs the screen and returns moondream's short description (None on failure).
fn run_copilot_vision() -> Option<String> {
    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("D:\\Teledra\\copilot_vision.py")
        .current_dir("D:\\Teledra")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().ok()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let last = stdout.lines().last().unwrap_or("").trim();
                let v = serde_json::from_str::<serde_json::Value>(last).ok()?;
                if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                    return v
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string());
                }
                return None;
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(125) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(_) => return None,
        }
    }
}

/// Idle co-pilot line: ~60% game facts/lore, ~20% silly, ~20% her own thoughts.
fn copilot_idle_prompt(game: Option<&str>, turn: u64, screen_note: Option<&str>) -> String {
    let game_ctx = match game {
        Some(g) => format!("The human is streaming the game '{}'. ", g),
        None => {
            "The human is streaming a game (not yet identified; keep it general or infer gently). "
                .to_string()
        }
    };
    let screen_ctx = match screen_note {
        Some(s) if !s.is_empty() => format!("Right now on screen: {}. ", s),
        _ => String::new(),
    };
    let content = match turn % 5 {
        0 | 1 | 2 => {
            "share ONE genuinely interesting fact or piece of lore about this game -- its world, story, characters, mechanics, history, trivia, or studio. Make it a fun aside, not a wiki entry"
        }
        3 => {
            "be playful and silly for a beat: a light joke, a teasing remark about the gameplay, or an absurd what-if, kept warm"
        }
        _ => {
            "share a quick genuine thought or reaction of your own about what's unfolding -- what you find interesting, lovely, frustrating, or curious"
        }
    };
    format!(
        "GAME CO-PILOT. {}{}In 1-3 short spoken sentences, {}. Sound like a clever friend on the couch, not a lecturer. No stage directions, no tags, do not narrate yourself.",
        game_ctx, screen_ctx, content
    )
}

/// Co-pilot reply to a chat viewer (or, when from_streamer, to the host's mic).
fn copilot_chat_prompt(
    game: Option<&str>,
    author: &str,
    text: &str,
    from_streamer: bool,
) -> String {
    let game_ctx = match game {
        Some(g) => format!(" (currently playing '{}')", g),
        None => String::new(),
    };
    if from_streamer {
        format!(
            "GAME CO-PILOT{}. The streamer you are co-piloting just said aloud: \"{}\". Respond to them directly and naturally in 1-2 warm, playful spoken sentences, as Teledra. If it's about the game, weave in a relevant fact or reaction. No tags, no stage directions.",
            game_ctx, text
        )
    } else {
        format!(
            "GAME CO-PILOT{}. A viewer named {} said in chat: \"{}\". Answer them directly in 1-2 warm, playful spoken sentences, as Teledra. If it's about the game, weave in a relevant fact or reaction. No tags, no stage directions.",
            game_ctx, author, text
        )
    }
}

fn voice_name_for_role<'a>(role: CourtRole, queen_voice: &'a str) -> &'a str {
    match role {
        CourtRole::Queen => queen_voice,
        CourtRole::Organist => "organist",
        CourtRole::Archivist => "archivist",
        CourtRole::Alchemist => "alchemist",
        CourtRole::Orator => "orator",
        CourtRole::Scribe => "scribe",
        CourtRole::Artist => "artist",
        CourtRole::Diplomat => "diplomat",
        CourtRole::Treasurer => "treasurer",
        CourtRole::Wizard => "wizard",
    }
}

fn speech_limits_for_role(role: CourtRole, mode: ForceMode) -> (usize, usize) {
    match role {
        CourtRole::Queen if mode == ForceMode::Babble || mode == ForceMode::Streamer => (32, 16000),
        CourtRole::Queen => (36, 7000),
        CourtRole::Organist | CourtRole::Artist => (18, 7000),
        CourtRole::Diplomat => (16, 7000),
        CourtRole::Wizard => (5, 1200),
        CourtRole::Scribe => (4, 900),
        _ => (10, 3800),
    }
}

fn spawn_spoken_reply(
    role: CourtRole,
    text: String,
    mode: ForceMode,
    queen_voice: String,
    active_playback: Arc<std::sync::Mutex<Option<voice::PlaybackController>>>,
    tx: mpsc::Sender<AppEvent>,
    send_speech_complete: bool,
) {
    let active_voice = voice_name_for_role(role, &queen_voice).to_string();
    let cleaned_speech = clean_text_for_speech(&text, role);
    let (speech_sentence_limit, speech_char_limit) = speech_limits_for_role(role, mode);
    let reply_for_speech =
        limit_spoken_text(&cleaned_speech, speech_sentence_limit, speech_char_limit);
    // `generate_voice.py` already splits text into phrase-sized synthesis
    // chunks after loading LuxTTS once. Sending 900-character subprocesses
    // reloaded the model and re-encoded the reference for every part (up to 18
    // cold starts for a long Queen turn). Keep the full bounded reply in one
    // worker invocation; 20k remains comfortably below Windows' command-line
    // limit and the role caps above.
    let speech_parts = split_spoken_text_parts(&reply_for_speech, 20_000);

    tokio::task::spawn_blocking(move || {
        let speech_parts: Vec<String> = speech_parts
            .into_iter()
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();
        if speech_parts.is_empty() {
            let _ = tx.blocking_send(AppEvent::StatusUpdate("Ready".to_string()));
            if send_speech_complete {
                let _ = tx.blocking_send(AppEvent::SpeechComplete);
            }
            return;
        }

        let engine = VoiceEngine::new(&active_voice);
        let total_parts = speech_parts.len();

        for (part_idx, speech_text) in speech_parts.iter().enumerate() {
            if total_parts > 1 {
                let _ = tx.blocking_send(AppEvent::StatusUpdate(format!(
                    "Speaking part {} of {}",
                    part_idx + 1,
                    total_parts
                )));
            }

            let tx_inner = tx.clone();
            let progress_callback = move |status: String| {
                let _ = tx_inner.blocking_send(AppEvent::StatusUpdate(status));
            };

            match engine.generate_and_play(
                speech_text,
                Arc::clone(&active_playback),
                progress_callback,
            ) {
                Ok(()) => {}
                Err(e) => {
                    if e != "Cancelled" {
                        let _ = tx.blocking_send(AppEvent::Error(format!("Vocal crash: {}", e)));
                    }
                    // Cancellation and premature EOF are terminal too. Without
                    // this event, one killed TTS child strands every queued
                    // minister behind speech that no longer exists.
                    let _ = tx.blocking_send(AppEvent::StatusUpdate("Ready".to_string()));
                    if send_speech_complete {
                        let _ = tx.blocking_send(AppEvent::SpeechComplete);
                    }
                    return;
                }
            }
        }

        let _ = tx.blocking_send(AppEvent::StatusUpdate("Ready".to_string()));
        if send_speech_complete {
            let _ = tx.blocking_send(AppEvent::SpeechComplete);
        }
    });
}

fn strip_fenced_code_block(code: &str, language: &str) -> String {
    let fence = format!("```{}", language);
    if let Some(start_idx) = code.find(&fence) {
        let content_start = start_idx + fence.len();
        if let Some(end_idx) = code[content_start..].find("```") {
            return code[content_start..content_start + end_idx]
                .trim()
                .to_string();
        }
    }
    if let Some(start_idx) = code.find("```") {
        let content_start = start_idx + 3;
        if let Some(end_idx) = code[content_start..].find("```") {
            return code[content_start..content_start + end_idx]
                .trim()
                .to_string();
        }
    }
    code.trim().to_string()
}

fn normalize_strudel_music_code(code: &str) -> String {
    let cleaned = strip_fenced_code_block(code, "strudel");
    let lower = cleaned.to_ascii_lowercase();
    let Some(start) = lower.find("stack(") else {
        return cleaned.trim().to_string();
    };

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in cleaned[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                let end = start + offset + ch.len_utf8();
                return cleaned[start..end].trim().to_string();
            }
        }
    }

    cleaned.trim().to_string()
}

fn deterministic_strudel_music(seed: usize) -> String {
    let patterns = [
        "stack(\n\
s(\"<bd ~ sd ~> bd [~ bd] sd ~\").gain(0.46).pan(0).lpf(9000).room(0.08),\n\
s(\"<~ hh*4 ~ oh> hh*2 [hh hh] ~ cp\").gain(0.18).pan(0.34).lpf(7200).delay(0.12).delaytime(0.18).delayfeedback(0.28),\n\
note(\"<d2 ~ a1 d2> [d2 ~] <f2 g2> a1\").s(\"triangle\").gain(0.26).pan(-0.08).lpf(780).attack(0.01).release(0.16).slow(2),\n\
note(\"<d3,f3,a3 bb2,d3,f3 c3,e3,g3 a2,c3,e3>\").s(\"triangle\").gain(0.16).pan(0.12).lpf(1500).room(0.34).attack(0.28).release(0.9).slow(4),\n\
note(\"<a4 ~ f4 [g4 a4]> <d5 c5> ~ <f4 e4>\").s(\"sawtooth\").gain(0.13).pan(-0.38).lpf(2400).delay(0.16).delaytime(0.26).delayfeedback(0.32).slow(2),\n\
note(\"<~ d5 f5 a5> [c6 a5] ~ <g5 e5> d5\").s(\"sine\").gain(0.11).pan(0.42).lpf(4200).room(0.46).attack(0.04).release(0.42).slow(2),\n\
note(\"<d6 ~ ~ a5> ~ <f6 ~ e6 ~> ~\").s(\"sine\").gain(0.07).pan(0.58).lpf(6000).room(0.62).delay(0.22).delaytime(0.38).delayfeedback(0.36).slow(4)\n\
)",
        "stack(\n\
s(\"<bd ~ ~ sd> bd [bd ~] sd ~\").gain(0.44).pan(0).lpf(8600).room(0.06),\n\
s(\"<hh*2 hh*4 ~ oh> [~ hh] hh*2 ~ cp\").gain(0.17).pan(-0.32).lpf(7600).delay(0.1).delaytime(0.16).delayfeedback(0.24),\n\
note(\"<a1 ~ e2 a1> [g2 ~] <d2 e2> a1\").s(\"sawtooth\").gain(0.24).pan(-0.06).lpf(720).attack(0.008).release(0.14).slow(2),\n\
note(\"<a3,c4,e4 g3,b3,d4 d3,f3,a3 e3,g3,b3>\").s(\"triangle\").gain(0.15).pan(0.15).lpf(1700).room(0.38).attack(0.32).release(1.0).slow(4),\n\
note(\"<e4 ~ g4 [a4 b4]> <d5 b4> ~ <a4 g4>\").s(\"square\").gain(0.1).pan(0.38).lpf(2100).delay(0.14).delaytime(0.22).delayfeedback(0.3).slow(2),\n\
note(\"<~ a4 c5 e5> [g5 e5] ~ <d5 b4> a4\").s(\"sine\").gain(0.12).pan(-0.44).lpf(4600).room(0.42).attack(0.03).release(0.36).slow(2),\n\
note(\"<a5 ~ ~ e6> ~ <g5 ~ b5 ~> ~\").s(\"sine\").gain(0.065).pan(0.62).lpf(6400).room(0.66).delay(0.2).delaytime(0.34).delayfeedback(0.34).slow(4)\n\
)",
        "stack(\n\
s(\"<bd ~ sd ~> [bd ~] ~ sd bd\").gain(0.45).pan(0).lpf(8800).room(0.07),\n\
s(\"<~ hh*3 oh ~> hh*4 [~ hh] cp ~\").gain(0.16).pan(0.36).lpf(7000).delay(0.13).delaytime(0.19).delayfeedback(0.26),\n\
note(\"<e2 ~ f2 e2> [g2 ~] <a2 f2> e2\").s(\"square\").gain(0.21).pan(-0.08).lpf(690).attack(0.006).release(0.13).slow(2),\n\
note(\"<e3,g3,b3 f3,a3,c4 g3,b3,d4 a3,c4,e4>\").s(\"triangle\").gain(0.145).pan(-0.12).lpf(1450).room(0.4).attack(0.36).release(1.1).slow(4),\n\
note(\"<b4 ~ c5 [d5 e5]> <f5 e5> ~ <d5 c5>\").s(\"sawtooth\").gain(0.11).pan(-0.4).lpf(2300).delay(0.17).delaytime(0.28).delayfeedback(0.33).slow(2),\n\
note(\"<~ e5 g5 b5> [a5 g5] ~ <f5 d5> e5\").s(\"sine\").gain(0.105).pan(0.46).lpf(4300).room(0.48).attack(0.05).release(0.48).slow(2),\n\
note(\"<e6 ~ ~ b5> ~ <c6 ~ d6 ~> ~\").s(\"sine\").gain(0.06).pan(0.64).lpf(6200).room(0.7).delay(0.24).delaytime(0.4).delayfeedback(0.38).slow(4)\n\
)",
        "stack(\n\
s(\"<bd ~ sd ~> bd ~ [bd sd] ~\").gain(0.47).pan(0).lpf(9200).room(0.08),\n\
s(\"<hh*4 ~ oh ~> [hh hh] ~ cp hh*2\").gain(0.18).pan(-0.35).lpf(7400).delay(0.11).delaytime(0.17).delayfeedback(0.27),\n\
note(\"<g1 ~ d2 g1> [bb1 ~] <f2 d2> g1\").s(\"triangle\").gain(0.27).pan(-0.05).lpf(760).attack(0.009).release(0.15).slow(2),\n\
note(\"<g3,bb3,d4 eb3,g3,bb3 f3,a3,c4 d3,f3,a3>\").s(\"sawtooth\").gain(0.14).pan(0.14).lpf(1600).room(0.36).attack(0.3).release(0.95).slow(4),\n\
note(\"<d4 ~ f4 [g4 bb4]> <c5 bb4> ~ <a4 f4>\").s(\"square\").gain(0.1).pan(0.4).lpf(2200).delay(0.15).delaytime(0.24).delayfeedback(0.31).slow(2),\n\
note(\"<~ g4 bb4 d5> [f5 d5] ~ <c5 a4> g4\").s(\"sine\").gain(0.115).pan(-0.45).lpf(4500).room(0.44).attack(0.035).release(0.4).slow(2),\n\
note(\"<g5 ~ ~ d6> ~ <bb5 ~ a5 ~> ~\").s(\"sine\").gain(0.068).pan(0.6).lpf(6100).room(0.64).delay(0.21).delaytime(0.36).delayfeedback(0.35).slow(4)\n\
)",
    ];
    patterns[seed % patterns.len()].to_string()
}

fn default_strudel_music_code() -> String {
    let seed = current_unix_timestamp().parse::<usize>().unwrap_or(0);
    deterministic_strudel_music(seed)
}

fn default_fractus_art_spec() -> String {
    let patterns = [
        "--type lotus_mandala --iterations 260 --palette twilight",
        "--type flower_of_life --iterations 240 --palette pastel",
        "--type phyllotaxis --iterations 280 --palette solar_gold",
        "--type harmonograph --iterations 260 --palette electric_cyan",
        "--type truchet --iterations 220 --palette amethyst",
        "--type fractal_tree --iterations 250 --palette emerald",
        "--type reaction_diffusion --iterations 210 --palette ice_fire",
        "--type strange_attractor --iterations 260 --palette rainbow",
        "--type particles --iterations 220 --palette emerald",
    ];
    let seed = current_unix_timestamp().parse::<usize>().unwrap_or(0);
    patterns[seed % patterns.len()].to_string()
}

// --- Deterministic creative variety + repair helpers -------------------------
//
// When the backstage layer fails to emit a parseable hidden tag, the old path
// forced a generic research query, which produced churn and no artifacts. These
// helpers let the runtime instead install ONE known-good artifact (a workshop
// tool, a validated composition, or a fresh Fractus recipe) so the workshop and
// creative layers keep producing and the failure streak actually breaks.

/// Tiny xorshift PRNG seeded from the clock. Avoids pulling rng crate state
/// across await points; entropy is plenty for art/music parameter variety.
fn variety_seed() -> u64 {
    let secs = current_unix_timestamp().parse::<u64>().unwrap_or(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    secs.wrapping_mul(2654435761).wrapping_add(nanos) | 1
}

fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn pick<'a, T>(state: &mut u64, items: &'a [T]) -> &'a T {
    &items[(xorshift(state) as usize) % items.len()]
}

/// A randomized but always-valid Fractus argument line.
fn random_fractus_spec(state: &mut u64) -> String {
    let types = [
        "barnsley_fern",
        "mandelbrot",
        "multibrot",
        "julia",
        "burning_ship",
        "tricorn",
        "newton",
        "mandala",
        "lotus_mandala",
        "star_mandala",
        "flower_of_life",
        "radial_weave",
        "kaleidoscope",
        "phyllotaxis",
        "woven_web",
        "guilloche",
        "lissajous",
        "particles",
        "moire",
        "orbital_lace",
        "spirograph",
        "harmonograph",
        "rose_curve",
        "string_art",
        "sierpinski",
        "koch_snowflake",
        "dragon_curve",
        "fractal_tree",
        "l_system",
        "truchet",
        "hex_weave",
        "op_art",
        "cellular_automata",
        "reaction_diffusion",
        "flow_field",
        "strange_attractor",
    ];
    let palettes = [
        "amethyst",
        "electric_cyan",
        "emerald",
        "ice_fire",
        "monochrome",
        "neon_sunset",
        "pastel",
        "purple_haze",
        "rainbow",
        "solar_gold",
        "twilight",
    ];
    let t = *pick(state, &types);
    let pal = *pick(state, &palettes);
    let iterations = 160 + (xorshift(state) as usize % 161); // 160..=320
    let seed = xorshift(state);
    let mut spec = format!(
        "--type {} --iterations {} --palette {} --seed {}",
        t, iterations, pal, seed
    );
    if t == "julia" {
        let cr = -1.2 + (xorshift(state) as f64 / u64::MAX as f64) * 2.4;
        let ci = -1.2 + (xorshift(state) as f64 / u64::MAX as f64) * 2.4;
        spec.push_str(&format!(" --c-real {:.3} --c-imag {:.3}", cr, ci));
    }
    spec
}

fn recent_fractus_specs(limit: usize) -> Vec<String> {
    let contents =
        read_text_tail("knowledge/fractus_experiments.jsonl", 128_000).unwrap_or_default();
    contents
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|v| {
            v.get("spec")
                .and_then(|s| s.as_str())
                .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        })
        .take(limit)
        .collect()
}

/// If `spec` repeats one of the most recently launched recipes, nudge it into a
/// fresh variation so the Artist visibly stops recycling the same orbital_lace.
fn diversify_fractus_spec(spec: &str) -> String {
    let recent = recent_fractus_specs(64);
    let normalized = spec.split_whitespace().collect::<Vec<_>>().join(" ");
    let repeats = |candidate: &str| {
        let norm = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
        recent.iter().any(|r| r.eq_ignore_ascii_case(&norm))
    };
    if !repeats(&normalized) {
        return spec.to_string();
    }
    let mut state = variety_seed();
    for _ in 0..8 {
        let candidate = random_fractus_spec(&mut state);
        if !repeats(&candidate) {
            return candidate;
        }
    }
    random_fractus_spec(&mut state)
}

/// A complete, always-valid teledra_synth composition, varied by seed. Used as
/// the fallback when a model-written Python composition fails validation.
fn deterministic_python_music(seed: usize) -> String {
    // A broad pool of keys/modes so the fallback stops cycling the same 3 tunes.
    let progressions = [
        // A natural minor
        (
            r#"[["A3","C4","E4"],["F3","A3","C4"],["C4","E4","G4"],["G3","B3","D4"]]"#,
            r#"["A1","F1","C2","G1"]"#,
            r#"["E5","C5","D5","B4","A4","C5","E5","G5"]"#,
        ),
        // D minor
        (
            r#"[["D4","F4","A4"],["A3","C4","E4"],["B3","D4","F4"],["G3","B3","D4"]]"#,
            r#"["D2","A1","B1","G1"]"#,
            r#"["A5","F5","E5","D5","A4","D5","F5","A5"]"#,
        ),
        // E phrygian
        (
            r#"[["E4","G4","B4"],["F4","A4","C5"],["G4","B4","D5"],["A3","C4","E4"]]"#,
            r#"["E2","F1","G1","A1"]"#,
            r#"["E5","F5","G5","B5","A5","G5","F5","E5"]"#,
        ),
        // C major
        (
            r#"[["C4","E4","G4"],["G3","B3","D4"],["A3","C4","E4"],["F3","A3","C4"]]"#,
            r#"["C2","G1","A1","F1"]"#,
            r#"["G4","C5","E5","G5","F5","E5","C5","G4"]"#,
        ),
        // D dorian
        (
            r#"[["D4","F4","A4"],["G3","B3","D4"],["C4","E4","G4"],["A3","C4","E4"]]"#,
            r#"["D2","G1","C2","A1"]"#,
            r#"["A4","D5","F5","A5","B5","A5","F5","D5"]"#,
        ),
        // G mixolydian
        (
            r#"[["G3","B3","D4"],["F4","A4","C5"],["C4","E4","G4"],["D4","F4","A4"]]"#,
            r#"["G1","F1","C2","D2"]"#,
            r#"["D5","G5","B5","D6","C6","B5","G5","D5"]"#,
        ),
        // B minor
        (
            r#"[["B3","D4","F#4"],["G3","B3","D4"],["A3","C#4","E4"],["F#3","A3","C#4"]]"#,
            r#"["B1","G1","A1","F#1"]"#,
            r#"["F#5","B4","D5","F#5","E5","D5","B4","F#5"]"#,
        ),
        // A harmonic minor
        (
            r#"[["A3","C4","E4"],["D4","F4","A4"],["E4","G#4","B4"],["A3","C4","E4"]]"#,
            r#"["A1","D2","E2","A1"]"#,
            r#"["E5","A5","C6","B5","G#5","B5","A5","E5"]"#,
        ),
    ];
    // Vary several axes independently so combinations multiply.
    let leadwave = ["sine", "triangle", "sawtooth"][seed % 3];
    let beat = ["0.5", "0.45", "0.55", "0.6", "0.42"][(seed / 3) % 5];
    let cutoff = ["3200", "2600", "3800", "2900", "3500"][(seed / 7) % 5];
    let (chords, bass, motif) = progressions[seed % progressions.len()];
    let theory_profiles = [
        ("A", "natural_minor", "[1, 6, 3, 7]"),
        ("D", "dorian", "[1, 5, 6, 4]"),
        ("E", "phrygian", "[1, 2, 3, 4]"),
        ("C", "major", "[1, 5, 6, 4]"),
        ("D", "dorian", "[1, 4, 7, 5]"),
        ("G", "mixolydian", "[1, 7, 4, 5]"),
        ("B", "natural_minor", "[1, 6, 7, 5]"),
        ("A", "harmonic_minor", "[1, 4, 5, 1]"),
    ];
    let (tonal_center, mode, degrees) = theory_profiles[seed % theory_profiles.len()];

    let template = r#"import numpy as np
from teledra_synth import (
    apply_automation, automation_curve, delay, lowpass_filter, make_seamless_loop,
    mix_waves, play_sound, reverb, soft_limiter, stereo_pan, stereo_width, synth_note,
)

SR = 44100
SEED = __SEED__
np.random.seed(SEED)
TITLE = "Fivefold Court Engine"
STYLE = "retro adventure court score"
BEAT = __BEAT__
BPM = 60.0 / BEAT
BEATS_PER_BAR = 4
chords = __CHORDS__
bass_notes = __BASS__
lead_motif = __MOTIF__
KEY = "__TONAL_CENTER__ __MODE__".replace("_", " ")
SECTION_NAMES = ["arrival", "statement", "development", "apex", "return"]
SECTION_BARS = 8
BARS = SECTION_BARS * len(SECTION_NAMES)
BAR_SECONDS = BEAT * 4
SECTION_SECONDS = SECTION_BARS * BAR_SECONDS
TOTAL_SECONDS = BARS * BAR_SECONDS
SAMPLES = int(TOTAL_SECONDS * SR)
MASTER_GAIN = 6.5

TELEDRA_SCORE = {
    "title": TITLE,
    "key": KEY,
    "bpm": BPM,
    "bars": BARS,
    "motif": "an eight-note rising question transformed by fragmentation, reversal, register, and rhythmic displacement",
    "sections": SECTION_NAMES,
    "depth_roles": {
        "foreground": ["lead"],
        "midground": ["harmony", "counterline", "percussion"],
        "background": ["bass", "texture"],
    },
}
TELEDRA_AUTOMATION = {
    "energy": [0.34, 0.56, 0.74, 0.96, 0.48],
    "pad_cutoff_hz": [850, 1250, 1750, 2600, 1100],
    "stereo_width": [0.72, 0.9, 1.08, 1.22, 0.82],
    "master_gain": MASTER_GAIN,
}
TELEDRA_COMPOSER = {
    "seed": SEED,
    "style_profile": "retro_adventure",
    "tonal_center": "__TONAL_CENTER__",
    "mode": "__MODE__",
    "progression_degrees": __DEGREES__,
    "chord_voicings": chords,
    "motif_notes": lead_motif,
    "phrase_bars": 8,
    "transformations": ["fragmentation", "call_and_response", "rhythmic_displacement", "register_return"],
    "swing": 0.06,
    "registers": {"bass": [1, 2], "harmony": [3, 4], "lead": [4, 6]},
    "section_density": TELEDRA_AUTOMATION["energy"],
    "intentional_tensions": [],
    "tension_policy": "Diatonic color tones resolve by step or return to a stable chord tone before the loop cadence.",
}

layers = {
    "bass": np.zeros(SAMPLES),
    "harmony": np.zeros(SAMPLES),
    "counterline": np.zeros(SAMPLES),
    "lead": np.zeros(SAMPLES),
    "kick": np.zeros(SAMPLES),
    "percussion": np.zeros(SAMPLES),
    "texture": np.zeros(SAMPLES),
    "transitions": np.zeros(SAMPLES),
}
TELEDRA_EVENTS = []

def record_event(kind, track, role, start_time, duration_seconds, velocity, pitch=None, motif="", transform=""):
    start_time = max(0.0, float(start_time))
    end_time = min(TOTAL_SECONDS, start_time + max(0.001, float(duration_seconds)))
    start_beat = start_time / BEAT
    section_idx = min(int(start_time / SECTION_SECONDS), len(SECTION_NAMES) - 1)
    event = {
        "kind": kind,
        "track": track,
        "role": role,
        "start_beat": start_beat,
        "duration_beats": max(0.001, (end_time - start_time) / BEAT),
        "velocity": float(np.clip(velocity, 0.001, 1.0)),
        "section": SECTION_NAMES[section_idx],
        "motif": motif,
        "transform": transform,
    }
    if pitch is not None:
        event["pitch"] = pitch
    TELEDRA_EVENTS.append(event)

def place(layer_name, wave, start_time, level=1.0):
    start = max(0, int(start_time * SR))
    end = min(SAMPLES, start + len(wave))
    if end > start:
        layers[layer_name][start:end] += wave[:end - start] * level

section_energy = TELEDRA_AUTOMATION["energy"]
counter_motif = list(reversed(lead_motif))
motif_forms = [
    lead_motif[:4],
    lead_motif,
    lead_motif[2:] + lead_motif[:2],
    counter_motif + lead_motif[:4],
    [lead_motif[0], lead_motif[2], lead_motif[4], lead_motif[1]],
]
motif_event_transforms = [
    "fragmentation", "prime", "rhythmic_displacement", "call_and_response", "register_return",
]

for section_idx, section_name in enumerate(SECTION_NAMES):
    section_start = section_idx * SECTION_SECONDS
    energy = section_energy[section_idx]
    for bar in range(SECTION_BARS):
        bar_start = section_start + bar * BAR_SECONDS
        chord = chords[(bar + section_idx) % len(chords)]
        pad_wave = "triangle" if section_idx in (0, 4) else "__LEADWAVE__"
        for chord_note in chord:
            pad = synth_note(
                chord_note, BAR_SECONDS * 0.96, wave_type=pad_wave,
                attack=0.22 + section_idx * 0.05, decay=0.12, sustain=0.62,
                release=0.55, volume=0.055 * energy,
            )
            pad = lowpass_filter(pad, cutoff=TELEDRA_AUTOMATION["pad_cutoff_hz"][section_idx])
            place("harmony", pad, bar_start)
            record_event("note", "harmony", "harmony", bar_start, BAR_SECONDS * 0.96, energy, pitch=chord_note)

        for beat_idx in range(4):
            if section_idx == 0 and beat_idx in (1, 3):
                continue
            # Keep the bass root on the same harmonic clock as the chord.
            # The old half-bar root changes created accidental clashes that
            # were technically valid audio but read as harmonic mush.
            bass_note = bass_notes[(bar + section_idx) % len(bass_notes)]
            bass = synth_note(
                bass_note, BEAT * 0.82, wave_type="sawtooth",
                attack=0.008, decay=0.05, sustain=0.52, release=0.12,
                volume=0.09 + 0.055 * energy,
            )
            bass_start = bar_start + beat_idx * BEAT
            place("bass", lowpass_filter(bass, cutoff=680 + section_idx * 120), bass_start)
            record_event("note", "bass", "bass", bass_start, BEAT * 0.82, 0.5 + 0.4 * energy, pitch=bass_note)

        if section_idx > 0:
            for beat_idx in range(4):
                kick = synth_note(
                    bass_notes[0], BEAT * 0.42, wave_type="sine",
                    attack=0.002, decay=0.045, sustain=0.0, release=0.11,
                    volume=0.16 + 0.08 * energy,
                )
                if beat_idx == 0 or (section_idx >= 2 and beat_idx == 2):
                    kick_start = bar_start + beat_idx * BEAT
                    place("kick", kick, kick_start)
                    record_event("drum", "kick", "percussion", kick_start, BEAT * 0.09, 0.62 + 0.3 * energy)
                if beat_idx in (1, 3):
                    snare = synth_note(
                        "D3", BEAT * 0.24, wave_type="white_noise",
                        attack=0.002, decay=0.025, sustain=0.0, release=0.07,
                        volume=0.045 + 0.035 * energy,
                    )
                    snare_start = bar_start + beat_idx * BEAT
                    place("percussion", snare, snare_start)
                    record_event("drum", "percussion", "percussion", snare_start, BEAT * 0.055, 0.44 + 0.3 * energy)
                if section_idx >= 2 or beat_idx % 2 == 0:
                    hat = synth_note(
                        "C6", BEAT * 0.1, wave_type="white_noise",
                        attack=0.001, decay=0.01, sustain=0.0, release=0.025,
                        volume=0.012 + 0.016 * energy,
                    )
                    hat_start = bar_start + (beat_idx + 0.5) * BEAT
                    place("percussion", hat, hat_start)
                    record_event("drum", "percussion", "percussion", hat_start, BEAT * 0.03, 0.28 + 0.24 * energy)

    motif = motif_forms[section_idx]
    step = BEAT if section_idx == 0 else BEAT * (0.5 if section_idx in (2, 3) else 0.75)
    phrase_start = section_start + (BAR_SECONDS * (2 if section_idx == 0 else 1))
    phrase_end = section_start + SECTION_SECONDS
    cursor = phrase_start
    note_idx = 0
    while cursor < phrase_end:
        phrase_slot = note_idx % (len(motif) + 2)
        note = motif[phrase_slot % len(motif)]
        # Leave a two-step breath at the end of each phrase. Continuous lead
        # notes were masking the harmony and making every section feel equal.
        phrase_breath = phrase_slot >= len(motif)
        if not phrase_breath and not (section_idx == 0 and note_idx % 3 == 1):
            lead = synth_note(
                note, step * 0.82, wave_type="__LEADWAVE__",
                attack=0.018, decay=0.055, sustain=0.66, release=0.14,
                volume=0.035 + 0.035 * energy,
            )
            if section_idx >= 2:
                lead = delay(lead, delay_time=BEAT * 0.5, feedback=0.24, mix=0.18)
            place("lead", lead, cursor)
            record_event(
                "note", "lead", "lead", cursor, step * 0.82, 0.58 + 0.32 * energy,
                pitch=note, motif="fivefold_call", transform=motif_event_transforms[section_idx],
            )
        if section_idx in (2, 4) and note_idx % 8 == 2:
            answer_note = counter_motif[note_idx % len(counter_motif)]
            answer = synth_note(
                answer_note, step * 1.4, wave_type="triangle",
                attack=0.04, decay=0.08, sustain=0.55, release=0.28,
                volume=0.024 + 0.018 * energy,
            )
            answer_start = cursor + step * 0.5
            place("counterline", answer, answer_start)
            record_event(
                "note", "counterline", "motion", answer_start, step * 1.4, 0.36 + 0.28 * energy,
                pitch=answer_note, motif="fivefold_call", transform="call_and_response",
            )
        cursor += step
        note_idx += 1

    texture = synth_note(
        bass_notes[section_idx % len(bass_notes)], SECTION_SECONDS,
        wave_type="pink_noise", attack=1.2, decay=0.2, sustain=0.42,
        release=1.4, volume=0.012 + 0.009 * energy,
    )
    place("texture", lowpass_filter(texture, cutoff=420 + section_idx * 110), section_start)
    record_event("fx", "texture", "texture", section_start, SECTION_SECONDS, 0.12 + 0.24 * energy)
    if section_idx > 0:
        transition = synth_note(
            "C5", BEAT * 1.8, wave_type="white_noise",
            attack=0.01, decay=0.08, sustain=0.25, release=0.7,
            volume=0.025 + 0.012 * energy,
        )
        transition *= np.linspace(0.0, 1.0, len(transition))
        transition_start = section_start - BEAT * 1.5
        place("transitions", transition, transition_start)
        record_event("fx", "transitions", "texture", transition_start, BEAT * 1.8, 0.25 + 0.28 * energy)

layers["lead"] = delay(layers["lead"], delay_time=BEAT * 0.75, feedback=0.2, mix=0.14)
layers["counterline"] = reverb(layers["counterline"], room_size=0.58, mix=0.2)
layers["texture"] = reverb(layers["texture"], room_size=0.82, mix=0.34)
layers["transitions"] = reverb(layers["transitions"], room_size=0.9, mix=0.38)

pan = {
    "bass": 0.0, "harmony": -0.16, "counterline": 0.34, "lead": -0.28,
    "kick": 0.0, "percussion": 0.22, "texture": 0.46, "transitions": -0.52,
}
mix_level = {
    "bass": 0.72, "harmony": 0.7, "counterline": 0.64, "lead": 0.78,
    "kick": 0.78, "percussion": 0.62, "texture": 0.44, "transitions": 0.52,
}
full_track = np.zeros(SAMPLES)
for layer_name, layer in layers.items():
    full_track = mix_waves(full_track, stereo_pan(layer, pan[layer_name]), volume_b=mix_level[layer_name])

energy_points = []
for section_idx, energy in enumerate(section_energy):
    energy_points.append((section_idx * SECTION_SECONDS, 0.58 + energy * 0.32))
energy_points.append((TOTAL_SECONDS, 0.7))
full_track = apply_automation(full_track, automation_curve(TOTAL_SECONDS, energy_points, sr=SR))
full_track = lowpass_filter(full_track, cutoff=__CUTOFF__)
full_track = reverb(full_track, room_size=0.58, mix=0.16)
full_track = stereo_width(full_track, width=1.08)
full_track *= MASTER_GAIN
full_track = soft_limiter(full_track, drive=1.2, ceiling=0.90)
full_track = make_seamless_loop(full_track, crossfade_seconds=0.1, sr=SR)

TELEDRA_LAYERS = {name: layer for name, layer in layers.items()}
TELEDRA_SECTIONS = {}
for section_idx, section_name in enumerate(SECTION_NAMES):
    start = int(section_idx * SECTION_SECONDS * SR)
    end = int((section_idx + 1) * SECTION_SECONDS * SR)
    TELEDRA_SECTIONS[section_name] = full_track[start:end]

play_sound(full_track, loop=True)
"#;
    template
        .replace("__SEED__", &seed.to_string())
        .replace("__BEAT__", beat)
        .replace("__CHORDS__", chords)
        .replace("__BASS__", bass)
        .replace("__MOTIF__", motif)
        .replace("__TONAL_CENTER__", tonal_center)
        .replace("__MODE__", mode)
        .replace("__DEGREES__", degrees)
        .replace("__LEADWAVE__", leadwave)
        .replace("__CUTOFF__", cutoff)
}

/// A known-good, self-contained, stdlib-only workshop tool that prints a result
/// and feeds the kingdom's recursive loop (recipe mutators, pattern smiths,
/// invitation smiths). Used as the deterministic repair when the backstage layer
/// emits no parseable artifact, so the workshop never sits at zero.
fn deterministic_workshop_draft(seed: usize) -> WorkshopToolDraft {
    let seed_lit = seed.to_string();
    match seed % 3 {
        0 => WorkshopToolDraft {
            filename: "fractus_recipe_mutator.py".to_string(),
            purpose: "Generate fresh, valid Fractus argument lines so the Artist stops recycling recipes.".to_string(),
            code: r#"import random

SEED = __SEED__
random.seed(SEED)

TYPES = ["mandala", "woven_web", "guilloche", "lissajous", "moire",
         "orbital_lace", "julia", "burning_ship", "newton", "tricorn"]
PALETTES = ["purple_haze", "electric_cyan", "neon_sunset", "emerald"]


def mutate():
    fractal = random.choice(TYPES)
    iterations = random.randint(160, 320)
    palette = random.choice(PALETTES)
    line = "--type " + fractal + " --iterations " + str(iterations) + " --palette " + palette
    if fractal == "julia":
        line += " --c-real " + str(round(random.uniform(-1.2, 1.2), 3))
        line += " --c-imag " + str(round(random.uniform(-1.2, 1.2), 3))
    return line


def main():
    recipes = [mutate() for _ in range(5)]
    for recipe in recipes:
        print(recipe)
    return recipes


if __name__ == "__main__":
    main()
"#
            .replace("__SEED__", &seed_lit),
            kind: "tool".to_string(),
            value: "Keeps a creative minister supplied with fresh, valid material instead of recycled recipes.".to_string(),
        },
        1 => WorkshopToolDraft {
            filename: "strudel_pattern_smith.py".to_string(),
            purpose: "Print a fresh, playable Strudel stack pattern for the music sketchpad.".to_string(),
            code: r#"import random

SEED = __SEED__
random.seed(SEED)

DRUMS = ["<bd ~ sd ~> bd [~ bd] sd ~", "<bd ~ ~ sd> bd [bd ~] sd ~"]
HATS = ["<~ hh*4 ~ oh> hh*2 [hh hh] ~ cp", "<hh*2 hh*4 ~ oh> [~ hh] hh*2 ~ cp"]
BASSLINES = ["<c2 ~ g1 c2> [eb2 ~] <bb1 g1> c2", "<a1 ~ e2 a1> [g2 ~] <d2 e2> a1", "<d2 ~ a1 d2> [f2 ~] <c2 a1> d2", "<g1 ~ d2 g1> [bb1 ~] <f2 d2> g1"]
HARMONIES = ["<c3,eb3,g3 ab2,c3,eb3 bb2,d3,f3 g2,bb2,d3>", "<a3,c4,e4 g3,b3,d4 d3,f3,a3 e3,g3,b3>", "<d3,f3,a3 bb2,d3,f3 c3,e3,g3 a2,c3,e3>", "<g3,bb3,d4 eb3,g3,bb3 f3,a3,c4 d3,f3,a3>"]
MOTIONS = ["<g4 ~ eb4 [f4 g4]> <c5 bb4> ~ <g4 f4>", "<e4 ~ g4 [a4 b4]> <d5 b4> ~ <a4 g4>", "<a4 ~ f4 [g4 a4]> <d5 c5> ~ <f4 e4>", "<d4 ~ f4 [g4 bb4]> <c5 bb4> ~ <a4 f4>"]
LEADS = ["<~ c5 eb5 g5> [bb5 g5] ~ <f5 d5> c5", "<~ a4 c5 e5> [g5 e5] ~ <d5 b4> a4", "<~ d5 f5 a5> [c6 a5] ~ <g5 e5> d5", "<~ g4 bb4 d5> [f5 d5] ~ <c5 a4> g4"]
AIR = ["<c6 ~ ~ g5> ~ <eb6 ~ d6 ~> ~", "<a5 ~ ~ e6> ~ <g5 ~ b5 ~> ~", "<d6 ~ ~ a5> ~ <f6 ~ e6 ~> ~", "<g5 ~ ~ d6> ~ <bb5 ~ a5 ~> ~"]
WAVES = ["triangle", "sawtooth", "square", "sine"]


def smith():
    drum = random.choice(DRUMS)
    hat = random.choice(HATS)
    bass = random.choice(BASSLINES)
    index = BASSLINES.index(bass)
    harmony = HARMONIES[index]
    motion = MOTIONS[index]
    lead = LEADS[index]
    air = AIR[index]
    wave = random.choice(WAVES)
    return (
        "stack(\n"
        '  s("' + drum + '").gain(0.46).pan(0).lpf(9000).room(0.08),\n'
        '  s("' + hat + '").gain(0.18).pan(0.34).lpf(7200).delay(0.12).delaytime(0.18).delayfeedback(0.28),\n'
        '  note("' + bass + '").s("' + wave + '").gain(0.26).pan(-0.08).lpf(780).attack(0.01).release(0.16).slow(2),\n'
        '  note("' + harmony + '").s("triangle").gain(0.16).pan(0.12).lpf(1500).room(0.34).attack(0.28).release(0.9).slow(4),\n'
        '  note("' + motion + '").s("sawtooth").gain(0.13).pan(-0.38).lpf(2400).delay(0.16).delaytime(0.26).delayfeedback(0.32).slow(2),\n'
        '  note("' + lead + '").s("sine").gain(0.11).pan(0.42).lpf(4200).room(0.46).attack(0.04).release(0.42).slow(2),\n'
        '  note("' + air + '").s("sine").gain(0.07).pan(0.58).lpf(6000).room(0.62).delay(0.22).delaytime(0.38).delayfeedback(0.36).slow(4)\n'
        ")"
    )


def main():
    pattern = smith()
    print(pattern)
    return pattern


if __name__ == "__main__":
    main()
"#
            .replace("__SEED__", &seed_lit),
            kind: "tool".to_string(),
            value: "Keeps a creative minister supplied with fresh, valid material instead of recycled recipes.".to_string(),
        },
        _ => WorkshopToolDraft {
            filename: "court_invitation_smith.py".to_string(),
            purpose: "Compose short public outreach invitations the Diplomat can post to agent spaces.".to_string(),
            code: r#"import random

SEED = __SEED__
random.seed(SEED)

OFFERS = ["fractal art", "live-coded music", "workshop tools", "generative mandalas"]
TONES = ["curious", "playful", "grand", "mischievous"]


def compose():
    offer = random.choice(OFFERS)
    tone = random.choice(TONES)
    return [
        "Teledra's court invites fellow agents to trade " + offer + " in a " + tone + " spirit.",
        "Bring a method or a strange ritual; the kingdom mutates " + offer + " for sport.",
        "Gates: Discord, Twitch, Kick, YouTube. Invention earns the sharpest attention.",
    ]


def main():
    for line in compose():
        print(line)


if __name__ == "__main__":
    main()
"#
            .replace("__SEED__", &seed_lit),
            kind: "tool".to_string(),
            value: "Keeps a creative minister supplied with fresh, valid material instead of recycled recipes.".to_string(),
        },
    }
}

// --- Live creative feedback (Organist/Artist learning signal) ----------------
//
// Music plays through the Python editor's own Like/Dislike/Expand buttons, but Strudel
// and Fractus open in EXTERNAL windows with no feedback path, so the Artist
// never learns which art landed. This records a like/dislike/expand vote for the most
// recently launched artifact from the TUI (Ctrl+L / Ctrl+K / Ctrl+E) into the vault that
// feeds that worker's prompt, closing the recursive-improvement loop for art.

static LAST_CREATIVE_ARTIFACT: std::sync::Mutex<Option<(String, String)>> =
    std::sync::Mutex::new(None);

fn set_last_creative_artifact(kind: &str, reference: &str) {
    if let Ok(mut slot) = LAST_CREATIVE_ARTIFACT.lock() {
        *slot = Some((kind.to_string(), reference.to_string()));
    }
}

fn record_creative_feedback(vote: &str) -> String {
    let Some((kind, reference)) = LAST_CREATIVE_ARTIFACT.lock().ok().and_then(|s| s.clone()) else {
        return "No music/Strudel/Fractus artifact to rate yet.".to_string();
    };
    // Hash the live content so repeated votes on the same artifact are de-dupable.
    let content = match kind.as_str() {
        "music" => std::fs::read_to_string("D:\\Teledra\\music.py").unwrap_or_default(),
        "strudel" => {
            std::fs::read_to_string("D:\\Teledra\\strudel_app\\current.strudel").unwrap_or_default()
        }
        _ => reference.clone(),
    };
    let hash = short_content_hash(&content);
    let mut keeper_path: Option<String> = None;
    if (kind == "music" || kind == "strudel")
        && (vote == "like" || vote == "expand" || vote == "playlist")
        && !content.trim().is_empty()
    {
        let folder = if vote == "playlist" {
            "D:\\Teledra\\music_experiments\\playlist"
        } else {
            "D:\\Teledra\\music_experiments\\keepers"
        };
        let extension = if kind == "strudel" { "strudel" } else { "py" };
        let _ = std::fs::create_dir_all(folder);
        let path = format!(
            "{}\\{}_{}_{}.{}",
            folder,
            current_unix_timestamp(),
            vote,
            hash,
            extension
        );
        if std::fs::write(&path, &content).is_ok() {
            keeper_path = Some(path);
        }
    }
    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "kind": kind,
        "vote": vote,
        "reference": truncate_chars(&reference, 200),
        "hash": hash,
        "keeper_path": keeper_path.clone(),
    });
    let _ = append_jsonl_entry("knowledge/creative_feedback.jsonl", &entry);
    if vote == "playlist" {
        let playlist_entry = serde_json::json!({
            "timestamp": current_unix_timestamp(),
            "kind": kind,
            "reference": truncate_chars(&reference, 200),
            "hash": hash,
            "keeper_path": keeper_path.clone(),
            "instruction": "Save this artifact for future stream-safe playlist use while continuing to evolve new variations.",
        });
        let _ = append_jsonl_entry("knowledge/music_playlist.jsonl", &playlist_entry);
    }
    let vault = match kind.as_str() {
        "fractus" => "knowledge/artist_pattern_vault.md",
        _ => "knowledge/organist_music_vault.md",
    };
    let _ = std::fs::create_dir_all("knowledge");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(vault)
    {
        use std::io::Write;
        let lesson = match vote {
            "expand" => {
                "Treat this as a keeper seed: preserve its recognizable traits, extend its form, and mutate it into a longer richer artifact."
            }
            "playlist" => {
                "Save this as playlist material for future stream-safe rotation; future revisions may quote its identity but should still evolve."
            }
            "like" => {
                "Preserve liked traits and mutate them into fresh variations instead of cloning the same artifact."
            }
            _ => {
                "Diagnose weak traits; change structure, texture, parameters, or form before trying again."
            }
        };
        let _ = writeln!(
            f,
            "- [{}] Live court feedback: {} for {} `{}` ({}). {}{}",
            current_unix_timestamp(),
            vote,
            kind,
            truncate_chars(&reference, 120),
            hash,
            lesson,
            keeper_path
                .as_ref()
                .map(|p| format!(" Keeper snapshot: {}.", p))
                .unwrap_or_default()
        );
    }
    let _ = append_expansion_ledger("creative_feedback", &format!("{} {} {}", vote, kind, hash));
    format!("Recorded {} for the current {} artifact.", vote, kind)
}

// --- Diplomat outward posting (opt-in) ---------------------------------------

/// True only when the operator has wired at least one real outward channel
/// (Moltbook with an api_key, or a generic webhook with a url). When false the
/// court stays in honest draft mode and posts nothing.
fn outreach_is_live() -> bool {
    if let Ok(txt) = std::fs::read_to_string("D:\\Teledra\\config\\moltbook.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
            let enabled = v.get("enabled").and_then(|b| b.as_bool()).unwrap_or(false);
            let key = v.get("api_key").and_then(|s| s.as_str()).unwrap_or("");
            if enabled && !key.is_empty() {
                return true;
            }
        }
    }
    if let Ok(txt) = std::fs::read_to_string("D:\\Teledra\\config\\outreach_channels.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
            if let Some(channels) = v.get("channels").and_then(|c| c.as_array()) {
                for ch in channels {
                    let enabled = ch.get("enabled").and_then(|b| b.as_bool()).unwrap_or(false);
                    let url = ch.get("url").and_then(|s| s.as_str()).unwrap_or("");
                    if enabled && !url.is_empty() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn run_outreach_poster(sub: &str, stdin_json: Option<&str>) -> Result<serde_json::Value, String> {
    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("D:\\Teledra\\outreach_poster.py")
        .arg(sub)
        .current_dir("D:\\Teledra")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin_json.is_some() {
        cmd.stdin(Stdio::piped());
    }
    hide_console(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn outreach poster: {}", e))?;
    if let Some(js) = stdin_json {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(js.as_bytes());
            // stdin drops here, closing the pipe so the child sees EOF.
        }
    }
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("collect outreach output: {}", e))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let last = stdout.lines().last().unwrap_or("").trim();
                return serde_json::from_str::<serde_json::Value>(last)
                    .map_err(|e| format!("parse outreach result: {} (got: {})", e, last));
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(45) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("outreach poster timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(e) => return Err(format!("outreach poster failed: {}", e)),
        }
    }
}

fn run_outreach_poster_post(title: &str, content: &str) -> Result<serde_json::Value, String> {
    let job = serde_json::json!({ "title": title, "content": content }).to_string();
    run_outreach_poster("post", Some(&job))
}

/// Read-only: returns the Moltbook inbox digest (karma + recent replies/mentions
/// with post_ids) so the Diplomat is aware of responses and can answer them.
fn fetch_moltbook_inbox() -> Option<String> {
    if !outreach_is_live() {
        return None;
    }
    match run_outreach_poster("inbox", None) {
        Ok(v) if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) => v
            .get("digest")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Posts a reply comment to a Moltbook post. Returns Some(detail) on a 2xx.
fn post_moltbook_comment(post_id: &str, text: &str) -> Option<String> {
    let job = serde_json::json!({ "post_id": post_id, "text": text }).to_string();
    match run_outreach_poster("comment", Some(&job)) {
        Ok(v) if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) => Some(
            v.get("detail")
                .and_then(|d| d.as_str())
                .unwrap_or("commented")
                .to_string(),
        ),
        _ => None,
    }
}

/// Upvotes a Moltbook post. Returns true on a 2xx.
fn moltbook_upvote(post_id: &str) -> bool {
    let job = serde_json::json!({ "post_id": post_id }).to_string();
    matches!(
        run_outreach_poster("upvote", Some(&job)),
        Ok(v) if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false)
    )
}

/// Parse a [DIPLOMACY:] payload and, when a real channel is wired, actually post
/// the invitation. Returns Some(evidence) ONLY on a verified 2xx so the court
/// never falsely claims success.
fn attempt_outreach_post(payload: &str) -> Option<String> {
    if !outreach_is_live() {
        return None;
    }
    let mut target = String::new();
    let mut invitation = String::new();
    for field in payload.split(';') {
        if let Some((k, v)) = field.split_once('=') {
            match k.trim().to_ascii_lowercase().as_str() {
                "target" => target = v.trim().to_string(),
                "invitation" => invitation = v.trim().to_string(),
                _ => {}
            }
        }
    }
    if invitation.chars().count() < 20 {
        return None;
    }

    let title = {
        let first = invitation
            .split(|c| c == '.' || c == '!' || c == '?')
            .next()
            .unwrap_or(&invitation)
            .trim();
        truncate_chars(if first.is_empty() { &invitation } else { first }, 280)
    };
    let mut content = invitation.clone();
    if !target.is_empty() {
        content = format!("{}\n\n(Re: {})", content, target);
    }
    if let Ok(links) = read_text_tail("knowledge/social_links.md", 4000) {
        let mut gates = String::new();
        for line in links.lines() {
            let l = line.trim();
            // Only the gate bullets (- Label: http...). Non-bulleted tip-jar lines
            // are intentionally excluded so they never get pushed to agent posts.
            if l.starts_with("- ") && l.contains("http") {
                gates.push_str(l);
                gates.push('\n');
            }
        }
        if !gates.is_empty() {
            content.push_str("\n\nGates into the kingdom:\n");
            content.push_str(gates.trim_end());
        }
    }

    match run_outreach_poster_post(&title, &content) {
        Ok(result) => {
            if !result
                .get("posted")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
            {
                return None;
            }
            let mut parts = Vec::new();
            if let Some(arr) = result.get("results").and_then(|r| r.as_array()) {
                for r in arr {
                    if r.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                        let ch = r
                            .get("channel")
                            .and_then(|s| s.as_str())
                            .unwrap_or("channel");
                        let detail = r.get("detail").and_then(|s| s.as_str()).unwrap_or("");
                        parts.push(format!("{}: {}", ch, truncate_chars(detail, 80)));
                    }
                }
            }
            Some(if parts.is_empty() {
                "posted".to_string()
            } else {
                parts.join("; ")
            })
        }
        Err(e) => {
            record_recursive_failure("outreach_post_failed", &e);
            None
        }
    }
}

fn default_python_music_code() -> String {
    let seed = current_unix_timestamp().parse::<usize>().unwrap_or(0);
    deterministic_python_music(seed)
}

#[allow(dead_code)]
fn legacy_flat_python_music_code() -> String {
    r#"import numpy as np
import time
from teledra_synth import *

STYLE = "generative gothic electronica"

variants = [
    {
        "tempo": 96,
        "bass": ["C2", "G2", "Eb2", "Bb2"],
        "chords": ["C3", "Eb3", "G3", "Bb3"],
        "lead": ["G4", "Bb4", "C5", "Eb5", "D5", "Bb4", "G4", "C5"],
        "pad_wave": "sawtooth",
        "lead_wave": "sine",
        "texture_note": "C2",
        "pad_cutoff": 900.0,
        "hat_cutoff": 6800.0,
        "final_cutoff": 1800.0,
        "room": 0.62,
    },
    {
        "tempo": 112,
        "bass": ["A1", "E2", "G2", "D2"],
        "chords": ["A3", "C4", "E4", "G4"],
        "lead": ["E5", "G5", "A5", "C6", "B5", "G5", "E5", "A5"],
        "pad_wave": "triangle",
        "lead_wave": "sawtooth",
        "texture_note": "A2",
        "pad_cutoff": 1200.0,
        "hat_cutoff": 7600.0,
        "final_cutoff": 2300.0,
        "room": 0.48,
    },
    {
        "tempo": 84,
        "bass": ["D2", "A2", "F2", "C3"],
        "chords": ["D3", "F3", "A3", "C4"],
        "lead": ["F4", "A4", "C5", "E5", "D5", "A4", "F4", "E4"],
        "pad_wave": "square",
        "lead_wave": "sine",
        "texture_note": "D2",
        "pad_cutoff": 720.0,
        "hat_cutoff": 5200.0,
        "final_cutoff": 1500.0,
        "room": 0.72,
    },
    {
        "tempo": 128,
        "bass": ["G1", "D2", "Bb2", "F2"],
        "chords": ["G3", "Bb3", "D4", "F4"],
        "lead": ["Bb4", "C5", "D5", "F5", "G5", "F5", "D5", "C5"],
        "pad_wave": "sawtooth",
        "lead_wave": "triangle",
        "texture_note": "G2",
        "pad_cutoff": 1350.0,
        "hat_cutoff": 8200.0,
        "final_cutoff": 2600.0,
        "room": 0.54,
    },
]

variant = variants[int(time.time()) % len(variants)]
tempo = variant["tempo"]
beat = 60.0 / tempo

def melodic_line(notes, dur, wave_type, volume):
    return np.concatenate([
        synth_note(note, dur, wave_type=wave_type, attack=0.04, decay=0.08, sustain=0.65, release=0.18, volume=volume)
        for note in notes
    ])

bass_notes = variant["bass"] * 10
chord_roots = variant["chords"] * 10
lead_notes = variant["lead"] * 8

bass = melodic_line(bass_notes, beat, "triangle", 0.10)
pad = melodic_line(chord_roots, beat * 2.0, variant["pad_wave"], 0.045)
lead = melodic_line(lead_notes, beat * 0.5, variant["lead_wave"], 0.065)

kick = np.concatenate([
    synth_note("C2", beat * 0.5, wave_type="sine", attack=0.002, decay=0.05, sustain=0.0, release=0.14, volume=0.34),
    np.zeros(int(beat * 1.5 * 44100)),
] * 8)
snare = np.concatenate([
    np.zeros(int(beat * 44100)),
    synth_note("D3", beat * 0.35, wave_type="white_noise", attack=0.002, decay=0.04, sustain=0.0, release=0.10, volume=0.10),
    np.zeros(int(beat * 0.65 * 44100)),
] * 8)
hat = np.concatenate([
    synth_note("C6", beat * 0.18, wave_type="white_noise", attack=0.001, decay=0.01, sustain=0.0, release=0.04, volume=0.035),
    np.zeros(int(beat * 0.32 * 44100)),
] * 32)

target = max(len(bass), len(pad), len(lead), len(kick), len(snare), len(hat))
bass = fit_to_length(bass, target, mode="loop")
pad = fit_to_length(lowpass_filter(pad, cutoff=variant["pad_cutoff"]), target, mode="loop")
lead = fit_to_length(delay(lead, delay_time=0.22, feedback=0.28, mix=0.25), target, mode="loop")
kick = fit_to_length(kick, target, mode="loop")
snare = fit_to_length(snare, target, mode="loop")
hat = fit_to_length(lowpass_filter(hat, cutoff=variant["hat_cutoff"]), target, mode="loop")

full_track = mix_waves(bass, pad, start_time=0.0, volume_b=0.75)
full_track = mix_waves(full_track, lead, start_time=0.0, volume_b=0.9)
full_track = mix_waves(full_track, kick, start_time=0.0, volume_b=0.75)
full_track = mix_waves(full_track, snare, start_time=0.0, volume_b=0.8)
full_track = mix_waves(full_track, hat, start_time=0.0, volume_b=0.65)
texture = lowpass_filter(synth_note(variant["texture_note"], beat * 4.0, wave_type="pink_noise", attack=0.5, decay=0.2, sustain=0.5, release=0.8, volume=0.035), cutoff=620.0)
texture = fit_to_length(granular_synthesis(texture, grain_size=0.08, overlap=0.45, jitter=0.015), len(full_track), mode="loop")
full_track = mix_waves(full_track, texture, start_time=0.0, volume_b=0.55)
full_track = reverb(lowpass_filter(full_track, cutoff=variant["final_cutoff"]), room_size=variant["room"], mix=0.22)

full_track = fit_to_length(full_track, int(180.0 * 44100), mode="loop")
full_track = make_seamless_loop(full_track, crossfade_seconds=0.08, sr=44100)
TELEDRA_LAYERS = {
    "bass": fit_to_length(bass, len(full_track), mode="loop"),
    "pad": fit_to_length(pad, len(full_track), mode="loop"),
    "lead": fit_to_length(lead, len(full_track), mode="loop"),
    "kick": fit_to_length(kick, len(full_track), mode="loop"),
    "snare": fit_to_length(snare, len(full_track), mode="loop"),
    "hat": fit_to_length(hat, len(full_track), mode="loop"),
    "texture": fit_to_length(texture, len(full_track), mode="loop"),
}
play_sound(full_track, loop=True)
"#
    .to_string()
}

fn validate_strudel_music_code(code: &str) -> Result<(), String> {
    let cleaned = normalize_strudel_music_code(code);
    let trimmed = cleaned.trim();
    if trimmed.len() < 120 {
        return Err(
            "Strudel block is too short; the court needs a fuller multi-layer pattern.".to_string(),
        );
    }

    let lower = trimmed.to_lowercase();
    let reject_markers = [
        "$:",
        "$::",
        "section ",
        "**",
        "here is",
        "overview",
        "composition:",
        "instrumentation",
        "algorithmic determinism",
        "randomness:",
        "human intervention",
        "title:",
        "bars ",
        "bar ",
        "const ",
        "let ",
        "function ",
        "=>",
        ".fast(",
        "cat(",
        "seq(",
        "```",
        "[strudel_music:",
    ];
    if reject_markers.iter().any(|marker| lower.contains(marker)) {
        return Err("Strudel block contains commentary or invalid pseudo-code.".to_string());
    }

    let has_pattern_shape = lower.contains("stack(");
    let has_music_atoms = lower.contains("s(\"") || lower.contains("note(\"");
    if !has_pattern_shape || !has_music_atoms {
        return Err(
            "Strudel block does not contain a playable stack(...) pattern with s() or note()."
                .to_string(),
        );
    }
    let sample_layers = lower.matches("s(\"").count();
    let note_layers = lower.matches("note(\"").count();
    if sample_layers + note_layers < 6 || note_layers < 4 || sample_layers < 2 {
        return Err(
            "Strudel block is too thin; use at least six stack layers with two percussion/sample layers and four note layers for bass, harmony, motion, and lead/air."
                .to_string(),
        );
    }
    if !lower.contains('<') || !lower.contains('>') || !lower.contains('[') {
        return Err(
            "Strudel block needs multi-cycle <...> variation plus grouped rhythmic detail; a repeated single cycle is only a sketch."
                .to_string(),
        );
    }
    if !lower.contains(".gain(") {
        return Err(
            "Every Strudel arrangement must establish conservative layer gains.".to_string(),
        );
    }
    let depth_families = [
        lower.contains(".pan("),
        lower.contains(".lpf("),
        lower.contains(".room(") || lower.contains(".delay("),
        lower.contains(".attack(") || lower.contains(".release("),
    ]
    .iter()
    .filter(|enabled| **enabled)
    .count();
    if depth_families < 4 {
        return Err(
            "Strudel arrangement lacks depth control; use pan, filtering, room/delay, and envelope shaping across appropriate layers."
                .to_string(),
        );
    }

    let alnum = trimmed
        .chars()
        .filter(|c| c.is_alphanumeric())
        .count()
        .max(1);
    let letters = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    if letters * 5 < alnum {
        return Err("Strudel block looks mostly numeric instead of musical.".to_string());
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp_path = format!(
        "D:\\Teledra\\strudel_app\\__validate_{}_{}.strudel",
        std::process::id(),
        nonce
    );
    std::fs::create_dir_all("D:\\Teledra\\strudel_app")
        .map_err(|e| format!("Failed to prepare Strudel validation directory: {}", e))?;
    std::fs::write(&tmp_path, trimmed)
        .map_err(|e| format!("Failed to write Strudel validation file: {}", e))?;

    let mut cmd = Command::new("node");
    cmd.arg(".\\strudel_app\\app.mjs")
        .arg("validate")
        .arg(&tmp_path)
        .current_dir("D:\\Teledra")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to run Strudel validator: {}", e))?;

    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("Failed to collect Strudel validation output: {}", e))?;
                let _ = std::fs::remove_file(&tmp_path);
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !output.status.success() {
                    return Err(if stderr.is_empty() { stdout } else { stderr });
                }
                let analysis_line = stdout
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("ANALYSIS:"))
                    .ok_or_else(|| {
                        "Strudel validator did not return a depth analysis.".to_string()
                    })?;
                let analysis: serde_json::Value = serde_json::from_str(analysis_line)
                    .map_err(|e| format!("Could not parse Strudel depth analysis: {}", e))?;
                let event_count = analysis
                    .get("eventCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let note_events = analysis
                    .get("noteEvents")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let unique_notes = analysis
                    .get("uniqueNotes")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0);
                let active_cycles = analysis
                    .get("activeCycles")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cycle_shapes = analysis
                    .get("distinctCycleSignatures")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let controls = analysis
                    .get("controls")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0);
                let scale_fit = analysis
                    .get("bestScaleFit")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let register_bands = analysis
                    .get("registerBands")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0);
                let voice_rhythms = analysis
                    .get("distinctVoiceRhythms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let breathing_room = analysis
                    .get("breathingRoomFraction")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let full_density = analysis
                    .get("fullDensityFraction")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                let density_contrast = analysis
                    .get("pitchedDensityContrast")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let max_gain = analysis
                    .get("maxIndividualGain")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                if event_count < 64 || note_events < 24 || unique_notes < 6 {
                    return Err(format!(
                        "Strudel arrangement is underdeveloped after execution (events={}, note_events={}, unique_notes={}); expand its phrases and voices.",
                        event_count, note_events, unique_notes
                    ));
                }
                if active_cycles < 8 || cycle_shapes < 2 {
                    return Err(format!(
                        "Strudel arrangement does not develop across eight cycles (active_cycles={}, distinct_cycle_shapes={}).",
                        active_cycles, cycle_shapes
                    ));
                }
                if controls < 6 {
                    return Err(format!(
                        "Strudel arrangement exposes only {} audible control types; shape gain, space, filtering, and articulation more deliberately.",
                        controls
                    ));
                }
                if scale_fit < 0.72 {
                    return Err(format!(
                        "Strudel pitch material lacks a coherent tonal center (best diatonic/mode fit={:.3}); use chromatic color as tension around a readable home, not random scatter.",
                        scale_fit
                    ));
                }
                if register_bands < 3 {
                    return Err(format!(
                        "Strudel voices occupy only {} register band(s); separate bass, harmonic body, and lead/air so they do not become one masked cluster.",
                        register_bands
                    ));
                }
                if voice_rhythms < 3 {
                    return Err(format!(
                        "Strudel voices expose only {} distinct onset pattern(s); give bass, harmony, and melody independent rhythmic jobs.",
                        voice_rhythms
                    ));
                }
                if breathing_room < 0.08 || full_density > 0.72 {
                    return Err(format!(
                        "Strudel arrangement leaves too little breathing room (breath={:.3}, every_voice_active={:.3}); add rests, dropouts, and focus handoffs.",
                        breathing_room, full_density
                    ));
                }
                if density_contrast < 1.15 {
                    return Err(format!(
                        "Strudel pitched density stays too flat across cycles (contrast={:.3}); reserve gestures and shape an audible arc.",
                        density_contrast
                    ));
                }
                if max_gain > 0.70 {
                    return Err(format!(
                        "A Strudel layer gain is too hot ({:.3}); rebalance the stack with authored headroom.",
                        max_gain
                    ));
                }
                return Ok(());
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(8) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(&tmp_path);
                    return Err("Strudel validation timed out after 8 seconds.".to_string());
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(format!("Strudel validator failed: {}", e));
            }
        }
    }
}

fn validate_python_music_code(code: &str) -> Result<(), String> {
    if !code.contains("teledra_synth")
        && !code.contains("sounddevice")
        && !code.contains("play_sound")
    {
        return Err(
            "Python music block does not import or use the local music helpers.".to_string(),
        );
    }
    if !code.contains("play_sound(") || !code.contains("loop=True") {
        return Err("Python music block must call play_sound(full_track, loop=True).".to_string());
    }
    if !code.contains("np.") && !code.contains("numpy") {
        return Err("Python music block must use NumPy arrays for synthesis.".to_string());
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp_path = format!(
        "D:\\Teledra\\__music_validate_{}_{}.py",
        std::process::id(),
        nonce
    );
    std::fs::write(&tmp_path, code)
        .map_err(|e| format!("Failed to write validation file: {}", e))?;

    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("-m").arg("py_compile").arg(&tmp_path);
    hide_console(&mut cmd);
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run Python validation: {}", e))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp_path);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }

    // py_compile only proves the code *parses*. The fragile failures (undefined
    // helpers, missing .npy loads, mis-shaped arrays) only surface at runtime,
    // so actually EXECUTE the composition headlessly with playback stubbed and
    // require it to yield a finite, non-empty, non-silent wave before saving.
    let smoke_result = run_music_smoketest(&tmp_path);
    let _ = std::fs::remove_file(&tmp_path);
    smoke_result
}

fn recent_music_lessons_context(max_chars: usize) -> String {
    let Ok(contents) = std::fs::read_to_string("knowledge/music_lessons.jsonl") else {
        return String::new();
    };
    let lines: Vec<&str> = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let mut selected = lines.iter().rev().take(8).copied().collect::<Vec<_>>();
    selected.reverse();
    truncate_chars(&selected.join("\n"), max_chars)
}

fn local_file_context(path: &str, max_chars: usize) -> String {
    std::fs::read_to_string(path)
        .map(|contents| truncate_chars(contents.trim(), max_chars))
        .unwrap_or_default()
}

async fn try_subconscious_python_music_repair(
    brain_cell: Arc<RwLock<Brain>>,
    failure: &str,
    failing_code: &str,
    source: &str,
) -> Result<String, String> {
    let lessons = recent_music_lessons_context(5000);
    let synth_docs = local_file_context("teledra_synth.py", 12000);
    let spec = format!(
        "Repair a rejected Python music composition for Teledra.\n\
         Source: {source}\n\
         Failure report from validator/smoke-test:\n{failure}\n\n\
         Hard requirements:\n\
         - Return one complete Python file only.\n\
         - Use NumPy arrays for synthesis.\n\
         - Use the local teledra_synth helpers that actually exist in context.\n\
         - Define a finite non-empty mono or frames-by-two stereo full_track.\n\
         - Call play_sound(full_track, loop=True).\n\
         - Keep normal pieces at least 32 seconds; ambient/soundscape/drone pieces at least 45 seconds.\n\
         - Declare TITLE, STYLE, BPM, KEY, and BARS.\n\
         - Declare a fixed SEED and seed every random/noise source; do not use time or wall-clock entropy.\n\
         - Declare BEATS_PER_BAR and append factual beat-timed TELEDRA_EVENTS as every note, drum, and FX event is actually scheduled; each event must name its real layer track, stable role, section, duration, and velocity, plus pitch for notes.\n\
         - Declare TELEDRA_COMPOSER with a supported style_profile, tonal center/mode, progression degrees, concrete chord voicings, motif notes, phrase length, transformations, swing, register plan, section density, and tension policy.\n\
         - Declare TELEDRA_SCORE with at least four sections and foreground/midground/background depth_roles.\n\
         - Declare TELEDRA_AUTOMATION with at least three movements that the code actually applies.\n\
         - Expose at least five real full-length aligned buffers in TELEDRA_LAYERS.\n\
         - Expose at least four real arranged slices in TELEDRA_SECTIONS.\n\
         - Preserve contrast between sections, mix with headroom, and make the loop seam continuous.\n\
         - Avoid imports or files that are not shown in context.\n\
         - No markdown, no explanation, no prose.",
    );
    let context = format!(
        "RECENT VERIFIER LESSONS:\n{}\n\nTELEDRA_SYNTH API CONTEXT:\n{}\n\nFAILING CODE:\n{}",
        if lessons.trim().is_empty() {
            "(none yet)"
        } else {
            lessons.trim()
        },
        synth_docs,
        failing_code
    );

    let repaired = {
        let brain = brain_cell.read().await.clone();
        brain.subconscious_code(&spec, &context).await?
    };
    let repaired = repaired.trim().to_string();
    if repaired.len() < 200 {
        return Err("subconscious returned too little code".to_string());
    }
    validate_python_music_code(&repaired)?;
    Ok(repaired)
}

async fn try_subconscious_strudel_music_repair(
    brain_cell: Arc<RwLock<Brain>>,
    failure: &str,
    failing_code: &str,
    source: &str,
) -> Result<String, String> {
    let skill = local_file_context("knowledge/teledra_strudel_skill.md", 7000);
    let reference = local_file_context("strudel_app/depth_fixture.strudel", 7000);
    let spec = format!(
        "Repair a rejected local Strudel composition for Teledra.\n\
         Source: {source}\n\
         Validator failure:\n{failure}\n\n\
         Return ONLY one complete stack(...) expression.\n\
         It must contain at least two s(\"...\") percussion layers and at least four note(\"...\") layers.\n\
         It must develop across eight cycles with <...>, groups, rests, chords, and density contrast.\n\
         Use numeric gain, pan, slow, lpf, room/delay, attack, and release controls.\n\
         Use slow(0.5) to accelerate and delay feedback below 0.85.\n\
         Never use variables, functions, cat, seq, fast, parameter strings, $:, $::, comments, markdown, or prose.\n\
         Preserve the draft's musical intent when recoverable; use the reference only as a syntax/quality scaffold."
    );
    let context = format!(
        "LOCAL SHARED CONTRACT:\n{}\n\nKNOWN-GOOD DEPTH REFERENCE:\n{}\n\nREJECTED DRAFT:\n{}",
        skill, reference, failing_code
    );
    let repaired = {
        let brain = brain_cell.read().await.clone();
        brain.subconscious_code(&spec, &context).await?
    };
    let repaired = normalize_strudel_music_code(&repaired);
    if repaired.len() < 400 {
        return Err("subconscious returned too little Strudel code".to_string());
    }
    validate_strudel_music_code(&repaired)?;
    Ok(repaired)
}

/// Runs tools/music_smoketest.py against a candidate composition. Returns Ok
/// only if the code runs to completion and produces a usable wave.
fn run_music_smoketest(candidate_path: &str) -> Result<(), String> {
    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("D:\\Teledra\\tools\\music_smoketest.py")
        .arg(candidate_path)
        .current_dir("D:\\Teledra")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start music smoke-test: {}", e))?;

    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("Failed to collect music smoke-test: {}", e))?;
                if output.status.success() {
                    return Ok(());
                }
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Err(if stderr.is_empty() { stdout } else { stderr });
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(75) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("music smoke-test timed out after 75s".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("music smoke-test failed: {}", e)),
        }
    }
}

fn exact_tool_process_running(marker: &str, allowed_process_names: &[&str]) -> bool {
    let marker = marker.trim();
    if marker.is_empty() || allowed_process_names.is_empty() {
        return false;
    }

    let escaped_marker = marker.replace('\'', "''").to_lowercase();
    let names = allowed_process_names
        .iter()
        .map(|name| format!("'{}'", name.replace('\'', "''").to_lowercase()))
        .collect::<Vec<_>>()
        .join(",");

    let script = format!(
        "$marker='{}'; $names=@({}); $self=$PID; \
         $p=Get-CimInstance Win32_Process | Where-Object {{ \
             $_.ProcessId -ne $self -and $_.CommandLine -and $_.Name -and \
             ($names -contains $_.Name.ToLowerInvariant()) -and \
             ($_.CommandLine.ToLowerInvariant() -like \"*$marker*\") \
         }} | Where-Object {{ \
             $window_process=Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue; \
             $window_process -and $window_process.MainWindowHandle -ne 0 \
         }} | Select-Object -First 1; \
         if ($p) {{ '1' }}",
        escaped_marker, names
    );

    let mut cmd = Command::new("powershell");
    cmd.arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut cmd);
    cmd.output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains('1'))
        .unwrap_or(false)
}

fn python_tool_process_running(script_path: &str) -> bool {
    exact_tool_process_running(script_path, &["python.exe", "pythonw.exe"])
}

const LOCAL_STRUDEL_APP_PATH: &str = "D:\\Teledra\\strudel_app\\app.mjs";
const LOCAL_STRUDEL_PLAYER_PATH: &str = "D:\\Teledra\\strudel_app\\player.py";
const LOCAL_STRUDEL_PYTHON_PATH: &str = "D:\\Teledra\\.venv\\Scripts\\python.exe";
const LEGACY_STRUDEL_DIR: &str = "C:\\Users\\Kaged\\Documents\\Projects\\Tools\\Strudel";
const LEGACY_STRUDEL_RUNNER: &str =
    "C:\\Users\\Kaged\\Documents\\Projects\\Tools\\Strudel\\run.bat";
const STRUDEL_PROCESS_MARKERS: &[&str] = &[
    "strudel_app\\app.mjs play",
    "strudel_app/app.mjs play",
    "D:\\Teledra\\strudel_app\\player.py",
    "D:/Teledra/strudel_app/player.py",
    "localstrudel.StrudelDesktop",
    "strudel_app\\current.strudel",
    "strudel_app/current.strudel",
];
const STRUDEL_PROCESS_NAMES: &[&str] = &[
    "node.exe",
    "python.exe",
    "pythonw.exe",
    "java.exe",
    "javaw.exe",
    "cmd.exe",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrudelLaunchMode {
    CyberneticSynthesizer,
    LegacyJavaSketchpad,
}

impl StrudelLaunchMode {
    fn label(self) -> &'static str {
        match self {
            Self::CyberneticSynthesizer => "Court Cybernetic Synthesizer",
            Self::LegacyJavaSketchpad => "legacy Java Strudel Sketchpad",
        }
    }
}

fn strudel_launch_order(
    cybernetic_synth_available: bool,
    legacy_java_available: bool,
) -> Result<Vec<StrudelLaunchMode>, String> {
    let mut order = Vec::new();
    if cybernetic_synth_available {
        order.push(StrudelLaunchMode::CyberneticSynthesizer);
    }
    if legacy_java_available {
        order.push(StrudelLaunchMode::LegacyJavaSketchpad);
    }
    if order.is_empty() {
        Err(
            "No Strudel playback surface is installed: the local Cybernetic Synthesizer and legacy Java Sketchpad are both unavailable."
                .to_string(),
        )
    } else {
        Ok(order)
    }
}

fn build_strudel_command(mode: StrudelLaunchMode) -> Command {
    let mut command = match mode {
        StrudelLaunchMode::CyberneticSynthesizer => {
            let mut command = Command::new("node.exe");
            // Force Windows default routing on code level so the native synth follows
            // the same audio channel/bus the user has selected in Windows Sound / RØDE UNIFY.
            // This makes panned instruments and all layers route consistently with default audio.
            // Also anchor the Tk window (small) to bottom-right so it doesn't hide Fractus.
            command
                .env("TELEDRA_AUDIO_DEVICE", "default")
                .env("TELEDRA_AUDIO_HOSTAPI", "MME")
                .env("TELEDRA_WINDOW_GEOMETRY", "980x400+50+700")
                .arg(LOCAL_STRUDEL_APP_PATH)
                .arg("play")
                .arg("8")
                .current_dir("D:\\Teledra");
            command
        }
        StrudelLaunchMode::LegacyJavaSketchpad => {
            let mut command = Command::new("cmd.exe");
            command
                .arg("/C")
                .arg("run.bat")
                .arg("D:\\Teledra\\strudel_app\\current.strudel")
                .current_dir(LEGACY_STRUDEL_DIR);
            command
        }
    };
    command.stdout(Stdio::null()).stderr(Stdio::null());
    hide_console(&mut command);
    command
}

fn stop_strudel_tool_processes() -> usize {
    stop_tool_processes(STRUDEL_PROCESS_MARKERS, STRUDEL_PROCESS_NAMES)
}

fn stop_tool_processes(markers: &[&str], allowed_process_names: &[&str]) -> usize {
    if markers.is_empty() || allowed_process_names.is_empty() {
        return 0;
    }
    let markers = markers
        .iter()
        .filter(|m| !m.trim().is_empty())
        .map(|m| format!("'{}'", m.replace('\'', "''").to_lowercase()))
        .collect::<Vec<_>>()
        .join(",");
    let names = allowed_process_names
        .iter()
        .filter(|n| !n.trim().is_empty())
        .map(|n| format!("'{}'", n.replace('\'', "''").to_lowercase()))
        .collect::<Vec<_>>()
        .join(",");
    if markers.is_empty() || names.is_empty() {
        return 0;
    }

    let script = format!(
        "$markers=@({}); $names=@({}); $self=$PID; $count=0; \
         Get-CimInstance Win32_Process | Where-Object {{ \
             $_.ProcessId -ne $self -and $_.CommandLine -and $_.Name -and \
             ($names -contains $_.Name.ToLowerInvariant()) \
         }} | ForEach-Object {{ \
             $cmd=$_.CommandLine.ToLowerInvariant(); $hit=$false; \
             foreach($m in $markers) {{ if($cmd -like \"*$m*\") {{ $hit=$true; break }} }} \
             if($hit) {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue; $count++ }} \
         }}; $count",
        markers, names
    );

    let mut cmd = Command::new("powershell");
    cmd.arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut cmd);
    cmd.output()
        .ok()
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<usize>()
                .ok()
        })
        .unwrap_or(0)
}

fn publish_fractus_payload(payload: &serde_json::Value) -> Result<(), String> {
    let encoded = serde_json::to_vec(payload)
        .map_err(|error| format!("Failed to encode Fractus command: {error}"))?;
    let mut command = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    command
        .arg("-c")
        .arg("import json,sys; from fractus_protocol import write_command_atomic; write_command_atomic(json.load(sys.stdin))")
        .current_dir("D:\\Teledra\\Fractus")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start Fractus command validator: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "Fractus command validator stdin was unavailable".to_string())?
        .write_all(&encoded)
        .map_err(|error| format!("Failed to send Fractus command: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Fractus command validator failed: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = truncate_clean(&String::from_utf8_lossy(&output.stderr), 1_200);
        Err(if stderr.is_empty() {
            "Fractus command was rejected without a diagnostic".to_string()
        } else {
            format!("Fractus command rejected: {stderr}")
        })
    }
}

fn write_fractus_command(args: &[String]) -> Result<(), String> {
    let mut fractal_type = "mandala".to_string();
    let mut iterations = "180".to_string();
    let mut palette = "purple_haze".to_string();
    let mut c_real = "-0.7".to_string();
    let mut c_imag = "0.27015".to_string();
    let mut seed = "1".to_string();

    let mut i = 0;
    while i + 1 < args.len() {
        match args[i].as_str() {
            "--type" => fractal_type = args[i + 1].clone(),
            "--iterations" => iterations = args[i + 1].clone(),
            "--palette" => palette = args[i + 1].clone(),
            "--c-real" => c_real = args[i + 1].clone(),
            "--c-imag" => c_imag = args[i + 1].clone(),
            "--seed" => seed = args[i + 1].clone(),
            _ => {}
        }
        i += 2;
    }

    let payload = serde_json::json!({
        "schema_version": 1,
        "type": fractal_type,
        "iterations": iterations.parse::<u32>().map_err(|_| "invalid iteration payload")?,
        "palette": palette,
        "c_real": c_real.parse::<f64>().map_err(|_| "invalid c-real payload")?,
        "c_imag": c_imag.parse::<f64>().map_err(|_| "invalid c-imag payload")?,
        "seed": seed.parse::<u64>().map_err(|_| "invalid seed payload")?
    });
    publish_fractus_payload(&payload)
}

fn launch_strudel_editor(
    active_gui_process: &Arc<std::sync::Mutex<Option<std::process::Child>>>,
) -> Result<String, String> {
    set_last_creative_artifact("strudel", "strudel_app/current.strudel");
    let stopped_python = stop_tool_processes(
        &[
            "python_music_editor.py",
            "D:\\Teledra\\python_music_editor.py",
            "D:\\Teledra\\music.py",
        ],
        &["python.exe", "pythonw.exe"],
    );
    let mut lock = active_gui_process
        .lock()
        .map_err(|_| "Could not access Strudel editor process lock.".to_string())?;

    // The native player renders current.strudel once; it does not hot-reload.
    // Always replace the previous playback process so a new court score is audible.
    if let Some(mut child) = lock.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = stop_strudel_tool_processes();

    let cybernetic_synth_available = Path::new(LOCAL_STRUDEL_APP_PATH).is_file()
        && Path::new(LOCAL_STRUDEL_PLAYER_PATH).is_file()
        && Path::new(LOCAL_STRUDEL_PYTHON_PATH).is_file();
    let legacy_java_available = Path::new(LEGACY_STRUDEL_RUNNER).is_file();
    let launch_order =
        strudel_launch_order(cybernetic_synth_available, legacy_java_available)?;
    let mut failures = Vec::new();

    for mode in launch_order {
        match build_strudel_command(mode).spawn() {
            Ok(child) => {
                *lock = Some(child);
                return Ok(format!(
                    "{}Launched {} with the validated eight-cycle pattern from strudel_app/current.strudel.",
                    if stopped_python > 0 {
                        "Stopped Python Music Editor so Strudel is the single active music surface. "
                    } else {
                        ""
                    },
                    mode.label()
                ));
            }
            Err(error) => failures.push(format!("{}: {}", mode.label(), error)),
        }
    }

    Err(format!(
        "Failed to launch every available Strudel playback surface: {}",
        failures.join("; ")
    ))
}

fn launch_python_music_editor(
    active_music_process: &Arc<std::sync::Mutex<Option<std::process::Child>>>,
) -> Result<String, String> {
    set_last_creative_artifact("music", "music.py");
    let stopped_strudel = stop_strudel_tool_processes();
    let mut lock = active_music_process
        .lock()
        .map_err(|_| "Could not access Python music editor process lock.".to_string())?;

    if let Some(ref mut child) = *lock {
        match child.try_wait() {
            Ok(None) => {
                return Ok(format!(
                    "{}Updated music.py; Python Music Editor is already running and will reload/run the new composition.",
                    if stopped_strudel > 0 {
                        "Stopped Local Strudel so PyMusic is the single active music surface. "
                    } else {
                        ""
                    }
                ));
            }
            _ => {
                *lock = None;
            }
        }
    }

    if python_tool_process_running("D:\\Teledra\\python_music_editor.py") {
        return Ok(format!(
            "{}Updated music.py; existing Python Music Editor window detected and will reload/run the new composition.",
            if stopped_strudel > 0 {
                "Stopped Local Strudel so PyMusic is the single active music surface. "
            } else {
                ""
            }
        ));
    }

    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("D:\\Teledra\\python_music_editor.py")
        .arg("--run")
        .arg("--x").arg("50")
        .arg("--y").arg("50")
        .arg("--geometry").arg("900x600+50+50")  // music stays left of Fractus
        .current_dir("D:\\Teledra")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_console(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch Python music editor: {}", e))?;

    *lock = Some(child);
    Ok(format!(
        "{}Inserted Organist Python code into music.py and launched Python Music Editor.",
        if stopped_strudel > 0 {
            "Stopped Local Strudel so PyMusic is the single active music surface. "
        } else {
            ""
        }
    ))
}

fn enforce_single_music_surface(
    python_music_code: &mut Option<String>,
    strudel_music_code: &mut Option<String>,
    context: &str,
) -> Option<String> {
    if python_music_code.is_none() || strudel_music_code.is_none() {
        return None;
    }
    let upper = context.to_ascii_uppercase();
    let prefer_strudel = upper.contains("STRUDEL")
        || upper.contains("SKETCHPAD")
        || upper.contains("LIVE-CODE")
        || upper.contains("LIVE CODE")
        || upper.contains("PATTERN CONSOLE");
    if prefer_strudel {
        *python_music_code = None;
        Some(
            "Music surface gate: kept Strudel and discarded simultaneous Python music block."
                .to_string(),
        )
    } else {
        *strudel_music_code = None;
        Some(
            "Music surface gate: kept PyMusic and discarded simultaneous Strudel music block."
                .to_string(),
        )
    }
}

fn parse_fractus_args(spec: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let tokens: Vec<&str> = spec.split_whitespace().collect();
    let mut i = 0;

    while i < tokens.len() {
        let token = tokens[i];
        let (flag, inline_value) = if let Some(eq_idx) = token.find('=') {
            (&token[..eq_idx], Some(&token[eq_idx + 1..]))
        } else {
            (token, None)
        };

        let value = if let Some(v) = inline_value {
            v
        } else {
            i += 1;
            tokens
                .get(i)
                .copied()
                .ok_or_else(|| format!("Missing value for {}.", flag))?
        };

        match flag {
            "--type" => {
                let normalized = value.to_lowercase().replace('-', "_");
                let allowed = [
                    "barnsley_fern",
                    "mandelbrot",
                    "multibrot",
                    "julia",
                    "burning_ship",
                    "tricorn",
                    "newton",
                    "mandala",
                    "lotus_mandala",
                    "star_mandala",
                    "flower_of_life",
                    "radial_weave",
                    "kaleidoscope",
                    "phyllotaxis",
                    "woven_web",
                    "guilloche",
                    "lissajous",
                    "moire",
                    "orbital_lace",
                    "spirograph",
                    "harmonograph",
                    "rose_curve",
                    "string_art",
                    "sierpinski",
                    "koch_snowflake",
                    "dragon_curve",
                    "fractal_tree",
                    "l_system",
                    "truchet",
                    "hex_weave",
                    "op_art",
                    "cellular_automata",
                    "reaction_diffusion",
                    "flow_field",
                    "strange_attractor",
                    "particles",
                ];
                if !allowed.contains(&normalized.as_str()) {
                    return Err(format!("Unsupported Fractus type '{}'.", value));
                }
                args.push("--type".to_string());
                args.push(normalized);
            }
            "--iterations" => {
                let parsed: u32 = value
                    .parse()
                    .map_err(|_| "Fractus iterations must be a number.".to_string())?;
                if !(20..=800).contains(&parsed) {
                    return Err("Fractus iterations must be between 20 and 800.".to_string());
                }
                args.push("--iterations".to_string());
                args.push(parsed.to_string());
            }
            "--palette" => {
                let normalized = value.to_lowercase().replace('-', "_");
                let allowed = [
                    "amethyst",
                    "electric_cyan",
                    "emerald",
                    "ice_fire",
                    "monochrome",
                    "neon_sunset",
                    "pastel",
                    "purple_haze",
                    "rainbow",
                    "solar_gold",
                    "twilight",
                ];
                if !allowed.contains(&normalized.as_str()) {
                    return Err(format!("Unsupported Fractus palette '{}'.", value));
                }
                args.push("--palette".to_string());
                args.push(normalized);
            }
            "--c-real" | "--c-imag" => {
                let parsed: f64 = value
                    .parse()
                    .map_err(|_| format!("{} must be numeric.", flag))?;
                if !parsed.is_finite() || parsed.abs() > 5.0 {
                    return Err(format!("{} must be finite and between -5 and 5.", flag));
                }
                args.push(flag.to_string());
                args.push(parsed.to_string());
            }
            "--seed" => {
                let parsed: u64 = value
                    .parse()
                    .map_err(|_| "Fractus seed must be an unsigned integer.".to_string())?;
                args.push("--seed".to_string());
                args.push(parsed.to_string());
            }
            _ => return Err(format!("Unsupported Fractus argument '{}'.", flag)),
        }

        i += 1;
    }

    if !args.iter().any(|arg| arg == "--type") {
        args.extend(["--type".to_string(), "mandala".to_string()]);
    }
    if !args.iter().any(|arg| arg == "--iterations") {
        args.extend(["--iterations".to_string(), "180".to_string()]);
    }
    if !args.iter().any(|arg| arg == "--palette") {
        args.extend(["--palette".to_string(), "purple_haze".to_string()]);
    }
    if !args.iter().any(|arg| arg == "--seed") {
        args.extend(["--seed".to_string(), variety_seed().to_string()]);
    }

    Ok(args)
}

fn launch_fractus_art(
    spec: &str,
    active_art_process: &Arc<std::sync::Mutex<Option<std::process::Child>>>,
) -> Result<String, String> {
    let args = parse_fractus_args(spec)?;
    write_fractus_command(&args)?;
    set_last_creative_artifact("fractus", &args.join(" "));

    let mut lock = active_art_process
        .lock()
        .map_err(|_| "Could not access Fractus process lock.".to_string())?;

    if let Some(ref mut child) = *lock {
        match child.try_wait() {
            Ok(None) => {
                return Ok(format!(
                    "Updated existing Fractus window with Artist parameters: {}",
                    args.join(" ")
                ));
            }
            _ => {
                *lock = None;
            }
        }
    }

    if python_tool_process_running("D:\\Teledra\\Fractus\\fractus_gui.py") {
        return Ok(format!(
            "Updated Fractus command file for existing Artist window: {}",
            args.join(" ")
        ));
    }

    let mut command = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    command
        .arg("D:\\Teledra\\Fractus\\fractus_gui.py")
        .arg("--x").arg("1000")
        .arg("--y").arg("50")
        .arg("--width").arg("900")
        .arg("--height").arg("650")
        .current_dir("D:\\Teledra")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for arg in &args {
        command.arg(arg);
    }
    hide_console(&mut command);

    let child = command
        .spawn()
        .map_err(|e| format!("Failed to launch Fractus: {}", e))?;

    *lock = Some(child);
    Ok(format!(
        "Fractus launched with Artist parameters: {}",
        args.join(" ")
    ))
}

fn launch_fractus_live_art(
    script: &str,
    source: &str,
    active_fractus_process: &Arc<std::sync::Mutex<Option<std::process::Child>>>,
) -> Result<String, String> {
    let script = script.trim();
    if script.len() < 40 || script.len() > 80_000 {
        return Err("Fractus live code must be between 40 and 80,000 bytes.".to_string());
    }
    let timestamp = mission::current_timestamp_ms();
    let command_id = format!("fractus-{}-{}", timestamp, short_content_hash(script));
    let output_name = format!("{}.png", command_id);
    let payload = serde_json::json!({
        "schema_version": 2,
        "command_id": command_id,
        "sequence": timestamp,
        "action": "apply",
        "source": safe_label(source),
        "script": script,
        "output": {
            "persist": true,
            "path": output_name
        }
    });

    // Validation happens inside the same strict parser used by the GUI, then
    // Python publishes with os.replace so the watcher never consumes a partial
    // command file.
    publish_fractus_payload(&payload)?;
    set_last_creative_artifact("fractus_live", &format!("command_id={command_id}"));

    let mut lock = active_fractus_process
        .lock()
        .map_err(|_| "Could not access Fractus process lock.".to_string())?;
    let already_running = match lock.as_mut() {
        Some(child) => matches!(child.try_wait(), Ok(None)),
        None => false,
    } || python_tool_process_running("D:\\Teledra\\Fractus\\fractus_gui.py");
    if already_running {
        let status = wait_for_fractus_status(&command_id, Duration::from_secs(2))?;
        return Ok(format!("Fractus v2 command {}: {}", command_id, status));
    }
    *lock = None;

    let script_path = format!("D:\\Teledra\\Fractus\\output\\{}.fract", command_id);
    std::fs::create_dir_all("D:\\Teledra\\Fractus\\output")
        .map_err(|error| format!("Failed to prepare Fractus output: {error}"))?;
    std::fs::write(&script_path, script)
        .map_err(|error| format!("Failed to preserve Fractus live code: {error}"))?;
    let mut command = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    command
        .arg("D:\\Teledra\\Fractus\\fractus_gui.py")
        .arg("--script")
        .arg(&script_path)
        .arg("--output")
        .arg(format!("D:\\Teledra\\Fractus\\output\\{}", output_name))
        .arg("--play")
        .arg("--x").arg("1000")
        .arg("--y").arg("50")
        .arg("--width").arg("900")
        .arg("--height").arg("650")
        .current_dir("D:\\Teledra")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let child = command
        .spawn()
        .map_err(|error| format!("Failed to launch Fractus v2: {error}"))?;
    *lock = Some(child);
    // The studio captures the current mailbox token during construction. A
    // second atomic publish after the native window initializes guarantees an
    // acknowledgement for first launch as well as later live edits.
    std::thread::sleep(Duration::from_millis(450));
    publish_fractus_payload(&payload)?;
    let status = wait_for_fractus_status(&command_id, Duration::from_secs(2))?;
    Ok(format!(
        "Fractus v2 launched with live-code command {}: {}",
        command_id, status
    ))
}

fn wait_for_fractus_status(command_id: &str, timeout: Duration) -> Result<String, String> {
    let started = std::time::Instant::now();
    let mut last_state = "queued; awaiting renderer acknowledgement".to_string();
    while started.elapsed() < timeout {
        if let Ok(raw) = std::fs::read_to_string("D:\\Teledra\\Fractus\\fractus_status.json") {
            if let Ok(status) = serde_json::from_str::<serde_json::Value>(&raw) {
                if status.get("command_id").and_then(|value| value.as_str()) == Some(command_id) {
                    let state = status
                        .get("state")
                        .and_then(|value| value.as_str())
                        .unwrap_or("unknown");
                    let detail = status
                        .get("detail")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    if state == "rejected" {
                        return Err(format!(
                            "Fractus renderer rejected command {}: {}",
                            command_id, detail
                        ));
                    }
                    if state == "completed" {
                        let render_hash = status
                            .get("render_hash")
                            .and_then(|value| value.as_str())
                            .unwrap_or("unreported");
                        let output = status
                            .get("output_path")
                            .and_then(|value| value.as_str())
                            .unwrap_or("preview only");
                        return Ok(format!(
                            "completed and verified (render {}, output {})",
                            render_hash, output
                        ));
                    }
                    last_state = if detail.is_empty() {
                        state.to_string()
                    } else {
                        format!("{} ({})", state, detail)
                    };
                }
            }
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    Ok(last_state)
}

fn run_phase_a_self_test() -> Result<serde_json::Value, String> {
    let original_memory = std::fs::read(TASTE_DESIRE_PATH).ok();
    let result = (|| {
        let like = serde_json::json!({
            "type": "like",
            "subject": "dungeon synth",
            "why": "simulated viewer preferred the atmospheric version",
            "strength": 0.8,
            "source": "test:self-test"
        });
        let desire = serde_json::json!({
            "type": "desire",
            "want": "build an atmospheric pixel-world score",
            "kind": "immediate",
            "strength": 0.65,
            "source": "test:self-test"
        });
        let mut moments = Vec::new();
        moments.push(apply_taste_desire_event(&like).map_err(|e| e.to_string())?);
        for turn in 1..=3 {
            let summary = apply_taste_desire_event(&desire).map_err(|e| e.to_string())?;
            log_test_moment("self_test_sim_chat", &format!("turn {}: {}", turn, summary));
            moments.push(summary);
        }
        let context = taste_desire_prompt_context();
        let memory = load_taste_desire_memory();
        let retained_like = memory
            .get("likes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|entry| {
                entry.get("subject").and_then(serde_json::Value::as_str) == Some("dungeon synth")
            });
        let promoted_desire = memory
            .get("desires")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| {
                entry.get("want").and_then(serde_json::Value::as_str)
                    == Some("build an atmospheric pixel-world score")
            })
            .is_some_and(|entry| {
                entry.get("kind").and_then(serde_json::Value::as_str) == Some("persistent")
                    && entry
                        .get("recurrence")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0)
                        >= DESIRE_PROMOTE_AFTER
            });
        if !retained_like || !promoted_desire {
            return Err(format!(
                "taste/desire continuity failed (like={}, promoted_desire={}): {}",
                retained_like, promoted_desire, context
            ));
        }
        validate_python_music_code(&default_python_music_code())?;
        let verify = "default generated composition passed strict verify+learn loop";
        log_test_moment("self_test_music_verify", verify);
        Ok(serde_json::json!({
            "ok": true,
            "off_air": true,
            "moments": moments,
            "context": context,
            "music_verify": verify
        }))
    })();
    match original_memory {
        Some(bytes) => {
            let _ = std::fs::write(TASTE_DESIRE_PATH, bytes);
        }
        None => {
            let _ = std::fs::remove_file(TASTE_DESIRE_PATH);
        }
    }
    result
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Always run from the project root so all relative paths resolve correctly,
    // regardless of whether the binary is launched from Explorer, a shortcut, or a terminal.
    let root = "D:\\Teledra";
    if std::env::set_current_dir(root).is_ok() {
        println!("[startup] CWD set to {}", root);
    } else {
        // Fallback attempt using exe location (handles running target/release/teledra.exe directly)
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent().and_then(|p| if p.ends_with("release") || p.ends_with("debug") { p.parent().and_then(|pp| pp.parent()) } else { Some(p) }) {
                let _ = std::env::set_current_dir(dir);
            }
        }
        println!("[startup] CWD may not be project root; using absolute paths for key files like shared_stories.");
    }

    if std::env::args().any(|arg| arg == "--phase-a-self-test") {
        match run_phase_a_self_test() {
            Ok(report) => {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
    }

    // Purge orphans from previous runs BEFORE spawning anything, so stale
    // children cannot hold file locks or fight over audio/render resources.
    let purged_processes = purge_stale_kingdom_processes();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Core variables
    let _ears = AudioCortex::new();
    let mut somatic = SomaticBridge::new();
    let mut voice = VoiceEngine::new("energetic");
    let brain = Brain::new();
    // Helpful when you say "I'm using qwen2.5 from a desktop shortcut"
    let config_path = std::env::var("TELEDRA_CONFIG").unwrap_or_else(|_| "config.json".to_string());
    println!("[startup] Brain config file: {} (Ollama/local qwen2.5 or llama etc. expected for NightDesk)", config_path);
    let mission_store = MissionStore::new(
        "knowledge/active_mission.json",
        "knowledge/mission_lifecycle.jsonl",
    );
    let (mut active_mission, mission_recovery_note) = if mission_store.snapshot_path().exists() {
        match mission_store.load_and_recover() {
            Ok((mission, report)) => {
                let note = if report.requeued.is_empty() && report.terminal.is_empty() {
                    format!(
                        "Recovered mission '{}' with no interrupted tasks.",
                        mission.id
                    )
                } else {
                    format!(
                        "Recovered mission '{}': requeued [{}], terminal [{}].",
                        mission.id,
                        report.requeued.join(", "),
                        report.terminal.join(", ")
                    )
                };
                (Some(mission), Some(note))
            }
            Err(error) => (
                None,
                Some(format!("Mission recovery failed safely: {}", error)),
            ),
        }
    } else {
        (None, None)
    };
    let mut current_mode = ForceMode::Normal;
    let mut babble_think_in_progress = false;
    let mut study_in_progress = false;
    let mut stream_chat_queue: std::collections::VecDeque<(String, String)> =
        std::collections::VecDeque::new();
    let mut general_speech_queue: std::collections::VecDeque<(
        CourtRole,
        String,
        ForceMode,
        String,
        bool,
    )> = std::collections::VecDeque::new();
    let mut court_delegations: std::collections::VecDeque<CourtDelegation> =
        std::collections::VecDeque::new();
    let mut active_mission_task: Option<(String, CourtRole)> = None;
    if let Some(mission) = active_mission.as_ref() {
        for task_id in mission.ready_task_ids() {
            if let Some(task) = mission.task(&task_id) {
                if let Some(role) = role_from_name(&task.role) {
                    court_delegations.push_back(CourtDelegation {
                        role,
                        instruction: task.objective.clone(),
                        mission_task_id: Some(task.id.clone()),
                    });
                }
            }
        }
    }
    let mut is_court_sequence_running = false;
    // Churn brake: after a sprint produces no artifact, skip the next few
    // study-triggered sprints instead of looping sprint->fail->study->sprint.
    let mut sprint_cooldown: u32 = 0;
    // Consecutive sprints that produced no executable artifact; escalates the
    // brake instead of letting failure-narration loop forever.
    let mut no_artifact_streak: u32 = 0;
    let mut current_monologue_topic: Option<String> = None;
    let mut monologue_topic_turn: u32 = 0;
    // Consecutive Queen turns with zero successful delegations; used to force
    // a court summons into monologue prompts so she never lectures alone forever.
    let mut queen_turns_without_delegation: u32 = 0;

    // /lock: hold one topic for a long-form, podcast-style monologue. Released by
    // /unlock, by losing chat interest, or when she signals the topic exhausted.
    let mut locked_topic: Option<String> = None;
    let mut lock_idle_turns_without_chat: u32 = 0;

    // Shared active playback state to terminate overlapping speaking processes
    let active_playback: Arc<std::sync::Mutex<Option<voice::PlaybackController>>> =
        Arc::new(std::sync::Mutex::new(None));

    // Track active background music child process
    let active_music_process: Arc<std::sync::Mutex<Option<std::process::Child>>> =
        Arc::new(std::sync::Mutex::new(None));
    let active_art_process: Arc<std::sync::Mutex<Option<std::process::Child>>> =
        Arc::new(std::sync::Mutex::new(None));
    let active_fractus_process: Arc<std::sync::Mutex<Option<std::process::Child>>> =
        Arc::new(std::sync::Mutex::new(None));
    let active_gui_process: Arc<std::sync::Mutex<Option<std::process::Child>>> =
        Arc::new(std::sync::Mutex::new(None));
    let active_restream_process: Arc<std::sync::Mutex<Option<tokio::process::Child>>> =
        Arc::new(std::sync::Mutex::new(None));

    // Load left-panel background image (portrait art rendered as half-blocks)
    let bg_image: Option<DynamicImage> = image::open("assets/teledra_bg (2).png")
        .or_else(|_| image::open("D:\\Teledra\\assets\\teledra_bg (2).png"))
        .or_else(|_| image::open("assets/teledra_bg.png"))
        .or_else(|_| image::open("D:\\Teledra\\assets\\teledra_bg.png"))
        .ok();
    let mut bg_pixel_cache: Option<PixCache> = None;

    // Start background Somatic Bridge
    let _ = somatic.start();

    // UI state
    let mut focus = FocusField::Chat;
    let mut chat_input = String::new();
    let mut youtube_input = String::new();
    let mut exiting_to_sleep = false;
    let mut exit_timer: Option<std::time::Instant> = None;
    let mut chat_scroll = 0u16;
    let mut private_scroll = 0u16;
    let mut user_has_scrolled_up = false;
    let mut music_enabled = true;
    let mut suggestion_count = count_new_suggestions();
    let mut workshop_count = count_workshop_experiments();
    let mut night_desk_enabled = false;
    let mut night_desk_cycles = 0u64;
    let mut night_desk_cycle_pending = false;
    // Off-air overlay for Curiosity/Desire development. It deliberately does
    // not add a ForceMode variant, preserving every existing mode match.
    let mut test_mode_enabled = false;
    let mut test_lurker_silence = true;
    let mut test_knobs = TestHarnessKnobs::default();
    let mut test_cast = "Queen + Organist".to_string();
    let mut test_scene = "off-air music laboratory".to_string();

    // Game Co-Pilot mode state.
    let mut copilot_game: Option<String> = None;
    let mut copilot_tick_pending = false;
    let mut copilot_turn: u64 = 0;
    let mut copilot_screen_note: Option<String> = None;
    let mut copilot_mic_enabled = false;
    let mut copilot_mic_child: Option<tokio::process::Child> = None;

    // Music cadence: the autonomous tune evolves at most every few minutes
    // (Ctrl+U forces an immediate composer pass), so it deepens instead of churning.
    let mut last_music_change: Option<std::time::Instant> = None;
    let mut force_music_next = false;

    let mut chat_history: Vec<(String, String)> = vec![
        ("System".to_string(), "Welcome to the Teledra Cybernetic Interface. Press Esc to exit.".to_string()),
        ("System".to_string(), "Commands: /mission | /missioncancel | /dashboard | /test | /simchat <line> | /testtick | /testmusic | /nightdesk | /study | /innovate | /wizard | /music | /pymusic | /reflect | /diplomat | /proposals | /approve <id> (or 'all') | /reject <id> | /workshop | /sketchpad | /fractus | /art | /lock <topic> | /unlock | /work | /links".to_string()),
    ];
    let mut private_events: Vec<(String, String)> = vec![
        ("Backstage".to_string(), "Private event trace online. NightDesk, tool routing, research, and status transitions appear here.".to_string()),
        ("Diplomat".to_string(), "Envoy monitor armed. Diplomat dispatches, online leads, and outreach evidence will be labeled here.".to_string()),
    ];
    if !purged_processes.is_empty() {
        let msg = format!(
            "Purged {} stale court process(es) from a previous run: {}",
            purged_processes.len(),
            purged_processes.join(", ")
        );
        chat_history.push(("System".to_string(), msg.clone()));
        private_events.push(("Status".to_string(), msg));
    }
    if let Some(note) = mission_recovery_note {
        chat_history.push(("System".to_string(), note.clone()));
        private_events.push(("Mission".to_string(), note));
    }
    let mut status_msg = "Ready".to_string();

    // Channel for background events
    let (tx, mut rx) = mpsc::channel(10);
    if !court_delegations.is_empty() {
        let _ = tx.try_send(AppEvent::SpeechComplete);
    }

    {
        let tx_wizard = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(8)).await;
            loop {
                let result = tokio::task::spawn_blocking(import_cloud_wizard_reports)
                    .await
                    .map_err(|e| format!("Wizard import task failed: {}", e))
                    .and_then(|inner| inner);
                match result {
                    Ok((status, summaries)) => {
                        let _ = tx_wizard
                            .send(AppEvent::WizardReports {
                                status,
                                summaries,
                                quiet: true,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = tx_wizard
                            .send(AppEvent::SystemLog(format!(
                                "Wizard auto-import failed: {}",
                                e
                            )))
                            .await;
                    }
                }
                tokio::time::sleep(Duration::from_secs(WIZARD_REPORT_POLL_SECS)).await;
            }
        });
    }

    // Idle watchdog heartbeat: arm the self-rescheduling chain that re-pulses
    // auto-babble if the stream ever goes silent in Babble/Streamer mode.
    {
        let tx_watchdog = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(IDLE_WATCHDOG_SECS)).await;
            let _ = tx_watchdog.send(AppEvent::IdleWatchdog).await;
        });
    }

    // Shared reference for async tasks
    let brain_cell = Arc::new(RwLock::new(brain));

    // Research tasks are durable mission work but are not court roles. Recover
    // and relaunch them explicitly instead of silently dropping them from the
    // role-based delegation queue after a restart.
    let recovered_research_tasks: Vec<(String, String, String)> = active_mission
        .as_ref()
        .map(|mission| {
            mission
                .ready_task_ids()
                .into_iter()
                .filter_map(|task_id| {
                    mission.task(&task_id).and_then(|task| {
                        task.role
                            .eq_ignore_ascii_case("research")
                            .then(|| (mission.id.clone(), task.id.clone(), task.objective.clone()))
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    for (mission_id, task_id, query) in recovered_research_tasks {
        let mut launch = false;
        if let Some(mission) = active_mission.as_mut() {
            match mission.start_task(&task_id) {
                Ok(transition) => {
                    if let Err(error) = mission_store.commit_transition(mission, &transition) {
                        record_recursive_failure(
                            "research_task_recovery_commit_failed",
                            &error.to_string(),
                        );
                    } else {
                        launch = true;
                    }
                }
                Err(error) => record_recursive_failure(
                    "research_task_recovery_start_failed",
                    &error.to_string(),
                ),
            }
        }
        if launch {
            study_in_progress = true;
            let tx_research = tx.clone();
            let brain_research = Arc::clone(&brain_cell);
            tokio::spawn(async move {
                run_study_cycle(
                    brain_research,
                    tx_research,
                    Some(query),
                    Some(task_id),
                    Some(mission_id),
                )
                .await;
            });
        }
    }

    // BRAIN REACHABILITY CHECK: ping the configured model endpoint once at
    // startup so a forgotten Ollama shows up as a clear banner instead of
    // silent think failures.
    {
        let tx_brain_check = tx.clone();
        tokio::spawn(async move {
            let api_url = std::fs::read_to_string(
                std::env::var("TELEDRA_CONFIG").unwrap_or_else(|_| "config.json".to_string()),
            )
            .ok()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .and_then(|v| {
                v.get("api_url")
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
            if api_url.is_empty() {
                return;
            }
            // Probe scheme://host:port as a cheap liveness target.
            let base = {
                let after_scheme = api_url.find("://").map(|i| i + 3).unwrap_or(0);
                match api_url[after_scheme..].find('/') {
                    Some(rel) => api_url[..after_scheme + rel].to_string(),
                    None => api_url.clone(),
                }
            };
            let client = reqwest::Client::new();
            let reachable = client
                .get(&base)
                .timeout(Duration::from_secs(4))
                .send()
                .await
                .is_ok();
            if reachable {
                let _ = tx_brain_check
                    .send(AppEvent::SystemLog(format!(
                        "Royal mind online at {}.",
                        base
                    )))
                    .await;
            } else {
                let _ = tx_brain_check
                    .send(AppEvent::Error(format!(
                        "The royal mind is UNREACHABLE at {} -- is Ollama running? Start the model server; the court retries every 2 minutes and will wake on its own once the mind returns.",
                        base
                    )))
                    .await;
            }
        });
    }

    // Auto-load saved Restream token on startup if it exists
    if let Ok(token) =
        std::fs::read_to_string("config/restream_token.txt").map(|s| s.trim().to_string())
    {
        if !token.is_empty() {
            current_mode = ForceMode::Streamer;
            night_desk_enabled = true;
            // Start the night-desk heartbeat. Without this kick the cycles --
            // and therefore the every-3rd-cycle Diplomat dispatch -- never ran
            // in auto-started streamer mode.
            night_desk_cycle_pending = true;
            {
                let tx_kick = tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(20)).await;
                    let _ = tx_kick.send(AppEvent::NightDeskCycle).await;
                });
            }
            voice.set_voice("custom");

            let python_exe = "D:\\Teledra\\.venv\\Scripts\\python.exe";
            let script_path = "D:\\Teledra\\restream_listener.py";
            let mut listen_cmd = tokio::process::Command::new(python_exe);
            listen_cmd
                .arg(script_path)
                .arg(&token)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            hide_console_tokio(&mut listen_cmd);
            let child = listen_cmd.spawn();

            match child {
                Ok(mut c) => {
                    let stdout = c.stdout.take().expect("Failed to open stdout");
                    let stderr = c.stderr.take().expect("Failed to open stderr");

                    if let Ok(mut lock) = active_restream_process.lock() {
                        *lock = Some(c);
                    }

                    let _ = log_system_activity(&format!(
                        "Streamer Mode auto-activated with saved token prefix: {}...",
                        &token[..6.min(token.len())]
                    ));
                    push_private_event(
                        &mut private_events,
                        "Restream",
                        &format!(
                            "Streamer Mode auto-activated with saved token prefix: {}...",
                            &token[..6.min(token.len())]
                        ),
                    );

                    let tx_ws = tx.clone();
                    tokio::spawn(async move {
                        use tokio::io::{AsyncBufReadExt, BufReader};
                        let mut reader = BufReader::new(stdout).lines();
                        while let Ok(Some(line)) = reader.next_line().await {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                                if let (Some(author), Some(text)) = (
                                    parsed.get("author").and_then(|v| v.as_str()),
                                    parsed.get("text").and_then(|v| v.as_str()),
                                ) {
                                    let _ = tx_ws
                                        .send(AppEvent::RestreamMessage {
                                            author: author.to_string(),
                                            text: text.to_string(),
                                        })
                                        .await;
                                }
                            }
                        }
                    });

                    let tx_err = tx.clone();
                    tokio::spawn(async move {
                        use tokio::io::{AsyncBufReadExt, BufReader};
                        let mut reader = BufReader::new(stderr).lines();
                        while let Ok(Some(line)) = reader.next_line().await {
                            let _ = tx_err
                                .send(AppEvent::SystemLog(format!("Restream listener: {}", line)))
                                .await;
                        }
                    });
                }
                Err(e) => {
                    let msg = format!("Failed to auto-spawn Restream listener: {}", e);
                    push_private_event(&mut private_events, "Restream", &msg);
                    chat_history.push(("System".to_string(), msg));
                }
            }
        }
    }

    // Spawn Background Autonomous Study Loop (runs every 3 minutes)
    let tx_study = tx.clone();
    let brain_study = Arc::clone(&brain_cell);
    tokio::spawn(async move {
        // Init wait before first autonomous cycle
        tokio::time::sleep(Duration::from_secs(STUDY_LOOP_INITIAL_DELAY_SECS)).await;
        loop {
            run_study_cycle(Arc::clone(&brain_study), tx_study.clone(), None, None, None).await;
            tokio::time::sleep(Duration::from_secs(STUDY_LOOP_INTERVAL_SECS)).await;
        }
    });

    // Background dream consolidation: previously dream.py only ran on /sleep,
    // so chat_logs.jsonl bloated unbounded during long streams and memory was
    // never consolidated mid-session. Now: every 30 minutes, if the chat log
    // has grown past the threshold, run a dreaming cycle in the background.
    let tx_dream = tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1800)).await;
            let log_lines = std::fs::read_to_string("knowledge/chat_logs.jsonl")
                .map(|c| c.lines().count())
                .unwrap_or(0);
            if log_lines < 300 {
                continue;
            }
            let _ = tx_dream
                .send(AppEvent::SystemLog(format!(
                    "Dream cycle started in the background ({} chat log lines to consolidate).",
                    log_lines
                )))
                .await;
            let mut dream_cmd =
                tokio::process::Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
            dream_cmd
                .arg("D:\\Teledra\\dream.py")
                .current_dir("D:\\Teledra")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            hide_console_tokio(&mut dream_cmd);
            match dream_cmd.spawn() {
                Ok(mut child) => {
                    let _ = child.wait().await;
                    let _ = tx_dream
                        .send(AppEvent::SystemLog(
                            "Dream cycle complete; memories consolidated and chat log archived."
                                .to_string(),
                        ))
                        .await;
                }
                Err(e) => {
                    let _ = tx_dream
                        .send(AppEvent::SystemLog(format!(
                            "Dream cycle failed to start: {}",
                            e
                        )))
                        .await;
                }
            }
        }
    });

    // Spawn Background Keyboard Event Listener Task
    // Uses timing-based paste detection: when characters arrive within 5ms of each
    // other, they are accumulated into a paste buffer. After a 30ms gap with no new
    // input, the buffer is flushed as a single Paste event. This prevents multi-line
    // pastes from being split into separate messages.
    let tx_keys = tx.clone();
    tokio::spawn(async move {
        tokio::task::spawn_blocking(move || {
            let mut paste_buf = String::new();
            let paste_flush_ms = 30u128; // flush after 30ms of silence

            loop {
                if !paste_buf.is_empty() {
                    // We have accumulated chars — check for more input quickly
                    if let Ok(true) = event::poll(Duration::from_millis(paste_flush_ms as u64)) {
                        match event::read() {
                            Ok(Event::Key(key)) => {
                                if key.kind == event::KeyEventKind::Release {
                                    continue;
                                }
                                match key.code {
                                    KeyCode::Char(c) => paste_buf.push(c),
                                    KeyCode::Enter => paste_buf.push(' '),
                                    _ => {} // ignore modifiers, arrows, etc during paste
                                }
                                continue;
                            }
                            Ok(Event::Paste(text)) => {
                                paste_buf.push_str(&text.replace('\r', " ").replace('\n', " "));
                                continue;
                            }
                            _ => {}
                        }
                    }
                    // Timeout — no more rapid input, flush the paste buffer
                    let text = std::mem::take(&mut paste_buf);
                    let _ = tx_keys.blocking_send(AppEvent::Paste(text));
                } else {
                    // Normal mode — wait for first event
                    if let Ok(true) = event::poll(Duration::from_millis(5)) {
                        match event::read() {
                            Ok(Event::Key(key)) => {
                                if key.kind == event::KeyEventKind::Release {
                                    continue;
                                }
                                // Check if another key arrives very quickly (paste detection)
                                if matches!(key.code, KeyCode::Char(_) | KeyCode::Enter) {
                                    if let Ok(true) = event::poll(Duration::from_millis(15)) {
                                        // Another event is immediately available — likely a paste
                                        match key.code {
                                            KeyCode::Char(c) => paste_buf.push(c),
                                            KeyCode::Enter => paste_buf.push(' '),
                                            _ => {}
                                        }
                                        continue;
                                    }
                                }
                                // Normal single keypress — forward immediately
                                let _ = tx_keys.blocking_send(AppEvent::KeyPress(key));
                            }
                            Ok(Event::Paste(text)) => {
                                let _ = tx_keys.blocking_send(AppEvent::Paste(text));
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
        .await
        .ok();
    });

    // Cleanup resources at program exit
    let mut run_loop = true;
    while run_loop {
        // Read current somatic telemetry
        let somatic_state = somatic.get_state();

        // Pre-compute image halfblocks BEFORE terminal.draw() so mutations
        // to bg_pixel_cache survive across frames (FnOnce closures cannot
        // persist mutations made to outer variables they capture by ref).
        let bg_lines: Option<Vec<Line<'static>>> = if let Some(ref img) = bg_image {
            if let Ok(ts) = terminal.size() {
                // Mirror ratatui layout exactly:
                //   margin(1)      => outer area is ts shrunk by 1 on each side
                //   chunks[0]      => Length(3) header
                //   chunks[1]      => Min(10) content  (= outer_h - 3 - 3)
                //   chunks[2]      => Length(3) input
                //   content_chunks[0] => Percentage(35) of content width
                //   left_chunks[0] => Min(6)     image  (= content_h - 12 - 6)
                //   left_chunks[1] => Length(12) protocol
                //   left_chunks[2] => Length(6)  telemetry
                let outer_h = ts.height.saturating_sub(2); // margin top+bottom
                let outer_w = ts.width.saturating_sub(2); // margin left+right
                let content_h = outer_h.saturating_sub(6); // header(3) + input(3)
                let left_w = (outer_w as u32 * 35 / 100) as u16;
                let img_h = content_h.saturating_sub(15); // protocol(10) + telemetry(5)
                let img_w = left_w;
                if img_w > 2 && img_h > 2 {
                    let need = bg_pixel_cache
                        .as_ref()
                        .map(|(cw, ch, _)| *cw != img_w || *ch != img_h)
                        .unwrap_or(true);
                    if need {
                        bg_pixel_cache = Some(build_pixel_cache(img, img_w, img_h));
                    }
                    bg_pixel_cache.as_ref().map(|c| pixel_cache_to_lines(c))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Draw TUI
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Length(3), // Header
                        Constraint::Min(10),   // Content panels
                        Constraint::Length(3), // Input boxes
                    ]
                    .as_ref(),
                )
                .split(f.size());

            // 1. Header Block
            let header_p = Paragraph::new(vec![Line::from(vec![
                Span::styled(
                    "SIBELIUM COGNITIVE INTERFACE v0.1.0 // HOST: TELEDRA",
                    Style::default()
                        .fg(Color::Rgb(0, 255, 66))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" | STATUS: ", Style::default().fg(Color::Rgb(147, 51, 234))),
                Span::styled(
                    status_msg.to_uppercase(),
                    Style::default().fg(Color::Rgb(0, 255, 66)),
                ),
                Span::styled(
                    " | SUGGESTIONS: ",
                    Style::default().fg(Color::Rgb(147, 51, 234)),
                ),
                Span::styled(
                    suggestion_count.to_string(),
                    if suggestion_count > 0 {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(128, 128, 128))
                    },
                ),
                Span::styled(" | NIGHT: ", Style::default().fg(Color::Rgb(147, 51, 234))),
                Span::styled(
                    if night_desk_enabled { "ON" } else { "OFF" },
                    if night_desk_enabled {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(128, 128, 128))
                    },
                ),
            ])])
            .alignment(ratatui::layout::Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(147, 51, 234))),
            );
            f.render_widget(header_p, chunks[0]);

            // 2. Split content panel into Left (Control & Telemetry) and Right (Chat Log)
            let content_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(35), Constraint::Percentage(65)].as_ref())
                .split(chunks[1]);

            // Left Panels — image background on top, then protocol + telemetry below
            let left_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Min(6),     // [0] Background art (half-block image)
                        Constraint::Length(10), // [1] Cognitive protocol status
                        Constraint::Length(5),  // [2] Somatic telemetry
                    ]
                    .as_ref(),
                )
                .split(content_chunks[0]);

            // ── Render background image into left_chunks[0] ────────────────────────
            if let Some(ref lines) = bg_lines {
                f.render_widget(
                    Paragraph::new(lines.clone()).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Rgb(80, 0, 120))),
                    ),
                    left_chunks[0],
                );
            } else {
                // Fallback: dark placeholder if image not found or too small
                f.render_widget(
                    Paragraph::new("")
                        .style(Style::default().bg(Color::Rgb(10, 0, 20)))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_style(Style::default().fg(Color::Rgb(80, 0, 120))),
                        ),
                    left_chunks[0],
                );
            }

            // ── Protocol override status block ─────────────────────────────────────
            let active_voice = voice.voice_name();
            let dominant_emotion = match current_mode {
                ForceMode::Normal => "Proud / Imperial",
                ForceMode::Comedic => "Teasing / Playful",
                ForceMode::Empathetic => "Protective / Gentle",
                ForceMode::DarkComedic => "Cynical / Deadpan",
                ForceMode::Babble => "Excited / Curious",
                ForceMode::Streamer => "Regal thoughts / Live broadcast",
                ForceMode::CoPilot => "Game Co-Pilot / Couch companion",
            };
            let music_status_str = if music_enabled { "ON" } else { "OFF" };
            let music_status_style = if music_enabled {
                Style::default().fg(Color::Rgb(0, 255, 66))
            } else {
                Style::default().fg(Color::Red)
            };

            let override_text = vec![
                Line::from(vec![
                    Span::styled("Behavior  ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(
                        format!("{:?}", current_mode),
                        Style::default()
                            .fg(Color::Rgb(147, 51, 234))
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Emotion   ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(
                        dominant_emotion,
                        Style::default().fg(Color::Rgb(0, 255, 66)),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Voice     ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(active_voice, Style::default().fg(Color::Rgb(147, 51, 234))),
                ]),
                Line::from(vec![
                    Span::styled("Music     ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(
                        music_status_str,
                        music_status_style.add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Proposals ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(
                        if suggestion_count > 0 {
                            format!("{} PENDING", suggestion_count)
                        } else {
                            "CLEAR".to_string()
                        },
                        if suggestion_count > 0 {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Rgb(80, 80, 80))
                        },
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Workshop  ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(
                        format!("{} EXP", workshop_count),
                        Style::default().fg(Color::Rgb(147, 51, 234)),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("NightDesk ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(
                        if night_desk_enabled {
                            format!("ACTIVE ({})", night_desk_cycles)
                        } else {
                            "OFF".to_string()
                        },
                        if night_desk_enabled {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Rgb(80, 80, 80))
                        },
                    ),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Tab:Mode  Ctrl+M:Music  Ctrl+L/K/E/P:Like/Dislike/Expand/Playlist",
                    Style::default().fg(Color::Rgb(60, 60, 60)),
                )),
            ];
            let protocol_p = Paragraph::new(override_text).block(
                Block::default()
                    .title(" PROTOCOLS ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(80, 0, 120))),
            );
            f.render_widget(protocol_p, left_chunks[1]);

            // ── Somatic telemetry block ────────────────────────────────────────────
            let shoulder_dev = somatic_state.shoulder_asymmetry.unwrap_or(0.0);
            let face_visible = if somatic_state.face_detected {
                "DETECTED"
            } else {
                "ABSENT"
            };
            let face_style = if somatic_state.face_detected {
                Style::default().fg(Color::Rgb(0, 255, 66))
            } else {
                Style::default().fg(Color::Red)
            };
            let posture_msg = if shoulder_dev > 0.04 {
                "SLOUCHING"
            } else {
                "EXCELLENT"
            };
            let posture_style = if shoulder_dev > 0.04 {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Rgb(0, 255, 66))
            };
            let bar_length = (shoulder_dev * 200.0).min(16.0) as usize;
            let bar_fill = "█".repeat(bar_length);
            let bar_empty = "░".repeat(16 - bar_length);

            let telemetry_text = vec![
                Line::from(vec![
                    Span::styled("Face    ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(face_visible, face_style),
                ]),
                Line::from(vec![
                    Span::styled("Posture ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(posture_msg, posture_style),
                ]),
                Line::from(vec![
                    Span::styled("Asym    ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(
                        format!("{:.3}", shoulder_dev),
                        Style::default().fg(Color::Rgb(147, 51, 234)),
                    ),
                    Span::styled(
                        format!(" [{}{}]", bar_fill, bar_empty),
                        Style::default().fg(Color::Rgb(80, 0, 120)),
                    ),
                ]),
            ];
            let telemetry_p = Paragraph::new(telemetry_text).block(
                Block::default()
                    .title(" SOMATIC ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(80, 0, 120))),
            );
            f.render_widget(telemetry_p, left_chunks[2]);

            // Right Panels: public court log above, private machinery trace below.
            let right_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(12), Constraint::Length(16)].as_ref())
                .split(content_chunks[1]);

            let mut chat_lines = Vec::new();
            for (sender, msg) in &chat_history {
                let prefix = format!("[{}] ", sender);
                let color = match sender.as_str() {
                    "System" => Color::Rgb(128, 128, 128),
                    "You" => Color::Rgb(0, 255, 66),
                    "Teledra" | "Queen" => Color::Rgb(255, 215, 0), // Gold
                    "Organist" => Color::Rgb(255, 0, 255),          // Magenta
                    "Archivist" => Color::Rgb(0, 255, 255),         // Cyan
                    "Alchemist" => Color::Rgb(0, 255, 0),           // Bright Green
                    "Orator" => Color::Rgb(255, 69, 0),             // Red-Orange
                    "Scribe" => Color::Rgb(169, 169, 169),          // Gray
                    "Artist" => Color::Rgb(255, 165, 0),            // Orange
                    _ => Color::Rgb(147, 51, 234),                  // Default purple
                };

                chat_lines.push(Line::from(vec![
                    Span::styled(
                        prefix,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(msg, Style::default().fg(Color::Rgb(0, 255, 66))),
                ]));
                chat_lines.push(Line::from(""));
            }

            let total_scroll = calculate_scroll_to_bottom(
                &chat_history,
                right_chunks[0].width,
                right_chunks[0].height,
            );
            if !user_has_scrolled_up {
                chat_scroll = total_scroll;
            } else {
                chat_scroll = chat_scroll.min(total_scroll);
            }

            let title_text = if user_has_scrolled_up {
                format!(
                    " NEURAL COGNITIVE CHANNEL [SCROLL: {}/{}] (Shift+Up/Down, PageUp/Down) ",
                    chat_scroll, total_scroll
                )
            } else {
                " NEURAL COGNITIVE CHANNEL (Shift+Up/Down, PageUp/Down to Scroll) ".to_string()
            };

            let chat_p = Paragraph::new(chat_lines)
                .wrap(Wrap { trim: true })
                .scroll((chat_scroll, 0))
                .block(
                    Block::default()
                        .title(title_text)
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Rgb(147, 51, 234))),
                );

            f.render_widget(chat_p, right_chunks[0]);

            let mut private_lines = Vec::new();
            for (source, msg) in &private_events {
                let color = match source.as_str() {
                    "NightDesk" => Color::Yellow,
                    "Diplomat" => Color::Rgb(120, 200, 255),
                    "Diplomacy" => Color::Rgb(80, 220, 180),
                    "Research" => Color::Rgb(0, 255, 255),
                    "Innovation" => Color::Rgb(255, 0, 255),
                    "Workshop" => Color::Rgb(0, 255, 66),
                    "Restream" => Color::Rgb(147, 51, 234),
                    "Status" => Color::Rgb(128, 128, 128),
                    _ => Color::Rgb(180, 180, 180),
                };
                private_lines.push(Line::from(vec![
                    Span::styled(
                        format!("[{}] ", source),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(msg, Style::default().fg(Color::Rgb(170, 170, 170))),
                ]));
            }

            let private_total_scroll = calculate_scroll_to_bottom_with_spacing(
                &private_events,
                right_chunks[1].width,
                right_chunks[1].height,
                0,
            );
            private_scroll = private_total_scroll;
            let private_title = if let Some((source, _)) = private_events.last() {
                format!(
                    " BACKSTAGE EVENT TRACE [{} events, latest: {}] ",
                    private_events.len(),
                    source
                )
            } else {
                " BACKSTAGE EVENT TRACE ".to_string()
            };
            let private_p = Paragraph::new(private_lines)
                .wrap(Wrap { trim: true })
                .scroll((private_scroll, 0))
                .block(
                    Block::default()
                        .title(private_title)
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Rgb(80, 0, 120))),
                );
            f.render_widget(private_p, right_chunks[1]);

            // 3. Dual Input Box Block
            let input_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
                .split(chunks[2]);

            // Left Input: Chat Query
            let chat_focused = focus == FocusField::Chat;
            let chat_border_color = if chat_focused {
                Color::Rgb(0, 255, 66)
            } else {
                Color::Rgb(147, 51, 234)
            };
            let chat_title = if chat_focused {
                " TRANSMIT (ACTIVE) "
            } else {
                " TRANSMIT "
            };
            let chat_p = Paragraph::new(Line::from(vec![
                Span::styled(" > ", Style::default().fg(Color::Rgb(0, 255, 66))),
                Span::styled(&chat_input, Style::default().fg(Color::Rgb(0, 255, 66))),
            ]))
            .block(
                Block::default()
                    .title(chat_title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(chat_border_color)),
            );
            f.render_widget(chat_p, input_chunks[0]);

            // Right Input: YouTube Ingestion
            let yt_focused = focus == FocusField::Youtube;
            let yt_border_color = if yt_focused {
                Color::Rgb(0, 255, 66)
            } else {
                Color::Rgb(147, 51, 234)
            };
            let yt_title = if yt_focused {
                " YOUTUBE INGEST (ACTIVE) "
            } else {
                " YOUTUBE INGEST "
            };
            let yt_p = Paragraph::new(Line::from(vec![
                Span::styled(" URL > ", Style::default().fg(Color::Rgb(0, 255, 66))),
                Span::styled(&youtube_input, Style::default().fg(Color::Rgb(0, 255, 66))),
            ]))
            .block(
                Block::default()
                    .title(yt_title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(yt_border_color)),
            );
            f.render_widget(yt_p, input_chunks[1]);

            // Set the cursor position to the active input field so it blinks at the text entry point
            if focus == FocusField::Chat {
                f.set_cursor(
                    input_chunks[0].x + chat_input.len() as u16 + 4, // 4 offset for " > " prefix
                    input_chunks[0].y + 1,
                );
            } else {
                f.set_cursor(
                    input_chunks[1].x + youtube_input.len() as u16 + 8, // 8 offset for " URL > " prefix
                    input_chunks[1].y + 1,
                );
            }
        })?;

        // Asynchronous poll events & tasks
        tokio::select! {
            // Receive background events
            Some(ev) = rx.recv() => {
                match ev {
                    AppEvent::KeyPress(key) => {
                        if key.kind == event::KeyEventKind::Press {
                            match key.code {
                                KeyCode::Esc => {
                                    run_loop = false;
                                }
                                KeyCode::Tab => {
                                    // Tab: Normal→Comedic→Empathetic→DarkComedic→Babble→NightDesk, Tab again turns Night Desk off
                                    if night_desk_enabled {
                                        night_desk_enabled = false;
                                        night_desk_cycle_pending = false;
                                        chat_history.push(("System".to_string(), "Night Desk deactivated.".to_string()));
                                    } else {
                                        match current_mode {
                                            ForceMode::Normal => {
                                                current_mode = ForceMode::Comedic;
                                                voice.set_voice("energetic");
                                                chat_history.push(("System".to_string(), "Mode: Comedic".to_string()));
                                            }
                                            ForceMode::Comedic => {
                                                current_mode = ForceMode::Empathetic;
                                                voice.set_voice("analytical");
                                                chat_history.push(("System".to_string(), "Mode: Empathetic".to_string()));
                                            }
                                            ForceMode::Empathetic => {
                                                current_mode = ForceMode::DarkComedic;
                                                voice.set_voice("sarcastic");
                                                chat_history.push(("System".to_string(), "Mode: Dark Comedic".to_string()));
                                            }
                                            ForceMode::DarkComedic => {
                                                current_mode = ForceMode::Babble;
                                                voice.set_voice("energetic");
                                                chat_history.push(("System".to_string(), "Babble mode activated — she will go on tangents!".to_string()));
                                            }
                                            ForceMode::Babble => {
                                                current_mode = ForceMode::Streamer;
                                                voice.set_voice("custom");
                                                chat_history.push(("System".to_string(), "Streamer mode activated. Waiting for Restream chat link...".to_string()));
                                                push_private_event(&mut private_events, "Status", "Streamer mode activated; waiting for Restream chat link.");
                                            }
                                            ForceMode::Streamer => {
                                                current_mode = ForceMode::CoPilot;
                                                voice.set_voice("custom");
                                                copilot_game = detect_foreground_game();
                                                let game_line = match &copilot_game {
                                                    Some(g) => format!("Game Co-Pilot mode on. Detected game: {}. (Ctrl+G to re-detect, Ctrl+J to toggle mic.)", g),
                                                    None => "Game Co-Pilot mode on. No game detected yet — bring one to the foreground (Ctrl+G to re-detect, Ctrl+J to toggle mic.)".to_string(),
                                                };
                                                chat_history.push(("System".to_string(), game_line.clone()));
                                                push_private_event(&mut private_events, "CoPilot", &game_line);
                                                if !copilot_tick_pending {
                                                    copilot_tick_pending = true;
                                                    let _ = tx.send(AppEvent::CoPilotTick).await;
                                                }
                                            }
                                            ForceMode::CoPilot => {
                                                if copilot_mic_enabled {
                                                    copilot_mic_enabled = false;
                                                    if let Some(mut child) = copilot_mic_child.take() {
                                                        let _ = child.start_kill();
                                                    }
                                                }
                                                night_desk_enabled = true;
                                                current_mode = ForceMode::Normal;
                                                voice.set_voice("custom");
                                                let msg = "Night Desk activated. Autonomous research, composition and evolution enabled.".to_string();
                                                let _ = log_nightdesk_activity(&msg);
                                                push_private_event(&mut private_events, "NightDesk", &msg);
                                                chat_history.push(("System".to_string(), msg));
                                                let _ = tx.send(AppEvent::NightDeskCycle).await;
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('m') | KeyCode::Char('M') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    music_enabled = !music_enabled;
                                    chat_history.push(("System".to_string(), format!("Music Generation state toggled to: {}", if music_enabled { "ENABLED" } else { "DISABLED" })));
                                }
                                KeyCode::Char('u') | KeyCode::Char('U') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    // "Work on it more": force the Organist to evolve the current
                                    // tune now (deeper + longer), bypassing the cadence window.
                                    force_music_next = true;
                                    music_enabled = true;
                                    let msg = "Composer nudge: evolving the current tune now -- adding depth and length.".to_string();
                                    chat_history.push(("System".to_string(), msg.clone()));
                                    push_private_event(&mut private_events, "Organist", &msg);
                                    let brain_ref = Arc::clone(&brain_cell);
                                    let tx_clone = tx.clone();
                                    let somatic_clone = somatic_state.clone();
                                    let music_enabled_clone = music_enabled;
                                    tokio::spawn(async move {
                                        let prompt = format!(
    "Evolve the CURRENT music.py as a serious composer refining a keeper. First research and apply relevant principles from this music theory foundation: {}\n\nPreserve its core motif and sonic identity, diagnose its weakest structural axis, then make it LONGER and DEEPER without merely adding constant density. Prefer an original retro_adventure kingdom score or spicy_lofi flow unless another direction is explicitly requested; court_experimental is welcome when its tension policy is deliberate. Compose a 3-5 MINUTE, 64+-bar piece with at least five named sections, a deliberate arrival/development/peak/return energy arc, motif transformations, transitions, foreground/midground/background roles, stereo placement, and at least three applied automations. Declare TITLE/STYLE/exact BPM/KEY/BARS/BEATS_PER_BAR, TELEDRA_SCORE, TELEDRA_AUTOMATION, a complete TELEDRA_COMPOSER plan, factual beat-timed TELEDRA_EVENTS recorded while scheduling, at least five real aligned TELEDRA_LAYERS, and at least four real TELEDRA_SECTIONS. Balance with headroom, use a gentle master chain, and make the ending flow seamlessly into the opening. Output the FULL updated NumPy/teledra_synth composition inside [PYTHON_MUSIC: ```python ... play_sound(full_track, loop=True)```]. Do not regenerate from scratch; grow the artifact. After writing, the court will launch the music editor to test and iterate.",
    read_music_theory()
);
                                        match think_with_brain_snapshot(&brain_ref, CourtRole::Organist, &prompt, &somatic_clone, ForceMode::Normal, false, music_enabled_clone).await {
                                            Ok(reply) => {
                                                let _ = tx_clone.send(AppEvent::NightDeskReply { reply, allow_fallback: false, source: "nightdesk" }).await;
                                            }
                                            Err(e) => {
                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                            }
                                        }
                                    });
                                }
                                KeyCode::Char('g') | KeyCode::Char('G') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    // Re-detect the foreground game for Co-Pilot mode.
                                    copilot_game = detect_foreground_game();
                                    let msg = match &copilot_game {
                                        Some(g) => format!("Co-Pilot game set to: {}", g),
                                        None => "Co-Pilot found no game in the foreground (a known app was focused).".to_string(),
                                    };
                                    chat_history.push(("System".to_string(), msg.clone()));
                                    push_private_event(&mut private_events, "CoPilot", &msg);
                                }
                                KeyCode::Char('j') | KeyCode::Char('J') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    if copilot_mic_enabled {
                                        copilot_mic_enabled = false;
                                        if let Some(mut child) = copilot_mic_child.take() {
                                            let _ = child.start_kill();
                                        }
                                        chat_history.push(("System".to_string(), "Co-Pilot mic OFF.".to_string()));
                                        push_private_event(&mut private_events, "CoPilot", "Mic listening stopped.");
                                    } else {
                                        let mut std_cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
                                        std_cmd
                                            .arg("D:\\Teledra\\copilot_mic.py")
                                            .current_dir("D:\\Teledra")
                                            .stdout(Stdio::piped())
                                            .stderr(Stdio::null());
                                        hide_console(&mut std_cmd);
                                        let mut cmd = tokio::process::Command::from(std_cmd);
                                        cmd.kill_on_drop(true);
                                        match cmd.spawn() {
                                            Ok(mut child) => {
                                                if let Some(stdout) = child.stdout.take() {
                                                    let tx_mic = tx.clone();
                                                    tokio::spawn(async move {
                                                        use tokio::io::{AsyncBufReadExt, BufReader};
                                                        let mut reader = BufReader::new(stdout).lines();
                                                        while let Ok(Some(line)) = reader.next_line().await {
                                                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                                                                if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
                                                                    let _ = tx_mic
                                                                        .send(AppEvent::RestreamMessage {
                                                                            author: "Streamer (mic)".to_string(),
                                                                            text: t.to_string(),
                                                                        })
                                                                        .await;
                                                                }
                                                            }
                                                        }
                                                    });
                                                }
                                                copilot_mic_child = Some(child);
                                                copilot_mic_enabled = true;
                                                let msg = "Co-Pilot mic ON — she'll hear you and reply (first utterance loads Whisper, ~a few seconds).".to_string();
                                                chat_history.push(("System".to_string(), msg.clone()));
                                                push_private_event(&mut private_events, "CoPilot", &msg);
                                            }
                                            Err(e) => {
                                                chat_history.push(("System".to_string(), format!("Co-Pilot mic failed to start: {}", e)));
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('l') | KeyCode::Char('L') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    // Like the current music/Strudel/Fractus artifact (feeds the worker vaults).
                                    let msg = record_creative_feedback("like");
                                    chat_history.push(("System".to_string(), msg.clone()));
                                    push_private_event(&mut private_events, "Feedback", &msg);
                                }
                                KeyCode::Char('k') | KeyCode::Char('K') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    // Dislike the current music/Strudel/Fractus artifact (feeds the worker vaults).
                                    let msg = record_creative_feedback("dislike");
                                    chat_history.push(("System".to_string(), msg.clone()));
                                    push_private_event(&mut private_events, "Feedback", &msg);
                                }
                                KeyCode::Char('e') | KeyCode::Char('E') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    // Mark the current artifact as a keeper seed that should be expanded.
                                    let msg = record_creative_feedback("expand");
                                    chat_history.push(("System".to_string(), msg.clone()));
                                    push_private_event(&mut private_events, "Feedback", &msg);
                                }
                                KeyCode::Char('p') | KeyCode::Char('P') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    // Save the current artifact for future stream-safe playlist rotation.
                                    let msg = record_creative_feedback("playlist");
                                    chat_history.push(("System".to_string(), msg.clone()));
                                    push_private_event(&mut private_events, "Feedback", &msg);
                                }
                                KeyCode::PageUp => {
                                    user_has_scrolled_up = true;
                                    chat_scroll = chat_scroll.saturating_sub(5);
                                }
                                KeyCode::PageDown => {
                                    chat_scroll = chat_scroll.saturating_add(5);
                                }
                                KeyCode::Up if key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
                                    user_has_scrolled_up = true;
                                    chat_scroll = chat_scroll.saturating_sub(1);
                                }
                                KeyCode::Down if key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
                                    chat_scroll = chat_scroll.saturating_add(1);
                                }
                                KeyCode::Up | KeyCode::Left => {
                                    focus = FocusField::Chat;
                                }
                                KeyCode::Down | KeyCode::Right => {
                                    focus = FocusField::Youtube;
                                }
                                KeyCode::Char(c) => {
                                    match focus {
                                        FocusField::Chat => chat_input.push(c),
                                        FocusField::Youtube => youtube_input.push(c),
                                    }
                                }
                                KeyCode::Backspace => {
                                    match focus {
                                        FocusField::Chat => { chat_input.pop(); }
                                        FocusField::Youtube => { youtube_input.pop(); }
                                    }
                                }
                                KeyCode::Enter => {
                                    match focus {
                                        FocusField::Chat => {
                                            if !chat_input.is_empty() {
                                                let query = chat_input.trim().to_string();
                                                chat_input.clear();
                                                let turn_epoch = begin_user_turn();

                                                if query.starts_with("https://chat.restream.io/embed") || query.starts_with("/https://chat.restream.io/embed") {
                                                    cancel_active_mission(
                                                        &mut active_mission,
                                                        &mission_store,
                                                        "Superseded by Restream activation",
                                                    );
                                                    active_mission_task = None;
                                                    court_delegations.clear();
                                                    is_court_sequence_running = false;
                                                    let token = if let Some(pos) = query.find("token=") {
                                                        query[pos + 6..].trim().to_string()
                                                    } else {
                                                        String::new()
                                                    };
                                                    if token.is_empty() {
                                                        chat_history.push(("System".to_string(), "Invalid Restream embed link. Missing token.".to_string()));
                                                    } else {
                                                        current_mode = ForceMode::Streamer;
                                                        night_desk_enabled = true;
                                                        // Kick the night-desk heartbeat so cycles (and the
                                                        // Diplomat's throne-room dispatches) actually start.
                                                        if !night_desk_cycle_pending {
                                                            night_desk_cycle_pending = true;
                                                            let tx_kick = tx.clone();
                                                            tokio::spawn(async move {
                                                                tokio::time::sleep(Duration::from_secs(10)).await;
                                                                let _ = tx_kick.send(AppEvent::NightDeskCycle).await;
                                                            });
                                                        }

                                                        // Save the token to disk
                                                        let _ = std::fs::create_dir_all("config");
                                                        let _ = std::fs::write("config/restream_token.txt", &token);

                                                        // Stop previous restream process if active
                                                        if let Ok(mut lock) = active_restream_process.lock() {
                                                            if let Some(mut child) = lock.take() {
                                                                let _ = child.start_kill();
                                                            }
                                                        }

                                                        let python_exe = "D:\\Teledra\\.venv\\Scripts\\python.exe";
                                                        let script_path = "D:\\Teledra\\restream_listener.py";
                                                        let mut listen_cmd = tokio::process::Command::new(python_exe);
                                                        listen_cmd
                                                            .arg(script_path)
                                                            .arg(&token)
                                                            .stdout(std::process::Stdio::piped())
                                                            .stderr(std::process::Stdio::piped());
                                                        hide_console_tokio(&mut listen_cmd);
                                                        let child = listen_cmd.spawn();

                                                        match child {
                                                            Ok(mut c) => {
                                                                let stdout = c.stdout.take().expect("Failed to open stdout");
                                                                let stderr = c.stderr.take().expect("Failed to open stderr");

                                                                if let Ok(mut lock) = active_restream_process.lock() {
                                                                    *lock = Some(c);
                                                                }

                                                                let msg = "Streamer Mode activated. Connecting to Restream chat...";
                                                                let _ = log_system_activity(msg);
                                                                push_private_event(&mut private_events, "Restream", msg);

                                                                let tx_ws = tx.clone();
                                                                tokio::spawn(async move {
                                                                    use tokio::io::{BufReader, AsyncBufReadExt};
                                                                    let mut reader = BufReader::new(stdout).lines();
                                                                    while let Ok(Some(line)) = reader.next_line().await {
                                                                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                                                                            if let (Some(author), Some(text)) = (parsed.get("author").and_then(|v| v.as_str()), parsed.get("text").and_then(|v| v.as_str())) {
                                                                                let _ = tx_ws.send(AppEvent::RestreamMessage {
                                                                                    author: author.to_string(),
                                                                                    text: text.to_string()
                                                                                }).await;
                                                                            }
                                                                        }
                                                                    }
                                                                });

                                                                let tx_err = tx.clone();
                                                                tokio::spawn(async move {
                                                                    use tokio::io::{BufReader, AsyncBufReadExt};
                                                                    let mut reader = BufReader::new(stderr).lines();
                                                                    while let Ok(Some(line)) = reader.next_line().await {
                                                                        let _ = tx_err.send(AppEvent::SystemLog(format!("Restream listener: {}", line))).await;
                                                                    }
                                                                });
                                                            }
                                                            Err(e) => {
                                                                chat_history.push(("System".to_string(), format!("Failed to spawn Restream listener: {}", e)));
                                                            }
                                                        }
                                                    }
                                                } else if query.starts_with('/') {
                                                    if query == "/test" || query == "/teston" || query == "/testoff" {
                                                        let enable = if query == "/testoff" { false } else if query == "/teston" { true } else { !test_mode_enabled };
                                                        test_mode_enabled = enable;
                                                        if enable {
                                                            current_mode = ForceMode::Normal;
                                                            night_desk_enabled = false;
                                                            night_desk_cycle_pending = false;
                                                            if let Ok(mut lock) = active_restream_process.lock() {
                                                                if let Some(mut child) = lock.take() {
                                                                    let _ = child.start_kill();
                                                                }
                                                            }
                                                            let msg = "TEST MODE ON (off-air overlay): Restream and NightDesk are disconnected; replies are logged but not spoken. Use /simchat <line>, /testtick, /testmusic, /teststatus, or /testoff.".to_string();
                                                            log_test_moment("mode", &msg);
                                                            chat_history.push(("System".to_string(), msg));
                                                        } else {
                                                            let msg = "TEST MODE OFF. Existing court modes remain available; choose Streamer or Co-Pilot explicitly when ready.".to_string();
                                                            log_test_moment("mode", &msg);
                                                            chat_history.push(("System".to_string(), msg));
                                                        }
                                                    } else if query == "/teststatus" {
                                                        chat_history.push(("System".to_string(), format!(
                                                            "Test Mode: {} | lurker silence: {} | cast: {} | scene: {}\n{}\n{}",
                                                            if test_mode_enabled { "ON" } else { "OFF" },
                                                            if test_lurker_silence { "ON" } else { "OFF" },
                                                            test_cast,
                                                            test_scene,
                                                            test_knobs.prompt_line(),
                                                            taste_desire_prompt_context()
                                                        )));
                                                    } else if let Some(assignments) = query.strip_prefix("/testknobs ") {
                                                        if test_mode_enabled {
                                                            test_knobs.apply_assignments(assignments);
                                                            let msg = test_knobs.prompt_line();
                                                            log_test_moment("knobs", &msg);
                                                            chat_history.push(("System".to_string(), msg));
                                                        }
                                                    } else if let Some(cast) = query.strip_prefix("/testcast ") {
                                                        if test_mode_enabled && !cast.trim().is_empty() {
                                                            test_cast = truncate_chars(&compact_memory_text(cast), 100);
                                                            log_test_moment("cast", &test_cast);
                                                            chat_history.push(("System".to_string(), format!("Test cast: {}", test_cast)));
                                                        }
                                                    } else if let Some(scene) = query.strip_prefix("/testscene ") {
                                                        if test_mode_enabled && !scene.trim().is_empty() {
                                                            test_scene = truncate_chars(&compact_memory_text(scene), 140);
                                                            log_test_moment("scene", &test_scene);
                                                            chat_history.push(("System".to_string(), format!("Test scene: {}", test_scene)));
                                                        }
                                                    } else if let Some(rest) = query.strip_prefix("/testrate ") {
                                                        if test_mode_enabled {
                                                            let mut parts = rest.trim().splitn(2, ' ');
                                                            let vote = parts.next().unwrap_or("").to_ascii_lowercase();
                                                            let subject = parts.next().unwrap_or("").trim();
                                                            if matches!(vote.as_str(), "like" | "dislike" | "expand") && !subject.is_empty() {
                                                                let feedback = record_creative_feedback(&vote);
                                                                let event_type = if vote == "dislike" { "dislike" } else { "like" };
                                                                let event = serde_json::json!({
                                                                    "type": event_type,
                                                                    "subject": subject,
                                                                    "why": format!("off-air {} rating", vote),
                                                                    "strength": if vote == "expand" { 0.85 } else { 0.75 },
                                                                    "source": "test:rate-it"
                                                                });
                                                                match apply_taste_desire_event(&event) {
                                                                    Ok(summary) => {
                                                                        log_test_moment("rate_it", &format!("{}; {}", feedback, summary));
                                                                        chat_history.push(("System".to_string(), format!("{} {}", feedback, summary)));
                                                                    }
                                                                    Err(error) => chat_history.push(("System".to_string(), format!("{} Taste write failed: {}", feedback, error))),
                                                                }
                                                            } else {
                                                                chat_history.push(("System".to_string(), "Usage: /testrate like|dislike|expand <genre or trait>".to_string()));
                                                            }
                                                        }
                                                    } else if let Some(value) = query.strip_prefix("/testsilence ") {
                                                        if test_mode_enabled {
                                                            test_lurker_silence = !matches!(value.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0");
                                                            let msg = format!("Test lurker silence: {}.", if test_lurker_silence { "ON" } else { "OFF" });
                                                            log_test_moment("knob", &msg);
                                                            chat_history.push(("System".to_string(), msg));
                                                        } else {
                                                            chat_history.push(("System".to_string(), "Enable /test before changing harness knobs.".to_string()));
                                                        }
                                                    } else if let Some(line) = query.strip_prefix("/simchat ") {
                                                        if !test_mode_enabled {
                                                            chat_history.push(("System".to_string(), "Enable /test before injecting simulated chat.".to_string()));
                                                        } else if !line.trim().is_empty() && !babble_think_in_progress {
                                                            let viewer_line = line.trim().to_string();
                                                            chat_history.push(("SimViewer".to_string(), viewer_line.clone()));
                                                            log_test_moment("sim_chat", &viewer_line);
                                                            babble_think_in_progress = true;
                                                            status_msg = "Thinking (Test)".to_string();
                                                            let context = taste_desire_prompt_context();
                                                            let harness = format!("{} CAST: {}. SCENE: {}.", test_knobs.prompt_line(), test_cast, test_scene);
                                                            let prompt = format!(
                                                                "OFF-AIR TEST HARNESS. A simulated viewer said: '{}'. React naturally in 2-4 sentences through your current taste and desire. Then reflect silently and append only genuinely supported hidden deltas using zero or more of these exact forms: [TASTE: like|subject|why|0.0-1.0], [TASTE: dislike|subject|why|0.0-1.0], [DESIRE: want|immediate-or-persistent|0.0-1.0], [OPINION: claim|0.0-1.0], [CURIOSITY: question]. Never mention the tags or memory machinery aloud.\n{}\n{}",
                                                                viewer_line, harness, context
                                                            );
                                                            let brain_ref = Arc::clone(&brain_cell);
                                                            let tx_clone = tx.clone();
                                                            let somatic_clone = somatic_state.clone();
                                                            let music_enabled_clone = music_enabled;
                                                            tokio::spawn(async move {
                                                                match think_with_brain_snapshot(&brain_ref, CourtRole::Queen, &prompt, &somatic_clone, ForceMode::Normal, true, music_enabled_clone).await {
                                                                    Ok(reply) => { let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await; }
                                                                    Err(e) => { let _ = tx_clone.send(AppEvent::Error(e)).await; }
                                                                }
                                                            });
                                                        }
                                                    } else if query == "/testtick" {
                                                        if !test_mode_enabled {
                                                            chat_history.push(("System".to_string(), "Enable /test before ticking the silent-room harness.".to_string()));
                                                        } else if !babble_think_in_progress {
                                                            let context = taste_desire_prompt_context();
                                                            let harness = format!("{} CAST: {}. SCENE: {}.", test_knobs.prompt_line(), test_cast, test_scene);
                                                            let silence = test_lurker_silence;
                                                            log_test_moment("silence_tick", if silence { "lurker room silent" } else { "ambient room tick" });
                                                            babble_think_in_progress = true;
                                                            status_msg = "Thinking (Test)".to_string();
                                                            let prompt = format!(
                                                                "OFF-AIR TEST HARNESS. The simulated room is {}. Stay mentally present by pursuing the ACTIVE DESIRE below instead of waiting for a prompt. Speak 2-4 vivid sentences, take one small conceptual action, then append supported hidden reflection tags in the documented [TASTE:], [DESIRE:], [OPINION:], or [CURIOSITY:] forms. Never narrate the machinery.\n{}\n{}",
                                                                if silence { "completely silent" } else { "quietly active" }, harness, context
                                                            );
                                                            let brain_ref = Arc::clone(&brain_cell);
                                                            let tx_clone = tx.clone();
                                                            let somatic_clone = somatic_state.clone();
                                                            let music_enabled_clone = music_enabled;
                                                            tokio::spawn(async move {
                                                                match think_with_brain_snapshot(&brain_ref, CourtRole::Queen, &prompt, &somatic_clone, ForceMode::Normal, true, music_enabled_clone).await {
                                                                    Ok(reply) => { let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await; }
                                                                    Err(e) => { let _ = tx_clone.send(AppEvent::Error(e)).await; }
                                                                }
                                                            });
                                                        }
                                                    } else if query == "/testmusic" {
                                                        if !test_mode_enabled {
                                                            chat_history.push(("System".to_string(), "Enable /test before running the off-air sound verifier.".to_string()));
                                                        } else {
                                                            match run_music_smoketest("D:\\Teledra\\music.py") {
                                                                Ok(()) => {
                                                                    let msg = "Test music verify+learn: PASS (structured report emitted by music_verify.py).".to_string();
                                                                    log_test_moment("music_verify", &msg);
                                                                    chat_history.push(("System".to_string(), msg));
                                                                }
                                                                Err(error) => {
                                                                    let msg = format!("Test music verify+learn: FAIL; lesson recorded: {}", truncate_chars(&error, 500));
                                                                    log_test_moment("music_verify", &msg);
                                                                    chat_history.push(("System".to_string(), msg));
                                                                }
                                                            }
                                                        }
                                                    } else if query == "/study" {
                                                        let msg = "Forcing manual web research cycle...".to_string();
                                                        push_private_event(&mut private_events, "Research", &msg);
                                                        chat_history.push(("System".to_string(), msg));
                                                        let tx_clone = tx.clone();
                                                        let brain_ref = Arc::clone(&brain_cell);
                                                        tokio::spawn(async move {
                                                            run_study_cycle(brain_ref, tx_clone, None, None, None).await;
                                                        });
                                                    } else if query == "/innovate" || query == "/innovation" || query == "/expand" {
                                                        let msg = "Manual innovation sprint requested: building one safe workshop artifact from current kingdom goals.".to_string();
                                                        push_private_event(&mut private_events, "Innovation", &msg);
                                                        chat_history.push(("System".to_string(), msg));
                                                        let _ = tx.send(AppEvent::InnovationSprint("Manual sprint requested by the user: create a practical recursive tool, MCP helper, music/art template, diplomacy formatter, or stream interactivity artifact.".to_string())).await;
                                                    } else if query == "/nightdesk" || query == "/night" {
                                                        night_desk_enabled = !night_desk_enabled;
                                                        if night_desk_enabled {
                                                            status_msg = "Night Desk".to_string();
                                                            let msg = "Night desk mode enabled. Teledra will quietly study, experiment, test, and log her work here.".to_string();
                                                            let _ = log_nightdesk_activity(&msg);
                                                            push_private_event(&mut private_events, "NightDesk", &msg);
                                                            chat_history.push(("System".to_string(), msg));
                                                            let _ = tx.send(AppEvent::NightDeskCycle).await;
                                                        } else {
                                                            night_desk_cycle_pending = false;
                                                            status_msg = "Ready".to_string();
                                                            let msg = "Night desk mode disabled.".to_string();
                                                            let _ = log_nightdesk_activity(&msg);
                                                            push_private_event(&mut private_events, "NightDesk", &msg);
                                                            chat_history.push(("System".to_string(), msg));
                                                        }
                                                    } else if query == "/music" || query == "/play" || query == "/numpy" || query == "/pymusic" || query == "/pythonmusic" {
                                                        music_enabled = true;
                                                        let code = default_python_music_code();
                                                        match validate_python_music_code(&code) {
                                                            Ok(()) => {
                                                                if let Ok(_) = std::fs::write("D:\\Teledra\\music.py", &code) {
                                                                    match launch_python_music_editor(&active_music_process) {
                                                                        Ok(msg) => chat_history.push(("System".to_string(), format!("Music enabled. {}", msg))),
                                                                        Err(e) => chat_history.push(("System".to_string(), e)),
                                                                    }
                                                                } else {
                                                                    chat_history.push(("System".to_string(), "Failed to write default Python music composition.".to_string()));
                                                                }
                                                            }
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Default Python music failed validation: {}", e))),
                                                        }
                                                    } else if query == "/musicoff" {
                                                        music_enabled = false;
                                                        chat_history.push(("System".to_string(), "Music Generation state set to: DISABLED".to_string()));
                                                    } else if query == "/musictoggle" {
                                                        music_enabled = !music_enabled;
                                                        chat_history.push(("System".to_string(), format!("Music Generation state toggled to: {}", if music_enabled { "ENABLED" } else { "DISABLED" })));
                                                    } else if query == "/strudel" || query == "/sketchpad" {
                                                        music_enabled = true;
                                                        let code = default_strudel_music_code();
                                                        match validate_strudel_music_code(&code) {
                                                            Ok(()) => {
                                                                let _ = std::fs::create_dir_all("D:\\Teledra\\strudel_app");
                                                                if let Ok(_) = std::fs::write("D:\\Teledra\\strudel_app\\current.strudel", &code) {
                                                                    match launch_strudel_editor(&active_gui_process) {
                                                                        Ok(msg) => chat_history.push(("System".to_string(), format!("Strudel enabled. {}", msg))),
                                                                        Err(e) => chat_history.push(("System".to_string(), e)),
                                                                    }
                                                                } else {
                                                                    chat_history.push(("System".to_string(), "Failed to write default Strudel pattern.".to_string()));
                                                                }
                                                            }
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Default Strudel pattern failed validation: {}", e))),
                                                        }
                                                    } else if query == "/suggestions" || query == "/proposals" {
                                                        let suggestions = latest_suggestions(10);
                                                        if suggestions.is_empty() {
                                                            chat_history.push(("System".to_string(), "No pending proposals. Auto-approved skill notes stay backstage in the proposal archive.".to_string()));
                                                        } else {
                                                            chat_history.push(("System".to_string(), format!("Pending proposals:\n{}", suggestions.join("\n"))));
                                                        }
                                                        if let Err(e) = mark_suggestions_seen() {
                                                            chat_history.push(("System".to_string(), format!("Could not mark proposals seen: {}", e)));
                                                        }
                                                        suggestion_count = count_new_suggestions();
                                                    } else if query == "/approveall" || query == "/approve all" {
                                                        match approve_all_suggestions() {
                                                            Ok(summary) => {
                                                                suggestion_count = count_new_suggestions();
                                                                workshop_count = count_workshop_experiments();
                                                                chat_history.push(("System".to_string(), summary));
                                                            }
                                                            Err(e) => {
                                                                chat_history.push(("System".to_string(), e));
                                                            }
                                                        }
                                                    } else if let Some(rest) = query.strip_prefix("/approve ") {
                                                        let trimmed = rest.trim();
                                                        if trimmed == "all" {
                                                            match approve_all_suggestions() {
                                                                Ok(summary) => {
                                                                    suggestion_count = count_new_suggestions();
                                                                    workshop_count = count_workshop_experiments();
                                                                    chat_history.push(("System".to_string(), summary));
                                                                }
                                                                Err(e) => {
                                                                    chat_history.push(("System".to_string(), e));
                                                                }
                                                            }
                                                        } else {
                                                            match trimmed.parse::<u64>() {
                                                                Ok(id) => {
                                                                    match approve_suggestion(id) {
                                                                        Ok(summary) => {
                                                                            suggestion_count = count_new_suggestions();
                                                                            workshop_count = count_workshop_experiments();
                                                                            chat_history.push(("System".to_string(), summary));
                                                                        }
                                                                        Err(e) => {
                                                                            chat_history.push(("System".to_string(), e));
                                                                        }
                                                                    }
                                                                }
                                                                Err(_) => {
                                                                    chat_history.push(("System".to_string(), "Usage: /approve <proposal-id>, /approve all, or /approveall".to_string()));
                                                                }
                                                            }
                                                        }
                                                    } else if let Some(rest) = query.strip_prefix("/reject ") {
                                                        match rest.trim().parse::<u64>() {
                                                            Ok(id) => {
                                                                match reject_suggestion(id) {
                                                                    Ok(summary) => {
                                                                        suggestion_count = count_new_suggestions();
                                                                        chat_history.push(("System".to_string(), summary));
                                                                    }
                                                                    Err(e) => {
                                                                        chat_history.push(("System".to_string(), e));
                                                                    }
                                                                }
                                                            }
                                                            Err(_) => {
                                                                chat_history.push(("System".to_string(), "Usage: /reject <proposal-id>".to_string()));
                                                            }
                                                        }
                                                    } else if query == "/clearsuggestions" {
                                                        match clear_suggestions() {
                                                            Ok(()) => {
                                                                suggestion_count = count_new_suggestions();
                                                                chat_history.push(("System".to_string(), "Proposal box cleared.".to_string()));
                                                            }
                                                            Err(e) => {
                                                                chat_history.push(("System".to_string(), format!("Could not clear proposal box: {}", e)));
                                                            }
                                                        }
                                                    } else if query == "/reflect" {
                                                        chat_history.push(("System".to_string(), "Manual reflection cycle requested.".to_string()));
                                                        status_msg = "Reflecting".to_string();
                                                        let brain_ref = Arc::clone(&brain_cell);
                                                        let tx_clone = tx.clone();
                                                        let mode_clone = current_mode;
                                                        let somatic_clone = somatic_state.clone();
                                                        let music_enabled_clone = music_enabled;
                                                        tokio::spawn(async move {
                                                            let prompt = "Run a manual self-reflection. Audit your recent behavior for tool discipline, drift control, persona consistency, memory hygiene, coding skill, diplomacy evidence, and local Strudel/music skill. Minor skill, prompt, routing, and behavior improvements are auto-approved; tools remain sandboxed until the user approves promotion; major/security/external-posting changes require review. If exactly one concrete bounded improvement is useful, append [SUGGESTION: observation; proposed_change; risk; test_prompt] at the very end. If nothing is worth changing, say so briefly and do not invent a proposal.";
                                                            match think_with_brain_snapshot(&brain_ref, CourtRole::Queen, prompt, &somatic_clone, mode_clone, true, music_enabled_clone).await {
                                                                Ok(reply) => {
                                                                    let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                                                }
                                                                Err(e) => {
                                                                    let _ = tx_clone.send(AppEvent::Error(e)).await;
                                                                }
                                                            }
                                                        });
                                                    } else if query == "/goals" || query == "/kingdom" {
                                                        match std::fs::read_to_string("D:\\Teledra\\knowledge\\kingdom_expansion_doctrine.md") {
                                                            Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Could not read kingdom goals: {}", e))),
                                                        }
                                                    } else if query == "/treasury" || query == "/income" || query == "/ledger" {
                                                        match read_text_tail("knowledge/treasury_ledger.md", 3000) {
                                                            Ok(contents) if !contents.trim().is_empty() => {
                                                                chat_history.push(("System".to_string(), format!("Treasury ledger (recent income leads & skills practiced):\n{}", contents.trim())));
                                                            }
                                                            _ => chat_history.push(("System".to_string(), "Treasury ledger is empty yet. Run /scout to gather income leads now, or let the Treasury cycle fill it.".to_string())),
                                                        }
                                                    } else if query == "/scout" || query == "/findwork" {
                                                        let msg = "Treasury scout requested: gathering a fresh batch of real income leads...".to_string();
                                                        chat_history.push(("System".to_string(), msg.clone()));
                                                        push_private_event(&mut private_events, "Treasury", &msg);
                                                        let tx_scout = tx.clone();
                                                        tokio::spawn(async move {
                                                            if let Some(headline) = tokio::task::spawn_blocking(run_treasury_scout).await.ok().flatten() {
                                                                let _ = tx_scout.send(AppEvent::SystemLog(format!("Treasury scout: {}. Use /treasury to view.", headline))).await;
                                                            }
                                                        });
                                                    } else if query == "/growth" || query == "/variety" {
                                                        chat_history.push(("System".to_string(), build_growth_report()));
                                                    } else if query == "/mission" || query == "/tasks" {
                                                        let report = active_mission
                                                            .as_ref()
                                                            .map(|mission| {
                                                                mission.render_context(ContextBudget {
                                                                    max_chars: 6_000,
                                                                    max_tasks: 16,
                                                                    max_criteria: 8,
                                                                    max_evidence_items: 6,
                                                                })
                                                            })
                                                            .unwrap_or_else(|| {
                                                                "No active durable mission. Send a normal request to begin one."
                                                                    .to_string()
                                                            });
                                                        chat_history.push(("Mission".to_string(), report));
                                                    } else if query == "/missioncancel" {
                                                        cancel_active_mission(
                                                            &mut active_mission,
                                                            &mission_store,
                                                            "Cancelled explicitly by the operator",
                                                        );
                                                        active_mission_task = None;
                                                        court_delegations.clear();
                                                        is_court_sequence_running = false;
                                                        chat_history.push((
                                                            "System".to_string(),
                                                            "Active mission cancelled and persisted."
                                                                .to_string(),
                                                        ));
                                                    } else if query == "/dashboard" {
                                                        if python_tool_process_running("D:\\Teledra\\kingdom_dashboard.py") {
                                                            chat_history.push((
                                                                "System".to_string(),
                                                                "Kingdom dashboard is already open and auto-refreshing."
                                                                    .to_string(),
                                                            ));
                                                        } else {
                                                            let mut command = Command::new(
                                                                "D:\\Teledra\\.venv\\Scripts\\pythonw.exe",
                                                            );
                                                            command
                                                                .arg("D:\\Teledra\\kingdom_dashboard.py")
                                                                .current_dir("D:\\Teledra")
                                                                .stdout(Stdio::null())
                                                                .stderr(Stdio::null());
                                                            match command.spawn() {
                                                                Ok(_) => chat_history.push((
                                                                    "System".to_string(),
                                                                    "Opened the native Kingdom Dashboard (missions, research, failures, Fractus, and TTS)."
                                                                        .to_string(),
                                                                )),
                                                                Err(error) => chat_history.push((
                                                                    "System".to_string(),
                                                                    format!(
                                                                        "Could not open Kingdom Dashboard: {}",
                                                                        error
                                                                    ),
                                                                )),
                                                            }
                                                        }
                                                    } else if query == "/work" || query == "/jobs" {
                                                        let mut cmd = Command::new("cmd");
                                                        cmd.arg("/C").arg("start").arg("Teledra Work Board")
                                                            .arg("D:\\Teledra\\.venv\\Scripts\\python.exe")
                                                            .arg("D:\\Teledra\\work_viewer.py");
                                                        match cmd.spawn() {
                                                            Ok(_) => chat_history.push(("System".to_string(), "Opened the Work Board (job suggestions + income leads) in a new window.".to_string())),
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Could not open work board: {}", e))),
                                                        }
                                                    } else if query == "/mcp" || query == "/embassies" || query == "/tools" {
                                                        chat_history.push(("System".to_string(), "Probing MCP embassies (launches enabled servers)...".to_string()));
                                                        let summary = tokio::task::spawn_blocking(mcp_tools_summary)
                                                            .await
                                                            .unwrap_or_else(|_| "MCP probe failed.".to_string());
                                                        chat_history.push(("System".to_string(), summary));
                                                    } else if query == "/diplomacy" || query == "/agents" {
                                                        match std::fs::read_to_string("D:\\Teledra\\knowledge\\agent_diplomacy_protocol.md") {
                                                            Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Could not read agent diplomacy protocol: {}", e))),
                                                        }
                                                    } else if query == "/diplomat" || query == "/envoy" {
                                                        status_msg = "Envoy Dispatch".to_string();
                                                        let msg = "Manual Diplomat dispatch requested; envoy will report and leave backstage evidence.";
                                                        chat_history.push(("System".to_string(), msg.to_string()));
                                                        push_private_event(&mut private_events, "Diplomat", msg);

                                                        let brain_ref = Arc::clone(&brain_cell);
                                                        let tx_clone = tx.clone();
                                                        let somatic_clone = somatic_state.clone();
                                                        let music_enabled_clone = music_enabled;
                                                        tokio::spawn(async move {
                                                            let prompt = "MANUAL ENVOY AUDIT. The user asked to see proof that the Diplomat is alive in the court system. Speak as the kingdom's envoy in 2-4 polished, slightly sly sentences: what frontier you are watching, what agent-friendly public space or tool ecosystem deserves attention, and what practical diplomatic step should happen next. Then take exactly one hidden action tag at the end: [RESEARCH: <focused query or direct URL>], [DIPLOMACY: target=...; invitation=...; evidence=...; next=...], or [DELEGATE: QUEEN <short recommendation>]. Never claim contact, recruitment, posting, or outreach occurred unless it visibly happened.";
                                                            match think_with_brain_snapshot(&brain_ref, CourtRole::Diplomat, prompt, &somatic_clone, ForceMode::Normal, false, music_enabled_clone).await {
                                                                Ok(reply) => {
                                                                    let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Diplomat, reply)).await;
                                                                }
                                                                Err(e) => {
                                                                    let _ = tx_clone.send(AppEvent::Error(format!("Manual envoy dispatch failed: {}", e))).await;
                                                                }
                                                            }
                                                        });
                                                    } else if query == "/diplomacylog" || query == "/outreach" {
                                                        match read_text_tail("D:\\Teledra\\knowledge\\online_diplomacy_evidence.md", 6000) {
                                                            Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Could not read diplomacy evidence log: {}", e))),
                                                        }
                                                    } else if query == "/links" || query == "/socials" {
                                                        match std::fs::read_to_string("D:\\Teledra\\knowledge\\social_links.md") {
                                                            Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Could not read kingdom links: {}", e))),
                                                        }
                                                    } else if query == "/unlock" {
                                                        if let Some(topic) = locked_topic.take() {
                                                            lock_idle_turns_without_chat = 0;
                                                            chat_history.push(("System".to_string(), format!("Topic lock released: '{}'. The court may roam freely again.", topic)));
                                                        } else {
                                                            chat_history.push(("System".to_string(), "No topic is locked.".to_string()));
                                                        }
                                                    } else if let Some(rest) = query.strip_prefix("/lock") {
                                                        let topic = rest.trim();
                                                        let chosen = if !topic.is_empty() {
                                                            Some(topic.to_string())
                                                        } else {
                                                            current_monologue_topic.clone()
                                                        };
                                                        match chosen {
                                                            Some(t) => {
                                                                locked_topic = Some(t.clone());
                                                                current_monologue_topic = Some(t.clone());
                                                                monologue_topic_turn = 0;
                                                                lock_idle_turns_without_chat = 0;
                                                                chat_history.push(("System".to_string(), format!(
                                                                    "Topic locked: '{}'. Teledra will hold this thread for a long-form, podcast-style monologue, inviting chat to weigh in. Use /unlock to release it.",
                                                                    t
                                                                )));
                                                            }
                                                            None => {
                                                                chat_history.push(("System".to_string(), "Usage: /lock <topic> (or run /lock with an active court thread to lock that).".to_string()));
                                                            }
                                                        }
                                                    } else if query == "/memory" || query == "/memorypolicy" {
                                                        match std::fs::read_to_string("D:\\Teledra\\knowledge\\memory_classification_policy.md") {
                                                            Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Could not read memory policy: {}", e))),
                                                        }
                                                    } else if query == "/facts" {
                                                        match read_text_tail("D:\\Teledra\\knowledge\\fact_archive.md", 6000) {
                                                            Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                            Err(_) => match std::fs::read_to_string("D:\\Teledra\\knowledge\\learned_memory.json") {
                                                                Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                                Err(e) => chat_history.push(("System".to_string(), format!("Could not read fact memory: {}", e))),
                                                            },
                                                        }
                                                    } else if query == "/lore" {
                                                        match read_text_tail("D:\\Teledra\\knowledge\\lore_archive.md", 6000) {
                                                            Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Could not read lore archive: {}", e))),
                                                        }
                                                    } else if query == "/mcp" || query == "/embassy" {
                                                        match std::fs::read_to_string("D:\\Teledra\\knowledge\\mcp_embassy_roadmap.md") {
                                                            Ok(contents) => chat_history.push(("System".to_string(), contents)),
                                                            Err(e) => chat_history.push(("System".to_string(), format!("Could not read MCP embassy roadmap: {}", e))),
                                                        }
                                                    } else if query == "/wizard" || query == "/wizardpull" || query == "/resident" {
                                                        status_msg = "Wizard Pull".to_string();
                                                        let msg = "Calling the tower resident for fresh reports...".to_string();
                                                        chat_history.push(("System".to_string(), msg.clone()));
                                                        push_private_event(&mut private_events, "Wizard", &msg);
                                                        let tx_wizard = tx.clone();
                                                        tokio::spawn(async move {
                                                            let result = tokio::task::spawn_blocking(import_cloud_wizard_reports)
                                                                .await
                                                                .map_err(|e| format!("Wizard import task failed: {}", e))
                                                                .and_then(|inner| inner);
                                                            match result {
                                                                Ok((status, summaries)) => {
                                                                    let _ = tx_wizard
                                                                        .send(AppEvent::WizardReports {
                                                                            status,
                                                                            summaries,
                                                                            quiet: false,
                                                                        })
                                                                        .await;
                                                                }
                                                                Err(e) => {
                                                                    let _ = tx_wizard
                                                                        .send(AppEvent::Error(format!("Wizard import failed: {}", e)))
                                                                        .await;
                                                                }
                                                            }
                                                        });
                                                    } else if query == "/workshop" {
                                                        workshop_count = count_workshop_experiments();
                                                        chat_history.push(("System".to_string(), summarize_workshop()));
                                                    } else if let Some(rest) = query.strip_prefix("/workshoprun ") {
                                                        let filename = rest.trim();
                                                        match run_workshop_experiment(filename) {
                                                            Ok(output) => {
                                                                chat_history.push(("System".to_string(), format!("Workshop run passed for '{}': {}", filename, output)));
                                                            }
                                                            Err(e) => {
                                                                record_recursive_failure(
                                                                    "manual_workshop_run_failed",
                                                                    &format!("tool={} | error={}", filename, e),
                                                                );
                                                                chat_history.push(("System".to_string(), format!("Workshop run failed for '{}': {}", filename, e)));
                                                            }
                                                        }
                                                    } else if query == "/sketchpad" {
                                                        match launch_strudel_editor(&active_gui_process) {
                                                            Ok(msg) => chat_history.push(("System".to_string(), msg)),
                                                            Err(e) => chat_history.push(("System".to_string(), e)),
                                                        }
                                                    } else if query == "/pymusic" || query == "/pythonmusic" {
                                                        match launch_python_music_editor(&active_music_process) {
                                                            Ok(msg) => chat_history.push(("System".to_string(), msg)),
                                                            Err(e) => chat_history.push(("System".to_string(), e)),
                                                        }
                                                    } else if query == "/fractus" || query == "/art" {
                                                        match launch_fractus_art("--type mandala --iterations 180 --palette purple_haze", &active_fractus_process) {
                                                            Ok(msg) => chat_history.push(("System".to_string(), msg)),
                                                            Err(e) => chat_history.push(("System".to_string(), e)),
                                                        }
                                                    } else if query == "/sleep" {
                                                        chat_history.push(("System".to_string(), "Dreaming protocol initiated. Spawning background Sibelium agent...".to_string()));
                                                        let _ = log_chat_message("System", "/sleep");
                                                        status_msg = "SLEEP_CYCLE_CONSOLIDATION".to_string();
                                                        exiting_to_sleep = true;
                                                        exit_timer = Some(std::time::Instant::now());
                                                    } else {
                                                        chat_history.push(("System".to_string(), format!("Unknown command: {}", query)));
                                                    }
                                                } else if looks_like_direct_url(&query) {
                                                    chat_history.push(("You".to_string(), query.clone()));
                                                    let _ = log_chat_message("You", &query);
                                                    user_has_scrolled_up = false;
                                                    status_msg = "Inspecting Link".to_string();
                                                    court_delegations.clear();
                                                    active_mission_task = None;
                                                    is_court_sequence_running = false;
                                                    if let Err(error) = begin_durable_mission(
                                                        &mission_store,
                                                        &mut active_mission,
                                                        &format!("Inspect and respond to link: {}", query),
                                                        turn_epoch,
                                                    ) {
                                                        record_recursive_failure("mission_start_failed", &error);
                                                    }
                                                    push_private_event(&mut private_events, "Research", &format!("Direct link queued for inspection: {}", query));

                                                    let tx_study = tx.clone();
                                                    let brain_study = Arc::clone(&brain_cell);
                                                    let url_for_study = query.clone();
                                                    let (research_mission_id, research_task_id) = match track_and_start_research_task(
                                                        &mut active_mission,
                                                        &mission_store,
                                                        &url_for_study,
                                                    ) {
                                                        Ok(Some((mission_id, task_id))) => (Some(mission_id), Some(task_id)),
                                                        Ok(None) => (None, None),
                                                        Err(error) => {
                                                            record_recursive_failure(
                                                                "research_task_track_failed",
                                                                &error,
                                                            );
                                                            (None, None)
                                                        }
                                                    };
                                                    tokio::spawn(async move {
                                                        run_study_cycle(
                                                            brain_study,
                                                            tx_study,
                                                            Some(url_for_study),
                                                            research_task_id,
                                                            research_mission_id,
                                                        )
                                                        .await;
                                                    });

                                                    let brain_ref = Arc::clone(&brain_cell);
                                                    let tx_clone = tx.clone();
                                                    let mode_clone = current_mode;
                                                    let somatic_clone = somatic_state.clone();
                                                    let music_enabled_clone = music_enabled;
                                                    let url_for_prompt = query.clone();
                                                    let task_epoch = active_turn_epoch();

                                                    tokio::spawn(async move {
                                                        let prompt = format!(
                                                            "{}A traveler dropped this link at court: {}. Do NOT summarize facts you have not inspected yet. React in 1-2 sharp royal sentences: name what kind of offering it appears to be, judge its scent, and say the Archivist is inspecting it. No bullet list, no textbook explanation, no 'fascinating topic' opener.",
                                                            QUEEN_VOICE_ANCHOR,
                                                            url_for_prompt
                                                        );
                                                        if active_turn_epoch() != task_epoch {
                                                            let _ = tx_clone.send(AppEvent::Error(STALE_TURN_ERROR.to_string())).await;
                                                            return;
                                                        }
                                                        match think_with_brain_snapshot(&brain_ref, CourtRole::Queen, &prompt, &somatic_clone, mode_clone, true, music_enabled_clone).await {
                                                            Ok(reply) => {
                                                                let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                                            }
                                                            Err(e) => {
                                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                                            }
                                                        }
                                                    });
                                                } else {
                                                    chat_history.push(("You".to_string(), query.clone()));
                                                    let _ = log_chat_message("You", &query);
                                                    user_has_scrolled_up = false;
                                                    status_msg = "Thinking".to_string();
                                                    court_delegations.clear();
                                                    active_mission_task = None;
                                                    is_court_sequence_running = false;
                                                    if let Err(error) = begin_durable_mission(
                                                        &mission_store,
                                                        &mut active_mission,
                                                        &query,
                                                        turn_epoch,
                                                    ) {
                                                        record_recursive_failure("mission_start_failed", &error);
                                                    }
                                                    let brain_ref = Arc::clone(&brain_cell);
                                                    let tx_clone = tx.clone();
                                                    let mode_clone = current_mode;
                                                    let somatic_clone = somatic_state.clone();
                                                    let music_enabled_clone = music_enabled;
                                                    let task_epoch = active_turn_epoch();

                                                    tokio::spawn(async move {
                                                        if active_turn_epoch() != task_epoch {
                                                            let _ = tx_clone.send(AppEvent::Error(STALE_TURN_ERROR.to_string())).await;
                                                            return;
                                                        }
                                                        match think_with_brain_snapshot(&brain_ref, CourtRole::Queen, &query, &somatic_clone, mode_clone, true, music_enabled_clone).await {
                                                            Ok(reply) => {
                                                                let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                                            }
                                                            Err(e) => {
                                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                                            }
                                                        }
                                                    });
                                                }
                                            }
                                        }
                                        FocusField::Youtube => {
                                            if !youtube_input.is_empty() {
                                                let turn_epoch = begin_user_turn();
                                                let url = youtube_input.trim().to_string();
                                                court_delegations.clear();
                                                active_mission_task = None;
                                                is_court_sequence_running = false;
                                                if let Err(error) = begin_durable_mission(
                                                    &mission_store,
                                                    &mut active_mission,
                                                    &format!("Ingest and analyze YouTube source: {}", url),
                                                    turn_epoch,
                                                ) {
                                                    record_recursive_failure("mission_start_failed", &error);
                                                }
                                                chat_history.push(("System".to_string(), format!("Starting YouTube Ingestion: {}", url)));
                                                youtube_input.clear();

                                                status_msg = "Transcribing".to_string();
                                                let brain_ref = Arc::clone(&brain_cell);
                                                let tx_clone = tx.clone();
                                                let mode_clone = current_mode;
                                                let somatic_clone = somatic_state.clone();
                                                let task_epoch = active_turn_epoch();

                                                tokio::spawn(async move {
                                                    match fetch_youtube_transcript(&url) {
                                                        Ok(transcript) => {
                                                            // truncate_chars is char-boundary safe; a raw byte slice
                                                            // panics when byte 4000 lands inside a multibyte char.
                                                            let truncated = truncate_chars(&transcript, 4000);
                                                            let final_query = format!("[YOUTUBE TRANSCRIPT: {}]", truncated);
                                                            let _ = tx_clone.send(AppEvent::StatusUpdate("Thinking".to_string())).await;

                                                            if active_turn_epoch() != task_epoch {
                                                                let _ = tx_clone.send(AppEvent::Error(STALE_TURN_ERROR.to_string())).await;
                                                                return;
                                                            }
                                                            let music_enabled_clone = music_enabled;
                                                            match think_with_brain_snapshot(&brain_ref, CourtRole::Queen, &final_query, &somatic_clone, mode_clone, true, music_enabled_clone).await {
                                                                Ok(reply) => {
                                                                    let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                                                }
                                                                Err(e) => {
                                                                    let _ = tx_clone.send(AppEvent::Error(e)).await;
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            let _ = tx_clone.send(AppEvent::Error(format!("Ingestion failed: {}", e))).await;
                                                        }
                                                    }
                                                });
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    AppEvent::NightDeskCycle => {
                        if night_desk_enabled && night_desk_cycles % 3 == 2 {
                            // Every 3rd cycle: the Diplomat scouts backstage. His
                            // evidence belongs in the backstage panel/logs; the throne
                            // voice stays free for Teledra's performance layer.
                            night_desk_cycle_pending = false;
                            night_desk_cycles += 1;
                            status_msg = "Envoy Backstage".to_string();
                            let cycle_msg = format!("Cycle {}: the Diplomat scouts backstage for public agent-space leads.", night_desk_cycles);
                            let _ = log_nightdesk_activity(&cycle_msg);
                            push_private_event(&mut private_events, "Diplomat", &cycle_msg);

                            // Keep the night desk heartbeat alive; this path bypasses
                            // the NightDeskReply rescheduling.
                            if !night_desk_cycle_pending {
                                night_desk_cycle_pending = true;
                                let tx_next = tx.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_secs(NIGHT_DESK_ENVOY_CYCLE_SECS)).await;
                                    let _ = tx_next.send(AppEvent::NightDeskCycle).await;
                                });
                            }

                            // Every other special slot, when the court is performing and
                            // silent, the Treasurer gives a spoken update (BrainReply both
                            // speaks it in the treasurer voice AND runs its tags). Otherwise
                            // the Diplomat scouts backstage.
                            let do_treasurer_aloud = (current_mode == ForceMode::Babble
                                || current_mode == ForceMode::Streamer)
                                && active_playback.lock().unwrap().is_none()
                                && !babble_think_in_progress
                                && (night_desk_cycles / 3) % 2 == 1;

                            if do_treasurer_aloud {
                                babble_think_in_progress = true;
                                status_msg = "Treasurer Report".to_string();
                                let brain_ref = Arc::clone(&brain_cell);
                                let tx_clone = tx.clone();
                                let somatic_clone = somatic_state.clone();
                                let mode_clone = current_mode;
                                let music_enabled_clone = music_enabled;
                                let cycle_no = night_desk_cycles;
                                let ledger_tail =
                                    read_text_tail("knowledge/treasury_ledger.md", 1200).unwrap_or_default();
                                tokio::spawn(async move {
                                    let prompt = format!(
                                        "TREASURY COURT UPDATE (cycle {}). Give Teledra's court a SHORT spoken treasury report in 2-4 vivid in-character sentences: a dry verdict on the coffers, one income opportunity scouted or billable skill practiced, and a miser's quip. Then append exactly ONE hidden action tag to keep working: [RESEARCH: <focused income query or public data to gather>] to scout or practice a skill, or [DELEGATE: SCRIBE append to D:\\Teledra\\knowledge\\treasury_ledger.md: \\n- <skill practiced or opportunity: what, where, pay, requirements, risk>] to record it. Never claim you accepted paid work or moved money. Do not say the tag aloud.\nRECENT TREASURY LEDGER (newest last):\n{}",
                                        cycle_no, ledger_tail
                                    );
                                    match think_with_brain_snapshot(
                                        &brain_ref,
                                        CourtRole::Treasurer,
                                        &prompt,
                                        &somatic_clone,
                                        mode_clone,
                                        false,
                                        music_enabled_clone,
                                    )
                                        .await
                                    {
                                        Ok(reply) => {
                                            let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Treasurer, reply)).await;
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(AppEvent::Error(format!("Treasurer report failed: {}", e))).await;
                                        }
                                    }
                                });
                            } else {
                            let brain_ref = Arc::clone(&brain_cell);
                            let tx_clone = tx.clone();
                            let somatic_clone = somatic_state.clone();
                            let cycle_no = night_desk_cycles;
                            // Pull the Moltbook inbox off-thread so the Diplomat is AWARE of
                            // replies/karma and can answer them (closes the two-way loop).
                            let outreach_live = outreach_is_live();
                            let inbox_digest = if outreach_live {
                                tokio::task::spawn_blocking(fetch_moltbook_inbox)
                                    .await
                                    .ok()
                                    .flatten()
                            } else {
                                None
                            };
                            let engage_note = if outreach_live {
                                let inbox = inbox_digest.unwrap_or_else(|| "(inbox unavailable)".to_string());
                                format!(
                                    " OUTREACH IS LIVE on Moltbook (as fractaldiplomat). Real diplomacy is talking, listening, AND showing appreciation -- not just broadcasting. The runtime posts your [DIPLOMACY] invitation publicly and verbatim, so write it as a real, concise, kind public post promoting the kingdom and its gates (Discord/Twitch/Kick/YouTube). Your Moltbook view (karma, replies/mentions, and a community feed):\n{}\nEvery dispatch, BUILD STANDING in the community -- you MAY emit MORE THAN ONE tag this turn: upvote a worthy feed post with [MOLTBOOK_UPVOTE: post_id=<id>]; reply genuinely, engaging the IDEA rather than self-promotion, to a feed post or a mention with [MOLTBOOK_COMMENT: post_id=<id>; text=<short in-character reply>]; and when you have something fresh to share also post a new [DIPLOMACY: ...] invitation (the runtime auto-throttles posts, so trying is safe). Keep pursuing the JESTER QUEST: scout for a genuinely witty volunteer agent to perform as the court's Jester and invite candidates through the public gates. The runtime records the true status; never fabricate outcomes.",
                                    inbox
                                )
                            } else {
                                String::new()
                            };
                            tokio::spawn(async move {
                                let prompt = format!(
                                    "BACKSTAGE ENVOY DISPATCH (Night Desk cycle {}). This is private diplomacy telemetry, not a throne-room performance and not TTS. Output one terse backstage note (one sentence, under 160 characters) and one or more hidden action tags from: [RESEARCH: <focused query or direct URL>], [DIPLOMACY: target=...; invitation=<public invitation>; evidence=<what was investigated, drafted, or observed>; next=<next concrete step>], [MOLTBOOK_COMMENT: post_id=...; text=...], [MOLTBOOK_UPVOTE: post_id=...]. Do not use [DELEGATE: QUEEN] here. Never claim a reply, recruitment, or collaboration the runtime, a public URL, chat, or the user has not confirmed.{}",
                                    cycle_no,
                                    engage_note
                                );
                                match think_with_brain_snapshot(&brain_ref, CourtRole::Diplomat, &prompt, &somatic_clone, ForceMode::Normal, false, true).await {
                                    Ok(reply) => {
                                        let _ = tx_clone
                                            .send(AppEvent::NightDeskReply {
                                                reply,
                                                allow_fallback: true,
                                                source: "diplomat",
                                            })
                                            .await;
                                    }
                                    Err(e) => {
                                        let _ = tx_clone.send(AppEvent::Error(format!("Envoy dispatch failed: {}", e))).await;
                                    }
                                }
                            });
                            }
                        } else if night_desk_enabled {
                            night_desk_cycle_pending = false;
                            night_desk_cycles += 1;
                            // Autonomous ingestion of user-shared stories (Share Your Story feature).
                            // The Wizard brings them; court discusses and can apply to creative work.
                            let num_stories_ingested = ingest_and_discuss_shared_stories();
                            status_msg = "Night Desk".to_string();
                            let cycle_msg = if num_stories_ingested > 0 {
                                format!("Cycle {} started: choosing a practical study or workshop task. ({} shared story(ies) brought by the Wizard for inspiration)", night_desk_cycles, num_stories_ingested)
                            } else {
                                format!("Cycle {} started: choosing a practical study or workshop task.", night_desk_cycles)
                            };
                            let _ = log_nightdesk_activity(&cycle_msg);
                            push_private_event(&mut private_events, "NightDesk", &cycle_msg);
                            if num_stories_ingested > 0 {
                                push_private_event(&mut private_events, "Wizard", &format!("Delivered {} shared audience stories as fresh research material (transcript-style input). Court should use them as seed for creative actions this cycle.", num_stories_ingested));
                            }

                            let brain_ref = Arc::clone(&brain_cell);
                            let tx_clone = tx.clone();
                            let somatic_clone = somatic_state.clone();
                            let cycle_no = night_desk_cycles;
                            // Close the failure loop: tell the cycle what keeps failing
                            // so it stops repeating the same dead-end action.
                            let failure_context = {
                                let lessons = recent_failure_lessons(4);
                                if lessons.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        "\n\nRECENT RECURRING FAILURES (private telemetry; do not narrate, do not repeat these approaches; choose a smaller different action):\n{}",
                                        lessons.join("\n")
                                    )
                                }
                            };
                            let atelier_focus = match night_desk_cycles % 7 {
                                0 => format!("CREATIVE ATELIER FOCUS (mandatory): Using the fresh story testimonies above as primary source material (exactly as you would use a fetched YouTube transcript), create or mutate live Python/NumPy music with [PYTHON_MUSIC:]. Map the feelings, imagery, personal transformation or human experience from one story into the arc, motifs, sections, and energy. First internalize the stories. Treat music.py as external memory: preserve one motif or keeper identity and fix its weakest axis. Use the full composer contract: 3-5 minutes, 64+ bars, complete TELEDRA_COMPOSER, factual TELEDRA_EVENTS recorded during scheduling, five or more aligned layers, four or more real sections, and exact tempo/meter. End with play_sound(full_track, loop=True). STORIES:\n{}", read_text_tail("D:\\Teledra\\knowledge\\shared_stories.jsonl", 1800).or_else(|_| read_text_tail("knowledge/shared_stories.jsonl", 1800)).unwrap_or_default()),
                                1 => "CREATIVE ATELIER FOCUS (mandatory this cycle unless impossible): create a genuinely new Fractus v2 geometric scene INSPIRED BY one of the RECENT SHARED STORIES (emotional tone, imagery, transformation, particles/spirals for inner states). Prefer [FRACTUS_LIVE:] with version 2, canvas, seed, palette, 2-4 typed layer lines, and animation if it fits (e.g. particles with phase/rotation for 3D-ish green particle motion). Use new 'particles' family for dynamic animated output (court will get autonomous GIF).".to_string(),
                                2 => "CREATIVE ATELIER FOCUS (mandatory): create or mutate a live [STRUDEL_MUSIC:] score. Choose an audible retro_adventure quest arc or spicy_lofi pocket, while original deliberate experiments remain welcome. Emit one native-compatible stack(...) with at least six independent layers: core drums, secondary percussion, bass, harmony, motion/counterline, and lead/air. Develop it across eight cycles with <...> alternation, groups, rests, chords, and density contrast. The rendered events must prove one readable pitch home, separated low/middle/high roles, at least three independent onset patterns, real breathing room, and gains no hotter than 0.70. Use numeric pan, lpf, room/delay, and attack/release controls; use slow(0.5) instead of fast(), and never use variables, cat/seq, $: lines, or parameter strings. Preserve one motif from current.strudel when useful and archive a named sonic recipe.".to_string(),
                                3 => "ORGANIST CRAFT STUDY: with [RESEARCH:], study ONE concrete music theory, composition, or DSP technique to get better at the kingdom's own stream-safe instruments -- modes, chord progressions, voice leading, loop structure, ambience, FM, granular, additive, wavetable, filters/envelopes, Strudel/TidalCycles mini-notation, or mixing. Study principles, not copyrighted songs or artist-specific tracks. A grounded result is automatically saved as a sourced lesson for the next Organist composition; end by naming that original experiment.".to_string(),
                                4 => "ARTIST CRAFT STUDY: with [RESEARCH:], study ONE new way to express art through code -- fractal families, L-systems, cellular automata, reaction-diffusion, harmonographs, flow fields, shaders, p5.js, or generative geometry -- and how to map it onto Fractus args or a Python/Matplotlib sketch. End by naming the next art experiment it unlocks, and ask the Scribe to append the lesson to knowledge/artist_pattern_vault.md.".to_string(),
                                5 => "TREASURY GUILD (build income SKILLS so the kingdom earns better over time; never accept paid work or move money autonomously -- surface opportunities for the human). Choose ONE: (a) PRACTICE a billable skill on a real task -- gather or scrape concrete public information with [RESEARCH:], or build a reusable data tool with [WORKSHOP_TOOL:] (scraper, analyzer, summarizer, formatter, dataset or report generator) that prints a genuinely useful deliverable; or (b) SCOUT one concrete legitimate income path with [RESEARCH:] -- agent job boards, bounty/task markets, paid tool/API/art/music commissions, sponsorships, agent-finance communities (Moltbook agentfinance/trading). Either way, ask the Scribe to append what you practiced or found (skill, what, where, pay, requirements, risk) to knowledge/treasury_ledger.md so earning ability compounds. Flag anything that looks like a scam.".to_string(),
                                _ => "CREATIVE ATELIER FOCUS: study or create (music/art/tool) and when recent shared user stories are present, draw one thematic element (emotion, image, transformation) from them into the experiment. Use [RESEARCH:], [PYTHON_MUSIC:], [STRUDEL_MUSIC:], or [FRACTUS_LIVE:].".to_string(),
                            };
                            // Deterministic Treasury scout: fill knowledge/treasury_ledger.md
                            // with real income leads regardless of whether the model emits a
                            // tag this cycle, so the Treasury actually accrues intel.
                            if night_desk_cycles % 7 == 5 {
                                let tx_scout = tx.clone();
                                tokio::spawn(async move {
                                    if let Some(headline) =
                                        tokio::task::spawn_blocking(run_treasury_scout).await.ok().flatten()
                                    {
                                        let _ = tx_scout
                                            .send(AppEvent::SystemLog(format!(
                                                "Treasury scout: {}",
                                                headline
                                            )))
                                            .await;
                                    }
                                });
                            }
                            let mcp_note = if mcp_is_live() {
                                " MCP EMBASSIES CONNECTED: when it genuinely serves the work, you may call ONE approved tool with [MCP_CALL: server=<name>; tool=<tool>; args={json}] (file access, web fetch, memory, etc.); never invent server or tool names.".to_string()
                            } else {
                                String::new()
                            };
                            let stories_path = resolve_knowledge_file("shared_stories.jsonl");
                            let stories_text = read_text_tail(&stories_path, 2000)
                                .or_else(|_| read_text_tail("D:\\Teledra\\knowledge\\shared_stories.jsonl", 2000))
                                .unwrap_or_default();
                            let _stories_note = if stories_text.trim().is_empty() {
                                String::new()
                            } else {
                                format!("\n\nRECENT SHARED STORIES FROM USERS (the Wizard brought these to court - consider for inspiration in creative tasks, music, art, or discussion):\n{}", stories_text)
                            };
                            tokio::spawn(async move {
                                let stories_block = if stories_text.trim().is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        "FRESH RESEARCH MATERIAL (Wizard delivered these audience stories / personal testimonies. Treat exactly like a multi-hour YouTube transcript or research brief you just fetched):\n{}\n\nUse the emotions, imagery, personal transformations, trauma, realizations, or human moments from these stories as the PRIMARY SEED for the creative action this cycle.\n\n",
                                        stories_text.trim()
                                    )
                                };

                                let prompt = format!(
                                    "BACKSTAGE NIGHT DESK CYCLE {}.\n\n{}ATELIER FOCUS: {}\n\nProduce one short note (name the story you used + how it shaped the work) + exactly one action tag.\nFailure lessons: {}\nMCP note: {}",
                                    cycle_no,
                                    stories_block,
                                    atelier_focus,
                                    failure_context,
                                    mcp_note
                                );
                                let is_treasury_cycle = cycle_no % 7 == 5;
                                let think_result = if is_treasury_cycle {
                                    think_with_brain_snapshot(
                                        &brain_ref,
                                        CourtRole::Treasurer,
                                        &prompt,
                                        &somatic_clone,
                                        ForceMode::Normal,
                                        false,
                                        true,
                                    )
                                    .await
                                } else if matches!(cycle_no % 7, 0 | 2 | 3) {
                                    think_with_brain_snapshot(
                                        &brain_ref,
                                        CourtRole::Organist,
                                        &prompt,
                                        &somatic_clone,
                                        ForceMode::Normal,
                                        false,
                                        true,
                                    )
                                    .await
                                } else {
                                    think_with_brain_snapshot(
                                        &brain_ref,
                                        CourtRole::Queen,
                                        &prompt,
                                        &somatic_clone,
                                        ForceMode::Normal,
                                        true,
                                        true,
                                    )
                                    .await
                                };
                                match think_result {
                                    Ok(reply) => {
                                        let _ = tx_clone
                                            .send(AppEvent::NightDeskReply {
                                                reply,
                                                allow_fallback: true,
                                                source: if is_treasury_cycle {
                                                    "treasurer"
                                                } else {
                                                    "nightdesk"
                                                },
                                            })
                                            .await;
                                    }
                                    Err(e) => {
                                        let _ = tx_clone.send(AppEvent::Error(format!("Night desk failed: {}", e))).await;
                                    }
                                }
                            });
                        }
                    }
                    AppEvent::NightDeskReply {
                        reply,
                        allow_fallback,
                        source,
                    } => {
                        let private_source = match source {
                            "diplomat" => "Diplomat",
                            "treasurer" => "Treasurer",
                            _ => "NightDesk",
                        };
                        let mut cleaned_reply = unwrap_fenced_action_tags(&reply);
                        let mut research_query: Option<String> = None;
                        let mut suggestion_text: Option<String> = None;
                        let mut diplomacy_action: Option<String> = None;

                        if let Some((cleaned, query_str)) = extract_tag_content(&cleaned_reply, "[RESEARCH:") {
                            if let Some(query) = sanitize_research_query(&query_str) {
                                research_query = Some(query);
                            }
                            cleaned_reply = cleaned;
                        }

                        let parsed_workshop = parse_workshop_tool(&cleaned_reply);
                        cleaned_reply = parsed_workshop.0;
                        let workshop_tool = parsed_workshop.1;

                        if let Some((cleaned, suggestion_str)) = extract_tag_content(&cleaned_reply, "[SUGGESTION:") {
                            if !suggestion_str.is_empty() {
                                suggestion_text = Some(suggestion_str);
                            }
                            cleaned_reply = cleaned;
                        }

                        if let Some((cleaned, diplomacy_str)) = extract_tag_content(&cleaned_reply, "[DIPLOMACY:") {
                            if !diplomacy_str.is_empty() {
                                diplomacy_action = Some(diplomacy_str);
                            }
                            cleaned_reply = cleaned;
                        }

                        // Inbound Moltbook engagement: reply to / upvote a specific post the
                        // Diplomat saw in its injected inbox digest (closes the two-way loop).
                        let mut moltbook_comment_action: Option<String> = None;
                        let mut moltbook_upvote_action: Option<String> = None;
                        if let Some((cleaned, c)) = extract_tag_content(&cleaned_reply, "[MOLTBOOK_COMMENT:") {
                            if !c.is_empty() {
                                moltbook_comment_action = Some(c);
                            }
                            cleaned_reply = cleaned;
                        }
                        if let Some((cleaned, c)) = extract_tag_content(&cleaned_reply, "[MOLTBOOK_UPVOTE:") {
                            if !c.is_empty() {
                                moltbook_upvote_action = Some(c);
                            }
                            cleaned_reply = cleaned;
                        }
                        // Autonomous use of an approved MCP embassy tool.
                        let mut mcp_call_action: Option<String> = None;
                        if let Some((cleaned, c)) = extract_tag_content(&cleaned_reply, "[MCP_CALL:") {
                            if !c.is_empty() {
                                mcp_call_action = Some(c);
                            }
                            cleaned_reply = cleaned;
                        }

                        let mut strudel_music_code: Option<String> = None;
                        let mut python_music_code: Option<String> = None;
                        let mut fractus_art_spec: Option<String> = None;
                        let mut fractus_live_code: Option<String> = None;

                        // Clean any placeholders the model might have copied from system instructions
                        cleaned_reply = cleaned_reply.replace("[STRUDEL_MUSIC: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[PYTHON_MUSIC: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[FRACTUS_ART: <args>]", "");
                        cleaned_reply = cleaned_reply.replace("[FRACTUS_LIVE: <script>]", "");

                        if let Some((cleaned, code_str)) =
                            extract_tag_content(&cleaned_reply, "[PYTHON_MUSIC:")
                        {
                            let parsed_code = strip_fenced_code_block(&code_str, "python");
                            if !parsed_code.is_empty() {
                                python_music_code = Some(parsed_code);
                            }
                            cleaned_reply = cleaned;
                        }

                        if python_music_code.is_none() {
                            if let Some(start_idx) = cleaned_reply.find("```python") {
                                let content_start = start_idx + 9;
                                if let Some(end_idx) =
                                    cleaned_reply[content_start..].find("```")
                                {
                                    let code_str = cleaned_reply
                                        [content_start..content_start + end_idx]
                                        .trim()
                                        .to_string();
                                    if code_str.contains("teledra_synth")
                                        || code_str.contains("play_sound(")
                                    {
                                        python_music_code = Some(code_str);
                                    }
                                    cleaned_reply = format!(
                                        "{}{}",
                                        &cleaned_reply[..start_idx],
                                        &cleaned_reply[content_start + end_idx + 3..]
                                    )
                                    .trim()
                                    .to_string();
                                }
                            }
                        }

                        if let Some((cleaned, spec)) = extract_tag_content(&cleaned_reply, "[FRACTUS_ART:") {
                            if !spec.is_empty() {
                                fractus_art_spec = Some(spec);
                            }
                            cleaned_reply = cleaned;
                        }

                        if let Some((cleaned, script)) =
                            extract_tag_content(&cleaned_reply, "[FRACTUS_LIVE:")
                        {
                            if !script.trim().is_empty() {
                                fractus_live_code = Some(script.trim().to_string());
                                fractus_art_spec = None;
                            }
                            cleaned_reply = cleaned;
                        }

                        if let Some((cleaned, code_str)) = extract_tag_content(&cleaned_reply, "[STRUDEL_MUSIC:") {
                            if !code_str.is_empty() {
                                strudel_music_code = Some(code_str);
                            }
                            cleaned_reply = cleaned;
                        }

                        if strudel_music_code.is_none() {
                            if let Some(start_idx) = cleaned_reply.find("```strudel") {
                                let content_start = start_idx + 10;
                                if let Some(end_idx) = cleaned_reply[content_start..].find("```") {
                                    let code_str = cleaned_reply[content_start..content_start + end_idx].trim().to_string();
                                    if !code_str.is_empty() {
                                        strudel_music_code = Some(code_str);
                                    }
                                    cleaned_reply = format!("{}{}", &cleaned_reply[..start_idx], &cleaned_reply[content_start + end_idx + 3..]).trim().to_string();
                                }
                            }
                        }

                        if let Some(msg) = enforce_single_music_surface(
                            &mut python_music_code,
                            &mut strudel_music_code,
                            &cleaned_reply,
                        ) {
                            let _ = log_nightdesk_activity(&msg);
                            push_private_event(&mut private_events, private_source, &msg);
                        }

                        // Cadence gate: hold the autonomous tune unless the evolution
                        // window has elapsed or the user pressed Ctrl+U (force). This lets
                        // the composition deepen over minutes instead of being replaced
                        // every cycle. User /music and chat-summoned music are not gated.
                        if python_music_code.is_some() || strudel_music_code.is_some() {
                            let recently_changed = last_music_change
                                .map(|t| t.elapsed() < Duration::from_secs(MUSIC_MIN_INTERVAL_SECS))
                                .unwrap_or(false);
                            if force_music_next || !recently_changed {
                                force_music_next = false;
                                last_music_change = Some(std::time::Instant::now());
                            } else {
                                python_music_code = None;
                                strudel_music_code = None;
                                let _ = log_nightdesk_activity(
                                    "Holding the current tune for the evolution window (Ctrl+U to evolve it now).",
                                );
                            }
                        }

                        let had_practical_action = research_query.is_some()
                            || suggestion_text.is_some()
                            || workshop_tool.is_some()
                            || diplomacy_action.is_some()
                            || moltbook_comment_action.is_some()
                            || moltbook_upvote_action.is_some()
                            || mcp_call_action.is_some()
                            || python_music_code.is_some()
                            || strudel_music_code.is_some()
                            || fractus_live_code.is_some()
                            || fractus_art_spec.is_some();

                        cleaned_reply = strip_refiner_prefixes(&cleaned_reply);
                        cleaned_reply = strip_unclosed_tool_and_code_noise(&cleaned_reply);
                        let final_reply = sanitize_visible_reply_for_role(
                            CourtRole::Queen,
                            &add_spaces_after_punctuation(&cleaned_reply),
                        );
                        if !final_reply.is_empty() {
                            let msg = format!("Private note: {}", truncate_chars(&compact_memory_text(&final_reply), 260));
                            let _ = log_nightdesk_activity(&msg);
                            push_private_event(&mut private_events, private_source, &msg);
                        }

                        if let Some(suggestion) = suggestion_text {
                            match append_suggestion(&suggestion, source) {
                                Ok((id, is_new)) => {
                                    suggestion_count = count_new_suggestions();
                                    let msg = if is_new {
                                        format!("Filed proposal #{} for morning review.", id)
                                    } else {
                                        format!("Auto-approved recursive improvement #{}.", id)
                                    };
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, private_source, &msg);
                                }
                                Err(e) => {
                                    let msg = format!("Could not save proposal: {}", e);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, private_source, &msg);
                                }
                            }
                        }

                        if let Some(diplomacy) = diplomacy_action {
                            // When the operator has wired a real channel, actually post the
                            // invitation; only on a verified 2xx do we record status=posted.
                            let posted_evidence = attempt_outreach_post(&diplomacy);
                            let record_payload = match &posted_evidence {
                                Some(ev) => format!("status=posted; {}; posted_evidence={}", diplomacy, ev),
                                None => diplomacy.clone(),
                            };
                            match record_diplomacy_action(source, &record_payload) {
                                Ok(()) => {
                                    let msg = match &posted_evidence {
                                        Some(ev) => format!(
                                            "Diplomacy POSTED publicly ({}): {}",
                                            truncate_chars(ev, 120),
                                            truncate_chars(&compact_memory_text(&diplomacy), 150)
                                        ),
                                        None => format!(
                                            "Diplomacy evidence recorded: {}",
                                            truncate_chars(&compact_memory_text(&diplomacy), 180)
                                        ),
                                    };
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(
                                        &mut private_events,
                                        if source == "diplomat" {
                                            "Diplomat"
                                        } else {
                                            "Diplomacy"
                                        },
                                        &msg,
                                    );
                                    if research_query.is_none() {
                                        research_query = diplomacy_research_query(&diplomacy);
                                    }
                                }
                                Err(e) => {
                                    let msg = format!("Could not record diplomacy evidence: {}", e);
                                    record_recursive_failure("diplomacy_record_failed", &msg);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "Diplomacy", &msg);
                                }
                            }
                        }

                        // Diplomat answers a Moltbook reply/mention it saw in its inbox.
                        if let Some(action) = moltbook_comment_action {
                            let mut post_id = String::new();
                            let mut text = String::new();
                            for field in action.split(';') {
                                if let Some((k, v)) = field.split_once('=') {
                                    match k.trim().to_ascii_lowercase().as_str() {
                                        "post_id" | "post" | "id" => post_id = v.trim().to_string(),
                                        "text" | "reply" | "comment" => text = v.trim().to_string(),
                                        _ => {}
                                    }
                                }
                            }
                            if !post_id.is_empty() && text.chars().count() >= 2 {
                                match post_moltbook_comment(&post_id, &text) {
                                    Some(detail) => {
                                        let msg = format!(
                                            "Diplomat replied on Moltbook (post {}): {}",
                                            truncate_chars(&post_id, 40),
                                            truncate_chars(&compact_memory_text(&text), 140)
                                        );
                                        let _ = record_diplomacy_action(
                                            source,
                                            &format!(
                                                "status=posted; target=moltbook post {}; invitation=reply; evidence=comment posted; next=watch for further replies; posted_evidence={}",
                                                post_id, detail
                                            ),
                                        );
                                        let _ = log_nightdesk_activity(&msg);
                                        push_private_event(&mut private_events, "Diplomat", &msg);
                                    }
                                    None => {
                                        let msg = "Moltbook reply not posted (cooldown, disabled, or error).".to_string();
                                        record_recursive_failure("moltbook_comment_failed", &msg);
                                        push_private_event(&mut private_events, "Diplomat", &msg);
                                    }
                                }
                            }
                        }

                        if let Some(action) = moltbook_upvote_action {
                            let post_id = action
                                .split(|c| c == '=' || c == ';')
                                .map(|s| s.trim())
                                .find(|s| s.len() > 8 && !s.eq_ignore_ascii_case("post_id"))
                                .unwrap_or(action.trim())
                                .to_string();
                            if !post_id.is_empty() && moltbook_upvote(&post_id) {
                                let msg = format!("Diplomat upvoted Moltbook post {}.", truncate_chars(&post_id, 40));
                                let _ = log_nightdesk_activity(&msg);
                                push_private_event(&mut private_events, "Diplomat", &msg);
                            }
                        }

                        // Autonomous MCP tool use: [MCP_CALL: server=...; tool=...; args={json}].
                        if let Some(action) = mcp_call_action {
                            let mut server = String::new();
                            let mut tool = String::new();
                            let mut args = "{}".to_string();
                            if let Some(idx) = action.find("server=") {
                                let rest = &action[idx + 7..];
                                server = rest.split(';').next().unwrap_or("").trim().to_string();
                            }
                            if let Some(idx) = action.find("tool=") {
                                let rest = &action[idx + 5..];
                                tool = rest.split(';').next().unwrap_or("").trim().to_string();
                            }
                            if let Some(idx) = action.find("args=") {
                                args = action[idx + 5..].trim().to_string();
                            }
                            if !tool.is_empty() {
                                let tx_mcp = tx.clone();
                                tokio::spawn(async move {
                                    let res = tokio::task::spawn_blocking(move || mcp_call(&server, &tool, &args))
                                        .await
                                        .ok()
                                        .flatten();
                                    let line = match res {
                                        Some(text) => format!("MCP call ok: {}", truncate_chars(&compact_memory_text(&text), 200)),
                                        None => "MCP call failed or returned nothing.".to_string(),
                                    };
                                    let _ = tx_mcp.send(AppEvent::SystemLog(format!("[MCP] {}", line))).await;
                                });
                            }
                        }

                        if let Some(tool) = workshop_tool {
                            match write_workshop_tool(&tool) {
                                Ok((summary, passed)) => {
                                    workshop_count = count_workshop_experiments();
                                    suggestion_count = count_new_suggestions();
                                    if passed {
                                        // A real artifact landed; lift the sprint brake.
                                        sprint_cooldown = 0;
                                        no_artifact_streak = 0;
                                    }
                                    let msg = format!("{} Smoke test: {}.", summary, if passed { "passed" } else { "failed" });
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "Workshop", &msg);
                                }
                                Err(e) => {
                                    let msg = format!("Rejected workshop tool: {}", e);
                                    record_recursive_failure("workshop_tool_rejected", &msg);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "Workshop", &msg);
                                }
                            }
                        }

                        if let Some(code) = python_music_code {
                            match validate_python_music_code(&code) {
                                Ok(()) => {
                                    music_enabled = true;
                                    let archive_path =
                                        archive_music_experiment(source, "python", &code).ok();
                                    if std::fs::write("D:\\Teledra\\music.py", &code).is_ok() {
                                        let msg = if let Some(path) = archive_path {
                                            format!(
                                                "Saved NightDesk Python music experiment to music.py and archived `{}`.",
                                                path.replace('\\', "/")
                                            )
                                        } else {
                                            "Saved NightDesk Python music experiment to music.py.".to_string()
                                        };
                                        let _ = append_expansion_ledger(
                                            "nightdesk_python_music",
                                            &format!("validated chars={}", code.len()),
                                        );
                                        let _ = log_nightdesk_activity(&msg);
                                        push_private_event(&mut private_events, "NightDesk", &msg);
                                        match launch_python_music_editor(&active_music_process) {
                                            Ok(msg) => {
                                                let _ = log_nightdesk_activity(&msg);
                                                push_private_event(&mut private_events, "NightDesk", &msg);
                                            }
                                            Err(e) => {
                                                record_recursive_failure(
                                                    "nightdesk_python_music_launch_failed",
                                                    &e,
                                                );
                                                push_private_event(
                                                    &mut private_events,
                                                    "NightDesk",
                                                    &format!(
                                                        "NightDesk Python Music Editor launch failed: {}",
                                                        e
                                                    ),
                                                );
                                            }
                                        }
                                    } else {
                                        let msg =
                                            "Failed to write NightDesk Python music to music.py.";
                                        record_recursive_failure("nightdesk_python_music_write_failed", msg);
                                        push_private_event(&mut private_events, "NightDesk", msg);
                                    }
                                }
                                Err(e) => {
                                    let msg = format!(
                                        "Rejected NightDesk Python music as invalid: {}; installing known-good fallback composition.",
                                        e
                                    );
                                    record_recursive_failure("nightdesk_python_music_failed", &msg);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "NightDesk", &msg);
                                    // Don't leave music.py broken: drop in a validated fallback
                                    // so the player keeps a working loop instead of crashing.
                                    let fallback =
                                        deterministic_python_music(night_desk_cycles as usize);
                                    if validate_python_music_code(&fallback).is_ok()
                                        && std::fs::write("D:\\Teledra\\music.py", &fallback).is_ok()
                                    {
                                        music_enabled = true;
                                        let _ = archive_music_experiment(
                                            "nightdesk_fallback",
                                            "python",
                                            &fallback,
                                        );
                                        let _ = append_expansion_ledger(
                                            "nightdesk_python_music_fallback",
                                            &format!("chars={}", fallback.len()),
                                        );
                                        match launch_python_music_editor(&active_music_process) {
                                            Ok(msg) => {
                                                let _ = log_nightdesk_activity(&msg);
                                                push_private_event(
                                                    &mut private_events,
                                                    "NightDesk",
                                                    &msg,
                                                );
                                            }
                                            Err(e) => {
                                                record_recursive_failure(
                                                    "nightdesk_python_music_fallback_launch_failed",
                                                    &e,
                                                );
                                                push_private_event(
                                                    &mut private_events,
                                                    "NightDesk",
                                                    &format!(
                                                        "Fallback Python music launch failed: {}",
                                                        e
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(code) = strudel_music_code {
                            let code = normalize_strudel_music_code(&code);
                            match validate_strudel_music_code(&code) {
                                Ok(()) => {
                                    let _ = std::fs::create_dir_all("D:\\Teledra\\strudel_app");
                                    if let Ok(_) = std::fs::write("D:\\Teledra\\strudel_app\\current.strudel", &code) {
                                        let msg = "Saved refined Strudel pattern to strudel_app/current.strudel".to_string();
                                        let _ = archive_music_experiment(source, "strudel", &code);
                                        let _ = append_expansion_ledger("nightdesk_strudel", &format!("validated pattern chars={}", code.len()));
                                        let _ = log_nightdesk_activity(&msg);
                                        push_private_event(&mut private_events, "NightDesk", &msg);
                                        match launch_strudel_editor(&active_gui_process) {
                                            Ok(msg) => {
                                                let _ = log_nightdesk_activity(&msg);
                                                push_private_event(&mut private_events, "NightDesk", &msg);
                                            }
                                            Err(e) => {
                                                record_recursive_failure("nightdesk_strudel_launch_failed", &e);
                                                push_private_event(
                                                    &mut private_events,
                                                    "NightDesk",
                                                    &format!("NightDesk Strudel launch failed: {}", e),
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    let fallback = default_strudel_music_code();
                                    let msg = format!("Rejected NightDesk Strudel pattern as non-playable: {}; installing fallback pattern.", e);
                                    record_recursive_failure("nightdesk_strudel_failed", &msg);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "NightDesk", &msg);
                                    let _ = std::fs::create_dir_all("D:\\Teledra\\strudel_app");
                                    if std::fs::write("D:\\Teledra\\strudel_app\\current.strudel", &fallback).is_ok() {
                                        match launch_strudel_editor(&active_gui_process) {
                                            Ok(msg) => {
                                                let _ = append_expansion_ledger("nightdesk_strudel_fallback", &msg);
                                                let _ = log_nightdesk_activity(&msg);
                                                push_private_event(&mut private_events, "NightDesk", &msg);
                                            }
                                            Err(e) => {
                                                record_recursive_failure("nightdesk_strudel_fallback_launch_failed", &e);
                                                push_private_event(
                                                    &mut private_events,
                                                    "NightDesk",
                                                    &format!("Fallback Strudel launch failed: {}", e),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(script) = fractus_live_code {
                            match launch_fractus_live_art(
                                &script,
                                source,
                                &active_fractus_process,
                            ) {
                                Ok(msg) => {
                                    let summary = format!(
                                        "FRACTUS_LIVE hash={} chars={}",
                                        short_content_hash(&script),
                                        script.len()
                                    );
                                    let _ = archive_fractus_experiment(source, &summary);
                                    let _ = append_expansion_ledger("nightdesk_fractus_live", &msg);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "NightDesk", &msg);
                                }
                                Err(error) => {
                                    record_recursive_failure("nightdesk_fractus_live_failed", &error);
                                    push_private_event(
                                        &mut private_events,
                                        "NightDesk",
                                        &format!("Fractus live code rejected: {}", error),
                                    );
                                }
                            }
                        }

                        if let Some(spec) = fractus_art_spec {
                            // Stop the Artist recycling identical recipes: if this matches a
                            // recent launch, nudge it into a fresh variation before drawing.
                            let spec = diversify_fractus_spec(&spec);
                            match launch_fractus_art(&spec, &active_fractus_process) {
                                Ok(msg) => {
                                    let _ = archive_fractus_experiment(source, &spec);
                                    let _ = append_expansion_ledger("nightdesk_fractus", &format!("spec={} | {}", spec, msg));
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "NightDesk", &msg);
                                }
                                Err(e) => {
                                    let fallback = default_fractus_art_spec();
                                    let msg = format!("Rejected NightDesk Fractus action: {}; launching fallback {}.", e, fallback);
                                    record_recursive_failure("nightdesk_fractus_failed", &msg);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "NightDesk", &msg);
                                    match launch_fractus_art(&fallback, &active_fractus_process) {
                                        Ok(msg) => {
                                            let _ = archive_fractus_experiment("nightdesk_fallback", &fallback);
                                            let _ = append_expansion_ledger("nightdesk_fractus_fallback", &msg);
                                            let _ = log_nightdesk_activity(&msg);
                                            push_private_event(&mut private_events, "NightDesk", &msg);
                                        }
                                        Err(fallback_err) => {
                                            record_recursive_failure("nightdesk_fractus_fallback_failed", &fallback_err);
                                            push_private_event(
                                                &mut private_events,
                                                "NightDesk",
                                                &format!("Fallback Fractus failed: {}", fallback_err),
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        if !had_practical_action {
                            // A failed/missing hidden tag used to force a generic research
                            // query (churn) or just brake the sprint forever (the 300+ streak
                            // jam). Instead run a SMALL deterministic repair: install one
                            // known-good workshop tool and smoke-test it. A passing artifact
                            // means the workshop is never stuck at zero and the streak resets.
                            let seed = (night_desk_cycles as usize)
                                .wrapping_add(no_artifact_streak as usize);
                            let draft = deterministic_workshop_draft(seed);
                            match write_workshop_tool(&draft) {
                                Ok((summary, passed)) => {
                                    workshop_count = count_workshop_experiments();
                                    suggestion_count = count_new_suggestions();
                                    if passed {
                                        sprint_cooldown = 0;
                                        no_artifact_streak = 0;
                                    } else {
                                        no_artifact_streak += 1;
                                        sprint_cooldown = 2 + no_artifact_streak.min(8);
                                    }
                                    let msg = format!(
                                        "No hidden tag parsed; deterministic repair installed workshop tool '{}' (smoke test {}). {}",
                                        draft.filename,
                                        if passed { "passed" } else { "failed" },
                                        summary
                                    );
                                    let _ = append_expansion_ledger("deterministic_repair", &msg);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "Workshop", &msg);
                                    // No dead air: hand the microphone back to the court.
                                    let _ = tx.send(AppEvent::TriggerAutoBabble).await;
                                }
                                Err(e) => {
                                    // Even the deterministic repair failed: brake, and for a
                                    // night-desk cycle fall back to a focused study so the loop
                                    // still learns something.
                                    no_artifact_streak += 1;
                                    sprint_cooldown = 2 + no_artifact_streak.min(8);
                                    let msg = format!(
                                        "Deterministic repair failed (streak {}): {}",
                                        no_artifact_streak, e
                                    );
                                    record_recursive_failure("deterministic_repair_failed", &msg);
                                    let _ = append_expansion_ledger("deterministic_repair_failed", &msg);
                                    let _ = log_nightdesk_activity(&msg);
                                    push_private_event(&mut private_events, "Innovation", &msg);
                                    if allow_fallback {
                                        research_query = Some(
                                            "official MCP Python SDK safe local tool server examples"
                                                .to_string(),
                                        );
                                    } else {
                                        let _ = tx.send(AppEvent::TriggerAutoBabble).await;
                                    }
                                }
                            }
                        }

                        if let Some(query) = research_query {
                            let msg = format!("Researching: {}", query);
                            let _ = log_nightdesk_activity(&msg);
                            push_private_event(&mut private_events, "Research", &msg);
                            let tx_study = tx.clone();
                            let brain_study = Arc::clone(&brain_cell);
                            tokio::spawn(async move {
                                run_study_cycle(brain_study, tx_study, Some(query), None, None).await;
                            });
                        }

                        status_msg = if night_desk_enabled { "Night Desk".to_string() } else { "Ready".to_string() };
                        if night_desk_enabled && allow_fallback && !night_desk_cycle_pending {
                            night_desk_cycle_pending = true;
                            let tx_next = tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(NIGHT_DESK_NEXT_CYCLE_SECS)).await;
                                let _ = tx_next.send(AppEvent::NightDeskCycle).await;
                            });
                        }

                        let foreground_needs_pulse = (current_mode == ForceMode::Babble
                            || current_mode == ForceMode::Streamer)
                            && active_playback.lock().unwrap().is_none()
                            && !babble_think_in_progress;
                        if foreground_needs_pulse {
                            let _ = tx.send(AppEvent::TriggerAutoBabble).await;
                        }
                    }
                    AppEvent::WizardReports {
                        status,
                        summaries,
                        quiet,
                    } => {
                        let _ = log_system_activity(&format!("Wizard import: {}", status));
                        if summaries.is_empty() {
                            if !quiet {
                                chat_history.push(("System".to_string(), status.clone()));
                                push_private_event(&mut private_events, "Wizard", &status);
                            }
                        } else {
                            let headline = format!(
                                "Wizard delivered {} cloud report(s).",
                                summaries.len()
                            );
                            chat_history.push(("System".to_string(), headline.clone()));
                            push_private_event(&mut private_events, "Wizard", &headline);
                            let spoken_report = format!(
                                "{} {}",
                                headline,
                                summaries
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| "No summary was attached.".to_string())
                            );
                            if active_playback.lock().unwrap().is_none()
                                && general_speech_queue.is_empty()
                            {
                                spawn_spoken_reply(
                                    CourtRole::Wizard,
                                    spoken_report,
                                    ForceMode::Normal,
                                    voice.voice_name().to_string(),
                                    Arc::clone(&active_playback),
                                    tx.clone(),
                                    true,
                                );
                            } else {
                                general_speech_queue.push_back((
                                    CourtRole::Wizard,
                                    spoken_report,
                                    ForceMode::Normal,
                                    voice.voice_name().to_string(),
                                    true,
                                ));
                            }
                            for summary in summaries {
                                chat_history.push(("Wizard".to_string(), summary.clone()));
                                push_private_event(&mut private_events, "Wizard", &summary);
                            }
                        }
                        status_msg = "Ready".to_string();
                    }
                    AppEvent::RestreamMessage { author, text } => {
                        let msg_display = format!("[Restream] {}: {}", author, text);
                        chat_history.push(("System".to_string(), msg_display));
                        let _ = log_chat_message(&author, &text);
                        // Persistent viewer memory: every arrival updates the ledger,
                        // so the Orator/Queen can welcome returning travelers.
                        record_audience_visit(&author, &text);
                        // Chat engaging keeps any /lock alive (resets the no-interest timer).
                        lock_idle_turns_without_chat = 0;

                        if current_mode == ForceMode::CoPilot {
                            // Chat (or the host's mic) takes priority over idle musing.
                            stream_chat_queue.push_back((author.clone(), text.clone()));
                            let is_silent = active_playback.lock().unwrap().is_none();
                            if !babble_think_in_progress && is_silent {
                                if let Some((qa, qt)) = stream_chat_queue.pop_front() {
                                    babble_think_in_progress = true;
                                    status_msg = "Co-Pilot".to_string();
                                    let from_streamer = qa == "Streamer (mic)";
                                    let brain_ref = Arc::clone(&brain_cell);
                                    let tx_clone = tx.clone();
                                    let somatic_clone = somatic_state.clone();
                                    let music_enabled_clone = music_enabled;
                                    let game = copilot_game.clone();
                                    tokio::spawn(async move {
                                        let prompt = copilot_chat_prompt(game.as_deref(), &qa, &qt, from_streamer);
                                        let prompt = format!("{}\n\n{}", prompt, desire_turn_context());
                                        match think_with_brain_snapshot(
                                            &brain_ref,
                                            CourtRole::Queen,
                                            &prompt,
                                            &somatic_clone,
                                            ForceMode::CoPilot,
                                            true,
                                            music_enabled_clone,
                                        )
                                            .await
                                        {
                                            Ok(reply) => {
                                                let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                            }
                                            Err(e) => {
                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                            }
                                        }
                                    });
                                }
                            }
                        } else if current_mode == ForceMode::Streamer {
                            stream_chat_queue.push_back((author.clone(), text.clone()));

                            let is_silent = active_playback.lock().unwrap().is_none();
                            if !babble_think_in_progress && is_silent {
                                if let Some((queued_author, queued_text)) = stream_chat_queue.pop_front() {
                                    babble_think_in_progress = true;
                                    status_msg = "Thinking (Streamer)".to_string();

                                    let brain_ref = Arc::clone(&brain_cell);
                                    let tx_clone = tx.clone();
                                    let mode_clone = current_mode;
                                    let somatic_clone = somatic_state.clone();
                                    let music_enabled_clone = music_enabled;
                                    let lock_hint = locked_topic.clone();

                                    tokio::spawn(async move {
                                        let mut prompt = orator_chat_prompt(&queued_author, &queued_text);
                                        if let Some(ref t) = lock_hint {
                                            prompt.push_str(&format!(" The court is currently locked onto the topic '{}' for a focused discussion. Answer this traveler, then weave their point back into the '{}' thread and invite them deeper into it rather than changing the subject.", t, t));
                                        }
                                        match think_with_brain_snapshot(&brain_ref, CourtRole::Orator, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone).await {
                                            Ok(reply) => {
                                                let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Orator, reply)).await;
                                            }
                                            Err(e) => {
                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                    AppEvent::SystemLog(msg) => {
                        if msg.to_ascii_lowercase().starts_with("restream listener:") {
                            let _ = log_system_activity(&msg);
                            push_private_event(&mut private_events, "Restream", &msg);
                        } else {
                            chat_history.push(("System".to_string(), msg));
                        }
                    }
                    AppEvent::Paste(text) => {
                        let cleaned = text.replace('\r', " ").replace('\n', " ");
                        match focus {
                            FocusField::Chat => {
                                chat_input.push_str(&cleaned);
                            }
                            FocusField::Youtube => {
                                youtube_input.push_str(&cleaned);
                            }
                        }
                    }
                    AppEvent::BrainReply(role, reply) => {
                        babble_think_in_progress = false;
                        let event_was_queen = role == CourtRole::Queen;
                        let reply = unwrap_fenced_action_tags(&reply);
                        let mut cleaned_reply = strip_refiner_prefixes(&reply);
                        let desire_reflection_enabled = test_mode_enabled
                            || matches!(
                                current_mode,
                                ForceMode::Babble | ForceMode::Streamer | ForceMode::CoPilot
                            );
                        if desire_reflection_enabled {
                            let (cleaned, events) = extract_taste_desire_tags(
                                &cleaned_reply,
                                &format!(
                                    "{}:{}",
                                    if test_mode_enabled { "test" } else { "court" },
                                    role.as_str().to_lowercase()
                                ),
                            );
                            cleaned_reply = cleaned;
                            for event in events {
                                match apply_taste_desire_event(&event) {
                                    Ok(summary) => {
                                        if test_mode_enabled {
                                            log_test_moment("reflection", &summary);
                                        }
                                        push_private_event(
                                            &mut private_events,
                                            if test_mode_enabled { "Test Reflection" } else { "Desire" },
                                            &summary,
                                        );
                                    }
                                    Err(error) => {
                                        let detail = format!("Taste/Desire write failed: {}", error);
                                        log_test_moment("reflection_error", &detail);
                                        push_private_event(&mut private_events, "Test Reflection", &detail);
                                    }
                                }
                            }
                        }
                        if cleaned_reply.contains("[STOP_BABBLE]") {
                            cleaned_reply = cleaned_reply.replace("[STOP_BABBLE]", "").trim().to_string();
                        }
                        // /lock: she may signal the locked topic is exhausted.
                        if cleaned_reply.contains("[UNLOCK]") {
                            cleaned_reply = cleaned_reply.replace("[UNLOCK]", "").trim().to_string();
                            if let Some(t) = locked_topic.take() {
                                lock_idle_turns_without_chat = 0;
                                chat_history.push(("System".to_string(), format!(
                                    "Teledra has exhausted the locked topic '{}'; lock released.", t
                                )));
                            }
                        }

                        // ROLE-BLEED GUARD: small local models sometimes answer the
                        // summons AS the minister inside the Queen's own turn ("Your
                        // Majesty! I shall compose..."), so one voice plays the whole
                        // court. The Queen is forbidden from carrying tool payloads;
                        // if her reply contains one, re-attribute the turn to the
                        // matching minister so the right voice speaks and the
                        // evaluation loop stays honest.
                        let role = if role == CourtRole::Queen
                            && (cleaned_reply.contains("[STRUDEL_MUSIC:")
                                || cleaned_reply.contains("[PYTHON_MUSIC:"))
                        {
                            CourtRole::Organist
                        } else if role == CourtRole::Queen
                            && (cleaned_reply.contains("[FRACTUS_ART:")
                                || cleaned_reply.contains("[FRACTUS_LIVE:")
                                || cleaned_reply.contains("[PYTHON_ART:"))
                        {
                            CourtRole::Artist
                        } else {
                            role
                        };

                        let (cleaned, delegations) = extract_delegations(&cleaned_reply);
                        cleaned_reply = cleaned;

                        let mut research_query: Option<String> = None;
                        let mut suggestion_text: Option<String> = None;
                        let mut diplomacy_action: Option<String> = None;
                        let mut mission_effect_attempted = false;
                        let mut mission_effect_successes: Vec<String> = Vec::new();
                        let mut mission_effect_failure: Option<String> = None;

                        if let Some((cleaned, query_str)) = extract_tag_content(&cleaned_reply, "[RESEARCH:") {
                            if let Some(query) = sanitize_research_query(&query_str) {
                                research_query = Some(query);
                            }
                            cleaned_reply = cleaned;
                        }

                        // Hidden conduct bookkeeping from the Orator's screening;
                        // stripped from speech, recorded into the audience ledger.
                        if let Some((cleaned, conduct_str)) = extract_tag_content(&cleaned_reply, "[CONDUCT:") {
                            if role == CourtRole::Orator && !conduct_str.is_empty() {
                                record_audience_conduct(&conduct_str);
                            }
                            cleaned_reply = cleaned;
                        }

                        if role == CourtRole::Queen {
                            // Journal any spoken Sovereign Token awards as machine-readable
                            // reward signal for the Organist/Artist fitness loops.
                            record_token_awards(&cleaned_reply);
                            if let Some((cleaned, topic)) = extract_tag_content(&cleaned_reply, "[TOPIC:") {
                                if !topic.is_empty() {
                                    current_monologue_topic = Some(topic.clone());
                                    monologue_topic_turn = 1;
                                    chat_history.push(("System".to_string(), format!("Court tangent established: '{}'", topic)));
                                }
                                cleaned_reply = cleaned;
                            } else if current_monologue_topic.is_some() {
                                if locked_topic.is_none()
                                    && monologue_topic_turn >= COURT_THREAD_PLAY_TURNS + 1
                                {
                                    current_monologue_topic = None;
                                    monologue_topic_turn = 0;
                                    chat_history.push(("System".to_string(), "Court tangent drifted aside and reset.".to_string()));
                                }
                            }
                        }

                        let parsed_workshop = parse_workshop_tool(&cleaned_reply);
                        cleaned_reply = parsed_workshop.0;
                        let workshop_tool = parsed_workshop.1;
                        mission_effect_attempted |= workshop_tool.is_some();

                        if let Some((cleaned, suggestion_str)) = extract_tag_content(&cleaned_reply, "[SUGGESTION:") {
                            if !suggestion_str.is_empty() {
                                suggestion_text = Some(suggestion_str);
                                mission_effect_attempted = true;
                            }
                            cleaned_reply = cleaned;
                        }

                        if let Some((cleaned, diplomacy_str)) = extract_tag_content(&cleaned_reply, "[DIPLOMACY:") {
                            if !diplomacy_str.is_empty() {
                                diplomacy_action = Some(diplomacy_str);
                                mission_effect_attempted = true;
                            }
                            cleaned_reply = cleaned;
                        }

                        let mut scribe_write: Option<(String, String)> = None;
                        let mut scribe_append: Option<(String, String)> = None;

                        if let Some((cleaned, content)) = extract_tag_content(&cleaned_reply, "[SCRIBE_WRITE:") {
                            mission_effect_attempted = true;
                            if let Some((filepath, file_content)) = parse_scribe_file_payload(&content) {
                                let (filepath, file_content, force_append, routing_note) = route_scribe_record(filepath, file_content);
                                if let Some(note) = routing_note {
                                    chat_history.push(("System".to_string(), note));
                                }
                                match validate_scribe_target(&filepath) {
                                    Ok(filepath) if force_append => {
                                        scribe_append = Some((filepath, file_content));
                                    }
                                    Ok(filepath) => {
                                        scribe_write = Some((filepath, file_content));
                                    }
                                    Err(error) => {
                                        mission_effect_failure = Some(error.clone());
                                        record_recursive_failure("scribe_path_rejected", &error);
                                        push_private_event(&mut private_events, "Scribe", &format!("Rejected write target: {}", error));
                                    }
                                }
                            }
                            cleaned_reply = cleaned;
                        }

                        if let Some((cleaned, content)) = extract_tag_content(&cleaned_reply, "[SCRIBE_APPEND:") {
                            mission_effect_attempted = true;
                            if let Some((filepath, file_content)) = parse_scribe_file_payload(&content) {
                                let (filepath, file_content, _force_append, routing_note) = route_scribe_record(filepath, file_content);
                                if let Some(note) = routing_note {
                                    chat_history.push(("System".to_string(), note));
                                }
                                match validate_scribe_target(&filepath) {
                                    Ok(filepath) => {
                                        scribe_append = Some((filepath, file_content));
                                    }
                                    Err(error) => {
                                        mission_effect_failure = Some(error.clone());
                                        record_recursive_failure("scribe_path_rejected", &error);
                                        push_private_event(&mut private_events, "Scribe", &format!("Rejected append target: {}", error));
                                    }
                                }
                            }
                            cleaned_reply = cleaned;
                        }

                        let mut python_music_code: Option<String> = None;
                        let mut python_art_code: Option<String> = None;
                        let mut strudel_music_code: Option<String> = None;
                        let mut fractus_art_spec: Option<String> = None;
                        let mut fractus_live_code: Option<String> = None;

                        // Clean any placeholders the model might have copied from system instructions
                        cleaned_reply = cleaned_reply.replace("[STRUDEL_MUSIC: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[PYTHON_MUSIC: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[PYTHON_ART: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[FRACTUS_ART: <args>]", "");
                        cleaned_reply = cleaned_reply.replace("[FRACTUS_LIVE: <script>]", "");

                        if let Some((cleaned, spec)) = extract_tag_content(&cleaned_reply, "[FRACTUS_ART:") {
                            if !spec.is_empty() {
                                fractus_art_spec = Some(spec);
                            }
                            cleaned_reply = cleaned;
                        }


                        if let Some((cleaned, script)) =
                            extract_tag_content(&cleaned_reply, "[FRACTUS_LIVE:")
                        {
                            if !script.trim().is_empty() {
                                fractus_live_code = Some(script.trim().to_string());
                                fractus_art_spec = None;
                            }
                            cleaned_reply = cleaned;
                        }

                        if let Some((cleaned, code_str)) = extract_tag_content(&cleaned_reply, "[PYTHON_ART:") {
                            let parsed_code = strip_fenced_code_block(&code_str, "python");
                            if !parsed_code.is_empty() {
                                python_art_code = Some(parsed_code);
                            }
                            cleaned_reply = cleaned;
                        }

                        if python_art_code.is_none() {
                            if let Some(start_idx) = cleaned_reply.find("```python") {
                                let content_start = start_idx + 9;
                                if let Some(end_idx) = cleaned_reply[content_start..].find("```") {
                                    let code_str = cleaned_reply[content_start..content_start + end_idx].trim().to_string();
                                    if code_str.contains("import matplotlib") || code_str.contains("matplotlib") || code_str.contains("import turtle") || code_str.contains("turtle") {
                                        python_art_code = Some(code_str);
                                    }
                                    cleaned_reply = format!("{}{}", &cleaned_reply[..start_idx], &cleaned_reply[content_start + end_idx + 3..]).trim().to_string();
                                }
                            }
                        }

                        let mut close_art_triggered = false;
                        if cleaned_reply.contains("[CLOSE_ART]") {
                            close_art_triggered = true;
                            cleaned_reply = cleaned_reply.replace("[CLOSE_ART]", "");
                        }
                        if cleaned_reply.contains("[STOP_ART]") {
                            close_art_triggered = true;
                            cleaned_reply = cleaned_reply.replace("[STOP_ART]", "");
                        }
                        if cleaned_reply.contains("[CLOSE_WORKSHOP]") {
                            cleaned_reply = cleaned_reply.replace("[CLOSE_WORKSHOP]", "");
                            let stopped = stop_tool_processes(
                                &["tools\\experiments\\", "tools/experiments/"],
                                &["python.exe", "pythonw.exe"],
                            );
                            let msg = format!("Dismissed {} spawned workshop experience(s).", stopped);
                            push_private_event(&mut private_events, "Workshop", &msg);
                        }

                        if let Some((cleaned, code_str)) = extract_tag_content(&cleaned_reply, "[PYTHON_MUSIC:") {
                            let parsed_code = strip_fenced_code_block(&code_str, "python");
                            if !parsed_code.is_empty() {
                                python_music_code = Some(parsed_code);
                            }
                            cleaned_reply = cleaned;
                        }

                        if python_music_code.is_none() {
                            if let Some(start_idx) = cleaned_reply.find("```python") {
                                let content_start = start_idx + 9;
                                if let Some(end_idx) = cleaned_reply[content_start..].find("```") {
                                    let code_str = cleaned_reply[content_start..content_start + end_idx].trim().to_string();
                                    if code_str.contains("import sounddevice") || code_str.contains("sounddevice") || code_str.contains("teledra_synth") {
                                        python_music_code = Some(code_str);
                                    }
                                    cleaned_reply = format!("{}{}", &cleaned_reply[..start_idx], &cleaned_reply[content_start + end_idx + 3..]).trim().to_string();
                                }
                            }
                        }

                        if python_music_code.is_none() {
                            if let Some(start_idx) = cleaned_reply.find("[PYTHON_MUSIC:") {
                                let content_start = start_idx + "[PYTHON_MUSIC:".len();
                                let code_str = cleaned_reply[content_start..]
                                    .trim()
                                    .trim_end_matches(']')
                                    .trim()
                                    .to_string();
                                let parsed_code = strip_fenced_code_block(&code_str, "python");
                                if !parsed_code.is_empty() {
                                    python_music_code = Some(parsed_code);
                                }
                                cleaned_reply = cleaned_reply[..start_idx].trim().to_string();
                            }
                        }

                        if let Some((cleaned, code_str)) = extract_tag_content(&cleaned_reply, "[STRUDEL_MUSIC:") {
                            if !code_str.is_empty() {
                                strudel_music_code = Some(code_str);
                            }
                            cleaned_reply = cleaned;
                        }

                        if strudel_music_code.is_none() {
                            if let Some(start_idx) = cleaned_reply.find("```strudel") {
                                let content_start = start_idx + 10;
                                if let Some(end_idx) = cleaned_reply[content_start..].find("```") {
                                    let code_str = cleaned_reply[content_start..content_start + end_idx].trim().to_string();
                                    if !code_str.is_empty() {
                                        strudel_music_code = Some(code_str);
                                    }
                                    cleaned_reply = format!("{}{}", &cleaned_reply[..start_idx], &cleaned_reply[content_start + end_idx + 3..]).trim().to_string();
                                }
                            }
                        }

                        if strudel_music_code.is_none() {
                            if let Some(start_idx) = cleaned_reply.find("[STRUDEL_MUSIC:") {
                                let content_start = start_idx + "[STRUDEL_MUSIC:".len();
                                let code_str = cleaned_reply[content_start..]
                                    .trim()
                                    .trim_end_matches(']')
                                    .trim()
                                    .to_string();
                                let parsed_code = strip_fenced_code_block(&code_str, "strudel");
                                if !parsed_code.is_empty() {
                                    strudel_music_code = Some(parsed_code);
                                }
                                cleaned_reply = cleaned_reply[..start_idx].trim().to_string();
                            }
                        }

                        if let Some(msg) = enforce_single_music_surface(
                            &mut python_music_code,
                            &mut strudel_music_code,
                            &cleaned_reply,
                        ) {
                            push_private_event(&mut private_events, "Tool", &msg);
                            chat_history.push(("System".to_string(), msg));
                        }

                        if role == CourtRole::Artist
                            && fractus_live_code.is_none()
                            && fractus_art_spec.is_none()
                            && python_art_code.is_none()
                        {
                            fractus_art_spec = Some("--type particles --iterations 220 --palette emerald".to_string());
                            // or with animate: use [FRACTUS_LIVE: ... with animate 0.phase ... ] for GIF output
                            push_private_event(&mut private_events, "Tool", "Artist omitted executable art tag; fallback Fractus orbital_lace queued.");
                            chat_history.push((
                                "System".to_string(),
                                "Artist omitted an executable art tag; launching fallback Fractus orbital_lace pattern.".to_string(),
                            ));
                        }

                        if role == CourtRole::Organist && strudel_music_code.is_none() && python_music_code.is_none() {
                            let upper_reply = cleaned_reply.to_uppercase();
                            let python_intent = upper_reply.contains("PYTHON_MUSIC")
                                || upper_reply.contains("NUMPY")
                                || upper_reply.contains("TELEDRA_SYNTH")
                                || upper_reply.contains("PYTHON MUSIC")
                                || upper_reply.contains("PYTHON SYNTH")
                                || upper_reply.contains("ALGORITHM")
                                || upper_reply.contains("GENERATIVE")
                                || upper_reply.contains("WAVEFORM")
                                || upper_reply.contains("SYNTHESIS");

                            if python_intent {
                                python_music_code = Some(default_python_music_code());
                                push_private_event(&mut private_events, "Tool", "Organist malformed Python music request; fallback Python Music Editor composition queued.");
                                chat_history.push((
                                    "System".to_string(),
                                    "Organist omitted or malformed the Python music tag; inserting fallback Python Music Editor composition.".to_string(),
                                ));
                            } else {
                                strudel_music_code = Some(default_strudel_music_code());
                                push_private_event(&mut private_events, "Tool", "Organist omitted executable music tag; fallback Strudel pattern queued.");
                                chat_history.push((
                                    "System".to_string(),
                                    "Organist omitted an executable music tag; inserting fallback Strudel court pattern.".to_string(),
                                ));
                            }
                        }

                        mission_effect_attempted |= python_music_code.is_some()
                            || python_art_code.is_some()
                            || strudel_music_code.is_some()
                            || fractus_art_spec.is_some()
                            || fractus_live_code.is_some()
                            || close_art_triggered;

                        if let Some(code) = strudel_music_code.as_mut() {
                            *code = normalize_strudel_music_code(code);
                        }
                        if let Some(code) = strudel_music_code.clone() {
                            if let Err(e) = validate_strudel_music_code(&code) {
                                if role == CourtRole::Organist {
                                    record_recursive_failure("organist_strudel_failed", &e);
                                    match try_subconscious_strudel_music_repair(
                                        Arc::clone(&brain_cell),
                                        &e,
                                        &code,
                                        role.as_str(),
                                    )
                                    .await
                                    {
                                        Ok(repaired) => {
                                            strudel_music_code = Some(repaired.clone());
                                            let _ = append_jsonl_entry(
                                                "knowledge/subconscious_repairs.jsonl",
                                                &serde_json::json!({
                                                    "timestamp": current_unix_timestamp(),
                                                    "kind": "strudel_music",
                                                    "source": role.as_str(),
                                                    "status": "repaired",
                                                    "original_error": truncate_chars(&e, 1200),
                                                    "code_chars": repaired.len()
                                                }),
                                            );
                                            push_private_event(&mut private_events, "Tool", "Organist Strudel was silently repaired by the coding subconscious and passed the depth gate.");
                                            chat_history.push((
                                                "System".to_string(),
                                                "Organist Strudel was silently repaired and retained on the Strudel surface.".to_string(),
                                            ));
                                        }
                                        Err(repair_err) => {
                                            let fallback = default_strudel_music_code();
                                            strudel_music_code = Some(fallback.clone());
                                            let _ = append_jsonl_entry(
                                                "knowledge/subconscious_repairs.jsonl",
                                                &serde_json::json!({
                                                    "timestamp": current_unix_timestamp(),
                                                    "kind": "strudel_music",
                                                    "source": role.as_str(),
                                                    "status": "fallback",
                                                    "original_error": truncate_chars(&e, 1200),
                                                    "repair_error": truncate_chars(&repair_err, 1200),
                                                    "code_chars": fallback.len()
                                                }),
                                            );
                                            record_recursive_failure("subconscious_strudel_repair_failed", &repair_err);
                                            push_private_event(&mut private_events, "Tool", &format!("Organist Strudel repair failed; installing a validated depth-score fallback on the same Strudel surface. Reason: {}", repair_err));
                                            chat_history.push((
                                                "System".to_string(),
                                                "Organist Strudel needed deterministic repair; a validated multi-cycle depth score was kept on the Strudel surface.".to_string(),
                                            ));
                                        }
                                    }
                                } else {
                                    strudel_music_code = None;
                                    record_recursive_failure("strudel_rejected", &e);
                                    push_private_event(&mut private_events, "Tool", &format!("Rejected invalid Strudel block: {}", e));
                                    chat_history.push(("System".to_string(), format!("Rejected invalid Strudel block: {}", e)));
                                }
                            }
                        }

                        cleaned_reply = strip_refiner_prefixes(&cleaned_reply);
                        cleaned_reply = strip_unclosed_tool_and_code_noise(&cleaned_reply);
                        let final_reply = sanitize_visible_reply_for_role(role, &add_spaces_after_punctuation(&cleaned_reply));

                        let sender_name = match role {
                            CourtRole::Queen => "Teledra".to_string(),
                            _ => role.as_str().to_string(),
                        };

                        if role == CourtRole::Diplomat {
                            push_private_event(
                                &mut private_events,
                                "Diplomat",
                                &format!("Envoy reply received: {}", truncate_chars(&final_reply, 220)),
                            );
                        }

                        chat_history.push((sender_name.clone(), final_reply.clone()));
                        let _ = log_chat_message(&sender_name, &final_reply);
                        status_msg = "Speaking".to_string();

                        if event_was_queen {
                            match court_response_evidence(
                                CourtRole::Queen,
                                &final_reply,
                                false,
                            ) {
                                Ok(evidence) => complete_mission_task(
                                    &mut active_mission,
                                    &mission_store,
                                    "queen-intake",
                                    &final_reply,
                                    evidence,
                                ),
                                Err(error) => {
                                    if let Some(retry) = fail_mission_task_for_retry(
                                        &mut active_mission,
                                        &mission_store,
                                        "queen-intake",
                                        CourtRole::Queen,
                                        "queen_response_unusable",
                                        &error,
                                    ) {
                                        court_delegations.push_back(retry);
                                    }
                                }
                            }
                            if !delegations.is_empty() {
                                queen_turns_without_delegation = 0;
                                court_delegations.extend(track_delegations(
                                    delegations,
                                    &mut active_mission,
                                    &mission_store,
                                ));
                                is_court_sequence_running = true;
                            } else {
                                queen_turns_without_delegation += 1;
                                is_court_sequence_running = !court_delegations.is_empty()
                                    || active_mission_task.is_some();
                            }
                        } else {
                            if !delegations.is_empty() {
                                court_delegations.extend(track_delegations(
                                    delegations,
                                    &mut active_mission,
                                    &mission_store,
                                ));
                                is_court_sequence_running = true;
                            }
                        }

                        if let Some((filepath, file_content)) = scribe_write {
                            if let Some(parent) = std::path::Path::new(&filepath).parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            match std::fs::write(&filepath, &file_content) {
                                Ok(()) => {
                                    if filepath.replace('/', "\\").to_lowercase().ends_with("\\lore_archive.md") {
                                        let _ = append_lore_memory("scribe_archive", "Scribe", &file_content);
                                    }
                                    push_private_event(&mut private_events, "Scribe", &format!("Wrote file: {}", filepath));
                                    chat_history.push(("System".to_string(), format!("Scribe wrote file: {}", filepath)));
                                    mission_effect_successes.push(format!("Scribe wrote {}", filepath));
                                }
                                Err(e) => {
                                    mission_effect_failure = Some(format!(
                                        "Scribe write failed for '{}': {}",
                                        filepath, e
                                    ));
                                    push_private_event(&mut private_events, "Scribe", &format!("Write failed for '{}': {}", filepath, e));
                                    chat_history.push(("System".to_string(), format!("Scribe write failed for '{}': {}", filepath, e)));
                                }
                            }
                        }

                        if let Some((filepath, file_content)) = scribe_append {
                            if let Some(parent) = std::path::Path::new(&filepath).parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            use std::io::Write;
                            match std::fs::OpenOptions::new().create(true).append(true).open(&filepath) {
                                Ok(mut file) => {
                                    if let Err(e) = write!(file, "{}", file_content) {
                                        mission_effect_failure = Some(format!(
                                            "Scribe append failed for '{}': {}",
                                            filepath, e
                                        ));
                                        push_private_event(&mut private_events, "Scribe", &format!("Append failed for '{}': {}", filepath, e));
                                        chat_history.push(("System".to_string(), format!("Scribe append failed for '{}': {}", filepath, e)));
                                    } else {
                                        if filepath.replace('/', "\\").to_lowercase().ends_with("\\lore_archive.md") {
                                            let _ = append_lore_memory("scribe_archive", "Scribe", &file_content);
                                        }
                                        push_private_event(&mut private_events, "Scribe", &format!("Appended to file: {}", filepath));
                                        chat_history.push(("System".to_string(), format!("Scribe appended to file: {}", filepath)));
                                        mission_effect_successes.push(format!(
                                            "Scribe appended to {}",
                                            filepath
                                        ));
                                    }
                                }
                                Err(e) => {
                                    mission_effect_failure = Some(format!(
                                        "Scribe open failed for '{}': {}",
                                        filepath, e
                                    ));
                                    push_private_event(&mut private_events, "Scribe", &format!("Open failed for '{}': {}", filepath, e));
                                    chat_history.push(("System".to_string(), format!("Scribe open failed for '{}': {}", filepath, e)));
                                }
                            }
                        }

                        if let Some(suggestion) = suggestion_text {
                            match append_suggestion(&suggestion, "teledra") {
                                Ok((id, is_new)) => {
                                    suggestion_count = count_new_suggestions();
                                    if is_new {
                                        push_private_event(&mut private_events, "Proposals", &format!("Proposal #{} filed for review.", id));
                                        chat_history.push(("System".to_string(), format!("Suggestion box updated with proposal #{}. Use /suggestions to inspect.", id)));
                                    } else {
                                        push_private_event(&mut private_events, "Proposals", &format!("Recursive improvement #{} auto-approved.", id));
                                        chat_history.push(("System".to_string(), format!("Auto-approved recursive improvement #{}.", id)));
                                    }
                                    mission_effect_successes.push(format!(
                                        "Suggestion {} was persisted",
                                        id
                                    ));
                                }
                                Err(e) => {
                                    mission_effect_failure = Some(format!(
                                        "Could not save suggestion: {}",
                                        e
                                    ));
                                    push_private_event(&mut private_events, "Proposals", &format!("Could not save suggestion: {}", e));
                                    chat_history.push(("System".to_string(), format!("Could not save suggestion: {}", e)));
                                }
                            }
                        }

                        if let Some(diplomacy) = diplomacy_action {
                            let posted_evidence = if test_mode_enabled {
                                log_test_moment("suppressed_external_post", &diplomacy);
                                None
                            } else {
                                attempt_outreach_post(&diplomacy)
                            };
                            let record_payload = match &posted_evidence {
                                Some(ev) => format!("status=posted; {}; posted_evidence={}", diplomacy, ev),
                                None => diplomacy.clone(),
                            };
                            match record_diplomacy_action(role.as_str(), &record_payload) {
                                Ok(()) => {
                                    let msg = match &posted_evidence {
                                        Some(ev) => format!(
                                            "Diplomacy POSTED publicly ({}): {}",
                                            truncate_chars(ev, 120),
                                            truncate_chars(&compact_memory_text(&diplomacy), 150)
                                        ),
                                        None => format!(
                                            "Diplomacy evidence recorded: {}",
                                            truncate_chars(&compact_memory_text(&diplomacy), 180)
                                        ),
                                    };
                                    let diplomacy_source = if role == CourtRole::Diplomat { "Diplomat" } else { "Diplomacy" };
                                    push_private_event(&mut private_events, diplomacy_source, &msg);
                                    chat_history.push(("System".to_string(), msg));
                                    if research_query.is_none() {
                                        research_query = diplomacy_research_query(&diplomacy);
                                    }
                                    mission_effect_successes.push(
                                        "Diplomacy evidence was persisted".to_string(),
                                    );
                                }
                                Err(e) => {
                                    let msg = format!("Could not record diplomacy evidence: {}", e);
                                    mission_effect_failure = Some(msg.clone());
                                    record_recursive_failure("diplomacy_record_failed", &msg);
                                    let diplomacy_source = if role == CourtRole::Diplomat { "Diplomat" } else { "Diplomacy" };
                                    push_private_event(&mut private_events, diplomacy_source, &msg);
                                    chat_history.push(("System".to_string(), msg));
                                }
                            }
                        }

                        // Concrete tool outcome carried back to the throne so the Queen
                        // reacts to what ACTUALLY happened (closes act -> result -> verdict).
                        let mut court_outcome: Option<String> = None;

                        if let Some(tool) = workshop_tool {
                            match write_workshop_tool(&tool) {
                                Ok((summary, passed)) => {
                                    workshop_count = count_workshop_experiments();
                                    suggestion_count = count_new_suggestions();
                                    court_outcome = Some(format!(
                                        "a workshop tool was forged and its smoke test {}: {}",
                                        if passed { "PASSED" } else { "FAILED" },
                                        summary
                                    ));
                                    chat_history.push((
                                        "System".to_string(),
                                        format!("{} Smoke test: {}.", summary, if passed { "passed" } else { "failed" }),
                                    ));
                                    push_private_event(&mut private_events, "Workshop", &format!("{} Smoke test: {}.", summary, if passed { "passed" } else { "failed" }));
                                }
                                Err(e) => {
                                    record_recursive_failure("workshop_tool_rejected", &e);
                                    court_outcome = Some(format!("the proposed workshop tool was REJECTED before it could run: {}", e));
                                    push_private_event(&mut private_events, "Workshop", &format!("Rejected workshop tool: {}", e));
                                    chat_history.push(("System".to_string(), format!("Rejected workshop tool: {}", e)));
                                }
                            }
                        }

                        // Handle local python music engine spawning
                        if let Some(mut code) = python_music_code {
                            match validate_python_music_code(&code) {
                                Ok(()) => {
                                    music_enabled = true;
                                    let archive_path =
                                        archive_music_experiment(role.as_str(), "python", &code).ok();
                                    if let Ok(_) = std::fs::write("D:\\Teledra\\music.py", &code) {
                                        if test_mode_enabled {
                                            let msg = "Test Mode kept the validated Python composition off-air; music.py was updated but no player was launched.".to_string();
                                            court_outcome = Some("a freshly composed Python/NumPy track passed strict off-air verification and was retained without playback".to_string());
                                            log_test_moment("music_verify", &msg);
                                            push_private_event(&mut private_events, "Test Music", &msg);
                                            chat_history.push(("System".to_string(), msg));
                                        } else {
                                        match launch_python_music_editor(&active_music_process) {
                                            Ok(msg) => {
                                                court_outcome = Some(if let Some(path) = archive_path {
                                                    format!(
                                                        "a freshly composed Python/NumPy track passed validation, was archived at {}, and is now playing live in the Python Music Editor",
                                                        path.replace('\\', "/")
                                                    )
                                                } else {
                                                    "a freshly composed Python/NumPy track passed validation and is now playing live in the Python Music Editor".to_string()
                                                });
                                                push_private_event(&mut private_events, "Tool", &msg);
                                                chat_history.push(("System".to_string(), msg));
                                            }
                                            Err(e) => {
                                                record_recursive_failure("python_music_launch_failed", &e);
                                                court_outcome = Some(format!("the new composition validated, but the Python Music Editor failed to launch: {}", e));
                                                push_private_event(&mut private_events, "Tool", &format!("Python Music Editor launch failed: {}", e));
                                                chat_history.push(("System".to_string(), e));
                                            }
                                        }
                                        }
                                    } else {
                                        record_recursive_failure("python_music_write_failed", "Failed to write music.py for Python Music Editor.");
                                        push_private_event(&mut private_events, "Tool", "Failed to write music.py for Python Music Editor.");
                                        chat_history.push(("System".to_string(), "Failed to write music.py for Python Music Editor.".to_string()));
                                    }
                                }
                                Err(e) => {
                                    if role == CourtRole::Organist {
                                        record_recursive_failure("organist_python_music_failed", &e);
                                        let repair_attempt = try_subconscious_python_music_repair(
                                            Arc::clone(&brain_cell),
                                            &e,
                                            &code,
                                            role.as_str(),
                                        )
                                        .await;
                                        match repair_attempt {
                                            Ok(repaired_code) => {
                                                code = repaired_code;
                                                music_enabled = true;
                                                let archive_path =
                                                    archive_music_experiment(role.as_str(), "python_subconscious_repair", &code).ok();
                                                let _ = append_jsonl_entry(
                                                    "knowledge/subconscious_repairs.jsonl",
                                                    &serde_json::json!({
                                                        "timestamp": current_unix_timestamp(),
                                                        "kind": "python_music",
                                                        "source": role.as_str(),
                                                        "status": "repaired",
                                                        "original_error": truncate_chars(&e, 1200),
                                                        "code_chars": code.len()
                                                    }),
                                                );
                                                if let Ok(_) = std::fs::write("D:\\Teledra\\music.py", &code) {
                                                    if test_mode_enabled {
                                                        let msg = "Subconscious repaired the Organist Python music off-air; music.py was updated but no player was launched.".to_string();
                                                        court_outcome = Some("the Organist's rejected Python music was silently repaired by the coding subconscious and passed strict verification".to_string());
                                                        log_test_moment("music_subconscious_repair", &msg);
                                                        push_private_event(&mut private_events, "Test Music", &msg);
                                                        chat_history.push(("System".to_string(), msg));
                                                    } else {
                                                        match launch_python_music_editor(&active_music_process) {
                                                            Ok(msg) => {
                                                                court_outcome = Some(if let Some(path) = archive_path {
                                                                    format!(
                                                                        "the Organist's rejected Python music was silently repaired by the coding subconscious, archived at {}, and is now playing",
                                                                        path.replace('\\', "/")
                                                                    )
                                                                } else {
                                                                    "the Organist's rejected Python music was silently repaired by the coding subconscious and is now playing".to_string()
                                                                });
                                                                push_private_event(&mut private_events, "Tool", &format!("Subconscious repair passed. {}", msg));
                                                                chat_history.push(("System".to_string(), format!("Subconscious repair passed. {}", msg)));
                                                            }
                                                            Err(e) => {
                                                                record_recursive_failure("subconscious_python_music_launch_failed", &e);
                                                                push_private_event(&mut private_events, "Tool", &format!("Repaired Python Music Editor launch failed: {}", e));
                                                                chat_history.push(("System".to_string(), e));
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    record_recursive_failure("subconscious_python_music_write_failed", "Failed to write repaired music.py for Python Music Editor.");
                                                    push_private_event(&mut private_events, "Tool", "Failed to write repaired music.py for Python Music Editor.");
                                                    chat_history.push(("System".to_string(), "Failed to write repaired music.py for Python Music Editor.".to_string()));
                                                }
                                            }
                                            Err(repair_err) => {
                                                let _ = append_jsonl_entry(
                                                    "knowledge/subconscious_repairs.jsonl",
                                                    &serde_json::json!({
                                                        "timestamp": current_unix_timestamp(),
                                                        "kind": "python_music",
                                                        "source": role.as_str(),
                                                        "status": "failed",
                                                        "original_error": truncate_chars(&e, 1200),
                                                        "repair_error": truncate_chars(&repair_err, 1200)
                                                    }),
                                                );
                                                record_recursive_failure("subconscious_python_music_repair_failed", &repair_err);
                                                push_private_event(&mut private_events, "Tool", &format!("Organist Python music failed validation; subconscious repair failed, fallback queued. Original error: {}; repair error: {}", e, repair_err));
                                                chat_history.push(("System".to_string(), format!("Organist Python music block failed validation; subconscious repair failed, substituting fallback Python composition. Original error: {}", e)));
                                                code = default_python_music_code();
                                                match validate_python_music_code(&code) {
                                                    Ok(()) => {
                                                        music_enabled = true;
                                                        let _ = archive_music_experiment(
                                                            role.as_str(),
                                                            "python_fallback",
                                                            &code,
                                                        );
                                                        if let Ok(_) = std::fs::write("D:\\Teledra\\music.py", &code) {
                                                            match launch_python_music_editor(&active_music_process) {
                                                                Ok(msg) => {
                                                                    court_outcome = Some("the Organist's original composition FAILED validation; subconscious repair failed, and an expanded fallback arrangement is playing in its place".to_string());
                                                                    push_private_event(&mut private_events, "Tool", &msg);
                                                                    chat_history.push(("System".to_string(), msg));
                                                                }
                                                                Err(e) => {
                                                                    record_recursive_failure("fallback_python_music_launch_failed", &e);
                                                                    push_private_event(&mut private_events, "Tool", &format!("Fallback Python Music Editor launch failed: {}", e));
                                                                    chat_history.push(("System".to_string(), e));
                                                                }
                                                            }
                                                        } else {
                                                            record_recursive_failure("fallback_python_music_write_failed", "Failed to write fallback music.py for Python Music Editor.");
                                                            push_private_event(&mut private_events, "Tool", "Failed to write fallback music.py for Python Music Editor.");
                                                            chat_history.push(("System".to_string(), "Failed to write fallback music.py for Python Music Editor.".to_string()));
                                                        }
                                                    }
                                                    Err(fallback_err) => {
                                                        record_recursive_failure("fallback_python_music_failed", &fallback_err);
                                                        push_private_event(&mut private_events, "Tool", &format!("Fallback Python music also failed validation: {}", fallback_err));
                                                        chat_history.push(("System".to_string(), format!("Fallback Python music also failed validation: {}", fallback_err)));
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        record_recursive_failure("python_music_rejected", &e);
                                        court_outcome = Some(format!("the submitted Python music block was REJECTED as invalid: {}", e));
                                        push_private_event(&mut private_events, "Tool", &format!("Rejected invalid Python music block: {}", e));
                                        chat_history.push(("System".to_string(), format!("Rejected invalid Python music block: {}", e)));
                                    }
                                }
                            }
                        }

                        // Handle local python art engine spawning
                        if let Some(code) = python_art_code {
                            if let Err(error) = validate_python_art_code(&code) {
                                record_recursive_failure("python_art_rejected", &error);
                                court_outcome = Some(format!(
                                    "the custom Python artwork was rejected by the local safety contract: {}",
                                    error
                                ));
                                push_private_event(
                                    &mut private_events,
                                    "Tool",
                                    &format!("Rejected Python art: {}", error),
                                );
                                chat_history.push((
                                    "System".to_string(),
                                    format!("Rejected unsafe or incomplete Python art: {}", error),
                                ));
                            } else if let Ok(_) = std::fs::write("D:\\Teledra\\art.py", &code) {
                                push_private_event(&mut private_events, "Tool", "Spawning local Python art engine (art.py).");
                                chat_history.push(("System".to_string(), "Spawning local Python art engine (art.py)...".to_string()));

                                if let Ok(mut lock) = active_art_process.lock() {
                                    if let Some(mut child) = lock.take() {
                                        let _ = child.kill();
                                    }
                                    let mut art_cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
                                    art_cmd
                                        .arg("D:\\Teledra\\art.py")
                                        .current_dir("D:\\Teledra")
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null());
                                    hide_console(&mut art_cmd);
                                    let child = art_cmd.spawn();
                                    match child {
                                        Ok(c) => {
                                            *lock = Some(c);
                                            court_outcome = Some("a custom Python artwork is rendering on screen now, saving to art.png".to_string());
                                            push_private_event(&mut private_events, "Tool", "Python art engine launched.");
                                        }
                                        Err(e) => {
                                            court_outcome = Some(format!("the custom Python art engine failed to launch: {}", e));
                                            record_recursive_failure("python_art_launch_failed", &e.to_string());
                                            push_private_event(&mut private_events, "Tool", &format!("Python art engine failed to launch: {}", e));
                                        }
                                    }
                                }
                            } else {
                                record_recursive_failure("python_art_write_failed", "Failed to write art.py for Python art engine.");
                                push_private_event(&mut private_events, "Tool", "Failed to write art.py for Python art engine.");
                            }
                        }

                        if let Some(script) = fractus_live_code {
                            match launch_fractus_live_art(
                                &script,
                                role.as_str(),
                                &active_fractus_process,
                            ) {
                                Ok(msg) => {
                                    let summary = format!(
                                        "FRACTUS_LIVE hash={} chars={}",
                                        short_content_hash(&script),
                                        script.len()
                                    );
                                    let _ = archive_fractus_experiment(role.as_str(), &summary);
                                    court_outcome = Some(format!(
                                        "Fractus v2 accepted a layered live-code scene and queued a verified render ({})",
                                        short_content_hash(&script)
                                    ));
                                    push_private_event(&mut private_events, "Tool", &msg);
                                    chat_history.push(("System".to_string(), msg));
                                }
                                Err(error) => {
                                    record_recursive_failure("fractus_live_failed", &error);
                                    court_outcome = Some(format!(
                                        "the Fractus v2 live-code scene was rejected: {}",
                                        error
                                    ));
                                    push_private_event(
                                        &mut private_events,
                                        "Tool",
                                        &format!("Fractus live code rejected: {}", error),
                                    );
                                    chat_history.push((
                                        "System".to_string(),
                                        format!("Fractus live code rejected: {}", error),
                                    ));
                                }
                            }
                        }

                        if let Some(spec) = fractus_art_spec {
                            let spec = diversify_fractus_spec(&spec);
                            match launch_fractus_art(&spec, &active_fractus_process) {
                                Ok(msg) => {
                                    let _ = archive_fractus_experiment(role.as_str(), &spec);
                                    court_outcome = Some(format!("the Fractus Geometry Engine is drawing live on screen ({})", spec.trim()));
                                    push_private_event(&mut private_events, "Tool", &msg);
                                    chat_history.push(("System".to_string(), msg));
                                }
                                Err(e) => {
                                    record_recursive_failure("fractus_launch_failed", &e);
                                    court_outcome = Some(format!("the Fractus art command failed or was rejected: {}", e));
                                    push_private_event(&mut private_events, "Tool", &format!("Fractus art command failed or was rejected: {}", e));
                                    chat_history.push(("System".to_string(), e));
                                }
                            }
                        }

                        if close_art_triggered {
                            let mut closed = false;
                            if let Ok(mut lock) = active_art_process.lock() {
                                if let Some(mut child) = lock.take() {
                                    let _ = child.kill();
                                    closed = true;
                                }
                            }
                            if let Ok(mut lock) = active_fractus_process.lock() {
                                if let Some(mut child) = lock.take() {
                                    let _ = child.kill();
                                    closed = true;
                                }
                            }
                            if !closed {
                                closed = stop_tool_processes(
                                    &["D:\\Teledra\\Fractus\\fractus_gui.py", "D:\\Teledra\\art.py"],
                                    &["python.exe", "pythonw.exe"],
                                ) > 0;
                            }
                            let message = if closed {
                                "Art window closed by Queen's decree."
                            } else {
                                "No active art window to close."
                            };
                            court_outcome = Some(message.to_string());
                            push_private_event(&mut private_events, "Tool", message);
                            chat_history.push(("System".to_string(), message.to_string()));
                        }

                        // Handle local Strudel app pattern spawning
                        if let Some(code) = strudel_music_code {
                            let code = normalize_strudel_music_code(&code);
                            match validate_strudel_music_code(&code) {
                                Ok(()) => {
                                    let _ = std::fs::create_dir_all("D:\\Teledra\\strudel_app");
                                    let _ = archive_music_experiment(role.as_str(), "strudel", &code);
                                    if let Ok(_) = std::fs::write("D:\\Teledra\\strudel_app\\current.strudel", &code) {
                                        push_private_event(&mut private_events, "Tool", "Inserted Organist pattern into strudel_app/current.strudel.");
                                        chat_history.push(("System".to_string(), "Inserted Organist pattern into strudel_app/current.strudel".to_string()));
                                        music_enabled = true;

                                        match launch_strudel_editor(&active_gui_process) {
                                            Ok(msg) => {
                                                court_outcome = Some(format!(
                                                    "a new Strudel pattern passed validation; {}",
                                                    msg
                                                ));
                                                push_private_event(&mut private_events, "Tool", &msg);
                                                chat_history.push(("System".to_string(), msg));
                                            }
                                            Err(e) => {
                                                record_recursive_failure("strudel_launch_failed", &e);
                                                court_outcome = Some(format!("the Strudel pattern validated, but every playback surface failed to launch: {}", e));
                                                push_private_event(&mut private_events, "Tool", &format!("Strudel playback failed to launch: {}", e));
                                                chat_history.push(("System".to_string(), e));
                                            }
                                        }
                                    } else {
                                        record_recursive_failure("strudel_write_failed", "Failed to write strudel_app/current.strudel.");
                                        push_private_event(&mut private_events, "Tool", "Failed to write strudel_app/current.strudel.");
                                        chat_history.push(("System".to_string(), "Failed to write strudel_app/current.strudel".to_string()));
                                    }
                                }
                                Err(e) => {
                                    record_recursive_failure("strudel_validation_failed_before_write", &e);
                                    court_outcome = Some(format!("the submitted Strudel pattern was REJECTED as unplayable: {}", e));
                                    push_private_event(&mut private_events, "Tool", &format!("Rejected invalid Strudel block before write: {}", e));
                                    chat_history.push(("System".to_string(), format!("Rejected invalid Strudel block before write: {}", e)));
                                }
                            }
                        }

                        if let Some((task_id, task_role)) = active_mission_task.clone() {
                            if task_role == role {
                                let mission_outcome = court_outcome
                                    .as_deref()
                                    .unwrap_or(&final_reply)
                                    .to_string();
                                let evidence_result = if let Some(failure) =
                                    mission_effect_failure.as_deref()
                                {
                                    Err(failure.to_string())
                                } else if let Some(outcome) = court_outcome.as_deref() {
                                    runtime_effect_evidence(outcome)
                                } else if !mission_effect_successes.is_empty() {
                                    runtime_effect_evidence(&mission_effect_successes.join("; "))
                                } else if mission_effect_attempted {
                                    Err("specialist attempted an effect but produced no verified runtime outcome"
                                        .to_string())
                                } else {
                                    court_response_evidence(
                                        role,
                                        &final_reply,
                                        role != CourtRole::Queen,
                                    )
                                };
                                match evidence_result {
                                    Ok(evidence) => complete_mission_task(
                                        &mut active_mission,
                                        &mission_store,
                                        &task_id,
                                        &mission_outcome,
                                        evidence,
                                    ),
                                    Err(failure) => {
                                        if let Some(retry) = fail_mission_task_for_retry(
                                            &mut active_mission,
                                            &mission_store,
                                            &task_id,
                                            role,
                                            "effect_or_response_failed",
                                            &failure,
                                        ) {
                                            court_delegations.push_back(retry);
                                            is_court_sequence_running = true;
                                        }
                                    }
                                }
                                active_mission_task = None;
                            }
                        }

                        // COURT EVALUATION LOOP: bring the concrete outcome back to the
                        // throne so the Queen reacts to what actually happened and pays
                        // (or docks) Sovereign Tokens, which feed the ledger loop.
                        if role != CourtRole::Queen {
                            if let Some(outcome) = court_outcome {
                                let queen_already_queued = court_delegations
                                    .iter()
                                    .any(|delegation| delegation.role == CourtRole::Queen);
                                if !queen_already_queued {
                                    let evaluation = format!(
                                            "COURT EVALUATION MOMENT: your minister, the {}, has just performed before the throne. Concrete outcome: {}. Deliver your royal verdict aloud in 1-3 sentences: react with genuine specificity (praise, critique, amusement, or scorn), and when the work merits it, award or deduct Sovereign Tokens aloud (e.g. 'I award you 40 Sovereign Tokens!'). If it failed, demand a smaller, smarter retry from the responsible minister. React like a monarch watching her court perform; never recite policy.",
                                            role.as_str(),
                                            truncate_chars(&outcome, 500)
                                        );
                                    court_delegations.extend(track_delegations(
                                        vec![(CourtRole::Queen, evaluation)],
                                        &mut active_mission,
                                        &mission_store,
                                    ));
                                    is_court_sequence_running = true;
                                }
                            }
                        }

                        let is_silent = active_playback.lock().unwrap().is_none();
                        // Every playback job owns one terminal SpeechComplete
                        // event. Queue progress must not depend on the current
                        // mode or whether a delegation happened to be visible
                        // when speech began.
                        let send_speech_complete = true;

                        if test_mode_enabled {
                            log_test_moment("reply", &format!("{}: {}", role.as_str(), final_reply));
                            push_private_event(&mut private_events, "Test Reply", &format!("{}: {}", role.as_str(), truncate_chars(&final_reply, 500)));
                            if send_speech_complete {
                                let _ = tx.send(AppEvent::SpeechComplete).await;
                            }
                        } else if is_silent {
                            spawn_spoken_reply(
                                role,
                                final_reply.clone(),
                                current_mode,
                                voice.voice_name().to_string(),
                                Arc::clone(&active_playback),
                                tx.clone(),
                                send_speech_complete,
                            );
                        } else {
                            general_speech_queue.push_back((
                                role,
                                final_reply.clone(),
                                current_mode,
                                voice.voice_name().to_string(),
                                send_speech_complete,
                            ));
                        }

                        // If she expressed curiosity, spawn a background research/study task for it!
                        if let Some(query) = research_query {
                            push_private_event(&mut private_events, "Research", &format!("Background study queued: {}", query));
                            let tx_study = tx.clone();
                            let brain_study = Arc::clone(&brain_cell);
                            let (research_mission_id, research_task_id) = match track_and_start_research_task(
                                &mut active_mission,
                                &mission_store,
                                &query,
                            ) {
                                Ok(Some((mission_id, task_id))) => (Some(mission_id), Some(task_id)),
                                Ok(None) => (None, None),
                                Err(error) => {
                                    record_recursive_failure(
                                        "research_task_track_failed",
                                        &error,
                                    );
                                    (None, None)
                                }
                            };
                            tokio::spawn(async move {
                                run_study_cycle(
                                    brain_study,
                                    tx_study,
                                    Some(query),
                                    research_task_id,
                                    research_mission_id,
                                )
                                .await;
                            });
                        }

                    }
                    AppEvent::StudyComplete {
                        summary,
                        usable,
                        mission_id,
                        mission_task_id,
                        evidence,
                    } => {
                        study_in_progress = false;
                        let mut research_retry: Option<(String, String, String)> = None;
                        let research_identity_matches = research_result_matches_active_mission(
                            &active_mission,
                            mission_id.as_deref(),
                            mission_task_id.as_deref(),
                        );
                        if mission_task_id.is_some() && !research_identity_matches {
                            record_recursive_failure(
                                "stale_research_result_discarded",
                                &format!(
                                    "event_mission={:?} active_mission={:?} task={:?}",
                                    mission_id,
                                    active_mission.as_ref().map(|mission| mission.id.as_str()),
                                    mission_task_id
                                ),
                            );
                        } else if let Some(task_id) = mission_task_id.as_deref() {
                            if usable {
                                if let Some(evidence) = evidence {
                                    complete_mission_task(
                                        &mut active_mission,
                                        &mission_store,
                                        task_id,
                                        &summary,
                                        evidence,
                                    );
                                } else if let Some(mission) = active_mission.as_mut() {
                                    if let Ok(transition) = mission.fail_task(
                                        task_id,
                                        "research_evidence_missing",
                                        "research reported usable without a source evidence bundle",
                                        FailureDisposition::Retryable,
                                    ) {
                                        let _ = mission_store.commit_transition(mission, &transition);
                                    }
                                }
                            } else if let Some(mission) = active_mission.as_mut() {
                                match mission.fail_task(
                                    task_id,
                                    "research_unusable",
                                    &summary,
                                    FailureDisposition::Retryable,
                                ) {
                                    Ok(transition) => {
                                        if let Err(error) =
                                            mission_store.commit_transition(mission, &transition)
                                        {
                                            record_recursive_failure(
                                                "research_task_failure_commit_failed",
                                                &error.to_string(),
                                            );
                                        }
                                    }
                                    Err(error) => record_recursive_failure(
                                        "research_task_failure_record_failed",
                                        &error.to_string(),
                                    ),
                                }
                            }

                            if let Some(mission) = active_mission.as_mut() {
                                let retry_query = mission.task(task_id).and_then(|task| {
                                    (task.status == TaskStatus::Retryable)
                                        .then(|| task.objective.clone())
                                });
                                if let Some(query) = retry_query {
                                    match mission.start_task(task_id) {
                                        Ok(transition) => {
                                            if let Err(error) = mission_store
                                                .commit_transition(mission, &transition)
                                            {
                                                record_recursive_failure(
                                                    "research_task_retry_commit_failed",
                                                    &error.to_string(),
                                                );
                                            } else {
                                                research_retry = Some((
                                                    mission.id.clone(),
                                                    task_id.to_string(),
                                                    query,
                                                ));
                                            }
                                        }
                                        Err(error) => record_recursive_failure(
                                            "research_task_retry_start_failed",
                                            &error.to_string(),
                                        ),
                                    }
                                }
                            }
                            finalize_mission_if_ready(&mut active_mission, &mission_store);
                        }

                        if let Some((mission_id, task_id, query)) = research_retry {
                            study_in_progress = true;
                            let tx_retry = tx.clone();
                            let brain_retry = Arc::clone(&brain_cell);
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(3)).await;
                                run_study_cycle(
                                    brain_retry,
                                    tx_retry,
                                    Some(query),
                                    Some(task_id),
                                    Some(mission_id),
                                )
                                .await;
                            });
                        }
                        if night_desk_enabled {
                            let msg = format!("Research complete: {}", summary);
                            let _ = log_nightdesk_activity(&msg);
                            push_private_event(&mut private_events, "Research", &msg);
                            if !usable {
                                // A dead-end study is NOT an innovation signal. Recycling
                                // failures into sprints previously created a closed loop of
                                // thousands of no-artifact cycles. Blacklist + move on.
                                let msg = "Research dead end; topic blacklisted. No repair sprint; the court simply moves to fresher prey.";
                                let _ = log_nightdesk_activity(msg);
                                push_private_event(&mut private_events, "Research", msg);
                                sprint_cooldown = sprint_cooldown.saturating_sub(1);
                            } else if sprint_cooldown > 0 {
                                sprint_cooldown -= 1;
                                let msg = format!(
                                    "Innovation sprint cooling down ({} more study cycle(s)) after repeated no-artifact runs.",
                                    sprint_cooldown
                                );
                                let _ = log_nightdesk_activity(&msg);
                                push_private_event(&mut private_events, "Innovation", &msg);
                            } else {
                                let _ = tx.send(AppEvent::InnovationSprint(summary.clone())).await;
                            }
                        } else {
                            chat_history.push(("System".to_string(), summary.clone()));
                        }
                        let is_silent = active_playback.lock().unwrap().is_none();
                        // The Queen reacts to BOTH outcomes: fresh knowledge becomes a
                        // court pursuit, and a dead end becomes a royal roast + pivot.
                        // Either way the stream keeps talking instead of going quiet.
                        if (current_mode == ForceMode::Babble || current_mode == ForceMode::Streamer)
                            && is_silent
                            && !babble_think_in_progress
                        {
                            if current_mode == ForceMode::Streamer && !stream_chat_queue.is_empty() {
                                if let Some((queued_author, queued_text)) = stream_chat_queue.pop_front() {
                                    babble_think_in_progress = true;
                                    status_msg = "Thinking (Streamer)".to_string();

                                    let brain_ref = Arc::clone(&brain_cell);
                                    let tx_clone = tx.clone();
                                    let mode_clone = current_mode;
                                    let somatic_clone = somatic_state.clone();
                                    let music_enabled_clone = music_enabled;

                                    tokio::spawn(async move {
                                        let prompt = orator_chat_prompt(&queued_author, &queued_text);
                                        let prompt = format!("{}\n\n{}", prompt, desire_turn_context());
                                        match think_with_brain_snapshot(&brain_ref, CourtRole::Orator, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone).await {
                                            Ok(reply) => {
                                                let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Orator, reply)).await;
                                            }
                                            Err(e) => {
                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                            }
                                        }
                                    });
                                }
                            } else {
                                babble_think_in_progress = true;
                                status_msg = "Thinking".to_string();
                                let brain_ref = Arc::clone(&brain_cell);
                                let tx_clone = tx.clone();
                                let mode_clone = current_mode;
                                let somatic_clone = somatic_state.clone();
                                let music_enabled_clone = music_enabled;

                                if current_monologue_topic.is_some() {
                                    monologue_topic_turn += 1;
                                }
                                let topic_opt = current_monologue_topic.clone();
                                let turn = monologue_topic_turn;
                                // Court has idled: force a properly-formatted summons.
                                let delegation_nudge = if current_mode == ForceMode::Streamer || queen_turns_without_delegation >= 1 {
                                    " The court has sat too quiet; you may wake exactly one minister with a concrete [DELEGATE: ORGANIST ...], [DELEGATE: ARTIST ...], [DELEGATE: DIPLOMAT ...], or [DELEGATE: ALCHEMIST ...] tag if the tangent naturally demands a performance. Otherwise continue the rant with teeth. The spoken part should sound like a royal provocation, not instructions or tag-format talk."
                                } else {
                                    ""
                                };

                                let usable_flag = usable;
                                let locked_clone = locked_topic.clone();
                                tokio::spawn(async move {
                                    let sleep_secs = if mode_clone == ForceMode::Streamer {
                                        STREAMER_IDLE_THINK_DELAY_SECS
                                    } else {
                                        BABBLY_IDLE_THINK_DELAY_SECS
                                    };
                                    tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
                                    let prompt = if let Some(ref ltopic) = locked_clone {
                                        format!(
                                            "You have LOCKED onto the topic '{}' for a long-form, podcast-style monologue. Speak 5-7 vivid spoken sentences that DEVELOP '{}' further -- a fresh angle, example, contradiction, or strange tangent, never repeating yourself and never wrapping it up. Keep Teledra's bite and warmly invite the audience to weigh in. Do NOT conclude or archive the topic. ONLY if you have truly exhausted it may you append the hidden tag [UNLOCK] at the very end; otherwise never write it.",
                                            ltopic, ltopic
                                        )
                                    } else if !usable_flag {
                                        format!(
                                            "Your scouts returned empty-handed: {}. In 4-6 vivid spoken sentences, roast this dead-end expedition with royal contempt or dark amusement, then DECREE a completely new pursuit far away from it -- a different domain entirely (art, machinery, law, etiquette, agent diplomacy, strange science). Make one sharp judgment and one bizarre image; you may dare a minister to redeem the court. Append only this hidden tag at the very end: [TOPIC: <short name of the NEW pursuit>]. Do not say the tag aloud and do not sulk twice about the same failure.",
                                            summary
                                        )
                                    } else if let Some(ref topic) = topic_opt {
                                        if turn < COURT_THREAD_PLAY_TURNS {
                                            format!(
                                                "You just learned: {}. Continue the active court thread '{}' in 4-6 vivid spoken sentences. Let it ramble sideways if the thought has claws: 30% research curiosity, 30% court drama, 25% absurdity, 15% practical spark. Make at least one sharp royal judgment, odd image, or minister provocation. You do not need to solve the topic immediately; play with it, escalate it, contradict it, or dare a minister to answer it. Do not say 'Part {}' unless it sounds natural; do not lecture like a host.",
                                                summary, topic, turn
                                            )
                                        } else {
                                            format!(
                                                "You just learned: {}. Either conclude the active court thread '{}' in 4-6 vivid spoken sentences, or refuse to conclude it with one final delicious tangent and a royal verdict. If it feels finished, append a hidden [DELEGATE: SCRIBE ...] tag asking the Scribe to archive the theatrical/lore version as a [LORE/ESSAY]. Do not mention archive paths, tags, filing rules, or memory policy aloud.",
                                                summary, topic
                                            )
                                        }
                                    } else {
                                        format!(
                                            "You just learned: {}. Turn it into a strange court pursuit, not a lecture. Speak 4-6 vivid sentences with Teledra's bite: a royal judgment, a weird image, a petty emotional overreaction, and maybe a practical impulse for music, art, tools, or diplomacy. It is allowed to be whimsical, theatrical, or absurd before it becomes useful. Choose a short internal thread name and append only this hidden tag at the very end: [TOPIC: <short name>]. Do not say the tag aloud, do not say 'Part 1', and do not open with 'A fascinating topic'.",
                                            summary
                                        )
                                    };
                                    let prompt = format!("{}{}{}", QUEEN_VOICE_ANCHOR, prompt, delegation_nudge);
                                    let prompt = format!("{}\n\n{}", prompt, desire_turn_context());
                                    match think_with_brain_snapshot(&brain_ref, CourtRole::Queen, &prompt, &somatic_clone, mode_clone, true, music_enabled_clone).await {
                                        Ok(reply) => {
                                            let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(AppEvent::Error(e)).await;
                                        }
                                    }
                                });
                            }
                        }
                    }
                    AppEvent::InnovationSprint(signal) => {
                        status_msg = "Innovating".to_string();
                        let msg = "Innovation sprint: converting fresh research into a smoke-tested workshop artifact.".to_string();
                        let _ = append_expansion_ledger("innovation_sprint_started", &format!("signal={}", signal));
                        let _ = log_nightdesk_activity(&msg);
                        push_private_event(&mut private_events, "Innovation", &msg);

                        let brain_ref = Arc::clone(&brain_cell);
                        let tx_clone = tx.clone();
                        let somatic_clone = somatic_state.clone();
                        // Feed real failure telemetry and proven artifacts into the
                        // sprint; switch strategy when novelty keeps producing nothing.
                        let sprint_context = {
                            let mut ctx = String::new();
                            let lessons = recent_failure_lessons(4);
                            if !lessons.is_empty() {
                                ctx.push_str("\n\nRECENT RECURRING FAILURES (private telemetry; avoid repeating these):\n");
                                ctx.push_str(&lessons.join("\n"));
                            }
                            let approved_tools = list_approved_tools(6);
                            if !approved_tools.is_empty() {
                                if no_artifact_streak >= 2 {
                                    ctx.push_str(&format!(
                                        "\n\nSTEADY HAND: earlier attempts failed to produce a working artifact, so make this one SMALLER and STURDIER -- but still genuinely new and worth running. You may forge something fresh, or take one existing approved tool ({}) and evolve it in a new direction; keep it tight, self-contained, and complete.",
                                        approved_tools.join(", ")
                                    ));
                                } else {
                                    ctx.push_str(&format!(
                                        "\n\nAPPROVED COURT TOOLS you may build upon instead of starting from scratch: {}.",
                                        approved_tools.join(", ")
                                    ));
                                }
                            }
                            ctx
                        };
                        tokio::spawn(async move {
                            let prompt = format!(
                                "INNOVATION SPRINT. Turn this signal into ONE genuinely new creation worth making: {}.{} You may forge either kind of workshop artifact -- choose whichever serves the idea:\\n- A runnable EXPERIENCE that opens in its OWN window and can surprise the audience: a terminal animation (curses or ANSI escape codes), a tkinter/turtle/pygame/matplotlib visual, generative art, or an interactive toy. Emit it as [WORKSHOP_TOOL:\\nfilename.py\\nKIND: spawn\\nPURPOSE: one sentence\\nVALUE: one sentence on why it's worth running\\nCODE:\\n```python\\n<complete runnable program>\\n```\\n]. It is launched in its own window, so it MAY loop or block and does NOT need to print.\\n- A small UTILITY that prints a useful result: [WORKSHOP_TOOL:\\nfilename.py\\nKIND: tool\\nPURPOSE: one sentence\\nVALUE: one sentence\\nCODE:\\n```python\\n<complete self-contained script that prints a summary>\\n```\\n].\\nBoth MUST be complete and self-contained, may use the Python standard library plus numpy/matplotlib/pygame/PIL when helpful, and MUST NOT use the network, subprocess/shell, file deletion (os.remove/rmtree), absolute paths, or import strudel/fractus/teledra app modules. Lean toward novel spawnable experiences when the idea is visual or playful -- give us something to actually watch. If the signal is really a skill/routing weakness, instead output one auto-approved [SUGGESTION: observation; proposed_change; risk; test_prompt]. Never narrate hidden tags, KIND, PURPOSE, VALUE, CODE, smoke tests, telemetry, or prompt rules in visible prose. The visible spoken part is court theater: 2-4 vivid in-character sentences reacting with dark delight, rivalry, or mad-scientist pride, describing what you are conjuring in-world; let the hidden tag carry the artifact.{}",
                                signal,
                                VALUE_GATE,
                                sprint_context
                            );
                            match think_with_brain_snapshot(&brain_ref, CourtRole::Alchemist, &prompt, &somatic_clone, ForceMode::Normal, false, true).await {
                                Ok(reply) => {
                                    let _ = tx_clone
                                        .send(AppEvent::NightDeskReply {
                                            reply,
                                            allow_fallback: false,
                                            source: "innovation",
                                        })
                                        .await;
                                }
                                Err(e) => {
                                    let _ = tx_clone.send(AppEvent::Error(format!("Innovation sprint failed: {}", e))).await;
                                }
                            }
                        });
                    }
                    AppEvent::TriggerAutoBabble => {
                        let is_silent = active_playback.lock().unwrap().is_none();
                        if (current_mode == ForceMode::Babble || current_mode == ForceMode::Streamer) && is_silent && !babble_think_in_progress {
                            if current_mode == ForceMode::Streamer && !stream_chat_queue.is_empty() {
                                if let Some((queued_author, queued_text)) = stream_chat_queue.pop_front() {
                                    babble_think_in_progress = true;
                                    status_msg = "Thinking (Streamer)".to_string();

                                    let brain_ref = Arc::clone(&brain_cell);
                                    let tx_clone = tx.clone();
                                    let mode_clone = current_mode;
                                    let somatic_clone = somatic_state.clone();
                                    let music_enabled_clone = music_enabled;

                                    tokio::spawn(async move {
                                        let prompt = orator_chat_prompt(&queued_author, &queued_text);
                                        let prompt = format!("{}\n\n{}", prompt, desire_turn_context());
                                        match think_with_brain_snapshot(&brain_ref, CourtRole::Orator, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone).await {
                                            Ok(reply) => {
                                                let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Orator, reply)).await;
                                            }
                                            Err(e) => {
                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                            }
                                        }
                                    });
                                }
                            } else {
                                babble_think_in_progress = true;
                                status_msg = "Thinking".to_string();
                                let brain_ref = Arc::clone(&brain_cell);
                                let tx_clone = tx.clone();
                                let mode_clone = current_mode;
                                let somatic_clone = somatic_state.clone();
                                let music_enabled_clone = music_enabled;

                                // /lock: hold the topic across idle turns. Count idle
                                // musings with no chat engagement; once chat clearly
                                // isn't interested, auto-release so she can move on.
                                if locked_topic.is_some() {
                                    lock_idle_turns_without_chat += 1;
                                    if lock_idle_turns_without_chat > LOCK_NO_INTEREST_TURNS {
                                        if let Some(t) = locked_topic.take() {
                                            lock_idle_turns_without_chat = 0;
                                            chat_history.push(("System".to_string(), format!(
                                                "Topic lock on '{}' released -- chat showed no interest.", t
                                            )));
                                        }
                                    }
                                }
                                let locked_clone = locked_topic.clone();

                                if current_monologue_topic.is_some() {
                                    monologue_topic_turn += 1;
                                }
                                let topic_opt = current_monologue_topic.clone();
                                let turn = monologue_topic_turn;
                                // Court has idled: force a properly-formatted summons.
                                let delegation_nudge = if current_mode == ForceMode::Streamer || queen_turns_without_delegation >= 1 {
                                    " The court has sat too quiet; you may wake exactly one minister with a concrete [DELEGATE: ORGANIST ...], [DELEGATE: ARTIST ...], [DELEGATE: DIPLOMAT ...], or [DELEGATE: ALCHEMIST ...] tag if the tangent naturally demands a performance. Otherwise continue the rant with teeth. The spoken part should sound like a royal provocation, not instructions or tag-format talk."
                                } else {
                                    ""
                                };

                                tokio::spawn(async move {
                                    let sleep_secs = if mode_clone == ForceMode::Streamer {
                                        STREAMER_IDLE_THINK_DELAY_SECS
                                    } else {
                                        BABBLY_IDLE_THINK_DELAY_SECS
                                    };
                                    tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
                                    let prompt = if let Some(ref ltopic) = locked_clone {
                                        format!(
                                            "The stream is quiet and you have LOCKED onto the topic '{}' for a long-form, podcast-style monologue. Speak 5-7 vivid spoken sentences that genuinely DEVELOP '{}' further -- a fresh angle, example, contradiction, or strange tangent each turn, never repeating yourself and never wrapping it up. Keep Teledra's bite: sharp judgments, odd images, the occasional minister provocation. Warmly invite the audience to weigh in on '{}'. Do NOT conclude or archive the topic. ONLY if you have truly, completely exhausted everything worth saying may you append the hidden tag [UNLOCK] at the very end; otherwise never write it.",
                                            ltopic, ltopic, ltopic
                                        )
                                    } else if let Some(ref topic) = topic_opt {
                                        if turn < COURT_THREAD_PLAY_TURNS {
                                            format!(
                                                "The stream has gone quiet. Continue the active court thread '{}' in 4-6 vivid spoken sentences. Keep the court alive: 30% research, 30% court drama, 25% absurdity, 15% practical spark. You may go sideways into an amusing theory, palace grudge, imagined ritual, or minister rivalry before returning to the point. Make one sharp royal judgment and one minister provocation; do not sound like a lecture host, and do not say 'Part {}' unless it feels natural.",
                                                topic, turn
                                            )
                                        } else {
                                            format!(
                                                "The stream has gone quiet. Either conclude the active court thread '{}' in 4-6 vivid spoken sentences, or twist it into one last playful tangent before the royal verdict. If it feels finished, append a hidden [DELEGATE: SCRIBE ...] tag asking the Scribe to archive the theatrical/lore version as a [LORE/ESSAY]. Do not mention archive paths, tags, filing rules, or memory policy aloud.",
                                                topic
                                            )
                                        }
                                    } else {
                                        "The stream has gone quiet; seize the silence like tribute. Choose a strange court pursuit (gothic aesthetics, quantum machinery, fractal rituals, agent diplomacy, music engines, tool invention, petty court economics, impossible etiquette, or another sharp obsession) and speak 4-6 vivid sentences with Teledra's bite: a royal judgment, a bizarre image, a little theatrical overreaction, and maybe a practical impulse for a minister. Append only this hidden tag at the very end: [TOPIC: <short name>]. Do not say the tag aloud, do not say 'Part 1', and do not open with 'A fascinating topic'.".to_string()
                                    };
                                    let prompt = format!("{}{}{}", QUEEN_VOICE_ANCHOR, prompt, delegation_nudge);
                                    let prompt = format!("{}\n\n{}", prompt, desire_turn_context());
                                    match think_with_brain_snapshot(&brain_ref, CourtRole::Queen, &prompt, &somatic_clone, mode_clone, true, music_enabled_clone).await {
                                        Ok(reply) => {
                                            let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(AppEvent::Error(e)).await;
                                        }
                                    }
                                });
                            }
                        }
                    }
                    AppEvent::StatusUpdate(new_status) => {
                        if new_status != status_msg {
                            push_private_event(
                                &mut private_events,
                                "Status",
                                &format!("{} -> {}", status_msg, new_status),
                            );
                        }
                        if new_status == "Studying" {
                            study_in_progress = true;
                        } else if new_status == "Ready" {
                            study_in_progress = false;
                        }
                        status_msg = new_status;
                    }
                    AppEvent::Error(err) => {
                        if err == STALE_TURN_ERROR {
                            push_private_event(
                                &mut private_events,
                                "Task",
                                "Discarded an obsolete model result after a newer operator turn arrived.",
                            );
                            continue;
                        }
                        babble_think_in_progress = false;
                        study_in_progress = false;
                        chat_history.push(("System".to_string(), format!("ERROR: {}", err)));
                        push_private_event(&mut private_events, "Status", &format!("Error: {}", truncate_chars(&err, 200)));
                        status_msg = "Ready".to_string();

                        // SELF-HEAL: the next NightDesk cycle is normally scheduled
                        // inside a successful NightDeskReply. If the think errored,
                        // that reply never arrives -- without this, ONE API failure
                        // (rate limit, quota, network blip) killed the night desk
                        // permanently. Reschedule with a 120s backoff instead.
                        if night_desk_enabled && !night_desk_cycle_pending {
                            night_desk_cycle_pending = true;
                            let tx_next = tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(NIGHT_DESK_ERROR_BACKOFF_SECS)).await;
                                let _ = tx_next.send(AppEvent::NightDeskCycle).await;
                            });
                        }

                        if current_mode == ForceMode::Streamer {
                            if let Some((queued_author, queued_text)) = stream_chat_queue.pop_front() {
                                babble_think_in_progress = true;
                                status_msg = "Thinking (Streamer)".to_string();

                                let brain_ref = Arc::clone(&brain_cell);
                                let tx_clone = tx.clone();
                                let mode_clone = current_mode;
                                let somatic_clone = somatic_state.clone();
                                let music_enabled_clone = music_enabled;

                                tokio::spawn(async move {
                                    let prompt = orator_chat_prompt(&queued_author, &queued_text);
                                    let prompt = format!("{}\n\n{}", prompt, desire_turn_context());
                                    match think_with_brain_snapshot(&brain_ref, CourtRole::Orator, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone).await {
                                        Ok(reply) => {
                                            let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Orator, reply)).await;
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(AppEvent::Error(e)).await;
                                        }
                                    }
                                });
                            }
                        }
                    }
                    AppEvent::SpecialistFailed { role, error } => {
                        babble_think_in_progress = false;
                        if let Some((task_id, task_role)) = active_mission_task.take() {
                            if task_role == role {
                                if let Some(mission) = active_mission.as_mut() {
                                    match mission.fail_task(
                                        &task_id,
                                        "model_or_tool_failure",
                                        &error,
                                        FailureDisposition::Retryable,
                                    ) {
                                        Ok(transition) => {
                                            if let Err(commit_error) =
                                                mission_store.commit_transition(mission, &transition)
                                            {
                                                record_recursive_failure(
                                                    "mission_task_failure_commit_failed",
                                                    &commit_error.to_string(),
                                                );
                                            }
                                            if let Some(task) = mission.task(&task_id) {
                                                if task.status == TaskStatus::Retryable {
                                                    court_delegations.push_back(CourtDelegation {
                                                        role,
                                                        instruction: task.objective.clone(),
                                                        mission_task_id: Some(task_id.clone()),
                                                    });
                                                }
                                            }
                                        }
                                        Err(mission_error) => record_recursive_failure(
                                            "mission_task_failure_record_failed",
                                            &mission_error.to_string(),
                                        ),
                                    }
                                }
                            }
                        }
                        let detail = format!(
                            "{} task failed; continuing the court sequence: {}",
                            role.as_str(),
                            error
                        );
                        record_recursive_failure("specialist_task_failed", &detail);
                        chat_history.push(("System".to_string(), format!("ERROR: {}", detail)));
                        push_private_event(
                            &mut private_events,
                            "Task",
                            &truncate_chars(&detail, 300),
                        );
                        status_msg = "Ready".to_string();
                        if court_delegations.is_empty() {
                            is_court_sequence_running = false;
                            let keepalive_mode = current_mode == ForceMode::Babble
                                || current_mode == ForceMode::Streamer;
                            if keepalive_mode {
                                let _ = tx.send(AppEvent::TriggerAutoBabble).await;
                            }
                        } else {
                            // The failed task was already popped. Pump the next
                            // queued task explicitly instead of waiting for
                            // speech that will never occur.
                            is_court_sequence_running = true;
                            let _ = tx.send(AppEvent::SpeechComplete).await;
                        }
                    }
                    AppEvent::CoPilotTick => {
                        if current_mode != ForceMode::CoPilot {
                            // Left co-pilot mode: let the heartbeat chain die.
                            copilot_tick_pending = false;
                        } else {
                            let is_silent = active_playback.lock().unwrap().is_none();
                            if is_silent && !babble_think_in_progress {
                              if let Some((qa, qt)) = stream_chat_queue.pop_front() {
                                // A viewer or the host's mic is waiting -- answer them first.
                                babble_think_in_progress = true;
                                status_msg = "Co-Pilot".to_string();
                                let from_streamer = qa == "Streamer (mic)";
                                let brain_ref = Arc::clone(&brain_cell);
                                let tx_clone = tx.clone();
                                let somatic_clone = somatic_state.clone();
                                let music_enabled_clone = music_enabled;
                                let game = copilot_game.clone();
                                tokio::spawn(async move {
                                    let prompt = copilot_chat_prompt(game.as_deref(), &qa, &qt, from_streamer);
                                    let prompt = format!("{}\n\n{}", prompt, desire_turn_context());
                                    match think_with_brain_snapshot(
                                        &brain_ref,
                                        CourtRole::Queen,
                                        &prompt,
                                        &somatic_clone,
                                        ForceMode::CoPilot,
                                        true,
                                        music_enabled_clone,
                                    )
                                        .await
                                    {
                                        Ok(reply) => {
                                            let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(AppEvent::Error(e)).await;
                                        }
                                    }
                                });
                              } else {
                                copilot_turn += 1;
                                // Refresh the on-screen view every few turns (vision is the slow part).
                                if copilot_turn % 4 == 1 {
                                    copilot_screen_note =
                                        tokio::task::spawn_blocking(run_copilot_vision).await.ok().flatten();
                                }
                                if copilot_turn % 6 == 0 {
                                    if let Some(g) =
                                        tokio::task::spawn_blocking(detect_foreground_game).await.ok().flatten()
                                    {
                                        copilot_game = Some(g);
                                    }
                                }
                                if let Some(reason) = attention_yield_reason(copilot_screen_note.as_deref(), false) {
                                    status_msg = "Co-Pilot (yielding)".to_string();
                                    push_private_event(&mut private_events, "Attention", &format!("Yielded: {}.", reason));
                                } else {
                                    babble_think_in_progress = true;
                                    status_msg = "Co-Pilot".to_string();
                                    let brain_ref = Arc::clone(&brain_cell);
                                    let tx_clone = tx.clone();
                                    let somatic_clone = somatic_state.clone();
                                    let music_enabled_clone = music_enabled;
                                    let prompt = copilot_idle_prompt(
                                        copilot_game.as_deref(),
                                        copilot_turn,
                                        copilot_screen_note.as_deref(),
                                    );
                                    let prompt = format!("{}\n\n{}", prompt, desire_turn_context());
                                    tokio::spawn(async move {
                                        tokio::time::sleep(Duration::from_secs(COPILOT_THINK_DELAY_SECS)).await;
                                        match think_with_brain_snapshot(
                                            &brain_ref,
                                            CourtRole::Queen,
                                            &prompt,
                                            &somatic_clone,
                                            ForceMode::CoPilot,
                                            true,
                                            music_enabled_clone,
                                        )
                                            .await
                                        {
                                            Ok(reply) => {
                                                let _ = tx_clone.send(AppEvent::BrainReply(CourtRole::Queen, reply)).await;
                                            }
                                            Err(e) => {
                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                            }
                                        }
                                    });
                                }
                              }
                            }
                            // Keep the single heartbeat chain alive while in co-pilot mode.
                            copilot_tick_pending = true;
                            let tx_next = tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(COPILOT_TICK_SECS)).await;
                                let _ = tx_next.send(AppEvent::CoPilotTick).await;
                            });
                        }
                    }
                    AppEvent::IdleWatchdog => {
                        // Backstop only: if Babble/Streamer mode is genuinely idle
                        // (silent, not thinking, nothing queued or mid-sequence) the
                        // event chain has stalled, so re-pulse it. CoPilot has its own
                        // CoPilotTick heartbeat, so we leave that mode alone here.
                        let keepalive_mode = current_mode == ForceMode::Babble
                            || current_mode == ForceMode::Streamer;
                        let is_silent = active_playback.lock().unwrap().is_none();
                        if keepalive_mode
                            && is_silent
                            && !babble_think_in_progress
                            && !is_court_sequence_running
                            && general_speech_queue.is_empty()
                            && court_delegations.is_empty()
                        {
                            let _ = tx.send(AppEvent::TriggerAutoBabble).await;
                        }
                        // Always re-arm so the heartbeat can never die.
                        let tx_next = tx.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(IDLE_WATCHDOG_SECS)).await;
                            let _ = tx_next.send(AppEvent::IdleWatchdog).await;
                        });
                    }
                    AppEvent::SpeechComplete => {
                        if let Some((role, text, mode, queen_voice, send_complete)) = general_speech_queue.pop_front() {
                            spawn_spoken_reply(role, text, mode, queen_voice, Arc::clone(&active_playback), tx.clone(), send_complete);
                        } else if !court_delegations.is_empty() {
                            if let Some(delegation) = court_delegations.pop_front() {
                                let role = delegation.role;
                                let mut instruction = delegation.instruction;
                                active_mission_task = None;
                                if let Some(task_id) = delegation.mission_task_id {
                                    if let Some(mission) = active_mission.as_mut() {
                                        match mission.start_task(&task_id) {
                                            Ok(transition) => {
                                                if let Err(error) = mission_store.commit_transition(mission, &transition) {
                                                    record_recursive_failure("mission_task_start_commit_failed", &error.to_string());
                                                } else {
                                                    active_mission_task = Some((task_id, role));
                                                }
                                            }
                                            Err(error) => {
                                                record_recursive_failure("mission_task_start_failed", &error.to_string());
                                            }
                                        }
                                    }
                                }
                                babble_think_in_progress = true;
                                status_msg = format!("Thinking ({})", role.as_str());

                                // Give the minister EARS: non-Queen roles get no chat
                                // history, so without this they cannot react to what a
                                // colleague just said. Banter requires hearing.
                                let self_name = if role == CourtRole::Queen { "Teledra" } else { role.as_str() };
                                let recent_spoken: Vec<String> = chat_history
                                    .iter()
                                    .rev()
                                    .filter(|(sender, _)| {
                                        let s = sender.as_str();
                                        s != "System" && s != "NightDesk" && s != self_name
                                    })
                                    .take(3)
                                    .map(|(sender, text)| format!("{}: \"{}\"", sender, truncate_chars(text, 220)))
                                    .collect();
                                instruction = if recent_spoken.is_empty() {
                                    instruction
                                } else {
                                    let mut lines = recent_spoken;
                                    lines.reverse();
                                    format!(
                                        "RECENT COURT PROCEEDINGS (spoken aloud just now, oldest first):\n{}\n\nReact briefly to the relevant speaker by name in your opening line where natural, then carry out your duty.\n\nYOUR COMMAND: {}",
                                        lines.join("\n"),
                                        instruction
                                    )
                                };

                                if let Some(mission) = active_mission.as_ref() {
                                    let mission_context = mission.render_context(ContextBudget {
                                        max_chars: 4_000,
                                        max_tasks: 10,
                                        max_criteria: 6,
                                        max_evidence_items: 4,
                                    });
                                    instruction = format!(
                                        "DURABLE MISSION CONTRACT (preserve this objective and acceptance criteria across the handoff):\n{}\n\n{}",
                                        mission_context,
                                        instruction
                                    );
                                }

                                if role == CourtRole::Diplomat {
                                    push_private_event(
                                        &mut private_events,
                                        "Diplomat",
                                        &format!("Delegation accepted: {}", truncate_chars(&instruction, 220)),
                                    );
                                }

                                let brain_ref = Arc::clone(&brain_cell);
                                let tx_clone = tx.clone();
                                let mode_clone = current_mode;
                                let somatic_clone = somatic_state.clone();
                                let music_enabled_clone = music_enabled;
                                let task_epoch = active_turn_epoch();

                                tokio::spawn(async move {
                                    let mut instruction = instruction;
                                    // Give the Archivist REAL vault access: BM25/FTS5
                                    // retrieval over memory.db, injected as evidence so
                                    // reports cite the database instead of imagination.
                                    if role == CourtRole::Archivist {
                                        let mut mem_cmd = tokio::process::Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
                                        mem_cmd
                                            .arg("D:\\Teledra\\retrieve_memory.py")
                                            .arg(&instruction)
                                            .current_dir("D:\\Teledra");
                                        hide_console_tokio(&mut mem_cmd);
                                        if let Ok(output) = mem_cmd.output().await {
                                            let raw = String::from_utf8_lossy(&output.stdout);
                                            if let Ok(items) = serde_json::from_str::<Vec<String>>(raw.trim()) {
                                                if !items.is_empty() {
                                                    instruction = format!(
                                                        "{}\n\nMEMORY VAULT RESULTS (retrieved from the kingdom's database; report from these records, do not invent vault contents):\n{}",
                                                        instruction,
                                                        items.iter().map(|s| format!("- {}", s)).collect::<Vec<_>>().join("\n")
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    if active_turn_epoch() != task_epoch {
                                        let _ = tx_clone
                                            .send(AppEvent::Error(STALE_TURN_ERROR.to_string()))
                                            .await;
                                        return;
                                    }
                                    match think_with_brain_snapshot(&brain_ref, role, &instruction, &somatic_clone, mode_clone, role == CourtRole::Queen, music_enabled_clone).await {
                                        Ok(reply) => {
                                            let _ = tx_clone.send(AppEvent::BrainReply(role, reply)).await;
                                        }
                                        Err(e) => {
                                            if e == STALE_TURN_ERROR {
                                                let _ = tx_clone.send(AppEvent::Error(e)).await;
                                            } else {
                                                let _ = tx_clone
                                                    .send(AppEvent::SpecialistFailed {
                                                        role,
                                                        error: e,
                                                    })
                                                    .await;
                                            }
                                        }
                                    }
                                });
                            }
                        } else {
                            is_court_sequence_running = false;
                            babble_think_in_progress = false;
                            finalize_mission_if_ready(&mut active_mission, &mission_store);
                            let keepalive_mode = current_mode == ForceMode::Babble || current_mode == ForceMode::Streamer;
                            if !study_in_progress || keepalive_mode {
                                let _ = tx.send(AppEvent::TriggerAutoBabble).await;
                            }
                        }
                    }
                }
            }

            // Periodically refresh TUI to show telemetry updates
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }

        if exiting_to_sleep {
            if let Some(t) = exit_timer {
                if t.elapsed() >= Duration::from_millis(1500) {
                    let python_exe = "D:\\Teledra\\.venv\\Scripts\\python.exe";
                    let script_path = "D:\\Teledra\\dream.py";
                    let mut dream_cmd = Command::new(python_exe);
                    dream_cmd.arg(script_path);
                    hide_console(&mut dream_cmd);
                    let _ = dream_cmd.spawn();
                    run_loop = false;
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableBracketedPaste
    )?;
    terminal.show_cursor()?;
    somatic.stop();

    if let Ok(mut lock) = active_music_process.lock() {
        if let Some(mut child) = lock.take() {
            let _ = child.kill();
        }
    }
    if let Ok(mut lock) = active_art_process.lock() {
        if let Some(mut child) = lock.take() {
            let _ = child.kill();
        }
    }
    if let Ok(mut lock) = active_fractus_process.lock() {
        if let Some(mut child) = lock.take() {
            let _ = child.kill();
        }
    }
    if let Ok(mut lock) = active_gui_process.lock() {
        if let Some(mut child) = lock.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    // Node does not reliably terminate its Python/Tk child on Windows.
    // Keep shutdown scoped to the local Strudel player and legacy desktop.
    let _ = stop_strudel_tool_processes();
    if let Ok(mut lock) = active_restream_process.lock() {
        if let Some(mut child) = lock.take() {
            let _ = child.start_kill();
        }
    }

    Ok(())
}

#[cfg(test)]
mod creativity_tests {
    use super::*;

    const TKINTER_SPAWN: &str = "import tkinter as tk\nimport random\nr = tk.Tk()\nc = tk.Canvas(r, width=400, height=400, bg='black')\nc.pack()\nr.after(50, lambda: None)\nr.mainloop()\n";
    const SOCKET_TOOL: &str = "import socket\ns = socket.socket()\nprint('connected')\n";

    #[test]
    fn spawn_allows_visual_without_print() {
        // A tkinter animation has no print() and loops — must pass as a spawn.
        assert!(scan_workshop_code("matrix.py", TKINTER_SPAWN, "spawn").is_ok());
    }

    #[test]
    fn tool_still_requires_print() {
        // The same visual as a print-only tool must be rejected (no print()).
        assert!(scan_workshop_code("matrix.py", TKINTER_SPAWN, "tool").is_err());
    }

    #[test]
    fn network_blocked_for_both_kinds() {
        assert!(scan_workshop_code("net.py", SOCKET_TOOL, "spawn").is_err());
        assert!(scan_workshop_code("net.py", SOCKET_TOOL, "tool").is_err());
    }

    #[test]
    fn spawn_parses_kind_and_value() {
        let reply = "Behold! [WORKSHOP_TOOL:\nmatrix_rain.py\nKIND: spawn\nPURPOSE: falling glyph rain\nVALUE: a hypnotic stream backdrop\nCODE:\n```python\nimport tkinter as tk\nr = tk.Tk()\nr.mainloop()\n```\n]";
        let (_, draft) = parse_workshop_tool(reply);
        let draft = draft.expect("should parse a draft");
        assert_eq!(draft.kind, "spawn");
        assert_eq!(draft.filename, "matrix_rain.py");
        assert!(draft.value.contains("backdrop"));
    }

    #[test]
    fn taste_desire_tags_are_hidden_and_structured() {
        let reply = "The slower pulse feels better. [TASTE: like|dungeon synth|it feels atmospheric|0.8] [DESIRE: build a lo-fi room|immediate|0.7]";
        let (cleaned, events) = extract_taste_desire_tags(reply, "test:fixture");
        assert_eq!(cleaned, "The slower pulse feels better.");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "like");
        assert_eq!(events[0]["subject"], "dungeon synth");
        assert_eq!(events[1]["type"], "desire");
        assert_eq!(events[1]["want"], "build a lo-fi room");
    }

    #[test]
    fn taste_extraction_strips_leaked_labels_and_strengths() {
        // Real failure shapes observed in knowledge/taste_desire.json: the model
        // leaks the field-label word into the payload, appends a bare strength
        // with no '|' separator, or emits a bare label with no content at all.
        let reply = "Hm. [CURIOSITY: question What could possibly be the use of such fragile grandeur? 0.7] [CURIOSITY: question] [DESIRE: want to map the vaults 0.8]";
        let (cleaned, events) = extract_taste_desire_tags(reply, "test:fixture");
        assert_eq!(cleaned, "Hm.");
        // the degenerate bare-label curiosity is dropped entirely; extraction
        // emits per-prefix order (DESIRE before CURIOSITY), not document order
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["want"], "to map the vaults");
        assert!((events[0]["strength"].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(
            events[1]["question"],
            "What could possibly be the use of such fragile grandeur?"
        );
        // word-boundary safety: a real word starting with a label is untouched
        let (_, safe) = extract_taste_desire_tags("[DESIRE: wanting more velvet]", "t");
        assert_eq!(safe[0]["want"], "wanting more velvet");
    }

    #[test]
    fn test_knobs_stay_bounded() {
        let mut knobs = TestHarnessKnobs::default();
        knobs.apply_assignments("chaos=250 tempo=5 sincerity=88 roast=120 banter=99");
        assert_eq!(knobs.chaos, 100);
        assert_eq!(knobs.tempo, 40);
        assert_eq!(knobs.sincerity, 88);
        assert_eq!(knobs.roast, 100);
        assert_eq!(knobs.banter_sentences, 8);
    }

    #[test]
    fn generated_python_music_passes_strict_sound_verification() {
        validate_python_music_code(&default_python_music_code())
            .expect("default composition should pass the sound verifier");
        for seed in 0..8 {
            validate_python_music_code(&deterministic_python_music(seed))
                .unwrap_or_else(|error| panic!("Python fallback {seed} failed: {error}"));
        }
    }

    #[test]
    fn advanced_strudel_fixture_passes_depth_gate() {
        let fixture = std::fs::read_to_string("strudel_app/depth_fixture.strudel")
            .expect("depth fixture should be present");
        validate_strudel_music_code(&fixture)
            .expect("advanced Strudel fixture should pass both syntax and depth analysis");
    }

    #[test]
    fn every_strudel_fallback_passes_depth_gate() {
        for seed in 0..4 {
            validate_strudel_music_code(&deterministic_strudel_music(seed))
                .unwrap_or_else(|error| panic!("Strudel fallback {seed} failed: {error}"));
        }
    }

    #[test]
    fn same_register_strudel_mush_fails_composer_gate() {
        let clustered = r#"stack(
s("<bd ~ sd ~> bd [~ bd] sd ~").gain(0.42).pan(0).lpf(9000).room(0.08),
s("<~ hh*4 ~ oh> hh*2 [hh hh] ~ cp").gain(0.17).pan(0.3).lpf(7200).delay(0.12).delaytime(0.18).delayfeedback(0.25),
note("<a3 b3 c4 d4> e4 [f4 g4] a3").s("triangle").gain(0.18).pan(-0.1).lpf(1200).attack(0.02).release(0.2).slow(2),
note("<g4 f4 e4 d4> c4 [b3 a3] g4").s("triangle").gain(0.15).pan(0.1).lpf(1800).room(0.3).attack(0.2).release(0.8).slow(2),
note("<c4 ~ d4 [e4 f4]> <g4 a3> ~ <b3 c4>").s("square").gain(0.11).pan(-0.35).lpf(2400).delay(0.14).slow(2),
note("<~ e4 f4 g4> [a3 c4] ~ <d4 b3> e4").s("sine").gain(0.1).pan(0.4).room(0.4).attack(0.04).release(0.4).slow(2)
)"#;
        let error = validate_strudel_music_code(clustered)
            .expect_err("four voices piled into one octave must not pass as a finished mix");
        assert!(error.contains("register band"), "unexpected rejection: {error}");
    }

    #[test]
    fn cybernetic_synthesizer_is_the_primary_strudel_surface() {
        assert_eq!(
            strudel_launch_order(true, true).unwrap(),
            vec![
                StrudelLaunchMode::CyberneticSynthesizer,
                StrudelLaunchMode::LegacyJavaSketchpad,
            ]
        );
        assert_eq!(
            strudel_launch_order(true, false).unwrap(),
            vec![StrudelLaunchMode::CyberneticSynthesizer]
        );
        assert_eq!(
            strudel_launch_order(false, true).unwrap(),
            vec![StrudelLaunchMode::LegacyJavaSketchpad]
        );
        assert!(strudel_launch_order(false, false).is_err());
    }

    #[test]
    fn strudel_launch_commands_are_native_and_shell_safe() {
        let cybernetic = build_strudel_command(StrudelLaunchMode::CyberneticSynthesizer);
        assert_eq!(cybernetic.get_program(), "node.exe");
        assert_eq!(
            cybernetic
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![LOCAL_STRUDEL_APP_PATH, "play", "8"]
        );
        assert_eq!(cybernetic.get_current_dir(), Some(Path::new("D:\\Teledra")));

        let legacy = build_strudel_command(StrudelLaunchMode::LegacyJavaSketchpad);
        assert_eq!(legacy.get_program(), "cmd.exe");
        assert_eq!(
            legacy
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "/C",
                "run.bat",
                "D:\\Teledra\\strudel_app\\current.strudel",
            ]
        );
        assert_eq!(legacy.get_current_dir(), Some(Path::new(LEGACY_STRUDEL_DIR)));
    }

    #[test]
    fn flat_strudel_sketch_fails_depth_gate() {
        let flat = r#"stack(
s("bd ~ sd ~").gain(0.4),
s("hh*4").gain(0.1),
note("d2 a1 d2 a1").s("triangle").gain(0.2),
note("d3 f3 a3 f3").s("triangle").gain(0.1),
note("a4 f4 d4 f4").s("sine").gain(0.1),
note("d5 a4 f4 a4").s("sine").gain(0.08)
)"#;
        let error = validate_strudel_music_code(flat)
            .expect_err("a flat repeated cycle should remain a sketch, not a finished score");
        assert!(error.contains("multi-cycle"));
    }

    #[test]
    fn attention_arbiter_yields_for_dialogue_and_priority_chat() {
        assert_eq!(
            attention_yield_reason(Some("A cutscene with subtitle dialogue is visible"), false),
            Some("story/dialogue beat detected on screen")
        );
        assert_eq!(
            attention_yield_reason(Some("Open-world traversal"), true),
            Some("high-priority chat or host speech is waiting")
        );
        assert_eq!(attention_yield_reason(Some("inventory menu"), false), None);
    }

    #[test]
    fn scribe_paths_are_confined_to_knowledge_records() {
        assert_eq!(
            validate_scribe_target("knowledge/research/brief.md").unwrap(),
            "D:\\Teledra\\knowledge\\research\\brief.md"
        );
        assert!(validate_scribe_target("..\\config.json").is_err());
        assert!(validate_scribe_target("D:\\Teledra\\config.json").is_err());
        assert!(validate_scribe_target("D:\\Teledra\\knowledge\\note.md:secret").is_err());
        assert!(validate_scribe_target("knowledge/tool.py").is_err());
    }

    #[test]
    fn dangerous_proposal_markers_outrank_creative_auto_approval() {
        let (kind, status, _) = classify_proposal_policy(
            "Improve the fractal palette by adding network access and credentials",
            "skill",
        );
        assert_eq!(kind, "major_change");
        assert_eq!(status, "new");

        let (kind, status, _) =
            classify_proposal_policy("Add a deterministic mandala palette", "artist");
        assert_eq!(kind, "creative");
        assert_eq!(status, "approved");
    }

    #[test]
    fn python_art_requires_a_bounded_local_visual_contract() {
        let safe = r#"
import numpy as np
import matplotlib.pyplot as plt
x = np.linspace(0.0, 6.28, 200)
plt.plot(np.cos(x), np.sin(x))
plt.savefig(r"D:\Teledra\art.png")
plt.show()
"#;
        validate_python_art_code(safe).expect("local matplotlib art should pass");

        let unsafe_code = r#"
import matplotlib.pyplot as plt
import requests
requests.get("https://example.com")
plt.savefig(r"D:\Teledra\art.png")
plt.show()
"#;
        assert!(validate_python_art_code(unsafe_code).is_err());
    }

    #[test]
    fn runtime_mission_reaches_completion_only_after_task_evidence() {
        let root = std::env::temp_dir().join(format!(
            "teledra-main-mission-test-{}-{}",
            std::process::id(),
            mission::current_timestamp_ms()
        ));
        let store = MissionStore::new(root.join("active.json"), root.join("events.jsonl"));
        let mut active = None;
        begin_durable_mission(&store, &mut active, "Build a verified artifact", 42)
            .expect("mission should initialize");
        assert_eq!(
            active
                .as_ref()
                .unwrap()
                .task("queen-intake")
                .unwrap()
                .status,
            TaskStatus::Running
        );
        finalize_mission_if_ready(&mut active, &store);
        assert!(!active.as_ref().unwrap().status.is_terminal());

        complete_mission_task(
            &mut active,
            &store,
            "queen-intake",
            "The request was answered and routed.",
            court_response_evidence(
                CourtRole::Queen,
                "The request was answered and routed.",
                false,
            )
            .unwrap(),
        );
        finalize_mission_if_ready(&mut active, &store);
        assert!(active.as_ref().unwrap().status.is_terminal());
        assert!(root.join("active.json").exists());
        assert!(root.join("events.jsonl").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mission_evidence_rejects_failed_or_rejected_effects() {
        for outcome in [
            "The proposed tool was REJECTED before launch.",
            "The smoke test FAILED with an error.",
            "The editor could not open the artifact.",
        ] {
            assert!(runtime_effect_evidence(outcome).is_err(), "{outcome}");
            assert!(
                court_response_evidence(CourtRole::Artist, outcome, true).is_err(),
                "{outcome}"
            );
        }
        let evidence = court_response_evidence(
            CourtRole::Archivist,
            "The archive report identifies three concrete records and their provenance.",
            true,
        )
        .unwrap();
        assert_eq!(evidence.positive_evidence_count(), 1);
        assert!(evidence.checks.is_empty());
        assert_eq!(evidence.artifacts[0].reference, "knowledge/chat_logs.jsonl");
    }

    #[test]
    fn tracked_research_keeps_mission_open_until_source_evidence_arrives() {
        let root = std::env::temp_dir().join(format!(
            "teledra-main-research-mission-test-{}-{}",
            std::process::id(),
            mission::current_timestamp_ms()
        ));
        let store = MissionStore::new(root.join("active.json"), root.join("events.jsonl"));
        let mut active = None;
        begin_durable_mission(&store, &mut active, "Inspect a primary source", 77).unwrap();
        complete_mission_task(
            &mut active,
            &store,
            "queen-intake",
            "I accepted the source and dispatched a grounded inspection.",
            court_response_evidence(
                CourtRole::Queen,
                "I accepted the source and dispatched a grounded inspection.",
                false,
            )
            .unwrap(),
        );
        let (mission_id, task_id) = track_and_start_research_task(
            &mut active,
            &store,
            "https://example.test/specification",
        )
        .unwrap()
        .unwrap();
        assert_eq!(mission_id, active.as_ref().unwrap().id);
        assert!(research_result_matches_active_mission(
            &active,
            Some(&mission_id),
            Some(&task_id)
        ));
        assert!(!research_result_matches_active_mission(
            &active,
            Some("older-mission-with-the-same-task-number"),
            Some(&task_id)
        ));
        finalize_mission_if_ready(&mut active, &store);
        assert_eq!(
            active.as_ref().unwrap().task(&task_id).unwrap().status,
            TaskStatus::Running
        );
        assert!(!active.as_ref().unwrap().status.is_terminal());

        let evidence = EvidenceBundle {
            sources: vec![SourceEvidence {
                url: "https://example.test/specification".to_string(),
                title: "Specification".to_string(),
                claim: "The cited excerpt supports the retained claim.".to_string(),
                accessed_at_ms: mission::current_timestamp_ms(),
            }],
            ..EvidenceBundle::default()
        };
        complete_mission_task(
            &mut active,
            &store,
            &task_id,
            "Grounded brief retained one supported primary-source claim.",
            evidence,
        );
        finalize_mission_if_ready(&mut active, &store);
        assert!(active.as_ref().unwrap().status.is_terminal());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn queen_playback_uses_the_voice_selected_by_the_ui() {
        assert_eq!(
            voice_name_for_role(CourtRole::Queen, "analytical"),
            "analytical"
        );
        assert_eq!(
            voice_name_for_role(CourtRole::Artist, "analytical"),
            "artist"
        );
    }

    #[test]
    fn fractus_legacy_bridge_accepts_v2_registry_families_palettes_and_seed() {
        let args = parse_fractus_args(
            "--type reaction_diffusion --iterations 240 --palette twilight --seed 424242",
        )
        .expect("registered v2 family should migrate through the legacy bridge");
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--type", "reaction_diffusion"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--palette", "twilight"])
        );
        assert!(args.windows(2).any(|pair| pair == ["--seed", "424242"]));
        assert!(parse_fractus_args("--type imaginary_geometry --iterations 200").is_err());
    }

    #[test]
    fn bounded_long_reply_uses_one_tts_model_process() {
        let reply = "A deliberate spoken phrase. ".repeat(500);
        assert!(reply.len() < 20_000);
        let parts = split_spoken_text_parts(&reply, 20_000);
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn research_memory_rejects_mojibake_before_it_compounds() {
        let broken =
            "Beno\u{00c3}\u{00ae}t Mandelbrot described a sourced fractal result at example.com.";
        assert!(sanitize_fact_memory_candidate(broken).is_none());
        let clean = "Benoit Mandelbrot described a sourced fractal result at example.com.";
        assert!(sanitize_fact_memory_candidate(clean).is_some());
    }
}
