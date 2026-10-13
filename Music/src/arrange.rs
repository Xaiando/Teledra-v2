//! Turns a score into timed, harmonised, dynamic note events.
//!
//! The previous engine divided the whole piece by a track's pattern length, so
//! a four-step pad held each note for ten seconds and every pattern played
//! exactly once. Nothing repeated, nothing varied, and the tempo was decorative.
//!
//! Written patterns loop against the bar grid. A composition plan shapes their
//! form, harmonic rhythm and development; harmony is voiced in simultaneous
//! parts while melodies retain the passing tones that give them a contour.

use crate::{
    Chord, ChordQuality, HarmonicPolicy, Instrument, InstrumentRole, Mode, MusicalForm, Score,
    Texture, Track, chord_intervals, scale_offset,
};

pub const STEPS_PER_BEAT: usize = 4;
pub const BEATS_PER_BAR: usize = 4;
pub const STEPS_PER_BAR: usize = STEPS_PER_BEAT * BEATS_PER_BAR;

/// Where the piece is, and therefore how it should behave.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    /// Sparse statement of the harmony.
    Intro,
    /// Parts enter and the filters open.
    Rise,
    /// Everything playing.
    Main,
    /// Rhythm section drops out; the harmony is exposed.
    Break,
    /// The fullest and brightest return.
    Finale,
}

impl Section {
    pub fn at(bar: usize, bars: usize) -> Self {
        // Short scores cannot support five sections without each lasting less
        // than a phrase, so they get the arc without the break.
        let position = bar as f32 / bars.max(1) as f32;
        if bars < 8 {
            return if position < 0.25 {
                Self::Rise
            } else {
                Self::Main
            };
        }
        match position {
            p if p < 0.15 => Self::Intro,
            p if p < 0.35 => Self::Rise,
            p if p < 0.62 => Self::Main,
            p if p < 0.80 => Self::Break,
            _ => Self::Finale,
        }
    }

    /// Overall loudness and brightness, 0..1.
    pub fn intensity(self) -> f32 {
        match self {
            Self::Intro => 0.42,
            Self::Rise => 0.68,
            Self::Main => 0.92,
            Self::Break => 0.55,
            Self::Finale => 1.0,
        }
    }

    /// Whether a legacy unplanned part sounds in this section. Planned scores
    /// already contain their authored entrances and rests.
    ///
    /// Removing parts is the only way to make adding them back feel like an
    /// event. A mix in which everything plays for the whole piece has no
    /// dynamics no matter how the notes are written.
    pub fn plays(self, instrument: Instrument) -> bool {
        // The harmonic bed: what holds a section together underneath everything
        // else, and the only thing playing when the piece is at its sparsest.
        let bed = matches!(
            instrument.role(),
            InstrumentRole::Bass | InstrumentRole::Harmony
        );
        // Voices that carry a line. Removing these is what makes the quiet
        // sections quiet; letting them all play everywhere flattened the arc
        // from a threefold level swing to barely half of one.
        let melodic = instrument.role() == InstrumentRole::Melody;
        let colour = instrument.role() == InstrumentRole::Colour;
        match self {
            Self::Intro => bed,
            Self::Rise => !matches!(instrument, Instrument::Snare) && !melodic,
            Self::Main | Self::Finale => true,
            Self::Break => bed || colour,
        }
    }
}

/// One note the synthesiser will sound.
#[derive(Clone, Copy, Debug)]
pub struct NoteEvent {
    pub track: usize,
    pub instrument: Instrument,
    pub start_frame: usize,
    pub gate_seconds: f32,
    /// Absolute dry-voice end at an incompatible harmony or the next attack of
    /// a single-line part. The synthesiser also bounds the release, so ending
    /// the gate cannot leave the old pitch ringing over its replacement.
    pub end_frame: Option<usize>,
    pub midi: f32,
    /// Previous pitch when the written notes touch or overlap. This connection
    /// is decided before humanization and gate taper, which create small gaps
    /// even in a legato phrase; an authored rest always breaks the connection.
    pub glide_from_midi: Option<f32>,
    pub velocity: f32,
    /// Arrangement automation for this note's filter, as a cutoff multiplier.
    pub cutoff_scale: f32,
    pub seed: u32,
}

/// Why one occurrence on a track's written grid did or did not trigger a voice.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteTriggerOutcome {
    Scheduled,
    WrittenRest,
    ZeroVelocity,
    MutedTrack,
    LegacySection,
    LegacyThinning,
    LegacySpacing,
    OutsideRender,
}

/// A trace of the actual scheduler, with chord tones counted as one attack.
#[derive(Clone, Debug, serde::Serialize)]
pub struct NoteTrigger {
    /// Zero-based track and absolute grid position in the rendered piece.
    pub track: usize,
    pub source_step: usize,
    /// Zero-based slot in the written pattern, after formal region restarts.
    pub pattern_step: usize,
    pub written_degree: Option<u8>,
    /// True for a legacy fill candidate in a rest rather than a written note.
    pub generated: bool,
    pub outcome: NoteTriggerOutcome,
    pub voiced_events: usize,
    pub start_frame: Option<usize>,
}

/// The plan for one render: every note, plus how long the tail runs.
pub struct Arrangement {
    pub events: Vec<NoteEvent>,
    pub frames: usize,
    pub tail_frames: usize,
    pub kick_onsets: Vec<usize>,
    pub trigger_audit: Vec<NoteTrigger>,
}

