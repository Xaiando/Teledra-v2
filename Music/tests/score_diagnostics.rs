use teledra_music::{
    CompositionPlan, HarmonicPolicy, Instrument, Mode, MusicalForm, Score, Texture,
};

fn doubled_scale() -> Score {
    let mut score = teledra_music::default_score();
    score.title = "Duplicated high scale regression".to_owned();
    score.bpm = 96;
    score.bars = 32;
    score.tonic = 7;
    score.mode = Mode::Dorian;
    score.composition = Some(CompositionPlan {
        form: MusicalForm::Rondo,
        phrase_bars: 4,
        steps_per_bar: 16,
        chord_bars: 2,
        development: 0.0,
        texture: Texture::Chamber,
        harmony: HarmonicPolicy::Chromatic,
    });
    let mut line = score.tracks[0].clone();
    line.name = "Celesta line".to_owned();
    line.instrument = Some(Instrument::Celesta);
    line.octave = 5;
    line.gain = 0.4;
    let degrees = [
        7, 9, 11, 13, 15, 17, 19, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3,
        2, 1, 0, 1, 2, 3, 4,
    ];
    line.steps = degrees
        .into_iter()
        .flat_map(|degree| [Some(degree), None])
        .collect();
    line.velocities = Some(
        (0..64)
            .map(|index| if index % 2 == 0 { 0.75 } else { 0.0 })
            .collect(),
    );
    line.durations = Some(vec![0.5; 64]);
    line.pan = Some(-0.8);
    let mut doubled = line.clone();
    doubled.name = "Arp line".to_owned();
    doubled.instrument = Some(Instrument::Arp);
    doubled.pan = Some(0.8);
    score.tracks = vec![line, doubled];
    score
}

#[test]
fn repeated_high_scale_is_not_awarded_complete_structure_or_breathing_for_nulls() {
    let score = doubled_scale();
    score.validate().unwrap();
    let report = score.composition_report();
    assert!(!report.register_fit && !report.independent_parts && !report.separation);
    assert!(
        !report.breathing_room,
        "held notes fill all alternating null slots"
    );
    assert!(
        !report.form,
        "a form label and 32 bars do not supply contrasting phrases"
    );
    assert!(report.quality < 100);
    assert_eq!(report.grade, "Needs score review");
    let context = score.theory_context();
    assert!(context.contains("MIDI 79..113"));
    assert!(context.contains("DOUBLING:"));
    assert!(context.contains("PHRASE SPACE:"));
    assert!(
        score
            .theory_report()
            .next_experiment
            .contains("Repair this concrete issue")
    );
    assert!(
        score
            .automatic_composition_blockers()
            .iter()
            .any(|concern| concern.contains("duplicates one out-of-range melody"))
    );
    assert!(!score.scalar_exercise_concerns().is_empty());
    // Compatibility and literal authoring remain separate from auto-composer acceptance.
    assert!(Score::from_json(&serde_json::to_string(&score).unwrap()).is_ok());
}

#[test]
fn shortened_written_notes_create_real_gaps_without_adding_more_null_slots() {
    let mut score = doubled_scale();
    for track in &mut score.tracks {
        track.durations = Some(vec![0.125; 64]);
    }
    assert!(score.composition_report().breathing_room);
    assert!(
        !score
            .orchestration_concerns()
            .iter()
            .any(|text| text.contains("PHRASE SPACE:"))
    );
}

#[test]
fn intentional_in_range_doubling_is_diagnosed_but_not_globally_prohibited() {
    let mut score = doubled_scale();
    for track in &mut score.tracks {
        track.octave = 4;
        track.steps = (0..16)
            .map(|index| {
                if index % 2 == 0 {
                    Some((index % 8) as u8)
                } else {
                    None
                }
            })
            .collect();
        track.velocities = None;
        track.durations = Some(vec![0.25; 16]);
    }
    let report = score.composition_report();
    assert!(report.register_fit);
    assert!(!report.independent_parts);
    assert!(score.automatic_composition_blockers().is_empty());
}

