//! Voice-level synthesis: band-limited oscillators, envelopes, filter, patches.
//!
//! The previous engine summed naive saw and square waves straight into the mix.
//! Those have infinite harmonics, so every partial above 24 kHz folded back down
//! as inharmonic noise — the metallic buzz that made the court sound cheap. Saw
//! and pulse are now band-limited with PolyBLEP, and every voice runs through a
//! resonant filter with its own envelope, which is what makes a synthesiser
//! sound like an instrument rather than a tone generator.

use std::f32::consts::{PI, TAU};

pub use crate::Instrument;

use crate::{SAMPLE_RATE, Waveform};

const SR: f32 = SAMPLE_RATE as f32;

mod presets;

impl Instrument {
    /// A one-line description of the patch's envelope, for auditing.
    pub fn patch_summary(self) -> (String, &'static str) {
        let patch = self.patch();
        let attack = if patch.amp.attack < 0.01 {
            "instant".to_owned()
        } else {
            format!("{:.0}ms", patch.amp.attack * 1000.0)
        };
        let decays = if patch.amp.sustain <= 0.02 {
            "to silence"
        } else {
            "sustains"
        };
        (attack, decays)
    }

    pub fn patch(self) -> Patch {
        match self {
            Self::SubBass => Patch {
                waveform: Waveform::Sine,
                voices: 1,
                sub: 0.0,
                amp: Adsr::new(0.008, 0.20, 0.85, 0.10),
                filter: Adsr::new(0.005, 0.20, 0.60, 0.10),
                cutoff_hz: 320.0,
                env_octaves: 0.6,
                resonance: 0.7,
                drive: 1.4,
                width: 0.0,
                reverb: 0.02,
                delay: 0.0,
                ..Patch::DEFAULT
            },
            Self::Bass => Patch {
                waveform: Waveform::Saw,
                voices: 2,
                detune_cents: 6.0,
                sub: 0.55,
                amp: Adsr::new(0.006, 0.16, 0.72, 0.09),
                filter: Adsr::new(0.004, 0.15, 0.30, 0.08),
                cutoff_hz: 240.0,
                env_octaves: 2.2,
                resonance: 1.5,
                drive: 1.9,
                width: 0.08,
                reverb: 0.05,
                delay: 0.03,
                ..Patch::DEFAULT
            },
            Self::Pad => Patch {
                waveform: Waveform::Saw,
                voices: 3,
                detune_cents: 9.0,
                sub: 0.12,
                amp: Adsr::new(0.45, 1.00, 0.62, 0.70),
                filter: Adsr::new(0.90, 1.60, 0.55, 1.00),
                cutoff_hz: 620.0,
                env_octaves: 1.9,
                resonance: 0.9,
                drive: 1.1,
                width: 1.0,
                reverb: 0.26,
                delay: 0.14,
                ..Patch::DEFAULT
            },
            Self::Keys => Patch {
                waveform: Waveform::Triangle,
                voices: 2,
                detune_cents: 7.0,
                sub: 0.18,
                amp: Adsr::new(0.012, 0.42, 0.52, 0.35),
                filter: Adsr::new(0.010, 0.50, 0.40, 0.30),
                cutoff_hz: 900.0,
                env_octaves: 1.2,
                resonance: 1.1,
                drive: 1.2,
                width: 0.55,
                reverb: 0.18,
                delay: 0.14,
                ..Patch::DEFAULT
            },
            Self::Pluck => Patch {
                waveform: Waveform::Square,
                voices: 2,
                detune_cents: 9.0,
                sub: 0.10,
                amp: Adsr::new(0.003, 0.20, 0.10, 0.16),
                filter: Adsr::new(0.002, 0.16, 0.08, 0.14),
                cutoff_hz: 850.0,
                env_octaves: 1.8,
                resonance: 1.2,
                drive: 1.3,
                width: 0.6,
                reverb: 0.18,
                delay: 0.22,
                ..Patch::DEFAULT
            },
            Self::Lead => Patch {
                waveform: Waveform::Saw,
                voices: 3,
                detune_cents: 11.0,
                sub: 0.14,
                amp: Adsr::new(0.020, 0.35, 0.68, 0.28),
                filter: Adsr::new(0.030, 0.45, 0.45, 0.25),
                cutoff_hz: 1_000.0,
                env_octaves: 1.4,
                resonance: 1.4,
                vibrato_cents: 7.0,
                vibrato_hz: 5.2,
                vibrato_delay: 0.22,
                drive: 1.5,
                width: 0.45,
                reverb: 0.20,
                delay: 0.26,
                ..Patch::DEFAULT
            },
            Self::Bell => Patch {
                waveform: Waveform::Sine,
                voices: 2,
                detune_cents: 0.0,
                ring: 2.0,
                ring_level: 0.22,
                amp: Adsr::new(0.002, 1.40, 0.06, 0.90),
                filter: Adsr::new(0.002, 0.90, 0.20, 0.70),
                cutoff_hz: 1_600.0,
                env_octaves: 1.0,
                resonance: 0.8,
                drive: 1.0,
                width: 0.75,
                reverb: 0.30,
                delay: 0.30,
                ..Patch::DEFAULT
            },
            Self::Arp => Patch {
                waveform: Waveform::Square,
                voices: 2,
                detune_cents: 5.0,
                sub: 0.08,
                amp: Adsr::new(0.002, 0.13, 0.06, 0.10),
                filter: Adsr::new(0.002, 0.11, 0.05, 0.09),
                cutoff_hz: 900.0,
                env_octaves: 1.5,
                resonance: 1.2,
                drive: 1.2,
                width: 0.8,
                reverb: 0.14,
                delay: 0.26,
                ..Patch::DEFAULT
            },
            // Drawbars: sine partials at the octaves and fifths of a tonewheel
            // organ, with no attack transient and no filter movement. An organ
            // is defined by which harmonics it has, not by what a filter takes
            // away, which is why the filtered triangle that stood in for it
            // sounded like neither an organ nor anything else.
            Self::Organ => Patch {
                waveform: Waveform::Sine,
                voices: 1,
                partials: [
                    (0.5, 0.30),
                    (1.0, 0.00),
                    (1.5, 0.16),
                    (2.0, 0.42),
                    (3.0, 0.12),
                    (4.0, 0.18),
                ],
                amp: Adsr::new(0.008, 0.02, 1.00, 0.09),
                filter: Adsr::new(0.001, 0.01, 1.0, 0.05),
                cutoff_hz: 4_000.0,
                env_octaves: 0.0,
                resonance: 0.7,
                vibrato_cents: 4.0,
                vibrato_hz: 6.4,
                vibrato_delay: 0.0,
                drive: 1.15,
                width: 0.45,
                reverb: 0.22,
                delay: 0.10,
                ..Patch::DEFAULT
            },
            // A struck string: immediate, then a long decay to nothing. The
            // sustain level is zero because a piano note dies whether or not
            // the key is held.
            Self::Piano => Patch {
                waveform: Waveform::Triangle,
                voices: 1,
                partials: [
                    (2.0, 0.34),
                    (3.0, 0.16),
                    (4.0, 0.09),
                    (5.01, 0.05),
                    (0.0, 0.0),
                    (0.0, 0.0),
                ],
                amp: Adsr::new(0.002, 1.70, 0.00, 0.32),
                filter: Adsr::new(0.002, 0.55, 0.10, 0.30),
                cutoff_hz: 1_100.0,
                env_octaves: 1.9,
                resonance: 0.8,
                drive: 1.2,
                width: 0.40,
                reverb: 0.24,
                delay: 0.10,
                ..Patch::DEFAULT
            },
            // Nearly a sine with breath across it. The noise is what makes it a
            // flute rather than a test tone.
            Self::Flute => Patch {
                waveform: Waveform::Sine,
                voices: 1,
                partials: [
                    (2.0, 0.11),
                    (3.0, 0.04),
                    (0.0, 0.0),
                    (0.0, 0.0),
                    (0.0, 0.0),
                    (0.0, 0.0),
                ],
                noise: 0.05,
                noise_decay: 0.9,
                amp: Adsr::new(0.075, 0.18, 0.86, 0.16),
                filter: Adsr::new(0.060, 0.30, 0.70, 0.16),
                cutoff_hz: 1_500.0,
                env_octaves: 0.8,
                resonance: 0.7,
                vibrato_cents: 11.0,
                vibrato_hz: 5.0,
                vibrato_delay: 0.30,
                drive: 1.0,
                width: 0.55,
                reverb: 0.34,
                delay: 0.16,
                ..Patch::DEFAULT
            },
            // Odd harmonics with the evens missing, which is what gives a
            // clarinet its hollow woody colour.
            Self::Reed => Patch {
                waveform: Waveform::Sine,
                voices: 1,
                partials: [
                    (3.0, 0.34),
                    (5.0, 0.17),
                    (7.0, 0.08),
                    (9.0, 0.04),
                    (0.0, 0.0),
                    (0.0, 0.0),
                ],
                noise: 0.03,
                noise_decay: 0.6,
                amp: Adsr::new(0.035, 0.14, 0.82, 0.14),
                filter: Adsr::new(0.030, 0.25, 0.65, 0.14),
                cutoff_hz: 1_300.0,
                env_octaves: 0.9,
                resonance: 0.9,
                vibrato_cents: 6.0,
                vibrato_hz: 4.6,
                vibrato_delay: 0.35,
                drive: 1.1,
                width: 0.42,
                reverb: 0.26,
                delay: 0.12,
                ..Patch::DEFAULT
            },
            // Bowed: the attack takes time because the bow has to set the
            // string moving, and the vibrato arrives after the note settles.
            Self::Cello => Patch {
                waveform: Waveform::Saw,
                voices: 2,
                detune_cents: 5.0,
                sub: 0.20,
                amp: Adsr::new(0.110, 0.30, 0.80, 0.32),
                filter: Adsr::new(0.130, 0.40, 0.62, 0.30),
                cutoff_hz: 520.0,
                env_octaves: 1.3,
                resonance: 1.0,
                vibrato_cents: 13.0,
                vibrato_hz: 5.4,
                vibrato_delay: 0.34,
                drive: 1.25,
                width: 0.50,
                reverb: 0.30,
                delay: 0.10,
                ..Patch::DEFAULT
            },
            // Plucked and left to ring: no sustain, a long tail, and none of
            // the bite of the square-wave pluck.
            Self::Harp => Patch {
                waveform: Waveform::Triangle,
                voices: 1,
                partials: [
                    (2.0, 0.26),
                    (3.0, 0.13),
                    (4.0, 0.06),
                    (0.0, 0.0),
                    (0.0, 0.0),
                    (0.0, 0.0),
                ],
                amp: Adsr::new(0.003, 1.30, 0.00, 0.55),
                filter: Adsr::new(0.003, 0.70, 0.12, 0.45),
                cutoff_hz: 1_250.0,
                env_octaves: 1.4,
                resonance: 0.7,
                drive: 1.0,
                width: 0.62,
                reverb: 0.34,
                delay: 0.18,
                ..Patch::DEFAULT
            },
            // Solo bowed strings keep a single pitch centre. Harmonics supply
            // the bow's brightness without the beating of a detuned synth pad.
            Self::Violin => Patch {
                waveform: Waveform::Sine,
                partials: [
                    (2.0, 0.55),
                    (3.0, 0.34),
                    (4.0, 0.21),
                    (5.0, 0.15),
                    (6.0, 0.10),
                    (8.0, 0.05),
                ],
                noise: 0.012,
                noise_decay: 0.20,
                amp: Adsr::new(0.085, 0.24, 0.78, 0.20),
                filter: Adsr::new(0.11, 0.30, 0.70, 0.19),
                cutoff_hz: 1_700.0,
                env_octaves: 0.9,
                resonance: 0.7,
                vibrato_cents: 8.0,
                vibrato_hz: 5.5,
                vibrato_delay: 0.32,
                drive: 0.8,
                width: 0.36,
                reverb: 0.23,
                delay: 0.05,
                ..Patch::DEFAULT
            },
            // The chalumeau colour is dominated by the fundamental and odd
            // partials, with less bite and vibrato than the generic reed.
            Self::Clarinet => Patch {
                waveform: Waveform::Sine,
                partials: [
                    (3.0, 0.46),
                    (5.0, 0.20),
                    (7.0, 0.07),
                    (9.0, 0.025),
                    (2.0, 0.025),
                    (0.0, 0.0),
                ],
                noise: 0.015,
                noise_decay: 0.15,
                amp: Adsr::new(0.048, 0.16, 0.83, 0.11),
                filter: Adsr::new(0.045, 0.22, 0.68, 0.11),
                cutoff_hz: 1_450.0,
                env_octaves: 0.65,
                resonance: 0.7,
                drive: 0.85,
                width: 0.30,
                reverb: 0.20,
                delay: 0.04,
                ..Patch::DEFAULT
            },
            // The oboe has strong even as well as odd partials and a narrower,
            // nasal spectrum. It remains distinct from the hollow clarinet.
            Self::Oboe => Patch {
                waveform: Waveform::Sine,
                partials: [
                    (2.0, 0.70),
                    (3.0, 0.48),
                    (4.0, 0.32),
                    (5.0, 0.19),
                    (6.0, 0.10),
                    (7.0, 0.05),
                ],
                noise: 0.015,
                noise_decay: 0.12,
                amp: Adsr::new(0.040, 0.13, 0.79, 0.12),
                filter: Adsr::new(0.035, 0.22, 0.75, 0.12),
                cutoff_hz: 2_000.0,
                env_octaves: 0.65,
                resonance: 0.75,
                vibrato_cents: 5.0,
                vibrato_hz: 5.1,
                vibrato_delay: 0.38,
                drive: 0.70,
                width: 0.28,
                reverb: 0.21,
                delay: 0.04,
                ..Patch::DEFAULT
            },
            // Rounded brass: the filter opens behind a soft attack, so the
            // horn blooms into its upper partials without a pitch scoop.
            Self::Horn => Patch {
                waveform: Waveform::Sine,
                partials: [
                    (2.0, 0.58),
                    (3.0, 0.28),
                    (4.0, 0.12),
                    (5.0, 0.06),
                    (6.0, 0.025),
                    (0.0, 0.0),
                ],
                amp: Adsr::new(0.095, 0.24, 0.82, 0.21),
                filter: Adsr::new(0.16, 0.30, 0.60, 0.20),
                cutoff_hz: 800.0,
                env_octaves: 1.15,
                resonance: 0.7,
                drive: 0.90,
                width: 0.34,
                reverb: 0.25,
                delay: 0.04,
                ..Patch::DEFAULT
            },
            Self::Trumpet => Patch {
                waveform: Waveform::Sine,
                partials: [
                    (2.0, 0.66),
                    (3.0, 0.48),
                    (4.0, 0.35),
                    (5.0, 0.24),
                    (6.0, 0.16),
                    (8.0, 0.07),
                ],
                noise: 0.025,
                noise_decay: 0.045,
                amp: Adsr::new(0.025, 0.15, 0.75, 0.12),
                filter: Adsr::new(0.020, 0.23, 0.55, 0.11),
                cutoff_hz: 1_550.0,
                env_octaves: 1.3,
                resonance: 0.7,
                vibrato_cents: 4.0,
                vibrato_hz: 5.3,
                vibrato_delay: 0.40,
                drive: 0.75,
                width: 0.30,
                reverb: 0.18,
                delay: 0.04,
                ..Patch::DEFAULT
            },
            // A nylon-style pluck: strong fundamental, brief pick noise, and
            // brightness that decays sooner than the body of the note.
            Self::Guitar => Patch {
                waveform: Waveform::Sine,
                partials: [
                    (2.0, 0.42),
                    (3.0, 0.24),
                    (4.0, 0.11),
                    (5.0, 0.065),
                    (6.0, 0.035),
                    (0.0, 0.0),
                ],
                noise: 0.045,
                noise_decay: 0.008,
                amp: Adsr::new(0.003, 1.15, 0.0, 0.22),
                filter: Adsr::new(0.002, 0.23, 0.06, 0.18),
                cutoff_hz: 1_100.0,
                env_octaves: 1.7,
                resonance: 0.7,
                drive: 0.90,
                width: 0.40,
                reverb: 0.16,
                delay: 0.07,
                ..Patch::DEFAULT
            },
            // Tuned bars: the first overtone sits two octaves above the
            // fundamental. Tremolo moves amplitude, leaving tuning intact.
            Self::Vibraphone => Patch {
                waveform: Waveform::Sine,
                partials: [
                    (4.0, 0.18),
                    (10.0, 0.035),
                    (0.0, 0.0),
                    (0.0, 0.0),
                    (0.0, 0.0),
                    (0.0, 0.0),
                ],
                amp: Adsr::new(0.004, 2.25, 0.0, 0.40),
                filter: Adsr::new(0.003, 0.65, 0.08, 0.35),
                cutoff_hz: 2_700.0,
                env_octaves: 1.0,
                resonance: 0.7,
                tremolo_depth: 0.13,
                tremolo_hz: 5.0,
                drive: 0.75,
                width: 0.60,
                reverb: 0.23,
                delay: 0.12,
                ..Patch::DEFAULT
            },
            Self::Kick => Patch {
                waveform: Waveform::Sine,
                voices: 1,
                noise: 0.09,
                noise_decay: 0.012,
                amp: Adsr::new(0.001, 0.26, 0.0, 0.05),
                filter: Adsr::new(0.001, 0.10, 0.0, 0.05),
                cutoff_hz: 120.0,
                env_octaves: 2.4,
                resonance: 0.7,
                pitch_octaves: 2.6,
                pitch_time: 0.045,
                drive: 2.4,
                width: 0.0,
                reverb: 0.04,
                delay: 0.0,
                ..Patch::DEFAULT
            },
            Self::Snare => Patch {
                waveform: Waveform::Triangle,
                voices: 1,
                noise: 0.85,
                noise_decay: 0.16,
                amp: Adsr::new(0.001, 0.17, 0.0, 0.09),
                filter: Adsr::new(0.001, 0.13, 0.0, 0.08),
                cutoff_hz: 1_900.0,
                env_octaves: 1.2,
                resonance: 1.1,
                pitch_octaves: 0.8,
                pitch_time: 0.03,
                drive: 1.7,
                width: 0.35,
                reverb: 0.20,
                delay: 0.12,
                ..Patch::DEFAULT
            },
            Self::Hat => Patch {
                waveform: Waveform::Square,
                voices: 1,
                noise: 0.70,
                noise_decay: 0.04,
                amp: Adsr::new(0.001, 0.055, 0.0, 0.04),
                filter: Adsr::new(0.001, 0.04, 0.0, 0.03),
                cutoff_hz: 11_000.0,
                env_octaves: 0.5,
                resonance: 1.0,
                high_pass_hz: 7_500.0,
                drive: 1.2,
                width: 0.9,
                reverb: 0.10,
                delay: 0.08,
                ..Patch::DEFAULT
            },
            Self::Perc => Patch {
                waveform: Waveform::Triangle,
                voices: 1,
                noise: 0.55,
                noise_decay: 0.09,
                amp: Adsr::new(0.001, 0.13, 0.0, 0.07),
                filter: Adsr::new(0.001, 0.10, 0.0, 0.06),
                cutoff_hz: 2_200.0,
                env_octaves: 1.0,
                resonance: 1.6,
                pitch_octaves: 1.1,
                pitch_time: 0.035,
                drive: 1.5,
                width: 0.85,
                reverb: 0.18,
                delay: 0.18,
                ..Patch::DEFAULT
            },
            _ => presets::patch(self),
        }
    }
}