/// A cheap deterministic hash, used wherever the arrangement wants variation.
///
/// Variation has to be reproducible: the Organist's keep/reject feedback only
/// means something if replaying the same score gives back the same music.
fn hash(seed: u64, a: u64, b: u64) -> u64 {
    let mut value =
        seed ^ a.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ b.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// The hash as a 0..1 fraction.
fn chance(seed: u64, a: u64, b: u64) -> f32 {
    (hash(seed, a, b) >> 40) as f32 / (1u64 << 24) as f32
}

/// How strongly a sixteenth-note position is accented.
///
/// Even velocities are the clearest sign of a machine playing. Downbeats are
/// loudest, backbeats next, and the sixteenths between them are ghosts.
fn accent(step_in_bar: usize) -> f32 {
    match step_in_bar % 16 {
        0 => 1.0,
        8 => 0.90,
        4 | 12 => 0.78,
        2 | 6 | 10 | 14 => 0.62,
        _ => 0.48,
    }
}

impl Score {
    /// Actual section boundaries used by the arranger, in bars with an
    /// exclusive end. Studios can display the form without approximating it.
    pub fn form_timeline(&self) -> Vec<(String, usize, usize)> {
        if !matches!(self.composition_plan().form, MusicalForm::Arc) {
            return self
                .form_regions()
                .into_iter()
                .map(|(theme, start, end)| {
                    (
                        match theme {
                            0 => "A",
                            1 => "B",
                            _ => "C",
                        }
                        .to_owned(),
                        start,
                        end,
                    )
                })
                .collect();
        }
        let mut timeline: Vec<(String, usize, usize)> = Vec::new();
        for bar in 0..usize::from(self.bars) {
            let label = match self.composition_plan().form {
                MusicalForm::Arc => format!("{:?}", self.section_at(bar)),
                _ => match self.theme_at(bar) {
                    0 => "A",
                    1 => "B",
                    _ => "C",
                }
                .to_owned(),
            };
            if let Some((previous, _, end)) = timeline.last_mut()
                && *previous == label
            {
                *end = bar + 1;
            } else {
                timeline.push((label, bar, bar + 1));
            }
        }
        timeline
    }

    fn form_regions(&self) -> Vec<(u8, usize, usize)> {
        let composition = self.composition_plan();
        let bars = usize::from(self.bars);
        let phrase_bars = usize::from(composition.phrase_bars).max(1);
        let themes: &[u8] = match composition.form {
            MusicalForm::Binary => &[0, 1],
            MusicalForm::Ternary => &[0, 1, 0],
            MusicalForm::Rondo => &[0, 1, 0, 2, 0],
            MusicalForm::Arc => return vec![(0, 0, bars)],
        };
        let phrases = bars / phrase_bars;
        // A short sketch cannot contain five complete phrases. Give its
        // regions whole bars; longer pieces always change at phrase boundaries.
        if phrases < themes.len() {
            return themes
                .iter()
                .enumerate()
                .filter_map(|(region, theme)| {
                    let start = region * bars / themes.len();
                    let end = (region + 1) * bars / themes.len();
                    (end > start).then_some((*theme, start, end))
                })
                .collect();
        }
        let mut lengths = match composition.form {
            MusicalForm::Binary => vec![phrases / 2, phrases - phrases / 2],
            MusicalForm::Ternary => {
                let outer = phrases / 3;
                vec![outer, phrases - outer * 2, outer]
            }
            MusicalForm::Rondo => {
                let mut regions = vec![phrases / 5; 5];
                for index in [0, 3, 4, 2, 1].into_iter().take(phrases % 5) {
                    regions[index] += 1;
                }
                regions
            }
            MusicalForm::Arc => unreachable!(),
        };
        for length in &mut lengths {
            *length *= phrase_bars;
        }
        // An incomplete closing phrase belongs at the end. Inserting its
        // extra bars in the middle would shift later entries off the phrase
        // grid (for example A4/B6/A4 in a fourteen-bar ternary).
        if let Some(last) = lengths.last_mut() {
            *last += bars % phrase_bars;
        }
        let mut start = 0;
        themes
            .iter()
            .zip(lengths)
            .map(|(theme, length)| {
                let region = (*theme, start, start + length);
                start += length;
                region
            })
            .collect()
    }

    fn region_start(&self, bar: usize) -> usize {
        self.form_regions()
            .iter()
            .find(|(_, start, end)| bar >= *start && bar < *end)
            .map_or(0, |(_, start, _)| *start)
    }

    fn theme_at(&self, bar: usize) -> u8 {
        let bars = usize::from(self.bars).max(1);
        match self.composition_plan().form {
            MusicalForm::Arc => u8::from(Section::at(bar, bars) == Section::Break),
            _ => self
                .form_regions()
                .iter()
                .find(|(_, start, end)| bar >= *start && bar < *end)
                .map_or(0, |(theme, _, _)| *theme),
        }
    }

    fn section_at(&self, bar: usize) -> Section {
        let bars = usize::from(self.bars).max(1);
        match self.composition_plan().form {
            MusicalForm::Arc => Section::at(bar, bars),
            MusicalForm::Binary => {
                if self.theme_at(bar) == 0 {
                    Section::Main
                } else {
                    Section::Finale
                }
            }
            MusicalForm::Ternary | MusicalForm::Rondo => {
                if self.theme_at(bar) == 0 {
                    Section::Main
                } else {
                    Section::Break
                }
            }
        }
    }

    fn developed_degree(&self, track: &Track, degree: u8, bar: usize) -> u8 {
        let development = self.composition_plan().development;
        if development <= 0.0 || !is_melodic(track.instrument()) {
            return degree;
        }
        match self.theme_at(bar) {
            // A is a literal return of the authored idea. B sequences its
            // contour; C reflects it around the opening note at high settings.
            1 => degree
                .saturating_add((development * 3.0).round() as u8)
                .min(24),
            2 if development >= 0.65 => {
                let pivot = track
                    .steps
                    .iter()
                    .flatten()
                    .next()
                    .copied()
                    .unwrap_or(degree);
                (2 * i16::from(pivot) - i16::from(degree)).clamp(0, 24) as u8
            }
            2 => degree
                .saturating_add((development * 5.0).round() as u8)
                .min(24),
            _ => degree,
        }
    }

    /// Keep every authored audible part. Texture changes articulation and
    /// brightness; it must not silently delete a flute from a harp/flute duet.
    fn texture_tracks(&self) -> Vec<usize> {
        let present: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(index, track)| {
                let audible = track.gain > 0.0
                    && track.steps.iter().enumerate().any(|(step, note)| {
                        note.is_some()
                            && track
                                .velocities
                                .as_ref()
                                .is_none_or(|values| values[step] > 0.0)
                    });
                audible.then_some(index)
            })
            .collect();
        present
    }

    /// Plans every note in the score.
    pub(crate) fn arrange(&self) -> Arrangement {
        self.arrange_with_audit(false)
    }

    /// Accounts for every written grid occurrence, including rests and mutes.
    /// A scheduled chord is one trigger with several voiced events. Timing is
    /// taken from the real scheduler rather than reconstructed from the score.
    pub fn note_trigger_audit(&self) -> Vec<NoteTrigger> {
        self.arrange_with_audit(true).trigger_audit
    }

    fn arrange_with_audit(&self, record_triggers: bool) -> Arrangement {
        let bars = usize::from(self.bars);
        let bar_frames =
            60.0 / f32::from(self.bpm) * BEATS_PER_BAR as f32 * crate::SAMPLE_RATE as f32;
        let frames = (bars as f32 * bar_frames) as usize;
        let seed = self.seed();
        let composition = self.composition_plan();
        let phrase_bars = usize::from(composition.phrase_bars).max(1);
        let development = composition.development.clamp(0.0, 1.0);
        // A plan contains authored orchestration: rests and attacks are part of
        // its composition. Only legacy patterns ask the arranger to invent or
        // remove hits. Development can still shape pitch and performance.
        let legacy_rhythm = self.composition.is_none();
        let harmonic_masks: Vec<u16> = (0..bars)
            .filter_map(|bar| self.chord_at(bar))
            .map(|chord| {
                let root =
                    i16::from(self.tonic) + i16::from(scale_offset(self.mode, chord.root_degree));
                chord_intervals(chord.quality)
                    .iter()
                    .fold(0, |mask, interval| {
                        mask | 1 << (root + i16::from(*interval)).rem_euclid(12)
                    })
            })
            .collect();
        let develop_harmony =
            development > 0.0 && matches!(composition.harmony, HarmonicPolicy::Diatonic);

        let mut events = Vec::new();
        let mut kick_onsets = Vec::new();
        let mut trigger_audit = Vec::new();

        // A section may only silence a part while some other part still plays.
        // The mute lists assume a full arrangement; applied to a sparse score
        // they can empty a whole section, and the verifier rightly rejects a
        // render with dead sections. A one-track score keeps its one track.
        let active_tracks = self.texture_tracks();
        let present: Vec<Instrument> = active_tracks
            .iter()
            .map(|index| self.tracks[*index].instrument())
            .collect();
        let muting_is_safe = |section: Section| present.iter().any(|part| section.plays(*part));

        for (index, track) in self.tracks.iter().enumerate() {
            let track_active = active_tracks.contains(&index);
            if !track_active && !record_triggers {
                continue;
            }
            let instrument = track.instrument();
            let patch = instrument.patch();

            // Short patterns span one bar. Longer phrases retain the selected
            // four/eight/sixteen-slot grid: sixty-four quarter-note slots can
            // describe sixteen bars rather than repeat a four-bar exercise.
            // Missing grid metadata preserves the legacy sixteenth-note grid.
            let divisions = self.grid_divisions(track);
            let frames_per_step = (bar_frames / divisions as f32).max(1.0);
            let track_steps = bars * divisions;
            // The position within the bar, expressed on the sixteenth grid the
            // accent table is written against.
            let minimum_spacing = instrument.attack_spacing() * bar_frames / 4.0;
            let first_event = events.len();
            let mut previous_onset: Option<f32> = None;
            let mut previous_voicing = Vec::new();
            let mut previous_glide: Option<(usize, f64, f32, Option<usize>)> = None;

            for step in 0..track_steps {
                let bar = step / divisions;
                let region_bar = bar - self.region_start(bar);
                let step_in_bar = (step % divisions) * STEPS_PER_BAR / divisions;
                let section = self.section_at(bar);
                let carries_theme =
                    !matches!(composition.form, MusicalForm::Arc) && is_melodic(instrument);
                // Returning A sections reuse the motif and its performance.
                // A contrasting section develops that motif rather than
                // transposing every consecutive phrase automatically.
                let theme = usize::from(self.theme_at(bar));
                let phrase_end = region_bar % phrase_bars == phrase_bars - 1;
                let fill =
                    legacy_rhythm && development > 0.0 && phrase_end && instrument.is_percussive();
                let pattern_index = (region_bar * divisions + step % divisions) % track.steps.len();
                let authored_velocity = track
                    .velocities
                    .as_ref()
                    .map(|values| values[pattern_index]);
                let written = track.steps[pattern_index];
                // A fill must be allowed to create a hit in an empty slot. The
                // old code checked for `None` and continued before it reached
                // the fill logic, so its promised phrase-ending drum fills
                // could never exist.
                let generated_fill = written.is_none()
                    && fill
                    && step_in_bar >= 12
                    && chance(seed, index as u64 + theme as u64 * 131, step_in_bar as u64)
                        < 0.42 * development;
                let mut record = |outcome, voiced_events, start_frame| {
                    if record_triggers {
                        trigger_audit.push(NoteTrigger {
                            track: index,
                            source_step: step,
                            pattern_step: pattern_index,
                            written_degree: written,
                            generated: generated_fill,
                            outcome,
                            voiced_events,
                            start_frame,
                        });
                    }
                };
                if authored_velocity == Some(0.0) {
                    record(
                        if written.is_some() {
                            NoteTriggerOutcome::ZeroVelocity
                        } else {
                            NoteTriggerOutcome::WrittenRest
                        },
                        0,
                        None,
                    );
                    continue;
                }
                let Some(degree) = written.or(generated_fill.then_some(0)) else {
                    record(NoteTriggerOutcome::WrittenRest, 0, None);
                    continue;
                };
                if !track_active {
                    record(NoteTriggerOutcome::MutedTrack, 0, None);
                    continue;
                }
                if legacy_rhythm
                    && development > 0.0
                    && !section.plays(instrument)
                    && !carries_theme
                    && muting_is_safe(section)
                {
                    record(NoteTriggerOutcome::LegacySection, 0, None);
                    continue;
                }
                if legacy_rhythm
                    && development > 0.0
                    && !self.keeps_note(seed, index, theme, step_in_bar, section, instrument, fill)
                {
                    record(NoteTriggerOutcome::LegacyThinning, 0, None);
                    continue;
                }
                let grid_onset = step as f32 * frames_per_step;
                if legacy_rhythm
                    && previous_onset.is_some_and(|last| grid_onset - last + 0.5 < minimum_spacing)
                {
                    record(NoteTriggerOutcome::LegacySpacing, 0, None);
                    continue;
                }
                previous_onset = Some(grid_onset);

                // Swing delays the off-eighths. Applied to the start only, so
                // the grid stays intact and the groove leans rather than
                // drifting out of time.
                let swing = if step_in_bar % 4 == 2 {
                    self.swing().clamp(0.0, 0.6) * 0.5
                } else {
                    0.0
                };
                // A few milliseconds of deterministic jitter, so simultaneous
                // parts do not phase-cancel into a single flat transient.
                // Micro-timing is a property of the groove, so the same step
                // leans the same way every bar. Re-rolled per note it is not
                // groove, it is sloppiness.
                let jitter = (chance(seed, index as u64 + 7_000, step_in_bar as u64) - 0.5) * 0.03;
                let start_frame =
                    ((step as f32 + swing + jitter) * frames_per_step).max(0.0) as usize;
                if start_frame >= frames {
                    record(NoteTriggerOutcome::OutsideRender, 0, None);
                    continue;
                }

                let chord = self.chord_at(bar);
                let degree = self.developed_degree(track, degree, bar);
                let cadence = phrase_end
                    && !((step + 1)..((bar + 1) * divisions)).any(|next| {
                        let slot = (pattern_index + next - step) % track.steps.len();
                        track.steps[slot].is_some()
                            && track
                                .velocities
                                .as_ref()
                                .is_none_or(|values| values[slot] > 0.0)
                    });
                let velocity = authored_velocity.unwrap_or_else(|| {
                    self.velocity_for(seed, index, theme, step_in_bar, section, instrument)
                });
                let texture_gate = if instrument.is_percussive() {
                    1.0
                } else {
                    match composition.texture {
                        Texture::Ensemble => 1.0,
                        Texture::Chamber => 0.9,
                        Texture::Ambient => 1.25,
                    }
                };
                let gate = instrument.gate() * track.gate_scale() * texture_gate;
                let gate_seconds = track.durations.as_ref().map_or_else(
                    || {
                        (gate * frames_per_step.max(minimum_spacing) / crate::SAMPLE_RATE as f32)
                            .max(0.01)
                    },
                    |values| values[pattern_index] * 60.0 / f32::from(self.bpm),
                );
                let intended_beats = track.durations.as_ref().map_or_else(
                    || {
                        f64::from(gate)
                            * (4.0 / divisions as f64).max(f64::from(instrument.attack_spacing()))
                    },
                    |values| f64::from(values[pattern_index]),
                );
                // A long held note is an arrival even when it enters off the
                // beat. Preserve brief passing tones, but do not let a weak
                // beat disguise a sustained clash with the accompaniment.
                let sustained_arrival = gate_seconds >= 2.0 * 60.0 / f32::from(self.bpm);
                let written_midi = self.pitch_for(
                    track,
                    instrument,
                    degree,
                    chord,
                    step_in_bar,
                    cadence || sustained_arrival,
                );
                // A line is fitted as a whole below. Folding individual notes
                // turns an ascending scale into sudden downward octave jumps,
                // and two instruments with different ranges cease to agree.
                let midi = if instrument.follows_harmony() && !instrument.is_chordal() {
                    written_midi
                } else {
                    fold_into_range(written_midi, instrument.natural_range())
                };

                let voicing = if instrument.is_chordal() {
                    chord.map(|chord| voice_chord(self, track, degree, chord, &previous_voicing))
                } else {
                    None
                };
                let pitches = voicing.as_deref().unwrap_or(std::slice::from_ref(&midi));
                // Equal-power gain keeps a seventh chord from becoming a
                // fourfold volume jump over the old single-note accompaniment.
                let voice_velocity = velocity / (pitches.len() as f32).sqrt();

                if instrument == Instrument::Kick {
                    kick_onsets.push(start_frame);
                }

                for (voice, midi) in pitches.iter().copied().enumerate() {
                    let end_frame = (develop_harmony && instrument.follows_harmony())
                        .then(|| next_conflicting_harmony(midi, bar, &harmonic_masks))
                        .flatten()
                        .map(|next_bar| (next_bar as f32 * bar_frames) as usize);
                    let glide_from_midi = if patch.portamento_seconds > 0.0 {
                        let connection = previous_glide.and_then(
                            |(previous_step, previous_beats, previous_midi, harmonic_end)| {
                                let separation_beats =
                                    (step - previous_step) as f64 * 4.0 / divisions as f64;
                                (previous_beats >= separation_beats
                                    && harmonic_end.is_none_or(|end| end as f32 > grid_onset))
                                .then_some(previous_midi)
                            },
                        );
                        previous_glide = Some((step, intended_beats, midi, end_frame));
                        connection
                    } else {
                        None
                    };
                    let gate_seconds = end_frame.map_or(gate_seconds, |end| {
                        let available =
                            end.saturating_sub(start_frame) as f32 / crate::SAMPLE_RATE as f32;
                        gate_seconds.min((available - patch.amp.tail().min(0.08)).max(0.01))
                    });
                    events.push(NoteEvent {
                        track: index,
                        instrument,
                        start_frame,
                        gate_seconds,
                        end_frame,
                        midi,
                        glide_from_midi,
                        velocity: voice_velocity,
                        cutoff_scale: self.cutoff_at(bar, step_in_bar, section, instrument),
                        seed: hash(seed, index as u64, step as u64 * 8 + voice as u64) as u32,
                    });
                }
                record(
                    NoteTriggerOutcome::Scheduled,
                    pitches.len(),
                    Some(start_frame),
                );
                if let Some(voicing) = voicing {
                    previous_voicing = voicing;
                }
            }
            if instrument.follows_harmony() && !instrument.is_chordal() {
                fit_phrase_register(&mut events[first_event..], instrument.natural_range());
            }
            // Single-line voices release before their replacement. Polyphonic
            // instruments can hold one written note while another begins;
            // explicit durations must not be shortened into staccato. Default
            // gates still avoid stacking sustained accompaniment. Intentional
            // harmonic cuts above apply independently to incompatible pitches.
            if instrument.is_monophonic()
                || (track.durations.is_none() && (minimum_spacing > 0.0 || instrument.is_chordal()))
            {
                cap_gates(&mut events[first_event..], frames);
            }
        }

        events.sort_unstable_by_key(|event| event.start_frame);
        // A final held note can outlive the bar grid by more than its release.
        // Size from the actual capped voices, then leave two seconds for the
        // effects after the last dry sample. The renderer wraps this complete
        // continuation over the loop rather than truncating written sustain.
        let last_voice_end = events
            .iter()
            .map(|event| {
                let voice_frames = ((event.gate_seconds + event.instrument.patch().amp.tail())
                    * crate::SAMPLE_RATE as f32) as usize
                    + 1;
                event
                    .start_frame
                    .saturating_add(voice_frames)
                    .min(event.end_frame.unwrap_or(usize::MAX))
            })
            .max()
            .unwrap_or(frames);
        let tail_frames = last_voice_end.saturating_sub(frames) + 2 * crate::SAMPLE_RATE as usize;

        Arrangement {
            events,
            frames,
            tail_frames,
            kick_onsets,
            trigger_audit,
        }
    }

    /// Whether a legacy pattern note sounds, given section and variation.
    #[allow(clippy::too_many_arguments)]
    fn keeps_note(
        &self,
        seed: u64,
        track: usize,
        phrase: usize,
        step_in_bar: usize,
        section: Section,
        instrument: Instrument,
        fill: bool,
    ) -> bool {
        // A fill on the last bar of a phrase: percussion doubles onto the
        // sixteenths it normally leaves empty, which is what signals that a
        // section is about to change.
        if fill && step_in_bar >= 12 && chance(seed, track as u64, step_in_bar as u64) < 0.55 {
            return true;
        }

        let section_density = match section {
            Section::Intro => 0.55,
            Section::Rise => 0.80,
            Section::Main => 0.97,
            Section::Break => 0.70,
            Section::Finale => 1.0,
        };
        let density = 1.0 - (1.0 - section_density) * self.composition_plan().development;
        // Only the parts that articulate rhythm are thinned, and only the
        // off-beats among those: a written note on a beat always sounds.
        let thinnable = (instrument.is_percussive()
            && !matches!(instrument, Instrument::Kick | Instrument::Snare))
            || instrument.role() == InstrumentRole::Colour
            || instrument == Instrument::Arp;
        if !thinnable || step_in_bar.is_multiple_of(4) {
            return true;
        }
        chance(
            seed,
            track as u64 + 31 + phrase as u64 * 97,
            step_in_bar as u64,
        ) < density
    }

    /// The velocity of one note, from accent, section, and humanisation.
    fn velocity_for(
        &self,
        seed: u64,
        track: usize,
        phrase: usize,
        step_in_bar: usize,
        section: Section,
        instrument: Instrument,
    ) -> f32 {
        let mut velocity = accent(step_in_bar);
        // Sustained parts should not pump with the beat grid the way a drum
        // does; they follow the section instead.
        if (instrument.role() == InstrumentRole::Harmony && instrument.attack_spacing() >= 1.0)
            || instrument == Instrument::SubBass
        {
            velocity = 0.85;
        }
        if matches!(self.composition_plan().texture, Texture::Ambient)
            && !instrument.is_percussive()
        {
            velocity = 0.75 + 0.25 * velocity;
        }
        velocity *= 0.55 + 0.45 * section.intensity();
        velocity *= 0.88
            + 0.24
                * chance(
                    seed,
                    track as u64 + 101 + phrase as u64 * 31,
                    step_in_bar as u64,
                );
        velocity.clamp(0.05, 1.0)
    }

    /// The authored progression index at each bar, matching the arranger's
    /// harmonic rhythm and restarts at formal section boundaries.
    pub fn harmonic_timeline(&self) -> Vec<usize> {
        if self.progression.is_empty() {
            return Vec::new();
        }
        (0..usize::from(self.bars))
            .map(|bar| self.chord_index_at(bar))
            .collect()
    }

    fn chord_index_at(&self, bar: usize) -> usize {
        let chord_bars = usize::from(self.composition_plan().chord_bars).max(1);
        ((bar - self.region_start(bar)) / chord_bars) % self.progression.len().max(1)
    }

    /// The chord governing a bar.
    fn chord_at(&self, bar: usize) -> Option<Chord> {
        if self.progression.is_empty() {
            return None;
        }
        Some(self.progression[self.chord_index_at(bar)])
    }

    /// Preserve written melodic movement between structural landing points.
    fn pitch_for(
        &self,
        track: &Track,
        instrument: Instrument,
        degree: u8,
        chord: Option<Chord>,
        step_in_bar: usize,
        cadence: bool,
    ) -> f32 {
        let base = 12 * (i16::from(track.octave) + 1) + i16::from(self.tonic);
        if !instrument.follows_harmony() {
            // Percussion is tuned by role, not by the key.
            let pitch = match instrument {
                Instrument::Kick => 36,
                Instrument::Snare => 62,
                Instrument::Hat => 84,
                Instrument::Perc => 55 + i16::from(degree % 5) * 2,
                _ => {
                    let (low, high) = instrument.natural_range();
                    (i16::from(low) + i16::from(high)) / 2
                }
            };
            return f32::from(pitch);
        }

        let Some(chord) = chord else {
            return f32::from(base + i16::from(scale_offset(self.mode, degree)));
        };
        let root = i16::from(scale_offset(self.mode, chord.root_degree));
        let intervals = chord_intervals(chord.quality);

        // Existing scores used bass degree zero as a root-following pattern.
        // An explicit composition plan opts into literal written bass degrees.
        if self.composition.is_none() {
            if instrument == Instrument::SubBass {
                return f32::from(base + root);
            }
            if instrument == Instrument::Bass {
                let offset = if step_in_bar % 8 == 6 {
                    i16::from(intervals[2])
                } else {
                    0
                };
                return f32::from(base + root + offset);
            }
        }

        // A weak-beat passing or neighbouring note belongs to the line,
        // including a walking bass. With zero development, every authored
        // degree is preserved. A written bass can therefore express an
        // inversion instead of being silently replaced by a stock root/fifth.
        let written = i16::from(scale_offset(self.mode, degree));
        let composition = self.composition_plan();
        let offset = if composition.development > 0.0
            && matches!(composition.harmony, HarmonicPolicy::Diatonic)
            && (cadence || step_in_bar == 0 || step_in_bar == 8)
        {
            nearest_chord_tone(written, root, intervals)
        } else {
            written
        };
        f32::from(base + offset)
    }

    /// The filter automation for one note.
    ///
    /// Two motions combined: the section's overall brightness, and a slow sweep
    /// across four bars. A static cutoff is the difference between a synthesiser
    /// that is playing and one that is merely on.
    fn cutoff_at(&self, bar: usize, step: usize, section: Section, instrument: Instrument) -> f32 {
        if instrument.is_percussive()
            || self
                .composition
                .as_ref()
                .is_some_and(|plan| plan.development == 0.0)
        {
            return 1.0;
        }
        let intensity = section.intensity();
        let sweep_position =
            (bar % 8) as f32 / 8.0 + (step % STEPS_PER_BAR) as f32 / STEPS_PER_BAR as f32 / 8.0;
        let sweep = (sweep_position * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        // Between roughly a third and twice the patch's own cutoff.
        let texture = match self.composition_plan().texture {
            Texture::Ensemble => 1.0,
            Texture::Chamber => 1.04,
            Texture::Ambient => 0.78,
        };
        let scale = (0.35 + 1.65 * (intensity * 0.72 + sweep * 0.28)) * texture;
        if section == Section::Rise {
            // A rise should audibly open across its own length.
            let progress = (bar as f32 / self.bars.max(1) as f32 - 0.15) / 0.20;
            return scale * (0.55 + 0.75 * progress.clamp(0.0, 1.0));
        }
        scale
    }
}

/// Choose one octave for the complete played line. Working ranges guide the
/// register; they are not hard sampler limits. When an unusually wide phrase
/// cannot fit, keep its contour and minimize overflow instead of inventing
/// octave jumps at the edges. Existing in-range writing remains untouched.
fn fit_phrase_register(events: &mut [NoteEvent], (low, high): (u8, u8)) {
    let Some(first) = events.first() else {
        return;
    };
    let (minimum, maximum) = events
        .iter()
        .fold((first.midi, first.midi), |(minimum, maximum), event| {
            (minimum.min(event.midi), maximum.max(event.midi))
        });
    let (low, high) = (f32::from(low), f32::from(high));
    if minimum >= low && maximum <= high {
        return;
    }
    let centre = (low + high) * 0.5;
    let mut best: Option<(f32, f32, f32)> = None;
    for octave in -10..=10 {
        let shift = octave as f32 * 12.0;
        let bottom = minimum + shift;
        let top = maximum + shift;
        if bottom < 0.0 || top > 127.0 {
            continue;
        }
        let overflow = (low - bottom).max(0.0).powi(2) + (top - high).max(0.0).powi(2);
        let distance = ((bottom + top) * 0.5 - centre).abs();
        if best.is_none_or(|(best_overflow, best_distance, _)| {
            overflow < best_overflow || (overflow == best_overflow && distance < best_distance)
        }) {
            best = Some((overflow, distance, shift));
        }
    }
    let shift = best.map_or(0.0, |(_, _, shift)| shift);
    for event in events {
        event.midi += shift;
        if let Some(previous) = &mut event.glide_from_midi {
            *previous += shift;
        }
    }
}

/// Folds a pitch by octaves until it sits in the instrument's own register.
///
/// Transposing rather than clamping keeps the written line's shape: a melody
/// pushed out of range comes back an octave down with its intervals intact,
/// where a clamp would flatten every high note onto the same ceiling.
fn fold_into_range(midi: f32, (low, high): (u8, u8)) -> f32 {
    let (low, high) = (f32::from(low), f32::from(high));
    if high - low < 12.0 {
        return midi.clamp(low, high);
    }
    let mut folded = midi;
    while folded > high {
        folded -= 12.0;
    }
    while folded < low {
        folded += 12.0;
    }
    folded
}

/// Moves a written pitch to the nearest tone of the current chord.
fn nearest_chord_tone(written: i16, root: i16, intervals: &[u8]) -> i16 {
    let mut best = written;
    let mut best_distance = i16::MAX;
    for octave in -2..=4 {
        for interval in intervals {
            let candidate = root + i16::from(*interval) + octave * 12;
            let distance = (candidate - written).abs();
            if distance < best_distance {
                best_distance = distance;
                best = candidate;
            }
        }
    }
    best
}

fn is_melodic(instrument: Instrument) -> bool {
    instrument.follows_harmony() && !instrument.is_chordal()
}

/// Let common tones ring, including through repeated chords, but finish a
/// developed diatonic voice before it becomes a non-chord tone in new harmony.
fn next_conflicting_harmony(midi: f32, bar: usize, masks: &[u16]) -> Option<usize> {
    let mut previous = *masks.get(bar)?;
    let pitch = 1 << (midi as i16).rem_euclid(12);
    for (next_bar, mask) in masks.iter().copied().enumerate().skip(bar + 1) {
        if mask != previous && mask & pitch == 0 {
            return Some(next_bar);
        }
        previous = mask;
    }
    None
}

/// Enumerate close-position inversions in the playable register. The first
/// voicing follows the authored octave; subsequent ones minimise the travel
/// of ordered voices within that same register, retaining common tones. A
/// locally cheaper inversion must not walk the accompaniment up an octave
/// every time the harmonic progression returns.
fn voice_chord(
    score: &Score,
    track: &Track,
    degree: u8,
    chord: Chord,
    previous: &[f32],
) -> Vec<f32> {
    let range = track.instrument().natural_range();
    let intervals = chord_intervals(chord.quality);
    let root = i16::from(score.tonic) + i16::from(scale_offset(score.mode, chord.root_degree));
    let pitches: Vec<f32> = (range.0..=range.1)
        .filter(|midi| {
            intervals.iter().any(|interval| {
                (i16::from(*midi) - root - i16::from(*interval)).rem_euclid(12) == 0
            })
        })
        .map(f32::from)
        .collect();
    let target = fold_into_range(
        f32::from(
            12 * (i16::from(track.octave) + 1)
                + i16::from(score.tonic)
                + i16::from(scale_offset(score.mode, degree))
                + 4,
        ),
        range,
    );
    let mut best_cost = f32::INFINITY;
    let mut best = Vec::new();
    let closest_register = pitches
        .windows(intervals.len())
        .map(|candidate| (candidate.iter().sum::<f32>() / candidate.len() as f32 - target).abs())
        .fold(f32::INFINITY, f32::min);
    for candidate in pitches.windows(intervals.len()) {
        let centre = candidate.iter().sum::<f32>() / candidate.len() as f32;
        // Allow nearby inversion choices without accumulating register drift.
        // Relative to the best available fit, this also works at patch edges
        // where the exact written centre has no complete playable chord.
        if (centre - target).abs() > closest_register + 2.0 {
            continue;
        }
        let cost = if previous.is_empty() {
            (centre - target).abs()
        } else {
            let travel: f32 = candidate
                .iter()
                .enumerate()
                .map(|(index, pitch)| {
                    // A triad/seventh change can add or remove the upper voice;
                    // compare the remaining voices without abandoning their line.
                    (pitch - previous[index.min(previous.len() - 1)]).abs()
                })
                .sum();
            let common = candidate
                .iter()
                .filter(|pitch| previous.contains(pitch))
                .count();
            travel - common as f32 * 0.75 + (centre - target).abs() * 0.08
        };
        if cost < best_cost {
            best_cost = cost;
            best = candidate.to_vec();
        }
    }
    best
}

fn cap_gates(events: &mut [NoteEvent], frames: usize) {
    let mut first = 0;
    while first < events.len() {
        let start = events[first].start_frame;
        let mut next = first + 1;
        while next < events.len() && events[next].start_frame == start {
            next += 1;
        }
        let next_attack = events.get(next).map(|event| event.start_frame);
        let end = next_attack.unwrap_or(frames);
        let available = end.saturating_sub(start) as f32 / crate::SAMPLE_RATE as f32;
        for event in &mut events[first..next] {
            event.gate_seconds = event.gate_seconds.min((available * 0.92).max(0.01));
            if event.instrument.is_monophonic()
                && let Some(next_attack) = next_attack
            {
                event.end_frame = Some(
                    event
                        .end_frame
                        .map_or(next_attack, |end| end.min(next_attack)),
                );
            }
        }
        first = next;
    }
}

impl Track {
    pub fn instrument(&self) -> Instrument {
        self.instrument
            .unwrap_or_else(|| Instrument::from_name(&self.name))
    }

    /// The operator's note-length multiplier, bounded so a score cannot ask for
    /// a note that outlives the piece.
    pub fn gate_scale(&self) -> f32 {
        self.gate.unwrap_or(1.0).clamp(0.1, 4.0)
    }
}

impl Score {
    /// How many voices sound simultaneously, and where the peaks fall.
    ///
    /// A hardware synth has a fixed voice pool — commonly 8 to 16 — and steals
    /// from it. This engine allocates a voice per note with no ceiling, so the
    /// only way to know the real load is to count it.
    pub fn voice_load_report(&self) -> String {
        use std::fmt::Write;

        let plan = self.arrange();
        let sr = crate::SAMPLE_RATE as usize;
        let frames = plan.frames;
        let window = sr / 20; // 50 ms resolution
        let slots = frames / window;
        let mut concurrent = vec![0usize; slots.max(1)];
        let mut per_role: std::collections::BTreeMap<String, usize> = Default::default();

        for event in &plan.events {
            let stop = event.start_frame
                + ((event.gate_seconds + event.instrument.patch().amp.tail()) * sr as f32) as usize;
            let stop = event.end_frame.map_or(stop, |end| stop.min(end));
            let first = event.start_frame / window;
            let last = (stop / window).min(slots.saturating_sub(1));
            for slot in first..=last {
                if let Some(count) = concurrent.get_mut(slot) {
                    *count += 1;
                }
            }
            let role = format!("{:?}", event.instrument);
            *per_role.entry(role).or_default() += 1;
        }

        let peak = concurrent.iter().copied().max().unwrap_or(0);
        let mean = concurrent.iter().sum::<usize>() as f32 / concurrent.len().max(1) as f32;
        let over_sixteen = concurrent.iter().filter(|count| **count > 16).count();
        let mut out = String::new();
        let _ = writeln!(
            out,
            "voices sounding at once: peak {peak}, mean {mean:.1}, above 16 for {:.0}% of the piece",
            100.0 * over_sixteen as f32 / concurrent.len().max(1) as f32
        );
        let _ = writeln!(out, "notes by role:");
        for (role, count) in per_role {
            let _ = writeln!(out, "  {role:<10} {count}");
        }
        out
    }

    /// A human-readable account of what the arranger scheduled.
    ///
    /// Audio problems are usually plan problems. Reading the plan directly is
    /// the difference between tuning a synthesiser and guessing at one.
    pub fn arrangement_report(&self) -> String {
        use std::fmt::Write;

        let plan = self.arrange_with_audit(true);
        let seconds = plan.frames as f32 / crate::SAMPLE_RATE as f32;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} — {} bpm, {} bars, {:.1}s, {} notes ({:.1}/s)",
            self.title,
            self.bpm,
            self.bars,
            seconds,
            plan.events.len(),
            plan.events.len() as f32 / seconds
        );

        let _ = writeln!(
            out,
            "\n{:<22} {:>6} {:>7} {:>8} {:>8}  distinct pitches",
            "track", "notes", "gate", "lowest", "highest"
        );
        for (index, track) in self.tracks.iter().enumerate() {
            let notes: Vec<_> = plan
                .events
                .iter()
                .filter(|event| event.track == index)
                .collect();
            let divisions = self.grid_divisions(track);
            let trace: Vec<_> = plan
                .trigger_audit
                .iter()
                .filter(|item| item.track == index)
                .collect();
            let written = trace
                .iter()
                .filter(|item| item.written_degree.is_some())
                .count();
            let scheduled = trace
                .iter()
                .filter(|item| {
                    item.written_degree.is_some() && item.outcome == NoteTriggerOutcome::Scheduled
                })
                .count();
            let generated = trace
                .iter()
                .filter(|item| item.generated && item.outcome == NoteTriggerOutcome::Scheduled)
                .count();
            let zero_velocity = trace
                .iter()
                .filter(|item| item.outcome == NoteTriggerOutcome::ZeroVelocity)
                .count();
            let muted = trace
                .iter()
                .filter(|item| item.outcome == NoteTriggerOutcome::MutedTrack)
                .count();
            let omitted = trace
                .iter()
                .filter(|item| {
                    matches!(
                        item.outcome,
                        NoteTriggerOutcome::LegacySection
                            | NoteTriggerOutcome::LegacyThinning
                            | NoteTriggerOutcome::LegacySpacing
                            | NoteTriggerOutcome::OutsideRender
                    )
                })
                .count();
            let _ = writeln!(
                out,
                "  {}: {scheduled}/{written} written attacks scheduled, {generated} generated, {zero_velocity} zero velocity, {muted} muted, {omitted} arranger omissions; {divisions} slots/bar",
                track.name
            );
            if notes.is_empty() {
                let _ = writeln!(out, "{:<22} {:>6}", track.name, 0);
                continue;
            }
            let mut pitches: Vec<i32> = notes.iter().map(|event| event.midi as i32).collect();
            pitches.sort_unstable();
            let lowest = *pitches.first().unwrap_or(&0);
            let highest = *pitches.last().unwrap_or(&0);
            pitches.dedup();
            let _ = writeln!(
                out,
                "{:<22} {:>6} {:>6.0}ms {:>8} {:>8}  {} pitches, {} voiced events",
                track.name,
                notes.len(),
                notes[0].gate_seconds * 1000.0,
                lowest,
                highest,
                pitches.len(),
                notes.len()
            );
        }

        // How many notes sound at once, sampled every half second. A piece that
        // is really only three notes shows up here immediately.
        let _ = writeln!(out, "\npolyphony over time (notes sounding, per 0.5s)");
        let window = crate::SAMPLE_RATE as usize / 2;
        let mut line = String::new();
        for slot in 0..(plan.frames / window) {
            let start = slot * window;
            let end = start + window;
            let sounding = plan
                .events
                .iter()
                .filter(|event| {
                    let stop = event.start_frame
                        + (event.gate_seconds * crate::SAMPLE_RATE as f32) as usize;
                    event.start_frame < end && stop > start
                })
                .count();
            line.push(match sounding {
                0 => '.',
                1..=2 => '_',
                3..=5 => '-',
                6..=9 => '=',
                _ => '#',
            });
        }
        let _ = writeln!(out, "{line}");
        let _ = writeln!(out, "  . none   _ 1-2   - 3-5   = 6-9   # 10+");
        out
    }
}

