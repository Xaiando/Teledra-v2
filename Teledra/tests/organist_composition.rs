//! An isolated, opt-in Organist → theory plan → verified audio acceptance check.
use teledra_agents::AgentId;
use teledra_court::Court;

#[path = "../src/composition.rs"]
mod composition;

#[tokio::test]
#[ignore = "requires the installed Ollama model; writes isolated proof artifacts and never starts playback"]
async fn live_organist_composes_a_theory_guided_original_study() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/organist-overhaul/model-study");
    std::fs::create_dir_all(&output).unwrap();
    let current = teledra_music::default_score();
    let context = format!(
        "MUSIC THEORY GUIDE:\n{}\nCURRENT ACCEPTED SCORE:\n{}\nCURRENT SCORE ANALYSIS:\n{}\nThe analysis describes notation, not subjective listening. This is an isolated original composition experiment.",
        teledra_music::THEORY_GUIDE,
        serde_json::to_string(&current).unwrap(),
        current.theory_context()
    );
    let prompt = "COMPOSER NUDGE: Compose an original 16-bar chamber study at 104 BPM in A harmonic_minor (tonic=9), using exactly three tracks: cello, organ and flute. Include a complete composition plan with ternary form, phrase_bars=4, chord_bars=2, development=0.25, texture=chamber, harmony=diatonic. Use a deliberate harmonic-minor cadence with a dominant_seventh chord on root_degree=4 resolving to minor on root_degree=0. Each track must use the atomic notes format. The flute needs a two- or four-bar motif of 32 or 64 note-event objects forming an original idea and answer, with at least four different scale degrees and degree:null rests for breath. Attach intentional velocity and duration to each note event. Shape the phrase through dynamics, rhythmic rests or varied note lengths; a repeated duration is legitimate when the rhythm calls for it. Include valid numeric expression fields in rest events. Degree0 already means A because tonic=9; do not add9 to melodic degrees. Give the cello an independent four-event rhythm and the organ four slow harmonic events. Use register separation and a gentle loop return. This is a new study: do not copy the current score or the profile example. Return the complete executable score object through the supplied schema; no prose.";
    let prepared =
        Court::default().prepare_turn_for_with_context(AgentId::Organist, prompt, &context);
    let brain = teledra_brain::Brain::new(Default::default()).unwrap();
    let response = composition::compose_score(&brain, &prepared.messages, 0.65)
        .await
        .unwrap();
    std::fs::write(output.join("organist-response.json"), &response).unwrap();
    let score = teledra_music::Score::from_json(&response)
        .expect("the actual Organist response must be an executable score");
    assert_eq!(score.bars, 16);
    assert_eq!(score.bpm, 104);
    assert_eq!(score.tonic, 9);
    let json = serde_json::to_value(&score).unwrap();
    assert_eq!(json["mode"], "harmonic_minor");
    assert_eq!(json["composition"]["form"], "ternary");
    assert_eq!(json["composition"]["texture"], "chamber");
    assert_eq!(score.tracks.len(), 3);
    assert!(score.progression.iter().any(|chord| {
        chord.root_degree == 4 && serde_json::to_value(chord.quality).unwrap() == "dominant_seventh"
    }));
    let flute = json["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["instrument"] == "flute")
        .expect("the requested melody instrument must exist");
    let steps = flute["steps"].as_array().unwrap();
    assert!(
        matches!(steps.len(), 32 | 64),
        "the melody must span a complete two- or four-bar pattern"
    );
    assert!(steps.iter().any(serde_json::Value::is_null));
    let degrees: std::collections::HashSet<_> =
        steps.iter().filter_map(serde_json::Value::as_u64).collect();
    assert!(
        degrees.len() >= 4,
        "the study needs an actual melodic contour"
    );
    let mut expressed = false;
    for field in ["velocities", "durations"] {
        let values = flute[field].as_array().expect("authored articulation");
        assert_eq!(values.len(), steps.len());
        let sounded: Vec<_> = values
            .iter()
            .zip(steps)
            .filter_map(|(value, degree)| (!degree.is_null()).then_some(value))
            .collect();
        expressed |= sounded.windows(2).any(|pair| pair[0] != pair[1]);
    }
    assert!(
        expressed,
        "at least one performance dimension must shape the sounded phrase"
    );
    assert_ne!(
        serde_json::to_value(&score.tracks).unwrap(),
        serde_json::to_value(&current.tracks).unwrap()
    );
    let mastered = score
        .mastered()
        .expect("planned chamber music must survive mastering");
    assert_eq!(mastered.bars, 16);
    assert_eq!(mastered.tracks.len(), 3);
    std::fs::write(
        output.join("accepted-score.json"),
        serde_json::to_string_pretty(&mastered).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("theory-analysis.txt"),
        mastered.theory_context(),
    )
    .unwrap();
    let audio = teledra_music::VerifiedAudio::new(mastered.render().unwrap())
        .expect("the original study must pass the production audio checks");
    audio.write_wav(&output.join("organist-study.wav")).unwrap();
    std::fs::write(
        output.join("audio-verification.json"),
        serde_json::to_string_pretty(audio.report()).unwrap(),
    )
    .unwrap();
    println!(
        "{}\n{}\n{}",
        mastered.title,
        mastered.theory_context(),
        serde_json::to_string_pretty(audio.report()).unwrap()
    );
}
