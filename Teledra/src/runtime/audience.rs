//! Cleaning live chat before the court sees it.
//!
//! Chat reaches the court by more than one road. Restream relays Discord as
//! "RestreamBot: [Discord: name] text", and Social Stream Ninja relays
//! Restream's chat too, so one viewer line arrived two or three times and
//! Teledra answered each copy. A viewer account named "Teledra" also showed in
//! the transcript exactly like the Queen herself.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Copies of one line arriving through different relays land within seconds.
const REPEAT_WINDOW: Duration = Duration::from_secs(45);

/// Prefix on every viewer line in the public transcript, so no viewer name,
/// even one that matches a court member, reads as the court speaking.
pub(crate) const CHAT_PREFIX: &str = "Chat · ";

/// The real author and text of a relayed line: `[Discord: name] text` from a
/// relay bot becomes `name (discord)` saying `text`.
pub(super) fn unwrap_relay(author: &str, text: &str) -> (String, String) {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[')
        && let Some((label, body)) = rest.split_once(']')
        && let Some((platform, name)) = label.split_once(':')
    {
        let platform = platform.trim();
        let name = name.trim();
        let body = body.trim();
        if !platform.is_empty()
            && platform.len() <= 20
            && platform.chars().all(|c| c.is_ascii_alphabetic())
            && !name.is_empty()
            && name.chars().count() <= 60
            && !body.is_empty()
        {
            return (
                format!("{name} ({})", platform.to_ascii_lowercase()),
                body.to_owned(),
            );
        }
    }
    (author.to_owned(), text.trim().to_owned())
}

/// Lowercase letters and digits only: relays differ in spacing, emoji and
/// punctuation but not in the words.
fn fingerprint(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Default)]
pub(super) struct RecentChat {
    seen: VecDeque<(String, Instant)>,
}

impl RecentChat {
    /// True when this text already arrived within the window, by any road.
    pub(super) fn is_repeat(&mut self, text: &str) -> bool {
        self.is_repeat_at(text, Instant::now())
    }

    fn is_repeat_at(&mut self, text: &str, now: Instant) -> bool {
        while self
            .seen
            .front()
            .is_some_and(|(_, at)| now.duration_since(*at) > REPEAT_WINDOW)
        {
            self.seen.pop_front();
        }
        let key = fingerprint(text);
        if key.is_empty() {
            return false;
        }
        if self.seen.iter().any(|(seen, _)| *seen == key) {
            return true;
        }
        self.seen.push_back((key, now));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relayed_discord_lines_belong_to_their_real_author() {
        assert_eq!(
            unwrap_relay(
                "RestreamBot (twitch)",
                "[Discord: teledra_36694] Hello court"
            ),
            ("teledra_36694 (discord)".into(), "Hello court".into())
        );
        assert_eq!(
            unwrap_relay("Mira (twitch)", "[not a relay] hi"),
            ("Mira (twitch)".into(), "[not a relay] hi".into())
        );
        assert_eq!(
            unwrap_relay("Mira", "plain words"),
            ("Mira".into(), "plain words".into())
        );
    }

    #[test]
    fn one_line_through_two_relays_is_answered_once() {
        let mut recent = RecentChat::default();
        let start = Instant::now();
        assert!(!recent.is_repeat_at("Hello court!", start));
        assert!(recent.is_repeat_at("hello   court", start + Duration::from_secs(5)));
        assert!(!recent.is_repeat_at("Hello court, again", start + Duration::from_secs(6)));
        // Long after, the same words are a new message.
        assert!(!recent.is_repeat_at("Hello court!", start + Duration::from_secs(120)));
    }
}
