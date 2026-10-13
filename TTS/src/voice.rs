//! Which voice each member of the court speaks in.

use std::fmt;
use std::str::FromStr;

/// A voice the worker knows how to render.
///
/// This is a closed set rather than a free string so a typo becomes a compile
/// error instead of a court member who silently falls back to the Queen — V1
/// resolved voices by name and mis-set voices were only ever noticed by ear.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum VoiceName {
    Queen,
    Organist,
    Alchemist,
    Archivist,
    Orator,
    Scribe,
    Artist,
    Diplomat,
    Treasurer,
    Malthus,
    Wizard,
}

impl VoiceName {
    pub const ALL: [VoiceName; 11] = [
        VoiceName::Queen,
        VoiceName::Organist,
        VoiceName::Alchemist,
        VoiceName::Archivist,
        VoiceName::Orator,
        VoiceName::Scribe,
        VoiceName::Artist,
        VoiceName::Diplomat,
        VoiceName::Treasurer,
        VoiceName::Malthus,
        VoiceName::Wizard,
    ];

    /// The identifier sent to the worker on the wire.
    pub fn slug(self) -> &'static str {
        match self {
            VoiceName::Queen => "queen",
            VoiceName::Organist => "organist",
            VoiceName::Alchemist => "alchemist",
            VoiceName::Archivist => "archivist",
            VoiceName::Orator => "orator",
            VoiceName::Scribe => "scribe",
            VoiceName::Artist => "artist",
            VoiceName::Diplomat => "diplomat",
            VoiceName::Treasurer => "treasurer",
            VoiceName::Malthus => "malthus",
            VoiceName::Wizard => "wizard",
        }
    }
}

impl fmt::Display for VoiceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.slug())
    }
}

impl FromStr for VoiceName {
    type Err = UnknownVoice;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let wanted = raw.trim().to_ascii_lowercase();
        VoiceName::ALL
            .into_iter()
            .find(|voice| voice.slug() == wanted)
            .ok_or_else(|| UnknownVoice(raw.to_owned()))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("'{0}' is not a voice of this court")]
pub struct UnknownVoice(pub String);

/// The Queen's reference recordings, selected by mode.
///
/// V1 kept several recordings for Teledra and chose between them; V2 shipped
/// only the clean one while the court's screen reported "custom", so the
/// operator was told a voice was in use that the worker could not produce.
/// The mode is resolved in one place and both the worker and the display read
/// it, so the screen cannot disagree with what is heard again.
pub const QUEEN_VOICE_MODES: [&str; 4] = ["custom", "clean", "sarcastic", "analytical"];

/// V1's working setup, and what the court has always claimed on screen.
pub const DEFAULT_QUEEN_VOICE_MODE: &str = "custom";

/// The Queen's configured reference mode, bounded to one that exists.
pub fn queen_voice_mode() -> &'static str {
    let wanted = std::env::var("TELEDRA_QUEEN_VOICE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    QUEEN_VOICE_MODES
        .into_iter()
        .find(|mode| *mode == wanted)
        .unwrap_or(DEFAULT_QUEEN_VOICE_MODE)
}

/// A voice together with an optional output-level trim.
///
/// LuxTTS matches output volume to the reference recording. The Organist and
/// Artist references are much quieter than the Queen, and fixed gains still
/// left them difficult to hear. Measure each utterance before playback so all
/// offices reach the same speaking level, regardless of the recording used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voice {
    pub name: VoiceName,
    pub gain: f32,
}

impl Voice {
    /// Bound an explicit output trim separately from automatic leveling.
    const MAX_GAIN: f32 = 8.0;
    /// Active speech target, approximately -12.8 dBFS before peak protection.
    const TARGET_RMS: f32 = 0.23;
    /// Do not turn near-silent output into amplified noise.
    const NOISE_FLOOR: f32 = 0.001;
    const MAX_LEVEL_GAIN: f32 = 32.0;
    /// Peak protection keeps a boosted voice below full scale.
    pub const CEILING: f32 = 0.98;

