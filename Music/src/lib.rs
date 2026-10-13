//! Court-owned score validation, synthesis, playback, and feedback.
//!
//! The score schema is the court's instrument. `arrange` decides what is played
//! and when, `synth` decides what each note sounds like, and `effects` decides
//! what the room around it sounds like. This module owns the schema, the mix,
//! and the boundary the rest of Teledra sees.

mod arrange;
mod composer;
mod effects;
mod instruments;
mod python_bridge;
mod synth;
mod theory;

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rodio::cpal::traits::HostTrait;
use rodio::{DeviceTrait, OutputStream, OutputStreamHandle, Sink};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use arrange::{NoteTrigger, NoteTriggerOutcome};
pub use composer::{compose_chamber_study, compose_layered_study};
pub use instruments::{Instrument, InstrumentInfo, InstrumentRole};
pub use python_bridge::run_python_note_server;
pub use theory::*;

pub const SAMPLE_RATE: u32 = 48_000;
pub const SCORE_SCHEMA: &str = include_str!("../score.schema.json");

pub fn composition_requested(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("court_score")
        || lower.contains("composer nudge")
        || lower
            .split(|c: char| !c.is_alphabetic())
            .any(|word| matches!(word, "compose" | "recompose"))
}
const MAX_AUDIO_SECONDS: usize = 10 * 60;
/// Maximum master peak. Quiet authored dynamics are never boosted to this level.
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
    /// Explicit musical intent. Legacy scores retain their ensemble defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<CompositionPlan>,
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
    Augmented,
    MajorSeventh,
    MinorSeventh,
    DominantSeventh,
    HalfDiminished,
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
    Lydian,
    Phrygian,
    HarmonicMinor,
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

/// Score-derived arrangement checks displayed in Organist Studio.
/// The percentage counts applicable checks; it does not rate listening quality.
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
    #[serde(default)]
    pub articulation: bool,
    #[serde(default)]
    pub separation: bool,
    #[serde(default)]
    pub register_fit: bool,
    #[serde(default)]
    pub independent_parts: bool,
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
    /// Written note dynamics aligned with steps; zero is an intentional rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocities: Option<Vec<f32>>,
    /// Written note lengths in beats, aligned with steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durations: Option<Vec<f32>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    Sine,
    Triangle,
    Square,
    Saw,
}

impl Track {
    /// Count written onsets that carry audible dynamics, before arrangement.
    pub fn active_note_count(&self) -> usize {
        self.steps
            .iter()
            .enumerate()
            .filter(|(index, note)| {
                note.is_some()
                    && self
                        .velocities
                        .as_ref()
                        .and_then(|values| values.get(*index))
                        .copied()
                        .unwrap_or(1.0)
                        > 0.001
            })
            .count()
    }
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
    /// A descriptive autocorrelation measure. Independent chord tones need
    /// not share a short period, so this is not a measure of artistic quality
    /// or a universal pitchedness threshold.
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
    clock: Arc<PlaybackClock>,
}

struct PlaybackClock {
    frames: AtomicU64,
    seek: AtomicU64,
    replacement: Mutex<Option<PlaybackReplacement>>,
    finished: AtomicBool,
}

struct PlaybackReplacement {
    samples: Vec<f32>,
    cursor: u64,
    looped: bool,
}

struct TransitionTail {
    samples: Vec<f32>,
    cursor: u64,
    looped: bool,
    consumed: usize,
}

const PLAYBACK_TRANSITION_FRAMES: usize = SAMPLE_RATE as usize / 25;

/// Counts the PCM frames consumed by the output, including completed loops.
/// A wall clock kept advancing during pause and let revisions cut a pass short.
struct PlaybackSource {
    samples: Vec<f32>,
    cursor: u64,
    looped: bool,
    clock: Arc<PlaybackClock>,
    transition: Option<TransitionTail>,
}

impl PlaybackSource {
    fn new(samples: Vec<f32>, looped: bool) -> Self {
        Self {
            samples,
            cursor: 0,
            looped,
            clock: Arc::new(PlaybackClock {
                frames: AtomicU64::new(0),
                seek: AtomicU64::new(u64::MAX),
                replacement: Mutex::new(None),
                finished: AtomicBool::new(false),
            }),
            transition: None,
        }
    }
}

impl Iterator for PlaybackSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.cursor.is_multiple_of(2) {
            let replacement = self.clock.replacement.lock().unwrap().take();
            if let Some(replacement) = replacement {
                self.transition = Some(TransitionTail {
                    samples: std::mem::replace(&mut self.samples, replacement.samples),
                    cursor: self.cursor,
                    looped: self.looped,
                    consumed: 0,
                });
                self.cursor = replacement.cursor;
                self.looped = replacement.looped;
                self.clock.finished.store(false, Ordering::Release);
                self.clock.frames.store(self.cursor / 2, Ordering::Relaxed);
            }
            let seek = self.clock.seek.swap(u64::MAX, Ordering::Relaxed);
            if seek != u64::MAX {
                self.cursor = seek.saturating_mul(2);
            }
        }
        if self.samples.is_empty() || (!self.looped && self.cursor >= self.samples.len() as u64) {
            // Close the race with a producer replacing a one-shot at its last
            // frame. Publication and final-source completion share this lock.
            let pending = self.clock.replacement.lock().unwrap();
            if pending.is_some() {
                drop(pending);
                return self.next();
            }
            self.clock.finished.store(true, Ordering::Release);
            return None;
        }
        let mut sample = self.samples[(self.cursor % self.samples.len() as u64) as usize];
        if let Some(tail) = &mut self.transition {
            let previous = if tail.samples.is_empty() {
                0.0
            } else if !tail.looped && tail.cursor >= tail.samples.len() as u64 {
                // At a one-shot's exact end, carry its last stereo frame down
                // through the overlap rather than jumping straight to zero.
                tail.samples[tail.samples.len().saturating_sub(2) + (tail.cursor % 2) as usize]
            } else {
                tail.samples[(tail.cursor % tail.samples.len() as u64) as usize]
            };
            let fraction = (tail.consumed / 2) as f32 / (PLAYBACK_TRANSITION_FRAMES - 1) as f32;
            let blend = fraction * fraction * (3.0 - 2.0 * fraction);
            // A convex blend preserves headroom even for correlated notes. The
            // incoming cursor advances from its first frame throughout the
            // overlap; no opening notes or attack samples are discarded.
            sample = previous * (1.0 - blend) + sample * blend;
            tail.cursor += 1;
            tail.consumed += 1;
            if tail.consumed == PLAYBACK_TRANSITION_FRAMES * 2 {
                self.transition = None;
            }
        }
        self.cursor += 1;
        if self.cursor.is_multiple_of(2) {
            self.clock.frames.store(self.cursor / 2, Ordering::Relaxed);
        }
        Some(sample)
    }
}

impl rodio::Source for PlaybackSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        (!self.looped).then(|| {
            std::time::Duration::from_secs_f64(
                self.samples.len() as f64 / (2.0 * f64::from(SAMPLE_RATE)),
            )
        })
    }
}

/// Audio that has passed the acoustic gate off the event-loop thread.
#[derive(Clone, Debug)]
pub struct VerifiedAudio {
    samples: Vec<f32>,
    report: VerificationReport,
}

impl VerifiedAudio {
    pub fn new(samples: Vec<f32>) -> Result<Self, MusicError> {
        let report = verify_audio(&samples)?;
        Ok(Self { samples, report })
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn report(&self) -> &VerificationReport {
        &self.report
    }

    pub fn write_wav(&self, path: &Path) -> Result<(), MusicError> {
        write_pcm_wav(path, &self.samples)
    }
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
        let player = Self::prepare_on(VerifiedAudio::new(samples)?, looped, output_device)?;
        player.resume();
        Ok(player)
    }

    /// Opens a paused replacement; the caller sets transport before starting it.
    pub fn prepare_on(
        audio: VerifiedAudio,
        looped: bool,
        output_device: Option<&str>,
    ) -> Result<Self, MusicError> {
        let stream = output_device
            .map(open_named_output)
            .unwrap_or_else(OutputStream::try_default);
        let (stream, handle) = stream.map_err(|error| MusicError::Output(error.to_string()))?;
        let sink = Sink::try_new(&handle).map_err(|error| MusicError::Output(error.to_string()))?;
        sink.pause();
        let source = PlaybackSource::new(audio.samples, looped);
        let clock = Arc::clone(&source.clock);
        sink.append(source);
        Ok(Self {
            _stream: stream,
            _handle: handle,
            sink,
            clock,
        })
    }