impl Mode {
    pub const fn scale(self) -> [u8; 7] {
        match self {
            Self::Major => [0, 2, 4, 5, 7, 9, 11],
            Self::Minor => [0, 2, 3, 5, 7, 8, 10],
            Self::Dorian => [0, 2, 3, 5, 7, 9, 10],
            Self::Mixolydian => [0, 2, 4, 5, 7, 9, 10],
            Self::Lydian => [0, 2, 4, 6, 7, 9, 11],
            Self::Phrygian => [0, 1, 3, 5, 7, 8, 10],
            Self::HarmonicMinor => [0, 2, 3, 5, 7, 8, 11],
        }
    }
}

impl ChordQuality {
    pub const fn intervals(self) -> &'static [u8] {
        match self {
            Self::Major => &[0, 4, 7],
            Self::Minor => &[0, 3, 7],
            Self::Diminished => &[0, 3, 6],
            Self::Augmented => &[0, 4, 8],
            Self::Suspended2 => &[0, 2, 7],
            Self::Suspended4 => &[0, 5, 7],
            Self::MajorSeventh => &[0, 4, 7, 11],
            Self::MinorSeventh => &[0, 3, 7, 10],
            Self::DominantSeventh => &[0, 4, 7, 10],
            Self::HalfDiminished => &[0, 3, 6, 10],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_dense_sustaining_parts_have_playable_attacks_and_no_gate_stacking() {
        for &instrument in Instrument::ALL
            .iter()
            .filter(|voice| voice.attack_spacing() > 0.0)
        {
            let mut score = crate::default_score();
            score.composition = None;
            score.bars = 4;
            score.tracks.truncate(1);
            score.tracks[0].instrument = Some(instrument);
            score.tracks[0].steps = vec![Some(0); 16];
            score.tracks[0].gate = Some(4.0);
            let plan = score.arrange();
            let min_frames =
                instrument.attack_spacing() * 60.0 / score.bpm as f32 * crate::SAMPLE_RATE as f32;
            assert!(plan.events.len() >= 8, "the part must remain audible");
            let mut attacks = plan.events.clone();
            attacks.dedup_by_key(|event| event.start_frame);
            for pair in attacks.windows(2) {
                let distance = (pair[1].start_frame - pair[0].start_frame) as f32;
                assert!(
                    distance >= min_frames * 0.96,
                    "attacks too close for {instrument:?}"
                );
                assert!(pair[0].gate_seconds * (crate::SAMPLE_RATE as f32) < distance);
            }
            assert!(
                plan.events.iter().all(|event| event.gate_seconds > 0.05),
                "simultaneous chord tones must not be shortened to zero"
            );
        }
    }

    #[test]
    fn reported_high_celesta_arp_line_keeps_its_contour_and_octave_relationship() {
        let degrees = [
            7, 9, 11, 13, 15, 17, 19, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4,
            3, 2, 1, 0, 1, 2, 3, 4,
        ];
        let mut score = solo_score(Instrument::Celesta);
        score.bars = 8;
        score.tonic = 7;
        score.mode = Mode::Dorian;
        score.progression.clear();
        score.composition.as_mut().unwrap().development = 0.0;
        score.composition.as_mut().unwrap().form = MusicalForm::Arc;
        score.tracks[0].octave = 5;
        score.tracks[0].steps = degrees
            .into_iter()
            .flat_map(|degree| [Some(degree), None])
            .collect();
        score.tracks[0].durations = Some(vec![0.5; 64]);
        let mut arp = score.tracks[0].clone();
        arp.instrument = Some(Instrument::Arp);
        score.tracks.push(arp);
        let plan = score.arrange();
        let played: Vec<Vec<f32>> = (0..2)
            .map(|track| {
                plan.events
                    .iter()
                    .filter(|event| event.track == track)
                    .map(|event| event.midi)
                    .collect()
            })
            .collect();
        let written: Vec<f32> = degrees
            .iter()
            .map(|degree| 79.0 + f32::from(scale_offset(score.mode, *degree)))
            .collect();
        assert_eq!(written.iter().copied().fold(f32::INFINITY, f32::min), 79.0);
        assert_eq!(written.iter().copied().fold(0.0, f32::max), 113.0);
        for (part, shift) in played.iter().zip([-24.0, -12.0]) {
            for (index, note) in part.iter().enumerate() {
                assert_eq!(*note, written[index % written.len()] + shift);
            }
            for (index, pair) in part.windows(2).enumerate() {
                assert_eq!(
                    pair[1] - pair[0],
                    written[(index + 1) % written.len()] - written[index % written.len()]
                );
            }
        }
        assert!(
            played[0]
                .iter()
                .zip(&played[1])
                .all(|(celesta, arp)| arp - celesta == 12.0)
        );
        let previous_wraps: Vec<_> = written
            .iter()
            .map(|note| fold_into_range(*note, Instrument::Celesta.natural_range()))
            .collect();
        assert!(
            previous_wraps[2] < previous_wraps[1],
            "fixture must reproduce the old descending leap in an ascending line"
        );
    }

    #[test]
    fn phrase_register_keeps_in_range_writing_exact_and_wide_phrases_in_safe_midi() {
        for &instrument in Instrument::ALL
            .iter()
            .filter(|voice| voice.follows_harmony() && !voice.is_chordal())
        {
            let mut score = solo_score(instrument);
            score.composition.as_mut().unwrap().development = 0.0;
            score.progression.clear();
            score.bars = 2;
            score.tracks[0].octave = 8;
            score.tracks[0].steps = vec![Some(0), Some(7), Some(14), Some(21)];
            let notes: Vec<_> = score
                .arrange()
                .events
                .iter()
                .map(|event| event.midi)
                .collect();
            assert!(
                notes.iter().all(|note| (0.0..=127.0).contains(note)),
                "{instrument:?}"
            );
            assert!(
                notes
                    .windows(2)
                    .all(|pair| matches!((pair[1] - pair[0]) as i32, 12 | -36)),
                "{instrument:?} changed written octave intervals"
            );
        }
        let mut score = solo_score(Instrument::Piano);
        score.composition.as_mut().unwrap().development = 0.0;
        score.tracks[0].steps = vec![Some(0), Some(2), Some(4), Some(7)];
        assert_eq!(
            score
                .arrange()
                .events
                .iter()
                .take(4)
                .map(|event| event.midi)
                .collect::<Vec<_>>(),
            [60.0, 64.0, 67.0, 72.0]
        );
    }

    #[test]
    fn explicit_quarter_grid_plays_a_sixty_four_slot_phrase_over_sixteen_bars() {
        let mut score = solo_score(Instrument::Piano);
        score.bars = 16;
        score.composition.as_mut().unwrap().form = MusicalForm::Arc;
        score.composition.as_mut().unwrap().development = 0.0;
        score.composition.as_mut().unwrap().steps_per_bar = 4;
        score.tracks[0].steps = (0..64).map(|index| Some((index / 4 % 7) as u8)).collect();
        let plan = score.arrange();
        assert_eq!(
            plan.events.len(),
            64,
            "the phrase must play once, not loop four times"
        );
        let beat_frames = 60.0 / score.bpm as f32 * crate::SAMPLE_RATE as f32;
        for pair in plan.events.windows(2) {
            let separation = (pair[1].start_frame - pair[0].start_frame) as f32;
            assert!((separation - beat_frames).abs() < beat_frames * 0.04);
        }
        let expected: Vec<_> = (0..64)
            .map(|index| 60.0 + f32::from(scale_offset(score.mode, (index / 4 % 7) as u8)))
            .collect();
        assert_eq!(
            plan.events
                .iter()
                .map(|event| event.midi)
                .collect::<Vec<_>>(),
            expected
        );
        let json = serde_json::to_string(&score).unwrap();
        let accepted = crate::Score::from_json(&json).unwrap();
        assert_eq!(accepted.composition_plan().steps_per_bar, 4);
        assert_eq!(accepted.arrange().events.len(), 64);
    }

    #[test]
    fn melodic_answers_keep_the_written_octave() {
        let score = crate::default_score();
        let track = &score.tracks[5];
        let chord = Some(Chord {
            root_degree: 0,
            quality: ChordQuality::Major,
        });
        for position in [0, 2, 4, 8, 14] {
            let low = score.pitch_for(track, Instrument::Flute, 0, chord, position, false);
            let high = score.pitch_for(track, Instrument::Flute, 7, chord, position, false);
            assert_eq!(high - low, 12.0, "position {position} collapsed the octave");
        }
    }

    fn solo_score(instrument: Instrument) -> Score {
        let mut score = crate::default_score();
        score.bpm = 120;
        score.bars = 12;
        score.tonic = 0;
        score.mode = Mode::Major;
        score.tracks.truncate(1);
        score.tracks[0].instrument = Some(instrument);
        score.tracks[0].octave = 4;
        score.tracks[0].gain = 0.5;
        score.tracks[0].steps = vec![Some(0)];
        score.progression = vec![Chord {
            root_degree: 0,
            quality: ChordQuality::Major,
        }];
        score.composition = Some(crate::CompositionPlan {
            form: MusicalForm::Ternary,
            development: 0.5,
            ..Default::default()
        });
        score
    }

    #[test]
    fn passing_and_neighbour_tones_survive_between_structural_beats() {
        let mut score = solo_score(Instrument::Piano);
        score.tracks[0].steps = [0, 1, 2, 3, 4, 3, 2, 1].into_iter().map(Some).collect();
        let plan = score.arrange();
        let first_bar: Vec<i32> = plan
            .events
            .iter()
            .take(8)
            .map(|event| event.midi as i32)
            .collect();
        assert_eq!(first_bar, [60, 62, 64, 65, 67, 65, 64, 62]);
        let phrase_final_note = plan.events[31].midi as i32;
        assert!(
            [0, 4, 7].contains(&phrase_final_note.rem_euclid(12)),
            "a phrase ending should resolve to its harmony"
        );
    }

    #[test]
    fn chordal_roles_sound_full_triads_and_sevenths_at_equal_power() {
        for &instrument in Instrument::ALL.iter().filter(|voice| voice.is_chordal()) {
            for quality in [ChordQuality::Major, ChordQuality::DominantSeventh] {
                let mut score = solo_score(instrument);
                score.progression[0].quality = quality;
                score.tracks[0].velocities = Some(vec![0.8]);
                let plan = score.arrange();
                let onset = plan.events[0].start_frame;
                let chord: Vec<_> = plan
                    .events
                    .iter()
                    .take_while(|event| event.start_frame == onset)
                    .collect();
                assert_eq!(chord.len(), quality.intervals().len());
                let mut classes: Vec<u8> =
                    chord.iter().map(|event| (event.midi as u8) % 12).collect();
                classes.sort_unstable();
                assert_eq!(classes, quality.intervals());
                assert!(
                    (chord
                        .iter()
                        .map(|event| event.velocity.powi(2))
                        .sum::<f32>()
                        - 0.64)
                        .abs()
                        < 0.0001
                );
                assert!(chord.iter().all(|event| event.gate_seconds > 1.0));
            }
        }
    }

    #[test]
    fn inversions_keep_common_tones_and_reduce_harmonic_travel() {
        let score = solo_score(Instrument::Organ);
        let track = &score.tracks[0];
        let first = voice_chord(
            &score,
            track,
            0,
            Chord {
                root_degree: 0,
                quality: ChordQuality::Major,
            },
            &[],
        );
        let second = voice_chord(
            &score,
            track,
            0,
            Chord {
                root_degree: 3,
                quality: ChordQuality::Major,
            },
            &first,
        );
        let travel: f32 = first.iter().zip(&second).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            travel <= 3.0,
            "I to IV should retain C and move E/F, G/A: {first:?} -> {second:?}"
        );
        assert!(first.iter().any(|pitch| second.contains(pitch)));
        let again = voice_chord(
            &score,
            track,
            0,
            Chord {
                root_degree: 0,
                quality: ChordQuality::Major,
            },
            &second,
        );
        assert_eq!(
            first, again,
            "returning harmony should not drift through registers"
        );
    }