    pub fn new(name: VoiceName) -> Self {
        Self { name, gain: 1.0 }
    }

    pub fn with_gain(name: VoiceName, gain: f32) -> Self {
        Self {
            name,
            gain: if gain.is_nan() {
                1.0
            } else {
                gain.clamp(0.0, Self::MAX_GAIN)
            },
        }
    }

    /// Levels the worker's 48 kHz speech and applies the output trim.
    ///
    /// One gain for the whole utterance preserves its internal dynamics. Quiet
    /// pauses are excluded from the measurement, so adding a pause cannot make
    /// the next line louder. Peak control changes a smooth gain envelope instead
    /// of saturating individual samples, which added harmonics to loud syllables.
    pub fn level(self, samples: &mut [f32]) {
        self.level_at_rate(samples, 48_000);
    }

    /// The same processing at the actual PCM rate, including limiter timing.
    pub fn level_at_rate(self, samples: &mut [f32], sample_rate: u32) {
        let sample_rate = sample_rate.max(1);
        let rms = Self::active_rms(samples, (sample_rate as usize / 50).max(1));
        let correction = if rms > Self::NOISE_FLOOR {
            (Self::TARGET_RMS / rms).min(Self::MAX_LEVEL_GAIN)
        } else {
            1.0
        };
        for sample in samples.iter_mut() {
            *sample = if sample.is_finite() {
                *sample * correction * self.gain
            } else {
                0.0
            };
        }
        Self::limit_peaks(samples, sample_rate);
    }

    fn active_rms(samples: &[f32], window_size: usize) -> f32 {
        let windows: Vec<(f64, usize)> = samples
            .chunks(window_size)
            .map(|window| {
                let energy = window
                    .iter()
                    .filter(|sample| sample.is_finite())
                    .map(|sample| f64::from(*sample).powi(2))
                    .sum::<f64>();
                (energy, window.len())
            })
            .collect();
        let strongest = windows
            .iter()
            .map(|(energy, count)| energy / *count as f64)
            .fold(0.0_f64, f64::max);
        // Gate at 20 dB below the strongest speech window, with an absolute
        // floor. This excludes sentence gaps without discarding soft syllables.
        let gate = (strongest * 0.01).max(f64::from(Self::NOISE_FLOOR).powi(2));
        let (energy, count) = windows
            .into_iter()
            .filter(|(energy, count)| energy / *count as f64 > gate)
            .fold((0.0, 0), |(sum, count), (next, size)| {
                (sum + next, count + size)
            });
        if count == 0 {
            0.0
        } else {
            (energy / count as f64).sqrt() as f32
        }
    }