#[test]
fn a_separate_bass_does_not_disguise_the_repeating_scale_exercise() {
    let mut score = doubled_scale();
    let mut bass = score.tracks[0].clone();
    bass.instrument = Some(Instrument::DoubleBass);
    bass.octave = 2;
    bass.steps = vec![Some(0), None, Some(4), None];
    bass.velocities = None;
    bass.durations = None;
    score.tracks.push(bass);
    assert!(
        score
            .automatic_composition_blockers()
            .iter()
            .all(|concern| !concern.contains("duplicates one out-of-range melody"))
    );
    assert!(!score.scalar_exercise_concerns().is_empty());
    assert!(
        !score.composition_report().register_fit,
        "warnings remain available for repair"
    );
}

#[test]
fn overlapping_held_notes_are_not_mistaken_for_release_gaps() {
    let mut score = doubled_scale();
    score.tracks.truncate(1);
    score.tracks[0].steps = vec![Some(0), None, Some(2), None];
    score.tracks[0].velocities = None;
    score.tracks[0].durations = Some(vec![8.0, 0.25, 0.125, 0.25]);
    assert!(
        !score.composition_report().breathing_room,
        "earlier long notes bridge later rests and loop boundary"
    );
}

#[test]
fn continuous_sixteenth_cello_is_blocked_with_exact_onset_spacing_evidence() {
    let mut score = doubled_scale();
    score.tracks.truncate(1);
    let track = &mut score.tracks[0];
    track.instrument = Some(Instrument::Cello);
    track.name = "Continuous cello".to_owned();
    track.octave = 2;
    track.steps = vec![Some(0); 16];
    track.velocities = None;
    track.durations = Some(vec![0.5; 16]);
    score.validate().unwrap();
    assert!(Score::from_json(&serde_json::to_string(&score).unwrap()).is_ok());
    let errors = score.automatic_composition_blockers();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("Continuous cello (cello)"));
    assert!(errors[0].contains("0.25 beats apart"));
    assert!(errors[0].contains("at least 1.00 beats"));
    assert!(!score.composition_report().articulation);

    score.tracks[0].steps = (0..16).map(|index| (index % 4 == 0).then_some(0)).collect();
    assert!(score.automatic_composition_blockers().is_empty());
    assert!(score.composition_report().articulation);
}

#[test]
fn rapid_unrestricted_patch_and_spaced_ambient_sustains_are_not_prohibited() {
    let mut score = doubled_scale();
    score.tracks.truncate(1);
    score.tracks[0].instrument = Some(Instrument::Arp);
    score.tracks[0].octave = 4;
    score.tracks[0].steps = vec![Some(0); 16];
    score.tracks[0].velocities = None;
    score.tracks[0].durations = Some(vec![0.5; 16]);
    assert!(score.composition_report().articulation);
    assert!(
        score.automatic_composition_blockers().is_empty(),
        "absence of rests alone is not an automatic rejection"
    );

    score.composition.as_mut().unwrap().texture = Texture::Ambient;
    score.tracks[0].instrument = Some(Instrument::Cello);
    score.tracks[0].octave = 2;
    score.tracks[0].steps = vec![Some(0); 4];
    score.tracks[0].durations = Some(vec![8.0; 4]);
    assert!(score.automatic_composition_blockers().is_empty());
    assert!(score.composition_report().articulation);
}

#[test]
fn spacing_diagnostics_check_the_loop_boundary_and_ignore_zero_velocity_attacks() {
    let mut score = doubled_scale();
    score.tracks.truncate(1);
    let track = &mut score.tracks[0];
    track.instrument = Some(Instrument::Cello);
    track.octave = 2;
    track.steps = (0..16)
        .map(|index| [0, 15].contains(&index).then_some(0))
        .collect();
    track.velocities = None;
    track.durations = None;
    assert!(
        !score.automatic_composition_blockers().is_empty(),
        "the final attack is too close to the first after the loop restarts"
    );
    score.tracks[0].steps = vec![Some(0); 16];
    score.tracks[0].velocities = Some(
        (0..16)
            .map(|index| if index % 4 == 0 { 0.7 } else { 0.0 })
            .collect(),
    );
    assert!(score.automatic_composition_blockers().is_empty());
}
