//! Voice-level synthesis: band-limited oscillators, envelopes, filter, patches.
//!
//! The previous engine summed naive saw and square waves straight into the mix.
//! Those have infinite harmonics, so every partial above 24 kHz folded back down
//! as inharmonic noise — the metallic buzz that made the court sound cheap. Saw
//! and pulse are now band-limited with PolyBLEP, and every voice runs through a
//! resonant filter with its own envelope, which is what makes a synthesiser
//! sound like an instrument rather than a tone generator.

use std::f32::consts::{PI, TAU};

use serde::{Deserialize, Serialize};

use crate::{SAMPLE_RATE, Waveform};

const SR: f32 = SAMPLE_RATE as f32;

/// The role a track plays, which selects its patch and its arrangement rules.
///
/// V1 carried 74 named patches. The court does not need that many to sound
/// professional; it needs each part to occupy its own register, envelope, and
/// stereo position so the mix has depth instead of a single crowded midrange.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Instrument {
    SubBass,
    Bass,
    Pad,
    #[default]
    Keys,
    Pluck,
    Lead,
    Bell,
    Arp,
    Organ,
    Piano,
    Flute,
    Reed,
    Cello,
    Harp,
    Kick,
    Snare,
    Hat,
    Perc,
}

impl Instrument {
    /// The role implied by a track's name.
    ///
    /// Scores arrive from the model as free text, so the name is the only hint
    /// available. Matching on it means an Organist who writes "Throne Bass"
    /// gets a bass patch without having to know the schema.
    pub fn from_name(name: &str) -> Self {
        let name = name.to_ascii_lowercase();
        for (needle, instrument) in [
            ("sub", Self::SubBass),
            ("kick", Self::Kick),
            ("snare", Self::Snare),
            ("clap", Self::Snare),
            ("hat", Self::Hat),
            ("cymbal", Self::Hat),
            ("shaker", Self::Hat),
            ("perc", Self::Perc),
            ("drum", Self::Perc),
            ("tom", Self::Perc),
            ("bass", Self::Bass),
            ("pad", Self::Pad),
            ("string", Self::Pad),
            ("choir", Self::Pad),
            ("harmony", Self::Pad),
            ("organ", Self::Organ),
            ("piano", Self::Piano),
            ("keyboard", Self::Piano),
            ("flute", Self::Flute),
            ("piccolo", Self::Flute),
            ("whistle", Self::Flute),
            ("clarinet", Self::Reed),
            ("oboe", Self::Reed),
            ("reed", Self::Reed),
            ("bassoon", Self::Reed),
            ("cello", Self::Cello),
            ("viola", Self::Cello),
            ("violin", Self::Cello),
            ("harp", Self::Harp),
            ("lyre", Self::Harp),
            ("chord", Self::Keys),
            ("key", Self::Keys),
            ("arp", Self::Arp),
            ("pluck", Self::Pluck),
            ("bell", Self::Bell),
            ("glass", Self::Bell),
            ("chime", Self::Bell),
            ("lead", Self::Lead),
            ("motif", Self::Lead),
            ("melody", Self::Lead),
        ] {
            if name.contains(needle) {
                return instrument;
            }
        }
        Self::Keys
    }

    pub const fn is_percussive(self) -> bool {
        matches!(self, Self::Kick | Self::Snare | Self::Hat | Self::Perc)
    }

    /// Percussion is unpitched, so it must not be transposed by the harmony.
    pub const fn follows_harmony(self) -> bool {
        !self.is_percussive()
    }

    /// How much of its step a note holds, before the release tail.
    pub const fn gate(self) -> f32 {
        match self {
            Self::Pad => 1.7,
            Self::Keys => 0.95,
            Self::SubBass => 0.9,
            Self::Bass => 0.72,
            Self::Lead => 0.85,
            Self::Bell => 1.2,
            Self::Organ => 1.0,
            Self::Piano => 1.6,
            Self::Flute => 1.1,
            Self::Reed => 1.0,
            Self::Cello => 1.9,
            Self::Harp => 1.5,
            Self::Arp | Self::Pluck => 0.5,
            Self::Kick | Self::Snare | Self::Perc => 0.3,
            Self::Hat => 0.16,
        }
    }

