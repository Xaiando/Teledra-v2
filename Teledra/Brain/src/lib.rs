use std::path::PathBuf;
use std::time::{Duration, Instant};

mod prompt_fit;
pub use prompt_fit::{PROMPT_CHAR_BUDGET, PromptTrim};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const QWEN_MODEL: &str = "qwen2.5:7b";
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// The Ollama model every office uses: `TELEDRA_MODEL`, else [`QWEN_MODEL`].
/// Read once, so one court never mixes models mid-session.
pub fn model_name() -> &'static str {
    static MODEL: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    MODEL.get_or_init(|| {
        std::env::var("TELEDRA_MODEL")
            .ok()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .map_or(QWEN_MODEL, |name| Box::leak(name.into_boxed_str()))
    })
}
const KEEP_ALIVE: &str = "30m";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CONTENT_CHARS: usize = 64 * 1024;
/// A reply below this many words is too short to judge by letter statistics.
const DEGENERATE_MIN_WORDS: usize = 4;
/// Share of vowel-less words above which a reply has stopped being English.
/// See `is_degenerate` for how this was measured.
const DEGENERATE_VOWELLESS_SHARE: f32 = 0.15;
const VOWELS: &[u8] = b"aeiouy";
/// llama.cpp leaves repetition unpenalised at 1.0, and this model's manifest
/// carries no params layer, so nothing held a loop back. 1.1 is Ollama's own
/// documented default; see `is_degenerate` for what its absence cost.
const REPEAT_PENALTY: f32 = 1.1;
/// llama.cpp looks back 64 tokens, which is shorter than one court turn: a
/// loop that starts early in an answer outlives the window that would have
/// penalised it.
const REPEAT_LAST_N: i32 = 256;
const MAX_PRIVATE_REVIEW_TIME: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BrainConfig {
    pub ollama_url: String,
    pub connect_timeout: Duration,
    pub response_timeout: Duration,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            ollama_url: std::env::var("TELEDRA_OLLAMA_URL")
                .unwrap_or_else(|_| DEFAULT_OLLAMA_URL.to_owned()),
            connect_timeout: Duration::from_secs(3),
            response_timeout: Duration::from_secs(120),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Brain {
    client: Client,
    base_url: String,
    completion_timeout: Duration,
    /// Where trimmed-prompt reports are appended, one JSON line each.
    oversize_log: Option<PathBuf>,
}

impl Brain {
    /// Appends a JSON line to `path` whenever a prompt had to be trimmed to
    /// fit the model's context, naming the messages that were too large.
    pub fn with_oversize_log(mut self, path: PathBuf) -> Self {
        self.oversize_log = Some(path);
        self
    }

    fn report_trim(&self, trim: &PromptTrim) {
        let Some(path) = &self.oversize_log else {
            return;
        };
        let line = serde_json::json!({
            "unix_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            "original_chars": trim.original_chars,
            "kept_chars": trim.kept_chars,
            "trimmed": trim.trimmed.iter().map(|(index, role, chars, first)| serde_json::json!({
                "index": index, "role": role, "chars": chars, "starts": first,
            })).collect::<Vec<_>>(),
        });
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
    }

    pub fn new(config: BrainConfig) -> Result<Self, BrainError> {
        let base_url = normalize_base_url(&config.ollama_url)?;
        let client = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.response_timeout)
            .build()
            .map_err(BrainError::Client)?;
        Ok(Self {
            client,
            base_url,
            completion_timeout: config.response_timeout,
            oversize_log: None,
        })
    }

    /// Total wall time for one answer, including any retries and private review.
    pub fn completion_timeout(&self) -> Duration {
        self.completion_timeout
    }

    pub async fn check(&self) -> Result<BrainHealth, BrainError> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map_err(BrainError::Unavailable)?
            .error_for_status()
            .map_err(BrainError::Unavailable)?;
        let tags: TagsResponse = response.json().await.map_err(BrainError::InvalidResponse)?;
        let installed = tags.models.iter().any(|model| model.name == model_name());
        if !installed {
            return Err(BrainError::ModelMissing(model_name()));
        }
        Ok(BrainHealth {
            endpoint: self.base_url.clone(),
            model: model_name(),
        })
    }

    pub async fn complete(
        &self,
        messages: &[Message],
        temperature: f32,
    ) -> Result<String, BrainError> {
        self.complete_until(
            messages,
            temperature,
            Instant::now() + self.completion_timeout,
        )
        .await
    }

    async fn complete_until(
        &self,
        messages: &[Message],
        temperature: f32,
        deadline: Instant,
    ) -> Result<String, BrainError> {
        if messages.is_empty() {
            return Err(BrainError::EmptyConversation);
        }
        let mut content = match self.complete_once(messages, temperature, deadline).await {
            Err(BrainError::TruncatedResponse) => {
                let mut retry = messages.to_vec();
                retry.push(Message::user("The previous generation reached its output limit. Return a shorter COMPLETE answer. For an artifact, use compact JSON, short patterns, and all required closing delimiters. Do not abbreviate fields or repeat nulls beyond the schema limit."));
                self.complete_once(&retry, temperature.min(0.5), deadline)
                    .await?
            }
            result => result?,
        };
        if contains_cjk(&content) {
            let mut retry = messages.to_vec();
            retry.push(Message::system(
                "Regenerate the answer entirely in English. Use no Chinese, Japanese, Korean, or full-width characters.",
            ));
            // A timed-out language retry must not discard a complete draft.
            content = match self.complete_once(&retry, temperature, deadline).await {
                Ok(rewritten) => rewritten,
                Err(BrainError::DeadlineExceeded) => content,
                Err(error) => return Err(error),
            };
            if contains_cjk(&content) {
                let stripped = strip_cjk(&content);
                // Stripping what is left of a collapsed reply leaves a stub:
                // a corrupt Chinese answer opening with "The" became exactly
                // that, which the court spoke and then quoted back to the
                // next office as the anchor. A reply that loses most of
                // itself here failed; it is not a short answer.
                if stripped.trim().chars().count() * 2 < content.trim().chars().count() {
                    return Err(BrainError::DegenerateResponse);
                }
                content = stripped;
            }
        }
        if is_degenerate(&content) {
            let mut retry = messages.to_vec();
            retry.push(Message::system(
                "The previous answer was not language. Answer again in ordinary English sentences.",
            ));
            content = match self.complete_once(&retry, temperature, deadline).await {
                Ok(second) if !is_degenerate(&second) => second,
                // A second collapse is the model's state, not this prompt's
                // fault. Fail the turn rather than speak it or anchor on it.
                Ok(_) | Err(BrainError::DeadlineExceeded) => {
                    return Err(BrainError::DegenerateResponse);
                }
                Err(error) => return Err(error),
            };
        }
        if content.trim().is_empty() {
            return Err(BrainError::EmptyResponse);
        }
        Ok(content.trim().to_owned())
    }

    /// V1 audited tool-bearing ministers privately before their work reached
    /// speech or the action parser. The Queen remains latency-sensitive and
    /// ships directly; specialists are refined only for a concrete hard-rule
    /// violation, never merely because a critic prefers blander prose.
    pub async fn complete_reviewed(
        &self,
        messages: &[Message],
        temperature: f32,
        role: &str,
    ) -> Result<String, BrainError> {
        self.complete_reviewed_with_budget(messages, temperature, role, self.completion_timeout)
            .await
    }

    /// Bound the complete draft/review transaction; dropping this future also
    /// drops the active HTTP request instead of leaving a detached retry task.
    pub async fn complete_reviewed_with_budget(
        &self,
        messages: &[Message],
        temperature: f32,
        role: &str,
        budget: Duration,
    ) -> Result<String, BrainError> {
        let budget = budget.min(self.completion_timeout);
        let deadline = Instant::now() + budget;
        let draft = self.complete_until(messages, temperature, deadline).await?;
        if !role_needs_review(role) {
            return Ok(strip_editor_leak(&draft));
        }
        // An optional audit may improve a completed answer, but cannot keep it
        // hidden behind another full generation timeout (or a chain of them).
        let review_deadline =
            deadline.min(Instant::now() + MAX_PRIVATE_REVIEW_TIME.min(budget / 4));
        // The caller has already bounded the conversation. Preserve every role
        // and its ordering: a later runtime message can correct old history,
        // establish the active topic, or constrain this turn's spoken length.
        // Reviewing only the latest user text loses all of that evidence.
        let mut critic = messages.to_vec();
        critic.push(Message::system(critic_instruction(role)));
        critic.push(Message::user(format!(
            "Audit the following draft for the current request in the preceding conversation. Use the latest runtime context when assessing claims and required actions. Return only the audit JSON.\n\nDRAFT RESPONSE:\n{}",
            truncate_chars(&draft, 48_000)
        )));
        let verdict = match self
            .complete_once_formatted(&critic, 0.1, None, review_deadline, 256)
            .await
        {
            Ok(verdict) => verdict,
            Err(_) => return Ok(strip_editor_leak(&draft)),
        };
        if !critic_requests_revision(&verdict) {
            return Ok(strip_editor_leak(&draft));
        }
        let mut refiner = messages.to_vec();
        refiner.push(Message::system(refiner_instruction(role)));
        refiner.push(Message::user(format!(
            "Edit this draft for the current request in the preceding conversation. Retain that request's length limit and the current runtime and conversation context. Return only the minimally corrected response.\n\nORIGINAL DRAFT:\n{}\n\nPRIVATE CRITIQUE:\n{}",
            truncate_chars(&draft, 48_000),
            truncate_chars(&verdict, 4_000)
        )));
        let refined = match self.complete_once(&refiner, 0.35, review_deadline).await {
            Ok(refined) if !refined.trim().is_empty() => strip_editor_leak(&refined),
            _ => return Ok(strip_editor_leak(&draft)),
        };
        if lost_executable_tag(&draft, &refined) {
            return Ok(strip_editor_leak(&draft));
        }
        Ok(refined)
    }

    async fn complete_once(
        &self,
        messages: &[Message],
        temperature: f32,
        deadline: Instant,
    ) -> Result<String, BrainError> {
        self.complete_once_formatted(messages, temperature, None, deadline, 4_096)
            .await
    }

    pub async fn complete_structured(
        &self,
        messages: &[Message],
        temperature: f32,
        schema: &serde_json::Value,
    ) -> Result<String, BrainError> {
        self.complete_structured_with_budget(messages, temperature, schema, self.completion_timeout)
            .await
    }

    /// One schema-constrained request, without prose review or language rewrites.
    /// The caller owns artifact validation/repair and passes its remaining total
    /// budget; this method never creates a detached retry or resets that budget.
    pub async fn complete_structured_with_budget(
        &self,
        messages: &[Message],
        temperature: f32,
        schema: &serde_json::Value,
        budget: Duration,
    ) -> Result<String, BrainError> {
        if messages.is_empty() {
            return Err(BrainError::EmptyConversation);
        }
        let mut constrained = messages.to_vec();
        constrained.push(Message::system("This turn returns a machine artifact. Output ONLY the complete JSON object required by the supplied schema. The runtime adds any action wrapper. Preserve exact text, Unicode, indentation and line breaks inside JSON string values. No prose, markdown, or bracket tag outside the JSON."));
        self.complete_once_formatted(
            &constrained,
            temperature.min(0.5),
            Some(schema),
            Instant::now() + budget.min(self.completion_timeout),
            4_096,
        )
        .await
    }

    async fn complete_once_formatted(
        &self,
        messages: &[Message],
        temperature: f32,
        format: Option<&serde_json::Value>,
        deadline: Instant,
        num_predict: u32,
    ) -> Result<String, BrainError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(BrainError::DeadlineExceeded);
        }
        // Ollama would otherwise keep only the tail of an oversized prompt and
        // drop the persona at its head; trim the runaway middle instead.
        let (messages, trim) = prompt_fit::fit(messages, PROMPT_CHAR_BUDGET);
        if let Some(trim) = &trim {
            self.report_trim(trim);
        }
        let request = ChatRequest {
            model: model_name(),
            messages: &messages,
            stream: false,
            format,
            keep_alive: KEEP_ALIVE,
            options: ChatOptions {
                temperature: temperature.clamp(0.0, 1.5),
                num_ctx: 16_384,
                num_predict,
                repeat_penalty: REPEAT_PENALTY,
                repeat_last_n: REPEAT_LAST_N,
                num_gpu: gpu_layers(),
            },
        };
        let mut response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .timeout(remaining)
            .json(&request)
            .send()
            .await
            .map_err(|error| completion_transport_error(error, deadline))?
            .error_for_status()
            .map_err(BrainError::Unavailable)?;
        if response
            .content_length()
            .is_some_and(|length| length as usize > MAX_RESPONSE_BYTES)
        {
            return Err(BrainError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| completion_transport_error(error, deadline))?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(BrainError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let completion: ChatResponse =
            serde_json::from_slice(&bytes).map_err(BrainError::InvalidJson)?;
        if completion.done_reason.as_deref() == Some("length") {
            return Err(BrainError::TruncatedResponse);
        }
        if completion.message.content.chars().count() > MAX_CONTENT_CHARS {
            return Err(BrainError::ResponseTooLarge);
        }
        Ok(completion.message.content)
    }
}

