//! The production Organist boundary: a draft and one factual review or repair.
use std::future::Future;

use teledra_brain::{Brain, BrainError, Message};

/// Keep rejected scalar notes out of the next generation's imitation context.
/// The original operator message remains separate and takes precedence.
pub(crate) fn score_context_for_prompt(score: &teledra_music::Score) -> String {
    let scalar = score.scalar_exercise_concerns();
    if scalar.is_empty() {
        return format!(
            "MUSIC THEORY GUIDE:\n{}\nCURRENT ACCEPTED SCORE:\n{}\nCURRENT SCORE ANALYSIS:\n{}",
            teledra_music::THEORY_GUIDE,
            serde_json::to_string(score).unwrap_or_default(),
            score.theory_context()
        );
    }
    let voices = score
        .tracks
        .iter()
        .map(|track| {
            let info = track.instrument().info();
            format!(
                "{}: sounding MIDI {}..{}, attacks at least {} beats apart",
                info.id, info.range[0], info.range[1], info.attack_spacing
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "CURRENT SCORE REPAIR BRIEF — rejected scalar material is deliberately omitted.\nPRESERVE, unless the operator requests otherwise: {} BPM; tonic={}; mode={}; {} bars. Useful harmonic progression: {}. These are continuity defaults, not instructions to copy the old notes or form.\nCURRENT VOICES AND LIMITS: {voices}. Retain explicitly requested voices; the full instrument schema remains available without any novelty quota.\nREPLACE: {}\nCOMPOSE: establish a short recognizable motif, give its answering phrase a different onset rhythm and harmonic destination, create a measured contrast, then return to the motif with a settled ending. Give other parts slower harmonic or answering jobs. Write these phrases into the notes, rather than relying on a changed title, patch or form label. Choose steps_per_bar=4 or 8 when quarter/eighth-note starts fit the idea; duration never advances the grid. Preserve every explicit operator requirement over these repair defaults.",
        score.bpm,
        score.tonic,
        serde_json::to_string(&score.mode).unwrap_or_default(),
        score.bars,
        serde_json::to_string(&score.progression).unwrap_or_default(),
        scalar.join(" ")
    )
}

const ENCODING_GUIDE: &str = "SCORE ENCODING: Honor the original requested length, tempo, mode and instruments. Every new score needs composition fields form, phrase_bars, chord_bars, development, texture and harmony. Optional steps_per_bar is 4, 8 or 16 (default 16). Author each track with one notes array of 4, 8, 16, 32 or 64 entries. Each entry is one atomic event: {\"degree\":0,\"velocity\":0.7,\"duration\":0.5}; a rest is {\"degree\":null,\"velocity\":0,\"duration\":0.25}. All three fields are required. Velocity is 0..1; duration is 0.125..8 quarter-note beats. Do not output separate steps, velocities or durations arrays; those are saved-score storage. Degree 0 means the selected tonic: tonic already transposes the score, so tonic=9 does not require adding 9 to each degree. Usually keep a melody within degrees 0..7. Write the motif, rhythmically distinct answer and cadence into the notes. Choose the grid to give these phrases enough time: 32 events at steps_per_bar=4 span eight bars; 64 span sixteen. Short supporting cells may repeat. Preserve explicitly requested note counts.";

// Keep the transport's actual truncation signal separate from other failures.
// Error-message matching would retry unrelated model or connection failures.
#[derive(Clone, Debug, Eq, PartialEq)]
enum CompletionFailure {
    Truncated,
    Other(String),
}

impl From<BrainError> for CompletionFailure {
    fn from(error: BrainError) -> Self {
        match error {
            BrainError::TruncatedResponse => Self::Truncated,
            error => Self::Other(error.to_string()),
        }
    }
}

const REGENERATE_COMPLETE_SCORE: &str = "ONE COMPLETE SCORE REGENERATION. The first response reached its output-token limit before the complete JSON score was finished. Nothing from that unfinished response was accepted. This is the final inference for this request. Regenerate one COMPLETE compact JSON score from the original brief, preserving its requested instruments, bars, tempo, key, form, rests and expressive intent. Return every required field and every complete track; no prose, placeholders, ellipses or partial arrays. Use concise names and concise numeric values. When the brief leaves pattern lengths open, choose complete 4, 8 or 16-event cells as each musical role needs, with short supporting cells. Preserve explicitly requested longer note-event counts instead of shortening them. Shorter repeating cells do not change the requested total bars. Do not add a review or attempt to continue the unfinished response.";

const ONSET_GRID_GUIDE: &str = "ONSET GRID: Slot spacing is 4/min(event_count, steps_per_bar) beats. With the default steps_per_bar=16, four slots start every 1 beat, eight every 0.5 beats, and 16/32/64 every 0.25 beats. With steps_per_bar=4, any allowed array starts every 1 beat. Duration is held note length only and NEVER delays the next slot. Two adjacent notes in a 16-slot cell with duration 0.5 overlap on the default grid; they are not eighth-note onsets. Use a coarser grid or null spacers to respect the voice's minimum attack spacing. End a note before the next sounding onset for a real release gap; a null alone does not stop an earlier held note.";

/// Produces a natively valid score without padding, dropping or rewriting notes.
/// A valid first draft receives its own score-derived analysis for one musical
/// revision. An invalid first draft uses that second call for structural repair.
/// A truncated first draft uses it to regenerate a compact complete score.
/// No request can spend more than two inferences or reset its total deadline.
// The opt-in Organist integration harnesses include this module and call the
// default-budget entry point; production supplies its budget explicitly.
#[allow(dead_code)]
pub async fn compose_score(
    brain: &Brain,
    messages: &[Message],
    temperature: f32,
) -> Result<String, String> {
    compose_score_with_budget(brain, messages, temperature, brain.completion_timeout()).await
}

pub async fn compose_score_with_budget(
    brain: &Brain,
    messages: &[Message],
    temperature: f32,
    budget: std::time::Duration,
) -> Result<String, String> {
    let schema: serde_json::Value = serde_json::from_str(teledra_music::SCORE_SCHEMA)
        .map_err(|error| format!("Bundled music schema is invalid: {error}"))?;
    let deadline = tokio::time::Instant::now() + budget.min(brain.completion_timeout());
    compose_with_deadline(messages, temperature, deadline, |messages, temperature| {
        let brain = brain.clone();
        let schema = schema.clone();
        async move {
            brain
                .complete_structured(&messages, temperature, &schema)
                .await
                .map_err(CompletionFailure::from)
        }
    })
    .await
}

#[cfg(test)]
async fn compose_with<F, Fut>(
    original: &[Message],
    temperature: f32,
    complete: F,
) -> Result<String, String>
where
    F: FnMut(Vec<Message>, f32) -> Fut,
    Fut: Future<Output = Result<String, CompletionFailure>>,
{
    compose_with_deadline(
        original,
        temperature,
        tokio::time::Instant::now() + std::time::Duration::from_secs(120),
        complete,
    )
    .await
}

async fn compose_with_deadline<F, Fut>(
    original: &[Message],
    temperature: f32,
    deadline: tokio::time::Instant,
    mut complete: F,
) -> Result<String, String>
where
    F: FnMut(Vec<Message>, f32) -> Fut,
    Fut: Future<Output = Result<String, CompletionFailure>>,
{
    let mut messages = original.to_vec();
    messages.push(Message::system(format!(
        "{ENCODING_GUIDE}\n{ONSET_GRID_GUIDE}"
    )));
    let mut valid_draft = None;
    for attempt in 0..2 {
        if tokio::time::Instant::now() >= deadline {
            return valid_draft.as_ref().map_or_else(
                || Err("The Organist exceeded the total composition deadline; previous playback is retained.".to_owned()),
                store_score,
            );
        }
        let attempt_deadline = if valid_draft.is_some() {
            deadline.min(tokio::time::Instant::now() + std::time::Duration::from_secs(15))
        } else {
            deadline
        };
        let result = tokio::time::timeout_at(attempt_deadline, complete(
            messages.clone(),
            if attempt == 0 {
                temperature
            } else if valid_draft.is_some() {
                0.35
            } else {
                0.2
            },
        ))
        .await
        .unwrap_or_else(|_| Err(CompletionFailure::Other("The Organist exceeded the total composition deadline; previous playback is retained.".to_owned())));
        let response = match result {
            Ok(response) => response,
            Err(_) if valid_draft.is_some() => {
                return store_score(valid_draft.as_ref().unwrap());
            }
            Err(CompletionFailure::Truncated) if attempt == 0 => {
                // The Brain returns no partial artifact on truncation, so keep
                // the original brief and regenerate rather than splice JSON.
                messages.push(Message::user(REGENERATE_COMPLETE_SCORE));
                continue;
            }
            Err(CompletionFailure::Truncated) => {
                return Err("The Organist reached its output limit without a complete score after two attempts; previous playback is retained.".to_owned());
            }
            Err(CompletionFailure::Other(error)) => return Err(error),
        };
        match validate_generated_score(&response) {
            Ok(score) if attempt == 0 => {
                messages.push(Message::assistant(response));
                messages.push(Message::user(format!(
                    "ONE MUSICAL REVIEW. Read the original brief again and compare every explicit requirement with the actual draft below. Honor the requested form, length, mode, instruments, note-event count, rests and expressive contrasts. First repair concrete register, duplicated-part or phrase-space concerns: out-of-range notes can lose their contour, different patches can double the same busy high line, and durations can fill every null slot. Preserve useful motif and ensemble identity; listener comfort and coherence take priority over novelty. If the draft already meets the brief and has no concrete problem, return it unchanged. Otherwise revise one specific weakness without adding arbitrary variation or new voices. The measured counts describe the draft, not the target: if the original brief requested four distinct sounding degrees and the draft has three, write a purposeful fourth sounding degree. Counts alone are not musical quality; a large number of different notes can merely be a scale exercise. Repeated notes, intentional doubling and equal durations are legitimate when they serve the brief. This is notation analysis, not evidence of listening or a beauty certificate. Return the full executable score using atomic notes events, including unchanged parts.\nACTUAL DRAFT THEORY ANALYSIS:\n{}\nMEASURED DRAFT NOTE FACTS:\n{}\n{ENCODING_GUIDE}",
                    score.theory_context(), draft_note_facts(&score)
                )));
                valid_draft = Some(score);
            }
            Ok(score) => return store_score(&score),
            Err(reason) if attempt == 0 => {
                let rejected_scalar = teledra_music::Score::from_json(&response)
                    .ok()
                    .filter(|score| !score.scalar_exercise_concerns().is_empty());
                if let Some(score) = rejected_scalar {
                    messages.push(Message::user(format!("The draft was a rejected scale exercise. Its note arrays are omitted to avoid repeating it.\n{}", score_context_for_prompt(&score))));
                } else {
                    messages.push(Message::assistant(response));
                }
                messages.push(Message::user(format!(
                    "ONE SCORE REPAIR REQUIRED. The draft could not be played. Fix every listed error while still honoring the original musical brief, requested notes/rests, expression and complete composition plan. Return the full corrected JSON score using atomic notes events: each degree travels with its own velocity and duration. Do not describe a correction or drop the requested expressive information.\nNATIVE VALIDATION ERRORS:\n{reason}\n{ENCODING_GUIDE}"
                )));
            }
            Err(_) if valid_draft.is_some() => {
                return store_score(valid_draft.as_ref().unwrap());
            }
            Err(reason) => {
                return Err(format!(
                    "The Organist's score still failed validation after one repair; previous playback is retained. {reason}"
                ));
            }
        }
    }
    unreachable!("the second bounded attempt always returns a revision, fallback or error")
}

fn store_score(score: &teledra_music::Score) -> Result<String, String> {
    serde_json::to_string(score)
        .map_err(|error| format!("The validated score could not be stored: {error}"))
}

/// Counts authored events that are capable of sounding, before orchestration.
/// Expression values attached only to rests or zero-velocity notes do not
/// establish audible variation in the motif.
fn draft_note_facts(score: &teledra_music::Score) -> String {
    use std::collections::BTreeSet;

    score.tracks.iter().enumerate().map(|(index, track)| {
        let sounded: Vec<_> = track.steps.iter().enumerate().filter_map(|(position, degree)| {
            let velocity = track.velocities.as_ref().map_or(1.0, |values| values[position]);
            (track.gain > 0.0 && velocity > 0.0).then_some((position, *degree))
                .and_then(|(position, degree)| degree.map(|degree| (position, degree)))
        }).collect();
        let degrees: BTreeSet<_> = sounded.iter().map(|(_, degree)| *degree).collect();
        let classes: BTreeSet<_> = degrees.iter().map(|degree| degree % 7).collect();
        let expression_count = |values: &Option<Vec<f32>>| values.as_ref().map_or_else(
            || "native defaults (not explicitly authored)".to_owned(),
            |values| sounded.iter().map(|(position, _)| values[*position].to_bits())
                .collect::<BTreeSet<_>>().len().to_string()
        );
        let rests = track.steps.iter().filter(|degree| degree.is_none()).count();
        format!(
            "Track {index} '{}' ({:?}): {} positions; {rests} null rests; {} sounding events; {} distinct sounding degrees {:?}; {} scale pitch classes; {} distinct sounded velocities; {} distinct sounded durations. Counts concern authored notation before native orchestration.",
            track.name, track.instrument(), track.steps.len(), sounded.len(), degrees.len(), degrees,
            classes.len(), expression_count(&track.velocities), expression_count(&track.durations)
        )
    }).collect::<Vec<_>>().join("\n")
}

pub(crate) fn validate_generated_score(raw: &str) -> Result<teledra_music::Score, String> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("The response must be one complete JSON score object: {error}"))?;
    let mut errors = Vec::new();
    for field in [
        "title",
        "bpm",
        "bars",
        "tonic",
        "mode",
        "progression",
        "tracks",
    ] {
        if value.get(field).is_none() {
            errors.push(format!("{field} is missing from the complete native score"));
        }
    }
    if let Some(plan) = value
        .get("composition")
        .and_then(serde_json::Value::as_object)
    {
        for field in [
            "form",
            "phrase_bars",
            "chord_bars",
            "development",
            "texture",
            "harmony",
        ] {
            if !plan.contains_key(field) {
                errors.push(format!(
                    "composition.{field} is missing from the new score plan"
                ));
            }
        }
    } else {
        errors.push("composition is missing: every newly generated piece needs the complete requested form, phrase_bars, chord_bars, development, texture and harmony plan".to_owned());
    }
    if let Some(tracks) = value.get("tracks").and_then(serde_json::Value::as_array) {
        for (index, track) in tracks.iter().enumerate() {
            let name = track
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unnamed");
            let Some(steps) = track.get("steps").and_then(serde_json::Value::as_array) else {
                continue;
            };
            for (field, low, high) in [("velocities", 0.0, 1.0), ("durations", 0.125, 8.0)] {
                let Some(values) = track.get(field) else {
                    continue;
                };
                let Some(values) = values.as_array() else {
                    errors.push(format!("tracks[{index}] '{name}': {field} must be a numeric array or omitted if not requested"));
                    continue;
                };
                if values.len() != steps.len() {
                    errors.push(format!(
                        "tracks[{index}] '{name}': steps has {} entries, but {field} has {}. {field} must contain exactly {} entries, including rest positions",
                        steps.len(), values.len(), steps.len()
                    ));
                }
                if let Some((position, _)) = values.iter().enumerate().find(|(_, value)| {
                    value
                        .as_f64()
                        .is_none_or(|number| !number.is_finite() || !(low..=high).contains(&number))
                }) {
                    errors.push(format!("tracks[{index}] '{name}': {field}[{position}] must be a finite number {low}..{high}"));
                }
            }
        }
    }
    // Score::from_json expands atomic events into the legacy storage arrays
    // losslessly and validates the resulting native score. No musical material
    // is invented, discarded, quantized or resized at this boundary.
    match teledra_music::Score::from_json(raw) {
        Ok(score) if errors.is_empty() => {
            let blockers = score.automatic_composition_blockers();
            if blockers.is_empty() {
                Ok(score)
            } else {
                Err(blockers.join("\n"))
            }
        }
        Ok(_) => Err(errors.join("\n")),
        Err(error) => {
            errors.push(format!("Native score parser: {error}"));
            Err(errors.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deadline_during_musical_review_retains_the_valid_complete_draft() {
        let draft = valid_score();
        let mut calls = 0;
        let result = compose_with_deadline(
            &[Message::user("Compose a complete study")],
            0.5,
            tokio::time::Instant::now() + std::time::Duration::from_millis(60),
            |_, _| {
                calls += 1;
                let draft = draft.clone();
                let first = calls == 1;
                async move {
                    if first {
                        Ok(draft)
                    } else {
                        std::future::pending().await
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            serde_json::to_value(teledra_music::Score::from_json(&result).unwrap()).unwrap(),
            serde_json::to_value(teledra_music::Score::from_json(&draft).unwrap()).unwrap()
        );
    }

    #[tokio::test]
    async fn incomplete_or_unstarted_compositions_cannot_outlive_the_request_deadline() {
        for first_response in [None, Some("{}".to_owned())] {
            let mut calls = 0;
            let error = compose_with_deadline(
                &[Message::user("Compose a complete study")],
                0.5,
                tokio::time::Instant::now() + std::time::Duration::from_millis(40),
                |_, _| {
                    calls += 1;
                    let response = if calls == 1 {
                        first_response.clone()
                    } else {
                        None
                    };
                    async move {
                        match response {
                            Some(response) => Ok(response),
                            None => std::future::pending().await,
                        }
                    }
                },
            )
            .await
            .unwrap_err();
            assert_eq!(calls, if first_response.is_some() { 2 } else { 1 });
            assert!(error.contains("total composition deadline"));
            assert!(error.contains("previous playback is retained"));
        }
        let mut calls = 0;
        let result = compose_with_deadline(
            &[Message::user("Compose a complete study")],
            0.5,
            tokio::time::Instant::now(),
            |_, _| {
                calls += 1;
                std::future::ready(Ok(valid_score()))
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls, 0, "do not send a request after its deadline");
    }

    fn valid_score() -> String {
        let mut score = serde_json::to_value(teledra_music::default_score()).unwrap();
        score["composition"] = serde_json::json!({
            "form": "ternary", "phrase_bars": 4, "chord_bars": 2,
            "development": 0.25, "texture": "chamber", "harmony": "diatonic"
        });
        serde_json::to_string(&score).unwrap()
    }

    fn duplicated_out_of_range_melody() -> String {
        let mut score: serde_json::Value = serde_json::from_str(&valid_score()).unwrap();
        let mut left = score["tracks"][0].clone();
        left["instrument"] = "celesta".into();
        left["octave"] = 5.into();
        left["steps"] = serde_json::json!([
            7, null, 9, null, 11, null, 13, null, 15, null, 17, null, 19, null, 20, null
        ]);
        left.as_object_mut().unwrap().remove("velocities");
        left["durations"] = serde_json::json!(vec![0.5; 16]);
        let mut right = left.clone();
        right["instrument"] = "arp".into();
        right["name"] = "Another patch on the same line".into();
        right["pan"] = 0.8.into();
        score["tracks"] = serde_json::json!([left, right]);
        score["tonic"] = 7.into();
        score["composition"]["development"] = 0.0.into();
        score["composition"]["harmony"] = "chromatic".into();
        score.to_string()
    }

    #[test]
    fn rejected_scale_context_keeps_settings_but_omits_its_note_arrays() {
        let raw = duplicated_out_of_range_melody();
        let score = teledra_music::Score::from_json(&raw).unwrap();
        assert!(!score.scalar_exercise_concerns().is_empty());
        let context = score_context_for_prompt(&score);
        assert!(context.contains("PRESERVE, unless the operator requests otherwise"));
        assert!(context.contains(&format!("{} BPM; tonic=7", score.bpm)));
        assert!(context.contains(&serde_json::to_string(&score.progression).unwrap()));
        assert!(context.contains("celesta: sounding MIDI"));
        assert!(context.contains("arp: sounding MIDI"));
        assert!(context.contains("answering phrase a different onset rhythm"));
        assert!(!context.contains("\"steps\""));
        assert!(!context.contains("\"durations\""));
        assert!(!context.contains(teledra_music::THEORY_GUIDE));
        assert!(context.len() < score.theory_context().len());
    }

    #[test]
    fn normal_score_context_retains_notes_and_complete_palette_knowledge() {
        let score = teledra_music::Score::from_json(&valid_score()).unwrap();
        assert!(score.scalar_exercise_concerns().is_empty());
        let context = score_context_for_prompt(&score);
        assert!(context.contains(&serde_json::to_string(&score).unwrap()));
        assert!(context.contains(teledra_music::THEORY_GUIDE));
        assert!(context.contains("theremin"));
        assert!(context.contains("piano"));
    }

    #[tokio::test]
    async fn scalar_repair_preserves_operator_request_without_replaying_rejected_notes() {
        let bad = duplicated_out_of_range_melody();
        let request = Message::user("Keep piano and theremin, C minor, 88 BPM and eight bars.");
        let mut calls = 0;
        compose_with(std::slice::from_ref(&request), 0.5, |messages, _| {
            calls += 1;
            if calls == 2 {
                assert!(messages.contains(&request));
                assert!(!messages.contains(&Message::assistant(bad.clone())));
                assert!(
                    messages
                        .iter()
                        .all(|message| !message.content.contains("\"steps\""))
                );
                assert!(
                    messages
                        .iter()
                        .any(|message| message.content.contains("CURRENT SCORE REPAIR BRIEF"))
                );
                assert!(
                    messages
                        .iter()
                        .any(|message| message.content.contains("SCALAR EXERCISE:"))
                );
                assert!(messages.iter().any(|message| {
                    message
                        .content
                        .contains("4/min(event_count, steps_per_bar)")
                }));
            }
            std::future::ready(Ok(if calls == 1 {
                bad.clone()
            } else {
                valid_score()
            }))
        })
        .await
        .unwrap();
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn duplicated_out_of_range_draft_gets_one_specific_repair_before_acceptance() {
        let bad = duplicated_out_of_range_melody();
        assert!(
            teledra_music::Score::from_json(&bad).is_ok(),
            "saved literal scores remain loadable"
        );
        assert!(
            validate_generated_score(&bad)
                .unwrap_err()
                .contains("entire pitched ensemble")
        );
        let mut calls = 0;
        let response =
            compose_with(
                &[Message::user("Compose a coherent miniature")],
                0.5,
                |messages, _| {
                    calls += 1;
                    if calls == 2 {
                        assert!(
                            messages
                                .last()
                                .unwrap()
                                .content
                                .contains("out-of-range melody")
                        );
                        assert!(
                            messages.last().unwrap().content.contains(
                                "changing names, pan or instruments alone does not repair"
                            )
                        );
                    }
                    std::future::ready(Ok(if calls == 1 {
                        bad.clone()
                    } else {
                        valid_score()
                    }))
                },
            )
            .await
            .unwrap();
        assert_eq!(calls, 2);
        assert!(validate_generated_score(&response).is_ok());
    }

    #[tokio::test]
    async fn unplayable_attack_density_uses_one_repair_with_precise_grid_evidence() {
        let mut dense: serde_json::Value = serde_json::from_str(&valid_score()).unwrap();
        dense["tracks"][0]["instrument"] = "cello".into();
        dense["tracks"][0]["octave"] = 2.into();
        dense["tracks"][0]["steps"] = serde_json::json!(vec![Some(0); 16]);
        dense["tracks"][0]["durations"] = serde_json::json!(vec![0.5; 16]);
        let error = validate_generated_score(&dense.to_string()).unwrap_err();
        assert!(error.contains("0.25 beats apart"));
        assert!(error.contains("at least 1.00 beats"));
        let mut spaced = dense.clone();
        spaced["tracks"][0]["steps"] = serde_json::json!(vec![Some(0); 4]);
        spaced["tracks"][0]["durations"] = serde_json::json!(vec![0.5; 4]);
        assert!(validate_generated_score(&spaced.to_string()).is_ok());
        let mut calls = 0;
        let response =
            compose_with(
                &[Message::user("Compose a sparse cello accompaniment")],
                0.5,
                |messages, _| {
                    calls += 1;
                    assert!(messages.iter().any(|message| message.content.contains(
                        "Two adjacent notes in a 16-slot cell with duration 0.5 overlap"
                    )));
                    if calls == 2 {
                        assert!(
                            messages
                                .last()
                                .unwrap()
                                .content
                                .contains("0.25 beats apart")
                        );
                        assert!(
                            messages
                                .last()
                                .unwrap()
                                .content
                                .contains("NEVER delays the next grid slot")
                        );
                    }
                    std::future::ready(Ok(if calls == 1 {
                        dense.to_string()
                    } else {
                        spaced.to_string()
                    }))
                },
            )
            .await
            .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            teledra_music::Score::from_json(&response).unwrap().tracks[0]
                .steps
                .len(),
            4
        );
    }

    #[tokio::test]
    async fn a_review_that_introduces_duplicated_out_of_range_lines_retains_the_valid_draft() {
        let mut calls = 0;
        let response = compose_with(
            &[Message::user("Compose a coherent miniature")],
            0.5,
            |messages, _| {
                calls += 1;
                if calls == 2 {
                    assert!(
                        messages
                            .last()
                            .unwrap()
                            .content
                            .contains("return it unchanged")
                    );
                    assert!(
                        messages
                            .last()
                            .unwrap()
                            .content
                            .contains("durations can fill every null slot")
                    );
                }
                std::future::ready(Ok(if calls == 1 {
                    valid_score()
                } else {
                    duplicated_out_of_range_melody()
                }))
            },
        )
        .await
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            serde_json::to_value(teledra_music::Score::from_json(&response).unwrap()).unwrap(),
            serde_json::to_value(teledra_music::Score::from_json(&valid_score()).unwrap()).unwrap()
        );
    }

    #[test]
    fn all_expression_counts_and_missing_plan_are_reported_together() {
        let mut score: serde_json::Value = serde_json::from_str(&valid_score()).unwrap();
        score.as_object_mut().unwrap().remove("composition");
        score["tracks"][0]["name"] = "Cello".into();
        score["tracks"][0]["steps"] = serde_json::json!([0, null, 2, null]);
        score["tracks"][0]["velocities"] = serde_json::json!([0.7, 0.1, 0.6]);
        score["tracks"][0]["durations"] = serde_json::json!([1.0, 0.25, 1.0, 0.25, 1.0]);
        score["tracks"][1]["name"] = "Flute".into();
        score["tracks"][1]["steps"] = serde_json::json!([0, null]);
        score["tracks"][1]["velocities"] = serde_json::json!([0.6]);
        let errors = validate_generated_score(&score.to_string()).unwrap_err();
        assert!(errors.contains("composition is missing"));
        assert!(errors.contains("'Cello': steps has 4 entries, but velocities has 3"));
        assert!(errors.contains("'Cello': steps has 4 entries, but durations has 5"));
        assert!(errors.contains("'Flute': steps has 2 entries, but velocities has 1"));
    }

    #[test]
    fn partial_new_scores_do_not_inherit_legacy_harmony_or_default_parts() {
        let mut missing: serde_json::Value = serde_json::from_str(&valid_score()).unwrap();
        missing.as_object_mut().unwrap().remove("progression");
        assert!(
            validate_generated_score(&missing.to_string())
                .unwrap_err()
                .contains("progression is missing")
        );
        let legacy = serde_json::json!({
            "title": "Unfinished draft",
            "harmony": {"tonic": "C", "mode": "minor"},
            "motif": ["C", "Eb", "G", "C"],
            "composition": missing["composition"]
        });
        // The general loader remains compatible with saved legacy material;
        // the production composer must author every part of a native score.
        assert!(teledra_music::Score::from_json(&legacy.to_string()).is_ok());
        let errors = validate_generated_score(&legacy.to_string()).unwrap_err();
        assert!(errors.contains("tracks is missing"));
        assert!(errors.contains("bpm is missing"));
    }

    #[tokio::test]
    async fn one_repair_preserves_the_original_request_and_invalid_draft() {
        let valid = valid_score();
        let original = vec![Message::user("Compose a 16-bar chamber study with rests.")];
        let mut calls = Vec::new();
        let response = compose_with(&original, 0.6, |messages, temperature| {
            calls.push((messages, temperature));
            std::future::ready(Ok(if calls.len() == 1 {
                "{}".to_owned()
            } else {
                valid.clone()
            }))
        })
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(teledra_music::Score::from_json(&response).unwrap()).unwrap(),
            serde_json::to_value(teledra_music::Score::from_json(&valid).unwrap()).unwrap(),
            "no generated notes were rewritten locally"
        );
        assert_eq!(calls.len(), 2);
        assert!(
            calls[0]
                .0
                .iter()
                .any(|message| message.content.contains("tonic already transposes"))
        );
        assert!(calls[1].0.contains(&original[0]));
        assert!(calls[1].0.contains(&Message::assistant("{}")));
        assert!(
            calls[1]
                .0
                .last()
                .unwrap()
                .content
                .contains("composition is missing")
        );
        assert_eq!(calls[1].1, 0.2);
    }

    #[tokio::test]
    async fn atomic_note_events_reach_storage_without_losing_expression_or_rests() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_score()).unwrap();
        let track = value["tracks"][0].as_object_mut().unwrap();
        track.remove("steps");
        track.remove("velocities");
        track.remove("durations");
        track.insert(
            "notes".to_owned(),
            serde_json::json!([
                {"degree": 0, "velocity": 0.72, "duration": 0.75},
                {"degree": null, "velocity": 0.0, "duration": 0.25},
                {"degree": 3, "velocity": 0.41, "duration": 1.5},
                {"degree": 2, "velocity": 0.68, "duration": 0.5}
            ]),
        );
        let mut calls = 0;
        let response = compose_with(
            &[Message::user("Compose a chamber piece")],
            0.5,
            |messages, _| {
                calls += 1;
                assert!(
                    messages
                        .last()
                        .unwrap()
                        .content
                        .contains("one atomic event")
                );
                std::future::ready(Ok(value.to_string()))
            },
        )
        .await
        .unwrap();
        assert_eq!(calls, 2, "a valid draft receives one musical review");
        let score = teledra_music::Score::from_json(&response).unwrap();
        assert_eq!(score.tracks[0].steps, vec![Some(0), None, Some(3), Some(2)]);
        assert_eq!(
            score.tracks[0].velocities.as_ref().unwrap(),
            &[0.72, 0.0, 0.41, 0.68]
        );
        assert_eq!(
            score.tracks[0].durations.as_ref().unwrap(),
            &[0.75, 0.25, 1.5, 0.5]
        );
        let canonical: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(canonical["tracks"][0].get("notes").is_none());
    }

    #[tokio::test]
    async fn a_second_invalid_draft_stops_without_a_third_inference() {
        let mut count = 0;
        let result = compose_with(&[Message::user("Compose a piece")], 0.5, |_, _| {
            count += 1;
            std::future::ready(Ok("{}".to_owned()))
        })
        .await;
        assert_eq!(count, 2);
        assert!(result.unwrap_err().contains("after one repair"));
    }

    #[tokio::test]
    async fn musical_review_uses_measured_draft_facts_and_the_original_brief() {
        let mut draft: serde_json::Value = serde_json::from_str(&valid_score()).unwrap();
        draft["tracks"][0]["name"] = "Measured Flute".into();
        draft["tracks"][0]["instrument"] = "flute".into();
        draft["tracks"][0]["steps"] = serde_json::json!([0, 2, 2, null, 4, 6, 5, 7]);
        draft["tracks"][0]["velocities"] =
            serde_json::json!([0.6, 0.6, 0.7, 0.2, 0.5, 0.0, 0.5, 0.5]);
        draft["tracks"][0]["durations"] =
            serde_json::json!([0.5, 0.5, 0.75, 8.0, 0.5, 4.0, 0.75, 0.5]);
        let mut revision = draft.clone();
        revision["tracks"][0]["steps"][2] = 3.into();
        let original = Message::user("Compose a flute answer with six distinct sounding degrees.");
        let theory = teledra_music::Score::from_json(&draft.to_string())
            .unwrap()
            .theory_context();
        let mut calls = 0;
        let response = compose_with(std::slice::from_ref(&original), 0.5, |messages, _| {
            calls += 1;
            if calls == 2 {
                assert!(messages.contains(&original));
                assert!(messages.contains(&Message::assistant(draft.to_string())));
                let review = &messages.last().unwrap().content;
                assert!(review.contains(&theory));
                assert!(review.contains("8 positions; 1 null rests; 6 sounding events"));
                assert!(review.contains("5 distinct sounding degrees {0, 2, 4, 5, 7}"));
                assert!(review.contains("3 distinct sounded velocities"));
                assert!(review.contains("2 distinct sounded durations"));
            }
            std::future::ready(Ok(if calls == 1 {
                draft.to_string()
            } else {
                revision.to_string()
            }))
        })
        .await
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            teledra_music::Score::from_json(&response).unwrap().tracks[0].steps[2],
            Some(3)
        );
    }

    #[tokio::test]
    async fn a_truncated_first_draft_regenerates_once_without_shortening_required_patterns() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_score()).unwrap();
        value["bars"] = 16.into();
        value["tracks"][0]["instrument"] = "flute".into();
        value["tracks"][0]["steps"] = serde_json::json!(
            (0..64)
                .map(|index| (index % 4 == 0).then_some(index % 7))
                .collect::<Vec<_>>()
        );
        value["tracks"][0]
            .as_object_mut()
            .unwrap()
            .remove("velocities");
        value["tracks"][0]
            .as_object_mut()
            .unwrap()
            .remove("durations");
        let complete_score = value.to_string();
        let original = Message::user(
            "Compose a 16-bar piece with an exactly 64-event flute melody and real rests.",
        );
        let mut calls = Vec::new();
        let response = compose_with(
            std::slice::from_ref(&original),
            0.6,
            |messages, temperature| {
                calls.push((messages, temperature));
                std::future::ready(if calls.len() == 1 {
                    Err(CompletionFailure::Truncated)
                } else {
                    Ok(complete_score.clone())
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(
            calls.len(),
            2,
            "regeneration consumes the final call without another review"
        );
        assert_eq!(calls[1].1, 0.2);
        assert!(calls[1].0.contains(&original));
        assert!(
            calls[1]
                .0
                .iter()
                .all(|message| message.role != teledra_brain::Role::Assistant),
            "no nonexistent partial JSON is replayed or spliced"
        );
        let retry = &calls[1].0.last().unwrap().content;
        assert!(retry.contains("When the brief leaves pattern lengths open"));
        assert!(retry.contains("4, 8 or 16-event cells"));
        assert!(retry.contains("Preserve explicitly requested longer note-event counts"));
        let accepted = teledra_music::Score::from_json(&response).unwrap();
        assert_eq!(accepted.bars, 16);
        assert_eq!(accepted.tracks[0].steps.len(), 64);
        assert_eq!(
            serde_json::to_value(accepted).unwrap(),
            serde_json::to_value(teledra_music::Score::from_json(&complete_score).unwrap())
                .unwrap()
        );
    }

    #[test]
    fn only_the_actual_brain_truncation_signal_maps_to_regeneration() {
        assert_eq!(
            CompletionFailure::from(BrainError::TruncatedResponse),
            CompletionFailure::Truncated
        );
        assert!(matches!(
            CompletionFailure::from(BrainError::ResponseTooLarge),
            CompletionFailure::Other(_)
        ));
        assert!(matches!(
            CompletionFailure::from(BrainError::ModelMissing("test-model")),
            CompletionFailure::Other(_)
        ));
    }

    #[tokio::test]
    async fn ordinary_initial_inference_errors_are_not_retried() {
        for message in [
            "model unavailable",
            "Qwen reached its output limit before completing the response",
        ] {
            let mut calls = 0;
            let error = compose_with(&[Message::user("Compose a piece")], 0.5, |_, _| {
                calls += 1;
                std::future::ready(Err(CompletionFailure::Other(message.to_owned())))
            })
            .await
            .unwrap_err();
            assert_eq!(
                calls, 1,
                "retry eligibility comes from the typed signal, never error text"
            );
            assert_eq!(error, message);
        }
    }

    #[tokio::test]
    async fn failed_regeneration_stops_after_two_inferences_without_accepting_fragments() {
        for final_attempt in [
            Err(CompletionFailure::Truncated),
            Ok("{\"title\":\"unfinished\"".to_owned()),
            Err(CompletionFailure::Other("model unavailable".to_owned())),
        ] {
            let mut calls = 0;
            let result = compose_with(&[Message::user("Compose a piece")], 0.5, |_, _| {
                calls += 1;
                std::future::ready(if calls == 1 {
                    Err(CompletionFailure::Truncated)
                } else {
                    final_attempt.clone()
                })
            })
            .await;
            assert_eq!(calls, 2);
            assert!(result.is_err());
            if final_attempt == Err(CompletionFailure::Truncated) {
                assert!(
                    result
                        .unwrap_err()
                        .contains("previous playback is retained")
                );
            }
        }
    }

    #[tokio::test]
    async fn failed_musical_review_retains_the_valid_draft_within_two_calls() {
        for failed_review in [
            Ok("{}".to_owned()),
            Err(CompletionFailure::Other("model unavailable".to_owned())),
            Err(CompletionFailure::Truncated),
        ] {
            let draft = valid_score();
            let mut calls = 0;
            let result = compose_with(&[Message::user("Compose a piece")], 0.5, |_, _| {
                calls += 1;
                std::future::ready(if calls == 1 {
                    Ok(draft.clone())
                } else {
                    failed_review.clone()
                })
            })
            .await
            .unwrap();
            assert_eq!(calls, 2);
            assert_eq!(
                serde_json::to_value(teledra_music::Score::from_json(&result).unwrap()).unwrap(),
                serde_json::to_value(teledra_music::Score::from_json(&draft).unwrap()).unwrap()
            );
        }
    }
}