/// A four-stage envelope in seconds, with sustain as a level.
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Adsr {
    pub const fn new(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Self {
            attack,
            decay,
            sustain,
            release,
        }
    }

    /// The envelope level `time` seconds into a note held for `gate` seconds.
    ///
    /// Decay and release are squared rather than linear. A linear fade sounds
    /// like a volume knob being turned; ears hear loudness roughly
    /// logarithmically, so the curved fall is what reads as a natural decay.
    pub fn level(&self, time: f32, gate: f32) -> f32 {
        if time < 0.0 || gate <= 0.0 {
            return 0.0;
        }
        // A short articulation still needs to reach its intended level. V1
        // fitted its attack inside the note; a fixed 550ms pad attack on a
        // 34ms written gate only reached 6% before fading away. Keep the patch's
        // normal attack for longer notes, and a smooth ramp for shorter ones.
        let attack = self.attack.min(gate.max(0.0) * 0.30);
        if time < gate {
            return self.held(time, attack);
        }
        let released = time - gate;
        if released >= self.release {
            return 0.0;
        }
        let from = self.held(gate, attack);
        let remaining = 1.0 - released / self.release.max(1e-4);
        from * remaining * remaining
    }

    /// The level while the key is still down.
    fn held(&self, time: f32, attack: f32) -> f32 {
        if time < attack {
            return time / attack.max(1e-4);
        }
        let decayed = time - attack;
        if decayed >= self.decay {
            return self.sustain;
        }
        let remaining = 1.0 - decayed / self.decay.max(1e-4);
        self.sustain + (1.0 - self.sustain) * remaining * remaining
    }

    /// How long a note keeps producing sound after its gate closes.
    pub fn tail(&self) -> f32 {
        self.release
    }
}

