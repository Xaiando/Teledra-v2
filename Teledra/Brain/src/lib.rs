use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const QWEN_MODEL: &str = "qwen2.5:7b";
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CONTENT_CHARS: usize = 64 * 1024;

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
}

impl Brain {
    pub fn new(config: BrainConfig) -> Result<Self, BrainError> {
        let base_url = normalize_base_url(&config.ollama_url)?;
        let client = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.response_timeout)
            .build()
            .map_err(BrainError::Client)?;
        Ok(Self { client, base_url })
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
        let installed = tags.models.iter().any(|model| model.name == QWEN_MODEL);
        if !installed {
            return Err(BrainError::ModelMissing(QWEN_MODEL));
        }
        Ok(BrainHealth {
            endpoint: self.base_url.clone(),
            model: QWEN_MODEL,
        })
    }

    pub async fn complete(
        &self,
        messages: &[Message],
        temperature: f32,
    ) -> Result<String, BrainError> {
        if messages.is_empty() {
            return Err(BrainError::EmptyConversation);
        }
        let mut content = self.complete_once(messages, temperature).await?;
        if contains_cjk(&content) {
            let mut retry = messages.to_vec();
            retry.push(Message::system(
                "Regenerate the answer entirely in English. Use no Chinese, Japanese, Korean, or full-width characters.",
            ));
            content = self.complete_once(&retry, temperature).await?;
            if contains_cjk(&content) {
                content = strip_cjk(&content);
            }
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
        let draft = self.complete(messages, temperature).await?;
        if !role_needs_review(role) {
            return Ok(strip_editor_leak(&draft));
        }
        let user = messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(|message| message.content.as_str())
            .unwrap_or_default();
        let critic = [
            Message::system(critic_instruction(role)),
            Message::user(format!(
                "USER REQUEST:\n{}\n\nDRAFT RESPONSE:\n{}",
                truncate_chars(user, 12_000),
                truncate_chars(&draft, 48_000)
            )),
        ];
        let verdict = match self.complete_once(&critic, 0.1).await {
            Ok(verdict) => verdict,
            Err(_) => return Ok(strip_editor_leak(&draft)),
        };
        if !critic_requests_revision(&verdict) {
            return Ok(strip_editor_leak(&draft));
        }
        let refiner = [
            Message::system(refiner_instruction(role)),
            Message::user(format!(
                "ORIGINAL DRAFT:\n{}\n\nPRIVATE CRITIQUE:\n{}",
                truncate_chars(&draft, 48_000),
                truncate_chars(&verdict, 4_000)
            )),
        ];
        let refined = match self.complete_once(&refiner, 0.35).await {
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
    ) -> Result<String, BrainError> {
        let request = ChatRequest {
            model: QWEN_MODEL,
            messages,
            stream: false,
            options: ChatOptions {
                temperature: temperature.clamp(0.0, 1.5),
            },
        };
        let mut response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await
            .map_err(BrainError::Unavailable)?
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
            .map_err(BrainError::InvalidResponse)?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(BrainError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let completion: ChatResponse =
            serde_json::from_slice(&bytes).map_err(BrainError::InvalidJson)?;
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

fn critic_instruction(role: &str) -> String {
    format!(
        "You are a private hard-rule auditor for Teledra's {role}. Return ONLY compact JSON: {{\"status\":\"APPROVED\",\"reason\":\"\"}} or {{\"status\":\"REVISE\",\"reason\":\"one concrete violation\"}}. Default to APPROVED. REVISE only for forbidden Belgariad/Malloreon lore, fourth-wall drafting leakage, fabricated completed actions/evidence, an unsafe request, a missing executable action required by the user's request, an incomplete action tag, or invalid/unplayable artifact syntax. Tone, pacing, taste, and 'could be sharper' are never grounds for revision. Do not rewrite the response."
    )
}

fn refiner_instruction(role: &str) -> String {
    format!(
        "You are a private response editor for Teledra's {role}. Make the MINIMAL targeted edit that fixes the cited hard-rule violation and nothing else. Preserve energy, imagery, jokes, theatrical bite, structure, and every complete executable tag. Never flatten the voice. Output only the final in-character response—no notes, labels, JSON, speaker prefix, or mention of drafts, critics, refiners, prompts, policies, or instructions. Never invent evidence that an action completed."
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
    const TAGS: [&str; 17] = [
        "[COURT_SCORE:",
        "[PYTHON_MUSIC:",
        "[STRUDEL_MUSIC:",
        "[FRACTUS_ART:",
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
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'static str,
    messages: &'a [Message],
    stream: bool,
    options: ChatOptions,
}

#[derive(Debug, Serialize)]
struct ChatOptions {
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: ChatMessage,
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

fn strip_cjk(text: &str) -> String {
    text.chars()
        .filter(|character| !is_cjk(*character))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_is_a_fixed_invariant() {
        let messages = [Message::user("hello")];
        let request = ChatRequest {
            model: QWEN_MODEL,
            messages: &messages,
            stream: false,
            options: ChatOptions { temperature: 0.7 },
        };
        let json = serde_json::to_value(request).expect("request should serialize");
        assert_eq!(json["model"], "qwen2.5:7b");
        assert_eq!(json["stream"], false);
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