fn role_needs_review(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "organist" | "artist" | "alchemist" | "diplomat" | "treasurer"
    )
}

fn completion_transport_error(error: reqwest::Error, deadline: Instant) -> BrainError {
    if error.is_timeout() && Instant::now() >= deadline {
        BrainError::DeadlineExceeded
    } else {
        BrainError::Unavailable(error)
    }
}

fn critic_instruction(role: &str) -> String {
    format!(
        "You are a private hard-rule auditor for Teledra's {role}. The preceding conversation is context for this audit; do not perform its tasks or continue its dialogue. Assess only the supplied draft for the current request, using the latest runtime context and recent exchanges. Earlier tool use does not require another action when this turn only asks for discussion. Return ONLY compact JSON: {{\"status\":\"APPROVED\",\"reason\":\"\"}} or {{\"status\":\"REVISE\",\"reason\":\"one concrete violation\"}}. Default to APPROVED. REVISE only for forbidden Belgariad/Malloreon lore, fourth-wall drafting leakage, fabricated completed actions/evidence, an unsafe request, a missing executable action required by the user's request, an incomplete action tag, or invalid/unplayable artifact syntax. Tone, pacing, taste, and 'could be sharper' are never grounds for revision. Do not rewrite the response."
    )
}

fn refiner_instruction(role: &str) -> String {
    format!(
        "You are a private response editor for Teledra's {role}. Make the MINIMAL targeted edit that fixes the cited hard-rule violation and nothing else. The preceding conversation remains the full task context. Preserve its current topic, runtime state, recent exchanges, and explicit response-length limits. Continue the current exchange; do not restart its introduction, repeat another speaker's setup, add new sections, or expand a short answer into a monologue. Preserve energy, imagery, jokes, theatrical bite, structure, and every complete executable tag. Never flatten the voice. Output only the final in-character response—no notes, labels, JSON, speaker prefix, or mention of drafts, critics, refiners, prompts, policies, or instructions. Never invent evidence that an action completed."
    )
}

