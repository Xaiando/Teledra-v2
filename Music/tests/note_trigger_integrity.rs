use teledra_music::{
    CompositionPlan, Instrument, MusicalForm, NoteTriggerOutcome, Score, default_score,
};

fn written_score(instruments: &[Instrument], grid: u8, development: f32) -> Score {
    let mut score = default_score();
    score.bars = 8;
    score.bpm = 220;
    score.composition = Some(CompositionPlan {
        form: MusicalForm::Arc,
        steps_per_bar: grid,
        development,
        ..Default::default()
    });
    let mut track = score.tracks[0].clone();
    track.steps = vec![Some(0); usize::from(grid)];
    track.velocities = Some(vec![0.6; usize::from(grid)]);
    track.durations = Some(vec![0.125; usize::from(grid)]);
    track.gain = 0.3;
    score.tracks = instruments
        .iter()
        .map(|&instrument| {
            let mut part = track.clone();
            part.name = instrument.label().to_owned();
            part.instrument = Some(instrument);
            part
        })
        .collect();
    score
}

#[test]
fn every_instrument_triggers_every_authored_onset_at_every_supported_grid() {
    for grid in [4, 8, 16] {
        for &instrument in Instrument::ALL {
            let score = written_score(&[instrument], grid, 0.0);
            score.validate().unwrap();
            let audit = score.note_trigger_audit();
            assert_eq!(audit.len(), usize::from(score.bars) * usize::from(grid));
            for (step, entry) in audit.iter().enumerate() {
                assert_eq!(
                    entry.outcome,
                    NoteTriggerOutcome::Scheduled,
                    "{instrument:?} grid {grid} step {step}"
                );
                assert_eq!(entry.source_step, step);
                assert_eq!(entry.pattern_step, step % usize::from(grid));
                assert!(!entry.generated);
                assert!(entry.voiced_events >= 1);
                assert_eq!(entry.voiced_events > 1, instrument.is_chordal());
                assert!(entry.start_frame.is_some());
            }
            assert!(
                audit
                    .windows(2)
                    .all(|pair| pair[0].start_frame < pair[1].start_frame)
            );
        }
    }
}

#[test]
fn positive_development_preserves_authored_entrances_rests_and_articulation() {
    // Violin previously disappeared in Intro/Break, Arp/Hat were thinned, and
    // the Hat's authored rests could become generated phrase-ending hits.
    let instruments = [
        Instrument::SubBass,
        Instrument::Violin,
        Instrument::Arp,
        Instrument::Hat,
    ];
    for grid in [4, 8, 16] {
        let mut score = written_score(&instruments, grid, 1.0);
        for part in &mut score.tracks {
            for slot in 0..part.steps.len() {
                if slot % 4 == 3 {
                    part.steps[slot] = None;
                }
                if slot % 4 == 1 {
                    part.velocities.as_mut().unwrap()[slot] = 0.0;
                }
            }
        }
        for entry in score.note_trigger_audit() {
            let expected = match entry.pattern_step % 4 {
                1 => NoteTriggerOutcome::ZeroVelocity,
                3 => NoteTriggerOutcome::WrittenRest,
                _ => NoteTriggerOutcome::Scheduled,
            };
            assert_eq!(
                entry.outcome, expected,
                "grid {grid}, track {}, step {}",
                entry.track, entry.source_step
            );
            assert!(!entry.generated, "authored rests must remain rests");
        }
    }
}

#[test]
fn trigger_audit_attributes_mutes_and_preserves_quiet_positive_dynamics() {
    let mut score = written_score(
        &[Instrument::Violin, Instrument::Piano, Instrument::Harp],
        16,
        0.0,
    );
    score.tracks[0].gain = 0.0;
    score.tracks[1].velocities = Some(vec![0.0; 16]);
    score.tracks[2].gain = 0.0001;
    score.tracks[2].velocities = Some(vec![0.0001; 16]);
    for entry in score.note_trigger_audit() {
        assert_eq!(
            entry.outcome,
            match entry.track {
                0 => NoteTriggerOutcome::MutedTrack,
                1 => NoteTriggerOutcome::ZeroVelocity,
                _ => NoteTriggerOutcome::Scheduled,
            }
        );
    }
    let report = score.arrangement_report();
    assert!(
        report.contains("0/128 written attacks scheduled, 0 generated, 0 zero velocity, 128 muted")
    );
    assert!(report.contains("128/128 written attacks scheduled"));
}

#[test]
fn legacy_spacing_omissions_have_exact_source_provenance() {
    let mut score = written_score(&[Instrument::Violin], 16, 0.0);
    score.composition = None;
    let audit = score.note_trigger_audit();
    assert!(
        audit
            .iter()
            .any(|entry| entry.outcome == NoteTriggerOutcome::LegacySpacing)
    );
    assert!(audit.iter().all(
        |entry| entry.written_degree == Some(0) && entry.pattern_step == entry.source_step % 16
    ));
    assert!(
        audit
            .iter()
            .filter(|entry| entry.outcome == NoteTriggerOutcome::LegacySpacing)
            .all(|entry| entry.voiced_events == 0 && entry.start_frame.is_none())
    );
}

#[test]
fn audit_pattern_positions_follow_explicit_form_restarts() {
    let mut score = written_score(&[Instrument::Piano], 8, 0.0);
    score.composition.as_mut().unwrap().form = MusicalForm::Binary;
    score.tracks[0].steps = (0..64).map(|slot| Some((slot % 7) as u8)).collect();
    score.tracks[0].velocities = Some(vec![0.6; 64]);
    score.tracks[0].durations = Some(vec![0.125; 64]);
    let audit = score.note_trigger_audit();
    assert_eq!(audit[32].pattern_step, 0);
    assert_eq!(audit[32].source_step, 32);
    for entry in audit {
        assert_eq!(entry.outcome, NoteTriggerOutcome::Scheduled);
        assert_eq!(
            entry.written_degree,
            score.tracks[0].steps[entry.pattern_step]
        );
    }
}
