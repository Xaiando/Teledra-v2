//! Court-owned score validation, synthesis, playback, and feedback.
//!
//! The score schema is the court's instrument. `arrange` decides what is played
//! and when, `synth` decides what each note sounds like, and `effects` decides
//! what the room around it sounds like. This module owns the schema, the mix,
//! and the boundary the rest of Teledra sees.

mod arrange;
mod effects;
mod synth;

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rodio::buffer::SamplesBuffer;
use rodio::cpal::traits::HostTrait;
use rodio::{DeviceTrait, OutputStream, OutputStreamHandle, Sink};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use synth::Instrument;

pub const SAMPLE_RATE: u32 = 48_000;
const MAX_AUDIO_SECONDS: usize = 10 * 60;
/// Peak the master is normalised to, leaving roughly 1 dB below the level at
/// which `verify_audio` refuses a render.
const MASTER_PEAK: f32 = 0.89;
/// How many variations share one key, mode, and tempo.
///
/// A variation lands every eighteen to thirty-six seconds, so six movements
/// give the ear a few minutes in one key before a deliberate modulation.
const HARMONIC_ANCHOR_VARIATIONS: usize = 6;

static FEEDBACK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Score {
    pub title: String,
    pub bpm: u16,
    pub bars: u16,
    pub tonic: u8,
    pub mode: Mode,
    #[serde(default)]
    pub progression: Vec<Chord>,
    pub tracks: Vec<Track>,
    /// How far the off-beats lean, 0..0.6. Absent means a straight grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swing: Option<f32>,
    /// Fixes the arrangement's variation, so a score always renders the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// How hard the kick ducks everything else, 0..0.85.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain: Option<f32>,
    /// Monotonic accepted variation number.
    ///
    /// Old and model-authored scores omit it. Native evolution increments it,
    /// and the runtime advances model-authored replacements before playback.
    /// This makes musical change visible in the DAW and durable across restarts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub evolution: u64,
}

const fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordQuality {
    Major,
    Minor,
    Diminished,
    Suspended2,
    Suspended4,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Chord {
    pub root_degree: u8,
    pub quality: ChordQuality,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Major,
    Minor,
    Dorian,
    Mixolydian,
}

/// The single musical axis a native variation develops.
///
/// Changing every axis at once produces novelty without identity. These
/// movements make frequent evolution legible: the listener can hear what was
/// developed while key and tempo remain stable until the modulation turn.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionKind {
    Statement,
    Pulse,
    Motif,
    Counterpoint,
    Texture,
    Harmony,
    Modulation,
}

impl EvolutionKind {
    pub const fn for_revision(revision: u64) -> Self {
        if revision == 0 {
            return Self::Statement;
        }
        match revision % HARMONIC_ANCHOR_VARIATIONS as u64 {
            1 => Self::Pulse,
            2 => Self::Motif,
            3 => Self::Counterpoint,
            4 => Self::Texture,
            5 => Self::Harmony,
            _ => Self::Modulation,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Statement => "Statement",
            Self::Pulse => "Pulse",
            Self::Motif => "Motif",
            Self::Counterpoint => "Counterpoint",
            Self::Texture => "Texture",
            Self::Harmony => "Harmony",
            Self::Modulation => "Modulation",
        }
    }
}

