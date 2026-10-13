//! A bounded native composition, with written musical relationships rather than
//! a random walk or a scale exercise. The score remains ordinary editable data.

use crate::{
    Chord, ChordQuality, CompositionPlan, HarmonicPolicy, Instrument, Mode, MusicError,
    MusicalForm, Score, Texture, Track, diatonic_quality, scale_offset,
};

mod layered;
pub use layered::compose_layered_study;

const GRID: usize = 8;
const BARS: usize = 8;

/// Compose an eight-bar chamber miniature in the reference key and tempo.
///
/// A two-bar question returns as an answer, a higher contrasting phrase changes
/// the rhythm, then the opening cell returns over a closing cadence. The seed
/// selects one of three original motif contours; it never randomizes pitches.
/// Suitable existing instruments retain their seats. A missing seat receives a
/// piano, guitar, or double bass, rather than an arbitrary catalog rotation.
pub fn compose_chamber_study(reference: &Score, seed: u64) -> Result<Score, MusicError> {
    let seed = seed & u64::from(u32::MAX);
    let (motif, title): ([u8; 5], &str) = match seed % 3 {
        0 => ([2, 4, 3, 2, 0], "Returning Light"),
        1 => ([4, 2, 3, 4, 2], "A Window Conversation"),
        _ => ([0, 2, 1, 2, 4], "A Small Answer"),
    };
    let melodic_voice = choose_voice(
        reference,
        &[
            Instrument::Piano,
            Instrument::ElectricPiano,
            Instrument::Guitar,
            Instrument::JazzGuitar,
            Instrument::Clarinet,
            Instrument::Flute,
        ],
        &[],
        Instrument::Piano,
    );
    let accompaniment_voice = choose_voice(
        reference,
        &[
            Instrument::Guitar,
            Instrument::JazzGuitar,
            Instrument::Piano,
            Instrument::ElectricPiano,
            Instrument::Harp,
            Instrument::Ukulele,
        ],
        &[melodic_voice],
        if melodic_voice == Instrument::Guitar {
            Instrument::Piano
        } else {
            Instrument::Guitar
        },
    );
    let bass_voice = choose_voice(
        reference,
        &[
            Instrument::DoubleBass,
            Instrument::Cello,
            Instrument::SubBass,
            Instrument::Bass,
        ],
        &[],
        Instrument::DoubleBass,
    );

    // In modes where V or VI is diminished, use a stable modal neighbour.
    // This miniature does not need a sustained tritone to establish contrast.
    let cadence = stable_root(reference.mode, &[4, 6, 3]);
    let contrast = stable_root(reference.mode, &[5, 6, 3]);
    let roots = [0, cadence, 0, 3, contrast, 3, cadence, 0];

    let mut melody = empty_track(melodic_voice, "Theme", 0.43, -0.18);
    let mut accompaniment = empty_track(accompaniment_voice, "Broken chords", 0.22, 0.30);
    let mut bass = empty_track(bass_voice, "Foundation", 0.19, 0.0);

    // The opening cell is repeated verbatim in bar three. Phrasing and harmony
    // turn its second occurrence into an answer rather than another scale lap.
    for bar in [0, 2] {
        write_cell(
            &mut melody,
            reference.bpm,
            bar,
            &motif,
            &[0, 2, 3, 4, 6],
            &[0.72, 0.44, 0.38, 0.72, 0.52],
            &[0.72, 0.61, 0.52, 0.65, 0.55],
        );
    }
    write_cell(
        &mut melody,
        reference.bpm,
        1,
        &[cadence, cadence + 2, cadence],
        &[0, 2, 3],
        &[0.80, 0.42, 0.75],
        &[0.63, 0.57, 0.49],
    );
    write_cell(
        &mut melody,
        reference.bpm,
        3,
        &[5, 3, 0],
        &[0, 2, 3],
        &[0.80, 0.42, 0.75],
        &[0.63, 0.55, 0.48],
    );

    // Contrast: displaced entries, a higher third-based arch, then a spacious
    // descending answer. Its highest note occurs once, not on every scale lap.
    write_cell(
        &mut melody,
        reference.bpm,
        4,
        &[
            contrast,
            contrast + 2,
            contrast + 4,
            contrast + 3,
            contrast + 2,
        ],
        &[0, 1, 3, 4, 6],
        &[0.42, 0.82, 0.44, 0.70, 0.48],
        &[0.65, 0.69, 0.75, 0.58, 0.55],
    );
    write_cell(
        &mut melody,
        reference.bpm,
        5,
        &[7, 5, 3],
        &[0, 2, 3],
        &[0.85, 0.42, 0.75],
        &[0.61, 0.53, 0.47],
    );

    // The penultimate bar opens the cadence. The final bar recalls the complete
    // opening contour at half-beat spacing and settles on tonic with a breath.
    write_cell(
        &mut melody,
        reference.bpm,
        6,
        &[cadence + 2, cadence, cadence + 1, cadence + 2, cadence + 4],
        &[0, 2, 3, 4, 6],
        &[0.70, 0.44, 0.38, 0.70, 0.48],
        &[0.65, 0.58, 0.51, 0.62, 0.53],
    );
    let mut closing = motif;
    closing[4] = 0;
    write_cell(
        &mut melody,
        reference.bpm,
        7,
        &closing,
        &[0, 1, 2, 3, 4],
        &[0.42, 0.42, 0.38, 0.42, 0.25],
        &[0.63, 0.57, 0.49, 0.53, 0.48],
    );

    for (bar, root) in roots.into_iter().enumerate() {
        // Accompaniment answers between melodic attacks. Its shorter, lower
        // chord fragments never copy the melody's contour or complete rhythm.
        if bar % 2 == 0 {
            let degrees = if bar == 4 {
                [root + 4, root + 2, root]
            } else {
                [root + 2, root + 4, root + 2]
            };
            write_cell(
                &mut accompaniment,
                reference.bpm,
                bar,
                &degrees,
                &[1, 3, 5],
                &[0.42, 0.42, 0.42],
                &[0.45, 0.40, 0.37],
            );
            write_cell(
                &mut bass,
                reference.bpm,
                bar,
                &[root, root + 4],
                &[0, 4],
                &[1.60, 0.65],
                &[0.54, 0.42],
            );
        } else {
            write_cell(
                &mut accompaniment,
                reference.bpm,
                bar,
                &[root + 2, root + 4],
                &[1, 3],
                &[0.44, 0.42],
                &[0.38, 0.33],
            );
            write_cell(
                &mut bass,
                reference.bpm,
                bar,
                &[root],
                &[0],
                &[1.55],
                &[0.48],
            );
        }
    }

    // Fit each complete line once, keeping interval shape and the three roles
    // in separate registers. No note-by-note octave folding is composed in.
    melody.octave = octave_for(&melody, reference.mode, reference.tonic, 68.0);
    accompaniment.octave = octave_for(&accompaniment, reference.mode, reference.tonic, 53.0);
    bass.octave = octave_for(&bass, reference.mode, reference.tonic, 39.0);

    let score = Score {
        title: title.to_owned(),
        bpm: reference.bpm,
        bars: BARS as u16,
        tonic: reference.tonic,
        mode: reference.mode,
        progression: roots
            .into_iter()
            .map(|root_degree| Chord {
                root_degree,
                quality: diatonic_quality(reference.mode, root_degree),
            })
            .collect(),
        tracks: vec![melody, accompaniment, bass],
        swing: Some(0.0),
        seed: Some(seed),
        sidechain: Some(0.0),
        evolution: reference.evolution.saturating_add(1),
        composition: Some(CompositionPlan {
            form: MusicalForm::Arc,
            phrase_bars: 2,
            chord_bars: 1,
            steps_per_bar: GRID as u8,
            development: 0.0,
            texture: Texture::Chamber,
            harmony: HarmonicPolicy::Diatonic,
        }),
    };
    score.validate()?;
    Ok(score)
}

