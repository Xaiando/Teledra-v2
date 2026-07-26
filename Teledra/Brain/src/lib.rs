use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const QWEN_MODEL: &str = "qwen2.5:7b";
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

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
        let request = ChatRequest {
            model: QWEN_MODEL,
            messages,
            stream: false,
            options: ChatOptions {
                temperature: temperature.clamp(0.0, 1.5),
            },
        };
        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await
            .map_err(BrainError::Unavailable)?
            .error_for_status()
            .map_err(BrainError::Unavailable)?;
        let completion: ChatResponse =
            response.json().await.map_err(BrainError::InvalidResponse)?;
        let content = completion.message.content.trim().to_owned();
        if content.is_empty() {
            return Err(BrainError::EmptyResponse);
        }
        Ok(content)
    }
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
}