    #[test]
    fn repeated_thirty_two_bar_harmony_stays_in_its_written_register() {
        let mut score = solo_score(Instrument::Keys);
        score.bars = 32;
        score.mode = Mode::Dorian;
        score.composition.as_mut().unwrap().form = MusicalForm::Arc;
        score.composition.as_mut().unwrap().development = 0.0;
        score.progression = [
            (0, ChordQuality::Minor),
            (4, ChordQuality::Minor),
            (0, ChordQuality::Minor),
            (3, ChordQuality::Major),
            (6, ChordQuality::Major),
            (3, ChordQuality::Major),
            (4, ChordQuality::Minor),
            (0, ChordQuality::Minor),
        ]
        .into_iter()
        .map(|(root_degree, quality)| Chord {
            root_degree,
            quality,
        })
        .collect();
        let plan = score.arrange();
        let chords: Vec<Vec<f32>> = plan
            .events
            .chunks_exact(3)
            .map(|events| {
                let mut pitches: Vec<_> = events.iter().map(|event| event.midi).collect();
                pitches.sort_by(f32::total_cmp);
                pitches
            })
            .collect();
        assert_eq!(chords.len(), 32);
        for bar in [0, 8, 16, 24, 31] {
            assert_eq!(
                chords[bar],
                [60.0, 63.0, 67.0],
                "tonic return at bar {} drifted into the lead",
                bar + 1
            );
        }
        assert!(
            chords
                .iter()
                .flatten()
                .all(|pitch| (55.0..=72.0).contains(pitch))
        );
        for pair in chords.windows(2) {
            let movement: f32 = pair[0]
                .iter()
                .zip(&pair[1])
                .map(|(a, b)| (a - b).abs())
                .sum();
            assert!(
                movement <= 9.0,
                "bounded register should still permit nearby voice leading: {pair:?}"
            );
        }
        // Register is authored, not permanently pinned to the first event.
        let mut raised = score.tracks[0].clone();
        raised.octave += 1;
        assert_eq!(
            voice_chord(&score, &raised, 0, score.progression[0], &chords[31]),
            [72.0, 75.0, 79.0]
        );
    }

