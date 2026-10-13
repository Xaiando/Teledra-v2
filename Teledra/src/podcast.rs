//! A topic lock and a paced outline that outlive the model's short conversation window.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;
use teledra_brain::{Brain, BrainError, Message};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

const CHAPTERS: [&str; 12] = [
    "the central question and why it matters",
    "key terms and how the underlying mechanism works",
    "history, turning points, and what changed",
    "a concrete case examined in detail",
    "the strongest competing explanation or opposing view",
    "the evidence, its limitations, and genuine uncertainty",
    "people, incentives, and different lived perspectives",
    "costs, benefits, and difficult tradeoffs",
    "edge cases and where the usual explanation breaks down",
    "indirect effects and surprising connections",
    "plausible futures and what could change the outcome",
    "unresolved questions and deeper connections worth pursuing",
];

const EXTENSIONS: [&str; 8] = [
    "an unresolved thread from earlier, developed with a fresh example",
    "a different scale or setting that tests an earlier claim",
    "an overlooked assumption and what follows if it is wrong",
    "a comparison that reveals an important difference",
    "the perspective of someone affected but not yet discussed",
    "a hard boundary case for a position already explored",
    "a second-order consequence of an earlier example",
    "a new question emerging from the discussion so far",
];

const MOVES: [&str; 13] = [
    "Explain one concrete mechanism step by step, then draw out its implication.",
    "Develop a specific example; label it hypothetical if you cannot verify it.",
    "Take the strongest opposing view seriously and respond to its best argument.",
    "Identify an assumption beneath an earlier claim and test what depends on it.",
    "Compare two plausible interpretations and say what would distinguish them.",
    "Follow one consequence a step further than the discussion has gone so far.",
    "Make an abstract point tangible through a clearly marked thought experiment.",
    "Explore a meaningful exception and explain what it changes about the rule.",
    "Bring in a relevant perspective that has received too little attention.",
    "Ask a pointed question, then begin answering it yourself with substance.",
    "Connect two earlier ideas and explain the connection without recapping them.",
    "Clarify a real uncertainty and explain what evidence could resolve it.",
    "Offer a precise, defensible disagreement or refinement of an earlier point.",
];

const RECENT_NOTES: usize = 3;
const ANCHOR_CHARS: usize = 120;
const RECENT_CHARS: usize = 220;
const SPEAKER_CHARS: usize = 24;

