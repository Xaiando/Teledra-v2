//! Opt-in production composer proof for the expanded instrument palette.
use teledra_agents::AgentId;
use teledra_court::Court;

#[path = "../src/composition.rs"]
mod composition;

#[tokio::test]
#[ignore = "requires the installed Ollama model; writes isolated audio without starting playback"]
async fn live_organist_uses_new_instruments_in_a_complete_study() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/music-quality/palette-72-model-study");
    std::fs::create_dir_all(&output).unwrap();
    let current = teledra_music::default_score();
    let context = format!(
        "{}\nCURRENT SCORE:\n{}\n{}",
        teledra_music::THEORY_GUIDE,
        serde_json::to_string(&current).unwrap(),
        current.theory_context()
    );
    let brief = "Compose a complete original eight-bar chamber miniature at 96 BPM in C major (tonic=0). Use exactly three complementary instruments from the supplied catalog: theremin, piano and marimba. Give theremin a spacious singing melody in octave 5, piano a quiet answering line in octave 3 and marimba restrained tonal punctuation in octave 4. Use a binary composition plan with phrase_bars=2, chord_bars=1, development=0.25, texture=chamber and harmony=diatonic. Use a clear tonic opening and return, with only compatible diatonic chord qualities. Author atomic notes events, including numeric velocity and duration on rests. Choose concise complete 4, 8 or 16 event patterns as each musical role needs, with few well-spaced notes for supporting voices. Write at least three different sounding degrees in the theremin line and actual null rests. Keep registers apart; avoid sustained clashes or busy simultaneous attacks. Return every score field and all three complete tracks using the supplied schema. Do not reuse the profile example's notes or title.";
    let prepared =
        Court::default().prepare_turn_for_with_context(AgentId::Organist, brief, &context);
    let brain = teledra_brain::Brain::new(Default::default()).unwrap();
    let response = composition::compose_score(&brain, &prepared.messages, 0.6)
        .await
        .unwrap();
    std::fs::write(output.join("organist-response.json"), &response).unwrap();
    let score = teledra_music::Score::from_json(&response)
        .unwrap()
        .mastered()
        .unwrap();
    assert_eq!((score.bars, score.bpm, score.tonic), (8, 96, 0));
    assert_eq!(score.tracks.len(), 3);
    for instrument in [
        teledra_music::Instrument::Theremin,
        teledra_music::Instrument::Piano,
        teledra_music::Instrument::Marimba,
    ] {
        let track = score
            .tracks
            .iter()
            .find(|track| track.instrument() == instrument)
            .expect("the composer must actually choose the requested new voice");
        assert!(matches!(track.steps.len(), 4 | 8 | 16));
        assert!(track.active_note_count() > 0);
        assert!(track.gain > 0.0);
        if instrument == teledra_music::Instrument::Theremin {
            let degrees: std::collections::BTreeSet<_> = track
                .steps
                .iter()
                .enumerate()
                .filter(|(position, _)| {
                    track
                        .velocities
                        .as_ref()
                        .is_none_or(|values| values[*position] > 0.0)
                })
                .filter_map(|(_, degree)| *degree)
                .collect();
            assert!(
                degrees.len() >= 3,
                "the theremin needs an audible melodic contour"
            );
            assert!(
                track.steps.iter().any(Option::is_none),
                "the melody needs real rests"
            );
        }
    }
    let audio = teledra_music::VerifiedAudio::new(score.render().unwrap()).unwrap();
    assert!((audio.report().duration_seconds - 20.0).abs() < 0.001);
    audio
        .write_wav(&output.join("new-instrument-study.wav"))
        .unwrap();
    std::fs::write(
        output.join("accepted-score.json"),
        serde_json::to_string_pretty(&score).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("audio-verification.json"),
        serde_json::to_string_pretty(audio.report()).unwrap(),
    )
    .unwrap();
    println!(
        "Verified {}: {:.1}s, {}",
        score.title,
        audio.report().duration_seconds,
        output.display()
    );
}