    #[test]
    fn literal_plan_keeps_patch_filter_without_hidden_section_automation() {
        for &instrument in Instrument::ALL {
            let mut score = solo_score(instrument);
            score.bars = 32;
            score.composition.as_mut().unwrap().development = 0.0;
            score.composition.as_mut().unwrap().form = MusicalForm::Arc;
            assert!(
                score
                    .arrange()
                    .events
                    .iter()
                    .all(|event| event.cutoff_scale == 1.0),
                "{instrument:?}"
            );
            if !instrument.is_percussive() {
                score.composition.as_mut().unwrap().development = 1.0;
                assert!(
                    score
                        .arrange()
                        .events
                        .iter()
                        .any(|event| event.cutoff_scale != 1.0)
                );
            }
        }
    }

    #[test]
    fn long_final_polyphonic_notes_receive_their_full_render_tail() {
        for instrument in [Instrument::Piano, Instrument::Organ] {
            let mut score = solo_score(instrument);
            score.bpm = 40;
            score.bars = 1;
            score.composition.as_mut().unwrap().development = 0.0;
            score.tracks[0].steps = vec![None; 8];
            score.tracks[0].steps[7] = Some(0);
            score.tracks[0].durations = Some(vec![8.0; 8]);
            let plan = score.arrange();
            for event in &plan.events {
                assert_eq!(event.gate_seconds, 12.0);
                let voice = crate::synth::Voice::new(
                    instrument.patch(),
                    440.0,
                    0.5,
                    event.gate_seconds,
                    event.seed,
                );
                assert!(
                    event.start_frame + voice.frames() + 2 * crate::SAMPLE_RATE as usize
                        <= plan.frames + plan.tail_frames,
                    "{instrument:?}: full written gate, release and effect continuation must fit"
                );
            }
            assert!(
                plan.tail_frames > plan.frames,
                "fixture exercises wrapping more than one loop"
            );
            let audio = score.render().unwrap();
            assert_eq!(audio.len(), plan.frames * 2);
            assert!(audio.iter().all(|sample| sample.is_finite()));
        }
    }

