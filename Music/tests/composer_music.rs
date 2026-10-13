use std::collections::HashSet;
use teledra_music::{Instrument, Mode, Score, VerifiedAudio, compose_chamber_study, default_score};

fn reference() -> Score {
    let mut score = default_score();
    score.bpm = 96;
    score.tonic = 7;
    score.mode = Mode::Dorian;
    score.tracks.truncate(2);
    for (track, voice) in score
        .tracks
        .iter_mut()
        .zip([Instrument::Celesta, Instrument::Arp])
    {
        track.instrument = Some(voice);
        track.name = voice.label().to_owned();
    }
    score
}

fn bar_notes(score: &Score, bar: usize) -> Vec<Option<u8>> {
    score.tracks[0].steps[bar * 8..bar * 8 + 8].to_vec()
}

#[test]
fn composed_material_has_question_answer_contrast_and_a_recognizable_return() {
    let previous = reference();
    for seed in 0..3 {
        let score = compose_chamber_study(&previous, seed).unwrap();
        assert_eq!(score.bpm, previous.bpm);
        assert_eq!(score.tonic, previous.tonic);
        assert!(matches!(score.mode, Mode::Dorian));
        assert_eq!(score.bars, 8);
        assert_eq!(score.composition_plan().steps_per_bar, 8);
        assert_eq!(score.composition_plan().development, 0.0);
        assert_eq!(
            bar_notes(&score, 0),
            bar_notes(&score, 2),
            "the opening must be recognizable"
        );
        assert_ne!(
            bar_notes(&score, 1),
            bar_notes(&score, 3),
            "the answer needs a different destination"
        );
        assert_ne!(
            bar_notes(&score, 0),
            bar_notes(&score, 4),
            "contrast needs different material"
        );
        let opening: Vec<_> = bar_notes(&score, 0).into_iter().flatten().take(4).collect();
        let closing: Vec<_> = bar_notes(&score, 7).into_iter().flatten().take(4).collect();
        assert_eq!(
            opening, closing,
            "the closing phrase must recall the opening cell"
        );
        assert_eq!(score.tracks[0].steps.iter().flatten().last(), Some(&0));
        assert_eq!(score.progression[6].root_degree, 4);
        assert_eq!(score.progression[7].root_degree, 0);

        let notes: Vec<_> = score.tracks[0].steps.iter().flatten().copied().collect();
        let intervals: Vec<_> = notes
            .windows(2)
            .map(|pair| i16::from(pair[1]) - i16::from(pair[0]))
            .collect();
        assert!(intervals.iter().any(|interval| *interval >= 2));
        assert!(intervals.iter().any(|interval| *interval <= -2));
        assert!(intervals.iter().collect::<HashSet<_>>().len() >= 5);
        let mut longest = 0;
        let mut run = 0;
        let mut direction = 0;
        for interval in intervals {
            if interval.abs() == 1 {
                run = if interval == direction { run + 1 } else { 1 };
                direction = interval;
                longest = longest.max(run);
            } else {
                run = 0;
                direction = 0;
            }
        }
        assert!(longest <= 3, "the melody became a scalar run");
        assert!(score.automatic_composition_blockers().is_empty());
    }
}

#[test]
fn supporting_parts_have_their_own_register_rhythm_and_harmonic_job() {
    let score = compose_chamber_study(&reference(), 0).unwrap();
    let [melody, accompaniment, bass] = score.tracks.as_slice() else {
        panic!("expected three purposeful parts")
    };
    assert_eq!(melody.instrument(), Instrument::Piano);
    assert_eq!(accompaniment.instrument(), Instrument::Guitar);
    assert_eq!(bass.instrument(), Instrument::DoubleBass);
    assert!(bass.octave < accompaniment.octave && accompaniment.octave < melody.octave);
    let rhythm =
        |track: &teledra_music::Track| track.steps.iter().map(Option::is_some).collect::<Vec<_>>();
    assert_ne!(rhythm(melody), rhythm(accompaniment));
    assert_ne!(rhythm(accompaniment), rhythm(bass));
    assert!(accompaniment.active_note_count() < melody.active_note_count());
    assert!(bass.active_note_count() < accompaniment.active_note_count());
    for bar in 0..8 {
        assert_eq!(
            bass.steps[bar * 8],
            Some(score.progression[bar].root_degree)
        );
    }
    let report = score.composition_report();
    assert!(report.melody && report.breathing_room && report.form);
    assert!(report.register_fit && report.independent_parts);
}

#[test]
fn phrase_ends_leave_real_space_and_releases_finish_before_new_harmonies() {
    for bpm in [40, 96, 220] {
        let mut previous = reference();
        previous.bpm = bpm;
        let score = compose_chamber_study(&previous, 0).unwrap();
        for track in &score.tracks {
            for phrase_end_bar in [1, 3, 5, 7] {
                let end = (0..8)
                    .filter_map(|slot| {
                        let index = phrase_end_bar * 8 + slot;
                        track.steps[index]
                            .map(|_| slot as f32 * 0.5 + track.durations.as_ref().unwrap()[index])
                    })
                    .fold(0.0, f32::max);
                assert!(
                    end <= 2.30,
                    "{} at {bpm}BPM leaves no phrase-ending breath",
                    track.name
                );
            }
        }
    }
}

#[test]
fn all_keys_modes_and_tempos_keep_the_bounded_composition_valid() {
    for mode in [
        Mode::Major,
        Mode::Minor,
        Mode::Dorian,
        Mode::Mixolydian,
        Mode::Lydian,
        Mode::Phrygian,
        Mode::HarmonicMinor,
    ] {
        for tonic in 0..12 {
            for bpm in [40, 96, 220] {
                let mut previous = reference();
                previous.tonic = tonic;
                previous.mode = mode;
                previous.bpm = bpm;
                let score = compose_chamber_study(&previous, u64::from(tonic)).unwrap();
                assert!(score.tracks.iter().all(|track| track.steps.len() == 64));
                assert!(score.composition_report().register_fit);
                assert!(score.automatic_composition_blockers().is_empty());
            }
        }
    }
}

#[test]
fn native_study_renders_complete_verified_audio_and_is_reproducible() {
    let reference = reference();
    for seed in 0..3 {
        let candidate = compose_chamber_study(&reference, seed).unwrap();
        assert!(candidate.automatic_composition_blockers().is_empty());
        let rendered = VerifiedAudio::new(candidate.render().unwrap()).unwrap();
        assert!(rendered.report().active_sections >= 7);
    }
    let score = compose_chamber_study(&reference, 0).unwrap();
    assert_eq!(
        serde_json::to_value(&score).unwrap(),
        serde_json::to_value(compose_chamber_study(&reference, 0).unwrap()).unwrap()
    );
    let audio = VerifiedAudio::new(score.render().unwrap()).unwrap();
    assert!((audio.report().duration_seconds - 20.0).abs() < 0.001);
    assert!(audio.report().active_sections >= 7);
    assert!(audio.report().peak < 0.89);
    assert!(audio.report().rms > 0.003);
    if let Ok(folder) = std::env::var("TELEDRA_COMPOSER_AUDIT_DIR") {
        let folder = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("chamber-study.json"),
            serde_json::to_vec_pretty(&score).unwrap(),
        )
        .unwrap();
        audio.write_wav(&folder.join("chamber-study.wav")).unwrap();
    }
}
