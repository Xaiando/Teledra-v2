//! Keeps every request inside the window the model actually has.
//!
//! Each request gets a 16,384-token window (Ollama 0.34 serves one request at
//! a time; earlier it split the window across two, leaving ~8,192). A longer
//! prompt is not refused: Ollama silently keeps its first four tokens and the
//! LAST part, discarding everything in between — which is where the persona
//! and rules live. On 19 September 115 requests arrived at 51,000–101,000 tokens; the
//! model answered from a tail of context with no idea who it was, and each of
//! those requests spent a full-context pass on the GPU.
//!
//! This trims the largest middle messages instead, keeping the first system
//! message (identity and rules) and the final message (the actual request)
//! whole, and reports what was cut so the source can be fixed.

use std::borrow::Cow;

use crate::{Message, Role};

/// About 10,000 tokens of English/JSON at ~3.6 characters a token, leaving
/// room in a 16,384-token window for the longest (4,096-token) answer.
pub const PROMPT_CHAR_BUDGET: usize = 36_000;
/// A trimmed message keeps at least this much of its head and tail.
const MIN_KEPT_CHARS: usize = 1_200;
/// The omission marker's own length may leave a prompt this far over budget.
const MARKER_SLACK: usize = 200;

/// What was trimmed from one oversized prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTrim {
    pub original_chars: usize,
    pub kept_chars: usize,
    /// (message index, role, original chars, first words) of each trimmed message.
    pub trimmed: Vec<(usize, &'static str, usize, String)>,
}

fn role_name(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn total(messages: &[Message]) -> usize {
    messages.iter().map(|m| m.content.chars().count()).sum()
}

/// Shortens `text` to `keep` characters: two thirds from the start, one third
/// from the end, with a visible marker where the middle was removed.
fn shorten(text: &str, keep: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= keep {
        return text.to_owned();
    }
    let head = keep * 2 / 3;
    let tail = keep - head;
    let omitted = chars.len() - keep;
    let mut out: String = chars[..head].iter().collect();
    out.push_str(&format!(
        "\n[… {omitted} characters omitted to fit the model's context …]\n"
    ));
    out.extend(&chars[chars.len() - tail..]);
    out
}

/// The messages to send, within `budget` characters when that is possible
/// without touching the first system message or the final message.
pub fn fit(messages: &[Message], budget: usize) -> (Cow<'_, [Message]>, Option<PromptTrim>) {
    let original = total(messages);
    if original <= budget || messages.len() < 3 {
        return (Cow::Borrowed(messages), None);
    }
    let mut fitted = messages.to_vec();
    let mut trimmed: Vec<(usize, &'static str, usize, String)> = Vec::new();
    let last = fitted.len() - 1;
    // Largest first: one runaway context usually accounts for nearly all of
    // it. Each middle message is shortened at most once, so this ends.
    loop {
        let excess = total(&fitted).saturating_sub(budget);
        if excess <= MARKER_SLACK {
            break;
        }
        let candidate = (1..last)
            .filter(|&i| !trimmed.iter().any(|(t, ..)| *t == i))
            .filter(|&i| fitted[i].content.chars().count() > MIN_KEPT_CHARS)
            .max_by_key(|&i| fitted[i].content.chars().count());
        let Some(index) = candidate else { break };
        let size = fitted[index].content.chars().count();
        let keep = size.saturating_sub(excess).max(MIN_KEPT_CHARS);
        let first: String = fitted[index]
            .content
            .split_whitespace()
            .take(12)
            .collect::<Vec<_>>()
            .join(" ");
        trimmed.push((index, role_name(&fitted[index].role), size, first));
        fitted[index].content = shorten(&fitted[index].content, keep);
    }
    // Still too long: the request itself is huge. Keep its end, which is
    // what Ollama would have kept anyway, but leave the persona in front.
    let excess = total(&fitted).saturating_sub(budget);
    if excess > MARKER_SLACK {
        let size = fitted[last].content.chars().count();
        let keep = size.saturating_sub(excess).max(budget / 3);
        if keep < size {
            let first: String = fitted[last]
                .content
                .split_whitespace()
                .take(12)
                .collect::<Vec<_>>()
                .join(" ");
            trimmed.push((last, role_name(&fitted[last].role), size, first));
            fitted[last].content = shorten(&fitted[last].content, keep);
        }
    }
    let kept_chars = total(&fitted);
    (
        Cow::Owned(fitted),
        Some(PromptTrim {
            original_chars: original,
            kept_chars,
            trimmed,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(n: usize) -> String {
        "word ".repeat(n / 5)
    }

    #[test]
    fn prompts_within_budget_are_untouched() {
        let messages = vec![Message::system("persona"), Message::user("hello")];
        let (fitted, trim) = fit(&messages, 100);
        assert!(matches!(fitted, Cow::Borrowed(_)));
        assert!(trim.is_none());
    }

    #[test]
    fn the_persona_and_request_survive_while_the_runaway_context_shrinks() {
        let persona = "You are Teledra, sovereign of the court.".to_owned();
        let request = "What is playing now?".to_owned();
        let messages = vec![
            Message::system(persona.clone()),
            Message::system(big(300_000)),
            Message::assistant("an earlier reply"),
            Message::user(request.clone()),
        ];
        let (fitted, trim) = fit(&messages, PROMPT_CHAR_BUDGET);
        assert!(total(&fitted) <= PROMPT_CHAR_BUDGET + MARKER_SLACK);
        assert_eq!(fitted[0].content, persona);
        assert_eq!(fitted[3].content, request);
        assert_eq!(fitted[2].content, "an earlier reply");
        assert!(fitted[1].content.contains("characters omitted"));
        let trim = trim.unwrap();
        assert_eq!(trim.trimmed.len(), 1);
        assert_eq!(trim.trimmed[0].0, 1);
        assert_eq!(trim.trimmed[0].1, "system");
    }

    #[test]
    fn many_mid_sized_messages_are_each_trimmed_once_and_the_loop_ends() {
        let mut messages = vec![Message::system("persona")];
        messages.extend((0..40).map(|_| Message::system(big(2_000))));
        messages.push(Message::user("the question"));
        let (fitted, trim) = fit(&messages, 20_000);
        assert_eq!(fitted[0].content, "persona");
        assert_eq!(fitted.last().unwrap().content, "the question");
        let trim = trim.unwrap();
        let mut indices: Vec<_> = trim.trimmed.iter().map(|(i, ..)| *i).collect();
        indices.dedup();
        assert_eq!(indices.len(), trim.trimmed.len());
        assert!(trim.kept_chars < trim.original_chars);
    }

    #[test]
    fn a_huge_request_keeps_its_end_and_the_persona_in_front() {
        let messages = vec![
            Message::system("persona"),
            Message::assistant("short"),
            Message::user(format!("{}THE QUESTION", big(90_000))),
        ];
        let (fitted, trim) = fit(&messages, 10_000);
        assert_eq!(fitted[0].content, "persona");
        assert!(fitted[2].content.ends_with("THE QUESTION"));
        assert!(total(&fitted) <= 10_000 + 200);
        assert!(trim.is_some());
    }
}
