//! The Organist's restored Python contract must survive prompt and tag handling.
use teledra_agents::AgentId;
use teledra_court::{Court, CourtAction, parse_response};

#[test]
fn python_music_is_an_executable_option_in_the_actual_organist_prompt() {
    let prepared = Court::default().prepare_turn_for_with_context(
        AgentId::Organist,
        "Compose a layered Python piece with piano and theremin.",
        "Selected engine: Python. Automatic alternation: on. Both engines share all 72 instruments.",
    );
    let prompt = prepared
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for instruction in [
        "native Court Synth",
        "[PYTHON_MUSIC: code]",
        "instrument_note(note, duration",
        "mix_waves(a, b, start_time=seconds",
        "play_sound(full_track, sr=44100, loop=True)",
        "one active music player",
        "Selected engine: Python",
        "all 72",
    ] {
        assert!(prompt.contains(instruction), "missing {instruction}");
    }
    assert!(!AgentId::Organist.profile().contains("await approval"));
}

#[test]
fn complete_python_source_with_arrays_and_unicode_survives_action_parsing() {
    let code = "import numpy as np\nfrom teledra_synth import instrument_note, mix_waves, play_sound\n# An answering phrase — after a rest\nfull_track = np.zeros(44100 * 8)\nfor index, note in enumerate(['C4', 'Eb4', 'G4']):\n    full_track = mix_waves(full_track, instrument_note(note, 0.5, instrument='piano'), start_time=index * 1.5)\nplay_sound(full_track, sr=44100, loop=True)";
    let parsed = parse_response(&format!("[PYTHON_MUSIC: {code}]"));
    let [CourtAction::CreativeArtifact { kind, content }] = parsed.actions.as_slice() else {
        panic!("Python must be retained as one executable artifact");
    };
    assert_eq!(kind, "python_music");
    assert_eq!(content, code);
    assert!(parsed.visible.is_empty());
}
