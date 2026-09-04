use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use super::SkillError;

const MODEL: &str = "qwen2.5:7b";
const MAX_PROMPT_CHARS: usize = 96 * 1024;
const MAX_RESPONSE_CHARS: usize = 64 * 1024;

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<Message<'a>>,
    options: Options,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct Options {
    temperature: f32,
    num_ctx: u32,
    num_predict: u32,
}

#[derive(Deserialize)]
struct Response {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

pub fn complete(system: &str, prompt: &str, temperature: f32) -> Result<String, SkillError> {
    let endpoint = std::env::var("TELEDRA_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned())
        .trim_end_matches('/')
        .to_owned();
    let system: String = system.chars().take(16 * 1024).collect();
    let prompt: String = prompt.chars().take(MAX_PROMPT_CHARS).collect();
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(180))
        .build()?;
    let response = client
        .post(format!("{endpoint}/api/chat"))
        .json(&Request {
            model: MODEL,
            stream: false,
            messages: vec![
                Message {
                    role: "system",
                    content: &system,
                },
                Message {
                    role: "user",
                    content: &prompt,
                },
            ],
            options: Options {
                temperature: temperature.clamp(0.0, 1.2),
                num_ctx: 32_768,
                num_predict: 8_192,
            },
        })
        .send()?
        .error_for_status()?
        .json::<Response>()?;
    let text: String = response
        .message
        .content
        .trim()
        .chars()
        .take(MAX_RESPONSE_CHARS)
        .collect();
    if text.is_empty() {
        Err(SkillError::Model(
            "Ollama returned an empty response".to_owned(),
        ))
    } else {
        Ok(text)
    }
}

pub fn available() -> bool {
    let endpoint = std::env::var("TELEDRA_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned())
        .trim_end_matches('/')
        .to_owned();
    Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .and_then(|client| client.get(format!("{endpoint}/api/tags")).send())
        .is_ok_and(|response| response.status().is_success())
}
