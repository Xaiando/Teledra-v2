//! Speech for the court.
//!
//! This crate consumes utterances the court has already decided to say. It
//! never edits their text and never calls Qwen — by the time an utterance
//! arrives here, what will be said is settled.
//!
//! # Why this exists in the shape it does
//!
//! V1 spawned a fresh synthesiser for every line. Measured on the operator's
//! machine that cost **15.6 s of wall clock to produce 3.5 s of audio**, of
//! which **10.4 s was loading the model** — paid again for every line, so a
//! stream ran with roughly twelve seconds of dead air between sentences.
//!
//! Keeping one worker warm changes the arithmetic completely: the same lines
//! then render in 1.5–3.5 s, which is *faster than they play back*. That is
//! the whole design. Two consequences follow and both are load bearing:
//!
//! 1. The model is loaded exactly once, at startup.
//! 2. Because synthesis outruns playback, the next utterance can be rendered
//!    while the current one is still being heard, so in steady state the gap
//!    between spoken lines is zero rather than merely smaller.

pub mod pcm;
pub mod voice;
pub mod worker;

pub use pcm::{Audio, PcmError};
pub use voice::{Voice, VoiceName};
pub use worker::{SynthesisError, Synthesizer};

use std::collections::VecDeque;

/// A line the court has committed to speaking.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Utterance {
    pub voice: VoiceName,
    pub text: String,
}

impl Utterance {
    pub fn new(voice: VoiceName, text: impl Into<String>) -> Self {
        Self {
            voice,
            text: text.into(),
        }
    }
}

/// What the court is told about its own voice, so a silent stage is always
/// explained rather than merely quiet.
#[derive(Clone, Debug, PartialEq)]
pub enum SpeechEvent {
    Started { voice: VoiceName },
    Finished { voice: VoiceName, seconds: f64 },
    Failed { voice: VoiceName, reason: String },
}

/// Orders utterances and keeps one rendered ahead of the one being heard.
///
/// The look-ahead is what removes the remaining gap: rendering line *n+1*
/// begins as soon as line *n* starts playing, and since a warm worker renders
/// faster than realtime, the audio is ready before it is needed.
#[derive(Debug)]
pub struct SpeechQueue {
    waiting: VecDeque<Utterance>,
    rendered: VecDeque<(VoiceName, Audio)>,
    look_ahead: usize,
}

impl Default for SpeechQueue {
    fn default() -> Self {
        Self::new(1)
    }
}

impl SpeechQueue {
    /// `look_ahead` is how many utterances may be rendered before they are
    /// needed. One is enough to close the gap; more only buys resilience
    /// against a slow line at the cost of wasted work when the court is
    /// interrupted.
    pub fn new(look_ahead: usize) -> Self {
        Self {
            waiting: VecDeque::new(),
            rendered: VecDeque::new(),
            look_ahead: look_ahead.max(1),
        }
    }

    pub fn enqueue(&mut self, utterance: Utterance) {
        self.waiting.push_back(utterance);
    }

    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    pub fn rendered(&self) -> usize {
        self.rendered.len()
    }

    pub fn is_idle(&self) -> bool {
        self.waiting.is_empty() && self.rendered.is_empty()
    }

    /// The next utterance that should be rendered now, if any.
    ///
    /// Returns `None` once enough audio is already banked, so a court that
    /// falls silent does not keep a GPU busy speculating.
    pub fn next_to_render(&mut self) -> Option<Utterance> {
        if self.rendered.len() >= self.look_ahead {
            return None;
        }
        self.waiting.pop_front()
    }

    pub fn accept(&mut self, voice: VoiceName, audio: Audio) {
        self.rendered.push_back((voice, audio));
    }

    /// The next audio to play, in the order the court asked for it.
    pub fn take_playable(&mut self) -> Option<(VoiceName, Audio)> {
        self.rendered.pop_front()
    }

    /// Abandons everything not yet heard. Used when the court is interrupted:
    /// finishing a stale thought over the top of a new one is worse than
    /// dropping it.
    pub fn clear(&mut self) {
        self.waiting.clear();
        self.rendered.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(seconds: f64) -> Audio {
        Audio {
            sample_rate: 48_000,
            samples: vec![0.0; (48_000.0 * seconds) as usize],
        }
    }

    #[test]
    fn utterances_are_heard_in_the_order_the_court_asked_for() {
        let mut queue = SpeechQueue::new(2);
        queue.enqueue(Utterance::new(VoiceName::Queen, "first"));
        queue.enqueue(Utterance::new(VoiceName::Organist, "second"));

        let first = queue.next_to_render().expect("a line to render");
        let second = queue.next_to_render().expect("a second line to render");
        assert_eq!(first.text, "first");
        assert_eq!(second.text, "second");

        // Rendering may finish out of order; playback must not.
        queue.accept(first.voice, audio(1.0));
        queue.accept(second.voice, audio(1.0));
        assert_eq!(queue.take_playable().expect("audio").0, VoiceName::Queen);
        assert_eq!(queue.take_playable().expect("audio").0, VoiceName::Organist);
    }

    #[test]
    fn rendering_runs_ahead_of_playback_but_not_without_limit() {
        // The look-ahead is the whole anti-dead-air mechanism: one line is
        // always being prepared while another is heard.
        let mut queue = SpeechQueue::new(1);
        for line in ["one", "two", "three"] {
            queue.enqueue(Utterance::new(VoiceName::Queen, line));
        }

        let first = queue.next_to_render().expect("render the first line");
        queue.accept(first.voice, audio(2.0));

        assert!(
            queue.next_to_render().is_none(),
            "with a line already banked the worker should idle, not speculate"
        );

        // Once that line is being heard, the next may be prepared.
        let _playing = queue.take_playable().expect("audio to play");
        let second = queue
            .next_to_render()
            .expect("the next line renders while the first is heard");
        assert_eq!(second.text, "two");
    }

    #[test]
    fn an_interrupted_court_drops_what_it_had_planned_to_say() {
        let mut queue = SpeechQueue::new(2);
        queue.enqueue(Utterance::new(VoiceName::Queen, "a long digression"));
        let rendering = queue.next_to_render().expect("a line");
        queue.accept(rendering.voice, audio(30.0));
        queue.enqueue(Utterance::new(VoiceName::Queen, "and another"));

        queue.clear();

        assert!(queue.is_idle());
        assert!(queue.take_playable().is_none());
        assert!(queue.next_to_render().is_none());
    }

    #[test]
    fn an_empty_queue_asks_for_no_work() {
        let mut queue = SpeechQueue::default();
        assert!(queue.is_idle());
        assert!(queue.next_to_render().is_none());
    }
}