    /// The range this voice actually lives in, as MIDI note numbers.
    ///
    /// An ensemble stays legible because each instrument occupies its own part
    /// of the register. Nothing enforced that here, so a cello could be written
    /// an octave above a flute and the mix became a crowd of parts competing
    /// for the same few hundred hertz. Notes outside the range are folded back
    /// by octaves rather than refused, which keeps the written line intact.
    pub const fn natural_range(self) -> (u8, u8) {
        match self {
            Self::SubBass => (24, 48),
            Self::Bass => (28, 55),
            Self::Cello => (36, 69),
            Self::Organ => (36, 84),
            Self::Piano => (33, 91),
            Self::Harp => (36, 91),
            Self::Pad => (45, 79),
            Self::Keys => (48, 84),
            Self::Reed => (50, 84),
            Self::Pluck => (55, 91),
            Self::Lead => (60, 91),
            Self::Flute => (72, 98),
            Self::Bell => (72, 100),
            Self::Arp => (67, 98),
            // Percussion is unpitched and its patch fixes the sound.
            Self::Kick => (30, 42),
            Self::Snare => (56, 68),
            Self::Hat => (78, 90),
            Self::Perc => (50, 80),
        }
    }

    /// The name this voice goes by, for track labels and reports.
    pub const fn label(self) -> &'static str {
        match self {
            Self::SubBass => "Sub Bass",
            Self::Bass => "Bass",
            Self::Pad => "Pad",
            Self::Keys => "Keys",
            Self::Pluck => "Pluck",
            Self::Lead => "Lead",
            Self::Bell => "Bell",
            Self::Arp => "Arp",
            Self::Organ => "Organ",
            Self::Piano => "Piano",
            Self::Flute => "Flute",
            Self::Reed => "Reed",
            Self::Cello => "Cello",
            Self::Harp => "Harp",
            Self::Kick => "Kick",
            Self::Snare => "Snare",
            Self::Hat => "Hat",
            Self::Perc => "Perc",
        }
    }

    /// Voices that can take over this one's job in an arrangement.
    ///
    /// A part written for keys is a middle voice whether an organ or a reed
    /// plays it; what changes is the colour, not the function. Grouping voices
    /// by role lets the runtime re-orchestrate a piece — swap the timbre while
    /// leaving the register, the rhythm, and the harmony exactly where they
    /// are. Percussion has no family: a kick that became a snare would be a
    /// different groove, not a different sound.
    pub const fn family(self) -> &'static [Self] {
        const LOW: &[Instrument] = &[Instrument::Bass, Instrument::Cello, Instrument::SubBass];
        const MIDDLE: &[Instrument] = &[
            Instrument::Keys,
            Instrument::Organ,
            Instrument::Pad,
            Instrument::Reed,
        ];
        const COLOUR: &[Instrument] = &[
            Instrument::Pluck,
            Instrument::Harp,
            Instrument::Piano,
            Instrument::Bell,
        ];
        const TOP: &[Instrument] = &[Instrument::Lead, Instrument::Flute, Instrument::Arp];
        match self {
            Self::SubBass | Self::Bass | Self::Cello => LOW,
            Self::Keys | Self::Organ | Self::Pad | Self::Reed => MIDDLE,
            Self::Pluck | Self::Harp | Self::Piano | Self::Bell => COLOUR,
            Self::Lead | Self::Flute | Self::Arp => TOP,
            Self::Kick | Self::Snare | Self::Hat | Self::Perc => &[],
        }
    }

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
        if time < 0.0 {
            return 0.0;
        }
        if time < gate {
            return self.held(time);
        }
        let released = time - gate;
        if released >= self.release {
            return 0.0;
        }
        let from = self.held(gate);
        let remaining = 1.0 - released / self.release.max(1e-4);
        from * remaining * remaining
    }

    /// The level while the key is still down.
    fn held(&self, time: f32) -> f32 {
        if time < self.attack {
            return time / self.attack.max(1e-4);
        }
        let decayed = time - self.attack;
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
    /// Detuned copies of the oscillator, which is what gives a patch width.
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
    pub drive: f32,
    pub width: f32,
    pub reverb: f32,
    pub delay: f32,
}