fn stable_root(mode: Mode, candidates: &[u8]) -> u8 {
    candidates
        .iter()
        .copied()
        .find(|degree| {
            !matches!(
                diatonic_quality(mode, *degree),
                ChordQuality::Diminished | ChordQuality::Augmented
            )
        })
        .unwrap_or(0)
}

fn choose_voice(
    reference: &Score,
    suitable: &[Instrument],
    taken: &[Instrument],
    fallback: Instrument,
) -> Instrument {
    reference
        .tracks
        .iter()
        .map(Track::instrument)
        .find(|voice| suitable.contains(voice) && !taken.contains(voice))
        .unwrap_or(fallback)
}

fn empty_track(instrument: Instrument, part: &str, gain: f32, pan: f32) -> Track {
    Track {
        name: format!("{} · {part}", instrument.label()),
        waveform: instrument.patch().waveform,
        gain,
        octave: 4,
        steps: vec![None; BARS * GRID],
        instrument: Some(instrument),
        gate: None,
        pan: Some(pan),
        velocities: Some(vec![0.0; BARS * GRID]),
        durations: Some(vec![0.25; BARS * GRID]),
    }
}

fn write_cell(
    track: &mut Track,
    bpm: u16,
    bar: usize,
    notes: &[u8],
    slots: &[usize],
    lengths: &[f32],
    velocities: &[f32],
) {
    let release_beats = track.instrument().patch().amp.release * f32::from(bpm) / 60.0;
    // A phrase ending needs a real-time breath after the release, including
    // slower instruments such as harp at fast tempi. Ordinary bar changes
    // keep their small beat-based clearance without altering the main rhythm.
    let clearance_beats = if bar % 2 == 1 {
        (0.16 * f32::from(bpm) / 60.0).max(0.12)
    } else {
        0.12
    };
    for (((&degree, &slot), &duration), &velocity) in
        notes.iter().zip(slots).zip(lengths).zip(velocities)
    {
        // Count the instrument's real release when leaving a breath and when
        // approaching another harmony. At fast tempi, omit a late pickup that
        // cannot release in time instead of letting it bleed into a new chord.
        let available = 4.0 - slot as f32 * 0.5 - release_beats - clearance_beats;
        if available < 0.125 {
            continue;
        }
        let index = bar * GRID + slot;
        track.steps[index] = Some(degree);
        track.durations.as_mut().unwrap()[index] = duration.min(available);
        track.velocities.as_mut().unwrap()[index] = velocity;
    }
}