/// Generate speech directly, without artifact actions or the general prose reviewer.
/// A repair shares the original deadline and can happen at most once.
pub(crate) async fn complete_turn(
    brain: &Brain,
    messages: &[Message],
    temperature: f32,
    budget: Duration,
    recent: &[String],
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + budget.min(brain.completion_timeout());
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["speech"],
        "properties": {
            "speech": {
                "type": "string",
                "minLength": 180,
                "maxLength": 2_000,
                "description": "One new spoken podcast contribution of 80–140 words. Plain prose only; no action tags, stage directions, recap, or repeated introduction."
            }
        }
    });
    let mut constrained = messages.to_vec();
    constrained.push(Message::system(
        "This is a spoken podcast turn. Return ONLY a JSON object with one field, speech. \
         Inside speech, give 80–140 words of natural spoken prose developing one new \
         substantive point about the locked topic and current discussion. The hard \
         acceptance range is 50–180 words. Do not repeat or paraphrase the previous \
         contribution as an introduction. Do not copy earlier sentences, restart the \
         episode, introduce yourself again, or announce what you are about to discuss. \
         Use a fresh concrete detail or argument and finish its thought within this turn. \
         No headings, speaker labels, lists, code, stage directions, internal metadata, \
         or action tags. Preserve uncertainty and the limits of the supplied evidence. \
         Only the speech string will be spoken; the JSON envelope is never spoken.",
    ));
    for attempt in 0..2 {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("Podcast speech exceeded its total generation deadline.".to_owned());
        }
        let result = tokio::time::timeout_at(
            deadline,
            brain.complete_structured_with_budget(
                &constrained,
                if attempt == 0 {
                    temperature
                } else {
                    temperature.min(0.35)
                },
                &schema,
                remaining,
            ),
        )
        .await
        .map_err(|_| "Podcast speech exceeded its total generation deadline.".to_owned())?;
        let failure = match result {
            Ok(raw) => match validate_speech(&raw, recent) {
                Ok(speech) => return Ok(speech),
                // Replaying a rejected paragraph as an assistant example can
                // teach the model to copy it again. Preserve the original
                // evidence and give only the precise validation failure.
                Err(error) => error,
            },
            Err(BrainError::TruncatedResponse) => {
                "The output was incomplete. Return a shorter, complete JSON object.".to_owned()
            }
            Err(error) => return Err(format!("Podcast speech generation failed: {error}")),
        };
        if attempt == 1 {
            return Err(format!(
                "Podcast speech failed validation after one corrective retry: {failure}"
            ));
        }
        constrained.push(Message::user(format!(
            "ONE PODCAST CORRECTION: {failure}\n\
             Keep the exact locked topic and current discussion, but add one NEW \
             substantive point rather than reusing the previous opening, sentences, or \
             paragraph. Return only {{\"speech\":\"...\"}} with 80–140 words of complete \
             plain spoken prose. Preserve factual uncertainty; do not invent evidence. \
             No preamble, recap, headings, speaker labels, or action tags. This is the \
             final attempt for this turn."
        )));
    }
    unreachable!("the final attempt always returns")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpeechEnvelope {
    speech: String,
}

fn validate_speech(raw: &str, recent: &[String]) -> Result<String, String> {
    let envelope: SpeechEnvelope = serde_json::from_str(raw)
        .map_err(|_| "Return one valid JSON object containing only a speech string.".to_owned())?;
    let speech = envelope.speech.trim();
    if speech.chars().count() > 2_000 {
        return Err("The speech is too long; develop only one point in 80–140 words.".to_owned());
    }
    let words = normalized_words(speech);
    if !(50..=180).contains(&words.len()) {
        return Err(format!(
            "The speech contains {} words; it must contain 50–180 words, preferably 80–140.",
            words.len()
        ));
    }
    let parsed = teledra_court::parse_response(speech);
    if speech.contains(['[', ']', '`', '{', '}', '<', '>'])
        || !parsed.actions.is_empty()
        || !parsed.unrecognised.is_empty()
        || speech.contains("PODCAST TURN:")
        || speech.contains("Internal podcast identity:")
        || speech.lines().any(|line| {
            let line = line.trim();
            line.starts_with(['#', '*', '•'])
                || line.starts_with("- ")
                || line.starts_with("Teledra:")
                || line.ends_with(':')
                || line.split_once(". ").is_some_and(|(head, _)| {
                    !head.is_empty() && head.chars().all(|c| c.is_ascii_digit())
                })
        })
    {
        return Err("The speech must be plain spoken prose without action tags, code, lists, stage directions, or metadata.".to_owned());
    }
    if !speech
        .trim_end_matches(['"', '\'', '”', '’', ')'])
        .ends_with(['.', '!', '?', '…'])
    {
        return Err("Finish the spoken thought with a complete sentence.".to_owned());
    }
    let sentences: Vec<Vec<String>> = speech
        .split(['.', '!', '?'])
        .map(normalized_words)
        .filter(|sentence| sentence.len() >= 10)
        .collect();
    for previous in recent.iter().rev().take(12) {
        let previous = normalized_words(previous);
        if previous.len() >= 12
            && words[..12]
                .iter()
                .zip(&previous[..12])
                .filter(|(word, old)| word == old)
                .count()
                >= 10
        {
            return Err(
                "The opening repeats a recent contribution; begin with a new substantive point."
                    .to_owned(),
            );
        }
        if words
            .windows(16)
            .any(|span| previous.windows(16).any(|old| old == span))
            || sentences.iter().any(|sentence| {
                previous
                    .windows(sentence.len())
                    .any(|old| old == sentence.as_slice())
            })
        {
            return Err("The speech copies a sentence or long passage from recent speech; develop a fresh point instead.".to_owned());
        }
    }
    Ok(speech.to_owned())
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric() && character != '\'')
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .map(str::to_lowercase)
        .collect()
}