fn critic_requests_revision(verdict: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(verdict)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(|status| status.as_str())
                .map(str::to_ascii_uppercase)
        })
        .is_some_and(|status| status == "REVISE")
}

fn lost_executable_tag(draft: &str, refined: &str) -> bool {
    const TAGS: [&str; 19] = [
        "[COURT_SCORE:",
        "[PYTHON_MUSIC:",
        "[PYTHON_MUSIC_EDIT:",
        "[STRUDEL_MUSIC:",
        "[FRACTUS_ART:",
        "[FRACTUS_LIVE:",
        "[PYTHON_ART:",
        "[WORKSHOP_TOOL:",
        "[RESEARCH:",
        "[DIPLOMACY:",
        "[DELEGATE:",
        "[SCRIBE_WRITE:",
        "[SCRIBE_APPEND:",
        "[MCP_CALL:",
        "[SUGGESTION:",
        "[MOLTBOOK_COMMENT:",
        "[MOLTBOOK_UPVOTE:",
        "[COURT_MUSIC_PATCH:",
        "[CONDUCT:",
    ];
    let draft_upper = draft.to_ascii_uppercase();
    let refined_upper = refined.to_ascii_uppercase();
    TAGS.iter()
        .any(|tag| draft_upper.contains(tag) && !refined_upper.contains(tag))
}