impl Patch {
    pub const DEFAULT: Self = Self {
        waveform: Waveform::Saw,
        voices: 1,
        detune_cents: 0.0,
        sub: 0.0,
        ring: 0.0,
        ring_level: 0.0,
        partials: [(0.0, 0.0); 6],
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
        let increment = (frequency / SR).clamp(0.0, 0.45);
        let phase = self.phase;
        self.phase = (self.phase + increment).fract();
        match waveform {
            // Sine has one partial and triangle's fall off as 1/n squared, so
            // neither needs correcting at any pitch the court will play.
            Waveform::Sine => (phase * TAU).sin(),
            Waveform::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
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
    elapsed: f32,
}

impl Voice {
    pub fn new(patch: Patch, frequency: f32, velocity: f32, gate_seconds: f32, seed: u32) -> Self {
        // Each voice starts its oscillators at a different point in the cycle.
        // Identical phases make every note begin with the same transient click
        // and make detuned copies cancel instead of beat.
        let offset = (seed % 997) as f32 / 997.0;
        Self {
            patch,
            frequency,
            velocity,
            gate_seconds,
            oscillators: [
                Oscillator::with_phase(offset),
                Oscillator::with_phase(offset + 0.37),
                Oscillator::with_phase(offset + 0.71),
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
            elapsed: 0.0,
        }
    }

    /// How many frames this voice will keep producing sound.
    pub fn frames(&self) -> usize {
        let seconds = self.gate_seconds + self.patch.amp.tail();
        (seconds * SR) as usize + 1
    }

    pub fn finished(&self) -> bool {
        self.elapsed > self.gate_seconds + self.patch.amp.tail()
    }

    /// The next sample, before panning. `cutoff_scale` is the arrangement's
    /// automation, applied on top of the patch's own filter envelope.
    pub fn next(&mut self, cutoff_scale: f32) -> f32 {
        if self.finished() {
            return 0.0;
        }
        let time = self.elapsed;
        self.elapsed += 1.0 / SR;

        let amplitude = self.patch.amp.level(time, self.gate_seconds);
        if amplitude <= 0.0 {
            return 0.0;
        }

        let mut frequency = self.frequency;
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
        let mut raw = 0.0;
        for index in 0..voices {
            // Spread the detuning symmetrically so the centre of the stack
            // stays on pitch: -d, 0, +d rather than 0, +d, +2d.
            let offset = index as f32 - (voices as f32 - 1.0) * 0.5;
            let detuned = frequency * 2.0_f32.powf(offset * self.patch.detune_cents / 1_200.0);
            raw += self.oscillators[index as usize].next(self.patch.waveform, detuned);
        }
        raw /= voices as f32;

        if self.patch.sub > 0.0 {
            raw += self.sub.next(Waveform::Sine, frequency * 0.5) * self.patch.sub;
        }
        if self.patch.ring_level > 0.0 {
            raw +=
                self.ring.next(Waveform::Sine, frequency * self.patch.ring) * self.patch.ring_level;
        }
        // The additive partials: the drawbars of the organ, the odd harmonics
        // of the reed, the two faint overtones that make a flute a flute.
        for (index, (ratio, level)) in self.patch.partials.iter().enumerate() {
            if *level <= 0.0 || *ratio <= 0.0 {
                continue;
            }
            raw += self.partials[index].next(Waveform::Sine, frequency * ratio) * level;
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
        driven * amplitude * self.velocity
    }
}
