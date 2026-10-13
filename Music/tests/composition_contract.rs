use teledra_music::{
    Chord, ChordQuality, CompositionPlan, HarmonicPolicy, Mode, Score, Texture, default_score,
};

fn chamber() -> Score {
    let mut score = default_score();
    score.title = "Written duet".to_owned();
    score.bars = 16;
    score.tracks.retain(|track| {
        matches!(
            track.instrument(),
            teledra_music::Instrument::Harp | teledra_music::Instrument::Flute
        )
    });
    score.composition = Some(CompositionPlan {
        texture: Texture::Chamber,
        development: 0.0,
        ..Default::default()
    });
    score
}

#[test]
fn explicit_piece_preserves_its_ensemble_form_and_written_melody() {
    let score = chamber();
    let mastered = score.mastered().unwrap();
    assert_eq!(mastered.bars, 16);
    assert_eq!(
        serde_json::to_value(&mastered.tracks).unwrap(),
        serde_json::to_value(&score.tracks).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&mastered.progression).unwrap(),
        serde_json::to_value(&score.progression).unwrap()
    );
    let mut varied = mastered;
    for _ in 0..3 {
        varied = varied.evolved().unwrap();
    }
    assert_eq!(varied.tracks[0].steps, score.tracks[0].steps);
    assert_eq!(varied.tracks[1].steps, score.tracks[1].steps);
    assert_eq!(varied.bars, 16);
}

#[test]
fn intentional_chromatic_harmony_survives_but_diatonic_repair_preserves_sevenths() {
    let mut score = chamber();
    score.mode = Mode::Minor;
    score.progression = vec![Chord {
        root_degree: 4,
        quality: ChordQuality::DominantSeventh,
    }];
    score.composition.as_mut().unwrap().harmony = HarmonicPolicy::Chromatic;
    let mastered = score.mastered().unwrap();
    assert_eq!(mastered.progression[0].quality.intervals(), &[0, 4, 7, 10]);
    score.composition.as_mut().unwrap().harmony = HarmonicPolicy::Diatonic;
    let repaired = score.mastered().unwrap();
    assert_eq!(repaired.progression[0].quality.intervals(), &[0, 3, 7, 10]);
    assert_eq!(repaired.progression[0].root_degree, 4);
}

#[test]
fn expression_arrays_round_trip_and_reject_misalignment_and_non_finite_data() {
    let mut score = chamber();
    let length = score.tracks[0].steps.len();
    score.tracks[0].velocities = Some(vec![0.6; length]);
    score.tracks[0].durations = Some(vec![0.75; length]);
    let loaded = Score::from_json(&serde_json::to_string(&score).unwrap()).unwrap();
    assert_eq!(loaded.tracks[0].durations, score.tracks[0].durations);
    score.tracks[0].velocities.as_mut().unwrap().pop();
    assert!(score.validate().is_err());
    score.tracks[0].velocities = None;
    score.tracks[0].durations.as_mut().unwrap()[0] = f32::NAN;
    assert!(score.validate().is_err());
}

#[test]
fn planned_wide_melody_degrees_are_never_reinterpreted_as_trigger_positions() {
    let mut score = chamber();
    score.tracks[0].steps = vec![Some(0), Some(4), Some(8), Some(12)];
    let loaded = Score::from_json(&serde_json::to_string(&score).unwrap()).unwrap();
    assert_eq!(loaded.tracks[0].steps, score.tracks[0].steps);
}

#[test]
fn additional_modes_construct_diatonic_harmony_and_zero_velocity_is_not_a_voice() {
    for mode in [Mode::Lydian, Mode::Phrygian, Mode::HarmonicMinor] {
        let mut score = chamber();
        score.mode = mode;
        score.progression = (0..7)
            .map(|root_degree| Chord {
                root_degree,
                quality: ChordQuality::Major,
            })
            .collect();
        let mastered = score.mastered().unwrap();
        let scale = mode.scale();
        for chord in mastered.progression {
            let root = scale[usize::from(chord.root_degree)];
            assert!(
                chord
                    .quality
                    .intervals()
                    .iter()
                    .all(|interval| scale.contains(&((root + interval) % 12)))
            );
        }
    }
    let mut score = chamber();
    for track in &mut score.tracks {
        track.velocities = Some(vec![0.0; track.steps.len()]);
    }
    assert!(
        score
            .tracks
            .iter()
            .all(|track| track.active_note_count() == 0)
    );
    assert!(!score.composition_report().melody);
    assert!(!score.composition_report().palette);
}

#[test]
fn native_experiments_preserve_mixer_mutes_and_try_different_motif_operations() {
    let mut score = default_score();
    score.composition = Some(CompositionPlan::default());
    score.tracks[0].gain = 0.0;
    let gains: Vec<_> = score.tracks.iter().map(|track| track.gain).collect();
    let melody = score
        .tracks
        .iter()
        .position(|track| track.instrument() == teledra_music::Instrument::Flute)
        .unwrap();
    score.tracks[melody].steps = vec![
        Some(0),
        Some(1),
        Some(3),
        Some(4),
        Some(6),
        Some(5),
        Some(4),
        Some(2),
    ];
    let mut motifs = std::collections::HashSet::new();
    for _ in 0..18 {
        score = score.evolved().unwrap();
        assert_eq!(
            score
                .tracks
                .iter()
                .map(|track| track.gain)
                .collect::<Vec<_>>(),
            gains
        );
        if score.evolution_kind() == teledra_music::EvolutionKind::Motif {
            motifs.insert(score.tracks[melody].steps.clone());
        }
    }
    assert!(
        motifs.len() >= 3,
        "successive motif turns must not alias to the same operation"
    );
}

