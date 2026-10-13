//! Score-derived compositional analysis and bounded thematic development.
//!
//! This is an account of authored notes and intentions, not an audio critic.
//! Harmonic functions are suggestions in the common-practice tonal vocabulary;
//! modal, ambient, and chromatic writing are deliberate alternatives.

use serde::{Deserialize, Serialize};

use crate::{Chord, ChordQuality, Instrument, InstrumentRole, Mode, Score, Track};

/// Original working guidance for the Organist, not an imported textbook.
/// Background references: Open Music Theory's harmonic functions and cadence
/// chapters, and University of Puget Sound's treatment of phrase structure.
pub const THEORY_GUIDE: &str = "COMPOSITION WORKBENCH\n\
Choose an intention, a recognisable motif, a phrase length, and a form before filling tracks. \
Do not substitute a repeating scale or fixed-interval walk for a composed melody. Use a brief scalar gesture inside a distinctive motif, then give it an answering phrase and an arrival with different rhythm or contour. \
Scale degrees in score.steps are zero-based: 0 is tonic, 2 the third, 4 the fifth, 7 the next tonic; null is a rest. \
An interval is a relationship between notes, not a random new pitch. Repeat the opening cell; develop an answering cell by diatonic sequence, contour inversion, fragmentation, rhythmic displacement, or augmentation. Preserve enough rhythm and contour for the idea to remain recognisable.\n\
Give harmony a direction when desired: tonic establishes home, predominant prepares motion, dominant creates an expectation of tonic. \
Major V or V7 contains a leading tone; natural-minor v does not have the same leading-tone pull. \
A V-I progression alone does not prove a perfect authentic cadence: phrase placement, bass position, and the melody's arrival matter. Modal loops and drones can close through melody, repetition, and rhythm instead.\n\
Voice-leading means following each individual line through changing chords: keep common tones, prefer nearby notes, resolve a chordal seventh down by step when using tonal syntax, and let contrasting lines sometimes move in opposite directions. \
Passing and neighbour notes can connect chord tones; avoid making every melody note a chord tone. \
Leave rests for another voice to answer, keep registers distinct, and use dissonance with a clear purpose.\n\
Try A-B contrast, A-B-A return, or A-B-A-C-A rondo; make the return recognisable. \
Change one main musical idea per experiment and compare the resulting score and verified render with the preceding version. \
Chamber and ambient pieces need not have drums. Chromatic harmony is permitted when explicitly chosen; explain the colour or destination.";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicalForm {
    #[default]
    Arc,
    Binary,
    Ternary,
    Rondo,
}

impl MusicalForm {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Arc => "arc (opening, build, contrast, return)",
            Self::Binary => "binary (A-B)",
            Self::Ternary => "ternary (A-B-A)",
            Self::Rondo => "rondo (A-B-A-C-A)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Texture {
    #[default]
    Ensemble,
    Chamber,
    Ambient,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarmonicPolicy {
    #[default]
    Diatonic,
    Chromatic,
}

/// Explicit musical decisions interpreted by the native arranger.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct CompositionPlan {
    pub form: MusicalForm,
    pub phrase_bars: u8,
    pub chord_bars: u8,
    /// Written grid divisions per bar: four, eight, or sixteen. Eight permits
    /// a complete eight-bar idea within the bounded sixty-four-slot pattern.
    pub steps_per_bar: u8,
    pub development: f32,
    pub texture: Texture,
    pub harmony: HarmonicPolicy,
}