/// A deterministic musical craft assessment displayed in Organist Studio.
///
/// This is not an aesthetic oracle. It checks the objective prerequisites of
/// an arrangement that can support great music: pulse, foundation, harmony,
/// melody, cadence, space, a useful palette, and enough form to develop.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompositionReport {
    pub quality: u8,
    pub grade: String,
    pub movement: EvolutionKind,
    pub pulse: bool,
    pub foundation: bool,
    pub harmony: bool,
    pub melody: bool,
    pub cadence: bool,
    pub breathing_room: bool,
    pub palette: bool,
    pub form: bool,
    pub guidance: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Track {
    pub name: String,
    pub waveform: Waveform,
    pub gain: f32,
    pub octave: i8,
    pub steps: Vec<Option<u8>>,
    /// The patch this track plays. Inferred from the name when absent, so a
    /// score written without knowing the schema still gets a sensible voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<Instrument>,
    /// Note length as a multiple of the patch's own, 0.1..4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<f32>,
    /// Stereo position, -1 hard left to 1 hard right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<f32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    Sine,
    Triangle,
    Square,
    Saw,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FeedbackEvent {
    pub score_title: String,
    pub verdict: FeedbackVerdict,
    pub note: String,
    pub unix_ms: u128,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VerificationReport {
    pub duration_seconds: f32,
    pub peak: f32,
    pub rms: f32,
    pub seam_delta: f32,
    pub active_sections: usize,
    pub total_sections: usize,
    /// Spread between the quietest and loudest eighth of the render. A piece
    /// that never changes level has no arrangement, however many notes it has.
    pub dynamic_range: f32,
    /// How strongly the waveform repeats at a musical period, 0..1.
    ///
    /// Pitched material is periodic; noise is not. Too many overlapping voices,
    /// too much reverb, or too much saturation all destroy periodicity, and all
    /// three sound the same from the audience: a wash. Reported so a render can
    /// be checked for being noise rather than only for being loud enough.
    pub periodicity: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackVerdict {
    Keep,
    Reject,
    Revise,
}

#[derive(Debug, Error)]
pub enum MusicError {
    #[error("invalid court score: {0}")]
    Invalid(String),
    #[error("music I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("audio output is unavailable: {0}")]
    Output(String),
    #[error("music JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct Player {
    _stream: OutputStream,
    _handle: OutputStreamHandle,
    sink: Sink,
}

impl Player {
    pub fn play(samples: Vec<f32>, looped: bool) -> Result<Self, MusicError> {
        Self::play_on(samples, looped, None)
    }

    pub fn play_on(
        samples: Vec<f32>,
        looped: bool,
        output_device: Option<&str>,
    ) -> Result<Self, MusicError> {
        use rodio::Source;
        verify_audio(&samples)?;
        let stream = output_device
            .map(open_named_output)
            .unwrap_or_else(OutputStream::try_default);
        let (stream, handle) = stream.map_err(|error| MusicError::Output(error.to_string()))?;
        let sink = Sink::try_new(&handle).map_err(|error| MusicError::Output(error.to_string()))?;
        let source = SamplesBuffer::new(2, SAMPLE_RATE, samples);
        if looped {
            sink.append(source.repeat_infinite());
        } else {
            sink.append(source);
        }
        sink.play();
        Ok(Self {
            _stream: stream,
            _handle: handle,
            sink,
        })
    }

    pub fn stop(&self) {
        self.sink.stop();
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn resume(&self) {
        self.sink.play();
    }

    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }
}

fn open_named_output(name: &str) -> Result<(OutputStream, OutputStreamHandle), rodio::StreamError> {
    let wanted = name.trim().to_ascii_lowercase();
    let device = rodio::cpal::default_host()
        .output_devices()
        .ok()
        .and_then(|mut devices| {
            devices.find(|device| {
                device
                    .name()
                    .is_ok_and(|candidate| candidate.to_ascii_lowercase().contains(&wanted))
            })
        });
    match device {
        Some(device) => OutputStream::try_from_device(&device),
        None => Err(rodio::StreamError::NoDevice),
    }
}

/// Rewrites a track whose steps are trigger positions rather than degrees.
///
/// The schema means "one scale degree per slot, or null for a rest". The
/// Organist repeatedly writes the other obvious thing: the sixteenth-note
/// positions a part fires on, as in `"steps": [0, 8, 16, 24, 32, 40, 48, 56]`
/// for a kick. Read as degrees those are far outside the scale, so the piece
/// either failed validation or, where the numbers happened to be small enough,
/// played at absurd pitches — the sour notes the operator heard.
///
/// A run of strictly ascending values with no rests, reaching beyond the
/// degrees the schema allows, is unambiguous: nobody writes a melody that way.
/// It is converted rather than refused, because the intent is perfectly clear.
fn positions_to_degrees(steps: &[Option<u8>]) -> Option<Vec<Option<u8>>> {
    if steps.len() < 2 {
        return None;
    }
    let values: Vec<u8> = steps.iter().copied().collect::<Option<Vec<_>>>()?;
    let ascending = values.windows(2).all(|pair| pair[1] > pair[0]);
    let beyond_degrees = values.iter().any(|value| *value > 20);
    if !ascending || !beyond_degrees {
        return None;
    }
    let last = *values.last()? as usize;
    // Round up to whole bars of sixteenths so the pattern keeps its meaning.
    let slots = (last / arrange::STEPS_PER_BAR + 1) * arrange::STEPS_PER_BAR;
    if slots > 64 {
        return None;
    }
    let mut rewritten = vec![None; slots];
    for position in values {
        rewritten[position as usize] = Some(0);
    }
    Some(rewritten)
}

impl Score {
    pub fn from_json(raw: &str) -> Result<Self, MusicError> {
        if let Ok(mut score) = serde_json::from_str::<Self>(raw) {
            score.normalise_step_positions();
            score.validate()?;
            return Ok(score);
        }
        let value: serde_json::Value = serde_json::from_str(raw)?;
        let title = value
            .get("title")
            .and_then(|value| value.as_str())
            .unwrap_or("Court Score")
            .to_owned();
        let bpm = value
            .pointer("/transport/bpm")
            .and_then(|value| value.as_u64())
            .unwrap_or(92)
            .clamp(40, 220) as u16;
        let bars = value
            .pointer("/transport/bars")
            .and_then(|value| value.as_u64())
            .unwrap_or(16)
            .clamp(1, 64) as u16;
        let tonic_name = value
            .pointer("/harmony/tonic")
            .and_then(|value| value.as_str())
            .unwrap_or("C");
        let tonic = pitch_class(tonic_name).unwrap_or(0);
        let mode = match value
            .pointer("/harmony/mode")
            .and_then(|value| value.as_str())
            .unwrap_or("minor")
            .to_ascii_lowercase()
            .as_str()
        {
            "major" | "ionian" => Mode::Major,
            "dorian" => Mode::Dorian,
            "mixolydian" => Mode::Mixolydian,
            _ => Mode::Minor,
        };
        let motif: Vec<Option<u8>> = value
            .get("motif")
            .and_then(|value| value.as_array())
            .map(|notes| {
                notes
                    .iter()
                    .filter_map(|note| note.as_str())
                    .map(|note| pitch_class(note).map(|pitch| (pitch + 12 - tonic) % 12))
                    .collect()
            })
            .filter(|notes: &Vec<Option<u8>>| !notes.is_empty())
            .unwrap_or_else(|| {
                vec![
                    Some(0),
                    Some(2),
                    Some(4),
                    None,
                    Some(5),
                    Some(4),
                    Some(2),
                    None,
                ]
            });
        let progression = value
            .pointer("/harmony/progression")
            .or_else(|| value.pointer("/harmony/chords"))
            .and_then(|value| value.as_array())
            .map(|chords| {
                chords
                    .iter()
                    .filter_map(|chord| chord.as_str())
                    .filter_map(|chord| parse_chord(chord, tonic))
                    .take(16)
                    .collect::<Vec<_>>()
            })
            .filter(|progression| !progression.is_empty())
            .unwrap_or_else(default_progression);

        // A score arriving as bare harmony plus a motif is completed into an
        // arrangement. V1's court always had a rhythm section, and a motif over
        // drums alone is all transient and no body: the mix reads as
        // percussion with a tune on top rather than as music.
        let mut tracks = rhythm_section();
        tracks.push(Track {
            name: "Court Harmony".to_owned(),
            waveform: Waveform::Saw,
            gain: 0.52,
            octave: 3,
            steps: vec![Some(0), Some(1), Some(2), Some(1)],
            instrument: Some(Instrument::Pad),
            gate: None,
            pan: None,
        });
        tracks.push(Track {
            name: "Court Keys".to_owned(),
            waveform: Waveform::Triangle,
            gain: 0.44,
            octave: 4,
            steps: vec![Some(0), None, Some(2), None, Some(4), None, Some(2), None],
            instrument: Some(Instrument::Keys),
            gate: None,
            pan: Some(0.2),
        });
        tracks.push(Track {
            name: "Court Bass".to_owned(),
            waveform: Waveform::Saw,
            gain: 0.7,
            octave: 2,
            steps: vec![Some(0), None, Some(0), None, Some(0), None, Some(4), None],
            instrument: Some(Instrument::Bass),
            gate: None,
            pan: None,
        });
        tracks.push(Track {
            name: "Authored Motif".to_owned(),
            waveform: Waveform::Saw,
            gain: 0.62,
            octave: 5,
            steps: motif,
            instrument: Some(Instrument::Lead),
            gate: None,
            pan: Some(-0.12),
        });

        let score = Self {
            title,
            bpm,
            bars,
            tonic,
            mode,
            progression,
            tracks,
            swing: None,
            seed: None,
            sidechain: None,
            evolution: 0,
        };
        score.validate()?;
        Ok(score)
    }

    /// Rewrites any track that listed trigger positions instead of degrees.
    ///
    /// Refusing them would be the tidier rule, but the intent is unmistakable
    /// and the court has no way to learn the distinction from an error message.
    fn normalise_step_positions(&mut self) {
        for track in &mut self.tracks {
            if let Some(rewritten) = positions_to_degrees(&track.steps) {
                track.steps = rewritten;
            }
        }
    }

    pub(crate) fn swing(&self) -> f32 {
        self.swing.unwrap_or(0.0)
    }

    pub(crate) fn seed(&self) -> u64 {
        self.seed.unwrap_or(0x5eed_c0de_1234_5678)
    }

    pub fn validate(&self) -> Result<(), MusicError> {
        if self.title.trim().is_empty() {
            return Err(MusicError::Invalid("title is empty".to_owned()));
        }
        if self.title.chars().count() > 160 {
            return Err(MusicError::Invalid(
                "title must be at most 160 characters".to_owned(),
            ));
        }
        if !(40..=220).contains(&self.bpm) {
            return Err(MusicError::Invalid("BPM must be 40..=220".to_owned()));
        }
        if !(1..=64).contains(&self.bars) {
            return Err(MusicError::Invalid("bars must be 1..=64".to_owned()));
        }
        if self.tonic > 11 {
            return Err(MusicError::Invalid(
                "tonic must be a pitch class 0..=11".to_owned(),
            ));
        }
        if self.tracks.is_empty() || self.tracks.len() > 8 {
            return Err(MusicError::Invalid("a score needs 1..=8 tracks".to_owned()));
        }
        if self.progression.len() > 16
            || self.progression.iter().any(|chord| chord.root_degree > 20)
        {
            return Err(MusicError::Invalid(
                "harmony progression must contain at most 16 bounded chords".to_owned(),
            ));
        }
        if self.swing().is_nan() || !(0.0..=0.6).contains(&self.swing()) {
            return Err(MusicError::Invalid("swing must be 0..=0.6".to_owned()));
        }
        if let Some(sidechain) = self.sidechain
            && (sidechain.is_nan() || !(0.0..=0.85).contains(&sidechain))
        {
            return Err(MusicError::Invalid("sidechain must be 0..=0.85".to_owned()));
        }
        for track in &self.tracks {
            if track.name.trim().is_empty() || track.name.chars().count() > 120 {
                return Err(MusicError::Invalid(
                    "track names must contain 1..=120 characters".to_owned(),
                ));
            }
            if track.steps.is_empty() || track.steps.len() > 64 {
                return Err(MusicError::Invalid(format!(
                    "track '{}' needs 1..=64 steps",
                    track.name
                )));
            }
            if !(0.0..=1.0).contains(&track.gain) {
                return Err(MusicError::Invalid(format!(
                    "track '{}' gain must be 0..=1",
                    track.name
                )));
            }
            if !(0..=8).contains(&track.octave) {
                return Err(MusicError::Invalid(format!(
                    "track '{}' octave must be 0..=8",
                    track.name
                )));
            }
            if track.steps.iter().flatten().any(|degree| *degree > 20) {
                return Err(MusicError::Invalid(format!(
                    "track '{}' scale degrees must be 0..=20",
                    track.name
                )));
            }
            if let Some(pan) = track.pan
                && (pan.is_nan() || !(-1.0..=1.0).contains(&pan))
            {
                return Err(MusicError::Invalid(format!(
                    "track '{}' pan must be -1..=1",
                    track.name
                )));
            }
            if let Some(gate) = track.gate
                && (gate.is_nan() || !(0.1..=4.0).contains(&gate))
            {
                return Err(MusicError::Invalid(format!(
                    "track '{}' gate must be 0.1..=4",
                    track.name
                )));
            }
        }
        Ok(())
    }

    /// Length of one complete arranged pass through this score.
    pub fn duration_seconds(&self) -> f32 {
        f32::from(self.bars) * 4.0 * 60.0 / f32::from(self.bpm)
    }

    /// A track name that still describes the voice after a re-orchestration.
    ///
    /// The name is what the DAW and the transcript show, and `from_name` reads
    /// it when a score arrives without an explicit role, so a track called
    /// "Keys" playing an organ is a lie in two places. Any flavour the Organist
    /// gave the name is kept: "Chapel Organ" becomes "Chapel Reed", not "Reed".
    fn renamed_for(name: &str, from: Instrument, to: Instrument) -> String {
        let old = from.label();
        if let Some(at) = name.to_ascii_lowercase().find(&old.to_ascii_lowercase()) {
            let mut renamed = String::with_capacity(name.len() + to.label().len());
            renamed.push_str(&name[..at]);
            renamed.push_str(to.label());
            renamed.push_str(&name[at + old.len()..]);
            return renamed;
        }
        // No label to swap, so replace the trailing word — the part of a name
        // like "Silver Flute" that says what is playing.
        match name.trim_end().rsplit_once(char::is_whitespace) {
            Some((prefix, _)) => format!("{prefix} {}", to.label()),
            None => to.label().to_owned(),
        }
    }

    pub const fn evolution_kind(&self) -> EvolutionKind {
        EvolutionKind::for_revision(self.evolution)
    }

    /// Restores the objective arrangement foundations missing from sparse
    /// legacy or model-authored scores.
    ///
    /// The original melody and voices are preserved whenever possible. Missing
    /// functional seats receive bounded native parts; if all eight seats are
    /// occupied, only a duplicate functional role is replaced. This turns an
    /// incomplete sketch into a performance arrangement without executing a
    /// second composer or inventing unbounded score data.
    pub fn mastered(&self) -> Result<Self, MusicError> {
        self.validate()?;
        let mut next = self.clone();
        next.bars = next.bars.clamp(8, 16);
        let cadence = next
            .progression
            .first()
            .zip(next.progression.last())
            .is_some_and(|(first, last)| {
                first.root_degree % 7 == 0 && matches!(last.root_degree % 7, 3 | 4 | 6)
            });
        if !(3..=8).contains(&next.progression.len()) || !cadence {
            next.progression = progression_for(next.mode, next.evolution as usize);
        }

        let report = next.composition_report();
        if !report.pulse {
            insert_performance_voice(&mut next, performance_track(Instrument::Kick));
        }
        if !report.foundation {
            insert_performance_voice(&mut next, performance_track(Instrument::Bass));
        }
        if !report.harmony {
            insert_performance_voice(&mut next, performance_track(Instrument::Organ));
        }
        if !report.melody {
            insert_performance_voice(&mut next, performance_track(Instrument::Flute));
        }

        if !next.composition_report().breathing_room
            && let Some(track) = next.tracks.iter_mut().find(|track| {
                matches!(
                    track.instrument(),
                    Instrument::Lead
                        | Instrument::Flute
                        | Instrument::Reed
                        | Instrument::Arp
                        | Instrument::Bell
                )
            })
        {
            for (index, step) in track.steps.iter_mut().enumerate() {
                if index % 4 == 3 {
                    *step = None;
                }
            }
        }

        for voice in [Instrument::Harp, Instrument::Hat, Instrument::Piano] {
            if next.composition_report().palette {
                break;
            }
            if !next.tracks.iter().any(|track| track.instrument() == voice) {
                insert_performance_voice(&mut next, performance_track(voice));
            }
        }

        next.validate()?;
        let report = next.composition_report();
        if report.quality < 75 {
            return Err(MusicError::Invalid(format!(
                "arrangement mastering reached only {}/100: {}",
                report.quality,
                report.guidance.join("; ")
            )));
        }
        Ok(next)
    }

    /// Scores the objective craft foundations of the current arrangement.
    pub fn composition_report(&self) -> CompositionReport {
        let instruments = self
            .tracks
            .iter()
            .map(Track::instrument)
            .collect::<Vec<_>>();
        let pulse = instruments.iter().any(|instrument| {
            matches!(
                instrument,
                Instrument::Kick | Instrument::Snare | Instrument::Hat | Instrument::Perc
            )
        }) && self.tracks.iter().any(|track| {
            track.instrument().is_percussive()
                && track.steps.iter().filter(|step| step.is_some()).count() >= 2
        });
        let foundation = instruments.iter().any(|instrument| {
            matches!(
                instrument,
                Instrument::SubBass | Instrument::Bass | Instrument::Cello
            )
        });
        let harmony = !self.progression.is_empty()
            && instruments.iter().any(|instrument| {
                matches!(
                    instrument,
                    Instrument::Pad | Instrument::Keys | Instrument::Organ | Instrument::Piano
                )
            });
        let melodic_tracks = self
            .tracks
            .iter()
            .filter(|track| {
                matches!(
                    track.instrument(),
                    Instrument::Lead
                        | Instrument::Flute
                        | Instrument::Reed
                        | Instrument::Arp
                        | Instrument::Bell
                )
            })
            .collect::<Vec<_>>();
        let melody = melodic_tracks.iter().any(|track| {
            let active = track.steps.iter().filter(|step| step.is_some()).count();
            let pitches = track
                .steps
                .iter()
                .flatten()
                .copied()
                .collect::<std::collections::HashSet<_>>();
            active >= 3 && pitches.len() >= 3
        });
        let cadence = self
            .progression
            .first()
            .zip(self.progression.last())
            .is_some_and(|(first, last)| {
                first.root_degree % 7 == 0 && matches!(last.root_degree % 7, 3 | 4 | 6)
            });
        let breathing_room = melodic_tracks.iter().any(|track| {
            let rests = track.steps.iter().filter(|step| step.is_none()).count();
            rests * 4 >= track.steps.len()
        });
        let palette = instruments
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len()
            >= 5;
        let form =
            self.bars >= 8 && (3..=8).contains(&self.progression.len()) && self.tracks.len() >= 5;

        let quality = u8::from(pulse) * 14
            + u8::from(foundation) * 14
            + u8::from(harmony) * 14
            + u8::from(melody) * 14
            + u8::from(cadence) * 12
            + u8::from(breathing_room) * 12
            + u8::from(palette) * 10
            + u8::from(form) * 10;
        let grade = match quality {
            90..=100 => "Masterwork",
            75..=89 => "Concert ready",
            55..=74 => "Developing",
            _ => "Sketch",
        }
        .to_owned();
        let mut guidance = Vec::new();
        for (ready, message) in [
            (pulse, "establish a deliberate rhythmic pulse"),
            (foundation, "give the harmony a bass foundation"),
            (harmony, "add a chordal middle voice"),
            (melody, "write a distinct, singable melodic line"),
            (cadence, "make the loop cadence back toward its tonic"),
            (breathing_room, "leave rests so the melody can breathe"),
            (palette, "use at least five distinct instrumental colours"),
            (form, "allow enough bars and parts for an arc"),
        ] {
            if !ready {
                guidance.push(message.to_owned());
            }
        }

        CompositionReport {
            quality,
            grade,
            movement: self.evolution_kind(),
            pulse,
            foundation,
            harmony,
            melody,
            cadence,
            breathing_room,
            palette,
            form,
            guidance,
        }
    }

    /// Produces a bounded, recognisably related but audibly different score.
    ///
    /// Model composition remains useful for genuinely new pieces, but it is
    /// not a clock: live evidence showed unchanged or malformed replies while
    /// one loop repeated for minutes. This transformation is the runtime's
    /// deterministic guarantee that tempo, harmony, rhythm, and melodic
    /// material move even when no inference is available.
    pub fn evolved(&self) -> Result<Self, MusicError> {
        self.validate()?;
        let mut next = self.clone();
        next.evolution = self.evolution.wrapping_add(1).max(1);
        let revision = next.evolution as usize;

        let kind = EvolutionKind::for_revision(next.evolution);
        let base_title = self
            .title
            .split(" // ")
            .next()
            .unwrap_or(self.title.as_str())
            .trim()
            .chars()
            .take(112)
            .collect::<String>();
        next.title = format!(
            "{base_title} // Movement {}: {}",
            next.evolution,
            kind.label()
        );

        // Key and tempo are anchors, not variables.
        //
        // Every variation used to move all three: a new tonic, the next mode,
        // and five to nine BPM. A variation is accepted at least every thirty-six
        // seconds, so the live ledger showed key or mode changing on 98% of
        // transitions and tempo on 96% — thousands of them. Music that changes
        // key and tempo every movement gives the ear no tonic to hold and
        // no pulse to settle into, which is heard as chaos however good each
        // individual piece is. They now hold for a long run and move together,
        // so the change reads as a deliberate turn rather than a lurch.
        if kind == EvolutionKind::Modulation {
            // A fourth or a fifth: the closest relations, so the new key still
            // sounds like somewhere the old one could go.
            let step = if (revision / HARMONIC_ANCHOR_VARIATIONS).is_multiple_of(2) {
                7
            } else {
                5
            };
            next.tonic = (self.tonic + step) % 12;
            next.mode = match self.mode {
                Mode::Major => Mode::Dorian,
                Mode::Dorian => Mode::Mixolydian,
                Mode::Mixolydian => Mode::Minor,
                Mode::Minor => Mode::Major,
            };

            // Tempo moves on the same boundary and by a smaller amount, so a
            // turn changes the colour of the music rather than its genre.
            // Sums to zero over the cycle, so a long stream never marches toward
            // either tempo limit, while still spanning enough to be heard.
            let delta = [6_i32, -4, 7, -9][(revision / HARMONIC_ANCHOR_VARIATIONS) % 4];
            let forward = i32::from(self.bpm) + delta;
            let tempo = if (60..=168).contains(&forward) {
                forward
            } else {
                i32::from(self.bpm) - delta
            };
            next.bpm = tempo.clamp(40, 220) as u16;
        }

        if kind == EvolutionKind::Harmony {
            if next.progression.len() < 3 {
                next.progression = progression_for(next.mode, revision);
            } else {
                let last = next.progression.len() - 1;
                let changed = 1 + revision % (last - 1).max(1);
                let root = (next.progression[changed].root_degree + 2 + revision as u8 % 2) % 7;
                next.progression[0] = Chord {
                    root_degree: 0,
                    quality: diatonic_quality(next.mode, 0),
                };
                next.progression[changed] = Chord {
                    root_degree: root,
                    quality: diatonic_quality(next.mode, root),
                };
                let cadence = if revision.is_multiple_of(2) { 4 } else { 6 };
                next.progression[last] = Chord {
                    root_degree: cadence,
                    quality: diatonic_quality(next.mode, cadence),
                };
            }
        } else if kind == EvolutionKind::Modulation {
            if next.progression.len() < 3 {
                next.progression = progression_for(next.mode, revision);
            } else {
                let last = next.progression.len() - 1;
                next.progression[0].root_degree = 0;
                next.progression[last].root_degree = 4;
                for chord in &mut next.progression {
                    chord.quality = diatonic_quality(next.mode, chord.root_degree);
                }
            }
        }

        // Orchestration is an axis too.
        //
        // Everything above rewrites notes; nothing touched who plays them. The
        // live ledger showed a piece four thousand variations deep still voiced
        // by the six synths its first score happened to name, because evolution
        // preserved the roster and a genuinely new score from the model is rare.
        // Re-voicing within a role changes the colour while leaving register,
        // rhythm, and harmony alone, so an acoustic ensemble is reachable from
        // any lineage rather than only from a fresh composition. It moves on its
        // own slower cadence: a part that changed instrument every movement
        // would be a patch demo, not an arrangement.
        let revoice = kind == EvolutionKind::Texture;

        for (index, track) in next.tracks.iter_mut().enumerate() {
            let mut instrument = track
                .instrument
                .unwrap_or_else(|| Instrument::from_name(&track.name));

            if revoice {
                let family = instrument.family();
                if !family.is_empty() {
                    // Offset by index so the ensemble re-colours a voice at a
                    // time rather than all moving in lockstep.
                    let seat = family.iter().position(|voice| *voice == instrument);
                    let step = revision / HARMONIC_ANCHOR_VARIATIONS + index;
                    let next_voice = family[(seat.unwrap_or(0) + step) % family.len()];
                    if next_voice != instrument {
                        track.name = Self::renamed_for(&track.name, instrument, next_voice);
                        instrument = next_voice;
                        track.instrument = Some(next_voice);
                    }
                }
            }

            let len = track.steps.len();
            if len == 0 {
                continue;
            }
            let target = (revision * 3 + index * 5) % len;
            let top_line = matches!(
                instrument,
                Instrument::Lead | Instrument::Flute | Instrument::Reed | Instrument::Arp
            );
            let counter_line = matches!(
                instrument,
                Instrument::Keys
                    | Instrument::Organ
                    | Instrument::Pad
                    | Instrument::Piano
                    | Instrument::Harp
                    | Instrument::Pluck
                    | Instrument::Bell
            );
            let foundation = matches!(
                instrument,
                Instrument::SubBass | Instrument::Bass | Instrument::Cello
            );

            match kind {
                EvolutionKind::Pulse if instrument.is_percussive() => {
                    track.steps.rotate_right(1 + (index + revision) % 2);
                    let active = track.steps.iter().filter(|step| step.is_some()).count();
                    track.steps[target] = match track.steps[target] {
                        Some(_) if active > 2 && instrument != Instrument::Kick => None,
                        None => Some(0),
                        value => value,
                    };
                    if instrument == Instrument::Kick {
                        track.steps[0] = Some(0);
                    }
                }
                EvolutionKind::Motif if top_line => {
                    track.steps.rotate_right(1 + (revision + index) % 3);
                    let shift = 1 + ((revision + index) % 2) as u8;
                    for (step, degree) in track.steps.iter_mut().enumerate() {
                        if let Some(degree) = degree
                            && step.is_multiple_of(2)
                        {
                            *degree = (*degree + shift) % 7;
                        }
                    }
                    if track.steps[target].is_none() {
                        track.steps[target] = Some(((revision + index) % 7) as u8);
                    }
                }
                EvolutionKind::Counterpoint if counter_line => {
                    track.steps.rotate_left(1 + (revision + index) % 2);
                    for degree in track.steps.iter_mut().flatten().step_by(2) {
                        *degree = (*degree + 2) % 7;
                    }
                }
                EvolutionKind::Harmony if foundation || counter_line => {
                    for degree in track.steps.iter_mut().flatten() {
                        *degree = (*degree + 1 + (index % 2) as u8) % 7;
                    }
                }
                _ => {}
            }
        }

        if kind == EvolutionKind::Pulse {
            next.swing = Some([0.10, 0.16, 0.22, 0.14][revision % 4]);
            next.sidechain = Some([0.28, 0.36, 0.44, 0.32][revision % 4]);
        }
        next.seed = Some(
            self.seed()
                .wrapping_add(0x9e37_79b9_7f4a_7c15_u64.wrapping_mul(next.evolution)),
        );
        next.validate()?;
        Ok(next)
    }

    /// Renders the score to interleaved stereo.
    pub fn render(&self) -> Result<Vec<f32>, MusicError> {
        self.validate()?;
        let plan = self.arrange();
        // Everything is rendered past the end of the loop, then folded back
        // over the opening. Reverb tails and note releases that run off the end
        // arrive at the start of the next pass, which is what lets the loop
        // repeat without a seam.
        let total = plan.frames + plan.tail_frames;

        let mut instruments = vec![0.0_f32; total * 2];
        let mut drums = vec![0.0_f32; total * 2];
        let mut delay_send = vec![0.0_f32; total * 2];
        let mut reverb_send = vec![0.0_f32; total * 2];

        for event in &plan.events {
            let track = &self.tracks[event.track];
            let patch = event.instrument.patch();
            let frequency = 440.0 * 2.0_f32.powf((event.midi - 69.0) / 12.0);
            if !frequency.is_finite() || frequency <= 0.0 {
                continue;
            }
            let level = track.gain * mix_level(event.instrument);
            let pan = track.pan.unwrap_or_else(|| default_pan(event.instrument));
            let (pan_left, pan_right) = equal_power(pan);
            let wide = patch.width > 0.5;

            let mut voice = synth::Voice::new(
                patch,
                frequency,
                event.velocity,
                event.gate_seconds,
                event.seed,
            );
            let mut chorus = wide.then(effects::Chorus::new);
            let frames = voice.frames().min(total.saturating_sub(event.start_frame));
            let percussive = event.instrument.is_percussive();

            for offset in 0..frames {
                let mono = voice.next(event.cutoff_scale) * level;
                if mono == 0.0 && voice.finished() {
                    break;
                }
                let (mut left, mut right) = (mono * pan_left, mono * pan_right);
                if let Some(chorus) = chorus.as_mut() {
                    let (a, b) = chorus.process(mono);
                    let blend = patch.width * 0.5;
                    left += a * blend;
                    right += b * blend;
                }

                let frame = (event.start_frame + offset) * 2;
                let bus = if percussive {
                    &mut drums
                } else {
                    &mut instruments
                };
                bus[frame] += left;
                bus[frame + 1] += right;
                delay_send[frame] += left * patch.delay;
                delay_send[frame + 1] += right * patch.delay;
                reverb_send[frame] += left * patch.reverb;
                reverb_send[frame + 1] += right * patch.reverb;
            }
        }

        // Ducking is applied to everything except the drums that trigger it,
        // including their own sends, so the pump is heard as movement rather
        // than as the reverb pulling the kick down with it.
        let duck = effects::sidechain_envelope(
            total,
            &plan.kick_onsets,
            self.bpm,
            self.sidechain.unwrap_or(0.34),
        );
        for (frame, gain) in duck.iter().enumerate().take(total) {
            let gain = *gain;
            for channel in 0..2 {
                let index = frame * 2 + channel;
                instruments[index] *= gain;
                delay_send[index] *= gain;
                reverb_send[index] *= gain;
            }
        }

        let mut delay = effects::PingPongDelay::dotted_eighth(self.bpm, 0.42);
        let mut reverb = effects::Reverb::hall(1.0);
        let mut master = vec![0.0_f32; total * 2];
        for frame in 0..total {
            let index = frame * 2;
            let (delay_left, delay_right) =
                delay.process((delay_send[index], delay_send[index + 1]));
            // The delay is not fed into the reverb. Smearing repeats into a
            // tail turns rhythmic echoes into a continuous wash.
            let wet_reverb_input = (reverb_send[index], reverb_send[index + 1]);
            let (reverb_left, reverb_right) = reverb.process(wet_reverb_input);

            let left = instruments[index] + drums[index] + delay_left * 0.30 + reverb_left * 0.26;
            let right = instruments[index + 1]
                + drums[index + 1]
                + delay_right * 0.30
                + reverb_right * 0.26;
            // Barely any master saturation. On dense material a hard curve
            // intermodulates every pair of partials, and that sits under the
            // music as broadband noise.
            master[index] = effects::saturate(left, 1.05);
            master[index + 1] = effects::saturate(right, 1.05);
        }

        // Fold the tail over the opening and discard it.
        let wrap = plan.tail_frames.min(plan.frames);
        for frame in 0..wrap {
            let source = (plan.frames + frame) * 2;
            let target = frame * 2;
            master[target] += master[source];
            master[target + 1] += master[source + 1];
        }
        master.truncate(plan.frames * 2);

        effects::master_tone(&mut master, 0.20);
        effects::limit(&mut master, 0.97);
        effects::normalise(&mut master, MASTER_PEAK);
        if master.iter().any(|sample| !sample.is_finite()) {
            return Err(MusicError::Invalid(
                "synthesis produced a non-finite sample".to_owned(),
            ));
        }
        Ok(master)
    }
}

/// Relative loudness per role, so a score does not have to balance its own mix.
fn mix_level(instrument: Instrument) -> f32 {
    match instrument {
        Instrument::Kick => 0.88,
        Instrument::Snare => 0.70,
        Instrument::Hat => 0.34,
        Instrument::Perc => 0.50,
        // The low end carries far more energy per unit of loudness than
        // anything else, so it is mixed well below the parts it supports. A
        // score with both a kick and a bass otherwise arrives boomy.
        Instrument::SubBass => 0.60,
        Instrument::Bass => 0.58,
        Instrument::Pad => 0.42,
        Instrument::Keys => 0.50,
        Instrument::Pluck => 0.56,
        Instrument::Arp => 0.38,
        Instrument::Lead => 0.66,
        Instrument::Bell => 0.52,
        // The acoustic voices sit lower than the synth ones. They carry more
        // harmonic content per note, so matching their nominal gain would put
        // them in front of everything else.
        Instrument::Organ => 0.44,
        Instrument::Piano => 0.54,
        Instrument::Flute => 0.50,
        Instrument::Reed => 0.48,
        Instrument::Cello => 0.56,
        Instrument::Harp => 0.50,
    }
}

/// Where a role sits by default.
///
/// Low frequencies stay centred because they carry the most energy and would
/// unbalance the mix anywhere else; everything above them is spread so the
/// parts stop competing for the same space.
fn default_pan(instrument: Instrument) -> f32 {
    match instrument {
        Instrument::Kick | Instrument::SubBass | Instrument::Bass | Instrument::Snare => 0.0,
        Instrument::Hat => 0.34,
        Instrument::Perc => -0.42,
        Instrument::Pad => 0.0,
        Instrument::Keys => 0.18,
        Instrument::Pluck => 0.30,
        Instrument::Arp => -0.28,
        Instrument::Lead => -0.10,
        Instrument::Bell => 0.22,
        // Roughly where these sit in front of a listener in a real ensemble.
        Instrument::Organ => 0.0,
        Instrument::Piano => -0.20,
        Instrument::Flute => 0.36,
        Instrument::Reed => -0.34,
        Instrument::Cello => 0.26,
        Instrument::Harp => -0.30,
    }
}

/// Constant-power panning, which keeps loudness even across the field.
fn equal_power(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// Kick, snare and hats: the pulse every other part is written against.
fn rhythm_section() -> Vec<Track> {
    vec![
        Track {
            name: "Court Kick".to_owned(),
            waveform: Waveform::Sine,
            gain: 0.92,
            octave: 2,
            steps: vec![
                Some(0),
                None,
                None,
                None,
                None,
                None,
                Some(0),
                None,
                None,
                None,
                Some(0),
                None,
                None,
                None,
                None,
                None,
            ],
            instrument: Some(Instrument::Kick),
            gate: None,
            pan: None,
        },
        Track {
            name: "Court Snare".to_owned(),
            waveform: Waveform::Triangle,
            gain: 0.72,
            octave: 3,
            steps: vec![
                None,
                None,
                None,
                None,
                Some(0),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
                None,
                None,
                None,
            ],
            instrument: Some(Instrument::Snare),
            gate: None,
            pan: None,
        },
        Track {
            name: "Court Hats".to_owned(),
            waveform: Waveform::Square,
            gain: 0.55,
            octave: 6,
            steps: vec![
                None,
                None,
                Some(0),
                None,
                None,
                None,
                Some(0),
                None,
                None,
                None,
                Some(0),
                None,
                None,
                Some(0),
                Some(0),
                None,
            ],
            instrument: Some(Instrument::Hat),
            gate: None,
            pan: None,
        },
    ]
}

fn arrangement_role(instrument: Instrument) -> usize {
    match instrument {
        Instrument::Kick | Instrument::Snare | Instrument::Hat | Instrument::Perc => 0,
        Instrument::SubBass | Instrument::Bass | Instrument::Cello => 1,
        Instrument::Pad | Instrument::Keys | Instrument::Organ => 2,
        Instrument::Lead | Instrument::Flute | Instrument::Reed | Instrument::Arp => 3,
        Instrument::Piano | Instrument::Harp | Instrument::Pluck | Instrument::Bell => 4,
    }
}

fn insert_performance_voice(score: &mut Score, track: Track) {
    if score.tracks.len() < 8 {
        score.tracks.push(track);
        return;
    }
    let mut counts = [0_usize; 5];
    for existing in &score.tracks {
        counts[arrangement_role(existing.instrument())] += 1;
    }
    if let Some(index) = score
        .tracks
        .iter()
        .rposition(|existing| counts[arrangement_role(existing.instrument())] > 1)
    {
        score.tracks[index] = track;
    }
}

fn performance_track(instrument: Instrument) -> Track {
    const R: Option<u8> = None;
    let (name, waveform, gain, octave, steps, gate, pan) = match instrument {
        Instrument::Kick => (
            "Master Kick",
            Waveform::Sine,
            0.86,
            2,
            vec![
                Some(0),
                R,
                R,
                R,
                Some(0),
                R,
                R,
                R,
                Some(0),
                R,
                Some(0),
                R,
                Some(0),
                R,
                R,
                R,
            ],
            None,
            Some(0.0),
        ),
        Instrument::Bass => (
            "Master Bass",
            Waveform::Saw,
            0.64,
            2,
            vec![Some(0), R, R, R, Some(0), R, Some(4), R],
            Some(0.88),
            Some(0.0),
        ),
        Instrument::Organ => (
            "Master Organ",
            Waveform::Triangle,
            0.46,
            4,
            vec![Some(0), R, R, R, Some(2), R, R, R],
            Some(1.4),
            Some(-0.08),
        ),
        Instrument::Flute => (
            "Master Flute",
            Waveform::Sine,
            0.52,
            6,
            vec![
                Some(0),
                R,
                R,
                Some(2),
                R,
                R,
                Some(4),
                R,
                R,
                Some(6),
                R,
                R,
                Some(4),
                R,
                R,
                R,
            ],
            Some(0.92),
            Some(0.28),
        ),
        Instrument::Harp => (
            "Master Harp",
            Waveform::Triangle,
            0.44,
            5,
            vec![Some(0), R, Some(2), Some(4), R, Some(6), R, Some(4)],
            Some(0.72),
            Some(-0.32),
        ),
        Instrument::Hat => (
            "Master Hats",
            Waveform::Square,
            0.44,
            6,
            vec![
                R,
                R,
                Some(0),
                R,
                R,
                R,
                Some(0),
                R,
                R,
                R,
                Some(0),
                R,
                R,
                Some(0),
                R,
                R,
            ],
            None,
            Some(0.34),
        ),
        Instrument::Piano => (
            "Master Piano",
            Waveform::Triangle,
            0.42,
            4,
            vec![Some(0), R, Some(2), R, Some(4), R, Some(2), R],
            Some(0.82),
            Some(-0.20),
        ),
        _ => unreachable!("only bounded mastering voices are constructed"),
    };
    Track {
        name: name.to_owned(),
        waveform,
        gain,
        octave,
        steps,
        instrument: Some(instrument),
        gate,
        pan,
    }
}

/// The score the court plays when it has written nothing else yet.
///
/// It exists to be heard on stream, so it is a complete arrangement rather than
/// a schema demonstration: rhythm section, sub, pad, arpeggio, and a lead.
pub fn default_score() -> Score {
    // An ensemble rather than a synth stack, written the way the Organist is
    // told to write: a sustaining bass, a chordal middle, a plucked colour, and
    // one breathing melody with rests in it. Each voice occupies its own
    // register, so nothing has to fight to be heard.
    let track =
        |name: &str, instrument: Instrument, gain: f32, octave: i8, steps: Vec<Option<u8>>| Track {
            name: name.to_owned(),
            waveform: Waveform::Sine,
            gain,
            octave,
            steps,
            instrument: Some(instrument),
            gate: None,
            pan: None,
        };
    const R: Option<u8> = None;

    Score {
        title: "Velvet Circuit Procession".to_owned(),
        bpm: 96,
        bars: 16,
        tonic: 2,
        mode: Mode::Dorian,
        progression: default_progression(),
        tracks: vec![
            track(
                "Court Kick",
                Instrument::Kick,
                0.88,
                2,
                vec![
                    Some(0),
                    R,
                    R,
                    R,
                    R,
                    R,
                    Some(0),
                    R,
                    R,
                    R,
                    Some(0),
                    R,
                    R,
                    R,
                    R,
                    R,
                ],
            ),
            track(
                "Court Hats",
                Instrument::Hat,
                0.5,
                6,
                vec![
                    R,
                    R,
                    Some(0),
                    R,
                    R,
                    R,
                    Some(0),
                    R,
                    R,
                    R,
                    Some(0),
                    R,
                    R,
                    Some(0),
                    R,
                    R,
                ],
            ),
            // Bowed, long, and alone at the bottom.
            track(
                "Throne Cello",
                Instrument::Cello,
                0.72,
                3,
                vec![Some(0), R, Some(4), R],
            ),
            // Held chords, which is what an organ is for.
            track(
                "Chapel Organ",
                Instrument::Organ,
                0.5,
                4,
                vec![Some(0), R, R, R, Some(2), R, R, R],
            ),
            // Plucked colour that decays on its own.
            track(
                "Court Harp",
                Instrument::Harp,
                0.5,
                5,
                vec![Some(0), R, Some(2), Some(4), R, Some(6), R, Some(4)],
            ),
            // The melody, with room to breathe between phrases.
            track(
                "Silver Flute",
                Instrument::Flute,
                0.56,
                6,
                vec![
                    Some(0),
                    R,
                    R,
                    Some(2),
                    R,
                    R,
                    Some(4),
                    R,
                    R,
                    R,
                    Some(2),
                    R,
                    R,
                    R,
                    R,
                    R,
                ],
            ),
            // A counter-line answering the flute, in the register below it.
            track(
                "Court Reed",
                Instrument::Reed,
                0.46,
                4,
                vec![R, R, R, R, Some(4), R, Some(2), R],
            ),
        ],
        swing: Some(0.14),
        seed: Some(0x07e1_ed2a),
        sidechain: Some(0.30),
        evolution: 0,
    }
}

pub fn write_wav(path: &Path, samples: &[f32]) -> Result<(), MusicError> {
    verify_audio(samples)?;
    let mut writer = BufWriter::new(File::create(path)?);
    let data_bytes = samples.len() as u32 * 2;
    writer.write_all(b"RIFF")?;
    writer.write_all(&(36 + data_bytes).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&2_u16.to_le_bytes())?;
    writer.write_all(&SAMPLE_RATE.to_le_bytes())?;
    writer.write_all(&(SAMPLE_RATE * 4).to_le_bytes())?;
    writer.write_all(&4_u16.to_le_bytes())?;
    writer.write_all(&16_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_bytes.to_le_bytes())?;
    for sample in samples {
        writer.write_all(&((*sample * i16::MAX as f32) as i16).to_le_bytes())?;
    }
    writer.flush()?;
    Ok(())
}

/// The most recent verdicts an operator recorded, newest first.
///
/// Feedback was written and never read. The buttons archived a keeper and a
/// rejection to disk, and no part of the court ever consulted either, so the
/// Organist composed with no knowledge of what had already been liked or
/// refused — every verdict the operator gave was discarded the moment it was
/// filed.
pub fn recent_feedback(directory: &Path, limit: usize) -> Vec<FeedbackEvent> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut events: Vec<FeedbackEvent> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|raw| serde_json::from_str::<FeedbackEvent>(&raw).ok())
        .collect();
    events.sort_by_key(|event| std::cmp::Reverse(event.unix_ms));
    events.truncate(limit);
    events
}

pub fn record_feedback(
    directory: &Path,
    score: &Score,
    verdict: FeedbackVerdict,
    note: impl Into<String>,
) -> Result<PathBuf, MusicError> {
    fs::create_dir_all(directory)?;
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let event = FeedbackEvent {
        score_title: score.title.clone(),
        verdict,
        note: note.into().chars().take(4_000).collect(),
        unix_ms,
    };
    let sequence = FEEDBACK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!(
        "music-feedback-{unix_ms}-{}-{sequence}.json",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    file.write_all(&serde_json::to_vec_pretty(&event)?)?;
    file.flush()?;
    Ok(path)
}

fn validate_audio(samples: &[f32]) -> Result<(), MusicError> {
    let maximum = SAMPLE_RATE as usize * 2 * MAX_AUDIO_SECONDS;
    if samples.is_empty() || !samples.len().is_multiple_of(2) || samples.len() > maximum {
        return Err(MusicError::Invalid(format!(
            "stereo audio must contain 1..={maximum} interleaved samples"
        )));
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(MusicError::Invalid(
            "audio contains a non-finite sample".to_owned(),
        ));
    }
    Ok(())
}

/// Rejects technically valid but unusable court music before it reaches OBS.
///
/// The checks mirror V1's verifier at the native score boundary: finite PCM,
/// meaningful level, no clipping, sustained sections, and a bounded loop seam.
/// Dynamic range is reported but not enforced, because a deliberately static
/// drone is a legitimate thing for the court to ask for.
pub fn verify_audio(samples: &[f32]) -> Result<VerificationReport, MusicError> {
    validate_audio(samples)?;
    let frames = samples.len() / 2;
    let duration_seconds = frames as f32 / SAMPLE_RATE as f32;
    if duration_seconds < 4.0 {
        return Err(MusicError::Invalid(format!(
            "render is too short for court playback ({duration_seconds:.1}s)"
        )));
    }
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    if peak >= 0.985 {
        return Err(MusicError::Invalid(format!(
            "render clips or leaves unsafe headroom (peak {peak:.3})"
        )));
    }
    let rms =
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt();
    if rms < 0.004 {
        return Err(MusicError::Invalid(format!(
            "render is effectively silent (RMS {rms:.4})"
        )));
    }

    let total_sections = 8.min(frames.max(1));
    let section_frames = frames / total_sections;
    let section_levels: Vec<f32> = (0..total_sections)
        .map(|section| {
            let start = section * section_frames * 2;
            let end = if section + 1 == total_sections {
                samples.len()
            } else {
                (section + 1) * section_frames * 2
            };
            let section = &samples[start..end];
            (section.iter().map(|sample| sample * sample).sum::<f32>()
                / section.len().max(1) as f32)
                .sqrt()
        })
        .collect();
    let active_sections = section_levels.iter().filter(|rms| **rms >= 0.001).count();
    if active_sections + 1 < total_sections {
        return Err(MusicError::Invalid(format!(
            "render has dead sections ({active_sections}/{total_sections} active)"
        )));
    }
    let quietest = section_levels.iter().copied().fold(f32::INFINITY, f32::min);
    let loudest = section_levels.iter().copied().fold(0.0_f32, f32::max);
    let dynamic_range = if quietest > 0.0 {
        loudest / quietest
    } else {
        f32::INFINITY
    };

    let seam_delta = seam_discontinuity(samples);
    if seam_delta > 14.0 {
        return Err(MusicError::Invalid(format!(
            "loop seam is too abrupt (delta {seam_delta:.1}x)"
        )));
    }

    Ok(VerificationReport {
        duration_seconds,
        peak,
        rms,
        seam_delta,
        active_sections,
        total_sections,
        dynamic_range,
        periodicity: periodicity(samples),
    })
}

/// How far the wrap from the last frame to the first jumps, as a multiple of
/// the render's own typical sample-to-sample step.
///
/// The earlier measure averaged the difference between the first and last 2048
/// frames. That asks whether the opening and the ending sound alike, which is a
/// question about arrangement, not about looping: a piece that ends loud and
/// begins loud failed it while looping perfectly, and the only way to pass was
/// to begin and end quietly. A click is a discontinuity, so that is what is
/// measured — the step across the wrap, against the steps everywhere else.
fn seam_discontinuity(samples: &[f32]) -> f32 {
    let frames = samples.len() / 2;
    if frames < 2 {
        return 0.0;
    }
    let typical = (1..frames)
        .map(|frame| {
            (samples[frame * 2] - samples[(frame - 1) * 2]).abs()
                + (samples[frame * 2 + 1] - samples[(frame - 1) * 2 + 1]).abs()
        })
        .sum::<f32>()
        / (2 * (frames - 1)) as f32;
    if typical <= 1e-9 {
        return 0.0;
    }
    let last = (frames - 1) * 2;
    let wrap = (samples[0] - samples[last])
        .abs()
        .max((samples[1] - samples[last + 1]).abs());
    wrap / typical
}

/// The strongest normalised autocorrelation at a musical period.
///
/// Cheaper than a spectrum and enough to separate pitched material from a wash:
/// a note repeats its waveform, noise does not. Sampled at several points so a
/// single sustained chord cannot stand in for the whole render.
fn periodicity(samples: &[f32]) -> f32 {
    const WINDOW: usize = 2_048;
    // 60 Hz to 800 Hz, which spans the bass through the top of most leads.
    const SHORTEST_LAG: usize = 60;
    const LONGEST_LAG: usize = 800;

    let frames = samples.len() / 2;
    if frames < WINDOW * 2 {
        return 0.0;
    }
    let probes = 8;
    let mut best_per_probe = Vec::with_capacity(probes);
    for probe in 0..probes {
        let start = frames / (probes + 1) * (probe + 1) - WINDOW / 2;
        let mono: Vec<f32> = (0..WINDOW)
            .map(|offset| {
                let frame = start + offset;
                (samples[frame * 2] + samples[frame * 2 + 1]) * 0.5
            })
            .collect();
        let energy: f32 = mono.iter().map(|sample| sample * sample).sum();
        if energy <= 1e-9 {
            continue;
        }
        let mut best = 0.0_f32;
        for lag in SHORTEST_LAG..LONGEST_LAG.min(WINDOW / 2) {
            let correlation: f32 = (0..WINDOW - lag)
                .map(|index| mono[index] * mono[index + lag])
                .sum();
            best = best.max(correlation / energy);
        }
        best_per_probe.push(best.clamp(0.0, 1.0));
    }
    if best_per_probe.is_empty() {
        return 0.0;
    }
    best_per_probe.iter().sum::<f32>() / best_per_probe.len() as f32
}

pub(crate) fn scale_offset(mode: Mode, degree: u8) -> u8 {
    let scale = mode.scale();
    let octaves = degree / 7;
    scale[usize::from(degree % 7)] + octaves * 12
}

/// The court's default loop: i, flat VII, IV, v in Dorian.
///
/// The previous second chord was a major triad on the sixth degree. In Dorian
/// that degree carries a diminished triad, so a major one contributed two notes
/// from outside the mode on every second bar. It was inaudible while the engine
/// only sustained three drones; once melodic parts were snapped to chord tones
/// it became the loudest thing in the mix that did not belong to the key.
fn default_progression() -> Vec<Chord> {
    vec![
        Chord {
            root_degree: 0,
            quality: ChordQuality::Minor,
        },
        Chord {
            root_degree: 6,
            quality: ChordQuality::Major,
        },
        Chord {
            root_degree: 3,
            quality: ChordQuality::Major,
        },
        Chord {
            root_degree: 4,
            quality: ChordQuality::Minor,
        },
    ]
}

fn diatonic_quality(mode: Mode, degree: u8) -> ChordQuality {
    let degree = usize::from(degree % 7);
    let qualities = match mode {
        Mode::Major => [
            ChordQuality::Major,
            ChordQuality::Minor,
            ChordQuality::Minor,
            ChordQuality::Major,
            ChordQuality::Major,
            ChordQuality::Minor,
            ChordQuality::Diminished,
        ],
        Mode::Minor => [
            ChordQuality::Minor,
            ChordQuality::Diminished,
            ChordQuality::Major,
            ChordQuality::Minor,
            ChordQuality::Minor,
            ChordQuality::Major,
            ChordQuality::Major,
        ],
        Mode::Dorian => [
            ChordQuality::Minor,
            ChordQuality::Minor,
            ChordQuality::Major,
            ChordQuality::Major,
            ChordQuality::Minor,
            ChordQuality::Diminished,
            ChordQuality::Major,
        ],
        Mode::Mixolydian => [
            ChordQuality::Major,
            ChordQuality::Minor,
            ChordQuality::Diminished,
            ChordQuality::Major,
            ChordQuality::Minor,
            ChordQuality::Minor,
            ChordQuality::Major,
        ],
    };
    qualities[degree]
}

fn progression_for(mode: Mode, revision: usize) -> Vec<Chord> {
    let roots: [u8; 4] = match revision % 4 {
        0 => [0, 5, 3, 4],
        1 => [0, 3, 5, 4],
        2 => [0, 1, 6, 4],
        _ => [0, 6, 3, 4],
    };
    roots
        .into_iter()
        .map(|root_degree| Chord {
            root_degree,
            quality: diatonic_quality(mode, root_degree),
        })
        .collect()
}

pub(crate) fn chord_intervals(quality: ChordQuality) -> [u8; 3] {
    quality.intervals()
}

fn parse_chord(value: &str, tonic: u8) -> Option<Chord> {
    let value = value.trim();
    let mut chars = value.char_indices();
    let (_, first) = chars.next()?;
    if !matches!(first.to_ascii_uppercase(), 'A'..='G') {
        return None;
    }
    let mut root_end = first.len_utf8();
    if let Some((index, accidental)) = chars.next()
        && matches!(accidental, '#' | 'b' | '♯' | '♭')
    {
        root_end = index + accidental.len_utf8();
    }
    let root = pitch_class(&value[..root_end])?;
    let suffix = value[root_end..].to_ascii_lowercase();
    let quality = if suffix.starts_with("dim") || suffix.starts_with('°') {
        ChordQuality::Diminished
    } else if suffix.starts_with("sus2") {
        ChordQuality::Suspended2
    } else if suffix.starts_with("sus4") || suffix.starts_with("sus") {
        ChordQuality::Suspended4
    } else if suffix.starts_with('m') && !suffix.starts_with("maj") {
        ChordQuality::Minor
    } else {
        ChordQuality::Major
    };
    let semitone = (root + 12 - tonic) % 12;
    let scale = [0, 2, 3, 5, 7, 9, 10];
    let root_degree = scale
        .iter()
        .position(|candidate| *candidate == semitone)
        .map(|index| index as u8)
        .unwrap_or_else(|| {
            scale
                .iter()
                .enumerate()
                .min_by_key(|(_, candidate)| candidate.abs_diff(semitone))
                .map(|(index, _)| index as u8)
                .unwrap_or(0)
        });
    Some(Chord {
        root_degree,
        quality,
    })
}

fn pitch_class(note: &str) -> Option<u8> {
    let mut chars = note.trim().chars();
    let base = match chars.next()?.to_ascii_uppercase() {
        'C' => 0_i8,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let accidental = match chars.next() {
        Some('#' | '♯') => 1,
        Some('b' | '♭') => -1,
        _ => 0,
    };
    Some((base + accidental).rem_euclid(12) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The track carrying the melody, whichever voice the score gave it to.
    ///
    /// Tests used to look for `Instrument::Lead` by name. The default score now
    /// gives its melody to a flute, and a test that hardcodes one instrument
    /// breaks whenever the orchestration changes rather than when the behaviour
    /// does.
    fn melody_track(score: &Score) -> usize {
        score
            .tracks
            .iter()
            .position(|track| {
                matches!(
                    track.instrument(),
                    Instrument::Lead | Instrument::Flute | Instrument::Reed | Instrument::Bell
                )
            })
            .expect("a score should carry a melody")
    }

    /// A short score, so tests that only need audio do not render a minute.
    fn brief() -> Score {
        Score {
            bars: 8,
            ..default_score()
        }
    }

    #[test]
    fn default_score_validates_and_renders_stereo_audio() {
        let score = default_score();
        score.validate().unwrap();
        let audio = score.render().unwrap();
        assert!(!audio.is_empty());
        assert!(audio.len().is_multiple_of(2), "audio must be interleaved");
        let report = verify_audio(&audio).unwrap();
        assert!(report.duration_seconds >= 30.0);
        assert!(report.active_sections + 1 >= report.total_sections);
    }

    #[test]
    fn default_arrangement_has_every_masterwork_foundation() {
        let report = default_score().composition_report();
        assert_eq!(report.quality, 100, "missing craft: {:?}", report.guidance);
        assert_eq!(report.grade, "Masterwork");
        assert!(report.pulse && report.foundation && report.harmony && report.melody);
        assert!(report.cadence && report.breathing_room && report.palette && report.form);
    }

    #[test]
    fn mastering_restores_a_sparse_legacy_lineage() {
        let mut sketch = default_score();
        let cello = sketch
            .tracks
            .iter()
            .find(|track| track.instrument() == Instrument::Cello)
            .unwrap()
            .clone();
        let flute = sketch
            .tracks
            .iter()
            .find(|track| track.instrument() == Instrument::Flute)
            .unwrap()
            .clone();
        sketch.bars = 32;
        sketch.progression = vec![
            Chord {
                root_degree: 6,
                quality: ChordQuality::Major,
            },
            Chord {
                root_degree: 2,
                quality: ChordQuality::Suspended4,
            },
        ];
        sketch.tracks = vec![cello, flute];
        assert!(sketch.composition_report().quality < 75);

        let mastered = sketch.mastered().unwrap();
        let report = mastered.composition_report();
        assert_eq!(mastered.bars, 16);
        assert_eq!(report.quality, 100, "missing craft: {:?}", report.guidance);
        assert!(
            mastered
                .tracks
                .iter()
                .any(|track| track.instrument() == Instrument::Cello)
        );
        assert!(
            mastered
                .tracks
                .iter()
                .any(|track| track.instrument() == Instrument::Flute)
        );
    }

    #[test]
    fn mastering_replaces_only_duplicate_roles_when_all_seats_are_full() {
        let mut sketch = default_score();
        sketch.tracks = (0..8)
            .map(|index| {
                let mut track = performance_track(Instrument::Flute);
                track.name = format!("Flute {index}");
                track
            })
            .collect();
        let mastered = sketch.mastered().unwrap();
        let report = mastered.composition_report();
        assert_eq!(mastered.tracks.len(), 8);
        assert!(report.quality >= 90, "missing craft: {:?}", report.guidance);
    }

    #[test]
    fn evolution_cycle_names_one_development_axis_and_preserves_craft() {
        let mut score = default_score();
        let expected = [
            EvolutionKind::Pulse,
            EvolutionKind::Motif,
            EvolutionKind::Counterpoint,
            EvolutionKind::Texture,
            EvolutionKind::Harmony,
            EvolutionKind::Modulation,
        ];
        let anchored_tonic = score.tonic;
        let anchored_tempo = score.bpm;
        for (index, expected_kind) in expected.into_iter().enumerate() {
            score = score.evolved().unwrap();
            assert_eq!(score.evolution_kind(), expected_kind);
            if index < 5 {
                assert_eq!(score.tonic, anchored_tonic);
                assert_eq!(score.bpm, anchored_tempo);
            }
            let report = score.composition_report();
            assert!(
                report.quality >= 88,
                "{} degraded the arrangement: {:?}",
                expected_kind.label(),
                report.guidance
            );
        }
        assert_ne!(score.tonic, anchored_tonic);
    }

    #[test]
    fn native_evolution_is_audibly_distinct_and_stays_verified() {
        let mut score = brief();
        let mut previous = score.render().unwrap();
        // Key and tempo are anchors and deliberately hold for a run, so the
        // audible difference between consecutive variations comes from the
        // arrangement. What must never happen is a variation that renders the
        // same PCM as the one before it.
        for revision in 1..=3 {
            let evolved = score.evolved().unwrap();
            assert_eq!(evolved.evolution, revision);
            let audio = evolved.render().unwrap();
            let report = verify_audio(&audio).unwrap();
            assert!(report.duration_seconds >= 4.0);
            let comparison = previous.len().min(audio.len()).min(96_000);
            assert!(
                previous[..comparison]
                    .iter()
                    .zip(&audio[..comparison])
                    .any(|(before, after)| before.to_bits() != after.to_bits()),
                "variation {revision} repeated the previous PCM"
            );
            score = evolved;
            previous = audio;
        }
    }

    #[test]
    fn long_running_evolution_does_not_drift_tempo_or_mix_to_the_ceiling() {
        let original = brief();
        let original_gains = original
            .tracks
            .iter()
            .map(|track| track.gain)
            .collect::<Vec<_>>();
        let mut score = original.clone();
        let (mut slowest, mut fastest) = (score.bpm, score.bpm);
        // Sixty variations is roughly forty minutes of stream. Tempo must move
        // perceptibly across that span while holding steady within any short
        // stretch of it, and it must not march toward either limit.
        for _ in 0..60 {
            score = score.evolved().unwrap();
            slowest = slowest.min(score.bpm);
            fastest = fastest.max(score.bpm);
        }
        assert!(
            fastest - slowest >= 8,
            "tempo did not evolve perceptibly across forty minutes"
        );
        assert!(
            (i32::from(score.bpm) - i32::from(original.bpm)).abs() <= 8,
            "tempo accumulated long-run drift: {} from {}",
            score.bpm,
            original.bpm
        );
        for (track, original_gain) in score.tracks.iter().zip(original_gains) {
            assert!(
                (track.gain - original_gain).abs() < 0.000_1,
                "{} accumulated long-run gain drift",
                track.name
            );
        }
    }

    #[test]
    fn a_long_lineage_reaches_the_acoustic_voices() {
        // The live ledger showed a piece four thousand variations deep still
        // played by the six synths its first score named. New instruments that
        // only a fresh composition can reach are not instruments the court can
        // use, so evolution has to be able to re-orchestrate.
        let mut score = brief();
        let mut heard: std::collections::HashSet<Instrument> = score
            .tracks
            .iter()
            .map(|track| track.instrument())
            .collect();
        for _ in 0..40 {
            score = score.evolved().expect("a valid score evolves");
            heard.extend(score.tracks.iter().map(|track| track.instrument()));
        }

        for voice in [
            Instrument::Organ,
            Instrument::Piano,
            Instrument::Cello,
            Instrument::Harp,
            Instrument::Reed,
        ] {
            assert!(
                heard.contains(&voice),
                "forty variations should have reached {voice:?}, heard {heard:?}"
            );
        }
    }

    #[test]
    fn re_orchestration_keeps_a_voice_long_enough_to_be_heard() {
        // A part that changed instrument every variation would be a patch demo.
        let mut score = brief();
        let mut previous: Vec<Instrument> = score
            .tracks
            .iter()
            .map(|track| track.instrument())
            .collect();
        let mut changes = 0;
        for _ in 0..20 {
            score = score.evolved().expect("a valid score evolves");
            let now: Vec<Instrument> = score
                .tracks
                .iter()
                .map(|track| track.instrument())
                .collect();
            if now != previous {
                changes += 1;
            }
            previous = now;
        }
        assert!(
            (2..=6).contains(&changes),
            "twenty variations should re-voice a handful of times, got {changes}"
        );
    }

    #[test]
    fn a_re_voiced_track_is_renamed_to_match() {
        // The name is what the DAW shows and what `from_name` reads back.
        assert_eq!(
            Score::renamed_for("Chapel Organ", Instrument::Organ, Instrument::Reed),
            "Chapel Reed"
        );
        assert_eq!(
            Score::renamed_for("Keys", Instrument::Keys, Instrument::Organ),
            "Organ"
        );
        // No label to swap: the trailing word is the one that names the voice.
        assert_eq!(
            Score::renamed_for("Silver Whistle", Instrument::Flute, Instrument::Lead),
            "Silver Lead"
        );
    }

    #[test]
    fn re_orchestration_leaves_the_groove_alone() {
        // Percussion has no family: a kick that became a snare would be a
        // different groove, not a different colour.
        let mut score = brief();
        for _ in 0..40 {
            score = score.evolved().expect("a valid score evolves");
            for track in &score.tracks {
                let instrument = track.instrument();
                if instrument.is_percussive() {
                    assert!(
                        instrument.family().is_empty(),
                        "{instrument:?} should never be re-voiced"
                    );
                }
            }
        }
        assert!(
            score
                .tracks
                .iter()
                .any(|track| track.instrument() == Instrument::Kick),
            "the kick should survive forty re-orchestrations"
        );
    }

    #[test]
    fn patterns_repeat_against_the_bar_grid() {
        // The V1-era renderer divided the whole piece by the pattern length, so
        // an eight-step motif became eight notes each several seconds long.
        // A pattern has to loop, or the tempo means nothing.
        let score = brief();
        let plan = score.arrange();
        let lead = melody_track(&score);
        let written = score.tracks[lead]
            .steps
            .iter()
            .filter(|step| step.is_some())
            .count();
        let notes = plan
            .events
            .iter()
            .filter(|event| event.track == lead)
            .count();
        // The melody rests in the sections that mute it, so this is not a count
        // of bars times pattern. What it does prove is that the pattern played
        // more than once instead of being stretched across the whole piece.
        assert!(
            notes > written,
            "a melody of {written} written notes over eight bars must repeat, got {notes} notes"
        );

        let longest = plan
            .events
            .iter()
            .map(|event| event.gate_seconds)
            .fold(0.0_f32, f32::max);
        assert!(
            longest < 2.5,
            "no note should sustain for seconds, got {longest:.2}s"
        );
    }

    #[test]
    fn the_master_is_voiced_as_a_bed_not_a_foreground_track() {
        // Measured against 40 sampled V1 renders, which the operator preferred
        // and which this engine is voiced to match: V1 carries roughly 1.7% of
        // its energy between 2 and 6 kHz. V2 carried 20% and was tiring to
        // listen to for any length of time. This guards the correction.
        let audio = default_score().render().unwrap();
        let mono: Vec<f32> = audio.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect();

        // A one-pole pair isolates 2-6 kHz well enough to measure a share.
        let energy_between = |low: f32, high: f32| {
            let (mut above_low, mut above_high) = (0.0_f32, 0.0_f32);
            let (mut sum, mut previous) = (0.0_f32, 0.0_f32);
            let alpha = |hz: f32| {
                let rc = 1.0 / (std::f32::consts::TAU * hz);
                let dt = 1.0 / SAMPLE_RATE as f32;
                rc / (rc + dt)
            };
            let (a_low, a_high) = (alpha(low), alpha(high));
            for sample in &mono {
                above_low = a_low * (above_low + sample - previous);
                above_high = a_high * (above_high + sample - previous);
                previous = *sample;
                let band = above_low - above_high;
                sum += band * band;
            }
            (sum / mono.len() as f32).sqrt()
        };
        let total = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
        let share = energy_between(2_000.0, 6_000.0) / total.max(1e-9);
        assert!(
            share < 0.12,
            "2-6 kHz share {share:.3} is where listening fatigue lives; V1 sits near 0.02"
        );
    }

    #[test]
    fn the_render_is_pitched_material_rather_than_a_wash() {
        // The failure this guards against is not silence or clipping, both of
        // which were already checked. It is a render that is technically fine
        // and sounds like noise: too many overlapping voices, too much reverb,
        // and too much saturation, which is exactly what the first version of
        // this engine produced.
        let audio = default_score().render().unwrap();
        let report = verify_audio(&audio).unwrap();
        assert!(
            report.periodicity > 0.45,
            "the court should be playing notes, not a wash (periodicity {:.3})",
            report.periodicity
        );

        // White noise must fail the same measure, or the threshold is
        // meaningless. Measured directly: noise cannot reach `verify_audio`,
        // because its loop seam is rejected first.
        let mut state = 12_345_u32;
        let noise: Vec<f32> = (0..SAMPLE_RATE as usize * 2 * 8)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32) * 1.4 - 0.7
            })
            .collect();
        let noise_periodicity = periodicity(&noise);
        assert!(
            noise_periodicity < 0.25,
            "noise must not pass as music (periodicity {noise_periodicity:.3})"
        );
    }

    #[test]
    fn the_default_progression_stays_inside_its_mode() {
        // Melodic parts are snapped to chord tones, so a chord that leaves the
        // mode does not merely pass through: it drags the melody out with it.
        let score = default_score();
        let in_key: Vec<u8> = score
            .mode
            .scale()
            .iter()
            .map(|step| (score.tonic + step) % 12)
            .collect();
        for chord in &score.progression {
            let root = scale_offset(score.mode, chord.root_degree);
            for interval in chord_intervals(chord.quality) {
                let pitch = (score.tonic + root + interval) % 12;
                assert!(
                    in_key.contains(&pitch),
                    "chord on degree {} ({:?}) leaves the mode",
                    chord.root_degree,
                    chord.quality
                );
            }
        }
    }

    #[test]
    fn a_part_that_never_rests_sounds_every_note_it_was_written() {
        // Notes used to be discarded twice over: a section could rest a part,
        // and a density roll could then thin what was left, so better than a
        // third of everything written was never heard. Only a deliberate
        // section rest may silence a note now.
        let pad_and_keys = Score {
            title: "Continuous".to_owned(),
            bars: 16,
            tracks: vec![
                Track {
                    name: "Pad".to_owned(),
                    waveform: Waveform::Saw,
                    gain: 0.5,
                    octave: 3,
                    steps: vec![Some(0), Some(2), Some(4), Some(2)],
                    instrument: Some(Instrument::Pad),
                    gate: None,
                    pan: None,
                },
                Track {
                    name: "Keys".to_owned(),
                    waveform: Waveform::Triangle,
                    gain: 0.5,
                    octave: 4,
                    steps: vec![
                        Some(0),
                        None,
                        Some(4),
                        Some(2),
                        Some(0),
                        None,
                        Some(4),
                        Some(6),
                    ],
                    instrument: Some(Instrument::Keys),
                    gate: None,
                    pan: None,
                },
            ],
            ..default_score()
        };
        let plan = pad_and_keys.arrange();
        for (index, track) in pad_and_keys.tracks.iter().enumerate() {
            let written = usize::from(pad_and_keys.bars)
                * track.steps.iter().filter(|step| step.is_some()).count();
            let sounded = plan
                .events
                .iter()
                .filter(|event| event.track == index)
                .count();
            assert_eq!(
                sounded, written,
                "'{}' rests in no section, so all {written} notes must sound",
                track.name
            );
        }
    }

    #[test]
    fn a_note_on_a_beat_is_never_thinned_away() {
        // Groove thinning may only touch off-beats. A written downbeat that
        // vanishes reads to the operator as the engine dropping notes.
        let score = default_score();
        let plan = score.arrange();
        let bar_frames = 60.0 / f32::from(score.bpm) * 4.0 * SAMPLE_RATE as f32;
        for (index, track) in score.tracks.iter().enumerate() {
            let divisions = track.steps.len().clamp(1, 16);
            let sounded_on_beat = plan
                .events
                .iter()
                .filter(|event| event.track == index)
                .filter(|event| {
                    // Distance to the nearest grid point, measured both ways.
                    // A note nudged a fraction early sits just below the next
                    // whole step, not far from it: comparing the fraction alone
                    // reads 0.99 as off-beat when it is a hair ahead of it.
                    let position = event.start_frame as f32 % bar_frames / bar_frames;
                    let offset = (position * divisions as f32).fract();
                    offset.min(1.0 - offset) < 0.2
                })
                .count();
            assert!(
                sounded_on_beat > 0 || track.steps.iter().all(Option::is_none),
                "'{}' lost every on-beat note",
                track.name
            );
        }
    }

    #[test]
    fn the_arrangement_moves_between_sections() {
        // Movement is the point: a piece where every eighth is the same level
        // has no arrangement, however many notes it contains.
        let audio = default_score().render().unwrap();
        let report = verify_audio(&audio).unwrap();
        assert!(
            report.dynamic_range > 1.35,
            "sections must differ in level, got {:.2}",
            report.dynamic_range
        );
    }

    #[test]
    fn sections_mute_and_restore_parts() {
        use arrange::Section;
        assert!(!Section::Intro.plays(Instrument::Kick), "the intro is bare");
        assert!(Section::Main.plays(Instrument::Kick));
        assert!(
            !Section::Break.plays(Instrument::Kick),
            "the break drops the rhythm section"
        );
        assert!(Section::Finale.plays(Instrument::Kick));
    }

    #[test]
    fn the_loop_seam_carries_the_tail_into_the_opening() {
        // Reverb and release tails are folded back over the start, so a repeat
        // continues rather than restarting. The pad opens with a half-second
        // attack, so without the fold the first frames would be near silence;
        // the wrapped tail is the only thing that can put energy there.
        let audio = default_score().render().unwrap();
        let report = verify_audio(&audio).unwrap();
        assert!(report.seam_delta < 14.0, "seam {:.1}x", report.seam_delta);

        // The measure has to reject an actual discontinuity, or passing it
        // means nothing. A loud tone cut mid-cycle is the click being guarded
        // against.
        let frames = SAMPLE_RATE as usize * 8;
        let clicking: Vec<f32> = (0..frames)
            .flat_map(|frame| {
                // A whole number of cycles plus a quarter: the buffer starts at
                // zero and ends at the top of the wave, so the wrap is a
                // full-scale step rather than a continuation.
                let phase = frame as f32 / frames as f32 * 1_760.25;
                let value = (phase * std::f32::consts::TAU).sin() * 0.8;
                [value, value]
            })
            .collect();
        assert!(
            seam_discontinuity(&clicking) > 14.0,
            "a tone cut mid-cycle must be caught, got {:.1}x",
            seam_discontinuity(&clicking)
        );

        let opening = SAMPLE_RATE as usize / 10 * 2;
        let opening_rms =
            (audio[..opening].iter().map(|s| s * s).sum::<f32>() / opening as f32).sqrt();
        assert!(
            opening_rms > report.rms * 0.15,
            "the opening ({opening_rms:.4}) should carry the wrapped tail, \
             not start from silence against an overall {:.4}",
            report.rms
        );
    }

    #[test]
    fn rendering_is_deterministic() {
        // Keep and reject feedback is only evidence if the same score gives
        // back the same audio.
        let score = brief();
        assert_eq!(score.render().unwrap(), score.render().unwrap());
    }

    #[test]
    fn the_master_leaves_headroom_without_clipping() {
        let audio = brief().render().unwrap();
        let peak = audio.iter().map(|sample| sample.abs()).fold(0.0, f32::max);
        assert!(
            (peak - MASTER_PEAK).abs() < 0.02,
            "master should normalise to {MASTER_PEAK}, got {peak:.3}"
        );
    }

    #[test]
    fn track_roles_are_inferred_from_their_names() {
        assert_eq!(Instrument::from_name("Throne Bass"), Instrument::Bass);
        assert_eq!(Instrument::from_name("Court Kick"), Instrument::Kick);
        assert_eq!(Instrument::from_name("Glass Motif"), Instrument::Bell);
        assert_eq!(Instrument::from_name("Authored Motif"), Instrument::Lead);
        assert_eq!(Instrument::from_name("Velvet Pad"), Instrument::Pad);
        assert_eq!(Instrument::from_name("something else"), Instrument::Keys);
    }

    #[test]
    fn melodic_parts_are_pulled_onto_chord_tones() {
        // A written degree that disagrees with the current chord is moved to
        // the nearest chord tone rather than sounded against it.
        let score = brief();
        let plan = score.arrange();
        let lead = melody_track(&score);
        let progression_length = score.progression.len();
        assert!(progression_length > 1);

        for event in plan.events.iter().filter(|event| event.track == lead) {
            let chord_pitches: Vec<u8> = score
                .progression
                .iter()
                .flat_map(|chord| {
                    let root = scale_offset(score.mode, chord.root_degree);
                    chord_intervals(chord.quality)
                        .map(|interval| (root + interval + score.tonic) % 12)
                })
                .collect();
            let pitch_class = (event.midi as i16).rem_euclid(12) as u8;
            assert!(
                chord_pitches.contains(&pitch_class),
                "lead pitch class {pitch_class} is not in any chord of the progression"
            );
        }
    }

    #[test]
    fn a_recorded_verdict_can_be_read_back() {
        // The buttons wrote verdicts that nothing ever read, so the court could
        // not tell a kept piece from a refused one.
        let directory = std::env::temp_dir().join(format!(
            "teledra-feedback-{}-{}",
            std::process::id(),
            FEEDBACK_SEQUENCE.load(Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&directory);

        let mut kept = default_score();
        kept.title = "A Piece Worth Keeping".to_owned();
        let mut refused = default_score();
        refused.title = "A Piece Worth Forgetting".to_owned();

        record_feedback(&directory, &kept, FeedbackVerdict::Keep, "like").unwrap();
        record_feedback(&directory, &refused, FeedbackVerdict::Reject, "too busy").unwrap();

        let events = recent_feedback(&directory, 8);
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(events.iter().any(|event| event.score_title == kept.title
            && matches!(event.verdict, FeedbackVerdict::Keep)));
        assert!(events.iter().any(|event| event.score_title == refused.title
            && matches!(event.verdict, FeedbackVerdict::Reject)));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn reading_feedback_from_nowhere_is_not_an_error() {
        assert!(recent_feedback(Path::new("no-such-feedback-directory"), 4).is_empty());
    }

    /// Verbatim from the stream: a kick written as the positions it fires on.
    #[test]
    fn trigger_positions_are_read_as_a_rhythm_not_as_pitches() {
        let score = Score::from_json(
            r#"{"title":"Positions","bpm":92,"bars":16,"tonic":0,"mode":"minor","tracks":[
                {"name":"Kick","waveform":"sine","gain":1.0,"octave":2,
                 "steps":[0,8,16,24,32,40,48,56]}]}"#,
        )
        .expect("a rhythm written as positions is still a rhythm");
        let steps = &score.tracks[0].steps;
        assert_eq!(
            steps.len(),
            64,
            "eight positions across four bars of sixteenths"
        );
        for position in [0, 8, 16, 24, 32, 40, 48, 56] {
            assert_eq!(steps[position], Some(0), "position {position} should sound");
        }
        assert!(steps[1].is_none(), "the slots between are rests");
        // And it renders, which the raw positions never could.
        verify_audio(&score.render().unwrap()).expect("playable");
    }

    #[test]
    fn a_real_melody_is_left_alone() {
        // Degrees that rise but stay inside the scale are a melody, not
        // positions, and must not be rewritten.
        let steps = vec![Some(0), Some(2), Some(4), Some(6)];
        assert!(positions_to_degrees(&steps).is_none());
        // Nor is a pattern with rests, however large its numbers look.
        let with_rests = vec![Some(0), None, Some(30)];
        assert!(positions_to_degrees(&with_rests).is_none());
    }

    #[test]
    fn key_and_tempo_hold_across_a_run_of_variations() {
        // A variation is accepted roughly every forty seconds. When each one
        // also moved the tonic, the mode and the tempo, the live ledger showed
        // key or mode changing on 98% of transitions and tempo on 96% — the ear
        // never got a tonic to hold or a pulse to settle into, which is what
        // "chaotic" meant. The anchors now outlast several variations.
        let mut score = default_score();
        let (mut key_moves, mut tempo_moves) = (0, 0);
        for _ in 0..40 {
            let next = score.evolved().unwrap();
            if next.tonic != score.tonic || next.mode.scale() != score.mode.scale() {
                key_moves += 1;
            }
            if next.bpm != score.bpm {
                tempo_moves += 1;
            }
            // Something must still change, or the variation is pointless.
            assert!(
                next.evolution > score.evolution,
                "a variation that changes nothing is not a variation"
            );
            score = next;
        }
        assert!(
            key_moves <= 6,
            "the key moved {key_moves} times in 40 variations; it should anchor"
        );
        assert!(key_moves >= 2, "the key should still move sometimes");
        assert!(tempo_moves <= 6, "the tempo moved {tempo_moves} times");
    }

    #[test]
    fn a_key_change_lands_on_a_close_relation() {
        // When the key does move it moves by a fourth or a fifth, so the new
        // one sounds like somewhere the old one could have gone.
        let mut score = default_score();
        let mut moves = Vec::new();
        for _ in 0..40 {
            let next = score.evolved().unwrap();
            if next.tonic != score.tonic {
                let step = (next.tonic + 12 - score.tonic) % 12;
                moves.push(step);
            }
            score = next;
        }
        assert!(!moves.is_empty());
        for step in moves {
            assert!(
                step == 5 || step == 7,
                "a jump of {step} semitones is not a close relation"
            );
        }
    }

    #[test]
    fn dangerous_score_sizes_are_rejected() {
        let mut score = default_score();
        score.bars = 10_000;
        assert!(score.validate().is_err());
    }

    #[test]
    fn out_of_range_performance_fields_are_rejected() {
        let mut score = default_score();
        score.swing = Some(2.0);
        assert!(score.validate().is_err());

        let mut score = default_score();
        score.sidechain = Some(-1.0);
        assert!(score.validate().is_err());

        let mut score = default_score();
        score.tracks[0].pan = Some(4.0);
        assert!(score.validate().is_err());
    }

    #[test]
    fn silent_audio_never_reaches_playback() {
        let silence = vec![0.0; SAMPLE_RATE as usize * 2 * 5];
        assert!(verify_audio(&silence).is_err());
    }

    #[test]
    fn named_chord_progressions_enter_the_native_score() {
        let score = Score::from_json(
            r#"{"title":"Cadence","transport":{"bpm":90,"bars":8},"harmony":{"tonic":"D","mode":"dorian","progression":["Dm","Bb","F","Csus4"]},"motif":["D","F","A","C"]}"#,
        )
        .unwrap();
        assert_eq!(score.progression.len(), 4);
        assert!(matches!(score.progression[0].quality, ChordQuality::Minor));
        assert!(matches!(
            score.progression[3].quality,
            ChordQuality::Suspended4
        ));
        // A bare harmony-and-motif score still arrives with a rhythm section.
        assert!(
            score
                .tracks
                .iter()
                .any(|track| track.instrument() == Instrument::Kick)
        );
        verify_audio(&score.render().unwrap()).expect("the score must render playable audio");
    }

    #[test]
    fn v1_era_scores_without_the_new_fields_still_load() {
        let score = Score::from_json(
            r#"{"title":"Legacy","bpm":92,"bars":8,"tonic":2,"mode":"dorian","tracks":[{"name":"Throne Bass","waveform":"triangle","gain":0.7,"octave":2,"steps":[0,null,0,null,3,null,4,null]}]}"#,
        )
        .unwrap();
        assert_eq!(score.tracks.len(), 1);
        assert_eq!(score.tracks[0].instrument(), Instrument::Bass);
        verify_audio(&score.render().unwrap()).expect("the score must render playable audio");
    }
}
