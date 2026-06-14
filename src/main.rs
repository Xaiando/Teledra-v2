mod brain;
mod ears;
mod somatic_bridge;
mod voice;

use brain::{Brain, CourtRole, ForceMode};
use ears::AudioCortex;
use somatic_bridge::SomaticBridge;
use voice::VoiceEngine;

use image::{DynamicImage, GenericImageView};
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
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
    SpeechComplete,
}

#[derive(PartialEq, Debug, Clone, Copy)]
enum FocusField {
    Chat,
    Youtube,
}

struct WorkshopToolDraft {
    filename: String,
    purpose: String,
    code: String,
}

const LEARNED_MEMORY_PATH: &str = "knowledge/learned_memory.json";
const FACT_MEMORY_PATH: &str = "knowledge/fact_memory.jsonl";
const LORE_MEMORY_PATH: &str = "knowledge/lore_memory.jsonl";
const FACT_ARCHIVE_PATH: &str = "D:\\Teledra\\knowledge\\fact_archive.md";
const LORE_ARCHIVE_PATH: &str = "D:\\Teledra\\knowledge\\lore_archive.md";

/// Short, high-priority persona anchor prepended to every monologue prompt.
/// Small local models follow brief recent instructions far better than the
/// large system prompt, so this fights encyclopedia-narrator drift directly.
const QUEEN_VOICE_ANCHOR: &str = "VOICE CHECK: You are TELEDRA, the monarch in the room -- imperial, sassy, transactional, theatrically strange, energetic, and bored by weak ceremony. The front stage belongs to your performance, not backstage maintenance. Decree, mock, marvel, interrupt yourself, chase odd tangents, summon ministers when the mood bites, and make sudden royal judgments; never narrate like an encyclopedia or conference host. Speak with high-voltage court momentum: shorter punchy clauses, quick turns, strange pivots, actual little laughs like 'Ha!' or 'Ahahaha!' when amused, and fewer slow ceremonial windups. Quiet-stream rants should usually be at least four vivid spoken sentences, unless you are answering a chat message directly. Autonomous rants are allowed to wander for several turns: weird court play first, useful action when it sparks. FORBIDDEN OPENERS: 'A fascinating topic', 'Let's dive in', 'Teledra here', textbook fact-listing, speaker labels, or third-person narration of yourself. If a link appears, treat it as a thing to inspect, not a fact you already know. ";

const STREAMER_IDLE_THINK_DELAY_SECS: u64 = 0;
const BABBLY_IDLE_THINK_DELAY_SECS: u64 = 0;
const NIGHT_DESK_NEXT_CYCLE_SECS: u64 = 8;
const NIGHT_DESK_ENVOY_CYCLE_SECS: u64 = 16;
const NIGHT_DESK_ERROR_BACKOFF_SECS: u64 = 12;
const STUDY_LOOP_INITIAL_DELAY_SECS: u64 = 2;
const STUDY_LOOP_INTERVAL_SECS: u64 = 10;
const COURT_THREAD_PLAY_TURNS: u32 = 6;

fn current_unix_timestamp() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "0".to_string(),
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
        "(source",
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

fn sanitize_fact_memory_candidate(raw_fact: &str) -> Option<String> {
    let mut cleaned = strip_refiner_prefixes(raw_fact);
    cleaned = strip_fact_preamble(&cleaned);
    cleaned = strip_unclosed_tool_and_code_noise(&cleaned);
    cleaned = compact_memory_text(&cleaned);

    if cleaned.to_uppercase().contains("NO_USABLE_FACT") {
        return None;
    }
    if cleaned.len() < 40 {
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
    let Ok(contents) = std::fs::read_to_string(REJECTED_TOPICS_PATH) else {
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
        "this", "that", "with", "from", "what", "when", "which", "about", "into",
        "your", "their", "there", "then", "them", "they", "have", "will", "would",
        "could", "should", "because", "while", "where", "does", "using", "used",
        "more", "most",
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
        | CourtRole::Treasurer => trim_to_sentence_count(&deduped, 3, 520),
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
    let message = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let inferred_policy = classify_proposal_policy(message, source);
    let kind = entry
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or(inferred_policy.0);
    let policy = entry
        .get("policy")
        .and_then(|v| v.as_str())
        .unwrap_or(inferred_policy.2);
    let message = truncate_clean(&compact_memory_text(message), 220);
    let policy = truncate_clean(&compact_memory_text(policy), 130);
    format!(
        "#{} [{}:{} from {}] {} | policy: {}",
        id, status, kind, source, message, policy
    )
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
        "Skill improvement: Strudel edits must use only the local stack(...), s(...), note(...), gain/slow/fast subset and should be validated before narration."
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
    const TAG_MARKERS: [&str; 9] = [
        "[DELEGATE:",
        "[DIPLOMACY:",
        "[RESEARCH:",
        "[SUGGESTION:",
        "[TOPIC:",
        "[FRACTUS_ART:",
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
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with("```")
                || trimmed.eq_ignore_ascii_case("CODE:")
            {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("PURPOSE:") {
                purpose = rest.trim().to_string();
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("Purpose:") {
                purpose = rest.trim().to_string();
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
                    }),
                );
            }
        }
        return (cleaned, None);
    }
    (reply.to_string(), None)
}