    #[test]
    fn authored_dynamics_durations_and_zero_development_are_respected() {
        let mut score = solo_score(Instrument::Piano);
        score.bars = 8;
        score.composition.as_mut().unwrap().form = MusicalForm::Arc;
        score.composition.as_mut().unwrap().development = 0.0;
        score.tracks[0].steps = vec![Some(1), None, Some(3), Some(5)];
        score.tracks[0].velocities = Some(vec![0.25, 1.0, 0.75, 0.0]);
        score.tracks[0].durations = Some(vec![0.25, 1.0, 0.5, 4.0]);
        let plan = score.arrange();
        assert_eq!(
            plan.events.len(),
            16,
            "no sectional deletion, fill, or zero-velocity note"
        );
        for pair in plan.events.chunks_exact(2) {
            assert_eq!([pair[0].midi, pair[1].midi], [62.0, 65.0]);
            assert_eq!([pair[0].velocity, pair[1].velocity], [0.25, 0.75]);
            assert_eq!([pair[0].gate_seconds, pair[1].gate_seconds], [0.125, 0.25]);
        }
    }

    #[test]
    fn bass_can_walk_and_state_an_authored_inversion() {
        let mut score = solo_score(Instrument::Bass);
        score.tracks[0].octave = 2;
        score.tracks[0].steps = vec![Some(2), Some(3), Some(4), Some(5)];
        let plan = score.arrange();
        assert_eq!(
            plan.events
                .iter()
                .take(4)
                .map(|event| event.midi as i32)
                .collect::<Vec<_>>(),
            [40, 41, 43, 45],
            "E in the bass voices first inversion; F and A remain walking tones"
        );
        score.tracks[0].instrument = Some(Instrument::SubBass);
        score.tracks[0].steps = vec![Some(1), Some(2), Some(3), Some(4)];
        score.composition.as_mut().unwrap().development = 0.0;
        let plan = score.arrange();
        assert_eq!(
            plan.events
                .iter()
                .take(4)
                .map(|event| event.midi as i32)
                .collect::<Vec<_>>(),
            [38, 40, 41, 43]
        );
    }