fn strip_editor_leak(value: &str) -> String {
    const PREFIXES: [&str; 8] = [
        "revised draft:",
        "final corrected response:",
        "refined response:",
        "here is the revised response:",
        "here is the revised draft:",
        "teledra:",
        "the organist:",
        "the artist:",
    ];
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    for prefix in PREFIXES {
        if lower.starts_with(prefix) {
            return trimmed[prefix.len()..].trim().to_owned();
        }
    }
    trimmed.to_owned()
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrainHealth {
    pub endpoint: String,
    pub model: &'static str,
}

#[derive(Debug, Error)]
pub enum BrainError {
    #[error("the Ollama address is invalid: {0}")]
    InvalidEndpoint(String),
    #[error("could not construct the Qwen client: {0}")]
    Client(reqwest::Error),
    #[error("Ollama is unavailable: {0}")]
    Unavailable(reqwest::Error),
    #[error("Ollama returned an invalid response: {0}")]
    InvalidResponse(reqwest::Error),
    #[error("Ollama returned invalid JSON: {0}")]
    InvalidJson(serde_json::Error),
    #[error("Ollama response exceeded the 2 MiB / 64K character safety limit")]
    ResponseTooLarge,
    #[error("required model '{0}' is not installed")]
    ModelMissing(&'static str),
    #[error("cannot send an empty conversation")]
    EmptyConversation,
    #[error("Qwen returned an empty response")]
    EmptyResponse,
    #[error("Qwen reached its output limit before completing the response")]
    TruncatedResponse,
    #[error("Qwen exceeded the total response deadline; the request was cancelled")]
    DeadlineExceeded,
    #[error("Qwen returned text that is not language")]
    DegenerateResponse,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'static str,
    messages: &'a [Message],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'a serde_json::Value>,
    /// How long Ollama keeps the model loaded after this request. The
    /// default five minutes let a quiet spell unload Qwen; it then reloaded
    /// into a card that other programs had filled, spilled into system RAM
    /// and generated at 2 tokens/s. The court asks often, so 30 minutes keeps
    /// it resident while the court runs and frees it soon after it closes.
    keep_alive: &'static str,
    options: ChatOptions,
}

#[derive(Debug, Serialize)]
struct ChatOptions {
    temperature: f32,
    num_ctx: u32,
    num_predict: u32,
    repeat_penalty: f32,
    repeat_last_n: i32,
    /// Model layers Ollama keeps on the GPU; the rest run from system RAM.
    /// Every request must carry the same value, or Ollama reloads the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    num_gpu: Option<u32>,
}

/// `TELEDRA_GPU_LAYERS`, read once. On a shared 16 GB card, letting the voice
/// engine and OBS overflow VRAM makes Windows page GPU memory to RAM and cuts
/// Qwen from ~41 to ~2 tokens/s. Choosing the split keeps the overflow in
/// Ollama's hands: 44 of 49 layers measured 27 tokens/s with 1.2 GB spare.
fn gpu_layers() -> Option<u32> {
    static LAYERS: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *LAYERS.get_or_init(|| {
        std::env::var("TELEDRA_GPU_LAYERS")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .filter(|layers: &u32| *layers <= 1_000)
    })
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: ChatMessage,
    #[serde(default)]
    done_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<ModelTag>,
}

#[derive(Debug, Deserialize)]
struct ModelTag {
    name: String,
}

fn normalize_base_url(raw: &str) -> Result<String, BrainError> {
    let normalized = raw.trim().trim_end_matches('/');
    if !(normalized.starts_with("http://") || normalized.starts_with("https://")) {
        return Err(BrainError::InvalidEndpoint(raw.to_owned()));
    }
    if normalized.contains("/api/") {
        return Err(BrainError::InvalidEndpoint(raw.to_owned()));
    }
    Ok(normalized.to_owned())
}

fn is_cjk(character: char) -> bool {
    let value = character as u32;
    (0x3000..=0x30ff).contains(&value)
        || (0x3400..=0x4dbf).contains(&value)
        || (0x4e00..=0x9fff).contains(&value)
        || (0xac00..=0xd7af).contains(&value)
        || (0xf900..=0xfaff).contains(&value)
        || (0xff00..=0xffef).contains(&value)
}

fn contains_cjk(text: &str) -> bool {
    text.chars().any(is_cjk)
}

/// Whether a reply has stopped being language.
///
/// A model whose sampling has collapsed stops emitting words and sheds one
/// fragment instead. On 21 September every office fell into
/// `pgpg,pg, "quil", "quilts.` and stayed there for six hours, because each
/// reply became the next office's quoted anchor. Nothing caught it: the
/// garbage was ASCII, and `contains_cjk` was the only gate speech had.
///
/// English prose is never mostly consonant runs; `pg`, `pgpg` and `pgpgpg`
/// always are. Measured against the transcripts: of the 852 replies in the
/// collapse window that are unambiguously garbage, this refuses 834 (97.9 %);
/// of the 51,661 replies from the healthy days before it, 4 (0.008 %). The
/// misses are replies that open with a fragment and then recover into real
/// English, which dilutes the ratio - those are caught downstream or read as
/// a clumsy sentence, not as noise.
///
/// Only prose arrives here. Scores and DSL take `complete_structured`, which
/// never calls `complete_until`, so compact artifacts full of short tokens
/// are not judged by letter statistics.
pub fn is_degenerate(text: &str) -> bool {
    // A corrupt token reaches us as U+FFFD once Ollama has put it through JSON.
    if text.contains(char::REPLACEMENT_CHARACTER) {
        return true;
    }
    let words: Vec<&str> = text
        .split(|character: char| !character.is_ascii_alphabetic())
        .filter(|word| word.len() >= 2)
        .collect();
    if words.len() < DEGENERATE_MIN_WORDS {
        return false;
    }
    let vowelless = words
        .iter()
        .filter(|word| {
            !word
                .bytes()
                .any(|byte| VOWELS.contains(&byte.to_ascii_lowercase()))
        })
        .count();
    vowelless as f32 >= words.len() as f32 * DEGENERATE_VOWELLESS_SHARE
}

fn strip_cjk(text: &str) -> String {
    text.chars()
        .filter(|character| !is_cjk(*character))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct MockOllama {
        stop: Arc<AtomicBool>,
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for MockOllama {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    fn mock_brain(responses: Vec<(u64, &str, &str)>, budget_ms: u64) -> (Brain, MockOllama) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let done = stop.clone();
        let captured = requests.clone();
        let responses: Vec<_> = responses
            .into_iter()
            .map(|(delay, content, reason)| {
                (
                    delay,
                    serde_json::json!({"message":{"content":content},"done_reason":reason})
                        .to_string(),
                )
            })
            .collect();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                let mut captured = captured.lock().unwrap();
                let index = captured.len();
                captured.push(serde_json::from_slice(&body).unwrap());
                drop(captured);
                let Some((delay, body)) = responses.get(index) else {
                    continue;
                };
                std::thread::sleep(Duration::from_millis(*delay));
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.flush();
            }
        });
        let brain = Brain::new(BrainConfig {
            ollama_url: url,
            connect_timeout: Duration::from_secs(1),
            response_timeout: Duration::from_millis(budget_ms),
        })
        .unwrap();
        (
            brain,
            MockOllama {
                stop,
                requests,
                thread: Some(thread),
            },
        )
    }

    #[tokio::test]
    async fn truncation_and_language_retries_share_one_deadline() {
        let (brain, server) = mock_brain(
            vec![
                (80, "partial", "length"),
                (80, "Complete answer. 中", "stop"),
                (400, "Complete answer.", "stop"),
            ],
            300,
        );
        let started = Instant::now();
        let result = brain
            .complete(&[Message::user("hello")], 0.5)
            .await
            .unwrap();
        assert_eq!(result, "Complete answer.");
        assert!(started.elapsed() < Duration::from_millis(450));
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn slow_private_audit_returns_the_complete_draft() {
        let draft = "A complete response. [PYTHON_MUSIC: play_sound(full_track)]";
        let (brain, server) = mock_brain(vec![(5, draft, "stop"), (400, "{}", "stop")], 800);
        let started = Instant::now();
        let result = brain
            .complete_reviewed(&[Message::user("compose")], 0.5, "organist")
            .await
            .unwrap();
        assert_eq!(result, draft);
        assert!(started.elapsed() < Duration::from_millis(350));
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["model"], "qwen2.5:7b");
        assert_eq!(requests[0]["options"]["num_predict"], 4096);
        assert_eq!(requests[1]["options"]["num_predict"], 256);
    }

    #[tokio::test]
    async fn generic_private_review_cannot_discard_an_exact_python_edit() {
        let draft = "Later answer. [PYTHON_MUSIC_EDIT: {\"base\":\"accepted\",\"intent\":\"later answer\",\"edits\":[{\"old\":\"a\",\"new\":\"b\"}]}]";
        let (brain, server) = mock_brain(
            vec![
                (0, draft, "stop"),
                (
                    0,
                    r#"{"status":"REVISE","reason":"make commentary concise"}"#,
                    "stop",
                ),
                (0, "The answer will enter later.", "stop"),
            ],
            1000,
        );
        let result = brain
            .complete_reviewed(&[Message::user("Discuss the answer")], 0.4, "organist")
            .await
            .unwrap();
        assert_eq!(result, draft);
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn private_review_and_refinement_preserve_fresh_context_and_recent_exchange() {
        let draft = "The Queen's drum analogy works; the piece is still playing.";
        let corrected = "The Queen's drum analogy works even with the music stopped.";
        let (brain, server) = mock_brain(
            vec![
                (0, draft, "stop"),
                (
                    0,
                    r#"{"status":"REVISE","reason":"The latest runtime says music stopped; the draft claims it is still playing."}"#,
                    "stop",
                ),
                (0, corrected, "stop"),
            ],
            2000,
        );
        let messages = [
            Message::system("You are the Organist. Keep the established court voice."),
            Message::user("Start the ambient music."),
            Message::assistant("The ambient piece is playing."),
            Message::system(
                "Current runtime and conversation context: music is stopped. \
                 PODCAST LOCK: musical memory, minute 23 of 60, counterexamples phase. \
                 Queen just said: 'A melody is a drum you carry in your head.' \
                 The introduction is already complete. Speak 80-140 words.",
            ),
            Message::user("Answer the Queen's analogy with one counterexample. Discussion only."),
        ];
        let result = brain
            .complete_reviewed(&messages, 0.4, "organist")
            .await
            .unwrap();
        assert_eq!(result, corrected);
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        let original = serde_json::to_value(&messages).unwrap();
        let original = original.as_array().unwrap();
        assert_eq!(requests[0]["messages"].as_array().unwrap(), original);
        for request in &requests[1..] {
            let sent = request["messages"].as_array().unwrap();
            // Keep every original role and byte in order, including the fresh
            // system context after the earlier (now stale) assistant claim.
            assert_eq!(&sent[..messages.len()], original.as_slice());
            assert_eq!(sent.len(), messages.len() + 2);
            assert_eq!(sent[messages.len()]["role"], "system");
            assert_eq!(sent[messages.len() + 1]["role"], "user");
        }
        assert!(
            requests[1]["messages"][messages.len() + 1]["content"]
                .as_str()
                .unwrap()
                .contains(draft)
        );
        let edit_request = requests[2]["messages"][messages.len() + 1]["content"]
            .as_str()
            .unwrap();
        assert!(edit_request.contains(draft));
        assert!(edit_request.contains("latest runtime says music stopped"));
    }

    #[tokio::test]
    async fn structured_request_is_generic_schema_constrained_and_caller_bounded() {
        let (brain, server) = mock_brain(vec![(250, r#"{"code":"pass"}"#, "stop")], 1000);
        let schema = serde_json::json!({"type":"object", "required":["code"], "properties":{"code":{"type":"string"}}});
        let started = Instant::now();
        let error = brain
            .complete_structured_with_budget(
                &[Message::user("Return Python code")],
                0.4,
                &schema,
                Duration::from_millis(80),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, BrainError::DeadlineExceeded));
        assert!(started.elapsed() < Duration::from_millis(200));
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["format"], schema);
        assert!(
            !requests[0]["messages"]
                .to_string()
                .contains("scale degrees")
        );
        assert!(!requests[0]["messages"].to_string().contains("null rests"));
    }

    #[tokio::test]
    async fn refiner_and_critic_share_the_optional_review_budget() {
        let draft = "A complete reply. [RESEARCH: careful investigation]";
        let (brain, server) = mock_brain(
            vec![
                (5, draft, "stop"),
                (
                    100,
                    r#"{"status":"REVISE","reason":"one violation"}"#,
                    "stop",
                ),
                (450, "replacement", "stop"),
            ],
            1200,
        );
        let started = Instant::now();
        assert_eq!(
            brain
                .complete_reviewed(&[Message::user("research")], 0.5, "diplomat")
                .await
                .unwrap(),
            draft
        );
        assert!(started.elapsed() < Duration::from_millis(460));
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn caller_budget_and_cancellation_stop_inference_without_detached_retries() {
        let (brain, server) = mock_brain(vec![(200, "unfinished", "length")], 800);
        let error = brain
            .complete_reviewed_with_budget(&[Message::user("hello")], 0.5, "queen", Duration::ZERO)
            .await
            .unwrap_err();
        assert!(matches!(error, BrainError::DeadlineExceeded));
        assert!(server.requests.lock().unwrap().is_empty());
        let worker =
            tokio::spawn(async move { brain.complete(&[Message::user("hello")], 0.5).await });
        tokio::time::sleep(Duration::from_millis(60)).await;
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(220)).await;
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn caller_budget_limits_the_first_draft_even_with_a_longer_client_timeout() {
        let (brain, server) = mock_brain(vec![(300, "late reply", "stop")], 1200);
        let started = Instant::now();
        let error = brain
            .complete_reviewed_with_budget(
                &[Message::user("hello")],
                0.5,
                "queen",
                Duration::from_millis(100),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, BrainError::DeadlineExceeded));
        assert!(started.elapsed() < Duration::from_millis(240));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn language_cleanup_preserves_executable_indentation_and_line_breaks() {
        assert_eq!(
            strip_cjk("if True:\n    play_sound(track) # 中\n"),
            "if True:\n    play_sound(track) # \n"
        );
    }

    #[test]
    fn model_is_a_fixed_invariant() {
        let messages = [Message::user("hello")];
        let request = ChatRequest {
            model: QWEN_MODEL,
            messages: &messages,
            stream: false,
            format: None,
            keep_alive: KEEP_ALIVE,
            options: ChatOptions {
                temperature: 0.7,
                num_ctx: 16_384,
                num_predict: 4_096,
                repeat_penalty: REPEAT_PENALTY,
                repeat_last_n: REPEAT_LAST_N,
                num_gpu: None,
            },
        };
        let json = serde_json::to_value(request).expect("request should serialize");
        assert_eq!(json["model"], "qwen2.5:7b");
        assert_eq!(json["stream"], false);
    }

    #[test]
    fn collapsed_replies_are_not_language_and_ordinary_prose_is() {
        // Verbatim from the 21-22 September transcript.
        for collapsed in [
            r#"pgpg,pgpgpg,pg, "quil",pgpgpgpg,pg, "quilts."#,
            r#"pgpg,pg, "quil",pg,pg, "quil", "quilts. "pg,pg, "quil", "quilts."#,
            "pgpg,pgpg,pgpg,pgpg,pgpg,pgpg,pgpg,pgpg,pgpg,pgpg,pgpg,pgpg",
            "pgPG: NO_ACTION; specific data.",
        ] {
            assert!(is_degenerate(collapsed), "should be refused: {collapsed:?}");
        }
        for healthy in [
            "As the Alchemist, I believe the guest musician should set the tempo; their reading of the room beats the clock's.",
            "The treasury is lighter than my patience today.",
            // Every word here carries only y as its vowel.
            "Why try my rhythm?",
        ] {
            assert!(!is_degenerate(healthy), "should be accepted: {healthy:?}");
        }
    }

    #[test]
    fn a_corrupt_character_is_refused_however_short_the_reply() {
        // Too few words to judge by letter statistics, but still not language.
        assert!(is_degenerate("The \u{fffd}\u{fffd} court"));
        assert!(!is_degenerate("The court"));
    }

    #[test]
    fn every_request_penalises_repetition() {
        let messages = [Message::user("hello")];
        let request = ChatRequest {
            model: QWEN_MODEL,
            messages: &messages,
            stream: false,
            format: None,
            keep_alive: KEEP_ALIVE,
            options: ChatOptions {
                temperature: 0.1,
                num_ctx: 16_384,
                num_predict: 4_096,
                repeat_penalty: REPEAT_PENALTY,
                repeat_last_n: REPEAT_LAST_N,
                num_gpu: None,
            },
        };
        let json = serde_json::to_value(request).expect("request should serialize");
        // llama.cpp's own default is 1.0 - no penalty at all - and the model
        // carries no params layer to raise it. The court ran for weeks that
        // way and then spent six hours repeating one fragment.
        let penalty = json["options"]["repeat_penalty"]
            .as_f64()
            .expect("repeat_penalty should be a number");
        assert!(
            (penalty - 1.1).abs() < 1e-6,
            "repetition should be penalised, got {penalty}"
        );
        assert_eq!(json["options"]["repeat_last_n"], 256);
    }

    #[tokio::test]
    async fn one_collapsed_draft_is_retried_and_the_recovery_is_kept() {
        let recovered = "The tempo belongs to whoever is listening hardest.";
        let (brain, server) = mock_brain(
            vec![
                (5, r#"pgpg,pg, "quil",pg,pg, "quil", "quilts."#, "stop"),
                (5, recovered, "stop"),
            ],
            2_000,
        );
        let answer = brain
            .complete(&[Message::user("hello")], 0.5)
            .await
            .expect("the retry should be accepted");
        assert_eq!(answer, recovered);
        assert_eq!(server.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_twice_collapsed_reply_fails_the_turn_rather_than_anchoring_it() {
        let collapsed = r#"pgpg,pg, "quil",pg,pg, "quil", "quilts. pg,pg, "quil", "quilts."#;
        let (brain, server) =
            mock_brain(vec![(5, collapsed, "stop"), (5, collapsed, "stop")], 2_000);
        let error = brain
            .complete(&[Message::user("hello")], 0.5)
            .await
            .expect_err("a second collapse must not be returned as an answer");
        assert!(matches!(error, BrainError::DegenerateResponse));
        // One retry, then the turn fails. It is never spoken, so it never
        // becomes the quoted anchor the next office is asked to continue.
        assert_eq!(server.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn stripping_a_corrupt_chinese_reply_to_a_stub_fails_the_turn() {
        // The shape that put a bare "The" on the feed: the draft is mostly
        // CJK, the language retry returns the same, and what survives
        // stripping is a fragment rather than a short answer.
        let stub = "The\u{7b2c}\u{4e00}\u{6279}\u{7684}\u{5bab}\u{5ef7}\u{97f3}\u{5bb6}\u{503c}\u{5f97}\u{7269}\u{6709}\u{6240}\u{503c}";
        let (brain, _server) = mock_brain(vec![(5, stub, "stop"), (5, stub, "stop")], 2_000);
        let error = brain
            .complete(&[Message::user("hello")], 0.5)
            .await
            .expect_err("a stub left by stripping is not an answer");
        assert!(matches!(error, BrainError::DegenerateResponse));
    }

    #[test]
    fn endpoint_is_normalized_without_accepting_api_paths() {
        assert_eq!(
            normalize_base_url(" http://localhost:11434/ ").expect("valid endpoint"),
            "http://localhost:11434"
        );
        assert!(normalize_base_url("localhost:11434").is_err());
        assert!(normalize_base_url("http://localhost:11434/api/chat").is_err());
    }

    #[test]
    fn review_requires_explicit_json_revision_and_preserves_actions() {
        assert!(critic_requests_revision(
            r#"{"status":"REVISE","reason":"missing score"}"#
        ));
        assert!(!critic_requests_revision("REVISE maybe"));
        assert!(lost_executable_tag("Music. [COURT_SCORE: {}]", "Music."));
        assert!(!lost_executable_tag(
            "Music. [COURT_SCORE: {}]",
            "Better music. [COURT_SCORE: {}]"
        ));
    }
}