    /// Offline look-ahead: lower the gain smoothly before a peak, hold for
    /// 20 ms, then recover over 80 ms. The hold keeps the gain from tracking
    /// individual pitch cycles. Backwards attack and forwards release passes
    /// cover every peak without delaying audio or clipping its shape.
    fn limit_peaks(samples: &mut [f32], sample_rate: u32) {
        if samples.iter().all(|sample| sample.abs() <= Self::CEILING) {
            return;
        }
        // Leave one rounding step of margin when multiplying f32 PCM.
        let ceiling = Self::CEILING * (1.0 - f32::EPSILON);
        let attack_step = 1.0 / (sample_rate as f32 * 0.005).max(1.0);
        let release_step = 1.0 / (sample_rate as f32 * 0.080).max(1.0);
        let mut gains = vec![1.0_f32; samples.len()];
        let mut gain = 1.0_f32;
        for (sample, envelope) in samples.iter().zip(gains.iter_mut()).rev() {
            let required = if sample.abs() > ceiling {
                ceiling / sample.abs()
            } else {
                1.0
            };
            gain = required.min(gain + attack_step);
            *envelope = gain;
        }
        gain = gains.first().copied().unwrap_or(1.0);
        let hold_samples = (sample_rate / 50).max(1);
        let mut hold = 0;
        for (sample, envelope) in samples.iter_mut().zip(gains) {
            if envelope <= gain {
                gain = envelope;
                hold = hold_samples;
            } else if hold > 0 {
                hold -= 1;
            } else {
                gain = envelope.min(gain + release_step);
            }
            *sample *= gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_survives_a_round_trip_through_its_slug() {
        for voice in VoiceName::ALL {
            assert_eq!(voice.slug().parse::<VoiceName>().unwrap(), voice);
        }
    }

    #[test]
    fn an_unknown_voice_is_refused_rather_than_substituted() {
        assert!("jester".parse::<VoiceName>().is_err());
        assert!("".parse::<VoiceName>().is_err());
        // Casing and stray whitespace are the operator's, not an error.
        assert_eq!("  Queen ".parse::<VoiceName>().unwrap(), VoiceName::Queen);
    }

    #[test]
    fn quiet_and_loud_recordings_reach_the_same_speaking_level() {
        // The quiet reference voices arrive well below the Queen. The old
        // fixed multipliers could not compensate for each generated line.
        for (voice, amplitude) in [
            (VoiceName::Queen, 0.35),
            (VoiceName::Organist, 0.025),
            (VoiceName::Artist, 0.04),
        ] {
            let mut samples = voiced_samples(amplitude);
            Voice::new(voice).level(&mut samples);
            let rms = Voice::active_rms(&samples, 960);
            assert!((rms - Voice::TARGET_RMS).abs() < 0.002, "{voice}: {rms}");
            assert!(samples.iter().all(|s| s.abs() <= Voice::CEILING));
        }
    }

    #[test]
    fn loud_syllables_below_the_ceiling_keep_their_waveform() {
        // RMS normalization of a high-crest voice can leave peaks above the
        // former 0.62 knee while remaining safely below full scale. Those
        // peaks must no longer be reshaped into extra harmonics.
        let mut samples: Vec<f32> = (0..48_000)
            .map(|i| {
                let amplitude = if i % 4_800 < 480 { 0.8 } else { 0.25 };
                amplitude * (i as f32 * std::f32::consts::TAU * 200.0 / 48_000.0).sin()
            })
            .collect();
        let before = samples.clone();
        let correction = Voice::TARGET_RMS / Voice::active_rms(&before, 960);
        assert!(before.iter().any(|sample| sample.abs() * correction > 0.62));
        assert!(
            before
                .iter()
                .all(|sample| sample.abs() * correction < Voice::CEILING)
        );
        Voice::new(VoiceName::Queen).level(&mut samples);
        for (got, original) in samples.iter().zip(before) {
            assert!((got - original * correction).abs() < 1e-6);
        }
    }

    #[test]
    fn peak_control_does_not_turn_a_tone_into_electrical_buzz() {
        // A sustained boosted tone isolates harmonic distortion. The former
        // tanh saturator added strong odd harmonics even though it never clipped.
        for rate in [24_000, 48_000, 96_000] {
            let mut samples: Vec<f32> = (0..rate)
                .map(|i| {
                    2.0 * (i as f64 * std::f64::consts::TAU * 200.0 / f64::from(rate)).sin() as f32
                })
                .collect();
            Voice::limit_peaks(&mut samples, rate);
            assert!(samples.iter().all(|sample| sample.abs() <= Voice::CEILING));
            // Skip the entrance and exit; use a whole number of tone periods.
            let steady = &samples[rate as usize / 4..rate as usize * 3 / 4];
            let amplitude = |harmonic: u32| {
                let (real, imag) =
                    steady
                        .iter()
                        .enumerate()
                        .fold((0.0, 0.0), |(re, im), (i, sample)| {
                            let phase =
                                i as f64 * std::f64::consts::TAU * 200.0 * f64::from(harmonic)
                                    / f64::from(rate);
                            (
                                re + f64::from(*sample) * phase.cos(),
                                im + f64::from(*sample) * phase.sin(),
                            )
                        });
                real.hypot(imag)
            };
            let distortion =
                (2..=10).map(|h| amplitude(h).powi(2)).sum::<f64>().sqrt() / amplitude(1);
            assert!(
                distortion < 0.003,
                "{rate} Hz: harmonic distortion {distortion}"
            );
        }
    }

    #[test]
    fn peak_control_looks_ahead_and_recovers_without_flattening_a_peak() {
        let mut samples = vec![0.1; 9_600];
        samples[4_800] = 2.0;
        Voice::limit_peaks(&mut samples, 48_000);
        assert_eq!(samples[0], 0.1);
        assert!(samples[4_799] < 0.06, "gain must drop before the transient");
        assert!(samples[4_800] <= Voice::CEILING);
        assert!(samples[4_801] < 0.06, "gain must recover gradually");
        assert_eq!(samples[9_599], 0.1);
    }

    #[test]
    fn pauses_do_not_change_speaking_level_or_internal_dynamics() {
        let mut compact = voiced_samples(0.04);
        let mut paused = vec![0.0; 48_000];
        paused.extend_from_slice(&compact);
        paused.extend_from_slice(&[0.0; 48_000]);
        let before = compact.clone();
        Voice::new(VoiceName::Artist).level(&mut compact);
        Voice::new(VoiceName::Artist).level(&mut paused);
        assert_eq!(&paused[48_000..96_000], compact);
        assert!(paused[..48_000].iter().all(|sample| *sample == 0.0));
        // A single multiplier preserves the difference between soft and loud
        // syllables, rather than pushing every short window to the same level.
        let scale = compact[100] / before[100];
        for (got, original) in compact.iter().zip(before) {
            assert!((got - original * scale).abs() < 1e-6);
        }
    }

    #[test]
    fn silence_and_noise_are_not_amplified() {
        let mut noise = vec![0.0001, -0.0001, 0.0, f32::NAN, f32::INFINITY];
        Voice::new(VoiceName::Organist).level(&mut noise);
        assert_eq!(noise, vec![0.0001, -0.0001, 0.0, 0.0, 0.0]);
        Voice::new(VoiceName::Artist).level(&mut []);
    }

    #[test]
    fn an_explicit_gain_still_controls_output_volume() {
        let mut normal = voiced_samples(0.1);
        let mut softer = normal.clone();
        let mut muted = normal.clone();
        Voice::new(VoiceName::Queen).level(&mut normal);
        Voice::with_gain(VoiceName::Queen, 0.5).level(&mut softer);
        Voice::with_gain(VoiceName::Queen, 0.0).level(&mut muted);
        for (normal, softer) in normal.iter().zip(softer) {
            assert!((normal * 0.5 - softer).abs() < 1e-6);
        }
        assert!(muted.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn a_boosted_voice_cannot_clip_the_output() {
        let organist = Voice::new(VoiceName::Organist);
        let mut samples = voiced_samples(0.02);
        samples[0] = 0.9;
        samples[1] = -0.9;
        samples[2] = 0.01;
        organist.level(&mut samples);
        assert!(
            samples.iter().all(|s| s.abs() <= Voice::CEILING),
            "gain must be bounded by the ceiling: {samples:?}"
        );
        // The quiet sample is still genuinely lifted.
        assert!(samples[2] > 0.01);
    }

    #[test]
    fn gain_is_bounded_even_when_set_absurdly() {
        let shouting = Voice::with_gain(VoiceName::Queen, 1_000.0);
        let mut samples = vec![0.5];
        shouting.level(&mut samples);
        assert!(samples[0] <= Voice::CEILING);
    }

    fn voiced_samples(amplitude: f32) -> Vec<f32> {
        (0..48_000)
            .map(|i| {
                let dynamics = if i < 24_000 { 0.5 } else { 1.0 };
                amplitude * dynamics * (i as f32 * std::f32::consts::TAU * 220.0 / 48_000.0).sin()
            })
            .collect()
    }
}
