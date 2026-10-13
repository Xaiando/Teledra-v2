use std::collections::HashSet;
use teledra_music::{
    CompositionPlan, Instrument, InstrumentRole, SCORE_SCHEMA, Score, Texture, VerifiedAudio,
    default_score,
};

#[test]
fn generation_schema_and_renderer_share_the_complete_palette() {
    assert!(
        Instrument::ALL.len() >= 70,
        "the court needs at least seventy playable instruments"
    );
    let schema: serde_json::Value = serde_json::from_str(SCORE_SCHEMA).unwrap();
    let names = schema["properties"]["tracks"]["items"]["properties"]["instrument"]["enum"]
        .as_array()
        .unwrap();
    assert_eq!(names.len(), Instrument::ALL.len());
    let mut ids = HashSet::new();
    let mut labels = HashSet::new();
    for &instrument in Instrument::ALL {
        assert!(ids.insert(instrument.id()), "duplicate instrument ID");
        assert!(
            labels.insert(instrument.label()),
            "duplicate instrument label"
        );
        let name = serde_json::to_value(instrument).unwrap();
        assert_eq!(name.as_str(), Some(instrument.id()));
        assert!(names.contains(&name), "missing generation voice {name}");
        assert_eq!(
            serde_json::from_value::<Instrument>(name).unwrap(),
            instrument
        );
        assert_eq!(Instrument::from_name(instrument.label()), instrument);
    }
}

#[test]
fn every_voice_survives_score_acceptance_and_produces_distinct_verified_audio() {
    let mut fingerprints = HashSet::new();
    for &instrument in Instrument::ALL {
        let mut score = default_score();
        score.bars = 2;
        score.bpm = 120;
        score.composition = Some(CompositionPlan {
            texture: Texture::Chamber,
            development: 0.0,
            ..Default::default()
        });
        score.tracks.truncate(1);
        let track = &mut score.tracks[0];
        track.name = instrument.label().to_owned();
        track.instrument = Some(instrument);
        track.octave = 4;
        track.gain = 0.5;
        // Short drums and damped plucks need an onset in every verification
        // window; a deliberately silent quarter of a solo is not a dead patch.
        track.steps = vec![Some(0), Some(2), Some(4), Some(0)];
        track.velocities = Some(vec![0.7, 0.6, 0.8, 0.6]);
        track.durations = Some(vec![0.75; 4]);
        let accepted = Score::from_json(&serde_json::to_string(&score).unwrap())
            .unwrap()
            .mastered()
            .unwrap();
        assert_eq!(accepted.tracks[0].instrument(), instrument);
        let samples = accepted
            .render()
            .unwrap_or_else(|error| panic!("{instrument:?} failed rendering: {error}"));
        let audio = VerifiedAudio::new(samples)
            .unwrap_or_else(|error| panic!("{instrument:?} failed audio verification: {error}"));
        assert!((audio.report().duration_seconds - 4.0).abs() < 0.001);
        let fingerprint = audio.samples().iter().fold(0_u64, |hash, sample| {
            hash.wrapping_mul(31)
                .wrapping_add(u64::from(sample.to_bits()))
        });
        assert!(
            fingerprints.insert(fingerprint),
            "{instrument:?} duplicates another timbre"
        );
    }
}

#[test]
fn native_texture_evolution_can_reach_every_pitched_instrument_without_changing_roles() {
    let mut score = default_score();
    let mut heard = HashSet::new();
    let roles: Vec<_> = score
        .tracks
        .iter()
        .map(|track| track.instrument().role())
        .collect();
    let chordal: Vec<_> = score
        .tracks
        .iter()
        .map(|track| track.instrument().is_chordal())
        .collect();
    for _ in 0..180 {
        score = score.evolved().unwrap();
        assert_eq!(
            score
                .tracks
                .iter()
                .map(|track| track.instrument().role())
                .collect::<Vec<_>>(),
            roles
        );
        assert_eq!(
            score
                .tracks
                .iter()
                .map(|track| track.instrument().is_chordal())
                .collect::<Vec<_>>(),
            chordal
        );
        let pitched: Vec<_> = score
            .tracks
            .iter()
            .map(|track| track.instrument())
            .filter(|instrument| !instrument.is_percussive())
            .collect();
        assert_eq!(
            pitched.iter().collect::<HashSet<_>>().len(),
            pitched.len(),
            "an evolution doubled a timbre"
        );
        heard.extend(score.tracks.iter().map(|track| track.instrument()));
    }
    for voice in Instrument::ALL
        .iter()
        .filter(|instrument| instrument.role() != InstrumentRole::Percussion)
    {
        assert!(
            heard.contains(voice),
            "native evolution never reaches {voice:?}"
        );
    }
}

#[test]
fn composer_context_names_unused_colours_and_respects_variations() {
    let context = default_score().theory_context();
    assert!(context.contains("Current instruments:"));
    assert!(context.contains("COMPLETE PLAYABLE INSTRUMENT CATALOG"));
    assert!(context.to_ascii_lowercase().contains("clarinet"));
    assert!(context.to_ascii_lowercase().contains("theremin"));
    assert!(context.contains("Preserve requested instruments"));
}

#[test]
fn role_metadata_keeps_chords_single_lines_and_drum_parts_separate() {
    assert!(Instrument::ALL.contains(&Instrument::Piano));
    assert!(Instrument::ALL.contains(&Instrument::Theremin));
    assert!(Instrument::Theremin.is_monophonic());
    assert!(!Instrument::Piano.is_monophonic());
    for &instrument in Instrument::ALL {
        assert_eq!(
            instrument.is_chordal(),
            instrument.role() == InstrumentRole::Harmony
        );
        assert!(!(instrument.is_chordal() && instrument.is_monophonic()));
        let (low, high) = instrument.natural_range();
        assert!(
            low < high && high <= 127,
            "{instrument:?} has an invalid playable range"
        );
        assert!((0.0..=1.0).contains(&instrument.mix_level()));
        assert!((-1.0..=1.0).contains(&instrument.default_pan()));
        if instrument.role() == InstrumentRole::Percussion {
            assert!(
                instrument.family().is_empty(),
                "a drum must keep its rhythmic role"
            );
        } else {
            assert!(instrument.family().contains(&instrument));
            assert!(
                instrument
                    .family()
                    .iter()
                    .all(|other| other.role() == instrument.role())
            );
        }
    }
}