/// Everything that distinguishes one instrument's timbre from another's.
#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub waveform: Waveform,
    /// Fundamental oscillator level; noise-only percussion sets this to zero.
    pub oscillator_level: f32,
    /// Unison width: one centred oscillator, or a centred oscillator supported
    /// by a quiet symmetric detuned pair. Two is narrower than three.
    pub voices: u8,
    pub detune_cents: f32,
    /// A sine an octave down, for weight the filter will not remove.
    pub sub: f32,
    /// A second oscillator at this ratio, for inharmonic metallic timbres.
    pub ring: f32,
    pub ring_level: f32,
    /// Sine partials as (ratio, level), summed on top of the oscillator.
    ///
    /// A filtered waveform can only subtract, so every patch built that way is
    /// some shade of the same buzz. Real instruments differ in which harmonics
    /// they have at all: an organ is drawbars at octaves and fifths, a clarinet
    /// is odd harmonics with the evens missing, a flute is nearly a sine with a
    /// breath on top. Those cannot be reached by filtering a saw, so they are
    /// specified directly. A zero level is an unused slot.
    pub partials: [(f32, f32); 6],
    /// Optional extra decay for upper partials, in seconds. Zero sustains them.
    pub partial_decay_seconds: f32,
    pub noise: f32,
    pub noise_decay: f32,
    pub amp: Adsr,
    pub filter: Adsr,
    pub cutoff_hz: f32,
    /// How far the filter envelope opens the cutoff, in octaves.
    pub env_octaves: f32,
    pub resonance: f32,
    pub high_pass_hz: f32,
    /// A downward pitch sweep at the note's onset, which is what makes a kick.
    pub pitch_octaves: f32,
    pub pitch_time: f32,
    pub vibrato_cents: f32,
    pub vibrato_hz: f32,
    pub vibrato_delay: f32,
    /// Suggested glide between adjacent monophonic notes, in seconds.
    pub portamento_seconds: f32,
    /// Amplitude modulation, independent of vibrato so tuned bars stay in tune.
    pub tremolo_depth: f32,
    pub tremolo_hz: f32,
    pub drive: f32,
    pub width: f32,
    pub reverb: f32,
    pub delay: f32,
}

