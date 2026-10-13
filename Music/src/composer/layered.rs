//! An explicitly written long form: supporting motion continues while the
//! foreground breathes. Nothing relies on random deletion in the renderer.
use super::*;

const LONG_BARS: usize = 32;

/// A 32-bar layered development of the native motif. The four chapters are a
/// restrained opening, a full statement with piano answers, a thinner contrast
/// building to the high point, and a recognizable return that sheds density.
pub fn compose_layered_study(reference: &Score, seed: u64) -> Result<Score, MusicError> {
    let miniature = compose_chamber_study(reference, seed)?;
    let mut theme = long_track(miniature.tracks[0].instrument(), "Theme", 0.39, -0.2);
    let mut motion = long_track(Instrument::Guitar, "Moving accompaniment", 0.22, 0.32);
    let mut bass = long_track(Instrument::DoubleBass, "Foundation", 0.25, 0.0);
    let mut harmony = long_track(Instrument::Keys, "Sustained harmony", 0.19, 0.12);
    let answer_voice = if theme.instrument() == Instrument::Piano {
        Instrument::ElectricPiano
    } else {
        Instrument::Piano
    };
    let mut answer = long_track(answer_voice, "Answering phrases", 0.29, -0.34);
    let mut pulse = long_track(Instrument::Shaker, "Pulse", 0.19, 0.24);

    for bar in 0..LONG_BARS {
        let chapter = bar / 8;
        let local = bar % 8;
        let root = miniature.progression[local].root_degree;
        let energy = [0.66, 0.90, 1.0, 0.82][chapter];
        let closing = bar == LONG_BARS - 1;

        // Keep the recognizable contour in opening, statement and return.
        // The contrast has displaced entries and a different chord-tone arch.
        if chapter == 2 {
            let (slots, degrees): (&[usize], Vec<u8>) = if local % 2 == 0 {
                (
                    &[0, 1, 3, 4, 6],
                    vec![root + 2, root + 4, root + 3, root + 2, root],
                )
            } else {
                (&[1, 3], vec![root + 4, root + 2])
            };
            for (&slot, degree) in slots.iter().zip(degrees) {
                put(
                    &mut theme,
                    bar,
                    slot,
                    degree,
                    if slot == 3 { 0.38 } else { 0.46 },
                    0.63,
                );
            }
        } else if closing {
            put(&mut theme, bar, 0, 0, 2.6, 0.52);
        } else {
            for slot in 0..GRID {
                let old = local * GRID + slot;
                if let Some(degree) = miniature.tracks[0].steps[old] {
                    // The opening states fragments; the statement reveals the
                    // complete motif. The last chapter gradually leaves space.
                    let fragment = match chapter {
                        0 => local < 4,
                        3 => local >= 4,
                        _ => false,
                    };
                    if fragment && slot >= 4 {
                        continue;
                    }
                    // A exposed strong arrival needs to agree with the new
                    // sustained harmonic layer. Neighbour notes remain on
                    // weaker starts; the renderer receives the final pitches.
                    let degree = if slot % 4 == 0 {
                        chord_anchor(degree, root)
                    } else {
                        degree
                    };
                    let duration = miniature.tracks[0].durations.as_ref().unwrap()[old];
                    let velocity = miniature.tracks[0].velocities.as_ref().unwrap()[old] * energy;
                    put(&mut theme, bar, slot, degree, duration, velocity);
                }
            }
        }

        // Chordal support lasts through the lead's breathing space. Its actual
        // release finishes at the next harmony, rather than a whole beat early.
        let release = harmony.instrument().patch().amp.release * f32::from(reference.bpm) / 60.0;
        put(
            &mut harmony,
            bar,
            0,
            0,
            (4.0 - release - 0.02).max(0.125),
            0.66 * energy,
        );

        // Foundation keeps a stable downbeat and adds a coordinated offbeat
        // approach in the developed chapters. It doesn't stop with the theme.
        put(
            &mut bass,
            bar,
            0,
            root,
            if chapter == 0 || closing { 3.1 } else { 1.6 },
            0.66 * energy,
        );
        if chapter != 0 && !closing {
            put(&mut bass, bar, 4, root + 4, 0.78, 0.49 * energy);
            if local % 4 == 2 {
                put(&mut bass, bar, 6, root + 2, 0.28, 0.44 * energy);
            }
        }

        // A stable offbeat figure supplies motion; changing its contour and
        // subtracting it at the contrast changes texture without a scale walk.
        if bar >= 2 && !matches!(bar, 16 | 17) && !closing {
            let shapes = [[0, 2, 4, 2], [4, 2, 0, 2], [0, 4, 2, 4], [2, 4, 2, 0]];
            for (i, offset) in shapes[local % 4].into_iter().enumerate() {
                if (chapter == 0 || bar >= 28) && i % 2 == 1 {
                    continue;
                }
                put(
                    &mut motion,
                    bar,
                    i * 2 + 1,
                    root + offset,
                    0.27,
                    (0.46 + if i == 0 { 0.08 } else { 0.0 }) * energy,
                );
            }
        }

        // Piano answers after the lead's last attack, entering only in chapter
        // two. These are different foreground turns, not a doubled melody.
        if bar >= 8 && local % 2 == 1 && bar < 28 {
            put(&mut answer, bar, 5, root + 4, 0.43, 0.60 * energy);
            put(&mut answer, bar, 7, root + 2, 0.31, 0.48 * energy);
        }

        // Four steady quarters, reserved for the full statement and the later
        // rise. Its absence gives the contrast a consequence the ear can hear.
        if (8..16).contains(&bar) || (20..28).contains(&bar) {
            for slot in [0, 2, 4, 6] {
                put(
                    &mut pulse,
                    bar,
                    slot,
                    0,
                    0.16,
                    if slot == 0 || slot == 4 { 0.53 } else { 0.37 },
                );
            }
        }
    }
    for (track, target) in [
        (&mut theme, 76.0),
        (&mut motion, 58.0),
        (&mut bass, 39.0),
        (&mut harmony, 56.0),
        (&mut answer, 67.0),
    ] {
        track.octave = octave_for(track, reference.mode, reference.tonic, target);
    }
    pulse.octave = 3;
    // Release tails belong to the written arrangement too. Restrict only the
    // final gate of each bar when its tail would cross a harmony change, and
    // move a late attack earlier when the voice cannot articulate in that gap.
    for (track, shift_complete_cell) in [(&mut motion, false), (&mut answer, true)] {
        for bar in 0..LONG_BARS {
            let release = track.instrument().patch().amp.release * f32::from(reference.bpm) / 60.0;
            let latest = ((4.0 - release - 0.15) * 2.0).floor().clamp(0.0, 7.0) as usize;
            let positions: Vec<_> = (0..GRID)
                .filter(|slot| track.steps[bar * GRID + slot].is_some())
                .collect();
            let shift = positions
                .last()
                .copied()
                .unwrap_or(0)
                .saturating_sub(latest);
            // Shift the whole answering cell, never swap its final note in
            // front of the first: tempo must not reverse the written contour.
            for slot in positions {
                if shift == 0 {
                    break;
                }
                // Keep the guitar's offbeat foundation. Only its final pickup
                // needs to move; shifting every attack aligns it with the pulse.
                if !shift_complete_cell && slot <= latest {
                    continue;
                }
                let from = bar * GRID + slot;
                let to = from - shift;
                track.steps.swap(from, to);
                track.durations.as_mut().unwrap().swap(from, to);
                track.velocities.as_mut().unwrap().swap(from, to);
            }
            for slot in 0..GRID {
                let at = bar * GRID + slot;
                if track.steps[at].is_some() {
                    let available = (4.0 - slot as f32 * 0.5 - release - 0.02).max(0.125);
                    track.durations.as_mut().unwrap()[at] =
                        track.durations.as_ref().unwrap()[at].min(available);
                }
            }
        }
    }
    let score = Score {
        title: "Light in Motion".to_owned(),
        bars: LONG_BARS as u16,
        tracks: vec![theme, harmony, bass, motion, answer, pulse],
        ..miniature
    };
    score.validate()?;
    Ok(score)
}