    #[test]
    fn explicit_polyphonic_chord_lengths_survive_following_attacks() {
        let mut score = solo_score(Instrument::Keys);
        score.composition.as_mut().unwrap().development = 0.0;
        score.tracks[0].steps = vec![Some(0), Some(0)];
        score.tracks[0].durations = Some(vec![8.0, 8.0]);
        let plan = score.arrange();
        for chord in plan.events.chunks_exact(3).take(4) {
            assert!(
                chord
                    .iter()
                    .all(|event| event.gate_seconds == 4.0 && event.end_frame.is_none())
            );
        }
    }

    #[test]
    fn explicit_one_second_piano_notes_overlap_half_second_attacks() {
        let mut score = solo_score(Instrument::Piano);
        score.composition.as_mut().unwrap().development = 0.0;
        score.composition.as_mut().unwrap().form = MusicalForm::Arc;
        score.tracks[0].steps = vec![Some(0), Some(2), Some(4), Some(2)];
        score.tracks[0].durations = Some(vec![2.0; 4]);
        let plan = score.arrange();
        assert!(
            plan.events
                .iter()
                .all(|event| event.gate_seconds == 1.0 && event.end_frame.is_none())
        );
        for pair in plan.events.windows(2) {
            let separation =
                (pair[1].start_frame - pair[0].start_frame) as f32 / crate::SAMPLE_RATE as f32;
            assert!((0.48..0.52).contains(&separation));
            assert!(pair[0].gate_seconds > separation);
        }
    }

    #[test]
    fn trigger_trace_matches_actual_voices_and_monophonic_caps() {
        for &instrument in Instrument::ALL {
            let mut score = solo_score(instrument);
            score.composition.as_mut().unwrap().development = 0.0;
            score.tracks[0].steps = vec![Some(0); 16];
            score.tracks[0].durations = Some(vec![2.0; 16]);
            let plan = score.arrange_with_audit(true);
            let ordinary = score.arrange();
            assert_eq!(ordinary.events.len(), plan.events.len());
            assert!(
                ordinary.trigger_audit.is_empty(),
                "ordinary renders do not allocate traces"
            );
            assert_eq!(
                plan.events.len(),
                plan.trigger_audit
                    .iter()
                    .map(|entry| entry.voiced_events)
                    .sum::<usize>()
            );
            for entry in &plan.trigger_audit {
                assert_eq!(entry.outcome, NoteTriggerOutcome::Scheduled);
                assert_eq!(
                    entry.voiced_events,
                    plan.events
                        .iter()
                        .filter(|event| Some(event.start_frame) == entry.start_frame)
                        .count()
                );
            }
            if instrument.is_monophonic() {
                for pair in plan.events.windows(2) {
                    assert!(
                        pair[0]
                            .end_frame
                            .is_some_and(|end| end <= pair[1].start_frame)
                    );
                    assert!(
                        pair[0].gate_seconds * (crate::SAMPLE_RATE as f32)
                            < (pair[1].start_frame - pair[0].start_frame) as f32
                    );
                }
            }
        }
    }

    #[test]
    fn sparse_chords_release_conflicting_voices_at_harmonic_changes() {
        let mut score = solo_score(Instrument::Pad);
        score.bars = 4;
        score.composition.as_mut().unwrap().form = MusicalForm::Arc;
        score.progression = [0, 3, 4, 0]
            .into_iter()
            .map(|root_degree| Chord {
                root_degree,
                quality: ChordQuality::Major,
            })
            .collect();
        score.tracks[0].steps = vec![None; 64];
        score.tracks[0].steps[0] = Some(0);
        score.tracks[0].durations = Some(vec![16.0; 64]);
        let plan = score.arrange();
        assert_eq!(plan.events.len(), 3);
        let bar_frames = crate::SAMPLE_RATE as usize * 2;
        for event in &plan.events {
            let common_with_f_major = event.midi as i16 % 12 == 0;
            let expected_end = bar_frames * if common_with_f_major { 2 } else { 1 };
            assert_eq!(event.end_frame, Some(expected_end));
            assert!(
                event.start_frame + ((event.gate_seconds * crate::SAMPLE_RATE as f32) as usize)
                    < expected_end,
                "the gate must leave time for release before the harmonic boundary"
            );
        }
    }

    #[test]
    fn common_tones_and_unchanged_chords_keep_their_sustain() {
        let c_major = (1 << 0) | (1 << 4) | (1 << 7);
        let f_major = (1 << 0) | (1 << 5) | (1 << 9);
        let g_major = (1 << 7) | (1 << 11) | (1 << 2);
        assert_eq!(
            next_conflicting_harmony(64.0, 0, &[c_major, c_major, f_major]),
            Some(2)
        );
        assert_eq!(
            next_conflicting_harmony(60.0, 0, &[c_major, f_major, g_major]),
            Some(2)
        );
        assert_eq!(
            next_conflicting_harmony(60.0, 0, &[c_major, f_major, c_major]),
            None
        );
        // A written suspension can resolve into one chord and become a clash
        // again when the earlier harmony returns.
        assert_eq!(
            next_conflicting_harmony(65.0, 0, &[c_major, f_major, c_major]),
            Some(2)
        );
    }

    #[test]
    fn long_offbeat_notes_arrive_in_harmony_while_brief_passing_tones_survive() {
        let mut score = solo_score(Instrument::Piano);
        score.tracks[0].steps = vec![None, Some(3), None, None];
        score.tracks[0].durations = Some(vec![4.0; 4]);
        let long = score.arrange().events[0];
        assert_eq!(long.midi, 64.0, "held F should resolve to E over C major");
        score.tracks[0].durations = Some(vec![0.25; 4]);
        assert_eq!(score.arrange().events[0].midi, 65.0);
        score.tracks[0].durations = Some(vec![4.0; 4]);
        score.composition.as_mut().unwrap().development = 0.0;
        assert_eq!(
            score.arrange().events[0].midi,
            65.0,
            "literal writing is preserved"
        );
    }

    #[test]
    fn literal_and_chromatic_plans_preserve_authored_suspensions() {
        let mut score = solo_score(Instrument::Pad);
        score.progression.push(Chord {
            root_degree: 4,
            quality: ChordQuality::Major,
        });
        for (development, harmony) in [
            (0.0, HarmonicPolicy::Diatonic),
            (0.5, HarmonicPolicy::Chromatic),
        ] {
            score.composition.as_mut().unwrap().development = development;
            score.composition.as_mut().unwrap().harmony = harmony;
            assert!(
                score
                    .arrange()
                    .events
                    .iter()
                    .all(|event| event.end_frame.is_none())
            );
        }
    }

    #[test]
    fn single_line_releases_end_before_the_next_pitch() {
        for &instrument in Instrument::ALL.iter().filter(|voice| voice.is_monophonic()) {
            let mut score = solo_score(instrument);
            score.bars = 4;
            score.bpm = 96;
            score.composition.as_mut().unwrap().development = 0.0;
            score.tracks[0].steps = (0..8).map(|step| Some(5 + (step % 2) as u8)).collect();
            score.tracks[0].durations = Some(vec![0.5; 8]);
            let plan = score.arrange();
            for pair in plan.events.windows(2) {
                assert_eq!(
                    pair[0].end_frame,
                    Some(pair[1].start_frame),
                    "{instrument:?}"
                );
            }
            assert!(
                plan.events.last().unwrap().end_frame.is_none(),
                "the final natural release remains audible"
            );
        }
    }

