//! Turns a score into timed, harmonised, dynamic note events.
//!
//! The previous engine divided the whole piece by a track's pattern length, so
//! a four-step pad held each note for ten seconds and every pattern played
//! exactly once. Nothing repeated, nothing varied, and the tempo was decorative.
//!
//! Here a step is a sixteenth note, patterns loop against the bar grid, and the
//! piece is divided into sections that mute parts, open filters, and change
//! dynamics. That structure is what "movement" means: the same eight bars of
//! material has to arrive, build, break, and return rather than simply repeat.

use crate::synth::Instrument;
use crate::{Chord, ChordQuality, Mode, Score, Track, chord_intervals, scale_offset};

pub const STEPS_PER_BEAT: usize = 4;
pub const BEATS_PER_BAR: usize = 4;
pub const STEPS_PER_BAR: usize = STEPS_PER_BEAT * BEATS_PER_BAR;

/// Bars in a phrase: the unit that repeats before anything is allowed to change.
///
/// Four is what the ear expects, and it matches the length of the default chord
/// progression, so a phrase covers the harmony exactly once.
pub const PHRASE_BARS: usize = 4;

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

    /// Whether a part sounds at all in this section.
    ///
    /// Removing parts is the only way to make adding them back feel like an
    /// event. A mix in which everything plays for the whole piece has no
    /// dynamics no matter how the notes are written.
    pub fn plays(self, instrument: Instrument) -> bool {
        use Instrument::{
            Arp, Bell, Cello, Flute, Harp, Keys, Lead, Organ, Pad, Piano, Pluck, Reed, Snare,
            SubBass,
        };
        // The harmonic bed: what holds a section together underneath everything
        // else, and the only thing playing when the piece is at its sparsest.
        let bed = matches!(instrument, Pad | Organ | Keys | Cello | SubBass);
        // Voices that carry a line. Removing these is what makes the quiet
        // sections quiet; letting them all play everywhere flattened the arc
        // from a threefold level swing to barely half of one.
        let melodic = matches!(instrument, Flute | Reed | Lead);
        let colour = matches!(instrument, Bell | Harp | Piano | Pluck | Arp);
        match self {
            Self::Intro => bed,
            Self::Rise => !matches!(instrument, Snare) && !melodic,
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
    pub midi: f32,
    pub velocity: f32,
    /// Arrangement automation for this note's filter, as a cutoff multiplier.
    pub cutoff_scale: f32,
    pub seed: u32,
}

/// The plan for one render: every note, plus how long the tail runs.
pub struct Arrangement {
    pub events: Vec<NoteEvent>,
    pub frames: usize,
    pub tail_frames: usize,
    pub kick_onsets: Vec<usize>,
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
    /// Plans every note in the score.
    pub(crate) fn arrange(&self) -> Arrangement {
        let bars = usize::from(self.bars);
        let bar_frames =
            60.0 / f32::from(self.bpm) * BEATS_PER_BAR as f32 * crate::SAMPLE_RATE as f32;
        let frames = (bars as f32 * bar_frames) as usize;
        let seed = self.seed();

        let mut events = Vec::new();
        let mut kick_onsets = Vec::new();
        let mut longest_tail = 0.0_f32;

        // A section may only silence a part while some other part still plays.
        // The mute lists assume a full arrangement; applied to a sparse score
        // they can empty a whole section, and the verifier rightly rejects a
        // render with dead sections. A one-track score keeps its one track.
        let present: Vec<Instrument> = self.tracks.iter().map(|track| track.instrument()).collect();
        let muting_is_safe = |section: Section| present.iter().any(|part| section.plays(*part));

        for (index, track) in self.tracks.iter().enumerate() {
            let instrument = track.instrument();
            let patch = instrument.patch();
            longest_tail = longest_tail.max(patch.amp.tail());

            // A pattern spans one bar and repeats, so its length chooses its
            // own division: four steps are quarter notes, sixteen are
            // sixteenths. Patterns longer than a bar run at sixteenths and span
            // several. This is what makes a written rhythm mean a rhythm rather
            // than a subdivision of however long the piece happens to be.
            let divisions = track.steps.len().clamp(1, STEPS_PER_BAR);
            let frames_per_step = (bar_frames / divisions as f32).max(1.0);
            let track_steps = bars * divisions;
            // The position within the bar, expressed on the sixteenth grid the
            // accent table is written against.
            let to_sixteenths = STEPS_PER_BAR / divisions;

            for step in 0..track_steps {
                let bar = step / divisions;
                let step_in_bar = (step % divisions) * to_sixteenths;
                let section = Section::at(bar, bars);
                if !section.plays(instrument) && muting_is_safe(section) {
                    continue;
                }

                // Every variation is keyed to the phrase and the position inside
                // the bar, never to the absolute step. Keyed to the step, the
                // thinning, the accents and the micro-timing all differed in
                // every bar, so no figure ever played twice: bar-to-bar
                // similarity sat at 0.65 where a shaped arrangement sits near
                // 0.85. The result was restless rather than repetitive, which
                // is the harder of the two to listen to — there was nothing to
                // recognise. A phrase now repeats and the next phrase differs.
                let phrase = bar / PHRASE_BARS;
                let phrase_end = bar % PHRASE_BARS == PHRASE_BARS - 1;
                let fill = phrase_end && instrument.is_percussive();
                let written = track.steps[step % track.steps.len()];
                // A fill must be allowed to create a hit in an empty slot. The
                // old code checked for `None` and continued before it reached
                // the fill logic, so its promised phrase-ending drum fills
                // could never exist.
                let generated_fill = written.is_none()
                    && fill
                    && step_in_bar >= 12
                    && chance(seed, index as u64 + phrase as u64 * 131, step_in_bar as u64) < 0.42;
                let Some(degree) = written.or(generated_fill.then_some(0)) else {
                    continue;
                };
                if !self.keeps_note(seed, index, phrase, step_in_bar, section, instrument, fill) {
                    continue;
                }

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
                    continue;
                }

                let chord = self.chord_at(bar);
                let midi = fold_into_range(
                    self.pitch_for(track, instrument, degree, chord, step, phrase),
                    instrument.natural_range(),
                );
                let velocity =
                    self.velocity_for(seed, index, phrase, step_in_bar, section, instrument);
                let gate = instrument.gate() * track.gate_scale();
                let gate_seconds = (gate * frames_per_step / crate::SAMPLE_RATE as f32).max(0.01);

                if instrument == Instrument::Kick {
                    kick_onsets.push(start_frame);
                }

                events.push(NoteEvent {
                    track: index,
                    instrument,
                    start_frame,
                    gate_seconds,
                    midi,
                    velocity,
                    cutoff_scale: self.cutoff_at(bar, step, section, instrument),
                    seed: hash(seed, index as u64, step as u64) as u32,
                });
            }
        }

        events.sort_unstable_by_key(|event| event.start_frame);
        // Two seconds beyond the longest release covers the reverb and delay
        // tails, which the renderer folds back over the opening so the loop
        // joins without a gap.
        let tail_frames = ((longest_tail + 2.0) * crate::SAMPLE_RATE as f32) as usize;

        Arrangement {
            events,
            frames,
            tail_frames,
            kick_onsets,
        }
    }

    /// Whether a written note actually sounds, given section and variation.
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

        let density = match section {
            Section::Intro => 0.55,
            Section::Rise => 0.80,
            Section::Main => 0.97,
            Section::Break => 0.70,
            Section::Finale => 1.0,
        };
        // Only the parts that articulate rhythm are thinned, and only the
        // off-beats among those: a written note on a beat always sounds.
        let thinnable = matches!(
            instrument,
            Instrument::Hat | Instrument::Perc | Instrument::Arp | Instrument::Pluck
        );
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
        if matches!(instrument, Instrument::Pad | Instrument::SubBass) {
            velocity = 0.85;
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

    /// The chord governing a bar.
    fn chord_at(&self, bar: usize) -> Option<Chord> {
        if self.progression.is_empty() {
            return None;
        }
        Some(self.progression[bar % self.progression.len()])
    }

    /// The MIDI pitch for one step, placed inside the current chord.
    ///
    /// This is the smallest useful piece of V1's harmony engine: a written
    /// degree that disagrees with the chord is moved to the nearest chord tone
    /// rather than sounded against it. It is what stops the court's music from
    /// producing accidental dissonance every time the progression moves.
    fn pitch_for(
        &self,
        track: &Track,
        instrument: Instrument,
        degree: u8,
        chord: Option<Chord>,
        step: usize,
        phrase: usize,
    ) -> f32 {
        let base = 12 * (i16::from(track.octave) + 1) + i16::from(self.tonic);
        if !instrument.follows_harmony() {
            // Percussion is tuned by role, not by the key.
            let pitch = match instrument {
                Instrument::Kick => 36,
                Instrument::Snare => 62,
                Instrument::Hat => 84,
                _ => 55 + i16::from(degree % 5) * 2,
            };
            return f32::from(pitch);
        }

        let Some(chord) = chord else {
            return f32::from(base + i16::from(scale_offset(self.mode, degree)));
        };
        let root = i16::from(scale_offset(self.mode, chord.root_degree));
        let intervals = chord_intervals(chord.quality);

        let developed_degree = match instrument {
            Instrument::Lead | Instrument::Flute | Instrument::Reed => match phrase % 4 {
                // statement, answering sequence, development, reprise
                0 | 3 => degree,
                1 => (degree + 2) % 7,
                _ => 6_u8.saturating_sub(degree % 7),
            },
            Instrument::Arp | Instrument::Bell | Instrument::Harp | Instrument::Pluck
                if phrase % 2 == 1 =>
            {
                (degree + 1) % 7
            }
            _ => degree,
        };
        let offset = match instrument {
            // The bass states the root, stepping to the fifth off the beat so
            // the low end moves without ever contradicting the harmony.
            Instrument::SubBass => root,
            Instrument::Bass => {
                if step % 8 == 6 {
                    root + i16::from(intervals[2])
                } else {
                    root
                }
            }
            // Chordal parts voice the triad, rotating which tone is on top.
            Instrument::Pad | Instrument::Keys => {
                root + i16::from(intervals[(step + usize::from(degree)) % intervals.len()])
            }
            // Melodic parts keep their written shape but land on chord tones.
            _ => {
                let written = i16::from(scale_offset(self.mode, developed_degree));
                nearest_chord_tone(written, root, intervals)
            }
        };
        f32::from(base + offset)
    }

    /// The filter automation for one note.
    ///
    /// Two motions combined: the section's overall brightness, and a slow sweep
    /// across four bars. A static cutoff is the difference between a synthesiser
    /// that is playing and one that is merely on.
    fn cutoff_at(&self, bar: usize, step: usize, section: Section, instrument: Instrument) -> f32 {
        if instrument.is_percussive() {
            return 1.0;
        }
        let intensity = section.intensity();
        let sweep_position =
            (bar % 8) as f32 / 8.0 + (step % STEPS_PER_BAR) as f32 / STEPS_PER_BAR as f32 / 8.0;
        let sweep = (sweep_position * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        // Between roughly a third and twice the patch's own cutoff.
        let scale = 0.35 + 1.65 * (intensity * 0.72 + sweep * 0.28);
        if section == Section::Rise {
            // A rise should audibly open across its own length.
            let progress = (bar as f32 / self.bars.max(1) as f32 - 0.15) / 0.20;
            return scale * (0.55 + 0.75 * progress.clamp(0.0, 1.0));
        }
        scale
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
fn nearest_chord_tone(written: i16, root: i16, intervals: [u8; 3]) -> i16 {
    let mut best = written;
    let mut best_distance = i16::MAX;
    for octave in -1..=2 {
        for interval in intervals {
            let candidate = root + i16::from(interval) + octave * 12;
            let distance = (candidate - written).abs();
            if distance < best_distance {
                best_distance = distance;
                best = candidate;
            }
        }
    }
    best
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

        let plan = self.arrange();
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
            let written =
                usize::from(self.bars) * track.steps.iter().filter(|step| step.is_some()).count();
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
                "{:<22} {:>6} {:>6.0}ms {:>8} {:>8}  {} pitches, {}/{} written notes sounded",
                track.name,
                notes.len(),
                notes[0].gate_seconds * 1000.0,
                lowest,
                highest,
                pitches.len(),
                notes.len(),
                written
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
        }
    }
}

impl ChordQuality {
    pub const fn intervals(self) -> [u8; 3] {
        match self {
            Self::Major => [0, 4, 7],
            Self::Minor => [0, 3, 7],
            Self::Diminished => [0, 3, 6],
            Self::Suspended2 => [0, 2, 7],
            Self::Suspended4 => [0, 5, 7],
        }
    }
}