fn long_track(instrument: Instrument, part: &str, gain: f32, pan: f32) -> Track {
    let mut track = empty_track(instrument, part, gain, pan);
    track.steps.resize(LONG_BARS * GRID, None);
    track
        .velocities
        .as_mut()
        .unwrap()
        .resize(LONG_BARS * GRID, 0.0);
    track
        .durations
        .as_mut()
        .unwrap()
        .resize(LONG_BARS * GRID, 0.25);
    track
}

fn put(track: &mut Track, bar: usize, slot: usize, degree: u8, duration: f32, velocity: f32) {
    let index = bar * GRID + slot;
    track.steps[index] = Some(degree);
    track.durations.as_mut().unwrap()[index] = duration;
    track.velocities.as_mut().unwrap()[index] = velocity;
}

fn chord_anchor(degree: u8, root: u8) -> u8 {
    (0..=20)
        .filter(|note| [root % 7, (root + 2) % 7, (root + 4) % 7].contains(&(note % 7)))
        .min_by_key(|note| note.abs_diff(degree))
        .unwrap_or(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_tempo_keeps_the_answering_contour_and_every_note() {
        for bpm in [40, 96, 220] {
            let mut reference = crate::default_score();
            reference.bpm = bpm;
            let score = compose_layered_study(&reference, 0).unwrap();
            for bar in (9..28).step_by(2) {
                let root = score.progression[bar % 8].root_degree;
                let notes: Vec<_> = score.tracks[4].steps[bar * 8..bar * 8 + 8]
                    .iter()
                    .flatten()
                    .copied()
                    .collect();
                assert_eq!(notes, [root + 4, root + 2], "answer reversed at {bpm} BPM");
            }
            assert!(
                score.automatic_composition_blockers().is_empty(),
                "{:?}",
                score.automatic_composition_blockers()
            );
            let guitar_slots: Vec<_> = score.tracks[3].steps[8 * 8..9 * 8]
                .iter()
                .enumerate()
                .filter_map(|(slot, degree)| degree.map(|_| slot))
                .collect();
            assert_eq!(
                &guitar_slots[..3],
                &[1, 3, 5],
                "motion lost its offbeats at {bpm} BPM"
            );
        }
    }

    #[test]
    fn chapters_have_written_entries_contrast_and_a_recognizable_return() {
        let score = compose_layered_study(&crate::default_score(), 0).unwrap();
        assert_eq!(score.bars, 32);
        assert!(score.tracks.iter().all(|track| track.steps.len() == 256));
        let [theme, _, _, motion, answer, pulse] = score.tracks.as_slice() else {
            panic!()
        };
        assert_ne!(theme.steps[..64], theme.steps[64..128]);
        assert_ne!(theme.steps[64..128], theme.steps[128..192]);
        assert_eq!(theme.steps[64..96], theme.steps[192..224]);
        assert!(answer.steps[..64].iter().all(Option::is_none));
        assert!(answer.steps[64..128].iter().any(Option::is_some));
        assert!(motion.steps[128..144].iter().all(Option::is_none));
        assert!(pulse.steps[128..160].iter().all(Option::is_none));
        assert!(pulse.steps[160..224].iter().any(Option::is_some));
        assert!(pulse.steps[224..].iter().all(Option::is_none));
    }

    #[test]
    fn supporting_harmony_continues_through_the_foreground_breaths() {
        let mut reference = crate::default_score();
        reference.bpm = 96;
        let score = compose_layered_study(&reference, 0).unwrap();
        let plan = score.arrange();
        let beat = 60.0 / f32::from(score.bpm) * crate::SAMPLE_RATE as f32;
        for bar in (1..31).step_by(2) {
            let probe = ((bar as f32 * 4.0 + 3.0) * beat) as usize;
            assert!(
                plan.events.iter().any(|event| {
                    let end = event.start_frame
                        + ((event.gate_seconds + event.instrument.patch().amp.release)
                            * crate::SAMPLE_RATE as f32) as usize;
                    event.track == 1 && event.start_frame < probe && end > probe
                }),
                "all layers stopped at bar {}",
                bar + 1
            );
        }
    }

    #[test]
    fn held_and_strong_theme_arrivals_agree_with_sustained_harmony() {
        for mode in [
            Mode::Major,
            Mode::Minor,
            Mode::Dorian,
            Mode::Mixolydian,
            Mode::Lydian,
            Mode::Phrygian,
            Mode::HarmonicMinor,
        ] {
            let mut reference = crate::default_score();
            reference.mode = mode;
            for seed in 0..3 {
                let score = compose_layered_study(&reference, seed).unwrap();
                for (slot, degree) in score.tracks[0].steps.iter().enumerate() {
                    if let Some(degree) = degree
                        && slot % 4 == 0
                    {
                        let root = score.progression[(slot / 8) % 8].root_degree;
                        assert_eq!(
                            chord_anchor(*degree, root),
                            *degree,
                            "bar {} mode {mode:?}",
                            slot / 8
                        );
                    }
                }
                assert!(
                    score.automatic_composition_blockers().is_empty(),
                    "{:?}",
                    score.automatic_composition_blockers()
                );
            }
        }
    }
}