#[test]
fn atomic_notes_convert_without_changing_the_written_music() {
    let mut value = serde_json::to_value(chamber()).unwrap();
    let track = value["tracks"][0].as_object_mut().unwrap();
    track.remove("steps");
    track.insert(
        "notes".into(),
        serde_json::json!([
            {"degree":0,"velocity":0.8,"duration":1.5},
            {"degree":null,"velocity":0,"duration":0.5},
            {"degree":2,"velocity":0.6,"duration":0.75},
            {"degree":7,"velocity":0.7,"duration":1}
        ]),
    );
    let score = Score::from_json(&value.to_string()).unwrap();
    assert_eq!(score.tracks[0].steps, [Some(0), None, Some(2), Some(7)]);
    assert_eq!(
        score.tracks[0].velocities.as_deref().unwrap(),
        [0.8, 0.0, 0.6, 0.7]
    );
    assert_eq!(
        score.tracks[0].durations.as_deref().unwrap(),
        [1.5, 0.5, 0.75, 1.0]
    );
    assert_eq!(
        serde_json::to_value(&score.tracks).unwrap(),
        serde_json::to_value(
            Score::from_json(&serde_json::to_string(&score).unwrap())
                .unwrap()
                .tracks
        )
        .unwrap()
    );
    value["tracks"][0]["steps"] = serde_json::json!([0, null]);
    assert!(
        Score::from_json(&value.to_string()).is_err(),
        "ambiguous note representations must be refused"
    );
    value["tracks"][0].as_object_mut().unwrap().remove("steps");
    value["tracks"][0]["notes"][2]["duration"] = serde_json::json!(-1);
    assert!(Score::from_json(&value.to_string()).is_err());
    value.as_object_mut().unwrap().remove("composition");
    value["tracks"][0]["notes"] = serde_json::json!([
        {"degree":0,"velocity":0.8,"duration":0.5},
        {"degree":4,"velocity":0.8,"duration":0.5},
        {"degree":8,"velocity":0.8,"duration":0.5},
        {"degree":12,"velocity":0.8,"duration":0.5}
    ]);
    let unplanned = Score::from_json(&value.to_string()).unwrap();
    assert_eq!(
        unplanned.tracks[0].steps,
        [Some(0), Some(4), Some(8), Some(12)]
    );
}

#[test]
fn texture_evolution_preserves_authored_instruments_and_polyphony() {
    use teledra_music::{EvolutionKind, Instrument};

    // Inspect scheduled notes, not just the new instrument's family label.
    // One authored attack per bar must remain a four-tone seventh chord or a
    // single melodic note after repeated native changes of instrumental colour.
    let scheduled_notes = |score: &Score| {
        score
            .note_trigger_audit()
            .iter()
            .map(|entry| entry.voiced_events)
            .sum::<usize>()
    };
    for instrument in [
        Instrument::Keys,
        Instrument::Organ,
        Instrument::Pad,
        Instrument::Reed,
    ] {
        let mut score = chamber();
        score.bars = 4;
        score.mode = Mode::Major;
        score.progression = vec![Chord {
            root_degree: 0,
            quality: ChordQuality::MajorSeventh,
        }];
        score.tracks.truncate(1);
        score.tracks[0].name = format!("Written {instrument:?}");
        score.tracks[0].instrument = Some(instrument);
        score.tracks[0].steps = vec![Some(0)];
        score.tracks[0].velocities = Some(vec![0.8]);
        score.tracks[0].durations = Some(vec![1.0]);
        let expected_notes = if instrument == Instrument::Reed {
            4
        } else {
            16
        };
        assert_eq!(scheduled_notes(&score), expected_notes);
        for turn in 0..12 {
            score.evolution = turn * 6 + 3;
            score = score.evolved().unwrap();
            assert_eq!(score.evolution_kind(), EvolutionKind::Texture);
            assert_eq!(score.tracks[0].instrument(), instrument);
            assert_eq!(
                scheduled_notes(&score),
                expected_notes,
                "{instrument:?} became {:?}",
                score.tracks[0].instrument()
            );
            assert_eq!(score.tracks[0].steps, [Some(0)]);
            assert_eq!(
                score.tracks[0].velocities.as_deref(),
                Some([0.8].as_slice())
            );
            assert_eq!(score.tracks[0].durations.as_deref(), Some([1.0].as_slice()));
        }
    }
}

#[test]
fn native_harmonic_experiments_keep_the_opening_tonic_and_cadential_arrival() {
    let mut score = default_score();
    let tonic = score.tonic;
    let bpm = score.bpm;
    let mut interior = std::collections::HashSet::new();
    for turn in 0..12 {
        score.evolution = turn * 6 + 4;
        score = score.evolved().unwrap();
        assert_eq!(
            score.evolution_kind(),
            teledra_music::EvolutionKind::Harmony
        );
        assert_eq!(score.progression.first().unwrap().root_degree, 0);
        assert!(matches!(
            score.progression.last().unwrap().root_degree,
            4 | 6
        ));
        assert_eq!(score.tonic, tonic);
        assert_eq!(score.bpm, bpm);
        let last = score.progression.len() - 1;
        interior.insert(
            score.progression[1..last]
                .iter()
                .map(|chord| chord.root_degree)
                .collect::<Vec<_>>(),
        );
    }
    assert!(
        interior.len() >= 2,
        "the harmony should still develop between its anchors"
    );
}
