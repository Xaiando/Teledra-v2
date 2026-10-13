//! Opt-in isolated proof against the captured, listener-reported regression.
use teledra_agents::AgentId;
use teledra_court::Court;

#[path = "../src/composition.rs"]
mod composition;

#[tokio::test]
#[ignore = "requires installed Ollama and captured rejected-score.json; never starts playback"]
async fn live_organist_replaces_the_reported_scale_with_authored_phrases() {
    let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let original = teledra_music::Score::from_json(
        &std::fs::read_to_string(workspace.join("target/phrase-composition/rejected-score.json"))
            .unwrap(),
    )
    .unwrap();
    let output = workspace.join("target/phrase-composition/model-study");
    std::fs::create_dir_all(&output).unwrap();
    assert!(!original.scalar_exercise_concerns().is_empty());
    let context = format!(
        "{}\nNAMED OPERATOR FEEDBACK: This is merely repeating a scale. It needs real composition.",
        composition::score_context_for_prompt(&original)
    );
    let brief = "Replace the rejected scale with a restrained eight-bar miniature at 96 BPM in C dorian (tonic 0). Choose two suitable voices with separate jobs: a recognizable melody and quieter, slower harmonic support. Use steps_per_bar=4 and write 32 atomic events for the melody, covering the actual eight bars. Bars 1–2 establish a short motif with a resting point; bars 3–4 answer it with a different onset rhythm and a settled destination; bars 5–6 contrast; bars 7–8 return and finish. Give support a concise 4- or 8-event repeating pattern. Use form arc, development 0, chamber texture and diatonic harmony so the authored timeline remains clear. Keep pitches in range, gains modest, and leave an audible phrase-ending release gap. Retain useful harmonic identity, but choose new notes instead of copying the rejected scale. Return the complete score through the schema.";
    std::fs::write(output.join("brief.txt"), brief).unwrap();
    std::fs::write(output.join("composer-context.txt"), &context).unwrap();
    let prepared =
        Court::default().prepare_turn_for_with_context(AgentId::Organist, brief, &context);
    let brain = teledra_brain::Brain::new(Default::default()).unwrap();
    // Exactly one production request; its existing budget permits at most two inferences.
    let response = match composition::compose_score(&brain, &prepared.messages, 0.35).await {
        Ok(response) => response,
        Err(error) => {
            std::fs::write(output.join("generation-error.txt"), &error).unwrap();
            panic!("bounded repair request failed: {error}");
        }
    };
    std::fs::write(output.join("organist-response.json"), &response).unwrap();
    let score = teledra_music::Score::from_json(&response)
        .unwrap()
        .mastered()
        .unwrap();
    let report = score.composition_report();
    let concerns = score.orchestration_concerns();
    std::fs::write(
        output.join("accepted-score.json"),
        serde_json::to_string_pretty(&score).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("score-checks.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("remaining-concerns.json"),
        serde_json::to_string_pretty(&concerns).unwrap(),
    )
    .unwrap();
    let audio = teledra_music::VerifiedAudio::new(score.render().unwrap()).unwrap();
    audio.write_wav(&output.join("repaired-study.wav")).unwrap();
    std::fs::write(
        output.join("audio-verification.json"),
        serde_json::to_string_pretty(audio.report()).unwrap(),
    )
    .unwrap();
    println!(
        "{}: {} bars, {}BPM, voices {}; register_fit={}, independent_parts={}, breathing_room={}, verified {:.1}s; concerns {:?}",
        score.title,
        score.bars,
        score.bpm,
        score
            .tracks
            .iter()
            .map(|track| track.instrument().id())
            .collect::<Vec<_>>()
            .join(", "),
        report.register_fit,
        report.independent_parts,
        report.breathing_room,
        audio.report().duration_seconds,
        concerns
    );
    assert_eq!((score.bars, score.bpm, score.tonic), (8, 96, 0));
    assert!(matches!(score.mode, teledra_music::Mode::Dorian));
    assert!(
        report.register_fit,
        "the reported range correction problem remains"
    );
    assert!(
        report.independent_parts,
        "the whole ensemble still duplicates the same line"
    );
    assert!(
        report.breathing_room,
        "the corrected notation still lacks a real release gap"
    );
    assert!(
        !concerns
            .iter()
            .any(|item| item.starts_with("PHRASE SPACE:")),
        "a foreground part still has no written release gap"
    );
    assert!(score.scalar_exercise_concerns().is_empty());
    let plan = score.composition.as_ref().unwrap();
    assert_eq!(plan.steps_per_bar, 4);
    let melody = score
        .tracks
        .iter()
        .max_by_key(|track| track.steps.len())
        .unwrap();
    assert_eq!(
        melody.steps.len(),
        32,
        "the requested melody must span all eight bars"
    );
    let onsets = |phrase: &[Option<u8>]| phrase.iter().map(Option::is_some).collect::<Vec<_>>();
    assert_ne!(
        onsets(&melody.steps[..8]),
        onsets(&melody.steps[8..16]),
        "the answering phrase must have its own onset rhythm"
    );
}