    /// Total consumed playback time across loops, frozen while playback pauses.
    pub fn position_seconds(&self) -> f64 {
        self.clock.frames.load(Ordering::Relaxed) as f64 / f64::from(SAMPLE_RATE)
    }

    /// Replace PCM on the existing output stream with a 40 ms smooth overlap.
    /// Pause and mute belong to the existing sink and remain unchanged. A new
    /// piece starts at zero; a mixer revision can preserve its current position.
    pub fn replace(
        &mut self,
        audio: VerifiedAudio,
        looped: bool,
        position_seconds: f64,
    ) -> Result<(), MusicError> {
        replace_playback(&self.sink, &self.clock, audio, looped, position_seconds)
    }

    /// Align a paused, prepared mixer replacement with the existing transport.
    pub fn seek_seconds(&self, seconds: f64) -> Result<(), MusicError> {
        let frames = (seconds * f64::from(SAMPLE_RATE)).round();
        if !seconds.is_finite() || seconds < 0.0 || frames >= (u64::MAX / 2) as f64 {
            return Err(MusicError::Invalid(
                "playback position must be finite, nonnegative and bounded".to_owned(),
            ));
        }
        self.clock.seek.store(frames as u64, Ordering::Relaxed);
        self.clock.frames.store(frames as u64, Ordering::Relaxed);
        Ok(())
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

    pub fn set_muted(&self, muted: bool) {
        self.sink.set_volume(if muted { 0.0 } else { 1.0 });
    }

    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }
}

fn replace_playback(
    sink: &Sink,
    clock: &Arc<PlaybackClock>,
    audio: VerifiedAudio,
    looped: bool,
    position_seconds: f64,
) -> Result<(), MusicError> {
    let frames = (position_seconds * f64::from(SAMPLE_RATE)).round();
    if !position_seconds.is_finite() || position_seconds < 0.0 || frames >= (u64::MAX / 2) as f64 {
        return Err(MusicError::Invalid(
            "replacement position must be finite, nonnegative and bounded".to_owned(),
        ));
    }
    clock.seek.store(u64::MAX, Ordering::Relaxed);
    let mut incoming = Some(PlaybackReplacement {
        samples: audio.samples,
        cursor: frames as u64 * 2,
        looped,
    });
    let attach_source = {
        let mut pending = clock.replacement.lock().unwrap();
        if clock.finished.load(Ordering::Acquire) || sink.empty() {
            pending.take();
            clock.finished.store(false, Ordering::Release);
            true
        } else {
            *pending = incoming.take();
            false
        }
    };
    if attach_source {
        // A finished one-shot source has left the sink. Reuse its output
        // stream and transport controls while attaching a fresh source.
        let incoming = incoming.unwrap();
        let mut source = PlaybackSource::new(incoming.samples, incoming.looped);
        source.clock = Arc::clone(clock);
        source.cursor = incoming.cursor;
        clock.frames.store(frames as u64, Ordering::Relaxed);
        sink.append(source);
    }
    Ok(())
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

/// Atomic authoring events keep pitch, duration and velocity together. This
/// conversion changes representation only; the persisted renderer schema stays
/// backward compatible with existing steps and expression arrays.
fn expand_authored_notes(value: &mut serde_json::Value) -> Result<(), MusicError> {
    let Some(tracks) = value
        .get_mut("tracks")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(());
    };
    for (index, track) in tracks.iter_mut().enumerate() {
        let Some(track) = track.as_object_mut() else {
            continue;
        };
        let Some(notes) = track.get("notes") else {
            continue;
        };
        if ["steps", "velocities", "durations"]
            .iter()
            .any(|field| track.contains_key(*field))
        {
            return Err(MusicError::Invalid(format!(
                "track {index} must use either atomic notes or steps/expression arrays, not both"
            )));
        }
        let notes = notes.as_array().ok_or_else(|| {
            MusicError::Invalid(format!(
                "track {index} notes must be an array of note objects"
            ))
        })?;
        if notes.is_empty() || notes.len() > 64 {
            return Err(MusicError::Invalid(format!(
                "track {index} needs 1..64 authored note positions"
            )));
        }
        let mut steps = Vec::with_capacity(notes.len());
        let mut velocities = Vec::with_capacity(notes.len());
        let mut durations = Vec::with_capacity(notes.len());
        for (position, note) in notes.iter().enumerate() {
            let valid = note.as_object().is_some_and(|object| object.len() == 3)
                && note.get("degree").is_some_and(|degree| {
                    degree.is_null() || degree.as_u64().is_some_and(|degree| degree <= 20)
                })
                && note
                    .get("velocity")
                    .and_then(serde_json::Value::as_f64)
                    .is_some_and(|value| (0.0..=1.0).contains(&value))
                && note
                    .get("duration")
                    .and_then(serde_json::Value::as_f64)
                    .is_some_and(|value| (0.125..=8.0).contains(&value));
            if !valid {
                return Err(MusicError::Invalid(format!(
                    "track {index} notes[{position}] needs exactly degree (null or 0..20), velocity (0..1), and duration (0.125..8 beats)"
                )));
            }
            steps.push(note["degree"].clone());
            velocities.push(note["velocity"].clone());
            durations.push(note["duration"].clone());
        }
        track.remove("notes");
        track.insert("steps".to_owned(), steps.into());
        track.insert("velocities".to_owned(), velocities.into());
        track.insert("durations".to_owned(), durations.into());
    }
    Ok(())
}