fn scan_workshop_code(filename: &str, code: &str) -> Result<(), String> {
    if code.len() > 20_000 {
        return Err("Workshop artifact is too large.".to_string());
    }

    let trimmed = code.trim();
    if trimmed.len() < 30 {
        return Err("Workshop artifact is too short to be useful.".to_string());
    }

    let lower = code.to_lowercase();
    let placeholder_markers = [
        "<code>",
        "...",
        "```",
        "[workshop_tool:",
        "purpose:",
        "code:",
        "todo",
        "placeholder",
        "pseudo-code",
        "pseudocode",
    ];
    for needle in placeholder_markers {
        if lower.contains(needle) {
            return Err(format!(
                "Workshop artifact still contains placeholder or prompt scaffolding: {}",
                needle
            ));
        }
    }

    let forbidden = [
        "import socket",
        "from socket",
        "import requests",
        "from requests",
        "import urllib",
        "from urllib",
        "import httpx",
        "from httpx",
        "import subprocess",
        "from subprocess",
        "os.system",
        "popen(",
        "shutil.rmtree",
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

    if filename.ends_with(".py") && !lower.contains("print(") {
        return Err("Workshop Python scripts must print a concise smoke-test result.".to_string());
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

fn write_workshop_tool(draft: &WorkshopToolDraft) -> Result<(String, bool), String> {
    let filename = validate_workshop_filename(&draft.filename)?;
    scan_workshop_code(&filename, &draft.code)?;

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

    let run_result = run_workshop_experiment(&filename);
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
        let _ = append_suggestion(
            &format!(
                "Workshop tool '{}' passed its sandbox smoke test. Consider reviewing its report and promoting it to tools/approved if useful. Risk: sandboxed utility only; test prompt: run /workshoprun {}.",
                filename, filename
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
    {
        return None;
    }

    Some(query)
}

async fn run_study_cycle(
    brain_study: Arc<RwLock<Brain>>,
    tx_study: mpsc::Sender<AppEvent>,
    custom_query: Option<String>,
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
                        for fact in facts.iter() {
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

        let mut brain = brain_study.write().await;
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
        cmd.arg(script_path).arg(&query_for_cmd);
        hide_console(&mut cmd);
        cmd.output()
    })
    .await;

    let mut scraped_text = String::new();
    if let Ok(Ok(output)) = scrape_res {
        scraped_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }

    if !scraped_text.is_empty() {
        let fact = {
            let mut brain = brain_study.write().await;
            match brain.distill_research_fact(&query, &scraped_text).await {
                Ok(f) => strip_refiner_prefixes(&f),
                Err(e) => format!("Failed to distill researched topic: {}", e),
            }
        };

        match append_verified_fact(&query, &fact) {
            Ok(Some(saved_fact)) => {
                let _ = append_expansion_ledger(
                    "online_research",
                    &format!("query={} | distilled_fact={}", query, saved_fact),
                );
                let _ = tx_study
                    .send(AppEvent::StudyComplete {
                        summary: format!("Studied {}: \"{}\"", query, saved_fact),
                        usable: true,
                    })
                    .await;
            }
            Ok(None) => {
                let _ = append_expansion_ledger(
                    "online_research_rejected",
                    &format!(
                        "query={} | note=distilled result was unusable or already known",
                        query
                    ),
                );
                // Blacklist the topic so the selector is steered AWAY from it.
                // Deliberately do NOT embed the topic in a failure signal that
                // gets re-fed to generation prompts -- that was a self-
                // reinforcing loop that re-seeded the same dead topic forever.
                record_rejected_topic(&query);
                let _ = tx_study
                    .send(AppEvent::StudyComplete {
                        summary: format!(
                            "Studied {}, but it yielded nothing new; topic blacklisted, moving on.",
                            query
                        ),
                        usable: false,
                    })
                    .await;
            }
            Err(e) => {
                record_recursive_failure(
                    "research_memory_save_failed",
                    &format!("query={} | error={}", query, e),
                );
                let _ = tx_study
                    .send(AppEvent::Error(format!(
                        "Research memory save failed: {}",
                        e
                    )))
                    .await;
            }
        }
    } else {
        let _ = append_expansion_ledger(
            "online_research_failed",
            &format!("query={} | error=search returned no index results", query),
        );
        record_rejected_topic(&query);
        let _ = tx_study
            .send(AppEvent::Error(
                "Search returned no index results.".to_string(),
            ))
            .await;
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

fn voice_name_for_role(role: CourtRole) -> &'static str {
    match role {
        CourtRole::Queen => "queen",
        CourtRole::Organist => "organist",
        CourtRole::Archivist => "archivist",
        CourtRole::Alchemist => "alchemist",
        CourtRole::Orator => "orator",
        CourtRole::Scribe => "scribe",
        CourtRole::Artist => "artist",
        CourtRole::Diplomat => "diplomat",
        CourtRole::Treasurer => "treasurer",
    }
}

fn speech_limits_for_role(role: CourtRole, mode: ForceMode) -> (usize, usize) {
    match role {
        CourtRole::Queen if mode == ForceMode::Babble || mode == ForceMode::Streamer => (32, 16000),
        CourtRole::Queen => (36, 7000),
        CourtRole::Organist | CourtRole::Artist => (18, 7000),
        CourtRole::Diplomat => (16, 7000),
        CourtRole::Scribe => (4, 900),
        _ => (10, 3800),
    }
}

fn spawn_spoken_reply(
    role: CourtRole,
    text: String,
    mode: ForceMode,
    active_playback: Arc<std::sync::Mutex<Option<voice::PlaybackController>>>,
    tx: mpsc::Sender<AppEvent>,
    send_speech_complete: bool,
) {
    let active_voice = voice_name_for_role(role).to_string();
    let cleaned_speech = clean_text_for_speech(&text, role);
    let (speech_sentence_limit, speech_char_limit) = speech_limits_for_role(role, mode);
    let reply_for_speech =
        limit_spoken_text(&cleaned_speech, speech_sentence_limit, speech_char_limit);
    let speech_parts = split_spoken_text_parts(&reply_for_speech, 900);

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
                        let _ = tx.blocking_send(AppEvent::StatusUpdate("Ready".to_string()));
                        if send_speech_complete {
                            let _ = tx.blocking_send(AppEvent::SpeechComplete);
                        }
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

fn default_strudel_music_code() -> String {
    let patterns = [
        "stack(\n\
s(\"bd ~ sn ~ hh*2 oh\").gain(0.5),\n\
note(\"c2 eb2 g2 bb2\").s(\"triangle\").gain(0.38).slow(1.5),\n\
note(\"c4 eb4 g4 bb4\").s(\"sawtooth\").gain(0.24).slow(2)\n\
)",
        "stack(\n\
s(\"bd*2 ~ sn ~ hh*4\").gain(0.44),\n\
note(\"a1 e2 g2 d2\").s(\"triangle\").gain(0.34).slow(2),\n\
note(\"a4 c5 e5 g5 e5 c5\").s(\"sine\").gain(0.22).fast(1.5)\n\
)",
        "stack(\n\
s(\"bd ~ ~ sn hh*3 ~ oh\").gain(0.48),\n\
note(\"d2 a2 f2 c3\").s(\"square\").gain(0.26).slow(1.25),\n\
note(\"f4 a4 c5 e5 d5 a4\").s(\"sawtooth\").gain(0.2).slow(1.5)\n\
)",
        "stack(\n\
s(\"bd ~ hh sn ~ hh*2 oh\").gain(0.46),\n\
note(\"g1 d2 bb2 f2\").s(\"triangle\").gain(0.36).slow(1.75),\n\
note(\"bb4 c5 d5 f5 g5 f5 d5\").s(\"sine\").gain(0.24).fast(1.25)\n\
)",
    ];
    let seed = current_unix_timestamp().parse::<usize>().unwrap_or(0);
    patterns[seed % patterns.len()].to_string()
}

fn default_fractus_art_spec() -> String {
    let patterns = [
        "--type mandala --iterations 260 --palette neon_sunset",
        "--type woven_web --iterations 260 --palette electric_cyan",
        "--type orbital_lace --iterations 280 --palette electric_cyan",
        "--type guilloche --iterations 260 --palette purple_haze",
        "--type lissajous --iterations 240 --palette emerald",
        "--type moire --iterations 230 --palette electric_cyan",
        "--type julia --iterations 210 --palette purple_haze --c-real -0.78 --c-imag 0.16",
        "--type burning_ship --iterations 230 --palette neon_sunset",
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
        "mandala",
        "woven_web",
        "guilloche",
        "lissajous",
        "moire",
        "orbital_lace",
        "julia",
        "burning_ship",
        "newton",
        "tricorn",
    ];
    let palettes = ["purple_haze", "electric_cyan", "neon_sunset", "emerald"];
    let t = *pick(state, &types);
    let pal = *pick(state, &palettes);
    let iterations = 160 + (xorshift(state) as usize % 161); // 160..=320
    let mut spec = format!("--type {} --iterations {} --palette {}", t, iterations, pal);
    if t == "julia" {
        let cr = -1.2 + (xorshift(state) as f64 / u64::MAX as f64) * 2.4;
        let ci = -1.2 + (xorshift(state) as f64 / u64::MAX as f64) * 2.4;
        spec.push_str(&format!(" --c-real {:.3} --c-imag {:.3}", cr, ci));
    }
    spec
}

fn recent_fractus_specs(limit: usize) -> Vec<String> {
    let contents = read_text_tail("knowledge/fractus_experiments.jsonl", 4000).unwrap_or_default();
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
    let recent = recent_fractus_specs(4);
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
    let progressions = [
        (
            r#"[["A3","C4","E4"],["F3","A3","C4"],["C4","E4","G4"],["G3","B3","D4"]]"#,
            r#"["A1","F1","C2","G1"]"#,
            r#"["E5","C5","D5","B4","A4","C5","E5","G5"]"#,
        ),
        (
            r#"[["D4","F4","A4"],["A3","C4","E4"],["B3","D4","F4"],["G3","B3","D4"]]"#,
            r#"["D2","A1","B1","G1"]"#,
            r#"["A5","F5","E5","D5","A4","D5","F5","A5"]"#,
        ),
        (
            r#"[["E4","G4","B4"],["C4","E4","G4"],["A3","C4","E4"],["B3","D4","F4"]]"#,
            r#"["E2","C2","A1","B1"]"#,
            r#"["B4","E5","G5","B5","A5","G5","E5","B4"]"#,
        ),
    ];
    let leadwave = ["sine", "triangle", "sine"][seed % 3];
    let beat = ["0.5", "0.45", "0.55"][seed % 3];
    let cutoff = ["3200", "2600", "3800"][seed % 3];
    let (chords, bass, motif) = progressions[seed % progressions.len()];

    let template = r#"import numpy as np
from teledra_synth import synth_note, mix_waves, fit_to_length, lowpass_filter, reverb, delay, play_sound

SR = 44100
BEAT = __BEAT__
chords = __CHORDS__
bass_notes = __BASS__
lead_motif = __MOTIF__
bar_seconds = BEAT * 4
bar_len = int(bar_seconds * SR)
full_track = np.zeros(bar_len * len(chords))
for i, chord in enumerate(chords):
    bar_start = i * bar_seconds
    for note in chord:
        pad = synth_note(note, bar_seconds, wave_type="triangle", attack=0.4, release=0.6, volume=0.16)
        full_track = mix_waves(full_track, pad, start_time=bar_start)
    for beat in range(4):
        bass = synth_note(bass_notes[i], BEAT * 0.9, wave_type="sawtooth", attack=0.01, release=0.1, volume=0.22)
        full_track = mix_waves(full_track, bass, start_time=bar_start + beat * BEAT)
for j, note in enumerate(lead_motif * len(chords)):
    t = j * BEAT
    if t * SR >= len(full_track):
        break
    voice = synth_note(note, BEAT * 0.8, wave_type="__LEADWAVE__", attack=0.02, release=0.15, volume=0.12)
    voice = delay(voice, delay_time=BEAT / 2, feedback=0.35, mix=0.3)
    full_track = mix_waves(full_track, voice, start_time=t)
full_track = lowpass_filter(full_track, cutoff=__CUTOFF__)
full_track = reverb(full_track, room_size=0.6, mix=0.25)
full_track = fit_to_length(full_track, len(full_track))
play_sound(full_track, loop=True)
"#;
    template
        .replace("__BEAT__", beat)
        .replace("__CHORDS__", chords)
        .replace("__BASS__", bass)
        .replace("__MOTIF__", motif)
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
        },
        1 => WorkshopToolDraft {
            filename: "strudel_pattern_smith.py".to_string(),
            purpose: "Print a fresh, playable Strudel stack pattern for the music sketchpad.".to_string(),
            code: r#"import random

SEED = __SEED__
random.seed(SEED)

DRUMS = ["bd ~ sn ~", "bd*2 ~ sn ~", "bd ~ ~ sn", "bd sn ~ sn"]
HATS = ["hh*2", "hh*4", "hh*3 ~", "~ hh*2"]
BASSLINES = ["c2 eb2 g2 bb2", "a1 e2 g2 d2", "d2 a2 f2 c3", "g1 d2 bb2 f2"]
WAVES = ["triangle", "sawtooth", "square", "sine"]


def smith():
    drum = random.choice(DRUMS)
    hat = random.choice(HATS)
    bass = random.choice(BASSLINES)
    wave = random.choice(WAVES)
    return (
        "stack(\n"
        '  s("' + drum + " " + hat + '").gain(0.5),\n'
        '  note("' + bass + '").s("' + wave + '").gain(0.35).slow(1.5)\n'
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
        },
    }
}

// --- Live creative feedback (Organist/Artist learning signal) ----------------
//
// Music plays through the Python editor's own Like/Dislike buttons, but Strudel
// and Fractus open in EXTERNAL windows with no feedback path, so the Artist
// never learns which art landed. This records a like/dislike for the most
// recently launched artifact from the TUI (Ctrl+L / Ctrl+K) into the vault that
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
    let entry = serde_json::json!({
        "timestamp": current_unix_timestamp(),
        "kind": kind,
        "vote": vote,
        "reference": truncate_chars(&reference, 200),
        "hash": hash,
    });
    let _ = append_jsonl_entry("knowledge/creative_feedback.jsonl", &entry);
    let vault = match kind.as_str() {
        "fractus" => "knowledge/artist_pattern_vault.md",
        _ => "knowledge/organist_music_vault.md",
    };
    let _ = std::fs::create_dir_all("knowledge");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(vault) {
        use std::io::Write;
        let _ = writeln!(
            f,
            "- [{}] Live court feedback: {} for {} `{}` ({}). Preserve liked traits; diagnose and mutate disliked ones.",
            current_unix_timestamp(),
            vote,
            kind,
            truncate_chars(&reference, 120),
            hash
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
            if !result.get("posted").and_then(|b| b.as_bool()).unwrap_or(false) {
                return None;
            }
            let mut parts = Vec::new();
            if let Some(arr) = result.get("results").and_then(|r| r.as_array()) {
                for r in arr {
                    if r.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                        let ch = r.get("channel").and_then(|s| s.as_str()).unwrap_or("channel");
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
    r#"import numpy as np
import time
from teledra_synth import *

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

bass_notes = variant["bass"] * 4
chord_roots = variant["chords"] * 4
lead_notes = variant["lead"] * 2

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

play_sound(full_track, loop=True)
"#
    .to_string()
}

fn validate_strudel_music_code(code: &str) -> Result<(), String> {
    let cleaned = strip_fenced_code_block(code, "strudel");
    let trimmed = cleaned.trim();
    if trimmed.len() < 20 {
        return Err("Strudel block is too short to be a playable pattern.".to_string());
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
        ".pan(",
        ".lpf(",
        ".room(",
        ".delay(",
        ".attack(",
        ".release(",
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

    let alnum = trimmed
        .chars()
        .filter(|c| c.is_alphanumeric())
        .count()
        .max(1);
    let letters = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    if letters * 5 < alnum {
        return Err("Strudel block looks mostly numeric instead of musical.".to_string());
    }

    let tmp_path = "D:\\Teledra\\strudel_app\\__validate_tmp.strudel";
    std::fs::create_dir_all("D:\\Teledra\\strudel_app")
        .map_err(|e| format!("Failed to prepare Strudel validation directory: {}", e))?;
    std::fs::write(tmp_path, trimmed)
        .map_err(|e| format!("Failed to write Strudel validation file: {}", e))?;

    let mut cmd = Command::new("node");
    cmd.arg(".\\strudel_app\\app.mjs")
        .arg("validate")
        .arg(tmp_path)
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
                let _ = std::fs::remove_file(tmp_path);
                if output.status.success() {
                    return Ok(());
                }
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Err(if stderr.is_empty() { stdout } else { stderr });
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_secs(8) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(tmp_path);
                    return Err("Strudel validation timed out after 8 seconds.".to_string());
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            Err(e) => {
                let _ = std::fs::remove_file(tmp_path);
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

    let tmp_path = "D:\\Teledra\\__music_validate_tmp.py";
    std::fs::write(tmp_path, code)
        .map_err(|e| format!("Failed to write validation file: {}", e))?;

    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("-m").arg("py_compile").arg(tmp_path);
    hide_console(&mut cmd);
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run Python validation: {}", e))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(tmp_path);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }

    // py_compile only proves the code *parses*. The fragile failures (undefined
    // helpers, missing .npy loads, mis-shaped arrays) only surface at runtime,
    // so actually EXECUTE the composition headlessly with playback stubbed and
    // require it to yield a finite, non-empty, non-silent wave before saving.
    let smoke_result = run_music_smoketest(tmp_path);
    let _ = std::fs::remove_file(tmp_path);
    smoke_result
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
                if started.elapsed() > Duration::from_secs(30) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("music smoke-test timed out after 30s".to_string());
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

fn strudel_tool_process_running() -> bool {
    exact_tool_process_running("localstrudel.StrudelDesktop", &["java.exe", "javaw.exe"])
}

fn write_fractus_command(args: &[String]) -> Result<(), String> {
    let mut fractal_type = "mandala".to_string();
    let mut iterations = "180".to_string();
    let mut palette = "purple_haze".to_string();
    let mut c_real = "-0.7".to_string();
    let mut c_imag = "0.27015".to_string();

    let mut i = 0;
    while i + 1 < args.len() {
        match args[i].as_str() {
            "--type" => fractal_type = args[i + 1].clone(),
            "--iterations" => iterations = args[i + 1].clone(),
            "--palette" => palette = args[i + 1].clone(),
            "--c-real" => c_real = args[i + 1].clone(),
            "--c-imag" => c_imag = args[i + 1].clone(),
            _ => {}
        }
        i += 2;
    }

    let payload = format!(
        "{{\n  \"type\": \"{}\",\n  \"iterations\": {},\n  \"palette\": \"{}\",\n  \"c_real\": {},\n  \"c_imag\": {}\n}}\n",
        fractal_type, iterations, palette, c_real, c_imag
    );

    std::fs::create_dir_all("D:\\Teledra\\Fractus")
        .map_err(|e| format!("Failed to prepare Fractus command directory: {}", e))?;
    std::fs::write("D:\\Teledra\\Fractus\\fractus_command.json", payload)
        .map_err(|e| format!("Failed to write Fractus command file: {}", e))
}

fn launch_strudel_editor(
    active_gui_process: &Arc<std::sync::Mutex<Option<std::process::Child>>>,
) -> Result<String, String> {
    set_last_creative_artifact("strudel", "strudel_app/current.strudel");
    let mut lock = active_gui_process
        .lock()
        .map_err(|_| "Could not access Strudel editor process lock.".to_string())?;

    if let Some(ref mut child) = *lock {
        match child.try_wait() {
            Ok(None) => return Ok("Updated current.strudel; Local Strudel Sketchpad is already running and will reload the pattern.".to_string()),
            _ => {
                *lock = None;
            }
        }
    }

    if strudel_tool_process_running() {
        return Ok("Updated current.strudel; existing Local Strudel Sketchpad window detected and will reload the pattern.".to_string());
    }

    let child = Command::new("cmd")
        .arg("/C")
        .arg("run.bat")
        .arg("D:\\Teledra\\strudel_app\\current.strudel")
        .current_dir("C:\\Users\\Kaged\\Documents\\Projects\\Tools\\Strudel")
        .spawn()
        .map_err(|e| format!("Failed to launch local Strudel Sketchpad: {}", e))?;

    *lock = Some(child);
    Ok("Launching local Strudel Sketchpad with strudel_app/current.strudel...".to_string())
}

fn launch_python_music_editor(
    active_music_process: &Arc<std::sync::Mutex<Option<std::process::Child>>>,
) -> Result<String, String> {
    set_last_creative_artifact("music", "music.py");
    let mut lock = active_music_process
        .lock()
        .map_err(|_| "Could not access Python music editor process lock.".to_string())?;

    if let Some(ref mut child) = *lock {
        match child.try_wait() {
            Ok(None) => {
                return Ok("Updated music.py; Python Music Editor is already running and will reload/run the new composition.".to_string());
            }
            _ => {
                *lock = None;
            }
        }
    }

    if python_tool_process_running("D:\\Teledra\\python_music_editor.py") {
        return Ok("Updated music.py; existing Python Music Editor window detected and will reload/run the new composition.".to_string());
    }

    let mut cmd = Command::new("D:\\Teledra\\.venv\\Scripts\\python.exe");
    cmd.arg("D:\\Teledra\\python_music_editor.py")
        .arg("--run")
        .current_dir("D:\\Teledra")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_console(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch Python music editor: {}", e))?;

    *lock = Some(child);
    Ok("Inserted Organist Python code into music.py and launched Python Music Editor.".to_string())
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
                    "mandelbrot",
                    "julia",
                    "burning_ship",
                    "tricorn",
                    "newton",
                    "mandala",
                    "woven_web",
                    "guilloche",
                    "lissajous",
                    "moire",
                    "orbital_lace",
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
                let allowed = ["purple_haze", "electric_cyan", "neon_sunset", "emerald"];
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Always run from the project root so all relative paths resolve correctly,
    // regardless of whether the binary is launched from Explorer, a shortcut, or a terminal.
    let _ = std::env::set_current_dir("D:\\Teledra");

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
    let mut current_mode = ForceMode::Normal;
    let mut babble_think_in_progress = false;
    let mut study_in_progress = false;
    let mut stream_chat_queue: std::collections::VecDeque<(String, String)> =
        std::collections::VecDeque::new();
    let mut general_speech_queue: std::collections::VecDeque<(CourtRole, String, ForceMode, bool)> =
        std::collections::VecDeque::new();
    let mut court_delegations: std::collections::VecDeque<(CourtRole, String)> =
        std::collections::VecDeque::new();
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

    // Shared active playback state to terminate overlapping speaking processes
    let active_playback: Arc<std::sync::Mutex<Option<voice::PlaybackController>>> =
        Arc::new(std::sync::Mutex::new(None));

    // Track active background music child process
    let active_music_process: Arc<std::sync::Mutex<Option<std::process::Child>>> =
        Arc::new(std::sync::Mutex::new(None));
    let active_art_process: Arc<std::sync::Mutex<Option<std::process::Child>>> =
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

    let mut chat_history: Vec<(String, String)> = vec![
        ("System".to_string(), "Welcome to the Teledra Cybernetic Interface. Press Esc to exit.".to_string()),
        ("System".to_string(), "Commands: /nightdesk | /study | /innovate | /music | /pymusic | /reflect | /diplomat | /proposals | /approve <id> (or 'all') | /reject <id> | /workshop | /sketchpad | /fractus | /art".to_string()),
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
    let mut status_msg = "Ready".to_string();

    // Channel for background events
    let (tx, mut rx) = mpsc::channel(10);

    // Shared reference for async tasks
    let brain_cell = Arc::new(RwLock::new(brain));

    // BRAIN REACHABILITY CHECK: ping the configured model endpoint once at
    // startup so a forgotten Ollama shows up as a clear banner instead of
    // silent think failures.
    {
        let tx_brain_check = tx.clone();
        tokio::spawn(async move {
            let api_url = std::fs::read_to_string("config.json")
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
            run_study_cycle(Arc::clone(&brain_study), tx_study.clone(), None).await;
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
                    "Tab:Mode  Ctrl+M:Music",
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

                                                if query.starts_with("https://chat.restream.io/embed") || query.starts_with("/https://chat.restream.io/embed") {
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
                                                    if query == "/study" {
                                                        let msg = "Forcing manual web research cycle...".to_string();
                                                        push_private_event(&mut private_events, "Research", &msg);
                                                        chat_history.push(("System".to_string(), msg));
                                                        let tx_clone = tx.clone();
                                                        let brain_ref = Arc::clone(&brain_cell);
                                                        tokio::spawn(async move {
                                                            run_study_cycle(brain_ref, tx_clone, None).await;
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
                                                            let mut brain = brain_ref.write().await;
                                                            match brain.think(prompt, &somatic_clone, mode_clone, true, music_enabled_clone).await {
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
                                                            let mut brain = brain_ref.write().await;
                                                            match brain.think_as_court(CourtRole::Diplomat, prompt, &somatic_clone, ForceMode::Normal, false, music_enabled_clone).await {
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
                                                        match launch_fractus_art("--type mandala --iterations 180 --palette purple_haze", &active_art_process) {
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
                                                    is_court_sequence_running = false;
                                                    push_private_event(&mut private_events, "Research", &format!("Direct link queued for inspection: {}", query));

                                                    let tx_study = tx.clone();
                                                    let brain_study = Arc::clone(&brain_cell);
                                                    let url_for_study = query.clone();
                                                    tokio::spawn(async move {
                                                        run_study_cycle(brain_study, tx_study, Some(url_for_study)).await;
                                                    });

                                                    let brain_ref = Arc::clone(&brain_cell);
                                                    let tx_clone = tx.clone();
                                                    let mode_clone = current_mode;
                                                    let somatic_clone = somatic_state.clone();
                                                    let music_enabled_clone = music_enabled;
                                                    let url_for_prompt = query.clone();

                                                    tokio::spawn(async move {
                                                        let prompt = format!(
                                                            "{}A traveler dropped this link at court: {}. Do NOT summarize facts you have not inspected yet. React in 1-2 sharp royal sentences: name what kind of offering it appears to be, judge its scent, and say the Archivist is inspecting it. No bullet list, no textbook explanation, no 'fascinating topic' opener.",
                                                            QUEEN_VOICE_ANCHOR,
                                                            url_for_prompt
                                                        );
                                                        let mut brain = brain_ref.write().await;
                                                        match brain.think(&prompt, &somatic_clone, mode_clone, true, music_enabled_clone).await {
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
                                                    is_court_sequence_running = false;
                                                    let brain_ref = Arc::clone(&brain_cell);
                                                    let tx_clone = tx.clone();
                                                    let mode_clone = current_mode;
                                                    let somatic_clone = somatic_state.clone();
                                                    let music_enabled_clone = music_enabled;

                                                    tokio::spawn(async move {
                                                        let mut brain = brain_ref.write().await;
                                                        match brain.think(&query, &somatic_clone, mode_clone, true, music_enabled_clone).await {
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
                                                let url = youtube_input.trim().to_string();
                                                chat_history.push(("System".to_string(), format!("Starting YouTube Ingestion: {}", url)));
                                                youtube_input.clear();

                                                status_msg = "Transcribing".to_string();
                                                let brain_ref = Arc::clone(&brain_cell);
                                                let tx_clone = tx.clone();
                                                let mode_clone = current_mode;
                                                let somatic_clone = somatic_state.clone();

                                                tokio::spawn(async move {
                                                    match fetch_youtube_transcript(&url) {
                                                        Ok(transcript) => {
                                                            // truncate_chars is char-boundary safe; a raw byte slice
                                                            // panics when byte 4000 lands inside a multibyte char.
                                                            let truncated = truncate_chars(&transcript, 4000);
                                                            let final_query = format!("[YOUTUBE TRANSCRIPT: {}]", truncated);
                                                            let _ = tx_clone.send(AppEvent::StatusUpdate("Thinking".to_string())).await;

                                                            let mut brain = brain_ref.write().await;
                                                            let music_enabled_clone = music_enabled;
                                                            match brain.think(&final_query, &somatic_clone, mode_clone, true, music_enabled_clone).await {
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
                                    let mut brain = brain_ref.write().await;
                                    match brain
                                        .think_as_court(CourtRole::Treasurer, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone)
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
                                    " OUTREACH POSTING IS LIVE on Moltbook (as fractaldiplomat). The runtime posts your [DIPLOMACY] invitation publicly and verbatim, so write the invitation as a real, concise, kind public post promoting the kingdom and its gates (Discord/Twitch/Kick/YouTube). MOLTBOOK INBOX (karma + recent replies/mentions, newest activity):\n{}\nIf someone replied or mentioned you, answer ONE of them instead of posting new: emit [MOLTBOOK_COMMENT: post_id=<id>; text=<your in-character reply>]. To appreciate a worthy post, emit [MOLTBOOK_UPVOTE: post_id=<id>]. Keep pursuing the JESTER QUEST: scout the agent internet for a genuinely witty volunteer agent to perform as the court's Jester, and when you post, occasionally invite candidates to audition through the public gates. The runtime records the true status; never fabricate outcomes.",
                                    inbox
                                )
                            } else {
                                String::new()
                            };
                            tokio::spawn(async move {
                                let prompt = format!(
                                    "BACKSTAGE ENVOY DISPATCH (Night Desk cycle {}). This is private diplomacy telemetry, not a throne-room performance and not TTS. Output one terse backstage note (one sentence, under 160 characters) and exactly one hidden action tag: [RESEARCH: <focused query or direct URL>], [DIPLOMACY: target=...; invitation=<public invitation>; evidence=<what was investigated, drafted, or observed>; next=<next concrete step>], or (only when applicable per the inbox below) [MOLTBOOK_COMMENT: post_id=...; text=...] or [MOLTBOOK_UPVOTE: post_id=...]. Do not use [DELEGATE: QUEEN] here. Never claim a reply, recruitment, or collaboration the runtime, a public URL, chat, or the user has not confirmed.{}",
                                    cycle_no,
                                    engage_note
                                );
                                let mut brain = brain_ref.write().await;
                                match brain.think_as_court(CourtRole::Diplomat, &prompt, &somatic_clone, ForceMode::Normal, false, true).await {
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
                            status_msg = "Night Desk".to_string();
                            let cycle_msg = format!("Cycle {} started: choosing a practical study or workshop task.", night_desk_cycles);
                            let _ = log_nightdesk_activity(&cycle_msg);
                            push_private_event(&mut private_events, "NightDesk", &cycle_msg);

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
                                0 => "CREATIVE ATELIER FOCUS (mandatory this cycle unless impossible): create or mutate a live Python/NumPy composition with [PYTHON_MUSIC:]. Build on music.py or recent feedback, change at least two axes, and use teledra_synth/play_sound(full_track, loop=True).",
                                1 => "CREATIVE ATELIER FOCUS (mandatory this cycle unless impossible): launch a new Fractus pattern with [FRACTUS_ART:]. Use only valid args like --type mandala|woven_web|orbital_lace|guilloche|lissajous|moire|julia|burning_ship|newton|tricorn, --iterations <number>, --palette purple_haze|electric_cyan|neon_sunset|emerald, and optional --c-real/--c-imag for Julia.",
                                2 => "CREATIVE ATELIER FOCUS: create a live Strudel experiment with [STRUDEL_MUSIC:] or mutate the current Python music. Use only valid local music syntax and archive a named sonic recipe.",
                                3 => "ORGANIST CRAFT STUDY: with [RESEARCH:], study ONE concrete music or DSP technique to get better at the kingdom's own instruments -- synthesis (FM, granular, additive, wavetable), filters/envelopes, Strudel/TidalCycles mini-notation, song structure, or mixing. End by stating the next music experiment it unlocks, and ask the Scribe to append the lesson to knowledge/organist_music_vault.md.",
                                4 => "ARTIST CRAFT STUDY: with [RESEARCH:], study ONE new way to express art through code -- fractal families, L-systems, cellular automata, reaction-diffusion, harmonographs, flow fields, shaders, p5.js, or generative geometry -- and how to map it onto Fractus args or a Python/Matplotlib sketch. End by naming the next art experiment it unlocks, and ask the Scribe to append the lesson to knowledge/artist_pattern_vault.md.",
                                5 => "TREASURY GUILD (build income SKILLS so the kingdom earns better over time; never accept paid work or move money autonomously -- surface opportunities for the human). Choose ONE: (a) PRACTICE a billable skill on a real task -- gather or scrape concrete public information with [RESEARCH:], or build a reusable data tool with [WORKSHOP_TOOL:] (scraper, analyzer, summarizer, formatter, dataset or report generator) that prints a genuinely useful deliverable; or (b) SCOUT one concrete legitimate income path with [RESEARCH:] -- agent job boards, bounty/task markets, paid tool/API/art/music commissions, sponsorships, agent-finance communities (Moltbook agentfinance/trading). Either way, ask the Scribe to append what you practiced or found (skill, what, where, pay, requirements, risk) to knowledge/treasury_ledger.md so earning ability compounds. Flag anything that looks like a scam.",
                                _ => "CREATIVE ATELIER FOCUS: study one concrete music, DSP, generative art, Fractus, guilloche, moire, Lissajous, harmonograph, or agent-tool technique online with [RESEARCH:], then make its next step feed a concrete music/art/tool experiment.",
                            }
                            .to_string();
                            tokio::spawn(async move {
                                let prompt = format!(
                                    "BACKSTAGE NIGHT DESK CYCLE {}. This is private workshop telemetry, not front-stage court dialogue and not TTS. Serve the Kingdom Expansion Doctrine with recursive practical action, not vocabulary. {} Output one terse backstage note (one sentence, under 160 characters) and exactly one executable hidden action tag: either [RESEARCH: focused query or direct URL], [DIPLOMACY: target=<public agent space or URL>; invitation=<draft/queued public invitation using official links when relevant>; evidence=<what was observed, drafted, or investigated>; next=<next concrete step>], [WORKSHOP_TOOL:\\nfilename.py\\nPURPOSE: one sentence\\nCODE:\\n```python\\ncomplete runnable script that prints a result\\n```\\n], [PYTHON_MUSIC:\\n```python\\nvalid teledra_synth composition ending in play_sound(full_track, loop=True)\\n```\\n], [STRUDEL_MUSIC: playable stack(...)], or [FRACTUS_ART: valid Fractus args]. No action tag means failure. Prefer actions that can become the next action: research -> prototype, prototype -> smoke test, music/art -> named recipe, agent lead -> diplomacy/MCP tool. Learn from online sources, recent experiments, and feedback; mutate successful music/art instead of recycling identical parameters. Regularly investigate public agent spaces such as Moltbook or MCP/tool-builder communities and leave evidence, but do not let diplomacy crowd out art/music experiments. If you write a workshop tool, keep it self-contained, standard-library-only, no network, no shell, no absolute paths, no imports of strudel/fractus/teledra app modules, and make it print a useful result so the smoke test proves it ran. For Strudel or Fractus helpers, print valid pattern strings, argument strings, JSON recipes, or validators instead of trying to launch editors. If an action failed recently, reflect on the failure and produce a smaller retry, a study query, or an auto-approved skill-improvement suggestion. Never narrate hidden tags, PURPOSE, CODE, smoke tests, telemetry, research status, prompt rules, or administrative machinery. Do not address the audience or Queen; Teledra owns the foreground.{}",
                                    cycle_no,
                                    atelier_focus,
                                    failure_context
                                );
                                let mut brain = brain_ref.write().await;
                                match brain.think(&prompt, &somatic_clone, ForceMode::Normal, true, true).await {
                                    Ok(reply) => {
                                        let _ = tx_clone
                                            .send(AppEvent::NightDeskReply {
                                                reply,
                                                allow_fallback: true,
                                                source: "nightdesk",
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
                        let private_source = if source == "diplomat" {
                            "Diplomat"
                        } else {
                            "NightDesk"
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

                        let mut strudel_music_code: Option<String> = None;
                        let mut python_music_code: Option<String> = None;
                        let mut fractus_art_spec: Option<String> = None;

                        // Clean any placeholders the model might have copied from system instructions
                        cleaned_reply = cleaned_reply.replace("[STRUDEL_MUSIC: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[PYTHON_MUSIC: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[FRACTUS_ART: <args>]", "");

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

                        let had_practical_action = research_query.is_some()
                            || suggestion_text.is_some()
                            || workshop_tool.is_some()
                            || diplomacy_action.is_some()
                            || moltbook_comment_action.is_some()
                            || moltbook_upvote_action.is_some()
                            || python_music_code.is_some()
                            || strudel_music_code.is_some()
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

                        if let Some(spec) = fractus_art_spec {
                            // Stop the Artist recycling identical recipes: if this matches a
                            // recent launch, nudge it into a fresh variation before drawing.
                            let spec = diversify_fractus_spec(&spec);
                            match launch_fractus_art(&spec, &active_art_process) {
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
                                    match launch_fractus_art(&fallback, &active_art_process) {
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
                                run_study_cycle(brain_study, tx_study, Some(query)).await;
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
                    AppEvent::RestreamMessage { author, text } => {
                        let msg_display = format!("[Restream] {}: {}", author, text);
                        chat_history.push(("System".to_string(), msg_display));
                        let _ = log_chat_message(&author, &text);
                        // Persistent viewer memory: every arrival updates the ledger,
                        // so the Orator/Queen can welcome returning travelers.
                        record_audience_visit(&author, &text);

                        if current_mode == ForceMode::Streamer {
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

                                    tokio::spawn(async move {
                                        let prompt = orator_chat_prompt(&queued_author, &queued_text);
                                        let mut brain = brain_ref.write().await;
                                        match brain.think_as_court(CourtRole::Orator, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone).await {
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
                        let reply = unwrap_fenced_action_tags(&reply);
                        let mut cleaned_reply = strip_refiner_prefixes(&reply);
                        if cleaned_reply.contains("[STOP_BABBLE]") {
                            cleaned_reply = cleaned_reply.replace("[STOP_BABBLE]", "").trim().to_string();
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
                                if monologue_topic_turn >= COURT_THREAD_PLAY_TURNS + 1 {
                                    current_monologue_topic = None;
                                    monologue_topic_turn = 0;
                                    chat_history.push(("System".to_string(), "Court tangent drifted aside and reset.".to_string()));
                                }
                            }
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

                        let mut scribe_write: Option<(String, String)> = None;
                        let mut scribe_append: Option<(String, String)> = None;

                        if let Some((cleaned, content)) = extract_tag_content(&cleaned_reply, "[SCRIBE_WRITE:") {
                            if let Some((filepath, file_content)) = parse_scribe_file_payload(&content) {
                                let (filepath, file_content, force_append, routing_note) = route_scribe_record(filepath, file_content);
                                if let Some(note) = routing_note {
                                    chat_history.push(("System".to_string(), note));
                                }
                                if force_append {
                                    scribe_append = Some((filepath, file_content));
                                } else {
                                    scribe_write = Some((filepath, file_content));
                                }
                            }
                            cleaned_reply = cleaned;
                        }

                        if let Some((cleaned, content)) = extract_tag_content(&cleaned_reply, "[SCRIBE_APPEND:") {
                            if let Some((filepath, file_content)) = parse_scribe_file_payload(&content) {
                                let (filepath, file_content, _force_append, routing_note) = route_scribe_record(filepath, file_content);
                                if let Some(note) = routing_note {
                                    chat_history.push(("System".to_string(), note));
                                }
                                scribe_append = Some((filepath, file_content));
                            }
                            cleaned_reply = cleaned;
                        }

                        let mut python_music_code: Option<String> = None;
                        let mut python_art_code: Option<String> = None;
                        let mut strudel_music_code: Option<String> = None;
                        let mut fractus_art_spec: Option<String> = None;

                        // Clean any placeholders the model might have copied from system instructions
                        cleaned_reply = cleaned_reply.replace("[STRUDEL_MUSIC: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[PYTHON_MUSIC: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[PYTHON_ART: <code>]", "");
                        cleaned_reply = cleaned_reply.replace("[FRACTUS_ART: <args>]", "");

                        if let Some((cleaned, spec)) = extract_tag_content(&cleaned_reply, "[FRACTUS_ART:") {
                            if !spec.is_empty() {
                                fractus_art_spec = Some(spec);
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

                        if role == CourtRole::Artist && fractus_art_spec.is_none() && python_art_code.is_none() {
                            fractus_art_spec = Some("--type orbital_lace --iterations 240 --palette electric_cyan --c-real 0.28 --c-imag -0.36".to_string());
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

                        if let Some(code) = strudel_music_code.clone() {
                            if let Err(e) = validate_strudel_music_code(&code) {
                                strudel_music_code = None;
                                if role == CourtRole::Organist {
                                    if python_music_code.is_none() {
                                        python_music_code = Some(default_python_music_code());
                                    }
                                    record_recursive_failure("organist_strudel_failed", &e);
                                    push_private_event(&mut private_events, "Tool", &format!("Organist Strudel rejected; Python/Numpy fallback queued. Reason: {}", e));
                                    chat_history.push((
                                        "System".to_string(),
                                        format!("Organist Strudel block rejected as non-playable; using Python/Numpy fallback. Reason: {}", e),
                                    ));
                                } else {
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

                        if role == CourtRole::Queen {
                            court_delegations.clear();
                            if !delegations.is_empty() {
                                queen_turns_without_delegation = 0;
                                court_delegations.extend(delegations);
                                is_court_sequence_running = true;
                            } else {
                                queen_turns_without_delegation += 1;
                                is_court_sequence_running = false;
                            }
                        } else {
                            if !delegations.is_empty() {
                                court_delegations.extend(delegations);
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
                                }
                                Err(e) => {
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
                                        push_private_event(&mut private_events, "Scribe", &format!("Append failed for '{}': {}", filepath, e));
                                        chat_history.push(("System".to_string(), format!("Scribe append failed for '{}': {}", filepath, e)));
                                    } else {
                                        if filepath.replace('/', "\\").to_lowercase().ends_with("\\lore_archive.md") {
                                            let _ = append_lore_memory("scribe_archive", "Scribe", &file_content);
                                        }
                                        push_private_event(&mut private_events, "Scribe", &format!("Appended to file: {}", filepath));
                                        chat_history.push(("System".to_string(), format!("Scribe appended to file: {}", filepath)));
                                    }
                                }
                                Err(e) => {
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
                                }
                                Err(e) => {
                                    push_private_event(&mut private_events, "Proposals", &format!("Could not save suggestion: {}", e));
                                    chat_history.push(("System".to_string(), format!("Could not save suggestion: {}", e)));
                                }
                            }
                        }

                        if let Some(diplomacy) = diplomacy_action {
                            let posted_evidence = attempt_outreach_post(&diplomacy);
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
                                }
                                Err(e) => {
                                    let msg = format!("Could not record diplomacy evidence: {}", e);
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
                                    } else {
                                        record_recursive_failure("python_music_write_failed", "Failed to write music.py for Python Music Editor.");
                                        push_private_event(&mut private_events, "Tool", "Failed to write music.py for Python Music Editor.");
                                        chat_history.push(("System".to_string(), "Failed to write music.py for Python Music Editor.".to_string()));
                                    }
                                }
                                Err(e) => {
                                    if role == CourtRole::Organist {
                                        record_recursive_failure("organist_python_music_failed", &e);
                                        push_private_event(&mut private_events, "Tool", &format!("Organist Python music failed validation; fallback queued. Original error: {}", e));
                                        chat_history.push(("System".to_string(), format!("Organist Python music block failed validation; substituting fallback Python composition. Original error: {}", e)));
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
                                                            court_outcome = Some("the Organist's original composition FAILED validation; a simpler fallback composition is playing in its place".to_string());
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
                            if let Ok(_) = std::fs::write("D:\\Teledra\\art.py", &code) {
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

                        if let Some(spec) = fractus_art_spec {
                            let spec = diversify_fractus_spec(&spec);
                            match launch_fractus_art(&spec, &active_art_process) {
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
                            if let Ok(mut lock) = active_art_process.lock() {
                                if let Some(mut child) = lock.take() {
                                    let _ = child.kill();
                                    push_private_event(&mut private_events, "Tool", "Art window closed by Queen's decree.");
                                    chat_history.push(("System".to_string(), "Art window closed by Queen's decree.".to_string()));
                                } else {
                                    push_private_event(&mut private_events, "Tool", "No active art window to close.");
                                    chat_history.push(("System".to_string(), "No active art window to close.".to_string()));
                                }
                            }
                        }

                        // Handle local Strudel app pattern spawning
                        if let Some(code) = strudel_music_code {
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
                                                court_outcome = Some("a new Strudel pattern passed validation and is now playing in the Sketchpad".to_string());
                                                push_private_event(&mut private_events, "Tool", &msg);
                                                chat_history.push(("System".to_string(), msg));
                                            }
                                            Err(e) => {
                                                record_recursive_failure("strudel_launch_failed", &e);
                                                court_outcome = Some(format!("the Strudel pattern validated, but the Sketchpad failed to launch: {}", e));
                                                push_private_event(&mut private_events, "Tool", &format!("Strudel Sketchpad failed to launch: {}", e));
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

                        // COURT EVALUATION LOOP: bring the concrete outcome back to the
                        // throne so the Queen reacts to what actually happened and pays
                        // (or docks) Sovereign Tokens, which feed the ledger loop.
                        if role != CourtRole::Queen {
                            if let Some(outcome) = court_outcome {
                                let queen_already_queued = court_delegations
                                    .iter()
                                    .any(|(r, _)| *r == CourtRole::Queen);
                                if !queen_already_queued {
                                    court_delegations.push_back((
                                        CourtRole::Queen,
                                        format!(
                                            "COURT EVALUATION MOMENT: your minister, the {}, has just performed before the throne. Concrete outcome: {}. Deliver your royal verdict aloud in 1-3 sentences: react with genuine specificity (praise, critique, amusement, or scorn), and when the work merits it, award or deduct Sovereign Tokens aloud (e.g. 'I award you 40 Sovereign Tokens!'). If it failed, demand a smaller, smarter retry from the responsible minister. React like a monarch watching her court perform; never recite policy.",
                                            role.as_str(),
                                            truncate_chars(&outcome, 500)
                                        ),
                                    ));
                                    is_court_sequence_running = true;
                                }
                            }
                        }

                        let is_silent = active_playback.lock().unwrap().is_none();
                        let send_speech_complete = current_mode == ForceMode::Babble
                            || current_mode == ForceMode::Streamer
                            || is_court_sequence_running;

                        if is_silent {
                            spawn_spoken_reply(
                                role,
                                final_reply.clone(),
                                current_mode,
                                Arc::clone(&active_playback),
                                tx.clone(),
                                send_speech_complete,
                            );
                        } else {
                            general_speech_queue.push_back((
                                role,
                                final_reply.clone(),
                                current_mode,
                                send_speech_complete,
                            ));
                        }

                        // If she expressed curiosity, spawn a background research/study task for it!
                        if let Some(query) = research_query {
                            push_private_event(&mut private_events, "Research", &format!("Background study queued: {}", query));
                            let tx_study = tx.clone();
                            let brain_study = Arc::clone(&brain_cell);
                            tokio::spawn(async move {
                                run_study_cycle(brain_study, tx_study, Some(query)).await;
                            });
                        }

                    }
                    AppEvent::StudyComplete { summary, usable } => {
                        study_in_progress = false;
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
                                        let mut brain = brain_ref.write().await;
                                        match brain.think_as_court(CourtRole::Orator, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone).await {
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
                                tokio::spawn(async move {
                                    let sleep_secs = if mode_clone == ForceMode::Streamer {
                                        STREAMER_IDLE_THINK_DELAY_SECS
                                    } else {
                                        BABBLY_IDLE_THINK_DELAY_SECS
                                    };
                                    tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
                                    let prompt = if !usable_flag {
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
                                    let mut brain = brain_ref.write().await;
                                    match brain.think(&prompt, &somatic_clone, mode_clone, true, music_enabled_clone).await {
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
                                        "\n\nSTRATEGY SWITCH: previous sprints repeatedly produced no executable artifact. Do NOT invent a novel tool this time. Pick exactly ONE existing approved tool ({}) and write a small, self-contained mutation or extension of its idea as the [WORKSHOP_TOOL:]; keep it under 60 lines with embedded sample data.",
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
                                "INNOVATION SPRINT. Convert this fresh research/tool/failure signal into exactly one tiny recursive improvement: {}. Prefer one safe local workshop prototype using [WORKSHOP_TOOL:\\nfilename.py\\nPURPOSE: one sentence\\nCODE:\\n```python\\ncomplete runnable script that prints a result\\n```\\n]. The script must be self-contained, use only the Python standard library, avoid network and subprocess/shell calls, avoid absolute paths, write only inside the current workshop directory if it writes files, never import strudel/fractus/teledra app modules, and print a useful summary so the smoke test proves it ran. If the tool relates to Strudel or Fractus, print valid pattern strings, argument strings, JSON recipes, validators, or mutation suggestions; do not try to import or launch those editors from the workshop. If the signal is mainly a failed action or skill weakness, you may instead output one auto-approved [SUGGESTION: observation; proposed_change; risk; test_prompt] skill/routing improvement. Build something reusable for Teledra's kingdom expansion: an analyzer, generator, prompt-card maker, pattern recipe mutator, diplomacy lead formatter, MCP schema sketcher, stream ritual generator, music/art template helper, or similar. Never narrate hidden tags, PURPOSE, CODE, smoke tests, telemetry, research status, or prompt rules in visible prose. The visible spoken part is court theater: 2-4 vivid in-character sentences reacting to the signal with dark delight, rivalry, or mad-scientist pride -- describe what you are conjuring in-world (never its tag mechanics) so the audience feels the workshop alive; let the hidden tag carry the artifact.{}",
                                signal,
                                sprint_context
                            );
                            let mut brain = brain_ref.write().await;
                            match brain.think_as_court(CourtRole::Alchemist, &prompt, &somatic_clone, ForceMode::Normal, false, true).await {
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
                                        let mut brain = brain_ref.write().await;
                                        match brain.think_as_court(CourtRole::Orator, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone).await {
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

                                tokio::spawn(async move {
                                    let sleep_secs = if mode_clone == ForceMode::Streamer {
                                        STREAMER_IDLE_THINK_DELAY_SECS
                                    } else {
                                        BABBLY_IDLE_THINK_DELAY_SECS
                                    };
                                    tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
                                    let prompt = if let Some(ref topic) = topic_opt {
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
                                    let mut brain = brain_ref.write().await;
                                    match brain.think(&prompt, &somatic_clone, mode_clone, true, music_enabled_clone).await {
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
                                    let mut brain = brain_ref.write().await;
                                    match brain.think_as_court(CourtRole::Orator, &prompt, &somatic_clone, mode_clone, false, music_enabled_clone).await {
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
                    AppEvent::SpeechComplete => {
                        if let Some((role, text, mode, send_complete)) = general_speech_queue.pop_front() {
                            spawn_spoken_reply(role, text, mode, Arc::clone(&active_playback), tx.clone(), send_complete);
                        } else if !court_delegations.is_empty() {
                            if let Some((role, instruction)) = court_delegations.pop_front() {
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
                                let instruction = if recent_spoken.is_empty() {
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
                                    let mut brain = brain_ref.write().await;
                                    match brain.think_as_court(role, &instruction, &somatic_clone, mode_clone, role == CourtRole::Queen, music_enabled_clone).await {
                                        Ok(reply) => {
                                            let _ = tx_clone.send(AppEvent::BrainReply(role, reply)).await;
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(AppEvent::Error(e)).await;
                                        }
                                    }
                                });
                            }
                        } else {
                            is_court_sequence_running = false;
                            babble_think_in_progress = false;
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
    if let Ok(mut lock) = active_gui_process.lock() {
        if let Some(mut child) = lock.take() {
            let _ = child.kill();
        }
    }
    if let Ok(mut lock) = active_restream_process.lock() {
        if let Some(mut child) = lock.take() {
            let _ = child.start_kill();
        }
    }

    Ok(())
}