    #[test]
    fn struck_and_plucked_voices_keep_their_overlapping_releases() {
        for &instrument in Instrument::ALL
            .iter()
            .filter(|voice| !voice.is_monophonic() && !voice.is_chordal() && !voice.is_percussive())
        {
            let mut score = solo_score(instrument);
            score.composition.as_mut().unwrap().development = 0.0;
            score.tracks[0].steps = vec![Some(0); 8];
            score.tracks[0].durations = Some(vec![0.5; 8]);
            assert!(
                score
                    .arrange()
                    .events
                    .iter()
                    .all(|event| event.end_frame.is_none()),
                "{instrument:?}"
            );
        }
    }

    #[test]
    fn every_unpitched_instrument_keeps_its_tuning_when_the_key_changes() {
        for &instrument in Instrument::ALL
            .iter()
            .filter(|voice| !voice.follows_harmony())
        {
            let mut score = solo_score(instrument);
            score.composition.as_mut().unwrap().development = 0.0;
            score.tracks[0].steps = vec![Some(0), Some(2), Some(4), None];
            score.tonic = 0;
            let original = score.arrange();
            score.tonic = 11;
            let transposed = score.arrange();
            assert!(
                !original.events.is_empty(),
                "{instrument:?} must be playable"
            );
            assert_eq!(
                original
                    .events
                    .iter()
                    .map(|event| event.midi)
                    .collect::<Vec<_>>(),
                transposed
                    .events
                    .iter()
                    .map(|event| event.midi)
                    .collect::<Vec<_>>(),
                "unpitched {instrument:?} followed the harmony"
            );
            let (low, high) = instrument.natural_range();
            assert!(
                original
                    .events
                    .iter()
                    .all(|event| (f32::from(low)..=f32::from(high)).contains(&event.midi))
            );
        }
    }

    #[test]
    fn a_harmony_cut_breaks_theremin_legato_even_with_a_long_written_duration() {
        let mut score = solo_score(Instrument::Theremin);
        score.bars = 2;
        score.composition.as_mut().unwrap().form = MusicalForm::Arc;
        score.composition.as_mut().unwrap().chord_bars = 1;
        score.tracks[0].steps = vec![Some(0), None];
        score.tracks[0].durations = Some(vec![8.0, 0.5]);
        score.progression.push(Chord {
            root_degree: 1,
            quality: ChordQuality::Minor,
        });
        let plan = score.arrange();
        assert_eq!(plan.events.len(), 2);
        assert!(plan.events[0].end_frame.is_some());
        assert!(
            plan.events[1].glide_from_midi.is_none(),
            "a harmonic cut must not slide the old pitch into the new harmony"
        );
    }

    #[test]
    fn tuned_timpani_follows_the_written_key_without_becoming_a_chord() {
        let mut score = solo_score(Instrument::Timpani);
        score.composition.as_mut().unwrap().development = 0.0;
        score.tracks[0].steps = vec![Some(0), None, None, None];
        score.tracks[0].octave = 3;
        score.tonic = 0;
        let original = score.arrange();
        score.tonic = 2;
        let transposed = score.arrange();
        assert_eq!(original.events.len(), transposed.events.len());
        for (before, after) in original.events.iter().zip(&transposed.events) {
            assert_eq!(after.midi - before.midi, 2.0);
        }
        assert!(
            original
                .events
                .windows(2)
                .all(|pair| pair[0].start_frame != pair[1].start_frame)
        );
    }

    #[test]
    fn zero_velocity_slots_do_not_hide_the_last_audible_cadence_note() {
        let mut score = solo_score(Instrument::Piano);
        score.bars = 4;
        score.composition.as_mut().unwrap().form = MusicalForm::Arc;
        score.tracks[0].steps = vec![Some(0), Some(1), Some(1), Some(1)];
        score.tracks[0].velocities = Some(vec![1.0, 1.0, 0.0, 0.0]);
        let plan = score.arrange();
        assert_eq!(plan.events.last().unwrap().midi, 60.0);
    }

    #[test]
    fn planned_forms_return_the_motif_and_obey_harmonic_rhythm() {
        let mut score = solo_score(Instrument::Piano);
        score.bars = 20;
        score.composition.as_mut().unwrap().form = MusicalForm::Rondo;
        score.composition.as_mut().unwrap().development = 0.8;
        score.composition.as_mut().unwrap().chord_bars = 2;
        score.progression.push(Chord {
            root_degree: 4,
            quality: ChordQuality::DominantSeventh,
        });
        assert_eq!(
            (0..5)
                .map(|region| score.theme_at(region * 4))
                .collect::<Vec<_>>(),
            [0, 1, 0, 2, 0]
        );
        assert_eq!(
            (0..6)
                .map(|bar| score.chord_at(bar).unwrap().root_degree)
                .collect::<Vec<_>>(),
            [0, 0, 4, 4, 0, 0]
        );
        score.tracks[0].steps = vec![Some(2), Some(3), Some(4), Some(2)];
        let plan = score.arrange();
        let phrase = |start_bar: usize| {
            plan.events
                .iter()
                .skip(start_bar * 4)
                .take(16)
                .map(|event| (event.midi as i32, event.velocity.to_bits(), event.seed))
                .collect::<Vec<_>>()
        };
        // Seeds identify individual voices, while pitch and expression make
        // the return perceptually recognisable.
        let pitches = |start_bar| {
            phrase(start_bar)
                .into_iter()
                .map(|(pitch, velocity, _)| (pitch, velocity))
                .collect::<Vec<_>>()
        };
        assert_eq!(pitches(0), pitches(8));
        assert_eq!(pitches(0), pitches(16));
        assert_ne!(pitches(0), pitches(4));
        assert_ne!(pitches(0), pitches(12));
        score.composition.as_mut().unwrap().form = MusicalForm::Ternary;
        score.bars = 12;
        assert_eq!(
            [score.theme_at(0), score.theme_at(4), score.theme_at(8)],
            [0, 1, 0]
        );
        score.composition.as_mut().unwrap().form = MusicalForm::Binary;
        assert_eq!([score.theme_at(0), score.theme_at(6)], [0, 1]);
    }

    #[test]
    fn sparse_textures_keep_audible_parts() {
        for instrument in [Instrument::Flute, Instrument::Kick] {
            for texture in [Texture::Chamber, Texture::Ambient] {
                let mut score = solo_score(instrument);
                score.composition.as_mut().unwrap().texture = texture;
                score.composition.as_mut().unwrap().form = MusicalForm::Arc;
                let plan = score.arrange();
                assert!(
                    plan.events.len() >= usize::from(score.bars),
                    "{texture:?} silenced {instrument:?}"
                );
            }
        }
    }

    #[test]
    fn chamber_and_ambient_textures_preserve_authored_duets_and_percussion() {
        let mut score = solo_score(Instrument::Harp);
        let mut flute = score.tracks[0].clone();
        flute.instrument = Some(Instrument::Flute);
        score.tracks.push(flute);
        let mut percussion = score.tracks[0].clone();
        percussion.instrument = Some(Instrument::Perc);
        score.tracks.push(percussion);
        for texture in [Texture::Chamber, Texture::Ambient] {
            score.composition.as_mut().unwrap().texture = texture;
            let plan = score.arrange();
            for track in 0..3 {
                assert!(
                    plan.events.iter().any(|event| event.track == track),
                    "{texture:?} discarded track {track}"
                );
            }
        }
    }

    #[test]
    fn explicit_form_boundaries_follow_phrases_and_restart_material() {
        let mut score = solo_score(Instrument::Piano);
        score.bars = 16;
        assert_eq!(
            score.form_timeline(),
            vec![
                ("A".into(), 0, 4),
                ("B".into(), 4, 12),
                ("A".into(), 12, 16)
            ]
        );
        score.composition.as_mut().unwrap().form = MusicalForm::Binary;
        assert_eq!(
            score.form_timeline(),
            vec![("A".into(), 0, 8), ("B".into(), 8, 16)]
        );
        score.composition.as_mut().unwrap().form = MusicalForm::Rondo;
        score.bars = 32;
        assert_eq!(
            score.form_timeline(),
            vec![
                ("A".into(), 0, 8),
                ("B".into(), 8, 12),
                ("A".into(), 12, 16),
                ("C".into(), 16, 24),
                ("A".into(), 24, 32)
            ]
        );
        score.composition.as_mut().unwrap().chord_bars = 2;
        score.progression = [0, 3, 4, 0]
            .into_iter()
            .map(|root_degree| Chord {
                root_degree,
                quality: ChordQuality::Major,
            })
            .collect();
        let harmony = score.harmonic_timeline();
        assert_eq!(&harmony[0..4], &harmony[12..16]);
        // An eight-bar written phrase cannot rely on absolute pattern phase:
        // the middle A starts at bar12, halfway around that eight-bar cycle.
        score.tracks[0].steps = (0..128)
            .map(|step| Some(((step / 16 + step % 3) % 7) as u8))
            .collect();
        let plan = score.arrange();
        let first: Vec<_> = plan
            .events
            .iter()
            .take(64)
            .map(|event| event.midi as i32)
            .collect();
        let returned: Vec<_> = plan
            .events
            .iter()
            .skip(12 * 16)
            .take(64)
            .map(|event| event.midi as i32)
            .collect();
        assert_eq!(first, returned);
    }

    #[test]
    fn incomplete_closing_phrases_do_not_shift_internal_form_boundaries() {
        let mut score = solo_score(Instrument::Piano);
        score.bars = 14;
        assert_eq!(
            score.form_timeline(),
            vec![("A".into(), 0, 4), ("B".into(), 4, 8), ("A".into(), 8, 14),]
        );
        for (form, bars) in [
            (MusicalForm::Binary, 15),
            (MusicalForm::Ternary, 23),
            (MusicalForm::Rondo, 31),
        ] {
            score.bars = bars;
            score.composition.as_mut().unwrap().form = form;
            let timeline = score.form_timeline();
            assert_eq!(timeline.last().unwrap().2, usize::from(bars));
            assert!(timeline.iter().all(|(_, start, _)| start.is_multiple_of(4)));
        }
    }
}