impl Score {
    pub fn from_json(raw: &str) -> Result<Self, MusicError> {
        let mut value: serde_json::Value = serde_json::from_str(raw)?;
        let atomic_authoring = value
            .get("tracks")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tracks| tracks.iter().any(|track| track.get("notes").is_some()));
        expand_authored_notes(&mut value)?;
        if let Ok(mut score) = serde_json::from_value::<Self>(value.clone()) {
            if !atomic_authoring {
                score.normalise_step_positions();
            }
            score.validate()?;
            return Ok(score);
        }
        // A malformed native score must not silently become an unrelated
        // default arrangement. Only the recognised legacy music shape adapts.
        if value.get("tracks").is_some() {
            let score = serde_json::from_value::<Self>(value)?;
            score.validate()?;
            return Ok(score);
        }
        if !value
            .get("harmony")
            .is_some_and(serde_json::Value::is_object)
            || !(value.get("motif").is_some_and(serde_json::Value::is_array)
                || value
                    .get("transport")
                    .is_some_and(serde_json::Value::is_object))
        {
            return Err(MusicError::Invalid(
                "expected a complete native score or a legacy harmony with transport/motif"
                    .to_owned(),
            ));
        }
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
            velocities: None,
            durations: None,
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
            velocities: None,
            durations: None,
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
            velocities: None,
            durations: None,
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
            velocities: None,
            durations: None,
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
            composition: None,
        };
        score.validate()?;
        Ok(score)
    }

    /// Rewrites any track that listed trigger positions instead of degrees.
    ///
    /// Refusing them would be the tidier rule, but the intent is unmistakable
    /// and the court has no way to learn the distinction from an error message.
    fn normalise_step_positions(&mut self) {
        // Explicitly planned scores use degrees exactly as authored, including
        // ascending wide intervals; do not reinterpret them as drum positions.
        if self.composition.is_some() {
            return;
        }
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
        if let Some(plan) = &self.composition
            && (!matches!(plan.phrase_bars, 2 | 4 | 8)
                || !matches!(plan.chord_bars, 1 | 2 | 4)
                || !matches!(plan.steps_per_bar, 4 | 8 | 16)
                || !plan.development.is_finite()
                || !(0.0..=1.0).contains(&plan.development))
        {
            return Err(MusicError::Invalid(
                "composition needs phrase_bars 2/4/8, chord_bars 1/2/4, steps_per_bar 4/8/16, and development 0..1"
                    .to_owned(),
            ));
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
            for (name, values, low, high) in [
                ("velocities", &track.velocities, 0.0, 1.0),
                ("durations", &track.durations, 0.125, 8.0),
            ] {
                if let Some(values) = values
                    && (values.len() != track.steps.len()
                        || values
                            .iter()
                            .any(|value| !value.is_finite() || !(low..=high).contains(value)))
                {
                    return Err(MusicError::Invalid(format!(
                        "track '{}' {name} must match steps length and contain finite values {low}..{high}",
                        track.name
                    )));
                }
            }
            if track.name.trim().is_empty() || track.name.chars().count() > 120 {
                return Err(MusicError::Invalid(
                    "track names must contain 1..=120 characters".to_owned(),
                ));
            }
            // Full native arrangements can write every bar explicitly. The
            // model's compact note schema remains separately bounded.
            if track.steps.is_empty() || track.steps.len() > 1024 {
                return Err(MusicError::Invalid(format!(
                    "track '{}' needs 1..=1024 steps",
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
        // V1 refused any composition under 120 seconds and canonicalised to 64
        // bars; this line used to cap everything at sixteen, which at 96 BPM is
        // forty seconds of music — a jingle looped and then rewritten every
        // forty-eight. A piece the ear can settle into needs two minutes at
        // least, and the schema allows sixty-four bars. The floor is set by
        // tempo so a slow piece is not padded and a fast one is not cut short.
        let planned = next.composition.is_some();
        if !planned {
            let two_minutes = ((120.0 * f32::from(next.bpm) / 240.0).ceil() as u16).div_ceil(4) * 4;
            next.bars = next.bars.max(two_minutes).clamp(32, 64);
        }
        let cadence = next
            .progression
            .first()
            .zip(next.progression.last())
            .is_some_and(|(first, last)| {
                first.root_degree % 7 == 0 && matches!(last.root_degree % 7, 3 | 4 | 6)
            });
        if !planned && (!(3..=8).contains(&next.progression.len()) || !cadence) {
            next.progression = progression_for(next.mode, next.evolution as usize);
        }
        // A valid root with the wrong quality is still out of key. Model
        // scores frequently label a Dorian tonic as major; keep the roots and
        // repair only qualities whose triad contains notes outside the mode.
        let scale = next.mode.scale();
        let chromatic = next.composition_plan().harmony == HarmonicPolicy::Chromatic;
        for chord in &mut next.progression {
            let root = scale[usize::from(chord.root_degree % 7)];
            if !chromatic
                && chord
                    .quality
                    .intervals()
                    .iter()
                    .any(|interval| !scale.contains(&((root + interval) % 12)))
            {
                chord.quality = diatonic_replacement(next.mode, chord.root_degree, chord.quality);
            }
        }

        // An explicit plan can be a solo, duet, drone, or short experiment.
        // Musical diagnostics guide the composer; they are not an instruction
        // to overwrite its ensemble, cadence, written rests, or form.
        if planned {
            next.validate()?;
            return Ok(next);
        }

        // Muting a seat is an operator mix decision, not a missing instrument.
        // Assess the authored structure for repairs, then report audible craft.
        let mut structure = next.clone();
        for track in &mut structure.tracks {
            if track.gain == 0.0 {
                track.gain = 0.5;
            }
        }
        let report = structure.composition_report();
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

        if !report.breathing_room
            && let Some(track) = next
                .tracks
                .iter_mut()
                .find(|track| track.instrument().role() == InstrumentRole::Melody)
        {
            for (index, step) in track.steps.iter_mut().enumerate() {
                if index % 4 == 3 {
                    *step = None;
                }
            }
        }

        for voice in [Instrument::Harp, Instrument::Hat, Instrument::Piano] {
            if next
                .tracks
                .iter()
                .map(Track::instrument)
                .collect::<std::collections::HashSet<_>>()
                .len()
                >= 5
            {
                break;
            }
            if !next.tracks.iter().any(|track| track.instrument() == voice) {
                insert_performance_voice(&mut next, performance_track(voice));
            }
        }

        next.validate()?;
        let report = next.composition_report();
        if report.quality < 75 && next.tracks.iter().all(|track| track.gain > 0.0) {
            return Err(MusicError::Invalid(format!(
                "arrangement mastering reached only {}/100: {}",
                report.quality,
                report.guidance.join("; ")
            )));
        }
        Ok(next)
    }

    /// Reports notation checks; the percentage does not rate listening quality.
    pub fn composition_report(&self) -> CompositionReport {
        let planned = self.composition.is_some();
        let intent = self.composition_plan();
        let audible: Vec<&Track> = self
            .tracks
            .iter()
            .filter(|track| track.gain > 0.001 && track.active_note_count() > 0)
            .collect();
        let instruments = audible
            .iter()
            .map(|track| track.instrument())
            .collect::<Vec<_>>();
        let pulse = (instruments
            .iter()
            .any(|instrument| instrument.is_percussive())
            && audible
                .iter()
                .any(|track| track.instrument().is_percussive() && track.active_note_count() >= 2))
            || (planned && audible.iter().any(|track| track.active_note_count() >= 2));
        let foundation = instruments
            .iter()
            .any(|instrument| instrument.role() == InstrumentRole::Bass);
        let harmony = !self.progression.is_empty()
            && instruments
                .iter()
                .any(|instrument| instrument.can_accompany_harmony());
        let melodic_tracks = audible
            .iter()
            .copied()
            .filter(|track| {
                track.instrument().role() == InstrumentRole::Melody
                    || track.instrument() == Instrument::Bell
                    || (planned
                        && !track.instrument().is_percussive()
                        && !track.instrument().is_chordal())
            })
            .collect::<Vec<_>>();
        let melody = melodic_tracks.iter().any(|track| {
            let positions = sounding_positions(track);
            let active = positions.len();
            let pitches = positions
                .into_iter()
                .map(|index| track.steps[index].unwrap())
                .collect::<std::collections::HashSet<_>>();
            active >= 3 && pitches.len() >= 3
        });
        let loop_cadence = self
            .progression
            .first()
            .zip(self.progression.last())
            .is_some_and(|(first, last)| {
                first.root_degree % 7 == 0 && matches!(last.root_degree % 7, 3 | 4 | 6)
            });
        // A written piece can arrive home before the loop boundary. Requiring
        // its last chord to remain dominant mislabels a completed V-I phrase
        // as unfinished. A tonic ending alone does not establish that motion.
        let closing_cadence = self.progression.windows(2).last().is_some_and(|pair| {
            matches!(pair[0].root_degree % 7, 3 | 4 | 6) && pair[1].root_degree % 7 == 0
        });
        let cadence = loop_cadence || closing_cadence;
        let breathing_room = melodic_tracks
            .iter()
            .any(|track| self.longest_written_gap(track) >= 0.25);
        let palette = !instruments.is_empty();
        let form = if planned {
            let authored_contrast = audible.iter().any(|track| {
                let phrase_cells = usize::from(intent.phrase_bars) * self.grid_divisions(track);
                let mut phrases = track.steps.chunks_exact(phrase_cells);
                phrases
                    .next()
                    .is_some_and(|first| phrases.any(|answer| answer != first))
            });
            self.bars >= u16::from(intent.phrase_bars) * 2
                && !audible.is_empty()
                && (intent.development > 0.0 || authored_contrast)
        } else {
            self.bars >= 8 && (3..=8).contains(&self.progression.len()) && self.tracks.len() >= 5
        };

        let articulation = audible.iter().all(|track| {
            minimum_written_attack_spacing(self, track)
                .is_none_or(|spacing| spacing + 0.001 >= track.instrument().attack_spacing())
        });
        // Two busy parts with almost the same written centre and pan compete
        // for the same register. This is a score diagnostic, not a taste score.
        let centre = |track: &Track| {
            let notes: Vec<f32> = sounding_positions(track)
                .into_iter()
                .map(|index| {
                    let degree = track.steps[index].unwrap();
                    let mut midi = 12.0 * (f32::from(track.octave) + 1.0)
                        + f32::from(self.tonic)
                        + f32::from(scale_offset(self.mode, degree));
                    let (low, high) = track.instrument().natural_range();
                    while midi > f32::from(high) {
                        midi -= 12.0;
                    }
                    while midi < f32::from(low) {
                        midi += 12.0;
                    }
                    midi
                })
                .collect();
            notes.iter().sum::<f32>() / notes.len().max(1) as f32
        };
        let separation = audible.iter().enumerate().all(|(i, left)| {
            audible.iter().skip(i + 1).all(|right| {
                left.instrument().is_percussive()
                    || right.instrument().is_percussive()
                    || (!same_written_line(left, right)
                        && ((centre(left) - centre(right)).abs() >= 5.0
                            || (left.pan.unwrap_or_else(|| default_pan(left.instrument()))
                                - right.pan.unwrap_or_else(|| default_pan(right.instrument())))
                            .abs()
                                >= 0.5
                            || left.steps.iter().flatten().count() * 4 < left.steps.len()
                            || right.steps.iter().flatten().count() * 4 < right.steps.len()))
            })
        });
        let register_fit = audible
            .iter()
            .all(|track| track.instrument().is_percussive() || self.written_register(track).2 == 0);
        let independent_parts = audible.iter().enumerate().all(|(index, left)| {
            audible.iter().skip(index + 1).all(|right| {
                left.instrument().is_percussive()
                    || right.instrument().is_percussive()
                    || !same_written_line(left, right)
            })
        });
        let checks = [
            pulse,
            foundation,
            harmony,
            melody,
            cadence,
            breathing_room,
            palette,
            form,
            articulation,
            separation,
            register_fit,
            independent_parts,
        ];
        // A chamber study can imply harmony without a separate bass seat;
        // an ambient study need not carry a drum pulse or a foreground tune.
        let applicable = if planned {
            [
                intent.texture != Texture::Ambient,
                intent.texture == Texture::Ensemble,
                intent.texture == Texture::Ensemble,
                intent.texture != Texture::Ambient,
                intent.texture == Texture::Ensemble,
                intent.texture != Texture::Ambient,
                true,
                true,
                true,
                true,
                true,
                true,
            ]
        } else {
            [true; 12]
        };
        let passed = checks
            .iter()
            .zip(applicable)
            .filter(|(ok, applies)| **ok && *applies)
            .count();
        let assessed = applicable.iter().filter(|applies| **applies).count();
        let quality = (passed * 100 / assessed.max(1)) as u8;
        let grade = if passed == assessed {
            "Structural checks only"
        } else {
            "Needs score review"
        }
        .to_owned();
        let mut guidance = self.orchestration_concerns();
        for ((ready, message), applies) in [
            (pulse, "establish a deliberate rhythmic pulse"),
            (foundation, "give the harmony a bass foundation"),
            (harmony, "add a chordal middle voice"),
            (melody, "write a distinct, singable melodic line"),
            (cadence, "make the loop cadence back toward its tonic"),
            (breathing_room, "leave a written release gap for the melody; null slots under held notes are not silence"),
            (palette, "provide at least one audible instrument"),
            (form, "a declared form and sufficient bars alone do not establish contrasting phrases; retain a stable loop when that is intended"),
            (
                articulation,
                "space sustained attacks; the arranger limits unplayable density",
            ),
            (
                separation,
                "separate competing parts by register, pan, or rests",
            ),
            (register_fit, "keep the written melody within each patch's usable register"),
            (independent_parts, "review exact same-register doubling; it is not evidence of independent counterpoint"),
        ]
        .into_iter()
        .zip(applicable)
        {
            if applies && !ready {
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
            articulation,
            separation,
            register_fit,
            independent_parts,
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
        let planned = self.composition.is_some();

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
            next.mode = if planned {
                self.mode
            } else {
                match self.mode {
                    Mode::Major => Mode::Dorian,
                    Mode::Dorian => Mode::Mixolydian,
                    Mode::Mixolydian => Mode::Minor,
                    Mode::Minor => Mode::Major,
                    Mode::Lydian => Mode::Major,
                    Mode::Phrygian => Mode::Minor,
                    Mode::HarmonicMinor => Mode::Minor,
                }
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
            if !planned {
                next.bpm = tempo.clamp(40, 220) as u16;
            }
        }

        if kind == EvolutionKind::Harmony {
            if planned {
                // Extend one internal harmony without changing the authored
                // roots, harmonic rhythm, or cadence of the experiment.
                let length = next.progression.len();
                if length > 0 {
                    let position = if length > 2 {
                        1 + revision % (length - 2)
                    } else {
                        0
                    };
                    let chromatic = next.composition_plan().harmony == HarmonicPolicy::Chromatic;
                    let chord = &mut next.progression[position];
                    let expanded = expanded_quality(chord.quality);
                    chord.quality = if chromatic {
                        expanded
                    } else {
                        diatonic_replacement(next.mode, chord.root_degree, expanded)
                    };
                }
            } else if next.progression.len() < 3 {
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
        } else if !planned && kind == EvolutionKind::Modulation {
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
        // An explicitly composed ensemble is part of the piece. Rotating a
        // piano line through every colour preset turned it into high chimes
        // regardless of the phrase or the composer's chosen instrumentation.
        // New ensembles belong to a deliberate new composition. Legacy scores
        // without a plan retain their existing orchestration development.
        let revoice = !planned && kind == EvolutionKind::Texture;

        // Decide every new voice before touching any track, so a voice already
        // taken by an earlier part is visible when a later one is chosen. The
        // index offset alone let two parts land on the same instrument — a
        // live lineage carried "Reed, Reed" and a walk produced "Pad … Pad" —
        // and two identical timbres in one register mask each other, which is
        // the exact fault the profile's "one thing at a time" rule exists to
        // prevent. Percussion is never re-voiced and never counted.
        let mut taken: Vec<Instrument> = next
            .tracks
            .iter()
            .map(Track::instrument)
            .filter(|instrument| instrument.is_percussive())
            .collect();
        let mut chosen: Vec<Option<Instrument>> = Vec::with_capacity(next.tracks.len());
        let mut role_seats = [0_usize; 5];
        for track in &next.tracks {
            let instrument = track.instrument();
            let family = instrument.family();
            if !revoice || family.is_empty() {
                if !instrument.is_percussive() {
                    taken.push(instrument);
                }
                chosen.push(None);
                continue;
            }
            // Advance once through every voice in this role. Adding an
            // increasing stride to the previous seat can cycle through only
            // part of a larger family and leave whole instruments unreachable.
            // Neighbouring seats stay distinct without changing the role of
            // their written material or turning a melody into chords.
            let role = arrangement_role(instrument);
            let seat = role_seats[role];
            role_seats[role] += 1;
            let step = revision / HARMONIC_ANCHOR_VARIATIONS;
            let candidate = (0..family.len())
                .map(|offset| family[(seat + step + offset) % family.len()])
                .find(|voice| !taken.contains(voice));
            let voice = candidate.unwrap_or(instrument);
            taken.push(voice);
            chosen.push((voice != instrument).then_some(voice));
        }

        for (index, track) in next.tracks.iter_mut().enumerate() {
            let mut instrument = track
                .instrument
                .unwrap_or_else(|| Instrument::from_name(&track.name));

            if let Some(next_voice) = chosen[index] {
                track.name = Self::renamed_for(&track.name, instrument, next_voice);
                instrument = next_voice;
                track.instrument = Some(next_voice);
            }

            let len = track.steps.len();
            if len == 0 {
                continue;
            }
            let target = (revision * 3 + index * 5) % len;
            let top_line = instrument.role() == InstrumentRole::Melody;
            let counter_line = matches!(
                instrument.role(),
                InstrumentRole::Harmony | InstrumentRole::Colour
            );

            // Development, not mutation.
            //
            // These arms used to rotate a part's steps and shift its degrees
            // modulo seven: one Motif variation rewrote 56% of the flute's
            // notes and moved its downbeat, the next rewrote 88% of the harp,
            // and that happened every forty-eight seconds. A listener never got
            // to learn the tune before it was scrambled. V1 never touched the
            // notes at all; it held a piece for five minutes. What a variation
            // may do now is what an arranger does between repeats: change the
            // register, the length of the notes, the density of a rhythm, which
            // chord comes next, who plays — never which pitches the tune is made
            // of, and never where the bar starts.
            match kind {
                EvolutionKind::Pulse if instrument == Instrument::Kick => {
                    // Develop accents, leaving the operator's gain and mute
                    // untouched. Rewriting gain previously revived muted kicks.
                    let accent =
                        [0.88_f32, 0.72, 0.94, 0.80][(revision / HARMONIC_ANCHOR_VARIATIONS) % 4];
                    let values = track.velocities.get_or_insert_with(|| vec![1.0; len]);
                    for (step, velocity) in values.iter_mut().enumerate() {
                        if track.steps[step].is_some() && *velocity > 0.0 {
                            *velocity = if step == 0 {
                                accent
                            } else {
                                (accent * 0.82).max(0.1)
                            };
                        }
                    }
                }
                EvolutionKind::Pulse if instrument.is_percussive() => {
                    // Add or remove one off-beat hit. An odd sixteenth is never
                    // a beat, so the pulse the listener counts is untouched.
                    let slot = (target | 1) % len;
                    let active = track.steps.iter().filter(|step| step.is_some()).count();
                    track.steps[slot] = match track.steps[slot] {
                        Some(_) if active > 2 => None,
                        Some(value) => Some(value),
                        None => Some(0),
                    };
                }
                EvolutionKind::Motif if top_line => {
                    if planned {
                        if self.composition_plan().development > 0.0 {
                            track.steps = theory::develop_expressed_motif(
                                track,
                                (revision / HARMONIC_ANCHOR_VARIATIONS) as u64,
                                false,
                            );
                        }
                        continue;
                    }
                    // The tune stays; it moves to a new register or breathes
                    // differently. Both are audible and neither is a wrong note.
                    let (low, high) = instrument.natural_range();
                    let low_octave = i8::try_from(low / 12).unwrap_or(2).saturating_sub(1);
                    let high_octave = i8::try_from(high / 12).unwrap_or(7).saturating_sub(1);
                    if (revision + index).is_multiple_of(2) {
                        let step = if track.octave >= high_octave { -1 } else { 1 };
                        track.octave = (track.octave + step).clamp(low_octave, high_octave);
                    } else {
                        let gates = [0.6_f32, 1.0, 1.4, 0.8];
                        track.gate = Some(gates[(revision + index) % gates.len()]);
                    }
                }
                EvolutionKind::Counterpoint if counter_line => {
                    if planned {
                        if self.composition_plan().development > 0.0 {
                            track.steps = theory::develop_expressed_motif(
                                track,
                                (revision / HARMONIC_ANCHOR_VARIATIONS) as u64,
                                true,
                            );
                        }
                        continue;
                    }
                    // Thin or thicken the rhythm of the middle voices. A rest
                    // that becomes a note repeats its neighbour, so the line
                    // gains motion without gaining a pitch it never had.
                    let thin = (revision / HARMONIC_ANCHOR_VARIATIONS + index).is_multiple_of(2);
                    let written: Vec<Option<u8>> = track.steps.clone();
                    for (position, step) in track.steps.iter_mut().enumerate() {
                        if position == 0 || position % 2 == 0 {
                            continue;
                        }
                        if thin {
                            *step = None;
                        } else if step.is_none() {
                            *step = written[position - 1]
                                .or(written.get(position + 1).copied().flatten());
                        }
                    }
                }
                _ => {}
            }
        }
        if !planned && kind == EvolutionKind::Harmony && next.progression.len() > 2 {
            // A reharmonisation the arranger will voice-lead: the same chords
            // in a new order, ending where a cadence can land. The bass and
            // middle voices used to be transposed a degree instead, which put
            // the whole foundation on non-chord tones.
            let last = next.progression.len() - 1;
            // The first tonic and final cadential chord are formal anchors.
            // Rotating the tonic into the middle undid the opening established
            // above and made an otherwise bounded experiment lose its home.
            next.progression[1..last].rotate_left(1);
        }
        if kind == EvolutionKind::Pulse {
            next.swing = Some([0.10, 0.16, 0.22, 0.14][revision % 4]);
            next.sidechain = Some([0.28, 0.36, 0.44, 0.32][revision % 4]);
        }
        next.seed = Some(
            self.seed()
                .wrapping_add(0x9e37_79b9_7f4a_7c15_u64.wrapping_mul(next.evolution)),
        );
        if !planned && self.bars >= 32 {
            let required = (next.bpm.div_ceil(2)).div_ceil(4) * 4;
            next.bars = next.bars.max(required).min(64);
        }
        next.validate()?;
        Ok(next)
    }

    /// Renders the score to interleaved stereo.
    pub fn render(&self) -> Result<Vec<f32>, MusicError> {
        self.validate()?;
        self.render_arrangement(self.arrange())
    }

    fn render_arrangement(&self, plan: arrange::Arrangement) -> Result<Vec<f32>, MusicError> {
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
            if let Some(previous_midi) = event.glide_from_midi {
                let previous_frequency = 440.0 * 2.0_f32.powf((previous_midi - 69.0) / 12.0);
                voice = voice.with_glide_from(previous_frequency, patch.portamento_seconds);
            }
            if let Some(end) = event.end_frame {
                voice = voice.with_end_frames(end.saturating_sub(event.start_frame));
            }
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
        for frame in 0..plan.tail_frames {
            let source = (plan.frames + frame) * 2;
            let target = (frame % plan.frames) * 2;
            master[target] += master[source];
            master[target + 1] += master[source + 1];
        }
        master.truncate(plan.frames * 2);

        effects::master_tone(&mut master);
        effects::limit(&mut master, 0.97);
        effects::attenuate_to_peak(&mut master, MASTER_PEAK);
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
    instrument.mix_level()
}

/// Where a role sits by default.
///
/// Low frequencies stay centred because they carry the most energy and would
/// unbalance the mix anywhere else; everything above them is spread so the
/// parts stop competing for the same space.
fn default_pan(instrument: Instrument) -> f32 {
    instrument.default_pan()
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
            velocities: None,
            durations: None,
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
            velocities: None,
            durations: None,
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
            velocities: None,
            durations: None,
        },
    ]
}

fn arrangement_role(instrument: Instrument) -> usize {
    match instrument.role() {
        InstrumentRole::Percussion => 0,
        InstrumentRole::Bass => 1,
        InstrumentRole::Harmony => 2,
        InstrumentRole::Melody => 3,
        InstrumentRole::Colour => 4,
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
        velocities: None,
        durations: None,
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
            velocities: None,
            durations: None,
        };
    const R: Option<u8> = None;

    Score {
        title: "Velvet Circuit Procession".to_owned(),
        bpm: 96,
        bars: 48,
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
                // A two-bar phrase: a question in the first bar, an answer that
                // lands on the tonic in the second. Sixteen steps was one bar
                // looped, which is a riff, not a tune.
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
                    Some(4),
                    R,
                    R,
                    Some(5),
                    R,
                    R,
                    Some(4),
                    R,
                    Some(2),
                    R,
                    R,
                    Some(0),
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
        composition: None,
    }
}

pub fn write_wav(path: &Path, samples: &[f32]) -> Result<(), MusicError> {
    verify_audio(samples)?;
    write_pcm_wav(path, samples)
}

fn write_pcm_wav(path: &Path, samples: &[f32]) -> Result<(), MusicError> {
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
    // Quiet dynamics are intentional now that mastering no longer boosts
    // every render. Keep an actual silence floor, not a minimum loudness.
    if rms < 0.0001 {
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
    let section_floor = (rms * 0.02).max(0.00001);
    let active_sections = section_levels
        .iter()
        .filter(|level| **level >= section_floor)
        .count();
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
    let scale = mode.scale();
    let root = scale[degree];
    let third = (scale[(degree + 2) % 7] + 12 - root) % 12;
    let fifth = (scale[(degree + 4) % 7] + 12 - root) % 12;
    match (third, fifth) {
        (3, 6) => ChordQuality::Diminished,
        (4, 8) => ChordQuality::Augmented,
        (3, 7) => ChordQuality::Minor,
        _ => ChordQuality::Major,
    }
}

fn diatonic_replacement(mode: Mode, degree: u8, written: ChordQuality) -> ChordQuality {
    let scale = mode.scale();
    let root = scale[usize::from(degree % 7)];
    if written.intervals().len() == 4 {
        for quality in [
            ChordQuality::MajorSeventh,
            ChordQuality::MinorSeventh,
            ChordQuality::DominantSeventh,
            ChordQuality::HalfDiminished,
        ] {
            if quality
                .intervals()
                .iter()
                .all(|interval| scale.contains(&((root + interval) % 12)))
            {
                return quality;
            }
        }
    }
    diatonic_quality(mode, degree)
}

fn expanded_quality(quality: ChordQuality) -> ChordQuality {
    match quality {
        ChordQuality::Major => ChordQuality::MajorSeventh,
        ChordQuality::Minor => ChordQuality::MinorSeventh,
        ChordQuality::Diminished => ChordQuality::HalfDiminished,
        ChordQuality::MajorSeventh | ChordQuality::DominantSeventh => ChordQuality::Major,
        ChordQuality::MinorSeventh => ChordQuality::Minor,
        ChordQuality::HalfDiminished => ChordQuality::Diminished,
        other => other,
    }
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

pub(crate) fn chord_intervals(quality: ChordQuality) -> &'static [u8] {
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

    #[test]
    fn playback_replacement_blends_the_join_and_consumes_every_new_frame() {
        let mut source = PlaybackSource::new(vec![0.4; 800], true);
        for _ in 0..100 {
            source.next().unwrap();
        }
        let incoming: Vec<f32> = (0..PLAYBACK_TRANSITION_FRAMES + 80)
            .flat_map(|frame| [-0.3 + frame as f32 * 0.00002, -0.2 + frame as f32 * 0.00001])
            .collect();
        *source.clock.replacement.lock().unwrap() = Some(PlaybackReplacement {
            samples: incoming.clone(),
            cursor: 0,
            looped: false,
        });
        let output: Vec<_> = source.by_ref().collect();
        assert_eq!(
            output.len(),
            incoming.len(),
            "the overlap must not discard or append incoming frames"
        );
        assert_eq!(
            &output[..2],
            &[0.4, 0.4],
            "start continuously from the outgoing signal"
        );
        assert_eq!(
            &output[PLAYBACK_TRANSITION_FRAMES * 2..],
            &incoming[PLAYBACK_TRANSITION_FRAMES * 2..]
        );
        assert!(
            output
                .chunks_exact(2)
                .collect::<Vec<_>>()
                .windows(2)
                .all(|frames| (frames[1][0] - frames[0][0]).abs() < 0.001)
        );
        assert!(output.iter().all(|sample| sample.abs() <= 0.4));
        assert_eq!(
            source.clock.frames.load(Ordering::Relaxed) as usize,
            incoming.len() / 2
        );
        assert!(source.next().is_none());
    }

    #[test]
    fn playback_replacement_at_the_last_frame_keeps_the_incoming_piece_alive() {
        let mut source = PlaybackSource::new(vec![0.2, -0.1, 0.2, -0.1], false);
        for _ in 0..4 {
            source.next().unwrap();
        }
        let incoming = vec![0.05; (PLAYBACK_TRANSITION_FRAMES + 20) * 2];
        *source.clock.replacement.lock().unwrap() = Some(PlaybackReplacement {
            samples: incoming.clone(),
            cursor: 0,
            looped: false,
        });
        let output: Vec<_> = source.by_ref().collect();
        assert_eq!(output.len(), incoming.len());
        assert_eq!(&output[..2], &[0.2, -0.1]);
        assert_eq!(
            &output[PLAYBACK_TRANSITION_FRAMES * 2..],
            &incoming[PLAYBACK_TRANSITION_FRAMES * 2..]
        );
        assert!(source.clock.finished.load(Ordering::Acquire));
        assert_eq!(
            source.clock.frames.load(Ordering::Relaxed) as usize,
            incoming.len() / 2
        );
    }

    #[test]
    fn playback_replacement_keeps_sink_mute_pause_and_requested_mix_position() {
        let samples: Vec<_> = (0..SAMPLE_RATE * 4)
            .flat_map(|frame| {
                let value = (f64::from(frame) * std::f64::consts::TAU * 220.0
                    / f64::from(SAMPLE_RATE))
                .sin() as f32
                    * 0.08;
                [value, value]
            })
            .collect();
        let audio = VerifiedAudio::new(samples.clone()).unwrap();
        let source = PlaybackSource::new(samples, true);
        let clock = source.clock.clone();
        let (sink, mut output) = Sink::new_idle();
        sink.append(source);
        sink.pause();
        sink.set_volume(0.0);
        replace_playback(&sink, &clock, audio, true, 1.25).unwrap();
        assert!(sink.is_paused());
        assert_eq!(sink.volume(), 0.0);
        assert!(output.by_ref().take(4000).all(|sample| sample == 0.0));
        assert_eq!(
            clock.frames.load(Ordering::Relaxed),
            0,
            "paused audio does not consume the queued replacement"
        );
        sink.play();
        assert!(output.by_ref().take(12000).all(|sample| sample == 0.0));
        assert!(
            clock.frames.load(Ordering::Relaxed) >= 60_000,
            "the replacement starts at its requested mixer position while muted"
        );
        assert_eq!(sink.volume(), 0.0);
        sink.pause();
        for _ in 0..1000 {
            output.next();
        }
        let paused = clock.frames.load(Ordering::Relaxed);
        for _ in 0..1000 {
            output.next();
        }
        assert_eq!(clock.frames.load(Ordering::Relaxed), paused);
    }

    #[test]
    fn playback_replacement_reuses_an_exhausted_sink_and_rejects_invalid_positions() {
        let source = PlaybackSource::new(vec![0.1; 8], false);
        let clock = source.clock.clone();
        let (sink, mut output) = Sink::new_idle();
        sink.append(source);
        for _ in 0..2000 {
            output.next();
        }
        assert!(sink.empty());
        let samples = (0..SAMPLE_RATE * 4)
            .flat_map(|frame| {
                let value = (f64::from(frame) * std::f64::consts::TAU * 220.0
                    / f64::from(SAMPLE_RATE))
                .sin() as f32
                    * 0.08;
                [value, value]
            })
            .collect();
        let audio = VerifiedAudio::new(samples).unwrap();
        for position in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(replace_playback(&sink, &clock, audio.clone(), false, position).is_err());
            assert!(sink.empty());
        }
        sink.pause();
        sink.set_volume(0.3);
        replace_playback(&sink, &clock, audio, false, 0.0).unwrap();
        assert!(!sink.empty());
        assert!(sink.is_paused());
        assert_eq!(sink.volume(), 0.3);
        assert_eq!(clock.frames.load(Ordering::Relaxed), 0);
    }

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
    fn cadence_report_recognizes_authored_arrivals_and_retains_loop_closures() {
        let mut score = default_score();
        let chords = |roots: &[u8]| {
            roots
                .iter()
                .map(|&root_degree| Chord {
                    root_degree,
                    quality: diatonic_quality(Mode::Major, root_degree),
                })
                .collect()
        };
        score.mode = Mode::Major;
        score.progression = chords(&[0, 4, 0, 3, 5, 3, 4, 0]);
        assert!(
            score.composition_report().cadence,
            "the native study's closing V-I must count"
        );
        for approach in [3, 4, 6] {
            score.progression = chords(&[0, 5, approach, 0]);
            assert!(
                score.composition_report().cadence,
                "closing root {approach}->0"
            );
            score.progression = chords(&[0, 3, 5, approach]);
            assert!(
                score.composition_report().cadence,
                "loop root {approach}->0"
            );
        }
    }

    #[test]
    fn cadence_report_does_not_award_a_terminal_tonic_without_supported_approach() {
        let mut score = default_score();
        for roots in [
            &[][..],
            &[0],
            &[0, 0],
            &[0, 1, 0],
            &[0, 2, 0],
            &[0, 5, 0],
            &[0, 4, 0, 0],
        ] {
            score.progression = roots
                .iter()
                .map(|&root_degree| Chord {
                    root_degree,
                    quality: diatonic_quality(score.mode, root_degree),
                })
                .collect();
            assert!(
                !score.composition_report().cadence,
                "{roots:?} lacks a closing or loop cadential approach"
            );
        }
    }

    #[test]
    fn default_arrangement_has_every_masterwork_foundation() {
        let report = default_score().composition_report();
        assert!(report.quality >= 80, "missing craft: {:?}", report.guidance);
        assert!(report.articulation);
        assert!(report.pulse && report.foundation && report.harmony && report.melody);
        assert!(report.cadence && report.breathing_room && report.palette && report.form);
    }

    #[test]
    fn keyboard_colours_keep_the_pianos_harmonic_role_without_added_chords() {
        let mut score = default_score();
        let harmony = score
            .tracks
            .iter()
            .position(|track| track.instrument().is_chordal())
            .unwrap();
        score.tracks[harmony].instrument = Some(Instrument::Piano);
        assert!(score.composition_report().harmony);
        for &instrument in Instrument::ALL
            .iter()
            .filter(|voice| voice.can_accompany_harmony() && !voice.is_chordal())
        {
            score.tracks[harmony].instrument = Some(instrument);
            assert!(
                score.composition_report().harmony,
                "{instrument:?} lost piano accompaniment credit"
            );
            let mastered = score.mastered().unwrap();
            assert_eq!(
                mastered.tracks.len(),
                score.tracks.len(),
                "{instrument:?} added an unrequested harmony track"
            );
            assert!(
                !mastered
                    .tracks
                    .iter()
                    .any(|track| track.instrument() == Instrument::Organ)
            );
            assert!(
                !instrument.is_chordal(),
                "{instrument:?} must retain its written single notes"
            );
        }
    }

    #[test]
    fn craft_distinguishes_crowding_density_and_silent_roles() {
        let mut score = default_score();
        let healthy = score.composition_report();
        let cello = score
            .tracks
            .iter_mut()
            .find(|track| track.instrument() == Instrument::Cello)
            .unwrap();
        cello.steps = vec![Some(0); 16];
        assert!(!score.composition_report().articulation);
        assert!(score.composition_report().quality < healthy.quality);
        let mut doubled = default_score();
        doubled.tracks = vec![doubled.tracks[2].clone(); 2];
        assert!(!doubled.composition_report().separation);
        for track in &mut score.tracks {
            track.gain = 0.0;
        }
        let silent = score.composition_report();
        assert!(!silent.pulse && !silent.foundation && !silent.harmony && !silent.melody);
    }

    #[test]
    fn unrelated_json_and_malformed_native_scores_do_not_become_default_music() {
        for raw in [
            "{}",
            "{\"message\":\"hello\"}",
            "{\"tracks\":[]}",
            "{\"bpm\":96}",
        ] {
            assert!(Score::from_json(raw).is_err(), "invented music from {raw}");
        }
        let mut raw = serde_json::to_value(default_score()).unwrap();
        raw["tracks"][0].as_object_mut().unwrap().remove("steps");
        assert!(Score::from_json(&raw.to_string()).is_err());
    }

    #[test]
    fn restoring_a_muted_arrangement_does_not_add_replacement_voices() {
        let mut score = default_score();
        for track in &mut score.tracks {
            track.gain = 0.0;
        }
        let restored = score.mastered().unwrap();
        assert_eq!(restored.tracks.len(), score.tracks.len());
        assert!(restored.tracks.iter().all(|track| track.gain == 0.0));
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
        assert!(
            mastered.duration_seconds() >= 120.0 && mastered.bars <= 64,
            "a mastered piece lasts at least two minutes, got {} bars at {} BPM",
            mastered.bars,
            mastered.bpm
        );
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
        for instrument in [
            Instrument::Kick,
            Instrument::Bass,
            Instrument::Organ,
            Instrument::Harp,
        ] {
            assert_eq!(
                mastered
                    .tracks
                    .iter()
                    .filter(|track| track.instrument() == instrument)
                    .count(),
                1
            );
        }
        let remaining_flutes: Vec<_> = mastered
            .tracks
            .iter()
            .filter(|track| track.instrument() == Instrument::Flute)
            .collect();
        assert_eq!(
            remaining_flutes.len(),
            4,
            "only duplicate seats make room for missing roles"
        );
        assert!(
            remaining_flutes
                .iter()
                .all(|track| track.steps == sketch.tracks[0].steps)
        );
        // Restoring missing seats does not turn the four retained identical
        // flute lines into independent counterpoint or certify good music.
        assert!(!report.independent_parts && !report.separation);
        assert!(report.quality < 100);
        assert_eq!(report.grade, "Needs score review");
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
                report.quality >= 80,
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
    fn a_variation_keeps_the_tune() {
        // One Motif variation used to rewrite 56% of the flute and move its
        // downbeat; a Counterpoint rewrote 88% of the harp. A listener who
        // cannot learn the tune cannot hear it developed. Pitch classes and
        // the downbeat of every melodic part survive every movement except
        // Modulation, which is allowed to move the whole key.
        let mut score = default_score();
        for _ in 0..12 {
            let next = score.evolved().expect("a valid score evolves");
            let kind = EvolutionKind::for_revision(next.evolution);
            for (before, after) in score.tracks.iter().zip(&next.tracks) {
                if before.instrument().is_percussive() || kind == EvolutionKind::Modulation {
                    continue;
                }
                let pitches = |track: &Track| {
                    track
                        .steps
                        .iter()
                        .flatten()
                        .copied()
                        .collect::<std::collections::BTreeSet<u8>>()
                };
                assert!(
                    pitches(after).is_subset(&pitches(before)),
                    "{kind:?} invented pitches in {}: {:?} -> {:?}",
                    before.name,
                    before.steps,
                    after.steps
                );
                assert_eq!(
                    before.steps[0].is_some(),
                    after.steps[0].is_some(),
                    "{kind:?} moved the downbeat of {}",
                    before.name
                );
            }
            score = next;
        }
    }

    #[test]
    fn every_movement_is_audible_to_the_runtime() {
        // The runtime rejects a variation whose heard material is unchanged,
        // so each movement has to touch tempo, bars, key, progression, or a
        // track — swing alone is invisible to that check.
        let mut score = default_score();
        for _ in 0..12 {
            let next = score.evolved().expect("a valid score evolves");
            let heard = |s: &Score| {
                serde_json::to_string(&(s.bpm, s.bars, s.tonic, &s.progression, &s.tracks)).unwrap()
            };
            assert_ne!(
                heard(&score),
                heard(&next),
                "{:?} changed nothing the runtime can hear",
                EvolutionKind::for_revision(next.evolution)
            );
            score = next;
        }
    }

    #[test]
    fn a_mastered_piece_lasts_at_least_two_minutes() {
        // V1 refused anything under 120 seconds. Mastering used to cap every
        // piece at sixteen bars, which at 96 BPM is forty seconds.
        let mut short = default_score();
        short.bars = 8;
        let mastered = short.mastered().expect("mastering succeeds");
        assert!(
            mastered.duration_seconds() >= 120.0,
            "{} bars at {} BPM is only {:.0}s",
            mastered.bars,
            mastered.bpm,
            mastered.duration_seconds()
        );
        assert!(mastered.bars <= 64);
        let mut lineage = mastered;
        for _ in 0..24 {
            lineage = lineage.evolved().unwrap();
            if lineage.bpm <= 128 {
                assert!(
                    lineage.duration_seconds() >= 120.0,
                    "modulation shortened the live piece"
                );
            }
        }
    }

    #[test]
    fn playback_clock_counts_complete_stereo_frames_across_loops() {
        let mut source = PlaybackSource::new(vec![0.1, 0.2, 0.3, 0.4], true);
        let clock = Arc::clone(&source.clock);
        assert_eq!(source.next(), Some(0.1));
        assert_eq!(clock.frames.load(Ordering::Relaxed), 0);
        assert_eq!(source.next(), Some(0.2));
        assert_eq!(clock.frames.load(Ordering::Relaxed), 1);
        assert_eq!(
            source.by_ref().take(4).collect::<Vec<_>>(),
            vec![0.3, 0.4, 0.1, 0.2]
        );
        assert_eq!(clock.frames.load(Ordering::Relaxed), 3);
        // Seeking to the old player's absolute frame retains the loop count
        // and starts a mixer replacement on the same left/right sample pair.
        clock.seek.store(5, Ordering::Relaxed);
        assert_eq!(source.next(), Some(0.3));
        assert_eq!(source.next(), Some(0.4));
        assert_eq!(clock.frames.load(Ordering::Relaxed), 6);
    }

    #[test]
    fn one_shot_playback_emits_the_complete_file_once() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let mut source = PlaybackSource::new(samples.clone(), false);
        let clock = Arc::clone(&source.clock);
        assert_eq!(source.by_ref().collect::<Vec<_>>(), samples);
        assert_eq!(source.next(), None);
        assert_eq!(clock.frames.load(Ordering::Relaxed), 2);
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
    fn re_orchestration_never_doubles_a_voice() {
        // A live lineage carried two reeds and a walk produced two pads: the
        // index offset let two parts of one family land on the same seat.
        // Doubling a timbre in one register is masking, not orchestration.
        let mut score = brief();
        for _ in 0..48 {
            score = score.evolved().expect("a valid score evolves");
            let mut pitched: Vec<Instrument> = score
                .tracks
                .iter()
                .map(Track::instrument)
                .filter(|instrument| !instrument.is_percussive())
                .collect();
            let before = pitched.len();
            pitched.sort_by_key(|instrument| format!("{instrument:?}"));
            pitched.dedup();
            assert_eq!(
                pitched.len(),
                before,
                "variation {} doubled a voice: {:?}",
                score.evolution,
                score
                    .tracks
                    .iter()
                    .map(Track::instrument)
                    .collect::<Vec<_>>()
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
    fn the_master_keeps_percussion_transients_audible_with_headroom() {
        // An aggregate spectrum from a different V1 arrangement is not an EQ
        // target for every V2 piece. Compare each actual transient to its dry
        // native voice instead; the old fixed presence notch buried these.
        for instrument in [Instrument::Hat, Instrument::Shaker] {
            let mut score = default_score();
            score.bpm = 120;
            score.bars = 2;
            score.seed = Some(42);
            score.progression.clear();
            score.sidechain = Some(0.0);
            score.composition = Some(CompositionPlan {
                development: 0.0,
                ..Default::default()
            });
            score.tracks.truncate(1);
            let track = &mut score.tracks[0];
            track.instrument = Some(instrument);
            track.gain = 0.5;
            track.pan = Some(0.0);
            track.steps = vec![Some(0), None, None, None];
            track.velocities = Some(vec![0.7; 4]);
            track.durations = Some(vec![0.25; 4]);
            let event = score.arrange().events[0];
            let frequency = 440.0 * 2.0_f32.powf((event.midi - 69.0) / 12.0);
            let mut voice = synth::Voice::new(
                instrument.patch(),
                frequency,
                event.velocity,
                event.gate_seconds,
                event.seed,
            );
            let frames = (SAMPLE_RATE as f32 * 0.12) as usize;
            let dry_energy = (0..frames)
                .map(|_| voice.next(event.cutoff_scale).powi(2))
                .sum::<f32>();
            let dry_rms = (dry_energy / frames as f32).sqrt() * 0.5 * instrument.mix_level()
                / std::f32::consts::SQRT_2;
            let audio = score.render().unwrap();
            let onset = &audio[event.start_frame * 2..(event.start_frame + frames) * 2];
            let output_rms = (onset.iter().map(|sample| sample * sample).sum::<f32>()
                / onset.len() as f32)
                .sqrt();
            assert!(
                output_rms > dry_rms * 0.35,
                "{instrument:?}: the mix buried its dry transient ({dry_rms} -> {output_rms})"
            );
            assert!(
                output_rms < dry_rms * 2.0,
                "{instrument:?}: unexpected transient boost"
            );
            assert!(
                audio
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= MASTER_PEAK + 0.0001)
            );
        }
    }
    #[test]
    fn polyphonic_music_retains_periodic_structure_above_white_noise() {
        let report = verify_audio(&default_score().render().unwrap()).unwrap();
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
        // Clean, equal-tempered chord tones do not share a short common
        // period. A fixed 0.45 threshold rejected a perfectly clean Cmaj7
        // (0.308). Calibrate against a deterministic noise control instead.
        let seventh: Vec<f32> = (0..SAMPLE_RATE as usize * 8)
            .flat_map(|frame| {
                let time = frame as f64 / f64::from(SAMPLE_RATE);
                let sample = [48.0_f64, 52.0, 55.0, 59.0]
                    .iter()
                    .map(|midi| {
                        let frequency = 440.0 * 2.0_f64.powf((midi - 69.0) / 12.0);
                        (std::f64::consts::TAU * frequency * time).sin()
                    })
                    .sum::<f64>() as f32
                    * 0.15;
                [sample, sample]
            })
            .collect();
        assert!(periodicity(&seventh) > noise_periodicity * 3.0);
        assert!(
            report.periodicity > noise_periodicity * 3.0,
            "default mix periodicity {:.3} did not separate from noise {:.3}",
            report.periodicity,
            noise_periodicity
        );
        assert!(noise_periodicity < 0.15);
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
                    velocities: None,
                    durations: None,
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
                    velocities: None,
                    durations: None,
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
                .map(|event| event.start_frame)
                .collect::<std::collections::HashSet<_>>()
                .len();
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
    fn theremin_renderer_glides_across_slow_connected_half_notes() {
        let score = theremin_connection_score(2, 2.0);
        let mut straight = score.arrange();
        for pair in straight.events.windows(2) {
            let tapered_end =
                pair[0].start_frame as f32 + pair[0].gate_seconds * SAMPLE_RATE as f32;
            assert!(pair[1].start_frame as f32 - tapered_end > 0.08 * SAMPLE_RATE as f32);
            assert_eq!(pair[1].glide_from_midi, Some(pair[0].midi));
        }
        for event in &mut straight.events {
            event.glide_from_midi = None;
        }
        let actual = score.render().unwrap();
        let without_glide = score.render_arrangement(straight).unwrap();
        let changed = actual
            .iter()
            .zip(without_glide)
            .any(|(left, right)| (left - right).abs() > 0.001);
        assert!(
            changed,
            "the full renderer ignored the authored legato connection"
        );
    }

    #[test]
    fn theremin_renderer_respects_explicit_rests_shorter_than_eighty_milliseconds() {
        let score = theremin_connection_score(4, 0.9);
        let mut forced = score.arrange();
        assert!(
            forced
                .events
                .iter()
                .all(|event| event.glide_from_midi.is_none())
        );
        for index in 1..forced.events.len() {
            let previous = forced.events[index - 1];
            let gap = forced.events[index].start_frame as f32
                - previous.start_frame as f32
                - previous.gate_seconds * SAMPLE_RATE as f32;
            assert!(gap > 0.0 && gap < 0.08 * SAMPLE_RATE as f32);
            forced.events[index].glide_from_midi = Some(previous.midi);
        }
        let actual = score.render().unwrap();
        let without_glide = score.render_arrangement(score.arrange()).unwrap();
        assert_eq!(actual, without_glide);
        let forced_glide = score.render_arrangement(forced).unwrap();
        assert!(
            actual
                .iter()
                .zip(forced_glide)
                .any(|(left, right)| (left - right).abs() > 0.001),
            "the render comparison must exercise audible glide"
        );
    }

    fn theremin_connection_score(divisions: usize, duration: f32) -> Score {
        let mut score = default_score();
        score.bars = 2;
        score.bpm = 96;
        score.progression.clear();
        score.composition = Some(CompositionPlan {
            development: 0.0,
            ..Default::default()
        });
        score.tracks.truncate(1);
        let track = &mut score.tracks[0];
        track.instrument = Some(Instrument::Theremin);
        track.octave = 4;
        track.gate = None;
        track.steps = (0..divisions)
            .map(|index| Some(if index % 2 == 0 { 0 } else { 4 }))
            .collect();
        track.durations = Some(vec![duration; divisions]);
        track.velocities = Some(vec![0.8; divisions]);
        score
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
            peak > 0.01 && peak <= MASTER_PEAK + 0.0001,
            "master should retain dynamics below {MASTER_PEAK}, got {peak:.3}"
        );
    }

    #[test]
    fn lowering_the_written_mix_actually_lowers_playback() {
        let mut score = theremin_connection_score(4, 0.6);
        score.tracks[0].instrument = Some(Instrument::Piano);
        score.tracks[0].gain = 0.4;
        let loud = score.render().unwrap();
        score.tracks[0].gain = 0.1;
        let quiet = score.render().unwrap();
        let rms = |samples: &[f32]| {
            (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32)
                .sqrt()
        };
        let ratio = rms(&quiet) / rms(&loud);
        assert!(
            (0.23..0.28).contains(&ratio),
            "a quarter-gain piano should be about 12 dB quieter, RMS ratio {ratio}"
        );
        score.tracks[0].gain = 0.0;
        assert!(score.render().unwrap().iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn native_variations_preserve_an_authored_ensemble() {
        let mut score = brief();
        score.composition = Some(CompositionPlan::default());
        let voices: Vec<_> = score.tracks.iter().map(Track::instrument).collect();
        for _ in 0..36 {
            score = score.evolved().unwrap();
            assert_eq!(
                score
                    .tracks
                    .iter()
                    .map(Track::instrument)
                    .collect::<Vec<_>>(),
                voices,
                "native evolution should preserve the composed instrumentation"
            );
        }
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
    fn melodic_lines_stay_in_the_scale_while_connecting_chords() {
        let score = brief();
        let plan = score.arrange();
        let lead = melody_track(&score);
        let progression_length = score.progression.len();
        assert!(progression_length > 1);

        for event in plan.events.iter().filter(|event| event.track == lead) {
            let scale_pitches: Vec<u8> = score
                .mode
                .scale()
                .iter()
                .map(|offset| (offset + score.tonic) % 12)
                .collect();
            let pitch_class = (event.midi as i16).rem_euclid(12) as u8;
            assert!(
                scale_pitches.contains(&pitch_class),
                "lead pitch class {pitch_class} left the selected scale"
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
    fn quiet_music_keeps_its_dynamics_and_section_verification() {
        let audio = brief().render().unwrap();
        let quiet: Vec<_> = audio.iter().map(|sample| sample * 0.02).collect();
        let report = verify_audio(&quiet).unwrap();
        assert_eq!(
            report.active_sections,
            verify_audio(&audio).unwrap().active_sections
        );
        let mut missing = quiet;
        let half = missing.len() / 2;
        missing[half..].fill(0.0);
        assert!(
            verify_audio(&missing).is_err(),
            "quiet material must still be complete"
        );
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
#[test]
#[ignore = "requires an audio device; playback remains muted throughout"]
fn operational_device_pause_and_muted_transport() {
    use std::time::{Duration, Instant};
    let samples = (0..48_000 * 4)
        .flat_map(|i| {
            let sample = (i as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin() * 0.1;
            [sample, sample]
        })
        .collect();
    let verified = VerifiedAudio::new(samples).unwrap();
    let player = Player::prepare_on(verified.clone(), false, None).unwrap();
    player.set_muted(true);
    assert!(player.sink.is_paused());
    std::thread::sleep(Duration::from_millis(4200));
    assert!(
        !player.is_finished(),
        "a prepared/paused player consumed the score"
    );
    player.resume();
    assert_eq!(player.sink.volume(), 0.0);
    let deadline = Instant::now() + Duration::from_secs(6);
    while !player.is_finished() {
        assert!(
            Instant::now() < deadline,
            "mute paused the actual audio clock"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(Player::prepare_on(verified, false, Some("teledra-missing-probe-device")).is_err());
}
