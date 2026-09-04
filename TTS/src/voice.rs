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
}

impl VoiceName {
    pub const ALL: [VoiceName; 9] = [
        VoiceName::Queen,
        VoiceName::Organist,
        VoiceName::Alchemist,
        VoiceName::Archivist,
        VoiceName::Orator,
        VoiceName::Scribe,
        VoiceName::Artist,
        VoiceName::Diplomat,
        VoiceName::Treasurer,
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

/// A voice together with the gain it needs to sit in the mix.
///
/// Reference recordings differ in level, and V1 discovered by ear that the
/// Organist and Alchemist were inaudible against the rest of the court; their
/// correction is carried here rather than being rediscovered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voice {
    pub name: VoiceName,
    pub gain: f32,
}

impl Voice {
    /// Beyond this, the clamp below does more shaping than the gain does.
    const MAX_GAIN: f32 = 8.0;
    /// Samples are clamped here so a boosted voice cannot clip the output.
    pub const CEILING: f32 = 0.98;

    pub fn new(name: VoiceName) -> Self {
        Self {
            name,
            gain: default_gain(name),
        }
    }

    pub fn with_gain(name: VoiceName, gain: f32) -> Self {
        Self {
            name,
            gain: gain.clamp(0.0, Self::MAX_GAIN),
        }
    }

    /// Below this the gained signal is passed through untouched.
    ///
    /// Speech spends most of its time well under the ceiling, so a knee this
    /// high leaves ordinary syllables exactly as synthesised and acts only on
    /// the peaks that would otherwise have been flattened.
    const KNEE: f32 = 0.62;

    /// Applies the voice's gain, bounding the result without clipping it.
    ///
    /// This previously clamped. Clamping is clipping: it squares off the top of
    /// every waveform that crosses the ceiling and adds broadband distortion
    /// exactly on the loudest, most intelligible syllables. It was not a rare
    /// edge case — the Queen's gain is above unity and her synthesised peaks
    /// approach full scale, so nearly every emphatic line she spoke was being
    /// flattened. Above the knee the signal is now compressed smoothly onto the
    /// ceiling instead, which is inaudible where clamping was not.
    pub fn level(self, samples: &mut [f32]) {
        for sample in samples {
            *sample = Self::soften(*sample * self.gain);
        }
    }

    /// A soft knee that maps any finite input into `+/-CEILING`.
    fn soften(sample: f32) -> f32 {
        if !sample.is_finite() {
            return 0.0;
        }
        let magnitude = sample.abs();
        if magnitude <= Self::KNEE {
            return sample;
        }
        // The excess above the knee is compressed by tanh, which is smooth at
        // the knee and asymptotic at the ceiling, so the curve has no corner
        // for the ear to hear.
        let headroom = Self::CEILING - Self::KNEE;
        let excess = (magnitude - Self::KNEE) / headroom;
        let shaped = Self::KNEE + headroom * excess.tanh();
        shaped.copysign(sample)
    }
}

/// Levels measured by ear against the original court mix.
fn default_gain(name: VoiceName) -> f32 {
    match name {
        VoiceName::Organist => 5.5,
        VoiceName::Artist => 3.25,
        VoiceName::Scribe => 2.55,
        VoiceName::Archivist => 2.45,
        VoiceName::Alchemist => 4.5,
        VoiceName::Orator => 2.45,
        VoiceName::Diplomat => 2.6,
        VoiceName::Treasurer => 2.5,
        VoiceName::Queen => 1.22,
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
    fn the_quiet_voices_are_lifted() {
        assert!(Voice::new(VoiceName::Organist).gain > Voice::new(VoiceName::Queen).gain);
        assert!(Voice::new(VoiceName::Alchemist).gain > Voice::new(VoiceName::Queen).gain);
        assert_eq!(Voice::new(VoiceName::Archivist).gain, 2.45);
    }

    #[test]
    fn loud_speech_is_compressed_rather_than_flattened() {
        // Clamping produced runs of identical samples at the ceiling — a
        // squared-off waveform, which is what distortion sounds like. The
        // softened curve must stay strictly monotonic instead, so louder input
        // is still louder output all the way up.
        let queen = Voice::new(VoiceName::Queen);
        let mut samples: Vec<f32> = (0..64).map(|i| i as f32 / 63.0).collect();
        queen.level(&mut samples);
        for pair in samples.windows(2) {
            assert!(
                pair[1] > pair[0],
                "the curve flattened at {pair:?}, which is clipping"
            );
        }
        assert!(samples.iter().all(|s| s.abs() <= Voice::CEILING));
    }

    #[test]
    fn quiet_speech_is_left_exactly_as_synthesised() {
        // The knee must not colour ordinary syllables.
        let queen = Voice::new(VoiceName::Queen);
        let mut samples = vec![0.1, -0.2, 0.35];
        let expected: Vec<f32> = samples.iter().map(|s| s * queen.gain).collect();
        queen.level(&mut samples);
        for (got, want) in samples.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{got} != {want}");
        }
    }

    #[test]
    fn a_boosted_voice_cannot_clip_the_output() {
        let organist = Voice::new(VoiceName::Organist);
        let mut samples = vec![0.9, -0.9, 0.01];
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
}
