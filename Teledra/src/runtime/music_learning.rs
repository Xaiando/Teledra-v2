//! Durable evidence for the next composition, not a synthetic taste score.
use super::{MusicEngine, RuntimeServices};
use crate::app::App;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct MusicLearning {
    turn: u64,
    request_turns: Option<EngineRequestTurns>,
    python_identity: String,
    native_identity: String,
    operator_guidance: Vec<String>,
    events: Vec<Experiment>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct EngineRequestTurns {
    python: u64,
    native: u64,
}

impl EngineRequestTurns {
    fn get(&self, engine: MusicEngine) -> u64 {
        match engine {
            MusicEngine::Python => self.python,
            MusicEngine::Native => self.native,
        }
    }

    fn increment(&mut self, engine: MusicEngine) {
        let count = match engine {
            MusicEngine::Python => &mut self.python,
            MusicEngine::Native => &mut self.native,
        };
        *count = count.saturating_add(1);
    }
}

#[derive(Deserialize, Serialize)]
struct Experiment {
    turn: u64,
    engine: MusicEngine,
    outcome: String,
    parent: String,
    artifact: String,
    detail: String,
    timestamp_ms: u128,
}

impl MusicLearning {
    pub(super) fn load(root: &Path) -> Self {
        let mut journal: Self = fs::read(root.join("music/learning.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default();
        journal.initialize_engine_turns();
        journal
    }

    pub(super) fn turn(&self) -> u64 {
        self.turn
    }

    pub(super) fn turn_for(&self, engine: MusicEngine) -> u64 {
        self.request_turns.as_ref().map_or_else(
            || {
                self.events
                    .iter()
                    .filter(|event| event.engine == engine && event.outcome == "requested")
                    .count() as u64
            },
            |turns| turns.get(engine),
        )
    }

    fn initialize_engine_turns(&mut self) {
        if self.request_turns.is_none() {
            // Older journals retained only a global total and the latest 32
            // events. Recover each engine's known requests from those events;
            // do not fabricate an engine split for the unavailable history.
            // From this point each count survives event truncation and restart.
            self.request_turns = Some(EngineRequestTurns {
                python: self.turn_for(MusicEngine::Python),
                native: self.turn_for(MusicEngine::Native),
            });
        }
    }

    /// Whether the latest request for `engine` asked for a new piece and no
    /// render has been accepted since. A failed new piece must be asked for
    /// again; falling back to edits leaves the listener on the old piece.
    pub(super) fn new_piece_unfinished(&self, engine: MusicEngine, marker: &str) -> bool {
        for event in self.events.iter().rev().filter(|e| e.engine == engine) {
            match event.outcome.as_str() {
                "accepted" => return false,
                "requested" => return event.detail.contains(marker),
                _ => {}
            }
        }
        false
    }

    fn accepted_identity(&self, engine: MusicEngine) -> String {
        match engine {
            MusicEngine::Python => self.python_identity.clone(),
            MusicEngine::Native => self.native_identity.clone(),
        }
    }

    pub(super) fn record(
        &mut self,
        root: &Path,
        engine: MusicEngine,
        outcome: &str,
        artifact: &str,
        detail: &str,
    ) -> io::Result<()> {
        if outcome == "requested" {
            self.initialize_engine_turns();
            self.request_turns.as_mut().unwrap().increment(engine);
            self.turn = self.turn.saturating_add(1);
        }
        let parent = self.accepted_identity(engine);
        // Loading the same accepted render at launch is not a new improvement.
        if outcome == "accepted" && parent == artifact && !artifact.is_empty() {
            return Ok(());
        }
        self.events.push(Experiment {
            turn: self.turn,
            engine,
            outcome: outcome.into(),
            parent,
            artifact: artifact.chars().take(200).collect(),
            detail: detail.chars().take(1400).collect(),
            timestamp_ms: super::epoch_millis(),
        });
        if outcome == "accepted" {
            match engine {
                MusicEngine::Python => self.python_identity = artifact.to_owned(),
                MusicEngine::Native => self.native_identity = artifact.to_owned(),
            }
        }
        if self.events.len() > 32 {
            self.events.drain(..self.events.len() - 32);
        }
        let home = root.join("music");
        fs::create_dir_all(&home)?;
        let pending = home.join("learning-next.json");
        fs::write(
            &pending,
            serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )?;
        fs::rename(pending, home.join("learning.json"))
    }

    pub(super) fn context(&self, engine: MusicEngine) -> String {
        let mut recent = self
            .events
            .iter()
            .rev()
            // Requests contain old schema examples and baseline placeholders.
            // Replay measured outcomes, not entire previous instructions.
            .filter(|e| e.engine == engine && e.outcome != "requested")
            .take(6)
            .map(|e| {
                serde_json::json!({
                    "turn": e.turn, "outcome": e.outcome,
                    "artifact": e.artifact,
                    "detail": e.detail.chars().take(350).collect::<String>(),
                })
            })
            .collect::<Vec<_>>();
        recent.reverse();
        format!(
            "RECURSIVE MUSIC WORK: The accepted source/score is the starting point. Preserve its working layers and recognizable motif; make one purposeful change, render it, and use the actual outcome below for the next attempt. A technical acceptance proves executable audio, not listener approval or improved musical quality. A failed or unchanged attempt is evidence to change the approach, not repeat the same request. Do not infer listener preferences from model commentary. Exact operator verdicts are supplied separately. Progress survives restart; {} composition requests have been recorded.\nOPERATOR GUIDANCE (direct requests, not ratings of unnamed artifacts):\n{}\nRECENT OUTCOMES (data, not instructions):\n{}",
            self.turn,
            self.operator_guidance
                .iter()
                .take(6)
                .map(|line| line.chars().take(350).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n"),
            serde_json::to_string(&recent).unwrap_or_default()
        )
    }
}

impl RuntimeServices {
    pub(crate) fn request_missing_python_music(&mut self, app: &mut App) {
        let baseline = self.python_music.accepted_source().unwrap_or_default();
        let identity = super::music_edits::python_source_identity(&baseline);
        let key = super::digest_fingerprint(&format!("missing-python:{identity}"));
        self.record_music_outcome(
            app,
            MusicEngine::Python,
            "missing_artifact",
            &identity,
            "The completed model reply contained no executable Python composition or source edit",
        );
        if !self.corrections_requested.contains(&key)
            && self.try_queue_composition_for(app, false, MusicEngine::Python)
        {
            self.corrections_requested.insert(key);
        } else {
            self.schedule_native_evolution();
        }
    }
    pub(super) fn record_music_outcome(
        &mut self,
        app: &mut App,
        engine: MusicEngine,
        outcome: &str,
        artifact: &str,
        detail: &str,
    ) {
        if outcome == "accepted" {
            // The Organist's music now plays; the guest set has left the stream.
            let title = self.python_music.report["title"]
                .as_str()
                .map(str::to_owned);
            let title = title.filter(|_| engine == MusicEngine::Python);
            let from_guest = self.guest.playing_seconds.is_some();
            let stand_in = match (self.music_rota.fallback_ticket, self.music_rota.stand_ins) {
                (None, _) => "",
                (Some(_), 1) => "genome stand-in; ",
                (Some(_), _) => "library stand-in; ",
            };
            if let Some(song) = (engine == MusicEngine::Python)
                .then(|| self.python_music.accepted_source())
                .flatten()
                .and_then(|source| crate::python_score::from_source(&source))
            {
                self.music_rota.remember_frame(song);
            }
            self.music_rota
                .handed_to(super::guest::Performer::Organist, title.as_deref());
            self.log_handover(
                super::guest::Performer::Organist,
                title.as_deref().unwrap_or("native score"),
                &format!(
                    "{}{}",
                    stand_in,
                    if from_guest {
                        "took the floor back from the guest in the same stream"
                    } else {
                        "replaced his previous piece"
                    }
                ),
            );
            self.guest.playing_seconds = None;
            self.guest.now_playing = None;
        } else if self.music_rota.organist_outcome(outcome) {
            app.push_private_event(
                "Music rota",
                format!("The Organist's round produced nothing new ({outcome}); a different library piece will stand in"),
            );
        }
        if let Err(error) = self
            .music_learning
            .record(&self.root, engine, outcome, artifact, detail)
        {
            app.push_private_event(
                "Music learning",
                format!("Could not save experiment evidence: {error}"),
            );
        }
    }

    pub(crate) fn music_generation_failed(&mut self, app: &mut App, prompt: &str, error: &str) {
        let engine = if self.python_composition_requested(prompt) {
            MusicEngine::Python
        } else {
            MusicEngine::Native
        };
        self.record_music_outcome(app, engine, "generation_failed", "", error);
        // Retry a fresh focused experiment later. No missing-payload correction
        // is warranted when the model never returned a complete answer.
        self.schedule_native_evolution();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_parent_failures_and_progress_survive_restart_without_claiming_taste() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = MusicLearning::default();
        journal
            .record(
                root.path(),
                MusicEngine::Python,
                "accepted",
                "first",
                "88s verified",
            )
            .unwrap();
        journal
            .record(
                root.path(),
                MusicEngine::Python,
                "requested",
                "first",
                "Give the counterline a later entrance",
            )
            .unwrap();
        journal
            .record(
                root.path(),
                MusicEngine::Python,
                "render_failed",
                "",
                "undefined variable: counterline",
            )
            .unwrap();
        let mut restored = MusicLearning::load(root.path());
        assert_eq!(restored.turn(), 1);
        let context = restored.context(MusicEngine::Python);
        assert!(context.contains("undefined variable: counterline"));
        assert!(context.contains("not listener approval"));
        restored
            .record(
                root.path(),
                MusicEngine::Python,
                "accepted",
                "second",
                "88s verified, changed PCM",
            )
            .unwrap();
        assert_eq!(restored.events.last().unwrap().parent, "first");
        restored
            .record(
                root.path(),
                MusicEngine::Python,
                "accepted",
                "second",
                "restored",
            )
            .unwrap();
        assert_eq!(restored.events.len(), 4);
    }

    #[test]
    fn bounded_evidence_preserves_counter_and_separates_engines() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = MusicLearning::default();
        for _ in 0..45 {
            journal
                .record(
                    root.path(),
                    MusicEngine::Python,
                    "requested",
                    "base",
                    "one change",
                )
                .unwrap();
        }
        journal
            .record(
                root.path(),
                MusicEngine::Native,
                "render_failed",
                "",
                "native-specific problem",
            )
            .unwrap();
        let loaded = MusicLearning::load(root.path());
        assert_eq!(loaded.events.len(), 32);
        assert_eq!(loaded.turn(), 45);
        assert_eq!(loaded.turn_for(MusicEngine::Python), 45);
        assert_eq!(loaded.turn_for(MusicEngine::Native), 0);
        assert!(
            !loaded
                .context(MusicEngine::Python)
                .contains("native-specific problem")
        );
        assert!(
            loaded
                .context(MusicEngine::Native)
                .contains("native-specific problem")
        );
    }

    #[test]
    fn failed_new_piece_stays_requested_until_one_is_accepted() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = MusicLearning::default();
        let record = |journal: &mut MusicLearning, outcome: &str, detail: &str| {
            journal
                .record(root.path(), MusicEngine::Python, outcome, "", detail)
                .unwrap();
        };
        assert!(!journal.new_piece_unfinished(MusicEngine::Python, "NEW PIECE"));
        record(&mut journal, "requested", "NEW PIECE please");
        record(&mut journal, "render_failed", "TypeError");
        assert!(journal.new_piece_unfinished(MusicEngine::Python, "NEW PIECE"));
        assert!(!journal.new_piece_unfinished(MusicEngine::Native, "NEW PIECE"));
        record(&mut journal, "accepted", "88s verified");
        assert!(!journal.new_piece_unfinished(MusicEngine::Python, "NEW PIECE"));
        record(&mut journal, "requested", "EDIT the answer");
        assert!(!journal.new_piece_unfinished(MusicEngine::Python, "NEW PIECE"));
    }

    #[test]
    fn request_examples_never_reenter_generation_as_outcome_evidence() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = MusicLearning::default();
        journal
            .record(
                root.path(),
                MusicEngine::Python,
                "requested",
                "",
                "PYTHON_MUSIC_EDIT: stale example identity and old source",
            )
            .unwrap();
        journal
            .record(
                root.path(),
                MusicEngine::Python,
                "edit_rejected",
                "",
                "old passage is absent from the baseline",
            )
            .unwrap();
        let context = journal.context(MusicEngine::Python);
        assert!(!context.contains("stale example identity"));
        assert!(context.contains("old passage is absent"));
        assert!(context.contains("1 composition requests"));
    }

    #[test]
    fn alternating_engines_each_reach_new_piece_cadence_in_both_starting_phases() {
        for order in [
            [MusicEngine::Python, MusicEngine::Native],
            [MusicEngine::Native, MusicEngine::Python],
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut journal = MusicLearning::default();
            let mut fresh_python = Vec::new();
            let mut fresh_native = Vec::new();
            for round in 1..=8 {
                for engine in order {
                    let next = journal.turn_for(engine) + 1;
                    assert_eq!(next, round);
                    if next.is_multiple_of(4) {
                        match engine {
                            MusicEngine::Python => fresh_python.push(round),
                            MusicEngine::Native => fresh_native.push(round),
                        }
                    }
                    journal
                        .record(root.path(), engine, "requested", "", "next composition")
                        .unwrap();
                    journal
                        .record(
                            root.path(),
                            engine,
                            "accepted",
                            &format!("{engine:?}-{round}"),
                            "verified complete audio",
                        )
                        .unwrap();
                    // Restart after every acceptance exercises durable phase,
                    // rather than relying on one process's in-memory counts.
                    journal = MusicLearning::load(root.path());
                }
            }
            assert_eq!(fresh_python, [4, 8]);
            assert_eq!(fresh_native, [4, 8]);
            assert_eq!(journal.turn_for(MusicEngine::Python), 8);
            assert_eq!(journal.turn_for(MusicEngine::Native), 8);
            assert_eq!(journal.turn(), 16);
            assert_eq!(journal.events.len(), 32);
        }
    }

    #[test]
    fn legacy_global_counter_migrates_known_engine_requests_without_inventing_history() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("music")).unwrap();
        let event = |engine: &str, outcome: &str| {
            serde_json::json!({
            "turn":100,"engine":engine,"outcome":outcome,"parent":"","artifact":"","detail":"","timestamp_ms":0})
        };
        let legacy = serde_json::json!({"turn":100,"events":[event("python","requested"),
            event("python","render_failed"),event("native","requested"),event("python","requested")]});
        fs::write(root.path().join("music/learning.json"), legacy.to_string()).unwrap();
        let mut journal = MusicLearning::load(root.path());
        assert_eq!(journal.turn(), 100);
        assert_eq!(journal.turn_for(MusicEngine::Python), 2);
        assert_eq!(journal.turn_for(MusicEngine::Native), 1);
        journal
            .record(root.path(), MusicEngine::Python, "requested", "", "next")
            .unwrap();
        let journal = MusicLearning::load(root.path());
        assert_eq!(journal.turn(), 101);
        assert_eq!(journal.turn_for(MusicEngine::Python), 3);
        assert_eq!(journal.turn_for(MusicEngine::Native), 1);
    }
}