impl Patch {
    pub const DEFAULT: Self = Self {
        waveform: Waveform::Saw,
        oscillator_level: 1.0,
        voices: 1,
        detune_cents: 0.0,
        sub: 0.0,
        ring: 0.0,
        ring_level: 0.0,
        partials: [(0.0, 0.0); 6],
        partial_decay_seconds: 0.0,
        noise: 0.0,
        noise_decay: 0.05,
        amp: Adsr::new(0.01, 0.3, 0.6, 0.2),
        filter: Adsr::new(0.01, 0.3, 0.4, 0.2),
        cutoff_hz: 800.0,
        env_octaves: 2.0,
        resonance: 1.0,
        high_pass_hz: 0.0,
        pitch_octaves: 0.0,
        pitch_time: 0.0,
        vibrato_cents: 0.0,
        vibrato_hz: 5.0,
        vibrato_delay: 0.0,
        portamento_seconds: 0.0,
        tremolo_depth: 0.0,
        tremolo_hz: 5.0,
        drive: 1.0,
        width: 0.5,
        reverb: 0.3,
        delay: 0.2,
    };
}

impl Default for Patch {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A band-limited oscillator.
///
/// PolyBLEP subtracts a polynomial approximation of the step discontinuity at
/// each wrap, which removes most of the aliasing for the cost of two branches.
#[derive(Clone, Copy, Debug, Default)]
pub struct Oscillator {
    phase: f32,
}

impl Oscillator {
    pub fn with_phase(phase: f32) -> Self {
        Self {
            phase: phase.fract(),
        }
    }