fn octave_for(track: &Track, mode: Mode, tonic: u8, target: f32) -> i8 {
    let pitches: Vec<_> = track
        .steps
        .iter()
        .flatten()
        .map(|degree| f32::from(scale_offset(mode, *degree)))
        .collect();
    let low = pitches.iter().copied().fold(f32::INFINITY, f32::min);
    let high = pitches.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let (bottom, top) = track.instrument().natural_range();
    (0..=8_i8)
        .min_by(|a, b| {
            let cost = |octave: i8| {
                let base = 12.0 * (f32::from(octave) + 1.0) + f32::from(tonic);
                let outside = (f32::from(bottom) - base - low).max(0.0)
                    + (base + high - f32::from(top)).max(0.0);
                outside * 100.0 + (base + (low + high) * 0.5 - target).abs()
            };
            cost(*a).total_cmp(&cost(*b))
        })
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arranger_plays_all_eight_written_bars_once_without_rewriting_the_notes() {
        let mut reference = crate::default_score();
        reference.tracks.clear();
        reference.bpm = 96;
        reference.tonic = 7;
        reference.mode = Mode::Dorian;
        let score = compose_chamber_study(&reference, 0).unwrap();
        let plan = score.arrange();
        let step_frames = 0.5 * 60.0 / f32::from(score.bpm) * crate::SAMPLE_RATE as f32;
        for (index, track) in score.tracks.iter().enumerate() {
            let events: Vec<_> = plan
                .events
                .iter()
                .filter(|event| event.track == index)
                .collect();
            let written: Vec<_> = track
                .steps
                .iter()
                .enumerate()
                .filter_map(|(step, degree)| degree.map(|degree| (step, degree)))
                .collect();
            assert_eq!(
                events.len(),
                written.len(),
                "{} lost or repeated written attacks",
                track.name
            );
            for (event, (step, degree)) in events.iter().zip(written) {
                let expected = 12 * (i16::from(track.octave) + 1)
                    + i16::from(score.tonic)
                    + i16::from(scale_offset(score.mode, degree));
                assert_eq!(
                    event.midi, expected as f32,
                    "{} rewrote step {step}",
                    track.name
                );
                assert!(
                    (event.start_frame as f32 - step as f32 * step_frames).abs()
                        <= step_frames * 0.016 + 1.0
                );
                let gate_and_release = event.gate_seconds + track.instrument().patch().amp.release;
                let next_bar_frame = ((step / GRID + 1) * GRID) as f32 * step_frames;
                assert!(
                    event.start_frame as f32 + gate_and_release * (crate::SAMPLE_RATE as f32)
                        < next_bar_frame,
                    "{} overlaps a new harmony at step {step}",
                    track.name
                );
            }
        }
    }

    #[test]
    fn every_motif_has_audible_phrase_space_at_every_supported_tempo() {
        let mut reference = crate::default_score();
        reference.tracks.clear();
        for bpm in 40..=220 {
            reference.bpm = bpm;
            for seed in 0..3 {
                let score = compose_chamber_study(&reference, seed).unwrap();
                let plan = score.arrange();
                let bar_frames = 4.0 * 60.0 / f32::from(bpm) * crate::SAMPLE_RATE as f32;
                for phrase_end_bar in [1, 3, 5, 7] {
                    let phrase_end = (phrase_end_bar + 1) as f32 * bar_frames;
                    let phrase_start = (phrase_end_bar - 1) as f32 * bar_frames;
                    let last_dry_end = plan
                        .events
                        .iter()
                        .filter(|event| {
                            // Recover the written bar before micro-timing: a
                            // next-phrase attack can lean a few frames early.
                            let step_frames = bar_frames / GRID as f32;
                            let bar =
                                ((event.start_frame as f32 / step_frames).round() as usize) / GRID;
                            bar >= phrase_end_bar - 1 && bar <= phrase_end_bar
                        })
                        .map(|event| {
                            event.start_frame as f32
                                + (event.gate_seconds + event.instrument.patch().amp.release)
                                    * crate::SAMPLE_RATE as f32
                        })
                        .fold(phrase_start, f32::max);
                    let dry_gap = (phrase_end - last_dry_end) / crate::SAMPLE_RATE as f32;
                    assert!(
                        dry_gap > 0.14,
                        "seed {seed}, {bpm}BPM, phrase {} has only {dry_gap:.3}s of actual release space",
                        phrase_end_bar / 2 + 1
                    );
                }
            }
        }
    }

    #[test]
    fn retained_harps_long_release_leaves_phrase_space_at_fast_tempos() {
        let mut reference = crate::default_score();
        for bpm in [180, 200, 220] {
            reference.bpm = bpm;
            for seed in 0..3 {
                let score = compose_chamber_study(&reference, seed).unwrap();
                assert_eq!(score.tracks[1].instrument(), Instrument::Harp);
                let plan = score.arrange();
                let bar_frames = 4.0 * 60.0 / f32::from(bpm) * crate::SAMPLE_RATE as f32;
                let step_frames = bar_frames / GRID as f32;
                for event in &plan.events {
                    let bar = (event.start_frame as f32 / step_frames).round() as usize / GRID;
                    let next_bar = (bar + 1) as f32 * bar_frames;
                    let dry_end = event.start_frame as f32
                        + (event.gate_seconds + event.instrument.patch().amp.release)
                            * crate::SAMPLE_RATE as f32;
                    assert!(
                        dry_end < next_bar,
                        "{:?} crosses a harmonic boundary at {bpm}BPM",
                        event.instrument
                    );
                    if bar % 2 == 1 {
                        let gap = (next_bar - dry_end) / crate::SAMPLE_RATE as f32;
                        assert!(
                            gap > 0.14,
                            "{:?} leaves only {gap:.3}s after its release at {bpm}BPM",
                            event.instrument
                        );
                    }
                }
            }
        }
    }
}