impl Default for CompositionPlan {
    fn default() -> Self {
        Self {
            form: MusicalForm::Arc,
            phrase_bars: 4,
            chord_bars: 1,
            steps_per_bar: 16,
            development: 0.35,
            texture: Texture::Ensemble,
            harmony: HarmonicPolicy::Diatonic,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HarmonicAnalysis {
    pub roman: String,
    /// A contextual suggestion, not a claim about every style's syntax.
    pub function: String,
    /// Enharmonic pitch-class names, not engraved note spellings.
    pub chord_tones: Vec<String>,
    pub chromatic_tones: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MotifAnalysis {
    pub track: String,
    /// Authored non-rest slots; the renderer can articulate or thin the part.
    pub sounded_notes: usize,
    pub step_count: usize,
    /// Signed diatonic distances between consecutive authored notes, including
    /// across rests. +1 means up one scale step, not up one semitone.
    pub intervals: Vec<i16>,
    pub contour: String,
    pub rest_fraction: f32,
    /// The shortest exact repeating cell, including its rests, when present.
    pub smallest_repeat_steps: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TheoryReport {
    pub key: String,
    pub scale: Vec<String>,
    pub progression: Vec<HarmonicAnalysis>,
    pub cadence: String,
    pub motif: Option<MotifAnalysis>,
    pub rhythm: String,
    pub form: String,
    pub voice_leading: Vec<String>,
    pub next_experiment: String,
    pub observations: Vec<String>,
}

impl Score {
    pub fn composition_plan(&self) -> CompositionPlan {
        self.composition.unwrap_or_default()
    }

    /// The authored grid, shared by arrangement, diagnostics and Studio.
    pub fn grid_divisions(&self, track: &Track) -> usize {
        track
            .steps
            .len()
            .clamp(1, usize::from(self.composition_plan().steps_per_bar.max(1)))
    }

    pub fn theory_report(&self) -> TheoryReport {
        let plan = self.composition_plan();
        let mode = self.mode.scale();
        let scale = mode
            .iter()
            .map(|offset| pitch_name(self.tonic.wrapping_add(*offset)))
            .collect();
        let progression: Vec<_> = self
            .progression
            .iter()
            .map(|chord| analyse_chord(self, *chord))
            .collect();
        let motif = self
            .tracks
            .iter()
            .filter(|track| {
                track.gain > 0.001
                    && !track.instrument().is_percussive()
                    && track.active_note_count() > 0
            })
            .max_by_key(|track| motif_priority(track))
            .map(analyse_motif);

        let active: Vec<_> = self
            .tracks
            .iter()
            .filter(|track| track.gain > 0.001 && track.active_note_count() > 0)
            .collect();
        let slots = active.iter().map(|track| track.steps.len()).sum::<usize>();
        let rests = active
            .iter()
            .map(|track| track.steps.len() - track.active_note_count())
            .sum::<usize>();
        let percussion = active
            .iter()
            .filter(|track| track.instrument().is_percussive())
            .count();
        let rhythm = format!(
            "4/4 at {} BPM; {} active authored parts, {} percussion; {:.0}% rest slots; swing {:.2}. Patterns of at most {} slots span one bar; longer patterns use that many slots per bar.",
            self.bpm,
            active.len(),
            percussion,
            100.0 * rests as f32 / slots.max(1) as f32,
            self.swing(),
            plan.steps_per_bar
        );
        let form = format!(
            "{}; {} bars; {}-bar phrases; one chord per {} bar(s); {:.0}% development; {:?} texture, {:?} harmony",
            plan.form.label(),
            self.bars,
            plan.phrase_bars,
            plan.chord_bars,
            plan.development * 100.0,
            plan.texture,
            plan.harmony
        );
        let mut observations = vec![
            "This describes the authored score, treating zero-velocity notes as silence; it does not measure the audio or certify a perfect cadence. Roman numerals use the parallel-major reference (b/# marks altered scale degrees).".to_owned(),
        ];
        observations.extend(self.orchestration_concerns());
        if !matches!(self.mode, Mode::Major | Mode::Minor | Mode::HarmonicMinor) {
            observations.push("The mode can establish home through a pedal, characteristic scale tone, and recurring motif; dominant-tonic syntax is optional.".to_owned());
        }
        for chord in &progression {
            if !chord.chromatic_tones.is_empty() {
                observations.push(format!(
                    "{} contains {} outside the chosen scale; {}.",
                    chord.roman, chord.chromatic_tones.join(", "),
                    match plan.harmony {
                        HarmonicPolicy::Diatonic => "the diatonic plan should correct this or explicitly choose chromatic harmony",
                        HarmonicPolicy::Chromatic => "retain it if its colour or resolution serves the experiment",
                    }
                ));
            }
        }
        let cadence = authored_cadence(self, &progression);
        let voice_leading = voice_leading_guidance(self, &progression, motif.as_ref());
        let next_experiment = next_experiment(self, &progression, motif.as_ref());
        TheoryReport {
            key: format!("{} {}", pitch_name(self.tonic), mode_label(self.mode)),
            scale,
            progression,
            cadence,
            motif,
            rhythm,
            form,
            voice_leading,
            next_experiment,
            observations,
        }
    }

    /// A bounded, concrete composition brief for model prompts and diagnostics.
    pub fn theory_context(&self) -> String {
        let report = self.theory_report();
        let harmony = report
            .progression
            .iter()
            .map(|chord| {
                format!(
                    "{} [{}; {}]",
                    chord.roman,
                    chord.function,
                    chord.chord_tones.join("/")
                )
            })
            .collect::<Vec<_>>()
            .join(" -> ");
        let motif = report.motif.as_ref().map_or_else(
            || "No active pitched motif is authored.".to_owned(),
            |motif| {
                let intervals = motif.intervals.iter().take(16).map(|interval| format!("{interval:+}")).collect::<Vec<_>>().join(",");
                let repeat = motif.smallest_repeat_steps.map_or_else(
                    || "no shorter exact repeat".to_owned(),
                    |steps| format!("exact {steps}-slot repeating cell")
                );
                format!("{}: {}/{} written note slots, {:.0}% rests; {} contour; scale-step intervals [{}{}]; {}.",
                    motif.track, motif.sounded_notes, motif.step_count, motif.rest_fraction * 100.0,
                    motif.contour, intervals, if motif.intervals.len() > 16 { ",..." } else { "" }, repeat)
            }
        );
        let roster = self
            .tracks
            .iter()
            .map(Track::instrument)
            .collect::<Vec<_>>();
        let current_voices = roster
            .iter()
            .map(|voice| voice.id())
            .collect::<Vec<_>>()
            .join(", ");
        let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
        for &voice in Instrument::ALL {
            let info = voice.info();
            let role = match info.role {
                InstrumentRole::Bass => "bass",
                InstrumentRole::Harmony => "harmony",
                InstrumentRole::Melody => "melody",
                InstrumentRole::Colour => "colour",
                InstrumentRole::Percussion => "percussion",
            };
            let entry = format!(
                "{} [{role}, {}..{}{}{}]",
                info.id,
                info.range[0],
                info.range[1],
                if info.attack_spacing > 0.0 {
                    format!(", attacks >= {} beats", info.attack_spacing)
                } else {
                    String::new()
                },
                if roster.contains(&voice) {
                    ", current"
                } else {
                    ""
                }
            );
            if let Some((_, entries)) = groups.iter_mut().find(|(group, _)| *group == info.group) {
                entries.push(entry);
            } else {
                groups.push((info.group, vec![entry]));
            }
        }
        let catalog = groups
            .into_iter()
            .map(|(group, entries)| format!("{group}: {}", entries.join("; ")))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "AUTHORED MUSIC THEORY\nKey: {}; scale pitch classes {}.\nPlan: {}.\nHarmony: {}.\nCadence: {}\nMotif: {}\nRhythm: {}\nVoice-leading: {}\nObservations: {}\nNext experiment: {}\nCurrent instruments: {current_voices}.\nCOMPLETE PLAYABLE INSTRUMENT CATALOG ({} synthesized voices; exact score instrument IDs)\n{catalog}\nBracket ranges are sounding MIDI notes, not scale degrees; C4=60, and score octave=4 anchors degree 0 near C4 plus tonic. Keep the whole phrase in range instead of relying on register correction. Bass grounds the harmony; harmony voices play chord voicings; melody voices carry independent lines; colour voices articulate answers or punctuation; percussion shapes rhythm. Leave room for slower strings, winds and pads. Theremin is a single-note singing electronic voice. Musical coherence and listener feedback take priority over novelty: the full catalog is available without any quota to change instruments or try unused families. Preserve a suitable ensemble while improving its notes and phrasing. Introduce another voice only for a clear musical role or an explicit request. Preserve requested instruments and the identity of a variation; never fill eight tracks merely to use the catalog.",
            report.key,
            report.scale.join(" "),
            report.form,
            if harmony.is_empty() {
                "no authored chord progression"
            } else {
                &harmony
            },
            report.cadence,
            motif,
            report.rhythm,
            report.voice_leading.join(" "),
            report.observations.join(" "),
            report.next_experiment,
            Instrument::ALL.len()
        )
    }

    /// Concrete notation concerns, not listening judgments or universal bans.
    pub fn orchestration_concerns(&self) -> Vec<String> {
        let pitched: Vec<_> = self
            .tracks
            .iter()
            .filter(|track| {
                track.gain > 0.001
                    && track.active_note_count() > 0
                    && !track.instrument().is_percussive()
            })
            .collect();
        let mut concerns = self.scalar_exercise_concerns();
        for track in &pitched {
            if let Some(concern) = attack_spacing_concern(self, track) {
                concerns.push(concern);
            }
            let (low, high, corrected, count) = self.written_register(track);
            if corrected > 0 {
                let range = track.instrument().info().range;
                concerns.push(format!("REGISTER: {} writes MIDI {low}..{high}; {corrected}/{count} sounding notes lie outside {}'s {}..{} range. The renderer would move the whole line to fit; choose the intended octave or reshape it.", track.name, track.instrument().id(), range[0], range[1]));
            }
            if self.composition_plan().texture != Texture::Ambient
                && matches!(
                    track.instrument().role(),
                    InstrumentRole::Melody | InstrumentRole::Colour
                )
                && track.active_note_count() >= 4
                && self.longest_written_gap(track) < 0.25
            {
                concerns.push(format!("PHRASE SPACE: {} has no written release gap of at least a quarter beat. Null slots can be filled by held notes; shorten a phrase-ending note or leave a longer pause before adding another line.", track.name));
            }
        }
        for (index, left) in pitched.iter().enumerate() {
            for right in pitched.iter().skip(index + 1) {
                if same_written_line(left, right) {
                    concerns.push(format!("DOUBLING: {} and {} repeat the same pitches and onsets in the same register. Panning and different patch names do not create independent parts. Keep deliberate doubling only if it serves the sound; otherwise give one part a simpler contrasting role or remove it.", left.name, right.name));
                }
            }
        }
        concerns
    }

    /// Automatic-composer guards for unplayable attacks, repeated scalar
    /// exercises and duplicated out-of-range melodies. Saved literal scores
    /// remain loadable independently of these generation checks.
    pub fn automatic_composition_blockers(&self) -> Vec<String> {
        let pitched: Vec<_> = self
            .tracks
            .iter()
            .filter(|track| {
                track.gain > 0.001
                    && track.active_note_count() > 0
                    && !track.instrument().is_percussive()
            })
            .collect();
        let mut blockers = Vec::new();
        if pitched.len() >= 2
            && pitched.iter().all(|track| {
                !track.instrument().is_chordal()
                    && track.instrument().role() != InstrumentRole::Bass
                    && track.active_note_count() >= 8
            })
            && pitched
                .iter()
                .skip(1)
                .all(|track| same_written_line(pitched[0], track))
            && pitched
                .iter()
                .any(|track| self.written_register(track).2 > 0)
        {
            blockers.push("The entire pitched ensemble duplicates one out-of-range melody without an independent supporting part. Correct the written register and give the parts purposeful roles before returning a complete score; changing names, pan or instruments alone does not repair these notes.".to_owned());
        }
        blockers.extend(self.scalar_exercise_concerns());
        blockers.extend(
            pitched
                .into_iter()
                .filter_map(|track| attack_spacing_concern(self, track)),
        );
        blockers
    }

    /// A narrow guard against presenting a repeating practice scale as a new
    /// composition. It evaluates actual note relationships and rhythm, so
    /// changing octaves, names, patches, or adding a chord track cannot repair
    /// it. Short scalar gestures and independent arpeggio support remain valid.
    pub fn scalar_exercise_concerns(&self) -> Vec<String> {
        let foreground: Vec<_> = self
            .tracks
            .iter()
            .filter(|track| {
                track.gain > 0.001
                    && !track.instrument().is_percussive()
                    && !track.instrument().is_chordal()
                    && track.instrument().role() != InstrumentRole::Bass
                    && track.active_note_count() > 0
            })
            .collect();
        let patterns: Vec<_> = foreground
            .iter()
            .map(|track| scalar_pattern(self, track))
            .collect();
        foreground.iter().zip(&patterns).filter_map(|(track, pattern)| {
            let &(covered, intervals) = pattern.as_ref()?;
            // An arpeggio/colour part can accompany an independently shaped,
            // audible melody. A token quiet extra part is not such a melody.
            let is_support = track.instrument() == Instrument::Arp
                || track.instrument().role() == InstrumentRole::Colour;
            let activity = |part: &Track| part.active_note_count() as f32
                * self.grid_divisions(part) as f32 / part.steps.len().max(1) as f32;
            let has_melody = is_support && foreground.iter().zip(&patterns).any(|(other, pattern)| {
                pattern.is_none() && other.gain >= track.gain * 0.5
                    && other.active_note_count() >= 4
                    && activity(other) >= activity(track) * 0.35
                    && sounding_positions(other).iter().map(|index| other.steps[*index].unwrap())
                        .collect::<std::collections::HashSet<_>>().len() >= 3
            });
            if has_melody { return None; }
            Some(format!("SCALAR EXERCISE: {} repeats long stepwise or fixed-interval runs across {covered}/{intervals} melodic transitions with nearly uniform rhythm. This is a repeated scale exercise, even when every pitch is in range. Compose a short identifiable motif, a contrasting answering phrase and a held or rested arrival; use scalar runs briefly inside that phrase instead of repeating the walk. Changing register, instruments or accompaniment alone does not repair the melody.",track.name))
        }).collect()
    }

    pub(crate) fn written_register(&self, track: &Track) -> (i16, i16, usize, usize) {
        let pitches: Vec<_> = sounding_positions(track)
            .into_iter()
            .map(|index| {
                let degree = track.steps[index].unwrap();
                12 * (i16::from(track.octave) + 1)
                    + i16::from(self.tonic)
                    + i16::from(self.mode.scale()[usize::from(degree % 7)])
                    + 12 * i16::from(degree / 7)
            })
            .collect();
        let range = track.instrument().info().range;
        let outside = pitches
            .iter()
            .filter(|&&note| note < i16::from(range[0]) || note > i16::from(range[1]))
            .count();
        (
            *pitches.iter().min().unwrap_or(&0),
            *pitches.iter().max().unwrap_or(&0),
            outside,
            pitches.len(),
        )
    }

    /// Longest gap in authored note lengths over the repeating pattern, in beats.
    /// This accounts for earlier held notes; it does not claim to measure tails.
    pub(crate) fn longest_written_gap(&self, track: &Track) -> f32 {
        let positions = sounding_positions(track);
        if positions.is_empty() {
            return 0.0;
        }
        let step = 4.0 / self.grid_divisions(track) as f32;
        let cycle = track.steps.len() as f32 * step;
        let texture_gate = match self.composition_plan().texture {
            Texture::Ensemble => 1.0,
            Texture::Chamber => 0.9,
            Texture::Ambient => 1.25,
        };
        let duration = |index: usize| {
            track
                .durations
                .as_ref()
                .and_then(|values| values.get(index))
                .copied()
                .unwrap_or_else(|| {
                    track.instrument().gate()
                        * track.gate_scale()
                        * texture_gate
                        * step.max(track.instrument().attack_spacing())
                })
        };
        positions
            .iter()
            .map(|&next| {
                let start = next as f32 * step;
                let prior_end = positions
                    .iter()
                    .map(|&prior| {
                        prior as f32 * step - if prior >= next { cycle } else { 0.0 }
                            + duration(prior)
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
                (start - prior_end).max(0.0)
            })
            .fold(0.0, f32::max)
    }
}

/// Count sustained runs, not the number of distinct notes. A varied phrase
/// containing an ascending flourish is different from a whole scale cycle.
fn scalar_pattern(score: &Score, track: &Track) -> Option<(usize, usize)> {
    let positions = sounding_positions(track);
    if positions.len() < 7 {
        return None;
    }
    let pitches: Vec<_> = positions
        .iter()
        .map(|index| i16::from(track.steps[*index].unwrap()))
        .collect();
    if pitches.iter().max()? - pitches.iter().min()? < 6
        || pitches
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            < 6
    {
        return None;
    }
    let intervals: Vec<_> = pitches.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let mut covered = 0;
    let mut start = 0;
    while start < intervals.len() {
        let interval = intervals[start];
        let mut end = start + 1;
        while end < intervals.len() && intervals[end] == interval {
            end += 1;
        }
        if (1..=4).contains(&interval.abs()) && end - start >= 4 {
            covered += end - start;
        }
        start = end;
    }
    if covered * 10 < intervals.len() * 7 {
        return None;
    }
    let gaps: Vec<usize> = positions
        .iter()
        .zip(positions.iter().cycle().skip(1))
        .take(positions.len())
        .map(|(&a, &b)| {
            if b > a {
                b - a
            } else {
                track.steps.len() - a + b
            }
        })
        .collect();
    let mode = gaps
        .iter()
        .copied()
        .max_by_key(|gap| gaps.iter().filter(|other| **other == *gap).count())?;
    if gaps.iter().filter(|gap| **gap == mode).count() * 4 < gaps.len() * 3 {
        return None;
    }
    if let Some(durations) = &track.durations {
        let step_beats = 4.0 / score.grid_divisions(track) as f32;
        let heard: Vec<f32> = positions
            .iter()
            .zip(&gaps)
            .map(|(index, gap)| {
                durations
                    .get(*index)
                    .copied()
                    .unwrap_or(step_beats)
                    .min(*gap as f32 * step_beats)
            })
            .collect();
        let regular = heard
            .iter()
            .map(|duration| {
                heard
                    .iter()
                    .filter(|other| (**other - *duration).abs() <= duration.max(0.01) * 0.15)
                    .count()
            })
            .max()
            .unwrap_or(0);
        if regular * 4 < heard.len() * 3 {
            return None;
        }
    }
    Some((covered, intervals.len()))
}

pub(crate) fn minimum_written_attack_spacing(score: &Score, track: &Track) -> Option<f32> {
    let onsets = sounding_positions(track);
    let step = 4.0 / score.grid_divisions(track) as f32;
    onsets
        .iter()
        .zip(onsets.iter().cycle().skip(1))
        .take(onsets.len())
        .map(|(&a, &b)| {
            let slots = if b > a {
                b - a
            } else {
                track.steps.len() - a + b
            };
            slots as f32 * step
        })
        .reduce(f32::min)
}

fn attack_spacing_concern(score: &Score, track: &Track) -> Option<String> {
    let written = minimum_written_attack_spacing(score, track)?;
    let minimum = track.instrument().attack_spacing();
    (written + 0.001 < minimum).then(|| format!(
        "ATTACK SPACING: {} ({}) has written onsets only {written:.2} beats apart; this patch develops most clearly with at least {minimum:.2} beats between attacks. Planned scores trigger every written note, but rapid retriggers can shorten the envelope before its attack develops. Use fewer onsets, null spacer slots, or composition.steps_per_bar=4/8 for a longer phrase at a slower grid. Note duration holds a note but NEVER delays the next grid slot.",
        track.name, track.instrument().id()))
}

pub(crate) fn sounding_positions(track: &Track) -> Vec<usize> {
    track
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, note)| {
            (note.is_some()
                && track
                    .velocities
                    .as_ref()
                    .and_then(|values| values.get(index))
                    .copied()
                    .unwrap_or(1.0)
                    > 0.001)
                .then_some(index)
        })
        .collect()
}

pub(crate) fn same_written_line(left: &Track, right: &Track) -> bool {
    left.octave == right.octave
        && left.steps.len() == right.steps.len()
        && left.steps.iter().enumerate().all(|(index, note)| {
            let left_active = note.is_some()
                && left
                    .velocities
                    .as_ref()
                    .and_then(|v| v.get(index))
                    .copied()
                    .unwrap_or(1.0)
                    > 0.001;
            let right_active = right.steps[index].is_some()
                && right
                    .velocities
                    .as_ref()
                    .and_then(|v| v.get(index))
                    .copied()
                    .unwrap_or(1.0)
                    > 0.001;
            (!left_active && !right_active)
                || (left_active && right_active && *note == right.steps[index])
        })
}

fn pitch_name(pitch: u8) -> String {
    [
        "C", "C#/Db", "D", "D#/Eb", "E", "F", "F#/Gb", "G", "G#/Ab", "A", "A#/Bb", "B",
    ][usize::from(pitch % 12)]
    .to_owned()
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Major => "major",
        Mode::Minor => "natural minor",
        Mode::Dorian => "Dorian",
        Mode::Mixolydian => "Mixolydian",
        Mode::Lydian => "Lydian",
        Mode::Phrygian => "Phrygian",
        Mode::HarmonicMinor => "harmonic minor",
    }
}

fn analyse_chord(score: &Score, chord: Chord) -> HarmonicAnalysis {
    let degree = usize::from(chord.root_degree % 7);
    let scale = score.mode.scale();
    let offset = scale[degree];
    let root = (score.tonic + offset) % 12;
    let relative: Vec<_> = chord
        .quality
        .intervals()
        .iter()
        .map(|interval| (offset + interval) % 12)
        .collect();
    let chord_tones = chord
        .quality
        .intervals()
        .iter()
        .map(|interval| pitch_name(root + interval))
        .collect();
    let chromatic_tones = relative
        .iter()
        .filter(|tone| !scale.contains(tone))
        .map(|tone| pitch_name(score.tonic + tone))
        .collect();
    let numeral = ["I", "II", "III", "IV", "V", "VI", "VII"][degree];
    let alteration = i16::from(offset) - i16::from([0_u8, 2, 4, 5, 7, 9, 11][degree]);
    let prefix = match alteration {
        -1 => "b",
        1 => "#",
        _ => "",
    };
    let (lower, suffix) = match chord.quality {
        ChordQuality::Major => (false, ""),
        ChordQuality::Minor => (true, ""),
        ChordQuality::Diminished => (true, "°"),
        ChordQuality::Augmented => (false, "+"),
        ChordQuality::Suspended2 => (false, "sus2"),
        ChordQuality::Suspended4 => (false, "sus4"),
        ChordQuality::MajorSeventh => (false, "maj7"),
        ChordQuality::MinorSeventh => (true, "7"),
        ChordQuality::DominantSeventh => (false, "7"),
        ChordQuality::HalfDiminished => (true, "ø7"),
    };
    let numeral = if lower {
        numeral.to_ascii_lowercase()
    } else {
        numeral.to_owned()
    };
    let function = match degree {
        0 => "tonic region",
        1 | 3 => "predominant or modal colour",
        4 if relative.contains(&11) => "dominant with leading tone",
        4 => "fifth-degree colour; no leading tone",
        6 if offset == 11 && relative.contains(&5) => "leading-tone dominant",
        6 => "subtonic modal colour",
        _ => "relative region; function depends on context",
    }
    .to_owned();
    HarmonicAnalysis {
        roman: format!("{prefix}{numeral}{suffix}"),
        function,
        chord_tones,
        chromatic_tones,
    }
}

fn authored_cadence(score: &Score, harmony: &[HarmonicAnalysis]) -> String {
    let Some(last) = score.progression.last() else {
        return "No authored chord cycle; shape arrivals with the motif, bass, and rests."
            .to_owned();
    };
    let last_degree = last.root_degree % 7;
    if harmony.len() >= 2 {
        let previous = &harmony[harmony.len() - 2];
        let previous_degree = score.progression[score.progression.len() - 2].root_degree % 7;
        if previous.function == "dominant with leading tone" && last_degree == 0 {
            return format!(
                "The authored cycle ends {}-{}: dominant-tonic arrival candidate. Its placement, bass inversion, and final melody determine the cadence type.",
                previous.roman,
                harmony[harmony.len() - 1].roman
            );
        }
        if previous_degree == 3 && last_degree == 0 {
            return "The authored cycle ends with fourth-degree-to-tonic motion, a plagal arrival candidate; confirm phrase placement in the arrangement.".to_owned();
        }
        if previous.function == "dominant with leading tone" && last_degree == 5 {
            return "The authored cycle ends with dominant-to-sixth-degree motion, a deceptive continuation candidate; reserve a later tonic arrival if desired.".to_owned();
        }
    }
    if last_degree == 4 {
        if harmony
            .last()
            .is_some_and(|chord| chord.function == "dominant with leading tone")
        {
            if score
                .progression
                .first()
                .is_some_and(|chord| chord.root_degree % 7 == 0)
            {
                return "The authored cycle ends on a leading-tone dominant and returns to tonic across its repeat; an open phrase can answer with that return.".to_owned();
            }
            return "The authored cycle ends on a leading-tone dominant: an open ending candidate rather than a confirmed half cadence.".to_owned();
        }
        return "The authored cycle ends on a fifth-degree chord without the leading tone: a modal return, not a conventional major-V authentic cadence.".to_owned();
    }
    if last_degree == 0 {
        return "The authored cycle returns to tonic without a conventional dominant approach; use melodic settling and a rest to articulate the arrival.".to_owned();
    }
    "The authored cycle leaves its home region open; plan a deliberate modal arrival, contrasting section, or later tonic return.".to_owned()
}

fn motif_priority(track: &Track) -> (u8, usize) {
    let name = track.name.to_ascii_lowercase();
    let named = ["motif", "melody", "theme", "lead"]
        .iter()
        .any(|word| name.contains(word));
    let role = match track.instrument().info().role {
        InstrumentRole::Melody => 5,
        InstrumentRole::Colour => 4,
        InstrumentRole::Harmony => 2,
        InstrumentRole::Bass => 1,
        InstrumentRole::Percussion => 0,
    };
    (role + if named { 8 } else { 0 }, track.active_note_count())
}

fn analyse_motif(track: &Track) -> MotifAnalysis {
    let steps: Vec<_> = track
        .steps
        .iter()
        .enumerate()
        .map(|(index, note)| {
            if track
                .velocities
                .as_ref()
                .and_then(|values| values.get(index))
                .is_some_and(|velocity| *velocity <= 0.001)
            {
                None
            } else {
                *note
            }
        })
        .collect();
    let notes: Vec<_> = steps.iter().flatten().copied().collect();
    let intervals: Vec<_> = notes
        .windows(2)
        .map(|pair| i16::from(pair[1]) - i16::from(pair[0]))
        .collect();
    let rises = intervals.iter().filter(|interval| **interval > 0).count();
    let falls = intervals.iter().filter(|interval| **interval < 0).count();
    let contour = match (rises, falls) {
        (0, 0) => "repeated pitch",
        (_, 0) => "ascending",
        (0, _) => "descending",
        _ if rises > falls * 2 => "mostly ascending with turns",
        _ if falls > rises * 2 => "mostly descending with turns",
        _ => "mixed / arching",
    }
    .to_owned();
    let len = steps.len();
    let smallest_repeat_steps = (1..=len / 2).find(|period| {
        len.is_multiple_of(*period) && (0..len).all(|index| steps[index] == steps[index % period])
    });
    MotifAnalysis {
        track: track.name.clone(),
        sounded_notes: notes.len(),
        step_count: len,
        intervals,
        contour,
        rest_fraction: (len - notes.len()) as f32 / len.max(1) as f32,
        smallest_repeat_steps,
    }
}

fn voice_leading_guidance(
    score: &Score,
    harmony: &[HarmonicAnalysis],
    motif: Option<&MotifAnalysis>,
) -> Vec<String> {
    let mut advice = Vec::new();
    for pair in harmony.windows(2).take(3) {
        let common: Vec<_> = pair[0]
            .chord_tones
            .iter()
            .filter(|tone| pair[1].chord_tones.contains(tone))
            .cloned()
            .collect();
        if !common.is_empty() {
            advice.push(format!("{} -> {} shares {}; keep a shared tone in one inner voice and move the other voices nearby.", pair[0].roman, pair[1].roman, common.join("/")));
            break;
        }
    }
    if score
        .progression
        .iter()
        .any(|chord| chord.quality.intervals().len() == 4)
    {
        advice.push("There are seventh chords: for a tonal resolution, try the chordal seventh descending by scale step while the leading tone rises to tonic.".to_owned());
    }
    if let Some(motif) = motif {
        let leaps = motif
            .intervals
            .iter()
            .filter(|interval| interval.abs() > 2)
            .count();
        if leaps > 0 {
            advice.push(format!("{} has {} authored leap(s) larger than a third; try a short stepwise return after one leap, preserving the opening.", motif.track, leaps));
        }
        if motif.rest_fraction > 0.0 {
            advice.push(format!("{} has {:.0}% untriggered slots. Check note durations before treating them as silence; preserve a real phrase-ending pause instead of filling every gap with an answering line.", motif.track, motif.rest_fraction * 100.0));
        } else {
            advice.push(format!(
                "{} has no authored rests; remove a phrase-end attack to give its answer space.",
                motif.track
            ));
        }
    }
    if advice.is_empty() {
        advice.push("Keep one pitch as a reference, move another line by nearby scale steps, and compare the resulting interval changes.".to_owned());
    }
    advice
}

fn next_experiment(
    score: &Score,
    harmony: &[HarmonicAnalysis],
    motif: Option<&MotifAnalysis>,
) -> String {
    let plan = score.composition_plan();
    if let Some(concern) = score.orchestration_concerns().first() {
        return format!(
            "Repair this concrete issue before changing the ensemble or developing more material: {concern} Preserve useful motif, tempo and harmonic identity while making the correction."
        );
    }
    if plan.harmony == HarmonicPolicy::Diatonic
        && let Some(chord) = harmony
            .iter()
            .find(|chord| !chord.chromatic_tones.is_empty())
    {
        return format!(
            "Study {}'s outside-scale {}: choose a diatonic quality or explicitly set composition.harmony=chromatic and give the colour a destination; preserve the motif and rhythm for comparison.",
            chord.roman,
            chord.chromatic_tones.join("/")
        );
    }
    let Some(motif) = motif else {
        return format!(
            "Write a four-note pitched motif with at least one rest, then an answering phrase over {} bars; retain the current tempo and percussion for comparison.",
            plan.phrase_bars
        );
    };
    if motif.rest_fraction == 0.0 && motif.sounded_notes > 3 {
        return format!(
            "In {}, keep the first and last notes but replace one interior phrase-ending attack with a rest; let a second voice answer there. Keep key and chords unchanged.",
            motif.track
        );
    }
    if plan.development == 0.0 {
        return format!(
            "Retain {}'s motif, register and instrumentation. A zero-development plan requests stable authored material; revise only a concrete weakness or an explicit listener request. A larger palette is available without any requirement to change voices.",
            motif.track
        );
    }
    if motif.smallest_repeat_steps.is_some() && plan.development > 0.0 {
        return format!(
            "Keep {}'s first repeating cell unchanged; sequence the answering cell up one scale degree in a contrasting phrase, then return to the original in the {} plan.",
            motif.track,
            plan.form.label()
        );
    }
    match (score.evolution / 6) % 5 {
        0 => format!(
            "Keep {}'s opening and rhythm; invert the interior contour of its answer in one {}-bar phrase, then restore the theme. Compare whether the return remains recognisable.",
            motif.track, plan.phrase_bars
        ),
        1 => format!(
            "Keep {} and the chord roots; retain common tones between chords and choose nearby upper-voice notes. Compare the harmonic movement without changing tempo or instrumentation.",
            motif.track
        ),
        2 => format!(
            "Let a quieter counter-line answer the rests in {}, moving opposite to its main contour; keep the {}-bar phrase and ending anchor.",
            motif.track, plan.phrase_bars
        ),
        3 => format!(
            "Develop the {} plan by making one contrasting phrase sparser and changing its register; return to {}'s original opening so the form is audible.",
            plan.form.label(),
            motif.track
        ),
        _ => format!(
            "Keep {} recognisable while comparing the current {}-bar harmonic rhythm with {} bars per chord; retain the key and use the same closing gesture.",
            motif.track,
            plan.chord_bars,
            if plan.chord_bars == 1 { 2 } else { 1 }
        ),
    }
}

/// Transform a connected interior cell while preserving rhythmic placement,
/// opening/closing anchors, and the score's 0..20 scale-degree range.
pub(crate) fn develop_motif(steps: &[Option<u8>], revision: u64) -> Vec<Option<u8>> {
    transform_motif(steps, revision, false)
}

/// A contrasting answer derived from the same cell, not independent random
/// notes. The arranger is responsible for spacing it against the main voice.
pub(crate) fn counter_motif(steps: &[Option<u8>], revision: u64) -> Vec<Option<u8>> {
    transform_motif(steps, revision, true)
}

/// Silent authored pitches are retained in the score but cannot become the
/// anchors of an audible theme. Mask them for analysis, then restore their
/// exact authored values without changing expression or rhythmic placement.
pub(crate) fn develop_expressed_motif(
    track: &Track,
    revision: u64,
    counter: bool,
) -> Vec<Option<u8>> {
    let active: Vec<_> = track
        .steps
        .iter()
        .enumerate()
        .map(|(index, note)| {
            if track
                .velocities
                .as_ref()
                .and_then(|values| values.get(index))
                == Some(&0.0)
            {
                None
            } else {
                *note
            }
        })
        .collect();
    let changed = if counter {
        counter_motif(&active, revision)
    } else {
        develop_motif(&active, revision)
    };
    changed
        .into_iter()
        .zip(active)
        .zip(&track.steps)
        .map(
            |((changed, active), written)| {
                if active.is_some() { changed } else { *written }
            },
        )
        .collect()
}

fn transform_motif(steps: &[Option<u8>], revision: u64, counter: bool) -> Vec<Option<u8>> {
    let mut developed = steps.to_vec();
    let positions: Vec<_> = steps
        .iter()
        .enumerate()
        .filter_map(|(index, note)| note.map(|_| index))
        .collect();
    if positions.len() < 3 {
        return developed;
    }
    // Longer themes keep two-note opening and closing signatures; a miniature
    // theme keeps one anchor at each end. Rests stay exactly where authored.
    let anchors = if positions.len() >= 6 { 2 } else { 1 };
    let interior = &positions[anchors..positions.len() - anchors];
    let original: Vec<_> = interior
        .iter()
        .map(|index| steps[*index].unwrap_or(0))
        .collect();
    let operation = if counter {
        1 + revision % 2
    } else {
        revision % 3
    };
    let low = original.iter().copied().min().unwrap_or(0);
    let high = original.iter().copied().max().unwrap_or(0);
    let mut notes = match operation {
        1 => original
            .iter()
            .map(|note| low + high - note)
            .collect::<Vec<_>>(),
        2 => original.iter().rev().copied().collect(),
        _ => sequence_cell(&original, revision),
    };
    if notes == original {
        notes = sequence_cell(&original, revision);
    }
    for (index, degree) in interior.iter().zip(notes) {
        developed[*index] = Some(degree.min(20));
    }
    developed
}

fn sequence_cell(notes: &[u8], revision: u64) -> Vec<u8> {
    let low = notes.iter().copied().min().unwrap_or(0);
    let high = notes.iter().copied().max().unwrap_or(0);
    // A single transposition preserves the cell's intervals. At a boundary use
    // the other direction; never clamp each pitch independently into a plateau.
    let up = if high >= 20 {
        false
    } else if low == 0 {
        true
    } else {
        revision.is_multiple_of(2)
    };
    if low == 0 && high >= 20 {
        return notes.to_vec();
    }
    notes
        .iter()
        .map(|note| if up { note + 1 } else { note - 1 })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_context_exposes_every_executable_voice_with_range_and_role() {
        let context = crate::default_score().theory_context();
        assert!(Instrument::ALL.len() >= 70);
        let schema: serde_json::Value = serde_json::from_str(crate::SCORE_SCHEMA).unwrap();
        let ids = schema["properties"]["tracks"]["items"]["properties"]["instrument"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(ids.len(), Instrument::ALL.len());
        for &instrument in Instrument::ALL {
            let info = instrument.info();
            assert!(
                ids.contains(&serde_json::Value::String(info.id.to_owned())),
                "{} absent from schema",
                info.id
            );
            assert!(
                context.contains(&format!("{} [", info.id)),
                "{} absent from composer context",
                info.id
            );
            assert!(context.contains(&format!("{}..{}", info.range[0], info.range[1])));
        }
        assert!(context.contains("theremin [melody"));
        assert!(context.contains("piano ["));
        assert!(context.contains("current]"));
    }

    #[test]
    fn new_melodic_voices_are_selected_as_motifs() {
        let mut score = crate::default_score();
        let mut theremin = score.tracks[0].clone();
        theremin.name = "Singing line".to_owned();
        theremin.instrument = Some(Instrument::Theremin);
        theremin.gain = 0.4;
        theremin.steps = vec![Some(0), None, Some(2), Some(4)];
        theremin.velocities = None;
        score.tracks.push(theremin);
        let mut bass = score.tracks[0].clone();
        bass.name = "Ground".to_owned();
        bass.instrument = Some(Instrument::DoubleBass);
        score.tracks = vec![bass, score.tracks.last().unwrap().clone()];
        assert_eq!(score.theory_report().motif.unwrap().track, "Singing line");
    }

    #[test]
    fn omitted_plan_members_have_musical_defaults() {
        let plan: CompositionPlan =
            serde_json::from_str(r#"{"texture":"ambient","form":"rondo"}"#).unwrap();
        assert_eq!(plan.texture, Texture::Ambient);
        assert_eq!(plan.form, MusicalForm::Rondo);
        assert_eq!(plan.phrase_bars, 4);
        assert_eq!(plan.chord_bars, 1);
        assert_eq!(plan.harmony, HarmonicPolicy::Diatonic);
        assert_eq!(plan.development, 0.35);
    }

    #[test]
    fn modal_minor_fifth_is_not_reported_as_a_leading_tone_dominant() {
        let mut score = crate::default_score();
        score.mode = Mode::Dorian;
        score.tonic = 2;
        score.progression = vec![
            Chord {
                root_degree: 0,
                quality: ChordQuality::Minor,
            },
            Chord {
                root_degree: 4,
                quality: ChordQuality::Minor,
            },
        ];
        let report = score.theory_report();
        assert_eq!(report.key, "D Dorian");
        assert_eq!(report.scale, ["D", "E", "F", "G", "A", "B", "C"]);
        assert_eq!(report.progression[1].roman, "v");
        assert!(report.progression[1].function.contains("no leading tone"));
        assert!(report.cadence.contains("without the leading tone"));
        assert!(
            report
                .progression
                .iter()
                .all(|chord| chord.chromatic_tones.is_empty())
        );
    }

    #[test]
    fn seventh_harmony_and_chromatic_policy_have_specific_analysis() {
        let mut score = crate::default_score();
        score.tonic = 0;
        score.mode = Mode::Minor;
        score.progression = vec![
            Chord {
                root_degree: 4,
                quality: ChordQuality::DominantSeventh,
            },
            Chord {
                root_degree: 0,
                quality: ChordQuality::Minor,
            },
        ];
        let report = score.theory_report();
        assert_eq!(report.progression[0].roman, "V7");
        assert_eq!(report.progression[0].chord_tones, ["G", "B", "D", "F"]);
        assert_eq!(report.progression[0].chromatic_tones, ["B"]);
        assert!(report.cadence.contains("arrival candidate"));
        assert!(
            report
                .next_experiment
                .contains("composition.harmony=chromatic")
        );
        score.mode = Mode::HarmonicMinor;
        assert!(
            score.theory_report().progression[0]
                .chromatic_tones
                .is_empty()
        );
        score.mode = Mode::Minor;
        score.composition = Some(CompositionPlan {
            harmony: HarmonicPolicy::Chromatic,
            ..Default::default()
        });
        assert!(
            !score
                .theory_report()
                .next_experiment
                .contains("choose a diatonic quality")
        );
    }

    #[test]
    fn roman_labels_include_mode_alterations_and_extensions() {
        let mut score = crate::default_score();
        score.mode = Mode::Dorian;
        let subtonic = analyse_chord(
            &score,
            Chord {
                root_degree: 6,
                quality: ChordQuality::MajorSeventh,
            },
        );
        assert_eq!(subtonic.roman, "bVIImaj7");
        score.mode = Mode::Minor;
        let supertonic = analyse_chord(
            &score,
            Chord {
                root_degree: 1,
                quality: ChordQuality::HalfDiminished,
            },
        );
        assert_eq!(supertonic.roman, "iiø7");
        assert!(supertonic.chromatic_tones.is_empty());
    }

    #[test]
    fn motif_report_counts_real_authored_intervals_and_rests() {
        let mut track = crate::default_score().tracks.remove(0);
        track.steps = vec![
            Some(0),
            None,
            Some(2),
            Some(1),
            Some(0),
            None,
            Some(2),
            Some(1),
        ];
        let analysis = analyse_motif(&track);
        assert_eq!(analysis.sounded_notes, 6);
        assert_eq!(analysis.intervals, [2, -1, -1, 2, -1]);
        assert_eq!(analysis.rest_fraction, 0.25);
        assert_eq!(analysis.smallest_repeat_steps, Some(4));
        assert_eq!(analysis.contour, "mixed / arching");
        assert_eq!(
            analyse_motif(&Track {
                steps: vec![Some(0), Some(1), Some(2)],
                ..track
            })
            .contour,
            "ascending"
        );
    }

    #[test]
    fn analysis_does_not_invent_melody_from_muted_or_percussion_parts() {
        let mut score = crate::default_score();
        for track in &mut score.tracks {
            if !track.instrument().is_percussive() {
                track.gain = 0.0;
            }
        }
        let report = score.theory_report();
        assert!(report.motif.is_none());
        assert!(
            report
                .next_experiment
                .contains("Write a four-note pitched motif")
        );
        assert!(score.theory_context().contains("No active pitched motif"));
    }

    #[test]
    fn zero_velocity_notes_are_rests_in_theory_as_well_as_playback() {
        let mut track = crate::default_score().tracks.remove(0);
        track.steps = vec![Some(0), Some(18), Some(2), Some(19)];
        track.velocities = Some(vec![0.8, 0.0, 0.7, 0.0]);
        let motif = analyse_motif(&track);
        assert_eq!(motif.sounded_notes, 2);
        assert_eq!(motif.intervals, [2]);
        assert_eq!(motif.rest_fraction, 0.5);
        assert_eq!(motif.contour, "ascending");
    }

    #[test]
    fn native_development_preserves_audible_anchors_and_silent_written_pitches() {
        for (instrument, evolution) in [(Instrument::Lead, 1), (Instrument::Piano, 2)] {
            let mut score = crate::default_score();
            score.composition = Some(CompositionPlan::default());
            score.evolution = evolution;
            score.tracks.truncate(1);
            let track = &mut score.tracks[0];
            track.instrument = Some(instrument);
            track.steps = (0..10).map(Some).collect();
            track.velocities = Some(vec![0.0, 0.0, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.0, 0.0]);
            track.durations = Some(vec![0.5; 10]);
            let next = score.evolved().unwrap();
            let old = &score.tracks[0];
            let new = &next.tracks[0];
            // Four silent pitches and four audible opening/closing anchors.
            for index in [0, 1, 2, 3, 6, 7, 8, 9] {
                assert_eq!(
                    new.steps[index], old.steps[index],
                    "{instrument:?}: slot {index}"
                );
            }
            assert_ne!(&new.steps[4..6], &old.steps[4..6]);
            assert_eq!(new.velocities, old.velocities);
            assert_eq!(new.durations, old.durations);
        }
    }

    #[test]
    fn thematic_changes_preserve_anchors_rests_and_connected_cell_intervals() {
        let theme = vec![
            Some(2),
            None,
            Some(4),
            Some(5),
            None,
            Some(7),
            Some(6),
            Some(4),
            Some(2),
            None,
        ];
        let developed = develop_motif(&theme, 0);
        assert_eq!(
            developed,
            vec![
                Some(2),
                None,
                Some(4),
                Some(6),
                None,
                Some(8),
                Some(7),
                Some(4),
                Some(2),
                None
            ]
        );
        // The transformed cell keeps +2,-1, so this is a recognisable sequence.
        let old_cell = [5_i16, 7, 6];
        let new_cell: Vec<_> = [3, 5, 6]
            .map(|index| i16::from(developed[index].unwrap()))
            .to_vec();
        assert_eq!(
            old_cell
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>(),
            new_cell
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>()
        );
        for revision in 0..24 {
            for changed in [
                develop_motif(&theme, revision),
                counter_motif(&theme, revision),
            ] {
                assert_eq!(changed.len(), theme.len());
                for index in [0, 2, 7, 8] {
                    assert_eq!(changed[index], theme[index]);
                }
                assert!(
                    changed
                        .iter()
                        .zip(&theme)
                        .all(|(a, b)| a.is_none() == b.is_none())
                );
                assert!(changed.iter().flatten().all(|note| *note <= 20));
            }
        }
    }

    #[test]
    fn inversion_reverses_interior_contour_without_random_note_edits() {
        let theme = vec![
            Some(0),
            Some(2),
            Some(4),
            Some(7),
            Some(5),
            Some(2),
            Some(0),
        ];
        let inverted = develop_motif(&theme, 1);
        assert_eq!(&inverted[2..5], &[Some(7), Some(4), Some(6)]);
        assert_eq!(&inverted[..2], &theme[..2]);
        assert_eq!(&inverted[5..], &theme[5..]);
        assert_eq!(counter_motif(&theme, 0), inverted);
    }

    #[test]
    fn sparse_and_boundary_motifs_remain_valid() {
        for theme in [
            vec![],
            vec![None],
            vec![Some(20)],
            vec![Some(0), None, Some(20)],
            vec![Some(0), Some(0), Some(20), Some(20), Some(0)],
        ] {
            for revision in 0..12 {
                let changed = develop_motif(&theme, revision);
                assert_eq!(changed.first(), theme.first());
                assert_eq!(changed.last(), theme.last());
                assert!(changed.iter().flatten().all(|note| *note <= 20));
            }
        }
    }
}