    pub fn next(&mut self, waveform: Waveform, frequency: f32) -> f32 {
        // An overtone beyond the safe band must disappear. Clamping its
        // frequency would turn unrelated high partials into one false pitch.
        if !frequency.is_finite() || frequency <= 0.0 || frequency >= SR * 0.45 {
            return 0.0;
        }
        let increment = frequency / SR;
        let phase = self.phase;
        self.phase = (self.phase + increment).fract();
        match waveform {
            // The triangle's corners also produce harmonics above Nyquist.
            // PolyBLAMP rounds those slope changes without a moving filter.
            Waveform::Sine => (phase * TAU).sin(),
            Waveform::Triangle => {
                1.0 - 4.0 * (phase - 0.5).abs() + 4.0 * poly_blamp(phase, increment)
                    - 4.0 * poly_blamp((phase + 0.5).fract(), increment)
            }
            Waveform::Saw => 2.0 * phase - 1.0 - poly_blep(phase, increment),
            Waveform::Square => {
                let naive = if phase < 0.5 { 1.0 } else { -1.0 };
                naive + poly_blep(phase, increment) - poly_blep((phase + 0.5).fract(), increment)
            }
        }
    }
}

/// The correction applied either side of a waveform's discontinuity.
fn poly_blep(phase: f32, increment: f32) -> f32 {
    if increment <= 0.0 {
        return 0.0;
    }
    if phase < increment {
        let t = phase / increment;
        t + t - t * t - 1.0
    } else if phase > 1.0 - increment {
        let t = (phase - 1.0) / increment;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// Integral of PolyBLEP, correcting a triangle's slope discontinuities.
fn poly_blamp(phase: f32, increment: f32) -> f32 {
    if increment <= 0.0 {
        return 0.0;
    }
    if phase < increment {
        let remaining = 1.0 - phase / increment;
        increment * remaining * remaining * remaining / 3.0
    } else if phase > 1.0 - increment {
        let progress = (phase - 1.0) / increment + 1.0;
        increment * progress * progress * progress / 3.0
    } else {
        0.0
    }
}

/// A topology-preserving state variable filter.
///
/// Stable at any cutoff up to Nyquist and cheap enough to modulate per sample,
/// which matters because a filter that only moves per note is not what makes a
/// synthesiser sound alive.
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pass {
    Low,
    High,
}

impl Svf {
    pub fn process(&mut self, input: f32, cutoff_hz: f32, resonance: f32, pass: Pass) -> f32 {
        let cutoff = cutoff_hz.clamp(20.0, SR * 0.45);
        let g = (PI * cutoff / SR).tan();
        let k = 1.0 / resonance.clamp(0.5, 6.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        if !self.ic1.is_finite() || !self.ic2.is_finite() {
            *self = Self::default();
            return 0.0;
        }
        match pass {
            Pass::Low => v2,
            Pass::High => input - k * v1 - v2,
        }
    }
}

/// Deterministic white noise.
///
/// A seeded generator rather than a random one, so a score renders identically
/// every time: the Organist's feedback is only evidence if the same score
/// produces the same audio.
#[derive(Clone, Copy, Debug)]
pub struct Noise {
    state: u32,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self {
            state: seed | 0x9e37_79b9,
        }
    }

    pub fn next(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// One sounding note, rendered into the caller's buffers.
pub struct Voice {
    patch: Patch,
    frequency: f32,
    velocity: f32,
    gate_seconds: f32,
    oscillators: [Oscillator; 3],
    sub: Oscillator,
    ring: Oscillator,
    partials: [Oscillator; 6],
    noise: Noise,
    filter: Svf,
    high_pass: Svf,
    vibrato: Oscillator,
    tremolo: Oscillator,
    frame: usize,
    end_frame: Option<usize>,
    glide_from_frequency: f32,
    glide_seconds: f32,
}

impl Voice {
    pub fn new(patch: Patch, frequency: f32, velocity: f32, gate_seconds: f32, seed: u32) -> Self {
        // Notes have deterministic phase variation, while copies within one
        // note start together. The old nearly equidistant unison phases erased
        // the fundamental, leaving additive upper harmonics to dominate.
        let offset = (seed % 997) as f32 / 997.0;
        Self {
            patch,
            frequency,
            velocity,
            gate_seconds,
            oscillators: [
                Oscillator::with_phase(offset),
                Oscillator::with_phase(offset),
                Oscillator::with_phase(offset),
            ],
            sub: Oscillator::with_phase(offset * 0.5),
            ring: Oscillator::with_phase(offset * 0.25),
            partials: std::array::from_fn(|index| {
                Oscillator::with_phase(offset + index as f32 * 0.17)
            }),
            noise: Noise::new(seed.wrapping_mul(2_654_435_761)),
            filter: Svf::default(),
            high_pass: Svf::default(),
            vibrato: Oscillator::default(),
            tremolo: Oscillator::default(),
            frame: 0,
            end_frame: None,
            glide_from_frequency: frequency,
            glide_seconds: 0.0,
        }
    }

    /// Glide from a preceding note in semitone space, reaching the written
    /// pitch within the note's gate. Invalid inputs leave the note unchanged.
    pub fn with_glide_from(mut self, frequency: f32, seconds: f32) -> Self {
        if frequency.is_finite()
            && frequency > 0.0
            && self.frequency.is_finite()
            && self.frequency > 0.0
            && seconds.is_finite()
            && seconds > 0.0
        {
            self.glide_from_frequency = frequency;
            self.glide_seconds = seconds.min(self.gate_seconds.max(0.0));
        }
        self
    }

    fn frequency_at(&self, time: f32) -> f32 {
        if self.glide_seconds > 0.0 && time < self.glide_seconds {
            let progress = (time / self.glide_seconds).clamp(0.0, 1.0);
            let smooth = progress * progress * (3.0 - 2.0 * progress);
            self.glide_from_frequency * (self.frequency / self.glide_from_frequency).powf(smooth)
        } else {
            self.frequency
        }
    }

    /// End this voice at a harmonic boundary, measured from its own onset.
    /// The last eight milliseconds fade smoothly, including a sustained patch
    /// whose ordinary release would otherwise carry a wrong chord tone over.
    pub fn with_end_frames(mut self, frames: usize) -> Self {
        if frames < self.frames() {
            self.end_frame = Some(frames);
        }
        self
    }

    /// How many frames this voice will keep producing sound.
    pub fn frames(&self) -> usize {
        let seconds = self.gate_seconds + self.patch.amp.tail();
        let natural = (seconds * SR) as usize + 1;
        self.end_frame.map_or(natural, |end| natural.min(end))
    }

    pub fn finished(&self) -> bool {
        self.frame >= self.frames()
    }

    /// The next sample, before panning. `cutoff_scale` is the arrangement's
    /// automation, applied on top of the patch's own filter envelope.
    pub fn next(&mut self, cutoff_scale: f32) -> f32 {
        if self.finished() {
            return 0.0;
        }
        // Derive time from the frame counter: repeated f32 additions drift by
        // milliseconds on long held notes and miss precise chord boundaries.
        let frame = self.frame;
        let time = frame as f32 / SR;
        self.frame += 1;

        let amplitude = self.patch.amp.level(time, self.gate_seconds);
        if amplitude <= 0.0 {
            return 0.0;
        }

        let mut frequency = self.frequency_at(time);
        if self.patch.pitch_octaves > 0.0 && time < self.patch.pitch_time {
            let remaining = 1.0 - time / self.patch.pitch_time.max(1e-4);
            frequency *= 2.0_f32.powf(self.patch.pitch_octaves * remaining * remaining);
        }
        if self.patch.vibrato_cents > 0.0 {
            let onset = ((time - self.patch.vibrato_delay) / 0.5).clamp(0.0, 1.0);
            let depth = self.vibrato.next(Waveform::Sine, self.patch.vibrato_hz) * onset;
            frequency *= 2.0_f32.powf(self.patch.vibrato_cents * depth / 1_200.0);
        }

        let voices = self.patch.voices.clamp(1, 3);
        let mut raw = self.oscillators[0].next(self.patch.waveform, frequency);
        if voices > 1 && self.patch.detune_cents > 0.0 {
            // Keep a strong oscillator exactly on the written pitch. Quiet
            // detuned neighbours add motion without being able to cancel the
            // centre as their phases separate during a sustained note.
            let neighbour_level = if voices == 2 { 0.14 } else { 0.20 };
            let spread = if voices == 2 { 0.5 } else { 1.0 };
            let detune = 2.0_f32.powf(spread * self.patch.detune_cents / 1_200.0);
            raw += neighbour_level
                * (self.oscillators[1].next(self.patch.waveform, frequency / detune)
                    + self.oscillators[2].next(self.patch.waveform, frequency * detune));
            raw /= 1.0 + 2.0 * neighbour_level;
        }
        raw *= self.patch.oscillator_level;

        let partial_envelope = if self.patch.partial_decay_seconds > 0.0 {
            (-time / self.patch.partial_decay_seconds).exp()
        } else {
            1.0
        };

        if self.patch.sub > 0.0 {
            raw += self.sub.next(Waveform::Sine, frequency * 0.5) * self.patch.sub;
        }
        if self.patch.ring_level > 0.0 {
            let ring_frequency = frequency * self.patch.ring;
            raw += self.ring.next(Waveform::Sine, ring_frequency)
                * self.patch.ring_level
                * partial_envelope
                * partial_band_gain(ring_frequency);
        }
        // The additive partials: the drawbars of the organ, the odd harmonics
        // of the reed, the two faint overtones that make a flute a flute.
        for (index, (ratio, level)) in self.patch.partials.iter().enumerate() {
            if *level <= 0.0 || *ratio <= 0.0 {
                continue;
            }
            let partial_frequency = frequency * ratio;
            raw += self.partials[index].next(Waveform::Sine, partial_frequency)
                * level
                * partial_envelope
                * partial_band_gain(partial_frequency);
        }
        if self.patch.noise > 0.0 {
            let decay = (-time / self.patch.noise_decay.max(1e-4)).exp();
            raw += self.noise.next() * self.patch.noise * decay;
        }

        let envelope = self.patch.filter.level(time, self.gate_seconds);
        let cutoff = self.patch.cutoff_hz
            * 2.0_f32.powf(self.patch.env_octaves * envelope)
            * cutoff_scale.clamp(0.05, 8.0);
        let mut value = self
            .filter
            .process(raw, cutoff, self.patch.resonance, Pass::Low);
        if self.patch.high_pass_hz > 0.0 {
            value = self
                .high_pass
                .process(value, self.patch.high_pass_hz, 0.7, Pass::High);
        }

        // Drive before the amplitude envelope, so quiet notes stay clean and
        // loud ones get the harmonic thickening — the way a real signal path
        // behaves, and the reason velocity ends up changing timbre, not volume.
        let driven = (value * self.patch.drive).tanh();
        let tremolo = if self.patch.tremolo_depth > 0.0 {
            let wave = self.tremolo.next(Waveform::Sine, self.patch.tremolo_hz);
            1.0 - self.patch.tremolo_depth.clamp(0.0, 1.0) * (0.5 + 0.5 * wave)
        } else {
            1.0
        };
        let boundary_fade = self.end_frame.map_or(1.0, |end| {
            let remaining = end.saturating_sub(frame + 1) as f32;
            let fade_frames = (SR * 0.008).min(end as f32).max(1.0);
            (remaining / fade_frames).clamp(0.0, 1.0)
        });
        driven * amplitude * self.velocity * tremolo * boundary_fade
    }
}

/// Roll additive harmonics off before the oscillator's safe upper limit.
fn partial_band_gain(frequency: f32) -> f32 {
    ((SR * 0.45 - frequency) / (SR * 0.05)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_VOICES: &[Instrument] = &[
        Instrument::Violin,
        Instrument::Clarinet,
        Instrument::Oboe,
        Instrument::Horn,
        Instrument::Trumpet,
        Instrument::Guitar,
        Instrument::Vibraphone,
    ];

    fn frequency(midi: u8) -> f32 {
        440.0 * 2.0_f32.powf((midi as f32 - 69.0) / 12.0)
    }

    fn render(instrument: Instrument, midi: u8, seconds: f32) -> Vec<f32> {
        let mut voice = Voice::new(instrument.patch(), frequency(midi), 0.7, seconds, 42);
        (0..(seconds * SR) as usize)
            .map(|_| voice.next(1.0))
            .collect()
    }

    #[test]
    fn short_notes_reach_audible_level_across_the_complete_catalog() {
        // The shortest legal authored gate at the fastest supported tempo.
        let gate = 0.125 * 60.0 / 220.0;
        for &instrument in Instrument::ALL {
            let patch = instrument.patch();
            let (low, high) = instrument.natural_range();
            for midi in [low, low + (high - low) / 2, high] {
                let mut voice = Voice::new(patch, frequency(midi), 0.7, gate, 42);
                let samples: Vec<_> = (0..voice.frames()).map(|_| voice.next(1.0)).collect();
                assert!(samples.iter().all(|sample| sample.is_finite()));
                assert_eq!(
                    samples[0], 0.0,
                    "{instrument:?}: the attack must start smoothly"
                );
                let onset = &samples[..(gate * SR) as usize];
                let peak = onset
                    .iter()
                    .map(|sample| sample.abs())
                    .fold(0.0_f32, f32::max);
                assert!(
                    peak > 0.02,
                    "{instrument:?} MIDI {midi}: short onset is too weak ({peak})"
                );
                let envelope_peak = (0..onset.len())
                    .map(|frame| patch.amp.level(frame as f32 / SR, gate))
                    .fold(0.0_f32, f32::max);
                assert!(
                    envelope_peak > 0.98,
                    "{instrument:?}: its short attack never reached full level"
                );
                let before = patch.amp.level(gate - 1.0 / SR, gate);
                let after = patch.amp.level(gate + 1.0 / SR, gate);
                assert!(
                    (before - after).abs() < 0.01,
                    "{instrument:?}: gate release discontinuity"
                );
            }
        }
    }

    #[test]
    fn longer_notes_keep_each_patches_original_envelope_stages() {
        for &instrument in Instrument::ALL {
            for envelope in [instrument.patch().amp, instrument.patch().filter] {
                let gate = (envelope.attack + envelope.decay + 1.0).max(envelope.attack / 0.30);
                assert!((envelope.level(envelope.attack * 0.5, gate) - 0.5).abs() < 0.001);
                assert!((envelope.level(envelope.attack, gate) - 1.0).abs() < 0.001);
                let middle_decay = envelope.attack + envelope.decay * 0.5;
                let expected = envelope.sustain + (1.0 - envelope.sustain) * 0.25;
                assert!((envelope.level(middle_decay, gate) - expected).abs() < 0.001);
                assert!(
                    (envelope.level(gate + envelope.release * 0.5, gate) - envelope.sustain * 0.25)
                        .abs()
                        < 0.001
                );
                assert_eq!(envelope.level(0.0, 0.0), 0.0);
            }
        }
    }

    // Autocorrelation measures the rendered waveform's repeating period,
    // including harmonics and the filter, rather than inspecting patch data.
    fn measured_frequency(samples: &[f32], expected: f32) -> f32 {
        let nominal_lag = SR / expected;
        let first = (nominal_lag * 0.90).floor().max(2.0) as usize;
        let last = (nominal_lag * 1.10).ceil() as usize;
        let window = samples.len() - last - 1;
        let error = |lag: usize| -> f64 {
            samples[..window]
                .iter()
                .zip(&samples[lag..lag + window])
                .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                .sum::<f64>()
                / window as f64
        };
        let lag = (first..=last)
            .min_by(|a, b| error(*a).total_cmp(&error(*b)))
            .unwrap();
        let left = error(lag - 1);
        let centre = error(lag);
        let right = error(lag + 1);
        let correction = 0.5 * (left - right) / (left - 2.0 * centre + right);
        SR / (lag as f32 + correction as f32)
    }

    #[test]
    fn additional_instruments_render_in_tune_across_their_ranges() {
        for instrument in NEW_VOICES {
            let (low, high) = instrument.natural_range();
            for midi in [low, (low + high) / 2, high] {
                let samples = render(*instrument, midi, 0.32);
                assert!(samples.iter().all(|sample| sample.is_finite()));
                let steady = &samples[(SR * 0.12) as usize..(SR * 0.30) as usize];
                let rms = (steady.iter().map(|v| v * v).sum::<f32>() / steady.len() as f32).sqrt();
                assert!(
                    rms > 0.012,
                    "{instrument:?} MIDI {midi} is too quiet: {rms}"
                );
                let actual = measured_frequency(steady, frequency(midi));
                let cents = 1_200.0 * (actual / frequency(midi)).log2();
                assert!(
                    cents.abs() < 14.0,
                    "{instrument:?} MIDI {midi} drifted {cents:.2} cents"
                );
            }
        }
    }

    #[test]
    fn named_parts_select_their_actual_instrument_before_generic_roles() {
        for (name, expected) in [
            ("Violin Lead", Instrument::Violin),
            ("Bass Clarinet", Instrument::Clarinet),
            ("Oboe Melody", Instrument::Oboe),
            ("French Horn Harmony", Instrument::Horn),
            ("Trumpet Motif", Instrument::Trumpet),
            ("Nylon Guitar Arp", Instrument::Guitar),
            ("Vibraphone Chords", Instrument::Vibraphone),
            ("Bassoon", Instrument::Bassoon),
        ] {
            assert_eq!(Instrument::from_name(name), expected);
        }
    }

    #[test]
    fn inaudible_partials_do_not_turn_into_a_clamped_false_pitch() {
        let mut patch = Patch {
            waveform: Waveform::Sine,
            cutoff_hz: 12_000.0,
            ..Patch::DEFAULT
        };
        let mut clean = Voice::new(patch, 5_000.0, 0.7, 0.1, 42);
        patch.partials[0] = (5.0, 0.9);
        patch.ring = 6.0;
        patch.ring_level = 0.5;
        let mut with_ultrasonic_partials = Voice::new(patch, 5_000.0, 0.7, 0.1, 42);
        for _ in 0..4_000 {
            assert_eq!(clean.next(1.0), with_ultrasonic_partials.next(1.0));
        }
    }

    #[test]
    fn harmony_boundary_ends_a_sustained_voice_exactly_with_a_smooth_fade() {
        let frames = SAMPLE_RATE as usize / 2;
        let patch = Instrument::Organ.patch();
        let mut voice = Voice::new(patch, 220.0, 0.7, 3.0, 42).with_end_frames(frames);
        assert_eq!(voice.frames(), frames);
        let samples: Vec<f32> = (0..frames).map(|_| voice.next(1.0)).collect();
        assert!(voice.finished());
        assert_eq!(samples[frames - 1], 0.0);
        assert_eq!(voice.next(1.0), 0.0);
        let last_two_ms = &samples[frames - (SR * 0.002) as usize..];
        assert!(
            last_two_ms
                .windows(2)
                .all(|pair| (pair[1] - pair[0]).abs() < 0.04)
        );
        assert!(samples[frames / 2].abs() > 0.001);
    }

    #[test]
    fn boundary_cap_does_not_extend_short_notes_or_leave_one_frame_sounding() {
        let make = || Voice::new(Instrument::Guitar.patch(), 220.0, 0.7, 0.1, 42);
        let natural_frames = make().frames();
        assert_eq!(
            make().with_end_frames(natural_frames + 1_000).frames(),
            natural_frames
        );
        let mut empty = make().with_end_frames(0);
        assert!(empty.finished());
        assert_eq!(empty.next(1.0), 0.0);
        let mut single = make().with_end_frames(1);
        assert_eq!(single.next(1.0), 0.0);
        assert!(single.finished());
    }

    #[test]
    fn complete_catalog_renders_finite_audible_notes_at_each_register() {
        let mut problems = Vec::new();
        for instrument in Instrument::ALL {
            let (low, high) = instrument.natural_range();
            for midi in [low, low + (high - low) / 2, high] {
                let mut voice = Voice::new(instrument.patch(), frequency(midi), 0.7, 0.35, 42);
                let mut peak = 0.0_f32;
                while !voice.finished() {
                    let sample = voice.next(1.0);
                    assert!(
                        sample.is_finite(),
                        "{instrument:?} MIDI {midi}: invalid sample"
                    );
                    peak = peak.max(sample.abs());
                }
                if peak < 0.007 {
                    problems.push(format!("{instrument:?} MIDI {midi}: inaudible peak {peak}"));
                }
                assert_eq!(voice.next(1.0), 0.0);
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn complete_pitched_catalog_preserves_the_written_note() {
        let mut problems = Vec::new();
        for instrument in Instrument::ALL
            .iter()
            .filter(|voice| voice.follows_harmony())
        {
            let patch = instrument.patch();
            let (low, high) = instrument.natural_range();
            for midi in [low, low + (high - low) / 2, high] {
                let samples = render(*instrument, midi, 0.34);
                // Damped plucks are measured before their intended decay to
                // silence; sustained instruments are measured after the onset.
                let (start, end) = if patch.amp.sustain < 0.02 && patch.amp.decay < 0.7 {
                    (0.025, 0.14)
                } else {
                    (0.12, 0.30)
                };
                let steady = &samples[(SR * start) as usize..(SR * end) as usize];
                let actual = measured_frequency(steady, frequency(midi));
                let cents = 1_200.0 * (actual / frequency(midi)).log2();
                let tolerance = if patch.voices > 1 && patch.detune_cents > 0.0 {
                    30.0
                } else {
                    14.0
                };
                if !cents.is_finite() || cents.abs() >= tolerance {
                    problems.push(format!("{instrument:?} MIDI {midi}: {cents:.2} cents"));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn instrument_names_do_not_alias_identical_audio() {
        // Render identical input, remove loudness differences, and compare the
        // waveforms. Changing only a name, pan, send, or gain cannot pass this.
        let voices: Vec<_> = Instrument::ALL
            .iter()
            .map(|instrument| {
                let samples = render(*instrument, 69, 0.42);
                let rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
                    / samples.len() as f32)
                    .sqrt();
                (
                    *instrument,
                    samples
                        .into_iter()
                        .map(|sample| sample / rms)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        for (index, (instrument, samples)) in voices.iter().enumerate() {
            for (other, other_samples) in &voices[index + 1..] {
                let difference = (samples
                    .iter()
                    .zip(other_samples)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
                    / samples.len() as f32)
                    .sqrt();
                assert!(
                    difference > 0.01,
                    "{instrument:?} and {other:?} have indistinguishable recipes ({difference})"
                );
            }
        }
    }

    #[test]
    fn theremin_glides_smoothly_to_the_target_then_stays_in_tune() {
        let patch = Instrument::Theremin.patch();
        assert!(patch.portamento_seconds > 0.0);
        assert!((4.0..=12.0).contains(&patch.vibrato_cents));
        let mut voice =
            Voice::new(patch, 440.0, 0.7, 0.8, 42).with_glide_from(220.0, patch.portamento_seconds);
        assert_eq!(voice.frequency_at(0.0), 220.0);
        assert!(
            (voice.frequency_at(patch.portamento_seconds / 2.0) - (220.0_f32 * 440.0).sqrt()).abs()
                < 0.01
        );
        assert_eq!(voice.frequency_at(patch.portamento_seconds), 440.0);
        let mut previous = 220.0;
        for frame in 0..(patch.portamento_seconds * SR) as usize {
            let current = voice.frequency_at(frame as f32 / SR);
            assert!(current >= previous);
            assert!(current - previous < 0.10);
            previous = current;
        }
        let samples: Vec<_> = (0..(SR * 0.8) as usize).map(|_| voice.next(1.0)).collect();
        assert!(
            samples
                .windows(2)
                .all(|pair| (pair[1] - pair[0]).abs() < 0.07)
        );
        let stable = &samples[(SR * 0.18) as usize..(SR * 0.28) as usize];
        let cents = 1_200.0 * (measured_frequency(stable, 440.0) / 440.0).log2();
        assert!(cents.abs() < 2.0, "glide did not settle: {cents:.2} cents");
    }

    #[test]
    fn short_glides_arrive_within_the_gate_and_invalid_values_are_ignored() {
        let make = || Voice::new(Instrument::Theremin.patch(), 440.0, 0.7, 0.04, 42);
        let voice = make().with_glide_from(880.0, 1.0);
        assert_eq!(voice.frequency_at(0.0), 880.0);
        assert_eq!(voice.frequency_at(0.04), 440.0);
        for (from, seconds) in [
            (f32::NAN, 0.1),
            (f32::INFINITY, 0.1),
            (0.0, 0.1),
            (-1.0, 0.1),
            (220.0, -1.0),
            (220.0, f32::NAN),
        ] {
            assert_eq!(
                make().with_glide_from(from, seconds).frequency_at(0.0),
                440.0
            );
        }
    }

    fn spectral_amplitude(samples: &[f32], frequency: f32) -> f64 {
        let (mut real, mut imaginary) = (0.0, 0.0);
        let length = samples.len();
        for (index, &sample) in samples.iter().enumerate() {
            let position = index as f64;
            let window = 0.5 - 0.5 * (std::f64::consts::TAU * position / (length - 1) as f64).cos();
            let angle = std::f64::consts::TAU * f64::from(frequency) * position / f64::from(SR);
            real += f64::from(sample) * window * angle.cos();
            imaginary += f64::from(sample) * window * angle.sin();
        }
        (real * real + imaginary * imaginary).sqrt()
    }

    #[test]
    fn ensemble_patches_retain_a_stronger_fundamental_than_any_upper_partial() {
        // A pitch detector still reports the right period when an ensemble's
        // fundamental cancels. Check the actual harmonic balance as well.
        for instrument in [
            Instrument::StringEnsemble,
            Instrument::BrassEnsemble,
            Instrument::Choir,
            Instrument::WarmPad,
        ] {
            for midi in [48, 69] {
                let samples = render(instrument, midi, 0.30);
                let onset = &samples[(0.03 * SR) as usize..(0.22 * SR) as usize];
                let fundamental = spectral_amplitude(onset, frequency(midi));
                for harmonic in 2..=8 {
                    let overtone = spectral_amplitude(onset, frequency(midi) * harmonic as f32);
                    assert!(
                        fundamental > overtone * 1.25,
                        "{instrument:?} MIDI {midi}: harmonic {harmonic} overwhelms the pitch centre"
                    );
                }
            }
        }
    }

    #[test]
    fn detuned_neighbours_cannot_cancel_a_held_notes_pitch_centre() {
        let patch = Patch {
            waveform: Waveform::Sine,
            voices: 3,
            detune_cents: 9.0,
            amp: Adsr::new(0.005, 0.01, 1.0, 0.05),
            filter: Adsr::new(0.005, 0.01, 1.0, 0.05),
            cutoff_hz: 4_000.0,
            env_octaves: 0.0,
            ..Patch::DEFAULT
        };
        let mut voice = Voice::new(patch, 440.0, 0.7, 2.0, 42);
        let samples: Vec<_> = (0..(2.0 * SR) as usize).map(|_| voice.next(1.0)).collect();
        for window in samples.chunks_exact((0.025 * SR) as usize).skip(1) {
            let rms = (window.iter().map(|sample| sample * sample).sum::<f32>()
                / window.len() as f32)
                .sqrt();
            assert!(rms > 0.15, "unison phase cancellation reduced RMS to {rms}");
        }
    }

    #[test]
    fn triangle_corner_correction_reduces_folded_upper_harmonics() {
        let pitch = 3_520.0;
        // The seventh harmonic is above Nyquist and folds back to 23,360Hz.
        let folded_seventh = SR - 7.0 * pitch;
        let mut corrected = Oscillator::with_phase(0.125);
        let mut naive_phase = 0.125_f32;
        let mut fixed = Vec::new();
        let mut naive = Vec::new();
        for _ in 0..SAMPLE_RATE / 2 {
            fixed.push(corrected.next(Waveform::Triangle, pitch));
            naive.push(1.0 - 4.0 * (naive_phase - 0.5).abs());
            naive_phase = (naive_phase + pitch / SR).fract();
        }
        let before = spectral_amplitude(&naive, folded_seventh) / spectral_amplitude(&naive, pitch);
        let after = spectral_amplitude(&fixed, folded_seventh) / spectral_amplitude(&fixed, pitch);
        assert!(
            after < before * 0.45,
            "triangle alias ratio: {before:.5} -> {after:.5}"
        );
        eprintln!(
            "triangle folded seventh relative to fundamental: {:.2}dB -> {:.2}dB",
            20.0 * before.log10(),
            20.0 * after.log10()
        );
    }
}
