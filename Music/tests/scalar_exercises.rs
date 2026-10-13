use teledra_music::{CompositionPlan, Instrument, Score, Texture, default_score};

fn study(degrees: &[u8]) -> Score {
    let mut score = default_score();
    score.bars = 16;
    score.tonic = 0;
    score.composition = Some(CompositionPlan {
        development: 0.0,
        texture: Texture::Chamber,
        ..Default::default()
    });
    score.tracks.truncate(1);
    let track = &mut score.tracks[0];
    track.name = "Piano melody".to_owned();
    track.instrument = Some(Instrument::Piano);
    track.octave = 3;
    track.gain = 0.4;
    track.steps = degrees.iter().copied().map(Some).collect();
    track.velocities = Some(vec![0.7; degrees.len()]);
    track.durations = Some(vec![0.5; degrees.len()]);
    score
}

#[test]
fn complete_uniform_scales_are_rejected_even_when_they_are_solo_and_in_range() {
    for degrees in [
        (0..8).collect::<Vec<_>>(),
        (0..8).rev().collect(),
        (0..=14).step_by(2).collect(),
    ] {
        let score = study(&degrees);
        assert!(score.composition_report().register_fit);
        let concerns = score.scalar_exercise_concerns();
        assert!(!concerns.is_empty(), "{degrees:?}");
        assert!(
            score
                .automatic_composition_blockers()
                .iter()
                .any(|text| text.contains("SCALAR EXERCISE"))
        );
        assert!(score.theory_context().contains("short identifiable motif"));
    }
}

#[test]
fn the_reported_wide_walk_remains_an_exercise_after_register_and_timbre_repairs() {
    let degrees = [
        7, 9, 11, 13, 15, 17, 19, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3,
        2, 1, 0, 1, 2, 3, 4,
    ];
    let mut score = study(&degrees);
    assert!(score.composition_report().register_fit);
    for voice in [
        Instrument::Piano,
        Instrument::ElectricPiano,
        Instrument::Arp,
    ] {
        score.tracks[0].instrument = Some(voice);
        assert!(!score.scalar_exercise_concerns().is_empty());
    }
    score.tracks[0].steps = degrees
        .iter()
        .flat_map(|degree| [Some(*degree), None])
        .collect();
    score.tracks[0].velocities = None;
    score.tracks[0].durations = Some(vec![0.125; 64]);
    assert!(
        !score.scalar_exercise_concerns().is_empty(),
        "uniform gaps between every scale note are not contrasting phrases"
    );
}

#[test]
fn brief_scalar_gestures_and_rhythmically_shaped_stepwise_phrases_are_allowed() {
    let gesture = study(&[0, 1, 2, 3, 4, 3, 2, 0, 2, 4, 3, 2, 4, 2, 1, 0]);
    assert!(gesture.scalar_exercise_concerns().is_empty());
    let mut shaped = study(&[7, 6, 5, 4, 3, 2, 1, 0]);
    shaped.tracks[0].steps = vec![
        Some(7),
        Some(6),
        None,
        Some(5),
        Some(4),
        None,
        None,
        Some(3),
        Some(2),
        Some(1),
        None,
        None,
        Some(0),
        None,
        None,
        None,
    ];
    shaped.tracks[0].durations = Some(vec![0.2; 16]);
    shaped.tracks[0].velocities = None;
    assert!(shaped.scalar_exercise_concerns().is_empty());
}

#[test]
fn arpeggio_support_is_allowed_beside_a_real_audible_melody_but_not_a_token_part() {
    let mut score = study(&(0..=14).step_by(2).collect::<Vec<_>>());
    score.tracks[0].instrument = Some(Instrument::Arp);
    let mut melody = study(&[0, 2, 1, 0, 4, 2, 3, 0]).tracks.remove(0);
    melody.gain = 0.4;
    score.tracks.push(melody);
    assert!(score.scalar_exercise_concerns().is_empty());
    score.tracks[1].gain = 0.01;
    assert!(!score.scalar_exercise_concerns().is_empty());
}

#[test]
fn drones_percussion_and_silent_scalar_slots_do_not_trigger_scale_guard() {
    let mut score = study(&[0; 16]);
    score.composition.as_mut().unwrap().texture = Texture::Ambient;
    assert!(score.scalar_exercise_concerns().is_empty());
    score.tracks[0].steps = (0..16).map(Some).collect();
    score.tracks[0].instrument = Some(Instrument::Perc);
    assert!(score.scalar_exercise_concerns().is_empty());
    score.tracks[0].instrument = Some(Instrument::Piano);
    score.tracks[0].velocities = Some(
        (0..16)
            .map(|i| if i % 4 == 0 { 0.7 } else { 0.0 })
            .collect(),
    );
    assert!(score.scalar_exercise_concerns().is_empty());
}

#[test]
fn attack_and_phrase_diagnostics_use_the_explicit_quarter_note_grid() {
    let mut score = study(&[0; 64]);
    score.tracks[0].instrument = Some(Instrument::Cello);
    assert!(
        score
            .automatic_composition_blockers()
            .iter()
            .any(|text| text.contains("0.25 beats apart"))
    );
    score.composition.as_mut().unwrap().steps_per_bar = 4;
    assert_eq!(score.grid_divisions(&score.tracks[0]), 4);
    assert!(score.automatic_composition_blockers().is_empty());
    assert!(score.composition_report().articulation);
    assert!(score.theory_context().contains("at most 4 slots"));
}
