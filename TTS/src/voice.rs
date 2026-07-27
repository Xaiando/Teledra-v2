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

    /// Applies the voice's gain, bounded so the result cannot clip.
    pub fn level(self, samples: &mut [f32]) {
        for sample in samples {
            *sample = (*sample * self.gain).clamp(-Self::CEILING, Self::CEILING);
        }
    }
}

/// Levels measured by ear against the rest of the court.
fn default_gain(name: VoiceName) -> f32 {
    match name {
        // Both reference recordings are inherently soft and were inaudible
        // on stream at unity.
        VoiceName::Organist => 5.5,
        VoiceName::Alchemist => 4.5,
        _ => 1.0,
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