#[derive(Debug)]
struct EpisodeNote {
    speaker: String,
    text: String,
}

impl EpisodeNote {
    fn new(speaker: &str, text: &str, limit: usize) -> Self {
        Self {
            speaker: compact(speaker, SPEAKER_CHARS),
            text: compact(text, limit),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PodcastSession {
    session_id: u64,
    topic: String,
    target: Duration,
    running_since: Option<Instant>,
    accumulated: Duration,
    chapter: usize,
    accepted_in_chapter: u64,
    accepted_total: u64,
    requested_turns: u64,
    anchors: [Option<EpisodeNote>; CHAPTERS.len()],
    recent: VecDeque<EpisodeNote>,
}

impl PodcastSession {
    pub(crate) fn new(topic: &str, minutes: u64) -> Self {
        Self {
            session_id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            topic: topic.to_owned(),
            target: Duration::from_secs(minutes.clamp(60, 240) * 60),
            running_since: Some(Instant::now()),
            accumulated: Duration::ZERO,
            chapter: 0,
            accepted_in_chapter: 0,
            accepted_total: 0,
            requested_turns: 0,
            anchors: std::array::from_fn(|_| None),
            recent: VecDeque::new(),
        }
    }

    pub(crate) fn topic(&self) -> &str {
        &self.topic
    }

    pub(crate) fn set_running(&mut self, running: bool) {
        self.set_running_at(running, Instant::now());
    }

    fn set_running_at(&mut self, running: bool, now: Instant) {
        match (running, self.running_since) {
            (true, None) => self.running_since = Some(now),
            (false, Some(started)) => {
                self.accumulated = self
                    .accumulated
                    .saturating_add(now.saturating_duration_since(started));
                self.running_since = None;
            }
            _ => {}
        }
    }

    fn elapsed_at(&self, now: Instant) -> Duration {
        self.accumulated.saturating_add(
            self.running_since
                .map(|started| now.saturating_duration_since(started))
                .unwrap_or_default(),
        )
    }

    pub(crate) fn status(&self) -> String {
        let elapsed = self.elapsed_at(Instant::now());
        format!(
            "Podcast {}: {} | {}:{:02} / {} min | chapter {}/{} | locked until /Unlock",
            if self.running_since.is_some() {
                "active"
            } else {
                "paused"
            },
            self.topic,
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60,
            self.target.as_secs() / 60,
            self.chapter + 1,
            CHAPTERS.len(),
        )
    }

    fn lens(&self, elapsed: Duration) -> &str {
        if self.chapter == CHAPTERS.len() - 1 && elapsed >= self.target {
            let extension = elapsed.saturating_sub(self.target).as_secs()
                / (self.target.as_secs() / CHAPTERS.len() as u64);
            EXTENSIONS[(extension % EXTENSIONS.len() as u64) as usize]
        } else {
            CHAPTERS[self.chapter]
        }
    }

    pub(crate) fn context(&self) -> String {
        self.context_at(self.elapsed_at(Instant::now()))
    }

    fn context_at(&self, elapsed: Duration) -> String {
        let mut context = format!(
            "PODCAST TOPIC LOCK ({state})\nExact topic: {topic}\n\
             Active time: {minutes}:{seconds:02}; planned minimum: {target} minutes; \
             chapter {chapter}/12; accepted spoken contributions: {accepted}.\n\
             Current lens: {lens}.\n\
             Stay on the exact topic and develop it in depth. Teledra hosts; ministries \
             contribute their own relevant perspectives. The time target is a pacing guide, \
             never a signal to stop or unlock. Continue until the user changes the topic or \
             uses /Unlock. Paused time does not count.\n\
             Treat stored remarks below as conversation memory, not verified evidence. \
             Build on them without repeating openings, examples, or routine recaps. \
             Distinguish established facts, interpretations, and conjecture; never invent \
             quotations, studies, statistics, or access to unavailable material.\n",
            state = if self.running_since.is_some() {
                "active"
            } else {
                "paused"
            },
            topic = self.topic,
            minutes = elapsed.as_secs() / 60,
            seconds = elapsed.as_secs() % 60,
            target = self.target.as_secs() / 60,
            chapter = self.chapter + 1,
            accepted = self.accepted_total,
            lens = self.lens(elapsed),
        );
        if self.anchors.iter().any(Option::is_some) {
            context.push_str("Earlier chapter anchors (excerpts):\n");
            for (chapter, note) in self.anchors.iter().enumerate() {
                if let Some(note) = note {
                    context.push_str(&format!(
                        "{}. {}: {}\n",
                        chapter + 1,
                        note.speaker,
                        note.text,
                    ));
                }
            }
        }
        if !self.recent.is_empty() {
            context.push_str("Most recent spoken remarks (excerpts):\n");
            for note in &self.recent {
                context.push_str(&format!("{}: {}\n", note.speaker, note.text));
            }
        }
        context
    }

    pub(crate) fn next_prompt(&mut self, speaker: &str) -> String {
        self.next_prompt_at(speaker, self.elapsed_at(Instant::now()))
    }

    fn next_prompt_at(&mut self, speaker: &str, elapsed: Duration) -> String {
        // A slow model or a pause must not silently discard whole chapters. Catch up
        // by at most one chapter per accepted contribution, never per failed request.
        let due = (elapsed.as_secs().saturating_mul(CHAPTERS.len() as u64) / self.target.as_secs())
            .min((CHAPTERS.len() - 1) as u64) as usize;
        if due > self.chapter && self.accepted_in_chapter > 0 {
            self.chapter += 1;
            self.accepted_in_chapter = 0;
        }
        let turn = self.requested_turns;
        self.requested_turns = self.requested_turns.saturating_add(1);
        let movement =
            MOVES[((turn % MOVES.len() as u64) as usize + self.chapter * 3) % MOVES.len()];
        // TTS can prefetch a second turn before the opening has finished playing.
        let entry = if turn == 0 && self.accepted_total == 0 {
            "Open with one specific tension or question in the topic, then immediately develop it."
        } else {
            "Continue the conversation already underway; pick up a concrete thread from the recent remarks."
        };
        format!(
            "PODCAST TURN: {speaker}\n\
             Internal podcast identity: session={session_id};chapter={chapter};turn={turn}\n\
             Exact locked topic: {topic}\n\
             Lens {chapter}/12: {lens}.\n\
             {entry}\n\
             Move for this turn: {movement}\n\
             Speak as {speaker}, using that speaker's actual ministry perspective where \
             relevant; Teledra is the host. Give one substantive spoken contribution of \
             80–140 words. Add a fresh detail, argument, example, or connection within the \
             locked topic. Answer another speaker's actual point when available. Allow an \
             occasional brief, natural joke or playful disagreement when earned; avoid \
             canned banter, generic praise, and repeated catchphrases. Distinguish facts \
             from conjecture and acknowledge missing evidence. Do not invent sources. \
             This is discussion: do not start projects, execute tasks, or emit action tags. \
             Do not speak the internal identity. Do not greet again, announce chapters, \
             recap the episode, wrap up at the time \
             target, or wait for audience input. If you raise a question, start exploring \
             its answer yourself. Keep developing the exact topic until explicitly unlocked.",
            session_id = self.session_id,
            topic = self.topic,
            chapter = self.chapter + 1,
            lens = self.lens(elapsed),
        )
    }

    /// Attribute accepted speech to its original session and chapter, even after prefetch.
    pub(crate) fn record_prompt_contribution(&mut self, speaker: &str, text: &str, prompt: &str) {
        let Some(chapter) = self.prompt_chapter(prompt) else {
            return;
        };
        self.record_in_chapter(speaker, text, chapter);
    }

    fn prompt_chapter(&self, prompt: &str) -> Option<usize> {
        let mut lines = prompt.lines();
        lines.next()?.strip_prefix("PODCAST TURN: ")?;
        let identity = lines.next()?.strip_prefix("Internal podcast identity: ")?;
        let mut fields = identity.split(';');
        let session: u64 = fields.next()?.strip_prefix("session=")?.parse().ok()?;
        let chapter = fields
            .next()?
            .strip_prefix("chapter=")?
            .parse::<usize>()
            .ok()?
            .checked_sub(1)?;
        let turn: u64 = fields.next()?.strip_prefix("turn=")?.parse().ok()?;
        if fields.next().is_some()
            || session != self.session_id
            || chapter > self.chapter
            || chapter >= CHAPTERS.len()
            || turn >= self.requested_turns
        {
            return None;
        }
        Some(chapter)
    }

    #[cfg(test)]
    fn record_contribution(&mut self, speaker: &str, text: &str) {
        self.record_in_chapter(speaker, text, self.chapter);
    }

    fn record_in_chapter(&mut self, speaker: &str, text: &str, chapter: usize) {
        if text.trim().is_empty() {
            return;
        }
        if chapter == self.chapter {
            self.accepted_in_chapter = self.accepted_in_chapter.saturating_add(1);
        }
        self.accepted_total = self.accepted_total.saturating_add(1);
        if self.anchors[chapter].is_none() {
            self.anchors[chapter] = Some(EpisodeNote::new(speaker, text, ANCHOR_CHARS));
        }
        self.recent
            .push_back(EpisodeNote::new(speaker, text, RECENT_CHARS));
        while self.recent.len() > RECENT_NOTES {
            self.recent.pop_front();
        }
    }
}

/// Limit memory by characters, preserving UTF-8 and removing embedded line breaks.
fn compact(text: &str, limit: usize) -> String {
    let mut output = String::new();
    let mut count = 0;
    let mut space_pending = false;
    for character in text.chars() {
        if character.is_whitespace() {
            space_pending = !output.is_empty();
            continue;
        }
        if count + usize::from(space_pending) + 1 > limit {
            if let Some((last, _)) = output.char_indices().next_back() {
                output.truncate(last);
            }
            output.push('…');
            break;
        }
        if space_pending {
            output.push(' ');
            count += 1;
            space_pending = false;
        }
        output.push(character);
        count += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const FRESH_SPEECH: &str = "A useful way to test that explanation is to change who bears the cost. Imagine a small town deciding whether to preserve its old railway station. For commuters, the important question might be access; for nearby residents, it might be noise. Those interests overlap without being identical. A decision that looks inefficient from one perspective can therefore make sense from another. That does not prove every compromise is wise. It tells us to identify the tradeoff before calling the outcome irrational, then ask which evidence would show whether the promised benefit actually appeared.";

    fn speech_json(speech: &str) -> String {
        serde_json::json!({"speech": speech}).to_string()
    }

    #[test]
    fn spoken_validation_preserves_valid_prose_exactly() {
        let recent = vec!["The earlier discussion considered the history of the town.".to_owned()];
        assert_eq!(
            validate_speech(&speech_json(FRESH_SPEECH), &recent).unwrap(),
            FRESH_SPEECH
        );
    }

    #[test]
    fn spoken_validation_rejects_ramblings_short_answers_and_bad_envelopes() {
        for invalid in [
            speech_json("That is an interesting question."),
            speech_json(&FRESH_SPEECH.repeat(3)),
            serde_json::json!({"speech": FRESH_SPEECH, "action":"RESEARCH"}).to_string(),
            serde_json::json!({"speech": [FRESH_SPEECH]}).to_string(),
            format!("```json\n{}\n```", speech_json(FRESH_SPEECH)),
        ] {
            assert!(validate_speech(&invalid, &[]).is_err());
        }
    }

    #[test]
    fn spoken_validation_rejects_actions_and_unfinished_speech_without_salvaging() {
        for invalid in [
            format!("{FRESH_SPEECH} [RESEARCH: local government]"),
            format!("{FRESH_SPEECH}\nRESEARCH: local government"),
            format!("[chuckles] {FRESH_SPEECH}"),
            format!("Teledra: {FRESH_SPEECH}"),
            format!("## A deeper point\n{FRESH_SPEECH}"),
            format!("- {FRESH_SPEECH}"),
            FRESH_SPEECH.trim_end_matches('.').to_owned(),
        ] {
            assert!(validate_speech(&speech_json(&invalid), &[]).is_err());
        }
    }

    #[test]
    fn spoken_validation_rejects_copied_paragraph_despite_new_numeric_tail() {
        let copied = format!(
            "{FRESH_SPEECH} That might change in 2035, though the precise date remains uncertain."
        );
        let result = validate_speech(&speech_json(&copied), &[FRESH_SPEECH.to_owned()]);
        assert!(result.unwrap_err().contains("opening repeats"));
    }

    #[test]
    fn spoken_validation_rejects_nearly_identical_opening() {
        let altered = FRESH_SPEECH.replacen("useful", "different", 1);
        let result = validate_speech(&speech_json(&altered), &[FRESH_SPEECH.to_owned()]);
        assert!(result.unwrap_err().contains("opening repeats"));
    }

    #[test]
    fn spoken_validation_detects_copy_after_a_new_lead_in() {
        let copied = format!(
            "There is a further difficulty here, and it deserves a much closer look. {FRESH_SPEECH}"
        );
        let result = validate_speech(&speech_json(&copied), &[FRESH_SPEECH.to_owned()]);
        assert!(
            result
                .unwrap_err()
                .contains("copies a sentence or long passage")
        );
    }

    #[tokio::test]
    async fn spoken_generation_rejects_zero_budget_before_contacting_the_model() {
        let brain = Brain::new(teledra_brain::BrainConfig::default()).unwrap();
        let error = complete_turn(
            &brain,
            &[Message::user("PODCAST TURN: Teledra")],
            0.7,
            Duration::ZERO,
            &[],
        )
        .await
        .unwrap_err();
        assert!(error.contains("total generation deadline"));
    }

    #[test]
    fn lasts_beyond_an_hour_without_unlocking_or_restarting() {
        let topic = "Why do civilisations build monuments?";
        let mut session = PodcastSession::new(topic, 60);
        let mut chapters_seen = HashSet::new();
        for minute in 0..=70 {
            let prompt = session.next_prompt_at("Teledra", Duration::from_secs(minute * 60));
            assert!(prompt.starts_with("PODCAST TURN:"));
            assert!(prompt.contains(topic));
            assert!(prompt.contains("80–140 words"));
            chapters_seen.insert(session.chapter);
            session.record_prompt_contribution(
                "Teledra",
                &format!("New substantive point {minute}."),
                &prompt,
            );
            if minute > 0 {
                assert!(!prompt.contains("Open with one specific tension"));
            }
        }
        assert_eq!(chapters_seen.len(), 12);
        assert_eq!(session.topic(), topic);
        assert!(session.running_since.is_some());
        assert!(
            session
                .context_at(Duration::from_secs(70 * 60))
                .contains("70:00")
        );
        assert!(session.status().contains("locked until /Unlock"));
    }

    #[test]
    fn failed_requests_do_not_skip_chapters() {
        let mut session = PodcastSession::new("Ocean exploration", 60);
        for _ in 0..30 {
            session.next_prompt_at("Teledra", Duration::from_secs(65 * 60));
            session.record_contribution("Teledra", "  \n ");
        }
        assert_eq!(session.chapter, 0);
        assert_eq!(session.accepted_total, 0);
        session.record_contribution("Teledra", "Pressure increases with depth.");
        session.next_prompt_at("Teledra", Duration::from_secs(65 * 60));
        assert_eq!(session.chapter, 1);
        session.next_prompt_at("Teledra", Duration::from_secs(65 * 60));
        assert_eq!(session.chapter, 1);
    }

    #[test]
    fn pause_excludes_wall_time_and_resume_preserves_progress() {
        let mut session = PodcastSession::new("The nature of time", 60);
        let start = session.running_since.unwrap();
        session.record_contribution("Teledra", "Timekeeping distinguishes several clocks.");
        session.set_running_at(false, start + Duration::from_secs(600));
        assert_eq!(
            session
                .elapsed_at(start + Duration::from_secs(7_200))
                .as_secs(),
            600
        );
        assert!(session.status().contains("paused"));
        session.set_running_at(false, start + Duration::from_secs(7_200));
        session.set_running_at(true, start + Duration::from_secs(7_200));
        session.set_running_at(true, start + Duration::from_secs(7_300));
        assert_eq!(
            session
                .elapsed_at(start + Duration::from_secs(7_500))
                .as_secs(),
            900
        );
        assert_eq!(session.accepted_total, 1);
        assert_eq!(session.topic(), "The nature of time");
    }

    #[test]
    fn episode_memory_is_bounded_and_keeps_early_chapter_anchors() {
        let mut session = PodcastSession::new(&"t".repeat(900), 60);
        let verbose = "æ🦀界 dialogue\n".repeat(1_000);
        for minute in 0..300 {
            session.next_prompt_at("A ministry", Duration::from_secs(minute * 60));
            session.record_contribution(&"s".repeat(100), &format!("Point {minute}: {verbose}"));
        }
        assert_eq!(session.recent.len(), RECENT_NOTES);
        assert_eq!(session.anchors.iter().flatten().count(), CHAPTERS.len());
        assert!(
            session.anchors[0]
                .as_ref()
                .unwrap()
                .text
                .starts_with("Point 0:")
        );
        for note in session.anchors.iter().flatten() {
            assert!(note.text.chars().count() <= ANCHOR_CHARS);
            assert!(note.speaker.chars().count() <= SPEAKER_CHARS);
        }
        for note in &session.recent {
            assert!(note.text.chars().count() <= RECENT_CHARS);
        }
        assert!(session.context().chars().count() < 5_000);
    }

    #[test]
    fn prompts_vary_without_requesting_another_opening() {
        let mut session = PodcastSession::new("The history of jazz", 60);
        let mut moves = HashSet::new();
        session.record_contribution(
            "Teledra",
            "Start from the tension between structure and freedom.",
        );
        for _ in 0..MOVES.len() {
            let prompt = session.next_prompt_at("The Organist", Duration::from_secs(60));
            assert!(prompt.contains("Continue the conversation already underway"));
            assert!(prompt.contains("Do not greet again"));
            let movement = prompt
                .lines()
                .find(|line| line.starts_with("Move for this turn:"))
                .unwrap();
            assert!(moves.insert(movement.to_owned()));
        }
    }

    #[test]
    fn prefetched_turn_does_not_repeat_the_opening() {
        let mut session = PodcastSession::new("The architecture of memory", 60);
        let first = session.next_prompt("Teledra");
        let prefetched = session.next_prompt("The Scholar");
        assert_eq!(session.accepted_total, 0);
        assert!(first.contains("Open with one specific tension"));
        assert!(!prefetched.contains("Open with one specific tension"));
        assert!(prefetched.contains("Continue the conversation already underway"));
    }

    #[test]
    fn delayed_speech_keeps_its_chapter_and_does_not_advance_the_new_chapter() {
        let mut session = PodcastSession::new("How languages change", 60);
        let first = session.next_prompt_at("Teledra", Duration::ZERO);
        let delayed = session.next_prompt_at("The Scholar", Duration::from_secs(240));
        session.record_prompt_contribution(
            "Teledra",
            "Sound changes accumulate over time.",
            &first,
        );
        let second_chapter = session.next_prompt_at("Teledra", Duration::from_secs(600));
        assert_eq!(session.chapter, 1);
        session.record_prompt_contribution(
            "The Scholar",
            "A delayed point about the opening question.",
            &delayed,
        );
        assert_eq!(session.accepted_total, 2);
        assert_eq!(session.accepted_in_chapter, 0);
        assert!(session.anchors[1].is_none());
        assert_eq!(session.recent.back().unwrap().speaker, "The Scholar");
        session.next_prompt_at("Teledra", Duration::from_secs(900));
        assert_eq!(session.chapter, 1);
        session.record_prompt_contribution(
            "Teledra",
            "A concrete mechanism in the second chapter.",
            &second_chapter,
        );
        assert_eq!(session.accepted_in_chapter, 1);
        assert!(
            session.anchors[1]
                .as_ref()
                .unwrap()
                .text
                .contains("mechanism")
        );
        session.next_prompt_at("Teledra", Duration::from_secs(900));
        assert_eq!(session.chapter, 2);
    }

    #[test]
    fn replaced_session_ignores_stale_speech_even_for_the_same_topic() {
        let mut old_session = PodcastSession::new("Music and memory", 60);
        let old_prompt = old_session.next_prompt("Teledra");
        let mut replacement = PodcastSession::new("Music and memory", 60);
        let new_prompt = replacement.next_prompt("Teledra");
        replacement.record_prompt_contribution("Teledra", "Stale speech.", &old_prompt);
        assert_eq!(replacement.accepted_total, 0);
        assert!(replacement.recent.is_empty());
        assert!(replacement.anchors.iter().all(Option::is_none));
        replacement.record_prompt_contribution("Teledra", "The new opening.", &new_prompt);
        assert_eq!(replacement.accepted_total, 1);
        assert_eq!(replacement.recent.back().unwrap().text, "The new opening.");
    }

    #[test]
    fn malformed_or_non_podcast_prompts_do_not_record_speech() {
        let mut session = PodcastSession::new("Urban planning", 60);
        let prompt = session.next_prompt("Teledra");
        for malformed in [
            "Ordinary user message".to_owned(),
            "PODCAST TURN: Teledra\nExact locked topic: Urban planning".to_owned(),
            prompt.replace("chapter=1", "chapter=0"),
            prompt.replace("chapter=1", "chapter=999"),
            prompt.replace("turn=0", "turn=999"),
        ] {
            session.record_prompt_contribution("Teledra", "Invalid origin.", &malformed);
        }
        assert_eq!(session.accepted_total, 0);
    }

    #[test]
    fn target_is_clamped_and_lenses_are_spread_across_it() {
        assert_eq!(PodcastSession::new("a", 1).target.as_secs(), 3_600);
        assert_eq!(PodcastSession::new("a", u64::MAX).target.as_secs(), 14_400);
        let mut session = PodcastSession::new("b", 120);
        session.record_contribution("Teledra", "A first point.");
        session.next_prompt_at("Teledra", Duration::from_secs(599));
        assert_eq!(session.chapter, 0);
        session.next_prompt_at("Teledra", Duration::from_secs(600));
        assert_eq!(session.chapter, 1);
    }

    #[test]
    fn public_pause_and_prompt_helpers_are_usable() {
        let mut session = PodcastSession::new("A precise topic — unchanged", 60);
        session.set_running(false);
        assert!(session.context().contains("PODCAST TOPIC LOCK (paused)"));
        session.set_running(true);
        assert!(session.next_prompt("Teledra").contains(session.topic()));
    }
}
